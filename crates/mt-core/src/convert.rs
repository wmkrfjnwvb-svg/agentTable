//! Converting values between data types (used when a field's type is changed).
//!
//! [`convertibility`] says what to expect for a pair of types before any data is touched;
//! [`convert`] converts one value and reports whether information was lost;
//! [`preview_conversion`] summarises a whole column so that a person can decide.
//!
//! # Rules
//!
//! | From → to | Result |
//! |---|---|
//! | same type | unchanged |
//! | anything → `text` | the plain text form (line breaks of long text become spaces: lossy) |
//! | anything → `long_text` | the plain text form |
//! | anything → `json` | natural JSON ([`Value::to_natural_json`]) |
//! | `text`/`long_text` → other | read with [`parse_input`]; text that cannot be read fails |
//! | `json` → other | strings and numbers are read as input; booleans, ID arrays and money objects map directly |
//! | `number` ↔ `percent` | 25 ↔ 25 % |
//! | `number` → `money` | needs a default currency in the context; `money` → `number` drops the currency (lossy) |
//! | `bool` → `number` | 1 / 0; `number` → `bool` only for 1 and 0 |
//! | `duration` ↔ `number` | milliseconds (a number must be whole) |
//! | `date` → `datetime` | midnight UTC; `datetime` → `date` drops the time (lossy unless midnight) |
//! | `reference` → `multi_reference` | one-item list; back keeps the first item (lossy if more) |
//! | everything else | impossible |
//!
//! Empty values (null, blank text, empty lists) convert to null.

use crate::coerce::{parse_input, points_to_fraction, ParseContext};
use crate::decimal::{parse_decimal, Decimal};
use crate::error::CoreError;
use crate::field::DataType;
use crate::money::{Currency, Money};
use crate::time::DurationMs;
use crate::value::Value;

/// What converting a column from one type to another will do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Convertibility {
    /// Every value converts and nothing is lost.
    Lossless,
    /// Values convert one by one; some may lose information or fail.
    Lossy,
    /// No value can be converted.
    Impossible,
}

/// What to expect when converting values of type `from` to type `to`.
#[must_use]
pub const fn convertibility(from: DataType, to: DataType) -> Convertibility {
    use Convertibility::{Impossible, Lossless, Lossy};
    use DataType as T;
    match (from, to) {
        (T::Text, T::Text)
        | (T::LongText, T::LongText)
        | (T::Number, T::Number)
        | (T::Money, T::Money)
        | (T::Percent, T::Percent)
        | (T::Bool, T::Bool)
        | (T::Date, T::Date)
        | (T::DateTime, T::DateTime)
        | (T::Duration, T::Duration)
        | (T::Reference, T::Reference)
        | (T::MultiReference, T::MultiReference)
        | (T::Link, T::Link)
        | (T::Principal, T::Principal)
        | (T::Attachment, T::Attachment)
        | (T::Json, T::Json) => Lossless,
        (T::LongText, T::Text) => Lossy,
        (_, T::Text | T::LongText | T::Json) => Lossless,
        (T::Text | T::LongText | T::Json, _) => Lossy,
        (T::Bool, T::Number)
        | (T::Duration, T::Number)
        | (T::Date, T::DateTime)
        | (T::Reference, T::MultiReference) => Lossless,
        (T::Number, T::Percent | T::Money | T::Bool | T::Duration)
        | (T::Percent | T::Money, T::Number)
        | (T::DateTime, T::Date)
        | (T::MultiReference, T::Reference) => Lossy,
        (
            T::Number
            | T::Money
            | T::Percent
            | T::Bool
            | T::Date
            | T::DateTime
            | T::Duration
            | T::Reference
            | T::MultiReference
            | T::Link
            | T::Principal
            | T::Attachment,
            _,
        ) => Impossible,
    }
}

/// The result of converting one value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converted {
    /// The converted value.
    pub value: Value,
    /// Whether information was lost (for example a dropped currency or time of day).
    pub lossy: bool,
}

fn kept(value: Value) -> Converted {
    Converted {
        value: value.normalized(),
        lossy: false,
    }
}

fn string_array(items: &[serde_json::Value]) -> Option<String> {
    let strings: Option<Vec<&str>> = items.iter().map(serde_json::Value::as_str).collect();
    strings.map(|s| s.join(","))
}

fn json_money(map: &serde_json::Map<String, serde_json::Value>) -> Option<Money> {
    if map.len() != 2 {
        return None;
    }
    let amount = parse_decimal(map.get("amount")?.as_str()?).ok()?;
    let currency = Currency::new(map.get("currency")?.as_str()?).ok()?;
    Money::new(amount, currency).ok()
}

fn from_json(json: &serde_json::Value, to: DataType, ctx: &ParseContext) -> Option<Value> {
    use serde_json::Value as J;
    match (json, to) {
        (J::Null, _) => Some(Value::Null),
        (J::Bool(b), DataType::Bool) => Some(Value::Bool(*b)),
        // Natural JSON stores a percent as its fraction.
        (J::String(s), DataType::Percent) => parse_decimal(s).ok().map(Value::Percent),
        (J::Number(n), DataType::Duration) => n
            .as_i64()
            .map(|ms| Value::Duration(DurationMs::from_millis(ms))),
        (J::Number(n), DataType::Number | DataType::Money | DataType::Percent) => {
            parse_input(to, &n.to_string(), ctx).ok()
        }
        (J::String(s), _) => parse_input(to, s, ctx).ok(),
        (
            J::Array(items),
            DataType::MultiReference | DataType::Link | DataType::Principal | DataType::Attachment,
        ) => {
            if items.is_empty() {
                Some(Value::Null)
            } else {
                parse_input(to, &string_array(items)?, ctx).ok()
            }
        }
        (J::Object(map), DataType::Money) => json_money(map).map(Value::Money),
        (J::Bool(_) | J::Number(_) | J::Array(_) | J::Object(_), _) => None,
    }
}

fn single_line(text: &str) -> String {
    let flattened: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    flattened.trim().to_owned()
}

/// Converts one value to another data type (see the module documentation for the rules).
///
/// # Errors
/// [`CoreError::Conversion`] if the pair of types is [`Convertibility::Impossible`] or this
/// particular value cannot be converted.
pub fn convert(value: &Value, to: DataType, ctx: &ParseContext) -> Result<Converted, CoreError> {
    // Empty values (null, blank text, empty lists) stay empty whatever the target type.
    if value.is_empty() {
        return Ok(kept(Value::Null));
    }
    let Some(from) = value.data_type() else {
        return Ok(kept(Value::Null));
    };
    let fail = |reason: &'static str| CoreError::Conversion { from, to, reason };
    if from == to {
        return Ok(kept(value.clone()));
    }
    // A percent too large to be written as percent points has no faithful text form.
    if let (Value::Percent(fraction), DataType::Text | DataType::LongText) = (value, to) {
        if fraction.checked_mul(Decimal::ONE_HUNDRED).is_none() {
            return Err(fail("number is too large"));
        }
    }
    match (value, to) {
        (Value::LongText(text), DataType::Text) => {
            let flattened = single_line(text);
            let lossy = flattened != *text;
            Ok(Converted {
                value: Value::Text(flattened).normalized(),
                lossy,
            })
        }
        // A JSON string becomes its content, so that text → JSON → text gives the text back.
        (Value::Json(serde_json::Value::String(text)), DataType::Text) => {
            let flattened = single_line(text);
            let lossy = flattened != *text;
            Ok(Converted {
                value: Value::Text(flattened).normalized(),
                lossy,
            })
        }
        (Value::Json(serde_json::Value::String(text)), DataType::LongText) => {
            Ok(kept(Value::LongText(text.clone())))
        }
        (_, DataType::Text) => Ok(kept(Value::Text(value.to_string()))),
        (_, DataType::LongText) => Ok(kept(Value::LongText(value.to_string()))),
        (_, DataType::Json) => Ok(kept(Value::Json(value.to_natural_json()))),
        (Value::Text(text) | Value::LongText(text), _) => parse_input(to, text, ctx)
            .map(kept)
            .map_err(|_| fail("the text cannot be read as the target type")),
        (Value::Json(json), _) => from_json(json, to, ctx)
            .map(kept)
            .ok_or_else(|| fail("the JSON value has no equivalent in the target type")),
        (Value::Number(points), DataType::Percent) => points_to_fraction(*points)
            .map(|f| kept(Value::Percent(f)))
            .ok_or_else(|| fail("too many decimal places")),
        (Value::Percent(fraction), DataType::Number) => fraction
            .checked_mul(Decimal::ONE_HUNDRED)
            .map(|p| kept(Value::Number(p)))
            .ok_or_else(|| fail("number is too large")),
        (Value::Number(amount), DataType::Money) => {
            let currency = ctx
                .default_currency
                .ok_or_else(|| fail("a default currency is needed"))?;
            Money::new(*amount, currency)
                .map(|m| kept(Value::Money(m)))
                .map_err(|_| fail("too many decimal places for the currency"))
        }
        (Value::Money(money), DataType::Number) => Ok(Converted {
            value: Value::Number(money.amount()),
            lossy: true,
        }),
        (Value::Bool(flag), DataType::Number) => Ok(kept(Value::Number(if *flag {
            Decimal::ONE
        } else {
            Decimal::ZERO
        }))),
        (Value::Number(number), DataType::Bool) => {
            if number.is_zero() {
                Ok(kept(Value::Bool(false)))
            } else if *number == Decimal::ONE {
                Ok(kept(Value::Bool(true)))
            } else {
                Err(fail("only 1 and 0 can become true and false"))
            }
        }
        (Value::Duration(duration), DataType::Number) => {
            Ok(kept(Value::Number(Decimal::from(duration.millis()))))
        }
        (Value::Number(number), DataType::Duration) => {
            use rust_decimal::prelude::ToPrimitive;
            if !number.fract().is_zero() {
                return Err(fail("a duration needs a whole number of milliseconds"));
            }
            number
                .to_i64()
                .map(|ms| kept(Value::Duration(DurationMs::from_millis(ms))))
                .ok_or_else(|| fail("number is too large"))
        }
        (Value::Date(date), DataType::DateTime) => Ok(kept(Value::DateTime(date.to_timestamp()))),
        (Value::DateTime(instant), DataType::Date) => Ok(Converted {
            value: Value::Date(instant.date()),
            lossy: instant.millis_of_day() != 0,
        }),
        (Value::Reference(item), DataType::MultiReference) => {
            Ok(kept(Value::MultiReference(vec![*item])))
        }
        (Value::MultiReference(items), DataType::Reference) => match items.first() {
            Some(first) => Ok(Converted {
                value: Value::Reference(*first),
                lossy: items.len() > 1,
            }),
            None => Ok(kept(Value::Null)),
        },
        (
            Value::Null
            | Value::Number(_)
            | Value::Money(_)
            | Value::Percent(_)
            | Value::Bool(_)
            | Value::Date(_)
            | Value::DateTime(_)
            | Value::Duration(_)
            | Value::Reference(_)
            | Value::MultiReference(_)
            | Value::Link(_)
            | Value::Principal(_)
            | Value::Attachment(_),
            _,
        ) => Err(fail("these types cannot be converted into each other")),
    }
}

/// Maximum number of failures listed in a [`ConversionPreview`].
pub const MAX_PREVIEW_FAILURES: usize = 20;

/// Summary of converting a column.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct ConversionPreview {
    /// Number of values examined.
    pub total: usize,
    /// Empty values (stay empty).
    pub empty: usize,
    /// Values that convert without loss.
    pub clean: usize,
    /// Values that convert but lose information.
    pub lossy: usize,
    /// Values that cannot be converted (they would become empty).
    pub failed: usize,
    /// Position and reason of the first failures (at most [`MAX_PREVIEW_FAILURES`]).
    pub first_failures: Vec<(usize, String)>,
}

/// Converts every value and counts the outcomes without changing anything.
#[must_use]
pub fn preview_conversion(values: &[Value], to: DataType, ctx: &ParseContext) -> ConversionPreview {
    let mut preview = ConversionPreview {
        total: values.len(),
        ..ConversionPreview::default()
    };
    for (index, value) in values.iter().enumerate() {
        if value.is_empty() {
            preview.empty = preview.empty.saturating_add(1);
            continue;
        }
        match convert(value, to, ctx) {
            Ok(Converted { lossy: false, .. }) => preview.clean = preview.clean.saturating_add(1),
            Ok(Converted { lossy: true, .. }) => preview.lossy = preview.lossy.saturating_add(1),
            Err(error) => {
                preview.failed = preview.failed.saturating_add(1);
                if preview.first_failures.len() < MAX_PREVIEW_FAILURES {
                    preview.first_failures.push((index, error.to_string()));
                }
            }
        }
    }
    preview
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::RefItemId;
    use crate::time::{Date, Timestamp};
    use crate::value::testkit::{dec, money, samples};

    fn ctx() -> ParseContext {
        ParseContext::default()
    }

    fn usd() -> ParseContext {
        ParseContext {
            default_currency: Some(Currency::new("USD").unwrap()),
        }
    }

    fn conv(value: &Value, to: DataType) -> Result<Converted, CoreError> {
        convert(value, to, &ctx())
    }

    fn reason(result: Result<Converted, CoreError>) -> &'static str {
        match result {
            Err(CoreError::Conversion { reason, .. }) => reason,
            other => panic!("expected a conversion error, got {other:?}"),
        }
    }

    #[test]
    fn std_matrix_agrees_with_convert_on_every_sample_pair() {
        for value in samples() {
            let Some(from) = value.data_type() else {
                continue;
            };
            for to in DataType::ALL {
                let result = convert(&value, *to, &usd());
                match convertibility(from, *to) {
                    Convertibility::Lossless => {
                        let converted =
                            result.unwrap_or_else(|e| panic!("{from} → {to} should convert: {e}"));
                        assert!(!converted.lossy, "{from} → {to} must not lose information");
                        assert_eq!(converted.value.data_type(), Some(*to), "{from} → {to}");
                    }
                    Convertibility::Lossy => {}
                    Convertibility::Impossible => {
                        assert!(
                            matches!(result, Err(CoreError::Conversion { .. })),
                            "{from} → {to} should be impossible"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn std_lossless_conversions_round_trip() {
        for value in samples() {
            let Some(from) = value.data_type() else {
                continue;
            };
            for to in DataType::ALL {
                // JSON → text keeps everything but reads back as a JSON string.
                let json_to_text = matches!(from, DataType::Json) && to.is_text();
                if *to == from
                    || json_to_text
                    || convertibility(from, *to) != Convertibility::Lossless
                {
                    continue;
                }
                let there = convert(&value, *to, &usd()).unwrap().value;
                let back = convert(&there, from, &usd())
                    .unwrap_or_else(|e| panic!("{from} → {to} → {from}: {e}"));
                assert_eq!(
                    back.value,
                    value.clone().normalized(),
                    "{from} → {to} → {from}"
                );
            }
        }
    }

    #[test]
    fn std_null_converts_to_null_everywhere() {
        for to in DataType::ALL {
            assert_eq!(
                conv(&Value::Null, *to).unwrap(),
                Converted {
                    value: Value::Null,
                    lossy: false
                }
            );
        }
    }

    #[test]
    fn std_identity_normalises() {
        let padded = Value::Number(Decimal::new(1500, 3));
        assert_eq!(
            conv(&padded, DataType::Number).unwrap().value.to_string(),
            "1.5"
        );
        for t in DataType::ALL {
            assert_eq!(convertibility(*t, *t), Convertibility::Lossless);
        }
    }

    #[test]
    fn std_long_text_to_text_flattens() {
        let result = conv(&Value::LongText("a\nb\r\nc".into()), DataType::Text).unwrap();
        assert_eq!(
            result,
            Converted {
                value: Value::Text("a b  c".into()),
                lossy: true
            }
        );
        let result = conv(&Value::LongText("plain".into()), DataType::Text).unwrap();
        assert_eq!(
            result,
            Converted {
                value: Value::Text("plain".into()),
                lossy: false
            }
        );
        let result = conv(&Value::LongText("\n".into()), DataType::Text).unwrap();
        assert_eq!(result.value, Value::Null);
    }

    #[test]
    fn std_text_to_typed_values() {
        assert_eq!(
            conv(&Value::Text("1,234.5".into()), DataType::Number)
                .unwrap()
                .value,
            Value::Number(dec("1234.5"))
        );
        assert_eq!(
            conv(&Value::Text("yes".into()), DataType::Bool)
                .unwrap()
                .value,
            Value::Bool(true)
        );
        assert_eq!(
            conv(&Value::Text("25%".into()), DataType::Percent)
                .unwrap()
                .value,
            Value::Percent(dec("0.25"))
        );
        assert_eq!(
            convert(&Value::Text("9.99".into()), DataType::Money, &usd())
                .unwrap()
                .value,
            Value::Money(money("9.99", "USD"))
        );
        assert!(
            reason(conv(&Value::Text("soon".into()), DataType::Date)).contains("cannot be read")
        );
        assert!(
            reason(conv(&Value::LongText("x".into()), DataType::Number)).contains("cannot be read")
        );
    }

    #[test]
    fn std_number_percent() {
        assert_eq!(
            conv(&Value::Number(dec("25")), DataType::Percent)
                .unwrap()
                .value,
            Value::Percent(dec("0.25"))
        );
        assert_eq!(
            conv(&Value::Percent(dec("0.125")), DataType::Number)
                .unwrap()
                .value,
            Value::Number(dec("12.5"))
        );
        let fine = Value::Number(Decimal::new(1, 27));
        assert!(reason(conv(&fine, DataType::Percent)).contains("too many decimal places"));
        let huge = Value::Percent(Decimal::MAX);
        assert!(reason(conv(&huge, DataType::Number)).contains("too large"));
        assert!(reason(conv(&huge, DataType::Text)).contains("too large"));
        assert!(reason(conv(&huge, DataType::LongText)).contains("too large"));
    }

    #[test]
    fn std_number_money() {
        assert!(
            reason(conv(&Value::Number(dec("5")), DataType::Money)).contains("default currency")
        );
        assert_eq!(
            convert(&Value::Number(dec("5")), DataType::Money, &usd())
                .unwrap()
                .value,
            Value::Money(money("5", "USD"))
        );
        assert!(reason(convert(
            &Value::Number(dec("5.555")),
            DataType::Money,
            &usd()
        ))
        .contains("too many decimal places for the currency"));
        assert_eq!(
            conv(&Value::Money(money("5.5", "USD")), DataType::Number).unwrap(),
            Converted {
                value: Value::Number(dec("5.5")),
                lossy: true
            }
        );
    }

    #[test]
    fn std_number_bool_and_duration() {
        assert_eq!(
            conv(&Value::Bool(true), DataType::Number).unwrap().value,
            Value::Number(dec("1"))
        );
        assert_eq!(
            conv(&Value::Bool(false), DataType::Number).unwrap().value,
            Value::Number(dec("0"))
        );
        assert_eq!(
            conv(&Value::Number(dec("1.0")), DataType::Bool)
                .unwrap()
                .value,
            Value::Bool(true)
        );
        assert_eq!(
            conv(&Value::Number(dec("0")), DataType::Bool)
                .unwrap()
                .value,
            Value::Bool(false)
        );
        assert!(reason(conv(&Value::Number(dec("2")), DataType::Bool)).contains("only 1 and 0"));
        assert_eq!(
            conv(&Value::Number(dec("1500")), DataType::Duration)
                .unwrap()
                .value,
            Value::Duration(DurationMs::from_millis(1_500))
        );
        assert!(
            reason(conv(&Value::Number(dec("1.5")), DataType::Duration)).contains("whole number")
        );
        assert!(
            reason(conv(&Value::Number(Decimal::MAX), DataType::Duration)).contains("too large")
        );
        assert_eq!(
            conv(
                &Value::Duration(DurationMs::from_millis(-7)),
                DataType::Number
            )
            .unwrap()
            .value,
            Value::Number(dec("-7"))
        );
    }

    #[test]
    fn std_date_datetime() {
        let day = Date::from_ymd(2024, 5, 6).unwrap();
        assert_eq!(
            conv(&Value::Date(day), DataType::DateTime).unwrap().value,
            Value::DateTime(day.to_timestamp())
        );
        assert_eq!(
            conv(&Value::DateTime(day.to_timestamp()), DataType::Date).unwrap(),
            Converted {
                value: Value::Date(day),
                lossy: false
            }
        );
        let afternoon = Timestamp::from_millis(day.to_timestamp().millis() + 50_000_000).unwrap();
        assert_eq!(
            conv(&Value::DateTime(afternoon), DataType::Date).unwrap(),
            Converted {
                value: Value::Date(day),
                lossy: true
            }
        );
    }

    #[test]
    fn std_reference_lists() {
        let id = RefItemId::from_u128;
        assert_eq!(
            conv(&Value::Reference(id(1)), DataType::MultiReference)
                .unwrap()
                .value,
            Value::MultiReference(vec![id(1)])
        );
        assert_eq!(
            conv(&Value::MultiReference(vec![id(1)]), DataType::Reference).unwrap(),
            Converted {
                value: Value::Reference(id(1)),
                lossy: false
            }
        );
        assert_eq!(
            conv(
                &Value::MultiReference(vec![id(2), id(1)]),
                DataType::Reference
            )
            .unwrap(),
            Converted {
                value: Value::Reference(id(2)),
                lossy: true
            }
        );
        assert_eq!(
            conv(&Value::MultiReference(vec![]), DataType::Reference)
                .unwrap()
                .value,
            Value::Null
        );
    }

    #[test]
    fn std_json_to_typed_values() {
        use serde_json::json;
        let j = |v: serde_json::Value| Value::Json(v);
        assert_eq!(
            conv(&j(json!(true)), DataType::Bool).unwrap().value,
            Value::Bool(true)
        );
        assert_eq!(
            conv(&j(json!("0.25")), DataType::Percent).unwrap().value,
            Value::Percent(dec("0.25"))
        );
        assert_eq!(
            conv(&j(json!(1500)), DataType::Duration).unwrap().value,
            Value::Duration(DurationMs::from_millis(1_500))
        );
        assert_eq!(
            conv(&j(json!(12)), DataType::Number).unwrap().value,
            Value::Number(dec("12"))
        );
        assert_eq!(
            conv(&j(json!("12.5")), DataType::Number).unwrap().value,
            Value::Number(dec("12.5"))
        );
        assert_eq!(
            conv(&j(json!("hello")), DataType::Text).unwrap().value,
            Value::Text("hello".into())
        );
        assert_eq!(
            conv(&j(json!("a\nb")), DataType::LongText).unwrap().value,
            Value::LongText("a\nb".into())
        );
        assert_eq!(
            conv(&j(json!("a\nb")), DataType::Text).unwrap(),
            Converted {
                value: Value::Text("a b".into()),
                lossy: true
            }
        );
        assert_eq!(
            conv(&j(json!([1])), DataType::Text).unwrap().value,
            Value::Text("[1]".into())
        );
        assert_eq!(
            conv(
                &j(json!({"amount": "3.5", "currency": "EUR"})),
                DataType::Money
            )
            .unwrap()
            .value,
            Value::Money(money("3.5", "EUR"))
        );
        let a = RefItemId::from_u128(1);
        assert_eq!(
            conv(&j(json!([a.to_string()])), DataType::MultiReference)
                .unwrap()
                .value,
            Value::MultiReference(vec![a])
        );
        assert_eq!(
            conv(&j(json!([])), DataType::Link).unwrap().value,
            Value::Null
        );
        for (json, to) in [
            (json!({"amount": "3.5"}), DataType::Money),
            (json!({"amount": 3.5, "currency": "EUR"}), DataType::Money),
            (
                json!({"amount": "3.5", "currency": "EUR", "x": 1}),
                DataType::Money,
            ),
            (
                json!({"amount": "3.555", "currency": "EUR"}),
                DataType::Money,
            ),
            (json!([1, 2]), DataType::MultiReference),
            (json!({"a": 1}), DataType::Number),
            (json!(1.5), DataType::Duration),
            (json!(true), DataType::Number),
            (json!([1]), DataType::Number),
            (json!(5), DataType::Date),
        ] {
            assert!(reason(conv(&j(json), to)).contains("no equivalent"), "{to}");
        }
    }

    #[test]
    fn invalid_impossible_pairs() {
        let day = Value::Date(Date::EPOCH);
        assert!(reason(conv(&day, DataType::Number)).contains("cannot be converted"));
        assert_eq!(
            convertibility(DataType::Date, DataType::Number),
            Convertibility::Impossible
        );
        assert_eq!(
            convertibility(DataType::Link, DataType::Principal),
            Convertibility::Impossible
        );
        assert_eq!(
            convertibility(DataType::Attachment, DataType::Link),
            Convertibility::Impossible
        );
        assert_eq!(
            convertibility(DataType::Money, DataType::Percent),
            Convertibility::Impossible
        );
        assert_eq!(
            convertibility(DataType::LongText, DataType::Text),
            Convertibility::Lossy
        );
        assert_eq!(
            convertibility(DataType::Text, DataType::LongText),
            Convertibility::Lossless
        );
        assert_eq!(
            convertibility(DataType::Json, DataType::Date),
            Convertibility::Lossy
        );
    }

    #[test]
    fn std_preview_counts_every_outcome() {
        let values = vec![
            Value::Text("12".into()),
            Value::Null,
            Value::Text("  ".into()),
            Value::Text("abc".into()),
            Value::Text("7.5".into()),
            Value::Text("1,5".into()),
        ];
        let preview = preview_conversion(&values, DataType::Number, &ctx());
        assert_eq!(
            (
                preview.total,
                preview.empty,
                preview.clean,
                preview.lossy,
                preview.failed
            ),
            (6, 2, 2, 0, 2)
        );
        assert_eq!(
            preview
                .first_failures
                .iter()
                .map(|(i, _)| *i)
                .collect::<Vec<_>>(),
            vec![3, 5]
        );
        assert!(preview.first_failures[0]
            .1
            .contains("cannot convert text to number"));

        let instants = vec![
            Value::DateTime(Timestamp::EPOCH),
            Value::DateTime(Timestamp::from_millis(1).unwrap()),
        ];
        let preview = preview_conversion(&instants, DataType::Date, &ctx());
        assert_eq!((preview.clean, preview.lossy, preview.failed), (1, 1, 0));
        let json = serde_json::to_string(&preview).unwrap();
        assert_eq!(
            serde_json::from_str::<ConversionPreview>(&json).unwrap(),
            preview
        );
    }

    #[test]
    fn edge_preview_lists_only_the_first_failures() {
        let values = vec![Value::Text("x".into()); MAX_PREVIEW_FAILURES + 5];
        let preview = preview_conversion(&values, DataType::Bool, &ctx());
        assert_eq!(preview.failed, MAX_PREVIEW_FAILURES + 5);
        assert_eq!(preview.first_failures.len(), MAX_PREVIEW_FAILURES);
        assert_eq!(
            preview_conversion(&[], DataType::Bool, &ctx()),
            ConversionPreview::default()
        );
    }
}

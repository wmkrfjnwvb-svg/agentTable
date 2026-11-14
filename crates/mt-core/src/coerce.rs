//! Reading user input.
//!
//! [`parse_input`] turns text typed by a person (or pasted, or imported from a file) into a
//! [`Value`] of a given [`DataType`]. The rules are **locale-neutral** and never guess:
//!
//! * empty or white-space-only input is [`Value::Null`];
//! * **number**: digits with `.` as the decimal point; an optional sign (`+`, `-`, `−`);
//!   grouping by spaces, or by commas when every group has three digits (Indian grouping such
//!   as `12,34,567` is accepted too). A lone comma as in `1,5` is rejected, because it could
//!   be a decimal comma. Locale-specific input (for example German `1.234,56`) is converted
//!   by the client before it reaches this function;
//! * **percent**: percent points with an optional `%` (`25` and `25%` are both 25 %);
//! * **money**: an amount and an ISO 4217 code in either order (`12.50 USD`, `USD 12.50`);
//!   the code may be omitted when the context supplies a default currency;
//! * **bool**: `true/false`, `yes/no`, `y/n`, `on/off`, `1/0` (case-insensitive);
//! * **date**: `YYYY-MM-DD`, also `YYYY/MM/DD` and `YYYY.MM.DD` (year first, so no
//!   day/month ambiguity);
//! * **datetime**: see [`Timestamp::parse_lenient`]; **duration**: see `DurationMs::from_str`;
//! * **IDs**: the canonical text; lists are separated by commas;
//! * **JSON**: any JSON text;
//! * **text**: surrounding white space is trimmed and line breaks are rejected;
//!   **long text** is kept as typed.
//!
//! For every type except free text, `parse_input(type, value.to_string())` gives the value back.

use crate::decimal::{parse_decimal, Decimal};
use crate::error::{clip, CoreError};
use crate::field::DataType;
use crate::id::{AttachmentId, PrincipalId, RecordId, RefItemId};
use crate::money::{Currency, Money};
use crate::time::{Date, DurationMs, Timestamp};
use crate::value::Value;
use core::str::FromStr;

/// Extra information for parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ParseContext {
    /// Currency assumed when money input carries no currency code.
    pub default_currency: Option<Currency>,
}

fn is_group_space(c: char) -> bool {
    matches!(c, ' ' | '\u{00A0}' | '\u{202F}' | '\u{2009}')
}

/// Checks comma grouping of an integer part: first group 1–3 digits, last group exactly 3,
/// groups in between 2 or 3 (Indian system).
fn commas_are_grouping(integer_part: &str) -> bool {
    let groups: Vec<&str> = integer_part.split(',').collect();
    let last_index = groups.len().saturating_sub(1);
    groups.iter().enumerate().all(|(index, group)| {
        let digits = group.bytes().all(|b| b.is_ascii_digit());
        let len_ok = if index == 0 {
            (1..=3).contains(&group.len())
        } else if index == last_index {
            group.len() == 3
        } else {
            (2..=3).contains(&group.len())
        };
        digits && len_ok
    })
}

/// Parses a forgiving, locale-neutral decimal (see the module documentation).
fn parse_number(input: &str) -> Result<Decimal, &'static str> {
    let trimmed = input.trim();
    let (negative, unsigned) = if let Some(rest) = trimmed
        .strip_prefix('-')
        .or_else(|| trimmed.strip_prefix('\u{2212}'))
    {
        (true, rest)
    } else {
        (false, trimmed.strip_prefix('+').unwrap_or(trimmed))
    };
    if unsigned.starts_with(['-', '+', '\u{2212}']) {
        return Err("more than one sign");
    }
    let (integer_raw, fraction) = match unsigned.split_once('.') {
        Some((integer_raw, fraction)) => (integer_raw, Some(fraction)),
        None => (unsigned, None),
    };
    let has_comma = integer_raw.contains(',');
    let has_space = integer_raw.chars().any(is_group_space);
    if has_comma && has_space {
        return Err("mixed grouping separators");
    }
    if has_comma && !commas_are_grouping(integer_raw) {
        return Err(
            "a comma is only accepted as a thousands separator; use `.` as the decimal point",
        );
    }
    if has_space {
        let pieces_ok = integer_raw
            .split(is_group_space)
            .all(|piece| !piece.is_empty() && piece.bytes().all(|b| b.is_ascii_digit()));
        if !pieces_ok {
            return Err("spaces are only accepted between digit groups");
        }
    }
    let mut plain = String::with_capacity(unsigned.len().saturating_add(1));
    if negative {
        plain.push('-');
    }
    plain.extend(
        integer_raw
            .chars()
            .filter(|c| *c != ',' && !is_group_space(*c)),
    );
    if let Some(fraction) = fraction {
        plain.push('.');
        plain.push_str(fraction);
    }
    parse_decimal(&plain).map_err(|error| {
        if let CoreError::InvalidDecimal { reason, .. } = error {
            reason
        } else {
            "not a number"
        }
    })
}

/// Converts percent points to a fraction (`25` → `0.25`) exactly; `None` if the result would
/// need more than 28 decimal places.
pub(crate) fn points_to_fraction(points: Decimal) -> Option<Decimal> {
    let points = points.normalize();
    // Multiplying by 0.01 adds two decimal places; beyond 28 the decimal type would round.
    if points.scale() > 26 {
        return None;
    }
    points
        .checked_mul(Decimal::new(1, 2))
        .map(|d| d.normalize())
}

fn parse_percent(input: &str) -> Result<Decimal, &'static str> {
    let trimmed = input.trim();
    let points_text = trimmed.strip_suffix('%').map_or(trimmed, str::trim_end);
    let points = parse_number(points_text)?;
    points_to_fraction(points).ok_or("too many decimal places for a percentage")
}

fn currency_token(token: &str) -> Option<Currency> {
    (token.len() == 3 && token.bytes().all(|b| b.is_ascii_alphabetic()))
        .then(|| Currency::new(&token.to_ascii_uppercase()).ok())
        .flatten()
}

fn parse_money(input: &str, ctx: &ParseContext) -> Result<Money, &'static str> {
    let trimmed = input.trim();
    let head = trimmed.get(..3).filter(|_| trimmed.is_char_boundary(3));
    let tail = trimmed
        .len()
        .checked_sub(3)
        .and_then(|start| trimmed.get(start..));
    let (amount_text, currency) = if let Some(currency) = head.and_then(currency_token) {
        (trimmed.get(3..).unwrap_or(""), currency)
    } else if let Some(currency) = tail.and_then(currency_token) {
        (
            trimmed.get(..trimmed.len().saturating_sub(3)).unwrap_or(""),
            currency,
        )
    } else if trimmed.chars().any(|c| c.is_ascii_alphabetic()) {
        return Err("unknown currency code");
    } else {
        (
            trimmed,
            ctx.default_currency
                .ok_or("a currency code is required (for example `12.50 USD`)")?,
        )
    };
    let amount = parse_number(amount_text)?;
    Money::new(amount, currency).map_err(|_| "too many decimal places for the currency")
}

fn parse_bool(input: &str) -> Result<bool, &'static str> {
    match input.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "y" | "on" | "1" => Ok(true),
        "false" | "no" | "n" | "off" | "0" => Ok(false),
        _ => Err("expected true/false, yes/no, on/off or 1/0"),
    }
}

fn parse_date(input: &str) -> Result<Date, &'static str> {
    let trimmed = input.trim();
    let bytes = trimmed.as_bytes();
    let separator = bytes.get(4).copied();
    let normalised = if trimmed.len() == 10
        && bytes.get(7).copied() == separator
        && matches!(separator, Some(b'/' | b'.'))
    {
        trimmed.replace(['/', '.'], "-")
    } else {
        trimmed.to_owned()
    };
    Date::from_str(&normalised).map_err(|_| "expected a date as YYYY-MM-DD")
}

fn parse_id_list<T: FromStr>(input: &str) -> Result<Vec<T>, &'static str> {
    input
        .split(',')
        .map(|part| {
            part.trim()
                .parse::<T>()
                .map_err(|_| "expected a comma-separated list of IDs")
        })
        .collect()
}

/// Reads user input as a value of `data_type`. See the module documentation for the accepted
/// notations. The result is normalised ([`Value::normalized`]).
///
/// # Errors
/// [`CoreError::Parse`] with the reason the input was not understood.
pub fn parse_input(
    data_type: DataType,
    input: &str,
    ctx: &ParseContext,
) -> Result<Value, CoreError> {
    let err = |reason: &'static str| CoreError::Parse {
        data_type,
        input: clip(input),
        reason,
    };
    if input.trim().is_empty() {
        return Ok(Value::Null);
    }
    let value = match data_type {
        DataType::Text => {
            let trimmed = input.trim();
            if trimmed.chars().any(char::is_control) {
                return Err(err(
                    "single-line text cannot contain line breaks or control characters",
                ));
            }
            Value::Text(trimmed.to_owned())
        }
        DataType::LongText => Value::LongText(input.to_owned()),
        DataType::Number => Value::Number(parse_number(input).map_err(err)?),
        DataType::Percent => Value::Percent(parse_percent(input).map_err(err)?),
        DataType::Money => Value::Money(parse_money(input, ctx).map_err(err)?),
        DataType::Bool => Value::Bool(parse_bool(input).map_err(err)?),
        DataType::Date => Value::Date(parse_date(input).map_err(err)?),
        DataType::DateTime => Value::DateTime(
            Timestamp::parse_lenient(input)
                .map_err(|_| err("expected a date and time such as 2024-05-06 07:08"))?,
        ),
        DataType::Duration => Value::Duration(
            DurationMs::from_str(input)
                .map_err(|_| err("expected a duration such as 1h30m, 1:30 or PT1H30M"))?,
        ),
        DataType::Reference => Value::Reference(
            input
                .trim()
                .parse::<RefItemId>()
                .map_err(|_| err("expected a reference item ID"))?,
        ),
        DataType::MultiReference => Value::MultiReference(parse_id_list(input).map_err(err)?),
        DataType::Link => Value::Link(parse_id_list::<RecordId>(input).map_err(err)?),
        DataType::Principal => Value::Principal(parse_id_list::<PrincipalId>(input).map_err(err)?),
        DataType::Attachment => {
            Value::Attachment(parse_id_list::<AttachmentId>(input).map_err(err)?)
        }
        DataType::Json => {
            Value::Json(serde_json::from_str(input).map_err(|_| err("not valid JSON"))?)
        }
    };
    Ok(value.normalized())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::testkit::{dec, money, samples};

    fn ctx() -> ParseContext {
        ParseContext::default()
    }

    fn parse(data_type: DataType, input: &str) -> Result<Value, CoreError> {
        parse_input(data_type, input, &ctx())
    }

    fn reason(result: Result<Value, CoreError>) -> &'static str {
        match result {
            Err(CoreError::Parse { reason, .. }) => reason,
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn std_empty_input_is_null_for_every_type() {
        for data_type in DataType::ALL {
            for input in ["", "   ", "\t\n"] {
                assert_eq!(
                    parse(*data_type, input).unwrap(),
                    Value::Null,
                    "{data_type}"
                );
            }
        }
    }

    #[test]
    fn std_display_round_trips_for_every_non_text_sample() {
        for value in samples() {
            let Some(data_type) = value.data_type() else {
                continue;
            };
            let parsed = parse(data_type, &value.to_string()).unwrap();
            assert_eq!(parsed, value.clone().normalized(), "{data_type}");
        }
    }

    #[test]
    fn std_numbers() {
        let cases = [
            ("42", "42"),
            (" 42 ", "42"),
            ("+42", "42"),
            ("-42.50", "-42.5"),
            ("\u{2212}3", "-3"),
            ("1,234", "1234"),
            ("1,234,567.89", "1234567.89"),
            ("12,34,567", "1234567"),
            ("1 234 567.5", "1234567.5"),
            ("1\u{00A0}234", "1234"),
            ("1\u{202F}234\u{2009}567", "1234567"),
            ("007", "7"),
            ("0.000", "0"),
            ("-0", "0"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                parse(DataType::Number, input).unwrap(),
                Value::Number(dec(expected)),
                "{input}"
            );
        }
    }

    #[test]
    fn invalid_numbers() {
        assert!(reason(parse(DataType::Number, "1,5")).contains("thousands separator"));
        assert!(
            reason(parse(DataType::Number, "1.234,56")).contains("digits after the decimal point")
        );
        assert!(reason(parse(DataType::Number, "1,23,4")).contains("thousands separator"));
        assert!(reason(parse(DataType::Number, "1,2345")).contains("thousands separator"));
        assert!(reason(parse(DataType::Number, ",123")).contains("thousands separator"));
        assert!(reason(parse(DataType::Number, "1 234,567")).contains("mixed grouping"));
        assert!(reason(parse(DataType::Number, "1  234")).contains("between digit groups"));
        assert!(reason(parse(DataType::Number, "--1")).contains("more than one sign"));
        assert!(reason(parse(DataType::Number, "+-1")).contains("more than one sign"));
        for input in [
            "abc", "1e3", "1.2.3", ".5", "5.", "0x10", "١٢٣", "1_000", "NaN", "Infinity",
        ] {
            assert!(parse(DataType::Number, input).is_err(), "{input}");
        }
        assert!(reason(parse(DataType::Number, &"9".repeat(40))).contains("too large"));
    }

    #[test]
    fn std_percent_is_percent_points() {
        assert_eq!(
            parse(DataType::Percent, "25%").unwrap(),
            Value::Percent(dec("0.25"))
        );
        assert_eq!(
            parse(DataType::Percent, "25").unwrap(),
            Value::Percent(dec("0.25"))
        );
        assert_eq!(
            parse(DataType::Percent, "12.5 %").unwrap(),
            Value::Percent(dec("0.125"))
        );
        assert_eq!(
            parse(DataType::Percent, "-150%").unwrap(),
            Value::Percent(dec("-1.5"))
        );
        assert_eq!(
            parse(DataType::Percent, "0").unwrap(),
            Value::Percent(dec("0"))
        );
    }

    #[test]
    fn invalid_percent() {
        assert!(parse(DataType::Percent, "%").is_err());
        assert!(parse(DataType::Percent, "25%%").is_err());
        assert!(parse(DataType::Percent, "%25").is_err());
        let too_fine = format!("0.{}1", "0".repeat(26));
        assert!(reason(parse(DataType::Percent, &too_fine)).contains("too many decimal places"));
    }

    #[test]
    fn std_money() {
        assert_eq!(
            parse(DataType::Money, "12.50 USD").unwrap(),
            Value::Money(money("12.5", "USD"))
        );
        assert_eq!(
            parse(DataType::Money, "USD 12.50").unwrap(),
            Value::Money(money("12.5", "USD"))
        );
        assert_eq!(
            parse(DataType::Money, "usd12.5").unwrap(),
            Value::Money(money("12.5", "USD"))
        );
        assert_eq!(
            parse(DataType::Money, "1,000 JPY").unwrap(),
            Value::Money(money("1000", "JPY"))
        );
        assert_eq!(
            parse(DataType::Money, "-3.999 BHD").unwrap(),
            Value::Money(money("-3.999", "BHD"))
        );
        let with_default = ParseContext {
            default_currency: Some(Currency::new("EUR").unwrap()),
        };
        assert_eq!(
            parse_input(DataType::Money, "7.5", &with_default).unwrap(),
            Value::Money(money("7.5", "EUR"))
        );
        assert_eq!(
            parse_input(DataType::Money, "7.5 USD", &with_default).unwrap(),
            Value::Money(money("7.5", "USD"))
        );
    }

    #[test]
    fn invalid_money() {
        assert!(reason(parse(DataType::Money, "12.50")).contains("currency code is required"));
        assert!(reason(parse(DataType::Money, "12.50 XXY")).contains("unknown currency"));
        assert!(parse(DataType::Money, "12.50 dollars").is_err());
        assert!(reason(parse(DataType::Money, "10.5 JPY"))
            .contains("too many decimal places for the currency"));
        assert!(reason(parse(DataType::Money, "12.505 USD"))
            .contains("too many decimal places for the currency"));
        assert!(parse(DataType::Money, "USD").is_err());
        assert!(parse(DataType::Money, "é1 USD").is_err());
        assert!(parse(DataType::Money, "12 €").is_err());
    }

    #[test]
    fn std_bool() {
        for input in ["true", "TRUE", "Yes", "y", "on", "1", " true "] {
            assert_eq!(
                parse(DataType::Bool, input).unwrap(),
                Value::Bool(true),
                "{input}"
            );
        }
        for input in ["false", "No", "N", "off", "0"] {
            assert_eq!(
                parse(DataType::Bool, input).unwrap(),
                Value::Bool(false),
                "{input}"
            );
        }
        assert!(reason(parse(DataType::Bool, "maybe")).contains("expected true/false"));
        assert!(parse(DataType::Bool, "2").is_err());
    }

    #[test]
    fn std_dates() {
        let expected = Value::Date(Date::from_ymd(2024, 2, 29).unwrap());
        for input in ["2024-02-29", "2024/02/29", "2024.02.29", " 2024-02-29 "] {
            assert_eq!(parse(DataType::Date, input).unwrap(), expected, "{input}");
        }
        for input in [
            "29/02/2024",
            "02/29/2024",
            "2023-02-29",
            "2024-2-29",
            "2024/02.29",
            "20240229",
            "2024-02-29T00:00",
        ] {
            assert!(
                reason(parse(DataType::Date, input)).contains("YYYY-MM-DD"),
                "{input}"
            );
        }
    }

    #[test]
    fn std_datetime_and_duration() {
        assert_eq!(
            parse(DataType::DateTime, "2024-05-06 07:08").unwrap(),
            Value::DateTime(Timestamp::from_millis(1_714_979_280_000).unwrap())
        );
        assert!(reason(parse(DataType::DateTime, "yesterday")).contains("date and time"));
        assert_eq!(
            parse(DataType::Duration, "1:30").unwrap(),
            Value::Duration(DurationMs::from_millis(5_400_000))
        );
        assert_eq!(
            parse(DataType::Duration, "90min").unwrap(),
            Value::Duration(DurationMs::from_millis(5_400_000))
        );
        assert!(reason(parse(DataType::Duration, "90")).contains("duration"));
    }

    #[test]
    fn std_ids_and_lists() {
        let a = RefItemId::from_u128(1);
        let b = RefItemId::from_u128(2);
        assert_eq!(
            parse(DataType::Reference, &format!(" {a} ")).unwrap(),
            Value::Reference(a)
        );
        assert_eq!(
            parse(DataType::MultiReference, &format!("{a},{b}")).unwrap(),
            Value::MultiReference(vec![a, b])
        );
        assert_eq!(
            parse(DataType::MultiReference, &format!("{a} , {a}")).unwrap(),
            Value::MultiReference(vec![a, a])
        );
        let record = RecordId::from_u128(3);
        assert_eq!(
            parse(DataType::Link, &record.to_string()).unwrap(),
            Value::Link(vec![record])
        );
        assert!(
            reason(parse(DataType::Reference, &record.to_string())).contains("reference item ID")
        );
        assert!(
            reason(parse(DataType::MultiReference, &format!("{a},,{b}")))
                .contains("comma-separated")
        );
        assert!(reason(parse(DataType::Link, &a.to_string())).contains("comma-separated"));
        assert!(parse(DataType::Principal, "me").is_err());
        assert!(parse(DataType::Attachment, "file.pdf").is_err());
    }

    #[test]
    fn std_json_and_text() {
        assert_eq!(
            parse(DataType::Json, r#"{"a":[1,2]}"#).unwrap(),
            Value::Json(serde_json::json!({"a": [1, 2]}))
        );
        assert_eq!(parse(DataType::Json, "null").unwrap(), Value::Null);
        assert!(reason(parse(DataType::Json, "{a:1}")).contains("not valid JSON"));
        assert_eq!(
            parse(DataType::Text, "  Hello world ").unwrap(),
            Value::Text("Hello world".into())
        );
        assert!(reason(parse(DataType::Text, "a\nb")).contains("single-line"));
        assert_eq!(
            parse(DataType::LongText, " a\nb ").unwrap(),
            Value::LongText(" a\nb ".into())
        );
    }

    #[test]
    fn edge_error_echo_is_clipped() {
        let long = "x".repeat(10_000);
        match parse(DataType::Number, &long) {
            Err(CoreError::Parse {
                input, data_type, ..
            }) => {
                assert!(input.chars().count() < 100);
                assert_eq!(data_type, DataType::Number);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

//! The cell value model.
//!
//! A [`Value`] is what one cell of a record holds. There is one variant per storage
//! [`DataType`], plus [`Value::Null`] for "no value".
//!
//! # Canonical serde form
//!
//! Values serialise as `{"type": "<data type>", "value": <payload>}`; null is
//! `{"type":"null"}`. Payloads:
//!
//! | type | payload |
//! |---|---|
//! | `text`, `long_text` | string |
//! | `number`, `percent` | canonical decimal **string** (`"12.5"`, see [`crate::decimal`]) |
//! | `money` | `{"amount": "<decimal string>", "currency": "USD"}` |
//! | `bool` | boolean |
//! | `date` | `"YYYY-MM-DD"` |
//! | `datetime` | integer milliseconds since the Unix epoch (UTC) |
//! | `duration` | integer milliseconds |
//! | `reference` | reference item ID string |
//! | `multi_reference`, `link`, `principal`, `attachment` | array of ID strings |
//! | `json` | any JSON |
//!
//! Decimals never appear as JSON numbers, so no float can corrupt them.
//!
//! # Percent
//!
//! A percent is stored as a **fraction**: 25 % is `Percent(0.25)`.

use crate::decimal::{canonical_string, Decimal};
use crate::field::DataType;
use crate::id::{AttachmentId, PrincipalId, RecordId, RefItemId};
use crate::money::Money;
use crate::time::{Date, DurationMs, Timestamp};
use core::fmt;

/// The content of one cell.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Value {
    /// No value.
    Null,
    /// Single-line text.
    Text(String),
    /// Multi-line text.
    LongText(String),
    /// An exact decimal number.
    Number(#[serde(with = "crate::decimal::serde_canonical")] Decimal),
    /// An exact amount with a currency.
    Money(Money),
    /// A percentage stored as a fraction (`0.25` is 25 %).
    Percent(#[serde(with = "crate::decimal::serde_canonical")] Decimal),
    /// True or false.
    Bool(bool),
    /// A calendar day.
    Date(Date),
    /// An instant in UTC.
    #[serde(rename = "datetime")]
    DateTime(Timestamp),
    /// A length of time.
    Duration(DurationMs),
    /// One item of a reference set (or of an inline option list).
    Reference(RefItemId),
    /// Several reference items, in the order chosen.
    MultiReference(Vec<RefItemId>),
    /// Links to records of another table, in order.
    Link(Vec<RecordId>),
    /// One or more principals (people, groups, agents).
    Principal(Vec<PrincipalId>),
    /// References to stored attachments, in order.
    Attachment(Vec<AttachmentId>),
    /// Free-form JSON.
    Json(serde_json::Value),
}

impl Value {
    /// The storage data type of this value; `None` for [`Value::Null`].
    #[must_use]
    pub const fn data_type(&self) -> Option<DataType> {
        match self {
            Value::Null => None,
            Value::Text(_) => Some(DataType::Text),
            Value::LongText(_) => Some(DataType::LongText),
            Value::Number(_) => Some(DataType::Number),
            Value::Money(_) => Some(DataType::Money),
            Value::Percent(_) => Some(DataType::Percent),
            Value::Bool(_) => Some(DataType::Bool),
            Value::Date(_) => Some(DataType::Date),
            Value::DateTime(_) => Some(DataType::DateTime),
            Value::Duration(_) => Some(DataType::Duration),
            Value::Reference(_) => Some(DataType::Reference),
            Value::MultiReference(_) => Some(DataType::MultiReference),
            Value::Link(_) => Some(DataType::Link),
            Value::Principal(_) => Some(DataType::Principal),
            Value::Attachment(_) => Some(DataType::Attachment),
            Value::Json(_) => Some(DataType::Json),
        }
    }

    /// The name used in the `type` tag of the serde form (`"null"` for null).
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self.data_type() {
            Some(data_type) => data_type.as_str(),
            None => "null",
        }
    }

    /// Whether this is [`Value::Null`].
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Whether the value counts as "empty" for a required-field check: null, empty or
    /// whitespace-only text, an empty list, or JSON `null`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Value::Null => true,
            Value::Text(s) | Value::LongText(s) => s.trim().is_empty(),
            Value::MultiReference(items) => items.is_empty(),
            Value::Link(items) => items.is_empty(),
            Value::Principal(items) => items.is_empty(),
            Value::Attachment(items) => items.is_empty(),
            Value::Json(json) => json.is_null(),
            Value::Number(_)
            | Value::Money(_)
            | Value::Percent(_)
            | Value::Bool(_)
            | Value::Date(_)
            | Value::DateTime(_)
            | Value::Duration(_)
            | Value::Reference(_) => false,
        }
    }

    /// The stored form of a value: empty values (see [`Value::is_empty`]) become
    /// [`Value::Null`] and decimals lose trailing zeros. Text is otherwise kept as is.
    #[must_use]
    pub fn normalized(self) -> Value {
        if self.is_empty() {
            return Value::Null;
        }
        match self {
            Value::Number(d) => Value::Number(d.normalize()),
            Value::Percent(d) => Value::Percent(d.normalize()),
            Value::Null
            | Value::Text(_)
            | Value::LongText(_)
            | Value::Money(_)
            | Value::Bool(_)
            | Value::Date(_)
            | Value::DateTime(_)
            | Value::Duration(_)
            | Value::Reference(_)
            | Value::MultiReference(_)
            | Value::Link(_)
            | Value::Principal(_)
            | Value::Attachment(_)
            | Value::Json(_) => self,
        }
    }

    /// The text of a `Text` or `LongText` value.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(s) | Value::LongText(s) => Some(s),
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
            | Value::Attachment(_)
            | Value::Json(_) => None,
        }
    }

    /// The decimal of a `Number` or `Percent` value, or the amount of a `Money` value.
    #[must_use]
    pub fn as_decimal(&self) -> Option<Decimal> {
        match self {
            Value::Number(d) | Value::Percent(d) => Some(*d),
            Value::Money(m) => Some(m.amount()),
            Value::Null
            | Value::Text(_)
            | Value::LongText(_)
            | Value::Bool(_)
            | Value::Date(_)
            | Value::DateTime(_)
            | Value::Duration(_)
            | Value::Reference(_)
            | Value::MultiReference(_)
            | Value::Link(_)
            | Value::Principal(_)
            | Value::Attachment(_)
            | Value::Json(_) => None,
        }
    }

    /// Number of items of a list value (`MultiReference`, `Link`, `Principal`, `Attachment`).
    #[must_use]
    pub fn item_count(&self) -> Option<usize> {
        match self {
            Value::MultiReference(items) => Some(items.len()),
            Value::Link(items) => Some(items.len()),
            Value::Principal(items) => Some(items.len()),
            Value::Attachment(items) => Some(items.len()),
            Value::Null
            | Value::Text(_)
            | Value::LongText(_)
            | Value::Number(_)
            | Value::Money(_)
            | Value::Percent(_)
            | Value::Bool(_)
            | Value::Date(_)
            | Value::DateTime(_)
            | Value::Duration(_)
            | Value::Reference(_)
            | Value::Json(_) => None,
        }
    }

    /// A "natural" JSON rendering used when a value is converted to a JSON field: text and
    /// IDs as strings, decimals as canonical strings, money as an object, dates as
    /// `YYYY-MM-DD`, instants as RFC 3339 strings, durations as milliseconds.
    #[must_use]
    pub fn to_natural_json(&self) -> serde_json::Value {
        use serde_json::Value as J;
        fn ids<T: fmt::Display>(items: &[T]) -> J {
            J::Array(items.iter().map(|i| J::String(i.to_string())).collect())
        }
        match self {
            Value::Null => J::Null,
            Value::Text(s) | Value::LongText(s) => J::String(s.clone()),
            Value::Number(d) | Value::Percent(d) => J::String(canonical_string(d)),
            Value::Money(m) => {
                let mut map = serde_json::Map::new();
                map.insert(
                    "amount".to_owned(),
                    J::String(canonical_string(&m.amount())),
                );
                map.insert(
                    "currency".to_owned(),
                    J::String(m.currency().code().to_owned()),
                );
                J::Object(map)
            }
            Value::Bool(b) => J::Bool(*b),
            Value::Date(d) => J::String(d.to_string()),
            Value::DateTime(t) => J::String(t.to_string()),
            Value::Duration(d) => J::Number(d.millis().into()),
            Value::Reference(id) => J::String(id.to_string()),
            Value::MultiReference(items) => ids(items),
            Value::Link(items) => ids(items),
            Value::Principal(items) => ids(items),
            Value::Attachment(items) => ids(items),
            Value::Json(json) => json.clone(),
        }
    }
}

/// Writes a percent fraction as percent points with a `%` sign (`0.25` → `25%`).
pub(crate) fn percent_string(fraction: &Decimal) -> String {
    match fraction.checked_mul(Decimal::ONE_HUNDRED) {
        Some(points) => format!("{}%", canonical_string(&points)),
        // Only for fractions near the 96-bit limit: write it as a plain factor instead.
        None => canonical_string(fraction),
    }
}

fn write_joined<T: fmt::Display>(f: &mut fmt::Formatter<'_>, items: &[T]) -> fmt::Result {
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            f.write_str(", ")?;
        }
        write!(f, "{item}")?;
    }
    Ok(())
}

/// Plain, locale-neutral text of a value. For every type except free text this is also valid
/// input for [`crate::coerce::parse_input`], which gives the same value back.
///
/// Null is the empty string; numbers are canonical decimals; percent is `25%`; money is
/// `12.50 USD`; lists are joined with `, `; JSON is compact.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => Ok(()),
            Value::Text(s) | Value::LongText(s) => f.write_str(s),
            Value::Number(d) => f.write_str(&canonical_string(d)),
            Value::Money(m) => write!(f, "{m}"),
            Value::Percent(d) => f.write_str(&percent_string(d)),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Date(d) => write!(f, "{d}"),
            Value::DateTime(t) => write!(f, "{t}"),
            Value::Duration(d) => write!(f, "{d}"),
            Value::Reference(id) => write!(f, "{id}"),
            Value::MultiReference(items) => write_joined(f, items),
            Value::Link(items) => write_joined(f, items),
            Value::Principal(items) => write_joined(f, items),
            Value::Attachment(items) => write_joined(f, items),
            Value::Json(json) => write!(f, "{json}"),
        }
    }
}

#[cfg(test)]
pub(crate) mod testkit {
    //! One sample value per variant, shared by unit tests across modules.
    use super::*;
    use crate::decimal::parse_decimal;
    use crate::money::Currency;

    pub(crate) fn dec(s: &str) -> Decimal {
        parse_decimal(s).unwrap()
    }

    pub(crate) fn money(amount: &str, code: &str) -> Money {
        Money::new(dec(amount), Currency::new(code).unwrap()).unwrap()
    }

    pub(crate) fn samples() -> Vec<Value> {
        vec![
            Value::Null,
            Value::Text("Hello".into()),
            Value::LongText("Line 1\nLine 2".into()),
            Value::Number(dec("12.5")),
            Value::Money(money("12.5", "USD")),
            Value::Percent(dec("0.25")),
            Value::Bool(true),
            Value::Date(Date::from_ymd(2024, 5, 6).unwrap()),
            Value::DateTime(Timestamp::from_millis(1_714_979_289_123).unwrap()),
            Value::Duration(DurationMs::from_millis(5_400_000)),
            Value::Reference(RefItemId::from_u128(1)),
            Value::MultiReference(vec![RefItemId::from_u128(1), RefItemId::from_u128(2)]),
            Value::Link(vec![RecordId::from_u128(3)]),
            Value::Principal(vec![PrincipalId::from_u128(4)]),
            Value::Attachment(vec![AttachmentId::from_u128(5)]),
            Value::Json(serde_json::json!({"a": [1, "x", null]})),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::*;
    use super::*;

    #[test]
    fn std_golden_serde_forms() {
        let expected = [
            r#"{"type":"null"}"#,
            r#"{"type":"text","value":"Hello"}"#,
            r#"{"type":"long_text","value":"Line 1\nLine 2"}"#,
            r#"{"type":"number","value":"12.5"}"#,
            r#"{"type":"money","value":{"amount":"12.5","currency":"USD"}}"#,
            r#"{"type":"percent","value":"0.25"}"#,
            r#"{"type":"bool","value":true}"#,
            r#"{"type":"date","value":"2024-05-06"}"#,
            r#"{"type":"datetime","value":1714979289123}"#,
            r#"{"type":"duration","value":5400000}"#,
            r#"{"type":"reference","value":"rfi_00000000000000000000000001"}"#,
            r#"{"type":"multi_reference","value":["rfi_00000000000000000000000001","rfi_00000000000000000000000002"]}"#,
            r#"{"type":"link","value":["rec_00000000000000000000000003"]}"#,
            r#"{"type":"principal","value":["prn_00000000000000000000000004"]}"#,
            r#"{"type":"attachment","value":["att_00000000000000000000000005"]}"#,
            r#"{"type":"json","value":{"a":[1,"x",null]}}"#,
        ];
        for (value, json) in samples().iter().zip(expected) {
            assert_eq!(serde_json::to_string(value).unwrap(), json);
            assert_eq!(&serde_json::from_str::<Value>(json).unwrap(), value);
        }
        assert_eq!(samples().len(), expected.len());
    }

    #[test]
    fn std_type_names_match_serde_tags() {
        for value in samples() {
            let json: serde_json::Value = serde_json::to_value(&value).unwrap();
            assert_eq!(json["type"], value.type_name());
            assert_eq!(value.is_null(), value.data_type().is_none());
        }
    }

    #[test]
    fn std_display_plain_text() {
        let expected = [
            "",
            "Hello",
            "Line 1\nLine 2",
            "12.5",
            "12.50 USD",
            "25%",
            "true",
            "2024-05-06",
            "2024-05-06T07:08:09.123Z",
            "PT1H30M",
            "rfi_00000000000000000000000001",
            "rfi_00000000000000000000000001, rfi_00000000000000000000000002",
            "rec_00000000000000000000000003",
            "prn_00000000000000000000000004",
            "att_00000000000000000000000005",
            r#"{"a":[1,"x",null]}"#,
        ];
        for (value, text) in samples().iter().zip(expected) {
            assert_eq!(value.to_string(), text);
        }
    }

    #[test]
    fn std_natural_json() {
        let expected = [
            serde_json::json!(null),
            serde_json::json!("Hello"),
            serde_json::json!("Line 1\nLine 2"),
            serde_json::json!("12.5"),
            serde_json::json!({"amount": "12.5", "currency": "USD"}),
            serde_json::json!("0.25"),
            serde_json::json!(true),
            serde_json::json!("2024-05-06"),
            serde_json::json!("2024-05-06T07:08:09.123Z"),
            serde_json::json!(5_400_000),
            serde_json::json!("rfi_00000000000000000000000001"),
            serde_json::json!([
                "rfi_00000000000000000000000001",
                "rfi_00000000000000000000000002"
            ]),
            serde_json::json!(["rec_00000000000000000000000003"]),
            serde_json::json!(["prn_00000000000000000000000004"]),
            serde_json::json!(["att_00000000000000000000000005"]),
            serde_json::json!({"a": [1, "x", null]}),
        ];
        for (value, json) in samples().iter().zip(expected) {
            assert_eq!(value.to_natural_json(), json);
        }
    }

    #[test]
    fn std_accessors() {
        assert_eq!(Value::Text("a".into()).as_text(), Some("a"));
        assert_eq!(Value::LongText("b".into()).as_text(), Some("b"));
        assert_eq!(Value::Number(dec("1")).as_decimal(), Some(dec("1")));
        assert_eq!(Value::Percent(dec("0.5")).as_decimal(), Some(dec("0.5")));
        assert_eq!(Value::Money(money("3", "JPY")).as_decimal(), Some(dec("3")));
        let mut text = 0;
        let mut decimal = 0;
        let mut counted = 0;
        for v in samples() {
            text += usize::from(v.as_text().is_some());
            decimal += usize::from(v.as_decimal().is_some());
            counted += usize::from(v.item_count().is_some());
        }
        assert_eq!((text, decimal, counted), (2, 3, 4));
        assert_eq!(Value::MultiReference(vec![]).item_count(), Some(0));
    }

    #[test]
    fn edge_empty_and_normalized() {
        let empties = [
            Value::Null,
            Value::Text(String::new()),
            Value::Text("  \t".into()),
            Value::LongText("\n".into()),
            Value::MultiReference(vec![]),
            Value::Link(vec![]),
            Value::Principal(vec![]),
            Value::Attachment(vec![]),
            Value::Json(serde_json::Value::Null),
        ];
        for v in empties {
            assert!(v.is_empty(), "{v:?}");
            assert_eq!(v.normalized(), Value::Null);
        }
        for v in samples().into_iter().skip(1) {
            assert!(!v.is_empty(), "{v:?}");
            assert_eq!(v.clone().normalized(), v);
        }
        assert!(!Value::Bool(false).is_empty());
        assert!(!Value::Number(dec("0")).is_empty());
        let n = Value::Number(Decimal::new(1500, 3)).normalized();
        assert_eq!(n.to_string(), "1.5");
        assert_eq!(
            Value::Percent(Decimal::new(2500, 4))
                .normalized()
                .to_string(),
            "25%"
        );
        // Equality ignores trailing zeros.
        assert_eq!(
            Value::Number(Decimal::new(1500, 3)),
            Value::Number(dec("1.5"))
        );
    }

    #[test]
    fn edge_percent_overflow_falls_back() {
        assert_eq!(
            percent_string(&Decimal::MAX),
            "79228162514264337593543950335"
        );
        assert_eq!(percent_string(&dec("-0.005")), "-0.5%");
    }

    #[test]
    fn invalid_serde_inputs() {
        for bad in [
            r#"{"type":"number","value":12.5}"#,
            r#"{"type":"number","value":"1e3"}"#,
            r#"{"type":"percent","value":1}"#,
            r#"{"type":"bool","value":"true"}"#,
            r#"{"type":"date","value":"2024-02-30"}"#,
            r#"{"type":"datetime","value":"2024-05-06T07:08:09Z"}"#,
            r#"{"type":"reference","value":"rec_00000000000000000000000001"}"#,
            r#"{"type":"link","value":"rec_00000000000000000000000001"}"#,
            r#"{"type":"money","value":{"amount":"1.5","currency":"JPY"}}"#,
            r#"{"type":"unknown","value":1}"#,
            r#"{"type":"text"}"#,
            r#"{"value":"x"}"#,
            r#""text""#,
        ] {
            assert!(serde_json::from_str::<Value>(bad).is_err(), "{bad}");
        }
    }
}

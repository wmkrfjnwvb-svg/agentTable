//! Order-preserving sort keys.
//!
//! [`sort_key`] turns a [`Value`] into bytes such that comparing two keys byte by byte gives
//! exactly the same result as [`compare_values`] on the values. Databases can therefore sort
//! and range-filter on a plain binary column and get the same order on SQLite, MySQL,
//! PostgreSQL and in the browser, independent of any database collation or number type.
//!
//! # The order
//!
//! 1. Null sorts first.
//! 2. Values of different data types sort by data type (declaration order of
//!    [`DataType`](crate::field::DataType)); inside one field all values share a type, so this
//!    only makes the order total.
//! 3. Within a type:
//!    * **text**: by the ASCII-lower-cased text (code point order), ties broken by the
//!      original text. Only ASCII letters are folded, so the order never changes with a Unicode
//!      or toolchain version; locale-aware collation is applied on top by the server when a
//!      view asks for it;
//!    * **number, percent**: numeric order (`1.50` equals `1.5`);
//!    * **money**: by currency code, then amount;
//!    * **bool**: false before true; **date, datetime, duration**: chronological / by length;
//!    * **reference**: by item ID; **lists**: element by element, a prefix before its extensions;
//!    * **JSON**: by its compact text.
//!
//! # Encoding
//!
//! `0x00` is null. Otherwise `0x01`, a type byte, then:
//!
//! * decimals: a sign byte (`0` negative, `1` zero, `2` positive); for non-zero values the
//!   decimal exponent plus 64 and the significant digits (each digit plus 1) ended by `0x00`;
//!   for negative values those bytes are inverted;
//! * integers (dates, instants, durations): big-endian with the sign bit flipped;
//! * text: bytes with `0x00` escaped as `0x00 0xFF`, ended by `0x00 0x00`;
//! * IDs: 16 big-endian bytes; list elements are each preceded by `0x01`, the list ends with `0x00`.

use crate::decimal::Decimal;
use crate::field::DataType;
use crate::value::Value;
use core::cmp::Ordering;

const NULL: u8 = 0x00;
const PRESENT: u8 = 0x01;

/// Position of a data type in the cross-type order.
const fn type_rank(data_type: DataType) -> u8 {
    match data_type {
        DataType::Text => 1,
        DataType::LongText => 2,
        DataType::Number => 3,
        DataType::Money => 4,
        DataType::Percent => 5,
        DataType::Bool => 6,
        DataType::Date => 7,
        DataType::DateTime => 8,
        DataType::Duration => 9,
        DataType::Reference => 10,
        DataType::MultiReference => 11,
        DataType::Link => 12,
        DataType::Principal => 13,
        DataType::Attachment => 14,
        DataType::Json => 15,
    }
}

fn push_decimal(out: &mut Vec<u8>, value: &Decimal) {
    let value = value.normalize();
    if value.is_zero() {
        out.push(1);
        return;
    }
    let negative = value.is_sign_negative();
    let digits_text = value.mantissa().unsigned_abs().to_string();
    let digits = digits_text.trim_end_matches('0').as_bytes();
    // value = 0.DIGITS × 10^exponent, exponent = number of integer digits.
    let length = i64::try_from(digits_text.len()).unwrap_or(i64::MAX);
    let exponent = length.saturating_sub(i64::from(value.scale()));
    // exponent is within -27..=29, so the biased value always fits a byte.
    let biased = u8::try_from(exponent.saturating_add(64)).unwrap_or(u8::MAX);
    let flip = if negative { 0xFF } else { 0x00 };
    out.push(if negative { 0 } else { 2 });
    out.push(biased ^ flip);
    for digit in digits {
        // ASCII digit '0'..='9' (48..=57) becomes 1..=10.
        out.push(digit.saturating_sub(47) ^ flip);
    }
    out.push(flip);
}

fn push_i64(out: &mut Vec<u8>, value: i64) {
    let flipped = u64::from_be_bytes(value.to_be_bytes()) ^ (1 << 63);
    out.extend_from_slice(&flipped.to_be_bytes());
}

fn push_escaped(out: &mut Vec<u8>, bytes: impl Iterator<Item = u8>) {
    for byte in bytes {
        out.push(byte);
        if byte == 0x00 {
            out.push(0xFF);
        }
    }
    out.extend_from_slice(&[0x00, 0x00]);
}

fn push_text(out: &mut Vec<u8>, text: &str) {
    push_escaped(out, text.bytes().map(|b| b.to_ascii_lowercase()));
    push_escaped(out, text.bytes());
}

fn push_ids(out: &mut Vec<u8>, ids: impl Iterator<Item = [u8; 16]>) {
    for id in ids {
        out.push(0x01);
        out.extend_from_slice(&id);
    }
    out.push(0x00);
}

fn json_text(json: &serde_json::Value) -> String {
    // Writing a `serde_json::Value` to a string cannot fail (keys are strings).
    serde_json::to_string(json).unwrap_or_default()
}

/// The order-preserving key of a value. See the module documentation.
#[must_use]
pub fn sort_key(value: &Value) -> Vec<u8> {
    let Some(data_type) = value.data_type() else {
        return vec![NULL];
    };
    let mut out = vec![PRESENT, type_rank(data_type)];
    match value {
        Value::Null => {}
        Value::Text(s) | Value::LongText(s) => push_text(&mut out, s),
        Value::Number(d) | Value::Percent(d) => push_decimal(&mut out, d),
        Value::Money(m) => {
            out.extend_from_slice(m.currency().code().as_bytes());
            push_decimal(&mut out, &m.amount());
        }
        Value::Bool(b) => out.push(u8::from(*b)),
        Value::Date(d) => push_i64(&mut out, d.to_timestamp().millis()),
        Value::DateTime(t) => push_i64(&mut out, t.millis()),
        Value::Duration(d) => push_i64(&mut out, d.millis()),
        Value::Reference(id) => out.extend_from_slice(&id.to_bytes()),
        Value::MultiReference(ids) => push_ids(&mut out, ids.iter().map(|i| i.to_bytes())),
        Value::Link(ids) => push_ids(&mut out, ids.iter().map(|i| i.to_bytes())),
        Value::Principal(ids) => push_ids(&mut out, ids.iter().map(|i| i.to_bytes())),
        Value::Attachment(ids) => push_ids(&mut out, ids.iter().map(|i| i.to_bytes())),
        Value::Json(json) => push_escaped(&mut out, json_text(json).bytes()),
    }
    out
}

/// The first `max_len` bytes of [`sort_key`]. Truncated keys keep the order *weakly*
/// (`a <= b` implies `key(a) <= key(b)`), which is what an index prefix needs; ties must be
/// resolved with the full value.
#[must_use]
pub fn sort_key_truncated(value: &Value, max_len: usize) -> Vec<u8> {
    let mut key = sort_key(value);
    key.truncate(max_len);
    key
}

fn compare_text(a: &str, b: &str) -> Ordering {
    let lower = |s: &str| {
        s.bytes()
            .map(|byte| byte.to_ascii_lowercase())
            .collect::<Vec<u8>>()
    };
    lower(a)
        .cmp(&lower(b))
        .then_with(|| a.as_bytes().cmp(b.as_bytes()))
}

/// The defined total order of values (see the module documentation). [`sort_key`] encodes
/// exactly this order.
#[must_use]
pub fn compare_values(a: &Value, b: &Value) -> Ordering {
    let (type_a, type_b) = match (a.data_type(), b.data_type()) {
        (None, None) => return Ordering::Equal,
        (None, Some(_)) => return Ordering::Less,
        (Some(_), None) => return Ordering::Greater,
        (Some(type_a), Some(type_b)) => (type_a, type_b),
    };
    let by_type = type_rank(type_a).cmp(&type_rank(type_b));
    if by_type != Ordering::Equal {
        return by_type;
    }
    match (a, b) {
        (Value::Text(x), Value::Text(y)) | (Value::LongText(x), Value::LongText(y)) => {
            compare_text(x, y)
        }
        (Value::Number(x), Value::Number(y)) | (Value::Percent(x), Value::Percent(y)) => x.cmp(y),
        (Value::Money(x), Value::Money(y)) => x
            .currency()
            .code()
            .cmp(y.currency().code())
            .then_with(|| x.amount().cmp(&y.amount())),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Date(x), Value::Date(y)) => x.cmp(y),
        (Value::DateTime(x), Value::DateTime(y)) => x.cmp(y),
        (Value::Duration(x), Value::Duration(y)) => x.cmp(y),
        (Value::Reference(x), Value::Reference(y)) => x.cmp(y),
        (Value::MultiReference(x), Value::MultiReference(y)) => x.cmp(y),
        (Value::Link(x), Value::Link(y)) => x.cmp(y),
        (Value::Principal(x), Value::Principal(y)) => x.cmp(y),
        (Value::Attachment(x), Value::Attachment(y)) => x.cmp(y),
        (Value::Json(x), Value::Json(y)) => json_text(x).as_bytes().cmp(json_text(y).as_bytes()),
        // Unreachable in practice: equal type ranks mean equal variants. Kept total and
        // explicit so that a new variant cannot be forgotten silently.
        (
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
            | Value::MultiReference(_)
            | Value::Link(_)
            | Value::Principal(_)
            | Value::Attachment(_)
            | Value::Json(_),
            _,
        ) => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::RefItemId;
    use crate::time::{Date, DurationMs, Timestamp};
    use crate::value::testkit::{dec, money, samples};

    fn assert_sorted(values: &[Value]) {
        for pair in values.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            assert_eq!(
                compare_values(a, b),
                Ordering::Less,
                "{a:?} should sort before {b:?}"
            );
            assert!(
                sort_key(a) < sort_key(b),
                "key of {a:?} should sort before key of {b:?}"
            );
            assert!(sort_key(b) > sort_key(a));
        }
    }

    #[test]
    fn std_samples_sort_by_type_with_null_first() {
        let values = samples();
        assert_eq!(sort_key(&Value::Null), vec![0x00]);
        assert_sorted(&values);
        for value in &values {
            assert_eq!(compare_values(value, value), Ordering::Equal);
        }
    }

    #[test]
    fn std_numbers_sort_numerically() {
        let texts = [
            "-79228162514264337593543950335",
            "-1000000",
            "-1200",
            "-123.45",
            "-123.4",
            "-123",
            "-12.3",
            "-1.5",
            "-1",
            "-0.5",
            "-0.123",
            "-0.12",
            "-0.0000000000000000000000000001",
            "0",
            "0.0000000000000000000000000001",
            "0.0001",
            "0.12",
            "0.123",
            "0.5",
            "1",
            "1.5",
            "9.99",
            "10",
            "12.3",
            "99",
            "100",
            "123",
            "123.4",
            "123.45",
            "1200",
            "1000000",
            "79228162514264337593543950335",
        ];
        let numbers: Vec<Value> = texts.iter().map(|t| Value::Number(dec(t))).collect();
        assert_sorted(&numbers);
        let percents: Vec<Value> = texts.iter().map(|t| Value::Percent(dec(t))).collect();
        assert_sorted(&percents);
    }

    #[test]
    fn edge_equal_numbers_with_different_scale_share_a_key() {
        let a = Value::Number(Decimal::new(1500, 3));
        let b = Value::Number(Decimal::new(15, 1));
        assert_eq!(compare_values(&a, &b), Ordering::Equal);
        assert_eq!(sort_key(&a), sort_key(&b));
        let zero = Value::Number(Decimal::new(0, 5));
        let negative_zero = Value::Number(-Decimal::ZERO);
        assert_eq!(sort_key(&zero), sort_key(&Value::Number(Decimal::ZERO)));
        assert_eq!(
            sort_key(&negative_zero),
            sort_key(&Value::Number(Decimal::ZERO))
        );
        assert_eq!(
            sort_key(&Value::Number(dec("1200"))),
            sort_key(&Value::Number(Decimal::new(120_000, 2)))
        );
    }

    #[test]
    fn std_text_sorts_case_insensitively_then_by_original() {
        let texts = [
            "", "\0", "\0a", "A", "a", "a\0", "a\0b", "aB", "ab", "abc", "B", "b", "z", "é", "日本",
        ];
        let values: Vec<Value> = texts.iter().map(|t| Value::Text((*t).to_owned())).collect();
        assert_sorted(&values);
        let long: Vec<Value> = texts
            .iter()
            .map(|t| Value::LongText((*t).to_owned()))
            .collect();
        assert_sorted(&long);
    }

    #[test]
    fn std_money_sorts_by_currency_then_amount() {
        let values = [
            Value::Money(money("-5", "EUR")),
            Value::Money(money("3.5", "EUR")),
            Value::Money(money("1", "JPY")),
            Value::Money(money("1000", "JPY")),
            Value::Money(money("0.01", "USD")),
            Value::Money(money("2", "USD")),
        ];
        assert_sorted(&values);
    }

    #[test]
    fn std_time_types_sort_chronologically() {
        let dates = [
            Date::MIN,
            Date::from_ymd(1969, 12, 31).unwrap(),
            Date::EPOCH,
            Date::from_ymd(2024, 2, 29).unwrap(),
            Date::MAX,
        ];
        assert_sorted(&dates.map(Value::Date));
        let instants = [
            Timestamp::MIN,
            Timestamp::from_millis(-1).unwrap(),
            Timestamp::EPOCH,
            Timestamp::from_millis(1).unwrap(),
            Timestamp::MAX,
        ];
        assert_sorted(&instants.map(Value::DateTime));
        let durations = [i64::MIN, -1, 0, 1, 86_400_000, i64::MAX]
            .map(|ms| Value::Duration(DurationMs::from_millis(ms)));
        assert_sorted(&durations);
        assert_sorted(&[Value::Bool(false), Value::Bool(true)]);
    }

    #[test]
    fn std_lists_sort_elementwise_with_prefix_first() {
        let id = RefItemId::from_u128;
        let values = [
            Value::MultiReference(vec![]),
            Value::MultiReference(vec![id(0)]),
            Value::MultiReference(vec![id(0), id(0)]),
            Value::MultiReference(vec![id(1)]),
            Value::MultiReference(vec![id(1), id(0)]),
            Value::MultiReference(vec![id(1), id(u128::MAX)]),
            Value::MultiReference(vec![id(2)]),
            Value::MultiReference(vec![id(u128::MAX)]),
        ];
        assert_sorted(&values);
        assert_sorted(&[
            Value::Reference(id(0)),
            Value::Reference(id(1)),
            Value::Reference(id(u128::MAX)),
        ]);
    }

    #[test]
    fn std_json_sorts_by_compact_text() {
        use serde_json::json;
        let values = [
            Value::Json(json!("a")),
            Value::Json(json!([1])),
            Value::Json(json!({"a": 1})),
            Value::Json(json!({"a": 2})),
        ];
        assert_sorted(&values);
    }

    #[test]
    fn edge_truncated_keys_keep_weak_order() {
        let a = Value::Text("abcdef".into());
        let b = Value::Text("abcxyz".into());
        assert!(sort_key_truncated(&a, 5) <= sort_key_truncated(&b, 5));
        assert_eq!(sort_key_truncated(&a, 5), sort_key_truncated(&b, 5));
        assert_eq!(sort_key_truncated(&a, 0), Vec::<u8>::new());
        assert_eq!(sort_key_truncated(&a, 10_000), sort_key(&a));
    }
}

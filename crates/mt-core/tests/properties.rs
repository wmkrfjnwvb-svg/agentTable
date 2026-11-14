//! Property tests over the public API: randomly generated values must satisfy the crate's
//! documented invariants. Failing cases are shrunk and their seed is saved by proptest under
//! `proptest-regressions/`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
use mt_core::coerce::{parse_input, ParseContext};
use mt_core::convert::{convert, convertibility, preview_conversion, Convertibility};
use mt_core::decimal::Decimal;
use mt_core::field::{DataType, FieldDef, OptionsSource, ValidationSpec};
use mt_core::id::{AttachmentId, FieldId, PrincipalId, RecordId, RefItemId, TableId};
use mt_core::money::{Currency, Money, ISO_4217};
use mt_core::sortkey::{compare_values, sort_key, sort_key_truncated};
use mt_core::time::{Date, DurationMs, Timestamp};
use mt_core::validate::{validate_value, AcceptAllOptions};
use mt_core::value::Value;
use proptest::prelude::*;
use std::cmp::Ordering;

fn decimal() -> impl Strategy<Value = Decimal> {
    prop_oneof![
        // Everyday magnitudes with up to 6 decimal places.
        (-1_000_000_000_i64..1_000_000_000, 0_u32..=6).prop_map(|(m, s)| Decimal::new(m, s)),
        // The whole 96-bit mantissa range with any scale.
        (
            any::<u32>(),
            any::<u32>(),
            any::<u32>(),
            any::<bool>(),
            0_u32..=28
        )
            .prop_map(|(lo, mid, hi, negative, scale)| Decimal::from_parts(
                lo, mid, hi, negative, scale
            )),
        Just(Decimal::ZERO),
        Just(Decimal::MAX),
        Just(Decimal::MIN),
    ]
}

fn currency() -> impl Strategy<Value = Currency> {
    (0..ISO_4217.len()).prop_map(|index| Currency::new(ISO_4217[index].0).unwrap())
}

fn money() -> impl Strategy<Value = Money> {
    (-1_000_000_000_000_i64..1_000_000_000_000, currency()).prop_map(|(minor, currency)| {
        let scale = u32::from(currency.minor_units().unwrap_or(2));
        Money::new(Decimal::new(minor, scale), currency).unwrap()
    })
}

fn text() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-zA-Z0-9 ]{0,12}",
        "\\PC{0,8}",
        // Includes NUL, control characters and multi-byte characters.
        proptest::collection::vec(any::<char>(), 0..6)
            .prop_map(|chars| chars.into_iter().collect()),
    ]
}

fn ids<T: 'static + std::fmt::Debug>(make: fn(u128) -> T) -> impl Strategy<Value = Vec<T>> {
    proptest::collection::vec(prop_oneof![0_u128..4, any::<u128>()].prop_map(make), 0..4)
}

fn json() -> impl Strategy<Value = serde_json::Value> {
    let leaf = prop_oneof![
        Just(serde_json::Value::Null),
        any::<bool>().prop_map(serde_json::Value::from),
        any::<i32>().prop_map(serde_json::Value::from),
        "[a-z]{0,6}".prop_map(serde_json::Value::from),
    ];
    leaf.prop_recursive(2, 8, 3, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..3).prop_map(serde_json::Value::from),
            proptest::collection::btree_map("[a-z]{1,3}", inner, 0..3)
                .prop_map(|map| serde_json::Value::Object(map.into_iter().collect())),
        ]
    })
}

fn value_of(data_type: DataType) -> BoxedStrategy<Value> {
    match data_type {
        DataType::Text => text().prop_map(Value::Text).boxed(),
        DataType::LongText => text().prop_map(Value::LongText).boxed(),
        DataType::Number => decimal().prop_map(Value::Number).boxed(),
        DataType::Money => money().prop_map(Value::Money).boxed(),
        DataType::Percent => decimal().prop_map(Value::Percent).boxed(),
        DataType::Bool => any::<bool>().prop_map(Value::Bool).boxed(),
        DataType::Date => (Date::MIN.to_timestamp().millis() / 86_400_000
            ..=Date::MAX.to_timestamp().millis() / 86_400_000)
            .prop_map(|days| Value::Date(Date::from_days(i32::try_from(days).unwrap()).unwrap()))
            .boxed(),
        DataType::DateTime => (Timestamp::MIN.millis()..=Timestamp::MAX.millis())
            .prop_map(|ms| Value::DateTime(Timestamp::from_millis(ms).unwrap()))
            .boxed(),
        DataType::Duration => prop_oneof![any::<i64>(), -100_000_000_i64..100_000_000]
            .prop_map(|ms| Value::Duration(DurationMs::from_millis(ms)))
            .boxed(),
        DataType::Reference => any::<u128>()
            .prop_map(|n| Value::Reference(RefItemId::from_u128(n)))
            .boxed(),
        DataType::MultiReference => ids(RefItemId::from_u128)
            .prop_map(Value::MultiReference)
            .boxed(),
        DataType::Link => ids(RecordId::from_u128).prop_map(Value::Link).boxed(),
        DataType::Principal => ids(PrincipalId::from_u128)
            .prop_map(Value::Principal)
            .boxed(),
        DataType::Attachment => ids(AttachmentId::from_u128)
            .prop_map(Value::Attachment)
            .boxed(),
        DataType::Json => json().prop_map(Value::Json).boxed(),
    }
}

fn data_type() -> impl Strategy<Value = DataType> {
    (0..DataType::ALL.len()).prop_map(|index| DataType::ALL[index])
}

fn any_value() -> impl Strategy<Value = Value> {
    prop_oneof![1 => Just(Value::Null), 15 => data_type().prop_flat_map(value_of)]
}

/// Two values of the same data type (the case that matters inside one column).
fn same_type_pair() -> impl Strategy<Value = (Value, Value)> {
    data_type().prop_flat_map(|t| (value_of(t), value_of(t)))
}

fn usd() -> ParseContext {
    ParseContext {
        default_currency: Some(Currency::new("USD").unwrap()),
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn sort_keys_order_like_compare_values_within_a_type((a, b) in same_type_pair()) {
        prop_assert_eq!(sort_key(&a).cmp(&sort_key(&b)), compare_values(&a, &b));
    }

    #[test]
    fn sort_keys_order_like_compare_values_across_types(a in any_value(), b in any_value()) {
        prop_assert_eq!(sort_key(&a).cmp(&sort_key(&b)), compare_values(&a, &b));
    }

    #[test]
    fn compare_values_is_a_total_order(a in any_value(), b in any_value(), c in any_value()) {
        prop_assert_eq!(compare_values(&a, &a), Ordering::Equal);
        prop_assert_eq!(compare_values(&a, &b), compare_values(&b, &a).reverse());
        if compare_values(&a, &b) != Ordering::Greater && compare_values(&b, &c) != Ordering::Greater {
            prop_assert_ne!(compare_values(&a, &c), Ordering::Greater);
        }
    }

    #[test]
    fn equal_keys_mean_equal_values_up_to_number_scale((a, b) in same_type_pair()) {
        if sort_key(&a) == sort_key(&b) {
            prop_assert_eq!(a.normalized(), b.normalized());
        }
    }

    #[test]
    fn truncated_keys_keep_the_order_weakly((a, b) in same_type_pair(), len in 0_usize..40) {
        if compare_values(&a, &b) != Ordering::Greater {
            prop_assert!(sort_key_truncated(&a, len) <= sort_key_truncated(&b, len));
        }
    }

    #[test]
    fn serde_round_trips_every_value(value in any_value()) {
        let json = serde_json::to_string(&value).unwrap();
        prop_assert_eq!(serde_json::from_str::<Value>(&json).unwrap(), value);
    }

    #[test]
    fn display_then_parse_gives_the_value_back(value in any_value()) {
        let Some(data_type) = value.data_type() else { return Ok(()) };
        // Free text is trimmed on input, and a percent beyond the percent-point range has no
        // faithful text form; both are documented.
        let percent_overflow = matches!(&value, Value::Percent(d) if d.checked_mul(Decimal::ONE_HUNDRED).is_none());
        if data_type.is_text() || percent_overflow {
            return Ok(());
        }
        let parsed = parse_input(data_type, &value.to_string(), &ParseContext::default()).unwrap();
        prop_assert_eq!(parsed, value.normalized());
    }

    #[test]
    fn parse_input_never_panics_on_arbitrary_text(data_type in data_type(), input in "\\PC{0,40}") {
        let _ = parse_input(data_type, &input, &usd());
    }

    #[test]
    fn conversion_agrees_with_the_matrix(value in any_value(), to in data_type()) {
        // Empty values stay empty whatever the pair of types.
        if value.is_empty() {
            prop_assert_eq!(convert(&value, to, &usd()).unwrap().value, Value::Null);
            return Ok(());
        }
        let Some(from) = value.data_type() else { return Ok(()) };
        let result = convert(&value, to, &usd());
        match convertibility(from, to) {
            Convertibility::Impossible => prop_assert!(result.is_err()),
            Convertibility::Lossless => {
                // The only lossless pair that can fail: a percent too large for percent points.
                let percent_overflow = matches!(&value, Value::Percent(d) if d.checked_mul(Decimal::ONE_HUNDRED).is_none());
                if !(percent_overflow && to.is_text()) {
                    let converted = result.unwrap();
                    prop_assert!(!converted.lossy);
                    // Text made of control characters only flattens to nothing.
                    prop_assert!(converted.value.data_type() == Some(to) || converted.value.is_null());
                }
            }
            Convertibility::Lossy => {
                if let Ok(converted) = result {
                    prop_assert!(converted.value.is_null() || converted.value.data_type() == Some(to));
                }
            }
        }
    }

    #[test]
    fn lossless_conversions_round_trip(value in any_value(), to in data_type()) {
        let Some(from) = value.data_type() else { return Ok(()) };
        let json_to_text = matches!(from, DataType::Json) && to.is_text();
        if from == to || json_to_text || convertibility(from, to) != Convertibility::Lossless {
            return Ok(());
        }
        // Text is trimmed and flattened when read back; skip text that would change.
        if let Some(text) = value.as_text() {
            if text.trim() != text || text.chars().any(char::is_control) {
                return Ok(());
            }
        }
        let Ok(there) = convert(&value, to, &usd()) else { return Ok(()) };
        let back = convert(&there.value, from, &usd()).unwrap();
        prop_assert_eq!(back.value, value.normalized());
    }

    #[test]
    fn preview_counts_add_up(values in proptest::collection::vec(any_value(), 0..20), to in data_type()) {
        let preview = preview_conversion(&values, to, &usd());
        prop_assert_eq!(preview.total, values.len());
        prop_assert_eq!(preview.empty + preview.clean + preview.lossy + preview.failed, preview.total);
        prop_assert!(preview.first_failures.len() <= preview.failed);
    }

    #[test]
    fn a_value_of_the_field_type_passes_a_field_without_rules(value in any_value()) {
        let Some(data_type) = value.data_type() else { return Ok(()) };
        let mut field = FieldDef::new(FieldId::from_u128(1), "f", data_type);
        if data_type.is_reference() {
            field.options = OptionsSource::RefSet { set: mt_core::id::RefSetId::from_u128(1), subtree: None, leaves_only: false };
        }
        if matches!(data_type, DataType::Link) {
            field.options = OptionsSource::Table { table: TableId::from_u128(1) };
        }
        field.check().unwrap();
        let issues = validate_value(&field, &value, None, &AcceptAllOptions);
        // Only two things can be wrong with a rule-free field: a control character in
        // single-line text, or the same item twice in a list.
        for issue in issues {
            let code = issue.code.as_str();
            prop_assert!(code == "control_character" || code == "duplicate_items", "unexpected {code}");
        }
    }

    #[test]
    fn required_rejects_exactly_the_empty_values(value in any_value()) {
        let Some(data_type) = value.data_type() else { return Ok(()) };
        let mut field = FieldDef::new(FieldId::from_u128(1), "f", data_type);
        field.validation = ValidationSpec { required: true, ..ValidationSpec::default() };
        let issues = validate_value(&field, &value, None, &AcceptAllOptions);
        let required = issues.iter().any(|issue| issue.code.as_str() == "required");
        prop_assert_eq!(required, value.is_empty());
    }

    #[test]
    fn ids_round_trip_and_sort_by_time(ms_a in 0_u64..=0xFFFF_FFFF_FFFF, ms_b in 0_u64..=0xFFFF_FFFF_FFFF, ra in any::<[u8; 10]>(), rb in any::<[u8; 10]>()) {
        let a = RecordId::new(ms_a, ra).unwrap();
        let b = RecordId::new(ms_b, rb).unwrap();
        prop_assert_eq!(a.to_string().parse::<RecordId>().unwrap(), a);
        prop_assert_eq!(a.timestamp_ms(), ms_a);
        prop_assert_eq!(a.to_string().cmp(&b.to_string()), a.cmp(&b));
        prop_assert_eq!(a.to_bytes().cmp(&b.to_bytes()), a.cmp(&b));
        if ms_a < ms_b {
            prop_assert!(a < b);
        }
        // A record ID never parses as another kind.
        prop_assert!(a.to_string().parse::<FieldId>().is_err());
    }
}

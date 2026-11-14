#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
//! Concurrency: `mt-core` has no shared mutable state, so its types can be used freely from
//! many threads and every function gives the same answer whichever thread runs it.

use mt_core::coerce::{parse_input, ParseContext};
use mt_core::field::{DataType, FieldDef};
use mt_core::id::{FieldId, RecordId};
use mt_core::provenance::CellMeta;
use mt_core::sortkey::sort_key;
use mt_core::validate::{validate_value, AcceptAllOptions, ValidationIssue};
use mt_core::value::Value;
use mt_core::CoreError;
use std::sync::Arc;
use std::thread;

fn assert_send_sync<T: Send + Sync + 'static>() {}

#[test]
fn concurrency_public_types_are_send_and_sync() {
    assert_send_sync::<Value>();
    assert_send_sync::<FieldDef>();
    assert_send_sync::<CellMeta>();
    assert_send_sync::<CoreError>();
    assert_send_sync::<ValidationIssue>();
    assert_send_sync::<RecordId>();
}

#[test]
fn concurrency_parallel_threads_compute_identical_results() {
    let inputs: Arc<Vec<(DataType, String)>> = Arc::new(
        (0..2_000)
            .map(|i| match i % 5 {
                0 => (DataType::Number, format!("{i}.{:03}", i % 1000)),
                1 => (DataType::Text, format!("Item {i}")),
                2 => (
                    DataType::Bool,
                    if i % 2 == 0 { "yes" } else { "no" }.to_owned(),
                ),
                3 => (
                    DataType::Date,
                    format!("20{:02}-{:02}-{:02}", i % 100, i % 12 + 1, i % 28 + 1),
                ),
                _ => (DataType::Duration, format!("{}m", i)),
            })
            .collect(),
    );
    let compute = |inputs: &[(DataType, String)]| -> Vec<(Vec<u8>, usize)> {
        inputs
            .iter()
            .map(|(data_type, text)| {
                let value = parse_input(*data_type, text, &ParseContext::default()).unwrap();
                let field = FieldDef::new(FieldId::from_u128(1), "f", *data_type);
                let issues = validate_value(&field, &value, None, &AcceptAllOptions);
                (sort_key(&value), issues.len())
            })
            .collect()
    };
    let expected = compute(&inputs);
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let inputs = Arc::clone(&inputs);
            thread::spawn(move || compute(&inputs))
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().unwrap(), expected);
    }
}

#[test]
fn concurrency_ids_generated_in_parallel_with_distinct_randomness_never_collide() {
    let handles: Vec<_> = (0_u8..8)
        .map(|worker| {
            thread::spawn(move || {
                (0_u16..1_000)
                    .map(|n| {
                        let [hi, lo] = n.to_be_bytes();
                        RecordId::new(1_700_000_000_000, [worker, hi, lo, 0, 0, 0, 0, 0, 0, 0])
                            .unwrap()
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let mut all: Vec<RecordId> = handles
        .into_iter()
        .flat_map(|h| h.join().unwrap())
        .collect();
    let total = all.len();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), total);
}

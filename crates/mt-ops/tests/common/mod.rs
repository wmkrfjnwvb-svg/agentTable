//! Shared helpers for the `mt-ops` tests.
#![allow(dead_code)]

use mt_core::decimal::Decimal;
use mt_core::id::{BaseId, FieldId, PrincipalId, RecordId, RefItemId, TableId};
use mt_core::time::{DurationMs, Timestamp};
use mt_core::value::Value;
use mt_ops::{Accepted, Batch, ClientId, ClientOpId, Decision, Op, Outcome, Rejection};
use proptest::prelude::*;
use std::collections::BTreeMap;

pub fn base_id() -> BaseId {
    BaseId::from_u128(1)
}

pub fn table(n: u128) -> TableId {
    TableId::from_u128(n)
}

pub fn record(n: u128) -> RecordId {
    RecordId::from_u128(n)
}

pub fn field(n: u128) -> FieldId {
    FieldId::from_u128(n)
}

pub fn client(n: u64) -> ClientId {
    ClientId(n)
}

pub fn id(c: u64, seq: u64) -> ClientOpId {
    ClientOpId {
        client: client(c),
        seq,
    }
}

pub fn text(s: &str) -> Value {
    Value::Text(s.to_owned())
}

pub fn batch(c: u64, seq: u64, ops: Vec<Op>) -> Batch {
    Batch {
        id: id(c, seq),
        base: base_id(),
        ops,
    }
}

pub fn create(t: u128, r: u128, cells: &[(u128, Value)]) -> Op {
    Op::Create {
        table: table(t),
        record: record(r),
        cells: cells.iter().map(|(f, v)| (field(*f), v.clone())).collect(),
    }
}

pub fn set(t: u128, r: u128, f: u128, value: Value, expected: u64) -> Op {
    Op::Set {
        table: table(t),
        record: record(r),
        field: field(f),
        value,
        expected,
    }
}

pub fn transition(t: u128, r: u128, f: u128, value: Value, expected: u64) -> Op {
    Op::Transition {
        table: table(t),
        record: record(r),
        field: field(f),
        value,
        expected,
    }
}

pub fn delete(t: u128, r: u128, expected: u64) -> Op {
    Op::Delete {
        table: table(t),
        record: record(r),
        expected,
    }
}

pub fn restore(t: u128, r: u128) -> Op {
    Op::Restore {
        table: table(t),
        record: record(r),
    }
}

pub fn accepted(decision: &Decision) -> Accepted {
    match &decision.outcome {
        Outcome::Accepted(a) => a.clone(),
        Outcome::Rejected(r) => panic!("refused: {r:?}"),
    }
}

pub fn rejected(decision: &Decision) -> Rejection {
    match &decision.outcome {
        Outcome::Rejected(r) => r.clone(),
        Outcome::Accepted(a) => panic!("accepted: {a:?}"),
    }
}

/// Values of every shape, including arbitrary JSON (which needs the wire's text form).
pub fn any_value() -> impl Strategy<Value = Value> {
    let json_leaf = prop_oneof![
        Just(serde_json::Value::Null),
        any::<bool>().prop_map(serde_json::Value::from),
        any::<i32>().prop_map(serde_json::Value::from),
        "[a-z]{0,6}".prop_map(serde_json::Value::from),
    ];
    let json = json_leaf.prop_recursive(2, 8, 3, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..3).prop_map(serde_json::Value::from),
            proptest::collection::btree_map("[a-z]{1,3}", inner, 0..3)
                .prop_map(|m| serde_json::Value::Object(m.into_iter().collect())),
        ]
    });
    prop_oneof![
        Just(Value::Null),
        "\\PC{0,8}".prop_map(Value::Text),
        "[a-z ]{0,20}".prop_map(Value::LongText),
        (-1_000_000_000_i64..1_000_000_000, 0_u32..=6)
            .prop_map(|(m, s)| Value::Number(Decimal::new(m, s))),
        (-1_000_i64..1_000, 0_u32..=2).prop_map(|(m, s)| Value::Percent(Decimal::new(m, s))),
        any::<bool>().prop_map(Value::Bool),
        (0_i64..4_000_000_000_000)
            .prop_map(|ms| Value::DateTime(Timestamp::from_millis(ms).unwrap())),
        any::<i64>().prop_map(|ms| Value::Duration(DurationMs::from_millis(ms))),
        any::<u128>().prop_map(|n| Value::Reference(RefItemId::from_u128(n))),
        proptest::collection::vec(any::<u128>(), 0..4)
            .prop_map(|v| Value::Link(v.into_iter().map(RecordId::from_u128).collect())),
        proptest::collection::vec(any::<u128>(), 0..3)
            .prop_map(|v| Value::Principal(v.into_iter().map(PrincipalId::from_u128).collect())),
        json.prop_map(Value::Json),
    ]
}

/// A small world: 2 tables × 4 records × 3 fields, so operations collide often.
pub fn any_op() -> impl Strategy<Value = Op> {
    let t = 1_u128..3;
    let r = 1_u128..5;
    let f = 1_u128..4;
    let small_value = prop_oneof![
        1 => Just(Value::Null),
        4 => "[a-c]{1,2}".prop_map(Value::Text),
        2 => (0_i64..5).prop_map(|n| Value::Number(Decimal::new(n, 0))),
    ];
    let version = prop_oneof![Just(0_u64), 0_u64..12];
    prop_oneof![
        3 => (t.clone(), r.clone(), proptest::collection::btree_map(f.clone(), small_value.clone(), 0..3))
            .prop_map(|(t, r, cells)| Op::Create { table: table(t), record: record(r), cells: cells.into_iter().map(|(f, v)| (field(f), v)).collect::<BTreeMap<_, _>>() }),
        6 => (t.clone(), r.clone(), f.clone(), small_value.clone(), version.clone()).prop_map(|(t, r, f, v, e)| set(t, r, f, v, e)),
        2 => (t.clone(), r.clone(), f, small_value, version.clone()).prop_map(|(t, r, f, v, e)| transition(t, r, f, v, e)),
        1 => (t.clone(), r.clone(), version).prop_map(|(t, r, e)| delete(t, r, e)),
        1 => (t, r).prop_map(|(t, r)| restore(t, r)),
    ]
}

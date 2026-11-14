//! Property tests for the chain and canonical JSON.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic
)]

mod common;

use common::{draft, entries, log, user};
use mt_audit::canonical::is_canonical;
use mt_audit::{
    canonical_json, verify_entries, AuditDraft, AuditEntry, Hash32, InMemoryAuditStore, ObjectRef,
    PrincipalKind, TraceId,
};
use proptest::prelude::*;
use serde_json::Value;

fn flip_text(text: &mut String, pos: usize, mask: u8) {
    // Fixture text is ASCII; a 7-bit mask keeps it valid UTF-8.
    let mut bytes = std::mem::take(text).into_bytes();
    let i = pos % bytes.len();
    bytes[i] ^= mask;
    *text = String::from_utf8(bytes).unwrap();
}

fn flip_hash(hash: &mut Hash32, pos: usize, mask: u8) {
    let mut bytes = *hash.as_bytes();
    bytes[pos % 32] ^= mask;
    *hash = Hash32::from_bytes(bytes);
}

/// Flips bits of exactly one byte of one field of `e`.
fn mutate(e: &mut AuditEntry, field: u8, pos: usize, mask: u8) {
    match field {
        0 => {
            let mut b = e.seq.to_be_bytes();
            b[pos % 8] ^= mask;
            e.seq = u64::from_be_bytes(b);
        }
        1 => flip_hash(&mut e.prev_hash, pos, mask),
        2 => flip_hash(&mut e.hash, pos, mask),
        3 => flip_text(&mut e.actor.id, pos, mask),
        4 => flip_text(&mut e.initiator.id, pos, mask),
        5 => flip_text(&mut e.action, pos, mask),
        6 => flip_text(&mut e.object.kind, pos, mask),
        7 => flip_text(&mut e.object.id, pos, mask),
        8 => flip_text(&mut e.details, pos, mask),
        9 => {
            let mut b = e.at_ms.to_be_bytes();
            b[pos % 8] ^= mask;
            e.at_ms = i64::from_be_bytes(b);
        }
        10 => {
            let mut b = *e.trace_id.unwrap().as_bytes();
            b[pos % 16] ^= mask;
            // 0x11.. fixture bytes never become all zero from a one-byte flip.
            e.trace_id = Some(TraceId::from_bytes(b).unwrap());
        }
        11 => e.trace_id = None,
        _ => {
            e.actor.kind = if e.actor.kind == PrincipalKind::User {
                PrincipalKind::Service
            } else {
                PrincipalKind::User
            };
        }
    }
}

fn traced_chain(n: u64) -> Vec<AuditEntry> {
    let log = log();
    let mut store = InMemoryAuditStore::new();
    for i in 1..=n {
        let d = draft(i).with_trace(TraceId::from_bytes([0x11; 16]).unwrap());
        log.append(&mut store, d).unwrap();
    }
    store.entries().to_vec()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// Any single-byte mutation of any field of any entry (including the last) is detected, and the
    /// verifier vouches for exactly the entries before the mutated one.
    #[test]
    fn prop_single_byte_mutation_is_detected_and_localised(
        n in 1u64..25, pick in any::<prop::sample::Index>(), field in 0u8..13,
        pos in 0usize..4096, mask in 1u8..=127,
    ) {
        let mut es = traced_chain(n);
        let idx = pick.index(es.len());
        let original = es[idx].clone();
        mutate(&mut es[idx], field, pos, mask);
        prop_assert_ne!(&es[idx], &original);
        let verdict = verify_entries(log().genesis(), &es);
        let finding = verdict.finding();
        prop_assert!(finding.is_some(), "undetected: field {} of entry {}", field, idx + 1);
        prop_assert_eq!(finding.unwrap().last_good.seq, idx as u64);
    }

    /// Removing, duplicating or swapping entries is always detected.
    #[test]
    fn prop_structural_tampering_is_detected(
        n in 2u64..30, a in any::<prop::sample::Index>(), b in any::<prop::sample::Index>(), op in 0u8..3,
    ) {
        let (log, mut es) = entries(n);
        let (i, j) = (a.index(es.len()), b.index(es.len()));
        match op {
            0 => {
                prop_assume!(i + 1 < es.len()); // removing the unsigned tail needs a checkpoint
                es.remove(i);
            }
            1 => {
                let copy = es[i].clone();
                es.insert(j, copy);
            }
            _ => {
                prop_assume!(i != j);
                es.swap(i, j);
            }
        }
        prop_assert!(!verify_entries(log.genesis(), &es).is_intact());
    }

    /// Appending arbitrary drafts always yields a verifiable chain, and details are canonical.
    #[test]
    fn prop_appended_chain_always_verifies(
        items in proptest::collection::vec(("[a-z]{1,8}", "[a-z.]{1,12}", any::<i64>(), json_value()), 0..20)
    ) {
        let log = log();
        let mut store = InMemoryAuditStore::new();
        for (who, action, at, details) in items {
            let d = AuditDraft::direct(user(&who), action, ObjectRef::new("record", "r"), at)
                .with_details(details);
            let e = log.append(&mut store, d).unwrap();
            prop_assert!(is_canonical(&e.details));
        }
        prop_assert!(verify_entries(log.genesis(), store.entries()).is_intact());
    }

    /// Canonical JSON is deterministic, re-parses to the same value, and is a fixed point.
    #[test]
    fn prop_canonical_json_round_trip(v in json_value()) {
        let text = canonical_json(&v).unwrap();
        let back: Value = serde_json::from_str(&text).unwrap();
        prop_assert_eq!(&back, &v);
        prop_assert_eq!(canonical_json(&back).unwrap(), text.clone());
        prop_assert!(is_canonical(&text));
    }
}

fn json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<u64>().prop_map(Value::from),
        "\\PC{0,12}".prop_map(Value::from),
        any::<String>().prop_map(Value::from),
    ];
    leaf.prop_recursive(4, 32, 6, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            proptest::collection::btree_map("\\PC{0,6}", inner, 0..6)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

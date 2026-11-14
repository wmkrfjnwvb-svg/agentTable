//! Shared test helpers: a tiny seeded generator (no external randomness) and fixtures.
#![allow(
    dead_code,
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::indexing_slicing
)]

use mt_audit::{
    AuditDraft, AuditEntry, AuditLog, HmacSha256Signer, InMemoryAuditStore, ObjectRef, Principal,
    PrincipalKind,
};
use serde_json::json;

/// SplitMix64: deterministic, seedable.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

pub fn user(id: &str) -> Principal {
    Principal::new(PrincipalKind::User, id)
}

pub fn draft(i: u64) -> AuditDraft {
    AuditDraft::direct(
        user(&format!("u{}", i % 7)),
        "record.update",
        ObjectRef::new("record", format!("rec{i}")),
        1_700_000_000_000 + i64::try_from(i).unwrap(),
    )
    .with_details(json!({"n": i, "field": "status", "new": "open"}))
}

pub fn signer() -> HmacSha256Signer {
    HmacSha256Signer::new("k1", b"0123456789abcdef0123456789abcdef").unwrap()
}

pub fn log() -> AuditLog {
    AuditLog::new("tenant-a").unwrap()
}

/// A store holding `n` valid entries.
pub fn chain(n: u64) -> (AuditLog, InMemoryAuditStore) {
    let log = log();
    let mut store = InMemoryAuditStore::new();
    for i in 1..=n {
        log.append(&mut store, draft(i)).unwrap();
    }
    (log, store)
}

pub fn entries(n: u64) -> (AuditLog, Vec<AuditEntry>) {
    let (log, store) = chain(n);
    (log, store.entries().to_vec())
}

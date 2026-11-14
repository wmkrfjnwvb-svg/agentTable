//! Audit chain: append, store contract, checkpoints, verification, tamper localisation, privacy,
//! canonical JSON, simulated concurrent appends, large chain.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic
)]

mod common;

use std::collections::BTreeSet;
use std::num::NonZeroU64;

use common::{chain, draft, entries, log, signer, user, Rng};
use mt_audit::canonical::{is_canonical, MAX_DEPTH};
use mt_audit::chain::{MAX_ACTION_BYTES, MAX_DETAILS_BYTES, MAX_ID_BYTES};
use mt_audit::privacy::value_digest;
use mt_audit::{
    canonical_json, find_fork, verify_against_checkpoint, verify_entries, verify_store, AuditDraft,
    AuditEntry, AuditError, AuditLog, AuditStore, ChainHead, ChainVerdict, ChainVerifier,
    Checkpoint, CheckpointSigner, CheckpointVerifier, Hash32, HmacSha256Signer, InMemoryAuditStore,
    ObjectRef, Principal, PrincipalKind, Privacy, TamperKind, TraceId,
};
use serde_json::{json, Value};

fn finding(verdict: &ChainVerdict) -> (u64, TamperKind, u64) {
    let f = verdict
        .finding()
        .unwrap_or_else(|| panic!("expected tampering, got {verdict:?}"));
    (f.seq, f.kind.clone(), f.last_good.seq)
}

// ---- standard ----

#[test]
fn standard_append_links_entries() {
    let (log, store) = chain(5);
    let es = store.entries();
    assert_eq!(es[0].seq, 1);
    assert_eq!(es[0].prev_hash, log.genesis().hash);
    for pair in es.windows(2) {
        assert_eq!(pair[1].prev_hash, pair[0].hash);
        assert_eq!(pair[1].seq, pair[0].seq + 1);
    }
    assert!(es.iter().all(AuditEntry::is_self_consistent));
    assert_eq!(log.head(&store).unwrap(), es[4].head());
    assert_eq!(store.get(3).unwrap().unwrap(), es[2]);
    assert_eq!(store.get(0).unwrap(), None);
    assert_eq!(store.get(6).unwrap(), None);
    assert_eq!(store.scan(3, 10).unwrap(), es[3..].to_vec());
    assert_eq!(
        verify_entries(log.genesis(), es),
        ChainVerdict::Intact {
            head: es[4].head(),
            entries: 5
        }
    );
    assert!(verify_entries(log.genesis(), es).is_intact());
    // A range verifies from any trusted anchor.
    assert!(verify_entries(es[1].head(), &es[2..]).is_intact());
    assert_eq!(log.chain_id(), "tenant-a");
}

#[test]
fn standard_dual_identity_and_trace_are_recorded_and_hashed() {
    let log = log();
    let mut store = InMemoryAuditStore::new();
    let trace = TraceId::from_bytes([7; 16]).unwrap();
    let agent = Principal::new(PrincipalKind::AiAgent, "agent-1");
    let d = AuditDraft::direct(
        agent.clone(),
        "ai.run",
        ObjectRef::new("ai_run", "run-1"),
        5,
    )
    .on_behalf_of(user("alice"))
    .with_details(json!({"tools": ["search"], "cost_micros": 1200}))
    .with_trace(trace);
    let e = log.append(&mut store, d).unwrap();
    assert_eq!(
        (e.actor.clone(), e.initiator.clone()),
        (agent, user("alice"))
    );
    assert_eq!(e.trace_id, Some(trace));
    assert_eq!(e.details, r#"{"cost_micros":1200,"tools":["search"]}"#);
    // Every identity-relevant field is covered by the hash.
    let variants: Vec<AuditEntry> = vec![
        AuditEntry {
            initiator: user("bob"),
            ..e.clone()
        },
        AuditEntry {
            actor: Principal::new(PrincipalKind::AiSkill, "agent-1"),
            ..e.clone()
        },
        AuditEntry {
            trace_id: None,
            ..e.clone()
        },
        AuditEntry {
            trace_id: Some(TraceId::from_bytes([8; 16]).unwrap()),
            ..e.clone()
        },
        AuditEntry {
            at_ms: 6,
            ..e.clone()
        },
        AuditEntry {
            action: "ai.run2".into(),
            ..e.clone()
        },
        AuditEntry {
            object: ObjectRef::new("ai_run", "run-2"),
            ..e.clone()
        },
        AuditEntry {
            object: ObjectRef::new("ai_ru", "nrun-1"),
            ..e.clone()
        },
    ];
    for v in variants {
        assert_ne!(v.compute_hash(), e.hash, "{v:?}");
    }
    // All principal kinds hash differently.
    let kinds = [
        PrincipalKind::User,
        PrincipalKind::Service,
        PrincipalKind::AiSkill,
        PrincipalKind::AiAgent,
        PrincipalKind::Rule,
        PrincipalKind::Schedule,
        PrincipalKind::System,
    ];
    let hashes: BTreeSet<Hash32> = kinds
        .iter()
        .map(|k| {
            AuditEntry {
                actor: Principal::new(*k, "x"),
                ..e.clone()
            }
            .compute_hash()
        })
        .collect();
    assert_eq!(hashes.len(), kinds.len());
}

#[test]
fn standard_golden_vector_is_stable() {
    // Guards the canonical serialisation: any change to the preimage breaks stored chains.
    // Expected values were computed independently (Python hashlib) from the documented preimage.
    let log = AuditLog::new("golden").unwrap();
    let d = AuditDraft::direct(
        user("u1"),
        "record.create",
        ObjectRef::new("record", "r1"),
        1_000,
    )
    .with_details(json!({"b": 1, "a": "x"}));
    let e = log.prepare(&log.genesis(), d).unwrap();
    assert_eq!(
        log.genesis().hash.to_string(),
        "46fbdb21becc54e946507282905d70c87429642c30d8d85012dbd78eec186e19",
        "genesis hash changed"
    );
    assert_eq!(
        e.hash.to_string(),
        "89d0691ff3126487d1ff98607e9a096976db6e7662e443487775009bb5385191",
        "entry hash changed"
    );
}

#[test]
fn standard_entry_and_hash_serde() {
    let (_, es) = entries(2);
    let text = serde_json::to_string(&es[1]).unwrap();
    assert_eq!(serde_json::from_str::<AuditEntry>(&text).unwrap(), es[1]);
    let h = es[1].hash;
    assert_eq!(h.to_string().parse::<Hash32>().unwrap(), h);
    assert_eq!(Hash32::from_bytes(*h.as_bytes()), h);
    assert_eq!(format!("{h:?}"), format!("Hash32({h})"));
    assert_eq!("xyz".parse::<Hash32>(), Err(AuditError::InvalidHash));
    assert_eq!(
        h.to_string().to_uppercase().parse::<Hash32>(),
        Err(AuditError::InvalidHash)
    );
    assert!(serde_json::from_str::<Hash32>("\"00\"").is_err());
}

#[test]
fn standard_chains_of_different_tenants_do_not_splice() {
    let (_, a) = entries(3);
    let other = AuditLog::new("tenant-b").unwrap();
    assert_ne!(other.genesis().hash, log().genesis().hash);
    let (seq, kind, good) = finding(&verify_entries(other.genesis(), &a));
    assert_eq!((seq, kind, good), (1, TamperKind::BrokenLink, 0));
}

// ---- checkpoints ----

#[test]
fn standard_periodic_checkpoints_and_store_verification() {
    let log = log().with_checkpoint_every(NonZeroU64::new(10).unwrap());
    let key = signer();
    let mut store = InMemoryAuditStore::new();
    let mut made = 0;
    for i in 1..=35 {
        let (entry, cp) = log.append_checkpointed(&mut store, draft(i), &key).unwrap();
        if let Some(cp) = cp {
            made += 1;
            assert_eq!(cp.head(), entry.head());
            assert_eq!(cp.at_ms, entry.at_ms);
            assert_eq!(entry.seq % 10, 0);
            cp.verify_signature("tenant-a", &key).unwrap();
        }
    }
    assert_eq!(made, 3);
    let manual = log.checkpoint(&mut store, &key, 99).unwrap();
    assert_eq!(manual.seq, 35);
    let seqs: Vec<u64> = store.checkpoints().unwrap().iter().map(|c| c.seq).collect();
    assert_eq!(seqs, [10, 20, 30, 35]);
    assert_eq!(
        verify_store(&log, &store, &key).unwrap(),
        ChainVerdict::Intact {
            head: store.head().unwrap().unwrap(),
            entries: 35
        }
    );
    // Without a cadence no checkpoint is made.
    let plain = common::log();
    let mut s2 = InMemoryAuditStore::new();
    assert!(plain
        .append_checkpointed(&mut s2, draft(1), &key)
        .unwrap()
        .1
        .is_none());
    let json = serde_json::to_string(&manual).unwrap();
    assert_eq!(serde_json::from_str::<Checkpoint>(&json).unwrap(), manual);
}

#[test]
fn standard_range_against_checkpoint() {
    let (log, mut store) = chain(20);
    let key = signer();
    let cp = log.checkpoint(&mut store, &key, 1).unwrap();
    let es = store.entries();
    let ok = verify_against_checkpoint(&log, es[9].head(), &es[10..], &cp, &key).unwrap();
    assert_eq!(
        ok,
        ChainVerdict::Intact {
            head: es[19].head(),
            entries: 10
        }
    );
    // Empty range whose anchor is the checkpointed head.
    assert!(
        verify_against_checkpoint(&log, es[19].head(), &[], &cp, &key)
            .unwrap()
            .is_intact()
    );
    // Anchor that contradicts the checkpoint.
    let wrong = ChainHead {
        seq: 20,
        hash: es[0].hash,
    };
    let (seq, kind, _) = finding(&verify_against_checkpoint(&log, wrong, &[], &cp, &key).unwrap());
    assert_eq!((seq, kind), (20, TamperKind::CheckpointMismatch));
    // Range that stops short of the checkpoint.
    let short = verify_against_checkpoint(&log, es[9].head(), &es[10..15], &cp, &key).unwrap();
    assert_eq!(
        finding(&short),
        (16, TamperKind::Truncated { stored_through: 15 }, 15)
    );
}

#[test]
fn invalid_checkpoint_inputs() {
    let (log, mut store) = chain(5);
    let key = signer();
    let cp = log.checkpoint(&mut store, &key, 1).unwrap();
    let es = store.entries().to_vec();
    // Checkpoint before the anchor.
    let early = Checkpoint::sign("tenant-a", es[1].head(), 1, &key).unwrap();
    assert_eq!(
        verify_against_checkpoint(&log, es[3].head(), &es[4..], &early, &key),
        Err(AuditError::CheckpointOutOfRange {
            checkpoint_seq: 2,
            anchor_seq: 4
        })
    );
    // Unknown key / algorithm is an error, not a verdict.
    let other = HmacSha256Signer::new("k2", &[9u8; 32]).unwrap();
    assert_eq!(
        verify_against_checkpoint(&log, log.genesis(), &es, &cp, &other),
        Err(AuditError::UnknownKey)
    );
    assert_eq!(
        verify_store(&log, &store, &other),
        Err(AuditError::UnknownKey)
    );
    let mut alg = cp.clone();
    alg.algorithm = "ed25519".into();
    assert_eq!(
        alg.verify_signature("tenant-a", &key),
        Err(AuditError::UnknownKey)
    );
    // Same key id, different secret: bad signature finding.
    let impostor = HmacSha256Signer::new("k1", &[9u8; 32]).unwrap();
    let v = verify_store(&log, &store, &impostor).unwrap();
    assert_eq!(finding(&v), (5, TamperKind::BadCheckpointSignature, 0));
    // Checkpoint signed for another chain does not verify here.
    assert_eq!(
        cp.verify_signature("tenant-b", &key),
        Err(AuditError::BadSignature)
    );
    // Every signed field matters.
    for mutate in [
        (|c: &mut Checkpoint| c.seq += 1) as fn(&mut Checkpoint),
        |c| c.at_ms += 1,
        |c| c.hash = Hash32::from_bytes([1; 32]),
        |c| c.signature[0] ^= 1,
        |c| c.signature.truncate(31),
        |c| c.signature.clear(),
    ] {
        let mut m = cp.clone();
        mutate(&mut m);
        assert_eq!(
            m.verify_signature("tenant-a", &key),
            Err(AuditError::BadSignature)
        );
    }
    // Signer construction and empty-chain checkpoint.
    assert_eq!(
        HmacSha256Signer::new("", &[1; 32]).unwrap_err(),
        AuditError::Signing("empty key id")
    );
    assert_eq!(
        HmacSha256Signer::new("k", &[1; 15]).unwrap_err(),
        AuditError::Signing("key too short")
    );
    let mut empty = InMemoryAuditStore::new();
    assert_eq!(
        log.checkpoint(&mut empty, &key, 1),
        Err(AuditError::NothingToCheckpoint { seq: 0 })
    );
    let dbg = format!("{key:?}");
    assert!(dbg.contains("k1") && !dbg.contains("48"), "{dbg}");
    assert_eq!((key.algorithm(), key.key_id()), ("hmac-sha256", "k1"));
}

/// A stand-in for an asymmetric scheme: shows the traits are enough to plug another signer in.
struct ToySigner;

impl CheckpointSigner for ToySigner {
    fn algorithm(&self) -> &str {
        "toy"
    }
    fn key_id(&self) -> &str {
        "pub-1"
    }
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, AuditError> {
        Ok(message.iter().rev().take(8).copied().collect())
    }
}

impl CheckpointVerifier for ToySigner {
    fn verify(&self, alg: &str, key: &str, msg: &[u8], sig: &[u8]) -> Result<(), AuditError> {
        if (alg, key) != ("toy", "pub-1") {
            return Err(AuditError::UnknownKey);
        }
        if self.sign(msg)? == sig {
            Ok(())
        } else {
            Err(AuditError::BadSignature)
        }
    }
}

struct FailingSigner;

impl CheckpointSigner for FailingSigner {
    fn algorithm(&self) -> &str {
        "none"
    }
    fn key_id(&self) -> &str {
        "none"
    }
    fn sign(&self, _: &[u8]) -> Result<Vec<u8>, AuditError> {
        Err(AuditError::Signing("hsm offline"))
    }
}

#[test]
fn anomalous_pluggable_and_failing_signers() {
    let (log, mut store) = chain(4);
    let cp = log.checkpoint(&mut store, &ToySigner, 1).unwrap();
    assert_eq!(cp.algorithm, "toy");
    assert!(verify_store(&log, &store, &ToySigner).unwrap().is_intact());
    assert_eq!(
        log.checkpoint(&mut store, &FailingSigner, 1),
        Err(AuditError::Signing("hsm offline"))
    );
    let every = log.with_checkpoint_every(NonZeroU64::new(1).unwrap());
    assert!(every
        .append_checkpointed(&mut store, draft(5), &FailingSigner)
        .is_err());
    assert_eq!(store.checkpoints().unwrap().len(), 1);
}

// ---- tamper detection and localisation ----

#[test]
fn tamper_modified_entry_is_localised() {
    let (log, mut store) = chain(10);
    store.entries_mut()[4].details = r#"{"n":999}"#.into();
    let v = verify_entries(log.genesis(), store.entries());
    assert_eq!(finding(&v), (5, TamperKind::Modified, 4));
    assert!(!v.is_intact());
}

#[test]
fn tamper_modified_and_rehashed_entry_breaks_next_link() {
    let (log, mut store) = chain(10);
    let e = &mut store.entries_mut()[4];
    e.action = "record.delete".into();
    e.hash = e.compute_hash();
    assert_eq!(
        finding(&verify_entries(log.genesis(), store.entries())),
        (6, TamperKind::BrokenLink, 5)
    );
}

#[test]
fn tamper_removed_entries_are_localised() {
    let (log, mut store) = chain(10);
    store.entries_mut().drain(3..6); // seq 4,5,6
    assert_eq!(
        finding(&verify_entries(log.genesis(), store.entries())),
        (4, TamperKind::Removed { first: 4, last: 6 }, 3)
    );
    let (log, mut store) = chain(10);
    store.entries_mut().remove(0);
    assert_eq!(
        finding(&verify_store(&log, &store, &signer()).unwrap()),
        (1, TamperKind::Removed { first: 1, last: 1 }, 0)
    );
}

#[test]
fn tamper_reordered_entries_are_localised() {
    let (log, mut store) = chain(10);
    store.entries_mut().swap(2, 7);
    assert_eq!(
        finding(&verify_entries(log.genesis(), store.entries())),
        (3, TamperKind::Reordered { found_seq: 8 }, 2)
    );
    // An entry from before the verified range shows up inside it.
    let (_, es) = entries(10);
    let mut tail = es[5..].to_vec();
    tail.insert(2, es[1].clone());
    assert_eq!(
        finding(&verify_entries(es[4].head(), &tail)),
        (8, TamperKind::Reordered { found_seq: 2 }, 7)
    );
}

#[test]
fn tamper_duplicate_and_fork_are_distinguished() {
    let (log, es) = entries(6);
    let mut dup = es.clone();
    dup.insert(4, es[2].clone());
    assert_eq!(
        finding(&verify_entries(log.genesis(), &dup)),
        (5, TamperKind::Duplicated { found_seq: 3 }, 4)
    );
    // A second, different entry for seq 3 (prepared on the same predecessor).
    let rival = log.prepare(&es[1].head(), draft(777)).unwrap();
    assert_eq!(rival.seq, 3);
    let mut forked = es.clone();
    forked.insert(3, rival.clone());
    assert_eq!(
        finding(&verify_entries(log.genesis(), &forked)),
        (3, TamperKind::Forked, 3)
    );
    // Two copies of the chain that diverge.
    let mut other = es[..2].to_vec();
    other.push(rival.clone());
    other.push(log.prepare(&rival.head(), draft(778)).unwrap());
    assert_eq!(find_fork(&es, &other), Some(3));
    assert_eq!(find_fork(&es, &es[..4]), None);
    assert_eq!(find_fork(&es, &es), None);
    assert!(
        verify_entries(log.genesis(), &other).is_intact(),
        "each branch is valid alone"
    );
}

#[test]
fn tamper_rewritten_suffix_is_caught_by_checkpoints() {
    let log = log().with_checkpoint_every(NonZeroU64::new(10).unwrap());
    let key = signer();
    let mut store = InMemoryAuditStore::new();
    for i in 1..=30 {
        log.append_checkpointed(&mut store, draft(i), &key).unwrap();
    }
    // Rewrite entry 14 and consistently re-hash everything after it.
    let mut head = store.entries()[12].head();
    for i in 13..30 {
        let e = &mut store.entries_mut()[i];
        if i == 13 {
            e.details = r#"{"n":"forged"}"#.into();
        }
        e.prev_hash = head.hash;
        e.hash = e.compute_hash();
        head = e.head();
    }
    assert!(
        verify_entries(log.genesis(), store.entries()).is_intact(),
        "chain alone cannot see it"
    );
    let v = verify_store(&log, &store, &key).unwrap();
    // The forgery lies in (10, 20].
    assert_eq!(finding(&v), (20, TamperKind::CheckpointMismatch, 10));

    // The verdict does not depend on the order the store returns its checkpoints in: an
    // attacker who shuffles them cannot blur where the forgery is.
    store.checkpoints_mut().reverse();
    let v = verify_store(&log, &store, &key).unwrap();
    assert_eq!(finding(&v), (20, TamperKind::CheckpointMismatch, 10));
}

#[test]
fn tamper_truncation_and_forged_checkpoint() {
    let log = log().with_checkpoint_every(NonZeroU64::new(5).unwrap());
    let key = signer();
    let mut store = InMemoryAuditStore::new();
    for i in 1..=12 {
        log.append_checkpointed(&mut store, draft(i), &key).unwrap();
    }
    let mut cut = store.clone();
    cut.entries_mut().truncate(8);
    assert_eq!(
        finding(&verify_store(&log, &cut, &key).unwrap()),
        (9, TamperKind::Truncated { stored_through: 8 }, 8)
    );
    let mut forged = store.clone();
    forged.checkpoints_mut()[1].signature[3] ^= 0x10;
    assert_eq!(
        finding(&verify_store(&log, &forged, &key).unwrap()),
        (10, TamperKind::BadCheckpointSignature, 5)
    );
    // Duplicate checkpoints for one seq are both checked.
    let mut twice = store;
    let cp = twice.checkpoints().unwrap()[0].clone();
    twice.put_checkpoint(cp).unwrap();
    assert!(verify_store(&log, &twice, &key).unwrap().is_intact());
}

#[test]
fn tamper_streaming_verifier_reports_raw_findings() {
    let (log, es) = entries(4);
    let mut v = ChainVerifier::new(log.genesis());
    v.push(&es[0]).unwrap();
    let f = v.push(&es[2]).unwrap_err();
    assert_eq!(
        (f.seq, f.kind),
        (2, TamperKind::OutOfSequence { found_seq: 3 })
    );
    assert_eq!(
        (v.head(), v.entries()),
        (es[0].head(), 1),
        "verifier does not advance on a finding"
    );
    v.push(&es[1]).unwrap();
    // At the end of the sequence space nothing can follow.
    let mut end = ChainVerifier::new(ChainHead {
        seq: u64::MAX,
        hash: es[0].hash,
    });
    assert!(matches!(
        end.push(&es[0]).unwrap_err().kind,
        TamperKind::OutOfSequence { .. }
    ));
}

/// Fails on the n-th scan, to exercise error propagation.
struct FlakyStore {
    inner: InMemoryAuditStore,
    fail_from_offset: usize,
}

impl AuditStore for FlakyStore {
    fn head(&self) -> Result<Option<ChainHead>, AuditError> {
        Err(AuditError::Store("head unavailable".into()))
    }
    fn append(&mut self, entry: AuditEntry) -> Result<(), AuditError> {
        self.inner.append(entry)
    }
    fn get(&self, seq: u64) -> Result<Option<AuditEntry>, AuditError> {
        self.inner.get(seq)
    }
    fn scan(&self, offset: usize, limit: usize) -> Result<Vec<AuditEntry>, AuditError> {
        if offset >= self.fail_from_offset {
            return Err(AuditError::Store("disk error".into()));
        }
        self.inner.scan(offset, limit)
    }
    fn put_checkpoint(&mut self, checkpoint: Checkpoint) -> Result<(), AuditError> {
        self.inner.put_checkpoint(checkpoint)
    }
    fn checkpoints(&self) -> Result<Vec<Checkpoint>, AuditError> {
        self.inner.checkpoints()
    }
}

#[test]
fn anomalous_store_failures_propagate() {
    let (log, inner) = chain(6);
    let mut flaky = FlakyStore {
        inner: inner.clone(),
        fail_from_offset: 6,
    };
    assert_eq!(
        verify_store(&log, &flaky, &signer()),
        Err(AuditError::Store("disk error".into()))
    );
    assert_eq!(
        log.append(&mut flaky, draft(7)),
        Err(AuditError::Store("head unavailable".into()))
    );
    assert!(log.checkpoint(&mut flaky, &signer(), 1).is_err());
    // Failure while looking ahead to classify a gap.
    let mut gapped = inner.clone();
    gapped.entries_mut().remove(1);
    let flaky = FlakyStore {
        inner: gapped,
        fail_from_offset: 2,
    };
    assert!(verify_store(&log, &flaky, &signer()).is_err());
    // Failure while looking back to classify a duplicate.
    let mut dup = inner;
    let first = dup.entries()[0].clone();
    dup.entries_mut().insert(3, first);
    struct BackFail(InMemoryAuditStore);
    impl AuditStore for BackFail {
        fn head(&self) -> Result<Option<ChainHead>, AuditError> {
            self.0.head()
        }
        fn append(&mut self, e: AuditEntry) -> Result<(), AuditError> {
            self.0.append(e)
        }
        fn get(&self, seq: u64) -> Result<Option<AuditEntry>, AuditError> {
            self.0.get(seq)
        }
        fn scan(&self, offset: usize, limit: usize) -> Result<Vec<AuditEntry>, AuditError> {
            if limit == 1 {
                return Err(AuditError::Store("lookup failed".into()));
            }
            self.0.scan(offset, limit)
        }
        fn put_checkpoint(&mut self, c: Checkpoint) -> Result<(), AuditError> {
            self.0.put_checkpoint(c)
        }
        fn checkpoints(&self) -> Result<Vec<Checkpoint>, AuditError> {
            self.0.checkpoints()
        }
    }
    assert_eq!(
        verify_store(&log, &BackFail(dup), &signer()),
        Err(AuditError::Store("lookup failed".into()))
    );
}

// ---- store contract, concurrency (deterministic interleavings) ----

#[test]
fn invalid_store_refuses_entries_that_do_not_extend_the_head() {
    let (log, mut store) = chain(3);
    let es = store.entries().to_vec();
    // Stale writer: prepared on an old head.
    let stale = log.prepare(&es[1].head(), draft(50)).unwrap();
    assert_eq!(
        store.append(stale),
        Err(AuditError::AppendConflict {
            expected_seq: 4,
            entry_seq: 3
        })
    );
    // Right seq, wrong predecessor (prepared on a fork).
    let foreign = log
        .prepare(
            &ChainHead {
                seq: 3,
                hash: es[0].hash,
            },
            draft(51),
        )
        .unwrap();
    assert_eq!(
        store.append(foreign),
        Err(AuditError::BrokenLink { seq: 4 })
    );
    let mut empty = InMemoryAuditStore::new();
    assert!(matches!(
        empty.append(es[1].clone()),
        Err(AuditError::AppendConflict { .. })
    ));
    assert_eq!(store.entries().len(), 3);
    // Sequence space exhausted.
    let top = ChainHead {
        seq: u64::MAX,
        hash: es[0].hash,
    };
    assert_eq!(
        log.prepare(&top, draft(1)),
        Err(AuditError::SequenceOverflow)
    );
    let mut full = InMemoryAuditStore::new();
    full.entries_mut().push(AuditEntry {
        seq: u64::MAX,
        ..es[0].clone()
    });
    assert_eq!(
        full.append(es[0].clone()),
        Err(AuditError::SequenceOverflow)
    );
    // get() still answers on a store whose positions were disturbed.
    let mut odd = store.clone();
    odd.entries_mut().swap(0, 2);
    assert_eq!(odd.get(1).unwrap().unwrap().seq, 1);
}

struct Writer {
    queue: Vec<AuditDraft>,
    prepared: Option<AuditEntry>,
    conflicts: u32,
}

fn simulate_concurrent_appends(seed: u64, writers: usize, per_writer: u64) -> u32 {
    let log = log();
    let mut store = InMemoryAuditStore::new();
    let mut rng = Rng(seed);
    let mut ws: Vec<Writer> = (0..writers)
        .map(|w| Writer {
            queue: (0..per_writer)
                .rev()
                .map(|i| {
                    AuditDraft::direct(
                        user(&format!("w{w}")),
                        "op",
                        ObjectRef::new("r", format!("{i}")),
                        0,
                    )
                    .with_details(json!({"writer": w, "i": i}))
                })
                .collect(),
            prepared: None,
            conflicts: 0,
        })
        .collect();
    while ws
        .iter()
        .any(|w| w.prepared.is_some() || !w.queue.is_empty())
    {
        let idx = usize::try_from(rng.below(writers as u64)).unwrap();
        let w = &mut ws[idx];
        match w.prepared.take() {
            // Step A: read the head and prepare (other writers may run before step B).
            None => {
                if let Some(d) = w.queue.last().cloned() {
                    w.prepared = Some(log.prepare(&log.head(&store).unwrap(), d).unwrap());
                }
            }
            // Step B: compare-and-set append.
            Some(entry) => match store.append(entry) {
                Ok(()) => {
                    w.queue.pop();
                }
                Err(AuditError::AppendConflict { .. }) => w.conflicts += 1,
                Err(other) => panic!("seed={seed}: unexpected {other:?}"),
            },
        }
    }
    let verdict = verify_entries(log.genesis(), store.entries());
    assert!(verdict.is_intact(), "seed={seed}: {verdict:?}");
    assert_eq!(
        store.entries().len() as u64,
        writers as u64 * per_writer,
        "seed={seed}"
    );
    // Nothing lost, nothing applied twice, per-writer order preserved.
    for w in 0..writers {
        let mine: Vec<u64> = store
            .entries()
            .iter()
            .filter(|e| e.actor.id == format!("w{w}"))
            .map(|e| {
                serde_json::from_str::<Value>(&e.details).unwrap()["i"]
                    .as_u64()
                    .unwrap()
            })
            .collect();
        assert_eq!(
            mine,
            (0..per_writer).collect::<Vec<_>>(),
            "seed={seed} writer={w}"
        );
    }
    ws.iter().map(|w| w.conflicts).sum()
}

#[test]
fn concurrency_interleaved_writers_keep_the_chain_linear() {
    let mut conflicts = 0;
    for seed in 0..200 {
        conflicts += simulate_concurrent_appends(seed, 1 + usize::try_from(seed % 5).unwrap(), 8);
    }
    assert!(conflicts > 0, "the simulation must actually produce races");
}

#[test]
fn concurrency_two_writers_same_head_exactly_one_wins() {
    let (log, mut store) = chain(2);
    let head = log.head(&store).unwrap();
    let a = log.prepare(&head, draft(100)).unwrap();
    let b = log.prepare(&head, draft(200)).unwrap();
    assert_eq!(store.append(a), Ok(()));
    assert!(matches!(
        store.append(b),
        Err(AuditError::AppendConflict {
            expected_seq: 4,
            entry_seq: 3
        })
    ));
    let retry = log.append(&mut store, draft(200)).unwrap();
    assert_eq!(retry.seq, 4);
    assert!(verify_entries(log.genesis(), store.entries()).is_intact());
}

// ---- edge: field limits, large chain ----

#[test]
fn invalid_fields_are_rejected() {
    let log = log();
    let head = log.genesis();
    let long = "x".repeat(MAX_ID_BYTES + 1);
    let base = || draft(1);
    let cases: Vec<(AuditDraft, &str, &str)> = vec![
        (
            AuditDraft {
                actor: user(""),
                ..base()
            },
            "actor.id",
            "empty",
        ),
        (
            AuditDraft {
                actor: user(&long),
                ..base()
            },
            "actor.id",
            "too long",
        ),
        (
            AuditDraft {
                initiator: user(""),
                ..base()
            },
            "initiator.id",
            "empty",
        ),
        (
            AuditDraft {
                action: String::new(),
                ..base()
            },
            "action",
            "empty",
        ),
        (
            AuditDraft {
                action: "a".repeat(MAX_ACTION_BYTES + 1),
                ..base()
            },
            "action",
            "too long",
        ),
        (
            AuditDraft {
                object: ObjectRef::new("", "1"),
                ..base()
            },
            "object.kind",
            "empty",
        ),
        (
            AuditDraft {
                object: ObjectRef::new("r", long.clone()),
                ..base()
            },
            "object.id",
            "too long",
        ),
        (
            base().with_details(json!({"blob": "y".repeat(MAX_DETAILS_BYTES)})),
            "details",
            "too long",
        ),
    ];
    for (d, field, problem) in cases {
        assert_eq!(
            log.prepare(&head, d),
            Err(AuditError::InvalidField { field, problem })
        );
    }
    assert!(log
        .prepare(&head, base().with_details(json!({"x": 1.5})))
        .is_err());
    assert!(AuditLog::new("").is_err());
    assert!(AuditLog::new(long).is_err());
    // Limits are inclusive.
    let ok = AuditDraft {
        actor: user(&"x".repeat(MAX_ID_BYTES)),
        ..base()
    };
    assert!(log.prepare(&head, ok).is_ok());
}

#[test]
fn edge_large_chain_100k_entries() {
    let log = log().with_checkpoint_every(NonZeroU64::new(10_000).unwrap());
    let key = signer();
    let mut store = InMemoryAuditStore::new();
    let start = std::time::Instant::now();
    for i in 1..=100_000u64 {
        log.append_checkpointed(&mut store, draft(i), &key).unwrap();
    }
    let appended = start.elapsed();
    let verdict = verify_store(&log, &store, &key).unwrap();
    let verified = start.elapsed() - appended;
    println!("100k entries: append {appended:?}, verify {verified:?}");
    assert_eq!(
        verdict,
        ChainVerdict::Intact {
            head: store.head().unwrap().unwrap(),
            entries: 100_000
        }
    );
    assert_eq!(store.checkpoints().unwrap().len(), 10);
    assert_eq!(store.get(77_777).unwrap().unwrap().seq, 77_777);
    // Performance sanity only (generous bound, debug build included).
    assert!(
        start.elapsed().as_secs() < 120,
        "too slow: {:?}",
        start.elapsed()
    );
    // Tampering deep inside is still localised exactly, across page boundaries.
    store.entries_mut()[65_432].at_ms += 1;
    assert_eq!(
        finding(&verify_store(&log, &store, &key).unwrap()),
        (65_433, TamperKind::Modified, 65_432)
    );
    store.entries_mut().remove(65_432);
    assert_eq!(
        finding(&verify_store(&log, &store, &key).unwrap()),
        (
            65_433,
            TamperKind::Removed {
                first: 65_433,
                last: 65_433
            },
            65_432
        )
    );
    let moved = store.entries_mut().remove(1_000);
    store.entries_mut().push(moved);
    assert_eq!(
        finding(&verify_store(&log, &store, &key).unwrap()),
        (1_001, TamperKind::Reordered { found_seq: 1_002 }, 1_000)
    );
}

// ---- canonical JSON ----

#[test]
fn canonical_json_rules() {
    let v = json!({"b": [1, -2, true, false, null], "a": {"z": "é", "y": "\"\\\u{8}\t\n\u{c}\r\u{1}\u{1f}"}, "": 0});
    let text = canonical_json(&v).unwrap();
    assert_eq!(
        text,
        r#"{"":0,"a":{"y":"\"\\\b\t\n\f\r\u0001\u001f","z":"é"},"b":[1,-2,true,false,null]}"#
    );
    assert!(is_canonical(&text));
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), v);
    assert_eq!(
        canonical_json(&json!(u64::MAX)).unwrap(),
        "18446744073709551615"
    );
    assert_eq!(
        canonical_json(&json!(i64::MIN)).unwrap(),
        "-9223372036854775808"
    );
    // Keys sort by UTF-8 bytes, not by UTF-16 units or locale.
    assert_eq!(
        canonical_json(&json!({"é": 1, "z": 2, "Z": 3, "\u{10000}": 4})).unwrap(),
        "{\"Z\":3,\"z\":2,\"é\":1,\"\u{10000}\":4}"
    );
    for bad in [
        r#"{"a": 1}"#,
        r#"{"b":1,"a":2}"#,
        "1.0",
        "1e3",
        "[1,]",
        r#"{"a":1,"a":1}"#,
        "\"\\u00e9\"",
    ] {
        assert!(!is_canonical(bad), "{bad}");
    }
    assert_eq!(
        canonical_json(&json!(0.5)),
        Err(AuditError::NotCanonical("non-integer number"))
    );
    let mut deep = json!(1);
    for _ in 0..MAX_DEPTH {
        deep = json!([deep]);
    }
    assert!(canonical_json(&deep).is_ok());
    assert_eq!(
        canonical_json(&json!({"k": deep})),
        Err(AuditError::NotCanonical("nested too deeply"))
    );
}

// ---- privacy mode ----

fn privacy() -> Privacy {
    Privacy::HashClassified {
        fields: ["email".to_owned(), "salary".to_owned()].into(),
        key: b"tenant-secret-key".to_vec(),
    }
}

#[test]
fn privacy_classified_values_are_replaced_by_digests() {
    let log = log().with_privacy(privacy());
    let mut store = InMemoryAuditStore::new();
    let details = json!({
        "status": "open",
        "email": "alice@example.com",
        "changes": [{"salary": {"old": 100, "new": 120}, "title": "Lead"}],
    });
    let e = log
        .append(&mut store, draft(1).with_details(details.clone()))
        .unwrap();
    assert!(!e.details.contains("alice@example.com") && !e.details.contains("120"));
    let stored: Value = serde_json::from_str(&e.details).unwrap();
    assert_eq!(stored["status"], "open");
    assert_eq!(stored["changes"][0]["title"], "Lead");
    let digest = value_digest(b"tenant-secret-key", "email", &json!("alice@example.com")).unwrap();
    assert!(digest.starts_with("hmac-sha256:") && digest.len() == 12 + 64);
    assert_eq!(stored["email"], json!({"$hash": digest}));
    assert!(stored["changes"][0]["salary"]["$hash"].is_string());
    // The key holder can confirm a value; other values, fields and keys give other digests.
    assert_ne!(
        digest,
        value_digest(b"tenant-secret-key", "email", &json!("bob@example.com")).unwrap()
    );
    assert_ne!(
        digest,
        value_digest(b"tenant-secret-key", "salary", &json!("alice@example.com")).unwrap()
    );
    assert_ne!(
        digest,
        value_digest(b"other-secret-key!", "email", &json!("alice@example.com")).unwrap()
    );
    // The chain over minimised entries verifies like any other.
    log.append(&mut store, draft(2)).unwrap();
    assert!(verify_entries(log.genesis(), store.entries()).is_intact());
    // Full mode keeps values.
    let full = common::log()
        .prepare(&common::log().genesis(), draft(1).with_details(details))
        .unwrap();
    assert!(full.details.contains("alice@example.com"));
}

#[test]
fn privacy_edge_and_invalid() {
    let p = privacy();
    assert_eq!(p.apply(&json!("scalar")).unwrap(), json!("scalar"));
    assert_eq!(
        p.apply(&json!([1, null, true])).unwrap(),
        json!([1, null, true])
    );
    assert!(
        p.apply(&json!({"email": 1.5})).is_err(),
        "classified float cannot be digested"
    );
    let mut deep = json!({"email": "x"});
    for _ in 0..=MAX_DEPTH {
        deep = json!([deep]);
    }
    assert_eq!(
        p.apply(&deep),
        Err(AuditError::NotCanonical("nested too deeply"))
    );
    assert_eq!(Privacy::Full.apply(&deep).unwrap(), deep);
    let dbg = format!("{p:?} {:?}", Privacy::Full);
    assert!(
        dbg.contains("email") && dbg.contains("Full") && !dbg.contains("116"),
        "{dbg}"
    );
}

#[test]
fn edge_checkpoint_at_the_anchor_and_unknown_keys() {
    let (log, store) = chain(6);
    let key = signer();
    let es = store.entries();
    // A checkpoint exactly at the anchor is compared with the anchor itself.
    let anchor = es[2].head();
    let good = Checkpoint::sign(log.chain_id(), anchor, 0, &key).unwrap();
    let verdict = verify_against_checkpoint(&log, anchor, &es[3..], &good, &key).unwrap();
    assert!(verdict.is_intact());
    assert_eq!(verdict.finding(), None);

    let wrong_head = ChainHead {
        seq: anchor.seq,
        hash: es[0].hash,
    };
    let bad = Checkpoint::sign(log.chain_id(), wrong_head, 0, &key).unwrap();
    let verdict = verify_against_checkpoint(&log, anchor, &es[3..], &bad, &key).unwrap();
    assert_eq!(finding(&verdict), (3, TamperKind::CheckpointMismatch, 3));

    // A verifier that does not know the signing key says so instead of guessing a verdict.
    let stranger = HmacSha256Signer::new("other-key", &[7u8; 32]).unwrap();
    assert_eq!(
        verify_against_checkpoint(&log, anchor, &es[3..], &good, &stranger),
        Err(AuditError::UnknownKey)
    );
}

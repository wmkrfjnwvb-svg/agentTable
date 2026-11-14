//! Chain verification and tamper localisation.
//!
//! What each kind of tampering looks like to the verifier (entries are read in storage order, starting
//! from a trusted anchor — the genesis head for a full chain):
//!
//! | Tampering | Finding |
//! |---|---|
//! | A field of entry `k` changed, hash left alone | [`TamperKind::Modified`] at `k` |
//! | Entry `k` changed and its hash recomputed | [`TamperKind::BrokenLink`] at `k + 1` (the change is in `k`) |
//! | Entries `k..=m` deleted | [`TamperKind::Removed`] `{ first: k, last: m }` |
//! | Entries swapped / moved | [`TamperKind::Reordered`] at the first displaced position |
//! | An entry stored twice | [`TamperKind::Duplicated`] |
//! | Two different entries with the same `seq` | [`TamperKind::Forked`] at that `seq` |
//! | Entry `k` and everything after it rewritten consistently | [`TamperKind::CheckpointMismatch`] at the first checkpoint ≥ `k`; the rewrite lies in `(last_good.seq, seq]` |
//! | Tail deleted | [`TamperKind::Truncated`] if a checkpoint covers the deleted part |
//! | Checkpoint forged or altered | [`TamperKind::BadCheckpointSignature`] |
//!
//! Limits, stated plainly: a hash chain alone cannot reveal that its *unsigned tail* was consistently
//! rewritten or cut off — only a checkpoint (or an externally remembered head) can. An attacker who can
//! also delete checkpoints from the store is only caught by checkpoints exported elsewhere.
//!
//! Verification stops at the first finding; `last_good` tells how far the chain is trustworthy.

use crate::chain::{AuditEntry, AuditLog, AuditStore, ChainHead};
use crate::checkpoint::{Checkpoint, CheckpointVerifier};
use crate::error::AuditError;

/// Number of entries fetched per page by [`verify_store`].
pub const VERIFY_PAGE: usize = 1024;

/// How the chain was found to be damaged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TamperKind {
    /// The entry's content does not match its stored hash.
    Modified,
    /// The entry is self-consistent but does not link to its predecessor: the predecessor was rewritten
    /// or the chain was spliced here.
    BrokenLink,
    /// Entries `first..=last` are missing.
    Removed {
        /// First missing sequence number.
        first: u64,
        /// Last missing sequence number.
        last: u64,
    },
    /// An entry sits at the wrong position; the expected entry exists elsewhere.
    Reordered {
        /// Sequence number found at the position.
        found_seq: u64,
    },
    /// The same entry is stored more than once.
    Duplicated {
        /// Sequence number of the repeated entry.
        found_seq: u64,
    },
    /// Two different entries claim the same sequence number.
    Forked,
    /// The stored entry at a checkpoint's `seq` has a different hash than the checkpoint attests:
    /// history was rewritten after the checkpoint was signed.
    CheckpointMismatch,
    /// The chain ends before a checkpoint's `seq`.
    Truncated {
        /// Last sequence number actually present.
        stored_through: u64,
    },
    /// A checkpoint's signature is invalid.
    BadCheckpointSignature,
    /// Reported only by the streaming [`ChainVerifier`]: the next entry has an unexpected sequence
    /// number. [`verify_entries`] and [`verify_store`] refine this into `Removed`, `Reordered`,
    /// `Duplicated` or `Forked`.
    OutOfSequence {
        /// Sequence number found.
        found_seq: u64,
    },
}

/// Where and how the chain is damaged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TamperFinding {
    /// The first sequence number that cannot be trusted.
    pub seq: u64,
    /// What is wrong.
    pub kind: TamperKind,
    /// The last position known to be good (for checkpoint findings: the last confirmed checkpoint, or
    /// the anchor).
    pub last_good: ChainHead,
}

/// Result of a verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainVerdict {
    /// Every entry verified.
    Intact {
        /// Position of the last verified entry (the anchor if there were none).
        head: ChainHead,
        /// Number of entries verified.
        entries: u64,
    },
    /// Tampering or corruption was found.
    Tampered(TamperFinding),
}

impl ChainVerdict {
    /// Whether the chain verified.
    #[must_use]
    pub const fn is_intact(&self) -> bool {
        match self {
            Self::Intact { .. } => true,
            Self::Tampered(_) => false,
        }
    }

    /// The finding, if the chain is damaged.
    #[must_use]
    pub const fn finding(&self) -> Option<&TamperFinding> {
        match self {
            Self::Intact { .. } => None,
            Self::Tampered(finding) => Some(finding),
        }
    }
}

/// Streaming verifier: feed entries one by one in storage order. Constant memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainVerifier {
    head: ChainHead,
    entries: u64,
}

impl ChainVerifier {
    /// Starts after the trusted position `anchor` (use [`AuditLog::genesis`] for a full chain).
    #[must_use]
    pub const fn new(anchor: ChainHead) -> Self {
        Self {
            head: anchor,
            entries: 0,
        }
    }

    /// The last verified position.
    #[must_use]
    pub const fn head(&self) -> ChainHead {
        self.head
    }

    /// Number of entries verified so far.
    #[must_use]
    pub const fn entries(&self) -> u64 {
        self.entries
    }

    /// Verifies the next entry. On a finding the verifier does not advance.
    pub fn push(&mut self, entry: &AuditEntry) -> Result<(), TamperFinding> {
        let finding = |seq, kind| TamperFinding {
            seq,
            kind,
            last_good: self.head,
        };
        let Some(expected) = self.head.seq.checked_add(1) else {
            return Err(finding(
                entry.seq,
                TamperKind::OutOfSequence {
                    found_seq: entry.seq,
                },
            ));
        };
        if entry.seq != expected {
            return Err(finding(
                expected,
                TamperKind::OutOfSequence {
                    found_seq: entry.seq,
                },
            ));
        }
        if !entry.is_self_consistent() {
            return Err(finding(expected, TamperKind::Modified));
        }
        if entry.prev_hash != self.head.hash {
            return Err(finding(expected, TamperKind::BrokenLink));
        }
        self.head = entry.head();
        self.entries = self.entries.saturating_add(1);
        Ok(())
    }
}

/// Verifies `entries` (in storage order) as the continuation of `anchor`.
///
/// For a full chain pass [`AuditLog::genesis`] as the anchor.
#[must_use]
pub fn verify_entries(anchor: ChainHead, entries: &[AuditEntry]) -> ChainVerdict {
    let mut fetch = slice_fetch(entries);
    // The slice source cannot fail and no signature is checked, so no error is possible here.
    run("", anchor, &mut fetch, &[], None).unwrap_or(ChainVerdict::Intact {
        head: anchor,
        entries: 0,
    })
}

/// Verifies a range of entries following `anchor` against one signed checkpoint.
///
/// The range must reach the checkpoint's `seq`. Returns [`AuditError::CheckpointOutOfRange`] if the
/// checkpoint lies before the anchor and [`AuditError::UnknownKey`] if the verifier cannot judge the
/// signature; a wrong signature is a finding, not an error.
pub fn verify_against_checkpoint(
    log: &AuditLog,
    anchor: ChainHead,
    entries: &[AuditEntry],
    checkpoint: &Checkpoint,
    verifier: &dyn CheckpointVerifier,
) -> Result<ChainVerdict, AuditError> {
    if checkpoint.seq < anchor.seq {
        return Err(AuditError::CheckpointOutOfRange {
            checkpoint_seq: checkpoint.seq,
            anchor_seq: anchor.seq,
        });
    }
    let mut fetch = slice_fetch(entries);
    run(
        log.chain_id(),
        anchor,
        &mut fetch,
        core::slice::from_ref(checkpoint),
        Some(verifier),
    )
}

/// Verifies everything in `store`: the whole chain from genesis and every stored checkpoint.
pub fn verify_store<S: AuditStore + ?Sized>(
    log: &AuditLog,
    store: &S,
    verifier: &dyn CheckpointVerifier,
) -> Result<ChainVerdict, AuditError> {
    // Sorted here so that the verdict does not depend on the order a (possibly tampered) store
    // returns them in: a checkpoint moved out of order must not be skipped.
    let mut checkpoints = store.checkpoints()?;
    checkpoints.sort_by_key(|c| c.seq);
    let mut fetch = |offset: usize, limit: usize| store.scan(offset, limit);
    run(
        log.chain_id(),
        log.genesis(),
        &mut fetch,
        &checkpoints,
        Some(verifier),
    )
}

/// The first sequence number at which two copies of a chain disagree, or `None` if one is a prefix of
/// the other (or they are equal). Both slices must start at the same sequence number.
#[must_use]
pub fn find_fork(a: &[AuditEntry], b: &[AuditEntry]) -> Option<u64> {
    a.iter()
        .zip(b)
        .find(|(x, y)| x.seq != y.seq || x.hash != y.hash)
        .map(|(x, y)| x.seq.min(y.seq))
}

type Fetch<'a> = dyn FnMut(usize, usize) -> Result<Vec<AuditEntry>, AuditError> + 'a;

fn slice_fetch(
    entries: &[AuditEntry],
) -> impl FnMut(usize, usize) -> Result<Vec<AuditEntry>, AuditError> + '_ {
    |offset, limit| Ok(entries.iter().skip(offset).take(limit).cloned().collect())
}

fn run(
    chain_id: &str,
    anchor: ChainHead,
    fetch: &mut Fetch<'_>,
    checkpoints: &[Checkpoint],
    verifier: Option<&dyn CheckpointVerifier>,
) -> Result<ChainVerdict, AuditError> {
    let mut chain = ChainVerifier::new(anchor);
    let mut pending = checkpoints;
    let mut attested = anchor;
    let mut offset = 0usize;
    if let Some(f) = settle(
        chain_id,
        &mut pending,
        chain.head(),
        verifier,
        &mut attested,
    )? {
        return Ok(ChainVerdict::Tampered(f));
    }
    loop {
        let page = fetch(offset, VERIFY_PAGE)?;
        if page.is_empty() {
            break;
        }
        for entry in &page {
            if let Err(f) = chain.push(entry) {
                return Ok(ChainVerdict::Tampered(refine(
                    f, entry, offset, anchor, fetch,
                )?));
            }
            offset = offset.saturating_add(1);
            if let Some(f) = settle(
                chain_id,
                &mut pending,
                chain.head(),
                verifier,
                &mut attested,
            )? {
                return Ok(ChainVerdict::Tampered(f));
            }
        }
    }
    if !pending.is_empty() {
        // Every remaining checkpoint lies beyond the last stored entry.
        let stored_through = chain.head().seq;
        return Ok(ChainVerdict::Tampered(TamperFinding {
            seq: stored_through.saturating_add(1),
            kind: TamperKind::Truncated { stored_through },
            last_good: chain.head(),
        }));
    }
    Ok(ChainVerdict::Intact {
        head: chain.head(),
        entries: chain.entries(),
    })
}

/// Checks every pending checkpoint at or below `head.seq`; `pending` is ascending by `seq`.
fn settle(
    chain_id: &str,
    pending: &mut &[Checkpoint],
    head: ChainHead,
    verifier: Option<&dyn CheckpointVerifier>,
    attested: &mut ChainHead,
) -> Result<Option<TamperFinding>, AuditError> {
    while let Some((checkpoint, rest)) = pending.split_first() {
        if checkpoint.seq > head.seq {
            break;
        }
        *pending = rest;
        if checkpoint.seq < head.seq {
            // Before the verified range (or out of order in the store): cannot be compared.
            continue;
        }
        let finding = |kind| TamperFinding {
            seq: checkpoint.seq,
            kind,
            last_good: *attested,
        };
        if let Some(verifier) = verifier {
            match checkpoint.verify_signature(chain_id, verifier) {
                Ok(()) => {}
                Err(AuditError::BadSignature) => {
                    return Ok(Some(finding(TamperKind::BadCheckpointSignature)));
                }
                Err(other) => return Err(other),
            }
        }
        if checkpoint.hash != head.hash {
            return Ok(Some(finding(TamperKind::CheckpointMismatch)));
        }
        *attested = head;
    }
    Ok(None)
}

/// Turns an `OutOfSequence` finding into a precise one by looking at the rest of the storage.
fn refine(
    finding: TamperFinding,
    entry: &AuditEntry,
    position: usize,
    anchor: ChainHead,
    fetch: &mut Fetch<'_>,
) -> Result<TamperFinding, AuditError> {
    let found_seq = match finding.kind {
        TamperKind::OutOfSequence { found_seq } => found_seq,
        TamperKind::Modified
        | TamperKind::BrokenLink
        | TamperKind::Removed { .. }
        | TamperKind::Reordered { .. }
        | TamperKind::Duplicated { .. }
        | TamperKind::Forked
        | TamperKind::CheckpointMismatch
        | TamperKind::Truncated { .. }
        | TamperKind::BadCheckpointSignature => return Ok(finding),
    };
    let expected = finding.seq;
    if found_seq < expected {
        // Compare with the verified entry that already holds this sequence number.
        let earlier = found_seq
            .checked_sub(anchor.seq)
            .and_then(|d| d.checked_sub(1))
            .and_then(|i| usize::try_from(i).ok());
        let original = match earlier {
            Some(index) => fetch(index, 1)?.into_iter().next(),
            None => None,
        };
        return Ok(match original {
            Some(original) if original.hash == entry.hash => TamperFinding {
                kind: TamperKind::Duplicated { found_seq },
                ..finding
            },
            Some(_) => TamperFinding {
                seq: found_seq,
                kind: TamperKind::Forked,
                ..finding
            },
            None => TamperFinding {
                kind: TamperKind::Reordered { found_seq },
                ..finding
            },
        });
    }
    // found_seq > expected: is the expected entry stored later (reordered) or gone (removed)?
    let mut offset = position.saturating_add(1);
    loop {
        let page = fetch(offset, VERIFY_PAGE)?;
        if page.is_empty() {
            break;
        }
        if page.iter().any(|e| e.seq == expected) {
            return Ok(TamperFinding {
                kind: TamperKind::Reordered { found_seq },
                ..finding
            });
        }
        offset = offset.saturating_add(page.len());
    }
    Ok(TamperFinding {
        kind: TamperKind::Removed {
            first: expected,
            last: found_seq.saturating_sub(1),
        },
        ..finding
    })
}

//! Operation delivery tracking, in the style of instant-messaging delivery states (doc 02 B10).
//!
//! Every client operation carries a [`ClientOpId`] and moves through
//! `created → sent → received → validated → committed(seq) | rejected(reason)`, then, once committed,
//! `broadcast → applied(by n clients)`.
//!
//! [`step`] is the pure transition function. Events may be delivered more than once (at-least-once
//! transports, client retries); an event that is already reflected in the state yields
//! [`StepOutcome::Duplicate`] and changes nothing. An event that contradicts the state is an error and
//! also changes nothing, so an illegal state is unreachable.
//!
//! | event \ state     | created | sent | received | validated | committed | broadcast | applied | rejected |
//! |-------------------|---------|------|----------|-----------|-----------|-----------|---------|----------|
//! | `Send`            | →sent   | dup  | dup      | dup       | dup       | dup       | dup     | dup      |
//! | `Receive`         | ✗       | →received | dup | dup       | dup       | dup       | dup     | dup      |
//! | `Validate`        | ✗       | ✗    | →validated | dup     | dup       | dup       | dup     | ✗        |
//! | `Commit(seq)`     | ✗       | ✗    | ✗        | →committed | dup¹     | dup¹      | dup¹    | ✗        |
//! | `Reject(reason)`  | ✗       | ✗    | →rejected | →rejected | ✗        | ✗         | ✗       | dup²     |
//! | `Broadcast`       | ✗       | ✗    | ✗        | ✗         | →broadcast | dup      | dup     | ✗        |
//! | `ClientApplied(c)`| ✗       | ✗    | ✗        | ✗         | ✗         | →applied  | →applied / dup³ | ✗ |
//!
//! ¹ same `seq` only; a different `seq` is [`AuditError::CommitConflict`].
//! ² the first reason is kept. ³ duplicate when client `c` already acknowledged.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::AuditError;

/// Client-generated operation identifier (UUIDv7 as 128 bits). The idempotency key of an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ClientOpId(pub u128);

/// Identifier of a client connection/replica that acknowledges applied operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ClientId(pub u128);

/// Where an operation is on its way from the client to every subscriber.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DeliveryState {
    /// Created locally; in the client outbox.
    Created,
    /// Handed to the transport.
    Sent,
    /// Received by the server.
    Received,
    /// Passed validation (schema, permissions, rules).
    Validated,
    /// Committed with a per-base sequence number.
    Committed {
        /// The per-base sequence number.
        seq: u64,
    },
    /// Refused; the operation will never be committed.
    Rejected {
        /// Why it was refused.
        reason: String,
    },
    /// Committed and fanned out to subscribers.
    Broadcast {
        /// The per-base sequence number.
        seq: u64,
    },
    /// Committed, fanned out and acknowledged by at least one client.
    Applied {
        /// The per-base sequence number.
        seq: u64,
        /// Clients that acknowledged applying the operation.
        by: BTreeSet<ClientId>,
    },
}

impl DeliveryState {
    /// Stable lowercase name of the state.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Sent => "sent",
            Self::Received => "received",
            Self::Validated => "validated",
            Self::Committed { .. } => "committed",
            Self::Rejected { .. } => "rejected",
            Self::Broadcast { .. } => "broadcast",
            Self::Applied { .. } => "applied",
        }
    }

    /// Whether the outcome is decided (committed or rejected). The outbox may drop such an operation.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        match self {
            Self::Created | Self::Sent | Self::Received | Self::Validated => false,
            Self::Committed { .. }
            | Self::Rejected { .. }
            | Self::Broadcast { .. }
            | Self::Applied { .. } => true,
        }
    }

    /// The committed sequence number, if the operation was committed.
    #[must_use]
    pub const fn committed_seq(&self) -> Option<u64> {
        match self {
            Self::Committed { seq } | Self::Broadcast { seq } | Self::Applied { seq, .. } => {
                Some(*seq)
            }
            Self::Created
            | Self::Sent
            | Self::Received
            | Self::Validated
            | Self::Rejected { .. } => None,
        }
    }

    /// Number of clients that acknowledged applying the operation.
    #[must_use]
    pub fn applied_count(&self) -> usize {
        match self {
            Self::Applied { by, .. } => by.len(),
            Self::Created
            | Self::Sent
            | Self::Received
            | Self::Validated
            | Self::Committed { .. }
            | Self::Rejected { .. }
            | Self::Broadcast { .. } => 0,
        }
    }
}

/// Something that happened to an operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum DeliveryEvent {
    /// The client handed the operation to the transport (also every retry).
    Send,
    /// The server received it.
    Receive,
    /// The server validated it.
    Validate,
    /// The server committed it at `seq` (≥ 1).
    Commit {
        /// The per-base sequence number.
        seq: u64,
    },
    /// The server refused it.
    Reject {
        /// Why.
        reason: String,
    },
    /// The committed change was fanned out.
    Broadcast,
    /// A client acknowledged applying the committed change.
    ClientApplied {
        /// The acknowledging client.
        client: ClientId,
    },
}

impl DeliveryEvent {
    /// Stable lowercase name of the event.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Send => "send",
            Self::Receive => "receive",
            Self::Validate => "validate",
            Self::Commit { .. } => "commit",
            Self::Reject { .. } => "reject",
            Self::Broadcast => "broadcast",
            Self::ClientApplied { .. } => "client_applied",
        }
    }
}

/// Result of a legal [`step`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    /// The state changed to the contained value.
    Advanced(DeliveryState),
    /// The event was already reflected in the state (idempotent re-delivery); nothing changed.
    Duplicate,
}

/// The pure transition function. See the table in the module documentation.
pub fn step(state: &DeliveryState, event: &DeliveryEvent) -> Result<StepOutcome, AuditError> {
    use DeliveryEvent as E;
    use DeliveryState as S;
    let illegal = || AuditError::IllegalTransition {
        from: state.name(),
        event: event.name(),
    };
    let advanced = |s: S| Ok(StepOutcome::Advanced(s));
    if matches!(event, E::Commit { seq: 0 }) {
        return Err(AuditError::ZeroSequence);
    }
    match (event, state) {
        (E::Send, S::Created) => advanced(S::Sent),
        (
            E::Send,
            S::Sent
            | S::Received
            | S::Validated
            | S::Committed { .. }
            | S::Rejected { .. }
            | S::Broadcast { .. }
            | S::Applied { .. },
        ) => Ok(StepOutcome::Duplicate),

        (E::Receive, S::Created) => Err(illegal()),
        (E::Receive, S::Sent) => advanced(S::Received),
        (
            E::Receive,
            S::Received
            | S::Validated
            | S::Committed { .. }
            | S::Rejected { .. }
            | S::Broadcast { .. }
            | S::Applied { .. },
        ) => Ok(StepOutcome::Duplicate),

        (E::Validate, S::Received) => advanced(S::Validated),
        (
            E::Validate,
            S::Validated | S::Committed { .. } | S::Broadcast { .. } | S::Applied { .. },
        ) => Ok(StepOutcome::Duplicate),
        (E::Validate, S::Created | S::Sent | S::Rejected { .. }) => Err(illegal()),

        (E::Commit { seq }, S::Validated) => advanced(S::Committed { seq: *seq }),
        (
            E::Commit { seq },
            S::Committed { seq: existing }
            | S::Broadcast { seq: existing }
            | S::Applied { seq: existing, .. },
        ) => {
            if seq == existing {
                Ok(StepOutcome::Duplicate)
            } else {
                Err(AuditError::CommitConflict {
                    existing: *existing,
                    attempted: *seq,
                })
            }
        }
        (E::Commit { .. }, S::Created | S::Sent | S::Received | S::Rejected { .. }) => {
            Err(illegal())
        }

        (E::Reject { reason }, S::Received | S::Validated) => advanced(S::Rejected {
            reason: reason.clone(),
        }),
        (E::Reject { .. }, S::Rejected { .. }) => Ok(StepOutcome::Duplicate),
        (
            E::Reject { .. },
            S::Created | S::Sent | S::Committed { .. } | S::Broadcast { .. } | S::Applied { .. },
        ) => Err(illegal()),

        (E::Broadcast, S::Committed { seq }) => advanced(S::Broadcast { seq: *seq }),
        (E::Broadcast, S::Broadcast { .. } | S::Applied { .. }) => Ok(StepOutcome::Duplicate),
        (E::Broadcast, S::Created | S::Sent | S::Received | S::Validated | S::Rejected { .. }) => {
            Err(illegal())
        }

        (E::ClientApplied { client }, S::Broadcast { seq }) => advanced(S::Applied {
            seq: *seq,
            by: BTreeSet::from([*client]),
        }),
        (E::ClientApplied { client }, S::Applied { seq, by }) => {
            if by.contains(client) {
                Ok(StepOutcome::Duplicate)
            } else {
                let mut by = by.clone();
                by.insert(*client);
                advanced(S::Applied { seq: *seq, by })
            }
        }
        (
            E::ClientApplied { .. },
            S::Created
            | S::Sent
            | S::Received
            | S::Validated
            | S::Committed { .. }
            | S::Rejected { .. },
        ) => Err(illegal()),
    }
}

/// Delivery ledger: the state of every known operation, keyed by client op id.
///
/// Used both as the client outbox view and as the server's idempotency table. A failed call never
/// changes the ledger.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryLedger {
    ops: BTreeMap<ClientOpId, DeliveryState>,
    by_seq: BTreeMap<u64, ClientOpId>,
}

impl DeliveryLedger {
    /// An empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a new operation in `created`. Re-registering a known id is a duplicate and keeps its
    /// state.
    pub fn create(&mut self, op: ClientOpId) -> StepOutcome {
        if self.ops.contains_key(&op) {
            return StepOutcome::Duplicate;
        }
        self.ops.insert(op, DeliveryState::Created);
        StepOutcome::Advanced(DeliveryState::Created)
    }

    /// Server-side entry point: records that `op` arrived. An unknown id starts at `received`
    /// (the server never observes `created`/`sent`); a known id is an idempotent re-delivery.
    pub fn receive(&mut self, op: ClientOpId) -> Result<StepOutcome, AuditError> {
        if self.ops.contains_key(&op) {
            return self.apply(op, &DeliveryEvent::Receive);
        }
        self.ops.insert(op, DeliveryState::Received);
        Ok(StepOutcome::Advanced(DeliveryState::Received))
    }

    /// Applies an event to a known operation.
    pub fn apply(
        &mut self,
        op: ClientOpId,
        event: &DeliveryEvent,
    ) -> Result<StepOutcome, AuditError> {
        let state = self.ops.get(&op).ok_or(AuditError::UnknownOperation)?;
        let outcome = step(state, event)?;
        if let StepOutcome::Advanced(next) = &outcome {
            if let DeliveryEvent::Commit { seq } = event {
                if self.by_seq.get(seq).is_some_and(|holder| *holder != op) {
                    return Err(AuditError::SequenceReused { seq: *seq });
                }
                self.by_seq.insert(*seq, op);
            }
            self.ops.insert(op, next.clone());
        }
        Ok(outcome)
    }

    /// The state of an operation, if known.
    #[must_use]
    pub fn state(&self, op: ClientOpId) -> Option<&DeliveryState> {
        self.ops.get(&op)
    }

    /// The operation committed at `seq`, if any.
    #[must_use]
    pub fn op_at_seq(&self, seq: u64) -> Option<ClientOpId> {
        self.by_seq.get(&seq).copied()
    }

    /// Operations whose outcome is not yet decided (what the outbox must keep and retry).
    pub fn pending(&self) -> impl Iterator<Item = (ClientOpId, &DeliveryState)> {
        self.ops
            .iter()
            .filter(|(_, s)| !s.is_terminal())
            .map(|(id, s)| (*id, s))
    }

    /// Number of known operations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Whether the ledger knows no operations.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Drops a decided operation from the ledger. Returns `false` (and keeps it) if the operation is
    /// unknown or still pending. After this, a late re-delivery of the id is no longer recognised.
    pub fn forget_terminal(&mut self, op: ClientOpId) -> bool {
        let Some(state) = self.ops.get(&op) else {
            return false;
        };
        if !state.is_terminal() {
            return false;
        }
        if let Some(seq) = state.committed_seq() {
            self.by_seq.remove(&seq);
        }
        self.ops.remove(&op);
        true
    }
}

/// An inclusive range of sequence numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeqRange {
    /// First sequence number in the range.
    pub first: u64,
    /// Last sequence number in the range (≥ `first`).
    pub last: u64,
}

/// What [`SeqTracker::observe`] concluded about a sequence number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeqObservation {
    /// The number extended the contiguous prefix (possibly closing gaps).
    InOrder {
        /// Everything up to and including this number has now been seen.
        contiguous_through: u64,
    },
    /// The number was seen before; apply nothing.
    Duplicate,
    /// The number is ahead of the contiguous prefix; these ranges are missing and must be resynced.
    Gap {
        /// All currently missing ranges, ascending.
        missing: Vec<SeqRange>,
    },
}

/// Gap and duplicate detector over the per-base sequence numbers a client receives.
///
/// Sequences start at 1. The tracker stores the contiguous prefix as one number and everything received
/// ahead of it as merged ranges, so memory is proportional to the number of gaps.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeqTracker {
    contiguous: u64,
    /// Ranges received ahead of the prefix: start → inclusive end. Never adjacent or overlapping.
    ahead: BTreeMap<u64, u64>,
}

impl SeqTracker {
    /// A tracker that has seen nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A tracker resuming after everything up to `seq` was applied (for example from a snapshot).
    #[must_use]
    pub fn starting_after(seq: u64) -> Self {
        Self {
            contiguous: seq,
            ahead: BTreeMap::new(),
        }
    }

    /// Everything up to and including this number has been seen (0 = nothing).
    #[must_use]
    pub const fn contiguous_through(&self) -> u64 {
        self.contiguous
    }

    /// The highest sequence number seen so far (0 = nothing).
    #[must_use]
    pub fn highest_seen(&self) -> u64 {
        self.ahead
            .last_key_value()
            .map_or(self.contiguous, |(_, end)| *end)
    }

    /// Whether `seq` has been seen.
    #[must_use]
    pub fn has_seen(&self, seq: u64) -> bool {
        seq <= self.contiguous
            || self
                .ahead
                .range(..=seq)
                .next_back()
                .is_some_and(|(_, end)| *end >= seq)
    }

    /// Records a received sequence number.
    pub fn observe(&mut self, seq: u64) -> Result<SeqObservation, AuditError> {
        if seq == 0 {
            return Err(AuditError::ZeroSequence);
        }
        if self.has_seen(seq) {
            return Ok(SeqObservation::Duplicate);
        }
        // Merge with the range that ends right before `seq` and the one that starts right after.
        let mut start = seq;
        let mut end = seq;
        if let Some(before) = seq.checked_sub(1) {
            let joined = self
                .ahead
                .range(..=before)
                .next_back()
                .map(|(s, e)| (*s, *e));
            if let Some((s, e)) = joined {
                if e == before {
                    start = s;
                }
            }
        }
        if let Some(after) = seq.checked_add(1) {
            if let Some(e) = self.ahead.remove(&after) {
                end = e;
            }
        }
        self.ahead.insert(start, end);
        self.absorb_prefix();
        if seq <= self.contiguous {
            Ok(SeqObservation::InOrder {
                contiguous_through: self.contiguous,
            })
        } else {
            Ok(SeqObservation::Gap {
                missing: self.missing(),
            })
        }
    }

    /// Declares everything up to `seq` applied (after a resync from a snapshot). Never moves backwards.
    pub fn resync_through(&mut self, seq: u64) {
        if seq <= self.contiguous {
            return;
        }
        self.contiguous = seq;
        let keep = match seq.checked_add(1) {
            Some(next) => self.ahead.split_off(&next),
            None => BTreeMap::new(),
        };
        // A range straddling `seq` keeps its tail.
        let straddling = self
            .ahead
            .last_key_value()
            .map(|(_, e)| *e)
            .filter(|e| *e > seq);
        self.ahead = keep;
        if let Some(end) = straddling {
            self.contiguous = end;
        }
        self.absorb_prefix();
    }

    /// The ranges not yet seen below the highest seen number, ascending.
    #[must_use]
    pub fn missing(&self) -> Vec<SeqRange> {
        let mut out = Vec::with_capacity(self.ahead.len());
        let mut covered = self.contiguous;
        for (start, end) in &self.ahead {
            if let (Some(first), Some(last)) = (covered.checked_add(1), start.checked_sub(1)) {
                if first <= last {
                    out.push(SeqRange { first, last });
                }
            }
            covered = *end;
        }
        out
    }

    /// Moves ranges that touch the contiguous prefix into it.
    fn absorb_prefix(&mut self) {
        while let Some(next) = self.contiguous.checked_add(1) {
            match self.ahead.remove(&next) {
                Some(end) => self.contiguous = end,
                None => break,
            }
        }
    }
}

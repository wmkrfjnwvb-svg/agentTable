//! The crate error type. Errors are values; nothing in this crate panics.

use thiserror::Error;

/// Every error this crate can return.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AuditError {
    /// A `traceparent` header (or a trace/span id) failed strict validation.
    #[error("invalid traceparent: {0}")]
    InvalidTraceparent(&'static str),
    /// A hex-encoded hash had the wrong length or alphabet.
    #[error("invalid hash encoding")]
    InvalidHash,
    /// A delivery event is not legal in the operation's current state.
    #[error("illegal delivery transition from {from} on {event}")]
    IllegalTransition {
        /// Name of the state the operation was in.
        from: &'static str,
        /// Name of the offending event.
        event: &'static str,
    },
    /// An operation was reported committed twice with different sequence numbers.
    #[error("operation already committed at seq {existing}, conflicting seq {attempted}")]
    CommitConflict {
        /// The sequence number recorded first.
        existing: u64,
        /// The conflicting sequence number.
        attempted: u64,
    },
    /// Two different operations were reported committed at the same sequence number.
    #[error("sequence number {seq} is already used by another operation")]
    SequenceReused {
        /// The contested sequence number.
        seq: u64,
    },
    /// A delivery event referenced an operation the ledger has never seen.
    #[error("unknown client operation")]
    UnknownOperation,
    /// A sequence number of 0 was supplied.
    #[error("sequence number 0 is not valid (sequences start at 1)")]
    ZeroSequence,
    /// A JSON value cannot be canonicalised (float, or nested too deeply).
    #[error("value cannot be canonicalised: {0}")]
    NotCanonical(&'static str),
    /// A text field of an audit entry was empty or too long.
    #[error("invalid audit field `{field}`: {problem}")]
    InvalidField {
        /// The field name.
        field: &'static str,
        /// What is wrong with it.
        problem: &'static str,
    },
    /// The store head moved between prepare and append (optimistic concurrency failure). Retry.
    #[error("append conflict: expected to append seq {expected_seq}, entry has seq {entry_seq}")]
    AppendConflict {
        /// The sequence number the store would accept next.
        expected_seq: u64,
        /// The sequence number of the rejected entry.
        entry_seq: u64,
    },
    /// The entry does not link to the store head (wrong `prev_hash`) or its hash is not self-consistent.
    #[error("entry at seq {seq} does not link to the chain head")]
    BrokenLink {
        /// Sequence number of the rejected entry.
        seq: u64,
    },
    /// The chain reached the maximum sequence number.
    #[error("sequence number overflow")]
    SequenceOverflow,
    /// A checkpoint was requested for an empty chain or a sequence number that is not stored.
    #[error("nothing to checkpoint at seq {seq}")]
    NothingToCheckpoint {
        /// The requested sequence number (0 for an empty chain).
        seq: u64,
    },
    /// The signer could not produce a signature, or the key is unusable.
    #[error("signing failed: {0}")]
    Signing(&'static str),
    /// A checkpoint signature did not verify.
    #[error("checkpoint signature invalid")]
    BadSignature,
    /// A checkpoint names a key or algorithm the verifier does not handle.
    #[error("unknown signing key or algorithm")]
    UnknownKey,
    /// A checkpoint lies before the range being verified.
    #[error(
        "checkpoint at seq {checkpoint_seq} is before the verified range (anchor seq {anchor_seq})"
    )]
    CheckpointOutOfRange {
        /// Sequence number of the checkpoint.
        checkpoint_seq: u64,
        /// Sequence number of the trusted anchor.
        anchor_seq: u64,
    },
    /// The underlying store failed (I/O implementations map their errors here).
    #[error("audit store failure: {0}")]
    Store(String),
    /// A process contract is not well formed.
    #[error("invalid process contract: {0}")]
    InvalidContract(&'static str),
    /// Journal events were not in strictly increasing sequence order.
    #[error("journal out of order: seq {seq} after {after}")]
    JournalOutOfOrder {
        /// The offending sequence number.
        seq: u64,
        /// The previous sequence number in the batch or cursor.
        after: u64,
    },
    /// An exception event is not legal in the exception's current status.
    #[error("illegal exception transition from {from} on {event}")]
    IllegalExceptionTransition {
        /// Name of the current status.
        from: &'static str,
        /// Name of the offending event.
        event: &'static str,
    },
}

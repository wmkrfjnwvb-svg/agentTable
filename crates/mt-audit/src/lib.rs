//! Trace and audit primitives for the platform (doc 17 §3.2 D15, §5 C10; doc 03 §4.4; doc 02 B10).
//!
//! The crate is pure and sans-IO: no clocks, sockets, threads or randomness. Time and identifiers are
//! always inputs. It compiles for `wasm32-unknown-unknown`.
//!
//! Modules:
//! - [`trace`]: W3C `traceparent` parsing/formatting and child-span derivation.
//! - [`delivery`]: IM-style operation delivery state machine, idempotent ledger and sequence-gap tracker.
//! - [`canonical`]: canonical JSON used for audit details.
//! - [`chain`]: hash-chained audit log, the [`chain::AuditStore`] trait and an in-memory store.
//! - [`checkpoint`]: signed checkpoints (HMAC-SHA256 built in; signer/verifier traits for other schemes).
//! - [`verify`]: chain verification and tamper localisation.
//! - [`privacy`]: value-hash minimisation of classified fields in audit details.
//! - [`process`]: process contracts and the incremental completeness verifier.
//! - [`exception`]: exception descriptors and their status state machine.
#![forbid(unsafe_code)]

pub mod canonical;
pub mod chain;
pub mod checkpoint;
pub mod delivery;
pub mod error;
pub mod exception;
mod hex;
pub mod privacy;
pub mod process;
pub mod trace;
pub mod verify;

pub use canonical::canonical_json;
pub use chain::{
    AuditDraft, AuditEntry, AuditLog, AuditStore, ChainHead, Hash32, InMemoryAuditStore, ObjectRef,
    Principal, PrincipalKind,
};
pub use checkpoint::{Checkpoint, CheckpointSigner, CheckpointVerifier, HmacSha256Signer};
pub use delivery::{
    ClientId, ClientOpId, DeliveryEvent, DeliveryLedger, DeliveryState, SeqObservation, SeqRange,
    SeqTracker, StepOutcome,
};
pub use error::AuditError;
pub use exception::{ExceptionDescriptor, ExceptionEvent, ExceptionKind, ExceptionStatus};
pub use privacy::Privacy;
pub use process::{
    BrokenReason, CompletenessVerifier, FollowUpRule, InstanceReport, InstanceStatus, LateReason,
    ProcessContract, ProcessEvent,
};
pub use trace::{SpanId, TraceContext, TraceFlags, TraceId};
pub use verify::{
    find_fork, verify_against_checkpoint, verify_entries, verify_store, ChainVerdict,
    ChainVerifier, TamperFinding, TamperKind,
};

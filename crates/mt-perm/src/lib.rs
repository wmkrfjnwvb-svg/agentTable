//! Access control (product design doc 17: domain D12, capability C5, process P9).
//!
//! * [`capability`] — what can be allowed: built-in actions and a base's own capabilities.
//! * [`grant`] — who may do what, where, to which records and fields, under which
//!   condition, until when, given by whom and why; capability sets; separation-of-duties
//!   rules.
//! * [`policy`] — the engine: decisions with their explanation (the grant chain or the
//!   reasons for refusal), row filters for lists, field permissions, dual identity for AI.
//! * [`share`] — share links: anonymous, narrow, expiring access to one table or record.
//! * [`review`] — periodic access reviews.
//!
//! One engine serves the UI, the API, exports, search, notifications, dashboards and AI.
//! It is pure: no I/O and no clock, so the same code runs on the server (enforcement) and
//! in the browser (hints).
#![forbid(unsafe_code)]

pub mod capability;
pub mod error;
pub mod grant;
pub mod policy;
pub mod review;
pub mod share;

pub use capability::{Action, Capability};
pub use error::PermError;
pub use grant::{
    CapabilitySet, FieldAccess, Grant, Granted, Inactive, Resource, Scope, SodKind, SodRule,
};
pub use policy::{
    Actor, Applied, Decision, Directory, Env, Facts, FieldMeta, FieldPermission, Policy,
    PrincipalKind, Reason, RecordFacts, Refusal, Request, RowFilter, RowFilterPart, RowRule,
    Violation, Visibility,
};
pub use review::{AccessReview, ReviewItem, ReviewState, Verdict};
pub use share::{ShareLink, ShareMode};

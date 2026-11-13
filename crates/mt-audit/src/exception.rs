//! Exception descriptors (doc 17 D7 `Exception`) and their status state machine.
//!
//! An exception is how an open loop becomes somebody's work: it has a kind, a subject, an owner and a
//! status. [`ExceptionDescriptor::step`] is the pure transition function:
//!
//! | event \ status | open | acknowledged | in_progress | resolved | dismissed |
//! |---|---|---|---|---|---|
//! | `Assign(owner)` | open (owner set) | →open (new owner) | →open (new owner) | ✗ | ✗ |
//! | `Acknowledge` | →acknowledged¹ | ✗ | ✗ | ✗ | ✗ |
//! | `Start` | →in_progress¹ | →in_progress | ✗ | ✗ | ✗ |
//! | `Resolve(at)` | →resolved | →resolved | →resolved | ✗ | ✗ |
//! | `Dismiss(at)` | →dismissed | →dismissed | →dismissed | ✗ | ✗ |
//! | `Reopen` | ✗ | ✗ | ✗ | →open | →open |
//!
//! ¹ requires an owner. Re-assigning to the *same* owner keeps the status.

use serde::{Deserialize, Serialize};

use crate::chain::{ObjectRef, Principal};
use crate::error::AuditError;
use crate::trace::TraceId;

/// What kind of problem an exception describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExceptionKind {
    /// Work item without an owner.
    Unassigned,
    /// A deadline or SLA passed.
    Overdue,
    /// An automated action failed.
    FailedAction,
    /// A value could not be mapped to reference data.
    UnmappedValue,
    /// An expected submission did not arrive.
    MissingSubmission,
    /// A data-quality rule failed.
    Quality,
    /// AI output quality fell below its threshold.
    AiQuality,
    /// A process instance stopped moving.
    StuckProcess,
    /// A required follow-up step of a process did not happen in time.
    MissingStep,
    /// An operation never reached a terminal delivery state.
    UndeliveredOperation,
    /// The audit chain failed verification.
    AuditIntegrity,
}

impl ExceptionKind {
    /// Stable snake_case name (same as the serialised form).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Unassigned => "unassigned",
            Self::Overdue => "overdue",
            Self::FailedAction => "failed_action",
            Self::UnmappedValue => "unmapped_value",
            Self::MissingSubmission => "missing_submission",
            Self::Quality => "quality",
            Self::AiQuality => "ai_quality",
            Self::StuckProcess => "stuck_process",
            Self::MissingStep => "missing_step",
            Self::UndeliveredOperation => "undelivered_operation",
            Self::AuditIntegrity => "audit_integrity",
        }
    }
}

/// Where an exception is in its handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExceptionStatus {
    /// Raised; nobody has reacted.
    Open,
    /// The owner has seen it.
    Acknowledged,
    /// The owner is working on it.
    InProgress,
    /// The problem is fixed.
    Resolved,
    /// Closed without a fix (false alarm, accepted).
    Dismissed,
}

impl ExceptionStatus {
    /// Stable snake_case name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Acknowledged => "acknowledged",
            Self::InProgress => "in_progress",
            Self::Resolved => "resolved",
            Self::Dismissed => "dismissed",
        }
    }

    /// Whether the exception is closed (resolved or dismissed).
    #[must_use]
    pub const fn is_closed(self) -> bool {
        match self {
            Self::Open | Self::Acknowledged | Self::InProgress => false,
            Self::Resolved | Self::Dismissed => true,
        }
    }
}

/// Something that happens to an exception.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ExceptionEvent {
    /// Give the exception to an owner.
    Assign {
        /// The new owner.
        owner: Principal,
    },
    /// The owner confirms having seen it.
    Acknowledge,
    /// The owner starts working on it.
    Start,
    /// The problem is fixed (by a person, or automatically when the condition clears).
    Resolve {
        /// When, UTC milliseconds.
        at_ms: i64,
    },
    /// Close without a fix.
    Dismiss {
        /// When, UTC milliseconds.
        at_ms: i64,
    },
    /// The problem came back.
    Reopen,
}

impl ExceptionEvent {
    /// Stable snake_case name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Assign { .. } => "assign",
            Self::Acknowledge => "acknowledge",
            Self::Start => "start",
            Self::Resolve { .. } => "resolve",
            Self::Dismiss { .. } => "dismiss",
            Self::Reopen => "reopen",
        }
    }
}

/// A described exception. Invariant: `closed_at_ms` is `Some` exactly when the status is closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExceptionDescriptor {
    /// Deterministic identity of the underlying condition; consumers upsert by this key.
    pub key: String,
    /// What kind of problem.
    pub kind: ExceptionKind,
    /// What it is about.
    pub subject: ObjectRef,
    /// Who must handle it.
    pub owner: Option<Principal>,
    /// Handling status.
    pub status: ExceptionStatus,
    /// Human-readable one-line summary.
    pub summary: String,
    /// When the condition began, UTC milliseconds.
    pub opened_at_ms: i64,
    /// When it was resolved or dismissed.
    pub closed_at_ms: Option<i64>,
    /// Trace of the action that caused it, if known.
    pub trace_id: Option<TraceId>,
}

impl ExceptionDescriptor {
    /// A new, open, unowned exception.
    pub fn open(
        key: impl Into<String>,
        kind: ExceptionKind,
        subject: ObjectRef,
        summary: impl Into<String>,
        opened_at_ms: i64,
    ) -> Self {
        Self {
            key: key.into(),
            kind,
            subject,
            owner: None,
            status: ExceptionStatus::Open,
            summary: summary.into(),
            opened_at_ms,
            closed_at_ms: None,
            trace_id: None,
        }
    }

    /// The pure transition function; see the table in the module documentation. On error `self` is
    /// unchanged (it is not consumed).
    pub fn step(&self, event: &ExceptionEvent) -> Result<Self, AuditError> {
        use ExceptionEvent as E;
        use ExceptionStatus as S;
        let illegal = || AuditError::IllegalExceptionTransition {
            from: self.status.name(),
            event: event.name(),
        };
        let mut next = self.clone();
        match (event, self.status) {
            (E::Assign { owner }, S::Open | S::Acknowledged | S::InProgress) => {
                if self.owner.as_ref() != Some(owner) {
                    next.owner = Some(owner.clone());
                    next.status = S::Open;
                }
            }
            (E::Acknowledge, S::Open) | (E::Start, S::Open) if self.owner.is_none() => {
                return Err(illegal());
            }
            (E::Acknowledge, S::Open) => next.status = S::Acknowledged,
            (E::Start, S::Open | S::Acknowledged) => next.status = S::InProgress,
            (E::Resolve { at_ms }, S::Open | S::Acknowledged | S::InProgress) => {
                next.status = S::Resolved;
                next.closed_at_ms = Some(*at_ms);
            }
            (E::Dismiss { at_ms }, S::Open | S::Acknowledged | S::InProgress) => {
                next.status = S::Dismissed;
                next.closed_at_ms = Some(*at_ms);
            }
            (E::Reopen, S::Resolved | S::Dismissed) => {
                next.status = S::Open;
                next.closed_at_ms = None;
            }
            (
                E::Assign { .. } | E::Resolve { .. } | E::Dismiss { .. },
                S::Resolved | S::Dismissed,
            )
            | (E::Acknowledge, S::Acknowledged | S::InProgress | S::Resolved | S::Dismissed)
            | (E::Start, S::InProgress | S::Resolved | S::Dismissed)
            | (E::Reopen, S::Open | S::Acknowledged | S::InProgress) => return Err(illegal()),
        }
        Ok(next)
    }
}

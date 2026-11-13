//! Process contracts and the completeness verifier (doc 03 §4.4, doc 02 B10 layer 3).
//!
//! A [`ProcessContract`] states what a finished process instance must look like:
//! - **terminal events**: the instance is finished once one of them occurred (and no *reopen event*
//!   occurred after it);
//! - **follow-up rules**: after a trigger event, a required event must occur within a deadline
//!   (for example "after `resolved`, `requester_notified` within 60 000 ms");
//! - optional **maximum duration** from the first event to the terminal event;
//! - optional **stuck threshold**: maximum silence of an unfinished instance.
//!
//! The [`CompletenessVerifier`] consumes the ordered event journal incrementally (it remembers a sequence
//! cursor and is serialisable, so it can be stored and resumed) and classifies every instance at a given
//! time `now_ms`:
//!
//! | Status | Meaning |
//! |---|---|
//! | `Broken(MissingFollowUp)` | a follow-up deadline passed and the required event has not occurred |
//! | `Broken(Stuck)` | unfinished and silent for longer than the stuck threshold |
//! | `Late(..)` | no open violation, but a follow-up was done after its deadline, or the maximum duration was exceeded |
//! | `Complete` | finished, no open follow-ups, everything on time |
//! | `InProgress` | otherwise |
//!
//! `Broken` wins over `Late`. Exception descriptors are emitted only for conditions that still need
//! action (missing follow-up → `missing_step`, stuck → `stuck_process`, unfinished past maximum duration
//! → `overdue`). Their keys are deterministic, so a consumer upserts by key and resolves exceptions whose
//! key is no longer reported.
//!
//! A required event satisfies **all** open obligations of its rule (one notification after two
//! `resolved` events covers both). Time comes from the events' `at_ms` and from `now_ms`; no clock is
//! read.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::chain::ObjectRef;
use crate::error::AuditError;
use crate::exception::{ExceptionDescriptor, ExceptionKind};

/// "After `trigger`, `required` must occur within `within_ms`."
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FollowUpRule {
    /// Event kind that starts the obligation.
    pub trigger: String,
    /// Event kind that fulfils it.
    pub required: String,
    /// Deadline in milliseconds after the trigger (≥ 0; the bound is inclusive).
    pub within_ms: i64,
}

/// The expected shape of a process. Build with [`ProcessContract::new`], which validates it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessContract {
    id: String,
    terminal_events: BTreeSet<String>,
    reopen_events: BTreeSet<String>,
    follow_ups: Vec<FollowUpRule>,
    max_duration_ms: Option<i64>,
    stuck_after_ms: Option<i64>,
}

impl ProcessContract {
    /// A contract with the given id and terminal event kinds (at least one).
    pub fn new<I, S>(id: impl Into<String>, terminal_events: I) -> Result<Self, AuditError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let id = id.into();
        let terminal_events: BTreeSet<String> =
            terminal_events.into_iter().map(Into::into).collect();
        if id.is_empty() {
            return Err(AuditError::InvalidContract("empty id"));
        }
        if terminal_events.is_empty() {
            return Err(AuditError::InvalidContract("no terminal event"));
        }
        if terminal_events.iter().any(String::is_empty) {
            return Err(AuditError::InvalidContract("empty event kind"));
        }
        Ok(Self {
            id,
            terminal_events,
            reopen_events: BTreeSet::new(),
            follow_ups: Vec::new(),
            max_duration_ms: None,
            stuck_after_ms: None,
        })
    }

    /// Adds an event kind that makes a finished instance unfinished again.
    pub fn with_reopen_event(mut self, kind: impl Into<String>) -> Result<Self, AuditError> {
        let kind = kind.into();
        if kind.is_empty() {
            return Err(AuditError::InvalidContract("empty event kind"));
        }
        if self.terminal_events.contains(&kind) {
            return Err(AuditError::InvalidContract(
                "event is both terminal and reopen",
            ));
        }
        self.reopen_events.insert(kind);
        Ok(self)
    }

    /// Adds a follow-up rule.
    pub fn with_follow_up(
        mut self,
        trigger: impl Into<String>,
        required: impl Into<String>,
        within_ms: i64,
    ) -> Result<Self, AuditError> {
        let rule = FollowUpRule {
            trigger: trigger.into(),
            required: required.into(),
            within_ms,
        };
        if rule.trigger.is_empty() || rule.required.is_empty() {
            return Err(AuditError::InvalidContract("empty event kind"));
        }
        if rule.trigger == rule.required {
            return Err(AuditError::InvalidContract(
                "follow-up requires its own trigger",
            ));
        }
        if rule.within_ms < 0 {
            return Err(AuditError::InvalidContract("negative deadline"));
        }
        self.follow_ups.push(rule);
        Ok(self)
    }

    /// Sets the maximum duration from the first event to the terminal event (> 0).
    pub fn with_max_duration_ms(mut self, ms: i64) -> Result<Self, AuditError> {
        if ms <= 0 {
            return Err(AuditError::InvalidContract("non-positive maximum duration"));
        }
        self.max_duration_ms = Some(ms);
        Ok(self)
    }

    /// Sets the maximum silence of an unfinished instance (> 0).
    pub fn with_stuck_after_ms(mut self, ms: i64) -> Result<Self, AuditError> {
        if ms <= 0 {
            return Err(AuditError::InvalidContract("non-positive stuck threshold"));
        }
        self.stuck_after_ms = Some(ms);
        Ok(self)
    }

    /// The contract id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The follow-up rules.
    #[must_use]
    pub fn follow_ups(&self) -> &[FollowUpRule] {
        &self.follow_ups
    }
}

/// One journal event of a process instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessEvent {
    /// Journal sequence number (≥ 1, strictly increasing across the journal).
    pub seq: u64,
    /// Process instance id (record id + lifecycle version, or workflow run id).
    pub instance: String,
    /// Event kind, e.g. `resolved`.
    pub kind: String,
    /// When it happened, UTC milliseconds.
    pub at_ms: i64,
}

impl ProcessEvent {
    /// Builds an event.
    pub fn new(seq: u64, instance: impl Into<String>, kind: impl Into<String>, at_ms: i64) -> Self {
        Self {
            seq,
            instance: instance.into(),
            kind: kind.into(),
            at_ms,
        }
    }
}

/// Why an instance is late.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "late", rename_all = "snake_case")]
pub enum LateReason {
    /// A required follow-up happened, but after its deadline.
    FollowUpLate {
        /// The trigger event kind.
        trigger: String,
        /// The required event kind.
        required: String,
        /// The deadline that was missed.
        deadline_ms: i64,
        /// When the required event actually occurred.
        done_at_ms: i64,
    },
    /// The instance is unfinished and past its maximum duration.
    DurationExceeded {
        /// When it should have finished.
        deadline_ms: i64,
    },
    /// The instance finished, but after its maximum duration.
    CompletedLate {
        /// When it should have finished.
        deadline_ms: i64,
        /// When it finished.
        completed_at_ms: i64,
    },
}

/// Why an instance is broken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "broken", rename_all = "snake_case")]
pub enum BrokenReason {
    /// A follow-up deadline passed without the required event.
    MissingFollowUp {
        /// The trigger event kind.
        trigger: String,
        /// The required event kind.
        required: String,
        /// Journal sequence number of the trigger event.
        trigger_seq: u64,
        /// The deadline that passed.
        deadline_ms: i64,
    },
    /// The instance is unfinished and has been silent for too long.
    Stuck {
        /// Time of its last event.
        last_event_at_ms: i64,
        /// The moment it counted as stuck.
        stuck_since_ms: i64,
    },
}

/// Classification of one instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum InstanceStatus {
    /// Finished, on time, nothing outstanding.
    Complete,
    /// Not finished (or follow-ups still open) and nothing violated.
    InProgress,
    /// Something happened too late.
    Late(LateReason),
    /// A contract obligation is violated right now.
    Broken(BrokenReason),
}

/// The verifier's verdict on one instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceReport {
    /// Process instance id.
    pub instance: String,
    /// Classification.
    pub status: InstanceStatus,
    /// Sequence number of the instance's first event.
    pub first_seq: u64,
    /// Sequence number of the instance's latest event.
    pub last_seq: u64,
    /// Exceptions that need action now (empty unless broken or overdue).
    pub exceptions: Vec<ExceptionDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Obligation {
    rule: usize,
    trigger_seq: u64,
    deadline_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct InstanceState {
    first_seq: u64,
    last_seq: u64,
    started_at_ms: i64,
    last_at_ms: i64,
    terminal_at_ms: Option<i64>,
    open: Vec<Obligation>,
    late: Option<LateReason>,
}

/// Incremental completeness verifier for one contract.
///
/// Serialise it to persist progress; deserialise to resume. Events at or below the cursor are ignored,
/// so re-feeding an overlapping batch is harmless.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletenessVerifier {
    contract: ProcessContract,
    cursor: u64,
    instances: BTreeMap<String, InstanceState>,
}

impl CompletenessVerifier {
    /// A verifier that has consumed nothing.
    #[must_use]
    pub const fn new(contract: ProcessContract) -> Self {
        Self {
            contract,
            cursor: 0,
            instances: BTreeMap::new(),
        }
    }

    /// The contract being verified.
    #[must_use]
    pub const fn contract(&self) -> &ProcessContract {
        &self.contract
    }

    /// Sequence number of the last consumed event (0 = none). Resume the journal after this.
    #[must_use]
    pub const fn cursor(&self) -> u64 {
        self.cursor
    }

    /// Number of instances currently tracked.
    #[must_use]
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// Consumes a batch of journal events. Returns how many were new (above the cursor).
    ///
    /// The batch must be strictly increasing in `seq` and contain no `seq` 0; otherwise nothing is
    /// consumed and an error is returned.
    pub fn ingest(&mut self, events: &[ProcessEvent]) -> Result<usize, AuditError> {
        let mut previous = 0u64;
        for event in events {
            if event.seq == 0 {
                return Err(AuditError::ZeroSequence);
            }
            if event.seq <= previous {
                return Err(AuditError::JournalOutOfOrder {
                    seq: event.seq,
                    after: previous,
                });
            }
            previous = event.seq;
        }
        let mut applied = 0usize;
        let cursor = self.cursor;
        for event in events.iter().filter(|e| e.seq > cursor) {
            self.apply(event);
            applied = applied.saturating_add(1);
        }
        self.cursor = self.cursor.max(previous);
        Ok(applied)
    }

    fn apply(&mut self, event: &ProcessEvent) {
        let state = self
            .instances
            .entry(event.instance.clone())
            .or_insert_with(|| InstanceState {
                first_seq: event.seq,
                last_seq: event.seq,
                started_at_ms: event.at_ms,
                last_at_ms: event.at_ms,
                terminal_at_ms: None,
                open: Vec::new(),
                late: None,
            });
        state.last_seq = event.seq;
        state.last_at_ms = event.at_ms;
        for (index, rule) in self.contract.follow_ups.iter().enumerate() {
            if rule.required == event.kind {
                let mut first_late = None;
                state.open.retain(|o| {
                    if o.rule != index {
                        return true;
                    }
                    if event.at_ms > o.deadline_ms && first_late.is_none() {
                        first_late = Some(o.deadline_ms);
                    }
                    false
                });
                if let (Some(deadline_ms), None) = (first_late, &state.late) {
                    state.late = Some(LateReason::FollowUpLate {
                        trigger: rule.trigger.clone(),
                        required: rule.required.clone(),
                        deadline_ms,
                        done_at_ms: event.at_ms,
                    });
                }
            }
        }
        for (index, rule) in self.contract.follow_ups.iter().enumerate() {
            if rule.trigger == event.kind {
                state.open.push(Obligation {
                    rule: index,
                    trigger_seq: event.seq,
                    deadline_ms: event.at_ms.saturating_add(rule.within_ms),
                });
            }
        }
        if self.contract.terminal_events.contains(&event.kind) {
            state.terminal_at_ms = Some(event.at_ms);
        } else if self.contract.reopen_events.contains(&event.kind) {
            state.terminal_at_ms = None;
        }
    }

    /// Classifies every tracked instance at time `now_ms`, ordered by instance id.
    #[must_use]
    pub fn evaluate(&self, now_ms: i64) -> Vec<InstanceReport> {
        self.instances
            .iter()
            .map(|(id, state)| self.report(id, state, now_ms))
            .collect()
    }

    /// Classifies one instance at time `now_ms`.
    #[must_use]
    pub fn evaluate_instance(&self, instance: &str, now_ms: i64) -> Option<InstanceReport> {
        self.instances
            .get(instance)
            .map(|state| self.report(instance, state, now_ms))
    }

    /// Drops instances that are `Complete` at `now_ms` and returns how many were dropped. A later event
    /// for a dropped instance starts a fresh instance record.
    pub fn prune_complete(&mut self, now_ms: i64) -> usize {
        let done: Vec<String> = self
            .evaluate(now_ms)
            .into_iter()
            .filter(|r| r.status == InstanceStatus::Complete)
            .map(|r| r.instance)
            .collect();
        for id in &done {
            self.instances.remove(id);
        }
        done.len()
    }

    fn report(&self, instance: &str, state: &InstanceState, now_ms: i64) -> InstanceReport {
        let subject = || ObjectRef::new("process_instance", instance);
        let prefix = format!("{}/{}", self.contract.id, instance);
        let mut exceptions = Vec::new();
        let mut broken: Option<BrokenReason> = None;

        let mut overdue: Vec<&Obligation> = state
            .open
            .iter()
            .filter(|o| o.deadline_ms < now_ms)
            .collect();
        overdue.sort_by_key(|o| (o.deadline_ms, o.trigger_seq, o.rule));
        for obligation in overdue {
            let Some(rule) = self.contract.follow_ups.get(obligation.rule) else {
                continue;
            };
            exceptions.push(ExceptionDescriptor::open(
                format!(
                    "{prefix}/missing_step/{}/{}",
                    rule.required, obligation.trigger_seq
                ),
                ExceptionKind::MissingStep,
                subject(),
                format!(
                    "`{}` did not follow `{}` in time",
                    rule.required, rule.trigger
                ),
                obligation.deadline_ms,
            ));
            broken.get_or_insert_with(|| BrokenReason::MissingFollowUp {
                trigger: rule.trigger.clone(),
                required: rule.required.clone(),
                trigger_seq: obligation.trigger_seq,
                deadline_ms: obligation.deadline_ms,
            });
        }

        let finished = state.terminal_at_ms.is_some();
        if let (false, Some(limit)) = (finished, self.contract.stuck_after_ms) {
            let stuck_since_ms = state.last_at_ms.saturating_add(limit);
            if stuck_since_ms < now_ms {
                exceptions.push(ExceptionDescriptor::open(
                    format!("{prefix}/stuck_process/{}", state.last_seq),
                    ExceptionKind::StuckProcess,
                    subject(),
                    "process instance has stopped moving",
                    stuck_since_ms,
                ));
                broken.get_or_insert(BrokenReason::Stuck {
                    last_event_at_ms: state.last_at_ms,
                    stuck_since_ms,
                });
            }
        }

        let mut late = state.late.clone();
        if let Some(limit) = self.contract.max_duration_ms {
            let deadline_ms = state.started_at_ms.saturating_add(limit);
            match state.terminal_at_ms {
                Some(completed_at_ms) if completed_at_ms > deadline_ms => {
                    late.get_or_insert(LateReason::CompletedLate {
                        deadline_ms,
                        completed_at_ms,
                    });
                }
                Some(_) => {}
                None if now_ms > deadline_ms => {
                    exceptions.push(ExceptionDescriptor::open(
                        format!("{prefix}/overdue"),
                        ExceptionKind::Overdue,
                        subject(),
                        "process instance exceeded its maximum duration",
                        deadline_ms,
                    ));
                    late.get_or_insert(LateReason::DurationExceeded { deadline_ms });
                }
                None => {}
            }
        }

        let status = match (broken, late) {
            (Some(reason), _) => InstanceStatus::Broken(reason),
            (None, Some(reason)) => InstanceStatus::Late(reason),
            (None, None) if finished && state.open.is_empty() => InstanceStatus::Complete,
            (None, None) => InstanceStatus::InProgress,
        };
        InstanceReport {
            instance: instance.to_owned(),
            status,
            first_seq: state.first_seq,
            last_seq: state.last_seq,
            exceptions,
        }
    }
}

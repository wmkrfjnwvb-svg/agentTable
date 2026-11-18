//! SLA clocks: how long something may take, counted in working time, with pauses.
//!
//! A clock is a sans-IO state machine: [`SlaClock::step`] takes an [`SlaInput`] with its
//! instant and returns what happened ([`SlaEvent`]). Time is an input; the clock never reads
//! one. Pausing (while waiting for the requester, for example) stops the count; a tick reports a
//! warning once, when the configured share of the target is used, and a breach once, with the
//! instant the target was actually exceeded.

use crate::calendar::BusinessCalendar;
use crate::error::LifecycleError;
use mt_core::time::{DurationMs, Timestamp};
use serde::{Deserialize, Serialize};

/// How long something may take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlaPolicy {
    /// Name ("First response", "Resolution").
    pub name: String,
    /// The allowed working time.
    pub target: DurationMs,
    /// Warn when this share (percent) of the target is used; 0 for no warning.
    #[serde(default)]
    pub warn_percent: u8,
    /// When work happens.
    pub calendar: BusinessCalendar,
}

impl SlaPolicy {
    /// Verifies the policy: a name, a positive target, a warning below 100 %, a valid calendar.
    ///
    /// # Errors
    /// [`LifecycleError::Invalid`].
    pub fn check(&self) -> Result<(), LifecycleError> {
        if self.name.trim().is_empty() {
            return Err(LifecycleError::invalid("an SLA policy needs a name"));
        }
        if self.target.millis() <= 0 {
            return Err(LifecycleError::invalid(
                "an SLA target is a positive duration",
            ));
        }
        if self.warn_percent >= 100 {
            return Err(LifecycleError::invalid(
                "the warning comes before the target: below 100 %",
            ));
        }
        self.calendar.check()
    }

    fn warn_after(&self) -> Option<DurationMs> {
        (self.warn_percent > 0).then(|| {
            let ms = self
                .target
                .millis()
                .saturating_mul(i64::from(self.warn_percent))
                .checked_div(100)
                .unwrap_or(0);
            DurationMs::from_millis(ms)
        })
    }
}

/// Where a clock stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ClockState {
    /// Counting since `since`.
    Running {
        /// When it last started or resumed.
        since: Timestamp,
    },
    /// Not counting.
    Paused,
    /// Stopped for good.
    Stopped {
        /// Whether the target was met.
        met: bool,
    },
}

/// What happens to a clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "input", rename_all = "snake_case")]
pub enum SlaInput {
    /// Stop counting for now.
    Pause {
        /// When.
        at: Timestamp,
    },
    /// Count again.
    Resume {
        /// When.
        at: Timestamp,
    },
    /// Stop for good (the work is done or abandoned).
    Stop {
        /// When.
        at: Timestamp,
    },
    /// Time passed: report a warning or a breach that happened by now.
    Tick {
        /// Now.
        at: Timestamp,
    },
}

impl SlaInput {
    const fn at(self) -> Timestamp {
        match self {
            SlaInput::Pause { at }
            | SlaInput::Resume { at }
            | SlaInput::Stop { at }
            | SlaInput::Tick { at } => at,
        }
    }
}

/// What a clock reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SlaEvent {
    /// The warning share of the target was used.
    Warned {
        /// When it was reached.
        at: Timestamp,
    },
    /// The target was exceeded.
    Breached {
        /// When it was exceeded.
        at: Timestamp,
    },
    /// The clock stopped within the target.
    Met,
    /// The clock stopped after the target.
    Missed,
}

/// A running SLA clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlaClock {
    /// The policy it counts against.
    pub policy: SlaPolicy,
    /// When it started.
    pub started_at: Timestamp,
    /// Working time counted before the current running period.
    pub counted: DurationMs,
    /// Where it stands.
    pub state: ClockState,
    /// When the warning was reported, if it was.
    pub warned_at: Option<Timestamp>,
    /// When the breach happened, if it did.
    pub breached_at: Option<Timestamp>,
    /// The last instant the clock was told about.
    pub last_seen: Timestamp,
}

impl SlaClock {
    /// Starts a clock.
    ///
    /// # Errors
    /// [`LifecycleError::Invalid`] for an invalid policy.
    pub fn start(policy: SlaPolicy, at: Timestamp) -> Result<SlaClock, LifecycleError> {
        policy.check()?;
        Ok(SlaClock {
            policy,
            started_at: at,
            counted: DurationMs::ZERO,
            state: ClockState::Running { since: at },
            warned_at: None,
            breached_at: None,
            last_seen: at,
        })
    }

    /// Working time used by `at`.
    #[must_use]
    pub fn used(&self, at: Timestamp) -> DurationMs {
        let running = match self.state {
            ClockState::Running { since } => self
                .policy
                .calendar
                .between(since, at)
                .unwrap_or(self.policy.target),
            ClockState::Paused | ClockState::Stopped { .. } => DurationMs::ZERO,
        };
        self.counted
            .checked_add(running)
            .unwrap_or(self.policy.target)
    }

    /// When the target is (or was) reached if the clock keeps running; `None` while paused
    /// or stopped.
    #[must_use]
    pub fn due_at(&self) -> Option<Timestamp> {
        let ClockState::Running { since } = self.state else {
            return None;
        };
        let left = self
            .policy
            .target
            .millis()
            .saturating_sub(self.counted.millis());
        self.policy
            .calendar
            .add(since, DurationMs::from_millis(left.max(0)))
    }

    /// The instant a share `limit` of the target is used, while running.
    fn reached(&self, limit: DurationMs) -> Option<Timestamp> {
        let ClockState::Running { since } = self.state else {
            return None;
        };
        let left = limit.millis().saturating_sub(self.counted.millis()).max(0);
        self.policy
            .calendar
            .add(since, DurationMs::from_millis(left))
    }

    /// Applies an input and reports what happened.
    ///
    /// # Errors
    /// [`LifecycleError::Time`] for an instant before the last one;
    /// [`LifecycleError::State`] for pausing a paused clock, resuming a running one, or any
    /// input but a tick after the clock stopped.
    pub fn step(&mut self, input: SlaInput) -> Result<Vec<SlaEvent>, LifecycleError> {
        let at = input.at();
        if at < self.last_seen {
            return Err(LifecycleError::Time(format!(
                "{at} is before {}",
                self.last_seen
            )));
        }
        let mut events = Vec::new();
        // Whatever the input, first report what happened up to `at`.
        if let Some(warn) = self.policy.warn_after() {
            if self.warned_at.is_none() {
                if let Some(when) = self.reached(warn).filter(|when| *when <= at) {
                    self.warned_at = Some(when);
                    events.push(SlaEvent::Warned { at: when });
                }
            }
        }
        if self.breached_at.is_none() {
            let target = self.policy.target;
            // Breached once the target is exceeded: strictly after the due instant.
            if let Some(when) = self.reached(target).filter(|when| *when < at) {
                self.breached_at = Some(when);
                events.push(SlaEvent::Breached { at: when });
            }
        }
        match (input, self.state) {
            (SlaInput::Tick { .. }, _) => {}
            (SlaInput::Pause { .. }, ClockState::Running { .. }) => {
                self.counted = self.used(at);
                self.state = ClockState::Paused;
            }
            (SlaInput::Resume { .. }, ClockState::Paused) => {
                self.state = ClockState::Running { since: at };
            }
            (SlaInput::Stop { .. }, ClockState::Running { .. } | ClockState::Paused) => {
                self.counted = self.used(at);
                let met = self.counted <= self.policy.target;
                self.state = ClockState::Stopped { met };
                events.push(if met { SlaEvent::Met } else { SlaEvent::Missed });
            }
            (SlaInput::Pause { .. }, ClockState::Paused) => {
                return Err(LifecycleError::state("the clock is already paused"));
            }
            (SlaInput::Resume { .. }, ClockState::Running { .. }) => {
                return Err(LifecycleError::state("the clock is already running"));
            }
            (
                SlaInput::Pause { .. } | SlaInput::Resume { .. } | SlaInput::Stop { .. },
                ClockState::Stopped { .. },
            ) => {
                return Err(LifecycleError::state("the clock has stopped"));
            }
        }
        self.last_seen = at;
        Ok(events)
    }
}

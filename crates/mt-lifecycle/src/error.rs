//! Errors of the crate.

/// Why a definition, a transition or a step was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LifecycleError {
    /// A lifecycle, calendar, policy or approval definition is not well formed.
    #[error("{0}")]
    Invalid(String),
    /// The step does not fit the current state of a clock or an approval.
    #[error("{0}")]
    State(String),
    /// Time went backwards.
    #[error("time went backwards: {0}")]
    Time(String),
    /// A running approval was given a definition of another version.
    #[error("this approval runs on version {running} of its definition, not {given}")]
    Version {
        /// The version the approval started with.
        running: u32,
        /// The version given.
        given: u32,
    },
    /// The actor may not act on this task.
    #[error("{0}")]
    NotAllowed(String),
}

impl LifecycleError {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        LifecycleError::Invalid(message.into())
    }

    pub(crate) fn state(message: impl Into<String>) -> Self {
        LifecycleError::State(message.into())
    }
}

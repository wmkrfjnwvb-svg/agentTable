//! The error type of the crate. Errors are values; nothing here panics.

use mt_core::CoreError;

/// Everything that can be wrong with a query or a view definition.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QueryError {
    /// A part of the definition is not acceptable.
    #[error("invalid {what}: {reason}")]
    Invalid {
        /// What was being checked (`"sort"`, `"group"`, `"view"`, …).
        what: &'static str,
        /// Why it was refused, in plain English.
        reason: String,
    },
    /// A size limit was exceeded.
    #[error("limit exceeded: {0}")]
    Limit(&'static str),
    /// An error from the foundation types (for example from the filter condition).
    #[error(transparent)]
    Core(#[from] CoreError),
}

impl QueryError {
    pub(crate) fn invalid(what: &'static str, reason: impl Into<String>) -> Self {
        QueryError::Invalid {
            what,
            reason: reason.into(),
        }
    }
}

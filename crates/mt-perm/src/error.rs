//! Errors of the crate.

use mt_core::id::{CapabilitySetId, GrantId, SodRuleId};

/// Why a change to the access policy, a share link or an access review was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PermError {
    /// A grant, capability set, rule or link is not well formed.
    #[error("{0}")]
    Invalid(String),
    /// The id is already used.
    #[error("{what} {id} already exists")]
    Duplicate {
        /// What kind of thing.
        what: &'static str,
        /// Its id.
        id: String,
    },
    /// The grant names a capability set that does not exist.
    #[error("capability set {0} does not exist")]
    UnknownSet(CapabilitySetId),
    /// The grant does not exist.
    #[error("grant {0} does not exist")]
    UnknownGrant(GrantId),
    /// The grant was already revoked.
    #[error("grant {0} was already revoked")]
    AlreadyRevoked(GrantId),
    /// The capability set is still used by grants.
    #[error("capability set {set} is still used by {grants} grant(s)")]
    SetInUse {
        /// The set.
        set: CapabilitySetId,
        /// How many grants use it.
        grants: usize,
    },
    /// The grant would give one principal two capabilities that must be held by different
    /// people.
    #[error("separation of duties: rule {rule} forbids holding `{first}` and `{second}` together")]
    SodConflict {
        /// The rule.
        rule: SodRuleId,
        /// One capability of the pair.
        first: String,
        /// The other.
        second: String,
    },
    /// The person making the change may not make it.
    #[error("not allowed: {0}")]
    NotAllowed(String),
    /// The access review does not accept this step in its current state.
    #[error("access review: {0}")]
    Review(String),
}

impl PermError {
    pub(crate) fn invalid(reason: impl Into<String>) -> Self {
        PermError::Invalid(reason.into())
    }
}

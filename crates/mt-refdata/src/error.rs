//! The error type of the crate. Errors are values; nothing here panics.

use mt_core::id::{RefItemId, RefSetId};
use mt_core::CoreError;

/// Everything that can go wrong with reference data and parameters.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RefDataError {
    /// A code, name, alias or other text is malformed.
    #[error("invalid {what}: {reason}")]
    Invalid {
        /// What was being checked (`"item code"`, `"alias"`, …).
        what: &'static str,
        /// Why it was refused, in plain English.
        reason: String,
    },
    /// Two items share a code (codes are compared without regard to ASCII case).
    #[error("the code `{code}` is used by more than one item")]
    DuplicateCode {
        /// The code.
        code: String,
    },
    /// Two items share an id.
    #[error("the item {id} appears more than once")]
    DuplicateItem {
        /// The id.
        id: RefItemId,
    },
    /// An item that the operation needs does not exist in the set.
    #[error("the item {id} is not in the set")]
    UnknownItem {
        /// The id.
        id: RefItemId,
    },
    /// A set that the operation needs is not loaded.
    #[error("the reference set {id} is not available")]
    UnknownSet {
        /// The id.
        id: RefSetId,
    },
    /// An item names a parent that does not exist.
    #[error("the item {item} names the parent {parent}, which is not in the set")]
    UnknownParent {
        /// The item.
        item: RefItemId,
        /// The missing parent.
        parent: RefItemId,
    },
    /// An item of a flat set has a parent.
    #[error("the item {item} has a parent, but the set is flat")]
    FlatSetHasParent {
        /// The item.
        item: RefItemId,
    },
    /// Following parents from this item never reaches a root.
    #[error("the item {item} is its own ancestor")]
    Cycle {
        /// An item on the cycle.
        item: RefItemId,
    },
    /// The tree is deeper than the set allows.
    #[error("the item {item} would be at level {depth}; the set allows {max}")]
    TooDeep {
        /// The item.
        item: RefItemId,
        /// Its level (1 = top).
        depth: usize,
        /// The deepest level allowed.
        max: u8,
    },
    /// One external value would map to two items.
    #[error("in system `{system}` the value `{value}` is an alias of more than one item")]
    AliasConflict {
        /// The external system.
        system: String,
        /// The external value.
        value: String,
    },
    /// A tenant tried to change something only the system may change.
    #[error("the item {id} is a system item; tenants may relabel, disable and add aliases, but not {action}")]
    SystemItem {
        /// The item.
        id: RefItemId,
        /// What was attempted.
        action: &'static str,
    },
    /// An overlay was given for a set that is not a system set.
    #[error("only system sets take a tenant overlay")]
    OverlayNotAllowed,
    /// The item still has children and cannot be removed.
    #[error("the item {id} has child items; move or remove them first")]
    HasChildren {
        /// The item.
        id: RefItemId,
    },
    /// The change was prepared against another version of the set.
    #[error("the set is at version {actual}, but the change was made against version {expected}")]
    VersionConflict {
        /// The version the caller saw.
        expected: u64,
        /// The current version.
        actual: u64,
    },
    /// The set's policy requires changes to be proposed and approved.
    #[error("changes to this set must be proposed and approved")]
    ApprovalRequired,
    /// The proposer tried to approve or reject their own proposal.
    #[error("a proposal cannot be decided by the person who made it")]
    SelfApproval,
    /// The proposal does not exist or is not pending any more.
    #[error("the proposal {id} is not pending")]
    NotPending {
        /// The proposal number.
        id: u64,
    },
    /// Only the proposer may withdraw a proposal.
    #[error("only the person who made a proposal can withdraw it")]
    NotProposer,
    /// A change list was empty.
    #[error("there is nothing to change")]
    EmptyChange,
    /// A size limit was exceeded.
    #[error("limit exceeded: {0}")]
    Limit(&'static str),
    /// The requested version does not exist.
    #[error("the set has no version {version}")]
    UnknownVersion {
        /// The version.
        version: u64,
    },
    /// A parameter is not registered.
    #[error("the parameter `{key}` is not registered")]
    UnknownParameter {
        /// The key.
        key: String,
    },
    /// A parameter value does not fit its definition.
    #[error("the value for `{key}` is not acceptable: {reason}")]
    InvalidParameter {
        /// The key.
        key: String,
        /// Why.
        reason: String,
    },
    /// A parameter cannot be set at the requested scope.
    #[error("the parameter `{key}` cannot be set at the {scope} level")]
    ScopeNotAllowed {
        /// The key.
        key: String,
        /// The scope name.
        scope: &'static str,
    },
    /// An error from the foundation types.
    #[error(transparent)]
    Core(#[from] CoreError),
}

impl RefDataError {
    /// Shorthand for [`RefDataError::Invalid`].
    pub(crate) fn invalid(what: &'static str, reason: impl Into<String>) -> Self {
        RefDataError::Invalid {
            what,
            reason: reason.into(),
        }
    }
}

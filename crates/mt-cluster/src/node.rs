//! Node identity and protocol version.

use core::fmt;
use serde::{Deserialize, Serialize};

/// Protocol version carried in the handshake. Peers are compatible when `major` is equal;
/// `minor` may differ (rolling upgrades).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProtocolVersion {
    /// Breaking-change counter.
    pub major: u16,
    /// Compatible-change counter.
    pub minor: u16,
}

impl ProtocolVersion {
    /// True when the two versions can talk to each other.
    #[must_use]
    pub fn is_compatible(self, other: Self) -> bool {
        self.major == other.major
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// The protocol version this build speaks.
pub const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion { major: 1, minor: 0 };

/// Identity of one running node instance.
///
/// * `id` is the stable identity (survives restarts; placement hashes it).
/// * `incarnation` identifies the *instance*: it must grow on every process start (use the
///   start time in ms) and is bumped when the node learns it was declared dead ("renew").
///   A higher incarnation supersedes every record of a lower one.
/// * `addr` is where the instance can be reached (opaque to this crate).
///
/// Ordering is `(id, incarnation, addr)`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId {
    /// Stable node identity.
    pub id: u64,
    /// Instance counter; grows on restart and on renew.
    pub incarnation: u64,
    /// Network address (at most [`crate::codec::MAX_ADDR_BYTES`] bytes).
    pub addr: String,
}

impl NodeId {
    /// Creates a node identity.
    #[must_use]
    pub fn new(id: u64, incarnation: u64, addr: impl Into<String>) -> Self {
        Self {
            id,
            incarnation,
            addr: addr.into(),
        }
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}@{}", self.id, self.incarnation, self.addr)
    }
}

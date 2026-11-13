//! The crate's single error type.

use crate::node::ProtocolVersion;

/// Every failure this crate can report. Errors are values; nothing in non-test code panics.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClusterError {
    /// A frame is longer than [`crate::codec::MAX_FRAME_BYTES`].
    #[error("frame of {len} bytes exceeds the limit of {max} bytes")]
    FrameTooLarge {
        /// Actual length.
        len: usize,
        /// Allowed maximum.
        max: usize,
    },
    /// A frame has no bytes at all.
    #[error("empty frame")]
    EmptyFrame,
    /// The first byte of the frame names a wire format this build does not speak.
    #[error("unsupported wire version {0}")]
    UnsupportedWireVersion(u8),
    /// The frame body is not a valid message.
    #[error("malformed frame: {0}")]
    Malformed(String),
    /// A valid message was followed by extra bytes.
    #[error("trailing bytes after message")]
    TrailingBytes,
    /// A decoded or to-be-encoded message breaks a structural limit.
    #[error("limit exceeded: {0}")]
    LimitExceeded(&'static str),
    /// The peer speaks an incompatible protocol version.
    #[error("incompatible protocol: ours {ours}, theirs {theirs}")]
    IncompatibleProtocol {
        /// Our version.
        ours: ProtocolVersion,
        /// The peer's version.
        theirs: ProtocolVersion,
    },
    /// The authentication tag did not verify.
    #[error("authentication failed")]
    AuthFailed,
    /// A handshake message arrived in a state that does not expect it.
    #[error("unexpected handshake message")]
    UnexpectedHandshake,
    /// The peer claims our own stable node id.
    #[error("peer uses our own node id")]
    SelfConnection,
    /// The cluster secret is shorter than [`crate::auth::MIN_SECRET_BYTES`].
    #[error("cluster secret too short")]
    SecretTooShort,
    /// The backing store failed; the operation may be retried.
    #[error("store unavailable: {0}")]
    StoreUnavailable(String),
    /// A compare-and-set lost a race with another node; retry after re-reading.
    #[error("lost a compare-and-set race")]
    Contended,
    /// The caller's epoch / lease term is no longer the current one.
    #[error("fenced: current epoch or term is {current}")]
    Fenced {
        /// The epoch (or lease term) currently recorded in the store.
        current: u64,
    },
    /// The epoch or term counter cannot be incremented any further.
    #[error("epoch counter overflow")]
    EpochOverflow,
    /// The node is draining and refuses to take new ownership.
    #[error("node is draining")]
    Draining,
    /// The lease is held by another node and has not expired.
    #[error("lease held by another node until {until_ms}")]
    LeaseHeld {
        /// Expiry of the current lease (ms).
        until_ms: u64,
    },
    /// The caller's own lease has expired; renew or re-claim before firing.
    #[error("lease expired")]
    LeaseExpired,
    /// The node does not hold the lease or epoch it tried to use.
    #[error("not held")]
    NotHeld,
}

impl ClusterError {
    /// True for [`ClusterError::Fenced`].
    #[must_use]
    pub fn is_fenced(&self) -> bool {
        matches!(self, ClusterError::Fenced { .. })
    }
}

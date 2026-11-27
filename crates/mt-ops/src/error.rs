//! Errors of the codec and of replaying accepted changes.

use thiserror::Error;

/// Something that cannot be decoded or replayed. Refusals of operations are not errors:
/// they are [`crate::Rejection`]s, part of the protocol.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OpsError {
    /// The bytes are not a message.
    #[error("malformed message: {0}")]
    Malformed(String),
    /// The frame is larger than allowed.
    #[error("the message has {bytes} bytes; at most {max} are allowed")]
    TooLarge {
        /// Its size.
        bytes: usize,
        /// The limit.
        max: usize,
    },
    /// The frame was written by an unknown version of the protocol.
    #[error("unknown wire version {0}")]
    UnknownVersion(u8),
    /// A message breaks a structural limit.
    #[error("limit exceeded: {0}")]
    Limit(&'static str),
    /// An accepted change does not follow the last one applied.
    #[error("expected sequence {expected}, got {got}: catch up first")]
    Gap {
        /// The next sequence expected.
        expected: u64,
        /// The sequence received.
        got: u64,
    },
    /// An accepted change does not fit the state it is replayed on.
    #[error("cannot replay sequence {seq}: {reason}")]
    Replay {
        /// Its sequence.
        seq: u64,
        /// Why.
        reason: String,
    },
}

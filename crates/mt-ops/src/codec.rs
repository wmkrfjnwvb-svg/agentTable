//! The binary wire format of the real-time protocol.
//!
//! Frame layout: `[WIRE_VERSION: u8][postcard(message)]`. Values inside travel as their
//! canonical JSON text (see `wire`). Decoding rejects oversized frames, unknown versions,
//! malformed bodies, trailing bytes and messages that break structural limits; it never
//! panics on any input.

use crate::error::OpsError;
use crate::op::{Batch, Effect, Op, Outcome, MAX_CELLS, MAX_OPS};
use serde::{Deserialize, Serialize};

/// Wire format version (first byte of every frame).
pub const WIRE_VERSION: u8 = 1;
/// Largest frame in bytes.
pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
/// Most effects in one accepted batch (a batch of creates with every cell).
pub const MAX_EFFECTS: usize = MAX_OPS * MAX_CELLS;

/// What a client sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientMessage {
    /// A batch to apply.
    Submit(Batch),
    /// Send every accepted batch after this sequence (after a reconnect).
    CatchUp {
        /// The last sequence the client has.
        after: u64,
    },
}

/// What the server sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerMessage {
    /// What became of a batch (accepted batches go to every client of the base; a refusal
    /// only to its sender).
    Outcome(Outcome),
    /// The client asked for batches the server no longer has: it must load a snapshot.
    Reload {
        /// The oldest sequence still available.
        oldest: u64,
    },
}

fn check_ops(ops: &[Op]) -> Result<(), OpsError> {
    if ops.len() > MAX_OPS {
        return Err(OpsError::Limit("too many operations"));
    }
    for op in ops {
        if let Op::Create { cells, .. } = op {
            if cells.len() > MAX_CELLS {
                return Err(OpsError::Limit("too many cells"));
            }
        }
    }
    Ok(())
}

fn check_effects(effects: &[Effect]) -> Result<(), OpsError> {
    if effects.len() > MAX_EFFECTS {
        return Err(OpsError::Limit("too many effects"));
    }
    for effect in effects {
        if let Effect::Created { cells, .. } = effect {
            if cells.len() > MAX_CELLS {
                return Err(OpsError::Limit("too many cells"));
            }
        }
    }
    Ok(())
}

fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, OpsError> {
    let bytes = postcard::to_extend(message, vec![WIRE_VERSION])
        .map_err(|e| OpsError::Malformed(e.to_string()))?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(OpsError::TooLarge {
            bytes: bytes.len(),
            max: MAX_FRAME_BYTES,
        });
    }
    Ok(bytes)
}

fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, OpsError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(OpsError::TooLarge {
            bytes: bytes.len(),
            max: MAX_FRAME_BYTES,
        });
    }
    let (version, body) = bytes
        .split_first()
        .ok_or_else(|| OpsError::Malformed("empty frame".to_owned()))?;
    if *version != WIRE_VERSION {
        return Err(OpsError::UnknownVersion(*version));
    }
    let (message, rest) =
        postcard::take_from_bytes::<T>(body).map_err(|e| OpsError::Malformed(e.to_string()))?;
    if !rest.is_empty() {
        return Err(OpsError::Malformed("trailing bytes".to_owned()));
    }
    Ok(message)
}

/// Encodes a client message.
///
/// # Errors
/// [`OpsError::Limit`] for a message no server would take; [`OpsError::TooLarge`].
pub fn encode_client(message: &ClientMessage) -> Result<Vec<u8>, OpsError> {
    match message {
        ClientMessage::Submit(batch) => check_ops(&batch.ops)?,
        ClientMessage::CatchUp { .. } => {}
    }
    encode(message)
}

/// Decodes a client message.
///
/// # Errors
/// Any [`OpsError`] of the codec.
pub fn decode_client(bytes: &[u8]) -> Result<ClientMessage, OpsError> {
    let message: ClientMessage = decode(bytes)?;
    match &message {
        ClientMessage::Submit(batch) => check_ops(&batch.ops)?,
        ClientMessage::CatchUp { .. } => {}
    }
    Ok(message)
}

/// Encodes a server message.
///
/// # Errors
/// [`OpsError::Limit`]; [`OpsError::TooLarge`].
pub fn encode_server(message: &ServerMessage) -> Result<Vec<u8>, OpsError> {
    check_server(message)?;
    encode(message)
}

/// Decodes a server message.
///
/// # Errors
/// Any [`OpsError`] of the codec.
pub fn decode_server(bytes: &[u8]) -> Result<ServerMessage, OpsError> {
    let message: ServerMessage = decode(bytes)?;
    check_server(&message)?;
    Ok(message)
}

fn check_server(message: &ServerMessage) -> Result<(), OpsError> {
    match message {
        ServerMessage::Outcome(Outcome::Accepted(accepted)) => check_effects(&accepted.effects),
        ServerMessage::Outcome(Outcome::Rejected(_)) | ServerMessage::Reload { .. } => Ok(()),
    }
}

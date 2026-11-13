//! Versioned binary codec with strict limits.
//!
//! Frame layout: `[WIRE_VERSION: u8][postcard(Message)]`. Decoding rejects oversized frames,
//! unknown versions, malformed bodies, trailing bytes and messages that break structural
//! limits. It never panics on any input.

use crate::error::ClusterError;
use crate::message::{Hello, Message, Update};
use crate::node::NodeId;

/// Wire format version (first byte of every frame).
pub const WIRE_VERSION: u8 = 1;
/// Maximum encoded frame size in bytes.
pub const MAX_FRAME_BYTES: usize = 256 * 1024;
/// Maximum number of membership updates in one message.
pub const MAX_UPDATES: usize = 1024;
/// Maximum length of a node address in bytes.
pub const MAX_ADDR_BYTES: usize = 128;
/// Maximum placement weight of a node.
pub const MAX_WEIGHT: u16 = 64;

fn check_node(node: &NodeId) -> Result<(), ClusterError> {
    if node.addr.is_empty() {
        return Err(ClusterError::LimitExceeded("empty node address"));
    }
    if node.addr.len() > MAX_ADDR_BYTES {
        return Err(ClusterError::LimitExceeded("node address too long"));
    }
    Ok(())
}

fn check_updates(updates: &[Update]) -> Result<(), ClusterError> {
    if updates.len() > MAX_UPDATES {
        return Err(ClusterError::LimitExceeded("too many updates"));
    }
    for u in updates {
        check_node(&u.node)?;
        if u.weight > MAX_WEIGHT {
            return Err(ClusterError::LimitExceeded("weight too large"));
        }
    }
    Ok(())
}

fn check_hello(h: &Hello) -> Result<(), ClusterError> {
    check_node(&h.node)
}

/// Checks the structural limits of a message (applied on both encode and decode).
pub fn validate(msg: &Message) -> Result<(), ClusterError> {
    match msg {
        Message::Hello(h) | Message::HelloAck(h) => check_hello(h),
        Message::HelloConfirm { tag: _ } => Ok(()),
        Message::Ping { seq: _, updates }
        | Message::Ack { seq: _, updates }
        | Message::Sync {
            want_reply: _,
            updates,
        } => check_updates(updates),
        Message::PingReq {
            seq: _,
            target,
            updates,
        } => {
            check_node(target)?;
            check_updates(updates)
        }
    }
}

/// Encodes a message into a frame.
pub fn encode(msg: &Message) -> Result<Vec<u8>, ClusterError> {
    validate(msg)?;
    let frame = postcard::to_extend(msg, vec![WIRE_VERSION])
        .map_err(|e| ClusterError::Malformed(e.to_string()))?;
    if frame.len() > MAX_FRAME_BYTES {
        return Err(ClusterError::FrameTooLarge {
            len: frame.len(),
            max: MAX_FRAME_BYTES,
        });
    }
    Ok(frame)
}

/// Decodes one frame into a message, enforcing every limit.
pub fn decode(frame: &[u8]) -> Result<Message, ClusterError> {
    if frame.len() > MAX_FRAME_BYTES {
        return Err(ClusterError::FrameTooLarge {
            len: frame.len(),
            max: MAX_FRAME_BYTES,
        });
    }
    let (version, body) = frame.split_first().ok_or(ClusterError::EmptyFrame)?;
    if *version != WIRE_VERSION {
        return Err(ClusterError::UnsupportedWireVersion(*version));
    }
    let (msg, rest) = postcard::take_from_bytes::<Message>(body)
        .map_err(|e| ClusterError::Malformed(e.to_string()))?;
    if !rest.is_empty() {
        return Err(ClusterError::TrailingBytes);
    }
    validate(&msg)?;
    Ok(msg)
}

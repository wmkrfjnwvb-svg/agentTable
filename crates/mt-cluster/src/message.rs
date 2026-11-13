//! Wire messages.
//!
//! The enum variant order and field order are part of the wire format (postcard encodes
//! variants by index). Only append new variants; never reorder.

use crate::node::{NodeId, ProtocolVersion};
use serde::{Deserialize, Serialize};

/// State of a member as seen by some node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MemberState {
    /// Believed healthy.
    Alive,
    /// A probe failed; will be declared dead unless it refutes in time.
    Suspect,
    /// Declared failed. Final for this incarnation.
    Dead,
    /// Left on purpose. Final for this incarnation.
    Left,
}

impl MemberState {
    /// Alive or Suspect: the node still counts as a cluster member for placement.
    #[must_use]
    pub fn is_active(self) -> bool {
        match self {
            MemberState::Alive | MemberState::Suspect => true,
            MemberState::Dead | MemberState::Left => false,
        }
    }

    /// Rank among terminal states (0 = not terminal).
    pub(crate) fn terminal_rank(self) -> u8 {
        match self {
            MemberState::Alive | MemberState::Suspect => 0,
            MemberState::Dead => 1,
            MemberState::Left => 2,
        }
    }
}

/// One membership fact, disseminated by gossip.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Update {
    /// The node the fact is about.
    pub node: NodeId,
    /// Refutation counter within one incarnation (only the node itself raises it).
    pub version: u64,
    /// Claimed state.
    pub state: MemberState,
    /// Placement weight of the node (0 = owns nothing).
    pub weight: u16,
}

impl Update {
    /// Total order used to merge facts about the same stable id; the greater fact wins.
    /// `(incarnation, terminal rank, version, state, weight, addr)`.
    pub(crate) fn precedence(&self) -> (u64, u8, u64, MemberState, u16, &str) {
        (
            self.node.incarnation,
            self.state.terminal_rank(),
            self.version,
            self.state,
            self.weight,
            self.node.addr.as_str(),
        )
    }
}

/// First handshake message (initiator → responder), also reused as the acknowledgement body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// Sender's protocol version.
    pub protocol: ProtocolVersion,
    /// Sender's identity.
    pub node: NodeId,
    /// Fresh random nonce chosen by the sender.
    pub nonce: [u8; 16],
    /// HMAC-SHA256 tag (see [`crate::auth`]).
    pub tag: [u8; 32],
}

/// Everything nodes send to each other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message {
    /// Handshake step 1.
    Hello(Hello),
    /// Handshake step 2; its tag also covers the initiator's nonce.
    HelloAck(Hello),
    /// Handshake step 3; proves the initiator saw the responder's nonce.
    HelloConfirm {
        /// HMAC-SHA256 tag.
        tag: [u8; 32],
    },
    /// Direct probe.
    Ping {
        /// Probe sequence number, echoed in the `Ack`.
        seq: u64,
        /// Piggybacked membership facts.
        updates: Vec<Update>,
    },
    /// Answer to a `Ping` (possibly relayed for an indirect probe).
    Ack {
        /// Sequence number of the probe being answered.
        seq: u64,
        /// Piggybacked membership facts.
        updates: Vec<Update>,
    },
    /// Request to probe `target` on the sender's behalf.
    PingReq {
        /// Sequence number the requester waits for.
        seq: u64,
        /// Node to probe.
        target: NodeId,
        /// Piggybacked membership facts.
        updates: Vec<Update>,
    },
    /// Anti-entropy / join / leave: a batch of facts, optionally asking for the peer's state.
    Sync {
        /// When true the receiver answers with its full member list.
        want_reply: bool,
        /// Membership facts.
        updates: Vec<Update>,
    },
}

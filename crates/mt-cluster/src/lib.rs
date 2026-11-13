//! `mt-cluster`: the sans-IO clustering core for a single-binary, leaderless N-to-N cluster.
//!
//! Everything here is a pure state machine: time, randomness (seeds / nonces) and the network
//! are inputs and outputs, so a deterministic simulator (see the `sim` feature) or a real
//! transport can drive the same code. See `README.md` for the design, invariants and the
//! known weaknesses of the protocol.
#![forbid(unsafe_code)]

pub mod auth;
pub mod codec;
pub mod error;
pub mod fencing;
pub mod hash;
pub mod membership;
pub mod message;
pub mod node;
pub mod placement;
pub mod rng;
pub mod routing;
pub mod scheduler;
#[cfg(feature = "sim")]
pub mod sim;
pub mod store;

pub use auth::{ClusterSecret, Handshake};
pub use codec::{decode, encode};
pub use error::ClusterError;
pub use fencing::{check_token, EpochRecord, EpochStore, Fencer, FencingToken};
pub use membership::{Config, Event, Input, Member, Membership, Output};
pub use message::{MemberState, Message, Update};
pub use node::{NodeId, ProtocolVersion, PROTOCOL_VERSION};
pub use placement::{moved_keys, KeyMove, Placement};
pub use routing::{FanOut, Route, RoutingTable};
pub use scheduler::{
    FireWindow, LeaseRecord, LeaseStore, LeaseToken, SchedEvent, SchedulerConfig, ShardScheduler,
};
pub use store::{MemoryEpochStore, MemoryLeaseStore, MemoryStore};

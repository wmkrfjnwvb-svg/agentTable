//! `mt-ops`: the operation model of a base — what clients send, how the base decides it, and
//! how clients keep a copy that converges.
//!
//! * [`Op`] and [`Batch`]: typed changes of records; a batch applies all or nothing.
//! * [`ClientOpId`]: a client's own increasing number for each batch. A batch received twice
//!   is recognised and gets the same outcome without applying again.
//! * [`BaseState::step`]: the decision, without I/O (the base actor runs it; storage commits
//!   the result). Accepted batches get the base's next sequence number (1, 2, 3, … no
//!   gaps), which orders the journal, the audit and what clients receive.
//! * Conflicts are per cell: a `Set` made on a cell version the client had not seen still
//!   applies — the last writer wins — and its effect names the overwritten value so the
//!   client can be told. Deliberate changes (`Transition`, `Delete`) are refused on a stale
//!   record version instead.
//! * [`Replica`]: a client's copy (confirmed state + pending batches, rebased on every
//!   change from the server).
//! * [`codec`]: the binary frames (postcard), with strict limits.
//!
//! Pure: no clocks, sockets or randomness; builds for `wasm32-unknown-unknown`.
#![forbid(unsafe_code)]

pub mod codec;
pub mod error;
pub mod op;
pub mod replica;
pub mod state;
pub mod wire;

pub use codec::{ClientMessage, ServerMessage};
pub use error::OpsError;
pub use op::{
    Accepted, Batch, ClientId, ClientOpId, Effect, Op, Outcome, Reason, Rejection, MAX_CELLS,
    MAX_OPS,
};
pub use replica::Replica;
pub use state::{BaseState, CellState, Decision, RecordState, DEDUP_WINDOW};

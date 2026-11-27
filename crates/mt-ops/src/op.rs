//! Operations: what a client asks to change, and what became of it.

use mt_core::id::{BaseId, FieldId, RecordId, TableId};
use mt_core::value::Value;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Most operations in one batch (also the largest synchronous import).
pub const MAX_OPS: usize = 500;
/// Most cells in one created record (the product's limit of fields per table).
pub const MAX_CELLS: usize = 400;

/// A connected client (a browser tab, an API session, an agent run). The server assigns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ClientId(pub u64);

/// A client's own number for a batch: unique per client and increasing, so a batch sent
/// twice (after a reconnect, say) is recognised and applied once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ClientOpId {
    /// The client.
    pub client: ClientId,
    /// Its batch number (from 1).
    pub seq: u64,
}

/// One change to the records of a base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    /// A new record with its first values (empty values are left out).
    Create {
        /// The table.
        table: TableId,
        /// The new record's id (chosen by the client, so it can refer to it at once).
        record: RecordId,
        /// Its values.
        #[serde(with = "crate::wire::cells")]
        cells: BTreeMap<FieldId, Value>,
    },
    /// A new value for one cell (`Null` empties it). `expected` is the cell version the
    /// client saw (0 for a cell never set). A stale version still applies — the last writer
    /// wins — and the result names the value that was overwritten.
    Set {
        /// The table.
        table: TableId,
        /// The record.
        record: RecordId,
        /// The field.
        field: FieldId,
        /// The new value.
        #[serde(with = "crate::wire::value")]
        value: Value,
        /// The cell version the client saw.
        expected: u64,
    },
    /// A deliberate change of one cell — a lifecycle transition — that applies only to the
    /// record version the client saw; otherwise the batch is refused.
    Transition {
        /// The table.
        table: TableId,
        /// The record.
        record: RecordId,
        /// The state field.
        field: FieldId,
        /// The new value.
        #[serde(with = "crate::wire::value")]
        value: Value,
        /// The record version the client saw.
        expected: u64,
    },
    /// Moves a record to the trash; refused unless the client saw its current version.
    Delete {
        /// The table.
        table: TableId,
        /// The record.
        record: RecordId,
        /// The record version the client saw.
        expected: u64,
    },
    /// Takes a record out of the trash.
    Restore {
        /// The table.
        table: TableId,
        /// The record.
        record: RecordId,
    },
}

impl Op {
    /// The record the operation is about.
    #[must_use]
    pub const fn target(&self) -> (TableId, RecordId) {
        match self {
            Op::Create { table, record, .. }
            | Op::Set { table, record, .. }
            | Op::Transition { table, record, .. }
            | Op::Delete { table, record, .. }
            | Op::Restore { table, record } => (*table, *record),
        }
    }
}

/// Operations a client sends together: they apply all, or none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Batch {
    /// The client's number for it.
    pub id: ClientOpId,
    /// The base it changes.
    pub base: BaseId,
    /// The operations, applied in order.
    pub ops: Vec<Op>,
}

/// One change that was made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// A record was created.
    Created {
        /// The table.
        table: TableId,
        /// The record.
        record: RecordId,
        /// Its values.
        #[serde(with = "crate::wire::cells")]
        cells: BTreeMap<FieldId, Value>,
    },
    /// A cell changed.
    Set {
        /// The table.
        table: TableId,
        /// The record.
        record: RecordId,
        /// The field.
        field: FieldId,
        /// The value before.
        #[serde(with = "crate::wire::value")]
        before: Value,
        /// The value after.
        #[serde(with = "crate::wire::value")]
        after: Value,
        /// When the client had not seen the latest value: the value its change replaced
        /// without the client knowing it (someone else's change).
        #[serde(with = "crate::wire::option")]
        overwritten: Option<Value>,
    },
    /// A record went to the trash.
    Deleted {
        /// The table.
        table: TableId,
        /// The record.
        record: RecordId,
    },
    /// A record came back from the trash.
    Restored {
        /// The table.
        table: TableId,
        /// The record.
        record: RecordId,
    },
}

/// A batch that was applied: its place in the base's order and what it changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accepted {
    /// The batch.
    pub id: ClientOpId,
    /// Its sequence number in the base (1, 2, 3, … without gaps).
    pub seq: u64,
    /// What changed, in order.
    pub effects: Vec<Effect>,
}

/// Why a batch was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// The batch is for another base.
    WrongBase,
    /// The batch has no operations.
    EmptyBatch,
    /// The batch has more than [`MAX_OPS`] operations.
    TooManyOps,
    /// A created record has more than [`MAX_CELLS`] values.
    TooManyCells,
    /// The batch number is not newer than the client's last one, and too old to be
    /// recognised: the client must catch up and send its changes again.
    TooOld {
        /// The client's newest batch number seen.
        newest: u64,
    },
    /// A record with this id exists already.
    RecordExists,
    /// There is no such record.
    UnknownRecord,
    /// The record is in the trash.
    RecordDeleted,
    /// The record is not in the trash.
    NotDeleted,
    /// The record changed since the client saw it.
    StaleRecord {
        /// The version the client saw.
        expected: u64,
        /// The current version.
        actual: u64,
    },
    /// The base has used every sequence number.
    SequenceExhausted,
}

impl Reason {
    /// The reason in words.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Reason::WrongBase => "the change is for another base".to_owned(),
            Reason::EmptyBatch => "there is nothing to change".to_owned(),
            Reason::TooManyOps => format!("at most {MAX_OPS} changes go together"),
            Reason::TooManyCells => format!("a record has at most {MAX_CELLS} values"),
            Reason::TooOld { newest } => format!(
                "this change is older than the last one received from this client (#{newest}): reload and try again"
            ),
            Reason::RecordExists => "the record exists already".to_owned(),
            Reason::UnknownRecord => "the record does not exist".to_owned(),
            Reason::RecordDeleted => "the record is in the trash".to_owned(),
            Reason::NotDeleted => "the record is not in the trash".to_owned(),
            Reason::StaleRecord { expected, actual } => format!(
                "the record changed since you saw it (version {expected}, now {actual}): look again first"
            ),
            Reason::SequenceExhausted => "the base cannot take more changes".to_owned(),
        }
    }
}

/// A batch that was refused; nothing of it applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejection {
    /// The batch.
    pub id: ClientOpId,
    /// The operation that could not apply (by position), if one.
    pub op: Option<usize>,
    /// Why.
    pub reason: Reason,
}

/// What became of a batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// It applied.
    Accepted(Accepted),
    /// It was refused.
    Rejected(Rejection),
}

impl Outcome {
    /// The batch it is about.
    #[must_use]
    pub const fn id(&self) -> ClientOpId {
        match self {
            Outcome::Accepted(a) => a.id,
            Outcome::Rejected(r) => r.id,
        }
    }
}

//! The state of one base as the operation pipeline sees it: records with a version per
//! record and per cell, the base's sequence, and the batches recently decided.
//!
//! [`BaseState::step`] is the whole decision for one batch, without I/O: recognise a batch
//! already decided, check its shape, apply every operation to a working copy, and commit
//! all of it with the next sequence number — or none of it.

use crate::error::OpsError;
use crate::op::{
    Accepted, Batch, ClientId, ClientOpId, Effect, Op, Outcome, Reason, Rejection, MAX_CELLS,
    MAX_OPS,
};
use mt_core::id::{BaseId, FieldId, RecordId, TableId};
use mt_core::value::Value;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

/// How many decided batches are remembered for recognising repeats.
pub const DEDUP_WINDOW: usize = 4096;

/// One cell: its value and the sequence of the batch that set it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellState {
    /// The value (never `Null`: an empty cell is absent).
    #[serde(with = "crate::wire::value")]
    pub value: Value,
    /// The sequence of the batch that set it.
    pub version: u64,
}

/// One record.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RecordState {
    /// Its cells.
    pub cells: BTreeMap<FieldId, CellState>,
    /// The sequence of the batch that changed it last.
    pub version: u64,
    /// The sequence of the batch that created it.
    pub created: u64,
    /// Whether it is in the trash.
    pub deleted: bool,
}

impl RecordState {
    /// A cell's value (`Null` when empty).
    #[must_use]
    pub fn value(&self, field: FieldId) -> Value {
        self.cells
            .get(&field)
            .map_or(Value::Null, |cell| cell.value.clone())
    }

    /// A cell's version (0 when never set).
    #[must_use]
    pub fn cell_version(&self, field: FieldId) -> u64 {
        self.cells.get(&field).map_or(0, |cell| cell.version)
    }
}

/// The result of [`BaseState::step`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// What became of the batch.
    pub outcome: Outcome,
    /// `true` when the batch had been decided before: nothing changed now, and the outcome
    /// is the one given then.
    pub repeated: bool,
}

/// The records of one base, their versions, and what was recently decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseState {
    base: BaseId,
    seq: u64,
    records: BTreeMap<(TableId, RecordId), RecordState>,
    recent: VecDeque<ClientOpId>,
    outcomes: BTreeMap<ClientOpId, Outcome>,
    newest: BTreeMap<ClientId, u64>,
}

/// Records changed by a batch that is being applied.
struct Work<'a> {
    state: &'a BaseState,
    seq: u64,
    touched: BTreeMap<(TableId, RecordId), RecordState>,
    effects: Vec<Effect>,
}

impl Work<'_> {
    /// The record as it is now in the batch (changed earlier in it, or as committed).
    fn current(&self, key: (TableId, RecordId)) -> Option<&RecordState> {
        self.touched
            .get(&key)
            .or_else(|| self.state.records.get(&key))
    }

    fn current_mut(&mut self, key: (TableId, RecordId)) -> Option<&mut RecordState> {
        BaseState::touch(&self.state.records, &mut self.touched, key)
    }

    /// The record as it was before the batch (what the client could have seen).
    fn before(&self, key: (TableId, RecordId)) -> Option<&RecordState> {
        self.state.records.get(&key)
    }

    fn apply(&mut self, op: &Op) -> Result<(), Reason> {
        let key = op.target();
        let (table, record) = key;
        let seq = self.seq;
        match op {
            Op::Create { cells, .. } => {
                if cells.len() > MAX_CELLS {
                    return Err(Reason::TooManyCells);
                }
                if self.current(key).is_some() {
                    return Err(Reason::RecordExists);
                }
                let kept: BTreeMap<FieldId, Value> = cells
                    .iter()
                    .filter(|(_, value)| !value.is_empty())
                    .map(|(field, value)| (*field, value.clone()))
                    .collect();
                let state = RecordState {
                    cells: kept
                        .iter()
                        .map(|(field, value)| {
                            (
                                *field,
                                CellState {
                                    value: value.clone(),
                                    version: seq,
                                },
                            )
                        })
                        .collect(),
                    version: seq,
                    created: seq,
                    deleted: false,
                };
                self.touched.insert(key, state);
                self.effects.push(Effect::Created {
                    table,
                    record,
                    cells: kept,
                });
                Ok(())
            }
            Op::Set {
                field,
                value,
                expected,
                ..
            } => self.set(key, *field, value, Some(*expected), None),
            Op::Transition {
                field,
                value,
                expected,
                ..
            } => self.set(key, *field, value, None, Some(*expected)),
            Op::Delete { expected, .. } => {
                let actual = self.before(key).map_or(0, |r| r.version);
                let rec = self.current_mut(key).ok_or(Reason::UnknownRecord)?;
                if rec.deleted {
                    return Err(Reason::RecordDeleted);
                }
                if *expected != actual {
                    return Err(Reason::StaleRecord {
                        expected: *expected,
                        actual,
                    });
                }
                rec.deleted = true;
                rec.version = seq;
                self.effects.push(Effect::Deleted { table, record });
                Ok(())
            }
            Op::Restore { .. } => {
                let rec = self.current_mut(key).ok_or(Reason::UnknownRecord)?;
                if !rec.deleted {
                    return Err(Reason::NotDeleted);
                }
                rec.deleted = false;
                rec.version = seq;
                self.effects.push(Effect::Restored { table, record });
                Ok(())
            }
        }
    }

    /// A cell change: `cell_seen` for a set (last writer wins, overwrites reported),
    /// `record_seen` for a transition (refused on a stale record).
    fn set(
        &mut self,
        key: (TableId, RecordId),
        field: FieldId,
        value: &Value,
        cell_seen: Option<u64>,
        record_seen: Option<u64>,
    ) -> Result<(), Reason> {
        let (table, record) = key;
        let seq = self.seq;
        let before_batch = self.before(key).cloned();
        if let Some(expected) = record_seen {
            let actual = before_batch.as_ref().map_or(0, |r| r.version);
            if expected != actual {
                return Err(Reason::StaleRecord { expected, actual });
            }
        }
        let rec = self.current_mut(key).ok_or(Reason::UnknownRecord)?;
        if rec.deleted {
            return Err(Reason::RecordDeleted);
        }
        let before = rec.value(field);
        if before == *value {
            return Ok(());
        }
        if value.is_empty() {
            rec.cells.remove(&field);
        } else {
            rec.cells.insert(
                field,
                CellState {
                    value: value.clone(),
                    version: seq,
                },
            );
        }
        rec.version = seq;
        // A stale cell version: the client replaces a value it never saw.
        let overwritten = cell_seen.and_then(|expected| {
            let seen = before_batch.as_ref().map_or(0, |r| r.cell_version(field));
            (expected != seen).then(|| {
                before_batch
                    .as_ref()
                    .map_or(Value::Null, |r| r.value(field))
            })
        });
        self.effects.push(Effect::Set {
            table,
            record,
            field,
            before,
            after: value.clone(),
            overwritten,
        });
        Ok(())
    }
}

impl BaseState {
    /// An empty base at sequence 0.
    #[must_use]
    pub fn new(base: BaseId) -> Self {
        BaseState {
            base,
            seq: 0,
            records: BTreeMap::new(),
            recent: VecDeque::new(),
            outcomes: BTreeMap::new(),
            newest: BTreeMap::new(),
        }
    }

    /// A base loaded from storage: its records at a sequence. Decided batches are learnt
    /// again by replaying the journal tail with [`BaseState::replay`] (or are unknown).
    #[must_use]
    pub fn from_records(
        base: BaseId,
        seq: u64,
        records: BTreeMap<(TableId, RecordId), RecordState>,
    ) -> Self {
        BaseState {
            records,
            seq,
            ..BaseState::new(base)
        }
    }

    /// The base.
    #[must_use]
    pub const fn base(&self) -> BaseId {
        self.base
    }

    /// The sequence of the last applied batch (0 before the first).
    #[must_use]
    pub const fn seq(&self) -> u64 {
        self.seq
    }

    /// A record (also when in the trash).
    #[must_use]
    pub fn record(&self, table: TableId, record: RecordId) -> Option<&RecordState> {
        self.records.get(&(table, record))
    }

    /// Every record, in id order.
    pub fn records(&self) -> impl Iterator<Item = (&(TableId, RecordId), &RecordState)> {
        self.records.iter()
    }

    /// The outcome given to a batch, if it is still remembered.
    #[must_use]
    pub fn outcome(&self, id: ClientOpId) -> Option<&Outcome> {
        self.outcomes.get(&id)
    }

    /// Learns a decision made earlier (read back from the journal after a restart) without
    /// applying anything, so the batch is recognised if it is sent again.
    pub fn recall(&mut self, outcome: Outcome) {
        self.remember(outcome);
    }

    fn remember(&mut self, outcome: Outcome) {
        let id = outcome.id();
        let newest = self.newest.entry(id.client).or_insert(0);
        *newest = (*newest).max(id.seq);
        if self.outcomes.insert(id, outcome).is_none() {
            self.recent.push_back(id);
        }
        while self.recent.len() > DEDUP_WINDOW {
            if let Some(old) = self.recent.pop_front() {
                self.outcomes.remove(&old);
            }
        }
    }

    fn reject(id: ClientOpId, op: Option<usize>, reason: Reason) -> Outcome {
        Outcome::Rejected(Rejection { id, op, reason })
    }

    /// Decides one batch. A batch decided before gets the same outcome again and changes
    /// nothing; otherwise all its operations apply with the next sequence number, or none.
    pub fn step(&mut self, batch: &Batch) -> Decision {
        let id = batch.id;
        if let Some(outcome) = self.outcomes.get(&id) {
            return Decision {
                outcome: outcome.clone(),
                repeated: true,
            };
        }
        // Not remembered: a refusal that is not about the batch itself is not remembered
        // either (the batch may be sent where it belongs, or again after catching up).
        if batch.base != self.base {
            return Decision {
                outcome: Self::reject(id, None, Reason::WrongBase),
                repeated: false,
            };
        }
        if let Some(newest) = self.newest.get(&id.client).copied() {
            if id.seq <= newest {
                return Decision {
                    outcome: Self::reject(id, None, Reason::TooOld { newest }),
                    repeated: false,
                };
            }
        }
        let outcome = self.decide(batch);
        if let Outcome::Accepted(accepted) = &outcome {
            self.seq = accepted.seq;
        }
        self.remember(outcome.clone());
        Decision {
            outcome,
            repeated: false,
        }
    }

    fn decide(&mut self, batch: &Batch) -> Outcome {
        let id = batch.id;
        if batch.ops.is_empty() {
            return Self::reject(id, None, Reason::EmptyBatch);
        }
        if batch.ops.len() > MAX_OPS {
            return Self::reject(id, None, Reason::TooManyOps);
        }
        let Some(seq) = self.seq.checked_add(1) else {
            return Self::reject(id, None, Reason::SequenceExhausted);
        };
        let mut work = Work {
            state: self,
            seq,
            touched: BTreeMap::new(),
            effects: Vec::new(),
        };
        for (index, op) in batch.ops.iter().enumerate() {
            if let Err(reason) = work.apply(op) {
                return Self::reject(id, Some(index), reason);
            }
        }
        let Work {
            touched, effects, ..
        } = work;
        self.records.extend(touched);
        Outcome::Accepted(Accepted { id, seq, effects })
    }

    /// Applies a batch accepted elsewhere — by the base's owner, read back from the
    /// journal, or received by a client — without deciding it again. It must be the next
    /// in sequence.
    ///
    /// # Errors
    /// [`OpsError::Gap`] when it is not the next; [`OpsError::Replay`] when its effects do
    /// not fit the records (the state then is unchanged).
    pub fn replay(&mut self, accepted: &Accepted) -> Result<(), OpsError> {
        let expected = self.seq.checked_add(1).ok_or(OpsError::Gap {
            expected: u64::MAX,
            got: accepted.seq,
        })?;
        if accepted.seq != expected {
            return Err(OpsError::Gap {
                expected,
                got: accepted.seq,
            });
        }
        let seq = accepted.seq;
        let fail = |reason: &str| OpsError::Replay {
            seq,
            reason: reason.to_owned(),
        };
        let mut touched: BTreeMap<(TableId, RecordId), RecordState> = BTreeMap::new();
        for effect in &accepted.effects {
            match effect {
                Effect::Created {
                    table,
                    record,
                    cells,
                } => {
                    let key = (*table, *record);
                    if touched.contains_key(&key) || self.records.contains_key(&key) {
                        return Err(fail("the record exists already"));
                    }
                    touched.insert(
                        key,
                        RecordState {
                            cells: cells
                                .iter()
                                .filter(|(_, v)| !v.is_empty())
                                .map(|(f, v)| {
                                    (
                                        *f,
                                        CellState {
                                            value: v.clone(),
                                            version: seq,
                                        },
                                    )
                                })
                                .collect(),
                            version: seq,
                            created: seq,
                            deleted: false,
                        },
                    );
                }
                Effect::Set {
                    table,
                    record,
                    field,
                    after,
                    ..
                } => {
                    let rec = Self::touch(&self.records, &mut touched, (*table, *record))
                        .ok_or_else(|| fail("the record does not exist"))?;
                    if after.is_empty() {
                        rec.cells.remove(field);
                    } else {
                        rec.cells.insert(
                            *field,
                            CellState {
                                value: after.clone(),
                                version: seq,
                            },
                        );
                    }
                    rec.version = seq;
                }
                Effect::Deleted { table, record } | Effect::Restored { table, record } => {
                    let deleted = matches!(effect, Effect::Deleted { .. });
                    let rec = Self::touch(&self.records, &mut touched, (*table, *record))
                        .ok_or_else(|| fail("the record does not exist"))?;
                    if rec.deleted == deleted {
                        return Err(fail(if deleted {
                            "the record is in the trash already"
                        } else {
                            "the record is not in the trash"
                        }));
                    }
                    rec.deleted = deleted;
                    rec.version = seq;
                }
            }
        }
        self.records.extend(touched);
        self.seq = seq;
        self.remember(Outcome::Accepted(accepted.clone()));
        Ok(())
    }

    fn touch<'t>(
        records: &BTreeMap<(TableId, RecordId), RecordState>,
        touched: &'t mut BTreeMap<(TableId, RecordId), RecordState>,
        key: (TableId, RecordId),
    ) -> Option<&'t mut RecordState> {
        match touched.entry(key) {
            std::collections::btree_map::Entry::Occupied(entry) => Some(entry.into_mut()),
            std::collections::btree_map::Entry::Vacant(entry) => {
                Some(entry.insert(records.get(&key)?.clone()))
            }
        }
    }

    /// Applies operations optimistically, as a client shows its own changes before the
    /// server answers: like [`BaseState::step`], without remembering the batch. Returns
    /// whether they applied.
    pub fn apply_local(&mut self, batch: &Batch) -> bool {
        match self.decide(batch) {
            Outcome::Accepted(accepted) => {
                self.seq = accepted.seq;
                true
            }
            Outcome::Rejected(_) => false,
        }
    }
}

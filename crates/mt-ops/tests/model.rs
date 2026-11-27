//! Model-based test: the base state machine against a naive reference written from the
//! rules, over random batches from several clients — including repeats and stale numbers.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_core::id::{FieldId, RecordId, TableId};
use mt_core::value::Value;
use mt_ops::{BaseState, Batch, ClientOpId, Effect, Op, Outcome, Reason};
use proptest::prelude::*;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Default)]
struct Rec {
    cells: BTreeMap<FieldId, (Value, u64)>,
    version: u64,
    created: u64,
    deleted: bool,
}

type Records = BTreeMap<(TableId, RecordId), Rec>;
type Decided = Result<u64, (Option<usize>, Reason)>;

#[derive(Debug, Clone, Default)]
struct Model {
    seq: u64,
    records: Records,
    decided: BTreeMap<ClientOpId, Decided>,
    newest: BTreeMap<u64, u64>,
}

impl Model {
    /// The rules, written plainly: work on a copy of everything, keep it only on success.
    fn apply(&self, ops: &[Op], seq: u64) -> Result<Records, (Option<usize>, Reason)> {
        let mut records = self.records.clone();
        for (i, op) in ops.iter().enumerate() {
            let key = op.target();
            let before = self.records.get(&key).cloned();
            let err = |r| Err((Some(i), r));
            match op {
                Op::Create { cells, .. } => {
                    if records.contains_key(&key) {
                        return err(Reason::RecordExists);
                    }
                    let cells = cells
                        .iter()
                        .filter(|(_, v)| !v.is_empty())
                        .map(|(f, v)| (*f, (v.clone(), seq)))
                        .collect();
                    records.insert(
                        key,
                        Rec {
                            cells,
                            version: seq,
                            created: seq,
                            deleted: false,
                        },
                    );
                }
                Op::Set { field, value, .. } | Op::Transition { field, value, .. } => {
                    if let Op::Transition { expected, .. } = op {
                        let actual = before.as_ref().map_or(0, |r| r.version);
                        if *expected != actual {
                            return err(Reason::StaleRecord {
                                expected: *expected,
                                actual,
                            });
                        }
                    }
                    let Some(rec) = records.get_mut(&key) else {
                        return err(Reason::UnknownRecord);
                    };
                    if rec.deleted {
                        return err(Reason::RecordDeleted);
                    }
                    let current = rec.cells.get(field).map_or(Value::Null, |c| c.0.clone());
                    if current != *value {
                        if value.is_empty() {
                            rec.cells.remove(field);
                        } else {
                            rec.cells.insert(*field, (value.clone(), seq));
                        }
                        rec.version = seq;
                    }
                }
                Op::Delete { expected, .. } => {
                    let actual = before.as_ref().map_or(0, |r| r.version);
                    let Some(rec) = records.get_mut(&key) else {
                        return err(Reason::UnknownRecord);
                    };
                    if rec.deleted {
                        return err(Reason::RecordDeleted);
                    }
                    if *expected != actual {
                        return err(Reason::StaleRecord {
                            expected: *expected,
                            actual,
                        });
                    }
                    rec.deleted = true;
                    rec.version = seq;
                }
                Op::Restore { .. } => {
                    let Some(rec) = records.get_mut(&key) else {
                        return err(Reason::UnknownRecord);
                    };
                    if !rec.deleted {
                        return err(Reason::NotDeleted);
                    }
                    rec.deleted = false;
                    rec.version = seq;
                }
            }
        }
        Ok(records)
    }

    /// Returns (accepted seq or refusal, repeated).
    fn step(&mut self, batch: &Batch) -> (Result<u64, (Option<usize>, Reason)>, bool) {
        if let Some(done) = self.decided.get(&batch.id) {
            return (*done, true);
        }
        let client = batch.id.client.0;
        if let Some(newest) = self.newest.get(&client) {
            if batch.id.seq <= *newest {
                return (Err((None, Reason::TooOld { newest: *newest })), false);
            }
        }
        let result = if batch.ops.is_empty() {
            Err((None, Reason::EmptyBatch))
        } else {
            match self.apply(&batch.ops, self.seq + 1) {
                Ok(records) => {
                    self.records = records;
                    self.seq += 1;
                    Ok(self.seq)
                }
                Err(e) => Err(e),
            }
        };
        self.decided.insert(batch.id, result);
        let newest = self.newest.entry(client).or_insert(0);
        *newest = (*newest).max(batch.id.seq);
        (result, false)
    }
}

#[derive(Debug, Clone)]
enum Action {
    /// A new batch from a client; `back` > 0 reuses an older number.
    New {
        client: u64,
        back: u64,
        ops: Vec<Op>,
    },
    /// A batch sent before, sent again.
    Again(usize),
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        8 => (1_u64..4, prop_oneof![6 => Just(0_u64), 1 => 1_u64..3], proptest::collection::vec(any_op(), 0..4))
            .prop_map(|(client, back, ops)| Action::New { client, back, ops }),
        2 => any::<usize>().prop_map(Action::Again),
    ]
}

fn records_of(state: &BaseState) -> BTreeMap<(TableId, RecordId), Rec> {
    state
        .records()
        .map(|(key, r)| {
            (
                *key,
                Rec {
                    cells: r
                        .cells
                        .iter()
                        .map(|(f, c)| (*f, (c.value.clone(), c.version)))
                        .collect(),
                    version: r.version,
                    created: r.created,
                    deleted: r.deleted,
                },
            )
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    /// Every decision equals the reference's; the records equal after every step; the
    /// sequence has no gaps; a batch is never applied twice; accepted effects, replayed on
    /// a copy, give the same records.
    #[test]
    fn model_the_base_decides_like_the_rules(actions in proptest::collection::vec(action(), 1..60)) {
        let mut state = BaseState::new(base_id());
        let mut model = Model::default();
        let mut copy = BaseState::new(base_id());
        let mut sent: Vec<Batch> = Vec::new();
        let mut next: BTreeMap<u64, u64> = BTreeMap::new();
        let mut accepted_ids = std::collections::BTreeSet::new();
        for action in actions {
            let batch = match action {
                Action::New { client, back, ops } => {
                    let n = next.entry(client).or_insert(0);
                    *n += 1;
                    batch(client, n.saturating_sub(back).max(1), ops)
                }
                Action::Again(i) if !sent.is_empty() => sent[i % sent.len()].clone(),
                Action::Again(_) => continue,
            };
            let decision = state.step(&batch);
            let (expected, repeated) = model.step(&batch);
            prop_assert_eq!(decision.repeated, repeated);
            match (&decision.outcome, &expected) {
                (Outcome::Accepted(a), Ok(seq)) => {
                    prop_assert_eq!(a.seq, *seq);
                    prop_assert_eq!(a.id, batch.id);
                    if !repeated {
                        prop_assert!(accepted_ids.insert(a.id), "applied twice");
                        copy.replay(a).unwrap();
                        // Every effect is a real change.
                        for effect in &a.effects {
                            if let Effect::Set { before, after, .. } = effect {
                                prop_assert_ne!(before, after);
                            }
                        }
                    }
                }
                (Outcome::Rejected(r), Err((op, reason))) => {
                    prop_assert_eq!(&r.reason, reason);
                    prop_assert_eq!(&r.op, op);
                }
                (got, want) => prop_assert!(false, "decided {:?}, rules say {:?}", got, want),
            }
            prop_assert_eq!(records_of(&state), model.records.clone());
            prop_assert_eq!(state.seq(), model.seq);
            prop_assert_eq!(records_of(&copy), model.records.clone(), "replay diverged");
            sent.push(batch);
        }
        prop_assert_eq!(accepted_ids.len() as u64, state.seq(), "the sequence has no gaps");
    }
}

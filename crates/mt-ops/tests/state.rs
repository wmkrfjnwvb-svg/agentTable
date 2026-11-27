//! The base's decision on batches: applying, sequences, repeats, conflicts, refusals.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_core::id::BaseId;
use mt_core::value::Value;
use mt_ops::{
    Accepted, BaseState, Batch, Effect, OpsError, Outcome, Reason, DEDUP_WINDOW, MAX_CELLS, MAX_OPS,
};
use std::collections::BTreeMap;

#[test]
fn std_batches_apply_in_order_with_versions() {
    let mut base = BaseState::new(base_id());
    let first = accepted(&base.step(&batch(
        1,
        1,
        vec![create(1, 1, &[(1, text("Laptop")), (2, Value::Null)])],
    )));
    assert_eq!(first.seq, 1);
    assert_eq!(
        first.effects,
        vec![Effect::Created {
            table: table(1),
            record: record(1),
            cells: BTreeMap::from([(field(1), text("Laptop"))])
        }],
        "empty values are left out"
    );
    let rec = base.record(table(1), record(1)).unwrap();
    assert_eq!(
        (
            rec.version,
            rec.created,
            rec.cell_version(field(1)),
            rec.cell_version(field(2))
        ),
        (1, 1, 1, 0)
    );
    let second = accepted(&base.step(&batch(
        1,
        2,
        vec![
            set(1, 1, 2, text("Dell"), 0),
            set(1, 1, 1, text("Laptop 14"), 1),
        ],
    )));
    assert_eq!(second.seq, 2);
    assert_eq!(second.effects.len(), 2);
    assert!(
        second.effects.iter().all(|e| matches!(
            e,
            Effect::Set {
                overwritten: None,
                ..
            }
        )),
        "both on what the client saw"
    );
    let rec = base.record(table(1), record(1)).unwrap();
    assert_eq!(rec.value(field(1)), text("Laptop 14"));
    assert_eq!(
        (
            rec.version,
            rec.cell_version(field(1)),
            rec.cell_version(field(2))
        ),
        (2, 2, 2)
    );
    assert_eq!(base.seq(), 2);
    // Emptying a cell removes it; setting the value it has is no change (but the batch
    // still takes a sequence number).
    let third = accepted(&base.step(&batch(
        1,
        3,
        vec![
            set(1, 1, 2, Value::Null, 2),
            set(1, 1, 1, text("Laptop 14"), 2),
        ],
    )));
    assert_eq!(third.seq, 3);
    assert_eq!(
        third.effects,
        vec![Effect::Set {
            table: table(1),
            record: record(1),
            field: field(2),
            before: text("Dell"),
            after: Value::Null,
            overwritten: None
        }]
    );
    assert_eq!(
        base.record(table(1), record(1))
            .unwrap()
            .cell_version(field(2)),
        0
    );
    assert_eq!(base.records().count(), 1);
}

#[test]
fn std_a_stale_cell_still_applies_and_names_what_it_overwrote() {
    let mut base = BaseState::new(base_id());
    accepted(&base.step(&batch(1, 1, vec![create(1, 1, &[(1, text("draft"))])])));
    // Alice and Bob both saw version 1 of the cell; Alice writes first.
    accepted(&base.step(&batch(1, 2, vec![set(1, 1, 1, text("Alice"), 1)])));
    let bob = accepted(&base.step(&batch(2, 1, vec![set(1, 1, 1, text("Bob"), 1)])));
    assert_eq!(
        bob.effects,
        vec![Effect::Set {
            table: table(1),
            record: record(1),
            field: field(1),
            before: text("Alice"),
            after: text("Bob"),
            overwritten: Some(text("Alice"))
        }]
    );
    assert_eq!(
        base.record(table(1), record(1)).unwrap().value(field(1)),
        text("Bob"),
        "the last writer wins"
    );
    // A cell never set has version 0; someone set it meanwhile.
    accepted(&base.step(&batch(1, 3, vec![set(1, 1, 2, text("x"), 0)])));
    let late = accepted(&base.step(&batch(2, 2, vec![set(1, 1, 2, text("y"), 0)])));
    assert!(
        matches!(&late.effects[0], Effect::Set { overwritten: Some(v), .. } if *v == text("x"))
    );
    // Its own earlier change in the same batch is not someone else's.
    let own = accepted(&base.step(&batch(
        2,
        3,
        vec![set(1, 1, 1, text("B2"), 3), set(1, 1, 1, text("B3"), 3)],
    )));
    assert!(
        own.effects.iter().all(|e| matches!(
            e,
            Effect::Set {
                overwritten: None,
                ..
            }
        )),
        "{own:?}"
    );
}

#[test]
fn std_deliberate_changes_need_the_version_seen() {
    let mut base = BaseState::new(base_id());
    accepted(&base.step(&batch(1, 1, vec![create(1, 1, &[(9, text("Draft"))])])));
    accepted(&base.step(&batch(1, 2, vec![set(1, 1, 1, text("edit"), 0)])));
    // The record is at version 2 now; a transition on version 1 is refused.
    let refused = rejected(&base.step(&batch(
        2,
        1,
        vec![transition(1, 1, 9, text("Submitted"), 1)],
    )));
    assert_eq!(
        refused.reason,
        Reason::StaleRecord {
            expected: 1,
            actual: 2
        }
    );
    assert_eq!(refused.op, Some(0));
    let moved = accepted(&base.step(&batch(
        2,
        2,
        vec![transition(1, 1, 9, text("Submitted"), 2)],
    )));
    assert_eq!(moved.seq, 3);
    // Delete and restore.
    assert_eq!(
        rejected(&base.step(&batch(2, 3, vec![delete(1, 1, 2)]))).reason,
        Reason::StaleRecord {
            expected: 2,
            actual: 3
        }
    );
    accepted(&base.step(&batch(2, 4, vec![delete(1, 1, 3)])));
    assert!(base.record(table(1), record(1)).unwrap().deleted);
    for op in [
        set(1, 1, 1, text("x"), 2),
        transition(1, 1, 9, text("x"), 4),
        delete(1, 1, 4),
    ] {
        assert_eq!(
            rejected(&base.step(&batch(3, base.seq() + 10, vec![op]))).reason,
            Reason::RecordDeleted
        );
    }
    let back = accepted(&base.step(&batch(2, 5, vec![restore(1, 1)])));
    assert_eq!(
        back.effects,
        vec![Effect::Restored {
            table: table(1),
            record: record(1)
        }]
    );
    assert_eq!(
        rejected(&base.step(&batch(2, 6, vec![restore(1, 1)]))).reason,
        Reason::NotDeleted
    );
    assert_eq!(
        base.record(table(1), record(1)).unwrap().value(field(9)),
        text("Submitted"),
        "values survive the trash"
    );
}

#[test]
fn std_a_batch_sent_twice_applies_once() {
    let mut base = BaseState::new(base_id());
    let b = batch(7, 1, vec![create(1, 1, &[(1, text("a"))])]);
    let first = base.step(&b);
    assert!(!first.repeated);
    let again = base.step(&b);
    assert!(again.repeated);
    assert_eq!(again.outcome, first.outcome);
    assert_eq!(base.seq(), 1);
    // A refusal is remembered too: the same batch gets the same answer.
    let bad = batch(7, 2, vec![set(1, 9, 1, text("x"), 0)]);
    let refused = base.step(&bad);
    assert_eq!(rejected(&refused).reason, Reason::UnknownRecord);
    accepted(&base.step(&batch(7, 3, vec![create(1, 9, &[])])));
    let still = base.step(&bad);
    assert!(still.repeated);
    assert_eq!(
        still.outcome, refused.outcome,
        "not applied now that the record exists"
    );
    assert_eq!(base.outcome(id(7, 2)), Some(&refused.outcome));
    // An older number that is no longer remembered is refused, not applied.
    assert_eq!(
        rejected(&base.step(&batch(7, 0, vec![create(1, 5, &[])]))).reason,
        Reason::TooOld { newest: 3 }
    );
}

#[test]
fn edge_batches_are_all_or_nothing() {
    let mut base = BaseState::new(base_id());
    accepted(&base.step(&batch(1, 1, vec![create(1, 1, &[(1, text("a"))])])));
    let before = base.clone();
    let refused = rejected(&base.step(&batch(
        1,
        2,
        vec![
            set(1, 1, 1, text("b"), 1),
            create(1, 2, &[]),
            set(1, 3, 1, text("c"), 0),
        ],
    )));
    assert_eq!(
        (refused.op, refused.reason),
        (Some(2), Reason::UnknownRecord)
    );
    assert_eq!(base.seq(), 1);
    assert_eq!(
        base.record(table(1), record(1)),
        before.record(table(1), record(1))
    );
    assert!(
        base.record(table(1), record(2)).is_none(),
        "the create in the refused batch did not apply"
    );
    // Create and change in one batch; create twice in one batch.
    let made = accepted(&base.step(&batch(
        1,
        3,
        vec![
            create(1, 2, &[]),
            set(1, 2, 1, text("x"), 0),
            delete(1, 2, 0),
        ],
    )));
    assert_eq!(made.effects.len(), 3);
    assert!(base.record(table(1), record(2)).unwrap().deleted);
    assert_eq!(
        rejected(&base.step(&batch(1, 4, vec![create(1, 4, &[]), create(1, 4, &[])]))).reason,
        Reason::RecordExists
    );
    assert_eq!(
        rejected(&base.step(&batch(1, 5, vec![create(1, 1, &[])]))).reason,
        Reason::RecordExists
    );
    // The same record id in another table is another record.
    accepted(&base.step(&batch(1, 6, vec![create(2, 1, &[])])));
}

#[test]
fn edge_the_window_of_remembered_batches_is_bounded() {
    let mut base = BaseState::new(base_id());
    accepted(&base.step(&batch(1, 1, vec![create(1, 1, &[])])));
    for n in 0..DEDUP_WINDOW as u64 + 5 {
        base.step(&batch(
            2,
            n + 1,
            vec![set(1, 1, 1, text(&format!("v{n}")), 0)],
        ));
    }
    assert!(base.outcome(id(1, 1)).is_none(), "the oldest is forgotten");
    assert!(base.outcome(id(2, DEDUP_WINDOW as u64 + 5)).is_some());
    // Forgotten, but still recognised as old: refused, not applied twice.
    let decision = base.step(&batch(1, 1, vec![create(1, 1, &[])]));
    assert_eq!(rejected(&decision).reason, Reason::TooOld { newest: 1 });
    assert!(!decision.repeated);
}

#[test]
fn invalid_batches_are_refused_whole() {
    let mut base = BaseState::new(base_id());
    let other = Batch {
        id: id(1, 1),
        base: BaseId::from_u128(99),
        ops: vec![create(1, 1, &[])],
    };
    assert_eq!(rejected(&base.step(&other)).reason, Reason::WrongBase);
    assert!(
        base.outcome(id(1, 1)).is_none(),
        "not remembered: it may still go to its base"
    );
    assert_eq!(
        rejected(&base.step(&batch(1, 1, vec![]))).reason,
        Reason::EmptyBatch
    );
    let many: Vec<_> = (0..=MAX_OPS as u128)
        .map(|n| create(1, n + 1, &[]))
        .collect();
    assert_eq!(
        rejected(&base.step(&batch(1, 2, many))).reason,
        Reason::TooManyOps
    );
    let wide: Vec<(u128, Value)> = (0..=MAX_CELLS as u128)
        .map(|n| (n + 1, text("x")))
        .collect();
    let refused = rejected(&base.step(&batch(1, 3, vec![create(1, 1, &wide)])));
    assert_eq!(
        (refused.op, refused.reason),
        (Some(0), Reason::TooManyCells)
    );
    for op in [
        set(1, 1, 1, text("x"), 0),
        transition(1, 1, 1, text("x"), 0),
        delete(1, 1, 0),
        restore(1, 1),
    ] {
        let n = base.seq() + 100;
        let r = rejected(&base.step(&batch(3, n + rand_seq(&op), vec![op])));
        assert_eq!(r.reason, Reason::UnknownRecord);
    }
    assert_eq!(base.seq(), 0);
    // Every reason reads as a sentence.
    for reason in [
        Reason::WrongBase,
        Reason::EmptyBatch,
        Reason::TooManyOps,
        Reason::TooManyCells,
        Reason::TooOld { newest: 3 },
        Reason::RecordExists,
        Reason::UnknownRecord,
        Reason::RecordDeleted,
        Reason::NotDeleted,
        Reason::StaleRecord {
            expected: 1,
            actual: 2,
        },
        Reason::SequenceExhausted,
    ] {
        assert!(!reason.text().is_empty());
    }
}

fn rand_seq(op: &mt_ops::Op) -> u64 {
    match op {
        mt_ops::Op::Set { .. } => 1,
        mt_ops::Op::Transition { .. } => 2,
        mt_ops::Op::Delete { .. } => 3,
        mt_ops::Op::Create { .. } | mt_ops::Op::Restore { .. } => 4,
    }
}

#[test]
fn edge_the_sequence_never_wraps() {
    let mut base = BaseState::from_records(base_id(), u64::MAX, BTreeMap::new());
    assert_eq!(
        rejected(&base.step(&batch(1, 1, vec![create(1, 1, &[])]))).reason,
        Reason::SequenceExhausted
    );
    let mut copy = BaseState::from_records(base_id(), u64::MAX, BTreeMap::new());
    assert!(matches!(
        copy.replay(&Accepted {
            id: id(1, 1),
            seq: 1,
            effects: vec![]
        }),
        Err(OpsError::Gap { .. })
    ));
}

#[test]
fn std_replaying_accepted_batches_rebuilds_the_state() {
    let mut owner = BaseState::new(base_id());
    let mut copy = BaseState::new(base_id());
    let batches = vec![
        batch(1, 1, vec![create(1, 1, &[(1, text("a")), (2, text("b"))])]),
        batch(
            1,
            2,
            vec![set(1, 1, 1, text("c"), 1), set(1, 1, 2, Value::Null, 1)],
        ),
        batch(2, 1, vec![delete(1, 1, 2)]),
        batch(2, 2, vec![restore(1, 1), create(2, 7, &[])]),
    ];
    for b in &batches {
        let a = accepted(&owner.step(b));
        copy.replay(&a).unwrap();
        assert_eq!(
            copy.records().collect::<Vec<_>>(),
            owner.records().collect::<Vec<_>>()
        );
    }
    assert_eq!(copy.seq(), owner.seq());
    // A replayed batch is remembered: sent again to the copy, it is recognised.
    assert!(copy.step(&batches[0]).repeated);
    // A base loaded from storage learns recent decisions from the journal.
    let mut fresh = BaseState::from_records(base_id(), owner.seq(), BTreeMap::new());
    let first = owner.outcome(id(1, 1)).unwrap().clone();
    fresh.recall(first.clone());
    let again = fresh.step(&batches[0]);
    assert!(again.repeated);
    assert_eq!(again.outcome, first);
    assert_eq!(fresh.seq(), owner.seq(), "recalling applies nothing");
    // Loaded from storage at a sequence, then continued.
    let loaded = BaseState::from_records(
        base_id(),
        owner.seq(),
        owner.records().map(|(k, v)| (*k, v.clone())).collect(),
    );
    assert_eq!(loaded.records().count(), 2);
    assert_eq!(loaded.base(), base_id());
}

#[test]
fn invalid_replays_change_nothing() {
    let mut copy = BaseState::new(base_id());
    let created = Accepted {
        id: id(1, 1),
        seq: 1,
        effects: vec![Effect::Created {
            table: table(1),
            record: record(1),
            cells: BTreeMap::new(),
        }],
    };
    assert_eq!(
        copy.replay(&Accepted {
            seq: 2,
            ..created.clone()
        }),
        Err(OpsError::Gap {
            expected: 1,
            got: 2
        })
    );
    copy.replay(&created).unwrap();
    let before = copy.clone();
    let bad = [
        vec![Effect::Created {
            table: table(1),
            record: record(1),
            cells: BTreeMap::new(),
        }],
        vec![Effect::Set {
            table: table(1),
            record: record(5),
            field: field(1),
            before: Value::Null,
            after: text("x"),
            overwritten: None,
        }],
        vec![Effect::Deleted {
            table: table(1),
            record: record(5),
        }],
        vec![Effect::Restored {
            table: table(1),
            record: record(1),
        }],
        vec![
            Effect::Deleted {
                table: table(1),
                record: record(1),
            },
            Effect::Deleted {
                table: table(1),
                record: record(1),
            },
        ],
        vec![
            Effect::Set {
                table: table(1),
                record: record(1),
                field: field(1),
                before: Value::Null,
                after: text("x"),
                overwritten: None,
            },
            Effect::Created {
                table: table(1),
                record: record(1),
                cells: BTreeMap::new(),
            },
        ],
    ];
    for effects in bad {
        let result = copy.replay(&Accepted {
            id: id(1, 2),
            seq: 2,
            effects,
        });
        assert!(
            matches!(result, Err(OpsError::Replay { seq: 2, .. })),
            "{result:?}"
        );
        assert_eq!(copy, before, "a failed replay changes nothing");
    }
    // Empty a cell by replay.
    copy.replay(&Accepted {
        id: id(1, 2),
        seq: 2,
        effects: vec![Effect::Set {
            table: table(1),
            record: record(1),
            field: field(1),
            before: Value::Null,
            after: text("x"),
            overwritten: None,
        }],
    })
    .unwrap();
    copy.replay(&Accepted {
        id: id(1, 3),
        seq: 3,
        effects: vec![Effect::Set {
            table: table(1),
            record: record(1),
            field: field(1),
            before: text("x"),
            after: Value::Null,
            overwritten: None,
        }],
    })
    .unwrap();
    assert!(copy.record(table(1), record(1)).unwrap().cells.is_empty());
    assert!(matches!(copy.outcome(id(1, 3)), Some(Outcome::Accepted(_))));
}

#[test]
fn std_outcomes_read_and_serialize_as_json() {
    let mut base = BaseState::new(base_id());
    let decision = base.step(&batch(1, 1, vec![create(1, 1, &[(1, text("a"))])]));
    assert_eq!(decision.outcome.id(), id(1, 1));
    let json = serde_json::to_value(&decision.outcome).unwrap();
    assert_eq!(json["accepted"]["seq"], 1);
    assert_eq!(
        json["accepted"]["effects"][0]["created"]["cells"][field(1).to_string()]["type"],
        "text"
    );
    let back: Outcome = serde_json::from_value(json).unwrap();
    assert_eq!(back, decision.outcome);
    let refused = base.step(&batch(1, 2, vec![]));
    assert_eq!(refused.outcome.id(), id(1, 2));
    let json = serde_json::to_value(&refused.outcome).unwrap();
    assert_eq!(json["rejected"]["reason"], "empty_batch");
    // A client applies its own batch optimistically.
    let mut view = base.clone();
    assert!(view.apply_local(&batch(1, 9, vec![set(1, 1, 1, text("b"), 1)])));
    assert!(!view.apply_local(&batch(1, 10, vec![set(1, 7, 1, text("b"), 1)])));
    assert!(view.outcome(id(1, 9)).is_none(), "not remembered");
}

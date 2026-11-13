//! Delivery state machine, ledger and gap tracker: standard, anomalous, edge, invalid, model-based.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, clippy::panic)]

mod common;

use std::collections::BTreeSet;

use common::Rng;
use mt_audit::delivery::step;
use mt_audit::{
    AuditError, ClientId, ClientOpId, DeliveryEvent as E, DeliveryLedger, DeliveryState as S,
    SeqObservation, SeqRange, SeqTracker, StepOutcome,
};
use proptest::prelude::*;

fn adv(state: &S, event: &E) -> S {
    match step(state, event).unwrap() {
        StepOutcome::Advanced(next) => next,
        StepOutcome::Duplicate => panic!("expected advance from {state:?} on {event:?}"),
    }
}

fn reject(reason: &str) -> E {
    E::Reject { reason: reason.to_owned() }
}

// ---- standard ----

#[test]
fn standard_happy_path_to_applied() {
    let mut s = S::Created;
    assert!(!s.is_terminal());
    for e in [E::Send, E::Receive, E::Validate] {
        s = adv(&s, &e);
        assert!(!s.is_terminal());
        assert_eq!(s.committed_seq(), None);
    }
    s = adv(&s, &E::Commit { seq: 42 });
    assert_eq!(s, S::Committed { seq: 42 });
    assert!(s.is_terminal());
    s = adv(&s, &E::Broadcast);
    assert_eq!(s.name(), "broadcast");
    assert_eq!(s.applied_count(), 0);
    s = adv(&s, &E::ClientApplied { client: ClientId(1) });
    s = adv(&s, &E::ClientApplied { client: ClientId(2) });
    assert_eq!(s.applied_count(), 2);
    assert_eq!(s.committed_seq(), Some(42));
    assert_eq!(s.name(), "applied");
    assert!(s.is_terminal());
}

#[test]
fn standard_rejection_from_received_and_validated() {
    let received = adv(&adv(&S::Created, &E::Send), &E::Receive);
    let r = adv(&received, &reject("no permission"));
    assert_eq!(r, S::Rejected { reason: "no permission".into() });
    assert!(r.is_terminal());
    assert_eq!(r.committed_seq(), None);
    let validated = adv(&received, &E::Validate);
    assert_eq!(adv(&validated, &reject("conflict")).name(), "rejected");
}

#[test]
fn standard_names_and_serde() {
    let names: Vec<&str> = [
        S::Created,
        S::Sent,
        S::Received,
        S::Validated,
        S::Committed { seq: 1 },
        S::Rejected { reason: String::new() },
        S::Broadcast { seq: 1 },
        S::Applied { seq: 1, by: BTreeSet::new() },
    ]
    .iter()
    .map(S::name)
    .collect();
    assert_eq!(
        names,
        ["created", "sent", "received", "validated", "committed", "rejected", "broadcast", "applied"]
    );
    let events = [
        E::Send,
        E::Receive,
        E::Validate,
        E::Commit { seq: 1 },
        reject("x"),
        E::Broadcast,
        E::ClientApplied { client: ClientId(3) },
    ];
    let event_names: Vec<&str> = events.iter().map(E::name).collect();
    assert_eq!(
        event_names,
        ["send", "receive", "validate", "commit", "reject", "broadcast", "client_applied"]
    );
    let json = serde_json::to_string(&S::Committed { seq: 9 }).unwrap();
    assert_eq!(json, r#"{"state":"committed","seq":9}"#);
    assert_eq!(serde_json::from_str::<S>(&json).unwrap(), S::Committed { seq: 9 });
    let ev = serde_json::to_string(&E::Commit { seq: 9 }).unwrap();
    assert_eq!(serde_json::from_str::<E>(&ev).unwrap(), E::Commit { seq: 9 });
}

#[test]
fn standard_ledger_outbox_flow() {
    let mut ledger = DeliveryLedger::new();
    assert!(ledger.is_empty());
    let a = ClientOpId(1);
    let b = ClientOpId(2);
    assert_eq!(ledger.create(a), StepOutcome::Advanced(S::Created));
    ledger.create(b);
    for e in [E::Send, E::Receive, E::Validate, E::Commit { seq: 1 }] {
        ledger.apply(a, &e).unwrap();
    }
    ledger.apply(b, &E::Send).unwrap();
    assert_eq!(ledger.len(), 2);
    assert_eq!(ledger.pending().map(|(id, _)| id).collect::<Vec<_>>(), [b]);
    assert_eq!(ledger.op_at_seq(1), Some(a));
    assert_eq!(ledger.op_at_seq(2), None);
    assert_eq!(ledger.state(a), Some(&S::Committed { seq: 1 }));
    assert!(!ledger.forget_terminal(b), "pending ops stay");
    assert!(ledger.forget_terminal(a));
    assert!(!ledger.forget_terminal(a));
    assert_eq!(ledger.op_at_seq(1), None);
    assert_eq!(ledger.state(a), None);
    ledger.apply(b, &E::Receive).unwrap();
    ledger.apply(b, &reject("bad")).unwrap();
    assert!(ledger.forget_terminal(b));
    assert!(ledger.is_empty());
}

#[test]
fn standard_server_side_receive() {
    let mut ledger = DeliveryLedger::new();
    let op = ClientOpId(7);
    assert_eq!(ledger.receive(op).unwrap(), StepOutcome::Advanced(S::Received));
    assert_eq!(ledger.receive(op).unwrap(), StepOutcome::Duplicate);
    ledger.apply(op, &E::Validate).unwrap();
    assert_eq!(ledger.receive(op).unwrap(), StepOutcome::Duplicate);
    // A client-side ledger that only created the op cannot "receive" it before sending.
    let mut client = DeliveryLedger::new();
    client.create(op);
    assert!(client.receive(op).is_err());
}

// ---- anomalous: re-delivery, duplicates ----

#[test]
fn anomalous_redelivery_is_idempotent() {
    let mut ledger = DeliveryLedger::new();
    let op = ClientOpId(1);
    ledger.create(op);
    assert_eq!(ledger.create(op), StepOutcome::Duplicate);
    let script = [
        E::Send,
        E::Receive,
        E::Validate,
        E::Commit { seq: 5 },
        E::Broadcast,
        E::ClientApplied { client: ClientId(1) },
    ];
    for (i, e) in script.iter().enumerate() {
        assert!(matches!(ledger.apply(op, e).unwrap(), StepOutcome::Advanced(_)));
        // Everything delivered so far may be delivered again without effect.
        let before = ledger.clone();
        for old in &script[..=i] {
            assert_eq!(ledger.apply(op, old).unwrap(), StepOutcome::Duplicate, "{old:?}");
        }
        assert_eq!(ledger, before);
    }
}

#[test]
fn anomalous_retry_after_rejection_keeps_first_reason() {
    let s = S::Rejected { reason: "first".into() };
    assert_eq!(step(&s, &E::Send).unwrap(), StepOutcome::Duplicate);
    assert_eq!(step(&s, &E::Receive).unwrap(), StepOutcome::Duplicate);
    assert_eq!(step(&s, &reject("second")).unwrap(), StepOutcome::Duplicate);
}

#[test]
fn anomalous_conflicting_commit_and_reused_seq() {
    let mut ledger = DeliveryLedger::new();
    let (a, b) = (ClientOpId(1), ClientOpId(2));
    for op in [a, b] {
        ledger.receive(op).unwrap();
        ledger.apply(op, &E::Validate).unwrap();
    }
    ledger.apply(a, &E::Commit { seq: 3 }).unwrap();
    assert_eq!(
        ledger.apply(a, &E::Commit { seq: 4 }),
        Err(AuditError::CommitConflict { existing: 3, attempted: 4 })
    );
    let before = ledger.clone();
    assert_eq!(ledger.apply(b, &E::Commit { seq: 3 }), Err(AuditError::SequenceReused { seq: 3 }));
    assert_eq!(ledger, before, "failed call leaves the ledger untouched");
    ledger.apply(b, &E::Commit { seq: 4 }).unwrap();
    for s in [S::Broadcast { seq: 3 }, S::Applied { seq: 3, by: BTreeSet::new() }] {
        assert!(matches!(step(&s, &E::Commit { seq: 9 }), Err(AuditError::CommitConflict { .. })));
        assert_eq!(step(&s, &E::Commit { seq: 3 }).unwrap(), StepOutcome::Duplicate);
    }
}

// ---- invalid ----

#[test]
fn invalid_transitions_are_errors() {
    let committed = S::Committed { seq: 1 };
    let rejected = S::Rejected { reason: "r".into() };
    let cases: Vec<(S, E)> = vec![
        (S::Created, E::Receive),
        (S::Created, E::Validate),
        (S::Sent, E::Validate),
        (rejected.clone(), E::Validate),
        (S::Created, E::Commit { seq: 1 }),
        (S::Sent, E::Commit { seq: 1 }),
        (S::Received, E::Commit { seq: 1 }),
        (rejected.clone(), E::Commit { seq: 1 }),
        (S::Created, reject("x")),
        (S::Sent, reject("x")),
        (committed.clone(), reject("x")),
        (S::Broadcast { seq: 1 }, reject("x")),
        (S::Applied { seq: 1, by: BTreeSet::new() }, reject("x")),
        (S::Created, E::Broadcast),
        (S::Sent, E::Broadcast),
        (S::Received, E::Broadcast),
        (S::Validated, E::Broadcast),
        (rejected.clone(), E::Broadcast),
        (S::Created, E::ClientApplied { client: ClientId(1) }),
        (S::Sent, E::ClientApplied { client: ClientId(1) }),
        (S::Received, E::ClientApplied { client: ClientId(1) }),
        (S::Validated, E::ClientApplied { client: ClientId(1) }),
        (committed, E::ClientApplied { client: ClientId(1) }),
        (rejected, E::ClientApplied { client: ClientId(1) }),
    ];
    for (state, event) in cases {
        let err = step(&state, &event).unwrap_err();
        assert_eq!(err, AuditError::IllegalTransition { from: state.name(), event: event.name() });
        assert!(err.to_string().contains(state.name()));
    }
    assert_eq!(step(&S::Validated, &E::Commit { seq: 0 }), Err(AuditError::ZeroSequence));
    let mut ledger = DeliveryLedger::new();
    assert_eq!(ledger.apply(ClientOpId(9), &E::Send), Err(AuditError::UnknownOperation));
}

// ---- model-based (seeded) ----

/// Independent reference model: a phase number plus flags, written without looking at the table.
#[derive(Clone, Debug, PartialEq)]
struct Model {
    phase: u8, // 0 created, 1 sent, 2 received, 3 validated, 4 committed, 5 rejected
    seq: u64,
    broadcast: bool,
    acks: BTreeSet<ClientId>,
}

#[derive(Debug, PartialEq)]
enum Expect {
    Advance,
    Dup,
    Error,
}

impl Model {
    fn apply(&mut self, e: &E) -> Expect {
        let p = self.phase;
        match e {
            E::Send if p == 0 => {
                self.phase = 1;
                Expect::Advance
            }
            E::Send => Expect::Dup,
            E::Receive if p == 0 => Expect::Error,
            E::Receive if p == 1 => {
                self.phase = 2;
                Expect::Advance
            }
            E::Receive => Expect::Dup,
            E::Validate if p == 2 => {
                self.phase = 3;
                Expect::Advance
            }
            E::Validate if p == 3 || p == 4 => Expect::Dup,
            E::Validate => Expect::Error,
            E::Commit { seq } if *seq == 0 => Expect::Error,
            E::Commit { seq } if p == 3 => {
                self.phase = 4;
                self.seq = *seq;
                Expect::Advance
            }
            E::Commit { seq } if p == 4 && *seq == self.seq => Expect::Dup,
            E::Commit { .. } => Expect::Error,
            E::Reject { .. } if p == 2 || p == 3 => {
                self.phase = 5;
                Expect::Advance
            }
            E::Reject { .. } if p == 5 => Expect::Dup,
            E::Reject { .. } => Expect::Error,
            E::Broadcast if p == 4 && !self.broadcast => {
                self.broadcast = true;
                Expect::Advance
            }
            E::Broadcast if p == 4 => Expect::Dup,
            E::Broadcast => Expect::Error,
            E::ClientApplied { client } if p == 4 && self.broadcast => {
                if self.acks.insert(*client) {
                    Expect::Advance
                } else {
                    Expect::Dup
                }
            }
            E::ClientApplied { .. } => Expect::Error,
        }
    }

    fn matches(&self, s: &S) -> bool {
        match s {
            S::Created => self.phase == 0,
            S::Sent => self.phase == 1,
            S::Received => self.phase == 2,
            S::Validated => self.phase == 3,
            S::Committed { seq } => {
                self.phase == 4 && *seq == self.seq && !self.broadcast && self.acks.is_empty()
            }
            S::Rejected { .. } => self.phase == 5,
            S::Broadcast { seq } => {
                self.phase == 4 && *seq == self.seq && self.broadcast && self.acks.is_empty()
            }
            S::Applied { seq, by } => {
                self.phase == 4 && *seq == self.seq && self.broadcast && *by == self.acks && !by.is_empty()
            }
        }
    }
}

fn random_event(rng: &mut Rng) -> E {
    match rng.below(9) {
        0 => E::Send,
        1 => E::Receive,
        2 => E::Validate,
        3 | 4 => E::Commit { seq: rng.below(4) },
        5 => E::Reject { reason: format!("r{}", rng.below(3)) },
        6 => E::Broadcast,
        _ => E::ClientApplied { client: ClientId(u128::from(rng.below(3))) },
    }
}

fn run_model(seed: u64, steps: usize) {
    let mut rng = Rng(seed);
    let mut ledger = DeliveryLedger::new();
    let op = ClientOpId(u128::from(seed));
    ledger.create(op);
    let mut model = Model { phase: 0, seq: 0, broadcast: false, acks: BTreeSet::new() };
    for i in 0..steps {
        // Bias towards progress so that deep states are reached, with plenty of noise.
        let event = if rng.below(3) == 0 {
            match model.phase {
                0 => E::Send,
                1 => E::Receive,
                2 => E::Validate,
                3 => E::Commit { seq: 1 + rng.below(3) },
                _ => random_event(&mut rng),
            }
        } else {
            random_event(&mut rng)
        };
        let before = ledger.state(op).unwrap().clone();
        let was_terminal = before.is_terminal();
        let committed_before = before.committed_seq();
        let result = ledger.apply(op, &event);
        let expected = model.apply(&event);
        let after = ledger.state(op).unwrap();
        let ctx = format!("seed={seed} step={i} event={event:?} before={before:?} after={after:?}");
        match (&result, &expected) {
            (Ok(StepOutcome::Advanced(next)), Expect::Advance) => assert_eq!(next, after, "{ctx}"),
            (Ok(StepOutcome::Duplicate), Expect::Dup) | (Err(_), Expect::Error) => {
                assert_eq!(&before, after, "{ctx}");
            }
            _ => panic!("model disagreement: got {result:?}, expected {expected:?}; {ctx}"),
        }
        assert!(model.matches(after), "{ctx} model={model:?}");
        // Safety invariants.
        assert!(!was_terminal || after.is_terminal(), "left terminal state; {ctx}");
        if let Some(seq) = committed_before {
            assert_eq!(after.committed_seq(), Some(seq), "committed seq changed; {ctx}");
        }
        if matches!(before, S::Rejected { .. }) {
            assert_eq!(&before, after, "rejected is absorbing; {ctx}");
        }
        assert!(after.applied_count() >= before.applied_count(), "{ctx}");
    }
}

#[test]
fn model_fixed_seeds() {
    for seed in 0..500 {
        run_model(seed, 60);
    }
}

proptest! {
    #[test]
    fn model_random_seeds(seed in any::<u64>()) {
        run_model(seed, 80);
    }
}

// ---- gap tracker ----

fn range(first: u64, last: u64) -> SeqRange {
    SeqRange { first, last }
}

#[test]
fn tracker_standard_in_order_gap_fill_and_duplicates() {
    let mut t = SeqTracker::new();
    assert_eq!(t.contiguous_through(), 0);
    assert_eq!(t.highest_seen(), 0);
    assert_eq!(t.observe(1).unwrap(), SeqObservation::InOrder { contiguous_through: 1 });
    assert_eq!(t.observe(2).unwrap(), SeqObservation::InOrder { contiguous_through: 2 });
    assert_eq!(t.observe(2).unwrap(), SeqObservation::Duplicate);
    assert_eq!(t.observe(5).unwrap(), SeqObservation::Gap { missing: vec![range(3, 4)] });
    assert_eq!(t.observe(5).unwrap(), SeqObservation::Duplicate);
    assert_eq!(
        t.observe(9).unwrap(),
        SeqObservation::Gap { missing: vec![range(3, 4), range(6, 8)] }
    );
    assert_eq!(t.highest_seen(), 9);
    assert!(t.has_seen(5) && !t.has_seen(6) && !t.has_seen(10));
    assert_eq!(
        t.observe(7).unwrap(),
        SeqObservation::Gap { missing: vec![range(3, 4), range(6, 6), range(8, 8)] }
    );
    assert_eq!(t.observe(6).unwrap(), SeqObservation::Gap { missing: vec![range(3, 4), range(8, 8)] });
    assert_eq!(t.observe(8).unwrap(), SeqObservation::Gap { missing: vec![range(3, 4)] });
    assert_eq!(t.observe(3).unwrap(), SeqObservation::InOrder { contiguous_through: 3 });
    assert_eq!(t.observe(4).unwrap(), SeqObservation::InOrder { contiguous_through: 9 });
    assert!(t.missing().is_empty());
}

#[test]
fn tracker_edge_resync_and_bounds() {
    let mut t = SeqTracker::starting_after(10);
    assert_eq!(t.observe(4).unwrap(), SeqObservation::Duplicate);
    assert_eq!(t.observe(0), Err(AuditError::ZeroSequence));
    t.observe(15).unwrap();
    t.observe(16).unwrap();
    t.observe(20).unwrap();
    t.resync_through(5); // never moves backwards
    assert_eq!(t.contiguous_through(), 10);
    t.resync_through(15); // lands inside the 15..=16 range: its tail is kept
    assert_eq!(t.contiguous_through(), 16);
    assert_eq!(t.missing(), vec![range(17, 19)]);
    t.resync_through(19); // touches 20
    assert_eq!(t.contiguous_through(), 20);
    t.observe(30).unwrap();
    t.resync_through(40);
    assert_eq!((t.contiguous_through(), t.highest_seen()), (40, 40));

    let mut top = SeqTracker::starting_after(u64::MAX - 2);
    assert_eq!(
        top.observe(u64::MAX).unwrap(),
        SeqObservation::Gap { missing: vec![range(u64::MAX - 1, u64::MAX - 1)] }
    );
    assert_eq!(
        top.observe(u64::MAX - 1).unwrap(),
        SeqObservation::InOrder { contiguous_through: u64::MAX }
    );
    assert_eq!(top.observe(u64::MAX).unwrap(), SeqObservation::Duplicate);
    let mut ahead = SeqTracker::new();
    ahead.observe(u64::MAX).unwrap();
    ahead.resync_through(u64::MAX);
    assert_eq!(ahead.contiguous_through(), u64::MAX);
    let json = serde_json::to_string(&ahead).unwrap();
    assert_eq!(serde_json::from_str::<SeqTracker>(&json).unwrap(), ahead);
}

proptest! {
    /// The tracker agrees with a plain set, under duplication and reordering.
    #[test]
    fn prop_tracker_matches_set_model(seqs in proptest::collection::vec(1u64..60, 0..200)) {
        let mut t = SeqTracker::new();
        let mut seen = BTreeSet::new();
        for s in seqs {
            let fresh = seen.insert(s);
            let obs = t.observe(s).unwrap();
            let contiguous = (1..).take_while(|n| seen.contains(n)).last().unwrap_or(0);
            let top = *seen.last().unwrap();
            let missing: Vec<u64> = (contiguous + 1..top).filter(|n| !seen.contains(n)).collect();
            let reported: Vec<u64> =
                t.missing().iter().flat_map(|r| r.first..=r.last).collect();
            prop_assert_eq!(&reported, &missing);
            prop_assert_eq!(t.contiguous_through(), contiguous);
            prop_assert_eq!(t.highest_seen(), top);
            match obs {
                SeqObservation::Duplicate => prop_assert!(!fresh),
                SeqObservation::InOrder { contiguous_through } => {
                    prop_assert!(fresh && s <= contiguous);
                    prop_assert_eq!(contiguous_through, contiguous);
                }
                SeqObservation::Gap { missing: m } => {
                    prop_assert!(fresh && s > contiguous);
                    prop_assert_eq!(m, t.missing());
                }
            }
            for n in 1..62 {
                prop_assert_eq!(t.has_seen(n), seen.contains(&n));
            }
        }
    }

    /// Any permutation with duplicates of 1..=n ends with no gaps.
    #[test]
    fn prop_tracker_converges(n in 1u64..80, seed in any::<u64>()) {
        let mut rng = Rng(seed);
        let mut order: Vec<u64> = (1..=n).chain(1..=n).collect();
        for i in (1..order.len()).rev() {
            order.swap(i, usize::try_from(rng.below(i as u64 + 1)).unwrap());
        }
        let mut t = SeqTracker::new();
        for s in order {
            t.observe(s).unwrap();
        }
        prop_assert_eq!(t.contiguous_through(), n);
        prop_assert!(t.missing().is_empty());
    }
}

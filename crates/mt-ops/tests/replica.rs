//! Client replicas: optimistic view, answers, rebase — and a seeded simulation of several
//! clients that must all end with the server's state.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_core::value::Value;
use mt_ops::{Accepted, BaseState, Batch, ClientId, Op, OpsError, Outcome, Replica};
use std::collections::{BTreeSet, VecDeque};

#[test]
fn std_a_replica_shows_its_changes_at_once_and_converges() {
    let mut server = BaseState::new(base_id());
    let mut alice = Replica::new(client(1), 0, server.clone());
    let mut bob = Replica::new(client(2), 0, server.clone());
    assert_eq!(alice.client(), client(1));
    let a1 = alice
        .submit(vec![create(1, 1, &[(1, text("draft"))])])
        .unwrap();
    assert_eq!(a1.id, id(1, 1));
    // Shown before the server answers.
    assert_eq!(
        alice
            .view()
            .record(table(1), record(1))
            .unwrap()
            .value(field(1)),
        text("draft")
    );
    assert!(alice.confirmed().record(table(1), record(1)).is_none());
    assert_eq!(alice.pending().count(), 1);
    let out = server.step(&a1).outcome;
    alice.receive(&out).unwrap();
    bob.receive(&out).unwrap();
    assert_eq!(alice.pending().count(), 0);
    assert_eq!(
        bob.confirmed()
            .record(table(1), record(1))
            .unwrap()
            .value(field(1)),
        text("draft")
    );
    // Both edit the same cell; Bob's pending edit is rebased on Alice's accepted one.
    let a2 = alice.submit(vec![set(1, 1, 1, text("Alice"), 1)]).unwrap();
    let b1 = bob.submit(vec![set(1, 1, 1, text("Bob"), 1)]).unwrap();
    let out_a = server.step(&a2).outcome;
    alice.receive(&out_a).unwrap();
    bob.receive(&out_a).unwrap();
    assert_eq!(
        bob.view()
            .record(table(1), record(1))
            .unwrap()
            .value(field(1)),
        text("Bob"),
        "his pending edit stays on top"
    );
    assert_eq!(
        bob.confirmed()
            .record(table(1), record(1))
            .unwrap()
            .value(field(1)),
        text("Alice")
    );
    let out_b = server.step(&b1).outcome;
    let Outcome::Accepted(Accepted { effects, .. }) = &out_b else {
        panic!()
    };
    assert!(
        matches!(&effects[0], mt_ops::Effect::Set { overwritten: Some(v), .. } if *v == text("Alice")),
        "Bob is told he overwrote Alice"
    );
    alice.receive(&out_b).unwrap();
    bob.receive(&out_b).unwrap();
    for replica in [&alice, &bob] {
        assert_eq!(
            replica.view().records().collect::<Vec<_>>(),
            server.records().collect::<Vec<_>>()
        );
    }
}

#[test]
fn std_refused_batches_leave_the_view() {
    let mut server = BaseState::new(base_id());
    let mut alice = Replica::new(client(1), 10, server.clone());
    let a = alice.submit(vec![create(1, 1, &[])]).unwrap();
    assert_eq!(a.id, id(1, 11), "numbers continue after the last one");
    alice.receive(&server.step(&a).outcome).unwrap();
    // A transition on a version the record no longer has.
    let mut bob = Replica::new(client(2), 0, alice.confirmed().clone());
    let b = bob.submit(vec![set(1, 1, 1, text("x"), 0)]).unwrap();
    let t = alice
        .submit(vec![transition(1, 1, 9, text("Submitted"), 1)])
        .unwrap();
    assert_eq!(
        alice
            .view()
            .record(table(1), record(1))
            .unwrap()
            .value(field(9)),
        text("Submitted"),
        "shown while it still applies"
    );
    let out_b = server.step(&b).outcome;
    alice.receive(&out_b).unwrap();
    assert_eq!(
        alice
            .view()
            .record(table(1), record(1))
            .unwrap()
            .value(field(9)),
        Value::Null,
        "Bob's change made it stale: no longer shown, the server will refuse it"
    );
    assert_eq!(alice.pending().count(), 1);
    let out_t = server.step(&t).outcome;
    assert!(matches!(out_t, Outcome::Rejected(_)));
    alice.receive(&out_t).unwrap();
    assert_eq!(alice.pending().count(), 0);
    assert_eq!(
        alice
            .view()
            .record(table(1), record(1))
            .unwrap()
            .value(field(9)),
        Value::Null,
        "gone from the view"
    );
    // A pending batch that no longer applies on the new confirmed state is left out of the
    // view (the server will refuse it).
    let d = alice.submit(vec![delete(1, 1, 2)]).unwrap();
    let mut carol = Replica::new(client(3), 0, alice.confirmed().clone());
    let c = carol.submit(vec![delete(1, 1, 2)]).unwrap();
    let out_c = server.step(&c).outcome;
    alice.receive(&out_c).unwrap();
    let again = alice.submit(vec![restore(1, 1)]).unwrap();
    assert!(
        !alice.view().record(table(1), record(1)).unwrap().deleted,
        "restore applies on top"
    );
    let _ = (d, again);
}

#[test]
fn invalid_missed_changes_are_a_gap() {
    let mut server = BaseState::new(base_id());
    let mut alice = Replica::new(client(1), 0, server.clone());
    let first = server.step(&batch(2, 1, vec![create(1, 1, &[])])).outcome;
    let second = server.step(&batch(2, 2, vec![create(1, 2, &[])])).outcome;
    assert_eq!(
        alice.receive(&second),
        Err(OpsError::Gap {
            expected: 1,
            got: 2
        })
    );
    alice.receive(&first).unwrap();
    alice.receive(&second).unwrap();
    // A repeat of something already applied is ignored.
    alice.receive(&first).unwrap();
    assert_eq!(alice.confirmed().seq(), 2);
    let mut full = Replica::new(client(1), u64::MAX, server.clone());
    assert_eq!(
        full.submit(vec![]),
        Err(OpsError::Limit("batch numbers exhausted"))
    );
}

/// A small deterministic generator (xorshift64*), so a failing run names its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

struct Sim {
    server: BaseState,
    journal: Vec<Accepted>,
    clients: Vec<Replica>,
    up: Vec<VecDeque<Batch>>,
    down: Vec<VecDeque<Outcome>>,
    made: Vec<u128>,
}

impl Sim {
    fn new(count: usize) -> Self {
        let server = BaseState::new(base_id());
        Sim {
            clients: (0..count)
                .map(|k| Replica::new(ClientId(k as u64 + 1), 0, server.clone()))
                .collect(),
            server,
            journal: Vec::new(),
            up: vec![VecDeque::new(); count],
            down: vec![VecDeque::new(); count],
            made: vec![0; count],
        }
    }

    fn edit(&mut self, k: usize, rng: &mut Rng) {
        let view = self.clients[k].view();
        let confirmed = self.clients[k].confirmed().clone();
        let known: Vec<_> = view.records().map(|(key, r)| (*key, r.deleted)).collect();
        let pick = |rng: &mut Rng| known.get(rng.below(known.len() as u64) as usize).copied();
        let value = |rng: &mut Rng| match rng.below(4) {
            0 => Value::Null,
            n => text(&format!("v{n}-{}", rng.below(3))),
        };
        let mut ops = Vec::new();
        for _ in 0..1 + rng.below(3) {
            let op = match (rng.below(10), pick(rng)) {
                (0..=2, _) | (_, None) => {
                    self.made[k] += 1;
                    let r = (k as u128 + 1) * 1_000_000 + self.made[k];
                    create(1 + u128::from(rng.below(2) as u8), r, &[(1, value(rng))])
                }
                (3..=6, Some(((t, r), _))) => {
                    let f = field(1 + u128::from(rng.below(3) as u8));
                    let seen = confirmed.record(t, r).map_or(0, |rec| rec.cell_version(f));
                    Op::Set {
                        table: t,
                        record: r,
                        field: f,
                        value: value(rng),
                        expected: seen,
                    }
                }
                (7, Some(((t, r), _))) => {
                    let seen = confirmed.record(t, r).map_or(0, |rec| rec.version);
                    Op::Transition {
                        table: t,
                        record: r,
                        field: field(9),
                        value: value(rng),
                        expected: seen,
                    }
                }
                (8, Some(((t, r), deleted))) => {
                    if deleted {
                        Op::Restore {
                            table: t,
                            record: r,
                        }
                    } else {
                        let seen = confirmed.record(t, r).map_or(0, |rec| rec.version);
                        Op::Delete {
                            table: t,
                            record: r,
                            expected: seen,
                        }
                    }
                }
                (_, Some(((t, r), _))) => Op::Restore {
                    table: t,
                    record: r,
                },
            };
            ops.push(op);
        }
        let batch = self.clients[k].submit(ops).unwrap();
        self.up[k].push_back(batch);
    }

    fn deliver_up(&mut self, k: usize) {
        let Some(batch) = self.up[k].pop_front() else {
            return;
        };
        let decision = self.server.step(&batch);
        match (&decision.outcome, decision.repeated) {
            (Outcome::Accepted(a), false) => {
                self.journal.push(a.clone());
                for queue in &mut self.down {
                    queue.push_back(decision.outcome.clone());
                }
            }
            (Outcome::Accepted(_), true) | (Outcome::Rejected(_), _) => {
                self.down[k].push_back(decision.outcome.clone())
            }
        }
    }

    fn deliver_down(&mut self, k: usize, seed: u64) {
        let Some(outcome) = self.down[k].pop_front() else {
            return;
        };
        if let Err(e) = self.clients[k].receive(&outcome) {
            panic!("seed {seed}: client {k} could not take {outcome:?}: {e}");
        }
    }

    /// The connection drops: what was in flight to the client is lost; it catches up from
    /// the journal and sends its unanswered batches again.
    fn reconnect(&mut self, k: usize) {
        self.down[k].clear();
        let after = self.clients[k].confirmed().seq();
        for accepted in self.journal.iter().filter(|a| a.seq > after) {
            self.down[k].push_back(Outcome::Accepted(accepted.clone()));
        }
        self.up[k] = self.clients[k].pending().cloned().collect();
    }

    fn quiet(&self) -> bool {
        self.up.iter().all(VecDeque::is_empty) && self.down.iter().all(VecDeque::is_empty)
    }
}

fn run(seed: u64, steps: usize, count: usize) {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut sim = Sim::new(count);
    for _ in 0..steps {
        let k = rng.below(count as u64) as usize;
        match rng.below(20) {
            0..=6 => sim.edit(k, &mut rng),
            7..=11 => sim.deliver_up(k),
            12..=17 => sim.deliver_down(k, seed),
            18 => {
                // A batch sent twice.
                if let Some(batch) = sim.clients[k].pending().next().cloned() {
                    sim.up[k].push_back(batch);
                }
            }
            _ => sim.reconnect(k),
        }
    }
    // Let everything arrive.
    let mut guard = 0;
    while !sim.quiet() {
        for k in 0..count {
            sim.deliver_up(k);
            sim.deliver_down(k, seed);
        }
        guard += 1;
        assert!(guard < 100_000, "seed {seed}: does not settle");
    }
    let truth: Vec<_> = sim.server.records().map(|(k, r)| (*k, r.clone())).collect();
    for (k, client) in sim.clients.iter().enumerate() {
        assert_eq!(
            client.pending().count(),
            0,
            "seed {seed}: client {k} has unanswered batches"
        );
        let view: Vec<_> = client
            .view()
            .records()
            .map(|(k, r)| (*k, r.clone()))
            .collect();
        assert_eq!(
            view, truth,
            "seed {seed}: client {k} differs from the server"
        );
        assert_eq!(client.confirmed().seq(), sim.server.seq(), "seed {seed}");
    }
    let seqs: Vec<u64> = sim.journal.iter().map(|a| a.seq).collect();
    assert_eq!(
        seqs,
        (1..=sim.server.seq()).collect::<Vec<_>>(),
        "seed {seed}: gaps in the journal"
    );
    let ids: BTreeSet<_> = sim.journal.iter().map(|a| a.id).collect();
    assert_eq!(
        ids.len(),
        sim.journal.len(),
        "seed {seed}: a batch applied twice"
    );
}

#[test]
fn concurrency_clients_editing_together_all_end_with_the_servers_state() {
    for seed in 1..=300 {
        run(seed, 400, 1 + (seed % 4) as usize);
    }
}

#[test]
fn concurrency_long_runs_converge() {
    for seed in [7, 42, 2026] {
        run(seed, 5_000, 5);
    }
}

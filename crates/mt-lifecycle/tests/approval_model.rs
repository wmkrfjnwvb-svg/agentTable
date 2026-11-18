#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
//! Model-based and simulation tests of the approval engine.
//!
//! * A reference model of one approval step (every mode) is compared with the engine after
//!   every decision.
//! * Random definitions (branches, parallel blocks, fill-in, notifications, timeouts) are
//!   driven by random events; invariants are checked after every event, refused events must
//!   change nothing, and replays and persisted copies must behave identically.
//! * Order independence: in "all" and "quorum" modes the outcome does not depend on the order
//!   in which people decide (checked on several threads at once).
//!
//! Every run is seeded; a failure prints its seed.

mod common;

use common::*;
use mt_core::id::PrincipalId;
use mt_lifecycle::approval::{
    Act, Approval, ApprovalDef, Approver, Case, Event, Join, Mode, Node, Output, Status, TaskKind,
    TaskStatus,
};
use std::collections::BTreeSet;

/// SplitMix64: small, seedable, good enough for driving simulations.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(n).unwrap()).unwrap()
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    fn pick<T: Clone>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())].clone()
    }
}

// ---- reference model of one step ----

#[derive(Debug, Clone, Copy, PartialEq)]
enum Decision {
    Approve,
    Reject,
}

fn expected(mode: Mode, n: usize, decisions: &[Decision]) -> Status {
    let approved = decisions
        .iter()
        .filter(|d| **d == Decision::Approve)
        .count();
    let rejected = decisions.len() - approved;
    match mode {
        Mode::Sequential | Mode::All if rejected > 0 => Status::Rejected,
        Mode::Sequential | Mode::All if approved == n => Status::Approved,
        Mode::Any => match decisions.first() {
            Some(Decision::Approve) => Status::Approved,
            Some(Decision::Reject) => Status::Rejected,
            None => Status::Running,
        },
        Mode::Quorum { required } if approved >= usize::from(required) => Status::Approved,
        Mode::Quorum { required } if rejected > n - usize::from(required) => Status::Rejected,
        Mode::Sequential | Mode::All | Mode::Quorum { .. } => Status::Running,
    }
}

#[test]
fn one_step_matches_the_reference_model_in_every_mode() {
    for seed in 0..3_000_u64 {
        let mut rng = Rng(seed);
        let n = 1 + rng.below(5);
        let approvers: Vec<u128> = (0..n).map(|i| 20 + u128::try_from(i).unwrap()).collect();
        let mode = match rng.below(4) {
            0 => Mode::Sequential,
            1 => Mode::All,
            2 => Mode::Any,
            _ => Mode::Quorum {
                required: u16::try_from(1 + rng.below(n)).unwrap(),
            },
        };
        let d = def(vec![approval_step("step", people(&approvers), mode)]);
        let w = World::new(100);
        let (mut a, _) = w.start(&d);
        let mut decisions: Vec<Decision> = Vec::new();
        let mut decided: BTreeSet<u128> = BTreeSet::new();
        while a.status == Status::Running {
            // Who should be asked now, according to the model.
            let waiting: Vec<u128> = match mode {
                Mode::Sequential => approvers
                    .iter()
                    .copied()
                    .filter(|p| !decided.contains(p))
                    .take(1)
                    .collect(),
                Mode::All | Mode::Any | Mode::Quorum { .. } => approvers
                    .iter()
                    .copied()
                    .filter(|p| !decided.contains(p))
                    .collect(),
            };
            assert_eq!(open_people(&a), waiting, "seed {seed}: {mode:?}");
            let who = rng.pick(&waiting);
            let decision = if rng.chance(60) {
                Decision::Approve
            } else {
                Decision::Reject
            };
            match decision {
                Decision::Approve => w.approve(&mut a, &d, who),
                Decision::Reject => w.reject(&mut a, &d, who),
            };
            decided.insert(who);
            decisions.push(decision);
            assert_eq!(
                a.status,
                expected(mode, n, &decisions),
                "seed {seed}: {mode:?} {decisions:?}"
            );
        }
        assert!(a.open_tasks().is_empty(), "seed {seed}");
    }
}

fn people(ns: &[u128]) -> Vec<Approver> {
    ns.iter().map(|n| person(*n)).collect()
}

// ---- order independence (concurrency) ----

fn permutations(items: &[u128]) -> Vec<Vec<u128>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut out = Vec::new();
    for (i, first) in items.iter().enumerate() {
        let mut rest = items.to_vec();
        rest.remove(i);
        for mut tail in permutations(&rest) {
            tail.insert(0, *first);
            out.push(tail);
        }
    }
    out
}

/// Applies decisions in `order` (people already closed are skipped) and returns the outcome.
fn outcome(d: &ApprovalDef, order: &[u128], approve: &BTreeSet<u128>) -> Status {
    let w = World::new(100);
    let (mut a, _) = w.start(d);
    for who in order {
        if a.status != Status::Running {
            break;
        }
        if a.tasks_of(p(*who)).is_empty() {
            continue;
        }
        if approve.contains(who) {
            w.approve(&mut a, d, *who);
        } else {
            w.reject(&mut a, d, *who);
        }
    }
    a.status
}

#[test]
fn all_and_quorum_outcomes_do_not_depend_on_who_decides_first() {
    let approvers = [20_u128, 21, 22, 23];
    let modes = [
        Mode::All,
        Mode::Quorum { required: 2 },
        Mode::Quorum { required: 3 },
    ];
    std::thread::scope(|scope| {
        for mode in modes {
            for mask in 0_u32..16 {
                scope.spawn(move || {
                    let approve: BTreeSet<u128> = approvers
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| mask & (1 << i) != 0)
                        .map(|(_, p)| *p)
                        .collect();
                    let d = def(vec![approval_step("step", people(&approvers), mode)]);
                    let outcomes: BTreeSet<String> = permutations(&approvers)
                        .iter()
                        .map(|order| format!("{:?}", outcome(&d, order, &approve)))
                        .collect();
                    assert_eq!(
                        outcomes.len(),
                        1,
                        "{mode:?} approvers {approve:?}: {outcomes:?}"
                    );
                    let expected = match mode {
                        Mode::All if approve.len() == 4 => Status::Approved,
                        Mode::Quorum { required } if approve.len() >= usize::from(required) => {
                            Status::Approved
                        }
                        Mode::Sequential | Mode::All | Mode::Any | Mode::Quorum { .. } => {
                            Status::Rejected
                        }
                    };
                    assert_eq!(
                        outcomes.into_iter().next().unwrap(),
                        format!("{expected:?}")
                    );
                });
            }
        }
    });
}

// ---- random definitions, random events ----

const STAFF: [u128; 9] = [
    MANAGER, CONTROLLER, CFO, LEGAL, FIN_A, FIN_B, FIN_C, DEPUTY, OUTSIDER,
];

fn approver(rng: &mut Rng) -> Approver {
    match rng.below(10) {
        0 => Approver::Field {
            field: f(MANAGER_FIELD),
        },
        1 => Approver::Capability {
            capability: "request.approve.finance".to_owned(),
        },
        2 => person(REQUESTER),
        _ => person(rng.pick(&STAFF)),
    }
}

fn node(rng: &mut Rng, depth: usize, counter: &mut usize) -> Node {
    *counter += 1;
    let key = format!("n{counter}");
    let kind = if depth >= 2 {
        rng.below(3)
    } else {
        rng.below(5)
    };
    match kind {
        0 | 1 => {
            let count = 1 + rng.below(3);
            let approvers: Vec<Approver> = (0..count).map(|_| approver(rng)).collect();
            let mode = match rng.below(4) {
                0 => Mode::Sequential,
                1 => Mode::All,
                2 => Mode::Any,
                _ => Mode::Quorum {
                    required: u16::try_from(1 + rng.below(count.min(2))).unwrap(),
                },
            };
            Node::Approval {
                key: key.clone(),
                name: key,
                approvers,
                mode,
                allow_return: rng.chance(30),
                timeout: rng
                    .chance(40)
                    .then(|| hours(1 + i64::try_from(rng.below(48)).unwrap())),
                escalate_to: rng.chance(50).then(|| approver(rng)),
            }
        }
        2 => {
            if rng.chance(50) {
                Node::FillIn {
                    key: key.clone(),
                    name: key,
                    assignee: approver(rng),
                    fields: vec![f(PAYMENT_DATE)],
                }
            } else {
                Node::Notify {
                    key: key.clone(),
                    name: key,
                    to: vec![approver(rng), approver(rng)],
                }
            }
        }
        3 => {
            let cases = (0..1 + rng.below(2))
                .map(|_| {
                    let limit = i64::try_from(rng.below(40_000)).unwrap();
                    Case {
                        name: format!("Up to {limit}"),
                        when: amount_le(limit),
                        then: list(rng, depth + 1, counter),
                    }
                })
                .collect();
            Node::Branch {
                key: key.clone(),
                name: key,
                cases,
                otherwise: list(rng, depth + 1, counter),
            }
        }
        _ => {
            let count = 2 + rng.below(2);
            let branches: Vec<Vec<Node>> =
                (0..count).map(|_| list(rng, depth + 1, counter)).collect();
            let join = match rng.below(3) {
                0 => Join::All,
                1 => Join::Any,
                _ => Join::AtLeast {
                    count: u16::try_from(1 + rng.below(count)).unwrap(),
                },
            };
            Node::Parallel {
                key: key.clone(),
                name: key,
                branches,
                join,
            }
        }
    }
}

fn list(rng: &mut Rng, depth: usize, counter: &mut usize) -> Vec<Node> {
    (0..rng.below(3) + usize::from(depth == 0))
        .map(|_| node(rng, depth, counter))
        .collect()
}

fn random_def(rng: &mut Rng) -> ApprovalDef {
    let mut counter = 0;
    let mut steps = list(rng, 0, &mut counter);
    if steps.is_empty() {
        steps.push(approval_step("only", vec![person(MANAGER)], Mode::Any));
    }
    def(steps)
}

fn random_event(rng: &mut Rng, a: &Approval) -> Event {
    let open: Vec<(u32, PrincipalId, TaskKind)> = a
        .open_tasks()
        .iter()
        .map(|t| (t.id, t.assignee, t.kind))
        .collect();
    let roll = rng.below(100);
    if open.is_empty() || roll < 8 {
        return Event::Tick;
    }
    if roll < 10 {
        return Event::Cancel { by: p(REQUESTER) };
    }
    let (task, assignee, kind) = rng.pick(&open);
    let actor = if rng.chance(5) {
        p(rng.pick(&STAFF))
    } else {
        assignee
    };
    let act = match (rng.below(12), kind) {
        (0..=5, TaskKind::Approve) => Act::Approve { comment: None },
        (0..=5, TaskKind::FillIn) => Act::Submit,
        (6 | 7, _) => Act::Reject {
            comment: if rng.chance(90) {
                "No".to_owned()
            } else {
                String::new()
            },
        },
        (8, _) => Act::Return {
            comment: "Fix it".to_owned(),
        },
        (9, _) => Act::Delegate {
            to: p(rng.pick(&[REQUESTER, DEPUTY, OUTSIDER, CFO, MANAGER])),
        },
        (10, _) => Act::AddApprover {
            who: p(rng.pick(&[REQUESTER, LEGAL, OUTSIDER, DEPUTY])),
        },
        _ => Act::Comment {
            text: "Looking".to_owned(),
        },
    };
    Event::Act { task, actor, act }
}

/// Checks the invariants of an approval and of the outputs just produced.
fn invariants(seed: u64, a: &Approval, outputs: &[Output], before: usize) {
    let open = a.open_tasks();
    match a.status {
        Status::Running => assert!(!open.is_empty(), "seed {seed}: running without a task"),
        Status::Stuck
        | Status::Approved
        | Status::Rejected
        | Status::Returned
        | Status::Cancelled => assert!(
            open.is_empty(),
            "seed {seed}: {:?} with open tasks",
            a.status
        ),
    }
    assert_eq!(a.status.is_final(), a.ended_at.is_some(), "seed {seed}");
    for (index, task) in a.tasks.iter().enumerate() {
        assert_eq!(
            usize::try_from(task.id).unwrap(),
            index + 1,
            "seed {seed}: task numbering"
        );
        if task.kind == TaskKind::Approve {
            assert_ne!(task.assignee, a.requester, "seed {seed}: own request");
        }
    }
    let mut seen = BTreeSet::new();
    for task in &open {
        assert!(
            seen.insert((task.node.clone(), task.assignee)),
            "seed {seed}: asked twice at a step: {:#?}\n{:#?}",
            a.tasks,
            a.history
        );
    }
    let mut next = before;
    for output in outputs {
        match output {
            Output::Opened {
                task,
                assignee: _,
                node,
            } => {
                next += 1;
                assert_eq!(usize::try_from(*task).unwrap(), next, "seed {seed}");
                assert!(
                    a.tasks.iter().any(|t| t.id == *task && &t.node == node),
                    "seed {seed}"
                );
            }
            Output::Closed { task, status } => {
                assert_ne!(*status, TaskStatus::Open, "seed {seed}");
                let now = a.tasks.iter().find(|t| t.id == *task).unwrap().status;
                assert_eq!(
                    now, *status,
                    "seed {seed}: task {task} closed as {status:?}"
                );
            }
            Output::Finished { status } => assert_eq!(*status, a.status, "seed {seed}"),
            Output::Notified { .. }
            | Output::Reassigned { .. }
            | Output::Overdue { .. }
            | Output::Stuck { .. } => {}
        }
    }
    assert_eq!(
        next,
        a.tasks.len(),
        "seed {seed}: every new task is announced"
    );
}

#[test]
fn random_definitions_under_random_events_keep_every_invariant() {
    let mut endings: std::collections::BTreeMap<String, usize> = Default::default();
    for seed in 0..1_500_u64 {
        let mut rng = Rng(seed ^ 0xA5A5_5A5A);
        let d = random_def(&mut rng);
        d.check().unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        let mut w = World::new(i64::try_from(rng.below(50_000)).unwrap());
        let (mut a, out) = w.start(&d);
        invariants(seed, &a, &out, 0);
        let mut finished = usize::from(out.iter().any(|o| matches!(o, Output::Finished { .. })));
        // From a random point on, a copy persisted as JSON follows the same events.
        let mut shadow: Option<Approval> = None;
        for _ in 0..60 {
            if a.status.is_final() {
                break;
            }
            if shadow.is_none() && rng.chance(10) {
                shadow = Some(serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap());
            }
            let event = random_event(&mut rng, &a);
            w.now = plus(w.now, i64::try_from(rng.below(30)).unwrap() * HOUR / 10);
            let before = a.clone();
            let result = w.step(&mut a, &d, event.clone());
            if let Some(copy) = shadow.as_mut() {
                let other = w.step(copy, &d, event.clone());
                assert_eq!(
                    other.as_ref().map_err(ToString::to_string),
                    result.as_ref().map_err(ToString::to_string),
                    "seed {seed}"
                );
                assert_eq!(copy, &a, "seed {seed}: the persisted copy diverged");
            }
            match result {
                Ok(out) => {
                    invariants(seed, &a, &out, before.tasks.len());
                    finished += out
                        .iter()
                        .filter(|o| matches!(o, Output::Finished { .. }))
                        .count();
                    if before.status == Status::Stuck {
                        assert!(
                            matches!(a.status, Status::Stuck | Status::Cancelled),
                            "seed {seed}"
                        );
                    }
                }
                Err(_) => assert_eq!(
                    a, before,
                    "seed {seed}: a refused event changed the approval"
                ),
            }
        }
        assert_eq!(
            finished,
            usize::from(a.status.is_final()),
            "seed {seed}: one Finished"
        );
        *endings.entry(format!("{:?}", a.status)).or_default() += 1;
    }
    // The generator reaches every ending (runs still going after 60 events are rare).
    for status in ["Approved", "Rejected", "Returned", "Cancelled", "Stuck"] {
        assert!(
            endings.get(status).copied().unwrap_or(0) > 0,
            "{status} never happened: {endings:?}"
        );
    }
}

#[test]
fn replaying_the_same_events_gives_the_same_approval() {
    for seed in 0..500_u64 {
        let mut rng = Rng(seed.wrapping_mul(7919));
        let d = random_def(&mut rng);
        let amount = i64::try_from(rng.below(50_000)).unwrap();
        let mut w = World::new(amount);
        let (mut a, _) = w.start(&d);
        let mut log: Vec<(Event, i64, Result<Vec<Output>, String>)> = Vec::new();
        for _ in 0..40 {
            if a.status.is_final() {
                break;
            }
            let event = random_event(&mut rng, &a);
            let gap = i64::try_from(rng.below(20)).unwrap() * HOUR / 4;
            w.now = plus(w.now, gap);
            let result = w.step(&mut a, &d, event.clone()).map_err(|e| e.to_string());
            log.push((event, gap, result));
        }
        // Replay on a fresh start and on a copy persisted as JSON at the start.
        let mut w2 = World::new(amount);
        let (fresh, _) = w2.start(&d);
        let mut stored: Approval =
            serde_json::from_str(&serde_json::to_string(&fresh).unwrap()).unwrap();
        let mut fresh = fresh;
        for (event, gap, result) in &log {
            w2.now = plus(w2.now, *gap);
            let left = w2
                .step(&mut fresh, &d, event.clone())
                .map_err(|e| e.to_string());
            let right = w2
                .step(&mut stored, &d, event.clone())
                .map_err(|e| e.to_string());
            assert_eq!(&left, result, "seed {seed}");
            assert_eq!(left, right, "seed {seed}");
        }
        assert_eq!(fresh, a, "seed {seed}");
        assert_eq!(stored, a, "seed {seed}");
    }
}

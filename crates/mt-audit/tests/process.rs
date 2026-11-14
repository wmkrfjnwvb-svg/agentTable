//! Process contracts, completeness verifier and exception state machine.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic
)]

mod common;

use common::{user, Rng};
use mt_audit::{
    AuditError, BrokenReason, CompletenessVerifier, ExceptionDescriptor, ExceptionEvent as X,
    ExceptionKind, ExceptionStatus as St, InstanceStatus, LateReason, ObjectRef, ProcessContract,
    ProcessEvent,
};
use proptest::prelude::*;

fn ev(seq: u64, instance: &str, kind: &str, at_ms: i64) -> ProcessEvent {
    ProcessEvent::new(seq, instance, kind, at_ms)
}

/// Ticket process: closed is terminal; after `resolved` the requester must be notified within 1000 ms.
fn ticket() -> ProcessContract {
    ProcessContract::new("ticket", ["closed", "cancelled"])
        .unwrap()
        .with_follow_up("resolved", "requester_notified", 1_000)
        .unwrap()
        .with_reopen_event("reopened")
        .unwrap()
}

fn status(v: &CompletenessVerifier, instance: &str, now: i64) -> InstanceStatus {
    v.evaluate_instance(instance, now).unwrap().status
}

// ---- standard ----

#[test]
fn standard_complete_in_progress_and_broken() {
    let mut v = CompletenessVerifier::new(ticket());
    assert_eq!(v.contract().id(), "ticket");
    assert_eq!(v.contract().follow_ups().len(), 1);
    let applied = v
        .ingest(&[
            ev(1, "t1", "opened", 0),
            ev(2, "t2", "opened", 10),
            ev(3, "t1", "resolved", 100),
            ev(4, "t2", "resolved", 200),
            ev(5, "t1", "requester_notified", 600),
            ev(6, "t1", "closed", 700),
            ev(7, "t3", "opened", 800),
        ])
        .unwrap();
    assert_eq!((applied, v.cursor(), v.instance_count()), (7, 7, 3));

    let at_900 = v.evaluate(900);
    assert_eq!(
        at_900
            .iter()
            .map(|r| r.instance.as_str())
            .collect::<Vec<_>>(),
        ["t1", "t2", "t3"]
    );
    assert_eq!(at_900[0].status, InstanceStatus::Complete);
    assert_eq!((at_900[0].first_seq, at_900[0].last_seq), (1, 6));
    assert_eq!(
        at_900[1].status,
        InstanceStatus::InProgress,
        "deadline 1200 not reached"
    );
    assert_eq!(at_900[2].status, InstanceStatus::InProgress);
    assert!(at_900.iter().all(|r| r.exceptions.is_empty()));

    // The deadline is inclusive: at exactly 1200 nothing is broken yet.
    assert_eq!(status(&v, "t2", 1_200), InstanceStatus::InProgress);
    let late = v.evaluate_instance("t2", 1_201).unwrap();
    assert_eq!(
        late.status,
        InstanceStatus::Broken(BrokenReason::MissingFollowUp {
            trigger: "resolved".into(),
            required: "requester_notified".into(),
            trigger_seq: 4,
            deadline_ms: 1_200,
        })
    );
    assert_eq!(late.exceptions.len(), 1);
    let x = &late.exceptions[0];
    assert_eq!(x.kind, ExceptionKind::MissingStep);
    assert_eq!(x.key, "ticket/t2/missing_step/requester_notified/4");
    assert_eq!(x.subject, ObjectRef::new("process_instance", "t2"));
    assert_eq!(
        (x.status, x.opened_at_ms, &x.owner),
        (St::Open, 1_200, &None)
    );
    assert_eq!(v.evaluate_instance("nope", 0), None);
}

#[test]
fn standard_late_follow_up_heals_broken_into_late() {
    let mut v = CompletenessVerifier::new(ticket());
    v.ingest(&[ev(1, "t", "resolved", 0)]).unwrap();
    assert!(matches!(status(&v, "t", 5_000), InstanceStatus::Broken(_)));
    v.ingest(&[
        ev(2, "t", "requester_notified", 4_000),
        ev(3, "t", "closed", 4_100),
    ])
    .unwrap();
    let r = v.evaluate_instance("t", 5_000).unwrap();
    assert_eq!(
        r.status,
        InstanceStatus::Late(LateReason::FollowUpLate {
            trigger: "resolved".into(),
            required: "requester_notified".into(),
            deadline_ms: 1_000,
            done_at_ms: 4_000,
        })
    );
    assert!(r.exceptions.is_empty(), "nothing left to act on");
    // On-time notification exactly at the deadline is on time.
    let mut ok = CompletenessVerifier::new(ticket());
    ok.ingest(&[
        ev(1, "t", "resolved", 0),
        ev(2, "t", "requester_notified", 1_000),
        ev(3, "t", "closed", 1_001),
    ])
    .unwrap();
    assert_eq!(status(&ok, "t", 9_999), InstanceStatus::Complete);
}

#[test]
fn standard_duration_and_stuck_rules() {
    let contract = ProcessContract::new("order", ["done"])
        .unwrap()
        .with_max_duration_ms(1_000)
        .unwrap()
        .with_stuck_after_ms(300)
        .unwrap();
    let mut v = CompletenessVerifier::new(contract);
    v.ingest(&[
        ev(1, "fast", "start", 0),
        ev(2, "fast", "done", 1_000),
        ev(3, "slow", "start", 0),
        ev(4, "slow", "done", 1_001),
        ev(5, "idle", "start", 2_000),
        ev(6, "busy", "start", 0),
        ev(7, "busy", "work", 1_900),
    ])
    .unwrap();
    assert_eq!(status(&v, "fast", 9_000), InstanceStatus::Complete);
    assert_eq!(
        status(&v, "slow", 9_000),
        InstanceStatus::Late(LateReason::CompletedLate {
            deadline_ms: 1_000,
            completed_at_ms: 1_001
        })
    );
    assert_eq!(status(&v, "idle", 2_300), InstanceStatus::InProgress);
    let idle = v.evaluate_instance("idle", 2_301).unwrap();
    assert_eq!(
        idle.status,
        InstanceStatus::Broken(BrokenReason::Stuck {
            last_event_at_ms: 2_000,
            stuck_since_ms: 2_300
        })
    );
    assert_eq!(idle.exceptions[0].kind, ExceptionKind::StuckProcess);
    assert_eq!(idle.exceptions[0].key, "order/idle/stuck_process/5");
    // Not stuck yet, but past the maximum duration: late with an overdue exception.
    let busy = v.evaluate_instance("busy", 2_000).unwrap();
    assert_eq!(
        busy.status,
        InstanceStatus::Late(LateReason::DurationExceeded { deadline_ms: 1_000 })
    );
    assert_eq!(busy.exceptions.len(), 1);
    assert_eq!(
        (busy.exceptions[0].kind, busy.exceptions[0].key.as_str()),
        (ExceptionKind::Overdue, "order/busy/overdue")
    );
    // Both stuck and overdue: broken wins, both exceptions are reported.
    let both = v.evaluate_instance("busy", 5_000).unwrap();
    assert!(matches!(
        both.status,
        InstanceStatus::Broken(BrokenReason::Stuck { .. })
    ));
    let kinds: Vec<_> = both.exceptions.iter().map(|x| x.kind).collect();
    assert_eq!(kinds, [ExceptionKind::StuckProcess, ExceptionKind::Overdue]);
    // A finished instance is never stuck.
    assert_eq!(status(&v, "fast", i64::MAX), InstanceStatus::Complete);
}

// ---- anomalous ----

#[test]
fn anomalous_reopen_repeated_triggers_and_events_after_terminal() {
    let mut v = CompletenessVerifier::new(ticket());
    v.ingest(&[
        ev(1, "t", "resolved", 0),
        ev(2, "t", "closed", 10),
        // Closed while the notification is still pending and within its deadline.
    ])
    .unwrap();
    assert_eq!(status(&v, "t", 500), InstanceStatus::InProgress);
    v.ingest(&[ev(3, "t", "requester_notified", 600)]).unwrap();
    assert_eq!(status(&v, "t", 700), InstanceStatus::Complete);
    v.ingest(&[ev(4, "t", "reopened", 800)]).unwrap();
    assert_eq!(status(&v, "t", 900), InstanceStatus::InProgress);
    // Resolved twice; one notification covers both obligations.
    v.ingest(&[ev(5, "t", "resolved", 1_000), ev(6, "t", "resolved", 1_100)])
        .unwrap();
    let broken = v.evaluate_instance("t", 5_000).unwrap();
    assert_eq!(broken.exceptions.len(), 2);
    assert_eq!(
        broken
            .exceptions
            .iter()
            .map(|x| x.key.as_str())
            .collect::<Vec<_>>(),
        [
            "ticket/t/missing_step/requester_notified/5",
            "ticket/t/missing_step/requester_notified/6"
        ]
    );
    assert!(matches!(
        broken.status,
        InstanceStatus::Broken(BrokenReason::MissingFollowUp {
            trigger_seq: 5,
            deadline_ms: 2_000,
            ..
        })
    ));
    v.ingest(&[
        ev(7, "t", "requester_notified", 2_050),
        ev(8, "t", "cancelled", 2_060),
    ])
    .unwrap();
    // Late for the first obligation (deadline 2000), on time for the second (2100).
    assert!(matches!(
        status(&v, "t", 9_000),
        InstanceStatus::Late(LateReason::FollowUpLate {
            deadline_ms: 2_000,
            done_at_ms: 2_050,
            ..
        })
    ));
    // A second late follow-up does not replace the first recorded reason.
    v.ingest(&[
        ev(9, "t", "resolved", 3_000),
        ev(10, "t", "requester_notified", 9_000),
    ])
    .unwrap();
    assert!(matches!(
        status(&v, "t", 9_500),
        InstanceStatus::Late(LateReason::FollowUpLate {
            deadline_ms: 2_000,
            ..
        })
    ));
}

#[test]
fn anomalous_multiple_rules_and_orphan_follow_up() {
    let contract = ProcessContract::new("p", ["end"])
        .unwrap()
        .with_follow_up("a", "b", 10)
        .unwrap()
        .with_follow_up("b", "c", 10)
        .unwrap()
        .with_follow_up("a", "c", 100)
        .unwrap();
    let mut v = CompletenessVerifier::new(contract);
    // `b` without a preceding `a` is harmless but itself triggers `c`.
    v.ingest(&[ev(1, "i", "b", 0)]).unwrap();
    assert_eq!(status(&v, "i", 10), InstanceStatus::InProgress);
    assert!(matches!(status(&v, "i", 11), InstanceStatus::Broken(_)));
    v.ingest(&[ev(2, "i", "c", 5), ev(3, "i", "a", 20), ev(4, "i", "b", 25)])
        .unwrap();
    // Open: a→c (deadline 120), b→c (deadline 35). The earliest deadline is reported first.
    let r = v.evaluate_instance("i", 1_000).unwrap();
    assert_eq!(r.exceptions.len(), 2);
    assert!(matches!(
        r.status,
        InstanceStatus::Broken(BrokenReason::MissingFollowUp {
            deadline_ms: 35,
            trigger_seq: 4,
            ..
        })
    ));
    v.ingest(&[ev(5, "i", "c", 30), ev(6, "i", "end", 31)])
        .unwrap();
    assert_eq!(status(&v, "i", 1_000), InstanceStatus::Complete);
}

// ---- edge ----

#[test]
fn edge_time_extremes_do_not_overflow() {
    let contract = ProcessContract::new("p", ["end"])
        .unwrap()
        .with_follow_up("a", "b", i64::MAX)
        .unwrap()
        .with_max_duration_ms(i64::MAX)
        .unwrap()
        .with_stuck_after_ms(i64::MAX)
        .unwrap();
    let mut v = CompletenessVerifier::new(contract);
    v.ingest(&[ev(u64::MAX - 1, "i", "a", i64::MAX - 5)])
        .unwrap();
    assert_eq!(status(&v, "i", i64::MAX), InstanceStatus::InProgress);
    assert_eq!(status(&v, "i", i64::MIN), InstanceStatus::InProgress);
    assert_eq!(v.cursor(), u64::MAX - 1);
    // Zero-length deadline: the follow-up must carry the same timestamp.
    let zero = ProcessContract::new("z", ["end"])
        .unwrap()
        .with_follow_up("a", "b", 0)
        .unwrap();
    let mut v = CompletenessVerifier::new(zero);
    v.ingest(&[ev(1, "i", "a", 7), ev(2, "i", "b", 7), ev(3, "i", "end", 7)])
        .unwrap();
    assert_eq!(status(&v, "i", 8), InstanceStatus::Complete);
    assert!(v.evaluate(0).len() == 1 && CompletenessVerifier::new(ticket()).evaluate(0).is_empty());
}

// ---- incremental / resumable ----

fn journal(seed: u64, n: u64) -> Vec<ProcessEvent> {
    let kinds = [
        "opened",
        "resolved",
        "requester_notified",
        "closed",
        "reopened",
        "comment",
        "cancelled",
    ];
    let mut rng = Rng(seed);
    let mut at = 0i64;
    (1..=n)
        .map(|seq| {
            at += i64::try_from(rng.below(700)).unwrap();
            let kind = kinds[usize::try_from(rng.below(kinds.len() as u64)).unwrap()];
            ev(seq, &format!("t{}", rng.below(6)), kind, at)
        })
        .collect()
}

#[test]
fn incremental_resume_from_cursor_matches_single_pass() {
    for seed in 0..100u64 {
        let events = journal(seed, 120);
        let mut whole = CompletenessVerifier::new(ticket());
        whole.ingest(&events).unwrap();

        let mut rng = Rng(seed ^ 0xabcd);
        let mut part = CompletenessVerifier::new(ticket());
        let mut pos = 0usize;
        while pos < events.len() {
            let take = 1 + usize::try_from(rng.below(15)).unwrap();
            let end = (pos + take).min(events.len());
            // Overlap with already consumed events (re-delivered batch prefix).
            let from = pos.saturating_sub(usize::try_from(rng.below(4)).unwrap());
            let applied = part.ingest(&events[from..end]).unwrap();
            assert_eq!(applied, end - pos, "seed={seed}");
            // Persist and restore between batches.
            let saved = serde_json::to_string(&part).unwrap();
            part = serde_json::from_str(&saved).unwrap();
            assert_eq!(part.cursor(), events[end - 1].seq, "seed={seed}");
            pos = end;
        }
        assert_eq!(part, whole, "seed={seed}");
        for now in [0, 5_000, 50_000, i64::MAX] {
            assert_eq!(
                part.evaluate(now),
                whole.evaluate(now),
                "seed={seed} now={now}"
            );
        }
    }
}

#[test]
fn incremental_prune_complete_bounds_memory() {
    let mut v = CompletenessVerifier::new(ticket());
    v.ingest(&[
        ev(1, "a", "opened", 0),
        ev(2, "a", "closed", 1),
        ev(3, "b", "opened", 2),
    ])
    .unwrap();
    assert_eq!(v.prune_complete(10), 1);
    assert_eq!(v.instance_count(), 1);
    assert_eq!(v.prune_complete(10), 0);
    assert_eq!(v.cursor(), 3, "pruning does not move the cursor");
    v.ingest(&[ev(4, "a", "comment", 20)]).unwrap();
    assert_eq!(v.evaluate_instance("a", 30).unwrap().first_seq, 4);
}

// ---- invalid ----

#[test]
fn invalid_contracts_and_journals() {
    let c = |r: Result<ProcessContract, AuditError>, why: &'static str| {
        assert_eq!(r.unwrap_err(), AuditError::InvalidContract(why));
    };
    c(ProcessContract::new("", ["end"]), "empty id");
    c(
        ProcessContract::new("p", Vec::<String>::new()),
        "no terminal event",
    );
    c(ProcessContract::new("p", [""]), "empty event kind");
    let p = || ProcessContract::new("p", ["end"]).unwrap();
    c(p().with_reopen_event(""), "empty event kind");
    c(
        p().with_reopen_event("end"),
        "event is both terminal and reopen",
    );
    c(p().with_follow_up("", "b", 1), "empty event kind");
    c(p().with_follow_up("a", "", 1), "empty event kind");
    c(
        p().with_follow_up("a", "a", 1),
        "follow-up requires its own trigger",
    );
    c(p().with_follow_up("a", "b", -1), "negative deadline");
    c(p().with_max_duration_ms(0), "non-positive maximum duration");
    c(p().with_stuck_after_ms(-5), "non-positive stuck threshold");

    let mut v = CompletenessVerifier::new(ticket());
    v.ingest(&[ev(1, "t", "opened", 0), ev(2, "t", "resolved", 1)])
        .unwrap();
    let before = v.clone();
    assert_eq!(
        v.ingest(&[ev(3, "t", "x", 2), ev(3, "t", "y", 3)]),
        Err(AuditError::JournalOutOfOrder { seq: 3, after: 3 })
    );
    assert_eq!(
        v.ingest(&[ev(5, "t", "x", 2), ev(4, "t", "y", 3)]),
        Err(AuditError::JournalOutOfOrder { seq: 4, after: 5 })
    );
    assert_eq!(
        v.ingest(&[ev(0, "t", "x", 2)]),
        Err(AuditError::ZeroSequence)
    );
    assert_eq!(v, before, "a rejected batch consumes nothing");
    assert_eq!(v.ingest(&[]).unwrap(), 0);
    assert_eq!(
        v.ingest(&[ev(1, "t", "opened", 0)]).unwrap(),
        0,
        "old events are skipped"
    );
    assert_eq!(v, before);
}

// ---- properties ----

proptest! {
    /// Classification is consistent with the reported exceptions, for arbitrary journals and times.
    #[test]
    fn prop_reports_are_internally_consistent(seed in any::<u64>(), n in 0u64..150, now in 0i64..120_000) {
        let contract = ticket().with_max_duration_ms(20_000).unwrap().with_stuck_after_ms(8_000).unwrap();
        let mut v = CompletenessVerifier::new(contract);
        v.ingest(&journal(seed, n)).unwrap();
        for r in v.evaluate(now) {
            let has = |k: ExceptionKind| r.exceptions.iter().any(|x| x.kind == k);
            match &r.status {
                InstanceStatus::Complete | InstanceStatus::InProgress => prop_assert!(r.exceptions.is_empty()),
                InstanceStatus::Late(LateReason::DurationExceeded { .. }) => {
                    prop_assert!(has(ExceptionKind::Overdue));
                    prop_assert!(!has(ExceptionKind::MissingStep) && !has(ExceptionKind::StuckProcess));
                }
                InstanceStatus::Late(_) => {
                    prop_assert!(!has(ExceptionKind::MissingStep) && !has(ExceptionKind::StuckProcess));
                }
                InstanceStatus::Broken(BrokenReason::MissingFollowUp { deadline_ms, .. }) => {
                    prop_assert!(has(ExceptionKind::MissingStep) && *deadline_ms < now);
                }
                InstanceStatus::Broken(BrokenReason::Stuck { stuck_since_ms, .. }) => {
                    prop_assert!(has(ExceptionKind::StuckProcess) && *stuck_since_ms < now);
                    prop_assert!(!has(ExceptionKind::MissingStep));
                }
            }
            let mut keys: Vec<&str> = r.exceptions.iter().map(|x| x.key.as_str()).collect();
            keys.sort_unstable();
            keys.dedup();
            prop_assert_eq!(keys.len(), r.exceptions.len(), "exception keys are unique");
            prop_assert!(r.first_seq <= r.last_seq);
        }
    }
}

// ---- exception state machine ----

fn exception() -> ExceptionDescriptor {
    ExceptionDescriptor::open(
        "k",
        ExceptionKind::Overdue,
        ObjectRef::new("record", "r1"),
        "late",
        100,
    )
}

fn assign(who: &str) -> X {
    X::Assign { owner: user(who) }
}

#[test]
fn exception_standard_lifecycle() {
    let x = exception();
    assert_eq!(
        (x.status, x.owner.clone(), x.closed_at_ms, x.trace_id),
        (St::Open, None, None, None)
    );
    let x = x.step(&assign("ann")).unwrap();
    assert_eq!((x.status, x.owner.clone()), (St::Open, Some(user("ann"))));
    let x = x.step(&X::Acknowledge).unwrap();
    assert_eq!(x.status, St::Acknowledged);
    assert_eq!(
        x.step(&assign("ann")).unwrap().status,
        St::Acknowledged,
        "same owner keeps status"
    );
    let x = x.step(&X::Start).unwrap();
    assert_eq!(x.status, St::InProgress);
    let handed_over = x.step(&assign("bob")).unwrap();
    assert_eq!(
        (handed_over.status, handed_over.owner.clone()),
        (St::Open, Some(user("bob")))
    );
    let done = x.step(&X::Resolve { at_ms: 500 }).unwrap();
    assert_eq!((done.status, done.closed_at_ms), (St::Resolved, Some(500)));
    assert!(done.status.is_closed());
    let again = done.step(&X::Reopen).unwrap();
    assert_eq!(
        (again.status, again.closed_at_ms, again.owner.clone()),
        (St::Open, None, Some(user("ann")))
    );
    let dismissed = again.step(&X::Dismiss { at_ms: 900 }).unwrap();
    assert_eq!(
        (dismissed.status, dismissed.closed_at_ms),
        (St::Dismissed, Some(900))
    );
    // System auto-resolution needs no owner; an owned open exception can be started directly.
    assert_eq!(
        exception().step(&X::Resolve { at_ms: 1 }).unwrap().status,
        St::Resolved
    );
    assert_eq!(
        exception()
            .step(&assign("a"))
            .unwrap()
            .step(&X::Start)
            .unwrap()
            .status,
        St::InProgress
    );
    let json = serde_json::to_string(&dismissed).unwrap();
    assert!(json.contains(r#""kind":"overdue""#) && json.contains(r#""status":"dismissed""#));
    assert_eq!(
        serde_json::from_str::<ExceptionDescriptor>(&json).unwrap(),
        dismissed
    );
}

#[test]
fn exception_names_match_serialised_form() {
    let kinds = [
        ExceptionKind::Unassigned,
        ExceptionKind::Overdue,
        ExceptionKind::FailedAction,
        ExceptionKind::UnmappedValue,
        ExceptionKind::MissingSubmission,
        ExceptionKind::Quality,
        ExceptionKind::AiQuality,
        ExceptionKind::StuckProcess,
        ExceptionKind::MissingStep,
        ExceptionKind::UndeliveredOperation,
        ExceptionKind::AuditIntegrity,
    ];
    for k in kinds {
        assert_eq!(
            serde_json::to_string(&k).unwrap(),
            format!("\"{}\"", k.name())
        );
    }
    for s in [
        St::Open,
        St::Acknowledged,
        St::InProgress,
        St::Resolved,
        St::Dismissed,
    ] {
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            format!("\"{}\"", s.name())
        );
    }
}

#[test]
fn exception_invalid_transitions() {
    let open = exception();
    let owned = open.step(&assign("a")).unwrap();
    let ack = owned.step(&X::Acknowledge).unwrap();
    let wip = ack.step(&X::Start).unwrap();
    let resolved = wip.step(&X::Resolve { at_ms: 1 }).unwrap();
    let dismissed = wip.step(&X::Dismiss { at_ms: 1 }).unwrap();
    let cases: Vec<(&ExceptionDescriptor, X)> = vec![
        (&open, X::Acknowledge), // no owner
        (&open, X::Start),       // no owner
        (&open, X::Reopen),
        (&ack, X::Acknowledge),
        (&ack, X::Reopen),
        (&wip, X::Acknowledge),
        (&wip, X::Start),
        (&wip, X::Reopen),
        (&resolved, assign("b")),
        (&resolved, X::Acknowledge),
        (&resolved, X::Start),
        (&resolved, X::Resolve { at_ms: 2 }),
        (&resolved, X::Dismiss { at_ms: 2 }),
        (&dismissed, assign("b")),
        (&dismissed, X::Acknowledge),
        (&dismissed, X::Start),
        (&dismissed, X::Resolve { at_ms: 2 }),
        (&dismissed, X::Dismiss { at_ms: 2 }),
    ];
    for (state, event) in cases {
        assert_eq!(
            state.step(&event),
            Err(AuditError::IllegalExceptionTransition {
                from: state.status.name(),
                event: event.name()
            }),
            "{:?} on {event:?}",
            state.status
        );
    }
}

fn random_exception_event(rng: &mut Rng) -> X {
    match rng.below(7) {
        0 | 1 => assign(["ann", "bob"][usize::try_from(rng.below(2)).unwrap()]),
        2 => X::Acknowledge,
        3 => X::Start,
        4 => X::Resolve {
            at_ms: i64::try_from(rng.below(1_000)).unwrap(),
        },
        5 => X::Dismiss {
            at_ms: i64::try_from(rng.below(1_000)).unwrap(),
        },
        _ => X::Reopen,
    }
}

fn run_exception_model(seed: u64) {
    let mut rng = Rng(seed);
    let mut x = exception();
    for i in 0..80 {
        let event = random_exception_event(&mut rng);
        let before = x.clone();
        // Reference model: which events are allowed where.
        let closed = before.status.is_closed();
        let allowed = match &event {
            X::Assign { .. } | X::Resolve { .. } | X::Dismiss { .. } => !closed,
            X::Acknowledge => before.status == St::Open && before.owner.is_some(),
            X::Start => {
                matches!(before.status, St::Open | St::Acknowledged) && before.owner.is_some()
            }
            X::Reopen => closed,
        };
        let result = before.step(&event);
        let ctx = format!("seed={seed} step={i} event={event:?} before={before:?}");
        assert_eq!(result.is_ok(), allowed, "{ctx}");
        if let Ok(next) = result {
            x = next;
        }
        // Invariants.
        assert_eq!(x.closed_at_ms.is_some(), x.status.is_closed(), "{ctx}");
        if matches!(x.status, St::Acknowledged | St::InProgress) {
            assert!(x.owner.is_some(), "{ctx}");
        }
        assert_eq!(
            (&x.key, x.kind, &x.subject, x.opened_at_ms),
            (
                &before.key,
                before.kind,
                &before.subject,
                before.opened_at_ms
            ),
            "{ctx}"
        );
        if before.owner.is_some() {
            assert!(x.owner.is_some(), "owner is never lost; {ctx}");
        }
    }
}

#[test]
fn exception_model_fixed_seeds() {
    for seed in 0..300 {
        run_exception_model(seed);
    }
}

proptest! {
    #[test]
    fn exception_model_random_seeds(seed in any::<u64>()) {
        run_exception_model(seed);
    }
}

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
//! The approval engine: modes, branches, parallel blocks, fill-in, notifications, delegation,
//! added approvers, send-back, escalation, cancellation, versions, simulation and persistence.

mod common;

use common::*;
use mt_core::condition::Operator;
use mt_core::field::DataType;
use mt_core::id::FieldId;
use mt_core::value::Value;
use mt_lifecycle::approval::{
    simulate, Act, Approval, ApprovalDef, Approver, Case, Event, Join, Mode, Node, Output,
    SimulatedStep, Status, TaskKind, TaskStatus, MAX_NESTING, MAX_NODES, MAX_TASKS,
};
use mt_lifecycle::LifecycleError;

fn people(ns: &[u128]) -> Vec<Approver> {
    ns.iter().map(|n| person(*n)).collect()
}

fn single(approvers: &[u128], mode: Mode) -> ApprovalDef {
    def(vec![approval_step("review", people(approvers), mode)])
}

fn finished(outputs: &[Output]) -> Option<Status> {
    outputs.iter().find_map(|o| match o {
        Output::Finished { status } => Some(*status),
        Output::Opened { .. }
        | Output::Closed { .. }
        | Output::Notified { .. }
        | Output::Reassigned { .. }
        | Output::Overdue { .. }
        | Output::Stuck { .. } => None,
    })
}

fn invalid(d: &ApprovalDef) -> String {
    match d.check() {
        Err(LifecycleError::Invalid(message)) => message,
        other => panic!("expected an invalid definition, got {other:?}"),
    }
}

fn not_allowed(result: Result<Vec<Output>, LifecycleError>) -> String {
    match result {
        Err(LifecycleError::NotAllowed(message)) => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

// ---- modes ----

#[test]
fn sequential_approvers_are_asked_one_after_another() {
    let w = World::new(500);
    let d = single(&[MANAGER, CONTROLLER], Mode::Sequential);
    let (mut a, out) = w.start(&d);
    assert_eq!(
        out,
        vec![Output::Opened {
            task: 1,
            assignee: p(MANAGER),
            node: "review".to_owned()
        }]
    );
    assert_eq!(a.status, Status::Running);
    let out = w.approve(&mut a, &d, MANAGER);
    assert_eq!(
        out,
        vec![
            Output::Closed {
                task: 1,
                status: TaskStatus::Approved
            },
            Output::Opened {
                task: 2,
                assignee: p(CONTROLLER),
                node: "review".to_owned()
            },
        ]
    );
    let out = w.approve(&mut a, &d, CONTROLLER);
    assert_eq!(finished(&out), Some(Status::Approved));
    assert_eq!(a.status, Status::Approved);
    assert_eq!(a.ended_at, Some(w.now));
    assert!(a.open_tasks().is_empty());
    let lines: Vec<&str> = a.history.iter().map(|e| e.what.as_str()).collect();
    assert_eq!(
        lines,
        [
            "requested approval (Purchase approval v1)",
            "approved",
            "approved"
        ]
    );
}

#[test]
fn a_sequential_rejection_ends_it_before_the_next_approver() {
    let w = World::new(500);
    let d = single(&[MANAGER, CONTROLLER], Mode::Sequential);
    let (mut a, _) = w.start(&d);
    let out = w.reject(&mut a, &d, MANAGER);
    assert_eq!(finished(&out), Some(Status::Rejected));
    assert_eq!(a.tasks.len(), 1, "the controller is never asked");
    assert_eq!(a.tasks[0].comment.as_deref(), Some("Over budget"));
}

#[test]
fn all_approvers_must_approve() {
    let w = World::new(500);
    let d = single(&[MANAGER, CONTROLLER, LEGAL], Mode::All);
    let (mut a, out) = w.start(&d);
    assert_eq!(out.len(), 3);
    assert_eq!(open_people(&a), [MANAGER, CONTROLLER, LEGAL]);
    w.approve(&mut a, &d, LEGAL);
    w.approve(&mut a, &d, MANAGER);
    assert_eq!(a.status, Status::Running);
    let out = w.approve(&mut a, &d, CONTROLLER);
    assert_eq!(finished(&out), Some(Status::Approved));
}

#[test]
fn in_all_mode_one_rejection_rejects_and_closes_the_rest() {
    let w = World::new(500);
    let d = single(&[MANAGER, CONTROLLER, LEGAL], Mode::All);
    let (mut a, _) = w.start(&d);
    w.approve(&mut a, &d, MANAGER);
    let out = w.reject(&mut a, &d, CONTROLLER);
    assert!(out.contains(&Output::Closed {
        task: 3,
        status: TaskStatus::Cancelled
    }));
    assert_eq!(finished(&out), Some(Status::Rejected));
    assert_eq!(a.tasks[2].status, TaskStatus::Cancelled);
}

#[test]
fn in_any_mode_the_first_decision_decides() {
    let w = World::new(500);
    let d = single(&[MANAGER, CONTROLLER], Mode::Any);
    let (mut a, _) = w.start(&d);
    let out = w.approve(&mut a, &d, CONTROLLER);
    assert_eq!(finished(&out), Some(Status::Approved));
    assert_eq!(a.tasks[0].status, TaskStatus::Cancelled);

    let (mut a, _) = w.start(&d);
    let out = w.reject(&mut a, &d, MANAGER);
    assert_eq!(finished(&out), Some(Status::Rejected));
}

#[test]
fn a_quorum_needs_enough_approvals_and_fails_when_they_are_out_of_reach() {
    let w = World::new(500);
    let d = single(&[FIN_A, FIN_B, FIN_C], Mode::Quorum { required: 2 });
    let (mut a, _) = w.start(&d);
    w.approve(&mut a, &d, FIN_A);
    w.reject(&mut a, &d, FIN_B);
    assert_eq!(a.status, Status::Running, "2 of 3 is still possible");
    let out = w.approve(&mut a, &d, FIN_C);
    assert_eq!(finished(&out), Some(Status::Approved));

    let (mut a, _) = w.start(&d);
    w.reject(&mut a, &d, FIN_A);
    let out = w.reject(&mut a, &d, FIN_C);
    assert_eq!(finished(&out), Some(Status::Rejected));
    assert_eq!(a.tasks[1].status, TaskStatus::Cancelled);
}

// ---- who approves ----

#[test]
fn the_requester_never_approves_their_own_request() {
    let w = World::new(500);
    let d = single(&[REQUESTER, MANAGER], Mode::All);
    let (a, _) = w.start(&d);
    assert_eq!(open_people(&a), [MANAGER]);
}

#[test]
fn a_step_with_nobody_left_is_stuck_until_cancelled() {
    let w = World::new(500);
    let d = single(&[REQUESTER], Mode::Sequential);
    let (mut a, out) = w.start(&d);
    assert_eq!(
        out,
        vec![Output::Stuck {
            node: "review".to_owned()
        }]
    );
    assert_eq!(a.status, Status::Stuck);
    assert!(!a.status.is_final());
    assert!(a.open_tasks().is_empty());
    assert_eq!(w.step(&mut a, &d, Event::Tick).unwrap(), vec![]);
    assert!(matches!(
        w.act(&mut a, &d, 1, MANAGER, Act::Submit),
        Err(LifecycleError::State(_))
    ));
    let out = w
        .step(&mut a, &d, Event::Cancel { by: p(CONTROLLER) })
        .unwrap();
    assert_eq!(finished(&out), Some(Status::Cancelled));
    assert_eq!(a.history.last().unwrap().what, "cancelled");
}

#[test]
fn a_quorum_larger_than_the_people_found_is_stuck() {
    let w = World::new(500);
    let d = def(vec![approval_step(
        "finance",
        vec![Approver::Capability {
            capability: "request.approve.cfo".to_owned(),
        }],
        Mode::Quorum { required: 2 },
    )]);
    d.check().unwrap();
    let (a, _) = w.start(&d);
    assert_eq!(a.status, Status::Stuck);
}

#[test]
fn approvers_come_from_people_fields_and_capabilities() {
    let w = World::new(500);
    let d = def(vec![approval_step(
        "review",
        vec![
            Approver::Field {
                field: f(MANAGER_FIELD),
            },
            Approver::Capability {
                capability: "request.approve.finance".to_owned(),
            },
            person(MANAGER),
        ],
        Mode::All,
    )]);
    let (a, _) = w.start(&d);
    assert_eq!(
        open_people(&a),
        [MANAGER, FIN_A, FIN_B, FIN_C],
        "no duplicates"
    );

    let mut empty = World::new(500);
    empty
        .record
        .insert(f(MANAGER_FIELD), Value::Text("nobody".to_owned()));
    empty.holders.clear();
    let only_field = def(vec![approval_step(
        "review",
        vec![Approver::Field {
            field: f(MANAGER_FIELD),
        }],
        Mode::Any,
    )]);
    assert_eq!(empty.start(&only_field).0.status, Status::Stuck);
    empty.record.remove(&f(MANAGER_FIELD));
    assert_eq!(empty.start(&only_field).0.status, Status::Stuck);
}

// ---- branches and parallel blocks ----

#[test]
fn the_amount_decides_the_route() {
    // Up to 10,000: the manager only.
    let w = World::new(8_000);
    let d = purchase();
    let (mut a, _) = w.start(&d);
    let out = w.approve(&mut a, &d, MANAGER);
    assert!(out.contains(&Output::Notified {
        node: "tell_requester".to_owned(),
        to: vec![p(REQUESTER)]
    }));
    assert_eq!(finished(&out), Some(Status::Approved));

    // Between: the manager, then two of three finance approvers.
    let w = World::new(15_000);
    let (mut a, _) = w.start(&d);
    w.approve(&mut a, &d, MANAGER);
    assert_eq!(open_people(&a), [FIN_A, FIN_B, FIN_C]);
    assert_eq!(a.open_tasks()[0].node, "finance");
    w.approve(&mut a, &d, FIN_B);
    let out = w.approve(&mut a, &d, FIN_C);
    assert_eq!(finished(&out), Some(Status::Approved));

    // Above 25,000: finance, then the CFO.
    let w = World::new(30_000);
    let (mut a, _) = w.start(&d);
    w.approve(&mut a, &d, MANAGER);
    w.approve(&mut a, &d, FIN_A);
    w.approve(&mut a, &d, FIN_B);
    assert_eq!(open_people(&a), [CFO]);
    assert_eq!(
        finished(&w.approve(&mut a, &d, CFO)),
        Some(Status::Approved)
    );
}

#[test]
fn parallel_branches_join_when_all_finish() {
    let w = World::new(500);
    let block = |join: Join| {
        def(vec![
            Node::Parallel {
                key: "checks".to_owned(),
                name: "Checks".to_owned(),
                branches: vec![
                    vec![approval_step("legal", people(&[LEGAL]), Mode::Any)],
                    vec![approval_step("finance", people(&[FIN_A]), Mode::Any)],
                    vec![
                        approval_step("controller", people(&[CONTROLLER]), Mode::Any),
                        approval_step("cfo", people(&[CFO]), Mode::Any),
                    ],
                ],
                join,
            },
            approval_step("final", people(&[MANAGER]), Mode::Any),
        ])
    };
    let d = block(Join::All);
    let (mut a, _) = w.start(&d);
    assert_eq!(open_people(&a), [LEGAL, FIN_A, CONTROLLER]);
    w.approve(&mut a, &d, CONTROLLER);
    assert_eq!(open_people(&a), [LEGAL, FIN_A, CFO]);
    w.approve(&mut a, &d, LEGAL);
    w.approve(&mut a, &d, CFO);
    assert_eq!(open_people(&a), [FIN_A]);
    w.approve(&mut a, &d, FIN_A);
    assert_eq!(open_people(&a), [MANAGER], "after the block");

    let d = block(Join::Any);
    let (mut a, _) = w.start(&d);
    let out = w.approve(&mut a, &d, FIN_A);
    assert_eq!(open_people(&a), [MANAGER]);
    assert_eq!(
        out.iter()
            .filter(|o| matches!(
                o,
                Output::Closed {
                    status: TaskStatus::Cancelled,
                    ..
                }
            ))
            .count(),
        2
    );

    let d = block(Join::AtLeast { count: 2 });
    let (mut a, _) = w.start(&d);
    w.approve(&mut a, &d, LEGAL);
    w.approve(&mut a, &d, CONTROLLER);
    assert_eq!(open_people(&a), [FIN_A, CFO]);
    w.approve(&mut a, &d, FIN_A);
    assert_eq!(open_people(&a), [MANAGER]);

    let d = block(Join::All);
    let (mut a, _) = w.start(&d);
    let out = w.reject(&mut a, &d, FIN_A);
    assert_eq!(
        finished(&out),
        Some(Status::Rejected),
        "a rejection anywhere rejects"
    );
    assert!(a.open_tasks().is_empty());
}

#[test]
fn a_stuck_branch_stops_the_parallel_block() {
    let w = World::new(500);
    let d = def(vec![Node::Parallel {
        key: "checks".to_owned(),
        name: "Checks".to_owned(),
        branches: vec![
            vec![approval_step("legal", people(&[LEGAL]), Mode::Any)],
            vec![approval_step("own", people(&[REQUESTER]), Mode::Any)],
        ],
        join: Join::Any,
    }]);
    let (a, out) = w.start(&d);
    assert_eq!(a.status, Status::Stuck);
    assert!(out.contains(&Output::Closed {
        task: 1,
        status: TaskStatus::Cancelled
    }));
    assert!(a.open_tasks().is_empty());
}

// ---- fill-in and notifications ----

#[test]
fn fill_in_steps_wait_for_the_fields() {
    let w = World::new(500);
    let d = def(vec![
        Node::FillIn {
            key: "attach_quote".to_owned(),
            name: "Attach the quote".to_owned(),
            assignee: person(REQUESTER),
            fields: vec![f(TITLE)],
        },
        approval_step("review", people(&[MANAGER]), Mode::Any),
    ]);
    let (mut a, _) = w.start(&d);
    let task = &a.open_tasks()[0];
    assert_eq!((task.kind, task.assignee), (TaskKind::FillIn, p(REQUESTER)));
    let id = task.id;
    let message = not_allowed(w.act(&mut a, &d, id, REQUESTER, Act::Approve { comment: None }));
    assert_eq!(message, "that does not fit this task");
    assert!(w
        .act(
            &mut a,
            &d,
            id,
            REQUESTER,
            Act::AddApprover { who: p(LEGAL) }
        )
        .is_err());
    // The requester may hand their own fill-in task on, and take part in comments.
    w.act(
        &mut a,
        &d,
        id,
        REQUESTER,
        Act::Comment {
            text: "On it".to_owned(),
        },
    )
    .unwrap();
    w.act(&mut a, &d, id, REQUESTER, Act::Delegate { to: p(DEPUTY) })
        .unwrap();
    let out = w.act(&mut a, &d, id, DEPUTY, Act::Submit).unwrap();
    assert!(out.contains(&Output::Closed {
        task: id,
        status: TaskStatus::Done
    }));
    assert_eq!(open_people(&a), [MANAGER]);
    let review = a.open_tasks()[0].id;
    assert_eq!(
        not_allowed(w.act(&mut a, &d, review, MANAGER, Act::Submit)),
        "that does not fit this task"
    );
}

#[test]
fn a_fill_in_step_with_nobody_is_stuck() {
    let w = World::new(500);
    let d = def(vec![Node::FillIn {
        key: "pay".to_owned(),
        name: "Payment date".to_owned(),
        assignee: Approver::Capability {
            capability: "nobody".to_owned(),
        },
        fields: vec![f(PAYMENT_DATE)],
    }]);
    assert_eq!(w.start(&d).0.status, Status::Stuck);
}

#[test]
fn notifications_do_not_wait() {
    let w = World::new(500);
    let d = def(vec![Node::Notify {
        key: "fyi".to_owned(),
        name: "For information".to_owned(),
        to: vec![
            person(REQUESTER),
            Approver::Capability {
                capability: "request.approve.cfo".to_owned(),
            },
        ],
    }]);
    let (a, out) = w.start(&d);
    assert_eq!(
        out,
        vec![
            Output::Notified {
                node: "fyi".to_owned(),
                to: vec![p(REQUESTER), p(CFO)]
            },
            Output::Finished {
                status: Status::Approved
            },
        ]
    );
    assert_eq!(a.status, Status::Approved);
}

// ---- acts ----

#[test]
fn rejections_and_returns_need_a_reason() {
    let w = World::new(500);
    let mut d = single(&[MANAGER], Mode::Any);
    let (mut a, _) = w.start(&d);
    for act in [
        Act::Reject {
            comment: " ".to_owned(),
        },
        Act::Comment {
            text: String::new(),
        },
    ] {
        assert_eq!(
            not_allowed(w.act(&mut a, &d, 1, MANAGER, act)),
            "give a reason"
        );
    }
    let back = || Act::Return {
        comment: "Add the quote".to_owned(),
    };
    assert!(not_allowed(w.act(&mut a, &d, 1, MANAGER, back())).contains("cannot send"));

    if let Node::Approval { allow_return, .. } = &mut d.steps[0] {
        *allow_return = true;
    }
    let (mut a, _) = w.start(&d);
    assert!(w
        .act(
            &mut a,
            &d,
            1,
            MANAGER,
            Act::Return {
                comment: String::new()
            }
        )
        .is_err());
    let out = w.act(&mut a, &d, 1, MANAGER, back()).unwrap();
    assert_eq!(finished(&out), Some(Status::Returned));
    assert_eq!(a.tasks[0].status, TaskStatus::Returned);
    assert_eq!(a.history.last().unwrap().what, "sent back to the requester");
}

#[test]
fn approval_comments_are_kept_and_blank_ones_dropped() {
    let w = World::new(500);
    let d = single(&[MANAGER, CONTROLLER], Mode::All);
    let (mut a, _) = w.start(&d);
    w.act(
        &mut a,
        &d,
        1,
        MANAGER,
        Act::Approve {
            comment: Some("Fine".to_owned()),
        },
    )
    .unwrap();
    w.act(
        &mut a,
        &d,
        2,
        CONTROLLER,
        Act::Approve {
            comment: Some("  ".to_owned()),
        },
    )
    .unwrap();
    assert_eq!(a.tasks[0].comment.as_deref(), Some("Fine"));
    assert_eq!(a.tasks[1].comment, None);
    assert_eq!(a.tasks[1].acted_at, Some(w.now));
}

#[test]
fn only_the_assignee_acts_on_an_open_task() {
    let w = World::new(500);
    let d = single(&[MANAGER, CONTROLLER], Mode::Sequential);
    let (mut a, _) = w.start(&d);
    let before = a.clone();
    assert_eq!(
        not_allowed(w.act(&mut a, &d, 1, CONTROLLER, Act::Approve { comment: None })),
        "task 1 is not yours"
    );
    assert!(matches!(
        w.act(&mut a, &d, 9, MANAGER, Act::Approve { comment: None }),
        Err(LifecycleError::State(_))
    ));
    assert_eq!(a, before, "refused acts change nothing");
    w.approve(&mut a, &d, MANAGER);
    assert!(matches!(
        w.act(&mut a, &d, 1, MANAGER, Act::Approve { comment: None }),
        Err(LifecycleError::State(m)) if m == "task 1 is closed"
    ));
    w.approve(&mut a, &d, CONTROLLER);
    assert!(matches!(
        w.step(&mut a, &d, Event::Tick),
        Err(LifecycleError::State(m)) if m == "the approval has ended"
    ));
}

#[test]
fn delegation_hands_the_task_to_someone_not_yet_asked() {
    let w = World::new(500);
    let d = single(&[MANAGER, CONTROLLER, LEGAL], Mode::Sequential);
    let (mut a, _) = w.start(&d);
    for to in [REQUESTER, MANAGER, CONTROLLER, LEGAL] {
        let message = not_allowed(w.act(&mut a, &d, 1, MANAGER, Act::Delegate { to: p(to) }));
        assert!(message.starts_with("delegate to someone else"), "{to}");
    }
    let out = w
        .act(&mut a, &d, 1, MANAGER, Act::Delegate { to: p(DEPUTY) })
        .unwrap();
    assert_eq!(
        out,
        vec![Output::Reassigned {
            task: 1,
            from: p(MANAGER),
            to: p(DEPUTY),
            escalated: false
        }]
    );
    assert_eq!(a.tasks[0].delegated_from, Some(p(MANAGER)));
    assert!(w
        .act(&mut a, &d, 1, MANAGER, Act::Approve { comment: None })
        .is_err());
    w.approve(&mut a, &d, DEPUTY);
    assert_eq!(open_people(&a), [CONTROLLER]);
    assert!(a.history.iter().any(|e| e.what.starts_with("delegated to")));
}

#[test]
fn added_approvers_join_the_running_step() {
    let w = World::new(500);
    // All at once: the added approver gets a task at once and must approve too.
    let d = single(&[MANAGER, CONTROLLER], Mode::All);
    let (mut a, _) = w.start(&d);
    let out = w
        .act(&mut a, &d, 1, MANAGER, Act::AddApprover { who: p(LEGAL) })
        .unwrap();
    assert_eq!(
        out,
        vec![Output::Opened {
            task: 3,
            assignee: p(LEGAL),
            node: "review".to_owned()
        }]
    );
    w.approve(&mut a, &d, MANAGER);
    w.approve(&mut a, &d, CONTROLLER);
    assert_eq!(a.status, Status::Running);
    w.approve(&mut a, &d, LEGAL);
    assert_eq!(a.status, Status::Approved);

    // One after another: the added approver comes right after the one who added them.
    let d = single(&[MANAGER, CONTROLLER], Mode::Sequential);
    let (mut a, _) = w.start(&d);
    w.act(&mut a, &d, 1, MANAGER, Act::AddApprover { who: p(LEGAL) })
        .unwrap();
    for who in [REQUESTER, LEGAL, CONTROLLER, MANAGER] {
        assert!(w
            .act(&mut a, &d, 1, MANAGER, Act::AddApprover { who: p(who) })
            .is_err());
    }
    w.approve(&mut a, &d, MANAGER);
    assert_eq!(open_people(&a), [LEGAL]);
    w.approve(&mut a, &d, LEGAL);
    assert_eq!(open_people(&a), [CONTROLLER]);
}

#[test]
fn an_approval_opens_a_bounded_number_of_tasks() {
    let w = World::new(500);
    let d = single(&[MANAGER], Mode::All);
    let (mut a, _) = w.start(&d);
    for n in 0..(MAX_TASKS - 1) {
        let who = 1_000 + u128::try_from(n).unwrap();
        w.act(&mut a, &d, 1, MANAGER, Act::AddApprover { who: p(who) })
            .unwrap();
    }
    assert_eq!(a.tasks.len(), MAX_TASKS);
    let message = not_allowed(w.act(&mut a, &d, 1, MANAGER, Act::AddApprover { who: p(5_000) }));
    assert!(message.contains("at most"));
}

#[test]
fn comments_leave_the_task_open() {
    let w = World::new(500);
    let d = single(&[MANAGER], Mode::Any);
    let (mut a, _) = w.start(&d);
    let out = w
        .act(
            &mut a,
            &d,
            1,
            MANAGER,
            Act::Comment {
                text: " Which supplier? ".to_owned(),
            },
        )
        .unwrap();
    assert!(out.is_empty());
    assert_eq!(a.history.last().unwrap().what, "commented: Which supplier?");
    assert_eq!(a.open_tasks().len(), 1);
}

// ---- time ----

fn timed(escalate_to: Option<Approver>) -> ApprovalDef {
    def(vec![Node::Approval {
        key: "review".to_owned(),
        name: "Review".to_owned(),
        approvers: people(&[MANAGER, CONTROLLER]),
        mode: Mode::All,
        allow_return: false,
        timeout: Some(hours(24)),
        escalate_to,
    }])
}

#[test]
fn overdue_tasks_are_escalated_once() {
    let mut w = World::new(500);
    let d = timed(Some(person(DEPUTY)));
    let (mut a, _) = w.start(&d);
    assert_eq!(a.tasks[0].due_at, Some(plus(w.now, 24 * HOUR)));
    w.now = plus(w.now, 23 * HOUR);
    assert_eq!(w.step(&mut a, &d, Event::Tick).unwrap(), vec![]);
    w.approve(&mut a, &d, CONTROLLER);
    w.now = plus(w.now, HOUR);
    let out = w.step(&mut a, &d, Event::Tick).unwrap();
    assert_eq!(
        out,
        vec![Output::Reassigned {
            task: 1,
            from: p(MANAGER),
            to: p(DEPUTY),
            escalated: true
        }]
    );
    assert!(a.tasks[0].escalated);
    assert_eq!(
        w.step(&mut a, &d, Event::Tick).unwrap(),
        vec![],
        "only once"
    );
    assert_eq!(
        finished(&w.approve(&mut a, &d, DEPUTY)),
        Some(Status::Approved)
    );
    assert!(a
        .history
        .iter()
        .any(|e| e.what.starts_with("overdue: escalated")));
}

#[test]
fn tasks_overdue_together_are_not_all_escalated_to_one_person() {
    // Found by the simulation: two overdue tasks of one step used to both go to the deputy.
    let mut w = World::new(500);
    let d = timed(Some(person(DEPUTY)));
    let (mut a, _) = w.start(&d);
    w.now = plus(w.now, 30 * HOUR);
    let out = w.step(&mut a, &d, Event::Tick).unwrap();
    assert_eq!(
        out,
        vec![
            Output::Reassigned {
                task: 1,
                from: p(MANAGER),
                to: p(DEPUTY),
                escalated: true
            },
            Output::Overdue { task: 2 },
        ]
    );
    assert_eq!(open_people(&a), [DEPUTY, CONTROLLER]);
}

#[test]
fn overdue_tasks_without_anyone_to_take_over_are_reported() {
    for target in [None, Some(person(CONTROLLER)), Some(person(REQUESTER))] {
        let mut w = World::new(500);
        let d = timed(target.clone());
        let (mut a, _) = w.start(&d);
        w.now = plus(w.now, 25 * HOUR);
        let out = w.step(&mut a, &d, Event::Tick).unwrap();
        assert_eq!(
            out,
            vec![Output::Overdue { task: 1 }, Output::Overdue { task: 2 }],
            "{target:?}"
        );
        assert_eq!(w.step(&mut a, &d, Event::Tick).unwrap(), vec![]);
    }
}

// ---- cancellation and versions ----

#[test]
fn cancelling_closes_every_open_task() {
    let w = World::new(500);
    let d = single(&[MANAGER, CONTROLLER], Mode::All);
    let (mut a, _) = w.start(&d);
    let out = w
        .step(&mut a, &d, Event::Cancel { by: p(REQUESTER) })
        .unwrap();
    assert_eq!(
        out,
        vec![
            Output::Closed {
                task: 1,
                status: TaskStatus::Cancelled
            },
            Output::Closed {
                task: 2,
                status: TaskStatus::Cancelled
            },
            Output::Finished {
                status: Status::Cancelled
            },
        ]
    );
    assert!(w
        .step(&mut a, &d, Event::Cancel { by: p(REQUESTER) })
        .is_err());
}

#[test]
fn a_running_approval_keeps_its_definition_version() {
    let w = World::new(500);
    let d = single(&[MANAGER], Mode::Any);
    let (mut a, _) = w.start(&d);
    let mut newer = d.clone();
    newer.version = 2;
    assert_eq!(
        w.step(&mut a, &newer, Event::Tick).unwrap_err(),
        LifecycleError::Version {
            running: 1,
            given: 2
        }
    );
    let mut other = d.clone();
    other.key = "other".to_owned();
    assert!(matches!(
        w.step(&mut a, &other, Event::Tick),
        Err(LifecycleError::State(_))
    ));
    // A definition edited without raising the version: the running step is gone.
    let mut edited = d.clone();
    edited.steps = vec![approval_step("renamed", people(&[MANAGER]), Mode::Any)];
    assert!(matches!(
        w.act(&mut a, &edited, 1, MANAGER, Act::Approve { comment: None }),
        Err(LifecycleError::State(m)) if m == "the step no longer exists"
    ));
    let mut a2 = a.clone();
    let out = w.step(&mut a2, &edited, Event::Tick).unwrap();
    assert!(out.is_empty(), "nothing escalates without a timeout");
}

#[test]
fn a_definition_edited_without_a_new_version_leaves_the_approval_stuck_not_broken() {
    let w = World::new(500);
    let d = single(&[MANAGER], Mode::Any);
    for edited_steps in [
        // The running step became a parallel block.
        vec![Node::Parallel {
            key: "review".to_owned(),
            name: "Review".to_owned(),
            branches: vec![
                vec![approval_step("a", people(&[LEGAL]), Mode::Any)],
                vec![approval_step("b", people(&[CFO]), Mode::Any)],
            ],
            join: Join::All,
        }],
        // The running step became a notification.
        vec![Node::Notify {
            key: "review".to_owned(),
            name: "Review".to_owned(),
            to: people(&[LEGAL]),
        }],
    ] {
        let (mut a, _) = w.start(&d);
        let mut edited = d.clone();
        edited.steps = edited_steps;
        let out = w.step(&mut a, &edited, Event::Tick).unwrap();
        assert!(out.is_empty());
        assert_eq!(a.status, Status::Stuck);
    }
    // A running parallel block that became a single step.
    let block = def(vec![Node::Parallel {
        key: "both".to_owned(),
        name: "Both".to_owned(),
        branches: vec![
            vec![approval_step("a", people(&[LEGAL]), Mode::Any)],
            vec![approval_step("b", people(&[CFO]), Mode::Any)],
        ],
        join: Join::All,
    }]);
    let (mut a, _) = w.start(&block);
    let mut edited = block.clone();
    edited.steps = vec![approval_step("both", people(&[LEGAL]), Mode::Any)];
    w.step(&mut a, &edited, Event::Tick).unwrap();
    assert_eq!(a.status, Status::Stuck);
}

#[test]
fn approvals_persist_and_resume() {
    let w = World::new(30_000);
    let d = purchase();
    let (mut a, _) = w.start(&d);
    w.approve(&mut a, &d, MANAGER);
    w.approve(&mut a, &d, FIN_A);
    let json = serde_json::to_string(&a).unwrap();
    let mut restored: Approval = serde_json::from_str(&json).unwrap();
    assert_eq!(restored, a);
    let left = w.approve(&mut a, &d, FIN_C);
    let right = w.approve(&mut restored, &d, FIN_C);
    assert_eq!(left, right);
    assert_eq!(restored, a);
    assert_eq!(open_people(&restored), [CFO]);
}

// ---- simulation ----

#[test]
fn simulation_shows_who_would_be_asked() {
    let d = purchase();
    let run = |amount: i64| {
        World::new(amount)
            .with(|env| simulate(&d, p(REQUESTER), env))
            .unwrap()
    };
    let step = |node: &str, people: &[u128]| SimulatedStep {
        node: node.to_owned(),
        people: people.iter().map(|n| p(*n)).collect(),
    };
    assert_eq!(
        run(800),
        (vec![step("manager", &[MANAGER])], Status::Approved)
    );
    assert_eq!(
        run(15_000),
        (
            vec![
                step("manager", &[MANAGER]),
                step("finance", &[FIN_A, FIN_B, FIN_C])
            ],
            Status::Approved
        )
    );
    assert_eq!(
        run(40_000),
        (
            vec![
                step("manager", &[MANAGER]),
                step("finance_large", &[FIN_A, FIN_B, FIN_C]),
                step("cfo", &[CFO]),
            ],
            Status::Approved
        )
    );
    let mut nobody = World::new(800);
    nobody.record.remove(&f(MANAGER_FIELD));
    let stuck = nobody.with(|env| simulate(&d, p(REQUESTER), env)).unwrap();
    assert_eq!(stuck, (vec![], Status::Stuck));
    let fill = def(vec![Node::FillIn {
        key: "pay".to_owned(),
        name: "Pay".to_owned(),
        assignee: person(FIN_A),
        fields: vec![f(PAYMENT_DATE)],
    }]);
    let done = nobody
        .with(|env| simulate(&fill, p(REQUESTER), env))
        .unwrap();
    assert_eq!(done, (vec![step("pay", &[FIN_A])], Status::Approved));
}

// ---- definitions ----

#[test]
fn the_demo_definition_is_valid_and_navigable() {
    let d = purchase();
    d.check().unwrap();
    d.check_in_table(&type_of).unwrap();
    let keys: Vec<&str> = d.nodes().iter().map(|n| n.key()).collect();
    assert_eq!(
        keys,
        [
            "manager",
            "by_amount",
            "finance_large",
            "cfo",
            "finance",
            "tell_requester"
        ]
    );
    assert_eq!(d.node("cfo").unwrap().name(), "cfo");
    assert_eq!(d.node("by_amount").unwrap().name(), "By amount");
    assert!(d.node("nothing").is_none());
    let json = serde_json::to_string(&d).unwrap();
    assert_eq!(serde_json::from_str::<ApprovalDef>(&json).unwrap(), d);
}

#[test]
fn status_finality() {
    for (status, done) in [
        (Status::Running, false),
        (Status::Stuck, false),
        (Status::Approved, true),
        (Status::Rejected, true),
        (Status::Returned, true),
        (Status::Cancelled, true),
    ] {
        assert_eq!(status.is_final(), done);
    }
}

#[test]
fn malformed_definitions_are_refused() {
    let base = || single(&[MANAGER], Mode::Any);
    let mut d = base();
    d.key = "Purchase".to_owned();
    assert!(invalid(&d).contains("approval key"));
    let mut d = base();
    d.name = String::new();
    assert!(invalid(&d).contains("needs a name"));
    let mut d = base();
    d.version = 0;
    assert!(invalid(&d).contains("versions start at 1"));
    let mut d = base();
    d.steps.clear();
    assert!(invalid(&d).contains("at least one step"));

    let check = |node: Node| invalid(&def(vec![node]));
    assert!(check(approval_step("Bad Key", people(&[MANAGER]), Mode::Any)).contains("step key"));
    let mut unnamed = approval_step("review", people(&[MANAGER]), Mode::Any);
    if let Node::Approval { name, .. } = &mut unnamed {
        name.clear();
    }
    assert!(check(unnamed).contains("needs a name"));
    assert!(check(approval_step("review", vec![], Mode::Any)).contains("no approver"));
    assert!(check(approval_step(
        "review",
        people(&[MANAGER]),
        Mode::Quorum { required: 0 }
    ))
    .contains("at least one approval"));
    assert!(check(approval_step(
        "review",
        people(&[MANAGER]),
        Mode::Quorum { required: 2 }
    ))
    .contains("more approvals"));
    let by_capability = approval_step(
        "review",
        vec![Approver::Capability {
            capability: "x".to_owned(),
        }],
        Mode::Quorum { required: 5 },
    );
    def(vec![by_capability]).check().unwrap();

    let branch = |cases: Vec<Case>| Node::Branch {
        key: "route".to_owned(),
        name: "Route".to_owned(),
        cases,
        otherwise: vec![],
    };
    assert!(check(branch(vec![])).contains("at least one case"));
    assert!(check(branch(vec![Case {
        name: " ".to_owned(),
        when: amount_le(1),
        then: vec![],
    }]))
    .contains("every case"));

    let parallel = |branches: Vec<Vec<Node>>, join: Join| Node::Parallel {
        key: "both".to_owned(),
        name: "Both".to_owned(),
        branches,
        join,
    };
    let one = || vec![approval_step("a", people(&[MANAGER]), Mode::Any)];
    let two = || vec![approval_step("b", people(&[LEGAL]), Mode::Any)];
    assert!(check(parallel(vec![one()], Join::All)).contains("two branches"));
    assert!(check(parallel(vec![one(), two()], Join::AtLeast { count: 0 })).contains("1 to 2"));
    assert!(check(parallel(vec![one(), two()], Join::AtLeast { count: 3 })).contains("1 to 2"));
    def(vec![parallel(
        vec![one(), two()],
        Join::AtLeast { count: 2 },
    )])
    .check()
    .unwrap();
    assert!(check(parallel(vec![one(), one()], Join::All)).contains("defined twice"));

    assert!(check(Node::FillIn {
        key: "fill".to_owned(),
        name: "Fill".to_owned(),
        assignee: person(MANAGER),
        fields: vec![],
    })
    .contains("no field"));
    assert!(check(Node::Notify {
        key: "tell".to_owned(),
        name: "Tell".to_owned(),
        to: vec![],
    })
    .contains("no recipient"));
}

#[test]
fn definitions_are_bounded() {
    // Nesting.
    let mut node = approval_step("leaf", people(&[MANAGER]), Mode::Any);
    for depth in 0..MAX_NESTING {
        node = Node::Branch {
            key: format!("level_{depth}"),
            name: "Level".to_owned(),
            cases: vec![Case {
                name: "Always".to_owned(),
                when: amount_gt(0),
                then: vec![node],
            }],
            otherwise: vec![],
        };
    }
    assert!(invalid(&def(vec![node])).contains("nested at most"));
    // Size.
    let steps = (0..=MAX_NODES)
        .map(|n| approval_step(&format!("s{n}"), people(&[MANAGER]), Mode::Any))
        .collect();
    assert!(invalid(&def(steps)).contains("at most"));
}

#[test]
fn field_references_are_checked_against_the_table() {
    let field_approver = |field: u128| {
        def(vec![approval_step(
            "review",
            vec![Approver::Field { field: f(field) }],
            Mode::Any,
        )])
    };
    field_approver(MANAGER_FIELD)
        .check_in_table(&type_of)
        .unwrap();
    let message = field_approver(AMOUNT)
        .check_in_table(&type_of)
        .unwrap_err()
        .to_string();
    assert!(message.contains("holds people"), "{message}");
    assert!(field_approver(999).check_in_table(&type_of).is_err());

    let mut escalating = timed(Some(Approver::Field { field: f(TITLE) }));
    assert!(escalating.check_in_table(&type_of).is_err());
    escalating = timed(Some(Approver::Field {
        field: f(MANAGER_FIELD),
    }));
    escalating.check_in_table(&type_of).unwrap();

    let branch = def(vec![Node::Branch {
        key: "route".to_owned(),
        name: "Route".to_owned(),
        cases: vec![Case {
            name: "Odd".to_owned(),
            when: test(TITLE, Operator::Gt, Value::Bool(true)),
            then: vec![],
        }],
        otherwise: vec![],
    }]);
    let message = branch.check_in_table(&type_of).unwrap_err().to_string();
    assert!(message.starts_with("case “Odd”"), "{message}");

    let fill = |assignee: Approver, field: u128| {
        def(vec![Node::FillIn {
            key: "fill".to_owned(),
            name: "Fill".to_owned(),
            assignee,
            fields: vec![f(field)],
        }])
    };
    assert!(fill(person(MANAGER), 999).check_in_table(&type_of).is_err());
    assert!(fill(Approver::Field { field: f(AMOUNT) }, TITLE)
        .check_in_table(&type_of)
        .is_err());
    fill(person(MANAGER), PAYMENT_DATE)
        .check_in_table(&type_of)
        .unwrap();

    let tell = def(vec![Node::Notify {
        key: "tell".to_owned(),
        name: "Tell".to_owned(),
        to: vec![Approver::Field { field: f(TITLE) }],
    }]);
    assert!(tell.check_in_table(&type_of).is_err());
    let parallel = def(vec![Node::Parallel {
        key: "both".to_owned(),
        name: "Both".to_owned(),
        branches: vec![
            vec![approval_step("a", people(&[MANAGER]), Mode::Any)],
            vec![approval_step("b", people(&[LEGAL]), Mode::Any)],
        ],
        join: Join::All,
    }]);
    parallel.check_in_table(&type_of).unwrap();
    let _: fn(FieldId) -> Option<DataType> = type_of;
}

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
//! Lifecycles: the closed-loop check, blockers, offers, transitions, editing rules, tallies,
//! and properties of the check.

mod common;

use common::*;
use mt_core::condition::{Operator, UtcContext};
use mt_core::id::{FieldId, LifecycleId, TableId};
use mt_core::state::StateCategory;
use mt_core::time::Date;
use mt_core::value::Value;
use mt_lifecycle::lifecycle::{
    Blocker, Lifecycle, Move, StateDef, Transition, MAX_STATES, MAX_TRANSITIONS,
};
use mt_lifecycle::LifecycleError;
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

fn state(key: &str, category: StateCategory) -> StateDef {
    StateDef {
        key: key.to_owned(),
        name: key.to_uppercase(),
        category,
        required: vec![],
        editable: None,
    }
}

fn go(key: &str, from: &[&str], to: &str) -> Transition {
    Transition {
        key: key.to_owned(),
        name: key.replace('_', " "),
        from: from.iter().map(|s| (*s).to_owned()).collect(),
        to: to.to_owned(),
        capability: None,
        guard: None,
        approval: None,
    }
}

/// The purchase request: draft → submitted → approved → ordered, with rejection, return and
/// cancellation.
fn purchase() -> Lifecycle {
    let mut submitted = state("submitted", StateCategory::PendingApproval);
    submitted.required = vec![f(AMOUNT), f(TITLE)];
    submitted.editable = Some(vec![]);
    let mut approved = state("approved", StateCategory::InProgress);
    approved.editable = Some(vec![f(PAYMENT_DATE)]);
    let mut submit = go("submit", &["draft"], "submitted");
    submit.guard = Some(amount_gt(0));
    submit.approval = Some("purchase_approval".to_owned());
    let mut approve = go("approve", &["submitted"], "approved");
    approve.capability = Some("request.approve".to_owned());
    let mut reject = go("reject", &["submitted"], "rejected");
    reject.capability = Some("request.approve".to_owned());
    let mut order = go("order", &["approved"], "ordered");
    order.capability = Some("order.place".to_owned());
    Lifecycle {
        id: LifecycleId::from_u128(1),
        table: TableId::from_u128(2),
        name: "Purchase request".to_owned(),
        states: vec![
            state("draft", StateCategory::New),
            submitted,
            approved,
            state("ordered", StateCategory::DoneSuccess),
            state("rejected", StateCategory::DoneFailure),
            state("cancelled", StateCategory::Cancelled),
        ],
        initial: "draft".to_owned(),
        transitions: vec![
            submit,
            approve,
            reject,
            go("send_back", &["submitted"], "draft"),
            order,
            go("cancel", &[], "cancelled"),
        ],
    }
}

fn ctx() -> UtcContext {
    UtcContext::on(Date::from_ymd(2026, 11, 18).unwrap())
}

fn record(amount: Option<i64>, title: bool) -> BTreeMap<FieldId, Value> {
    let mut r = BTreeMap::new();
    if let Some(a) = amount {
        r.insert(f(AMOUNT), Value::Number(dec(a)));
    }
    if title {
        r.insert(f(TITLE), Value::Text("Laptops".to_owned()));
    }
    r
}

fn error(lc: &Lifecycle) -> String {
    match lc.check() {
        Err(LifecycleError::Invalid(message)) => message,
        other => panic!("expected an invalid lifecycle, got {other:?}"),
    }
}

// ---- standard ----

#[test]
fn the_purchase_lifecycle_is_closed() {
    let lc = purchase();
    lc.check().unwrap();
    lc.check_in_table(&type_of).unwrap();
    assert_eq!(
        lc.state("approved").unwrap().category,
        StateCategory::InProgress
    );
    assert_eq!(lc.transition("order").unwrap().to, "ordered");
    assert!(lc.state("nowhere").is_none());
    assert!(lc.transition("nothing").is_none());
}

#[test]
fn a_complete_draft_can_be_submitted_into_an_approval() {
    let lc = purchase();
    let r = record(Some(1_200), true);
    let lookup = |field: FieldId| r.get(&field);
    let moved = lc
        .take("submit", "draft", &lookup, &ctx(), &|_| false)
        .unwrap();
    assert_eq!(
        moved,
        Move {
            from: "draft".to_owned(),
            to: "submitted".to_owned(),
            approval: Some("purchase_approval".to_owned()),
            category: StateCategory::PendingApproval,
        }
    );
}

#[test]
fn offers_list_every_way_out_with_its_blockers() {
    let lc = purchase();
    let r = record(Some(0), false);
    let lookup = |field: FieldId| r.get(&field);
    let offers = lc.offers("draft", &lookup, &ctx(), &|_| true);
    let keys: Vec<&str> = offers.iter().map(|o| o.key.as_str()).collect();
    assert_eq!(
        keys,
        ["submit", "cancel"],
        "cancel leaves from any open state"
    );
    assert_eq!(
        offers[0].blockers,
        vec![Blocker::Guard, Blocker::Required { field: f(TITLE) }]
    );
    assert!(offers[1].blockers.is_empty());
    assert_eq!(offers[1].to, "cancelled");
    assert!(
        lc.offers("ordered", &lookup, &ctx(), &|_| true).is_empty(),
        "final"
    );
    assert!(lc.offers("nowhere", &lookup, &ctx(), &|_| true).is_empty());
}

#[test]
fn capabilities_are_asked_of_the_caller() {
    let lc = purchase();
    let r = record(Some(5), true);
    let lookup = |field: FieldId| r.get(&field);
    let refused = lc
        .take("approve", "submitted", &lookup, &ctx(), &|cap| {
            cap == "order.place"
        })
        .unwrap_err();
    assert_eq!(
        refused,
        vec![Blocker::Capability {
            capability: "request.approve".to_owned()
        }]
    );
    let moved = lc
        .take("approve", "submitted", &lookup, &ctx(), &|cap| {
            cap == "request.approve"
        })
        .unwrap();
    assert_eq!(moved.approval, None);
    assert_eq!(moved.category, StateCategory::InProgress);
}

#[test]
fn every_blocker_is_reported_at_once() {
    let lc = purchase();
    let r = record(None, false);
    let lookup = |field: FieldId| r.get(&field);
    let blockers = lc
        .take("submit", "approved", &lookup, &ctx(), &|_| false)
        .unwrap_err();
    assert_eq!(
        blockers,
        vec![
            Blocker::WrongState {
                current: "approved".to_owned()
            },
            Blocker::Guard,
            Blocker::Required { field: f(AMOUNT) },
            Blocker::Required { field: f(TITLE) },
        ]
    );
    assert_eq!(
        lc.take("teleport", "draft", &lookup, &ctx(), &|_| true)
            .unwrap_err(),
        vec![Blocker::WrongState {
            current: "draft".to_owned()
        }]
    );
    let submit = lc.transition("submit").unwrap();
    assert_eq!(
        lc.blockers(submit, "limbo", &lookup, &ctx(), &|_| true),
        vec![Blocker::WrongState {
            current: "limbo".to_owned()
        }]
    );
}

#[test]
fn empty_values_do_not_satisfy_required_fields() {
    let lc = purchase();
    let mut r = record(Some(10), false);
    r.insert(f(TITLE), Value::Text(String::new()));
    let lookup = |field: FieldId| r.get(&field);
    assert_eq!(
        lc.take("submit", "draft", &lookup, &ctx(), &|_| true)
            .unwrap_err(),
        vec![Blocker::Required { field: f(TITLE) }]
    );
}

#[test]
fn states_limit_what_may_change() {
    let lc = purchase();
    assert!(
        lc.may_edit("draft", f(TITLE)),
        "all fields when unrestricted"
    );
    assert!(
        !lc.may_edit("submitted", f(TITLE)),
        "nothing while approving"
    );
    assert!(lc.may_edit("approved", f(PAYMENT_DATE)));
    assert!(!lc.may_edit("approved", f(AMOUNT)));
    assert!(
        !lc.may_edit("ordered", f(PAYMENT_DATE)),
        "final states are closed"
    );
    assert!(!lc.may_edit("nowhere", f(TITLE)));
}

#[test]
fn tallies_count_every_state() {
    let lc = purchase();
    let counts = lc.tally(["draft", "draft", "ordered", "ghost"]);
    assert_eq!(counts.len(), 6);
    assert_eq!(counts["draft"], 2);
    assert_eq!(counts["ordered"], 1);
    assert_eq!(counts["submitted"], 0);
    assert!(!counts.contains_key("ghost"));
}

#[test]
fn lifecycles_round_trip_through_json() {
    let lc = purchase();
    let json = serde_json::to_string(&lc).unwrap();
    assert!(
        !json.contains("\"editable\":null"),
        "absent parts are left out"
    );
    assert_eq!(serde_json::from_str::<Lifecycle>(&json).unwrap(), lc);
}

// ---- invalid ----

#[test]
fn malformed_lifecycles_are_refused() {
    let mut lc = purchase();
    lc.name = " ".to_owned();
    assert!(error(&lc).contains("name"));

    let mut lc = purchase();
    lc.states.clear();
    assert!(error(&lc).contains("states"));

    let mut lc = purchase();
    for n in 0..MAX_STATES {
        lc.states
            .push(state(&format!("extra_{n}"), StateCategory::InProgress));
    }
    assert!(error(&lc).contains("states"));

    let mut lc = purchase();
    for n in 0..=MAX_TRANSITIONS {
        lc.transitions
            .push(go(&format!("t_{n}"), &["draft"], "ordered"));
    }
    assert!(error(&lc).contains("transitions"));

    let mut lc = purchase();
    lc.states[0].key = "Draft".to_owned();
    assert!(error(&lc).contains("not a state key"));

    let mut lc = purchase();
    lc.states[1].key = "draft".to_owned();
    assert!(error(&lc).contains("twice"));

    let mut lc = purchase();
    lc.states[2].name = String::new();
    assert!(error(&lc).contains("needs a name"));

    let mut lc = purchase();
    lc.initial = "start".to_owned();
    assert!(error(&lc).contains("does not exist"));

    let mut lc = purchase();
    lc.initial = "ordered".to_owned();
    assert!(error(&lc).contains("final state"));

    let mut lc = purchase();
    for s in &mut lc.states {
        if s.category.is_terminal() {
            s.category = StateCategory::InProgress;
        }
    }
    assert!(error(&lc).contains("needs a final state"));
}

#[test]
fn malformed_transitions_are_refused() {
    let mut lc = purchase();
    lc.transitions[0].key = "Submit!".to_owned();
    assert!(error(&lc).contains("not a transition key"));

    let mut lc = purchase();
    lc.transitions[1].key = "submit".to_owned();
    assert!(error(&lc).contains("twice"));

    let mut lc = purchase();
    lc.transitions[2].name = "\t".to_owned();
    assert!(error(&lc).contains("button label"));

    let mut lc = purchase();
    lc.transitions[3].to = "archive".to_owned();
    assert!(error(&lc).contains("unknown state `archive`"));

    let mut lc = purchase();
    lc.transitions[3].from = vec!["limbo".to_owned()];
    assert!(error(&lc).contains("unknown state `limbo`"));

    let mut lc = purchase();
    lc.transitions[3].from = vec!["draft".to_owned()];
    lc.transitions[3].to = "draft".to_owned();
    assert!(error(&lc).contains("leads back"));

    let mut lc = purchase();
    lc.transitions[1].capability = Some("Not a key".to_owned());
    assert!(error(&lc).contains("capability"));
}

#[test]
fn unreachable_and_dead_end_states_are_refused() {
    let mut lc = purchase();
    lc.states
        .push(state("archived", StateCategory::DoneSuccess));
    assert!(error(&lc).contains("`archived` can never be reached"));

    // Without the generic cancel, a waiting state with no way out is a dead end.
    let mut lc = purchase();
    lc.transitions.retain(|t| t.key != "cancel");
    lc.states.retain(|s| s.key != "cancelled");
    lc.states
        .push(state("on_hold", StateCategory::WaitingExternal));
    lc.transitions.push(go("hold", &["approved"], "on_hold"));
    assert!(error(&lc).contains("`on_hold` could never finish"));
    lc.transitions.push(go("release", &["on_hold"], "approved"));
    lc.check().unwrap();
}

#[test]
fn field_references_are_checked_against_the_table() {
    let unknown = f(999);
    let mut lc = purchase();
    lc.states[1].required.push(unknown);
    assert!(lc
        .check_in_table(&type_of)
        .unwrap_err()
        .to_string()
        .contains("unknown field"));

    let mut lc = purchase();
    lc.states[2].editable = Some(vec![unknown]);
    assert!(lc.check_in_table(&type_of).is_err());

    let mut lc = purchase();
    lc.transitions[0].guard = Some(test(AMOUNT, Operator::Contains, Value::Bool(true)));
    let message = lc.check_in_table(&type_of).unwrap_err().to_string();
    assert!(message.starts_with("the guard of `submit`"), "{message}");
}

// ---- properties ----

const CATEGORIES: [StateCategory; 8] = [
    StateCategory::New,
    StateCategory::InProgress,
    StateCategory::WaitingInternal,
    StateCategory::WaitingExternal,
    StateCategory::PendingApproval,
    StateCategory::DoneSuccess,
    StateCategory::DoneFailure,
    StateCategory::Cancelled,
];

fn arbitrary() -> impl Strategy<Value = Lifecycle> {
    (2_usize..8).prop_flat_map(|n| {
        (
            proptest::collection::vec(0_usize..8, n),
            proptest::collection::vec((proptest::collection::vec(0..n, 0..3), 0..n), 0..(n * 3)),
            0..n,
        )
            .prop_map(move |(categories, edges, initial)| Lifecycle {
                id: LifecycleId::from_u128(1),
                table: TableId::from_u128(1),
                name: "Random".to_owned(),
                states: categories
                    .iter()
                    .enumerate()
                    .map(|(i, c)| state(&format!("s{i}"), CATEGORIES[*c]))
                    .collect(),
                initial: format!("s{initial}"),
                transitions: edges
                    .iter()
                    .enumerate()
                    .map(|(i, (from, to))| {
                        let from: Vec<String> = from.iter().map(|s| format!("s{s}")).collect();
                        let to = format!("s{to}");
                        Transition {
                            key: format!("t{i}"),
                            name: format!("T{i}"),
                            from,
                            to,
                            capability: None,
                            guard: None,
                            approval: None,
                        }
                    })
                    .collect(),
            })
    })
}

/// Closed lifecycles: a spine s0 → s1 → … → a final state, plus random extra transitions.
fn closed() -> impl Strategy<Value = Lifecycle> {
    (3_usize..8).prop_flat_map(|n| {
        (
            proptest::collection::vec(0_usize..8, n),
            proptest::collection::vec((0..n, 0..n), 0..(n * 2)),
            proptest::bool::ANY,
        )
            .prop_map(move |(categories, extra, cancel)| {
                let mut states: Vec<StateDef> = categories
                    .iter()
                    .enumerate()
                    .map(|(i, c)| state(&format!("s{i}"), CATEGORIES[*c]))
                    .collect();
                states[0].category = StateCategory::New;
                states[n - 1].category = StateCategory::DoneSuccess;
                let mut transitions: Vec<Transition> = (0..n - 1)
                    .map(|i| {
                        go(
                            &format!("next{i}"),
                            &[&format!("s{i}")],
                            &format!("s{}", i + 1),
                        )
                    })
                    .collect();
                for (i, (from, to)) in extra.into_iter().enumerate() {
                    if from != to {
                        transitions.push(go(
                            &format!("extra{i}"),
                            &[&format!("s{from}")],
                            &format!("s{to}"),
                        ));
                    }
                }
                if cancel {
                    states.push(state("gone", StateCategory::Cancelled));
                    transitions.push(go("cancel", &[], "gone"));
                }
                Lifecycle {
                    id: LifecycleId::from_u128(1),
                    table: TableId::from_u128(1),
                    name: "Closed".to_owned(),
                    states,
                    initial: "s0".to_owned(),
                    transitions,
                }
            })
    })
}

/// Successor states, computed independently of the crate.
fn successors(lc: &Lifecycle, key: &str) -> BTreeSet<String> {
    let category = lc.states.iter().find(|s| s.key == key).unwrap().category;
    lc.transitions
        .iter()
        .filter(|t| {
            if t.from.is_empty() {
                !category.is_terminal()
            } else {
                t.from.iter().any(|f| f == key)
            }
        })
        .map(|t| t.to.clone())
        .collect()
}

fn reachable(lc: &Lifecycle, from: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::from([from.to_owned()]);
    let mut todo = vec![from.to_owned()];
    while let Some(key) = todo.pop() {
        for next in successors(lc, &key) {
            if seen.insert(next.clone()) {
                todo.push(next);
            }
        }
    }
    seen
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// The check accepts exactly the closed lifecycles: every state reachable, every state able
    /// to finish (for well-formed keys, a terminal state and a non-terminal start).
    #[test]
    fn the_check_accepts_exactly_the_closed_lifecycles(lc in arbitrary()) {
        let self_loop = lc.transitions.iter().any(|t| t.from.contains(&t.to));
        let initial_terminal = lc.state(&lc.initial).unwrap().category.is_terminal();
        let has_terminal = lc.states.iter().any(|s| s.category.is_terminal());
        let all_reached = reachable(&lc, &lc.initial).len() == lc.states.len();
        let all_finish = lc.states.iter().all(|s| {
            reachable(&lc, &s.key)
                .iter()
                .any(|k| lc.state(k).unwrap().category.is_terminal())
        });
        let expected = !self_loop && !initial_terminal && has_terminal && all_reached && all_finish;
        prop_assert_eq!(lc.check().is_ok(), expected, "{:?}", lc.check());
    }

    /// In a closed lifecycle, following any transition never strands a record.
    #[test]
    fn walks_through_a_closed_lifecycle_can_always_finish(
        lc in closed(),
        choices in proptest::collection::vec(0_usize..10, 0..30),
    ) {
        prop_assert!(lc.check().is_ok(), "{:?}", lc.check());
        let none = |_: FieldId| None;
        let mut current = lc.initial.clone();
        for choice in choices {
            let offers = lc.offers(&current, &none, &ctx(), &|_| true);
            if offers.is_empty() {
                prop_assert!(lc.state(&current).unwrap().category.is_terminal());
                break;
            }
            let offer = &offers[choice % offers.len()];
            prop_assert!(offer.blockers.is_empty());
            current = lc.take(&offer.key, &current, &none, &ctx(), &|_| true).unwrap().to;
        }
        let finishes = reachable(&lc, &current)
            .iter()
            .any(|k| lc.state(k).unwrap().category.is_terminal());
        prop_assert!(finishes);
    }
}

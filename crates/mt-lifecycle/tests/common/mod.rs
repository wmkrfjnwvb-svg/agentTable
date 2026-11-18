//! Shared fixtures: people, fields, a record, a directory of capability holders, instants.
#![allow(dead_code)]

use mt_core::condition::{Condition, Operand, Operator, Test, UtcContext};
use mt_core::field::DataType;
use mt_core::id::{FieldId, PrincipalId};
use mt_core::time::{Date, DurationMs, Timestamp};
use mt_core::value::Value;
use mt_lifecycle::approval::{
    Act, Approval, ApprovalDef, Approver, Env, Event, Mode, Node, Output, People,
};
use mt_lifecycle::LifecycleError;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

pub const REQUESTER: u128 = 1;
pub const MANAGER: u128 = 2;
pub const CONTROLLER: u128 = 3;
pub const CFO: u128 = 4;
pub const LEGAL: u128 = 5;
pub const FIN_A: u128 = 6;
pub const FIN_B: u128 = 7;
pub const FIN_C: u128 = 8;
pub const DEPUTY: u128 = 9;
pub const OUTSIDER: u128 = 10;

pub const AMOUNT: u128 = 101;
pub const MANAGER_FIELD: u128 = 102;
pub const PAYMENT_DATE: u128 = 103;
pub const TITLE: u128 = 104;

pub const HOUR: i64 = 3_600_000;
pub const MINUTE: i64 = 60_000;

pub fn p(n: u128) -> PrincipalId {
    PrincipalId::from_u128(n)
}

pub fn f(n: u128) -> FieldId {
    FieldId::from_u128(n)
}

pub fn dec(n: i64) -> Decimal {
    Decimal::from(n)
}

pub fn hours(n: i64) -> DurationMs {
    DurationMs::from_millis(n * HOUR)
}

pub fn ms(n: i64) -> DurationMs {
    DurationMs::from_millis(n)
}

/// Midnight UTC of a day in November 2026 (the 16th is a Monday).
pub fn day(d: u32) -> Timestamp {
    Date::from_ymd(2026, 11, d).unwrap().to_timestamp()
}

/// An instant in November 2026, UTC.
pub fn at(d: u32, h: i64, m: i64) -> Timestamp {
    Timestamp::from_millis(day(d).millis() + h * HOUR + m * MINUTE).unwrap()
}

pub fn plus(t: Timestamp, millis: i64) -> Timestamp {
    Timestamp::from_millis(t.millis() + millis).unwrap()
}

/// The data types of the fixture table.
pub fn type_of(field: FieldId) -> Option<DataType> {
    match field.as_u128() {
        AMOUNT => Some(DataType::Number),
        MANAGER_FIELD => Some(DataType::Principal),
        PAYMENT_DATE => Some(DataType::Date),
        TITLE => Some(DataType::Text),
        _ => None,
    }
}

pub fn test(field: u128, op: Operator, value: Value) -> Condition {
    Condition::Test(Test {
        field: f(field),
        op,
        operand: Operand::Value(value),
    })
}

pub fn amount_le(n: i64) -> Condition {
    test(AMOUNT, Operator::Le, Value::Number(dec(n)))
}

pub fn amount_gt(n: i64) -> Condition {
    test(AMOUNT, Operator::Gt, Value::Number(dec(n)))
}

/// The record, the directory and the clock an approval runs against.
#[derive(Debug, Clone)]
pub struct World {
    pub record: BTreeMap<FieldId, Value>,
    pub holders: BTreeMap<String, Vec<PrincipalId>>,
    pub ctx: UtcContext,
    pub now: Timestamp,
}

impl People for World {
    fn holders(&self, capability: &str) -> Vec<PrincipalId> {
        self.holders.get(capability).cloned().unwrap_or_default()
    }
}

impl World {
    /// A purchase request of `amount` whose manager is MANAGER; finance is A, B and C.
    pub fn new(amount: i64) -> World {
        let mut record = BTreeMap::new();
        record.insert(f(AMOUNT), Value::Number(dec(amount)));
        record.insert(f(MANAGER_FIELD), Value::Principal(vec![p(MANAGER)]));
        record.insert(f(TITLE), Value::Text("Laptops".to_owned()));
        let mut holders = BTreeMap::new();
        holders.insert(
            "request.approve.finance".to_owned(),
            vec![p(FIN_A), p(FIN_B), p(FIN_C)],
        );
        holders.insert("request.approve.cfo".to_owned(), vec![p(CFO)]);
        World {
            record,
            holders,
            ctx: UtcContext::on(Date::from_ymd(2026, 11, 18).unwrap()),
            now: at(18, 10, 0),
        }
    }

    pub fn with<R>(&self, f: impl FnOnce(Env<'_>) -> R) -> R {
        let lookup = |field: FieldId| self.record.get(&field);
        f(Env {
            people: self,
            value_of: &lookup,
            context: &self.ctx,
            now: self.now,
        })
    }

    pub fn start(&self, def: &ApprovalDef) -> (Approval, Vec<Output>) {
        self.with(|env| Approval::start(def, p(REQUESTER), env))
            .unwrap()
    }

    pub fn step(
        &self,
        approval: &mut Approval,
        def: &ApprovalDef,
        event: Event,
    ) -> Result<Vec<Output>, LifecycleError> {
        self.with(|env| approval.step(def, event, env))
    }

    pub fn act(
        &self,
        approval: &mut Approval,
        def: &ApprovalDef,
        task: u32,
        actor: u128,
        act: Act,
    ) -> Result<Vec<Output>, LifecycleError> {
        self.step(
            approval,
            def,
            Event::Act {
                task,
                actor: p(actor),
                act,
            },
        )
    }

    /// The person's only open task, approved.
    pub fn approve(&self, approval: &mut Approval, def: &ApprovalDef, who: u128) -> Vec<Output> {
        let task = only_task(approval, who);
        self.act(approval, def, task, who, Act::Approve { comment: None })
            .unwrap()
    }

    /// The person's only open task, rejected.
    pub fn reject(&self, approval: &mut Approval, def: &ApprovalDef, who: u128) -> Vec<Output> {
        let task = only_task(approval, who);
        self.act(
            approval,
            def,
            task,
            who,
            Act::Reject {
                comment: "Over budget".to_owned(),
            },
        )
        .unwrap()
    }
}

pub fn only_task(approval: &Approval, who: u128) -> u32 {
    let tasks = approval.tasks_of(p(who));
    assert_eq!(tasks.len(), 1, "{who} has {} open tasks", tasks.len());
    tasks[0].id
}

pub fn open_people(approval: &Approval) -> Vec<u128> {
    approval
        .open_tasks()
        .iter()
        .map(|t| t.assignee.as_u128())
        .collect()
}

pub fn person(n: u128) -> Approver {
    Approver::Person { id: p(n) }
}

pub fn approval_step(key: &str, approvers: Vec<Approver>, mode: Mode) -> Node {
    Node::Approval {
        key: key.to_owned(),
        name: key.replace('_', " "),
        approvers,
        mode,
        allow_return: false,
        timeout: None,
        escalate_to: None,
    }
}

pub fn def(steps: Vec<Node>) -> ApprovalDef {
    ApprovalDef {
        key: "purchase_approval".to_owned(),
        name: "Purchase approval".to_owned(),
        version: 1,
        steps,
    }
}

/// The purchase approval of the product's demo: the manager approves; above 10,000 two of
/// three finance approvers as well; above 25,000 the CFO after them.
pub fn purchase() -> ApprovalDef {
    use mt_lifecycle::approval::Case;
    let manager = approval_step(
        "manager",
        vec![Approver::Field {
            field: f(MANAGER_FIELD),
        }],
        Mode::Sequential,
    );
    let finance = |key: &str| {
        approval_step(
            key,
            vec![Approver::Capability {
                capability: "request.approve.finance".to_owned(),
            }],
            Mode::Quorum { required: 2 },
        )
    };
    let cfo = approval_step(
        "cfo",
        vec![Approver::Capability {
            capability: "request.approve.cfo".to_owned(),
        }],
        Mode::Any,
    );
    def(vec![
        manager,
        Node::Branch {
            key: "by_amount".to_owned(),
            name: "By amount".to_owned(),
            cases: vec![
                Case {
                    name: "Up to 10,000".to_owned(),
                    when: amount_le(10_000),
                    then: vec![],
                },
                Case {
                    name: "Above 25,000".to_owned(),
                    when: amount_gt(25_000),
                    then: vec![finance("finance_large"), cfo],
                },
            ],
            otherwise: vec![finance("finance")],
        },
        Node::Notify {
            key: "tell_requester".to_owned(),
            name: "Tell the requester".to_owned(),
            to: vec![person(REQUESTER)],
        },
    ])
}

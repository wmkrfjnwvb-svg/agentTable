//! A small organisation for the tests.
//!
//! People: Alice (access manager), Bob and Carol (finance), Dave (sales), a guest, an AI
//! agent, an API app and a principal that was removed. Groups: Finance (Bob, Carol) inside
//! EMEA (which also holds Dave directly).
//!
//! Places: workspace W1 with base B1 (tables Orders and Payments) and workspace W2 with
//! base B2 (table Other).
//!
//! Orders have an amount, an owner, a requester, a region (from a tree World › EMEA ›
//! DE / FR, World › APAC › JP), a status, a salary-like confidential field and a note.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use mt_core::condition::{Condition, Operand, Operator, UtcContext};
use mt_core::field::{Classification, DataType};
use mt_core::id::{
    BaseId, CapabilitySetId, FieldId, GrantId, PrincipalId, RecordId, RefItemId, RefSetId,
    SodRuleId, TableId, WorkspaceId,
};
use mt_core::time::{Date, Timestamp};
use mt_core::value::Value;
use mt_perm::{
    Action, Actor, Capability, CapabilitySet, Decision, Directory, Env, Facts, FieldMeta, Grant,
    Granted, Policy, PrincipalKind, RecordFacts, Request, Resource, SodKind, SodRule,
};
use std::collections::{BTreeMap, BTreeSet};

pub fn p(n: u128) -> PrincipalId {
    PrincipalId::from_u128(n)
}

pub const ALICE: u128 = 1;
pub const BOB: u128 = 2;
pub const CAROL: u128 = 3;
pub const DAVE: u128 = 4;
pub const GUEST: u128 = 5;
pub const AGENT: u128 = 6;
pub const APP: u128 = 7;
pub const REMOVED: u128 = 8;
pub const FINANCE: u128 = 20;
pub const EMEA: u128 = 21;
pub const EMPTY_GROUP: u128 = 22;

/// The directory of the organisation.
#[derive(Debug, Clone, Default)]
pub struct Org {
    pub kinds: BTreeMap<PrincipalId, PrincipalKind>,
    /// Direct members of each group.
    pub members: BTreeMap<PrincipalId, BTreeSet<PrincipalId>>,
}

impl Org {
    pub fn new() -> Org {
        let mut org = Org::default();
        for (n, kind) in [
            (ALICE, PrincipalKind::Member),
            (BOB, PrincipalKind::Member),
            (CAROL, PrincipalKind::Member),
            (DAVE, PrincipalKind::Member),
            (GUEST, PrincipalKind::Guest),
            (AGENT, PrincipalKind::Agent),
            (APP, PrincipalKind::App),
            (FINANCE, PrincipalKind::Group),
            (EMEA, PrincipalKind::Group),
            (EMPTY_GROUP, PrincipalKind::Group),
        ] {
            org.kinds.insert(p(n), kind);
        }
        org.members.insert(p(FINANCE), [p(BOB), p(CAROL)].into());
        org.members.insert(p(EMEA), [p(FINANCE), p(DAVE)].into());
        org
    }
}

impl Directory for Org {
    fn kind(&self, principal: PrincipalId) -> Option<PrincipalKind> {
        self.kinds.get(&principal).copied()
    }

    fn groups_of(&self, principal: PrincipalId) -> BTreeSet<PrincipalId> {
        let mut found = BTreeSet::new();
        let mut frontier = vec![principal];
        while let Some(current) = frontier.pop() {
            for (group, members) in &self.members {
                if members.contains(&current) && found.insert(*group) {
                    frontier.push(*group);
                }
            }
        }
        found
    }

    fn members_of(&self, group: PrincipalId) -> BTreeSet<PrincipalId> {
        let mut found = BTreeSet::new();
        let mut frontier = vec![group];
        while let Some(current) = frontier.pop() {
            for member in self.members.get(&current).into_iter().flatten() {
                if found.insert(*member) {
                    frontier.push(*member);
                }
            }
        }
        found
    }
}

pub fn f(n: u128) -> FieldId {
    FieldId::from_u128(n)
}

pub const AMOUNT: u128 = 101;
pub const OWNER: u128 = 102;
pub const REQUESTER: u128 = 103;
pub const REGION: u128 = 104;
pub const STATUS: u128 = 105;
pub const SALARY: u128 = 106;
pub const NOTE: u128 = 107;
pub const SECRET: u128 = 108;
pub const TOTAL: u128 = 109;
pub const EMAIL: u128 = 110;

pub fn item(n: u128) -> RefItemId {
    RefItemId::from_u128(n)
}

pub const REGIONS: u128 = 50;
pub const WORLD: u128 = 1;
pub const R_EMEA: u128 = 2;
pub const DE: u128 = 3;
pub const FR: u128 = 4;
pub const APAC: u128 = 5;
pub const JP: u128 = 6;

/// Table facts: owner fields and the region tree.
pub struct Tables;

impl Facts for Tables {
    fn owner_fields(&self, table: TableId) -> Vec<FieldId> {
        if table == ORDERS_T {
            vec![f(OWNER), f(REQUESTER)]
        } else {
            vec![]
        }
    }

    fn is_within(&self, set: RefSetId, root: RefItemId, it: RefItemId) -> bool {
        if set != RefSetId::from_u128(REGIONS) {
            return false;
        }
        let parent = |n: u128| match n {
            R_EMEA | APAC => Some(WORLD),
            DE | FR => Some(R_EMEA),
            JP => Some(APAC),
            _ => None,
        };
        let mut current = Some(it.as_u128());
        while let Some(node) = current {
            if node == root.as_u128() {
                return true;
            }
            current = parent(node);
        }
        false
    }
}

pub const W1: WorkspaceId = WorkspaceId::from_u128(1);
pub const W2: WorkspaceId = WorkspaceId::from_u128(2);
pub const B1: BaseId = BaseId::from_u128(1);
pub const B2: BaseId = BaseId::from_u128(2);
pub const ORDERS_T: TableId = TableId::from_u128(1);
pub const PAYMENTS_T: TableId = TableId::from_u128(2);
pub const OTHER_T: TableId = TableId::from_u128(3);

pub const TENANT: Resource = Resource::Tenant;
pub const WS1: Resource = Resource::Workspace { workspace: W1 };
pub const BASE1: Resource = Resource::Base {
    workspace: W1,
    base: B1,
};
pub const ORDERS: Resource = Resource::Table {
    workspace: W1,
    base: B1,
    table: ORDERS_T,
};
pub const PAYMENTS: Resource = Resource::Table {
    workspace: W1,
    base: B1,
    table: PAYMENTS_T,
};
pub const OTHER: Resource = Resource::Table {
    workspace: W2,
    base: B2,
    table: OTHER_T,
};

/// The data types of the Orders table.
pub fn order_type(field: FieldId) -> Option<DataType> {
    Some(match field.as_u128() {
        AMOUNT | SALARY | TOTAL => DataType::Number,
        OWNER | REQUESTER => DataType::Principal,
        REGION => DataType::Reference,
        STATUS | NOTE | SECRET | EMAIL => DataType::Text,
        _ => return None,
    })
}

/// A record of the Orders table.
#[derive(Debug, Clone)]
pub struct Rec {
    pub id: RecordId,
    pub cells: BTreeMap<FieldId, Value>,
}

impl RecordFacts for Rec {
    fn id(&self) -> RecordId {
        self.id
    }

    fn value(&self, field: FieldId) -> Option<&Value> {
        self.cells.get(&field)
    }
}

pub fn num(n: i64) -> Value {
    Value::Number(n.into())
}

/// An order: its id, amount, owner, region.
pub fn order(id: u128, amount: i64, owner: Option<u128>, region: Option<u128>) -> Rec {
    let mut cells = BTreeMap::new();
    cells.insert(f(AMOUNT), num(amount));
    if let Some(owner) = owner {
        cells.insert(f(OWNER), Value::Principal(vec![p(owner)]));
    }
    if let Some(region) = region {
        cells.insert(f(REGION), Value::Reference(item(region)));
    }
    cells.insert(f(STATUS), Value::Text("open".into()));
    Rec {
        id: RecordId::from_u128(id),
        cells,
    }
}

pub fn t(ms: i64) -> Timestamp {
    Timestamp::from_millis(1_800_000_000_000 + ms).unwrap()
}

pub const NOW: i64 = 1_000_000;

pub fn today() -> UtcContext {
    UtcContext::on(Date::from_ymd(2027, 1, 15).unwrap())
}

/// The environment: the organisation, the tables and a context.
pub struct World {
    pub org: Org,
    pub tables: Tables,
    pub context: UtcContext,
}

impl World {
    pub fn new() -> World {
        World {
            org: Org::new(),
            tables: Tables,
            context: today(),
        }
    }

    pub fn env(&self) -> Env<'_> {
        Env {
            directory: &self.org,
            facts: &self.tables,
            context: &self.context,
        }
    }
}

pub fn read() -> Capability {
    Action::RecordRead.into()
}

pub fn cap(action: Action) -> Capability {
    action.into()
}

pub fn custom(key: &str) -> Capability {
    Capability::custom(key).unwrap()
}

pub fn gid(n: u128) -> GrantId {
    GrantId::from_u128(n)
}

pub fn grant(id: u128, to: u128, capability: Capability, on: Resource) -> Grant {
    Grant::new(
        gid(id),
        p(to),
        Granted::Capability(capability),
        on,
        p(ALICE),
        t(0),
        "needed for the job",
    )
}

pub fn set_grant(id: u128, to: u128, set: u128, on: Resource) -> Grant {
    Grant::new(
        gid(id),
        p(to),
        Granted::Set(CapabilitySetId::from_u128(set)),
        on,
        p(ALICE),
        t(0),
        "needed for the job",
    )
}

pub fn set(id: u128, name: &str, capabilities: &[Capability]) -> CapabilitySet {
    CapabilitySet {
        id: CapabilitySetId::from_u128(id),
        name: name.into(),
        description: None,
        capabilities: capabilities.iter().cloned().collect(),
    }
}

pub fn rule(id: u128, first: Capability, second: Capability, kind: SodKind) -> SodRule {
    SodRule {
        id: SodRuleId::from_u128(id),
        name: "Four eyes".into(),
        first,
        second,
        kind,
    }
}

/// `owner = me`.
pub fn mine() -> Condition {
    Condition::test(f(OWNER), Operator::Eq, Operand::Me)
}

/// `amount <= limit`.
pub fn up_to(limit: i64) -> Condition {
    Condition::compare(f(AMOUNT), Operator::Le, num(limit))
}

/// Asks the policy about a record (or the place, without one) at NOW.
pub fn ask(
    policy: &Policy,
    world: &World,
    actor: Actor,
    capability: &Capability,
    on: Resource,
    record: Option<&Rec>,
) -> Decision {
    policy.decide(
        &Request {
            actor,
            capability,
            on,
            record: record.map(|r| r as &dyn RecordFacts),
            prior: &[],
            now: t(NOW),
        },
        &world.env(),
    )
}

pub fn allowed(
    policy: &Policy,
    world: &World,
    who: u128,
    capability: &Capability,
    on: Resource,
    record: Option<&Rec>,
) -> bool {
    ask(policy, world, Actor::direct(p(who)), capability, on, record).allowed
}

/// The fields of the Orders table as the engine sees them.
pub fn order_fields() -> Vec<FieldMeta> {
    let meta = |id: u128, classification: Classification| FieldMeta {
        id: f(id),
        classification,
        masked: false,
        generated: false,
        view_capability: None,
        edit_capability: None,
    };
    vec![
        meta(AMOUNT, Classification::Internal),
        meta(OWNER, Classification::Internal),
        meta(STATUS, Classification::Public),
        FieldMeta {
            masked: true,
            ..meta(NOTE, Classification::Internal)
        },
        meta(SALARY, Classification::Confidential),
        meta(EMAIL, Classification::Personal),
        meta(SECRET, Classification::SensitivePersonal),
        FieldMeta {
            generated: true,
            ..meta(TOTAL, Classification::Internal)
        },
        FieldMeta {
            view_capability: Some(Capability::custom("salary.view").unwrap()),
            edit_capability: Some(Capability::custom("salary.edit").unwrap()),
            ..meta(REQUESTER, Classification::Internal)
        },
    ]
}

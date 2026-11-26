//! The demo workspace: a small purchasing process.
//!
//! * **Suppliers** — names, countries (ISO reference data), contacts (personal data), bank
//!   accounts (confidential, masked, behind a field-group capability), and the open spend
//!   rolled up from their requests.
//! * **Purchase requests** — numbered automatically, owned by their requester, linked to a
//!   supplier, a cost centre (a reference tree) and line items; subtotal, tax and total are
//!   formulas, and "needs CFO" compares the total with a parameter.
//! * **Request lines** — quantity × unit price.
//!
//! The people are Alice (administrator), Bob (buyer), Carol (controller, approves up to
//! 10 000), Dave (finance, approves up to 100 000 and places orders), Erin (CFO), Gina (a
//! guest at a supplier) and an AI purchasing assistant.
//!
//! Requests follow a lifecycle (draft → submitted → approved → ordered, or rejected or
//! cancelled). Submitting starts the purchase approval: whoever may approve the amount (by
//! the access rules, never the requester), then the CFO above the CFO limit.
//!
//! Records are created, submitted and approved through the real operations, as the people
//! who do it, so the audit log tells the story from the first second.

use crate::error::{ApiError, ApiResult};
use crate::model::{Base, CellInput, Data, LinkPair, Person, Table, Workspace, BASE, WORKSPACE};
use mt_core::condition::{Condition, Operator};
use mt_core::decimal::Decimal;
use mt_core::field::{
    Classification, ComputedKind, DataType, FieldDef, FieldGroup, FieldRole, InlineOption, Mask,
    OptionsSource, SystemField, Widget,
};
use mt_core::format::FormatId;
use mt_core::id::{FieldGroupId, FieldId, PrincipalId, RecordId, RefItemId, RefSetId, TableId};
use mt_core::money::{Currency, Money};
use mt_core::numbering::NumberingPattern;
use mt_core::time::Timestamp;
use mt_core::value::Value;
use mt_perm::{
    Action, Actor, Capability, CapabilitySet, FieldAccess, Grant, Granted, Policy, PrincipalKind,
    Resource, Scope, SodKind, SodRule,
};
use mt_refdata::{RefItem, RefSet, SetScope, State, Structure};
use std::collections::BTreeMap;

/// Alice: administers the workspace.
pub const ALICE: PrincipalId = PrincipalId::from_u128(1);
/// Bob: buys things; sees and changes his own requests.
pub const BOB: PrincipalId = PrincipalId::from_u128(2);
/// Carol: controller; reads every request and approves up to 10 000.
pub const CAROL: PrincipalId = PrincipalId::from_u128(3);
/// Dave: finance; sees bank details and the audit log.
pub const DAVE: PrincipalId = PrincipalId::from_u128(4);
/// Gina: a guest at a supplier; sees her own company only.
pub const GINA: PrincipalId = PrincipalId::from_u128(5);
/// The AI purchasing assistant.
pub const ASSISTANT: PrincipalId = PrincipalId::from_u128(6);
/// Erin: chief financial officer; approves requests above the CFO limit.
pub const ERIN: PrincipalId = PrincipalId::from_u128(7);
/// The purchasing team (Bob, Carol).
pub const TEAM: PrincipalId = PrincipalId::from_u128(20);
/// Finance (Dave).
pub const FINANCE: PrincipalId = PrincipalId::from_u128(21);

/// Suppliers.
pub const SUPPLIERS: TableId = TableId::from_u128(1);
/// Purchase requests.
pub const REQUESTS: TableId = TableId::from_u128(2);
/// Request lines.
pub const LINES: TableId = TableId::from_u128(3);

const fn f(n: u128) -> FieldId {
    FieldId::from_u128(n)
}

/// Supplier fields.
pub mod supplier {
    use super::{f, FieldId};
    /// Name.
    pub const NAME: FieldId = f(101);
    /// Country.
    pub const COUNTRY: FieldId = f(102);
    /// Contact email.
    pub const EMAIL: FieldId = f(103);
    /// Phone.
    pub const PHONE: FieldId = f(104);
    /// IBAN.
    pub const IBAN: FieldId = f(105);
    /// Rating.
    pub const RATING: FieldId = f(106);
    /// Preferred.
    pub const PREFERRED: FieldId = f(107);
    /// Requests (link).
    pub const REQUESTS: FieldId = f(108);
    /// Open spend (roll-up).
    pub const SPEND: FieldId = f(109);
}

/// Purchase request fields.
pub mod request {
    use super::{f, FieldId};
    /// Number.
    pub const NUMBER: FieldId = f(201);
    /// Title.
    pub const TITLE: FieldId = f(202);
    /// Requester.
    pub const REQUESTER: FieldId = f(203);
    /// Supplier (link).
    pub const SUPPLIER: FieldId = f(204);
    /// Cost centre.
    pub const COST_CENTRE: FieldId = f(205);
    /// Status.
    pub const STATUS: FieldId = f(206);
    /// Needed by.
    pub const NEEDED_BY: FieldId = f(207);
    /// Lines (link).
    pub const LINES: FieldId = f(208);
    /// Subtotal (roll-up).
    pub const SUBTOTAL: FieldId = f(209);
    /// Tax (formula).
    pub const TAX: FieldId = f(210);
    /// Total (formula).
    pub const TOTAL: FieldId = f(211);
    /// Days left (formula).
    pub const DAYS_LEFT: FieldId = f(212);
    /// Needs CFO (formula).
    pub const NEEDS_CFO: FieldId = f(213);
    /// Created (system).
    pub const CREATED: FieldId = f(214);
    /// Last changed by (system).
    pub const CHANGED_BY: FieldId = f(215);
}

/// Request line fields.
pub mod line {
    use super::{f, FieldId};
    /// Item.
    pub const ITEM: FieldId = f(301);
    /// Request (link).
    pub const REQUEST: FieldId = f(302);
    /// Category.
    pub const CATEGORY: FieldId = f(303);
    /// Quantity.
    pub const QUANTITY: FieldId = f(304);
    /// Unit price.
    pub const PRICE: FieldId = f(305);
    /// Amount (formula).
    pub const AMOUNT: FieldId = f(306);
}

/// Cost centres.
pub const COST_CENTRES: RefSetId = RefSetId::from_u128(1);
/// The status options.
pub mod status {
    use mt_core::id::RefItemId;
    /// Draft.
    pub const DRAFT: RefItemId = RefItemId::from_u128(9001);
    /// Submitted.
    pub const SUBMITTED: RefItemId = RefItemId::from_u128(9002);
    /// Approved.
    pub const APPROVED: RefItemId = RefItemId::from_u128(9003);
    /// Rejected.
    pub const REJECTED: RefItemId = RefItemId::from_u128(9004);
    /// Ordered.
    pub const ORDERED: RefItemId = RefItemId::from_u128(9005);
    /// Cancelled.
    pub const CANCELLED: RefItemId = RefItemId::from_u128(9006);
}

fn cc(n: u128) -> RefItemId {
    RefItemId::from_u128(100_u128.saturating_add(n))
}

fn usd() -> ApiResult<Currency> {
    Currency::new("USD").map_err(|e| ApiError::internal(e.to_string()))
}

fn money(amount: i64) -> ApiResult<Value> {
    Money::new(Decimal::from(amount), usd()?)
        .map(Value::Money)
        .map_err(|e| ApiError::internal(e.to_string()))
}

fn option(id: u128, label: &str, color: &str) -> InlineOption {
    InlineOption {
        id: RefItemId::from_u128(id),
        label: label.to_owned(),
        color: Some(color.to_owned()),
        enabled: true,
    }
}

fn field(id: FieldId, name: &str, data_type: DataType) -> FieldDef {
    FieldDef::new(id, name, data_type)
}

fn people() -> Vec<Person> {
    let person = |id, name: &str, kind, title: &str| Person {
        id,
        name: name.to_owned(),
        kind,
        title: title.to_owned(),
        members: Vec::new(),
    };
    vec![
        person(
            ALICE,
            "Alice Admin",
            PrincipalKind::Member,
            "Workspace administrator",
        ),
        person(
            BOB,
            "Bob Buyer",
            PrincipalKind::Member,
            "Buyer — sees his own requests",
        ),
        person(
            CAROL,
            "Carol Controller",
            PrincipalKind::Member,
            "Controller — approves up to 10,000",
        ),
        person(
            DAVE,
            "Dave Finance",
            PrincipalKind::Member,
            "Finance — approves up to 100,000, places orders",
        ),
        person(
            ERIN,
            "Erin CFO",
            PrincipalKind::Member,
            "Chief financial officer — approves above 25,000",
        ),
        person(
            GINA,
            "Gina Guest",
            PrincipalKind::Guest,
            "Guest at Northwind Traders",
        ),
        person(
            ASSISTANT,
            "Purchasing assistant",
            PrincipalKind::Agent,
            "AI agent — acts for a person",
        ),
        Person {
            members: vec![BOB, CAROL],
            ..person(TEAM, "Purchasing team", PrincipalKind::Group, "Group")
        },
        Person {
            members: vec![DAVE],
            ..person(FINANCE, "Finance", PrincipalKind::Group, "Group")
        },
    ]
}

fn cost_centres() -> State {
    let mut set = RefSet::new(
        COST_CENTRES,
        "cost_centre",
        "Cost centres",
        Structure::Tree { max_depth: 4 },
        SetScope::Tenant,
    );
    set.description = Some("Who pays: the finance dimension every request carries".to_owned());
    let item = |n: u128, code: &str, name: &str, parent: Option<u128>, sort: i64| {
        let mut item = RefItem::new(cc(n), code, name);
        item.parent = parent.map(cc);
        item.sort = sort;
        item
    };
    State::new(
        set,
        vec![
            item(0, "CC-100", "Company", None, 0),
            item(10, "CC-110", "Operations", Some(0), 1),
            item(11, "CC-111", "Facilities", Some(10), 1),
            item(12, "CC-112", "IT", Some(10), 2),
            item(20, "CC-120", "Sales", Some(0), 2),
            item(21, "CC-121", "Sales EMEA", Some(20), 1),
            item(22, "CC-122", "Sales APAC", Some(20), 2),
            item(30, "CC-130", "Finance", Some(0), 3),
        ],
    )
}

fn suppliers_table() -> Table {
    let mut name = field(supplier::NAME, "Name", DataType::Text);
    name.validation.required = true;
    let mut country = field(supplier::COUNTRY, "Country", DataType::Reference);
    country.options = OptionsSource::of_set(mt_refdata::builtin::system_set_id("iso_3166_1"));
    country.widget = Widget::SearchPicker;
    let mut email = field(supplier::EMAIL, "Contact email", DataType::Text);
    email.validation.format = Some(FormatId::Email);
    email.classification = Classification::Personal;
    let mut phone = field(supplier::PHONE, "Phone", DataType::Text);
    phone.validation.format = Some(FormatId::PhoneE164);
    phone.classification = Classification::Personal;
    let mut iban = field(supplier::IBAN, "IBAN", DataType::Text);
    iban.validation.format = Some(FormatId::Iban);
    iban.classification = Classification::Confidential;
    iban.format.mask = Some(Mask { visible_tail: 4 });
    let mut rating = field(supplier::RATING, "Rating", DataType::Number);
    rating.widget = Widget::Rating;
    rating.validation.min = Some(Decimal::ZERO);
    rating.validation.max = Some(Decimal::from(5));
    rating.validation.max_decimals = Some(0);
    let preferred = field(supplier::PREFERRED, "Preferred", DataType::Bool);
    let mut requests = field(supplier::REQUESTS, "Requests", DataType::Link);
    requests.options = OptionsSource::Table { table: REQUESTS };
    Table {
        id: SUPPLIERS,
        base: BASE,
        name: "Suppliers".to_owned(),
        description: "Companies we buy from".to_owned(),
        primary: supplier::NAME,
        fields: vec![
            name, country, email, phone, iban, rating, preferred, requests,
        ],
        groups: vec![FieldGroup {
            id: FieldGroupId::from_u128(1),
            name: "Bank details".to_owned(),
            fields: vec![supplier::IBAN],
            description: Some("Visible to finance only".to_owned()),
            visible_when: None,
            view_capability: Some("supplier.bank.view".to_owned()),
            edit_capability: Some("supplier.bank.edit".to_owned()),
        }],
        records: Vec::new(),
    }
}

fn requests_table() -> ApiResult<Table> {
    let mut number = field(request::NUMBER, "No.", DataType::Text);
    number.format.numbering = Some(
        NumberingPattern::new("PR-{YYYY}-{seq:4}")
            .map_err(|e| ApiError::internal(e.to_string()))?,
    );
    let mut title = field(request::TITLE, "Title", DataType::Text);
    title.validation.required = true;
    let mut requester = field(request::REQUESTER, "Requester", DataType::Principal);
    requester.role = Some(FieldRole::PartyRequester);
    let mut supplier_link = field(request::SUPPLIER, "Supplier", DataType::Link);
    supplier_link.options = OptionsSource::Table { table: SUPPLIERS };
    supplier_link.validation.max_items = Some(1);
    let mut cost_centre = field(request::COST_CENTRE, "Cost centre", DataType::Reference);
    cost_centre.options = OptionsSource::of_set(COST_CENTRES);
    cost_centre.role = Some(FieldRole::Dimension);
    let mut status_field = field(request::STATUS, "Status", DataType::Reference);
    status_field.role = Some(FieldRole::State);
    status_field.widget = Widget::Segmented;
    status_field.options = OptionsSource::Inline {
        options: vec![
            option(9001, "Draft", "grey"),
            option(9002, "Submitted", "blue"),
            option(9003, "Approved", "green"),
            option(9004, "Rejected", "red"),
            option(9005, "Ordered", "purple"),
            option(9006, "Cancelled", "grey"),
        ],
    };
    let mut needed_by = field(request::NEEDED_BY, "Needed by", DataType::Date);
    needed_by.role = Some(FieldRole::TimeDue);
    let mut lines = field(request::LINES, "Lines", DataType::Link);
    lines.options = OptionsSource::Table { table: LINES };
    lines.widget = Widget::LineItems;
    let mut created = field(request::CREATED, "Created", DataType::DateTime);
    created.system = Some(SystemField::CreatedAt);
    let mut changed_by = field(request::CHANGED_BY, "Last changed by", DataType::Principal);
    changed_by.system = Some(SystemField::ModifiedBy);
    Ok(Table {
        id: REQUESTS,
        base: BASE,
        name: "Purchase requests".to_owned(),
        description: "What we want to buy, from whom, and why".to_owned(),
        primary: request::TITLE,
        fields: vec![
            number,
            title,
            requester,
            supplier_link,
            cost_centre,
            status_field,
            needed_by,
            lines,
            created,
            changed_by,
        ],
        groups: Vec::new(),
        records: Vec::new(),
    })
}

fn lines_table() -> ApiResult<Table> {
    let mut item = field(line::ITEM, "Item", DataType::Text);
    item.validation.required = true;
    let mut request_link = field(line::REQUEST, "Request", DataType::Link);
    request_link.options = OptionsSource::Table { table: REQUESTS };
    request_link.validation.max_items = Some(1);
    let mut category = field(line::CATEGORY, "Category", DataType::Reference);
    category.options = OptionsSource::Inline {
        options: vec![
            option(9101, "Hardware", "blue"),
            option(9102, "Software", "purple"),
            option(9103, "Services", "green"),
            option(9104, "Office supplies", "orange"),
        ],
    };
    let mut quantity = field(line::QUANTITY, "Quantity", DataType::Number);
    quantity.validation.required = true;
    quantity.validation.min = Some(Decimal::ONE);
    let mut price = field(line::PRICE, "Unit price", DataType::Money);
    price.validation.currencies = vec![usd()?];
    Ok(Table {
        id: LINES,
        base: BASE,
        name: "Request lines".to_owned(),
        description: "What each request contains".to_owned(),
        primary: line::ITEM,
        fields: vec![item, request_link, category, quantity, price],
        groups: Vec::new(),
        records: Vec::new(),
    })
}

fn grant(
    id: u128,
    to: PrincipalId,
    what: Granted,
    on: Resource,
    reason: &str,
    now: Timestamp,
) -> Grant {
    Grant::new(
        mt_core::id::GrantId::from_u128(id),
        to,
        what,
        on,
        ALICE,
        now,
        reason,
    )
}

fn cap(action: Action) -> Granted {
    Granted::Capability(action.into())
}

fn custom(key: &str) -> ApiResult<Granted> {
    Capability::custom(key)
        .map(Granted::Capability)
        .map_err(|e| ApiError::internal(e.to_string()))
}

fn table(id: TableId) -> Resource {
    Resource::Table {
        workspace: WORKSPACE,
        base: BASE,
        table: id,
    }
}

fn policy(ws: &mut Workspace, now: Timestamp) -> ApiResult<()> {
    let perm = |e: mt_perm::PermError| ApiError::internal(e.to_string());
    let set = |id: u128, name: &str, capabilities: Vec<Capability>| CapabilitySet {
        id: mt_core::id::CapabilitySetId::from_u128(id),
        name: name.to_owned(),
        description: None,
        capabilities: capabilities.into_iter().collect(),
    };
    let base = Resource::Base {
        workspace: WORKSPACE,
        base: BASE,
    };
    let keys = |keys: &[&str]| -> ApiResult<Vec<Capability>> {
        keys.iter()
            .map(|key| key.parse::<Capability>().map_err(perm))
            .collect()
    };
    let policy: &mut Policy = &mut ws.data.policy;
    policy
        .add_set(set(
            4,
            "Purchasing administration",
            keys(&["supplier.bank.view", "supplier.bank.edit", "order.place"])?,
        ))
        .map_err(perm)?;
    policy
        .add_set(set(
            2,
            "Requester",
            keys(&["record.read", "record.create", "record.update"])?,
        ))
        .map_err(perm)?;
    policy
        .add_set(set(
            3,
            "Line editor",
            keys(&[
                "record.read",
                "record.create",
                "record.update",
                "record.delete",
            ])?,
        ))
        .map_err(perm)?;
    policy
        .add_rule(SodRule {
            id: mt_core::id::SodRuleId::from_u128(1),
            name: "Nobody approves their own request".to_owned(),
            first: Action::RecordCreate.into(),
            second: Capability::custom("request.approve").map_err(perm)?,
            kind: SodKind::SameRecord,
        })
        .map_err(perm)?;
    policy
        .add_rule(SodRule {
            id: mt_core::id::SodRuleId::from_u128(2),
            name: "Who edits bank details does not approve".to_owned(),
            first: Capability::custom("supplier.bank.edit").map_err(perm)?,
            second: Capability::custom("request.approve").map_err(perm)?,
            kind: SodKind::Hold,
        })
        .map_err(perm)?;

    let set_grant = |id: u128| Granted::Set(mt_core::id::CapabilitySetId::from_u128(id));
    let limit = |amount: i64| -> ApiResult<Condition> {
        Ok(Condition::compare(
            request::TOTAL,
            Operator::Le,
            money(amount)?,
        ))
    };
    let grants = vec![
        grant(21, ALICE, set_grant(4), base, "Administers purchasing", now),
        grant(
            2,
            TEAM,
            cap(Action::RecordRead),
            table(SUPPLIERS),
            "The team chooses suppliers",
            now,
        ),
        {
            let mut g = grant(
                3,
                TEAM,
                set_grant(2),
                table(REQUESTS),
                "Everyone in the team raises requests",
                now,
            );
            g.scope = Scope::Own;
            g
        },
        grant(
            4,
            TEAM,
            set_grant(3),
            table(LINES),
            "Requests need lines",
            now,
        ),
        grant(
            5,
            CAROL,
            cap(Action::RecordRead),
            table(REQUESTS),
            "The controller reviews all requests",
            now,
        ),
        {
            let mut g = grant(
                6,
                CAROL,
                custom("request.approve")?,
                table(REQUESTS),
                "Approves up to 10,000",
                now,
            );
            g.condition = Some(limit(10_000)?);
            g
        },
        grant(
            7,
            FINANCE,
            cap(Action::RecordRead),
            base,
            "Finance sees all purchasing data",
            now,
        ),
        grant(
            8,
            FINANCE,
            cap(Action::DataConfidential),
            table(SUPPLIERS),
            "Pays suppliers",
            now,
        ),
        grant(
            9,
            FINANCE,
            cap(Action::DataPersonal),
            table(SUPPLIERS),
            "Contacts suppliers",
            now,
        ),
        grant(
            10,
            FINANCE,
            custom("supplier.bank.view")?,
            table(SUPPLIERS),
            "Pays suppliers",
            now,
        ),
        grant(
            11,
            FINANCE,
            cap(Action::DataUnmask),
            table(SUPPLIERS),
            "Checks bank details",
            now,
        ),
        grant(
            12,
            FINANCE,
            cap(Action::AuditRead),
            Resource::Tenant,
            "Internal control",
            now,
        ),
        {
            let mut g = grant(
                13,
                FINANCE,
                custom("request.approve")?,
                table(REQUESTS),
                "Approves up to 100,000",
                now,
            );
            g.condition = Some(limit(100_000)?);
            g
        },
        grant(
            15,
            ASSISTANT,
            cap(Action::RecordRead),
            base,
            "Answers questions about purchasing",
            now,
        ),
        grant(
            16,
            ASSISTANT,
            cap(Action::RecordUpdate),
            table(LINES),
            "Fixes line items when asked",
            now,
        ),
        grant(
            17,
            TEAM,
            cap(Action::DataPersonal),
            table(SUPPLIERS),
            "Contacts suppliers",
            now,
        ),
        grant(
            19,
            ERIN,
            cap(Action::RecordRead),
            base,
            "The CFO sees all purchasing",
            now,
        ),
        grant(
            20,
            FINANCE,
            custom("order.place")?,
            table(REQUESTS),
            "Places approved orders",
            now,
        ),
    ];
    for grant in grants {
        ws.data
            .policy
            .give(grant, &DirectoryOf(&ws.data.people))
            .map_err(perm)?;
    }
    Ok(())
}

/// Gina, a guest at Northwind, sees her own company only (its contact details too), without
/// the bank or spend fields. Given once the supplier exists.
fn guest_access(ws: &mut Workspace, northwind: Option<RecordId>, now: Timestamp) -> ApiResult<()> {
    let northwind = northwind.ok_or_else(|| ApiError::internal("Northwind is missing"))?;
    let only_northwind = Scope::Records {
        records: [northwind].into(),
    };
    let mut read = grant(
        14,
        GINA,
        cap(Action::RecordRead),
        table(SUPPLIERS),
        "Supplier portal",
        now,
    );
    read.scope = only_northwind.clone();
    read.fields = FieldAccess::only([
        supplier::NAME,
        supplier::COUNTRY,
        supplier::EMAIL,
        supplier::PHONE,
        supplier::RATING,
    ]);
    let mut contacts = grant(
        18,
        GINA,
        cap(Action::DataPersonal),
        table(SUPPLIERS),
        "Her own company's contacts",
        now,
    );
    contacts.scope = only_northwind;
    for grant in [read, contacts] {
        ws.data
            .policy
            .give(grant, &DirectoryOf(&ws.data.people))
            .map_err(|e| ApiError::internal(e.to_string()))?;
    }
    Ok(())
}

/// The people as a directory before the workspace exists.
struct DirectoryOf<'a>(&'a [Person]);

impl mt_perm::Directory for DirectoryOf<'_> {
    fn kind(&self, principal: PrincipalId) -> Option<PrincipalKind> {
        self.0.iter().find(|p| p.id == principal).map(|p| p.kind)
    }

    fn groups_of(&self, principal: PrincipalId) -> std::collections::BTreeSet<PrincipalId> {
        self.0
            .iter()
            .filter(|p| p.members.contains(&principal))
            .map(|p| p.id)
            .collect()
    }

    fn members_of(&self, group: PrincipalId) -> std::collections::BTreeSet<PrincipalId> {
        self.0
            .iter()
            .find(|p| p.id == group)
            .map(|p| p.members.iter().copied().collect())
            .unwrap_or_default()
    }
}

fn text(value: &str) -> CellInput {
    CellInput::Text {
        text: value.to_owned(),
    }
}

fn value(value: Value) -> CellInput {
    CellInput::Value { value }
}

/// Adds the formula fields once the tables exist (formulas are written with field names).
fn formulas(ws: &mut Workspace) -> ApiResult<()> {
    let specs: [(TableId, FieldId, &str, DataType, ComputedKind, &str); 7] = [
        (
            LINES,
            line::AMOUNT,
            "Amount",
            DataType::Money,
            ComputedKind::Formula,
            "{Quantity} * {Unit price}",
        ),
        (
            REQUESTS,
            request::SUBTOTAL,
            "Subtotal",
            DataType::Money,
            ComputedKind::Rollup,
            "SUM({Lines}.{Amount})",
        ),
        (
            REQUESTS,
            request::TAX,
            "Tax",
            DataType::Money,
            ComputedKind::Formula,
            r#"{Subtotal} * PARAM("tax.rate")"#,
        ),
        (
            REQUESTS,
            request::TOTAL,
            "Total",
            DataType::Money,
            ComputedKind::Formula,
            "{Subtotal} + {Tax}",
        ),
        (
            REQUESTS,
            request::DAYS_LEFT,
            "Days left",
            DataType::Number,
            ComputedKind::Formula,
            "{Needed by} - TODAY()",
        ),
        (
            REQUESTS,
            request::NEEDS_CFO,
            "Needs CFO",
            DataType::Bool,
            ComputedKind::Formula,
            r#"{Total} > PARAM("approval.cfo_limit")"#,
        ),
        (
            SUPPLIERS,
            supplier::SPEND,
            "Open spend",
            DataType::Money,
            ComputedKind::Rollup,
            "SUM({Requests}.{Total})",
        ),
    ];
    for (table_id, id, name, data_type, kind, source) in specs {
        let table = ws.table(table_id)?;
        let schema = crate::formula::TableSchema { ws, table };
        let expr = mt_formula::parse(source, &schema)
            .map_err(|e| ApiError::internal(format!("{name}: {e}")))?;
        let mut def = FieldDef::new(id, name, data_type);
        def.widget = Widget::default_for(data_type);
        mt_formula::field::attach(&mut def, kind, &expr);
        mt_formula::field::check_field(&def, &schema)
            .map_err(|e| ApiError::internal(format!("{name}: {e}")))?;
        let position = if table_id == REQUESTS {
            table
                .fields
                .iter()
                .position(|f| f.id == request::LINES)
                .map(|p| p.saturating_add(1))
        } else {
            None
        };
        let fields = &mut ws
            .data
            .tables
            .iter_mut()
            .find(|t| t.id == table_id)
            .ok_or_else(|| ApiError::internal("table vanished"))?
            .fields;
        let at = position.map_or(fields.len(), |p| {
            // After the lines and the formulas added before this one.
            let after = fields
                .iter()
                .skip(p)
                .take_while(|f| f.computed.is_some())
                .count();
            p.saturating_add(after)
        });
        fields.insert(at.min(fields.len()), def);
    }
    ws.rebuild_graph()
}

/// The request lifecycle and the purchase approval, published through the designer like any
/// process a person draws.
fn process(ws: &mut Workspace, now: Timestamp) -> ApiResult<()> {
    use crate::design::{
        ApprovalDesign, ProcessDesign, SimpleCondition, StateDesign, StepDesign, TransitionDesign,
    };
    use mt_core::state::StateCategory;
    use mt_lifecycle::approval::{Approver, Mode};
    let state = |key: &str, name: &str, category: StateCategory| StateDesign {
        key: key.to_owned(),
        name: name.to_owned(),
        category,
        required: Vec::new(),
        locked: false,
        editable: Vec::new(),
    };
    let go = |key: &str, name: &str, from: &[&str], to: &str, capability: Option<&str>| {
        TransitionDesign {
            key: key.to_owned(),
            name: name.to_owned(),
            from: from.iter().map(|s| (*s).to_owned()).collect(),
            to: to.to_owned(),
            capability: capability.map(str::to_owned),
            when: None,
        }
    };
    let mut submitted = state("submitted", "Submitted", StateCategory::PendingApproval);
    submitted.required = vec![request::TITLE, request::SUPPLIER, request::LINES];
    submitted.locked = true;
    let mut approved = state("approved", "Approved", StateCategory::InProgress);
    approved.locked = true;
    approved.editable = vec![request::NEEDED_BY];
    let mut submit = go(
        "submit",
        "Submit for approval",
        &["draft"],
        "submitted",
        None,
    );
    submit.when = Some(SimpleCondition {
        field: request::TOTAL,
        op: Operator::Gt,
        text: "0 USD".to_owned(),
    });
    let design = ProcessDesign {
        name: "Purchase request".to_owned(),
        states: vec![
            state("draft", "Draft", StateCategory::New),
            submitted,
            approved,
            state("ordered", "Ordered", StateCategory::DoneSuccess),
            state("rejected", "Rejected", StateCategory::DoneFailure),
            state("cancelled", "Cancelled", StateCategory::Cancelled),
        ],
        initial: "draft".to_owned(),
        transitions: vec![
            submit,
            go("withdraw", "Withdraw", &["submitted"], "draft", None),
            go(
                "order",
                "Place the order",
                &["approved"],
                "ordered",
                Some("order.place"),
            ),
            go("revise", "Revise", &["rejected"], "draft", None),
            go("cancel", "Cancel", &[], "cancelled", None),
        ],
        approval: Some(ApprovalDesign {
            name: "Purchase approval".to_owned(),
            started_by: "submit".to_owned(),
            approved: "approved".to_owned(),
            rejected: "rejected".to_owned(),
            returned: "draft".to_owned(),
            steps: vec![
                StepDesign::Approval {
                    name: "Approver within limits".to_owned(),
                    approvers: vec![Approver::Capability {
                        capability: "request.approve".to_owned(),
                    }],
                    mode: Mode::Any,
                    can_send_back: true,
                    timeout_hours: None,
                    escalate_to: None,
                    when: None,
                },
                StepDesign::Approval {
                    name: "CFO".to_owned(),
                    approvers: vec![Approver::Person { id: ERIN }],
                    mode: Mode::Any,
                    can_send_back: true,
                    timeout_hours: None,
                    escalate_to: None,
                    when: Some(SimpleCondition {
                        field: request::NEEDS_CFO,
                        op: Operator::Eq,
                        text: "yes".to_owned(),
                    }),
                },
                StepDesign::Notify {
                    name: "Approved — ready to order".to_owned(),
                    to: vec![Approver::Capability {
                        capability: "order.place".to_owned(),
                    }],
                    when: None,
                },
            ],
        }),
    };
    crate::design::publish(ws, Actor::direct(ALICE), REQUESTS, design, now)
}

/// The open task of a person on a record's approval, approved.
fn approve_as(
    ws: &mut Workspace,
    who: PrincipalId,
    record: RecordId,
    now: Timestamp,
) -> ApiResult<()> {
    let (run, task) = ws
        .data
        .process
        .runs
        .iter()
        .filter(|r| r.record == record)
        .find_map(|r| r.approval.tasks_of(who).first().map(|t| (r.id, t.id)))
        .ok_or_else(|| ApiError::internal(format!("{} has no task", ws.name_of(who))))?;
    crate::process::act(
        ws,
        Actor::direct(who),
        run,
        task,
        mt_lifecycle::approval::Act::Approve { comment: None },
        &BTreeMap::new(),
        now,
    )
    .map(|_| ())
}

/// An empty workspace: the built-in reference sets and one administrator who may do
/// everything. Users build their bases from here, or install a template.
///
/// # Errors
/// Only if the built-in definitions are inconsistent (a bug).
pub fn empty() -> ApiResult<Workspace> {
    let data = Data {
        bases: Vec::new(),
        tables: Vec::new(),
        refsets: vec![
            mt_refdata::builtin::countries(),
            mt_refdata::builtin::currencies(),
        ],
        people: people().into_iter().filter(|p| p.id == ALICE).collect(),
        policy: Policy::new(),
        params: BTreeMap::new(),
        counters: BTreeMap::new(),
        next_id: 0,
        audit: Vec::new(),
        process: crate::process::Process::default(),
        links: Vec::new(),
        reviews: Vec::new(),
        shares: Vec::new(),
        refset_history: Vec::new(),
    };
    let mut ws = Workspace::new(data)?;
    let perm = |e: mt_perm::PermError| ApiError::internal(e.to_string());
    ws.data
        .policy
        .add_set(CapabilitySet {
            id: mt_core::id::CapabilitySetId::from_u128(1),
            name: "Workspace administration".to_owned(),
            description: Some("Everything, everywhere".to_owned()),
            capabilities: Action::ALL.iter().map(|a| Capability::from(*a)).collect(),
        })
        .map_err(perm)?;
    let admin = grant(
        1,
        ALICE,
        Granted::Set(mt_core::id::CapabilitySetId::from_u128(1)),
        Resource::Tenant,
        "Administers the workspace",
        Timestamp::EPOCH,
    );
    ws.data
        .policy
        .give(admin, &DirectoryOf(&ws.data.people))
        .map_err(perm)?;
    Ok(ws)
}

/// The purchasing sample installed on an empty workspace (tests, `--demo`).
///
/// # Errors
/// Only if the demo definitions are inconsistent (a bug).
pub fn demo(now: Timestamp) -> ApiResult<Workspace> {
    let mut ws = empty()?;
    install_purchasing(&mut ws, Actor::direct(ALICE), now)?;
    Ok(ws)
}

/// The templates that can be installed.
#[must_use]
pub fn templates() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![(
        "purchasing",
        "Purchasing (sample)",
        "Suppliers, purchase requests with lines, cost centres, formulas, approvals routed by \
         limits, a CFO step, and seven people with different access",
    )]
}

/// Installs the purchasing sample as its own base: tables, cost centres, people, access,
/// the request lifecycle and approval, and records taken through the process.
///
/// # Errors
/// 409 when it is installed already.
pub fn install_purchasing(ws: &mut Workspace, actor: Actor, now: Timestamp) -> ApiResult<()> {
    if ws.data.bases.iter().any(|b| b.id == BASE) {
        return Err(ApiError::conflict(
            "the purchasing sample is installed already",
        ));
    }
    ws.data.bases.push(Base {
        id: BASE,
        name: "Purchasing".to_owned(),
        description: "Suppliers, purchase requests and their approval (sample)".to_owned(),
    });
    ws.audit_object(
        actor,
        "template.install",
        mt_audit::ObjectRef::new("base", BASE.to_string()),
        serde_json::json!({"template": "purchasing", "base": "Purchasing"}),
        now,
    )?;
    ws.data
        .tables
        .extend([suppliers_table(), requests_table()?, lines_table()?]);
    ws.data.refsets.push(cost_centres());
    ws.rebuild_refdata()?;
    for person in people() {
        if ws.person(person.id).is_none() {
            ws.data.people.push(person);
        }
    }
    ws.data.params.extend([
        ("tax.rate".to_owned(), Value::Percent(Decimal::new(8, 2))),
        ("approval.cfo_limit".to_owned(), money(25_000)?),
    ]);
    ws.data.links.extend([
        LinkPair {
            table: REQUESTS,
            field: request::LINES,
            inverse_table: LINES,
            inverse_field: line::REQUEST,
        },
        LinkPair {
            table: REQUESTS,
            field: request::SUPPLIER,
            inverse_table: SUPPLIERS,
            inverse_field: supplier::REQUESTS,
        },
    ]);
    ws.rebuild_graph()?;
    formulas(ws)?;
    policy(ws, now)?;
    process(ws, now)?;
    records(ws, now)?;
    Ok(())
}

fn country(code: &str) -> Value {
    Value::Reference(mt_refdata::builtin::system_item_id("iso_3166_1", code))
}

fn records(ws: &mut Workspace, now: Timestamp) -> ApiResult<()> {
    let alice = Actor::direct(ALICE);
    let mut suppliers: BTreeMap<&str, RecordId> = BTreeMap::new();
    let supplier_rows: [(&str, &str, &str, &str, &str, &str, bool); 4] = [
        (
            "Northwind Traders",
            "US",
            "orders@northwind.example",
            "+14155550123",
            "GB82WEST12345698765432",
            "4",
            true,
        ),
        (
            "Contoso Supplies",
            "DE",
            "einkauf@contoso.example",
            "+4930123456",
            "DE89370400440532013000",
            "5",
            true,
        ),
        (
            "Fabrikam Industries",
            "JP",
            "sales@fabrikam.example",
            "+81312345678",
            "NL91ABNA0417164300",
            "3",
            false,
        ),
        (
            "Tailspin Events",
            "FR",
            "",
            "+33123456789",
            "FR1420041010050500013M02606",
            "4",
            false,
        ),
    ];
    for (name, code, email, phone, iban, rating, preferred) in supplier_rows {
        let mut cells = BTreeMap::from([
            (supplier::NAME, text(name)),
            (supplier::COUNTRY, value(country(code))),
            (supplier::PHONE, text(phone)),
            (supplier::IBAN, text(iban)),
            (supplier::RATING, text(rating)),
            (supplier::PREFERRED, value(Value::Bool(preferred))),
        ]);
        if !email.is_empty() {
            cells.insert(supplier::EMAIL, text(email));
        }
        let id = ws.create_record(alice, SUPPLIERS, &cells, now)?;
        suppliers.insert(name, id);
    }
    guest_access(ws, suppliers.get("Northwind Traders").copied(), now)?;
    let day = |offset: i32| {
        now.date()
            .checked_add_days(offset)
            .map_or_else(String::new, |d| d.to_string())
    };
    let supplier_of = |name: &str| {
        suppliers
            .get(name)
            .copied()
            .map_or(Value::Null, |id| Value::Link(vec![id]))
    };
    type LineSpec<'a> = (&'a str, u128, &'a str, &'a str);
    type RequestSpec<'a> = (
        PrincipalId,
        &'a str,
        &'a str,
        u128,
        RefItemId,
        i32,
        Vec<LineSpec<'a>>,
    );
    let requests: [RequestSpec<'_>; 5] = [
        (
            BOB,
            "Laptops for new hires",
            "Northwind Traders",
            12,
            status::SUBMITTED,
            14,
            vec![
                ("Laptop 14\"", 9101, "3", "1450.00"),
                ("Docking station", 9101, "3", "180.00"),
                ("Wireless mouse", 9101, "3", "25.00"),
            ],
        ),
        (
            BOB,
            "Office chairs",
            "Contoso Supplies",
            11,
            status::DRAFT,
            30,
            vec![("Ergonomic chair", 9104, "6", "320.00")],
        ),
        (
            CAROL,
            "Design tool licences",
            "Fabrikam Industries",
            21,
            status::APPROVED,
            -5,
            vec![
                ("Annual licence", 9102, "12", "600.00"),
                ("Onboarding workshop", 9103, "1", "2500.00"),
            ],
        ),
        (
            CAROL,
            "Trade fair booth",
            "Tailspin Events",
            22,
            status::SUBMITTED,
            45,
            vec![
                ("Booth construction", 9103, "1", "18000.00"),
                ("Travel", 9103, "4", "1900.00"),
            ],
        ),
        (
            BOB,
            "Printer toner",
            "Northwind Traders",
            11,
            status::ORDERED,
            3,
            vec![("Toner cartridge", 9104, "10", "89.90")],
        ),
    ];
    let mut created: Vec<(PrincipalId, RecordId, RefItemId)> = Vec::new();
    for (who, title, supplier_name, centre, state, due, lines) in requests {
        let actor = Actor::direct(who);
        let cells = BTreeMap::from([
            (request::TITLE, text(title)),
            (request::SUPPLIER, value(supplier_of(supplier_name))),
            (request::COST_CENTRE, value(Value::Reference(cc(centre)))),
            (request::NEEDED_BY, text(&day(due))),
        ]);
        let request_id = ws.create_record(actor, REQUESTS, &cells, now)?;
        created.push((who, request_id, state));
        for (item, category, quantity, price) in lines {
            let cells = BTreeMap::from([
                (line::ITEM, text(item)),
                (line::REQUEST, value(Value::Link(vec![request_id]))),
                (
                    line::CATEGORY,
                    value(Value::Reference(RefItemId::from_u128(category))),
                ),
                (line::QUANTITY, text(quantity)),
                (line::PRICE, text(&format!("{price} USD"))),
            ]);
            ws.create_record(actor, LINES, &cells, now)?;
        }
    }
    // Take each request to its state the way people would.
    for (who, id, state) in created {
        let submit = |ws: &mut Workspace| {
            crate::process::transition(ws, Actor::direct(who), REQUESTS, id, "submit", now)
        };
        if state == status::SUBMITTED {
            submit(ws)?;
            // Above the controller's limit, finance approves first; the CFO is next.
            if who == CAROL {
                approve_as(ws, DAVE, id, now)?;
            }
        } else if state == status::APPROVED {
            submit(ws)?;
            approve_as(ws, DAVE, id, now)?;
        } else if state == status::ORDERED {
            submit(ws)?;
            approve_as(ws, CAROL, id, now)?;
            crate::process::transition(ws, Actor::direct(DAVE), REQUESTS, id, "order", now)?;
        }
    }
    Ok(())
}

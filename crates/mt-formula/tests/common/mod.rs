//! Shared fixtures: an "orders" table that links to "order lines", one record, and an
//! environment with a fixed today, working days, parameters and reference labels.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use mt_core::decimal::{canonical_string, Decimal, Rounding};
use mt_core::field::DataType;
use mt_core::id::{FieldId, PrincipalId, RecordId, RefItemId};
use mt_core::money::{Currency, Money};
use mt_core::time::{Date, DurationMs, Timestamp, Weekday};
use mt_core::value::Value;
use mt_formula::{evaluate, parse, Env, Expr, Names, Record, Schema, Val};
use std::collections::BTreeMap;

// Fields of the orders table.
pub const TITLE: u128 = 1;
pub const QTY: u128 = 2;
pub const PRICE: u128 = 3;
pub const DISCOUNT: u128 = 4;
pub const DUE: u128 = 5;
pub const CREATED: u128 = 6;
pub const DONE: u128 = 7;
pub const STATUS: u128 = 8;
pub const TAGS: u128 = 9;
pub const LINES: u128 = 10;
pub const EFFORT: u128 = 11;
pub const NOTES: u128 = 12;
pub const OWNER: u128 = 13;
pub const EMPTY_NUMBER: u128 = 14;
pub const EMPTY_DATE: u128 = 15;
pub const EMPTY_TEXT: u128 = 16;
pub const ODD_NAME: u128 = 17;
pub const EMPTY_MONEY: u128 = 18;
pub const SHIPPING: u128 = 19;
pub const FILES: u128 = 20;
pub const EXTRA: u128 = 21;

// Fields of the order lines table.
pub const LINE_AMOUNT: u128 = 101;
pub const LINE_QTY: u128 = 102;
pub const LINE_NAME: u128 = 103;
pub const LINE_SHIPPED: u128 = 104;
pub const LINE_LABELS: u128 = 105;
pub const LINE_DONE: u128 = 106;

pub fn fid(n: u128) -> FieldId {
    FieldId::from_u128(n)
}

pub fn item(n: u128) -> RefItemId {
    RefItemId::from_u128(n)
}

pub fn dec(text: &str) -> Decimal {
    text.parse().unwrap()
}

pub fn num(text: &str) -> Value {
    Value::Number(dec(text))
}

pub fn money(amount: &str, currency: &str) -> Value {
    Value::Money(Money::new(dec(amount), Currency::new(currency).unwrap()).unwrap())
}

pub fn day(y: i32, m: u32, d: u32) -> Date {
    Date::from_ymd(y, m, d).unwrap()
}

pub fn at(y: i32, m: u32, d: u32, hour: i64, minute: i64) -> Timestamp {
    Timestamp::from_millis(
        day(y, m, d).to_timestamp().millis() + hour * 3_600_000 + minute * 60_000,
    )
    .unwrap()
}

const ORDER_FIELDS: &[(u128, &str, DataType)] = &[
    (TITLE, "Title", DataType::Text),
    (QTY, "Qty", DataType::Number),
    (PRICE, "Price", DataType::Money),
    (DISCOUNT, "Discount", DataType::Percent),
    (DUE, "Due", DataType::Date),
    (CREATED, "Created", DataType::DateTime),
    (DONE, "Done", DataType::Bool),
    (STATUS, "Status", DataType::Reference),
    (TAGS, "Tags", DataType::MultiReference),
    (LINES, "Lines", DataType::Link),
    (EFFORT, "Effort", DataType::Duration),
    (NOTES, "Notes", DataType::LongText),
    (OWNER, "Owner", DataType::Principal),
    (EMPTY_NUMBER, "Empty number", DataType::Number),
    (EMPTY_DATE, "Empty date", DataType::Date),
    (EMPTY_TEXT, "Empty text", DataType::Text),
    (ODD_NAME, "Odd }name\\", DataType::Text),
    (EMPTY_MONEY, "Empty money", DataType::Money),
    (SHIPPING, "Shipping", DataType::Money),
    (FILES, "Files", DataType::Attachment),
    (EXTRA, "Extra", DataType::Json),
];

const LINE_FIELDS: &[(u128, &str, DataType)] = &[
    (LINE_AMOUNT, "Amount", DataType::Money),
    (LINE_QTY, "Qty", DataType::Number),
    (LINE_NAME, "Name", DataType::Text),
    (LINE_SHIPPED, "Shipped", DataType::Date),
    (LINE_LABELS, "Labels", DataType::MultiReference),
    (LINE_DONE, "Done", DataType::Bool),
];

fn by_name(fields: &[(u128, &str, DataType)], name: &str) -> Option<FieldId> {
    fields
        .iter()
        .find(|(_, field_name, _)| field_name.eq_ignore_ascii_case(name))
        .map(|(id, _, _)| fid(*id))
}

fn type_in(fields: &[(u128, &str, DataType)], field: FieldId) -> Option<DataType> {
    fields
        .iter()
        .find(|(id, _, _)| fid(*id) == field)
        .map(|(_, _, data_type)| *data_type)
}

fn name_in(fields: &[(u128, &str, DataType)], field: FieldId) -> Option<String> {
    fields
        .iter()
        .find(|(id, _, _)| fid(*id) == field)
        .map(|(_, name, _)| (*name).to_owned())
}

/// The orders table and, through `Lines`, the order lines table.
pub struct Orders;

impl Schema for Orders {
    fn field_by_name(&self, name: &str) -> Option<FieldId> {
        by_name(ORDER_FIELDS, name)
    }

    fn field_type(&self, field: FieldId) -> Option<DataType> {
        type_in(ORDER_FIELDS, field)
    }

    fn linked_field_by_name(&self, link: FieldId, name: &str) -> Option<FieldId> {
        (link == fid(LINES))
            .then(|| by_name(LINE_FIELDS, name))
            .flatten()
    }

    fn linked_field_type(&self, link: FieldId, field: FieldId) -> Option<DataType> {
        (link == fid(LINES))
            .then(|| type_in(LINE_FIELDS, field))
            .flatten()
    }
}

impl Names for Orders {
    fn field(&self, field: FieldId) -> Option<String> {
        name_in(ORDER_FIELDS, field)
    }

    fn linked_field(&self, link: FieldId, field: FieldId) -> Option<String> {
        (link == fid(LINES))
            .then(|| name_in(LINE_FIELDS, field))
            .flatten()
    }
}

/// A record with the values of linked records.
#[derive(Debug, Clone, Default)]
pub struct Rec {
    pub cells: BTreeMap<FieldId, Value>,
    pub linked: BTreeMap<(FieldId, FieldId), Vec<Value>>,
}

impl Record for Rec {
    fn value(&self, field: FieldId) -> Option<&Value> {
        self.cells.get(&field)
    }

    fn linked_values(&self, link: FieldId, field: FieldId) -> Vec<Value> {
        self.linked.get(&(link, field)).cloned().unwrap_or_default()
    }
}

/// One order with three lines.
///
/// | Field | Value |
/// |---|---|
/// | Title | `Spring order` |
/// | Qty | 3 |
/// | Price | 19.99 USD |
/// | Discount | 10 % |
/// | Due | 2026-05-20 |
/// | Created | 2026-05-01 08:15 UTC |
/// | Done | false |
/// | Status | item 1 ("Open") |
/// | Tags | items 1, 2 |
/// | Effort | 1 h 30 min |
/// | Shipping | 4.50 USD |
///
/// Lines: amounts 10.00, 25.50, 4.25 USD · quantities 1, 2, (none) · names Bolt, nut, Washer ·
/// shipped 2026-05-03, (none), 2026-05-10 · done yes, no, yes.
pub fn order() -> Rec {
    let mut rec = Rec::default();
    let cells = [
        (TITLE, Value::Text("Spring order".into())),
        (QTY, num("3")),
        (PRICE, money("19.99", "USD")),
        (DISCOUNT, Value::Percent(dec("0.1"))),
        (DUE, Value::Date(day(2026, 5, 20))),
        (CREATED, Value::DateTime(at(2026, 5, 1, 8, 15))),
        (DONE, Value::Bool(false)),
        (STATUS, Value::Reference(item(1))),
        (TAGS, Value::MultiReference(vec![item(1), item(2)])),
        (
            LINES,
            Value::Link(vec![
                RecordId::from_u128(1),
                RecordId::from_u128(2),
                RecordId::from_u128(3),
            ]),
        ),
        (EFFORT, Value::Duration(DurationMs::from_millis(5_400_000))),
        (NOTES, Value::LongText("Fragile.\nHandle with care".into())),
        (OWNER, Value::Principal(vec![PrincipalId::from_u128(7)])),
        (ODD_NAME, Value::Text("odd".into())),
        (SHIPPING, money("4.50", "USD")),
        (EMPTY_TEXT, Value::Text("   ".into())),
    ];
    for (field, value) in cells {
        rec.cells.insert(fid(field), value);
    }
    let linked = [
        (
            LINE_AMOUNT,
            vec![
                money("10.00", "USD"),
                money("25.50", "USD"),
                money("4.25", "USD"),
            ],
        ),
        (LINE_QTY, vec![num("1"), num("2"), Value::Null]),
        (
            LINE_NAME,
            vec![
                Value::Text("Bolt".into()),
                Value::Text("nut".into()),
                Value::Text("Washer".into()),
            ],
        ),
        (
            LINE_SHIPPED,
            vec![
                Value::Date(day(2026, 5, 3)),
                Value::Null,
                Value::Date(day(2026, 5, 10)),
            ],
        ),
        (
            LINE_LABELS,
            vec![
                Value::MultiReference(vec![item(1)]),
                Value::MultiReference(vec![]),
                Value::MultiReference(vec![item(2), item(1)]),
            ],
        ),
        (
            LINE_DONE,
            vec![Value::Bool(true), Value::Bool(false), Value::Bool(true)],
        ),
    ];
    for (field, values) in linked {
        rec.linked.insert((fid(LINES), fid(field)), values);
    }
    rec
}

/// Wednesday 2026-05-13, 10:30 UTC. Monday to Friday are working days, except the holiday
/// on Monday 2026-05-25.
#[derive(Debug, Clone, Copy)]
pub struct World {
    /// Hours the local day starts before UTC midnight (9 for Tokyo, -7 for Los Angeles).
    pub hours_ahead: i64,
    /// A calendar without any working day.
    pub idle: bool,
}

pub const UTC: World = World {
    hours_ahead: 0,
    idle: false,
};
pub const TOKYO: World = World {
    hours_ahead: 9,
    idle: false,
};
pub const LOS_ANGELES: World = World {
    hours_ahead: -7,
    idle: false,
};
pub const IDLE: World = World {
    hours_ahead: 0,
    idle: true,
};

impl Env for World {
    fn today(&self) -> Date {
        day(2026, 5, 13)
    }

    fn now(&self) -> Timestamp {
        at(2026, 5, 13, 10, 30)
    }

    fn start_of_day(&self, date: Date) -> Timestamp {
        Timestamp::from_millis(date.to_timestamp().millis() - self.hours_ahead * 3_600_000).unwrap()
    }

    fn is_workday(&self, date: Date) -> bool {
        !self.idle
            && !matches!(date.weekday(), Weekday::Saturday | Weekday::Sunday)
            && date != day(2026, 5, 25)
    }

    fn param(&self, key: &str) -> Option<Value> {
        match key {
            "tax.rate" => Some(Value::Percent(dec("0.19"))),
            "app.name" => Some(Value::Text("Demo".into())),
            "ship.flat" => Some(money("5.00", "USD")),
            "ship.eur" => Some(money("5.00", "EUR")),
            _ => None,
        }
    }

    fn label(&self, item: RefItemId) -> Option<String> {
        match item.as_u128() {
            1 => Some("Open".into()),
            2 => Some("Closed".into()),
            _ => None,
        }
    }

    fn code(&self, item: RefItemId) -> Option<String> {
        self.label(item).map(|label| label.to_lowercase())
    }

    fn attribute(&self, item: RefItemId, name: &str) -> Option<Value> {
        match (item.as_u128(), name) {
            (1, "weight") => Some(num("5")),
            (2, "weight") => Some(num("0.5")),
            (1, "colour") => Some(Value::Text("green".into())),
            _ => None,
        }
    }
}

pub fn compile(source: &str) -> Expr {
    parse(source, &Orders).unwrap_or_else(|error| panic!("`{source}` does not parse: {error}"))
}

/// A value as shown in the tests: text in quotes, lists in brackets, errors by code.
pub fn show(value: &Val) -> String {
    match value {
        Val::Blank => "(blank)".into(),
        Val::Number(n) => canonical_string(n),
        Val::Text(text) => format!("\"{text}\""),
        Val::Bool(true) => "TRUE".into(),
        Val::Bool(false) => "FALSE".into(),
        Val::Date(d) => d.to_string(),
        Val::DateTime(t) => t.to_string(),
        Val::Duration(d) => format!("{}ms", d.millis()),
        Val::Money(amount, currency) => {
            format!(
                "{} {}",
                canonical_string(&Rounding::HalfEven.round(*amount, 6)),
                currency.code()
            )
        }
        Val::Ref(item) => format!("ref:{}", item.as_u128()),
        Val::List(items) => format!(
            "[{}]",
            items.iter().map(show).collect::<Vec<_>>().join(", ")
        ),
        Val::Error(error) => error.code().to_owned(),
    }
}

/// Evaluates a formula for the sample order in the UTC world.
pub fn run(source: &str) -> String {
    run_in(source, &order(), &UTC)
}

/// Evaluates a formula for a record in a world.
pub fn run_in(source: &str, record: &Rec, world: &World) -> String {
    show(&evaluate(&compile(source), &Orders, record, world))
}

/// Checks a table of `(formula, expected)` and reports every difference at once.
pub fn check(cases: &[(&str, &str)]) {
    check_in(cases, &order(), &UTC);
}

/// As [`check`], for a record in a world.
pub fn check_in(cases: &[(&str, &str)], record: &Rec, world: &World) {
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|(source, expected)| {
            let found = run_in(source, record, world);
            (found != *expected)
                .then(|| format!("{source}\n     expected {expected}\n     found    {found}"))
        })
        .collect();
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

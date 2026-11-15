//! Shared fixtures: a small "deals" table held in memory.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use mt_core::condition::UtcContext;
use mt_core::decimal::Decimal;
use mt_core::field::DataType;
use mt_core::id::{FieldId, PrincipalId, RecordId, RefItemId};
use mt_core::money::{Currency, Money};
use mt_core::time::{Date, DurationMs, Timestamp};
use mt_core::value::Value;
use mt_query::{Env, Row};
use std::collections::BTreeMap;

pub const TITLE: u128 = 1;
pub const AMOUNT: u128 = 2;
pub const QTY: u128 = 3;
pub const STAGE: u128 = 4;
pub const CLOSE: u128 = 5;
pub const CREATED: u128 = 6;
pub const OWNERS: u128 = 7;
pub const DONE: u128 = 8;
pub const TAGS: u128 = 9;
pub const NOTES: u128 = 10;
pub const RATE: u128 = 11;
pub const EFFORT: u128 = 12;
pub const EXTRA: u128 = 13;
pub const FILES: u128 = 14;

// Stage items. Their ids are deliberately not in pipeline order.
pub const LEAD: u128 = 40;
pub const QUALIFIED: u128 = 10;
pub const WON: u128 = 30;
pub const LOST: u128 = 20;

pub fn fid(n: u128) -> FieldId {
    FieldId::from_u128(n)
}

pub fn rec(n: u128) -> RecordId {
    RecordId::from_u128(n)
}

pub fn item(n: u128) -> RefItemId {
    RefItemId::from_u128(n)
}

pub fn person(n: u128) -> PrincipalId {
    PrincipalId::from_u128(n)
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

pub fn text(s: &str) -> Value {
    Value::Text(s.into())
}

pub fn day(y: i32, m: u32, d: u32) -> Date {
    Date::from_ymd(y, m, d).unwrap()
}

pub fn date(y: i32, m: u32, d: u32) -> Value {
    Value::Date(day(y, m, d))
}

pub fn instant(y: i32, m: u32, d: u32, hour: i64) -> Value {
    Value::DateTime(
        Timestamp::from_millis(day(y, m, d).to_timestamp().millis() + hour * 3_600_000).unwrap(),
    )
}

pub fn minutes(m: i64) -> Value {
    Value::Duration(DurationMs::from_millis(m * 60_000))
}

pub fn stage(n: u128) -> Value {
    Value::Reference(item(n))
}

pub fn type_of(field: FieldId) -> Option<DataType> {
    Some(match field.as_u128() {
        TITLE => DataType::Text,
        AMOUNT => DataType::Money,
        QTY => DataType::Number,
        STAGE => DataType::Reference,
        CLOSE => DataType::Date,
        CREATED => DataType::DateTime,
        OWNERS => DataType::Principal,
        DONE => DataType::Bool,
        TAGS => DataType::MultiReference,
        NOTES => DataType::LongText,
        RATE => DataType::Percent,
        EFFORT => DataType::Duration,
        EXTRA => DataType::Json,
        FILES => DataType::Attachment,
        _ => return None,
    })
}

/// A record held in memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rec {
    pub id: RecordId,
    pub cells: BTreeMap<FieldId, Value>,
}

impl Row for Rec {
    fn id(&self) -> RecordId {
        self.id
    }

    fn value(&self, field: FieldId) -> Option<&Value> {
        self.cells.get(&field)
    }
}

pub fn row(id: u128, cells: &[(u128, Value)]) -> Rec {
    Rec {
        id: rec(id),
        cells: cells
            .iter()
            .map(|(field, value)| (fid(*field), value.clone()))
            .collect(),
    }
}

/// The table's environment: stages are ordered by pipeline position and searched by label;
/// people are searched by name.
pub struct Deals;

impl Env for Deals {
    fn data_type(&self, field: FieldId) -> Option<DataType> {
        type_of(field)
    }

    fn order_value(&self, field: FieldId, value: &Value) -> Option<Value> {
        match (field.as_u128(), value) {
            (STAGE, Value::Reference(id)) => {
                let position = match id.as_u128() {
                    LEAD => 1,
                    QUALIFIED => 2,
                    WON => 3,
                    LOST => 4,
                    _ => return None,
                };
                Some(Value::Number(Decimal::from(position)))
            }
            _ => None,
        }
    }

    fn display_text(&self, field: FieldId, value: &Value) -> Option<String> {
        match (field.as_u128(), value) {
            (STAGE, Value::Reference(id)) => Some(
                match id.as_u128() {
                    LEAD => "Lead",
                    QUALIFIED => "Qualified",
                    WON => "Won",
                    LOST => "Lost",
                    _ => return None,
                }
                .to_owned(),
            ),
            (OWNERS, Value::Principal(people)) => Some(
                people
                    .iter()
                    .map(|p| match p.as_u128() {
                        1 => "Aiko Tanaka",
                        2 => "Ben Schmidt",
                        _ => "Someone",
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            _ => None,
        }
    }
}

/// An environment that knows the field types and nothing else.
pub struct Plain;

impl Env for Plain {
    fn data_type(&self, field: FieldId) -> Option<DataType> {
        type_of(field)
    }

    fn order_value(&self, _field: FieldId, _value: &Value) -> Option<Value> {
        None
    }

    fn display_text(&self, _field: FieldId, _value: &Value) -> Option<String> {
        None
    }
}

pub fn ctx() -> UtcContext {
    UtcContext {
        me: Some(person(1)),
        ..UtcContext::on(day(2026, 5, 13))
    }
}

/// Eight deals.
///
/// | id | title | amount | qty | stage | close | owners | done |
/// |---|---|---|---|---|---|---|---|
/// | 1 | Alpha renewal | 1200.00 USD | 3 | won | 2026-05-02 | 1 | yes |
/// | 2 | beta pilot | 300.00 USD | 1 | lead | 2026-05-20 | 2 | no |
/// | 3 | Gamma expansion | 5000.00 USD | 10 | qualified | 2026-06-01 | 1, 2 | no |
/// | 4 | delta upsell | 300.00 USD | — | lead | — | — | — |
/// | 5 | Epsilon | 750.50 USD | 2.5 | lost | 2026-04-15 | 2 | yes |
/// | 6 | Zeta | — | 1.0 | — | 2026-05-20 | 1 | no |
/// | 7 | eta trial | 80.00 USD | 1 | qualified | 2026-06-30 | 2, 1 | no |
/// | 8 | Theta | 300.00 USD | 4 | won | 2026-05-02 | 1 | yes |
pub fn deals() -> Vec<Rec> {
    vec![
        row(
            1,
            &[
                (TITLE, text("Alpha renewal")),
                (AMOUNT, money("1200.00", "USD")),
                (QTY, num("3")),
                (STAGE, stage(WON)),
                (CLOSE, date(2026, 5, 2)),
                (OWNERS, Value::Principal(vec![person(1)])),
                (DONE, Value::Bool(true)),
                (CREATED, instant(2026, 4, 1, 9)),
                (RATE, Value::Percent(dec("0.9"))),
                (EFFORT, minutes(90)),
                (TAGS, Value::MultiReference(vec![item(1), item(2)])),
            ],
        ),
        row(
            2,
            &[
                (TITLE, text("beta pilot")),
                (AMOUNT, money("300.00", "USD")),
                (QTY, num("1")),
                (STAGE, stage(LEAD)),
                (CLOSE, date(2026, 5, 20)),
                (OWNERS, Value::Principal(vec![person(2)])),
                (DONE, Value::Bool(false)),
                (CREATED, instant(2026, 4, 30, 23)),
                (RATE, Value::Percent(dec("0.1"))),
                (EFFORT, minutes(30)),
                (TAGS, Value::MultiReference(vec![item(2), item(1)])),
            ],
        ),
        row(
            3,
            &[
                (TITLE, text("Gamma expansion")),
                (AMOUNT, money("5000.00", "USD")),
                (QTY, num("10")),
                (STAGE, stage(QUALIFIED)),
                (CLOSE, date(2026, 6, 1)),
                (OWNERS, Value::Principal(vec![person(1), person(2)])),
                (DONE, Value::Bool(false)),
                (CREATED, instant(2026, 5, 1, 0)),
                (RATE, Value::Percent(dec("0.5"))),
                (TAGS, Value::MultiReference(vec![item(3)])),
            ],
        ),
        row(
            4,
            &[
                (TITLE, text("delta upsell")),
                (AMOUNT, money("300.00", "USD")),
                (STAGE, stage(LEAD)),
                (
                    NOTES,
                    Value::LongText("Follow up after the Berlin fair".into()),
                ),
            ],
        ),
        row(
            5,
            &[
                (TITLE, text("Epsilon")),
                (AMOUNT, money("750.50", "USD")),
                (QTY, num("2.5")),
                (STAGE, stage(LOST)),
                (CLOSE, date(2026, 4, 15)),
                (OWNERS, Value::Principal(vec![person(2)])),
                (DONE, Value::Bool(true)),
                (CREATED, instant(2026, 3, 10, 12)),
                (RATE, Value::Percent(dec("0"))),
                (EFFORT, minutes(45)),
            ],
        ),
        row(
            6,
            &[
                (TITLE, text("Zeta")),
                (QTY, num("1.0")),
                (CLOSE, date(2026, 5, 20)),
                (OWNERS, Value::Principal(vec![person(1)])),
                (DONE, Value::Bool(false)),
                (TAGS, Value::MultiReference(vec![])),
            ],
        ),
        row(
            7,
            &[
                (TITLE, text("eta trial")),
                (AMOUNT, money("80.00", "USD")),
                (QTY, num("1")),
                (STAGE, stage(QUALIFIED)),
                (CLOSE, date(2026, 6, 30)),
                (OWNERS, Value::Principal(vec![person(2), person(1)])),
                (DONE, Value::Bool(false)),
                (CREATED, instant(2026, 5, 12, 8)),
            ],
        ),
        row(
            8,
            &[
                (TITLE, text("Theta")),
                (AMOUNT, money("300.00", "USD")),
                (QTY, num("4")),
                (STAGE, stage(WON)),
                (CLOSE, date(2026, 5, 2)),
                (OWNERS, Value::Principal(vec![person(1)])),
                (DONE, Value::Bool(true)),
                (CREATED, instant(2026, 4, 1, 9)),
            ],
        ),
    ]
}

pub fn ids(rows: &[RecordId]) -> Vec<u128> {
    rows.iter().map(|id| id.as_u128()).collect()
}

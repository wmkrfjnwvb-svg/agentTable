//! Shared fixtures: a region tree, a flat priority list and a small system set.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use mt_core::field::RefScope;
use mt_core::id::{PrincipalId, RefItemId, RefSetId};
use mt_core::time::{Date, Timestamp};
use mt_refdata::{RefItem, RefSet, SetScope, State, Structure};
use std::collections::BTreeSet;

pub fn rid(n: u128) -> RefItemId {
    RefItemId::from_u128(n)
}

pub fn person(n: u128) -> PrincipalId {
    PrincipalId::from_u128(n)
}

pub fn day(y: i32, m: u32, d: u32) -> Date {
    Date::from_ymd(y, m, d).unwrap()
}

pub fn at(ms: i64) -> Timestamp {
    Timestamp::from_millis(ms).unwrap()
}

pub const REGION: u128 = 700;
pub const PRIORITY: u128 = 701;
pub const SYSTEM: u128 = 702;

pub fn region_set() -> RefSet {
    RefSet::new(
        RefSetId::from_u128(REGION),
        "region",
        "Region",
        Structure::Tree { max_depth: 4 },
        SetScope::Tenant,
    )
}

pub fn item(id: u128, code: &str, name: &str, parent: Option<u128>) -> RefItem {
    let mut item = RefItem::new(rid(id), code, name);
    item.parent = parent.map(rid);
    item
}

pub fn aliases(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

/// ```text
/// Americas (3)                       sort 0
/// Asia (1)                           sort 1
///   Japan (11)       sap: JP, JPN · hris: 81 · ja: 日本
///     Kansai (112)   disabled
///       Osaka (1121)
///     Kanto (111)
///       Tokyo (1111)
///       Yokohama (1112)
///   Korea (12)       valid from 2025-01-01
///     Seoul (121)
/// Europe (2)                         sort 2
///   Germany (21)     sap: DE · de: Deutschland
///     Bavaria (211)
///     Berlin (212)   valid to 2025-12-31
/// ```
pub fn region_items() -> Vec<RefItem> {
    let mut items = vec![
        item(1, "asia", "Asia", None),
        item(11, "jp", "Japan", Some(1)),
        item(111, "jp-kanto", "Kanto", Some(11)),
        item(1111, "jp-tokyo", "Tokyo", Some(111)),
        item(1112, "jp-yokohama", "Yokohama", Some(111)),
        item(112, "jp-kansai", "Kansai", Some(11)),
        item(1121, "jp-osaka", "Osaka", Some(112)),
        item(12, "kr", "Korea", Some(1)),
        item(121, "kr-seoul", "Seoul", Some(12)),
        item(2, "europe", "Europe", None),
        item(21, "de", "Germany", Some(2)),
        item(211, "de-by", "Bavaria", Some(21)),
        item(212, "de-be", "Berlin", Some(21)),
        item(3, "americas", "Americas", None),
    ];
    for item in &mut items {
        match item.id.as_u128() {
            1 => item.sort = 1,
            2 => item.sort = 2,
            11 => {
                item.names.insert("ja".into(), "日本".into());
                item.aliases.insert("sap".into(), aliases(&["JP", "JPN"]));
                item.aliases.insert("hris".into(), aliases(&["81"]));
            }
            112 => item.enabled = false,
            12 => item.valid_from = Some(day(2025, 1, 1)),
            21 => {
                item.names.insert("de".into(), "Deutschland".into());
                item.aliases.insert("sap".into(), aliases(&["DE"]));
            }
            212 => item.valid_to = Some(day(2025, 12, 31)),
            _ => {}
        }
    }
    items
}

pub fn region_state() -> State {
    State::new(region_set(), region_items())
}

pub fn priority_set() -> RefSet {
    RefSet::new(
        RefSetId::from_u128(PRIORITY),
        "ticket_priority",
        "Priority",
        Structure::Flat,
        SetScope::Tenant,
    )
}

pub fn priority_items() -> Vec<RefItem> {
    let mut items = vec![
        item(31, "low", "Low", None),
        item(32, "normal", "Normal", None),
        item(33, "high", "High", None),
    ];
    for (index, item) in items.iter_mut().enumerate() {
        item.sort = i64::try_from(index).unwrap();
    }
    items
}

/// A small system set (as shipped with the product).
pub fn system_set() -> RefSet {
    RefSet::new(
        RefSetId::from_u128(SYSTEM),
        "return_reason",
        "Return reason",
        Structure::tree(),
        SetScope::System,
    )
}

pub fn system_items() -> Vec<RefItem> {
    let mut defect = item(41, "defect", "Defective", None);
    defect.aliases.insert("shop".into(), aliases(&["D"]));
    vec![
        defect,
        item(42, "wrong_item", "Wrong item", None),
        item(43, "changed_mind", "Changed mind", None),
    ]
}

pub fn system_state() -> State {
    State::new(system_set(), system_items())
}

pub fn scope(set: u128) -> RefScope {
    RefScope {
        set: RefSetId::from_u128(set),
        subtree: None,
        level: None,
        leaves_only: false,
    }
}

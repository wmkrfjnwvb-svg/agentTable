//! Built-in system sets (product design doc 07 §2.4). They are read-only for tenants, who
//! adapt them through an overlay: relabel, disable, add aliases, add their own items.
//!
//! | Code | Content |
//! |---|---|
//! | `iso_3166_1` | Countries (alpha-2 codes) |
//! | `iso_4217` | Currencies; each item's value is its number of minor units |
//! | `iso_639_1` | Languages (two-letter codes) |
//! | `iso_5218` | Sex: not known, male, female, not applicable (with en, de, ja, ko, hi labels) |
//! | `fiscal_period_type` | Month, quarter, half-year, year (with en, de, ja, ko labels) |
//!
//! Ids are derived from the codes ([`system_item_id`]), so they are the same on every
//! installation and in every release: data exported from one instance imports into another.
//!
//! Known limit: countries, currencies and languages are labelled with their codes. Display
//! names per locale are delivered as a data pack, not compiled in.

use crate::change::State;
use crate::model::{RefItem, RefSet, SetScope, Structure};
use mt_core::codes::{ISO_3166_ALPHA2, ISO_639_1};
use mt_core::decimal::Decimal;
use mt_core::id::{RefItemId, RefSetId};
use mt_core::money::ISO_4217;
use mt_core::value::Value;

/// A stable 80-bit number derived from text (two FNV-1a passes). The upper 48 bits, which
/// hold the creation time in generated ids, are zero: built-in ids can never collide with
/// generated ones.
#[must_use]
pub fn stable_number(text: &str) -> u128 {
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let pass = |basis: u64| {
        text.bytes().fold(basis, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(PRIME)
        })
    };
    let low = pass(0xcbf2_9ce4_8422_2325);
    let high = pass(0x84a5_1c0f_3e2b_9d67) & 0xFFFF;
    u128::from(high).checked_shl(64).unwrap_or(0) | u128::from(low)
}

/// The id of the system set with the given code.
#[must_use]
pub fn system_set_id(set_code: &str) -> RefSetId {
    RefSetId::from_u128(stable_number(&format!("set/{set_code}")))
}

/// The id of an item of a system set.
#[must_use]
pub fn system_item_id(set_code: &str, item_code: &str) -> RefItemId {
    RefItemId::from_u128(stable_number(&format!("item/{set_code}/{item_code}")))
}

fn system_set(code: &str, name: &str, description: &str) -> RefSet {
    let mut set = RefSet::new(
        system_set_id(code),
        code,
        name,
        Structure::Flat,
        SetScope::System,
    );
    set.description = Some(description.to_owned());
    set
}

fn item(set_code: &str, code: &str, name: &str, sort: usize) -> RefItem {
    let mut item = RefItem::new(system_item_id(set_code, code), code, name);
    item.sort = i64::try_from(sort).unwrap_or(i64::MAX);
    item
}

fn labelled(set_code: &str, sort: usize, code: &str, names: &[(&str, &str)]) -> RefItem {
    let english = names
        .iter()
        .find(|(locale, _)| *locale == "en")
        .map_or(code, |(_, name)| *name);
    let mut entry = item(set_code, code, english, sort);
    for (locale, name) in names {
        if *locale != "en" {
            entry.names.insert((*locale).to_owned(), (*name).to_owned());
        }
    }
    entry
}

/// Countries: ISO 3166-1 alpha-2.
#[must_use]
pub fn countries() -> State {
    const CODE: &str = "iso_3166_1";
    let items = ISO_3166_ALPHA2
        .iter()
        .enumerate()
        .map(|(index, code)| item(CODE, code, code, index))
        .collect();
    State::new(
        system_set(
            CODE,
            "Countries",
            "ISO 3166-1 countries and territories (alpha-2 codes).",
        ),
        items,
    )
}

/// Currencies: ISO 4217. The value of each item is its number of minor units (2 for USD,
/// 0 for JPY); currencies without minor units in the standard have no value.
#[must_use]
pub fn currencies() -> State {
    const CODE: &str = "iso_4217";
    let items = ISO_4217
        .iter()
        .enumerate()
        .map(|(index, (code, minor_units))| {
            let mut entry = item(CODE, code, code, index);
            entry.value = minor_units.map(|units| Value::Number(Decimal::from(units)));
            entry
        })
        .collect();
    State::new(
        system_set(
            CODE,
            "Currencies",
            "ISO 4217 currencies; the value is the number of minor units.",
        ),
        items,
    )
}

/// Languages: ISO 639-1.
#[must_use]
pub fn languages() -> State {
    const CODE: &str = "iso_639_1";
    let items = ISO_639_1
        .iter()
        .enumerate()
        .map(|(index, code)| item(CODE, code, code, index))
        .collect();
    State::new(
        system_set(CODE, "Languages", "ISO 639-1 languages (two-letter codes)."),
        items,
    )
}

/// Sex: ISO/IEC 5218. This is not gender identity, which a tenant models as its own set.
#[must_use]
pub fn sex() -> State {
    const CODE: &str = "iso_5218";
    let rows: [(&str, [(&str, &str); 5]); 4] = [
        (
            "0",
            [
                ("en", "Not known"),
                ("de", "Nicht bekannt"),
                ("ja", "不明"),
                ("ko", "알 수 없음"),
                ("hi", "ज्ञात नहीं"),
            ],
        ),
        (
            "1",
            [
                ("en", "Male"),
                ("de", "Männlich"),
                ("ja", "男性"),
                ("ko", "남성"),
                ("hi", "पुरुष"),
            ],
        ),
        (
            "2",
            [
                ("en", "Female"),
                ("de", "Weiblich"),
                ("ja", "女性"),
                ("ko", "여성"),
                ("hi", "महिला"),
            ],
        ),
        (
            "9",
            [
                ("en", "Not applicable"),
                ("de", "Nicht anwendbar"),
                ("ja", "適用不能"),
                ("ko", "해당 없음"),
                ("hi", "लागू नहीं"),
            ],
        ),
    ];
    let items = rows
        .iter()
        .enumerate()
        .map(|(index, (code, names))| labelled(CODE, index, code, names))
        .collect();
    State::new(
        system_set(
            CODE,
            "Sex",
            "ISO/IEC 5218 codes for the representation of human sexes.",
        ),
        items,
    )
}

/// Fiscal period types.
#[must_use]
pub fn fiscal_period_types() -> State {
    const CODE: &str = "fiscal_period_type";
    let rows: [(&str, [(&str, &str); 4]); 4] = [
        (
            "month",
            [("en", "Month"), ("de", "Monat"), ("ja", "月"), ("ko", "월")],
        ),
        (
            "quarter",
            [
                ("en", "Quarter"),
                ("de", "Quartal"),
                ("ja", "四半期"),
                ("ko", "분기"),
            ],
        ),
        (
            "half_year",
            [
                ("en", "Half-year"),
                ("de", "Halbjahr"),
                ("ja", "半期"),
                ("ko", "반기"),
            ],
        ),
        (
            "year",
            [("en", "Year"), ("de", "Jahr"), ("ja", "年"), ("ko", "연")],
        ),
    ];
    let items = rows
        .iter()
        .enumerate()
        .map(|(index, (code, names))| labelled(CODE, index, code, names))
        .collect();
    State::new(
        system_set(
            CODE,
            "Fiscal period types",
            "The lengths a fiscal period can have.",
        ),
        items,
    )
}

/// Every built-in system set.
#[must_use]
pub fn all() -> Vec<State> {
    vec![
        countries(),
        currencies(),
        languages(),
        sex(),
        fiscal_period_types(),
    ]
}

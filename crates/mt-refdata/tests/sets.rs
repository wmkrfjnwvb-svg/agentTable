//! Reference sets: definitions, the effective set, tree queries, selection rules, labels,
//! search, crosswalks, tenant overlays, the option resolver and the built-in sets.
//! Categories are marked in the test names (`std_`, `anomalous_`, `edge_`, `invalid_`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_core::field::{DataType, FieldDef, OptionsSource, RefScope};
use mt_core::id::{FieldId, RefSetId};
use mt_core::validate::{validate_cascade, validate_value, IssueCode, OptionResolver};
use mt_core::value::Value;
use mt_refdata::model::{
    check_item_code, check_set_code, check_system_name, MAX_ALIASES_PER_SYSTEM, MAX_ALIAS_SYSTEMS,
    MAX_ATTRIBUTES, MAX_CODE_CHARS, MAX_TEXT_CHARS, MAX_TREE_DEPTH,
};
use mt_refdata::{
    builtin, ChangePolicy, EffectiveSet, NoParameters, Origin, Overlay, RefData, RefDataError,
    RefItem, RefSet, Relabel, SetScope, Structure,
};

fn regions() -> EffectiveSet {
    EffectiveSet::build(&region_set(), &region_items(), None).unwrap()
}

fn ids(items: &[mt_core::id::RefItemId]) -> Vec<u128> {
    items.iter().map(|id| id.as_u128()).collect()
}

fn build_err(set: &RefSet, items: &[RefItem], overlay: Option<&Overlay>) -> RefDataError {
    EffectiveSet::build(set, items, overlay).unwrap_err()
}

// ---------------------------------------------------------------------------------------------
// Structure and tree queries
// ---------------------------------------------------------------------------------------------

#[test]
fn std_tree_structure_and_display_order() {
    let set = regions();
    assert_eq!(set.len(), 14);
    assert!(!set.is_empty());
    assert_eq!(set.set().code, "region");
    // Roots by sort position, then name: Americas (0), Asia (1), Europe (2).
    assert_eq!(ids(set.roots()), vec![3, 1, 2]);
    // Siblings with equal sort are ordered by name.
    assert_eq!(ids(set.children(rid(11))), vec![112, 111]);
    assert_eq!(ids(set.children(rid(111))), vec![1111, 1112]);
    assert!(set.children(rid(1111)).is_empty());
    assert!(set.children(rid(9999)).is_empty());

    assert_eq!(set.parent(rid(1111)), Some(rid(111)));
    assert_eq!(set.parent(rid(1)), None);
    assert_eq!(set.depth(rid(1)), Some(1));
    assert_eq!(set.depth(rid(1111)), Some(4));
    assert_eq!(set.depth(rid(9999)), None);
    assert!(set.is_leaf(rid(1111)) && set.is_leaf(rid(3)));
    assert!(!set.is_leaf(rid(11)) && !set.is_leaf(rid(9999)));

    assert_eq!(ids(&set.ancestors(rid(1111))), vec![111, 11, 1]);
    assert!(set.ancestors(rid(1)).is_empty());
    assert_eq!(ids(&set.path(rid(1111))), vec![1, 11, 111, 1111]);
    assert!(set.path(rid(9999)).is_empty());

    assert!(set.is_within(rid(1), rid(1111)));
    assert!(set.is_within(rid(11), rid(111)));
    assert!(
        !set.is_within(rid(1111), rid(1111)),
        "an item is not within itself"
    );
    assert!(!set.is_within(rid(2), rid(1111)));
    assert!(!set.is_within(rid(1111), rid(1)));
    assert!(!set.is_within(rid(1), rid(9999)));

    // Parents before children, siblings in display order.
    assert_eq!(
        ids(&set.descendants(rid(11))),
        vec![112, 1121, 111, 1111, 1112]
    );
    assert!(set.descendants(rid(3)).is_empty());
    assert_eq!(
        ids(&set.walk()),
        vec![3, 1, 11, 112, 1121, 111, 1111, 1112, 12, 121, 2, 21, 211, 212]
    );
    assert_eq!(set.ids().count(), 14);
}

#[test]
fn std_lookup_by_code_ignores_ascii_case() {
    let set = regions();
    assert_eq!(set.by_code("jp").unwrap().id, rid(11));
    assert_eq!(set.by_code("JP").unwrap().id, rid(11));
    assert_eq!(set.by_code("Jp-Tokyo").unwrap().id, rid(1111));
    assert!(set.by_code("xx").is_none());
    assert_eq!(set.get(rid(11)).unwrap().code, "jp");
    assert_eq!(set.origin(rid(11)), Some(Origin::Tenant));
    assert_eq!(set.origin(rid(9999)), None);
}

#[test]
fn std_flat_set() {
    let set = EffectiveSet::build(&priority_set(), &priority_items(), None).unwrap();
    assert_eq!(ids(set.roots()), vec![31, 32, 33]);
    assert!(set
        .ids()
        .all(|id| set.depth(id) == Some(1) && set.is_leaf(id)));
    assert_eq!(Structure::Flat.max_depth(), 1);
    assert_eq!(Structure::tree().max_depth(), MAX_TREE_DEPTH);
}

#[test]
fn edge_empty_set_and_single_item() {
    let empty = EffectiveSet::build(&region_set(), &[], None).unwrap();
    assert!(empty.is_empty());
    assert!(empty.walk().is_empty() && empty.roots().is_empty());
    assert!(empty.options(&scope(REGION), day(2026, 1, 1)).is_empty());
    assert!(empty.search("a", "en", 10).is_empty());
    assert_eq!(empty.systems().count(), 0);

    let one = EffectiveSet::build(&region_set(), &[item(1, "a", "A", None)], None).unwrap();
    assert_eq!(ids(&one.walk()), vec![1]);
    assert_eq!(one.path_label(rid(1), "en", " / "), Some("A".to_owned()));
}

#[test]
fn edge_tree_at_the_depth_limit() {
    let mut set = region_set();
    set.structure = Structure::Tree {
        max_depth: MAX_TREE_DEPTH,
    };
    let chain: Vec<RefItem> = (1..=u128::from(MAX_TREE_DEPTH))
        .map(|n| {
            item(
                n,
                &format!("c{n}"),
                &format!("Level {n}"),
                (n > 1).then(|| n - 1),
            )
        })
        .collect();
    let built = EffectiveSet::build(&set, &chain, None).unwrap();
    assert_eq!(
        built.depth(rid(u128::from(MAX_TREE_DEPTH))),
        Some(MAX_TREE_DEPTH)
    );
    assert_eq!(
        built.ancestors(rid(u128::from(MAX_TREE_DEPTH))).len(),
        usize::from(MAX_TREE_DEPTH) - 1
    );

    // One level more is refused, and the error names the level.
    let mut too_deep = chain;
    let n = u128::from(MAX_TREE_DEPTH) + 1;
    too_deep.push(item(n, "deep", "Too deep", Some(n - 1)));
    assert_eq!(
        build_err(&set, &too_deep, None),
        RefDataError::TooDeep {
            item: rid(n),
            depth: usize::from(MAX_TREE_DEPTH) + 1,
            max: MAX_TREE_DEPTH
        }
    );
}

#[test]
fn invalid_tree_shapes() {
    let set = region_set();
    // A parent that does not exist.
    assert_eq!(
        build_err(&set, &[item(1, "a", "A", Some(9))], None),
        RefDataError::UnknownParent {
            item: rid(1),
            parent: rid(9)
        }
    );
    // An item that is its own parent, and longer cycles.
    assert_eq!(
        build_err(&set, &[item(1, "a", "A", Some(1))], None),
        RefDataError::Cycle { item: rid(1) }
    );
    let two = [item(1, "a", "A", Some(2)), item(2, "b", "B", Some(1))];
    assert!(matches!(
        build_err(&set, &two, None),
        RefDataError::Cycle { .. }
    ));
    // A healthy branch hanging off a cycle is still reported as a cycle.
    let tail = [
        item(1, "a", "A", Some(2)),
        item(2, "b", "B", Some(3)),
        item(3, "c", "C", Some(2)),
    ];
    assert!(matches!(
        build_err(&set, &tail, None),
        RefDataError::Cycle { .. }
    ));
    // Deeper than the set allows (max_depth 4).
    let deep: Vec<RefItem> = (1..=5_u128)
        .map(|n| item(n, &format!("c{n}"), "X", (n > 1).then(|| n - 1)))
        .collect();
    assert_eq!(
        build_err(&set, &deep, None),
        RefDataError::TooDeep {
            item: rid(5),
            depth: 5,
            max: 4
        }
    );
    // A parent in a flat set.
    let flat = [
        item(31, "low", "Low", None),
        item(32, "normal", "Normal", Some(31)),
    ];
    assert_eq!(
        build_err(&priority_set(), &flat, None),
        RefDataError::FlatSetHasParent { item: rid(32) }
    );
}

#[test]
fn invalid_duplicates() {
    let set = region_set();
    let twice = [item(1, "a", "A", None), item(1, "b", "B", None)];
    assert_eq!(
        build_err(&set, &twice, None),
        RefDataError::DuplicateItem { id: rid(1) }
    );
    // Codes that differ only in case are the same code.
    let same_code = [item(1, "jp", "A", None), item(2, "JP", "B", None)];
    assert!(matches!(
        build_err(&set, &same_code, None),
        RefDataError::DuplicateCode { .. }
    ));
    // The same name twice is allowed (two "Springfield"s under different parents).
    let same_name = [
        item(1, "a", "Springfield", None),
        item(2, "b", "Springfield", None),
    ];
    EffectiveSet::build(&set, &same_name, None).unwrap();
}

// ---------------------------------------------------------------------------------------------
// Definitions
// ---------------------------------------------------------------------------------------------

#[test]
fn std_set_definition_with_custom_attributes() {
    let mut set = region_set();
    set.names.insert("de".into(), "Region (DE)".into());
    set.description = Some("Sales regions".into());
    set.owner = Some(person(1));
    set.change_policy = ChangePolicy::Approval;
    let mut timezone = FieldDef::new(FieldId::from_u128(1), "Time zone", DataType::Text);
    timezone.validation.required = true;
    let mut tax = FieldDef::new(FieldId::from_u128(2), "Tax rate", DataType::Percent);
    tax.validation.max = Some("1".parse().unwrap());
    set.item_schema = vec![timezone, tax];
    set.check().unwrap();
    assert_eq!(set.display_name("de-AT"), "Region (DE)");
    assert_eq!(set.display_name("ja"), "Region");
    assert_eq!(
        set.attribute(FieldId::from_u128(2)).unwrap().name,
        "Tax rate"
    );
    assert!(set.attribute(FieldId::from_u128(9)).is_none());
    let json = serde_json::to_string(&set).unwrap();
    assert_eq!(serde_json::from_str::<RefSet>(&json).unwrap(), set);

    // Items must satisfy the attribute definitions.
    let mut tokyo = item(1, "tokyo", "Tokyo", None);
    assert!(
        EffectiveSet::build(&set, std::slice::from_ref(&tokyo), None)
            .unwrap_err()
            .to_string()
            .contains("needs a value for `Time zone`")
    );
    tokyo
        .attributes
        .insert(FieldId::from_u128(1), Value::Text("Asia/Tokyo".into()));
    tokyo.attributes.insert(
        FieldId::from_u128(2),
        Value::Percent("0.1".parse().unwrap()),
    );
    tokyo.value = Some(Value::Number("3".parse().unwrap()));
    let built = EffectiveSet::build(&set, std::slice::from_ref(&tokyo), None).unwrap();
    assert_eq!(built.get(rid(1)).unwrap().attributes.len(), 2);
    let json = serde_json::to_string(&tokyo).unwrap();
    assert_eq!(serde_json::from_str::<RefItem>(&json).unwrap(), tokyo);

    // An item stored without the optional keys loads with the defaults (enabled, no parent).
    let minimal: RefItem =
        serde_json::from_str(r#"{"id":"rfi_00000000000000000000000009","code":"x","name":"X"}"#)
            .unwrap();
    assert!(minimal.enabled && minimal.parent.is_none() && minimal.sort == 0);
    assert_eq!(
        RefItem::new(rid(9), "x", "X").under(rid(1)).parent,
        Some(rid(1))
    );

    // Attribute definitions may refer to each other (checked against the schema).
    let mut ranged = set.clone();
    let mut low = FieldDef::new(FieldId::from_u128(3), "Low", DataType::Number);
    low.validation.cross_field = vec![mt_core::field::CrossFieldRule {
        comparison: mt_core::field::Comparison::Le,
        other: FieldId::from_u128(4),
    }];
    ranged.item_schema.push(low);
    assert!(
        matches!(ranged.check(), Err(RefDataError::Core(_))),
        "the other attribute does not exist"
    );
    ranged.item_schema.push(FieldDef::new(
        FieldId::from_u128(4),
        "High",
        DataType::Number,
    ));
    ranged.check().unwrap();

    tokyo.attributes.insert(
        FieldId::from_u128(2),
        Value::Percent("1.5".parse().unwrap()),
    );
    assert!(matches!(
        build_err(&set, std::slice::from_ref(&tokyo), None),
        RefDataError::Invalid {
            what: "custom attribute",
            ..
        }
    ));
    tokyo
        .attributes
        .insert(FieldId::from_u128(2), Value::Text("ten".into()));
    assert!(matches!(
        build_err(&set, std::slice::from_ref(&tokyo), None),
        RefDataError::Invalid {
            what: "custom attribute",
            ..
        }
    ));
    tokyo.attributes.remove(&FieldId::from_u128(2));
    tokyo
        .attributes
        .insert(FieldId::from_u128(77), Value::Bool(true));
    assert!(build_err(&set, &[tokyo], None)
        .to_string()
        .contains("undefined attribute"));
}

#[test]
fn invalid_set_definitions() {
    let invalid = |edit: &dyn Fn(&mut RefSet)| {
        let mut set = region_set();
        edit(&mut set);
        set.check().unwrap_err()
    };
    assert!(matches!(
        invalid(&|s| s.code = "Region".into()),
        RefDataError::Invalid {
            what: "set code",
            ..
        }
    ));
    assert!(matches!(
        invalid(&|s| s.code = String::new()),
        RefDataError::Invalid {
            what: "set code",
            ..
        }
    ));
    assert!(matches!(
        invalid(&|s| s.name = "  ".into()),
        RefDataError::Invalid {
            what: "set name",
            ..
        }
    ));
    assert!(matches!(
        invalid(&|s| {
            s.names.insert("german".into(), "x".into());
        }),
        RefDataError::Invalid {
            what: "set name",
            ..
        }
    ));
    assert!(matches!(
        invalid(&|s| s.description = Some("x".repeat(MAX_TEXT_CHARS * 4 + 1))),
        RefDataError::Invalid {
            what: "set description",
            ..
        }
    ));
    assert!(matches!(
        invalid(&|s| s.structure = Structure::Tree { max_depth: 0 }),
        RefDataError::Invalid {
            what: "tree depth",
            ..
        }
    ));
    assert!(matches!(
        invalid(&|s| s.structure = Structure::Tree {
            max_depth: MAX_TREE_DEPTH + 1
        }),
        RefDataError::Invalid {
            what: "tree depth",
            ..
        }
    ));

    let attribute =
        |id: u128, name: &str| FieldDef::new(FieldId::from_u128(id), name, DataType::Text);
    assert!(
        invalid(&|s| s.item_schema = vec![attribute(1, "A"), attribute(1, "B")])
            .to_string()
            .contains("defined twice")
    );
    assert!(
        invalid(&|s| s.item_schema = vec![attribute(1, "Zone"), attribute(2, "zone")])
            .to_string()
            .contains("used twice")
    );
    assert_eq!(
        invalid(&|s| s.item_schema = (0..=MAX_ATTRIBUTES)
            .map(|n| attribute(n as u128, &format!("A{n}")))
            .collect()),
        RefDataError::Limit("too many custom attributes")
    );
    // Attribute definitions go through the field framework's own checks.
    assert!(matches!(
        invalid(&|s| s.item_schema = vec![attribute(1, "")]),
        RefDataError::Core(_)
    ));
    let mut numbered = attribute(1, "No");
    numbered.format.numbering = Some(mt_core::numbering::NumberingPattern::new("A-{seq}").unwrap());
    assert!(invalid(&|s| s.item_schema = vec![numbered.clone()])
        .to_string()
        .contains("auto-numbering"));
    // A broken set definition is refused before any item is looked at.
    let mut broken = region_set();
    broken.code = "Bad Code".into();
    assert!(matches!(
        build_err(&broken, &[], None),
        RefDataError::Invalid {
            what: "set code",
            ..
        }
    ));
}

#[test]
fn invalid_codes_and_names() {
    for code in [
        "a",
        "A1",
        "9",
        "jp-tokyo",
        "x.y_z",
        &"c".repeat(MAX_CODE_CHARS),
    ] {
        check_item_code(code).unwrap();
    }
    for code in [
        "",
        "-a",
        ".a",
        "a b",
        "ä",
        "a/b",
        &"c".repeat(MAX_CODE_CHARS + 1),
    ] {
        assert!(check_item_code(code).is_err(), "{code}");
    }
    for code in ["a", "cost_center", "iso_3166_1"] {
        check_set_code(code).unwrap();
    }
    for code in [
        "",
        "A",
        "1a",
        "a-b",
        "a.b",
        "_a",
        &"c".repeat(MAX_CODE_CHARS + 1),
    ] {
        assert!(check_set_code(code).is_err(), "{code}");
    }
    for system in ["sap", "hris_a", "shop.eu", "x-1"] {
        check_system_name(system).unwrap();
    }
    for system in ["", "SAP", "a b", &"s".repeat(MAX_CODE_CHARS + 1)] {
        assert!(check_system_name(system).is_err(), "{system}");
    }

    let set = region_set();
    let bad = |edit: &dyn Fn(&mut RefItem)| {
        let mut entry = item(1, "a", "A", None);
        edit(&mut entry);
        entry.check(&set).unwrap_err()
    };
    assert!(matches!(
        bad(&|i| i.code = "a b".into()),
        RefDataError::Invalid {
            what: "item code",
            ..
        }
    ));
    assert!(matches!(
        bad(&|i| i.name = String::new()),
        RefDataError::Invalid {
            what: "item name",
            ..
        }
    ));
    assert!(matches!(
        bad(&|i| i.name = "x".repeat(MAX_TEXT_CHARS + 1)),
        RefDataError::Invalid {
            what: "item name",
            ..
        }
    ));
    assert!(matches!(
        bad(&|i| i.name = "a\nb".into()),
        RefDataError::Invalid {
            what: "item name",
            ..
        }
    ));
    assert!(matches!(
        bad(&|i| {
            i.names.insert("xx-yy".into(), "A".into());
        }),
        RefDataError::Invalid {
            what: "item name",
            ..
        }
    ));
    assert!(matches!(
        bad(&|i| {
            i.names.insert("de".into(), " ".into());
        }),
        RefDataError::Invalid {
            what: "item name",
            ..
        }
    ));
    assert!(matches!(
        bad(&|i| {
            i.valid_from = Some(day(2026, 1, 2));
            i.valid_to = Some(day(2026, 1, 1));
        }),
        RefDataError::Invalid {
            what: "validity period",
            ..
        }
    ));
    assert!(matches!(
        bad(&|i| i.value = Some(Value::Text("  ".into()))),
        RefDataError::Invalid {
            what: "item value",
            ..
        }
    ));
    // A one-day validity period is fine.
    let mut one_day = item(1, "a", "A", None);
    one_day.valid_from = Some(day(2026, 1, 1));
    one_day.valid_to = Some(day(2026, 1, 1));
    one_day.check(&set).unwrap();
    assert!(one_day.is_valid_on(day(2026, 1, 1)));
    assert!(!one_day.is_valid_on(day(2025, 12, 31)) && !one_day.is_valid_on(day(2026, 1, 2)));
}

// ---------------------------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------------------------

#[test]
fn std_selection_respects_status_validity_and_the_whole_branch() {
    let set = regions();
    let all = scope(REGION);
    let mid_2025 = day(2025, 6, 1);
    let ok = |id: u128, on| set.is_selectable(&all, rid(id), on);

    assert!(ok(1111, mid_2025));
    assert!(ok(11, mid_2025));
    // Disabled, and everything below a disabled item.
    assert!(!ok(112, mid_2025));
    assert!(!ok(1121, mid_2025), "Osaka is below the disabled Kansai");
    assert!(!set.is_active(rid(1121), mid_2025));
    // Not yet valid, and everything below it.
    assert!(!ok(12, day(2024, 12, 31)));
    assert!(
        !ok(121, day(2024, 12, 31)),
        "Seoul is below Korea, which starts in 2025"
    );
    assert!(ok(12, day(2025, 1, 1)) && ok(121, day(2025, 1, 1)));
    // No longer valid; the last valid day is included.
    assert!(ok(212, day(2025, 12, 31)));
    assert!(!ok(212, day(2026, 1, 1)));
    // Unknown items and other sets.
    assert!(!ok(9999, mid_2025));
    assert!(!set.is_selectable(&scope(PRIORITY), rid(11), mid_2025));
    assert!(!set.is_active(rid(9999), mid_2025));
}

#[test]
fn std_selection_respects_the_field_scope() {
    let set = regions();
    let on = day(2025, 6, 1);
    let within = |subtree: u128| RefScope {
        subtree: Some(rid(subtree)),
        ..scope(REGION)
    };
    // Subtree: strict descendants only.
    assert!(set.is_selectable(&within(11), rid(1111), on));
    assert!(!set.is_selectable(&within(11), rid(11), on));
    assert!(!set.is_selectable(&within(11), rid(121), on));
    assert_eq!(ids(&set.options(&within(11), on)), vec![111, 1111, 1112]);
    assert!(set.options(&within(9999), on).is_empty());
    // Level.
    let level = |n: u8| RefScope {
        level: Some(n),
        ..scope(REGION)
    };
    assert_eq!(ids(&set.options(&level(1), on)), vec![3, 1, 2]);
    assert_eq!(ids(&set.options(&level(2), on)), vec![11, 12, 21]);
    assert!(set.options(&level(9), on).is_empty());
    // Leaves only (structural leaves that are selectable).
    let leaves = RefScope {
        leaves_only: true,
        ..scope(REGION)
    };
    assert_eq!(
        ids(&set.options(&leaves, on)),
        vec![3, 1111, 1112, 121, 211, 212]
    );
    // Combined: leaves below Japan at level 4.
    let combined = RefScope {
        subtree: Some(rid(11)),
        level: Some(4),
        leaves_only: true,
        ..scope(REGION)
    };
    assert_eq!(ids(&set.options(&combined, on)), vec![1111, 1112]);
    // Everything selectable on that day, in display order.
    assert_eq!(
        ids(&set.options(&scope(REGION), on)),
        vec![3, 1, 11, 111, 1111, 1112, 12, 121, 2, 21, 211, 212]
    );
    assert_eq!(set.options(&scope(REGION), day(2024, 1, 1)).len(), 10);
}

// ---------------------------------------------------------------------------------------------
// Labels, search, crosswalks
// ---------------------------------------------------------------------------------------------

#[test]
fn std_labels_and_paths() {
    let set = regions();
    assert_eq!(set.label(rid(11), "en"), Some("Japan"));
    assert_eq!(set.label(rid(11), "ja"), Some("日本"));
    assert_eq!(set.label(rid(11), "ja-JP"), Some("日本"));
    assert_eq!(set.label(rid(21), "de-CH"), Some("Deutschland"));
    assert_eq!(set.label(rid(9999), "en"), None);
    assert_eq!(set.get(rid(11)).unwrap().display_name("ko"), "Japan");
    assert_eq!(
        set.path_label(rid(1111), "en", " / "),
        Some("Asia / Japan / Kanto / Tokyo".to_owned())
    );
    assert_eq!(
        set.path_label(rid(1111), "ja", " > "),
        Some("Asia > 日本 > Kanto > Tokyo".to_owned())
    );
    assert_eq!(set.path_label(rid(9999), "en", " / "), None);
}

#[test]
fn std_search_matches_labels_and_codes_case_insensitively() {
    let set = regions();
    assert_eq!(ids(&set.search("TOK", "en", 10)), vec![1111]);
    assert_eq!(
        ids(&set.search("jp-", "en", 10)),
        vec![112, 1121, 111, 1111, 1112]
    );
    assert_eq!(ids(&set.search("jp-", "en", 2)), vec![112, 1121]);
    assert_eq!(ids(&set.search("日本", "ja", 10)), vec![11]);
    assert_eq!(ids(&set.search("deutsch", "de", 10)), vec![21]);
    assert!(set.search("deutsch", "en", 10).is_empty());
    assert!(set.search("   ", "en", 10).is_empty());
    assert!(set.search("tok", "en", 0).is_empty());
    assert!(set.search("zzz", "en", 10).is_empty());
}

#[test]
fn std_crosswalk_both_directions() {
    let set = regions();
    // Inbound: what does the external system mean?
    assert_eq!(set.crosswalk("sap", "JP"), Some(rid(11)));
    assert_eq!(set.crosswalk("sap", "JPN"), Some(rid(11)));
    assert_eq!(set.crosswalk("sap", "DE"), Some(rid(21)));
    assert_eq!(set.crosswalk("hris", "81"), Some(rid(11)));
    // Exact match: no guessing.
    assert_eq!(set.crosswalk("sap", "jp"), None);
    assert_eq!(set.crosswalk("sap", "81"), None);
    assert_eq!(set.crosswalk("unknown", "JP"), None);
    // Outbound: what do we send? The first alias.
    assert_eq!(set.alias("sap", rid(11)), Some("JP"));
    assert_eq!(set.alias("hris", rid(11)), Some("81"));
    assert_eq!(set.alias("hris", rid(21)), None);
    assert_eq!(set.alias("sap", rid(9999)), None);
    assert_eq!(set.systems().collect::<Vec<_>>(), vec!["hris", "sap"]);
}

#[test]
fn invalid_aliases() {
    let set = region_set();
    // The same external value for two items is ambiguous.
    let mut a = item(1, "a", "A", None);
    let mut b = item(2, "b", "B", None);
    a.aliases.insert("sap".into(), aliases(&["X"]));
    b.aliases.insert("sap".into(), aliases(&["X"]));
    assert_eq!(
        build_err(&set, &[a.clone(), b.clone()], None),
        RefDataError::AliasConflict {
            system: "sap".into(),
            value: "X".into()
        }
    );
    // The same value in different systems is fine.
    b.aliases.clear();
    b.aliases.insert("hris".into(), aliases(&["X"]));
    EffectiveSet::build(&set, &[a, b], None).unwrap();

    let bad = |edit: &dyn Fn(&mut RefItem)| {
        let mut entry = item(1, "a", "A", None);
        edit(&mut entry);
        entry.check(&set).unwrap_err()
    };
    assert!(matches!(
        bad(&|i| {
            i.aliases.insert("SAP".into(), aliases(&["X"]));
        }),
        RefDataError::Invalid {
            what: "external system name",
            ..
        }
    ));
    assert!(matches!(
        bad(&|i| {
            i.aliases.insert("sap".into(), aliases(&[]));
        }),
        RefDataError::Invalid { what: "alias", .. }
    ));
    assert!(matches!(
        bad(&|i| {
            i.aliases.insert("sap".into(), aliases(&[""]));
        }),
        RefDataError::Invalid { what: "alias", .. }
    ));
    assert!(matches!(
        bad(&|i| {
            i.aliases.insert("sap".into(), aliases(&[" X"]));
        }),
        RefDataError::Invalid { what: "alias", .. }
    ));
    assert_eq!(
        bad(&|i| {
            i.aliases.insert(
                "sap".into(),
                (0..=MAX_ALIASES_PER_SYSTEM)
                    .map(|n| format!("v{n}"))
                    .collect(),
            );
        }),
        RefDataError::Limit("too many aliases in one external system")
    );
    assert_eq!(
        bad(&|i| {
            for n in 0..=MAX_ALIAS_SYSTEMS {
                i.aliases.insert(format!("s{n}"), aliases(&["X"]));
            }
        }),
        RefDataError::Limit("aliases for too many external systems")
    );
}

// ---------------------------------------------------------------------------------------------
// Tenant overlays on system sets
// ---------------------------------------------------------------------------------------------

fn overlay() -> Overlay {
    let mut overlay = Overlay::default();
    // The tenant adds its own reason, under a system item, and one more below that.
    overlay
        .added
        .push(item(51, "defect_packaging", "Damaged packaging", Some(41)));
    overlay
        .added
        .push(item(52, "defect_box", "Crushed box", Some(51)));
    // It calls "Changed mind" something friendlier, also in German.
    overlay.relabel.insert(
        rid(43),
        Relabel {
            name: Some("No longer needed".into()),
            names: [("de".to_owned(), "Nicht mehr benötigt".to_owned())].into(),
        },
    );
    // It never accepts "Wrong item" returns.
    overlay.disabled.insert(rid(42));
    // Its shop system uses other codes.
    overlay.aliases.insert(
        rid(41),
        [
            ("shop".to_owned(), aliases(&["DEF"])),
            ("erp".to_owned(), aliases(&["10"])),
        ]
        .into(),
    );
    overlay
}

#[test]
fn std_effective_set_is_system_plus_additions_minus_disabled_with_tenant_labels() {
    let plain = EffectiveSet::build(&system_set(), &system_items(), None).unwrap();
    assert_eq!(plain.len(), 3);
    assert!(plain
        .ids()
        .all(|id| plain.origin(id) == Some(Origin::System)));

    let set = EffectiveSet::build(&system_set(), &system_items(), Some(&overlay())).unwrap();
    assert_eq!(set.len(), 5);
    assert_eq!(set.origin(rid(41)), Some(Origin::System));
    assert_eq!(set.origin(rid(51)), Some(Origin::Tenant));
    assert_eq!(ids(&set.descendants(rid(41))), vec![51, 52]);
    assert_eq!(set.depth(rid(52)), Some(3));
    // Relabelled; the code is untouched.
    assert_eq!(set.label(rid(43), "en"), Some("No longer needed"));
    assert_eq!(set.label(rid(43), "de"), Some("Nicht mehr benötigt"));
    assert_eq!(set.get(rid(43)).unwrap().code, "changed_mind");
    // Disabled for this tenant only.
    let on = day(2026, 1, 1);
    assert!(!set.is_selectable(&scope(SYSTEM), rid(42), on));
    assert!(plain.is_selectable(&scope(SYSTEM), rid(42), on));
    // The item is still there for existing records.
    assert_eq!(set.label(rid(42), "en"), Some("Wrong item"));
    // Aliases: the system's and the tenant's together.
    assert_eq!(set.crosswalk("shop", "D"), Some(rid(41)));
    assert_eq!(set.crosswalk("shop", "DEF"), Some(rid(41)));
    assert_eq!(set.crosswalk("erp", "10"), Some(rid(41)));
    assert_eq!(set.alias("shop", rid(41)), Some("D"));
    // The overlay round-trips through storage.
    let json = serde_json::to_string(&overlay()).unwrap();
    assert_eq!(serde_json::from_str::<Overlay>(&json).unwrap(), overlay());
    assert!(Overlay::default().is_empty() && !overlay().is_empty());
    assert_eq!(serde_json::to_string(&Overlay::default()).unwrap(), "{}");
}

#[test]
fn invalid_overlays() {
    let err = |edit: &dyn Fn(&mut Overlay)| {
        let mut o = overlay();
        edit(&mut o);
        build_err(&system_set(), &system_items(), Some(&o))
    };
    // Only system sets take an overlay; an empty one is the same as none.
    assert_eq!(
        build_err(&region_set(), &region_items(), Some(&overlay())),
        RefDataError::OverlayNotAllowed
    );
    EffectiveSet::build(&region_set(), &region_items(), Some(&Overlay::default())).unwrap();
    // Unknown targets.
    assert_eq!(
        err(&|o| {
            o.disabled.insert(rid(99));
        }),
        RefDataError::UnknownItem { id: rid(99) }
    );
    assert_eq!(
        err(&|o| {
            o.relabel.insert(rid(99), Relabel::default());
        }),
        RefDataError::UnknownItem { id: rid(99) }
    );
    assert_eq!(
        err(&|o| {
            o.aliases.insert(rid(99), Default::default());
        }),
        RefDataError::UnknownItem { id: rid(99) }
    );
    // The overlay's switches are for system items; the tenant edits its own items directly.
    assert!(matches!(
        err(&|o| {
            o.disabled.insert(rid(51));
        }),
        RefDataError::Invalid {
            what: "overlay",
            ..
        }
    ));
    assert!(matches!(
        err(&|o| {
            o.relabel.insert(rid(51), Relabel::default());
        }),
        RefDataError::Invalid {
            what: "overlay",
            ..
        }
    ));
    // A tenant code may not collide with a system code (in any case).
    assert!(matches!(
        err(&|o| o.added.push(item(60, "DEFECT", "Mine", None))),
        RefDataError::DuplicateCode { .. }
    ));
    assert_eq!(
        err(&|o| o.added.push(item(41, "mine", "Mine", None))),
        RefDataError::DuplicateItem { id: rid(41) }
    );
    // Bad labels and aliases are refused.
    assert!(matches!(
        err(&|o| {
            o.relabel.insert(
                rid(41),
                Relabel {
                    name: Some(String::new()),
                    names: Default::default(),
                },
            );
        }),
        RefDataError::Invalid {
            what: "item name",
            ..
        }
    ));
    assert!(matches!(
        err(&|o| {
            o.relabel.insert(
                rid(41),
                Relabel {
                    name: None,
                    names: [("zz_zz".to_owned(), "x".to_owned())].into(),
                },
            );
        }),
        RefDataError::Invalid {
            what: "item name",
            ..
        }
    ));
    assert!(matches!(
        err(&|o| {
            o.aliases
                .insert(rid(42), [("SHOP".to_owned(), aliases(&["W"]))].into());
        }),
        RefDataError::Invalid {
            what: "external system name",
            ..
        }
    ));
    // A tenant alias may not shadow a system alias of another item.
    assert_eq!(
        err(&|o| {
            o.aliases
                .insert(rid(42), [("shop".to_owned(), aliases(&["D"]))].into());
        }),
        RefDataError::AliasConflict {
            system: "shop".into(),
            value: "D".into()
        }
    );
    // Tenant additions obey the tree rules too.
    assert_eq!(
        err(&|o| o.added.push(item(61, "orphan", "Orphan", Some(999)))),
        RefDataError::UnknownParent {
            item: rid(61),
            parent: rid(999)
        }
    );
}

// ---------------------------------------------------------------------------------------------
// The catalogue and the option resolver
// ---------------------------------------------------------------------------------------------

fn catalogue() -> RefData {
    let mut data = RefData::new();
    assert!(data.is_empty());
    data.insert(regions()).unwrap();
    data.insert(EffectiveSet::build(&priority_set(), &priority_items(), None).unwrap())
        .unwrap();
    data
}

#[test]
fn std_catalogue_lookup_and_replacement() {
    let mut data = catalogue();
    assert_eq!(data.len(), 2);
    assert_eq!(data.by_code("region").unwrap().len(), 14);
    assert!(data.by_code("nope").is_none());
    assert!(data.get(RefSetId::from_u128(REGION)).is_some());
    // Replacing a set (after a change) keeps the catalogue consistent.
    let smaller = EffectiveSet::build(&region_set(), &region_items()[..3], None).unwrap();
    data.insert(smaller).unwrap();
    assert_eq!(data.len(), 2);
    assert_eq!(data.by_code("region").unwrap().len(), 3);
    // Another set may not take a code that is in use.
    let mut clash = priority_set();
    clash.code = "region".into();
    let clash = EffectiveSet::build(&clash, &priority_items(), None).unwrap();
    assert!(matches!(
        data.insert(clash),
        Err(RefDataError::DuplicateCode { .. })
    ));
    // A set may be renamed in place (same id, new code).
    let mut renamed = region_set();
    renamed.code = "sales_region".into();
    data.insert(EffectiveSet::build(&renamed, &region_items(), None).unwrap())
        .unwrap();
    assert!(data.by_code("region").is_none());
    assert_eq!(data.by_code("sales_region").unwrap().len(), 14);
    assert!(data.remove(RefSetId::from_u128(REGION)).is_some());
    assert!(data.remove(RefSetId::from_u128(REGION)).is_none());
    assert!(data.by_code("sales_region").is_none());
    assert_eq!(data.len(), 1);
}

#[test]
fn std_resolver_drives_field_validation() {
    let data = catalogue();
    let on = day(2025, 6, 1);
    let resolver = data.resolver(on, &NoParameters);

    let mut region = FieldDef::new(FieldId::from_u128(1), "Region", DataType::Reference);
    region.options = OptionsSource::RefSet {
        set: RefSetId::from_u128(REGION),
        subtree: None,
        level: Some(2),
        leaves_only: false,
        cascade_from: None,
    };
    region.check().unwrap();
    let check = |field: &FieldDef, value: Value| {
        validate_value(field, &value, None, &resolver)
            .into_iter()
            .map(|i| i.code)
            .collect::<Vec<_>>()
    };
    assert!(check(&region, Value::Reference(rid(11))).is_empty());
    // Wrong level, disabled branch, unknown item: not allowed.
    assert_eq!(
        check(&region, Value::Reference(rid(111))),
        vec![IssueCode::OptionNotAllowed]
    );
    assert_eq!(
        check(&region, Value::Reference(rid(9999))),
        vec![IssueCode::OptionNotAllowed]
    );
    // An item of another set is not an item of this field.
    assert_eq!(
        check(&region, Value::Reference(rid(31))),
        vec![IssueCode::OptionNotAllowed]
    );
    // A value already on the record stays valid after its item was disabled.
    let mut any_level = region.clone();
    any_level.options = OptionsSource::of_set(RefSetId::from_u128(REGION));
    let kansai = Value::Reference(rid(112));
    assert_eq!(
        check(&any_level, kansai.clone()),
        vec![IssueCode::OptionNotAllowed]
    );
    assert!(validate_value(&any_level, &kansai, Some(&kansai), &resolver).is_empty());

    // The effective date matters: Korea starts in 2025.
    let earlier = data.resolver(day(2024, 6, 1), &NoParameters);
    assert!(earlier.is_selectable(&scope(REGION), rid(11)));
    assert!(!earlier.is_selectable(&scope(REGION), rid(12)));
    assert!(resolver.is_selectable(&scope(REGION), rid(12)));
    // A set that is not loaded offers nothing.
    assert!(!resolver.is_selectable(&scope(999), rid(11)));
    assert!(!resolver.is_within(RefSetId::from_u128(999), rid(1), rid(11)));
    assert!(!resolver.parameter_offers("any.key", rid(11)));

    // A cascading city field: must be below the chosen prefecture, even when disabled later.
    let mut city = FieldDef::new(FieldId::from_u128(2), "City", DataType::Reference);
    city.options = OptionsSource::RefSet {
        set: RefSetId::from_u128(REGION),
        subtree: None,
        level: None,
        leaves_only: true,
        cascade_from: Some(FieldId::from_u128(1)),
    };
    let kanto = Value::Reference(rid(111));
    let lookup = |f: FieldId| (f == FieldId::from_u128(1)).then_some(&kanto);
    assert!(validate_cascade(&city, &Value::Reference(rid(1111)), &lookup, &resolver).is_empty());
    let osaka = validate_cascade(&city, &Value::Reference(rid(1121)), &lookup, &resolver);
    assert_eq!(
        osaka.into_iter().map(|i| i.code).collect::<Vec<_>>(),
        vec![IssueCode::CascadeMismatch]
    );
}

// ---------------------------------------------------------------------------------------------
// Built-in system sets
// ---------------------------------------------------------------------------------------------

#[test]
fn std_builtin_sets_are_valid_and_stable() {
    let sets = builtin::all();
    assert_eq!(sets.len(), 5);
    let mut data = RefData::new();
    let mut seen = std::collections::BTreeSet::new();
    for state in &sets {
        assert_eq!(state.set.scope, SetScope::System);
        let effective = state.effective().unwrap();
        for id in effective.ids() {
            // Built-in ids carry no creation time, so they cannot collide with generated ids…
            assert_eq!(id.as_u128() >> 80, 0);
            // …and are unique across all built-in sets.
            assert!(seen.insert(id), "duplicate built-in id {id}");
        }
        assert!(seen.insert(mt_core::id::RefItemId::from_u128(state.set.id.as_u128())));
        data.insert(effective).unwrap();
    }
    assert_eq!(data.by_code("iso_3166_1").unwrap().len(), 249);
    assert_eq!(
        data.by_code("iso_639_1").unwrap().len(),
        mt_core::codes::ISO_639_1.len()
    );
    assert_eq!(
        data.by_code("iso_4217").unwrap().len(),
        mt_core::money::ISO_4217.len()
    );

    // Ids are derived from codes: the same everywhere, pinned here against accidental change.
    assert_eq!(
        builtin::system_item_id("iso_3166_1", "JP"),
        data.by_code("iso_3166_1")
            .unwrap()
            .by_code("jp")
            .unwrap()
            .id
    );
    assert_eq!(builtin::stable_number(""), 0x9d67_cbf2_9ce4_8422_2325);
    assert_eq!(
        builtin::stable_number("item/iso_3166_1/JP"),
        0xb3aa_f9b2_445b_8e32_45e8
    );
    assert_eq!(
        builtin::system_set_id("iso_4217"),
        builtin::currencies().set.id
    );

    // Currencies carry their minor units.
    let currencies = data.by_code("iso_4217").unwrap();
    assert_eq!(
        currencies.by_code("USD").unwrap().value,
        Some(Value::Number("2".parse().unwrap()))
    );
    assert_eq!(
        currencies.by_code("JPY").unwrap().value,
        Some(Value::Number("0".parse().unwrap()))
    );
    assert_eq!(
        currencies.by_code("KWD").unwrap().value,
        Some(Value::Number("3".parse().unwrap()))
    );

    // Sex and fiscal periods come with labels for the target markets.
    let sex = data.by_code("iso_5218").unwrap();
    let female = sex.by_code("2").unwrap().id;
    assert_eq!(sex.label(female, "en"), Some("Female"));
    assert_eq!(sex.label(female, "de"), Some("Weiblich"));
    assert_eq!(sex.label(female, "ja"), Some("女性"));
    assert_eq!(sex.label(female, "ko"), Some("여성"));
    assert_eq!(sex.label(female, "hi"), Some("महिला"));
    assert_eq!(ids(sex.roots()).len(), 4);
    assert_eq!(sex.get(sex.roots()[0]).unwrap().code, "0");
    let periods = data.by_code("fiscal_period_type").unwrap();
    assert_eq!(
        periods.label(periods.by_code("half_year").unwrap().id, "de"),
        Some("Halbjahr")
    );

    // A tenant adapts a built-in set through an overlay.
    let mut tenant = builtin::sex();
    tenant
        .overlay
        .disabled
        .insert(builtin::system_item_id("iso_5218", "9"));
    let adapted = tenant.effective().unwrap();
    let on = day(2026, 1, 1);
    let options = adapted.options(
        &RefScope {
            set: tenant.set.id,
            subtree: None,
            level: None,
            leaves_only: false,
        },
        on,
    );
    assert_eq!(options.len(), 3);
}

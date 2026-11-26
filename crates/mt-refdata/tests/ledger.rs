//! Changes, the versioned ledger, four-eyes approval, "as of" history and impact analysis.
//! Includes a model-based test of the ledger state machine.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_core::id::RefItemId;
use mt_core::value::Value;
use mt_refdata::ledger::{MAX_CHANGES_PER_COMMIT, MAX_PENDING_PROPOSALS};
use mt_refdata::{
    impact, Change, ChangePolicy, ItemPatch, Ledger, NoUsage, Proposal, ProposalStatus,
    RefDataError, Reference, ReferenceKind, Revision, State, Usage,
};
use proptest::prelude::*;
use std::collections::BTreeMap;

fn add(id: u128, code: &str, name: &str, parent: Option<u128>) -> Change {
    Change::Add {
        item: item(id, code, name, parent),
    }
}

fn rename(id: u128, name: &str) -> Change {
    Change::Update {
        id: rid(id),
        patch: ItemPatch {
            name: Some(name.into()),
            ..ItemPatch::default()
        },
    }
}

fn direct() -> Ledger {
    Ledger::new(region_state()).unwrap()
}

fn with_approval() -> Ledger {
    let mut state = region_state();
    state.set.change_policy = ChangePolicy::Approval;
    Ledger::new(state).unwrap()
}

// ---------------------------------------------------------------------------------------------
// Changes on a tenant set
// ---------------------------------------------------------------------------------------------

#[test]
fn std_every_kind_of_change_on_a_tenant_set() {
    let mut ledger = direct();
    assert_eq!(ledger.version(), 0);
    let alice = person(1);

    // Add.
    let v = ledger
        .commit(
            alice,
            at(10),
            0,
            vec![add(113, "jp-chubu", "Chubu", Some(11))],
        )
        .unwrap();
    assert_eq!(v, 1);
    assert_eq!(ledger.effective().parent(rid(113)), Some(rid(11)));

    // Update: name, localised name, value, sort, validity.
    let patch = ItemPatch {
        name: Some("Chūbu".into()),
        names: [("ja".to_owned(), "中部".to_owned())].into(),
        value: Some(Some(Value::Number("7".parse().unwrap()))),
        sort: Some(-5),
        valid_from: Some(Some(day(2026, 1, 1))),
        valid_to: Some(Some(day(2030, 12, 31))),
        attributes: BTreeMap::new(),
    };
    ledger
        .commit(
            alice,
            at(20),
            1,
            vec![Change::Update {
                id: rid(113),
                patch,
            }],
        )
        .unwrap();
    let chubu = ledger.effective().get(rid(113)).unwrap().clone();
    assert_eq!((chubu.name.as_str(), chubu.sort), ("Chūbu", -5));
    assert_eq!(chubu.names.get("ja").map(String::as_str), Some("中部"));
    assert_eq!(chubu.value, Some(Value::Number("7".parse().unwrap())));
    assert_eq!(
        (chubu.valid_from, chubu.valid_to),
        (Some(day(2026, 1, 1)), Some(day(2030, 12, 31)))
    );
    assert_eq!(chubu.code, "jp-chubu", "codes never change");
    // The new sort position puts it first among Japan's children.
    assert_eq!(ledger.effective().children(rid(11))[0], rid(113));

    // Clearing with explicit nulls.
    let clear = ItemPatch {
        value: Some(None),
        valid_from: Some(None),
        valid_to: Some(None),
        ..ItemPatch::default()
    };
    ledger
        .commit(
            alice,
            at(30),
            2,
            vec![Change::Update {
                id: rid(113),
                patch: clear,
            }],
        )
        .unwrap();
    let chubu = ledger.effective().get(rid(113)).unwrap();
    assert_eq!(
        (chubu.value.clone(), chubu.valid_from, chubu.valid_to),
        (None, None, None)
    );

    // Move. A branch that would end up too deep is refused as a whole…
    assert!(matches!(
        ledger.commit(
            alice,
            at(40),
            3,
            vec![Change::Move {
                id: rid(111),
                parent: Some(rid(113))
            }]
        ),
        Err(RefDataError::TooDeep {
            depth: 5,
            max: 4,
            ..
        })
    ));
    // …a leaf fits, and moves back again.
    ledger
        .commit(
            alice,
            at(40),
            3,
            vec![Change::Move {
                id: rid(1112),
                parent: Some(rid(113)),
            }],
        )
        .unwrap();
    assert_eq!(ledger.effective().depth(rid(1112)), Some(4));
    assert!(ledger.effective().is_within(rid(113), rid(1112)));
    assert!(!ledger.effective().is_within(rid(111), rid(1112)));
    ledger
        .commit(
            alice,
            at(41),
            4,
            vec![Change::Move {
                id: rid(1112),
                parent: Some(rid(111)),
            }],
        )
        .unwrap();

    // Disable and enable.
    ledger
        .commit(
            alice,
            at(50),
            5,
            vec![Change::SetEnabled {
                id: rid(113),
                enabled: false,
            }],
        )
        .unwrap();
    assert!(!ledger.effective().is_active(rid(113), day(2026, 6, 1)));
    ledger
        .commit(
            alice,
            at(51),
            6,
            vec![Change::SetEnabled {
                id: rid(113),
                enabled: true,
            }],
        )
        .unwrap();

    // Aliases: set, replace, remove.
    let alias = |values: &[&str]| Change::SetAliases {
        id: rid(113),
        system: "sap".into(),
        values: aliases(values),
    };
    ledger
        .commit(alice, at(60), 7, vec![alias(&["CHU"])])
        .unwrap();
    assert_eq!(ledger.effective().crosswalk("sap", "CHU"), Some(rid(113)));
    ledger
        .commit(alice, at(61), 8, vec![alias(&["CB"])])
        .unwrap();
    assert_eq!(ledger.effective().crosswalk("sap", "CHU"), None);
    assert_eq!(ledger.effective().crosswalk("sap", "CB"), Some(rid(113)));
    ledger.commit(alice, at(62), 9, vec![alias(&[])]).unwrap();
    assert_eq!(ledger.effective().alias("sap", rid(113)), None);

    // Remove.
    ledger
        .commit(alice, at(70), 10, vec![Change::Remove { id: rid(113) }])
        .unwrap();
    assert!(ledger.effective().get(rid(113)).is_none());
    assert_eq!(ledger.version(), 11);
    assert_eq!(ledger.revisions().len(), 11);
    assert!(ledger
        .revisions()
        .iter()
        .all(|r| r.by == alice && r.approved_by.is_none()));
    assert_eq!(Change::Remove { id: rid(113) }.item(), Some(rid(113)));
    assert_eq!(add(5, "x", "X", None).item(), Some(rid(5)));
    assert_eq!(rename(6, "X").item(), Some(rid(6)));
    assert_eq!(
        Change::SetPolicy {
            policy: ChangePolicy::Approval
        }
        .item(),
        None
    );
    assert_eq!(
        Change::Move {
            id: rid(7),
            parent: None
        }
        .item(),
        Some(rid(7))
    );
    assert_eq!(
        Change::SetEnabled {
            id: rid(8),
            enabled: true
        }
        .item(),
        Some(rid(8))
    );
    assert_eq!(alias(&["X"]).item(), Some(rid(113)));
    // A state refuses an empty change list on its own, too.
    assert_eq!(
        region_state().apply(&[]).unwrap_err(),
        RefDataError::EmptyChange
    );
}

#[test]
fn std_patches_distinguish_absent_null_and_value_in_json() {
    // Absent: leave as is. Null: clear. A value: set.
    let keep: ItemPatch = serde_json::from_str(r#"{"name":"Kanto"}"#).unwrap();
    assert_eq!(
        (keep.value.clone(), keep.valid_from, keep.valid_to),
        (None, None, None)
    );
    let clear: ItemPatch =
        serde_json::from_str(r#"{"value":null,"valid_from":null,"valid_to":null}"#).unwrap();
    assert_eq!(
        (clear.value.clone(), clear.valid_from, clear.valid_to),
        (Some(None), Some(None), Some(None))
    );
    let set: ItemPatch =
        serde_json::from_str(r#"{"value":{"type":"bool","value":true},"valid_to":"2026-12-31"}"#)
            .unwrap();
    assert_eq!(set.value, Some(Some(Value::Bool(true))));
    assert_eq!(set.valid_to, Some(Some(day(2026, 12, 31))));
    assert_eq!(set.valid_from, None);
    for patch in [keep, clear, set] {
        let json = serde_json::to_string(&patch).unwrap();
        assert_eq!(
            serde_json::from_str::<ItemPatch>(&json).unwrap(),
            patch,
            "{json}"
        );
    }
    assert_eq!(serde_json::to_string(&ItemPatch::default()).unwrap(), "{}");
    // Whole changes round-trip with a readable tag.
    let change = Change::Move {
        id: rid(111),
        parent: None,
    };
    let json = serde_json::to_string(&change).unwrap();
    assert!(json.starts_with(r#"{"op":"move""#), "{json}");
    assert_eq!(serde_json::from_str::<Change>(&json).unwrap(), change);
}

#[test]
fn std_attribute_patches_set_and_remove() {
    use mt_core::field::{DataType, FieldDef};
    use mt_core::id::FieldId;
    let mut state = region_state();
    state.set.item_schema = vec![FieldDef::new(
        FieldId::from_u128(1),
        "Time zone",
        DataType::Text,
    )];
    let mut ledger = Ledger::new(state).unwrap();
    let set = |value: Value| Change::Update {
        id: rid(11),
        patch: ItemPatch {
            attributes: [(FieldId::from_u128(1), value)].into(),
            ..ItemPatch::default()
        },
    };
    ledger
        .commit(
            person(1),
            at(1),
            0,
            vec![set(Value::Text("Asia/Tokyo".into()))],
        )
        .unwrap();
    assert_eq!(ledger.effective().get(rid(11)).unwrap().attributes.len(), 1);
    // An empty value removes the attribute.
    ledger
        .commit(person(1), at(2), 1, vec![set(Value::Null)])
        .unwrap();
    assert!(ledger
        .effective()
        .get(rid(11))
        .unwrap()
        .attributes
        .is_empty());
    // A value of the wrong type is refused and nothing changes.
    let err = ledger
        .commit(person(1), at(3), 2, vec![set(Value::Bool(true))])
        .unwrap_err();
    assert!(matches!(
        err,
        RefDataError::Invalid {
            what: "custom attribute",
            ..
        }
    ));
    assert_eq!(ledger.version(), 2);
}

#[test]
fn invalid_changes_are_refused_and_leave_everything_as_it_was() {
    let mut ledger = direct();
    let before = ledger.clone();
    let alice = person(1);
    let cases: Vec<(Vec<Change>, &str)> = vec![
        (vec![], "nothing to change"),
        (vec![add(11, "dup", "Dup", None)], "appears more than once"),
        (
            vec![add(900, "JP", "Dup code", None)],
            "used by more than one item",
        ),
        (vec![add(900, "x", "X", Some(999))], "names the parent"),
        (vec![add(900, "x", "X", Some(1111))], "would be at level 5"),
        (vec![add(900, "bad code", "X", None)], "invalid item code"),
        (vec![rename(999, "X")], "is not in the set"),
        (vec![rename(11, "  ")], "invalid item name"),
        (
            vec![Change::Update {
                id: rid(11),
                patch: ItemPatch::default(),
            }],
            "nothing to change",
        ),
        (
            vec![Change::Move {
                id: rid(999),
                parent: None,
            }],
            "is not in the set",
        ),
        (
            vec![Change::Move {
                id: rid(11),
                parent: Some(rid(11)),
            }],
            "its own ancestor",
        ),
        (
            vec![Change::Move {
                id: rid(11),
                parent: Some(rid(1111)),
            }],
            "its own ancestor",
        ),
        (
            vec![Change::Move {
                id: rid(1),
                parent: Some(rid(21)),
            }],
            "would be at level",
        ),
        (
            vec![Change::SetEnabled {
                id: rid(999),
                enabled: false,
            }],
            "is not in the set",
        ),
        (
            vec![Change::SetAliases {
                id: rid(999),
                system: "sap".into(),
                values: aliases(&["X"]),
            }],
            "is not in the set",
        ),
        (
            vec![Change::SetAliases {
                id: rid(21),
                system: "sap".into(),
                values: aliases(&["JP"]),
            }],
            "alias of more than one item",
        ),
        (
            vec![Change::SetAliases {
                id: rid(21),
                system: "SAP".into(),
                values: aliases(&["X"]),
            }],
            "invalid external system name",
        ),
        (vec![Change::Remove { id: rid(999) }], "is not in the set"),
        (vec![Change::Remove { id: rid(11) }], "has child items"),
        // A batch is all-or-nothing: the good first change does not survive the bad second.
        (
            vec![add(900, "ok", "Fine", None), rename(999, "X")],
            "is not in the set",
        ),
    ];
    for (changes, expected) in cases {
        let err = ledger.commit(alice, at(1), 0, changes.clone()).unwrap_err();
        assert!(err.to_string().contains(expected), "{changes:?}: {err}");
        assert_eq!(ledger, before, "{changes:?} changed something");
    }
    // Validity that ends before it starts.
    let backwards = ItemPatch {
        valid_from: Some(Some(day(2026, 2, 1))),
        valid_to: Some(Some(day(2026, 1, 1))),
        ..ItemPatch::default()
    };
    assert!(ledger
        .commit(
            alice,
            at(1),
            0,
            vec![Change::Update {
                id: rid(11),
                patch: backwards
            }]
        )
        .is_err());
    let too_many = vec![rename(11, "X"); MAX_CHANGES_PER_COMMIT + 1];
    assert_eq!(
        ledger.commit(alice, at(1), 0, too_many),
        Err(RefDataError::Limit("too many changes in one commit"))
    );
    assert_eq!(ledger, before);
}

#[test]
fn std_batches_apply_in_order_and_are_verified_once() {
    let mut ledger = direct();
    // Later changes may depend on earlier ones in the same commit.
    let changes = vec![
        add(4, "africa", "Africa", None),
        add(41, "ke", "Kenya", Some(4)),
        add(411, "ke-nbo", "Nairobi", Some(41)),
        rename(41, "Republic of Kenya"),
        Change::Move {
            id: rid(121),
            parent: Some(rid(41)),
        },
    ];
    ledger.commit(person(1), at(1), 0, changes).unwrap();
    assert_eq!(ledger.version(), 1, "one commit, one version");
    let set = ledger.effective();
    assert_eq!(
        set.path_label(rid(411), "en", " / "),
        Some("Africa / Republic of Kenya / Nairobi".to_owned())
    );
    assert_eq!(set.parent(rid(121)), Some(rid(41)));
    // An intermediate state may be invalid as long as the result is valid: swap two codes'
    // parents through a temporary cycle-free order.
    let swap = vec![
        Change::Move {
            id: rid(211),
            parent: Some(rid(1)),
        },
        Change::Move {
            id: rid(211),
            parent: Some(rid(21)),
        },
    ];
    ledger.commit(person(1), at(2), 1, swap).unwrap();
}

#[test]
fn concurrency_stale_writers_get_a_version_conflict() {
    let mut ledger = direct();
    // Two editors both looked at version 0.
    ledger
        .commit(person(1), at(1), 0, vec![rename(11, "Nippon")])
        .unwrap();
    let err = ledger
        .commit(person(2), at(2), 0, vec![rename(11, "Japan (JP)")])
        .unwrap_err();
    assert_eq!(
        err,
        RefDataError::VersionConflict {
            expected: 0,
            actual: 1
        }
    );
    assert_eq!(ledger.effective().label(rid(11), "en"), Some("Nippon"));
    // After looking again, the second editor succeeds.
    ledger
        .commit(person(2), at(3), 1, vec![rename(11, "Japan (JP)")])
        .unwrap();
    // A version from the future is a conflict too.
    assert!(matches!(
        ledger.commit(person(2), at(4), 9, vec![rename(11, "X")]),
        Err(RefDataError::VersionConflict {
            expected: 9,
            actual: 2
        })
    ));
}

// ---------------------------------------------------------------------------------------------
// Changes on a system set go to the tenant overlay
// ---------------------------------------------------------------------------------------------

#[test]
fn std_tenant_changes_to_a_system_set_become_an_overlay() {
    let mut ledger = Ledger::new(system_state()).unwrap();
    let alice = person(1);
    let changes = vec![
        add(51, "defect_packaging", "Damaged packaging", Some(41)),
        rename(43, "No longer needed"),
        Change::Update {
            id: rid(43),
            patch: ItemPatch {
                names: [("de".to_owned(), "Nicht mehr benötigt".to_owned())].into(),
                ..ItemPatch::default()
            },
        },
        Change::SetEnabled {
            id: rid(42),
            enabled: false,
        },
        Change::SetAliases {
            id: rid(41),
            system: "erp".into(),
            values: aliases(&["10"]),
        },
        Change::SetAliases {
            id: rid(51),
            system: "erp".into(),
            values: aliases(&["11"]),
        },
    ];
    ledger.commit(alice, at(1), 0, changes).unwrap();
    let state = ledger.state();
    // The system items are untouched; everything is in the overlay.
    assert_eq!(state.items, system_items());
    assert_eq!(state.overlay.added.len(), 1);
    assert_eq!(
        state.overlay.relabel[&rid(43)].name.as_deref(),
        Some("No longer needed")
    );
    assert!(state.overlay.disabled.contains(&rid(42)));
    assert_eq!(state.overlay.aliases[&rid(41)]["erp"], aliases(&["10"]));
    let set = ledger.effective();
    assert_eq!(set.label(rid(43), "de"), Some("Nicht mehr benötigt"));
    assert_eq!(set.crosswalk("erp", "10"), Some(rid(41)));
    assert_eq!(set.crosswalk("erp", "11"), Some(rid(51)));
    assert_eq!(
        set.crosswalk("shop", "D"),
        Some(rid(41)),
        "the system alias is still there"
    );
    assert!(!set.is_active(rid(42), day(2026, 1, 1)));

    // Undo the switches: the overlay shrinks again.
    ledger
        .commit(
            alice,
            at(2),
            1,
            vec![
                Change::SetEnabled {
                    id: rid(42),
                    enabled: true,
                },
                Change::SetAliases {
                    id: rid(41),
                    system: "erp".into(),
                    values: aliases(&[]),
                },
            ],
        )
        .unwrap();
    assert!(ledger.state().overlay.disabled.is_empty());
    assert!(ledger.state().overlay.aliases.is_empty());

    // The tenant's own item can be changed freely and removed.
    ledger
        .commit(
            alice,
            at(3),
            2,
            vec![
                Change::Update {
                    id: rid(51),
                    patch: ItemPatch {
                        sort: Some(3),
                        valid_to: Some(Some(day(2030, 1, 1))),
                        ..ItemPatch::default()
                    },
                },
                Change::Move {
                    id: rid(51),
                    parent: None,
                },
                Change::SetEnabled {
                    id: rid(51),
                    enabled: false,
                },
            ],
        )
        .unwrap();
    assert_eq!(ledger.effective().depth(rid(51)), Some(1));
    ledger
        .commit(alice, at(4), 3, vec![Change::Remove { id: rid(51) }])
        .unwrap();
    assert!(ledger.state().overlay.added.is_empty());
    let json = serde_json::to_string(ledger.state()).unwrap();
    assert_eq!(
        &serde_json::from_str::<State>(&json).unwrap(),
        ledger.state()
    );
}

#[test]
fn invalid_tenants_cannot_change_what_belongs_to_the_system() {
    let mut ledger = Ledger::new(system_state()).unwrap();
    let before = ledger.clone();
    let patch = |patch: ItemPatch| Change::Update { id: rid(41), patch };
    let cases = vec![
        patch(ItemPatch {
            sort: Some(1),
            ..ItemPatch::default()
        }),
        patch(ItemPatch {
            value: Some(Some(Value::Bool(true))),
            ..ItemPatch::default()
        }),
        patch(ItemPatch {
            valid_from: Some(Some(day(2026, 1, 1))),
            ..ItemPatch::default()
        }),
        patch(ItemPatch {
            valid_to: Some(None),
            ..ItemPatch::default()
        }),
        patch(ItemPatch {
            attributes: [(mt_core::id::FieldId::from_u128(1), Value::Bool(true))].into(),
            ..ItemPatch::default()
        }),
        Change::Move {
            id: rid(41),
            parent: Some(rid(42)),
        },
        Change::Remove { id: rid(41) },
    ];
    for change in cases {
        let err = ledger
            .commit(person(1), at(1), 0, vec![change.clone()])
            .unwrap_err();
        assert!(
            matches!(err, RefDataError::SystemItem { .. }),
            "{change:?}: {err}"
        );
        assert!(err.to_string().contains("system item"));
        assert_eq!(ledger, before);
    }
    // A system item with tenant children cannot lose them by accident either.
    ledger
        .commit(
            person(1),
            at(1),
            0,
            vec![
                add(51, "mine", "Mine", Some(41)),
                add(52, "mine2", "Mine 2", Some(51)),
            ],
        )
        .unwrap();
    assert_eq!(
        ledger.commit(person(1), at(2), 1, vec![Change::Remove { id: rid(51) }]),
        Err(RefDataError::HasChildren { id: rid(51) })
    );
}

// ---------------------------------------------------------------------------------------------
// Four-eyes approval
// ---------------------------------------------------------------------------------------------

#[test]
fn std_proposals_need_a_second_person() {
    let mut ledger = with_approval();
    let (alice, bob) = (person(1), person(2));
    // Direct commits are refused under the approval policy.
    assert_eq!(
        ledger.commit(alice, at(1), 0, vec![rename(11, "Nippon")]),
        Err(RefDataError::ApprovalRequired)
    );

    let id = ledger
        .propose(
            alice,
            at(10),
            vec![rename(11, "Nippon")],
            Some("Customer request".into()),
        )
        .unwrap();
    assert_eq!(id, 1);
    assert_eq!(ledger.pending().count(), 1);
    assert_eq!(ledger.version(), 0, "a proposal changes nothing yet");
    assert_eq!(ledger.effective().label(rid(11), "en"), Some("Japan"));

    // The author can neither approve nor reject their own proposal.
    assert_eq!(
        ledger.approve(id, alice, at(11)),
        Err(RefDataError::SelfApproval)
    );
    assert_eq!(
        ledger.reject(id, alice, at(11), "changed my mind"),
        Err(RefDataError::SelfApproval)
    );
    assert_eq!(ledger.pending().count(), 1);

    let version = ledger.approve(id, bob, at(20)).unwrap();
    assert_eq!(version, 1);
    assert_eq!(ledger.effective().label(rid(11), "en"), Some("Nippon"));
    let revision = &ledger.revisions()[0];
    assert_eq!(
        (revision.by, revision.approved_by, revision.at),
        (alice, Some(bob), at(20))
    );
    assert_eq!(
        ledger.proposals()[0].status,
        ProposalStatus::Approved {
            by: bob,
            at: at(20),
            version: 1
        }
    );
    assert_eq!(ledger.pending().count(), 0);
    // A decided proposal cannot be decided again.
    assert_eq!(
        ledger.approve(id, bob, at(21)),
        Err(RefDataError::NotPending { id })
    );
    assert_eq!(
        ledger.reject(id, bob, at(21), "late"),
        Err(RefDataError::NotPending { id })
    );
    assert_eq!(
        ledger.withdraw(id, alice, at(21)),
        Err(RefDataError::NotPending { id })
    );
    assert_eq!(
        ledger.approve(99, bob, at(21)),
        Err(RefDataError::NotPending { id: 99 })
    );
}

#[test]
fn std_reject_and_withdraw() {
    let mut ledger = with_approval();
    let (alice, bob, carol) = (person(1), person(2), person(3));
    let first = ledger
        .propose(alice, at(1), vec![rename(11, "A")], None)
        .unwrap();
    let second = ledger
        .propose(alice, at(2), vec![rename(11, "B")], None)
        .unwrap();

    assert!(matches!(
        ledger.reject(first, bob, at(3), "  "),
        Err(RefDataError::Invalid {
            what: "rejection reason",
            ..
        })
    ));
    ledger
        .reject(first, bob, at(3), "Not our naming convention")
        .unwrap();
    assert_eq!(
        ledger.proposals()[0].status,
        ProposalStatus::Rejected {
            by: bob,
            at: at(3),
            reason: "Not our naming convention".into()
        }
    );
    // Only the author withdraws.
    assert_eq!(
        ledger.withdraw(second, carol, at(4)),
        Err(RefDataError::NotProposer)
    );
    ledger.withdraw(second, alice, at(5)).unwrap();
    assert_eq!(
        ledger.proposals()[1].status,
        ProposalStatus::Withdrawn { at: at(5) }
    );
    assert_eq!(ledger.version(), 0);
    assert_eq!(ledger.pending().count(), 0);
    let json = serde_json::to_string(&ledger.proposals()[0]).unwrap();
    assert_eq!(
        serde_json::from_str::<Proposal>(&json).unwrap(),
        ledger.proposals()[0]
    );
}

#[test]
fn anomalous_proposal_overtaken_by_another_change() {
    let mut ledger = with_approval();
    let (alice, bob, carol) = (person(1), person(2), person(3));
    // Alice proposes to remove Seoul; Carol proposes to add a district below Seoul.
    let remove = ledger
        .propose(alice, at(1), vec![Change::Remove { id: rid(121) }], None)
        .unwrap();
    let extend = ledger
        .propose(
            carol,
            at(2),
            vec![add(1211, "kr-gangnam", "Gangnam", Some(121))],
            None,
        )
        .unwrap();
    // Carol's is approved first.
    ledger.approve(extend, bob, at(3)).unwrap();
    // Alice's no longer applies: Seoul has a child now. The approval fails cleanly and the
    // proposal stays pending, so that it can be withdrawn or rejected.
    assert_eq!(
        ledger.approve(remove, bob, at(4)),
        Err(RefDataError::HasChildren { id: rid(121) })
    );
    assert_eq!(ledger.version(), 1);
    assert_eq!(ledger.proposals()[0].status, ProposalStatus::Pending);
    ledger.withdraw(remove, alice, at(5)).unwrap();

    // A proposal that still applies to the newer state is approved on top of it.
    let rename_japan = ledger
        .propose(alice, at(6), vec![rename(11, "Nippon")], None)
        .unwrap();
    let other = ledger
        .propose(carol, at(7), vec![rename(21, "Deutschland")], None)
        .unwrap();
    ledger.approve(other, bob, at(8)).unwrap();
    let version = ledger.approve(rename_japan, bob, at(9)).unwrap();
    assert_eq!(version, 3);
    assert_eq!(ledger.proposals()[2].base_version, 1);
    assert_eq!(ledger.effective().label(rid(11), "en"), Some("Nippon"));
    assert_eq!(ledger.effective().label(rid(21), "en"), Some("Deutschland"));
}

#[test]
fn invalid_proposals() {
    let mut ledger = with_approval();
    let alice = person(1);
    assert_eq!(
        ledger.propose(alice, at(1), vec![], None),
        Err(RefDataError::EmptyChange)
    );
    // A proposal that is wrong today is refused today.
    assert_eq!(
        ledger.propose(alice, at(1), vec![rename(999, "X")], None),
        Err(RefDataError::UnknownItem { id: rid(999) })
    );
    assert!(matches!(
        ledger.propose(alice, at(1), vec![rename(11, "X")], Some("\n".into())),
        Err(RefDataError::Invalid {
            what: "proposal note",
            ..
        })
    ));
    assert_eq!(ledger.proposals().len(), 0);
    for n in 0..MAX_PENDING_PROPOSALS {
        ledger
            .propose(alice, at(1), vec![rename(11, &format!("N{n}"))], None)
            .unwrap();
    }
    assert_eq!(
        ledger.propose(alice, at(1), vec![rename(11, "One more")], None),
        Err(RefDataError::Limit("too many pending proposals"))
    );
    // Deciding one makes room again.
    ledger.withdraw(1, alice, at(2)).unwrap();
    ledger
        .propose(alice, at(3), vec![rename(11, "One more")], None)
        .unwrap();
    // Proposals are also available under the direct policy (optional review).
    let mut open = direct();
    let id = open
        .propose(alice, at(1), vec![rename(11, "X")], None)
        .unwrap();
    open.approve(id, person(2), at(2)).unwrap();
    assert_eq!(open.version(), 1);
}

// ---------------------------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------------------------

#[test]
fn std_as_of_browsing_and_restore() {
    let mut ledger = direct();
    let alice = person(1);
    ledger
        .commit(alice, at(100), 0, vec![rename(11, "Nippon")])
        .unwrap();
    ledger
        .commit(alice, at(200), 1, vec![add(4, "africa", "Africa", None)])
        .unwrap();
    ledger
        .commit(
            alice,
            at(300),
            2,
            vec![Change::SetEnabled {
                id: rid(2),
                enabled: false,
            }],
        )
        .unwrap();

    assert_eq!(&ledger.as_of(0).unwrap(), ledger.genesis());
    assert_eq!(ledger.as_of(0).unwrap(), region_state());
    let v1 = ledger.as_of(1).unwrap().effective().unwrap();
    assert_eq!(v1.label(rid(11), "en"), Some("Nippon"));
    assert!(v1.get(rid(4)).is_none());
    let v2 = ledger.as_of(2).unwrap().effective().unwrap();
    assert!(v2.get(rid(4)).is_some() && v2.get(rid(2)).unwrap().enabled);
    assert_eq!(&ledger.as_of(3).unwrap(), ledger.state());
    assert_eq!(
        ledger.as_of(4),
        Err(RefDataError::UnknownVersion { version: 4 })
    );

    // Which version was current at a moment?
    assert_eq!(ledger.version_at(at(99)), 0);
    assert_eq!(ledger.version_at(at(100)), 1);
    assert_eq!(ledger.version_at(at(250)), 2);
    assert_eq!(ledger.version_at(at(9_999)), 3);

    // Storage keeps the starting state, the revisions and the proposals; loading replays
    // and verifies them.
    let revisions: Vec<Revision> =
        serde_json::from_str(&serde_json::to_string(ledger.revisions()).unwrap()).unwrap();
    let restored = Ledger::restore(
        ledger.genesis().clone(),
        revisions.clone(),
        ledger.proposals().to_vec(),
    )
    .unwrap();
    assert_eq!(restored, ledger);

    // Corrupted history does not load.
    let mut gap = revisions.clone();
    gap.remove(1);
    assert_eq!(
        Ledger::restore(region_state(), gap, vec![]).unwrap_err(),
        RefDataError::UnknownVersion { version: 3 }
    );
    let mut wrong = revisions.clone();
    wrong[0].changes = vec![rename(999, "X")];
    assert_eq!(
        Ledger::restore(region_state(), wrong, vec![]).unwrap_err(),
        RefDataError::UnknownItem { id: rid(999) }
    );
    let mut empty = revisions.clone();
    empty[2].changes.clear();
    assert_eq!(
        Ledger::restore(region_state(), empty, vec![]).unwrap_err(),
        RefDataError::EmptyChange
    );
    // A starting state that is not a valid set does not load either.
    let mut broken = region_state();
    broken.items.push(item(11, "again", "Again", None));
    assert!(matches!(
        Ledger::new(broken),
        Err(RefDataError::DuplicateItem { .. })
    ));
}

#[test]
fn invalid_restored_proposals_must_fit_the_history() {
    let proposal = |id: u64, base: u64, status: ProposalStatus| Proposal {
        id,
        base_version: base,
        by: person(1),
        at: at(1),
        changes: vec![rename(11, "X")],
        note: None,
        status,
    };
    let restore = |proposals: Vec<Proposal>| Ledger::restore(region_state(), vec![], proposals);
    restore(vec![proposal(1, 0, ProposalStatus::Pending)]).unwrap();
    // Numbers must be 1, 2, 3, …
    assert!(restore(vec![proposal(2, 0, ProposalStatus::Pending)]).is_err());
    // A proposal cannot be based on, or approved into, a version that does not exist.
    assert!(restore(vec![proposal(1, 5, ProposalStatus::Pending)]).is_err());
    assert!(restore(vec![proposal(
        1,
        0,
        ProposalStatus::Approved {
            by: person(2),
            at: at(2),
            version: 1
        }
    )])
    .is_err());
    assert!(restore(vec![proposal(
        1,
        0,
        ProposalStatus::Approved {
            by: person(2),
            at: at(2),
            version: 0
        }
    )])
    .is_err());
}

// ---------------------------------------------------------------------------------------------
// Impact analysis
// ---------------------------------------------------------------------------------------------

struct Counts(BTreeMap<u128, u64>, BTreeMap<u128, Vec<Reference>>);

impl Usage for Counts {
    fn records(&self, item: RefItemId) -> u64 {
        self.0.get(&item.as_u128()).copied().unwrap_or(0)
    }
    fn references(&self, item: RefItemId) -> Vec<Reference> {
        self.1.get(&item.as_u128()).cloned().unwrap_or_default()
    }
}

#[test]
fn std_impact_of_each_kind_of_change() {
    let set = region_state().effective().unwrap();
    let filter = Reference {
        kind: ReferenceKind::Filter,
        name: "Open deals in Kanto".into(),
    };
    let mapping = Reference {
        kind: ReferenceKind::Mapping,
        name: "SAP customers".into(),
    };
    let usage = Counts(
        [(11, 5), (111, 40), (1111, 300), (212, 0)].into(),
        [(111, vec![filter.clone()]), (11, vec![mapping.clone()])].into(),
    );
    let touched = |change: &Change| {
        impact(&set, change, &usage)
            .items
            .iter()
            .map(|i| i.item.as_u128())
            .collect::<Vec<_>>()
    };

    // Nothing in use is touched by adding or enabling.
    assert!(impact(&set, &add(900, "x", "X", None), &usage).is_harmless());
    assert!(impact(
        &set,
        &Change::SetEnabled {
            id: rid(11),
            enabled: true
        },
        &usage
    )
    .items
    .is_empty());

    // Disabling or moving touches the whole branch.
    let disable = Change::SetEnabled {
        id: rid(11),
        enabled: false,
    };
    assert_eq!(touched(&disable), vec![11, 112, 1121, 111, 1111, 1112]);
    let result = impact(&set, &disable, &usage);
    assert_eq!(result.records(), 345);
    assert_eq!(result.reference_count(), 2);
    assert!(!result.is_harmless());
    assert_eq!(
        result.blocked, None,
        "disabling is never blocked: records keep their value"
    );
    assert_eq!(
        touched(&Change::Move {
            id: rid(111),
            parent: Some(rid(12))
        }),
        vec![111, 1111, 1112]
    );

    // A rename touches the item; a validity change touches the branch.
    assert_eq!(touched(&rename(111, "Kantō")), vec![111]);
    let narrow = Change::Update {
        id: rid(111),
        patch: ItemPatch {
            valid_to: Some(Some(day(2026, 1, 1))),
            ..ItemPatch::default()
        },
    };
    assert_eq!(touched(&narrow), vec![111, 1111, 1112]);
    // Aliases matter to integrations.
    let alias = Change::SetAliases {
        id: rid(11),
        system: "sap".into(),
        values: aliases(&["J"]),
    };
    assert_eq!(
        impact(&set, &alias, &usage).items[0].references,
        vec![mapping]
    );

    // Removing something that is still in use is blocked; unused items can go.
    let blocked = impact(&set, &Change::Remove { id: rid(1111) }, &usage);
    assert!(blocked
        .blocked
        .as_deref()
        .unwrap()
        .contains("300 record(s)"));
    let free = impact(&set, &Change::Remove { id: rid(212) }, &usage);
    assert!(free.is_harmless() && free.blocked.is_none());
    // An unknown item touches nothing; a source without data sees no use anywhere.
    assert!(impact(&set, &Change::Remove { id: rid(999) }, &usage)
        .items
        .is_empty());
    assert!(impact(&set, &disable, &NoUsage).is_harmless());
    let json = serde_json::to_string(&result).unwrap();
    assert_eq!(
        serde_json::from_str::<mt_refdata::Impact>(&json).unwrap(),
        result
    );
}

// ---------------------------------------------------------------------------------------------
// Model-based test of the ledger state machine
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Op {
    Commit(u8, Vec<Change>, bool),
    Propose(u8, Vec<Change>),
    Approve(u8, u8),
    Reject(u8, u8),
    Withdraw(u8, u8),
}

fn arb_change() -> impl Strategy<Value = Change> {
    let any_id = prop_oneof![
        proptest::sample::select(vec![
            1_u128, 2, 3, 11, 12, 21, 111, 112, 121, 211, 212, 1111, 1112, 1121
        ]),
        900_u128..906,
    ];
    let parent = proptest::option::of(proptest::sample::select(vec![
        1_u128, 2, 11, 111, 1111, 900, 901,
    ]));
    prop_oneof![
        3 => (900_u128..906, parent.clone()).prop_map(|(id, parent)| add(id, &format!("n{id}"), &format!("New {id}"), parent)),
        3 => (any_id.clone(), "[A-Za-z]{1,6}").prop_map(|(id, name)| rename(id, &name)),
        2 => (any_id.clone(), parent).prop_map(|(id, parent)| Change::Move { id: rid(id), parent: parent.map(rid) }),
        2 => (any_id.clone(), any::<bool>()).prop_map(|(id, enabled)| Change::SetEnabled { id: rid(id), enabled }),
        2 => (any_id.clone(), proptest::collection::btree_set("[A-C]", 0..2)).prop_map(|(id, values)| Change::SetAliases { id: rid(id), system: "sap".into(), values }),
        1 => any_id.prop_map(|id| Change::Remove { id: rid(id) }),
        1 => any::<bool>().prop_map(|approval| Change::SetPolicy { policy: if approval { ChangePolicy::Approval } else { ChangePolicy::Direct } }),
    ]
}

fn arb_op() -> impl Strategy<Value = Op> {
    let changes = || proptest::collection::vec(arb_change(), 1..4);
    prop_oneof![
        4 => (1_u8..4, changes(), any::<bool>()).prop_map(|(who, c, stale)| Op::Commit(who, c, stale)),
        3 => (1_u8..4, changes()).prop_map(|(who, c)| Op::Propose(who, c)),
        3 => (1_u8..12, 1_u8..4).prop_map(|(id, who)| Op::Approve(id, who)),
        1 => (1_u8..12, 1_u8..4).prop_map(|(id, who)| Op::Reject(id, who)),
        1 => (1_u8..12, 1_u8..4).prop_map(|(id, who)| Op::Withdraw(id, who)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Whatever people do, in any order:
    /// * the current state is always a valid set and equals a replay of the history;
    /// * a failed operation changes nothing;
    /// * versions go up by exactly one per accepted change;
    /// * nobody ever approves their own proposal;
    /// * every past version can still be read, and is what it was at the time.
    #[test]
    fn model_ledger_history_is_consistent(ops in proptest::collection::vec(arb_op(), 1..40), approval in any::<bool>()) {
        let mut state = region_state();
        if approval {
            state.set.change_policy = ChangePolicy::Approval;
        }
        let mut ledger = Ledger::new(state).unwrap();
        let mut snapshots = vec![ledger.state().clone()];
        let mut clock = 0_i64;
        for op in ops {
            clock += 1;
            let before = ledger.clone();
            let result: Result<(), RefDataError> = match op {
                Op::Commit(who, changes, stale) => {
                    let expected = if stale { ledger.version().wrapping_sub(1) } else { ledger.version() };
                    ledger.commit(person(u128::from(who)), at(clock), expected, changes).map(|_| ())
                }
                Op::Propose(who, changes) => ledger.propose(person(u128::from(who)), at(clock), changes, None).map(|_| ()),
                Op::Approve(id, who) => ledger.approve(u64::from(id), person(u128::from(who)), at(clock)).map(|_| ()),
                Op::Reject(id, who) => ledger.reject(u64::from(id), person(u128::from(who)), at(clock), "no"),
                Op::Withdraw(id, who) => ledger.withdraw(u64::from(id), person(u128::from(who)), at(clock)),
            };
            match result {
                Err(_) => prop_assert_eq!(&ledger, &before, "a failed operation changed the ledger"),
                Ok(()) => {
                    prop_assert!(ledger.version() == before.version() || ledger.version() == before.version() + 1);
                }
            }
            if ledger.version() > before.version() {
                snapshots.push(ledger.state().clone());
            }
        }
        // While a set needs approval, nothing changes without it — not even the policy.
        for (index, revision) in ledger.revisions().iter().enumerate() {
            let policy = ledger.as_of(u64::try_from(index).unwrap()).unwrap().set.change_policy;
            if policy == ChangePolicy::Approval {
                prop_assert!(revision.approved_by.is_some(), "a change bypassed approval");
            }
        }
        // The state is a valid set and matches its own effective set.
        prop_assert_eq!(&ledger.state().effective().unwrap(), ledger.effective());
        // Versions are 1, 2, 3, … and nobody approved their own change.
        for (index, revision) in ledger.revisions().iter().enumerate() {
            prop_assert_eq!(revision.version, u64::try_from(index).unwrap() + 1);
            prop_assert_ne!(Some(revision.by), revision.approved_by);
        }
        // Every version reads back exactly as it was.
        prop_assert_eq!(snapshots.len(), usize::try_from(ledger.version()).unwrap() + 1);
        for (version, snapshot) in snapshots.iter().enumerate() {
            prop_assert_eq!(&ledger.as_of(u64::try_from(version).unwrap()).unwrap(), snapshot);
        }
        // Loading from storage reproduces the ledger.
        let restored = Ledger::restore(ledger.genesis().clone(), ledger.revisions().to_vec(), ledger.proposals().to_vec()).unwrap();
        prop_assert_eq!(&restored, &ledger);
        // Proposal bookkeeping: approved proposals point at real revisions made by their authors.
        for proposal in ledger.proposals() {
            if let ProposalStatus::Approved { by, version, .. } = &proposal.status {
                let revision = &ledger.revisions()[usize::try_from(*version).unwrap() - 1];
                prop_assert_eq!(revision.by, proposal.by);
                prop_assert_eq!(revision.approved_by, Some(*by));
                prop_assert_eq!(&revision.changes, &proposal.changes);
            }
        }
    }
}

#[test]
fn std_four_eyes_is_switched_on_at_once_and_off_only_with_a_second_person() {
    let alice = person(1);
    let bob = person(2);
    let mut ledger = Ledger::new(region_state()).unwrap();
    let to = |policy| Change::SetPolicy { policy };
    // Switching four eyes on is a change like any other.
    assert_eq!(
        ledger.commit(alice, at(1), 0, vec![to(ChangePolicy::Approval)]),
        Ok(1)
    );
    assert_eq!(ledger.state().set.change_policy, ChangePolicy::Approval);
    // From now on, nothing applies at once — switching it off neither.
    assert_eq!(
        ledger.commit(alice, at(2), 1, vec![to(ChangePolicy::Direct)]),
        Err(RefDataError::ApprovalRequired)
    );
    assert_eq!(
        ledger.commit(alice, at(2), 1, vec![rename(111, "Paris")]),
        Err(RefDataError::ApprovalRequired)
    );
    // The same policy again is no change.
    assert_eq!(
        ledger.propose(alice, at(3), vec![to(ChangePolicy::Approval)], None),
        Err(RefDataError::EmptyChange)
    );
    let id = ledger
        .propose(
            alice,
            at(3),
            vec![to(ChangePolicy::Direct)],
            Some("Year-end freeze over".into()),
        )
        .unwrap();
    assert_eq!(
        ledger.approve(id, alice, at(4)),
        Err(RefDataError::SelfApproval)
    );
    assert_eq!(ledger.approve(id, bob, at(4)), Ok(2));
    assert_eq!(ledger.state().set.change_policy, ChangePolicy::Direct);
    assert_eq!(ledger.revisions()[1].approved_by, Some(bob));
    // History reads back, policy included.
    assert_eq!(
        ledger.as_of(1).unwrap().set.change_policy,
        ChangePolicy::Approval
    );
    assert_eq!(
        ledger.as_of(0).unwrap().set.change_policy,
        ChangePolicy::Direct
    );
    // A policy change touches no record.
    let impact = impact::impact(ledger.effective(), &to(ChangePolicy::Approval), &NoUsage);
    assert!(impact.is_harmless());
}

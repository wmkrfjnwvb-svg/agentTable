//! The building blocks: capabilities, places, field lists, grants, capability sets and
//! separation-of-duties rules, each by itself.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_core::condition::{Condition, Operator};
use mt_core::id::{CapabilitySetId, RecordId, RefSetId};
use mt_perm::grant::{MAX_REASON_CHARS, MAX_SCOPE_RECORDS, MAX_SET_CAPABILITIES};
use mt_perm::{
    Action, Capability, FieldAccess, Granted, Inactive, PermError, Resource, Scope, SodKind,
};
use std::collections::BTreeSet;

fn invalid(result: Result<(), PermError>) -> String {
    match result {
        Err(PermError::Invalid(reason)) => reason,
        other => panic!("expected an invalid definition, got {other:?}"),
    }
}

#[test]
fn std_built_in_capabilities() {
    assert_eq!(Action::ALL.len(), 24);
    let keys: BTreeSet<&str> = Action::ALL.iter().map(|a| a.key()).collect();
    assert_eq!(keys.len(), 24, "keys are unique");
    for action in Action::ALL.iter().copied() {
        let capability = Capability::from(action);
        assert_eq!(capability.key(), action.key());
        assert_eq!(action.to_string(), action.key());
        assert_eq!(capability.key().parse::<Capability>().unwrap(), capability);
        assert_eq!(capability.is_record_scoped(), action.is_record_scoped());
        assert_eq!(capability.is_administrative(), action.is_administrative());
        assert!(capability.includes(&capability));
        // Inclusions never lead to an administrative action and never form a cycle.
        for included in action.includes() {
            assert!(!included.is_administrative() || action.is_administrative());
            assert!(!included.includes().contains(&action));
        }
        let json = serde_json::to_string(&capability).unwrap();
        assert_eq!(json, format!("\"{}\"", action.key()));
        assert_eq!(
            serde_json::from_str::<Capability>(&json).unwrap(),
            capability
        );
    }
    // Whoever may change, delete, comment, export or approve a record may also see it.
    for action in [
        Action::RecordUpdate,
        Action::RecordDelete,
        Action::RecordComment,
        Action::RecordExport,
        Action::ProcessApprove,
    ] {
        assert!(cap(action).includes(&read()), "{action}");
        assert!(!read().includes(&cap(action)));
    }
    assert!(cap(Action::SchemaManage).includes(&cap(Action::ViewManage)));
    assert!(!cap(Action::RecordCreate).includes(&read()));
    assert!(!cap(Action::AccessManage).includes(&cap(Action::MembersManage)));
    // Which actions are about records, and which administer.
    assert!(cap(Action::ProcessApprove).is_record_scoped());
    assert!(!cap(Action::SchemaManage).is_record_scoped());
    assert!(cap(Action::AuditRead).is_administrative());
    assert!(!cap(Action::RecordDelete).is_administrative());
    assert!(
        cap(Action::DataUnmask).is_administrative() && cap(Action::DataUnmask).is_record_scoped()
    );
}

#[test]
fn std_custom_capabilities() {
    for key in [
        "salary.view",
        "transition:approve_po",
        "button:send-reminder",
        "a",
        "x1_2",
    ] {
        let capability = Capability::custom(key).unwrap();
        assert_eq!(capability, Capability::Custom(key.to_owned()));
        assert_eq!(capability.key(), key);
        assert_eq!(capability.to_string(), key);
        assert!(capability.is_record_scoped());
        assert!(!capability.is_administrative());
        let json = serde_json::to_string(&capability).unwrap();
        assert_eq!(
            serde_json::from_str::<Capability>(&json).unwrap(),
            capability
        );
    }
    // A custom capability includes only itself.
    assert!(!custom("salary.edit").includes(&custom("salary.view")));
    assert!(!custom("record.reader").includes(&read()));
    assert!(!read().includes(&custom("record.reader")));
    // Reading a built-in key gives the built-in capability.
    assert_eq!("record.read".parse::<Capability>().unwrap(), read());
    // Capabilities order by kind, then key: built-in first.
    assert!(read() < custom("a"));
}

#[test]
fn invalid_capability_keys() {
    for key in ["", "Salary", "1x", "salary view", "salary/view", "é", "-x"] {
        assert!(
            invalid(key.parse::<Capability>().map(|_| ())).contains("is not a capability key"),
            "{key}"
        );
        assert!(Capability::custom(key).is_err(), "{key}");
        assert!(
            serde_json::from_str::<Capability>(&format!("\"{key}\"")).is_err(),
            "{key}"
        );
    }
    let long = "a".repeat(129);
    assert!(Capability::custom(&long).is_err());
    assert!(Capability::custom(&"a".repeat(128)).is_ok());
    assert_eq!(
        invalid(Capability::custom("record.read").map(|_| ())),
        "`record.read` is a built-in capability"
    );
}

#[test]
fn std_places_contain_each_other() {
    let w2 = Resource::Workspace { workspace: W2 };
    let base2 = Resource::Base {
        workspace: W2,
        base: B2,
    };
    let all = [TENANT, WS1, BASE1, ORDERS, PAYMENTS, w2, base2, OTHER];
    // The tenant contains everything; a table only itself.
    for place in all {
        assert!(TENANT.covers(&place));
        assert!(place.covers(&place));
        assert!(place.overlaps(&TENANT));
    }
    assert!(WS1.covers(&BASE1) && WS1.covers(&ORDERS) && WS1.covers(&PAYMENTS));
    assert!(!WS1.covers(&OTHER) && !WS1.covers(&base2) && !WS1.covers(&TENANT));
    assert!(BASE1.covers(&ORDERS) && !BASE1.covers(&WS1) && !BASE1.covers(&OTHER));
    assert!(!ORDERS.covers(&PAYMENTS) && !ORDERS.covers(&BASE1));
    assert!(ORDERS.overlaps(&BASE1) && BASE1.overlaps(&ORDERS));
    assert!(!ORDERS.overlaps(&PAYMENTS) && !WS1.overlaps(&w2) && !ORDERS.overlaps(&OTHER));
    // A base of the same id in another workspace is another base.
    let elsewhere = Resource::Base {
        workspace: W2,
        base: B1,
    };
    assert!(!BASE1.covers(&elsewhere) && !elsewhere.covers(&ORDERS));
    // Ancestors.
    assert_eq!(
        (TENANT.workspace(), TENANT.base(), TENANT.table()),
        (None, None, None)
    );
    assert_eq!(
        (WS1.workspace(), WS1.base(), WS1.table()),
        (Some(W1), None, None)
    );
    assert_eq!(
        (BASE1.workspace(), BASE1.base(), BASE1.table()),
        (Some(W1), Some(B1), None)
    );
    assert_eq!(
        (ORDERS.workspace(), ORDERS.base(), ORDERS.table()),
        (Some(W1), Some(B1), Some(ORDERS_T))
    );
    // Stored form.
    let json = serde_json::to_value(ORDERS).unwrap();
    assert_eq!(json["level"], "table");
    assert_eq!(serde_json::from_value::<Resource>(json).unwrap(), ORDERS);
    assert_eq!(
        serde_json::to_value(TENANT).unwrap(),
        serde_json::json!({"level": "tenant"})
    );
}

#[test]
fn std_field_lists_add_up_and_intersect() {
    let only = |ids: &[u128]| FieldAccess::only(ids.iter().map(|n| f(*n)));
    let except = |ids: &[u128]| FieldAccess::except(ids.iter().map(|n| f(*n)));
    assert!(FieldAccess::All.allows(f(1)));
    assert!(only(&[1, 2]).allows(f(2)) && !only(&[1, 2]).allows(f(3)));
    assert!(except(&[1]).allows(f(2)) && !except(&[1]).allows(f(1)));
    assert!(FieldAccess::none().is_none() && !FieldAccess::none().allows(f(1)));
    assert!(!FieldAccess::All.is_none() && !only(&[1]).is_none() && !except(&[1]).is_none());
    // "Every field except none" is every field.
    assert_eq!(except(&[]), FieldAccess::All);
    // Union: two grants add up.
    assert_eq!(FieldAccess::All.union(&only(&[1])), FieldAccess::All);
    assert_eq!(only(&[1]).union(&FieldAccess::All), FieldAccess::All);
    assert_eq!(only(&[1, 2]).union(&only(&[2, 3])), only(&[1, 2, 3]));
    assert_eq!(except(&[1, 2]).union(&except(&[2, 3])), except(&[2]));
    assert_eq!(except(&[1, 2]).union(&except(&[3])), FieldAccess::All);
    assert_eq!(only(&[1]).union(&except(&[1, 2])), except(&[2]));
    assert_eq!(except(&[1, 2]).union(&only(&[1, 2])), FieldAccess::All);
    assert_eq!(FieldAccess::none().union(&only(&[4])), only(&[4]));
    // Intersection: an agent for a person sees what both may see.
    assert_eq!(FieldAccess::All.intersection(&only(&[1])), only(&[1]));
    assert_eq!(except(&[1]).intersection(&FieldAccess::All), except(&[1]));
    assert_eq!(only(&[1, 2]).intersection(&only(&[2, 3])), only(&[2]));
    assert_eq!(except(&[1]).intersection(&except(&[2])), except(&[1, 2]));
    assert_eq!(only(&[1, 2, 3]).intersection(&except(&[2])), only(&[1, 3]));
    assert_eq!(except(&[2]).intersection(&only(&[2])), FieldAccess::none());
    // Stored form.
    for access in [FieldAccess::All, only(&[1, 2]), except(&[3])] {
        let json = serde_json::to_string(&access).unwrap();
        assert_eq!(serde_json::from_str::<FieldAccess>(&json).unwrap(), access);
    }
}

#[test]
fn std_grants_and_their_validity() {
    let mut g = grant(1, BOB, read(), ORDERS);
    g.check().unwrap();
    assert!(!g.is_narrowed());
    assert_eq!(g.status(t(NOW)), Ok(()));
    // A validity window: from inclusive, to exclusive.
    g.valid_from = Some(t(100));
    g.valid_to = Some(t(200));
    g.check().unwrap();
    assert_eq!(g.status(t(99)), Err(Inactive::NotYetValid));
    assert_eq!(g.status(t(100)), Ok(()));
    assert_eq!(g.status(t(199)), Ok(()));
    assert_eq!(g.status(t(200)), Err(Inactive::Expired));
    // A revocation takes effect at its instant.
    g.revoked_at = Some(t(150));
    assert_eq!(g.status(t(149)), Ok(()));
    assert_eq!(g.status(t(150)), Err(Inactive::Revoked));
    assert_eq!(g.status(t(300)), Err(Inactive::Revoked));
    // Narrowing.
    let mut narrowed = grant(2, BOB, read(), ORDERS);
    narrowed.scope = Scope::Own;
    assert!(narrowed.is_narrowed());
    narrowed.scope = Scope::All;
    narrowed.condition = Some(up_to(10));
    assert!(narrowed.is_narrowed());
    narrowed.condition = None;
    narrowed.fields = FieldAccess::only([f(AMOUNT)]);
    assert!(narrowed.is_narrowed());
    narrowed.check().unwrap();
    narrowed.check_in_table(&order_type).unwrap();
    // The stored form keeps everything.
    let mut full = grant(3, FINANCE, cap(Action::ProcessApprove), ORDERS);
    full.scope = Scope::RefSubtree {
        field: f(REGION),
        set: RefSetId::from_u128(REGIONS),
        root: item(R_EMEA),
    };
    full.condition = Some(up_to(10_000));
    full.fields = FieldAccess::except([f(SALARY)]);
    full.valid_to = Some(t(5_000));
    full.revoked_at = Some(t(4_000));
    full.revoked_by = Some(p(ALICE));
    full.check().unwrap();
    full.check_in_table(&order_type).unwrap();
    let json = serde_json::to_string(&full).unwrap();
    assert_eq!(serde_json::from_str::<mt_perm::Grant>(&json).unwrap(), full);
    // Scopes in their stored form.
    for scope in [
        Scope::All,
        Scope::Own,
        Scope::Team,
        Scope::Where { condition: mine() },
        Scope::Records {
            records: [RecordId::from_u128(1)].into(),
        },
    ] {
        let json = serde_json::to_string(&scope).unwrap();
        assert_eq!(serde_json::from_str::<Scope>(&json).unwrap(), scope);
    }
}

#[test]
fn invalid_grants() {
    let with = |edit: &dyn Fn(&mut mt_perm::Grant)| {
        let mut g = grant(1, BOB, read(), ORDERS);
        edit(&mut g);
        invalid(g.check())
    };
    assert_eq!(with(&|g| g.reason = "  ".into()), "a grant needs a reason");
    assert_eq!(
        with(&|g| g.reason = "x".repeat(MAX_REASON_CHARS + 1)),
        "the reason is longer than 500 characters"
    );
    assert_eq!(
        with(&|g| {
            g.valid_from = Some(t(10));
            g.valid_to = Some(t(10));
        }),
        "the grant ends before it starts"
    );
    // Only actions on records can be limited to some records or fields.
    assert!(with(&|g| {
        g.what = Granted::Capability(cap(Action::SchemaManage));
        g.scope = Scope::Own;
    })
    .contains("`schema.manage` is not about individual records"));
    assert!(with(&|g| {
        g.what = Granted::Capability(cap(Action::AccessManage));
        g.fields = FieldAccess::only([f(AMOUNT)]);
    })
    .contains("cannot be limited"));
    // Conditions and field lists belong to one table.
    let one_table = "a grant limited to some records or fields must be given on one table";
    assert_eq!(
        with(&|g| {
            g.on = BASE1;
            g.condition = Some(up_to(5));
        }),
        one_table
    );
    assert_eq!(
        with(&|g| {
            g.on = WS1;
            g.fields = FieldAccess::only([f(AMOUNT)]);
        }),
        one_table
    );
    assert_eq!(
        with(&|g| {
            g.on = TENANT;
            g.scope = Scope::Where { condition: mine() };
        }),
        one_table
    );
    // Ownership scopes work anywhere.
    let mut anywhere = grant(1, BOB, read(), TENANT);
    anywhere.scope = Scope::Team;
    anywhere.check().unwrap();
    // Record lists.
    let records = "a record list has between 1 and 10000 records";
    assert_eq!(
        with(&|g| g.scope = Scope::Records {
            records: BTreeSet::new()
        }),
        records
    );
    assert_eq!(
        with(&|g| {
            g.scope = Scope::Records {
                records: (0..=MAX_SCOPE_RECORDS as u128)
                    .map(RecordId::from_u128)
                    .collect(),
            };
        }),
        records
    );
    assert_eq!(
        with(&|g| g.fields = FieldAccess::none()),
        "the grant includes no field at all"
    );
    // Conditions within limits.
    let mut deep = up_to(1);
    for _ in 0..9 {
        deep = deep.negated();
    }
    assert!(with(&|g| g.condition = Some(deep.clone())).contains("at most 100 tests and 8 levels"));
    let wide = Condition::All(vec![up_to(1); 101]);
    assert!(with(&|g| g.scope = Scope::Where {
        condition: wide.clone()
    })
    .contains("at most"));
}

#[test]
fn invalid_grants_in_their_table() {
    let with = |edit: &dyn Fn(&mut mt_perm::Grant)| {
        let mut g = grant(1, BOB, read(), ORDERS);
        edit(&mut g);
        g.check().unwrap();
        invalid(g.check_in_table(&order_type))
    };
    let message =
        with(&|g| g.condition = Some(Condition::compare(f(STATUS), Operator::Lt, num(3))));
    assert!(
        !message.is_empty(),
        "a number compared with a text field is refused: {message}"
    );
    assert!(with(&|g| g.scope = Scope::Where {
        condition: Condition::is_empty(f(999))
    })
    .contains("unknown field"));
    assert!(with(&|g| {
        g.scope = Scope::RefSubtree {
            field: f(AMOUNT),
            set: RefSetId::from_u128(REGIONS),
            root: item(WORLD),
        };
    })
    .contains("a subtree scope needs a reference field"));
    assert!(with(&|g| {
        g.scope = Scope::RefSubtree {
            field: f(999),
            set: RefSetId::from_u128(REGIONS),
            root: item(WORLD),
        };
    })
    .contains("is not a field of the table"));
    assert!(with(&|g| g.fields = FieldAccess::only([f(AMOUNT), f(999)]))
        .contains("is not a field of the table"));
    assert!(
        with(&|g| g.fields = FieldAccess::except([f(998)])).contains("is not a field of the table")
    );
    // Valid ones pass.
    let mut fine = grant(1, BOB, read(), ORDERS);
    fine.scope = Scope::Records {
        records: [RecordId::from_u128(1)].into(),
    };
    fine.check_in_table(&order_type).unwrap();
    fine.scope = Scope::Own;
    fine.check_in_table(&order_type).unwrap();
}

#[test]
fn std_capability_sets_and_rules() {
    let approver = set(
        1,
        "Approver up to 10 000",
        &[read(), cap(Action::ProcessApprove)],
    );
    approver.check().unwrap();
    let json = serde_json::to_string(&approver).unwrap();
    assert_eq!(
        serde_json::from_str::<mt_perm::CapabilitySet>(&json).unwrap(),
        approver
    );
    assert_eq!(
        invalid(set(1, " ", &[read()]).check()),
        "a capability set needs a name"
    );
    assert_eq!(
        invalid(set(1, &"n".repeat(256), &[read()]).check()),
        "the name of a capability set is longer than 255 characters"
    );
    assert!(invalid(set(1, "Empty", &[]).check()).contains("between 1 and 200"));
    let many: Vec<_> = (0..=MAX_SET_CAPABILITIES)
        .map(|n| custom(&format!("c{n}")))
        .collect();
    assert!(invalid(set(1, "Many", &many).check()).contains("between 1 and 200"));
    assert!(set(1, "Most", &many[..MAX_SET_CAPABILITIES])
        .check()
        .is_ok());

    let four_eyes = rule(
        1,
        cap(Action::RecordCreate),
        cap(Action::ProcessApprove),
        SodKind::SameRecord,
    );
    four_eyes.check().unwrap();
    assert_eq!(
        four_eyes.counterpart(&cap(Action::RecordCreate)),
        Some(&cap(Action::ProcessApprove))
    );
    assert_eq!(
        four_eyes.counterpart(&cap(Action::ProcessApprove)),
        Some(&cap(Action::RecordCreate))
    );
    assert_eq!(four_eyes.counterpart(&read()), None);
    assert_eq!(
        invalid(rule(1, read(), read(), SodKind::Hold).check()),
        "a separation-of-duties rule needs two different capabilities"
    );
    let mut unnamed = four_eyes.clone();
    unnamed.name = String::new();
    assert_eq!(
        invalid(unnamed.check()),
        "a separation-of-duties rule needs a name"
    );
    let json = serde_json::to_string(&four_eyes).unwrap();
    assert_eq!(
        serde_json::from_str::<mt_perm::SodRule>(&json).unwrap(),
        four_eyes
    );
    // Errors read well.
    assert_eq!(
        PermError::SodConflict {
            rule: four_eyes.id,
            first: "a".into(),
            second: "b".into()
        }
        .to_string(),
        format!(
            "separation of duties: rule {} forbids holding `a` and `b` together",
            four_eyes.id
        )
    );
    assert_eq!(
        PermError::SetInUse {
            set: CapabilitySetId::from_u128(1),
            grants: 2
        }
        .to_string(),
        format!(
            "capability set {} is still used by 2 grant(s)",
            CapabilitySetId::from_u128(1)
        )
    );
}

//! The decision engine: decisions and their explanation, groups, validity, scopes, limits,
//! guests, agents acting for a person, capability sets, separation of duties, delegated
//! administration, row filters, field permissions and what a principal holds.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_core::id::{CapabilitySetId, RecordId, RefSetId, SodRuleId};
use mt_core::value::Value;
use mt_perm::{
    Action, Actor, Applied, Capability, FieldAccess, FieldPermission, Granted, Inactive, PermError,
    Policy, PrincipalKind, Reason, Request, Scope, SodKind, Visibility,
};

fn give(policy: &mut Policy, world: &World, grant: mt_perm::Grant) {
    policy.give(grant, &world.org).unwrap();
}

fn reasons(decision: &mt_perm::Decision) -> Vec<Reason> {
    decision.refusals.iter().map(|r| r.reason.clone()).collect()
}

#[test]
fn std_nobody_may_do_anything_by_default() {
    let (policy, world) = (Policy::new(), World::new());
    let decision = ask(
        &policy,
        &world,
        Actor::direct(p(BOB)),
        &read(),
        ORDERS,
        None,
    );
    assert!(!decision.allowed);
    assert!(decision.chain.is_empty());
    assert_eq!(reasons(&decision), [Reason::NoGrant]);
    assert!(decision.fields.is_none());
}

#[test]
fn std_a_grant_allows_and_the_decision_explains_why() {
    let (mut policy, world) = (Policy::new(), World::new());
    give(&mut policy, &world, grant(1, BOB, read(), ORDERS));
    let decision = ask(
        &policy,
        &world,
        Actor::direct(p(BOB)),
        &read(),
        ORDERS,
        None,
    );
    assert!(decision.allowed);
    assert_eq!(
        decision.chain,
        [Applied {
            grant: gid(1),
            principal: p(BOB),
            holder: p(BOB),
            through_set: None,
            capability: read(),
            on: ORDERS,
        }]
    );
    assert!(decision.refusals.is_empty());
    assert_eq!(decision.fields, FieldAccess::All);
    // Somewhere else the grant does not apply, and the decision says so.
    for elsewhere in [PAYMENTS, BASE1, OTHER, TENANT] {
        let decision = ask(
            &policy,
            &world,
            Actor::direct(p(BOB)),
            &read(),
            elsewhere,
            None,
        );
        assert!(!decision.allowed);
        assert_eq!(reasons(&decision), [Reason::Elsewhere]);
        assert_eq!(decision.refusals[0].grant, Some(gid(1)));
        assert_eq!(decision.refusals[0].principal, p(BOB));
    }
    // A grant on the base covers its tables, not another workspace's.
    give(&mut policy, &world, grant(2, CAROL, read(), BASE1));
    assert!(allowed(&policy, &world, CAROL, &read(), ORDERS, None));
    assert!(allowed(&policy, &world, CAROL, &read(), PAYMENTS, None));
    assert!(allowed(&policy, &world, CAROL, &read(), BASE1, None));
    assert!(!allowed(&policy, &world, CAROL, &read(), WS1, None));
    assert!(!allowed(&policy, &world, CAROL, &read(), OTHER, None));
    // Other capabilities are not given.
    let decision = ask(
        &policy,
        &world,
        Actor::direct(p(BOB)),
        &cap(Action::RecordDelete),
        ORDERS,
        None,
    );
    assert_eq!(reasons(&decision), [Reason::NoGrant]);
    // The decision is stored as it is shown.
    let json = serde_json::to_string(&decision).unwrap();
    assert_eq!(
        serde_json::from_str::<mt_perm::Decision>(&json).unwrap(),
        decision
    );
}

#[test]
fn std_inclusions_and_capability_sets_in_the_chain() {
    let (mut policy, world) = (Policy::new(), World::new());
    give(
        &mut policy,
        &world,
        grant(1, BOB, cap(Action::RecordUpdate), ORDERS),
    );
    let decision = ask(
        &policy,
        &world,
        Actor::direct(p(BOB)),
        &read(),
        ORDERS,
        None,
    );
    assert!(decision.allowed);
    assert_eq!(
        decision.chain[0].capability,
        cap(Action::RecordUpdate),
        "update includes read"
    );
    policy
        .add_set(set(7, "Approver", &[read(), cap(Action::ProcessApprove)]))
        .unwrap();
    give(&mut policy, &world, set_grant(2, CAROL, 7, ORDERS));
    let decision = ask(
        &policy,
        &world,
        Actor::direct(p(CAROL)),
        &read(),
        ORDERS,
        None,
    );
    assert_eq!(
        decision.chain[0].through_set,
        Some(CapabilitySetId::from_u128(7))
    );
    assert_eq!(
        decision.chain[0].capability,
        read(),
        "the exact capability is preferred"
    );
    assert!(allowed(
        &policy,
        &world,
        CAROL,
        &cap(Action::ProcessApprove),
        ORDERS,
        None
    ));
    assert!(!allowed(
        &policy,
        &world,
        CAROL,
        &cap(Action::RecordDelete),
        ORDERS,
        None
    ));
    // Several grants: every one that applies is in the chain.
    give(&mut policy, &world, grant(3, CAROL, read(), BASE1));
    let decision = ask(
        &policy,
        &world,
        Actor::direct(p(CAROL)),
        &read(),
        ORDERS,
        None,
    );
    let grants: Vec<_> = decision.chain.iter().map(|link| link.grant).collect();
    assert_eq!(grants.len(), 2);
    assert!(grants.contains(&gid(2)) && grants.contains(&gid(3)));
}

#[test]
fn std_groups_including_nested_groups() {
    let (mut policy, world) = (Policy::new(), World::new());
    give(&mut policy, &world, grant(1, EMEA, read(), ORDERS));
    for member in [BOB, CAROL, DAVE] {
        let decision = ask(
            &policy,
            &world,
            Actor::direct(p(member)),
            &read(),
            ORDERS,
            None,
        );
        assert!(decision.allowed, "{member}");
        assert_eq!(decision.chain[0].holder, p(EMEA));
        assert_eq!(decision.chain[0].principal, p(member));
    }
    assert!(!allowed(&policy, &world, ALICE, &read(), ORDERS, None));
    // A grant to an empty group is allowed and reaches nobody.
    give(
        &mut policy,
        &world,
        grant(2, EMPTY_GROUP, cap(Action::RecordDelete), ORDERS),
    );
    assert!(!allowed(
        &policy,
        &world,
        BOB,
        &cap(Action::RecordDelete),
        ORDERS,
        None
    ));
}

#[test]
fn std_validity_and_revocation() {
    let (mut policy, world) = (Policy::new(), World::new());
    let mut later = grant(1, BOB, read(), ORDERS);
    later.valid_from = Some(t(NOW + 1));
    give(&mut policy, &world, later);
    let mut ended = grant(2, BOB, read(), ORDERS);
    ended.valid_to = Some(t(NOW));
    give(&mut policy, &world, ended);
    give(&mut policy, &world, grant(3, BOB, read(), ORDERS));
    policy.revoke(gid(3), p(ALICE), t(NOW - 1)).unwrap();
    let decision = ask(
        &policy,
        &world,
        Actor::direct(p(BOB)),
        &read(),
        ORDERS,
        None,
    );
    assert!(!decision.allowed);
    let mut found = reasons(&decision);
    found.sort_by_key(|r| format!("{r:?}"));
    assert_eq!(
        found,
        [
            Reason::Inactive {
                why: Inactive::Expired
            },
            Reason::Inactive {
                why: Inactive::NotYetValid
            },
            Reason::Inactive {
                why: Inactive::Revoked
            },
        ]
    );
    // The revocation is recorded, not erased.
    let revoked = policy.grant(gid(3)).unwrap();
    assert_eq!(
        (revoked.revoked_at, revoked.revoked_by),
        (Some(t(NOW - 1)), Some(p(ALICE)))
    );
    assert_eq!(
        policy.revoke(gid(3), p(ALICE), t(NOW)),
        Err(PermError::AlreadyRevoked(gid(3)))
    );
    assert_eq!(
        policy.revoke(gid(9), p(ALICE), t(NOW)),
        Err(PermError::UnknownGrant(gid(9)))
    );
    assert_eq!(policy.grants().count(), 3);
    assert_eq!(policy.grants_of(p(BOB)).count(), 3);
    assert_eq!(policy.grants_of(p(CAROL)).count(), 0);
}

#[test]
fn std_scopes_decide_which_records() {
    let (mut policy, world) = (Policy::new(), World::new());
    let scoped = |id: u128, to: u128, scope: Scope| {
        let mut g = grant(id, to, read(), ORDERS);
        g.scope = scope;
        g
    };
    give(&mut policy, &world, scoped(1, BOB, Scope::Own));
    give(&mut policy, &world, scoped(2, DAVE, Scope::Team));
    give(
        &mut policy,
        &world,
        scoped(
            3,
            CAROL,
            Scope::Where {
                condition: mine().negated(),
            },
        ),
    );
    give(
        &mut policy,
        &world,
        scoped(
            4,
            ALICE,
            Scope::RefSubtree {
                field: f(REGION),
                set: RefSetId::from_u128(REGIONS),
                root: item(R_EMEA),
            },
        ),
    );
    let bobs = order(10, 100, Some(BOB), Some(DE));
    let carols = order(11, 100, Some(CAROL), Some(JP));
    let alices = order(12, 100, Some(ALICE), Some(FR));
    let nobodys = order(13, 100, None, None);
    let mut finance_owned = order(14, 100, None, Some(R_EMEA));
    finance_owned
        .cells
        .insert(f(OWNER), Value::Principal(vec![p(FINANCE)]));
    let mut requested_by_bob = order(15, 100, Some(ALICE), Some(APAC));
    requested_by_bob
        .cells
        .insert(f(REQUESTER), Value::Principal(vec![p(BOB)]));
    let mut multi_region = order(16, 100, None, None);
    multi_region
        .cells
        .insert(f(REGION), Value::MultiReference(vec![item(JP), item(FR)]));

    let can =
        |who: u128, record: &Rec| allowed(&policy, &world, who, &read(), ORDERS, Some(record));
    // Own: the owner fields (owner, requester) name the principal.
    assert!(can(BOB, &bobs) && can(BOB, &requested_by_bob));
    assert!(!can(BOB, &carols) && !can(BOB, &nobodys) && !can(BOB, &finance_owned));
    // Team: Dave is in EMEA, which holds Finance (Bob, Carol): their records, and records
    // owned by the groups, are the team's.
    assert!(can(DAVE, &bobs) && can(DAVE, &carols) && can(DAVE, &finance_owned));
    assert!(!can(DAVE, &alices) && !can(DAVE, &nobodys));
    // A condition, with `me` being the principal checked: records Carol does not own.
    assert!(can(CAROL, &bobs) && can(CAROL, &nobodys) && !can(CAROL, &carols));
    // A reference subtree: EMEA and below; a multi-reference matches with any item.
    assert!(can(ALICE, &bobs) && can(ALICE, &alices) && can(ALICE, &finance_owned));
    assert!(can(ALICE, &multi_region));
    assert!(!can(ALICE, &carols) && !can(ALICE, &nobodys) && !can(ALICE, &requested_by_bob));
    let mut text_region = order(17, 1, None, None);
    text_region
        .cells
        .insert(f(REGION), Value::Text("EMEA".into()));
    assert!(!can(ALICE, &text_region));
    // A subtree of another reference set never matches.
    let mut other_set = Policy::new();
    give(
        &mut other_set,
        &world,
        scoped(
            5,
            ALICE,
            Scope::RefSubtree {
                field: f(REGION),
                set: RefSetId::from_u128(99),
                root: item(WORLD),
            },
        ),
    );
    assert!(!allowed(
        &other_set,
        &world,
        ALICE,
        &read(),
        ORDERS,
        Some(&bobs)
    ));
    // A list of records.
    let mut listed = Policy::new();
    give(
        &mut listed,
        &world,
        scoped(
            6,
            BOB,
            Scope::Records {
                records: [RecordId::from_u128(11)].into(),
            },
        ),
    );
    assert!(allowed(
        &listed,
        &world,
        BOB,
        &read(),
        ORDERS,
        Some(&carols)
    ));
    let decision = ask(
        &listed,
        &world,
        Actor::direct(p(BOB)),
        &read(),
        ORDERS,
        Some(&bobs),
    );
    assert_eq!(reasons(&decision), [Reason::OutOfScope]);
    // Ownership needs a table: on another table nothing is owned.
    let mut elsewhere = Policy::new();
    let mut own_payments = grant(7, BOB, read(), PAYMENTS);
    own_payments.scope = Scope::Own;
    give(&mut elsewhere, &world, own_payments);
    assert!(!allowed(
        &elsewhere,
        &world,
        BOB,
        &read(),
        PAYMENTS,
        Some(&bobs)
    ));
    // Without a record the question is about the table as a whole.
    assert!(allowed(&policy, &world, BOB, &read(), ORDERS, None));
}

#[test]
fn std_conditions_are_limits() {
    let (mut policy, world) = (Policy::new(), World::new());
    let approve = cap(Action::ProcessApprove);
    let mut small = grant(1, BOB, approve.clone(), ORDERS);
    small.condition = Some(up_to(10_000));
    give(&mut policy, &world, small);
    let mut own_large = grant(2, BOB, approve.clone(), ORDERS);
    own_large.scope = Scope::Own;
    own_large.condition = Some(up_to(50_000));
    give(&mut policy, &world, own_large);
    let check = |amount: i64, owner: u128| {
        ask(
            &policy,
            &world,
            Actor::direct(p(BOB)),
            &approve,
            ORDERS,
            Some(&order(1, amount, Some(owner), None)),
        )
    };
    assert!(check(10_000, CAROL).allowed);
    let over = check(10_001, CAROL);
    assert!(!over.allowed);
    let mut found = reasons(&over);
    found.sort_by_key(|r| format!("{r:?}"));
    assert_eq!(found, [Reason::ConditionNotMet, Reason::OutOfScope]);
    // Bob's own records: up to 50 000 through the second grant, explained as such.
    let own = check(30_000, BOB);
    assert!(own.allowed);
    assert_eq!(own.chain.len(), 1);
    assert_eq!(own.chain[0].grant, gid(2));
    assert_eq!(
        own.refusals.len(),
        1,
        "the first grant's limit is still reported"
    );
    assert_eq!(own.refusals[0].reason, Reason::ConditionNotMet);
    assert!(!check(60_000, BOB).allowed);
    // A record without an amount does not satisfy "amount ≤ limit".
    let mut no_amount = order(2, 0, Some(CAROL), None);
    no_amount.cells.remove(&f(AMOUNT));
    assert!(!allowed(
        &policy,
        &world,
        BOB,
        &approve,
        ORDERS,
        Some(&no_amount)
    ));
}

#[test]
fn std_guests_never_administer() {
    let (mut policy, mut world) = (Policy::new(), World::new());
    world
        .org
        .members
        .entry(p(EMEA))
        .or_default()
        .insert(p(GUEST));
    give(&mut policy, &world, grant(1, GUEST, read(), ORDERS));
    assert!(allowed(&policy, &world, GUEST, &read(), ORDERS, None));
    // Not directly …
    let refused = policy.give(
        grant(2, GUEST, cap(Action::SchemaManage), BASE1),
        &world.org,
    );
    assert_eq!(
        refused,
        Err(PermError::Invalid(
            "a guest cannot be given `schema.manage`".into()
        ))
    );
    policy
        .add_set(set(3, "Builder", &[read(), cap(Action::AutomationManage)]))
        .unwrap();
    assert!(policy
        .give(set_grant(3, GUEST, 3, BASE1), &world.org)
        .is_err());
    // … and not through a group either.
    give(
        &mut policy,
        &world,
        grant(4, EMEA, cap(Action::AuditRead), TENANT),
    );
    assert!(allowed(
        &policy,
        &world,
        DAVE,
        &cap(Action::AuditRead),
        TENANT,
        None
    ));
    let decision = ask(
        &policy,
        &world,
        Actor::direct(p(GUEST)),
        &cap(Action::AuditRead),
        TENANT,
        None,
    );
    assert!(!decision.allowed);
    assert_eq!(reasons(&decision), [Reason::Administrative]);
    assert!(!policy
        .capabilities(p(GUEST), TENANT, t(NOW), &world.org)
        .contains(&cap(Action::AuditRead)));
}

#[test]
fn std_unknown_principals_get_nothing() {
    let (mut policy, mut world) = (Policy::new(), World::new());
    let refused = policy.give(grant(1, REMOVED, read(), ORDERS), &world.org);
    assert_eq!(
        refused,
        Err(PermError::Invalid(format!(
            "{} is not a principal of this tenant",
            p(REMOVED)
        )))
    );
    // A principal removed after being given access.
    world.org.kinds.insert(p(REMOVED), PrincipalKind::Member);
    give(&mut policy, &world, grant(1, REMOVED, read(), ORDERS));
    assert!(allowed(&policy, &world, REMOVED, &read(), ORDERS, None));
    world.org.kinds.remove(&p(REMOVED));
    let decision = ask(
        &policy,
        &world,
        Actor::direct(p(REMOVED)),
        &read(),
        ORDERS,
        None,
    );
    assert!(!decision.allowed);
    assert_eq!(reasons(&decision), [Reason::UnknownPrincipal]);
    assert!(policy
        .capabilities(p(REMOVED), TENANT, t(NOW), &world.org)
        .is_empty());
}

#[test]
fn std_an_agent_does_no_more_than_the_person_it_acts_for() {
    let (mut policy, world) = (Policy::new(), World::new());
    let mut agent_read = grant(1, AGENT, read(), TENANT);
    agent_read.fields = FieldAccess::All;
    give(&mut policy, &world, agent_read);
    let mut agent_orders = grant(5, AGENT, read(), ORDERS);
    agent_orders.fields = FieldAccess::except([f(SALARY)]);
    let mut bob_own = grant(2, BOB, read(), ORDERS);
    bob_own.scope = Scope::Own;
    bob_own.fields = FieldAccess::only([f(AMOUNT), f(SALARY)]);
    give(&mut policy, &world, bob_own);
    let bobs = order(1, 10, Some(BOB), None);
    let carols = order(2, 10, Some(CAROL), None);
    let for_bob = Actor::agent(p(AGENT), p(BOB));
    // Allowed when both are: the chain shows both.
    let decision = ask(&policy, &world, for_bob, &read(), ORDERS, Some(&bobs));
    assert!(decision.allowed);
    let principals: Vec<_> = decision.chain.iter().map(|link| link.principal).collect();
    assert_eq!(principals, [p(AGENT), p(BOB)]);
    // Fields: what both may see.
    assert_eq!(decision.fields, FieldAccess::only([f(AMOUNT), f(SALARY)]));
    let mut narrow = policy.clone();
    give(&mut narrow, &world, {
        let mut g = agent_orders.clone();
        g.id = gid(6);
        g
    });
    // (Grant 1 gives all fields on the tenant; fields add up per principal.)
    let both = ask(&narrow, &world, for_bob, &read(), ORDERS, Some(&bobs));
    assert_eq!(both.fields, FieldAccess::only([f(AMOUNT), f(SALARY)]));
    // Not on Carol's record, which Bob may not read, though the agent may.
    let decision = ask(&policy, &world, for_bob, &read(), ORDERS, Some(&carols));
    assert!(!decision.allowed);
    assert_eq!(decision.refusals[0].principal, p(BOB));
    assert_eq!(decision.refusals[0].reason, Reason::OutOfScope);
    assert!(decision.fields.is_none());
    assert!(allowed(
        &policy,
        &world,
        AGENT,
        &read(),
        ORDERS,
        Some(&carols)
    ));
    // Not where only the person has access: an agent without its own grant can do nothing.
    give(
        &mut policy,
        &world,
        grant(3, BOB, cap(Action::RecordDelete), ORDERS),
    );
    let decision = ask(
        &policy,
        &world,
        for_bob,
        &cap(Action::RecordDelete),
        ORDERS,
        Some(&bobs),
    );
    assert!(!decision.allowed);
    assert_eq!(decision.refusals[0].principal, p(AGENT));
    assert_eq!(decision.refusals[0].reason, Reason::NoGrant);
    // Acting for oneself is acting directly.
    let same = Actor {
        principal: p(BOB),
        on_behalf_of: Some(p(BOB)),
    };
    assert_eq!(
        ask(&policy, &world, same, &read(), ORDERS, Some(&bobs)),
        ask(
            &policy,
            &world,
            Actor::direct(p(BOB)),
            &read(),
            ORDERS,
            Some(&bobs)
        )
    );
    let json = serde_json::to_string(&for_bob).unwrap();
    assert_eq!(serde_json::from_str::<Actor>(&json).unwrap(), for_bob);
}

#[test]
fn std_narrowed_grants_of_a_set_give_only_record_actions() {
    let (mut policy, world) = (Policy::new(), World::new());
    policy
        .add_set(set(
            1,
            "Owner of own orders",
            &[read(), cap(Action::SchemaManage)],
        ))
        .unwrap();
    let mut own = set_grant(1, BOB, 1, ORDERS);
    own.scope = Scope::Own;
    give(&mut policy, &world, own);
    assert!(allowed(
        &policy,
        &world,
        BOB,
        &read(),
        ORDERS,
        Some(&order(1, 1, Some(BOB), None))
    ));
    let decision = ask(
        &policy,
        &world,
        Actor::direct(p(BOB)),
        &cap(Action::SchemaManage),
        ORDERS,
        None,
    );
    assert!(!decision.allowed);
    assert_eq!(reasons(&decision), [Reason::OutOfScope]);
    let held = policy.capabilities(p(BOB), ORDERS, t(NOW), &world.org);
    assert!(held.contains(&read()) && !held.contains(&cap(Action::SchemaManage)));
}

#[test]
fn std_capability_sets_are_managed() {
    let (mut policy, world) = (Policy::new(), World::new());
    let reader = set(1, "Reader", &[read()]);
    policy.add_set(reader.clone()).unwrap();
    assert!(matches!(
        policy.add_set(reader.clone()),
        Err(PermError::Duplicate {
            what: "capability set",
            ..
        })
    ));
    assert!(matches!(
        policy.add_set(set(2, "", &[read()])),
        Err(PermError::Invalid(_))
    ));
    assert_eq!(
        policy.give(set_grant(1, BOB, 9, ORDERS), &world.org),
        Err(PermError::UnknownSet(CapabilitySetId::from_u128(9)))
    );
    give(&mut policy, &world, set_grant(1, BOB, 1, ORDERS));
    assert!(!allowed(
        &policy,
        &world,
        BOB,
        &cap(Action::RecordComment),
        ORDERS,
        None
    ));
    // Changing the set changes what its grants give.
    policy
        .update_set(
            set(
                1,
                "Reader and commenter",
                &[read(), cap(Action::RecordComment)],
            ),
            t(NOW),
            &world.org,
        )
        .unwrap();
    assert!(allowed(
        &policy,
        &world,
        BOB,
        &cap(Action::RecordComment),
        ORDERS,
        None
    ));
    assert_eq!(
        policy.set(CapabilitySetId::from_u128(1)).unwrap().name,
        "Reader and commenter"
    );
    assert_eq!(policy.sets().count(), 1);
    assert_eq!(
        policy.update_set(set(8, "Ghost", &[read()]), t(NOW), &world.org),
        Err(PermError::UnknownSet(CapabilitySetId::from_u128(8)))
    );
    assert!(
        policy.set(CapabilitySetId::from_u128(8)).is_none(),
        "an unknown set is not created"
    );
    assert!(policy
        .update_set(set(1, "", &[read()]), t(NOW), &world.org)
        .is_err());
    // A set in use cannot be removed; once its grants are revoked it can.
    assert_eq!(
        policy.remove_set(CapabilitySetId::from_u128(1)),
        Err(PermError::SetInUse {
            set: CapabilitySetId::from_u128(1),
            grants: 1
        })
    );
    policy.revoke(gid(1), p(ALICE), t(NOW)).unwrap();
    policy.remove_set(CapabilitySetId::from_u128(1)).unwrap();
    assert_eq!(
        policy.remove_set(CapabilitySetId::from_u128(1)),
        Err(PermError::UnknownSet(CapabilitySetId::from_u128(1)))
    );
    // A grant of a set that no longer exists gives nothing.
    assert!(!allowed(&policy, &world, BOB, &read(), ORDERS, None));
}

#[test]
fn std_separation_of_duties_when_holding() {
    let (mut policy, world) = (Policy::new(), World::new());
    let maintain = custom("supplier.maintain");
    let release = custom("payment.release");
    policy
        .add_rule(rule(1, maintain.clone(), release.clone(), SodKind::Hold))
        .unwrap();
    assert!(matches!(
        policy.add_rule(rule(1, maintain.clone(), release.clone(), SodKind::Hold)),
        Err(PermError::Duplicate { .. })
    ));
    let conflict = Err(PermError::SodConflict {
        rule: SodRuleId::from_u128(1),
        first: "supplier.maintain".into(),
        second: "payment.release".into(),
    });
    give(&mut policy, &world, grant(1, BOB, maintain.clone(), ORDERS));
    // The same person, overlapping places: refused, whichever side comes second.
    assert_eq!(
        policy.give(grant(2, BOB, release.clone(), ORDERS), &world.org),
        conflict
    );
    assert_eq!(
        policy.give(grant(2, BOB, release.clone(), BASE1), &world.org),
        conflict
    );
    // Places that do not overlap are fine.
    give(&mut policy, &world, grant(2, BOB, release.clone(), OTHER));
    give(
        &mut policy,
        &world,
        grant(3, BOB, release.clone(), PAYMENTS),
    );
    // Through a group the member would hold both.
    assert_eq!(
        policy.give(grant(4, FINANCE, release.clone(), ORDERS), &world.org),
        conflict
    );
    assert_eq!(
        policy.give(grant(4, EMEA, release.clone(), TENANT), &world.org),
        conflict
    );
    // Both in one grant.
    policy
        .add_set(set(
            5,
            "Everything payable",
            &[maintain.clone(), release.clone()],
        ))
        .unwrap();
    assert_eq!(
        policy.give(set_grant(4, CAROL, 5, ORDERS), &world.org),
        conflict
    );
    // A set change that would bring them together is refused and changes nothing.
    policy
        .add_set(set(6, "Maintainer", &[maintain.clone()]))
        .unwrap();
    give(&mut policy, &world, set_grant(4, CAROL, 6, ORDERS));
    give(&mut policy, &world, grant(7, CAROL, read(), ORDERS));
    assert_eq!(
        policy.update_set(
            set(6, "Maintainer", &[maintain.clone(), release.clone()]),
            t(NOW),
            &world.org
        ),
        conflict
    );
    assert_eq!(
        policy
            .set(CapabilitySetId::from_u128(6))
            .unwrap()
            .capabilities
            .len(),
        1
    );
    // Expired and revoked grants do not count.
    let mut expired = grant(8, DAVE, maintain.clone(), ORDERS);
    expired.valid_to = Some(t(1));
    give(&mut policy, &world, expired);
    let mut later = grant(9, DAVE, release.clone(), ORDERS);
    later.granted_at = t(NOW);
    give(&mut policy, &world, later);
    policy.revoke(gid(1), p(ALICE), t(NOW)).unwrap();
    let mut after = grant(10, BOB, release.clone(), ORDERS);
    after.granted_at = t(NOW);
    give(&mut policy, &world, after);
    assert_eq!(policy.rules().count(), 1);
    assert!(policy.remove_rule(SodRuleId::from_u128(1)));
    assert!(!policy.remove_rule(SodRuleId::from_u128(1)));
}

#[test]
fn std_existing_violations_are_found() {
    let (mut policy, world) = (Policy::new(), World::new());
    let maintain = custom("supplier.maintain");
    let release = custom("payment.release");
    give(&mut policy, &world, grant(1, BOB, maintain.clone(), ORDERS));
    give(
        &mut policy,
        &world,
        grant(2, FINANCE, release.clone(), BASE1),
    );
    give(
        &mut policy,
        &world,
        grant(3, DAVE, maintain.clone(), ORDERS),
    );
    give(
        &mut policy,
        &world,
        grant(4, DAVE, release.clone(), PAYMENTS),
    );
    assert!(
        policy.violations(t(NOW), &world.org).is_empty(),
        "no rule yet"
    );
    // The rule arrives after the grants: they stay, and the review must resolve them.
    policy
        .add_rule(rule(1, maintain, release, SodKind::Hold))
        .unwrap();
    let found = policy.violations(t(NOW), &world.org);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].principal, p(BOB));
    assert_eq!((found[0].first, found[0].second), (gid(1), gid(2)));
    assert_eq!(found[0].rule, SodRuleId::from_u128(1));
    policy.revoke(gid(2), p(ALICE), t(NOW - 1)).unwrap();
    assert!(policy.violations(t(NOW), &world.org).is_empty());
}

#[test]
fn std_separation_of_duties_on_one_record() {
    let (mut policy, world) = (Policy::new(), World::new());
    let create = cap(Action::RecordCreate);
    let approve = cap(Action::ProcessApprove);
    policy
        .add_rule(rule(
            1,
            create.clone(),
            approve.clone(),
            SodKind::SameRecord,
        ))
        .unwrap();
    // Holding both is fine …
    give(&mut policy, &world, grant(1, BOB, create.clone(), ORDERS));
    give(&mut policy, &world, grant(2, BOB, approve.clone(), ORDERS));
    let record = order(1, 10, Some(BOB), None);
    let decide = |prior: &[Capability]| {
        policy.decide(
            &Request {
                actor: Actor::direct(p(BOB)),
                capability: &approve,
                on: ORDERS,
                record: Some(&record),
                prior,
                now: t(NOW),
            },
            &world.env(),
        )
    };
    assert!(decide(&[]).allowed);
    assert!(decide(&[read()]).allowed);
    // … but not approving one's own request.
    let decision = decide(&[create.clone()]);
    assert!(!decision.allowed);
    assert_eq!(
        decision.chain.len(),
        1,
        "the grant still applies; the rule forbids"
    );
    assert_eq!(
        reasons(&decision),
        [Reason::SeparationOfDuties {
            rule: SodRuleId::from_u128(1)
        }]
    );
    assert!(decision.fields.is_none());
}

#[test]
fn std_administering_access() {
    let (mut policy, world) = (Policy::new(), World::new());
    let env = world.env();
    give(
        &mut policy,
        &world,
        grant(1, ALICE, cap(Action::AccessManage), WS1),
    );
    let mut to_bob = grant(2, BOB, read(), ORDERS);
    policy.give_as(p(ALICE), to_bob.clone(), &env).unwrap();
    assert!(allowed(&policy, &world, BOB, &read(), ORDERS, None));
    // Not outside where Alice manages access.
    to_bob.id = gid(3);
    to_bob.on = OTHER;
    assert!(matches!(
        policy.give_as(p(ALICE), to_bob.clone(), &env),
        Err(PermError::NotAllowed(_))
    ));
    // The grant must name its giver.
    to_bob.on = ORDERS;
    to_bob.granted_by = p(CAROL);
    assert!(matches!(
        policy.give_as(p(ALICE), to_bob.clone(), &env),
        Err(PermError::NotAllowed(_))
    ));
    // Nobody gives access to themselves, nor to a group they belong to.
    give(
        &mut policy,
        &world,
        grant(4, BOB, cap(Action::AccessManage), TENANT),
    );
    let mut to_self = grant(5, BOB, cap(Action::RecordDelete), ORDERS);
    to_self.granted_by = p(BOB);
    assert_eq!(
        policy.give_as(p(BOB), to_self.clone(), &env),
        Err(PermError::NotAllowed(
            "nobody can give access to themselves".into()
        ))
    );
    to_self.principal = p(EMEA);
    assert!(policy.give_as(p(BOB), to_self.clone(), &env).is_err());
    to_self.principal = p(DAVE);
    policy.give_as(p(BOB), to_self, &env).unwrap();
    // Ids are unique.
    assert!(matches!(
        policy.give(grant(1, CAROL, read(), ORDERS), &world.org),
        Err(PermError::Duplicate { what: "grant", .. })
    ));
}

#[test]
fn std_row_filters_agree_with_decisions() {
    let (mut policy, world) = (Policy::new(), World::new());
    let env = world.env();
    let filter = |policy: &Policy, actor: Actor, capability: &Capability| {
        policy.row_filter(actor, capability, ORDERS, t(NOW), &env)
    };
    assert!(filter(&policy, Actor::direct(p(BOB)), &read()).admits_nothing());
    let mut own = grant(1, BOB, read(), ORDERS);
    own.scope = Scope::Own;
    give(&mut policy, &world, own);
    let mut small = grant(2, BOB, read(), ORDERS);
    small.condition = Some(up_to(100));
    give(&mut policy, &world, small);
    give(&mut policy, &world, grant(3, CAROL, read(), ORDERS));
    give(&mut policy, &world, grant(4, AGENT, read(), TENANT));
    let bob = filter(&policy, Actor::direct(p(BOB)), &read());
    assert!(!bob.admits_nothing() && !bob.admits_everything());
    assert_eq!(bob.parts.len(), 1);
    assert_eq!(bob.parts[0].rules.len(), 2);
    assert_eq!(bob.table, Some(ORDERS_T));
    let carol = filter(&policy, Actor::direct(p(CAROL)), &read());
    assert!(carol.admits_everything() && carol.parts[0].rules.is_empty());
    let agent = filter(&policy, Actor::agent(p(AGENT), p(BOB)), &read());
    assert_eq!(agent.parts.len(), 2);
    let records = [
        order(1, 50, Some(BOB), None),
        order(2, 500, Some(BOB), None),
        order(3, 50, Some(CAROL), None),
        order(4, 500, Some(CAROL), None),
        order(5, 500, None, None),
    ];
    for actor in [
        Actor::direct(p(BOB)),
        Actor::direct(p(CAROL)),
        Actor::direct(p(DAVE)),
        Actor::agent(p(AGENT), p(BOB)),
        Actor::agent(p(AGENT), p(CAROL)),
    ] {
        let rows = filter(&policy, actor, &read());
        for record in &records {
            let decided = ask(&policy, &world, actor, &read(), ORDERS, Some(record)).allowed;
            assert_eq!(
                rows.admits(record, &env),
                decided,
                "{actor:?} on {:?}",
                record.id
            );
        }
    }
    let admitted: Vec<_> = records
        .iter()
        .filter(|r| bob.admits(*r, &env))
        .map(|r| r.id.as_u128())
        .collect();
    assert_eq!(admitted, [1, 2, 3]);
    // Actions that are not about records: everything, if allowed at all.
    give(
        &mut policy,
        &world,
        grant(5, BOB, cap(Action::SchemaManage), BASE1),
    );
    assert!(filter(&policy, Actor::direct(p(BOB)), &cap(Action::SchemaManage)).admits_everything());
    assert!(filter(&policy, Actor::direct(p(CAROL)), &cap(Action::SchemaManage)).admits_nothing());
    let json = serde_json::to_string(&bob).unwrap();
    assert_eq!(
        serde_json::from_str::<mt_perm::RowFilter>(&json).unwrap(),
        bob
    );
}

fn permissions(
    policy: &Policy,
    world: &World,
    actor: Actor,
    record: Option<&Rec>,
) -> Vec<(u128, Visibility, bool)> {
    let fields = order_fields();
    let found = policy.field_permissions(
        actor,
        ORDERS,
        record.map(|r| r as &dyn mt_perm::RecordFacts),
        t(NOW),
        &world.env(),
        &fields,
    );
    assert_eq!(found.len(), fields.len());
    fields
        .iter()
        .map(|meta| {
            let FieldPermission {
                visibility,
                editable,
            } = found[&meta.id];
            (meta.id.as_u128(), visibility, editable)
        })
        .collect()
}

#[test]
fn std_field_permissions() {
    use Visibility::{Hidden, Masked, Visible};
    let (mut policy, world) = (Policy::new(), World::new());
    let record = order(1, 10, Some(BOB), None);
    // Nothing granted: every field hidden.
    for (_, visibility, editable) in
        permissions(&policy, &world, Actor::direct(p(BOB)), Some(&record))
    {
        assert_eq!((visibility, editable), (Hidden, false));
    }
    let mut reader = grant(1, BOB, cap(Action::RecordUpdate), ORDERS);
    reader.fields = FieldAccess::except([f(STATUS)]);
    give(&mut policy, &world, reader);
    give(&mut policy, &world, grant(2, BOB, read(), ORDERS));
    assert_eq!(
        permissions(&policy, &world, Actor::direct(p(BOB)), Some(&record)),
        [
            (AMOUNT, Visible, true),
            (OWNER, Visible, true),
            // Readable through the second grant, not editable through the first.
            (STATUS, Visible, false),
            // Masked for whoever may not unmask; masked values are not edited.
            (NOTE, Masked, false),
            // Classified fields need clearance.
            (SALARY, Hidden, false),
            (EMAIL, Hidden, false),
            (SECRET, Hidden, false),
            // Generated values are never edited.
            (TOTAL, Visible, false),
            // A field group's capability protects it.
            (REQUESTER, Hidden, false),
        ]
    );
    // Clearances, unmasking and the group's capabilities.
    for (id, capability) in [
        (3, cap(Action::DataConfidential)),
        (4, cap(Action::DataPersonal)),
        (5, cap(Action::DataUnmask)),
        (6, custom("salary.view")),
    ] {
        give(&mut policy, &world, grant(id, BOB, capability, ORDERS));
    }
    assert_eq!(
        permissions(&policy, &world, Actor::direct(p(BOB)), Some(&record)),
        [
            (AMOUNT, Visible, true),
            (OWNER, Visible, true),
            (STATUS, Visible, false),
            (NOTE, Visible, true),
            (SALARY, Visible, true),
            (EMAIL, Visible, true),
            (SECRET, Hidden, false),
            (TOTAL, Visible, false),
            (REQUESTER, Visible, false),
        ]
    );
    give(
        &mut policy,
        &world,
        grant(7, BOB, cap(Action::DataSensitive), ORDERS),
    );
    give(
        &mut policy,
        &world,
        grant(8, BOB, custom("salary.edit"), ORDERS),
    );
    let all = permissions(&policy, &world, Actor::direct(p(BOB)), Some(&record));
    assert_eq!(all[6], (SECRET, Visible, true));
    assert_eq!(all[8], (REQUESTER, Visible, true));
    // An agent acting for Bob, allowed to read everything but change nothing.
    give(&mut policy, &world, grant(9, AGENT, read(), TENANT));
    for capability in [
        cap(Action::DataConfidential),
        cap(Action::DataPersonal),
        cap(Action::DataSensitive),
        cap(Action::DataUnmask),
        custom("salary.view"),
    ] {
        let id = 10 + policy.grants().count() as u128;
        give(&mut policy, &world, grant(id, AGENT, capability, ORDERS));
    }
    for (id, visibility, editable) in permissions(
        &policy,
        &world,
        Actor::agent(p(AGENT), p(BOB)),
        Some(&record),
    ) {
        let expected = if id == STATUS { Visible } else { Visible };
        assert_eq!((visibility, editable), (expected, false), "{id}");
    }
    // The agent alone without unmasking for Carol, who may read but holds no clearance.
    give(&mut policy, &world, grant(40, CAROL, read(), ORDERS));
    let for_carol = permissions(
        &policy,
        &world,
        Actor::agent(p(AGENT), p(CAROL)),
        Some(&record),
    );
    assert_eq!(for_carol[3], (NOTE, Masked, false));
    assert_eq!(for_carol[4], (SALARY, Hidden, false));
    assert_eq!(for_carol[0], (AMOUNT, Visible, false));
    // Table level (no record): the same rules.
    let table = permissions(&policy, &world, Actor::direct(p(CAROL)), None);
    assert_eq!(table[0], (AMOUNT, Visible, false));
}

#[test]
fn std_what_a_principal_holds() {
    let (mut policy, world) = (Policy::new(), World::new());
    give(
        &mut policy,
        &world,
        grant(1, FINANCE, cap(Action::RecordUpdate), BASE1),
    );
    give(
        &mut policy,
        &world,
        grant(2, BOB, cap(Action::SchemaManage), ORDERS),
    );
    give(
        &mut policy,
        &world,
        grant(3, BOB, cap(Action::AuditRead), OTHER),
    );
    let mut ended = grant(4, BOB, cap(Action::RecordDelete), ORDERS);
    ended.valid_to = Some(t(1));
    give(&mut policy, &world, ended);
    let held = policy.capabilities(p(BOB), ORDERS, t(NOW), &world.org);
    assert_eq!(
        held,
        [
            read(),
            cap(Action::RecordUpdate),
            cap(Action::ViewManage),
            cap(Action::SchemaManage)
        ]
        .into_iter()
        .collect()
    );
    let in_base = policy.capabilities(p(BOB), BASE1, t(NOW), &world.org);
    assert_eq!(
        in_base,
        [read(), cap(Action::RecordUpdate)].into_iter().collect()
    );
    assert!(policy
        .capabilities(p(ALICE), ORDERS, t(NOW), &world.org)
        .is_empty());
}

#[test]
fn std_the_policy_is_stored_and_read_back() {
    let (mut policy, world) = (Policy::new(), World::new());
    policy.add_set(set(1, "Reader", &[read()])).unwrap();
    policy
        .add_rule(rule(1, custom("a.b"), custom("c.d"), SodKind::Hold))
        .unwrap();
    give(&mut policy, &world, set_grant(1, BOB, 1, ORDERS));
    let mut limited = grant(2, CAROL, cap(Action::ProcessApprove), ORDERS);
    limited.condition = Some(up_to(5));
    give(&mut policy, &world, limited);
    policy.revoke(gid(1), p(ALICE), t(NOW)).unwrap();
    let json = serde_json::to_string(&policy).unwrap();
    let loaded: Policy = serde_json::from_str(&json).unwrap();
    assert_eq!(loaded, policy);
    // The indexes are rebuilt: decisions are the same.
    assert_eq!(loaded.grants_of(p(CAROL)).count(), 1);
    let approve = cap(Action::ProcessApprove);
    let record = order(1, 3, None, None);
    assert_eq!(
        ask(
            &loaded,
            &world,
            Actor::direct(p(CAROL)),
            &approve,
            ORDERS,
            Some(&record)
        ),
        ask(
            &policy,
            &world,
            Actor::direct(p(CAROL)),
            &approve,
            ORDERS,
            Some(&record)
        )
    );
    assert!(serde_json::from_str::<Policy>(r#"{"grants": 1}"#).is_err());
}

#[test]
fn std_reasons_read_well() {
    let cases = [
        (Reason::UnknownPrincipal, "the principal does not exist"),
        (Reason::NoGrant, "no grant gives this capability"),
        (
            Reason::Inactive {
                why: Inactive::Revoked,
            },
            "the grant was revoked",
        ),
        (
            Reason::Inactive {
                why: Inactive::NotYetValid,
            },
            "the grant is not valid yet",
        ),
        (
            Reason::Inactive {
                why: Inactive::Expired,
            },
            "the grant has expired",
        ),
        (Reason::Elsewhere, "the grant was given somewhere else"),
        (
            Reason::OutOfScope,
            "the record is outside the grant's scope",
        ),
        (
            Reason::ConditionNotMet,
            "the record does not satisfy the grant's condition",
        ),
        (Reason::Administrative, "guests cannot administer"),
        (
            Reason::SeparationOfDuties {
                rule: SodRuleId::from_u128(1),
            },
            "separation of duties: the same principal already did the paired action on this record",
        ),
    ];
    for (reason, text) in cases {
        assert_eq!(reason.to_string(), text);
        let json = serde_json::to_string(&reason).unwrap();
        assert_eq!(serde_json::from_str::<Reason>(&json).unwrap(), reason);
    }
    let _ = Granted::Capability(read());
}

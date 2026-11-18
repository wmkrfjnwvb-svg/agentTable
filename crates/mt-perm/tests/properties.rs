//! Properties of the decision engine over random policies, records and actors: row filters
//! agree with decisions, agents never exceed the person they act for, adding a grant never
//! takes access away and revoking one never gives it, every refusal is explained, and the
//! policy survives storage. Plus thread-safety.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_core::id::{RecordId, RefSetId};
use mt_perm::{Action, Actor, Capability, FieldAccess, Policy, Scope};
use proptest::prelude::*;

const PRINCIPALS: &[u128] = &[ALICE, BOB, CAROL, DAVE, GUEST, AGENT, FINANCE, EMEA];
const CAPABILITIES: &[Action] = &[
    Action::RecordRead,
    Action::RecordUpdate,
    Action::ProcessApprove,
    Action::SchemaManage,
];

fn scope() -> impl Strategy<Value = Scope> {
    prop_oneof![
        3 => Just(Scope::All),
        2 => Just(Scope::Own),
        2 => Just(Scope::Team),
        1 => Just(Scope::Where { condition: mine() }),
        1 => (1_u128..5).prop_map(|root| Scope::RefSubtree { field: f(REGION), set: RefSetId::from_u128(REGIONS), root: item(root) }),
        1 => prop::collection::btree_set(1_u128..6, 1..3).prop_map(|ids| Scope::Records { records: ids.into_iter().map(RecordId::from_u128).collect() }),
    ]
}

#[derive(Debug, Clone)]
struct Spec {
    to: u128,
    action: Action,
    on: u8,
    scope: Scope,
    limit: Option<i64>,
    fields: Option<Vec<u128>>,
    ends: Option<i64>,
}

fn spec() -> impl Strategy<Value = Spec> {
    (
        prop::sample::select(PRINCIPALS),
        prop::sample::select(CAPABILITIES),
        0_u8..4,
        scope(),
        prop::option::of(0_i64..200),
        prop::option::of(prop::collection::vec(
            prop::sample::select(&[AMOUNT, OWNER, REGION, STATUS][..]),
            1..3,
        )),
        prop::option::of(-10_i64..10),
    )
        .prop_map(|(to, action, on, scope, limit, fields, ends)| Spec {
            to,
            action,
            on,
            scope,
            limit,
            fields,
            ends,
        })
}

/// Builds the policy from specs, skipping those the engine refuses (guests and admin rights).
fn policy(specs: &[Spec]) -> Policy {
    let org = Org::new();
    let mut policy = Policy::new();
    for (index, spec) in specs.iter().enumerate() {
        let on = match spec.on {
            0 => TENANT,
            1 => BASE1,
            2 => ORDERS,
            _ => PAYMENTS,
        };
        let mut g = grant(index as u128 + 1, spec.to, spec.action.into(), on);
        let record_scoped = spec.action.is_record_scoped();
        if record_scoped && on.table().is_some() {
            g.scope = spec.scope.clone();
            g.condition = spec.limit.map(up_to);
            if let Some(fields) = &spec.fields {
                g.fields = FieldAccess::only(fields.iter().map(|n| f(*n)));
            }
        } else if record_scoped && matches!(spec.scope, Scope::Own | Scope::Team) {
            g.scope = spec.scope.clone();
        }
        g.valid_to = spec.ends.map(|d| t(NOW + d));
        let _ = policy.give(g, &org);
    }
    policy
}

fn records() -> Vec<Rec> {
    vec![
        order(1, 50, Some(BOB), Some(DE)),
        order(2, 150, Some(CAROL), Some(JP)),
        order(3, 10, Some(DAVE), Some(FR)),
        order(4, 199, None, None),
        order(5, 0, Some(GUEST), Some(APAC)),
    ]
}

fn actors() -> Vec<Actor> {
    let mut out: Vec<Actor> = [ALICE, BOB, CAROL, DAVE, GUEST, AGENT]
        .iter()
        .map(|n| Actor::direct(p(*n)))
        .collect();
    out.push(Actor::agent(p(AGENT), p(BOB)));
    out.push(Actor::agent(p(AGENT), p(CAROL)));
    out
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, ..ProptestConfig::default() })]

    /// A list never shows a record the actor could not open, and shows every one they could.
    #[test]
    fn prop_row_filters_agree_with_decisions(specs in prop::collection::vec(spec(), 0..10)) {
        let world = World::new();
        let env = world.env();
        let policy = policy(&specs);
        for actor in actors() {
            for action in CAPABILITIES {
                let capability: Capability = (*action).into();
                let filter = policy.row_filter(actor, &capability, ORDERS, t(NOW), &env);
                for record in records() {
                    let decided = ask(&policy, &world, actor, &capability, ORDERS, Some(&record)).allowed;
                    prop_assert_eq!(filter.admits(&record, &env), decided, "{:?} {} {:?}", actor, capability, record.id);
                }
            }
        }
    }

    /// An agent acting for a person is allowed only what both are allowed, and sees only the
    /// fields both may see.
    #[test]
    fn prop_agents_never_exceed_the_person(specs in prop::collection::vec(spec(), 0..10)) {
        let world = World::new();
        let policy = policy(&specs);
        for person in [BOB, CAROL, DAVE, GUEST] {
            for action in CAPABILITIES {
                let capability: Capability = (*action).into();
                for record in records() {
                    let agent = ask(&policy, &world, Actor::agent(p(AGENT), p(person)), &capability, ORDERS, Some(&record));
                    let alone = ask(&policy, &world, Actor::direct(p(AGENT)), &capability, ORDERS, Some(&record));
                    let human = ask(&policy, &world, Actor::direct(p(person)), &capability, ORDERS, Some(&record));
                    prop_assert_eq!(agent.allowed, alone.allowed && human.allowed);
                    if agent.allowed {
                        for field in [AMOUNT, OWNER, REGION, STATUS, SALARY] {
                            let fid = f(field);
                            prop_assert_eq!(agent.fields.allows(fid), alone.fields.allows(fid) && human.fields.allows(fid));
                        }
                    } else {
                        prop_assert!(!agent.refusals.is_empty(), "every refusal is explained");
                    }
                }
            }
        }
    }

    /// Grants only ever add access: one more grant never refuses what was allowed, and
    /// revoking one never allows what was refused.
    #[test]
    fn prop_access_is_monotonic(specs in prop::collection::vec(spec(), 1..10), extra in spec()) {
        let world = World::new();
        let before = policy(&specs);
        let mut with_extra = specs.clone();
        with_extra.push(extra);
        let after = policy(&with_extra);
        let mut revoked = before.clone();
        revoked.revoke(gid(1), p(ALICE), t(0)).ok();
        for actor in actors() {
            for action in CAPABILITIES {
                let capability: Capability = (*action).into();
                for record in records() {
                    let was = ask(&before, &world, actor, &capability, ORDERS, Some(&record)).allowed;
                    let now = ask(&after, &world, actor, &capability, ORDERS, Some(&record)).allowed;
                    let less = ask(&revoked, &world, actor, &capability, ORDERS, Some(&record)).allowed;
                    prop_assert!(!was || now, "a grant took access away");
                    prop_assert!(was || !less, "a revocation gave access");
                }
            }
        }
    }

    /// The stored policy decides exactly like the original.
    #[test]
    fn prop_the_policy_survives_storage(specs in prop::collection::vec(spec(), 0..10)) {
        let world = World::new();
        let policy = policy(&specs);
        let loaded: Policy = serde_json::from_str(&serde_json::to_string(&policy).unwrap()).unwrap();
        prop_assert_eq!(&loaded, &policy);
        for actor in actors() {
            for record in records() {
                prop_assert_eq!(
                    ask(&loaded, &world, actor, &read(), ORDERS, Some(&record)),
                    ask(&policy, &world, actor, &read(), ORDERS, Some(&record))
                );
            }
        }
    }
}

#[test]
fn concurrency_one_policy_answers_many_threads() {
    fn shareable<T: Send + Sync>() {}
    shareable::<Policy>();
    shareable::<mt_perm::Decision>();
    shareable::<mt_perm::AccessReview>();
    let world = World::new();
    let mut policy = Policy::new();
    let org = Org::new();
    let mut own = grant(1, EMEA, read(), ORDERS);
    own.scope = Scope::Team;
    policy.give(own, &org).unwrap();
    let mut limited = grant(2, FINANCE, cap(Action::ProcessApprove), ORDERS);
    limited.condition = Some(up_to(100));
    policy.give(limited, &org).unwrap();
    let records = records();
    let expected: Vec<bool> = records
        .iter()
        .map(|r| {
            ask(
                &policy,
                &world,
                Actor::direct(p(BOB)),
                &read(),
                ORDERS,
                Some(r),
            )
            .allowed
        })
        .collect();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    let world = World::new();
                    (0..500)
                        .map(|i| {
                            let r = &records[i % records.len()];
                            (
                                i % records.len(),
                                ask(
                                    &policy,
                                    &world,
                                    Actor::direct(p(BOB)),
                                    &read(),
                                    ORDERS,
                                    Some(r),
                                )
                                .allowed,
                            )
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for handle in handles {
            for (index, allowed) in handle.join().unwrap() {
                assert_eq!(allowed, expected[index]);
            }
        }
    });
    let _ = world;
}

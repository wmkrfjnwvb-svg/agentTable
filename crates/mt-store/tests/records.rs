//! Records in physical tables and the journal, on every engine.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_cluster::NodeId;
use mt_core::decimal::Decimal;
use mt_core::id::{BaseId, FieldId, PrincipalId, RecordId, RefItemId, TableId};
use mt_core::money::{Currency, Money};
use mt_core::sortkey::compare_values;
use mt_core::time::{Date, DurationMs, Timestamp};
use mt_core::value::Value;
use mt_ops::{Accepted, BaseState, Batch, ClientId, ClientOpId, Op, Outcome};
use mt_store::{StoreError, Tenant};
use std::collections::BTreeMap;
use std::sync::Arc;

const ALICE: PrincipalId = PrincipalId::from_u128(1);

fn base() -> BaseId {
    BaseId::from_u128(1)
}

fn tbl(n: u128) -> TableId {
    TableId::from_u128(n)
}

fn rec(n: u128) -> RecordId {
    RecordId::from_u128(n)
}

fn fld(n: u128) -> FieldId {
    FieldId::from_u128(n)
}

fn dec(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn batch(seq: u64, ops: Vec<Op>) -> Batch {
    Batch {
        id: ClientOpId {
            client: ClientId(1),
            seq,
        },
        base: base(),
        ops,
    }
}

fn set(t: u128, r: u128, f: u128, value: Value) -> Op {
    Op::Set {
        table: tbl(t),
        record: rec(r),
        field: fld(f),
        value,
        expected: 0,
    }
}

/// Decides a batch on the owner and commits it.
async fn apply(owner: &mut BaseState, t: &Tenant, b: Batch) -> Accepted {
    let Outcome::Accepted(accepted) = owner.step(&b).outcome else {
        panic!("refused")
    };
    t.prepare(base(), &accepted).await.unwrap();
    t.commit(base(), &accepted, ALICE, NOW, None).await.unwrap();
    accepted
}

fn every_kind_of_value() -> Vec<Value> {
    vec![
        Value::Text("Zürich — 東京 \"quoted\" ' 😀".to_owned()),
        Value::LongText("line 1\nline 2\n".repeat(500)),
        Value::Number(dec("-12345678901234567890.123456")),
        Value::Money(Money::new(dec("1234.50"), Currency::new("EUR").unwrap()).unwrap()),
        Value::Percent(dec("0.08")),
        Value::Bool(true),
        Value::Date(Date::from_days(20_000).unwrap()),
        Value::DateTime(Timestamp::from_millis(NOW).unwrap()),
        Value::Duration(DurationMs::from_millis(-90_000)),
        Value::Reference(RefItemId::from_u128(7)),
        Value::MultiReference(vec![RefItemId::from_u128(1), RefItemId::from_u128(2)]),
        Value::Link(vec![rec(1), rec(2)]),
        Value::Principal(vec![ALICE]),
        Value::Json(serde_json::json!({"nested": [1, true, null, "x"]})),
    ]
}

#[tokio::test]
async fn std_committed_batches_load_back_exactly() {
    on_every_db(|store| async move {
        let t = store.create_tenant(tenant(1), "Acme", NOW).await.unwrap();
        let mut owner = BaseState::new(base());
        let values = every_kind_of_value();
        let cells: BTreeMap<FieldId, Value> = values
            .iter()
            .enumerate()
            .map(|(i, v)| (fld(i as u128 + 1), v.clone()))
            .collect();
        let mut accepted = vec![
            apply(
                &mut owner,
                &t,
                batch(
                    1,
                    vec![Op::Create {
                        table: tbl(1),
                        record: rec(1),
                        cells: cells.clone(),
                    }],
                ),
            )
            .await,
        ];
        accepted.push(
            apply(
                &mut owner,
                &t,
                batch(
                    2,
                    vec![
                        Op::Create {
                            table: tbl(1),
                            record: rec(2),
                            cells: BTreeMap::new(),
                        },
                        set(1, 2, 3, Value::Text("new field later".to_owned())),
                        set(1, 1, 1, Value::Null),
                        Op::Create {
                            table: tbl(2),
                            record: rec(1),
                            cells: BTreeMap::from([(fld(99), Value::Bool(false))]),
                        },
                    ],
                ),
            )
            .await,
        );
        accepted.push(
            apply(
                &mut owner,
                &t,
                batch(
                    3,
                    vec![Op::Delete {
                        table: tbl(1),
                        record: rec(2),
                        expected: 2,
                    }],
                ),
            )
            .await,
        );
        accepted.push(
            apply(
                &mut owner,
                &t,
                batch(
                    4,
                    vec![
                        Op::Restore {
                            table: tbl(1),
                            record: rec(2),
                        },
                        set(1, 1, 2, Value::Null),
                    ],
                ),
            )
            .await,
        );
        let (seq, records) = t.load_base(base()).await.unwrap();
        assert_eq!(seq, 4, "{}", name(store.dialect()));
        let expected: BTreeMap<_, _> = owner.records().map(|(k, v)| (*k, v.clone())).collect();
        assert_eq!(
            records,
            expected,
            "{}: every value, version and flag",
            name(store.dialect())
        );
        // The journal gives back exactly what was accepted.
        assert_eq!(t.journal(base(), 0, 100).await.unwrap(), accepted);
        assert_eq!(
            t.journal(base(), 2, 100).await.unwrap(),
            accepted[2..].to_vec()
        );
        assert_eq!(
            t.journal(base(), 0, 1).await.unwrap(),
            accepted[..1].to_vec()
        );
        assert_eq!(
            t.journal_tail(base(), 2).await.unwrap(),
            accepted[2..].to_vec()
        );
        // A restarted owner continues: recognises old batches, numbers on.
        let mut restarted = BaseState::from_records(base(), seq, records);
        for a in t.journal_tail(base(), 100).await.unwrap() {
            restarted.recall(Outcome::Accepted(a));
        }
        assert!(restarted.step(&batch(4, vec![])).repeated);
        let Outcome::Accepted(fifth) = restarted
            .step(&batch(5, vec![set(1, 1, 3, Value::Bool(true))]))
            .outcome
        else {
            panic!()
        };
        assert_eq!(fifth.seq, 5);
        // Another base of the tenant is separate.
        assert_eq!(
            t.load_base(BaseId::from_u128(2)).await.unwrap(),
            (0, BTreeMap::new())
        );
        store
    })
    .await;
}

#[tokio::test]
async fn edge_a_superseded_owner_cannot_commit() {
    on_every_db(|store| async move {
        let t = store.create_tenant(tenant(1), "Acme", NOW).await.unwrap();
        let resource = format!("base:{}", base()).into_bytes();
        let a = t
            .acquire(&resource, &NodeId::new(1, 1, "a:1"), None)
            .await
            .unwrap();
        let mut owner = BaseState::new(base());
        let Outcome::Accepted(first) = owner
            .step(&batch(
                1,
                vec![Op::Create {
                    table: tbl(1),
                    record: rec(1),
                    cells: BTreeMap::new(),
                }],
            ))
            .outcome
        else {
            panic!()
        };
        t.prepare(base(), &first).await.unwrap();
        t.commit(base(), &first, ALICE, NOW, Some(&a))
            .await
            .unwrap();
        // Node B takes the base over (A is partitioned or paused).
        let b = t
            .acquire(&resource, &NodeId::new(2, 1, "b:1"), None)
            .await
            .unwrap();
        let Outcome::Accepted(second) = owner
            .step(&batch(2, vec![set(1, 1, 1, Value::Bool(true))]))
            .outcome
        else {
            panic!()
        };
        t.prepare(base(), &second).await.unwrap();
        let refused = t.commit(base(), &second, ALICE, NOW, Some(&a)).await;
        assert!(
            matches!(refused, Err(StoreError::Fenced { current: 2 })),
            "{}: {refused:?}",
            name(store.dialect())
        );
        assert_eq!(t.last_seq(base()).await.unwrap(), 1, "nothing written");
        assert!(t.load_base(base()).await.unwrap().1[&(tbl(1), rec(1))]
            .cells
            .is_empty());
        // The new owner writes.
        t.commit(base(), &second, ALICE, NOW, Some(&b))
            .await
            .unwrap();
        // A token for a resource nobody acquired.
        let ghost = mt_cluster::FencingToken {
            resource: b"base:none".to_vec(),
            epoch: 1,
            owner: NodeId::new(3, 1, "c:1"),
        };
        assert!(matches!(
            t.commit(base(), &second, ALICE, NOW, Some(&ghost)).await,
            Err(StoreError::Fenced { current: 0 })
        ));
        store
    })
    .await;
}

#[tokio::test]
async fn invalid_commits_out_of_sequence_change_nothing() {
    on_every_db(|store| async move {
        let t = store.create_tenant(tenant(1), "Acme", NOW).await.unwrap();
        let mut owner = BaseState::new(base());
        let first = apply(
            &mut owner,
            &t,
            batch(
                1,
                vec![Op::Create {
                    table: tbl(1),
                    record: rec(1),
                    cells: BTreeMap::new(),
                }],
            ),
        )
        .await;
        // Again (a repeat) and a gap.
        assert!(matches!(
            t.commit(base(), &first, ALICE, NOW, None).await,
            Err(StoreError::Conflict(_))
        ));
        let gap = Accepted {
            seq: 3,
            ..first.clone()
        };
        assert!(matches!(
            t.commit(base(), &gap, ALICE, NOW, None).await,
            Err(StoreError::Conflict(_))
        ));
        // Effects that do not fit the rows: the whole batch is refused.
        let unfit = Accepted {
            seq: 2,
            effects: vec![
                mt_ops::Effect::Deleted {
                    table: tbl(1),
                    record: rec(1),
                },
                mt_ops::Effect::Deleted {
                    table: tbl(1),
                    record: rec(9),
                },
            ],
            id: ClientOpId {
                client: ClientId(1),
                seq: 2,
            },
        };
        let refused = t.commit(base(), &unfit, ALICE, NOW, None).await;
        assert!(
            matches!(&refused, Err(StoreError::Conflict(m)) if m.contains("does not fit")),
            "{refused:?}"
        );
        let (seq, records) = t.load_base(base()).await.unwrap();
        assert_eq!(seq, 1);
        assert!(
            !records[&(tbl(1), rec(1))].deleted,
            "the first effect was rolled back too"
        );
        // Columns are made before a write that needs them.
        assert!(matches!(
            t.ensure_field(tbl(7), fld(1)).await,
            Err(StoreError::NotFound(_))
        ));
        t.ensure_table(base(), tbl(7)).await.unwrap();
        t.ensure_table(base(), tbl(7)).await.unwrap();
        t.ensure_field(tbl(7), fld(1)).await.unwrap();
        t.ensure_field(tbl(7), fld(1)).await.unwrap();
        store
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrency_one_batch_committed_from_many_tasks_lands_once() {
    on_every_db(|store| async move {
        let t = Arc::new(store.create_tenant(tenant(1), "Acme", NOW).await.unwrap());
        let mut owner = BaseState::new(base());
        let Outcome::Accepted(first) = owner
            .step(&batch(
                1,
                vec![Op::Create {
                    table: tbl(1),
                    record: rec(1),
                    cells: BTreeMap::from([(fld(1), Value::Bool(true))]),
                }],
            ))
            .outcome
        else {
            panic!()
        };
        t.prepare(base(), &first).await.unwrap();
        let handles: Vec<_> = (0..6)
            .map(|_| {
                let t = t.clone();
                let first = first.clone();
                tokio::spawn(
                    async move { t.commit(base(), &first, ALICE, NOW, None).await.is_ok() },
                )
            })
            .collect();
        let mut ok = 0;
        for h in handles {
            ok += usize::from(h.await.unwrap());
        }
        assert_eq!(ok, 1, "{}", name(store.dialect()));
        assert_eq!(t.journal(base(), 0, 10).await.unwrap().len(), 1);
        store
    })
    .await;
}

#[tokio::test]
async fn std_the_database_sorts_by_stored_keys_like_the_engine() {
    on_every_db(|store| async move {
        let t = store.create_tenant(tenant(1), "Acme", NOW).await.unwrap();
        let mut owner = BaseState::new(base());
        let numbers = [
            "10",
            "-3.5",
            "2",
            "0",
            "1e3",
            "-1000000",
            "0.001",
            "2.00",
            "999999999999.99",
        ];
        let texts = ["b", "B", "a", "ä", "Zebra", "zebra", "", "10", "9", "æ"];
        let mut ops = Vec::new();
        for (i, n) in numbers.iter().enumerate() {
            let mut cells = BTreeMap::from([(
                fld(1),
                Value::Number(n.parse::<Decimal>().unwrap_or_else(|_| dec("1000"))),
            )]);
            if let Some(text) = texts.get(i) {
                cells.insert(fld(2), Value::Text((*text).to_owned()));
            }
            ops.push(Op::Create {
                table: tbl(1),
                record: rec(i as u128 + 1),
                cells,
            });
        }
        ops.push(Op::Create {
            table: tbl(1),
            record: rec(50),
            cells: BTreeMap::new(),
        });
        let accepted = apply(&mut owner, &t, batch(1, ops)).await;
        let _ = accepted;
        for field in [fld(1), fld(2)] {
            let mut expected: Vec<(Option<Value>, RecordId)> = owner
                .records()
                .map(|((_, r), state)| (state.cells.get(&field).map(|c| c.value.clone()), *r))
                .collect();
            expected.sort_by(|(a, ra), (b, rb)| match (a, b) {
                (Some(a), Some(b)) => compare_values(a, b).then(ra.cmp(rb)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => ra.cmp(rb),
            });
            let expected: Vec<RecordId> = expected.into_iter().map(|(_, r)| r).collect();
            assert_eq!(
                t.ids_by(tbl(1), field).await.unwrap(),
                expected,
                "{}: {field}",
                name(store.dialect())
            );
        }
        store
    })
    .await;
}

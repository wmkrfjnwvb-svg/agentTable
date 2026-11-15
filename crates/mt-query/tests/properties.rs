//! Property tests of the evaluator and the aggregates, plus the concurrency checks.
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
use mt_core::decimal::Decimal;
use mt_core::field::DataType;
use mt_core::id::RecordId;
use mt_core::value::Value;
use mt_query::{
    aggregate, compare, matches, run, Aggregate, AggregateFn, DateBucket, Direction, Empties,
    Group, GroupKey, Query, QueryResult, SortKey,
};
use proptest::prelude::*;
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::thread;

/// Small domains, so that ties, empties and shared groups are common.
fn arb_row(id: u128) -> impl Strategy<Value = Rec> {
    (
        proptest::option::of("[a-cA-C]{0,2}"),
        proptest::option::of(-3_i64..4),
        proptest::option::of(proptest::sample::select(vec![LEAD, QUALIFIED, WON, LOST])),
        proptest::option::of(0_i32..70),
        proptest::option::of(any::<bool>()),
        proptest::option::of(proptest::collection::vec(1_u128..4, 0..3)),
        proptest::option::of((1_i64..500, any::<bool>())),
    )
        .prop_map(move |(title, qty, stage_item, close, done, tags, amount)| {
            let mut cells = Vec::new();
            if let Some(title) = title {
                cells.push((TITLE, text(&title)));
            }
            if let Some(qty) = qty {
                cells.push((QTY, Value::Number(Decimal::from(qty))));
            }
            if let Some(stage_item) = stage_item {
                cells.push((STAGE, stage(stage_item)));
            }
            if let Some(close) = close {
                cells.push((
                    CLOSE,
                    Value::Date(day(2026, 4, 1).checked_add_days(close).unwrap()),
                ));
            }
            if let Some(done) = done {
                cells.push((DONE, Value::Bool(done)));
            }
            if let Some(tags) = tags {
                cells.push((
                    TAGS,
                    Value::MultiReference(tags.into_iter().map(item).collect()),
                ));
            }
            if let Some((cents, euro)) = amount {
                let currency = if euro { "EUR" } else { "USD" };
                cells.push((
                    AMOUNT,
                    money(&format!("{}.{:02}", cents / 100, cents % 100), currency),
                ));
            }
            row(id, &cells)
        })
}

fn arb_rows() -> impl Strategy<Value = Vec<Rec>> {
    (0_usize..30).prop_flat_map(|n| (1..=n).map(|i| arb_row(i as u128)).collect::<Vec<_>>())
}

fn arb_direction() -> impl Strategy<Value = Direction> {
    prop_oneof![Just(Direction::Asc), Just(Direction::Desc)]
}

fn arb_query() -> impl Strategy<Value = Query> {
    let sortable = vec![TITLE, QTY, STAGE, CLOSE, DONE, TAGS, AMOUNT];
    let groupable = vec![TITLE, QTY, STAGE, CLOSE, DONE, TAGS];
    let sort = proptest::sample::subsequence(sortable, 0..3).prop_flat_map(|fields| {
        fields
            .into_iter()
            .map(|field| {
                (
                    arb_direction(),
                    prop_oneof![
                        Just(Empties::Natural),
                        Just(Empties::First),
                        Just(Empties::Last)
                    ],
                )
                    .prop_map(move |(direction, empties)| SortKey {
                        field: fid(field),
                        direction,
                        empties,
                    })
            })
            .collect::<Vec<_>>()
    });
    let group = proptest::sample::subsequence(groupable, 0..3).prop_flat_map(|fields| {
        fields
            .into_iter()
            .map(|field| {
                (arb_direction(), any::<bool>()).prop_map(move |(direction, bucketed)| GroupKey {
                    field: fid(field),
                    direction,
                    bucket: (bucketed && field == CLOSE).then_some(DateBucket::Month),
                })
            })
            .collect::<Vec<_>>()
    });
    let filter = prop_oneof![
        Just(None),
        Just(Some(Condition::is_not_empty(fid(QTY)))),
        Just(Some(Condition::compare(
            fid(DONE),
            Operator::Ne,
            Value::Bool(true)
        ))),
        Just(Some(Condition::compare(fid(QTY), Operator::Ge, num("0")))),
    ];
    (filter, sort, group).prop_map(|(filter, sort, group)| Query {
        filter,
        sort,
        group,
        search: None,
        aggregates: vec![
            Aggregate {
                field: fid(QTY),
                function: AggregateFn::Sum,
            },
            Aggregate {
                field: fid(QTY),
                function: AggregateFn::CountFilled,
            },
            Aggregate {
                field: fid(TITLE),
                function: AggregateFn::CountEmpty,
            },
        ],
    })
}

/// The leaf rows of a group tree, in order.
fn leaves(groups: &[Group], out: &mut Vec<RecordId>) {
    for group in groups {
        if group.groups.is_empty() {
            out.extend(&group.rows);
        } else {
            assert!(group.rows.is_empty());
            leaves(&group.groups, out);
        }
    }
}

fn check_subtotals(
    groups: &[Group],
    parent_count: usize,
    parent_sum: &Option<Value>,
) -> Result<(), TestCaseError> {
    if groups.is_empty() {
        return Ok(());
    }
    // Counts and sums of the groups of one level add up to their parent's.
    prop_assert_eq!(groups.iter().map(|g| g.count).sum::<usize>(), parent_count);
    let mut sum = Decimal::ZERO;
    for group in groups {
        if let Some(Value::Number(n)) = &group.aggregates[0].value {
            sum += *n;
        }
        let inner = if group.groups.is_empty() {
            group.rows.len()
        } else {
            group.groups.iter().map(|g| g.count).sum()
        };
        prop_assert_eq!(inner, group.count);
        check_subtotals(&group.groups, group.count, &group.aggregates[0].value)?;
    }
    prop_assert_eq!(&Some(Value::Number(sum.normalize())), parent_sum);
    // No two groups of one level have the same key.
    let keys: BTreeSet<String> = groups.iter().map(|g| format!("{:?}", g.key)).collect();
    prop_assert_eq!(keys.len(), groups.len());
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 400, ..ProptestConfig::default() })]

    /// The result is exactly the matching records, each once, in `compare` order, and does
    /// not depend on the order the records were given in.
    #[test]
    fn result_is_the_sorted_selection(rows in arb_rows(), q in arb_query(), seed in any::<u64>()) {
        q.check(&type_of).unwrap();
        let result = run(&q, &rows, &ctx(), &Deals);
        let expected: BTreeSet<RecordId> = rows.iter().filter(|r| matches(&q, *r, &ctx(), &Deals)).map(|r| r.id).collect();
        prop_assert_eq!(result.rows.len(), expected.len());
        prop_assert_eq!(result.rows.iter().copied().collect::<BTreeSet<_>>(), expected);
        let by_id = |id: RecordId| rows.iter().find(|r| r.id == id).unwrap();
        for pair in result.rows.windows(2) {
            prop_assert_eq!(compare(&q, by_id(pair[0]), by_id(pair[1]), &ctx(), &Deals), Ordering::Less);
        }
        // Shuffle the input: same answer.
        let mut shuffled = rows.clone();
        let mut state = seed | 1;
        for i in (1..shuffled.len()).rev() {
            state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            shuffled.swap(i, usize::try_from(state >> 33).unwrap() % (i + 1));
        }
        prop_assert_eq!(run(&q, &shuffled, &ctx(), &Deals), result);
    }

    /// `compare` is a total order: antisymmetric, and equal only for the same record.
    #[test]
    fn compare_is_a_total_order(rows in arb_rows(), q in arb_query()) {
        for a in &rows {
            for b in &rows {
                let ab = compare(&q, a, b, &ctx(), &Deals);
                prop_assert_eq!(ab, compare(&q, b, a, &ctx(), &Deals).reverse());
                prop_assert_eq!(ab == Ordering::Equal, a.id == b.id);
            }
        }
    }

    /// Groups partition the result: their leaves, in order, are the result's rows; counts
    /// and sums of sub-groups add up; totals equal the aggregates over all rows.
    #[test]
    fn groups_partition_the_result(rows in arb_rows(), q in arb_query()) {
        let result = run(&q, &rows, &ctx(), &Deals);
        if q.group.is_empty() {
            prop_assert!(result.groups.is_empty());
        } else {
            let mut flat = Vec::new();
            leaves(&result.groups, &mut flat);
            prop_assert_eq!(&flat, &result.rows);
            check_subtotals(&result.groups, result.rows.len(), &result.totals[0].value)?;
        }
        // Totals agree with aggregating the selected rows directly.
        let selected: Vec<&Rec> = rows.iter().filter(|r| result.rows.contains(&r.id)).collect();
        let direct = aggregate(AggregateFn::Sum, DataType::Number, selected.iter().map(|r| r.cells.get(&fid(QTY))));
        prop_assert_eq!(&result.totals[0], &direct);
        let filled = aggregate(AggregateFn::CountFilled, DataType::Number, selected.iter().map(|r| r.cells.get(&fid(QTY))));
        prop_assert_eq!(&result.totals[1], &filled);
    }

    /// Pages tile the result.
    #[test]
    fn pages_tile_the_result(rows in arb_rows(), q in arb_query(), size in 1_usize..7) {
        let result = run(&q, &rows, &ctx(), &Deals);
        let mut collected = Vec::new();
        let mut offset = 0;
        loop {
            let page = result.page(offset, size);
            if page.is_empty() {
                break;
            }
            prop_assert!(page.len() <= size);
            collected.extend_from_slice(page);
            offset += size;
        }
        prop_assert_eq!(collected, result.rows);
    }

    /// Aggregates are consistent with each other.
    #[test]
    fn aggregates_are_consistent(values in proptest::collection::vec(proptest::option::of(-1000_i64..1000), 0..40)) {
        let cells: Vec<Option<Value>> = values.iter().map(|v| v.map(|n| Value::Number(Decimal::new(n, 1)))).collect();
        let run = |function| aggregate(function, DataType::Number, cells.iter().map(Option::as_ref)).value;
        let number = |value: Option<Value>| match value { Some(Value::Number(n)) => Some(n), _ => None };
        let n = Decimal::from(cells.len() as u64);
        let empty = number(run(AggregateFn::CountEmpty)).unwrap();
        let filled = number(run(AggregateFn::CountFilled)).unwrap();
        prop_assert_eq!(empty + filled, n);
        prop_assert!(number(run(AggregateFn::CountUnique)).unwrap() <= filled);
        let sum = number(run(AggregateFn::Sum)).unwrap();
        prop_assert_eq!(sum, values.iter().flatten().map(|v| Decimal::new(*v, 1)).sum::<Decimal>());
        match (number(run(AggregateFn::Min)), number(run(AggregateFn::Max)), number(run(AggregateFn::Average)), number(run(AggregateFn::Median)), number(run(AggregateFn::Range))) {
            (Some(min), Some(max), Some(average), Some(median), Some(range)) => {
                prop_assert!(min <= average && average <= max);
                prop_assert!(min <= median && median <= max);
                prop_assert_eq!(range, max - min);
                prop_assert_eq!((average * filled).round_dp(6), sum.round_dp(6));
            }
            (None, None, None, None, None) => prop_assert_eq!(filled, Decimal::ZERO),
            other => prop_assert!(false, "inconsistent aggregates: {:?}", other),
        }
        // Shares add up to one when there are records.
        let share = |function| match run(function) { Some(Value::Percent(p)) => Some(p), _ => None };
        if let (Some(a), Some(b)) = (share(AggregateFn::PercentEmpty), share(AggregateFn::PercentFilled)) {
            prop_assert!((a + b - Decimal::ONE).abs() <= Decimal::new(1, 6));
        } else {
            prop_assert!(cells.is_empty());
        }
    }

    /// Results survive storage.
    #[test]
    fn results_round_trip(rows in arb_rows(), q in arb_query()) {
        let result = run(&q, &rows, &ctx(), &Deals);
        let json = serde_json::to_string(&result).unwrap();
        prop_assert_eq!(serde_json::from_str::<QueryResult>(&json).unwrap(), result);
        let json = serde_json::to_string(&q).unwrap();
        prop_assert_eq!(serde_json::from_str::<Query>(&json).unwrap(), q);
    }
}

// ---------------------------------------------------------------------------------------------
// Concurrency and scale
// ---------------------------------------------------------------------------------------------

fn assert_send_sync<T: Send + Sync + 'static>() {}

#[test]
fn concurrency_types_are_send_and_sync() {
    assert_send_sync::<Query>();
    assert_send_sync::<QueryResult>();
    assert_send_sync::<mt_query::View>();
    assert_send_sync::<mt_query::QueryError>();
}

#[test]
fn concurrency_parallel_evaluations_agree() {
    let rows = Arc::new(deals());
    let q = Arc::new(Query {
        group: vec![GroupKey::by(fid(STAGE))],
        sort: vec![SortKey::desc(fid(AMOUNT))],
        aggregates: vec![Aggregate {
            field: fid(AMOUNT),
            function: AggregateFn::Sum,
        }],
        ..Query::default()
    });
    let expected = run(&q, &rows, &ctx(), &Deals);
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let (rows, q) = (Arc::clone(&rows), Arc::clone(&q));
            thread::spawn(move || {
                (0..100)
                    .map(|_| run(&q, &rows, &ctx(), &Deals))
                    .next_back()
                    .unwrap()
            })
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().unwrap(), expected);
    }
}

#[test]
fn edge_fifty_thousand_rows() {
    let rows: Vec<Rec> = (1..=50_000_u128)
        .map(|n| {
            row(
                n,
                &[
                    (QTY, Value::Number(Decimal::from((n * 7919) % 1000))),
                    (STAGE, stage([LEAD, QUALIFIED, WON, LOST][(n % 4) as usize])),
                    (TITLE, text(&format!("Deal {}", n % 97))),
                ],
            )
        })
        .collect();
    let q = Query {
        group: vec![GroupKey::by(fid(STAGE))],
        sort: vec![SortKey::desc(fid(QTY)), SortKey::asc(fid(TITLE))],
        aggregates: vec![Aggregate {
            field: fid(QTY),
            function: AggregateFn::Sum,
        }],
        ..Query::default()
    };
    let result = run(&q, &rows, &ctx(), &Deals);
    assert_eq!(result.rows.len(), 50_000);
    assert_eq!(result.groups.len(), 4);
    assert!(result.groups.iter().all(|g| g.count == 12_500));
    let total: Decimal = result
        .groups
        .iter()
        .map(|g| match &g.aggregates[0].value {
            Some(Value::Number(n)) => *n,
            _ => Decimal::ZERO,
        })
        .sum();
    assert_eq!(result.totals[0].value, Some(Value::Number(total)));
}

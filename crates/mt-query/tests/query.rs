//! The evaluator: filter, search, sort, group, totals, paging, and the checks on queries.
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
use mt_core::condition::{Condition, Context, Operand, Operator, Period, PeriodUnit, UtcContext};
use mt_core::id::PrincipalId;
use mt_core::limits::MAX_GROUP_LEVELS;
use mt_core::time::{Date, Timestamp, Weekday};
use mt_core::value::Value;
use mt_query::query::{MAX_AGGREGATES, MAX_SEARCH_CHARS, MAX_SEARCH_WORDS, MAX_SORT_KEYS};
use mt_query::{
    compare, day_of, matches, run, Aggregate, AggregateFn, DateBucket, Direction, Empties, Group,
    GroupKey, Query, QueryError, QueryResult, Search, SortKey,
};
use std::cmp::Ordering;

fn go(query: &Query) -> QueryResult {
    query.check(&type_of).unwrap();
    run(query, &deals(), &ctx(), &Deals)
}

fn order(query: &Query) -> Vec<u128> {
    ids(&go(query).rows)
}

fn sorted(keys: Vec<SortKey>) -> Query {
    Query {
        sort: keys,
        ..Query::default()
    }
}

// ---------------------------------------------------------------------------------------------
// Filter and order
// ---------------------------------------------------------------------------------------------

#[test]
fn std_no_query_returns_everything_in_record_order() {
    assert_eq!(order(&Query::default()), vec![1, 2, 3, 4, 5, 6, 7, 8]);
    // The input order does not matter.
    let mut reversed = deals();
    reversed.reverse();
    assert_eq!(
        ids(&run(&Query::default(), &reversed, &ctx(), &Deals).rows),
        vec![1, 2, 3, 4, 5, 6, 7, 8]
    );
}

#[test]
fn std_filter_selects_records() {
    let won = Condition::compare(fid(STAGE), Operator::Eq, stage(WON));
    assert_eq!(
        order(&Query {
            filter: Some(won),
            ..Query::default()
        }),
        vec![1, 8]
    );
    // "Mine, closing this month, not done".
    let mine = Condition::All(vec![
        Condition::test(fid(OWNERS), Operator::HasAny, Operand::Me),
        Condition::test(
            fid(CLOSE),
            Operator::Eq,
            Operand::Period(Period::this(PeriodUnit::Month)),
        ),
        Condition::compare(fid(DONE), Operator::Ne, Value::Bool(true)),
    ]);
    assert_eq!(
        order(&Query {
            filter: Some(mine),
            ..Query::default()
        }),
        vec![6]
    );
    // Nothing matches.
    let none = Condition::never();
    let result = go(&Query {
        filter: Some(none),
        ..Query::default()
    });
    assert!(result.rows.is_empty() && result.groups.is_empty());
}

#[test]
fn std_sort_single_key_with_record_id_as_the_last_word() {
    // Amounts: 80, 300, 300, 300, 750.50, 1200, 5000; record 6 has none.
    assert_eq!(
        order(&sorted(vec![SortKey::asc(fid(AMOUNT))])),
        vec![6, 7, 2, 4, 8, 5, 1, 3]
    );
    // Descending: empties go last, and ties are still by ascending record id.
    assert_eq!(
        order(&sorted(vec![SortKey::desc(fid(AMOUNT))])),
        vec![3, 1, 5, 2, 4, 8, 7, 6]
    );
    // Text: case does not separate "beta" from "Alpha".
    assert_eq!(
        order(&sorted(vec![SortKey::asc(fid(TITLE))])),
        vec![1, 2, 4, 5, 7, 3, 8, 6]
    );
    // Numbers compare by value: 1 and 1.0 are a tie.
    assert_eq!(
        order(&sorted(vec![SortKey::asc(fid(QTY))])),
        vec![4, 2, 6, 7, 5, 1, 8, 3]
    );
}

#[test]
fn std_empties_placement() {
    let key = |direction, empties| {
        sorted(vec![SortKey {
            field: fid(QTY),
            direction,
            empties,
        }])
    };
    assert_eq!(order(&key(Direction::Asc, Empties::Natural))[0], 4);
    assert_eq!(order(&key(Direction::Asc, Empties::First))[0], 4);
    assert_eq!(
        *order(&key(Direction::Asc, Empties::Last)).last().unwrap(),
        4
    );
    assert_eq!(
        *order(&key(Direction::Desc, Empties::Natural))
            .last()
            .unwrap(),
        4
    );
    assert_eq!(order(&key(Direction::Desc, Empties::First))[0], 4);
    assert_eq!(
        *order(&key(Direction::Desc, Empties::Last)).last().unwrap(),
        4
    );
    // Blanks last, values ascending.
    assert_eq!(
        order(&key(Direction::Asc, Empties::Last)),
        vec![2, 6, 7, 5, 1, 8, 3, 4]
    );
    // An empty list and a blank text are empty too.
    let tags = sorted(vec![SortKey {
        field: fid(TAGS),
        direction: Direction::Asc,
        empties: Empties::Last,
    }]);
    assert_eq!(order(&tags)[3..], [4, 5, 6, 7, 8]);
}

#[test]
fn std_multi_key_sort() {
    // By close date, then by amount descending.
    let q = sorted(vec![SortKey::asc(fid(CLOSE)), SortKey::desc(fid(AMOUNT))]);
    // No date: 4. Then 04-15: 5. 05-02: 1 (1200), 8 (300). 05-20: 2 (300), 6 (none, last). 06-01: 3. 06-30: 7.
    assert_eq!(order(&q), vec![4, 5, 1, 8, 2, 6, 3, 7]);
}

#[test]
fn std_references_are_ordered_by_the_environment_not_by_id() {
    let q = sorted(vec![SortKey::asc(fid(STAGE))]);
    // Pipeline order: (none) 6; lead 2, 4; qualified 3, 7; won 1, 8; lost 5.
    assert_eq!(order(&q), vec![6, 2, 4, 3, 7, 1, 8, 5]);
    // Without that knowledge the ids decide: qualified(10) < lost(20) < won(30) < lead(40).
    let plain = run(&q, &deals(), &ctx(), &Plain);
    assert_eq!(ids(&plain.rows), vec![6, 3, 7, 5, 1, 8, 2, 4]);
}

#[test]
fn std_compare_and_matches_agree_with_run() {
    let q = Query {
        filter: Some(Condition::is_not_empty(fid(AMOUNT))),
        group: vec![GroupKey::by(fid(STAGE))],
        sort: vec![SortKey::desc(fid(AMOUNT))],
        ..Query::default()
    };
    let rows = deals();
    let result = run(&q, &rows, &ctx(), &Deals);
    // Every adjacent pair of the result is in `compare` order, and `matches` is the filter.
    let by_id = |id: u128| rows.iter().find(|r| r.id == rec(id)).unwrap();
    for pair in result.rows.windows(2) {
        let (a, b) = (by_id(pair[0].as_u128()), by_id(pair[1].as_u128()));
        assert_eq!(compare(&q, a, b, &ctx(), &Deals), Ordering::Less);
        assert_eq!(compare(&q, b, a, &ctx(), &Deals), Ordering::Greater);
        assert_eq!(compare(&q, a, a, &ctx(), &Deals), Ordering::Equal);
    }
    for r in &rows {
        assert_eq!(matches(&q, r, &ctx(), &Deals), result.rows.contains(&r.id));
    }
}

#[test]
fn std_paging() {
    let result = go(&sorted(vec![SortKey::asc(fid(TITLE))]));
    assert_eq!(ids(result.page(0, 3)), vec![1, 2, 4]);
    assert_eq!(ids(result.page(3, 3)), vec![5, 7, 3]);
    assert_eq!(ids(result.page(6, 3)), vec![8, 6]);
    assert!(result.page(8, 3).is_empty());
    assert!(result.page(100, 3).is_empty());
    assert!(result.page(2, 0).is_empty());
    assert_eq!(result.page(0, usize::MAX).len(), 8);
    assert_eq!(result.page(usize::MAX, usize::MAX).len(), 0);
}

// ---------------------------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------------------------

fn search(text: &str, fields: &[u128]) -> Vec<u128> {
    let q = Query {
        search: Some(Search {
            text: text.into(),
            fields: fields.iter().map(|f| fid(*f)).collect(),
        }),
        ..Query::default()
    };
    order(&q)
}

#[test]
fn std_search_finds_words_in_any_listed_field() {
    let all = [TITLE, NOTES, STAGE, OWNERS, AMOUNT, CLOSE, QTY];
    // Case-insensitive, substring.
    assert_eq!(search("ALPHA", &all), vec![1]);
    assert_eq!(
        search("eta", &all),
        vec![2, 6, 7, 8],
        "beta, Zeta, eta, Theta"
    );
    // Long text.
    assert_eq!(search("berlin", &all), vec![4]);
    // Labels of references and names of people come from the environment.
    assert_eq!(search("qualified", &all), vec![3, 7]);
    assert_eq!(search("schmidt", &all), vec![2, 3, 5, 7]);
    // Numbers and dates are searched as they are written.
    assert_eq!(search("750.5", &all), vec![5]);
    assert_eq!(search("2026-06", &all), vec![3, 7]);
    // Every word must be found, each anywhere.
    assert_eq!(search("tanaka won", &all), vec![1, 8]);
    assert_eq!(search("tanaka lost", &all), Vec::<u128>::new());
    assert_eq!(search("  trial   SCHMIDT ", &all), vec![7]);
    // Only the listed fields are searched.
    assert_eq!(search("berlin", &[TITLE]), Vec::<u128>::new());
    assert_eq!(search("qualified", &[TITLE, NOTES]), Vec::<u128>::new());
    // A blank search matches everything.
    assert_eq!(search("   ", &all).len(), 8);
    // Without label knowledge, references are not searchable (never by raw id).
    let q = Query {
        search: Some(Search {
            text: "qualified".into(),
            fields: vec![fid(STAGE)],
        }),
        ..Query::default()
    };
    assert!(run(&q, &deals(), &ctx(), &Plain).rows.is_empty());
    // Checkboxes and JSON are not text.
    assert_eq!(search("true", &[DONE, EXTRA]), Vec::<u128>::new());
}

#[test]
fn std_search_combines_with_the_filter() {
    let q = Query {
        filter: Some(Condition::compare(
            fid(DONE),
            Operator::Eq,
            Value::Bool(true),
        )),
        search: Some(Search {
            text: "tanaka".into(),
            fields: vec![fid(OWNERS)],
        }),
        ..Query::default()
    };
    assert_eq!(order(&q), vec![1, 8]);
}

// ---------------------------------------------------------------------------------------------
// Groups and totals
// ---------------------------------------------------------------------------------------------

fn sum(field: u128) -> Aggregate {
    Aggregate {
        field: fid(field),
        function: AggregateFn::Sum,
    }
}

fn keys(groups: &[Group]) -> Vec<Value> {
    groups.iter().map(|g| g.key.clone()).collect()
}

#[test]
fn std_one_level_of_groups_with_subtotals() {
    let q = Query {
        group: vec![GroupKey::by(fid(STAGE))],
        sort: vec![SortKey::desc(fid(AMOUNT))],
        aggregates: vec![
            sum(AMOUNT),
            sum(QTY),
            Aggregate {
                field: fid(TITLE),
                function: AggregateFn::CountFilled,
            },
        ],
        ..Query::default()
    };
    let result = go(&q);
    // Groups in pipeline order, the records without a stage first.
    assert_eq!(
        keys(&result.groups),
        vec![
            Value::Null,
            stage(LEAD),
            stage(QUALIFIED),
            stage(WON),
            stage(LOST)
        ]
    );
    let rows: Vec<Vec<u128>> = result.groups.iter().map(|g| ids(&g.rows)).collect();
    assert_eq!(
        rows,
        vec![vec![6], vec![2, 4], vec![3, 7], vec![1, 8], vec![5]]
    );
    assert_eq!(ids(&result.rows), vec![6, 2, 4, 3, 7, 1, 8, 5]);
    assert_eq!(
        result.groups.iter().map(|g| g.count).collect::<Vec<_>>(),
        vec![1, 2, 2, 2, 1]
    );
    assert!(result
        .groups
        .iter()
        .all(|g| g.groups.is_empty() && g.field == fid(STAGE)));

    // Subtotals.
    let subtotal =
        |index: usize, aggregate: usize| result.groups[index].aggregates[aggregate].value.clone();
    assert_eq!(
        subtotal(0, 0),
        None,
        "no amounts, so no currency to total in"
    );
    assert_eq!(subtotal(1, 0), Some(money("600.00", "USD")));
    assert_eq!(subtotal(2, 0), Some(money("5080.00", "USD")));
    assert_eq!(subtotal(3, 0), Some(money("1500.00", "USD")));
    assert_eq!(subtotal(4, 0), Some(money("750.50", "USD")));
    assert_eq!(subtotal(1, 1), Some(num("1")));
    assert_eq!(subtotal(2, 1), Some(num("11")));
    assert_eq!(subtotal(3, 2), Some(num("2")));
    // Totals over everything.
    assert_eq!(result.totals[0].value, Some(money("7930.50", "USD")));
    assert_eq!(result.totals[1].value, Some(num("22.5")));
    assert_eq!(result.totals[2].value, Some(num("8")));
}

#[test]
fn std_two_levels_with_date_buckets_and_descending_groups() {
    let q = Query {
        group: vec![
            GroupKey {
                field: fid(CLOSE),
                direction: Direction::Desc,
                bucket: Some(DateBucket::Month),
            },
            GroupKey::by(fid(DONE)),
        ],
        aggregates: vec![sum(QTY)],
        ..Query::default()
    };
    let result = go(&q);
    // Months newest first; the records without a date last (empty is the smallest value).
    assert_eq!(
        keys(&result.groups),
        vec![
            date(2026, 6, 1),
            date(2026, 5, 1),
            date(2026, 4, 1),
            Value::Null
        ]
    );
    let may = &result.groups[1];
    assert_eq!(may.count, 4);
    assert!(
        may.rows.is_empty(),
        "records hang under the innermost level"
    );
    assert_eq!(
        keys(&may.groups),
        vec![Value::Bool(false), Value::Bool(true)]
    );
    assert_eq!(ids(&may.groups[0].rows), vec![2, 6]);
    assert_eq!(ids(&may.groups[1].rows), vec![1, 8]);
    assert_eq!(may.aggregates[0].value, Some(num("9")));
    assert_eq!(may.groups[1].aggregates[0].value, Some(num("7")));
    // The record without a date: its own outer group, and inside it the one without a tick.
    let undated = &result.groups[3];
    assert_eq!(
        (undated.count, keys(&undated.groups)),
        (1, vec![Value::Null])
    );
    assert_eq!(ids(&result.rows), vec![3, 7, 2, 6, 1, 8, 5, 4]);
}

#[test]
fn std_every_bucket_size() {
    let bucket = |bucket| {
        let q = Query {
            group: vec![GroupKey {
                field: fid(CLOSE),
                direction: Direction::Asc,
                bucket: Some(bucket),
            }],
            ..Query::default()
        };
        keys(&go(&q).groups)
    };
    assert_eq!(bucket(DateBucket::Day).len(), 6);
    // Weeks start on Monday in this context: 04-15 → 04-13, 05-02 → 04-27, 05-20 → 05-18, 06-01 → 06-01, 06-30 → 06-29.
    assert_eq!(
        bucket(DateBucket::Week),
        vec![
            Value::Null,
            date(2026, 4, 13),
            date(2026, 4, 27),
            date(2026, 5, 18),
            date(2026, 6, 1),
            date(2026, 6, 29)
        ]
    );
    assert_eq!(
        bucket(DateBucket::Month),
        vec![
            Value::Null,
            date(2026, 4, 1),
            date(2026, 5, 1),
            date(2026, 6, 1)
        ]
    );
    assert_eq!(
        bucket(DateBucket::Quarter),
        vec![Value::Null, date(2026, 4, 1)]
    );
    assert_eq!(
        bucket(DateBucket::Year),
        vec![Value::Null, date(2026, 1, 1)]
    );
    // Sunday weeks move the boundaries.
    let sunday = UtcContext {
        week_start: Weekday::Sunday,
        ..ctx()
    };
    let q = Query {
        group: vec![GroupKey {
            field: fid(CLOSE),
            direction: Direction::Asc,
            bucket: Some(DateBucket::Week),
        }],
        ..Query::default()
    };
    assert_eq!(
        keys(&run(&q, &deals(), &sunday, &Deals).groups)[1..3],
        [date(2026, 4, 12), date(2026, 4, 26)]
    );
    // Without a bucket every distinct date is a group.
    assert_eq!(
        keys(
            &go(&Query {
                group: vec![GroupKey::by(fid(CLOSE))],
                ..Query::default()
            })
            .groups
        )
        .len(),
        6
    );
}

#[test]
fn std_date_time_buckets_follow_the_viewers_day() {
    // Record 2 was created 2026-04-30 23:00 UTC, record 3 on 2026-05-01 00:00 UTC.
    let q = Query {
        filter: Some(Condition::test(
            fid(CREATED),
            Operator::Ge,
            Operand::Value(date(2026, 4, 30)),
        )),
        group: vec![GroupKey {
            field: fid(CREATED),
            direction: Direction::Asc,
            bucket: Some(DateBucket::Month),
        }],
        ..Query::default()
    };
    let utc = run(&q, &deals(), &ctx(), &Deals);
    assert_eq!(keys(&utc.groups), vec![date(2026, 4, 1), date(2026, 5, 1)]);
    assert_eq!(ids(&utc.groups[0].rows), vec![2]);

    // In Tokyo (UTC+9) 23:00 UTC is already 08:00 on 1 May: both fall into May.
    struct Tokyo;
    impl Context for Tokyo {
        fn me(&self) -> Option<PrincipalId> {
            None
        }
        fn today(&self) -> Date {
            day(2026, 5, 13)
        }
        fn week_start(&self) -> Weekday {
            Weekday::Monday
        }
        fn fiscal_year_start_month(&self) -> u32 {
            4
        }
        fn start_of_day(&self, date: Date) -> Timestamp {
            Timestamp::from_millis(date.to_timestamp().millis() - 9 * 3_600_000).unwrap()
        }
    }
    let tokyo = run(&q, &deals(), &Tokyo, &Deals);
    assert_eq!(keys(&tokyo.groups), vec![date(2026, 5, 1)]);
    assert_eq!(ids(&tokyo.groups[0].rows), vec![2, 3, 7]);

    // The day an instant falls on, on both sides of UTC.
    let at = |d: Date, hour: i64| {
        Timestamp::from_millis(d.to_timestamp().millis() + hour * 3_600_000).unwrap()
    };
    assert_eq!(day_of(at(day(2026, 4, 30), 23), &Tokyo), day(2026, 5, 1));
    assert_eq!(day_of(at(day(2026, 4, 30), 14), &Tokyo), day(2026, 4, 30));
    assert_eq!(day_of(at(day(2026, 4, 30), 15), &Tokyo), day(2026, 5, 1));
    assert_eq!(day_of(at(day(2026, 4, 30), 23), &ctx()), day(2026, 4, 30));
    struct LosAngeles;
    impl Context for LosAngeles {
        fn me(&self) -> Option<PrincipalId> {
            None
        }
        fn today(&self) -> Date {
            day(2026, 5, 13)
        }
        fn week_start(&self) -> Weekday {
            Weekday::Sunday
        }
        fn fiscal_year_start_month(&self) -> u32 {
            1
        }
        fn start_of_day(&self, date: Date) -> Timestamp {
            Timestamp::from_millis(date.to_timestamp().millis() + 7 * 3_600_000).unwrap()
        }
    }
    assert_eq!(
        day_of(at(day(2026, 5, 1), 3), &LosAngeles),
        day(2026, 4, 30)
    );
    assert_eq!(day_of(at(day(2026, 5, 1), 7), &LosAngeles), day(2026, 5, 1));
    // At the ends of the calendar the answer stays inside it.
    assert_eq!(day_of(Timestamp::MAX, &ctx()), Date::MAX);
    assert_eq!(day_of(Timestamp::MIN, &ctx()), Date::MIN);
    assert_eq!(day_of(Timestamp::MIN, &LosAngeles), Date::MIN);
}

#[test]
fn std_a_list_value_is_one_group_whatever_its_order() {
    let q = Query {
        group: vec![GroupKey::by(fid(TAGS))],
        ..Query::default()
    };
    let result = go(&q);
    // {1,2} and {2,1} are the same group; an empty list is "no value".
    let sizes: Vec<(Value, usize)> = result
        .groups
        .iter()
        .map(|g| (g.key.clone(), g.count))
        .collect();
    assert_eq!(sizes.len(), 3);
    assert_eq!(sizes[0], (Value::Null, 5));
    assert_eq!(sizes[1], (Value::MultiReference(vec![item(1), item(2)]), 2));
    assert_eq!(sizes[2], (Value::MultiReference(vec![item(3)]), 1));
    // People: {1,2} and {2,1} together.
    let q = Query {
        group: vec![GroupKey::by(fid(OWNERS))],
        ..Query::default()
    };
    let both = go(&q)
        .groups
        .into_iter()
        .find(|g| g.count == 2 && g.key != Value::Null && ids(&g.rows) == vec![3, 7]);
    assert!(both.is_some());
    // Numbers group by value: 1 and 1.0 are one group.
    let q = Query {
        group: vec![GroupKey::by(fid(QTY))],
        ..Query::default()
    };
    let ones = go(&q)
        .groups
        .into_iter()
        .find(|g| g.key == num("1"))
        .unwrap();
    assert_eq!(ids(&ones.rows), vec![2, 6, 7]);
}

#[test]
fn edge_groups_with_the_same_position_do_not_interleave() {
    // An environment that gives every stage the same position.
    struct Flat;
    impl mt_query::Env for Flat {
        fn data_type(&self, field: mt_core::id::FieldId) -> Option<mt_core::field::DataType> {
            type_of(field)
        }
        fn order_value(&self, field: mt_core::id::FieldId, _value: &Value) -> Option<Value> {
            (field == fid(STAGE)).then(|| num("1"))
        }
        fn display_text(&self, _field: mt_core::id::FieldId, _value: &Value) -> Option<String> {
            None
        }
    }
    let q = Query {
        group: vec![GroupKey::by(fid(STAGE))],
        sort: vec![SortKey::asc(fid(TITLE))],
        ..Query::default()
    };
    let result = run(&q, &deals(), &ctx(), &Flat);
    // Still exactly one group per stage, each complete.
    assert_eq!(result.groups.len(), 5);
    let mut seen = std::collections::BTreeSet::new();
    for group in &result.groups {
        assert!(
            seen.insert(format!("{:?}", group.key)),
            "a group appeared twice"
        );
    }
    assert_eq!(result.groups.iter().map(|g| g.count).sum::<usize>(), 8);
}

#[test]
fn edge_empty_table_and_unchecked_queries() {
    let q = Query {
        group: vec![GroupKey::by(fid(STAGE))],
        aggregates: vec![sum(QTY), sum(AMOUNT)],
        ..Query::default()
    };
    let empty: Vec<Rec> = vec![];
    let result = run(&q, &empty, &ctx(), &Deals);
    assert!(result.rows.is_empty() && result.groups.is_empty());
    assert_eq!(
        result.totals[0].value,
        Some(num("0")),
        "the sum of no numbers is zero"
    );
    assert_eq!(
        result.totals[1].value, None,
        "the sum of no money has no currency"
    );

    // A query that was never checked still evaluates: unknown fields are empty, and
    // aggregates that do not apply have no value.
    let odd = Query {
        filter: Some(Condition::is_empty(fid(999))),
        group: vec![
            GroupKey {
                field: fid(998),
                direction: Direction::Asc,
                bucket: Some(DateBucket::Month),
            },
            GroupKey {
                field: fid(TITLE),
                direction: Direction::Asc,
                bucket: Some(DateBucket::Year),
            },
        ],
        sort: vec![SortKey::asc(fid(997))],
        aggregates: vec![sum(996), sum(TITLE)],
        ..Query::default()
    };
    let result = run(&odd, &deals(), &ctx(), &Deals);
    assert_eq!(result.rows.len(), 8);
    assert_eq!(result.groups.len(), 1);
    assert_eq!(
        result.groups[0].groups.len(),
        8,
        "a bucket on a text field falls back to the value"
    );
    assert!(result
        .totals
        .iter()
        .all(|t| t.value.is_none() && t.problem.is_none()));
}

// ---------------------------------------------------------------------------------------------
// Checks and storage
// ---------------------------------------------------------------------------------------------

#[test]
fn invalid_queries() {
    let err = |q: Query| q.check(&type_of).unwrap_err();
    let reason = |q: Query| err(q).to_string();
    // Filter problems come from the condition's own check.
    assert!(matches!(
        err(Query {
            filter: Some(Condition::is_empty(fid(99))),
            ..Query::default()
        }),
        QueryError::Core(_)
    ));
    // Search.
    let s = |text: &str, fields: Vec<u128>| Query {
        search: Some(Search {
            text: text.into(),
            fields: fields.into_iter().map(fid).collect(),
        }),
        ..Query::default()
    };
    assert_eq!(
        err(s(&"x".repeat(MAX_SEARCH_CHARS + 1), vec![TITLE])),
        QueryError::Limit("search text too long")
    );
    assert_eq!(
        err(s(&"w ".repeat(MAX_SEARCH_WORDS + 1), vec![TITLE])),
        QueryError::Limit("too many search words")
    );
    assert!(reason(s("x", vec![])).contains("names no field"));
    assert!(reason(s("x", vec![99])).contains("not in the table"));
    // Groups.
    let g = |keys: Vec<GroupKey>| Query {
        group: keys,
        ..Query::default()
    };
    assert!(reason(g(vec![GroupKey::by(fid(99))])).contains("not in the table"));
    assert!(
        reason(g(vec![GroupKey::by(fid(STAGE)), GroupKey::by(fid(STAGE))]))
            .contains("grouped twice")
    );
    for field in [EXTRA, NOTES, FILES] {
        assert!(
            reason(g(vec![GroupKey::by(fid(field))])).contains("cannot be grouped"),
            "{field}"
        );
    }
    assert!(reason(g(vec![GroupKey {
        field: fid(QTY),
        direction: Direction::Asc,
        bucket: Some(DateBucket::Month)
    }]))
    .contains("cannot be grouped by day"));
    let many: Vec<GroupKey> = [TITLE, AMOUNT, QTY, STAGE, CLOSE, CREATED]
        .iter()
        .map(|f| GroupKey::by(fid(*f)))
        .collect();
    assert_eq!(many.len(), MAX_GROUP_LEVELS + 1);
    assert_eq!(
        err(g(many.clone())),
        QueryError::Limit("too many group levels")
    );
    g(many[..MAX_GROUP_LEVELS].to_vec())
        .check(&type_of)
        .unwrap();
    // Sort.
    assert!(reason(sorted(vec![SortKey::asc(fid(99))])).contains("not in the table"));
    assert!(reason(sorted(vec![
        SortKey::asc(fid(QTY)),
        SortKey::desc(fid(QTY))
    ]))
    .contains("sorted twice"));
    for field in [EXTRA, FILES] {
        assert!(reason(sorted(vec![SortKey::asc(fid(field))])).contains("cannot be sorted"));
    }
    let too_many: Vec<SortKey> = (0..=MAX_SORT_KEYS)
        .map(|_| SortKey::asc(fid(TITLE)))
        .collect();
    assert_eq!(
        err(sorted(too_many)),
        QueryError::Limit("too many sort keys")
    );
    // Aggregates.
    let a = |list: Vec<Aggregate>| Query {
        aggregates: list,
        ..Query::default()
    };
    assert!(reason(a(vec![sum(99)])).contains("not in the table"));
    assert!(reason(a(vec![sum(TITLE)])).contains("`sum` cannot be computed for a text field"));
    assert_eq!(
        err(a(vec![sum(QTY); MAX_AGGREGATES + 1])),
        QueryError::Limit("too many aggregates")
    );
    a(vec![sum(QTY); MAX_AGGREGATES]).check(&type_of).unwrap();
}

#[test]
fn std_queries_and_results_survive_storage() {
    let q = Query {
        filter: Some(Condition::is_not_empty(fid(AMOUNT))),
        search: Some(Search {
            text: "a".into(),
            fields: vec![fid(TITLE)],
        }),
        group: vec![GroupKey {
            field: fid(CLOSE),
            direction: Direction::Desc,
            bucket: Some(DateBucket::Quarter),
        }],
        sort: vec![SortKey {
            field: fid(QTY),
            direction: Direction::Desc,
            empties: Empties::Last,
        }],
        aggregates: vec![sum(AMOUNT)],
    };
    let json = serde_json::to_string(&q).unwrap();
    assert_eq!(serde_json::from_str::<Query>(&json).unwrap(), q);
    // The empty query is the empty object, and defaults fill in.
    assert_eq!(serde_json::to_string(&Query::default()).unwrap(), "{}");
    let minimal: Query =
        serde_json::from_str(r#"{"sort":[{"field":"fld_00000000000000000000000001"}]}"#).unwrap();
    assert_eq!(minimal.sort, vec![SortKey::asc(fid(1))]);
    let result = go(&q);
    let json = serde_json::to_string(&result).unwrap();
    assert_eq!(serde_json::from_str::<QueryResult>(&json).unwrap(), result);
}

#[test]
fn edge_grouping_by_links_and_files_is_set_based_too() {
    use mt_core::id::AttachmentId;
    // Not offered by the query check for attachments, but the evaluator stays consistent.
    const LINKS: u128 = 15;
    let link = |ids: &[u128]| Value::Link(ids.iter().map(|n| rec(*n)).collect());
    let file =
        |ids: &[u128]| Value::Attachment(ids.iter().map(|n| AttachmentId::from_u128(*n)).collect());
    let rows = vec![
        row(1, &[(LINKS, link(&[5, 6])), (FILES, file(&[1]))]),
        row(2, &[(LINKS, link(&[6, 5, 5])), (FILES, file(&[1, 1]))]),
        row(3, &[(LINKS, link(&[7]))]),
    ];
    let by = |field: u128| Query {
        group: vec![GroupKey::by(fid(field))],
        ..Query::default()
    };
    let groups = run(&by(LINKS), &rows, &ctx(), &Plain).groups;
    assert_eq!(
        groups.iter().map(|g| ids(&g.rows)).collect::<Vec<_>>(),
        vec![vec![1, 2], vec![3]]
    );
    let groups = run(&by(FILES), &rows, &ctx(), &Plain).groups;
    assert_eq!(
        groups.iter().map(|g| ids(&g.rows)).collect::<Vec<_>>(),
        vec![vec![3], vec![1, 2]]
    );
}

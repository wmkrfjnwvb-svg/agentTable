//! The in-memory evaluator. It defines what a [`Query`] means; the SQL compiler of the
//! storage layer must agree with it on every table (tested there on all three databases).
//!
//! Evaluation order: filter → search → group → sort within groups → totals.
//!
//! # Order
//!
//! Records are ordered by the group keys (outermost first), then by the sort keys, then by
//! record id. Because the record id is always the last key, the order is total and stable:
//! the same query on the same data gives the same sequence everywhere, which is what paging
//! and incremental updates rely on.
//!
//! Values are compared with `mt_core::sortkey::compare_values` (the order the database
//! indexes use). An [`Env`] may substitute the value a field is ordered by — reference items
//! are ordered by their position in the set, people by name — and supplies the texts quick
//! search looks at.
//!
//! # Groups
//!
//! Each distinct key is a group; records without a value form their own group. A list value
//! (several tags, several people) is one key as a whole, whatever the order of its items,
//! so every record is in exactly one group per level and subtotals add up to the total.

use crate::aggregate::{aggregate, AggregateValue};
use crate::query::{DateBucket, Direction, Empties, GroupKey, Query, Search};
use core::cmp::Ordering;
use mt_core::condition::{fold_text, Context, Period, PeriodUnit, UtcContext};
use mt_core::field::DataType;
use mt_core::id::{FieldId, RecordId};
use mt_core::sortkey::compare_values;
use mt_core::time::{Date, Timestamp};
use mt_core::value::Value;
use serde::{Deserialize, Serialize};

/// One record as the evaluator sees it.
pub trait Row {
    /// The record id.
    fn id(&self) -> RecordId;

    /// The value of a field (`None` for a field without a value).
    fn value(&self, field: FieldId) -> Option<&Value>;
}

/// What the evaluator needs to know beyond the records themselves.
pub trait Env {
    /// The data type of a field of the table.
    fn data_type(&self, field: FieldId) -> Option<DataType>;

    /// The value to order and group by instead of the stored one, if any: for a reference
    /// item its position in the set, for a person their name. `None` keeps the stored value.
    fn order_value(&self, field: FieldId, value: &Value) -> Option<Value>;

    /// The text people see for a value that is not text by itself (reference labels, names
    /// of people, linked record titles), for quick search.
    fn display_text(&self, field: FieldId, value: &Value) -> Option<String>;
}

/// The civil day an instant falls on in the context's time zone.
#[must_use]
pub fn day_of(at: Timestamp, ctx: &dyn Context) -> Date {
    let utc = at.date();
    // The local day is the UTC day or one of its neighbours.
    let starts = |day: Date| ctx.start_of_day(day) <= at;
    let next = utc.checked_add_days(1);
    let previous = utc.checked_add_days(-1);
    if next.is_some_and(starts) {
        next.unwrap_or(utc)
    } else if starts(utc) {
        utc
    } else {
        previous.unwrap_or(utc)
    }
}

/// The first day of the bucket a day belongs to.
fn bucket_start(day: Date, bucket: DateBucket, ctx: &dyn Context) -> Date {
    // "The unit that contains today", asked on the day in question. Only the calendar
    // settings of the context matter for that.
    let on_that_day = UtcContext {
        me: None,
        today: day,
        week_start: ctx.week_start(),
        fiscal_year_start_month: ctx.fiscal_year_start_month(),
    };
    let unit = match bucket {
        DateBucket::Day => return day,
        DateBucket::Week => PeriodUnit::Week,
        DateBucket::Month => PeriodUnit::Month,
        DateBucket::Quarter => PeriodUnit::Quarter,
        DateBucket::Year => PeriodUnit::Year,
    };
    // At the very end of the calendar a period cannot be closed; the day itself is then
    // its own bucket.
    Period::this(unit)
        .resolve(&on_that_day)
        .map_or(day, |(first, _)| first)
}

/// The key a record has at one group level: the bucket for bucketed dates, otherwise the
/// value itself in a canonical form (empty → null, list items sorted and de-duplicated,
/// trailing zeros dropped), after the environment's substitution.
fn group_key<R: Row + ?Sized>(row: &R, key: &GroupKey, ctx: &dyn Context) -> Value {
    let Some(value) = row.value(key.field).filter(|v| !v.is_empty()) else {
        return Value::Null;
    };
    if let Some(bucket) = key.bucket {
        let day = match value {
            Value::Date(day) => Some(*day),
            Value::DateTime(at) => Some(day_of(*at, ctx)),
            Value::Null
            | Value::Text(_)
            | Value::LongText(_)
            | Value::Number(_)
            | Value::Money(_)
            | Value::Percent(_)
            | Value::Bool(_)
            | Value::Duration(_)
            | Value::Reference(_)
            | Value::MultiReference(_)
            | Value::Link(_)
            | Value::Principal(_)
            | Value::Attachment(_)
            | Value::Json(_) => None,
        };
        if let Some(day) = day {
            return Value::Date(bucket_start(day, bucket, ctx));
        }
    }
    canonical(value.clone())
}

/// A value in the form in which equal-looking values are equal.
fn canonical(value: Value) -> Value {
    fn set<T: Ord>(mut items: Vec<T>) -> Vec<T> {
        items.sort_unstable();
        items.dedup();
        items
    }
    match value.normalized() {
        Value::MultiReference(items) => Value::MultiReference(set(items)),
        Value::Link(items) => Value::Link(set(items)),
        Value::Principal(items) => Value::Principal(set(items)),
        Value::Attachment(items) => Value::Attachment(set(items)),
        other @ (Value::Null
        | Value::Text(_)
        | Value::LongText(_)
        | Value::Number(_)
        | Value::Money(_)
        | Value::Percent(_)
        | Value::Bool(_)
        | Value::Date(_)
        | Value::DateTime(_)
        | Value::Duration(_)
        | Value::Reference(_)
        | Value::Json(_)) => other,
    }
}

/// The value a record is ordered by for one field.
fn order_value<R: Row + ?Sized>(row: &R, field: FieldId, env: &dyn Env) -> Value {
    match row.value(field).filter(|v| !v.is_empty()) {
        None => Value::Null,
        Some(value) => env
            .order_value(field, value)
            .unwrap_or_else(|| value.clone()),
    }
}

/// Compares two order values under a direction and an empties rule.
fn compare_keys(a: &Value, b: &Value, direction: Direction, empties: Empties) -> Ordering {
    let placed = match (a.is_null(), b.is_null(), empties) {
        (true, true, _) => return Ordering::Equal,
        (true, false, Empties::First) | (false, true, Empties::Last) => Some(Ordering::Less),
        (true, false, Empties::Last) | (false, true, Empties::First) => Some(Ordering::Greater),
        (true, false, Empties::Natural) | (false, true, Empties::Natural) | (false, false, _) => {
            None
        }
    };
    if let Some(ordering) = placed {
        return ordering;
    }
    let natural = compare_values(a, b);
    match direction {
        Direction::Asc => natural,
        Direction::Desc => natural.reverse(),
    }
}

/// Whether a record passes the query's filter and search.
#[must_use]
pub fn matches(query: &Query, row: &dyn Row, ctx: &dyn Context, env: &dyn Env) -> bool {
    let passes_filter = query
        .filter
        .as_ref()
        .is_none_or(|filter| filter.evaluate(&|field| row.value(field), ctx));
    passes_filter
        && query
            .search
            .as_ref()
            .is_none_or(|search| search_matches(search, row, env))
}

/// Whether every word of the search text occurs in at least one of the search fields.
/// A search without words matches everything.
fn search_matches(search: &Search, row: &dyn Row, env: &dyn Env) -> bool {
    let words: Vec<String> = search.text.split_whitespace().map(fold_text).collect();
    if words.is_empty() {
        return true;
    }
    let texts: Vec<String> = search
        .fields
        .iter()
        .filter_map(|field| {
            let value = row.value(*field).filter(|v| !v.is_empty())?;
            let text = match value {
                Value::Text(text) | Value::LongText(text) => Some(text.clone()),
                Value::Number(_)
                | Value::Money(_)
                | Value::Percent(_)
                | Value::Date(_)
                | Value::DateTime(_)
                | Value::Duration(_) => Some(value.to_string()),
                Value::Reference(_)
                | Value::MultiReference(_)
                | Value::Link(_)
                | Value::Principal(_)
                | Value::Attachment(_) => env.display_text(*field, value),
                Value::Null | Value::Bool(_) | Value::Json(_) => None,
            };
            text.map(|t| fold_text(&t))
        })
        .collect();
    words
        .iter()
        .all(|word| texts.iter().any(|text| text.contains(word.as_str())))
}

/// A record with the keys the query orders and groups it by, computed once.
struct Keyed<'r, R: ?Sized> {
    row: &'r R,
    id: RecordId,
    /// Per group level: the value the group is ordered by, and the group's key.
    groups: Vec<(Value, Value)>,
    /// Per sort key: the value the record is ordered by.
    sorts: Vec<Value>,
}

fn keyed<'r, R: Row + ?Sized>(
    query: &Query,
    row: &'r R,
    ctx: &dyn Context,
    env: &dyn Env,
) -> Keyed<'r, R> {
    let groups = query
        .group
        .iter()
        .map(|key| {
            let group = group_key(row, key, ctx);
            // Buckets and the empty group are ordered as they are; other keys may be
            // ordered by a substitute (a reference item's position, a person's name).
            let order = if group.is_null() || key.bucket.is_some() {
                None
            } else {
                env.order_value(key.field, &group)
            };
            (order.unwrap_or_else(|| group.clone()), group)
        })
        .collect();
    let sorts = query
        .sort
        .iter()
        .map(|key| order_value(row, key.field, env))
        .collect();
    Keyed {
        row,
        id: row.id(),
        groups,
        sorts,
    }
}

/// Whether two group keys denote the same group.
fn same_group(a: &Value, b: &Value) -> bool {
    compare_values(a, b) == Ordering::Equal
}

fn compare_keyed<R: ?Sized>(query: &Query, a: &Keyed<'_, R>, b: &Keyed<'_, R>) -> Ordering {
    for (key, ((order_a, group_a), (order_b, group_b))) in
        query.group.iter().zip(a.groups.iter().zip(&b.groups))
    {
        // Groups are ordered by their order value; two different groups that share one
        // (two items with the same position) are still kept apart by their keys.
        let ordering = compare_keys(order_a, order_b, key.direction, Empties::Natural)
            .then_with(|| compare_values(group_a, group_b));
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    for (key, (value_a, value_b)) in query.sort.iter().zip(a.sorts.iter().zip(&b.sorts)) {
        let ordering = compare_keys(value_a, value_b, key.direction, key.empties);
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    a.id.cmp(&b.id)
}

/// The order of two records under the query (group keys, sort keys, record id). This is the
/// comparison [`run`] sorts by, so a client can place a changed record into a loaded result
/// without evaluating the query again.
#[must_use]
pub fn compare(
    query: &Query,
    a: &dyn Row,
    b: &dyn Row,
    ctx: &dyn Context,
    env: &dyn Env,
) -> Ordering {
    compare_keyed(
        query,
        &keyed(query, a, ctx, env),
        &keyed(query, b, ctx, env),
    )
}

/// One group of a grouped result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    /// The field this level groups by.
    pub field: FieldId,
    /// The group's key: the shared value, the first day of the bucket, or null for the
    /// records without a value.
    pub key: Value,
    /// Number of records in the group, sub-groups included.
    pub count: usize,
    /// The query's aggregates over the group's records, in the query's order.
    pub aggregates: Vec<AggregateValue>,
    /// Sub-groups (empty at the innermost level).
    pub groups: Vec<Group>,
    /// The group's records in order (only at the innermost level).
    pub rows: Vec<RecordId>,
}

/// The answer to a query.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct QueryResult {
    /// Every matching record, in order.
    pub rows: Vec<RecordId>,
    /// The query's aggregates over all matching records, in the query's order.
    pub totals: Vec<AggregateValue>,
    /// The groups, outermost level first (empty when the query does not group).
    pub groups: Vec<Group>,
}

impl QueryResult {
    /// A window of the ordered records: `limit` records after skipping `offset`.
    #[must_use]
    pub fn page(&self, offset: usize, limit: usize) -> &[RecordId] {
        let end = offset.saturating_add(limit).min(self.rows.len());
        self.rows.get(offset.min(end)..end).unwrap_or(&[])
    }
}

fn totals<R: Row>(query: &Query, rows: &[Keyed<'_, R>], env: &dyn Env) -> Vec<AggregateValue> {
    query
        .aggregates
        .iter()
        .map(|a| match env.data_type(a.field) {
            Some(data_type) => aggregate(
                a.function,
                data_type,
                rows.iter().map(|keyed| keyed.row.value(a.field)),
            ),
            None => AggregateValue::default(),
        })
        .collect()
}

fn build_groups<R: Row>(
    query: &Query,
    rows: &[Keyed<'_, R>],
    level: usize,
    env: &dyn Env,
) -> Vec<Group> {
    let Some(key) = query.group.get(level) else {
        return Vec::new();
    };
    let key_of = |keyed: &Keyed<'_, R>| keyed.groups.get(level).map(|(_, group)| group.clone());
    // The rows are in order, so the records of one group are adjacent.
    let mut out: Vec<Group> = Vec::new();
    let mut rest = rows;
    while let Some(group) = rest.first().and_then(key_of) {
        let length = rest
            .iter()
            .take_while(|keyed| key_of(keyed).is_some_and(|other| same_group(&other, &group)))
            .count();
        let (members, remaining) = rest.split_at(length.min(rest.len()));
        rest = remaining;
        let groups = build_groups(query, members, level.saturating_add(1), env);
        let leaf_rows = if groups.is_empty() {
            members.iter().map(|keyed| keyed.id).collect()
        } else {
            Vec::new()
        };
        out.push(Group {
            field: key.field,
            key: group,
            count: members.len(),
            aggregates: totals(query, members, env),
            groups,
            rows: leaf_rows,
        });
    }
    out
}

/// Evaluates a query over records held in memory.
///
/// The query should have passed [`Query::check`]; an unchecked query still evaluates without
/// failing (unknown fields count as empty, aggregates that do not apply have no value).
#[must_use]
pub fn run<R: Row>(query: &Query, rows: &[R], ctx: &dyn Context, env: &dyn Env) -> QueryResult {
    let mut selected: Vec<Keyed<'_, R>> = rows
        .iter()
        .filter(|row| matches(query, *row, ctx, env))
        .map(|row| keyed(query, row, ctx, env))
        .collect();
    selected.sort_by(|a, b| compare_keyed(query, a, b));
    QueryResult {
        rows: selected.iter().map(|keyed| keyed.id).collect(),
        totals: totals(query, &selected, env),
        groups: build_groups(query, &selected, 0, env),
    }
}

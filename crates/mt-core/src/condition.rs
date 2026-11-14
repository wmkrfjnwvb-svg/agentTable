//! Conditions: boolean expressions over the fields of one record.
//!
//! One condition language serves every place that asks "does this record match?": view
//! filters, conditional requirements, visibility of field groups, colour rules, rule triggers
//! and permission conditions. This module defines the data ([`Condition`]), checks it against
//! the field types ([`Condition::check`]) and evaluates it in memory
//! ([`Condition::evaluate`]). The query crate compiles the same data to SQL; both must give
//! the same answer for every record, which is why the semantics below are fixed and simple.
//!
//! # Semantics
//!
//! * **Two-valued.** A test is true or false; there is no "unknown".
//! * **Empty values.** A *positive* operator (`eq`, `lt`, `contains`, `in`, `has_any`,
//!   `between`, …) is false when the field is empty. Each *negative* operator is exactly the
//!   negation of its positive twin (`ne` = not `eq`, `not_contains` = not `contains`,
//!   `not_in` = not `in`, `has_none` = not `has_any`), so "is not X" includes records where
//!   the field is blank, and `not(eq)` always equals `ne`.
//! * **Text** is compared after [`fold_text`] (Unicode lowercase character by character,
//!   plus final sigma, sharp s and long s):
//!   `eq`, the ordering operators and `contains` are all case-insensitive and consistent with
//!   each other.
//! * **Money** in different currencies is never equal and has no order.
//! * **Lists** (multi-reference, link, principal, attachment): `eq` means "the same set of
//!   items"; `has_any`, `has_all`, `has_none` compare with another list.
//! * **Dates.** A [`Period`] operand ("this week", "the last 7 days") is an interval of whole
//!   days. For a point `x` and an interval `[first, last]`: `eq` and `between` mean inside,
//!   `lt` before `first`, `le` not after `last`, `gt` after `last`, `ge` not before `first`.
//!   A date-time field compared with a date (or a period) uses the day boundaries supplied by
//!   the [`Context`], which is where time zones live.
//! * **Ill-typed tests** (possible only if [`Condition::check`] was skipped) behave like a
//!   mismatch: positive operators false, negative operators true. Evaluation never fails.

use crate::error::{clip, CoreError};
use crate::field::{string_enum, DataType};
use crate::id::{FieldId, PrincipalId};
use crate::limits::MAX_FILTER_CONDITIONS;
use crate::sortkey::compare_values;
use crate::time::{Date, Timestamp, Weekday};
use crate::value::Value;
use core::cmp::Ordering;
use core::fmt;
use core::str::FromStr;
use std::collections::BTreeSet;

/// Deepest nesting of `all` / `any` / `not` groups.
pub const MAX_CONDITION_DEPTH: usize = 8;
/// Maximum number of items in a list operand.
pub const MAX_OPERAND_ITEMS: usize = 1_000;

string_enum! {
    /// The comparison a [`Test`] performs.
    Operator, "operator";
    /// The field has no value.
    IsEmpty => "is_empty",
    /// The field has a value.
    IsNotEmpty => "is_not_empty",
    /// Equal.
    Eq => "eq",
    /// Not equal (true for an empty field).
    Ne => "ne",
    /// Less than / before.
    Lt => "lt",
    /// Less than or equal / not after.
    Le => "le",
    /// Greater than / after.
    Gt => "gt",
    /// Greater than or equal / not before.
    Ge => "ge",
    /// Within an inclusive range or a period.
    Between => "between",
    /// The text contains the operand.
    Contains => "contains",
    /// The text does not contain the operand (true for an empty field).
    NotContains => "not_contains",
    /// The text starts with the operand.
    StartsWith => "starts_with",
    /// The text ends with the operand.
    EndsWith => "ends_with",
    /// The value is one of the listed values.
    In => "in",
    /// The value is none of the listed values (true for an empty field).
    NotIn => "not_in",
    /// The list shares at least one item with the operand.
    HasAny => "has_any",
    /// The list contains every item of the operand.
    HasAll => "has_all",
    /// The list shares no item with the operand (true for an empty field).
    HasNone => "has_none",
}

impl Operator {
    /// Whether the operator can be used on a field of the given data type.
    #[must_use]
    pub const fn applies_to(self, data_type: DataType) -> bool {
        match self {
            Operator::IsEmpty | Operator::IsNotEmpty | Operator::Eq | Operator::Ne => true,
            Operator::Lt | Operator::Le | Operator::Gt | Operator::Ge | Operator::Between => {
                matches!(
                    data_type,
                    DataType::Text
                        | DataType::Number
                        | DataType::Money
                        | DataType::Percent
                        | DataType::Date
                        | DataType::DateTime
                        | DataType::Duration
                )
            }
            Operator::Contains
            | Operator::NotContains
            | Operator::StartsWith
            | Operator::EndsWith => data_type.is_text(),
            Operator::In | Operator::NotIn => matches!(
                data_type,
                DataType::Text
                    | DataType::Number
                    | DataType::Money
                    | DataType::Percent
                    | DataType::Date
                    | DataType::DateTime
                    | DataType::Duration
                    | DataType::Reference
            ),
            Operator::HasAny | Operator::HasAll | Operator::HasNone => data_type.is_list(),
        }
    }
}

string_enum! {
    /// The unit of a [`Period`].
    PeriodUnit, "period unit";
    /// Calendar days.
    Day => "day",
    /// Weeks, starting on the context's first day of the week.
    Week => "week",
    /// Calendar months.
    Month => "month",
    /// Calendar quarters (January, April, July, October).
    Quarter => "quarter",
    /// Calendar years.
    Year => "year",
    /// Quarters of the fiscal year.
    FiscalQuarter => "fiscal_quarter",
    /// Fiscal years, starting in the context's fiscal start month.
    FiscalYear => "fiscal_year",
}

/// A span of whole days relative to today: `count` consecutive units, the first of which is
/// `offset` units away from the unit that contains today.
///
/// | Meaning | unit | offset | count |
/// |---|---|---|---|
/// | today | day | 0 | 1 |
/// | yesterday | day | -1 | 1 |
/// | the last 7 days, including today | day | -6 | 7 |
/// | the next 30 days, from today | day | 0 | 30 |
/// | this week | week | 0 | 1 |
/// | last month | month | -1 | 1 |
/// | the last 3 full months | month | -3 | 3 |
/// | this fiscal year | fiscal_year | 0 | 1 |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Period {
    /// The unit.
    pub unit: PeriodUnit,
    /// How many units away the first unit is (negative = past).
    #[serde(default)]
    pub offset: i32,
    /// How many units the period spans (at least 1).
    #[serde(default = "one")]
    pub count: u32,
}

const fn one() -> u32 {
    1
}

impl Period {
    /// The unit that contains today (`this(Week)` is this week).
    #[must_use]
    pub const fn this(unit: PeriodUnit) -> Self {
        Period {
            unit,
            offset: 0,
            count: 1,
        }
    }

    /// The unit before the current one.
    #[must_use]
    pub const fn last(unit: PeriodUnit) -> Self {
        Period {
            unit,
            offset: -1,
            count: 1,
        }
    }

    /// The unit after the current one.
    #[must_use]
    pub const fn next(unit: PeriodUnit) -> Self {
        Period {
            unit,
            offset: 1,
            count: 1,
        }
    }

    /// The last `days` days, today included.
    #[must_use]
    pub fn last_days(days: u32) -> Self {
        let back = i32::try_from(days.saturating_sub(1)).unwrap_or(i32::MAX);
        Period {
            unit: PeriodUnit::Day,
            offset: back.saturating_neg(),
            count: days,
        }
    }

    /// The next `days` days, today included.
    #[must_use]
    pub const fn next_days(days: u32) -> Self {
        Period {
            unit: PeriodUnit::Day,
            offset: 0,
            count: days,
        }
    }

    /// The first and last day of the period for the given context, or `None` when the period
    /// is empty (`count` 0) or leaves the supported date range.
    #[must_use]
    pub fn resolve(&self, ctx: &dyn Context) -> Option<(Date, Date)> {
        if self.count == 0 {
            return None;
        }
        let today = ctx.today();
        let offset = i64::from(self.offset);
        let count = i64::from(self.count);
        let days = |first: Date, length: i64| -> Option<(Date, Date)> {
            let last = first.checked_add_days(i32::try_from(length.checked_sub(1)?).ok()?)?;
            Some((first, last))
        };
        // Periods made of months: `anchor` is the index of the first month of the current unit.
        let months = |per_unit: i64, anchor: i64| -> Option<(Date, Date)> {
            let first = anchor.checked_add(offset.checked_mul(per_unit)?)?;
            let after = first.checked_add(count.checked_mul(per_unit)?)?;
            let start = month_start(first)?;
            let end = month_start(after)?.checked_add_days(-1)?;
            Some((start, end))
        };
        let align = |index: i64, size: i64, shift: i64| -> Option<i64> {
            let shifted = index.checked_sub(shift)?;
            shifted
                .checked_sub(shifted.checked_rem_euclid(size)?)?
                .checked_add(shift)
        };
        let index = month_index(today);
        let fiscal_shift = i64::from(ctx.fiscal_year_start_month().clamp(1, 12)).saturating_sub(1);
        match self.unit {
            PeriodUnit::Day => {
                let first = today.checked_add_days(self.offset)?;
                days(first, count)
            }
            PeriodUnit::Week => {
                let today_index = i32::from(today.weekday().days_from_monday());
                let start_index = i32::from(ctx.week_start().days_from_monday());
                let back = today_index
                    .checked_sub(start_index)?
                    .checked_rem_euclid(7)?;
                let this_week = today.checked_add_days(back.checked_neg()?)?;
                let shift = i32::try_from(offset.checked_mul(7)?).ok()?;
                let first = this_week.checked_add_days(shift)?;
                days(first, count.checked_mul(7)?)
            }
            PeriodUnit::Month => months(1, index),
            PeriodUnit::Quarter => months(3, align(index, 3, 0)?),
            PeriodUnit::Year => months(12, align(index, 12, 0)?),
            PeriodUnit::FiscalQuarter => months(3, align(index, 3, fiscal_shift)?),
            PeriodUnit::FiscalYear => months(12, align(index, 12, fiscal_shift)?),
        }
    }
}

/// `year * 12 + (month - 1)`.
fn month_index(date: Date) -> i64 {
    let (year, month, _) = date.ymd();
    i64::from(year)
        .saturating_mul(12)
        .saturating_add(i64::from(month))
        .saturating_sub(1)
}

/// The first day of the month with the given index, if it is in range.
fn month_start(index: i64) -> Option<Date> {
    let year = i32::try_from(index.checked_div_euclid(12)?).ok()?;
    let month = u32::try_from(index.checked_rem_euclid(12)?)
        .ok()?
        .checked_add(1)?;
    Date::from_ymd(year, month, 1).ok()
}

/// What "me", "today" and "the start of a day" mean for one evaluation. Time zones live
/// here: the caller (the server with a time-zone database, the browser through `Intl`)
/// answers these questions; this crate only does calendar arithmetic on the answers.
pub trait Context {
    /// The principal the condition is evaluated for, if any.
    fn me(&self) -> Option<PrincipalId>;

    /// Today's date in the relevant time zone.
    fn today(&self) -> Date;

    /// The first day of the week.
    fn week_start(&self) -> Weekday;

    /// The month (1–12) in which the fiscal year starts.
    fn fiscal_year_start_month(&self) -> u32;

    /// The instant at which the given day starts in the relevant time zone.
    fn start_of_day(&self, date: Date) -> Timestamp;
}

/// A [`Context`] in UTC with explicit settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UtcContext {
    /// The current principal.
    pub me: Option<PrincipalId>,
    /// Today's date.
    pub today: Date,
    /// The first day of the week.
    pub week_start: Weekday,
    /// The month (1–12) in which the fiscal year starts.
    pub fiscal_year_start_month: u32,
}

impl UtcContext {
    /// A context for `today` with Monday weeks, a January fiscal year and nobody signed in.
    #[must_use]
    pub const fn on(today: Date) -> Self {
        UtcContext {
            me: None,
            today,
            week_start: Weekday::Monday,
            fiscal_year_start_month: 1,
        }
    }
}

impl Context for UtcContext {
    fn me(&self) -> Option<PrincipalId> {
        self.me
    }

    fn today(&self) -> Date {
        self.today
    }

    fn week_start(&self) -> Weekday {
        self.week_start
    }

    fn fiscal_year_start_month(&self) -> u32 {
        self.fiscal_year_start_month
    }

    fn start_of_day(&self, date: Date) -> Timestamp {
        date.to_timestamp()
    }
}

/// What a [`Test`] compares the field with.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Operand {
    /// Nothing (`is_empty`, `is_not_empty`).
    #[default]
    None,
    /// A literal value.
    Value(Value),
    /// Several literal values (`in`, `not_in`).
    List(Vec<Value>),
    /// An inclusive range: lowest and highest value (`between`).
    Range(Value, Value),
    /// The value of another field of the same record.
    Field(FieldId),
    /// The current principal (principal fields).
    Me,
    /// A span of days relative to today (date and date-time fields).
    Period(Period),
}

impl Operand {
    /// Whether this is [`Operand::None`].
    #[must_use]
    pub const fn is_none(&self) -> bool {
        matches!(self, Operand::None)
    }
}

/// One comparison of a field.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Test {
    /// The field.
    pub field: FieldId,
    /// The comparison.
    pub op: Operator,
    /// What to compare with.
    #[serde(default, skip_serializing_if = "Operand::is_none")]
    pub operand: Operand,
}

/// A boolean expression over the fields of one record.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    /// True when every part is true (true when there are no parts).
    All(Vec<Condition>),
    /// True when at least one part is true (false when there are no parts).
    Any(Vec<Condition>),
    /// True when the inner condition is false.
    Not(Box<Condition>),
    /// One comparison.
    Test(Test),
}

fn invalid(reason: impl Into<String>) -> CoreError {
    CoreError::InvalidDefinition {
        reason: reason.into(),
    }
}

/// Text as it is compared by conditions: every character replaced by its Unicode lowercase
/// form (without context rules, so the result does not depend on neighbouring characters),
/// plus the three foldings that lowercasing misses and people expect: Greek final sigma
/// equals sigma (`ς` → `σ`), sharp s equals double s (`ß`, `ẞ` → `ss`, so `STRASSE` matches
/// `Straße`) and long s equals s (`ſ` → `s`).
///
/// Storage keeps a column folded by this same function, so SQL matches the same bytes.
#[must_use]
pub fn fold_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for lower in text.chars().flat_map(char::to_lowercase) {
        match lower {
            'ς' => out.push('σ'),
            'ß' => out.push_str("ss"),
            'ſ' => out.push('s'),
            other => out.push(other),
        }
    }
    out
}

/// The items of a list value as raw ids; `None` for values that are not lists.
fn items_of(value: &Value) -> Option<Vec<u128>> {
    match value {
        Value::MultiReference(items) => Some(items.iter().map(|i| i.as_u128()).collect()),
        Value::Link(items) => Some(items.iter().map(|i| i.as_u128()).collect()),
        Value::Principal(items) => Some(items.iter().map(|i| i.as_u128()).collect()),
        Value::Attachment(items) => Some(items.iter().map(|i| i.as_u128()).collect()),
        Value::Null
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
        | Value::Json(_) => None,
    }
}

/// Where a day lies relative to the interval `[first, last]`.
fn date_position(day: Date, first: Date, last: Date) -> Ordering {
    if day < first {
        Ordering::Less
    } else if day > last {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

/// Where an instant lies relative to the days `[first, last]` of the context's time zone.
fn instant_position(at: Timestamp, first: Date, last: Date, ctx: &dyn Context) -> Ordering {
    if at < ctx.start_of_day(first) {
        return Ordering::Less;
    }
    // The interval ends where the day after `last` starts; at the end of the calendar there
    // is no such day and nothing can be later.
    let after = last
        .checked_add_days(1)
        .is_some_and(|next| at >= ctx.start_of_day(next));
    if after {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

/// Where a date or date-time value lies relative to the days `[first, last]`; `None` for
/// other values.
fn position(value: &Value, first: Date, last: Date, ctx: &dyn Context) -> Option<Ordering> {
    match value {
        Value::Date(day) => Some(date_position(*day, first, last)),
        Value::DateTime(at) => Some(instant_position(*at, first, last, ctx)),
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
    }
}

/// Equality as conditions define it (see the module documentation).
fn equal(value: &Value, other: &Value, ctx: &dyn Context) -> bool {
    if let (Some(a), Some(b)) = (value.as_text(), other.as_text()) {
        return fold_text(a) == fold_text(b);
    }
    if let (Some(a), Some(b)) = (items_of(value), items_of(other)) {
        let (a, b): (BTreeSet<u128>, BTreeSet<u128>) =
            (a.into_iter().collect(), b.into_iter().collect());
        return value.data_type() == other.data_type() && a == b;
    }
    if let (Value::DateTime(_), Value::Date(day)) = (value, other) {
        return position(value, *day, *day, ctx) == Some(Ordering::Equal);
    }
    value.data_type() == other.data_type() && compare_values(value, other) == Ordering::Equal
}

/// The order of two values, if they are comparable.
fn order(value: &Value, other: &Value, ctx: &dyn Context) -> Option<Ordering> {
    if let (Some(a), Some(b)) = (value.as_text(), other.as_text()) {
        return Some(fold_text(a).cmp(&fold_text(b)));
    }
    if let (Value::DateTime(_), Value::Date(day)) = (value, other) {
        return position(value, *day, *day, ctx);
    }
    if let (Value::Money(a), Value::Money(b)) = (value, other) {
        return (a.currency() == b.currency()).then(|| compare_values(value, other));
    }
    let ordered = value.data_type().is_some_and(|t| {
        matches!(
            t,
            DataType::Number
                | DataType::Percent
                | DataType::Date
                | DataType::DateTime
                | DataType::Duration
        )
    });
    (ordered && value.data_type() == other.data_type()).then(|| compare_values(value, other))
}

/// The operand of a test with everything looked up.
enum Resolved<'r> {
    One(&'r Value),
    Many(&'r [Value]),
    Range(&'r Value, &'r Value),
    Days(Date, Date),
}

impl Test {
    /// Evaluates the test for one record. `value_of` returns the value of a field of the
    /// record (`None` counts as empty).
    #[must_use]
    pub fn evaluate<'a>(
        &self,
        value_of: &dyn Fn(FieldId) -> Option<&'a Value>,
        ctx: &dyn Context,
    ) -> bool {
        let value = value_of(self.field).filter(|v| !v.is_empty());
        match self.op {
            Operator::IsEmpty => value.is_none(),
            Operator::IsNotEmpty => value.is_some(),
            Operator::Ne => !self.holds(Operator::Eq, value, value_of, ctx),
            Operator::NotContains => !self.holds(Operator::Contains, value, value_of, ctx),
            Operator::NotIn => !self.holds(Operator::In, value, value_of, ctx),
            Operator::HasNone => !self.holds(Operator::HasAny, value, value_of, ctx),
            Operator::Eq
            | Operator::Lt
            | Operator::Le
            | Operator::Gt
            | Operator::Ge
            | Operator::Between
            | Operator::Contains
            | Operator::StartsWith
            | Operator::EndsWith
            | Operator::In
            | Operator::HasAny
            | Operator::HasAll => self.holds(self.op, value, value_of, ctx),
        }
    }

    /// Whether the positive operator `op` holds. False for an empty field or an operand that
    /// cannot be used.
    fn holds<'a>(
        &self,
        op: Operator,
        value: Option<&Value>,
        value_of: &dyn Fn(FieldId) -> Option<&'a Value>,
        ctx: &dyn Context,
    ) -> bool {
        let Some(value) = value else {
            return false;
        };
        let me;
        let resolved = match &self.operand {
            Operand::None => return false,
            Operand::Value(literal) => Resolved::One(literal),
            Operand::List(items) => Resolved::Many(items),
            Operand::Range(low, high) => Resolved::Range(low, high),
            Operand::Field(other) => match value_of(*other).filter(|v| !v.is_empty()) {
                Some(other_value) => Resolved::One(other_value),
                None => return false,
            },
            Operand::Me => match ctx.me() {
                Some(principal) => {
                    me = Value::Principal(vec![principal]);
                    Resolved::One(&me)
                }
                None => return false,
            },
            Operand::Period(period) => match period.resolve(ctx) {
                Some((first, last)) => Resolved::Days(first, last),
                None => return false,
            },
        };
        let is = |wanted: &[Ordering], found: Option<Ordering>| {
            found.is_some_and(|ordering| wanted.contains(&ordering))
        };
        // The position of the value relative to the operand, read as a point or an interval.
        let relative = |resolved: &Resolved<'_>| match resolved {
            Resolved::One(other) => order(value, other, ctx),
            Resolved::Days(first, last) => position(value, *first, *last, ctx),
            Resolved::Many(_) | Resolved::Range(..) => None,
        };
        let text = |matcher: &dyn Fn(&str, &str) -> bool| match &resolved {
            Resolved::One(other) => match (value.as_text(), other.as_text()) {
                (Some(haystack), Some(needle)) => {
                    let needle = fold_text(needle);
                    !needle.is_empty() && matcher(&fold_text(haystack), &needle)
                }
                (Some(_), None) | (None, Some(_)) | (None, None) => false,
            },
            Resolved::Many(_) | Resolved::Range(..) | Resolved::Days(..) => false,
        };
        let lists = |matcher: &dyn Fn(&BTreeSet<u128>, &BTreeSet<u128>) -> bool| match &resolved {
            Resolved::One(other) => match (items_of(value), items_of(other)) {
                (Some(mine), Some(theirs)) => {
                    let mine: BTreeSet<u128> = mine.into_iter().collect();
                    let theirs: BTreeSet<u128> = theirs.into_iter().collect();
                    value.data_type() == other.data_type() && matcher(&mine, &theirs)
                }
                (Some(_), None) | (None, Some(_)) | (None, None) => false,
            },
            Resolved::Many(_) | Resolved::Range(..) | Resolved::Days(..) => false,
        };
        match op {
            Operator::Eq => match &resolved {
                Resolved::One(other) => equal(value, other, ctx),
                Resolved::Days(..) => is(&[Ordering::Equal], relative(&resolved)),
                Resolved::Many(_) | Resolved::Range(..) => false,
            },
            Operator::Lt => is(&[Ordering::Less], relative(&resolved)),
            Operator::Le => is(&[Ordering::Less, Ordering::Equal], relative(&resolved)),
            Operator::Gt => is(&[Ordering::Greater], relative(&resolved)),
            Operator::Ge => is(&[Ordering::Greater, Ordering::Equal], relative(&resolved)),
            Operator::Between => match &resolved {
                Resolved::Range(low, high) => {
                    is(
                        &[Ordering::Greater, Ordering::Equal],
                        order(value, low, ctx),
                    ) && is(&[Ordering::Less, Ordering::Equal], order(value, high, ctx))
                }
                Resolved::Days(..) => is(&[Ordering::Equal], relative(&resolved)),
                Resolved::One(_) | Resolved::Many(_) => false,
            },
            Operator::Contains => text(&|haystack, needle| haystack.contains(needle)),
            Operator::StartsWith => text(&|haystack, needle| haystack.starts_with(needle)),
            Operator::EndsWith => text(&|haystack, needle| haystack.ends_with(needle)),
            Operator::In => match &resolved {
                Resolved::Many(items) => items.iter().any(|item| equal(value, item, ctx)),
                Resolved::One(_) | Resolved::Range(..) | Resolved::Days(..) => false,
            },
            Operator::HasAny => lists(&|mine, theirs| !mine.is_disjoint(theirs)),
            Operator::HasAll => lists(&|mine, theirs| !theirs.is_empty() && theirs.is_subset(mine)),
            // Handled by `evaluate`; as positive operators they never hold.
            Operator::IsEmpty
            | Operator::IsNotEmpty
            | Operator::Ne
            | Operator::NotContains
            | Operator::NotIn
            | Operator::HasNone => false,
        }
    }

    /// Checks the test against the field types.
    fn check(&self, type_of: &dyn Fn(FieldId) -> Option<DataType>) -> Result<(), CoreError> {
        let field_type = type_of(self.field)
            .ok_or_else(|| invalid(format!("a condition uses the unknown field {}", self.field)))?;
        let op = self.op;
        if !op.applies_to(field_type) {
            return Err(invalid(format!(
                "`{op}` cannot be used on a {field_type} field"
            )));
        }
        let wrong_operand = || {
            invalid(format!(
                "`{op}` on a {field_type} field has the wrong kind of operand"
            ))
        };
        let is_date = matches!(field_type, DataType::Date | DataType::DateTime);
        // A literal fits when it is a non-empty value of the field's type; text and long text
        // are interchangeable and a date-time field may be compared with a date.
        let literal_fits = |literal: &Value| {
            let fits = literal.data_type().is_some_and(|t| {
                t == field_type
                    || (t.is_text() && field_type.is_text())
                    || (t == DataType::Date && field_type == DataType::DateTime)
            });
            if literal.is_empty() || !fits {
                Err(invalid(format!(
                    "`{op}` on a {field_type} field needs a non-empty {field_type} value"
                )))
            } else {
                Ok(())
            }
        };
        let field_fits = |other: FieldId| {
            if other == self.field {
                return Err(invalid("a condition compares a field with itself"));
            }
            let other_type = type_of(other)
                .ok_or_else(|| invalid(format!("a condition uses the unknown field {other}")))?;
            if other_type == field_type || (other_type.is_text() && field_type.is_text()) {
                Ok(())
            } else {
                Err(invalid(format!(
                    "a {field_type} field cannot be compared with a {other_type} field"
                )))
            }
        };
        let period_fits = |period: &Period| {
            if !is_date {
                Err(invalid(format!(
                    "a period cannot be compared with a {field_type} field"
                )))
            } else if period.count == 0 {
                Err(invalid("a period spans at least one unit"))
            } else {
                Ok(())
            }
        };
        let me_fits = || {
            if field_type == DataType::Principal {
                Ok(())
            } else {
                Err(invalid(format!(
                    "\"me\" cannot be compared with a {field_type} field"
                )))
            }
        };
        match op {
            Operator::IsEmpty | Operator::IsNotEmpty => match &self.operand {
                Operand::None => Ok(()),
                Operand::Value(_)
                | Operand::List(_)
                | Operand::Range(..)
                | Operand::Field(_)
                | Operand::Me
                | Operand::Period(_) => Err(invalid(format!("`{op}` takes no operand"))),
            },
            Operator::Eq | Operator::Ne => match &self.operand {
                Operand::Value(literal) => literal_fits(literal),
                Operand::Field(other) => field_fits(*other),
                Operand::Me => me_fits(),
                Operand::Period(period) => period_fits(period),
                Operand::None | Operand::List(_) | Operand::Range(..) => Err(wrong_operand()),
            },
            Operator::Lt | Operator::Le | Operator::Gt | Operator::Ge => match &self.operand {
                Operand::Value(literal) => literal_fits(literal),
                Operand::Field(other) => field_fits(*other),
                Operand::Period(period) => period_fits(period),
                Operand::None | Operand::List(_) | Operand::Range(..) | Operand::Me => {
                    Err(wrong_operand())
                }
            },
            Operator::Between => match &self.operand {
                Operand::Range(low, high) => {
                    literal_fits(low)?;
                    literal_fits(high)?;
                    if compare_values(low, high) == Ordering::Greater {
                        return Err(invalid("a range starts above its end"));
                    }
                    Ok(())
                }
                Operand::Period(period) => period_fits(period),
                Operand::None
                | Operand::Value(_)
                | Operand::List(_)
                | Operand::Field(_)
                | Operand::Me => Err(wrong_operand()),
            },
            Operator::Contains
            | Operator::NotContains
            | Operator::StartsWith
            | Operator::EndsWith => match &self.operand {
                Operand::Value(literal) => literal_fits(literal),
                Operand::Field(other) => field_fits(*other),
                Operand::None
                | Operand::List(_)
                | Operand::Range(..)
                | Operand::Me
                | Operand::Period(_) => Err(wrong_operand()),
            },
            Operator::In | Operator::NotIn => match &self.operand {
                Operand::List(items) => {
                    if items.is_empty() {
                        return Err(invalid(format!("`{op}` needs at least one value")));
                    }
                    if items.len() > MAX_OPERAND_ITEMS {
                        return Err(invalid(format!(
                            "`{op}` lists more than {MAX_OPERAND_ITEMS} values"
                        )));
                    }
                    items.iter().try_for_each(literal_fits)
                }
                Operand::None
                | Operand::Value(_)
                | Operand::Range(..)
                | Operand::Field(_)
                | Operand::Me
                | Operand::Period(_) => Err(wrong_operand()),
            },
            Operator::HasAny | Operator::HasAll | Operator::HasNone => match &self.operand {
                Operand::Value(literal) => {
                    literal_fits(literal)?;
                    if literal.item_count().is_some_and(|n| n > MAX_OPERAND_ITEMS) {
                        return Err(invalid(format!(
                            "`{op}` lists more than {MAX_OPERAND_ITEMS} items"
                        )));
                    }
                    Ok(())
                }
                Operand::Field(other) => field_fits(*other),
                Operand::Me => me_fits(),
                Operand::None | Operand::List(_) | Operand::Range(..) | Operand::Period(_) => {
                    Err(wrong_operand())
                }
            },
        }
    }
}

impl Condition {
    /// A condition that is always true.
    #[must_use]
    pub const fn always() -> Self {
        Condition::All(Vec::new())
    }

    /// A condition that is never true.
    #[must_use]
    pub const fn never() -> Self {
        Condition::Any(Vec::new())
    }

    /// One comparison.
    #[must_use]
    pub const fn test(field: FieldId, op: Operator, operand: Operand) -> Self {
        Condition::Test(Test { field, op, operand })
    }

    /// `field` is empty.
    #[must_use]
    pub const fn is_empty(field: FieldId) -> Self {
        Condition::test(field, Operator::IsEmpty, Operand::None)
    }

    /// `field` has a value.
    #[must_use]
    pub const fn is_not_empty(field: FieldId) -> Self {
        Condition::test(field, Operator::IsNotEmpty, Operand::None)
    }

    /// `field` compares like `op` with the literal `value`.
    #[must_use]
    pub const fn compare(field: FieldId, op: Operator, value: Value) -> Self {
        Condition::test(field, op, Operand::Value(value))
    }

    /// The negation of this condition.
    #[must_use]
    pub fn negated(self) -> Self {
        Condition::Not(Box::new(self))
    }

    /// Evaluates the condition for one record. `value_of` returns the value of a field of the
    /// record (`None` counts as empty).
    #[must_use]
    pub fn evaluate<'a>(
        &self,
        value_of: &dyn Fn(FieldId) -> Option<&'a Value>,
        ctx: &dyn Context,
    ) -> bool {
        match self {
            Condition::All(parts) => parts.iter().all(|part| part.evaluate(value_of, ctx)),
            Condition::Any(parts) => parts.iter().any(|part| part.evaluate(value_of, ctx)),
            Condition::Not(inner) => !inner.evaluate(value_of, ctx),
            Condition::Test(test) => test.evaluate(value_of, ctx),
        }
    }

    /// Number of tests in the condition.
    #[must_use]
    pub fn test_count(&self) -> usize {
        match self {
            Condition::All(parts) | Condition::Any(parts) => parts
                .iter()
                .fold(0_usize, |sum, part| sum.saturating_add(part.test_count())),
            Condition::Not(inner) => inner.test_count(),
            Condition::Test(_) => 1,
        }
    }

    /// Nesting depth: 0 for a single test, 1 for a group of tests, and so on.
    #[must_use]
    pub fn depth(&self) -> usize {
        match self {
            Condition::All(parts) | Condition::Any(parts) => parts
                .iter()
                .map(Condition::depth)
                .max()
                .unwrap_or(0)
                .saturating_add(1),
            Condition::Not(inner) => inner.depth().saturating_add(1),
            Condition::Test(_) => 0,
        }
    }

    /// Every field the condition reads (tested fields and field operands). Used to decide
    /// when a condition must be re-evaluated and whether a viewer may see all its inputs.
    #[must_use]
    pub fn fields(&self) -> BTreeSet<FieldId> {
        let mut out = BTreeSet::new();
        self.collect_fields(&mut out);
        out
    }

    fn collect_fields(&self, out: &mut BTreeSet<FieldId>) {
        match self {
            Condition::All(parts) | Condition::Any(parts) => {
                for part in parts {
                    part.collect_fields(out);
                }
            }
            Condition::Not(inner) => inner.collect_fields(out),
            Condition::Test(test) => {
                out.insert(test.field);
                if let Operand::Field(other) = &test.operand {
                    out.insert(*other);
                }
            }
        }
    }

    /// Whether the answer can depend on who asks or on the day it is asked ("me", periods).
    /// Such conditions cannot be cached across users or days.
    #[must_use]
    pub fn is_contextual(&self) -> bool {
        match self {
            Condition::All(parts) | Condition::Any(parts) => {
                parts.iter().any(Condition::is_contextual)
            }
            Condition::Not(inner) => inner.is_contextual(),
            Condition::Test(test) => match &test.operand {
                Operand::Me | Operand::Period(_) => true,
                // A date-time compared with a date depends on the viewer's day boundaries.
                Operand::Value(Value::Date(_)) => true,
                Operand::None
                | Operand::Value(_)
                | Operand::List(_)
                | Operand::Range(..)
                | Operand::Field(_) => false,
            },
        }
    }

    /// Checks the condition against the field types of its table: size limits, known fields,
    /// operators that fit the field type and operands that fit the operator.
    ///
    /// # Errors
    /// [`CoreError::InvalidDefinition`] describing the first problem found.
    pub fn check(&self, type_of: &dyn Fn(FieldId) -> Option<DataType>) -> Result<(), CoreError> {
        if self.depth() > MAX_CONDITION_DEPTH {
            return Err(invalid(format!(
                "a condition is nested deeper than {MAX_CONDITION_DEPTH} levels"
            )));
        }
        if self.test_count() > MAX_FILTER_CONDITIONS {
            return Err(invalid(format!(
                "a condition has more than {MAX_FILTER_CONDITIONS} tests"
            )));
        }
        self.check_tests(type_of)
    }

    fn check_tests(&self, type_of: &dyn Fn(FieldId) -> Option<DataType>) -> Result<(), CoreError> {
        match self {
            Condition::All(parts) | Condition::Any(parts) => {
                parts.iter().try_for_each(|part| part.check_tests(type_of))
            }
            Condition::Not(inner) => inner.check_tests(type_of),
            Condition::Test(test) => test.check(type_of),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::testkit::fid;
    use crate::id::{AttachmentId, RecordId, RefItemId};
    use crate::time::DurationMs;
    use crate::value::testkit::dec;
    use std::collections::BTreeMap;

    type Record = BTreeMap<FieldId, Value>;

    // Field ids used by the tests, with their types.
    const TITLE: u128 = 1;
    const NOTES: u128 = 2;
    const AMOUNT: u128 = 3;
    const PRICE: u128 = 4;
    const DONE: u128 = 5;
    const DUE: u128 = 6;
    const CREATED: u128 = 7;
    const STATUS: u128 = 8;
    const TAGS: u128 = 9;
    const OWNERS: u128 = 10;
    const FILES: u128 = 11;
    const EXTRA: u128 = 12;
    const BUDGET: u128 = 13;
    const START: u128 = 14;
    const LINKS: u128 = 15;
    const RATE: u128 = 16;
    const EFFORT: u128 = 17;
    const WATCHERS: u128 = 18;
    const CODE: u128 = 19;

    fn type_of(field: FieldId) -> Option<DataType> {
        Some(match field.as_u128() {
            TITLE | CODE => DataType::Text,
            NOTES => DataType::LongText,
            AMOUNT | BUDGET => DataType::Number,
            PRICE => DataType::Money,
            DONE => DataType::Bool,
            DUE | START => DataType::Date,
            CREATED => DataType::DateTime,
            STATUS => DataType::Reference,
            TAGS => DataType::MultiReference,
            OWNERS | WATCHERS => DataType::Principal,
            FILES => DataType::Attachment,
            EXTRA => DataType::Json,
            LINKS => DataType::Link,
            RATE => DataType::Percent,
            EFFORT => DataType::Duration,
            _ => return None,
        })
    }

    fn money(amount: &str, currency: &str) -> Value {
        Value::Money(crate::value::testkit::money(amount, currency))
    }

    fn date(y: i32, m: u32, d: u32) -> Date {
        Date::from_ymd(y, m, d).unwrap()
    }

    fn item(n: u128) -> RefItemId {
        RefItemId::from_u128(n)
    }

    fn person(n: u128) -> PrincipalId {
        PrincipalId::from_u128(n)
    }

    fn ctx() -> UtcContext {
        // Wednesday 2026-05-13.
        UtcContext {
            me: Some(person(7)),
            ..UtcContext::on(date(2026, 5, 13))
        }
    }

    fn record() -> Record {
        let mut r = Record::new();
        r.insert(fid(TITLE), Value::Text("Hello World".into()));
        r.insert(fid(NOTES), Value::LongText("Ärzte ohne Grenzen".into()));
        r.insert(fid(AMOUNT), Value::Number(dec("10.50")));
        r.insert(fid(PRICE), money("12.00", "USD"));
        r.insert(fid(DONE), Value::Bool(true));
        r.insert(fid(DUE), Value::Date(date(2026, 5, 15)));
        r.insert(
            fid(CREATED),
            Value::DateTime(
                date(2026, 5, 13)
                    .to_timestamp()
                    .checked_add(DurationMs::from_millis(3_600_000))
                    .unwrap(),
            ),
        );
        r.insert(fid(STATUS), Value::Reference(item(1)));
        r.insert(fid(TAGS), Value::MultiReference(vec![item(1), item(2)]));
        r.insert(fid(OWNERS), Value::Principal(vec![person(7), person(8)]));
        r.insert(fid(BUDGET), Value::Number(dec("20")));
        r.insert(fid(START), Value::Date(date(2026, 5, 1)));
        r.insert(fid(LINKS), Value::Link(vec![RecordId::from_u128(5)]));
        r.insert(fid(RATE), Value::Percent(dec("0.25")));
        r.insert(
            fid(EFFORT),
            Value::Duration(DurationMs::from_millis(5_400_000)),
        );
        r.insert(fid(CODE), Value::Text("hello world".into()));
        r
    }

    fn eval(condition: &Condition, record: &Record) -> bool {
        condition.check(&type_of).unwrap();
        condition.evaluate(&|f| record.get(&f), &ctx())
    }

    fn t(field: u128, op: Operator, operand: Operand) -> Condition {
        Condition::test(fid(field), op, operand)
    }

    fn v(field: u128, op: Operator, value: Value) -> Condition {
        Condition::compare(fid(field), op, value)
    }

    fn text(s: &str) -> Value {
        Value::Text(s.into())
    }

    #[test]
    fn std_emptiness() {
        let r = record();
        assert!(eval(&Condition::is_not_empty(fid(TITLE)), &r));
        assert!(!eval(&Condition::is_empty(fid(TITLE)), &r));
        // A field without a value, a blank text and an empty list are all empty.
        assert!(eval(&Condition::is_empty(fid(FILES)), &r));
        let mut blank = record();
        blank.insert(fid(TITLE), text("   "));
        blank.insert(fid(TAGS), Value::MultiReference(vec![]));
        blank.insert(fid(AMOUNT), Value::Null);
        for field in [TITLE, TAGS, AMOUNT] {
            assert!(eval(&Condition::is_empty(fid(field)), &blank), "{field}");
            assert!(!eval(&Condition::is_not_empty(fid(field)), &blank));
        }
        // False is a value, not "empty".
        let mut unchecked = record();
        unchecked.insert(fid(DONE), Value::Bool(false));
        assert!(eval(&Condition::is_not_empty(fid(DONE)), &unchecked));
    }

    #[test]
    fn std_text_comparisons_fold_case_consistently() {
        let r = record();
        use Operator::{Contains, EndsWith, Eq, Ge, Gt, Le, Lt, Ne, NotContains, StartsWith};
        assert!(eval(&v(TITLE, Eq, text("hello world")), &r));
        assert!(eval(&v(TITLE, Eq, text("HELLO WORLD")), &r));
        assert!(!eval(&v(TITLE, Ne, text("HELLO WORLD")), &r));
        assert!(eval(&v(TITLE, Ne, text("hello")), &r));
        assert!(eval(&v(TITLE, Contains, text("LO wo")), &r));
        assert!(!eval(&v(TITLE, Contains, text("low")), &r));
        assert!(eval(&v(TITLE, NotContains, text("low")), &r));
        assert!(eval(&v(TITLE, StartsWith, text("HELL")), &r));
        assert!(!eval(&v(TITLE, StartsWith, text("world")), &r));
        assert!(eval(&v(TITLE, EndsWith, text("WORLD")), &r));
        // Non-ASCII letters fold too.
        assert!(eval(&v(NOTES, Contains, text("ÄRZTE")), &r));
        assert!(eval(
            &v(NOTES, StartsWith, Value::LongText("ärzte OHNE".into())),
            &r
        ));
        // Ordering agrees with equality: "Hello World" is neither before nor after "hello world".
        assert!(eval(&v(TITLE, Le, text("hello world")), &r));
        assert!(eval(&v(TITLE, Ge, text("HELLO WORLD")), &r));
        assert!(!eval(&v(TITLE, Lt, text("hello world")), &r));
        assert!(!eval(&v(TITLE, Gt, text("hello world")), &r));
        assert!(eval(&v(TITLE, Lt, text("help")), &r));
        assert!(eval(&v(TITLE, Gt, text("HELLO")), &r));
        // Another text field as the operand.
        assert!(eval(&t(TITLE, Eq, Operand::Field(fid(CODE))), &r));
        assert!(eval(&t(NOTES, NotContains, Operand::Field(fid(TITLE))), &r));
        assert_eq!(fold_text("ΟΔΟΣ Straße İ"), "οδοσ strasse i\u{307}");
        // Foldings that plain lowercasing misses.
        assert_eq!(fold_text("Οδυσσεύς"), fold_text("ΟΔΥΣΣΕΎΣ"));
        assert_eq!(fold_text("STRASSE"), fold_text("Straße"));
        assert_eq!(fold_text("GROẞ"), "gross");
        assert_eq!(fold_text("ſtaff"), "staff");
        let mut street = record();
        street.insert(fid(TITLE), text("Hauptstraße 5"));
        assert!(eval(&v(TITLE, Contains, text("STRASSE")), &street));
        assert!(eval(&v(TITLE, Eq, text("HAUPTSTRASSE 5")), &street));
    }

    #[test]
    fn std_numeric_money_percent_duration_comparisons() {
        let r = record();
        use Operator::{Between, Eq, Ge, Gt, In, Le, Lt, Ne, NotIn};
        let n = |s: &str| Value::Number(dec(s));
        assert!(eval(&v(AMOUNT, Eq, n("10.5")), &r));
        assert!(eval(&v(AMOUNT, Ne, n("10.51")), &r));
        assert!(eval(&v(AMOUNT, Gt, n("10")), &r));
        assert!(eval(&v(AMOUNT, Ge, n("10.5")), &r));
        assert!(!eval(&v(AMOUNT, Lt, n("10.5")), &r));
        assert!(eval(&v(AMOUNT, Le, n("10.5")), &r));
        assert!(eval(
            &t(AMOUNT, Between, Operand::Range(n("10.5"), n("11"))),
            &r
        ));
        assert!(eval(
            &t(AMOUNT, Between, Operand::Range(n("0"), n("10.5"))),
            &r
        ));
        assert!(!eval(
            &t(AMOUNT, Between, Operand::Range(n("10.6"), n("11"))),
            &r
        ));
        assert!(eval(
            &t(AMOUNT, In, Operand::List(vec![n("1"), n("10.50")])),
            &r
        ));
        assert!(eval(
            &t(AMOUNT, NotIn, Operand::List(vec![n("1"), n("2")])),
            &r
        ));
        // Another field as the operand: amount < budget.
        assert!(eval(&t(AMOUNT, Lt, Operand::Field(fid(BUDGET))), &r));
        assert!(!eval(&t(BUDGET, Le, Operand::Field(fid(AMOUNT))), &r));

        assert!(eval(&v(PRICE, Eq, money("12", "USD")), &r));
        assert!(eval(&v(PRICE, Gt, money("11.99", "USD")), &r));
        assert!(eval(&v(RATE, Ge, Value::Percent(dec("0.25"))), &r));
        assert!(eval(&v(RATE, Lt, Value::Percent(dec("0.3"))), &r));
        let minutes = |m: i64| Value::Duration(DurationMs::from_millis(m * 60_000));
        assert!(eval(&v(EFFORT, Eq, minutes(90)), &r));
        assert!(eval(
            &t(EFFORT, Between, Operand::Range(minutes(60), minutes(120))),
            &r
        ));
        assert!(eval(&v(DONE, Eq, Value::Bool(true)), &r));
        assert!(eval(&v(DONE, Ne, Value::Bool(false)), &r));
    }

    #[test]
    fn edge_money_in_another_currency_is_neither_equal_nor_ordered() {
        let r = record();
        use Operator::{Between, Eq, Ge, Gt, Le, Lt, Ne};
        let eur = money("12.00", "EUR");
        assert!(!eval(&v(PRICE, Eq, eur.clone()), &r));
        assert!(eval(&v(PRICE, Ne, eur.clone()), &r));
        for op in [Lt, Le, Gt, Ge] {
            assert!(!eval(&v(PRICE, op, eur.clone()), &r), "{op}");
        }
        let range = Operand::Range(money("1", "EUR"), money("99", "EUR"));
        assert!(!eval(&t(PRICE, Between, range), &r));
    }

    #[test]
    fn std_references_and_lists() {
        let r = record();
        use Operator::{Eq, HasAll, HasAny, HasNone, In, Ne, NotIn};
        let refs = |ids: &[u128]| Value::MultiReference(ids.iter().map(|n| item(*n)).collect());
        assert!(eval(&v(STATUS, Eq, Value::Reference(item(1))), &r));
        assert!(eval(&v(STATUS, Ne, Value::Reference(item(2))), &r));
        let list =
            |ids: &[u128]| Operand::List(ids.iter().map(|n| Value::Reference(item(*n))).collect());
        assert!(eval(&t(STATUS, In, list(&[3, 1])), &r));
        assert!(eval(&t(STATUS, NotIn, list(&[3, 4])), &r));

        // Tags are {1, 2}.
        assert!(eval(&v(TAGS, HasAny, refs(&[2, 9])), &r));
        assert!(!eval(&v(TAGS, HasAny, refs(&[8, 9])), &r));
        assert!(eval(&v(TAGS, HasAll, refs(&[2, 1])), &r));
        assert!(!eval(&v(TAGS, HasAll, refs(&[1, 2, 3])), &r));
        assert!(eval(&v(TAGS, HasNone, refs(&[8, 9])), &r));
        assert!(!eval(&v(TAGS, HasNone, refs(&[1])), &r));
        // "Is exactly": the same set, whatever the order or repetition.
        assert!(eval(&v(TAGS, Eq, refs(&[2, 1, 1])), &r));
        assert!(!eval(&v(TAGS, Eq, refs(&[1])), &r));
        assert!(eval(&v(TAGS, Ne, refs(&[1])), &r));

        let links = Value::Link(vec![RecordId::from_u128(5), RecordId::from_u128(6)]);
        assert!(eval(&v(LINKS, HasAny, links.clone()), &r));
        assert!(!eval(&v(LINKS, HasAll, links), &r));
        // Another list field of the same type as the operand.
        let mut both = record();
        both.insert(fid(WATCHERS), Value::Principal(vec![person(8), person(9)]));
        assert!(eval(
            &t(OWNERS, HasAny, Operand::Field(fid(WATCHERS))),
            &both
        ));
        assert!(!eval(
            &t(OWNERS, HasAll, Operand::Field(fid(WATCHERS))),
            &both
        ));
    }

    #[test]
    fn std_me() {
        use Operator::{Eq, HasAll, HasAny, HasNone, Ne};
        let r = record(); // owners are {7, 8}; "me" is 7
        assert!(eval(&t(OWNERS, HasAny, Operand::Me), &r));
        assert!(eval(&t(OWNERS, HasAll, Operand::Me), &r));
        assert!(!eval(&t(OWNERS, HasNone, Operand::Me), &r));
        // "Is exactly me" is false while someone else is on the record too.
        assert!(!eval(&t(OWNERS, Eq, Operand::Me), &r));
        assert!(eval(&t(OWNERS, Ne, Operand::Me), &r));
        let mut mine = record();
        mine.insert(fid(OWNERS), Value::Principal(vec![person(7)]));
        assert!(eval(&t(OWNERS, Eq, Operand::Me), &mine));

        // Nobody signed in: positive tests are false, negative ones true.
        let anonymous = UtcContext::on(date(2026, 5, 13));
        let run = |c: &Condition| c.evaluate(&|f| r.get(&f), &anonymous);
        assert!(!run(&t(OWNERS, HasAny, Operand::Me)));
        assert!(run(&t(OWNERS, HasNone, Operand::Me)));
        assert!(!run(&t(OWNERS, Eq, Operand::Me)));
    }

    #[test]
    fn std_dates_and_periods() {
        use Operator::{Between, Eq, Ge, Gt, Le, Lt, Ne};
        use PeriodUnit::{Day, Month, Week};
        let r = record(); // due 2026-05-15 (Friday); today is Wednesday 2026-05-13
        let p = |period: Period| Operand::Period(period);
        assert!(eval(&v(DUE, Eq, Value::Date(date(2026, 5, 15))), &r));
        assert!(eval(&v(DUE, Gt, Value::Date(date(2026, 5, 14))), &r));
        assert!(eval(&t(DUE, Gt, Operand::Field(fid(START))), &r));
        assert!(eval(
            &t(
                DUE,
                Between,
                Operand::Range(
                    Value::Date(date(2026, 5, 15)),
                    Value::Date(date(2026, 5, 31))
                )
            ),
            &r
        ));

        // This week is Mon 11 – Sun 17 May.
        assert!(eval(&t(DUE, Eq, p(Period::this(Week))), &r));
        assert!(eval(&t(DUE, Between, p(Period::this(Week))), &r));
        assert!(!eval(&t(DUE, Ne, p(Period::this(Week))), &r));
        assert!(!eval(&t(DUE, Eq, p(Period::this(Day))), &r));
        assert!(eval(&t(DUE, Gt, p(Period::this(Day))), &r)); // after today
        assert!(eval(&t(DUE, Ge, p(Period::this(Day))), &r)); // today or later
        assert!(!eval(&t(DUE, Lt, p(Period::this(Day))), &r)); // overdue?
        assert!(!eval(&t(DUE, Le, p(Period::this(Day))), &r));
        assert!(eval(&t(DUE, Le, p(Period::this(Week))), &r)); // by the end of this week
        assert!(eval(&t(DUE, Lt, p(Period::next(Week))), &r)); // before next week starts
        assert!(eval(&t(DUE, Eq, p(Period::next_days(3))), &r)); // within the next 3 days
        assert!(!eval(&t(DUE, Eq, p(Period::next_days(2))), &r));
        assert!(eval(&t(START, Eq, p(Period::last_days(13))), &r)); // 1 May is 12 days ago
        assert!(!eval(&t(START, Eq, p(Period::last_days(12))), &r));
        assert!(eval(&t(START, Eq, p(Period::this(Month))), &r));
        assert!(eval(&t(START, Lt, p(Period::this(Week))), &r));
    }

    #[test]
    fn std_date_time_fields_use_the_context_day_boundaries() {
        use Operator::{Eq, Ge, Gt, Le, Lt};
        // Created 2026-05-13 01:00 UTC.
        let r = record();
        let today = Value::Date(date(2026, 5, 13));
        assert!(eval(&v(CREATED, Eq, today.clone()), &r));
        assert!(eval(&v(CREATED, Le, today.clone()), &r));
        assert!(eval(&v(CREATED, Ge, today.clone()), &r));
        assert!(!eval(&v(CREATED, Lt, today.clone()), &r));
        assert!(!eval(&v(CREATED, Gt, today), &r));
        assert!(eval(&v(CREATED, Gt, Value::Date(date(2026, 5, 12))), &r));
        assert!(eval(
            &t(CREATED, Eq, Operand::Period(Period::this(PeriodUnit::Day))),
            &r
        ));
        let exact = r.get(&fid(CREATED)).unwrap().clone();
        assert!(eval(&v(CREATED, Eq, exact.clone()), &r));
        assert!(eval(&v(CREATED, Le, exact), &r));

        // A viewer whose day starts 9 hours before UTC midnight (UTC+9): 01:00 UTC on the
        // 13th is 10:00 on the 13th there; 20:00 UTC on the 12th would also be the 13th.
        struct Tokyo;
        impl Context for Tokyo {
            fn me(&self) -> Option<PrincipalId> {
                None
            }
            fn today(&self) -> Date {
                Date::from_ymd(2026, 5, 13).unwrap()
            }
            fn week_start(&self) -> Weekday {
                Weekday::Sunday
            }
            fn fiscal_year_start_month(&self) -> u32 {
                4
            }
            fn start_of_day(&self, date: Date) -> Timestamp {
                Timestamp::from_millis(date.to_timestamp().millis() - 9 * 3_600_000).unwrap()
            }
        }
        let mut late = record();
        late.insert(
            fid(CREATED),
            Value::DateTime(
                Timestamp::from_millis(date(2026, 5, 12).to_timestamp().millis() + 20 * 3_600_000)
                    .unwrap(),
            ),
        );
        let on_13th = v(CREATED, Eq, Value::Date(date(2026, 5, 13)));
        assert!(on_13th.evaluate(&|f| late.get(&f), &Tokyo));
        assert!(!on_13th.evaluate(&|f| late.get(&f), &ctx()));
        assert!(on_13th.is_contextual());
    }

    #[test]
    fn std_period_resolution() {
        use PeriodUnit::{Day, FiscalQuarter, FiscalYear, Month, Quarter, Week, Year};
        let c = ctx(); // Wednesday 2026-05-13, Monday weeks, fiscal year from January
        let span = |period: Period, c: &UtcContext| period.resolve(c).unwrap();
        let d = date;
        assert_eq!(
            span(Period::this(Day), &c),
            (d(2026, 5, 13), d(2026, 5, 13))
        );
        assert_eq!(
            span(Period::last(Day), &c),
            (d(2026, 5, 12), d(2026, 5, 12))
        );
        assert_eq!(
            span(Period::next(Day), &c),
            (d(2026, 5, 14), d(2026, 5, 14))
        );
        assert_eq!(
            span(Period::last_days(7), &c),
            (d(2026, 5, 7), d(2026, 5, 13))
        );
        assert_eq!(
            span(Period::next_days(30), &c),
            (d(2026, 5, 13), d(2026, 6, 11))
        );
        assert_eq!(
            span(Period::this(Week), &c),
            (d(2026, 5, 11), d(2026, 5, 17))
        );
        assert_eq!(
            span(Period::last(Week), &c),
            (d(2026, 5, 4), d(2026, 5, 10))
        );
        assert_eq!(
            span(Period::next(Week), &c),
            (d(2026, 5, 18), d(2026, 5, 24))
        );
        assert_eq!(
            span(Period::this(Month), &c),
            (d(2026, 5, 1), d(2026, 5, 31))
        );
        assert_eq!(
            span(Period::last(Month), &c),
            (d(2026, 4, 1), d(2026, 4, 30))
        );
        assert_eq!(
            span(
                Period {
                    unit: Month,
                    offset: -3,
                    count: 3
                },
                &c
            ),
            (d(2026, 2, 1), d(2026, 4, 30))
        );
        assert_eq!(
            span(Period::this(Quarter), &c),
            (d(2026, 4, 1), d(2026, 6, 30))
        );
        assert_eq!(
            span(Period::last(Quarter), &c),
            (d(2026, 1, 1), d(2026, 3, 31))
        );
        assert_eq!(
            span(Period::this(Year), &c),
            (d(2026, 1, 1), d(2026, 12, 31))
        );
        assert_eq!(
            span(Period::last(Year), &c),
            (d(2025, 1, 1), d(2025, 12, 31))
        );
        // With a January fiscal year, fiscal units equal calendar units.
        assert_eq!(
            span(Period::this(FiscalYear), &c),
            span(Period::this(Year), &c)
        );
        assert_eq!(
            span(Period::this(FiscalQuarter), &c),
            span(Period::this(Quarter), &c)
        );

        // Sunday weeks; fiscal year from April (Japan, India).
        let april = UtcContext {
            week_start: Weekday::Sunday,
            fiscal_year_start_month: 4,
            ..c
        };
        assert_eq!(
            span(Period::this(Week), &april),
            (d(2026, 5, 10), d(2026, 5, 16))
        );
        assert_eq!(
            span(Period::this(FiscalYear), &april),
            (d(2026, 4, 1), d(2027, 3, 31))
        );
        assert_eq!(
            span(Period::last(FiscalYear), &april),
            (d(2025, 4, 1), d(2026, 3, 31))
        );
        assert_eq!(
            span(Period::this(FiscalQuarter), &april),
            (d(2026, 4, 1), d(2026, 6, 30))
        );
        // In February the fiscal year that started last April is still running.
        let february = UtcContext {
            today: d(2026, 2, 10),
            ..april
        };
        assert_eq!(
            span(Period::this(FiscalYear), &february),
            (d(2025, 4, 1), d(2026, 3, 31))
        );
        assert_eq!(
            span(Period::this(FiscalQuarter), &february),
            (d(2026, 1, 1), d(2026, 3, 31))
        );
        // Fiscal year from October (US federal), asked in November.
        let october = UtcContext {
            today: d(2026, 11, 2),
            fiscal_year_start_month: 10,
            ..c
        };
        assert_eq!(
            span(Period::this(FiscalYear), &october),
            (d(2026, 10, 1), d(2027, 9, 30))
        );
        assert_eq!(
            span(Period::next(FiscalQuarter), &october),
            (d(2027, 1, 1), d(2027, 3, 31))
        );
        // An out-of-range start month is clamped rather than trusted.
        let odd = UtcContext {
            fiscal_year_start_month: 99,
            ..c
        };
        assert_eq!(
            span(Period::this(FiscalYear), &odd),
            (d(2025, 12, 1), d(2026, 11, 30))
        );
    }

    #[test]
    fn edge_week_start_on_every_weekday_contains_today() {
        for today_offset in 0..14 {
            let today = date(2026, 5, 4).checked_add_days(today_offset).unwrap();
            for week_start in Weekday::ALL {
                let c = UtcContext {
                    week_start: *week_start,
                    ..UtcContext::on(today)
                };
                let (first, last) = Period::this(PeriodUnit::Week).resolve(&c).unwrap();
                assert_eq!(first.weekday(), *week_start);
                assert_eq!(last.days() - first.days(), 6);
                assert!(first <= today && today <= last);
            }
        }
    }

    #[test]
    fn edge_periods_at_the_ends_of_the_calendar() {
        use PeriodUnit::{Day, Month, Week, Year};
        let end = UtcContext::on(Date::MAX);
        let start = UtcContext::on(Date::MIN);
        assert_eq!(
            Period::this(Day).resolve(&end),
            Some((Date::MAX, Date::MAX))
        );
        assert_eq!(Period::next(Day).resolve(&end), None);
        assert_eq!(Period::last(Day).resolve(&start), None);
        // The month after December 9999 does not exist, so the last month cannot be closed.
        assert_eq!(Period::this(Month).resolve(&end), None);
        assert_eq!(Period::this(Year).resolve(&end), None);
        assert_eq!(Period::last(Week).resolve(&start), None);
        assert_eq!(
            Period {
                unit: Day,
                offset: 0,
                count: 0
            }
            .resolve(&end),
            None
        );
        assert_eq!(
            Period {
                unit: Month,
                offset: i32::MAX,
                count: u32::MAX
            }
            .resolve(&end),
            None
        );
        assert_eq!(
            Period {
                unit: Week,
                offset: i32::MIN,
                count: 1
            }
            .resolve(&end),
            None
        );
        assert_eq!(
            Period::last_days(0),
            Period {
                unit: Day,
                offset: 0,
                count: 0
            }
        );
        assert_eq!(Period::last_days(u32::MAX).offset, -i32::MAX);

        // A date-time on the very last day is "on" that day, not after it.
        let last_instant = Value::DateTime(Timestamp::MAX);
        assert_eq!(
            position(&last_instant, Date::MAX, Date::MAX, &end),
            Some(Ordering::Equal)
        );
        // An unresolvable period makes positive tests false and negative ones true.
        let mut r = Record::new();
        r.insert(fid(DUE), Value::Date(Date::MAX));
        let within = t(DUE, Operator::Eq, Operand::Period(Period::this(Month)));
        assert!(!within.evaluate(&|f| r.get(&f), &end));
        let outside = t(DUE, Operator::Ne, Operand::Period(Period::this(Month)));
        assert!(outside.evaluate(&|f| r.get(&f), &end));
    }

    #[test]
    fn std_empty_fields_make_positive_tests_false_and_negative_tests_true() {
        use Operator::{
            Between, Contains, EndsWith, Eq, Ge, Gt, HasAll, HasAny, HasNone, In, Le, Lt, Ne,
            NotContains, NotIn, StartsWith,
        };
        let empty = Record::new();
        let n = Value::Number(dec("1"));
        let refs = Value::MultiReference(vec![item(1)]);
        let positive = [
            v(AMOUNT, Eq, n.clone()),
            v(AMOUNT, Lt, n.clone()),
            v(AMOUNT, Le, n.clone()),
            v(AMOUNT, Gt, n.clone()),
            v(AMOUNT, Ge, n.clone()),
            t(AMOUNT, Between, Operand::Range(n.clone(), n.clone())),
            t(AMOUNT, In, Operand::List(vec![n.clone()])),
            v(TITLE, Contains, text("a")),
            v(TITLE, StartsWith, text("a")),
            v(TITLE, EndsWith, text("a")),
            v(TAGS, HasAny, refs.clone()),
            v(TAGS, HasAll, refs.clone()),
            t(DUE, Eq, Operand::Period(Period::this(PeriodUnit::Week))),
            t(OWNERS, HasAny, Operand::Me),
        ];
        for condition in &positive {
            assert!(!eval(condition, &empty), "{condition:?}");
        }
        let negative = [
            v(AMOUNT, Ne, n.clone()),
            t(AMOUNT, NotIn, Operand::List(vec![n])),
            v(TITLE, NotContains, text("a")),
            v(TAGS, HasNone, refs),
            t(DUE, Ne, Operand::Period(Period::this(PeriodUnit::Week))),
            t(OWNERS, HasNone, Operand::Me),
        ];
        for condition in &negative {
            assert!(eval(condition, &empty), "{condition:?}");
        }
        // A field operand that is empty behaves the same way.
        let r = record(); // no value in EXTRA / WATCHERS
        assert!(!eval(&t(OWNERS, HasAny, Operand::Field(fid(WATCHERS))), &r));
        assert!(eval(&t(OWNERS, HasNone, Operand::Field(fid(WATCHERS))), &r));
    }

    #[test]
    fn std_groups_and_negation() {
        let r = record();
        let yes = Condition::is_not_empty(fid(TITLE));
        let no = Condition::is_empty(fid(TITLE));
        assert!(eval(&Condition::always(), &r));
        assert!(!eval(&Condition::never(), &r));
        assert!(eval(&Condition::All(vec![yes.clone(), yes.clone()]), &r));
        assert!(!eval(&Condition::All(vec![yes.clone(), no.clone()]), &r));
        assert!(eval(&Condition::Any(vec![no.clone(), yes.clone()]), &r));
        assert!(!eval(&Condition::Any(vec![no.clone(), no.clone()]), &r));
        assert!(eval(&no.clone().negated(), &r));
        assert!(!eval(&yes.clone().negated(), &r));
        let nested = Condition::All(vec![
            Condition::Any(vec![no.clone(), Condition::All(vec![yes.clone()])]),
            Condition::Not(Box::new(Condition::Any(vec![no.clone()]))),
        ]);
        assert!(eval(&nested, &r));
        assert_eq!(nested.test_count(), 3);
        assert_eq!(nested.depth(), 3);
        assert_eq!(yes.depth(), 0);
        assert_eq!(Condition::always().depth(), 1);
        assert_eq!(Condition::always().test_count(), 0);
    }

    #[test]
    fn std_fields_and_context_dependence() {
        let condition = Condition::All(vec![
            t(AMOUNT, Operator::Lt, Operand::Field(fid(BUDGET))),
            Condition::Any(vec![Condition::is_empty(fid(TITLE)).negated()]),
        ]);
        assert_eq!(
            condition.fields().into_iter().collect::<Vec<_>>(),
            vec![fid(TITLE), fid(AMOUNT), fid(BUDGET)]
        );
        assert!(!condition.is_contextual());
        assert!(t(OWNERS, Operator::HasAny, Operand::Me).is_contextual());
        assert!(Condition::All(vec![t(
            DUE,
            Operator::Lt,
            Operand::Period(Period::this(PeriodUnit::Day))
        )])
        .is_contextual());
        assert!(t(
            DUE,
            Operator::Eq,
            Operand::Period(Period::this(PeriodUnit::Day))
        )
        .negated()
        .is_contextual());
        assert!(!v(AMOUNT, Operator::Eq, Value::Number(dec("1"))).is_contextual());
        assert!(!Condition::always().is_contextual());
    }

    #[test]
    fn std_serde_forms() {
        let condition = Condition::All(vec![
            Condition::is_not_empty(fid(TITLE)),
            t(OWNERS, Operator::HasAny, Operand::Me),
            t(
                DUE,
                Operator::Le,
                Operand::Period(Period::this(PeriodUnit::Week)),
            ),
            v(AMOUNT, Operator::Gt, Value::Number(dec("5"))).negated(),
            t(
                AMOUNT,
                Operator::Between,
                Operand::Range(Value::Number(dec("1")), Value::Number(dec("2"))),
            ),
            t(
                STATUS,
                Operator::In,
                Operand::List(vec![Value::Reference(item(1))]),
            ),
            t(AMOUNT, Operator::Lt, Operand::Field(fid(BUDGET))),
            Condition::Any(vec![]),
        ]);
        let json = serde_json::to_string(&condition).unwrap();
        assert_eq!(serde_json::from_str::<Condition>(&json).unwrap(), condition);

        // The wire form is readable and stable.
        let simple = Condition::is_empty(fid(TITLE));
        assert_eq!(
            serde_json::to_string(&simple).unwrap(),
            r#"{"test":{"field":"fld_00000000000000000000000001","op":"is_empty"}}"#
        );
        let me = t(OWNERS, Operator::HasAny, Operand::Me);
        assert!(serde_json::to_string(&me)
            .unwrap()
            .ends_with(r#""op":"has_any","operand":{"kind":"me"}}}"#));
        // Defaults: a period without offset and count is "this unit".
        let period: Period = serde_json::from_str(r#"{"unit":"week"}"#).unwrap();
        assert_eq!(period, Period::this(PeriodUnit::Week));
        assert!(serde_json::from_str::<Condition>(
            r#"{"test":{"field":"fld_00000000000000000000000001","op":"equals"}}"#
        )
        .is_err());
        for op in Operator::ALL {
            assert_eq!(op.as_str().parse::<Operator>().unwrap(), *op);
        }
        for unit in PeriodUnit::ALL {
            assert_eq!(unit.to_string().parse::<PeriodUnit>().unwrap(), *unit);
        }
    }

    fn reason(result: Result<(), CoreError>) -> String {
        match result {
            Err(CoreError::InvalidDefinition { reason }) => reason,
            other => panic!("expected InvalidDefinition, got {other:?}"),
        }
    }

    #[test]
    fn invalid_operators_for_field_types() {
        // Which operators each data type accepts.
        let ordered = [
            DataType::Text,
            DataType::Number,
            DataType::Money,
            DataType::Percent,
            DataType::Date,
            DataType::DateTime,
            DataType::Duration,
        ];
        for data_type in DataType::ALL {
            for op in Operator::ALL {
                let expected = match op {
                    Operator::IsEmpty | Operator::IsNotEmpty | Operator::Eq | Operator::Ne => true,
                    Operator::Lt
                    | Operator::Le
                    | Operator::Gt
                    | Operator::Ge
                    | Operator::Between => ordered.contains(data_type),
                    Operator::Contains
                    | Operator::NotContains
                    | Operator::StartsWith
                    | Operator::EndsWith => {
                        matches!(data_type, DataType::Text | DataType::LongText)
                    }
                    Operator::In | Operator::NotIn => {
                        ordered.contains(data_type) || *data_type == DataType::Reference
                    }
                    Operator::HasAny | Operator::HasAll | Operator::HasNone => data_type.is_list(),
                };
                assert_eq!(op.applies_to(*data_type), expected, "{op} on {data_type}");
            }
        }
        let c = v(DONE, Operator::Gt, Value::Bool(true));
        assert!(reason(c.check(&type_of)).contains("`gt` cannot be used on a bool field"));
        let c = v(EXTRA, Operator::Contains, text("x"));
        assert!(reason(c.check(&type_of)).contains("cannot be used on a json field"));
        let c = v(TITLE, Operator::HasAny, text("x"));
        assert!(reason(c.check(&type_of)).contains("cannot be used"));
    }

    #[test]
    fn invalid_operands() {
        use Operator::{Between, Contains, Eq, HasAny, In, IsEmpty, Lt};
        let n = |s: &str| Value::Number(dec(s));
        let cases: Vec<(Condition, &str)> = vec![
            (t(99, Eq, Operand::Value(n("1"))), "unknown field"),
            (t(AMOUNT, Lt, Operand::Field(fid(98))), "unknown field"),
            (
                t(TITLE, IsEmpty, Operand::Value(text("x"))),
                "takes no operand",
            ),
            (t(AMOUNT, Eq, Operand::None), "wrong kind of operand"),
            (
                t(AMOUNT, Eq, Operand::List(vec![n("1")])),
                "wrong kind of operand",
            ),
            (
                t(AMOUNT, Eq, Operand::Value(text("1"))),
                "needs a non-empty number value",
            ),
            (
                t(AMOUNT, Eq, Operand::Value(Value::Null)),
                "needs a non-empty number value",
            ),
            (
                t(TITLE, Eq, Operand::Value(text("  "))),
                "needs a non-empty text value",
            ),
            (t(AMOUNT, Eq, Operand::Me), "\"me\" cannot be compared"),
            (
                t(AMOUNT, Eq, Operand::Period(Period::this(PeriodUnit::Day))),
                "a period cannot be compared",
            ),
            (
                t(
                    DUE,
                    Eq,
                    Operand::Period(Period {
                        unit: PeriodUnit::Day,
                        offset: 0,
                        count: 0,
                    }),
                ),
                "at least one unit",
            ),
            (t(AMOUNT, Lt, Operand::Me), "wrong kind of operand"),
            (t(AMOUNT, Lt, Operand::Field(fid(AMOUNT))), "with itself"),
            (
                t(AMOUNT, Lt, Operand::Field(fid(TITLE))),
                "cannot be compared with a text field",
            ),
            (
                t(DUE, Lt, Operand::Value(Value::DateTime(Timestamp::EPOCH))),
                "needs a non-empty date value",
            ),
            (
                t(AMOUNT, Between, Operand::Value(n("1"))),
                "wrong kind of operand",
            ),
            (
                t(AMOUNT, Between, Operand::Range(n("2"), n("1"))),
                "starts above its end",
            ),
            (
                t(AMOUNT, Between, Operand::Range(n("1"), text("2"))),
                "needs a non-empty number value",
            ),
            (
                t(AMOUNT, Between, Operand::Range(text("1"), n("2"))),
                "needs a non-empty number value",
            ),
            (
                t(
                    AMOUNT,
                    Between,
                    Operand::Period(Period::this(PeriodUnit::Day)),
                ),
                "a period cannot be compared",
            ),
            (
                t(TITLE, Contains, Operand::Value(text(""))),
                "needs a non-empty text value",
            ),
            (t(TITLE, Contains, Operand::Me), "wrong kind of operand"),
            (
                t(TITLE, Contains, Operand::Field(fid(AMOUNT))),
                "cannot be compared with a number field",
            ),
            (t(AMOUNT, In, Operand::List(vec![])), "at least one value"),
            (
                t(
                    AMOUNT,
                    In,
                    Operand::List(vec![n("1"); MAX_OPERAND_ITEMS + 1]),
                ),
                "more than 1000 values",
            ),
            (
                t(AMOUNT, In, Operand::List(vec![n("1"), text("x")])),
                "needs a non-empty number value",
            ),
            (
                t(AMOUNT, In, Operand::Value(n("1"))),
                "wrong kind of operand",
            ),
            (
                t(TAGS, HasAny, Operand::Value(Value::MultiReference(vec![]))),
                "needs a non-empty multi_reference value",
            ),
            (
                t(
                    TAGS,
                    HasAny,
                    Operand::Value(Value::Link(vec![RecordId::from_u128(1)])),
                ),
                "needs a non-empty multi_reference value",
            ),
            (
                t(
                    TAGS,
                    HasAny,
                    Operand::Value(Value::MultiReference(vec![item(1); MAX_OPERAND_ITEMS + 1])),
                ),
                "more than 1000 items",
            ),
            (t(TAGS, HasAny, Operand::Me), "\"me\" cannot be compared"),
            (
                t(TAGS, HasAny, Operand::List(vec![])),
                "wrong kind of operand",
            ),
            (
                t(TAGS, HasAny, Operand::Field(fid(OWNERS))),
                "cannot be compared with a principal field",
            ),
        ];
        for (condition, expected) in cases {
            let got = reason(condition.check(&type_of));
            assert!(got.contains(expected), "{condition:?}: {got}");
        }
        // Accepted combinations that look unusual.
        let ok = [
            t(TITLE, Eq, Operand::Value(Value::LongText("x".into()))),
            t(NOTES, Contains, Operand::Field(fid(TITLE))),
            t(CREATED, Lt, Operand::Value(Value::Date(date(2026, 1, 1)))),
            t(
                CREATED,
                Between,
                Operand::Range(Value::Date(date(2026, 1, 1)), Value::Date(date(2026, 1, 2))),
            ),
            t(OWNERS, Eq, Operand::Me),
            t(
                FILES,
                HasAny,
                Operand::Value(Value::Attachment(vec![AttachmentId::from_u128(1)])),
            ),
            t(
                EXTRA,
                Eq,
                Operand::Value(Value::Json(serde_json::json!({"a": 1}))),
            ),
        ];
        for condition in ok {
            condition.check(&type_of).unwrap();
        }
    }

    #[test]
    fn invalid_size_limits() {
        let leaf = Condition::is_empty(fid(TITLE));
        let mut deep = leaf.clone();
        for _ in 0..MAX_CONDITION_DEPTH {
            deep = deep.negated();
        }
        deep.check(&type_of).unwrap();
        let too_deep = deep.negated();
        assert!(reason(too_deep.check(&type_of)).contains("nested deeper"));

        let wide = Condition::All(vec![leaf.clone(); MAX_FILTER_CONDITIONS]);
        wide.check(&type_of).unwrap();
        let too_wide = Condition::All(vec![leaf; MAX_FILTER_CONDITIONS + 1]);
        assert!(reason(too_wide.check(&type_of)).contains("more than 100 tests"));
    }

    #[test]
    fn anomalous_ill_typed_tests_never_fail_and_read_as_a_mismatch() {
        // These would be rejected by `check`; evaluation must still be total.
        let r = record();
        let run = |c: &Condition| c.evaluate(&|f| r.get(&f), &ctx());
        let n = Value::Number(dec("1"));
        let positive = [
            t(TITLE, Operator::Eq, Operand::Value(n.clone())),
            t(TITLE, Operator::Lt, Operand::Value(n.clone())),
            t(TITLE, Operator::Eq, Operand::None),
            t(
                TITLE,
                Operator::Eq,
                Operand::List(vec![text("hello world")]),
            ),
            t(TITLE, Operator::Eq, Operand::Range(text("a"), text("z"))),
            t(TITLE, Operator::Lt, Operand::Range(text("a"), text("z"))),
            t(TITLE, Operator::Lt, Operand::List(vec![text("z")])),
            t(TITLE, Operator::Between, Operand::Value(text("z"))),
            t(TITLE, Operator::Between, Operand::List(vec![text("z")])),
            t(TITLE, Operator::Contains, Operand::Value(n.clone())),
            t(TITLE, Operator::Contains, Operand::Value(text(""))),
            t(TITLE, Operator::Contains, Operand::List(vec![text("h")])),
            t(
                TITLE,
                Operator::Contains,
                Operand::Range(text("a"), text("z")),
            ),
            t(
                TITLE,
                Operator::Contains,
                Operand::Period(Period::this(PeriodUnit::Day)),
            ),
            t(AMOUNT, Operator::Contains, Operand::Value(text("1"))),
            t(AMOUNT, Operator::Contains, Operand::Value(n.clone())),
            t(TITLE, Operator::In, Operand::Value(text("hello world"))),
            t(TITLE, Operator::In, Operand::Range(text("a"), text("z"))),
            t(
                TITLE,
                Operator::In,
                Operand::Period(Period::this(PeriodUnit::Day)),
            ),
            t(TITLE, Operator::HasAny, Operand::Value(text("hello world"))),
            t(TAGS, Operator::HasAny, Operand::Value(text("x"))),
            t(
                TAGS,
                Operator::HasAny,
                Operand::Value(Value::Link(vec![RecordId::from_u128(1)])),
            ),
            t(TAGS, Operator::HasAny, Operand::List(vec![])),
            t(TAGS, Operator::HasAny, Operand::Range(n.clone(), n.clone())),
            t(
                TAGS,
                Operator::HasAny,
                Operand::Period(Period::this(PeriodUnit::Day)),
            ),
            t(
                TAGS,
                Operator::HasAll,
                Operand::Value(Value::MultiReference(vec![])),
            ),
            t(
                TAGS,
                Operator::Eq,
                Operand::Value(Value::Link(vec![RecordId::from_u128(1)])),
            ),
            t(
                TAGS,
                Operator::Lt,
                Operand::Value(Value::MultiReference(vec![item(9)])),
            ),
            t(
                AMOUNT,
                Operator::Eq,
                Operand::Period(Period::this(PeriodUnit::Day)),
            ),
            t(
                AMOUNT,
                Operator::Lt,
                Operand::Period(Period::this(PeriodUnit::Day)),
            ),
            t(AMOUNT, Operator::Eq, Operand::Me),
            t(DONE, Operator::Lt, Operand::Value(Value::Bool(true))),
            t(
                DUE,
                Operator::Eq,
                Operand::Value(Value::DateTime(Timestamp::EPOCH)),
            ),
            t(
                STATUS,
                Operator::Lt,
                Operand::Value(Value::Reference(item(9))),
            ),
            t(
                EXTRA,
                Operator::Eq,
                Operand::Value(Value::Json(serde_json::json!(1))),
            ),
            t(TITLE, Operator::Eq, Operand::Field(fid(999))),
        ];
        for condition in &positive {
            assert!(!run(condition), "{condition:?}");
            assert!(run(&condition.clone().negated()));
        }
        // The negative twins of ill-typed tests are true.
        assert!(run(&t(TITLE, Operator::Ne, Operand::Value(n.clone()))));
        assert!(run(&t(
            TITLE,
            Operator::NotContains,
            Operand::Value(n.clone())
        )));
        assert!(run(&t(TITLE, Operator::NotIn, Operand::Value(n.clone()))));
        assert!(run(&t(TITLE, Operator::HasNone, Operand::Value(n))));
        // `holds` is only ever asked about positive operators; asked about the others it says no.
        let probe = Test {
            field: fid(TITLE),
            op: Operator::Eq,
            operand: Operand::Value(text("hello world")),
        };
        let value = r.get(&fid(TITLE));
        for op in [
            Operator::IsEmpty,
            Operator::IsNotEmpty,
            Operator::Ne,
            Operator::NotContains,
            Operator::NotIn,
            Operator::HasNone,
        ] {
            assert!(!probe.holds(op, value, &|f| r.get(&f), &ctx()));
        }
        // Operands that are not dates have no position relative to a period.
        for value in crate::value::testkit::samples() {
            let is_date = matches!(value, Value::Date(_) | Value::DateTime(_));
            assert_eq!(
                position(&value, Date::EPOCH, Date::EPOCH, &ctx()).is_some(),
                is_date
            );
            assert_eq!(
                items_of(&value).is_some(),
                value.data_type().is_some_and(DataType::is_list)
            );
        }
    }
}

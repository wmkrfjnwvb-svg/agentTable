//! Aggregates: the totals in a view's footer, the subtotals of its groups and the summary of
//! a selection.
//!
//! All arithmetic is exact decimal arithmetic. Rules that matter for trust in the numbers:
//!
//! * **Empty is not zero.** Empty cells are counted by the counting functions and ignored by
//!   the numeric ones; the average of `10` and an empty cell is `10`.
//! * **Money never mixes currencies.** A sum, average, minimum or range over amounts in more
//!   than one currency has no value and reports [`AggregateProblem::MixedCurrencies`]; the
//!   view shows that instead of a wrong number.
//! * **Overflow is reported, not wrapped** ([`AggregateProblem::Overflow`]).
//! * **Rounding is explicit**: averages of money are rounded half-even to the currency's
//!   minor units, averages of durations to whole milliseconds, percentages of counts to six
//!   decimal places; averages of numbers keep the precision of exact division.
//! * The sum of no numbers is `0` (as in every spreadsheet); the average, minimum, maximum
//!   and median of no values is nothing.

use mt_core::decimal::{Decimal, Rounding};
use mt_core::field::DataType;
use mt_core::id::FieldId;
use mt_core::money::{Currency, Money};
use mt_core::sortkey::{compare_values, sort_key};
use mt_core::time::DurationMs;
use mt_core::value::Value;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// A function that turns the values of one field into one value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateFn {
    /// Number of records without a value.
    CountEmpty,
    /// Number of records with a value.
    CountFilled,
    /// Number of distinct values.
    CountUnique,
    /// Share of records without a value.
    PercentEmpty,
    /// Share of records with a value.
    PercentFilled,
    /// Number of ticked checkboxes.
    CountChecked,
    /// Share of ticked checkboxes (an empty checkbox is not ticked).
    PercentChecked,
    /// Total.
    Sum,
    /// Arithmetic mean of the values present.
    Average,
    /// Middle value (the mean of the two middle values for an even count).
    Median,
    /// Smallest value, or earliest date.
    Min,
    /// Largest value, or latest date.
    Max,
    /// Largest minus smallest; for dates, the time between the earliest and the latest.
    Range,
}

impl AggregateFn {
    /// Every function, in declaration order.
    pub const ALL: &'static [AggregateFn] = &[
        AggregateFn::CountEmpty,
        AggregateFn::CountFilled,
        AggregateFn::CountUnique,
        AggregateFn::PercentEmpty,
        AggregateFn::PercentFilled,
        AggregateFn::CountChecked,
        AggregateFn::PercentChecked,
        AggregateFn::Sum,
        AggregateFn::Average,
        AggregateFn::Median,
        AggregateFn::Min,
        AggregateFn::Max,
        AggregateFn::Range,
    ];

    /// The stable name (also the serde form).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            AggregateFn::CountEmpty => "count_empty",
            AggregateFn::CountFilled => "count_filled",
            AggregateFn::CountUnique => "count_unique",
            AggregateFn::PercentEmpty => "percent_empty",
            AggregateFn::PercentFilled => "percent_filled",
            AggregateFn::CountChecked => "count_checked",
            AggregateFn::PercentChecked => "percent_checked",
            AggregateFn::Sum => "sum",
            AggregateFn::Average => "average",
            AggregateFn::Median => "median",
            AggregateFn::Min => "min",
            AggregateFn::Max => "max",
            AggregateFn::Range => "range",
        }
    }

    /// Whether the function can be computed for a field of the given data type.
    #[must_use]
    pub const fn applies_to(self, data_type: DataType) -> bool {
        let numeric = matches!(
            data_type,
            DataType::Number | DataType::Percent | DataType::Money | DataType::Duration
        );
        match self {
            AggregateFn::CountEmpty
            | AggregateFn::CountFilled
            | AggregateFn::PercentEmpty
            | AggregateFn::PercentFilled => true,
            AggregateFn::CountUnique => !matches!(data_type, DataType::Json | DataType::Attachment),
            AggregateFn::CountChecked | AggregateFn::PercentChecked => {
                matches!(data_type, DataType::Bool)
            }
            AggregateFn::Sum | AggregateFn::Average | AggregateFn::Median => numeric,
            AggregateFn::Min | AggregateFn::Max | AggregateFn::Range => {
                numeric || matches!(data_type, DataType::Date | DataType::DateTime)
            }
        }
    }
}

impl core::fmt::Display for AggregateFn {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A total to compute: a function over a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Aggregate {
    /// The field.
    pub field: FieldId,
    /// The function.
    pub function: AggregateFn,
}

/// Why an aggregate has no value although there were values to aggregate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateProblem {
    /// The amounts are in more than one currency.
    MixedCurrencies,
    /// The result does not fit the number range.
    Overflow,
}

/// The result of an aggregate.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AggregateValue {
    /// The value, if there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// Why there is none, if something went wrong.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<AggregateProblem>,
}

impl AggregateValue {
    const fn of(value: Value) -> Self {
        AggregateValue {
            value: Some(value),
            problem: None,
        }
    }

    const fn nothing() -> Self {
        AggregateValue {
            value: None,
            problem: None,
        }
    }

    const fn failed(problem: AggregateProblem) -> Self {
        AggregateValue {
            value: None,
            problem: Some(problem),
        }
    }
}

fn count(n: usize) -> Value {
    Value::Number(Decimal::from(u64::try_from(n).unwrap_or(u64::MAX)))
}

/// `part / whole` as a percent value rounded to six decimal places; nothing for an empty whole.
fn share(part: usize, whole: usize) -> AggregateValue {
    let part = Decimal::from(u64::try_from(part).unwrap_or(u64::MAX));
    let whole = Decimal::from(u64::try_from(whole).unwrap_or(u64::MAX));
    match part.checked_div(whole) {
        Some(fraction) => AggregateValue::of(Value::Percent(
            Rounding::HalfEven.round(fraction, 6).normalize(),
        )),
        None => AggregateValue::nothing(),
    }
}

/// The numbers behind a list of values of one numeric data type.
enum Numbers {
    /// Plain numbers or percent fractions.
    Plain(Vec<Decimal>),
    /// Amounts in one currency.
    Money(Currency, Vec<Decimal>),
    /// Durations in milliseconds.
    Duration(Vec<Decimal>),
    /// Dates as days since the epoch, or instants as milliseconds since the epoch.
    Instants(Vec<Decimal>),
}

/// Extracts the numbers of the filled values. `Err` for mixed currencies; `Ok(None)` when
/// the values are not of the numeric data type asked for.
fn numbers(data_type: DataType, filled: &[&Value]) -> Result<Option<Numbers>, AggregateProblem> {
    let mut plain = Vec::with_capacity(filled.len());
    let mut currency: Option<Currency> = None;
    for value in filled {
        let number = match (data_type, value) {
            (DataType::Number, Value::Number(n)) | (DataType::Percent, Value::Percent(n)) => *n,
            (DataType::Money, Value::Money(m)) => {
                if currency.is_some_and(|c| c != m.currency()) {
                    return Err(AggregateProblem::MixedCurrencies);
                }
                currency = Some(m.currency());
                m.amount()
            }
            (DataType::Duration, Value::Duration(d)) => Decimal::from(d.millis()),
            (DataType::Date, Value::Date(d)) => Decimal::from(d.days()),
            (DataType::DateTime, Value::DateTime(t)) => Decimal::from(t.millis()),
            _ => return Ok(None),
        };
        plain.push(number);
    }
    Ok(Some(match data_type {
        DataType::Money => match currency {
            Some(currency) => Numbers::Money(currency, plain),
            None => return Ok(None),
        },
        DataType::Duration => Numbers::Duration(plain),
        DataType::Date | DataType::DateTime => Numbers::Instants(plain),
        DataType::Text
        | DataType::LongText
        | DataType::Number
        | DataType::Percent
        | DataType::Bool
        | DataType::Reference
        | DataType::MultiReference
        | DataType::Link
        | DataType::Principal
        | DataType::Attachment
        | DataType::Json => Numbers::Plain(plain),
    }))
}

fn total(numbers: &[Decimal]) -> Option<Decimal> {
    numbers
        .iter()
        .try_fold(Decimal::ZERO, |sum, n| sum.checked_add(*n))
}

/// Whole milliseconds, rounded half-even.
fn millis(value: Decimal) -> Option<i64> {
    i64::try_from(Rounding::HalfEven.round(value, 0).mantissa()).ok()
}

/// Wraps a number as a value of the numeric family it came from.
fn wrap(data_type: DataType, family: &Numbers, number: Decimal) -> AggregateValue {
    let value = match family {
        Numbers::Plain(_) => {
            if data_type == DataType::Percent {
                Some(Value::Percent(number.normalize()))
            } else {
                Some(Value::Number(number.normalize()))
            }
        }
        Numbers::Money(currency, _) => Some(Value::Money(Money::rounded(
            number,
            *currency,
            Rounding::HalfEven,
        ))),
        Numbers::Duration(_) | Numbers::Instants(_) => {
            millis(number).map(|ms| Value::Duration(DurationMs::from_millis(ms)))
        }
    };
    value.map_or(
        AggregateValue::failed(AggregateProblem::Overflow),
        AggregateValue::of,
    )
}

fn list(family: &Numbers) -> &[Decimal] {
    match family {
        Numbers::Plain(n) | Numbers::Money(_, n) | Numbers::Duration(n) | Numbers::Instants(n) => n,
    }
}

/// Computes one aggregate over the values of a field. `values` yields the value of each
/// record (`None` or an empty value for a record without one). An aggregate that does not
/// apply to the data type has no value.
#[must_use]
pub fn aggregate<'a>(
    function: AggregateFn,
    data_type: DataType,
    values: impl Iterator<Item = Option<&'a Value>>,
) -> AggregateValue {
    let mut records = 0_usize;
    let mut filled: Vec<&Value> = Vec::new();
    for value in values {
        records = records.saturating_add(1);
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            filled.push(value);
        }
    }
    if !function.applies_to(data_type) {
        return AggregateValue::nothing();
    }
    match function {
        AggregateFn::CountEmpty => AggregateValue::of(count(records.saturating_sub(filled.len()))),
        AggregateFn::CountFilled => AggregateValue::of(count(filled.len())),
        AggregateFn::CountUnique => {
            let distinct: BTreeSet<Vec<u8>> = filled.iter().map(|v| sort_key(v)).collect();
            AggregateValue::of(count(distinct.len()))
        }
        AggregateFn::PercentEmpty => share(records.saturating_sub(filled.len()), records),
        AggregateFn::PercentFilled => share(filled.len(), records),
        AggregateFn::CountChecked | AggregateFn::PercentChecked => {
            let checked = filled
                .iter()
                .filter(|v| matches!(v, Value::Bool(true)))
                .count();
            if function == AggregateFn::CountChecked {
                AggregateValue::of(count(checked))
            } else {
                share(checked, records)
            }
        }
        AggregateFn::Sum => numeric(Numeric::Sum, data_type, &filled),
        AggregateFn::Average => numeric(Numeric::Average, data_type, &filled),
        AggregateFn::Median => numeric(Numeric::Median, data_type, &filled),
        AggregateFn::Min => numeric(Numeric::Min, data_type, &filled),
        AggregateFn::Max => numeric(Numeric::Max, data_type, &filled),
        AggregateFn::Range => numeric(Numeric::Range, data_type, &filled),
    }
}

/// The functions that calculate with the values (the others only count them).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Numeric {
    Sum,
    Average,
    Median,
    Min,
    Max,
    Range,
}

fn numeric(function: Numeric, data_type: DataType, filled: &[&Value]) -> AggregateValue {
    let family = match numbers(data_type, filled) {
        Err(problem) => return AggregateValue::failed(problem),
        // Values of another data type than the field's, or no amounts at all (the sum of no
        // money has no currency): nothing to show.
        Ok(None) => return AggregateValue::nothing(),
        Ok(Some(family)) => family,
    };
    let list = list(&family);
    let overflow = AggregateValue::failed(AggregateProblem::Overflow);
    let mean = |values: &[Decimal]| {
        let n = Decimal::from(u64::try_from(values.len()).unwrap_or(u64::MAX));
        total(values).and_then(|sum| sum.checked_div(n))
    };
    match function {
        Numeric::Sum => total(list).map_or(overflow, |sum| wrap(data_type, &family, sum)),
        Numeric::Average => {
            if list.is_empty() {
                AggregateValue::nothing()
            } else {
                mean(list).map_or(overflow, |average| wrap(data_type, &family, average))
            }
        }
        Numeric::Median => {
            let mut sorted = list.to_vec();
            sorted.sort_unstable();
            let middle = sorted.len().checked_div(2).unwrap_or(0);
            let upper = sorted.get(middle).copied();
            let lower = middle.checked_sub(1).and_then(|i| sorted.get(i)).copied();
            let even = sorted.len().checked_rem(2) == Some(0);
            match (lower, upper, even) {
                (Some(low), Some(high), true) => {
                    mean(&[low, high]).map_or(overflow, |median| wrap(data_type, &family, median))
                }
                (_, Some(median), _) => wrap(data_type, &family, median),
                (_, None, _) => AggregateValue::nothing(),
            }
        }
        Numeric::Min | Numeric::Max => {
            // The original value is returned, so dates stay dates and money keeps its scale.
            let pick = if function == Numeric::Min {
                filled.iter().copied().min_by(|a, b| compare_values(a, b))
            } else {
                filled.iter().copied().max_by(|a, b| compare_values(a, b))
            };
            pick.map_or(AggregateValue::nothing(), |value| {
                AggregateValue::of(value.clone().normalized())
            })
        }
        Numeric::Range => {
            let (Some(min), Some(max)) = (list.iter().min(), list.iter().max()) else {
                return AggregateValue::nothing();
            };
            let Some(span) = max.checked_sub(*min) else {
                return overflow;
            };
            // Dates are counted in days; the span is reported as a duration.
            let span = if data_type == DataType::Date {
                span.checked_mul(Decimal::from(86_400_000_i64))
            } else {
                Some(span)
            };
            span.map_or(overflow, |span| wrap(data_type, &family, span))
        }
    }
}

/// What a selection of cells adds up to, as shown in the status bar.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SelectionSummary {
    /// Number of selected cells.
    pub cells: usize,
    /// Number of selected cells with a value.
    pub filled: usize,
    /// Sum, average, minimum and maximum, when every filled cell is of the same numeric
    /// data type (and, for money, the same currency).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numbers: Option<NumericSummary>,
}

/// The numeric part of a [`SelectionSummary`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumericSummary {
    /// Total.
    pub sum: Value,
    /// Mean.
    pub average: Value,
    /// Smallest.
    pub min: Value,
    /// Largest.
    pub max: Value,
}

/// Summarises a selection of cells. Cells of different numeric kinds (numbers next to
/// amounts, or amounts in two currencies) are counted but not added up.
#[must_use]
pub fn summarize<'a>(cells: impl Iterator<Item = Option<&'a Value>>) -> SelectionSummary {
    let cells: Vec<Option<&Value>> = cells.collect();
    let filled: Vec<&Value> = cells
        .iter()
        .filter_map(|cell| cell.filter(|v| !v.is_empty()))
        .collect();
    let data_type = filled.first().and_then(|v| v.data_type()).filter(|t| {
        matches!(
            t,
            DataType::Number | DataType::Percent | DataType::Money | DataType::Duration
        ) && filled.iter().all(|v| v.data_type() == Some(*t))
    });
    let numbers = data_type.and_then(|data_type| {
        let run = |function| aggregate(function, data_type, filled.iter().map(|v| Some(*v))).value;
        Some(NumericSummary {
            sum: run(AggregateFn::Sum)?,
            average: run(AggregateFn::Average)?,
            min: run(AggregateFn::Min)?,
            max: run(AggregateFn::Max)?,
        })
    });
    SelectionSummary {
        cells: cells.len(),
        filled: filled.len(),
        numbers,
    }
}

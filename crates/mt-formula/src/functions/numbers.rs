//! Calculations with numbers, amounts and durations.

use crate::ast::NumberFn;
use crate::error::ErrorValue;
use crate::value::{whole, Val};
use mt_core::decimal::{Decimal, Rounding};
use mt_core::money::Currency;
use mt_core::time::{Date, DurationMs, Timestamp};

/// The argument at `index` (an empty value when it was left out).
pub(crate) fn value(values: &[Val], index: usize) -> &Val {
    values.get(index).unwrap_or(&Val::Blank)
}

/// Every value of the arguments, with lists expanded.
pub(crate) fn flatten(values: &[Val]) -> impl Iterator<Item = &Val> {
    values.iter().flat_map(|value| {
        if let Val::List(items) = value {
            items.iter().collect::<Vec<_>>()
        } else {
            vec![value]
        }
    })
}

fn count(n: usize) -> Val {
    Val::Number(Decimal::from(u64::try_from(n).unwrap_or(u64::MAX)))
}

/// The values of one kind found among the arguments of an aggregate function. Empty
/// values, text, true/false and references are skipped, as a spreadsheet skips them in a
/// range; mixing kinds that cannot be added is an error.
pub(crate) enum Series {
    /// Nothing to aggregate.
    Empty,
    /// Plain numbers.
    Numbers(Vec<Decimal>),
    /// Amounts in one currency.
    Money(Currency, Vec<Decimal>),
    /// Durations in milliseconds.
    Durations(Vec<Decimal>),
    /// Days.
    Dates(Vec<Date>),
    /// Instants.
    Instants(Vec<Timestamp>),
}

impl Series {
    /// Collects the values. Dates and instants are accepted only when `with_dates` is set
    /// (minimum and maximum).
    pub(crate) fn of<'v>(
        values: impl Iterator<Item = &'v Val>,
        with_dates: bool,
    ) -> Result<Series, ErrorValue> {
        let mut series = Series::Empty;
        for value in values {
            series = match (series, value) {
                (series, Val::Blank | Val::Text(_) | Val::Bool(_) | Val::Ref(_)) => series,
                (Series::Empty, Val::Number(n)) => Series::Numbers(vec![*n]),
                (Series::Numbers(mut list), Val::Number(n)) => {
                    list.push(*n);
                    Series::Numbers(list)
                }
                (Series::Empty, Val::Money(amount, currency)) => {
                    Series::Money(*currency, vec![*amount])
                }
                (Series::Money(currency, mut list), Val::Money(amount, other)) => {
                    if currency != *other {
                        return Err(ErrorValue::Currency);
                    }
                    list.push(*amount);
                    Series::Money(currency, list)
                }
                (Series::Empty, Val::Duration(d)) => {
                    Series::Durations(vec![Decimal::from(d.millis())])
                }
                (Series::Durations(mut list), Val::Duration(d)) => {
                    list.push(Decimal::from(d.millis()));
                    Series::Durations(list)
                }
                (Series::Empty, Val::Date(d)) if with_dates => Series::Dates(vec![*d]),
                (Series::Dates(mut list), Val::Date(d)) => {
                    list.push(*d);
                    Series::Dates(list)
                }
                (Series::Empty, Val::DateTime(t)) if with_dates => Series::Instants(vec![*t]),
                (Series::Instants(mut list), Val::DateTime(t)) => {
                    list.push(*t);
                    Series::Instants(list)
                }
                // Two kinds that cannot be combined, or dates where they make no sense.
                // (Lists were flattened and errors returned before the values got here.)
                _ => return Err(ErrorValue::Value),
            };
        }
        Ok(series)
    }

    /// The decimals of a numeric series and a way to wrap a result as the same kind.
    fn decimals(&self) -> Option<(&[Decimal], Kind)> {
        match self {
            Series::Numbers(list) => Some((list, Kind::Number)),
            Series::Money(currency, list) => Some((list, Kind::Money(*currency))),
            Series::Durations(list) => Some((list, Kind::Duration)),
            Series::Empty | Series::Dates(_) | Series::Instants(_) => None,
        }
    }

    /// The total. Nothing to add up is zero. (Dates never get here: a series that is to
    /// be added is collected without them.)
    pub(crate) fn sum(&self) -> Result<Val, ErrorValue> {
        match self.decimals() {
            Some((list, kind)) => kind.wrap(total(list)?),
            None => Ok(Val::Number(Decimal::ZERO)),
        }
    }

    /// The mean; nothing for no values.
    pub(crate) fn average(&self) -> Result<Val, ErrorValue> {
        match self.decimals() {
            Some((list, kind)) => kind.wrap(mean(list)?),
            None => Ok(Val::Blank),
        }
    }

    /// The middle value (the mean of the two middle values for an even count); nothing
    /// for no values.
    fn median(&self) -> Result<Val, ErrorValue> {
        let Some((list, kind)) = self.decimals() else {
            return Ok(Val::Blank);
        };
        let mut sorted = list.to_vec();
        sorted.sort_unstable();
        let middle = sorted.len().checked_div(2).unwrap_or(0);
        let upper = sorted.get(middle).copied().ok_or(ErrorValue::Value)?;
        let even = sorted.len().checked_rem(2) == Some(0);
        match middle.checked_sub(1).and_then(|i| sorted.get(i)) {
            Some(lower) if even => kind.wrap(mean(&[*lower, upper])?),
            Some(_) | None => kind.wrap(upper),
        }
    }

    fn extreme(&self, largest: bool) -> Result<Val, ErrorValue> {
        fn pick<T: Ord + Copy>(list: &[T], largest: bool) -> Option<T> {
            if largest {
                list.iter().copied().max()
            } else {
                list.iter().copied().min()
            }
        }
        let found = match self {
            Series::Empty => return Ok(Val::Blank),
            Series::Numbers(list) => pick(list, largest).map(Val::Number),
            Series::Money(currency, list) => {
                pick(list, largest).map(|amount| Val::Money(amount, *currency))
            }
            Series::Durations(list) => pick(list, largest)
                .and_then(whole)
                .map(|ms| Val::Duration(DurationMs::from_millis(ms))),
            Series::Dates(list) => pick(list, largest).map(Val::Date),
            Series::Instants(list) => pick(list, largest).map(Val::DateTime),
        };
        found.ok_or(ErrorValue::Value)
    }
}

/// What kind of value a numeric result is.
#[derive(Clone, Copy)]
enum Kind {
    Number,
    Money(Currency),
    Duration,
}

impl Kind {
    fn wrap(self, number: Decimal) -> Result<Val, ErrorValue> {
        match self {
            Kind::Number => Ok(Val::Number(number)),
            Kind::Money(currency) => Ok(Val::Money(number, currency)),
            Kind::Duration => whole(Rounding::HalfEven.round(number, 0))
                .map(|ms| Val::Duration(DurationMs::from_millis(ms)))
                .ok_or(ErrorValue::Num),
        }
    }
}

fn total(list: &[Decimal]) -> Result<Decimal, ErrorValue> {
    list.iter()
        .try_fold(Decimal::ZERO, |sum, n| sum.checked_add(*n))
        .ok_or(ErrorValue::Num)
}

fn mean(list: &[Decimal]) -> Result<Decimal, ErrorValue> {
    let n = Decimal::from(u64::try_from(list.len()).unwrap_or(u64::MAX));
    total(list)?.checked_div(n).ok_or(ErrorValue::Num)
}

/// Applies a calculation to a number, or to the amount of a money value keeping its
/// currency.
fn on_number(
    value: &Val,
    calculate: impl Fn(Decimal) -> Result<Decimal, ErrorValue>,
) -> Result<Val, ErrorValue> {
    if let Val::Money(amount, currency) = value {
        Ok(Val::Money(calculate(*amount)?, *currency))
    } else {
        Ok(Val::Number(calculate(value.number()?)?))
    }
}

/// Ten to the power of `digits` (0 ≤ digits ≤ 28).
fn power_of_ten(digits: u32) -> Option<Decimal> {
    (0..digits).try_fold(Decimal::ONE, |power, _| power.checked_mul(Decimal::TEN))
}

/// Rounds to `digits` decimal places; negative digits round to tens, hundreds, ….
fn round_to(x: Decimal, digits: i64, mode: Rounding) -> Result<Decimal, ErrorValue> {
    if digits >= 0 {
        let places = u32::try_from(digits.min(28)).unwrap_or(28);
        return Ok(mode.round(x, places));
    }
    let scale = u32::try_from(digits.unsigned_abs())
        .ok()
        .filter(|d| *d <= 28)
        .and_then(power_of_ten)
        .ok_or(ErrorValue::Num)?;
    let scaled = x.checked_div(scale).ok_or(ErrorValue::Num)?;
    mode.round(scaled, 0)
        .checked_mul(scale)
        .ok_or(ErrorValue::Num)
}

/// The nearest multiple of `step` not below (`up`) or not above `x`.
fn to_multiple(x: Decimal, step: Decimal, up: bool) -> Result<Decimal, ErrorValue> {
    if step.is_zero() {
        return Ok(Decimal::ZERO);
    }
    let quotient = x.checked_div(step).ok_or(ErrorValue::Num)?;
    let whole_steps = if up {
        quotient.ceil()
    } else {
        quotient.floor()
    };
    whole_steps.checked_mul(step).ok_or(ErrorValue::Num)
}

fn power(base: Decimal, exponent: i64) -> Result<Decimal, ErrorValue> {
    if exponent.unsigned_abs() > 10_000 {
        return Err(ErrorValue::Num);
    }
    if exponent < 0 && base.is_zero() {
        return Err(ErrorValue::DivZero);
    }
    // Square and multiply.
    let mut result = Decimal::ONE;
    let mut factor = base;
    let mut remaining = exponent.unsigned_abs();
    while remaining > 0 {
        if remaining.checked_rem(2) == Some(1) {
            result = result.checked_mul(factor).ok_or(ErrorValue::Num)?;
        }
        remaining = remaining.checked_div(2).unwrap_or(0);
        if remaining > 0 {
            factor = factor.checked_mul(factor).ok_or(ErrorValue::Num)?;
        }
    }
    if exponent < 0 {
        Decimal::ONE.checked_div(result).ok_or(ErrorValue::Num)
    } else {
        Ok(result)
    }
}

/// Evaluates a number function on evaluated, error-free arguments.
pub(crate) fn call(function: NumberFn, values: &[Val]) -> Result<Val, ErrorValue> {
    let digits = || match values.get(1) {
        Some(digits) => digits.integer(),
        None => Ok(0),
    };
    match function {
        NumberFn::Sum => Series::of(flatten(values), false)?.sum(),
        NumberFn::Average => Series::of(flatten(values), false)?.average(),
        NumberFn::Median => Series::of(flatten(values), false)?.median(),
        NumberFn::Min => Series::of(flatten(values), true)?.extreme(false),
        NumberFn::Max => Series::of(flatten(values), true)?.extreme(true),
        NumberFn::Count => {
            // Counts what can be calculated with; mixed kinds are fine for counting.
            let countable = flatten(values)
                .filter(|v| {
                    matches!(
                        v,
                        Val::Number(_)
                            | Val::Money(..)
                            | Val::Duration(_)
                            | Val::Date(_)
                            | Val::DateTime(_)
                    )
                })
                .count();
            Ok(count(countable))
        }
        NumberFn::CountA => Ok(count(flatten(values).filter(|v| !v.is_blank()).count())),
        NumberFn::Abs => on_number(value(values, 0), |x| Ok(x.abs())),
        NumberFn::Sign => {
            let x = if let Val::Money(amount, _) = value(values, 0) {
                *amount
            } else {
                value(values, 0).number()?
            };
            Ok(Val::Number(if x.is_zero() {
                Decimal::ZERO
            } else if x.is_sign_negative() {
                Decimal::NEGATIVE_ONE
            } else {
                Decimal::ONE
            }))
        }
        NumberFn::Round => {
            let digits = digits()?;
            on_number(value(values, 0), |x| round_to(x, digits, Rounding::HalfUp))
        }
        NumberFn::RoundUp => {
            let digits = digits()?;
            on_number(value(values, 0), |x| round_to(x, digits, Rounding::Up))
        }
        NumberFn::RoundDown => {
            let digits = digits()?;
            on_number(value(values, 0), |x| round_to(x, digits, Rounding::Down))
        }
        NumberFn::Int => Ok(Val::Number(value(values, 0).number()?.floor())),
        NumberFn::Mod => {
            let (x, divisor) = (value(values, 0).number()?, value(values, 1).number()?);
            if divisor.is_zero() {
                return Err(ErrorValue::DivZero);
            }
            let whole_steps = x.checked_div(divisor).ok_or(ErrorValue::Num)?.floor();
            whole_steps
                .checked_mul(divisor)
                .and_then(|product| x.checked_sub(product))
                .map(Val::Number)
                .ok_or(ErrorValue::Num)
        }
        NumberFn::Power => {
            let exponent = value(values, 1).integer()?;
            power(value(values, 0).number()?, exponent).map(Val::Number)
        }
        NumberFn::Ceiling | NumberFn::Floor => {
            let step = match values.get(1) {
                Some(step) => step.number()?,
                None => Decimal::ONE,
            };
            let up = function == NumberFn::Ceiling;
            on_number(value(values, 0), |x| to_multiple(x, step, up))
        }
    }
}

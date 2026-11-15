//! Dates, times, working days and durations.
//!
//! "Today", "now", the start of a day (the time zone) and which days are working days are
//! all questions to the [`Env`]; this module only does calendar arithmetic on the answers.

use crate::ast::DateFn;
use crate::error::ErrorValue;
use crate::eval::Env;
use crate::functions::numbers::value;
use crate::value::{whole, Val};
use mt_core::decimal::{Decimal, Rounding};
use mt_core::time::{days_in_month, Date, DurationMs, Timestamp};

/// Longest span, in days, that `NETWORKDAYS` walks through (about a hundred years).
const MAX_SPAN_DAYS: i64 = 36_600;
/// Largest number of working days `WORKDAY` moves by.
const MAX_WORKDAYS: i64 = 10_000;

/// The civil day an instant falls on in the environment's time zone.
pub(crate) fn day_of(at: Timestamp, env: &dyn Env) -> Date {
    let utc = at.date();
    // The local day is the UTC day or one of its neighbours.
    let starts = |day: Date| env.start_of_day(day) <= at;
    let next = utc.checked_add_days(1);
    if next.is_some_and(starts) {
        next.unwrap_or(utc)
    } else if starts(utc) {
        utc
    } else {
        utc.checked_add_days(-1).unwrap_or(utc)
    }
}

/// A day from a date, a date-time (its local day) or a text written as `YYYY-MM-DD`.
fn day(value: &Val, env: &dyn Env) -> Result<Date, ErrorValue> {
    match value {
        Val::Date(day) => Ok(*day),
        Val::DateTime(at) => Ok(day_of(*at, env)),
        Val::Text(text) => text.trim().parse::<Date>().map_err(|_| ErrorValue::Value),
        Val::Error(error) => Err(*error),
        Val::Blank
        | Val::Number(_)
        | Val::Bool(_)
        | Val::Duration(_)
        | Val::Money(..)
        | Val::Ref(_)
        | Val::List(_) => Err(ErrorValue::Value),
    }
}

fn number(n: i64) -> Val {
    Val::Number(Decimal::from(n))
}

fn days_between(later: Date, earlier: Date) -> i64 {
    i64::from(later.days()).saturating_sub(i64::from(earlier.days()))
}

/// The day `months` months later, keeping the day of the month where the target month has
/// it and using the month's last day otherwise (31 January + 1 month = 28 or 29 February).
fn add_months(day: Date, months: i64) -> Result<Date, ErrorValue> {
    let (year, month, day_of_month) = day.ymd();
    let index = i64::from(year)
        .checked_mul(12)
        .and_then(|i| i.checked_add(i64::from(month)))
        .and_then(|i| i.checked_sub(1))
        .and_then(|i| i.checked_add(months))
        .ok_or(ErrorValue::Num)?;
    let target_year = i32::try_from(index.div_euclid(12)).map_err(|_| ErrorValue::Num)?;
    let target_month = u32::try_from(index.rem_euclid(12))
        .map_err(|_| ErrorValue::Num)?
        .saturating_add(1);
    let last = days_in_month(target_year, target_month);
    Date::from_ymd(target_year, target_month, day_of_month.min(last)).map_err(|_| ErrorValue::Num)
}

fn add_days(day: Date, days: i64) -> Result<Date, ErrorValue> {
    i32::try_from(days)
        .ok()
        .and_then(|count| day.checked_add_days(count))
        .ok_or(ErrorValue::Num)
}

/// Moves a day by a count of units named in words.
fn shift(day: Date, count: i64, unit: &str) -> Result<Date, ErrorValue> {
    match unit.trim().to_ascii_lowercase().as_str() {
        "d" | "day" | "days" => add_days(day, count),
        "w" | "week" | "weeks" => add_days(day, count.checked_mul(7).ok_or(ErrorValue::Num)?),
        "m" | "month" | "months" => add_months(day, count),
        "y" | "year" | "years" => add_months(day, count.checked_mul(12).ok_or(ErrorValue::Num)?),
        _ => Err(ErrorValue::Value),
    }
}

/// Milliseconds from the start of the local day to the instant.
fn since_midnight(at: Timestamp, env: &dyn Env) -> i64 {
    at.millis()
        .saturating_sub(env.start_of_day(day_of(at, env)).millis())
}

/// A duration as a number of units of `unit_millis` milliseconds.
fn in_units(value: &Val, unit_millis: i64) -> Result<Val, ErrorValue> {
    match value {
        Val::Blank => Ok(Val::Blank),
        Val::Duration(duration) => Decimal::from(duration.millis())
            .checked_div(Decimal::from(unit_millis))
            .map(Val::Number)
            .ok_or(ErrorValue::Num),
        Val::Error(error) => Err(*error),
        Val::Number(_)
        | Val::Text(_)
        | Val::Bool(_)
        | Val::Date(_)
        | Val::DateTime(_)
        | Val::Money(..)
        | Val::Ref(_)
        | Val::List(_) => Err(ErrorValue::Value),
    }
}

/// Whether a date argument is missing: no value, or blank text.
fn missing(value: &Val) -> bool {
    match value {
        Val::Blank => true,
        Val::Text(text) => text.trim().is_empty(),
        Val::Number(_)
        | Val::Bool(_)
        | Val::Date(_)
        | Val::DateTime(_)
        | Val::Duration(_)
        | Val::Money(..)
        | Val::Ref(_)
        | Val::List(_)
        | Val::Error(_) => false,
    }
}

/// How many leading arguments of a function are dates. Without one of them there is
/// nothing to calculate, and the result is empty rather than an error.
const fn date_arguments(function: DateFn) -> usize {
    match function {
        DateFn::Year
        | DateFn::Month
        | DateFn::Day
        | DateFn::Hour
        | DateFn::Minute
        | DateFn::Weekday
        | DateFn::IsoWeekNum
        | DateFn::DateAdd
        | DateFn::EDate
        | DateFn::EoMonth
        | DateFn::Workday => 1,
        DateFn::DateDif | DateFn::Days | DateFn::NetworkDays => 2,
        DateFn::Today
        | DateFn::Now
        | DateFn::Date
        | DateFn::Duration
        | DateFn::ToHours
        | DateFn::ToDays => 0,
    }
}

/// Evaluates a date function on evaluated, error-free arguments.
#[allow(clippy::too_many_lines)]
pub(crate) fn call(function: DateFn, values: &[Val], env: &dyn Env) -> Result<Val, ErrorValue> {
    let arg = |index: usize| value(values, index);
    if (0..date_arguments(function)).any(|index| missing(arg(index))) {
        return Ok(Val::Blank);
    }
    match function {
        DateFn::Today => Ok(Val::Date(env.today())),
        DateFn::Now => Ok(Val::DateTime(env.now())),
        DateFn::Date => {
            let year = i32::try_from(arg(0).integer()?).map_err(|_| ErrorValue::Num)?;
            let month = u32::try_from(arg(1).integer()?).map_err(|_| ErrorValue::Num)?;
            let day_of_month = u32::try_from(arg(2).integer()?).map_err(|_| ErrorValue::Num)?;
            Date::from_ymd(year, month, day_of_month)
                .map(Val::Date)
                .map_err(|_| ErrorValue::Num)
        }
        DateFn::Year => Ok(number(i64::from(day(arg(0), env)?.year()))),
        DateFn::Month => Ok(number(i64::from(day(arg(0), env)?.month()))),
        DateFn::Day => Ok(number(i64::from(day(arg(0), env)?.day()))),
        DateFn::Hour | DateFn::Minute => {
            let millis = if let Val::DateTime(at) = arg(0) {
                since_midnight(*at, env)
            } else {
                // A plain date has no time of day.
                day(arg(0), env).map(|_| 0)?
            };
            let hours = millis.checked_div(3_600_000).unwrap_or(0);
            let minutes = millis
                .checked_rem(3_600_000)
                .and_then(|rest| rest.checked_div(60_000))
                .unwrap_or(0);
            Ok(number(if function == DateFn::Hour {
                hours
            } else {
                minutes
            }))
        }
        DateFn::Weekday => {
            let from_monday = i64::from(day(arg(0), env)?.weekday().days_from_monday());
            let numbering = match values.get(1) {
                Some(kind) => kind.integer()?,
                None => 1,
            };
            match numbering {
                // Sunday = 1 … Saturday = 7.
                1 => Ok(number(
                    from_monday
                        .saturating_add(1)
                        .rem_euclid(7)
                        .saturating_add(1),
                )),
                // Monday = 1 … Sunday = 7.
                2 => Ok(number(from_monday.saturating_add(1))),
                _ => Err(ErrorValue::Num),
            }
        }
        DateFn::IsoWeekNum => {
            // The ISO week of a day is the week of the Thursday of its week.
            let date = day(arg(0), env)?;
            let from_monday = i64::from(date.weekday().days_from_monday());
            let thursday = add_days(date, 3_i64.saturating_sub(from_monday))?;
            let january_first =
                Date::from_ymd(thursday.year(), 1, 1).map_err(|_| ErrorValue::Num)?;
            let week = days_between(thursday, january_first)
                .checked_div(7)
                .unwrap_or(0)
                .saturating_add(1);
            Ok(number(week))
        }
        DateFn::DateDif => {
            let (start, end) = (day(arg(0), env)?, day(arg(1), env)?);
            if start > end {
                return Err(ErrorValue::Num);
            }
            let unit = arg(2).text(&|item| env.label(item))?;
            let (y1, m1, d1) = start.ymd();
            let (y2, m2, d2) = end.ymd();
            // Complete months: the day of the month must have been reached.
            let months = i64::from(y2)
                .saturating_sub(i64::from(y1))
                .saturating_mul(12)
                .saturating_add(i64::from(m2))
                .saturating_sub(i64::from(m1))
                .saturating_sub(i64::from(d2 < d1));
            match unit.trim().to_ascii_uppercase().as_str() {
                "D" => Ok(number(days_between(end, start))),
                "M" => Ok(number(months)),
                "Y" => Ok(number(months.checked_div(12).unwrap_or(0))),
                _ => Err(ErrorValue::Value),
            }
        }
        DateFn::DateAdd => {
            let count = arg(1).integer()?;
            let unit = arg(2).text(&|item| env.label(item))?;
            if let Val::DateTime(at) = arg(0) {
                // A date-time keeps its time of day in the local time zone.
                let moved = shift(day_of(*at, env), count, &unit)?;
                env.start_of_day(moved)
                    .millis()
                    .checked_add(since_midnight(*at, env))
                    .and_then(|millis| Timestamp::from_millis(millis).ok())
                    .map(Val::DateTime)
                    .ok_or(ErrorValue::Num)
            } else {
                shift(day(arg(0), env)?, count, &unit).map(Val::Date)
            }
        }
        DateFn::EDate => add_months(day(arg(0), env)?, arg(1).integer()?).map(Val::Date),
        DateFn::EoMonth => {
            let moved = add_months(day(arg(0), env)?, arg(1).integer()?)?;
            let (year, month, _) = moved.ymd();
            Date::from_ymd(year, month, days_in_month(year, month))
                .map(Val::Date)
                .map_err(|_| ErrorValue::Num)
        }
        DateFn::Days => Ok(number(days_between(day(arg(0), env)?, day(arg(1), env)?))),
        DateFn::NetworkDays => {
            let (start, end) = (day(arg(0), env)?, day(arg(1), env)?);
            let (first, last, sign) = if start <= end {
                (start, end, 1_i64)
            } else {
                (end, start, -1_i64)
            };
            if days_between(last, first) > MAX_SPAN_DAYS {
                return Err(ErrorValue::Limit);
            }
            let mut working = 0_i64;
            let mut current = Some(first);
            while let Some(date) = current.filter(|date| *date <= last) {
                if env.is_workday(date) {
                    working = working.saturating_add(1);
                }
                current = date.checked_add_days(1);
            }
            Ok(number(working.saturating_mul(sign)))
        }
        DateFn::Workday => {
            let mut date = day(arg(0), env)?;
            let wanted = arg(1).integer()?;
            if wanted.unsigned_abs() > MAX_WORKDAYS.unsigned_abs() {
                return Err(ErrorValue::Limit);
            }
            let step = if wanted < 0 { -1 } else { 1 };
            let mut remaining = wanted.unsigned_abs();
            // A calendar without working days must not loop forever.
            let mut budget = remaining.saturating_mul(7).saturating_add(3_660);
            while remaining > 0 {
                budget = budget.checked_sub(1).ok_or(ErrorValue::Limit)?;
                date = date.checked_add_days(step).ok_or(ErrorValue::Num)?;
                if env.is_workday(date) {
                    remaining = remaining.saturating_sub(1);
                }
            }
            Ok(Val::Date(date))
        }
        DateFn::Duration => {
            let part = |index: usize, unit_millis: i64| match values.get(index) {
                Some(amount) => amount
                    .number()?
                    .checked_mul(Decimal::from(unit_millis))
                    .ok_or(ErrorValue::Num),
                None => Ok(Decimal::ZERO),
            };
            // An argument's own error comes before an overflow of the total.
            let (hours, minutes, seconds) =
                (part(0, 3_600_000)?, part(1, 60_000)?, part(2, 1_000)?);
            let total = hours
                .checked_add(minutes)
                .and_then(|sum| sum.checked_add(seconds))
                .ok_or(ErrorValue::Num)?;
            whole(Rounding::HalfEven.round(total, 0))
                .map(|millis| Val::Duration(DurationMs::from_millis(millis)))
                .ok_or(ErrorValue::Num)
        }
        DateFn::ToHours => in_units(arg(0), 3_600_000),
        DateFn::ToDays => in_units(arg(0), 86_400_000),
    }
}

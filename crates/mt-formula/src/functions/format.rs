//! `TEXT(value, format)`: numbers and dates as text.
//!
//! **Numbers and amounts** — the format is made of `0` (a digit that is always shown), `#`
//! (ignored, for familiar patterns such as `#,##0`), `,` (group thousands), one optional `.`
//! followed by `0`s (decimal places) and an optional final `%` (show as percent):
//! `0`, `0.00`, `#,##0`, `#,##0.00`, `0%`, `0.0%`, `000`. Rounding is half away from zero.
//!
//! **Dates and date-times** — letters are placeholders, ignoring case, as in spreadsheets:
//! `yyyy` `yy` year · `mmmm` `mmm` month name · `mm` `m` month, or minutes when next to
//! hours or seconds · `dddd` `ddd` weekday name · `dd` `d` day · `hh` `h` hours (24) ·
//! `ss` `s` seconds. Everything else is copied: `yyyy-mm-dd`, `d mmm yyyy`, `hh:mm`.
//! Names are in English; localised display is the user interface's job.

use crate::error::ErrorValue;
use crate::eval::Env;
use crate::functions::dates::day_of;
use crate::value::{whole, Val};
use mt_core::decimal::{canonical_string, Decimal, Rounding};
use mt_core::time::{Date, Weekday};

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn weekday_name(day: Weekday) -> &'static str {
    match day {
        Weekday::Monday => "Monday",
        Weekday::Tuesday => "Tuesday",
        Weekday::Wednesday => "Wednesday",
        Weekday::Thursday => "Thursday",
        Weekday::Friday => "Friday",
        Weekday::Saturday => "Saturday",
        Weekday::Sunday => "Sunday",
    }
}

/// Formats a number with a pattern such as `#,##0.00` or `0.0%`.
fn number(value: Decimal, pattern: &str) -> Result<String, ErrorValue> {
    let (body, percent) = match pattern.strip_suffix('%') {
        Some(body) => (body, true),
        None => (pattern, false),
    };
    let (whole_part, fraction_part) = body.split_once('.').unwrap_or((body, ""));
    let well_formed = !body.is_empty()
        && whole_part.chars().all(|c| matches!(c, '0' | '#' | ','))
        && fraction_part.chars().all(|c| c == '0')
        && whole_part.contains(['0', '#']);
    if !well_formed {
        return Err(ErrorValue::Value);
    }
    let decimals = u32::try_from(fraction_part.len())
        .ok()
        .filter(|d| *d <= 28)
        .ok_or(ErrorValue::Value)?;
    let min_digits = whole_part.chars().filter(|c| *c == '0').count().max(1);
    let grouped = whole_part.contains(',');

    let shown = if percent {
        value
            .checked_mul(Decimal::ONE_HUNDRED)
            .ok_or(ErrorValue::Num)?
    } else {
        value
    };
    let rounded = Rounding::HalfUp.round(shown, decimals);
    let magnitude = rounded.abs();
    let mut digits = canonical_string(&magnitude.trunc());
    while digits.len() < min_digits {
        digits.insert(0, '0');
    }
    if grouped {
        let mut with_separators = String::new();
        for (index, digit) in digits.chars().enumerate() {
            let remaining = digits.len().saturating_sub(index);
            if index > 0 && remaining.checked_rem(3) == Some(0) {
                with_separators.push(',');
            }
            with_separators.push(digit);
        }
        digits = with_separators;
    }
    let mut out = String::new();
    if rounded.is_sign_negative() && !rounded.is_zero() {
        out.push('-');
    }
    out.push_str(&digits);
    if decimals > 0 {
        let scale = (0..decimals)
            .try_fold(Decimal::ONE, |power, _| power.checked_mul(Decimal::TEN))
            .ok_or(ErrorValue::Num)?;
        let fraction = magnitude
            .fract()
            .checked_mul(scale)
            .and_then(whole)
            .ok_or(ErrorValue::Num)?;
        let width = usize::try_from(decimals).unwrap_or(0);
        out.push('.');
        out.push_str(&format!("{fraction:0width$}"));
    }
    if percent {
        out.push('%');
    }
    Ok(out)
}

/// One piece of a date pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Piece {
    /// A run of one placeholder letter (lower case) and its length.
    Letters(char, usize),
    /// A character copied as it is.
    Literal(char),
}

/// Formats a day and the milliseconds since its start.
fn date(day: Date, millis_of_day: i64, pattern: &str) -> Result<String, ErrorValue> {
    let mut pieces: Vec<Piece> = Vec::new();
    for c in pattern.chars() {
        let lower = c.to_ascii_lowercase();
        if matches!(lower, 'y' | 'm' | 'd' | 'h' | 's') {
            if let Some(Piece::Letters(last, length)) = pieces.last_mut() {
                if *last == lower {
                    *length = length.saturating_add(1);
                    continue;
                }
            }
            pieces.push(Piece::Letters(lower, 1));
        } else {
            pieces.push(Piece::Literal(c));
        }
    }
    if !pieces.iter().any(|p| matches!(p, Piece::Letters(..))) {
        return Err(ErrorValue::Value);
    }
    // Which runs of letters there are, without the literals between them, to decide
    // whether an `m` stands for minutes.
    let letters: Vec<char> = pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Letters(letter, _) => Some(*letter),
            Piece::Literal(_) => None,
        })
        .collect();
    let (year, month, day_of_month) = day.ymd();
    let hours = millis_of_day.checked_div(3_600_000).unwrap_or(0);
    let minutes = millis_of_day
        .checked_rem(3_600_000)
        .and_then(|rest| rest.checked_div(60_000))
        .unwrap_or(0);
    let seconds = millis_of_day
        .checked_rem(60_000)
        .and_then(|rest| rest.checked_div(1_000))
        .unwrap_or(0);
    let month_name = usize::try_from(month.saturating_sub(1))
        .ok()
        .and_then(|index| MONTHS.get(index))
        .copied()
        .unwrap_or("");
    let padded = |n: i64, length: usize| {
        if length >= 2 {
            format!("{n:02}")
        } else {
            n.to_string()
        }
    };
    let mut out = String::new();
    let mut letter_index = 0_usize;
    for piece in &pieces {
        match *piece {
            Piece::Literal(c) => out.push(c),
            Piece::Letters(letter, length) => {
                let previous = letter_index
                    .checked_sub(1)
                    .and_then(|i| letters.get(i))
                    .copied();
                let next = letters.get(letter_index.saturating_add(1)).copied();
                letter_index = letter_index.saturating_add(1);
                match letter {
                    'y' => {
                        if length <= 2 {
                            out.push_str(&format!("{:02}", year.rem_euclid(100)));
                        } else {
                            out.push_str(&format!("{year:04}"));
                        }
                    }
                    'm' if length <= 2 && (previous == Some('h') || next == Some('s')) => {
                        out.push_str(&padded(minutes, length));
                    }
                    'm' => match length {
                        1 | 2 => out.push_str(&padded(i64::from(month), length)),
                        3 => out.push_str(month_name.get(..3).unwrap_or(month_name)),
                        _ => out.push_str(month_name),
                    },
                    'd' => match length {
                        1 | 2 => out.push_str(&padded(i64::from(day_of_month), length)),
                        3 => {
                            let name = weekday_name(day.weekday());
                            out.push_str(name.get(..3).unwrap_or(name));
                        }
                        _ => out.push_str(weekday_name(day.weekday())),
                    },
                    'h' => out.push_str(&padded(hours, length)),
                    // Only `y`, `m`, `d`, `h` and `s` are collected as letters.
                    _ => out.push_str(&padded(seconds, length)),
                }
            }
        }
    }
    Ok(out)
}

/// `TEXT(value, pattern)`.
pub(crate) fn format(value: &Val, pattern: &str, env: &dyn Env) -> Result<String, ErrorValue> {
    match value {
        Val::Blank => Ok(String::new()),
        Val::Number(n) | Val::Money(n, _) => number(*n, pattern),
        Val::Date(day) => date(*day, 0, pattern),
        Val::DateTime(at) => {
            let day = day_of(*at, env);
            let since_midnight = at
                .millis()
                .checked_sub(env.start_of_day(day).millis())
                .ok_or(ErrorValue::Num)?;
            date(day, since_midnight, pattern)
        }
        Val::Text(_)
        | Val::Bool(_)
        | Val::Duration(_)
        | Val::Ref(_)
        | Val::List(_)
        | Val::Error(_) => Err(ErrorValue::Value),
    }
}

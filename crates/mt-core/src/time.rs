//! Calendar dates, UTC instants and durations.
//!
//! * [`Date`] — a calendar day (proleptic Gregorian), stored as days since 1970-01-01.
//! * [`Timestamp`] — an instant in UTC, stored as integer milliseconds since the Unix epoch.
//! * [`DurationMs`] — a signed length of time in milliseconds.
//!
//! Supported range: years 0001 to 9999 inclusive. Nothing here reads a clock; "now" is always
//! an input supplied by the caller. No time-zone database is included: field time zones are
//! metadata applied by higher layers.

use crate::decimal::Decimal;
use crate::error::{clip, CoreError};
use core::fmt;
use core::str::FromStr;
use rust_decimal::prelude::ToPrimitive;

const MS_PER_SECOND: i64 = 1_000;
const MS_PER_MINUTE: i64 = 60_000;
const MS_PER_HOUR: i64 = 3_600_000;
const MS_PER_DAY: i64 = 86_400_000;

const MIN_DAYS: i32 = -719_162; // 0001-01-01
const MAX_DAYS: i32 = 2_932_896; // 9999-12-31

/// Whether `year` is a leap year in the Gregorian calendar.
#[must_use]
pub const fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Number of days in a month; `0` if `month` is not 1–12.
#[must_use]
pub const fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Days since 1970-01-01 for a valid civil date (Howard Hinnant's `days_from_civil`).
/// Callers guarantee 1 ≤ year ≤ 9999 and a valid month/day, so no step can overflow.
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = i64::from(year).saturating_sub(i64::from(month <= 2));
    let m = i64::from(month);
    let era = y / 400;
    let yoe = y % 400;
    let mp = if m > 2 { m.saturating_sub(3) } else { m.saturating_add(9) };
    let doy = (mp.saturating_mul(153).saturating_add(2) / 5).saturating_add(i64::from(day)).saturating_sub(1);
    let doe = yoe.saturating_mul(365).saturating_add(yoe / 4).saturating_sub(yoe / 100).saturating_add(doy);
    era.saturating_mul(146_097).saturating_add(doe).saturating_sub(719_468)
}

/// Civil date for days since 1970-01-01 (Howard Hinnant's `civil_from_days`).
/// Callers guarantee the day number is within the supported range.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days.saturating_add(719_468);
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = doe.saturating_sub(doe / 1_460).saturating_add(doe / 36_524).saturating_sub(doe / 146_096) / 365;
    let doy = doe.saturating_sub(yoe.saturating_mul(365).saturating_add(yoe / 4).saturating_sub(yoe / 100));
    let mp = doy.saturating_mul(5).saturating_add(2) / 153;
    let day = doy.saturating_sub(mp.saturating_mul(153).saturating_add(2) / 5).saturating_add(1);
    let month = if mp < 10 { mp.saturating_add(3) } else { mp.saturating_sub(9) };
    let year = yoe.saturating_add(era.saturating_mul(400)).saturating_add(i64::from(month <= 2));
    (
        i32::try_from(year).unwrap_or(0),
        u32::try_from(month).unwrap_or(0),
        u32::try_from(day).unwrap_or(0),
    )
}

/// Reads a run of ASCII digits of an exact length as a number.
fn fixed_digits(text: &str, len: usize) -> Option<u32> {
    if text.len() == len && text.bytes().all(|b| b.is_ascii_digit()) {
        text.parse().ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------------------------
// Date
// ---------------------------------------------------------------------------------------------

/// A calendar day without a time or zone. Text form: `YYYY-MM-DD`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date(i32);

impl Date {
    /// The first supported day, `0001-01-01`.
    pub const MIN: Date = Date(MIN_DAYS);
    /// The last supported day, `9999-12-31`.
    pub const MAX: Date = Date(MAX_DAYS);
    /// `1970-01-01`.
    pub const EPOCH: Date = Date(0);

    /// Builds a date from year, month (1–12) and day (1–31).
    ///
    /// # Errors
    /// [`CoreError::InvalidTime`] if the date does not exist or the year is outside 1–9999.
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Result<Self, CoreError> {
        let err = |reason| CoreError::InvalidTime { input: format!("{year}-{month}-{day}"), reason };
        if !(1..=9999).contains(&year) {
            return Err(err("year must be between 1 and 9999"));
        }
        if !(1..=12).contains(&month) {
            return Err(err("month must be between 1 and 12"));
        }
        if day < 1 || day > days_in_month(year, month) {
            return Err(err("day does not exist in that month"));
        }
        Ok(Date(i32::try_from(days_from_civil(year, month, day)).unwrap_or(0)))
    }

    /// Builds a date from days since 1970-01-01.
    ///
    /// # Errors
    /// [`CoreError::TimeOutOfRange`] outside years 1–9999.
    pub fn from_days(days: i32) -> Result<Self, CoreError> {
        if (MIN_DAYS..=MAX_DAYS).contains(&days) {
            Ok(Date(days))
        } else {
            Err(CoreError::TimeOutOfRange(i64::from(days)))
        }
    }

    /// Days since 1970-01-01 (negative before).
    #[must_use]
    pub const fn days(self) -> i32 {
        self.0
    }

    /// `(year, month, day)`.
    #[must_use]
    pub fn ymd(self) -> (i32, u32, u32) {
        civil_from_days(i64::from(self.0))
    }

    /// The year (1–9999).
    #[must_use]
    pub fn year(self) -> i32 {
        self.ymd().0
    }

    /// The month (1–12).
    #[must_use]
    pub fn month(self) -> u32 {
        self.ymd().1
    }

    /// The day of the month (1–31).
    #[must_use]
    pub fn day(self) -> u32 {
        self.ymd().2
    }

    /// Midnight UTC at the start of this day.
    #[must_use]
    pub fn to_timestamp(self) -> Timestamp {
        Timestamp(i64::from(self.0).saturating_mul(MS_PER_DAY))
    }

    /// The date `days` later (or earlier when negative); `None` if out of range.
    #[must_use]
    pub fn checked_add_days(self, days: i32) -> Option<Date> {
        self.0.checked_add(days).and_then(|d| Date::from_days(d).ok())
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, m, d) = self.ymd();
        write!(f, "{y:04}-{m:02}-{d:02}")
    }
}

impl fmt::Debug for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for Date {
    type Err = CoreError;

    /// Parses the strict form `YYYY-MM-DD`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || CoreError::InvalidTime { input: clip(s), reason: "expected YYYY-MM-DD" };
        let mut parts = s.splitn(3, '-');
        let year = parts.next().and_then(|p| fixed_digits(p, 4)).ok_or_else(err)?;
        let month = parts.next().and_then(|p| fixed_digits(p, 2)).ok_or_else(err)?;
        let day = parts.next().and_then(|p| fixed_digits(p, 2)).ok_or_else(err)?;
        Date::from_ymd(i32::try_from(year).unwrap_or(0), month, day)
    }
}

impl serde::Serialize for Date {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for Date {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str> as serde::Deserialize>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------------------------------
// Timestamp
// ---------------------------------------------------------------------------------------------

/// An instant in UTC: integer milliseconds since `1970-01-01T00:00:00Z`.
///
/// Text form (RFC 3339, always UTC, always milliseconds): `YYYY-MM-DDTHH:MM:SS.mmmZ`. The text
/// form is fixed-width, so it sorts chronologically. Serde form: the integer.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i64);

impl Timestamp {
    /// `0001-01-01T00:00:00.000Z`.
    pub const MIN: Timestamp = Timestamp(-62_135_596_800_000);
    /// `9999-12-31T23:59:59.999Z`.
    pub const MAX: Timestamp = Timestamp(253_402_300_799_999);
    /// `1970-01-01T00:00:00.000Z`.
    pub const EPOCH: Timestamp = Timestamp(0);

    /// Wraps milliseconds since the Unix epoch.
    ///
    /// # Errors
    /// [`CoreError::TimeOutOfRange`] outside years 1–9999.
    pub fn from_millis(millis: i64) -> Result<Self, CoreError> {
        if (Self::MIN.0..=Self::MAX.0).contains(&millis) {
            Ok(Timestamp(millis))
        } else {
            Err(CoreError::TimeOutOfRange(millis))
        }
    }

    /// Milliseconds since the Unix epoch.
    #[must_use]
    pub const fn millis(self) -> i64 {
        self.0
    }

    /// The UTC calendar day this instant falls on.
    #[must_use]
    pub fn date(self) -> Date {
        Date(i32::try_from(self.0.div_euclid(MS_PER_DAY)).unwrap_or(0))
    }

    /// Milliseconds since UTC midnight (0 ..= 86 399 999).
    #[must_use]
    pub fn millis_of_day(self) -> u32 {
        u32::try_from(self.0.rem_euclid(MS_PER_DAY)).unwrap_or(0)
    }

    /// This instant shifted by a duration; `None` if the result is out of range.
    #[must_use]
    pub fn checked_add(self, duration: DurationMs) -> Option<Timestamp> {
        self.0.checked_add(duration.0).and_then(|ms| Timestamp::from_millis(ms).ok())
    }

    /// `self - earlier` as a duration (negative if `earlier` is later).
    #[must_use]
    pub fn duration_since(self, earlier: Timestamp) -> DurationMs {
        // Both operands are within ±2.6e14, so the difference cannot overflow.
        DurationMs(self.0.saturating_sub(earlier.0))
    }

    /// Parses forgiving, locale-neutral date-time input:
    ///
    /// * RFC 3339 (`2024-05-06T07:08:09.123+09:00`, `…Z`);
    /// * a space instead of `T`;
    /// * seconds may be omitted (`2024-05-06 07:08`);
    /// * the offset may be omitted, in which case the time is taken as **UTC**;
    /// * a date alone (`2024-05-06`) means midnight UTC.
    ///
    /// Fractions finer than a millisecond are truncated. Leap seconds (`:60`) are rejected.
    ///
    /// # Errors
    /// [`CoreError::InvalidTime`] or [`CoreError::TimeOutOfRange`].
    pub fn parse_lenient(input: &str) -> Result<Self, CoreError> {
        parse_timestamp(input.trim(), false)
    }
}

fn parse_timestamp(s: &str, strict: bool) -> Result<Timestamp, CoreError> {
    let err = |reason| CoreError::InvalidTime { input: clip(s), reason };
    let date_text = s.get(..10).ok_or_else(|| err("expected a date YYYY-MM-DD"))?;
    let date: Date = date_text.parse()?;
    let rest = s.get(10..).unwrap_or("");
    if rest.is_empty() {
        if strict {
            return Err(err("expected a time after the date"));
        }
        return Ok(date.to_timestamp());
    }
    let after_sep = match rest.strip_prefix('T') {
        Some(r) => r,
        None => match rest.strip_prefix(' ') {
            Some(r) if !strict => r,
            Some(_) | None => return Err(err("expected `T` between date and time")),
        },
    };
    // Split the clock from the offset.
    let offset_at = after_sep.find(['Z', '+', '-']);
    let (clock, offset_text) = match offset_at {
        Some(at) => (after_sep.get(..at).unwrap_or(""), after_sep.get(at..).unwrap_or("")),
        None => (after_sep, ""),
    };
    let offset_minutes: i64 = if offset_text.is_empty() {
        if strict {
            return Err(err("expected `Z` or a UTC offset"));
        }
        0
    } else if offset_text == "Z" {
        0
    } else {
        let negative = offset_text.starts_with('-');
        let body = offset_text.get(1..).unwrap_or("");
        let (hh, mm) = body.split_once(':').ok_or_else(|| err("offset must be ±HH:MM"))?;
        let hh = fixed_digits(hh, 2).ok_or_else(|| err("offset must be ±HH:MM"))?;
        let mm = fixed_digits(mm, 2).ok_or_else(|| err("offset must be ±HH:MM"))?;
        if hh > 23 || mm > 59 {
            return Err(err("offset out of range"));
        }
        let total = i64::from(hh).saturating_mul(60).saturating_add(i64::from(mm));
        if negative {
            total.saturating_neg()
        } else {
            total
        }
    };
    // Clock: HH:MM[:SS[.fraction]]
    let (hms, fraction) = match clock.split_once('.') {
        Some((hms, fraction)) => (hms, Some(fraction)),
        None => (clock, None),
    };
    let mut parts = hms.split(':');
    let hour = parts.next().and_then(|p| fixed_digits(p, 2)).ok_or_else(|| err("expected HH:MM"))?;
    let minute = parts.next().and_then(|p| fixed_digits(p, 2)).ok_or_else(|| err("expected HH:MM"))?;
    let second = match parts.next() {
        Some(p) => Some(fixed_digits(p, 2).ok_or_else(|| err("seconds must be two digits"))?),
        None => None,
    };
    if parts.next().is_some() {
        return Err(err("too many `:` in the time"));
    }
    if strict && second.is_none() {
        return Err(err("seconds are required"));
    }
    if hour > 23 || minute > 59 || second.unwrap_or(0) > 59 {
        return Err(err("time of day out of range"));
    }
    let millis = match fraction {
        None => 0,
        Some(f) => {
            if second.is_none() || f.is_empty() || f.len() > 9 || !f.bytes().all(|b| b.is_ascii_digit()) {
                return Err(err("fraction must be 1 to 9 digits after the seconds"));
            }
            let padded: String = f.chars().chain("00".chars()).take(3).collect();
            padded.parse::<u32>().unwrap_or(0)
        }
    };
    let total = i64::from(date.days())
        .saturating_mul(MS_PER_DAY)
        .saturating_add(i64::from(hour).saturating_mul(MS_PER_HOUR))
        .saturating_add(i64::from(minute).saturating_mul(MS_PER_MINUTE))
        .saturating_add(i64::from(second.unwrap_or(0)).saturating_mul(MS_PER_SECOND))
        .saturating_add(i64::from(millis))
        .saturating_sub(offset_minutes.saturating_mul(MS_PER_MINUTE));
    Timestamp::from_millis(total)
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, mo, d) = self.date().ymd();
        let ms = self.millis_of_day();
        let (h, mi, s, frac) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1_000 % 60, ms % 1_000);
        write!(f, "{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{frac:03}Z")
    }
}

impl fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for Timestamp {
    type Err = CoreError;

    /// Parses strict RFC 3339: `YYYY-MM-DDTHH:MM:SS[.fraction](Z|±HH:MM)`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_timestamp(s, true)
    }
}

impl serde::Serialize for Timestamp {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(self.0)
    }
}

impl<'de> serde::Deserialize<'de> for Timestamp {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let millis = <i64 as serde::Deserialize>::deserialize(deserializer)?;
        Timestamp::from_millis(millis).map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------------------------------
// Duration
// ---------------------------------------------------------------------------------------------

/// A signed length of time in milliseconds.
///
/// Text form (ISO 8601 with hours as the largest unit): `PT1H30M`, `PT0.5S`, `-PT2H`, `PT0S`.
/// Serde form: the integer.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct DurationMs(i64);

impl DurationMs {
    /// Zero length.
    pub const ZERO: DurationMs = DurationMs(0);

    /// Wraps a number of milliseconds.
    #[must_use]
    pub const fn from_millis(millis: i64) -> Self {
        DurationMs(millis)
    }

    /// The number of milliseconds.
    #[must_use]
    pub const fn millis(self) -> i64 {
        self.0
    }

    /// Sum of two durations; `None` on overflow.
    #[must_use]
    pub fn checked_add(self, other: DurationMs) -> Option<DurationMs> {
        self.0.checked_add(other.0).map(DurationMs)
    }
}

impl fmt::Display for DurationMs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let abs = self.0.unsigned_abs();
        let (h, m, s, ms) = (abs / 3_600_000, abs / 60_000 % 60, abs / 1_000 % 60, abs % 1_000);
        if self.0 < 0 {
            f.write_str("-")?;
        }
        f.write_str("PT")?;
        if h > 0 {
            write!(f, "{h}H")?;
        }
        if m > 0 {
            write!(f, "{m}M")?;
        }
        if ms > 0 {
            let frac = format!("{ms:03}");
            write!(f, "{s}.{}S", frac.trim_end_matches('0'))?;
        } else if s > 0 || abs == 0 {
            write!(f, "{s}S")?;
        }
        Ok(())
    }
}

impl fmt::Debug for DurationMs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

const ISO_DATE_UNITS: &[(&str, i64)] = &[("D", MS_PER_DAY)];
const ISO_TIME_UNITS: &[(&str, i64)] = &[("H", MS_PER_HOUR), ("M", MS_PER_MINUTE), ("S", MS_PER_SECOND)];
const PLAIN_UNITS: &[(&str, i64)] = &[
    ("d", MS_PER_DAY),
    ("h", MS_PER_HOUR),
    ("m", MS_PER_MINUTE),
    ("min", MS_PER_MINUTE),
    ("s", MS_PER_SECOND),
    ("sec", MS_PER_SECOND),
    ("ms", 1),
];

/// Multiplies an exact decimal amount by a unit size; the result must be whole milliseconds.
fn component_ms(amount: &str, unit_ms: i64) -> Option<i64> {
    let amount = crate::decimal::parse_decimal(amount).ok()?;
    let product = amount.checked_mul(Decimal::from(unit_ms))?;
    if !product.fract().is_zero() {
        return None;
    }
    product.to_i64()
}

/// Reads a sequence of `<number><unit>` components and sums them. Units must appear in the
/// order of `units` and at most once each; `None` on any error or if there are no components.
fn sum_components(text: &str, units: &[(&str, i64)], fold_case: bool) -> Option<i64> {
    let mut rest = text.trim_start();
    let mut total: i64 = 0;
    let mut count = 0_u32;
    let mut last_size = i64::MAX;
    while !rest.is_empty() {
        let number_len = rest.bytes().take_while(|b| b.is_ascii_digit() || *b == b'.').count();
        let number = rest.get(..number_len)?;
        let after_number = rest.get(number_len..)?;
        let unit_len = after_number.bytes().take_while(u8::is_ascii_alphabetic).count();
        let unit_raw = after_number.get(..unit_len)?;
        let unit = if fold_case { unit_raw.to_ascii_lowercase() } else { unit_raw.to_owned() };
        let (_, size) = units.iter().find(|(name, _)| *name == unit)?;
        if *size >= last_size {
            return None;
        }
        last_size = *size;
        total = total.checked_add(component_ms(number, *size)?)?;
        count = count.saturating_add(1);
        rest = after_number.get(unit_len..)?.trim_start();
    }
    (count > 0).then_some(total)
}

fn parse_clock_duration(text: &str) -> Option<i64> {
    let mut parts = text.split(':');
    let hours_text = parts.next()?;
    let minutes_text = parts.next()?;
    let seconds_text = parts.next();
    if parts.next().is_some() {
        return None;
    }
    if hours_text.is_empty() || !hours_text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hours = component_ms(hours_text, MS_PER_HOUR)?;
    let minutes = fixed_digits(minutes_text, 2).filter(|m| *m < 60)?;
    let seconds_ms = match seconds_text {
        None => 0,
        Some(sec) => {
            let whole = sec.split('.').next()?;
            fixed_digits(whole, 2).filter(|s| *s < 60)?;
            component_ms(sec, MS_PER_SECOND)?
        }
    };
    hours.checked_add(i64::from(minutes).checked_mul(MS_PER_MINUTE)?)?.checked_add(seconds_ms)
}

impl FromStr for DurationMs {
    type Err = CoreError;

    /// Parses a duration in one of three locale-neutral notations (an optional leading `-`
    /// or `+` applies to the whole value):
    ///
    /// * ISO 8601: `P1DT2H30M15.5S`, `PT90M` (units `D`, `H`, `M`, `S`; upper case);
    /// * units: `1d 2h 30m 15s 500ms`, `1.5h`, `90min` (largest unit first, each at most once);
    /// * clock: `H:MM` or `H:MM:SS[.fff]` (`1:30` is one hour thirty minutes).
    ///
    /// A bare number is rejected because its unit would be a guess. Every component must be a
    /// whole number of milliseconds.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || CoreError::InvalidTime { input: clip(s), reason: "expected a duration such as 1h30m, 1:30 or PT1H30M" };
        let trimmed = s.trim();
        let (negative, body) = match trimmed.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
        };
        let magnitude = if let Some(iso) = body.strip_prefix('P') {
            let (date_part, time_part) = match iso.split_once('T') {
                Some((date_part, time_part)) => (date_part, Some(time_part)),
                None => (iso, None),
            };
            let days = if date_part.is_empty() { Some(0) } else { sum_components(date_part, ISO_DATE_UNITS, false) };
            let time = match time_part {
                None => (!date_part.is_empty()).then_some(0),
                Some(t) => sum_components(t, ISO_TIME_UNITS, false),
            };
            match (days, time) {
                (Some(d), Some(t)) if !iso.contains(' ') => d.checked_add(t),
                (Some(_) | None, Some(_) | None) => None,
            }
        } else if body.contains(':') {
            parse_clock_duration(body)
        } else {
            sum_components(body, PLAIN_UNITS, true)
        };
        let magnitude = magnitude.ok_or_else(err)?;
        let signed = if negative { magnitude.checked_neg() } else { Some(magnitude) };
        signed.map(DurationMs).ok_or_else(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }
    fn dur(s: &str) -> i64 {
        s.parse::<DurationMs>().unwrap().millis()
    }

    #[test]
    fn std_date_basics() {
        let d = Date::from_ymd(2024, 2, 29).unwrap();
        assert_eq!(d.to_string(), "2024-02-29");
        assert_eq!(format!("{d:?}"), "2024-02-29");
        assert_eq!((d.year(), d.month(), d.day()), (2024, 2, 29));
        assert_eq!("2024-02-29".parse::<Date>().unwrap(), d);
        assert_eq!(Date::EPOCH.to_string(), "1970-01-01");
        assert_eq!(Date::from_ymd(1970, 1, 1).unwrap().days(), 0);
        assert_eq!(Date::from_ymd(2000, 3, 1).unwrap().days(), 11_017);
        assert_eq!(d.checked_add_days(1).unwrap().to_string(), "2024-03-01");
        assert_eq!(d.checked_add_days(-29).unwrap().to_string(), "2024-01-31");
        assert_eq!(d.to_timestamp().to_string(), "2024-02-29T00:00:00.000Z");
        assert_eq!(serde_json::to_string(&d).unwrap(), "\"2024-02-29\"");
        assert_eq!(serde_json::from_str::<Date>("\"2024-02-29\"").unwrap(), d);
    }

    #[test]
    fn edge_date_range() {
        assert_eq!(Date::MIN.to_string(), "0001-01-01");
        assert_eq!(Date::MAX.to_string(), "9999-12-31");
        assert_eq!(Date::from_ymd(1, 1, 1).unwrap(), Date::MIN);
        assert_eq!(Date::from_ymd(9999, 12, 31).unwrap(), Date::MAX);
        assert!(Date::from_days(MIN_DAYS - 1).is_err());
        assert!(Date::from_days(MAX_DAYS + 1).is_err());
        assert_eq!(Date::MAX.checked_add_days(1), None);
        assert_eq!(Date::MAX.checked_add_days(i32::MAX), None);
        assert_eq!(Date::MIN.checked_add_days(-1), None);
        assert_eq!(Date::from_ymd(1969, 12, 31).unwrap().days(), -1);
    }

    #[test]
    fn edge_every_day_round_trips() {
        // Exhaustive over the whole supported range: 3.65 million days.
        let mut prev = (0, 12, 31);
        for days in MIN_DAYS..=MAX_DAYS {
            let date = Date::from_days(days).unwrap();
            let (y, m, d) = date.ymd();
            assert_eq!(Date::from_ymd(y, m, d).unwrap().days(), days);
            let expected = if prev.2 < days_in_month(prev.0, prev.1) {
                (prev.0, prev.1, prev.2 + 1)
            } else if prev.1 < 12 {
                (prev.0, prev.1 + 1, 1)
            } else {
                (prev.0 + 1, 1, 1)
            };
            assert_eq!((y, m, d), expected);
            prev = (y, m, d);
        }
        assert_eq!(prev, (9999, 12, 31));
    }

    #[test]
    fn std_leap_years() {
        assert!(is_leap_year(2000));
        assert!(!is_leap_year(1900));
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(2023));
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2024, 4), 30);
        assert_eq!(days_in_month(2024, 13), 0);
    }

    #[test]
    fn invalid_dates() {
        assert!(Date::from_ymd(2023, 2, 29).is_err());
        assert!(Date::from_ymd(2023, 0, 1).is_err());
        assert!(Date::from_ymd(2023, 13, 1).is_err());
        assert!(Date::from_ymd(2023, 4, 31).is_err());
        assert!(Date::from_ymd(2023, 4, 0).is_err());
        assert!(Date::from_ymd(0, 1, 1).is_err());
        assert!(Date::from_ymd(10_000, 1, 1).is_err());
        for bad in ["", "2024", "2024-1-01", "2024-01-1", "24-01-01", "2024/01/01", "2024-01-01 ", "2024-01-01-01", "abcd-01-01", "+024-01-01", "2024-02-30", "0000-01-01"] {
            assert!(bad.parse::<Date>().is_err(), "{bad:?}");
        }
        assert!(serde_json::from_str::<Date>("\"2024-13-01\"").is_err());
        assert!(serde_json::from_str::<Date>("20240101").is_err());
    }

    #[test]
    fn std_timestamp_basics() {
        let t = ts("2024-05-06T07:08:09.123Z");
        assert_eq!(t.millis(), 1_714_979_289_123);
        assert_eq!(t.to_string(), "2024-05-06T07:08:09.123Z");
        assert_eq!(format!("{t:?}"), "2024-05-06T07:08:09.123Z");
        assert_eq!(t.date().to_string(), "2024-05-06");
        assert_eq!(t.millis_of_day(), 7 * 3_600_000 + 8 * 60_000 + 9_123);
        assert_eq!(ts("2024-05-06T16:08:09.123+09:00"), t);
        assert_eq!(ts("2024-05-06T02:38:09.123-04:30"), t);
        assert_eq!(ts("2024-05-06T07:08:09Z").millis(), 1_714_979_289_000);
        assert_eq!(ts("2024-05-06T07:08:09.1Z").millis(), 1_714_979_289_100);
        assert_eq!(ts("2024-05-06T07:08:09.123999999Z"), t);
        assert_eq!(serde_json::to_string(&t).unwrap(), "1714979289123");
        assert_eq!(serde_json::from_str::<Timestamp>("1714979289123").unwrap(), t);
        assert_eq!(Timestamp::EPOCH.to_string(), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn std_timestamp_lenient() {
        let t = ts("2024-05-06T07:08:00.000Z");
        assert_eq!(Timestamp::parse_lenient("2024-05-06 07:08").unwrap(), t);
        assert_eq!(Timestamp::parse_lenient(" 2024-05-06T07:08 ").unwrap(), t);
        assert_eq!(Timestamp::parse_lenient("2024-05-06 07:08:00").unwrap(), t);
        assert_eq!(Timestamp::parse_lenient("2024-05-06 16:08+09:00").unwrap(), t);
        assert_eq!(Timestamp::parse_lenient("2024-05-06").unwrap(), ts("2024-05-06T00:00:00Z"));
        assert_eq!(Timestamp::parse_lenient("2024-05-06T07:08:00Z").unwrap(), t);
    }

    #[test]
    fn std_timestamp_arithmetic() {
        let t = ts("2024-01-01T00:00:00Z");
        let later = t.checked_add(DurationMs::from_millis(MS_PER_DAY)).unwrap();
        assert_eq!(later.to_string(), "2024-01-02T00:00:00.000Z");
        assert_eq!(later.duration_since(t).millis(), MS_PER_DAY);
        assert_eq!(t.duration_since(later).millis(), -MS_PER_DAY);
    }

    #[test]
    fn edge_timestamp_range_and_negative() {
        assert_eq!(Timestamp::MIN.to_string(), "0001-01-01T00:00:00.000Z");
        assert_eq!(Timestamp::MAX.to_string(), "9999-12-31T23:59:59.999Z");
        assert_eq!(ts("0001-01-01T00:00:00Z"), Timestamp::MIN);
        assert_eq!(ts("9999-12-31T23:59:59.999Z"), Timestamp::MAX);
        assert!(Timestamp::from_millis(Timestamp::MAX.millis() + 1).is_err());
        assert!(Timestamp::from_millis(Timestamp::MIN.millis() - 1).is_err());
        assert!(Timestamp::from_millis(i64::MIN).is_err());
        let before_epoch = Timestamp::from_millis(-1).unwrap();
        assert_eq!(before_epoch.to_string(), "1969-12-31T23:59:59.999Z");
        assert_eq!(before_epoch.date().to_string(), "1969-12-31");
        assert_eq!(before_epoch.millis_of_day(), 86_399_999);
        assert_eq!(Timestamp::MAX.checked_add(DurationMs::from_millis(1)), None);
        assert_eq!(Timestamp::MAX.checked_add(DurationMs::from_millis(i64::MAX)), None);
        // Offsets can push a representable local time out of range.
        assert!("0001-01-01T00:00:00+01:00".parse::<Timestamp>().is_err());
        assert!("9999-12-31T23:59:59-01:00".parse::<Timestamp>().is_err());
        assert_eq!(Timestamp::MAX.duration_since(Timestamp::MIN).millis(), 315_537_897_599_999);
    }

    #[test]
    fn invalid_timestamps() {
        for bad in [
            "",
            "2024-05-06",
            "2024-05-06T",
            "2024-05-06 07:08:09Z",
            "2024-05-06T07:08Z",
            "2024-05-06T07:08:09",
            "2024-05-06T7:08:09Z",
            "2024-05-06T07:08:60Z",
            "2024-05-06T24:00:00Z",
            "2024-05-06T07:60:00Z",
            "2024-05-06T07:08:09.Z",
            "2024-05-06T07:08:09.1234567890Z",
            "2024-05-06T07:08:09.12aZ",
            "2024-05-06T07:08:09+0900",
            "2024-05-06T07:08:09+9:00",
            "2024-05-06T07:08:09+09:0",
            "2024-05-06T07:08:09+24:00",
            "2024-05-06T07:08:09+09:60",
            "2024-05-06T07:08:09:10Z",
            "2024-05-06T07:08:09z",
            "2024-05-06X07:08:09Z",
            "2024-13-06T07:08:09Z",
            "2024-05-06T07:08:9Z",
            "2024-05-06T07:08:09ZZ",
            "20é4-05-06T07:08:09Z",
        ] {
            assert!(bad.parse::<Timestamp>().is_err(), "{bad:?}");
        }
        for bad in ["", "2024", "2024-05-06 07", "2024-05-06 07:08.5", "2024-05-06_07:08", "yesterday"] {
            assert!(Timestamp::parse_lenient(bad).is_err(), "{bad:?}");
        }
        assert!(serde_json::from_str::<Timestamp>("999999999999999999").is_err());
        assert!(serde_json::from_str::<Timestamp>("\"2024-05-06T07:08:09Z\"").is_err());
    }

    #[test]
    fn std_duration_display() {
        assert_eq!(DurationMs::ZERO.to_string(), "PT0S");
        assert_eq!(DurationMs::from_millis(5_400_000).to_string(), "PT1H30M");
        assert_eq!(DurationMs::from_millis(500).to_string(), "PT0.5S");
        assert_eq!(DurationMs::from_millis(-7_200_000).to_string(), "-PT2H");
        assert_eq!(DurationMs::from_millis(3_661_001).to_string(), "PT1H1M1.001S");
        assert_eq!(DurationMs::from_millis(60_000).to_string(), "PT1M");
        assert_eq!(DurationMs::from_millis(59_000).to_string(), "PT59S");
        assert_eq!(format!("{:?}", DurationMs::from_millis(1_000)), "PT1S");
        assert_eq!(DurationMs::default(), DurationMs::ZERO);
        assert_eq!(serde_json::to_string(&DurationMs::from_millis(5)).unwrap(), "5");
        assert_eq!(serde_json::from_str::<DurationMs>("-5").unwrap().millis(), -5);
    }

    #[test]
    fn std_duration_parse_all_notations() {
        assert_eq!(dur("PT1H30M"), 5_400_000);
        assert_eq!(dur("P1DT2H30M15.5S"), 86_400_000 + 7_200_000 + 1_800_000 + 15_500);
        assert_eq!(dur("P2D"), 172_800_000);
        assert_eq!(dur("PT0S"), 0);
        assert_eq!(dur("-PT2H"), -7_200_000);
        assert_eq!(dur("1h30m"), 5_400_000);
        assert_eq!(dur("1h 30m"), 5_400_000);
        assert_eq!(dur(" 1d 2h 3m 4s 5ms "), 86_400_000 + 7_200_000 + 180_000 + 4_000 + 5);
        assert_eq!(dur("1.5h"), 5_400_000);
        assert_eq!(dur("90min"), 5_400_000);
        assert_eq!(dur("45sec"), 45_000);
        assert_eq!(dur("2H"), 7_200_000);
        assert_eq!(dur("+15m"), 900_000);
        assert_eq!(dur("-15m"), -900_000);
        assert_eq!(dur("1:30"), 5_400_000);
        assert_eq!(dur("0:00:01.5"), 1_500);
        assert_eq!(dur("100:00:00"), 360_000_000);
        assert_eq!(dur("-1:30"), -5_400_000);
    }

    #[test]
    fn std_duration_arithmetic() {
        let a = DurationMs::from_millis(5);
        assert_eq!(a.checked_add(DurationMs::from_millis(7)), Some(DurationMs::from_millis(12)));
        assert_eq!(DurationMs::from_millis(i64::MAX).checked_add(a), None);
    }

    #[test]
    fn edge_duration_extremes() {
        assert_eq!(DurationMs::from_millis(i64::MAX).to_string(), "PT2562047788015H12M55.807S");
        assert_eq!(DurationMs::from_millis(i64::MIN).to_string(), "-PT2562047788015H12M55.808S");
        let round: DurationMs = "PT2562047788015H12M55.807S".parse().unwrap();
        assert_eq!(round.millis(), i64::MAX);
        // i64::MIN magnitude does not fit a positive i64, so the text form of MIN is rejected.
        assert!("-PT2562047788015H12M55.808S".parse::<DurationMs>().is_err());
        assert!("99999999999999999999h".parse::<DurationMs>().is_err());
        assert!("2562047788015h 13m".parse::<DurationMs>().is_err());
        assert!("9999999999999999:00".parse::<DurationMs>().is_err());
    }

    #[test]
    fn invalid_durations() {
        for bad in [
            "", "-", "5", "1.5", "h", "1x", "1h1h", "30m1h", "1h30", "1.0005s", "0.5ms", "PT", "P", "P1H", "PT1D",
            "pt1h", "PT1h", "PT 1H", "P1D T1H", "1:3", "1:60", "1:30:60", "1:30:5", ":30", "1:30:15:00",
            "1.5:30", "a:30", "1:30:1a", "--1h", "1 h x", "1..5h", "PT1M1H",
        ] {
            assert!(bad.parse::<DurationMs>().is_err(), "{bad:?}");
        }
    }
}

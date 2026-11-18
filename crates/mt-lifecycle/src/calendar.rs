//! Business calendars: when work happens.
//!
//! A [`BusinessCalendar`] lists the working hours of each weekday, the days off, and the
//! calendar's offset from UTC. Deadlines are computed in working time: "8 working hours" from
//! Friday 16:00 in an office open 9–17 on weekdays is Monday 16:00.
//!
//! The offset is fixed. Where daylight saving applies, the caller keeps one calendar per
//! period — the crate never looks up time zones (it has no clock and no time-zone database).

use crate::error::LifecycleError;
use mt_core::time::{Date, DurationMs, Timestamp, Weekday};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Minutes in a day.
pub const DAY_MINUTES: u16 = 1440;
/// Longest span, in days, a calendar computation walks through (about ten years).
pub const MAX_SPAN_DAYS: i32 = 3_660;
/// Largest offset from UTC, in minutes.
pub const MAX_OFFSET_MINUTES: u32 = 840;
/// Most days off in one calendar.
pub const MAX_HOLIDAYS: usize = 10_000;

const MINUTE_MS: i64 = 60_000;
const DAY_MS: i64 = 86_400_000;

/// Working hours of one day: intervals `[start, end)` in minutes after local midnight.
pub type Hours = Vec<(u16, u16)>;

/// When work happens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusinessCalendar {
    /// Working hours per weekday, Monday first.
    pub week: [Hours; 7],
    /// Days off (local dates).
    #[serde(default)]
    pub holidays: BTreeSet<Date>,
    /// Offset of local time from UTC, in minutes (+540 for Tokyo, −300 for New York in winter).
    #[serde(default)]
    pub utc_offset_minutes: i32,
}

impl BusinessCalendar {
    /// Round the clock, every day, in UTC: working time equals elapsed time.
    #[must_use]
    pub fn always() -> Self {
        BusinessCalendar {
            week: core::array::from_fn(|_| vec![(0, DAY_MINUTES)]),
            holidays: BTreeSet::new(),
            utc_offset_minutes: 0,
        }
    }

    /// An office open from `open` to `close` (minutes after midnight) Monday to Friday.
    #[must_use]
    pub fn office(open: u16, close: u16, utc_offset_minutes: i32) -> Self {
        let day = vec![(open, close)];
        BusinessCalendar {
            week: [
                day.clone(),
                day.clone(),
                day.clone(),
                day.clone(),
                day,
                Vec::new(),
                Vec::new(),
            ],
            holidays: BTreeSet::new(),
            utc_offset_minutes,
        }
    }

    /// Verifies the calendar: ordered, non-overlapping intervals within the day, some working
    /// time each week, a sane offset and a bounded holiday list.
    ///
    /// # Errors
    /// [`LifecycleError::Invalid`].
    pub fn check(&self) -> Result<(), LifecycleError> {
        if self.utc_offset_minutes.unsigned_abs() > MAX_OFFSET_MINUTES {
            return Err(LifecycleError::invalid("a UTC offset is at most 14 hours"));
        }
        if self.holidays.len() > MAX_HOLIDAYS {
            return Err(LifecycleError::invalid(format!(
                "a calendar has at most {MAX_HOLIDAYS} days off"
            )));
        }
        for hours in &self.week {
            let mut previous_end = 0_u16;
            for (index, (start, end)) in hours.iter().enumerate() {
                if start >= end || *end > DAY_MINUTES {
                    return Err(LifecycleError::invalid(
                        "working hours run from a start to a later end within the day",
                    ));
                }
                if index > 0 && *start < previous_end {
                    return Err(LifecycleError::invalid(
                        "working hours of a day are ordered and do not overlap",
                    ));
                }
                previous_end = *end;
            }
        }
        if self.week.iter().all(Vec::is_empty) {
            return Err(LifecycleError::invalid("the calendar has no working time"));
        }
        Ok(())
    }

    fn offset_ms(&self) -> i64 {
        i64::from(self.utc_offset_minutes).saturating_mul(MINUTE_MS)
    }

    /// The local date of an instant.
    #[must_use]
    pub fn local_date(&self, at: Timestamp) -> Date {
        Timestamp::from_millis(at.millis().saturating_add(self.offset_ms()))
            .map_or_else(|_| at.date(), Timestamp::date)
    }

    /// Whether a local date has working time.
    #[must_use]
    pub fn is_working_day(&self, date: Date) -> bool {
        !self.holidays.contains(&date) && !self.hours(date).is_empty()
    }

    fn hours(&self, date: Date) -> &[(u16, u16)] {
        if self.holidays.contains(&date) {
            return &[];
        }
        let index = usize::from(date.weekday().days_from_monday());
        self.week.get(index).map_or(&[], Vec::as_slice)
    }

    /// The working intervals of a local date as UTC milliseconds `[start, end)`.
    fn intervals(&self, date: Date) -> Vec<(i64, i64)> {
        let midnight = date
            .to_timestamp()
            .millis()
            .saturating_sub(self.offset_ms());
        self.hours(date)
            .iter()
            .map(|(start, end)| {
                (
                    midnight.saturating_add(i64::from(*start).saturating_mul(MINUTE_MS)),
                    midnight.saturating_add(i64::from(*end).saturating_mul(MINUTE_MS)),
                )
            })
            .collect()
    }

    /// The instant `duration` of working time after `start`. Zero is `start` itself. `None`
    /// when the calendar has no working time within [`MAX_SPAN_DAYS`].
    #[must_use]
    pub fn add(&self, start: Timestamp, duration: DurationMs) -> Option<Timestamp> {
        let mut remaining = duration.millis().max(0);
        if remaining == 0 {
            return Some(start);
        }
        let mut cursor = start.millis();
        let first = self.local_date(start);
        // The day before covers intervals that cross local midnight into UTC.
        for offset in -1..=MAX_SPAN_DAYS {
            let date = first.checked_add_days(offset)?;
            for (from, to) in self.intervals(date) {
                let begin = from.max(cursor);
                if begin >= to {
                    continue;
                }
                let available = to.saturating_sub(begin);
                if available >= remaining {
                    return Timestamp::from_millis(begin.saturating_add(remaining)).ok();
                }
                remaining = remaining.saturating_sub(available);
                cursor = to;
            }
        }
        None
    }

    /// Working time in `[from, to)`; zero when `to` is not after `from`. `None` for a span
    /// longer than [`MAX_SPAN_DAYS`].
    #[must_use]
    pub fn between(&self, from: Timestamp, to: Timestamp) -> Option<DurationMs> {
        let (a, b) = (from.millis(), to.millis());
        if b <= a {
            return Some(DurationMs::ZERO);
        }
        if b.saturating_sub(a).checked_div(DAY_MS).unwrap_or(0) > i64::from(MAX_SPAN_DAYS) {
            return None;
        }
        let first = self.local_date(from);
        let last = self.local_date(to);
        let mut total = 0_i64;
        let mut offset = -1_i32;
        loop {
            let date = first.checked_add_days(offset)?;
            if date > last.checked_add_days(1)? {
                break;
            }
            for (start, end) in self.intervals(date) {
                let lo = start.max(a);
                let hi = end.min(b);
                if hi > lo {
                    total = total.saturating_add(hi.saturating_sub(lo));
                }
            }
            offset = offset.checked_add(1)?;
        }
        Some(DurationMs::from_millis(total))
    }

    /// The first working instant at or after `at`.
    #[must_use]
    pub fn next_working(&self, at: Timestamp) -> Option<Timestamp> {
        let first = self.local_date(at);
        for offset in -1..=MAX_SPAN_DAYS {
            let date = first.checked_add_days(offset)?;
            for (from, to) in self.intervals(date) {
                if to > at.millis() {
                    return Timestamp::from_millis(from.max(at.millis())).ok();
                }
            }
        }
        None
    }
}

/// The weekdays in calendar order, Monday first.
pub const WEEKDAYS: [Weekday; 7] = [
    Weekday::Monday,
    Weekday::Tuesday,
    Weekday::Wednesday,
    Weekday::Thursday,
    Weekday::Friday,
    Weekday::Saturday,
    Weekday::Sunday,
];

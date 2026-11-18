#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
//! Business calendars: deadlines in working time, days off, time zones, and properties.

mod common;

use common::*;
use mt_core::time::{Date, DurationMs, Timestamp};
use mt_lifecycle::calendar::{
    BusinessCalendar, DAY_MINUTES, MAX_HOLIDAYS, MAX_SPAN_DAYS, WEEKDAYS,
};
use mt_lifecycle::LifecycleError;
use proptest::prelude::*;

const NINE: u16 = 9 * 60;
const FIVE_PM: u16 = 17 * 60;

fn office() -> BusinessCalendar {
    BusinessCalendar::office(NINE, FIVE_PM, 0)
}

fn date(d: u32) -> Date {
    Date::from_ymd(2026, 11, d).unwrap()
}

// ---- standard ----

#[test]
fn eight_working_hours_from_friday_afternoon_end_on_monday_afternoon() {
    // Friday 20 November 16:00 + 8 h of a 9–17 office: 1 h on Friday, 7 h on Monday.
    let cal = office();
    assert_eq!(cal.add(at(20, 16, 0), hours(8)), Some(at(23, 16, 0)));
    assert_eq!(cal.between(at(20, 16, 0), at(23, 16, 0)), Some(hours(8)));
}

#[test]
fn days_off_are_skipped() {
    let mut cal = office();
    cal.holidays.insert(date(23));
    assert_eq!(cal.add(at(20, 16, 0), hours(8)), Some(at(24, 16, 0)));
    assert!(!cal.is_working_day(date(23)));
    assert!(cal.is_working_day(date(24)));
    assert!(!cal.is_working_day(date(21)), "Saturday");
}

#[test]
fn a_lunch_break_is_not_working_time() {
    let mut cal = office();
    cal.week[0] = vec![(NINE, 12 * 60), (13 * 60, FIVE_PM)];
    cal.check().unwrap();
    // Monday 16th: 9–12 is 3 h, then 13:00 + 1 h.
    assert_eq!(cal.add(at(16, 9, 0), hours(4)), Some(at(16, 14, 0)));
    assert_eq!(cal.between(at(16, 11, 0), at(16, 14, 0)), Some(hours(2)));
}

#[test]
fn round_the_clock_counts_elapsed_time() {
    let cal = BusinessCalendar::always();
    cal.check().unwrap();
    assert_eq!(cal.add(at(21, 23, 0), hours(30)), Some(at(23, 5, 0)));
    assert_eq!(cal.between(at(21, 23, 0), at(23, 5, 0)), Some(hours(30)));
    assert_eq!(cal.next_working(at(22, 3, 7)), Some(at(22, 3, 7)));
}

#[test]
fn next_working_instant() {
    let cal = office();
    assert_eq!(
        cal.next_working(at(21, 10, 0)),
        Some(at(23, 9, 0)),
        "Saturday"
    );
    assert_eq!(
        cal.next_working(at(20, 17, 0)),
        Some(at(23, 9, 0)),
        "Friday close"
    );
    assert_eq!(cal.next_working(at(20, 12, 0)), Some(at(20, 12, 0)), "open");
    assert_eq!(
        cal.next_working(at(20, 6, 0)),
        Some(at(20, 9, 0)),
        "before opening"
    );
}

// ---- time zones ----

#[test]
fn tokyo_office_hours_are_utc_midnight_to_eight() {
    // 9–17 at UTC+9 is 00:00–08:00 UTC.
    let cal = BusinessCalendar::office(NINE, FIVE_PM, 540);
    cal.check().unwrap();
    // Monday 16:00 local (07:00 UTC) + 2 h: one today, one from Tuesday 09:00 local.
    assert_eq!(cal.add(at(16, 7, 0), hours(2)), Some(at(17, 1, 0)));
    assert_eq!(cal.local_date(at(16, 20, 0)), date(17));
}

#[test]
fn new_york_office_hours_are_utc_afternoon() {
    // 9–17 at UTC−5 is 14:00–22:00 UTC.
    let cal = BusinessCalendar::office(NINE, FIVE_PM, -300);
    assert_eq!(cal.add(at(20, 21, 0), hours(2)), Some(at(23, 15, 0)));
    assert_eq!(
        cal.local_date(at(20, 2, 0)),
        date(19),
        "still Thursday in New York"
    );
    assert_eq!(cal.between(at(20, 0, 0), at(21, 0, 0)), Some(hours(8)));
}

#[test]
fn working_days_that_cross_utc_midnight() {
    // Whole local weekdays at UTC+9: Monday local starts on Sunday 15:00 UTC.
    let mut cal = BusinessCalendar::office(0, DAY_MINUTES, 540);
    cal.check().unwrap();
    // Friday 23:00 local (14:00 UTC) + 2 h: one hour on Friday, then Monday 00:00 local.
    assert_eq!(cal.add(at(20, 14, 0), hours(2)), Some(at(22, 16, 0)));
    assert_eq!(cal.between(at(20, 14, 0), at(22, 16, 0)), Some(hours(2)));
    cal.holidays.insert(date(23));
    assert_eq!(cal.add(at(20, 14, 0), hours(2)), Some(at(23, 16, 0)));
}

// ---- edge ----

#[test]
fn zero_and_negative_durations_are_the_start() {
    let cal = office();
    assert_eq!(
        cal.add(at(21, 10, 0), DurationMs::ZERO),
        Some(at(21, 10, 0))
    );
    assert_eq!(cal.add(at(21, 10, 0), ms(-5)), Some(at(21, 10, 0)));
    assert_eq!(
        cal.between(at(21, 10, 0), at(21, 10, 0)),
        Some(DurationMs::ZERO)
    );
    assert_eq!(
        cal.between(at(22, 10, 0), at(21, 10, 0)),
        Some(DurationMs::ZERO)
    );
}

#[test]
fn an_interval_end_is_exclusive() {
    let cal = office();
    // Exactly 8 h from opening ends at closing, on the same day.
    assert_eq!(cal.add(at(16, 9, 0), hours(8)), Some(at(16, 17, 0)));
    assert_eq!(
        cal.add(at(16, 9, 0), ms(8 * HOUR + 1)),
        Some(plus(at(17, 9, 0), 1))
    );
}

#[test]
fn a_calendar_without_working_time_has_no_deadline() {
    let cal = BusinessCalendar {
        week: Default::default(),
        holidays: Default::default(),
        utc_offset_minutes: 0,
    };
    assert!(cal.check().is_err());
    assert_eq!(cal.add(at(16, 9, 0), hours(1)), None);
    assert_eq!(cal.next_working(at(16, 9, 0)), None);
    assert_eq!(
        cal.between(at(16, 9, 0), at(18, 9, 0)),
        Some(DurationMs::ZERO)
    );
}

#[test]
fn a_deadline_beyond_ten_years_of_working_time_is_none() {
    // One working minute a weekday: 2,700 minutes is more than ten years away.
    let cal = BusinessCalendar::office(0, 1, 0);
    assert_eq!(cal.add(at(10, 0, 0), ms(2_700 * MINUTE)), None);
    assert!(cal.add(at(10, 0, 0), ms(2_000 * MINUTE)).is_some());
}

#[test]
fn spans_beyond_ten_years_are_refused() {
    let cal = office();
    let start = at(16, 0, 0);
    let far =
        Timestamp::from_millis(start.millis() + i64::from(MAX_SPAN_DAYS + 1) * 24 * HOUR).unwrap();
    assert_eq!(cal.between(start, far), None);
    // Near the end of representable time the walk stops instead of overflowing.
    assert_eq!(cal.add(Timestamp::MAX, hours(1)), None);
}

#[test]
fn weekdays_are_listed_monday_first() {
    for (index, weekday) in WEEKDAYS.iter().enumerate() {
        assert_eq!(usize::from(weekday.days_from_monday()), index);
    }
    assert_eq!(date(16).weekday(), WEEKDAYS[0]);
}

#[test]
fn calendars_round_trip_through_json() {
    let mut cal = BusinessCalendar::office(NINE, FIVE_PM, 60);
    cal.holidays.insert(date(25));
    let json = serde_json::to_string(&cal).unwrap();
    assert_eq!(
        serde_json::from_str::<BusinessCalendar>(&json).unwrap(),
        cal
    );
    // Days off and the offset may be left out.
    let short: BusinessCalendar =
        serde_json::from_str(r#"{"week":[[[0,60]],[],[],[],[],[],[]]}"#).unwrap();
    assert_eq!(short.utc_offset_minutes, 0);
    short.check().unwrap();
}

// ---- invalid ----

#[test]
fn invalid_calendars_are_refused() {
    let invalid = |cal: BusinessCalendar| matches!(cal.check(), Err(LifecycleError::Invalid(_)));
    assert!(invalid(BusinessCalendar::office(NINE, FIVE_PM, 900)));
    assert!(invalid(BusinessCalendar::office(NINE, FIVE_PM, -900)));
    assert!(
        invalid(BusinessCalendar::office(FIVE_PM, NINE, 0)),
        "start after end"
    );
    assert!(
        invalid(BusinessCalendar::office(NINE, NINE, 0)),
        "empty interval"
    );
    assert!(
        invalid(BusinessCalendar::office(NINE, DAY_MINUTES + 1, 0)),
        "past midnight"
    );
    let mut overlapping = office();
    overlapping.week[2] = vec![(NINE, 12 * 60), (11 * 60, FIVE_PM)];
    assert!(invalid(overlapping));
    let mut many = office();
    for n in 0..=MAX_HOLIDAYS {
        many.holidays
            .insert(Date::from_days(i32::try_from(n).unwrap()).unwrap());
    }
    assert!(invalid(many));
    assert!(BusinessCalendar::office(NINE, FIVE_PM, 840).check().is_ok());
}

// ---- properties ----

fn calendar() -> impl Strategy<Value = BusinessCalendar> {
    (
        0_u16..20 * 60,
        30_u16..6 * 60,
        -840_i32..=840,
        proptest::collection::btree_set(0_i32..120, 0..12),
        proptest::bool::ANY,
    )
        .prop_map(|(open, length, offset, holidays, saturday)| {
            let close = (open + length).min(DAY_MINUTES);
            let mut cal = BusinessCalendar::office(open, close, offset);
            if saturday {
                cal.week[5] = vec![(0, 120), (open, close)];
                cal.week[5].sort_unstable();
                cal.week[5].dedup();
                if cal.week[5].windows(2).any(|w| w[1].0 < w[0].1) {
                    cal.week[5] = vec![(open, close)];
                }
            }
            cal.holidays = holidays
                .into_iter()
                .map(|d| date(16).checked_add_days(d).unwrap())
                .collect();
            cal
        })
}

fn instant() -> impl Strategy<Value = Timestamp> {
    (0_i64..90 * 24 * 60).prop_map(|minutes| plus(day(10), minutes * MINUTE + 17))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn adding_then_measuring_gives_the_duration_back(
        cal in calendar(),
        start in instant(),
        minutes in 0_i64..200 * 60,
    ) {
        prop_assert!(cal.check().is_ok());
        let duration = DurationMs::from_millis(minutes * MINUTE);
        let end = cal.add(start, duration).unwrap();
        prop_assert!(end >= start);
        prop_assert_eq!(cal.between(start, end), Some(duration));
        if minutes > 0 {
            // The deadline is the end of working time: the last millisecond before it counts.
            prop_assert_eq!(cal.between(plus(end, -1), end), Some(ms(1)));
        }
    }

    #[test]
    fn working_time_is_additive(cal in calendar(), a in instant(), b in instant(), c in instant()) {
        let mut points = [a, b, c];
        points.sort();
        let [a, b, c] = points;
        let ab = cal.between(a, b).unwrap().millis();
        let bc = cal.between(b, c).unwrap().millis();
        prop_assert_eq!(cal.between(a, c).unwrap().millis(), ab + bc);
        prop_assert!(ab + bc <= c.millis() - a.millis());
    }

    #[test]
    fn the_next_working_instant_is_the_first(cal in calendar(), at in instant()) {
        let next = cal.next_working(at).unwrap();
        prop_assert!(next >= at);
        prop_assert_eq!(cal.between(at, next), Some(DurationMs::ZERO));
        prop_assert_eq!(cal.between(next, plus(next, 1)), Some(ms(1)));
        prop_assert!(cal.is_working_day(cal.local_date(next)));
    }
}

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
//! SLA clocks: warnings, breaches at the exact instant, pauses, stops, errors, and a
//! model-based test against a simple reference.

mod common;

use common::*;
use mt_core::time::{DurationMs, Timestamp};
use mt_lifecycle::calendar::BusinessCalendar;
use mt_lifecycle::sla::{ClockState, SlaClock, SlaEvent, SlaInput, SlaPolicy};
use mt_lifecycle::LifecycleError;
use proptest::prelude::*;

fn policy(target_hours: i64, warn_percent: u8) -> SlaPolicy {
    SlaPolicy {
        name: "Resolution".to_owned(),
        target: hours(target_hours),
        warn_percent,
        calendar: BusinessCalendar::always(),
    }
}

fn t(h: i64) -> Timestamp {
    plus(at(16, 0, 0), h * HOUR)
}

fn tick(at: Timestamp) -> SlaInput {
    SlaInput::Tick { at }
}

// ---- standard ----

#[test]
fn warns_once_then_breaches_once_at_the_exact_instants() {
    let mut clock = SlaClock::start(policy(4, 75), t(0)).unwrap();
    assert_eq!(clock.due_at(), Some(t(4)));
    assert_eq!(clock.step(tick(t(2))).unwrap(), vec![]);
    assert_eq!(
        clock.step(tick(t(3))).unwrap(),
        vec![SlaEvent::Warned { at: t(3) }]
    );
    assert_eq!(
        clock.step(tick(t(4))).unwrap(),
        vec![],
        "at the target it is still met"
    );
    assert_eq!(
        clock.step(tick(plus(t(4), 1))).unwrap(),
        vec![SlaEvent::Breached { at: t(4) }]
    );
    assert_eq!(clock.step(tick(t(9))).unwrap(), vec![]);
    assert_eq!(
        clock.step(SlaInput::Stop { at: t(5 + 4) }).unwrap(),
        vec![SlaEvent::Missed]
    );
    assert_eq!(clock.state, ClockState::Stopped { met: false });
    assert_eq!(clock.counted, hours(9));
    assert_eq!(clock.warned_at, Some(t(3)));
    assert_eq!(clock.breached_at, Some(t(4)));
}

#[test]
fn a_late_tick_reports_both_in_order() {
    let mut clock = SlaClock::start(policy(4, 50), t(0)).unwrap();
    assert_eq!(
        clock.step(tick(t(10))).unwrap(),
        vec![
            SlaEvent::Warned { at: t(2) },
            SlaEvent::Breached { at: t(4) }
        ]
    );
}

#[test]
fn stopping_in_time_is_met() {
    let mut clock = SlaClock::start(policy(4, 0), t(0)).unwrap();
    assert_eq!(
        clock.step(SlaInput::Stop { at: t(4) }).unwrap(),
        vec![SlaEvent::Met]
    );
    assert_eq!(clock.state, ClockState::Stopped { met: true });
    assert_eq!(clock.due_at(), None);
    assert_eq!(
        clock.used(t(8)),
        hours(4),
        "a stopped clock keeps its count"
    );
}

#[test]
fn pauses_stop_the_count() {
    let mut clock = SlaClock::start(policy(4, 0), t(0)).unwrap();
    clock.step(SlaInput::Pause { at: t(1) }).unwrap();
    assert_eq!(clock.state, ClockState::Paused);
    assert_eq!(clock.due_at(), None);
    assert_eq!(clock.used(t(50)), hours(1));
    assert_eq!(
        clock.step(tick(t(50))).unwrap(),
        vec![],
        "no breach while paused"
    );
    clock.step(SlaInput::Resume { at: t(3) }).unwrap_err();
    let mut clock = SlaClock::start(policy(4, 0), t(0)).unwrap();
    clock.step(SlaInput::Pause { at: t(1) }).unwrap();
    clock.step(SlaInput::Resume { at: t(3) }).unwrap();
    assert_eq!(clock.due_at(), Some(t(6)));
    assert_eq!(clock.used(t(5)), hours(3));
    assert_eq!(
        clock.step(SlaInput::Stop { at: t(5) }).unwrap(),
        vec![SlaEvent::Met]
    );
}

#[test]
fn stopping_while_paused_keeps_the_count() {
    let mut clock = SlaClock::start(policy(4, 0), t(0)).unwrap();
    clock.step(SlaInput::Pause { at: t(5) }).unwrap();
    assert_eq!(
        clock.breached_at,
        Some(t(4)),
        "pausing late reports the breach first"
    );
    assert_eq!(
        clock.step(SlaInput::Stop { at: t(9) }).unwrap(),
        vec![SlaEvent::Missed]
    );
    assert_eq!(clock.counted, hours(5));
}

#[test]
fn deadlines_follow_the_business_calendar() {
    let policy = SlaPolicy {
        name: "First response".to_owned(),
        target: hours(8),
        warn_percent: 50,
        calendar: BusinessCalendar::office(9 * 60, 17 * 60, 0),
    };
    // Friday 16:00: 1 h on Friday, then Monday 09:00–16:00.
    let mut clock = SlaClock::start(policy, at(20, 16, 0)).unwrap();
    assert_eq!(clock.due_at(), Some(at(23, 16, 0)));
    assert_eq!(
        clock.step(tick(at(22, 12, 0))).unwrap(),
        vec![],
        "weekends do not count"
    );
    assert_eq!(
        clock.step(tick(at(23, 18, 0))).unwrap(),
        vec![
            SlaEvent::Warned { at: at(23, 12, 0) },
            SlaEvent::Breached { at: at(23, 16, 0) }
        ]
    );
}

#[test]
fn clocks_round_trip_through_json() {
    let mut clock = SlaClock::start(policy(4, 75), t(0)).unwrap();
    clock.step(SlaInput::Pause { at: t(1) }).unwrap();
    let json = serde_json::to_string(&clock).unwrap();
    let mut back: SlaClock = serde_json::from_str(&json).unwrap();
    assert_eq!(back, clock);
    back.step(SlaInput::Resume { at: t(2) }).unwrap();
    assert_eq!(back.due_at(), Some(t(5)));
}

// ---- invalid and anomalous ----

#[test]
fn invalid_policies_are_refused() {
    let invalid =
        |p: SlaPolicy| matches!(SlaClock::start(p, t(0)), Err(LifecycleError::Invalid(_)));
    let mut p = policy(4, 0);
    p.name = "  ".to_owned();
    assert!(invalid(p));
    let mut p = policy(4, 0);
    p.target = DurationMs::ZERO;
    assert!(invalid(p));
    assert!(invalid(policy(4, 100)));
    let mut p = policy(4, 0);
    p.calendar = BusinessCalendar::office(9, 8, 0);
    assert!(invalid(p));
}

#[test]
fn inputs_that_do_not_fit_the_state_are_refused_without_change() {
    let mut clock = SlaClock::start(policy(4, 0), t(0)).unwrap();
    assert!(matches!(
        clock.step(SlaInput::Resume { at: t(1) }),
        Err(LifecycleError::State(_))
    ));
    clock.step(SlaInput::Pause { at: t(1) }).unwrap();
    assert!(matches!(
        clock.step(SlaInput::Pause { at: t(2) }),
        Err(LifecycleError::State(_))
    ));
    clock.step(SlaInput::Stop { at: t(2) }).unwrap();
    let stopped = clock.clone();
    for input in [
        SlaInput::Pause { at: t(3) },
        SlaInput::Resume { at: t(3) },
        SlaInput::Stop { at: t(3) },
    ] {
        assert!(matches!(clock.step(input), Err(LifecycleError::State(_))));
        assert_eq!(clock, stopped);
    }
    assert_eq!(clock.step(tick(t(9))).unwrap(), vec![]);
}

#[test]
fn time_never_goes_backwards() {
    let mut clock = SlaClock::start(policy(4, 0), t(5)).unwrap();
    let before = clock.clone();
    let error = clock.step(tick(t(4))).unwrap_err();
    assert!(matches!(error, LifecycleError::Time(_)));
    assert!(error.to_string().starts_with("time went backwards"));
    assert_eq!(clock, before);
    // The same instant again is fine.
    assert_eq!(clock.step(tick(t(5))).unwrap(), vec![]);
}

// ---- model-based ----

#[derive(Debug, Clone, Copy)]
enum Kind {
    Pause,
    Resume,
    Stop,
    Tick,
}

/// The reference: elapsed time on a round-the-clock calendar, in plain integers.
#[derive(Debug, Default)]
struct Model {
    counted: i64,
    since: Option<i64>,
    stopped: bool,
    warned: bool,
    breached: bool,
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn the_clock_matches_the_reference(
        target_min in 1_i64..600,
        warn in 0_u8..100,
        steps in proptest::collection::vec((0_u8..4, 0_i64..240), 1..40),
    ) {
        let start = t(0).millis();
        let p = SlaPolicy {
            name: "Model".to_owned(),
            target: DurationMs::from_millis(target_min * MINUTE),
            warn_percent: warn,
            calendar: BusinessCalendar::always(),
        };
        let target = target_min * MINUTE;
        let warn_ms = target * i64::from(warn) / 100;
        let mut clock = SlaClock::start(p, t(0)).unwrap();
        let mut model = Model { since: Some(start), ..Model::default() };
        let mut now = start;
        for (kind, gap) in steps {
            now += gap * MINUTE;
            let kind = [Kind::Pause, Kind::Resume, Kind::Stop, Kind::Tick][usize::from(kind)];
            let at = Timestamp::from_millis(now).unwrap();
            let input = match kind {
                Kind::Pause => SlaInput::Pause { at },
                Kind::Resume => SlaInput::Resume { at },
                Kind::Stop => SlaInput::Stop { at },
                Kind::Tick => SlaInput::Tick { at },
            };
            // What the reference expects.
            let used = model.counted + model.since.map_or(0, |s| now - s);
            let mut expected = Vec::new();
            if let Some(since) = model.since {
                if warn > 0 && !model.warned && used >= warn_ms {
                    model.warned = true;
                    let when = since + (warn_ms - model.counted).max(0);
                    expected.push(SlaEvent::Warned { at: Timestamp::from_millis(when).unwrap() });
                }
                if !model.breached && used > target {
                    model.breached = true;
                    let when = since + (target - model.counted).max(0);
                    expected.push(SlaEvent::Breached { at: Timestamp::from_millis(when).unwrap() });
                }
            }
            let refused = match kind {
                Kind::Tick => false,
                Kind::Pause => model.stopped || model.since.is_none(),
                Kind::Resume => model.stopped || model.since.is_some(),
                Kind::Stop => model.stopped,
            };
            let result = clock.step(input);
            if refused {
                prop_assert!(matches!(result, Err(LifecycleError::State(_))), "{kind:?} at {now}");
                continue;
            }
            match kind {
                Kind::Tick => {}
                Kind::Pause => {
                    model.counted = used;
                    model.since = None;
                }
                Kind::Resume => model.since = Some(now),
                Kind::Stop => {
                    model.counted = used;
                    model.since = None;
                    model.stopped = true;
                    expected.push(if used <= target { SlaEvent::Met } else { SlaEvent::Missed });
                }
            }
            prop_assert_eq!(result.unwrap(), expected);
            prop_assert_eq!(clock.used(at).millis(), model.counted + model.since.map_or(0, |s| now - s));
            if let Some(since) = model.since {
                let due = since + (target - model.counted).max(0);
                prop_assert_eq!(clock.due_at(), Some(Timestamp::from_millis(due).unwrap()));
            }
        }
    }
}

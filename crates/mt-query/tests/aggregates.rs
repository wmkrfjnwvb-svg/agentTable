//! Aggregate arithmetic: every function on every applicable type, with the cases where a
//! wrong number would be worse than no number (mixed currencies, overflow, rounding).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_core::decimal::Decimal;
use mt_core::field::DataType;
use mt_core::value::Value;
use mt_query::{aggregate, summarize, AggregateFn, AggregateProblem, AggregateValue};

fn run(function: AggregateFn, data_type: DataType, values: &[Option<Value>]) -> AggregateValue {
    aggregate(function, data_type, values.iter().map(Option::as_ref))
}

fn value(function: AggregateFn, data_type: DataType, values: &[Option<Value>]) -> Option<Value> {
    let result = run(function, data_type, values);
    assert_eq!(result.problem, None, "{function}");
    result.value
}

fn some(values: Vec<Value>) -> Vec<Option<Value>> {
    values.into_iter().map(Some).collect()
}

fn percent(text: &str) -> Value {
    Value::Percent(dec(text))
}

use AggregateFn::{
    Average, CountChecked, CountEmpty, CountFilled, CountUnique, Max, Median, Min, PercentChecked,
    PercentEmpty, PercentFilled, Range, Sum,
};

#[test]
fn std_counting_functions() {
    let values = vec![
        Some(text("a")),
        None,
        Some(text("b")),
        Some(text("a")),
        Some(text("  ")),
        Some(Value::Null),
    ];
    let t = DataType::Text;
    assert_eq!(
        value(CountEmpty, t, &values),
        Some(num("3")),
        "missing, blank and null are all empty"
    );
    assert_eq!(value(CountFilled, t, &values), Some(num("3")));
    assert_eq!(value(CountUnique, t, &values), Some(num("2")));
    assert_eq!(value(PercentEmpty, t, &values), Some(percent("0.5")));
    assert_eq!(value(PercentFilled, t, &values), Some(percent("0.5")));
    // Thirds are rounded to six decimal places.
    let thirds = vec![Some(text("a")), None, None];
    assert_eq!(value(PercentFilled, t, &thirds), Some(percent("0.333333")));
    assert_eq!(value(PercentEmpty, t, &thirds), Some(percent("0.666667")));
    // No records: counts are zero, shares have no value.
    assert_eq!(value(CountEmpty, t, &[]), Some(num("0")));
    assert_eq!(value(CountFilled, t, &[]), Some(num("0")));
    assert_eq!(value(CountUnique, t, &[]), Some(num("0")));
    assert_eq!(value(PercentEmpty, t, &[]), None);
    assert_eq!(value(PercentFilled, t, &[]), None);
    // Distinct numbers ignore trailing zeros; distinct lists are distinct as written.
    let numbers = some(vec![num("1"), num("1.0"), num("1.00"), num("2")]);
    assert_eq!(
        value(CountUnique, DataType::Number, &numbers),
        Some(num("2"))
    );
}

#[test]
fn std_checkboxes() {
    let ticks = vec![
        Some(Value::Bool(true)),
        Some(Value::Bool(false)),
        None,
        Some(Value::Bool(true)),
    ];
    let t = DataType::Bool;
    assert_eq!(value(CountChecked, t, &ticks), Some(num("2")));
    // An empty checkbox is not ticked: 2 of 4.
    assert_eq!(value(PercentChecked, t, &ticks), Some(percent("0.5")));
    assert_eq!(
        value(CountFilled, t, &ticks),
        Some(num("3")),
        "false is a value"
    );
    assert_eq!(value(CountChecked, t, &[]), Some(num("0")));
    assert_eq!(value(PercentChecked, t, &[]), None);
}

#[test]
fn std_numbers() {
    let values = vec![
        Some(num("10")),
        None,
        Some(num("2.5")),
        Some(num("-4")),
        Some(num("7.25")),
    ];
    let t = DataType::Number;
    assert_eq!(value(Sum, t, &values), Some(num("15.75")));
    assert_eq!(
        value(Average, t, &values),
        Some(num("3.9375")),
        "the empty cell is not a zero"
    );
    assert_eq!(
        value(Median, t, &values),
        Some(num("4.875")),
        "mean of 2.5 and 7.25"
    );
    assert_eq!(value(Min, t, &values), Some(num("-4")));
    assert_eq!(value(Max, t, &values), Some(num("10")));
    assert_eq!(value(Range, t, &values), Some(num("14")));
    // Odd count: the middle value itself.
    let odd = some(vec![num("9"), num("1"), num("5")]);
    assert_eq!(value(Median, t, &odd), Some(num("5")));
    // One value.
    let one = some(vec![num("3.10")]);
    for function in [Sum, Average, Median, Min, Max] {
        assert_eq!(value(function, t, &one), Some(num("3.1")), "{function}");
    }
    assert_eq!(value(Range, t, &one), Some(num("0")));
    // Thirds keep the precision of exact division.
    let thirds = some(vec![num("1"), num("0"), num("0")]);
    assert_eq!(
        value(Average, t, &thirds),
        Some(num("0.3333333333333333333333333333"))
    );
    // No values.
    assert_eq!(value(Sum, t, &[None, None]), Some(num("0")));
    for function in [Average, Median, Min, Max, Range] {
        assert_eq!(value(function, t, &[None, None]), None, "{function}");
    }
}

#[test]
fn std_percent_and_duration() {
    let rates = some(vec![percent("0.25"), percent("0.5"), percent("0.75")]);
    let t = DataType::Percent;
    assert_eq!(value(Sum, t, &rates), Some(percent("1.5")));
    assert_eq!(value(Average, t, &rates), Some(percent("0.5")));
    assert_eq!(value(Median, t, &rates), Some(percent("0.5")));
    assert_eq!(value(Range, t, &rates), Some(percent("0.5")));
    assert_eq!(value(Sum, t, &[]), Some(percent("0")));

    let efforts = some(vec![minutes(90), minutes(30), minutes(45)]);
    let t = DataType::Duration;
    assert_eq!(value(Sum, t, &efforts), Some(minutes(165)));
    assert_eq!(value(Average, t, &efforts), Some(minutes(55)));
    assert_eq!(value(Median, t, &efforts), Some(minutes(45)));
    assert_eq!(value(Min, t, &efforts), Some(minutes(30)));
    assert_eq!(value(Range, t, &efforts), Some(minutes(60)));
    assert_eq!(value(Sum, t, &[]), Some(minutes(0)));
    // Averages of durations are rounded to whole milliseconds, half to even.
    let ms = |n: i64| Value::Duration(mt_core::time::DurationMs::from_millis(n));
    assert_eq!(
        value(Average, t, &some(vec![ms(1), ms(2)])),
        Some(ms(2)),
        "1.5 → 2"
    );
    assert_eq!(
        value(Average, t, &some(vec![ms(2), ms(3)])),
        Some(ms(2)),
        "2.5 → 2"
    );
    assert_eq!(
        value(Average, t, &some(vec![ms(1), ms(1), ms(2)])),
        Some(ms(1)),
        "1.33 → 1"
    );
}

#[test]
fn std_money_keeps_its_currency_and_rounds_to_minor_units() {
    let t = DataType::Money;
    let usd = some(vec![money("10.00", "USD"), money("10.01", "USD")]);
    assert_eq!(value(Sum, t, &usd), Some(money("20.01", "USD")));
    // 10.005 rounds half to even: 10.00.
    assert_eq!(value(Average, t, &usd), Some(money("10.00", "USD")));
    assert_eq!(value(Median, t, &usd), Some(money("10.00", "USD")));
    // 10.015 rounds half to even: 10.02.
    let next = some(vec![money("10.01", "USD"), money("10.02", "USD")]);
    assert_eq!(value(Average, t, &next), Some(money("10.02", "USD")));
    assert_eq!(value(Min, t, &usd), Some(money("10.00", "USD")));
    assert_eq!(value(Max, t, &usd), Some(money("10.01", "USD")));
    assert_eq!(value(Range, t, &usd), Some(money("0.01", "USD")));
    // Yen have no minor units: 100 and 101 average to 100 (100.5 → even).
    let jpy = some(vec![money("100", "JPY"), money("101", "JPY")]);
    assert_eq!(value(Average, t, &jpy), Some(money("100", "JPY")));
    assert_eq!(value(Sum, t, &jpy), Some(money("201", "JPY")));
    // Dinars have three.
    let kwd = some(vec![money("1.000", "KWD"), money("2.001", "KWD")]);
    assert_eq!(value(Average, t, &kwd), Some(money("1.500", "KWD")));
    // No amounts: no currency, so no total.
    assert_eq!(value(Sum, t, &[None]), None);
    assert_eq!(value(Average, t, &[]), None);
}

#[test]
fn anomalous_mixed_currencies_have_no_total() {
    let t = DataType::Money;
    let mixed = some(vec![
        money("10.00", "USD"),
        money("10.00", "EUR"),
        money("5.00", "USD"),
    ]);
    for function in [Sum, Average, Median, Min, Max, Range] {
        let result = run(function, t, &mixed);
        assert_eq!(result.value, None, "{function}");
        assert_eq!(
            result.problem,
            Some(AggregateProblem::MixedCurrencies),
            "{function}"
        );
    }
    // Counting still works.
    assert_eq!(value(CountFilled, t, &mixed), Some(num("3")));
    assert_eq!(value(CountUnique, t, &mixed), Some(num("3")));
}

#[test]
fn anomalous_overflow_is_reported() {
    let t = DataType::Number;
    let huge = some(vec![
        Value::Number(Decimal::MAX),
        Value::Number(Decimal::MAX),
    ]);
    for function in [Sum, Average, Median] {
        let result = run(function, t, &huge);
        assert_eq!(
            (result.value, result.problem),
            (None, Some(AggregateProblem::Overflow)),
            "{function}"
        );
    }
    let spread = some(vec![
        Value::Number(Decimal::MAX),
        Value::Number(Decimal::MIN),
    ]);
    assert_eq!(
        run(Range, t, &spread).problem,
        Some(AggregateProblem::Overflow)
    );
    // Min and max never overflow.
    assert_eq!(value(Max, t, &huge), Some(Value::Number(Decimal::MAX)));
    // Durations beyond the range of milliseconds.
    let long = some(vec![
        Value::Duration(mt_core::time::DurationMs::from_millis(i64::MAX)),
        minutes(1),
    ]);
    assert_eq!(
        run(Sum, DataType::Duration, &long).problem,
        Some(AggregateProblem::Overflow)
    );
}

#[test]
fn std_dates() {
    let days = vec![
        Some(date(2026, 5, 20)),
        None,
        Some(date(2026, 4, 15)),
        Some(date(2026, 6, 30)),
    ];
    let t = DataType::Date;
    assert_eq!(value(Min, t, &days), Some(date(2026, 4, 15)), "earliest");
    assert_eq!(value(Max, t, &days), Some(date(2026, 6, 30)), "latest");
    // The range of dates is a length of time: 76 days.
    assert_eq!(value(Range, t, &days), Some(minutes(76 * 24 * 60)));
    assert_eq!(value(Range, t, &[Some(date(2026, 1, 1))]), Some(minutes(0)));
    let instants = some(vec![instant(2026, 5, 1, 0), instant(2026, 5, 1, 13)]);
    let t = DataType::DateTime;
    assert_eq!(value(Min, t, &instants), Some(instant(2026, 5, 1, 0)));
    assert_eq!(value(Range, t, &instants), Some(minutes(13 * 60)));
    assert_eq!(value(Range, t, &[]), None);
}

#[test]
fn invalid_functions_that_do_not_apply_have_no_value() {
    // Which functions each data type accepts.
    let numeric = [
        DataType::Number,
        DataType::Percent,
        DataType::Money,
        DataType::Duration,
    ];
    let dated = [DataType::Date, DataType::DateTime];
    for data_type in DataType::ALL {
        for function in AggregateFn::ALL {
            let expected = match function {
                CountEmpty | CountFilled | PercentEmpty | PercentFilled => true,
                CountUnique => !matches!(data_type, DataType::Json | DataType::Attachment),
                CountChecked | PercentChecked => *data_type == DataType::Bool,
                Sum | Average | Median => numeric.contains(data_type),
                Min | Max | Range => numeric.contains(data_type) || dated.contains(data_type),
            };
            assert_eq!(
                function.applies_to(*data_type),
                expected,
                "{function} on {data_type}"
            );
            assert_eq!(function.to_string(), function.as_str());
            let json = serde_json::to_string(function).unwrap();
            assert_eq!(json, format!("\"{}\"", function.as_str()));
        }
    }
    let texts = some(vec![text("a"), text("b")]);
    assert_eq!(run(Sum, DataType::Text, &texts), AggregateValue::default());
    assert_eq!(
        run(CountChecked, DataType::Number, &some(vec![num("1")])),
        AggregateValue::default()
    );
    // Values of another type than the field's (corrupt data) give no number rather than a wrong one.
    assert_eq!(
        run(Sum, DataType::Number, &texts),
        AggregateValue::default()
    );
    assert_eq!(
        run(Average, DataType::Money, &some(vec![num("1")])),
        AggregateValue::default()
    );
    let result = run(Sum, DataType::Number, &some(vec![num("1")]));
    let json = serde_json::to_string(&result).unwrap();
    assert_eq!(
        serde_json::from_str::<AggregateValue>(&json).unwrap(),
        result
    );
}

#[test]
fn std_selection_summary() {
    // Numbers: counted and added up; empties are counted as cells only.
    let cells = [Some(num("10")), None, Some(num("2.5")), Some(Value::Null)];
    let summary = summarize(cells.iter().map(Option::as_ref));
    assert_eq!((summary.cells, summary.filled), (4, 2));
    let numbers = summary.numbers.clone().unwrap();
    assert_eq!(
        (numbers.sum, numbers.average, numbers.min, numbers.max),
        (num("12.5"), num("6.25"), num("2.5"), num("10"))
    );
    let json = serde_json::to_string(&summary).unwrap();
    assert_eq!(
        serde_json::from_str::<mt_query::SelectionSummary>(&json).unwrap(),
        summary
    );

    // One currency: added up. Two currencies, or numbers next to amounts: only counted.
    let usd = some(vec![money("1.50", "USD"), money("2.50", "USD")]);
    assert_eq!(
        summarize(usd.iter().map(Option::as_ref))
            .numbers
            .unwrap()
            .sum,
        money("4.00", "USD")
    );
    let mixed = some(vec![money("1.50", "USD"), money("2.50", "EUR")]);
    let summary = summarize(mixed.iter().map(Option::as_ref));
    assert_eq!(
        (summary.cells, summary.filled, summary.numbers),
        (2, 2, None)
    );
    let kinds = some(vec![num("1"), money("2.50", "EUR")]);
    assert_eq!(summarize(kinds.iter().map(Option::as_ref)).numbers, None);
    // Text and dates are counted.
    let other = some(vec![text("a"), date(2026, 1, 1)]);
    let summary = summarize(other.iter().map(Option::as_ref));
    assert_eq!(
        (summary.cells, summary.filled, summary.numbers),
        (2, 2, None)
    );
    // Nothing selected.
    let nothing: Vec<Option<Value>> = vec![];
    let summary = summarize(nothing.iter().map(Option::as_ref));
    assert_eq!(
        (summary.cells, summary.filled, summary.numbers),
        (0, 0, None)
    );
    // Durations and percentages are numeric too.
    let efforts = some(vec![minutes(30), minutes(90)]);
    assert_eq!(
        summarize(efforts.iter().map(Option::as_ref))
            .numbers
            .unwrap()
            .average,
        minutes(60)
    );
}

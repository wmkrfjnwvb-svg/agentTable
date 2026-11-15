//! Operators, the rules for combining kinds of values, logic, errors as values, results of
//! other computed fields, conversion to cell values and the evaluation bounds.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_core::field::DataType;
use mt_core::time::DurationMs;
use mt_core::value::Value;
use mt_formula::eval::{MAX_LIST_ITEMS, MAX_TEXT_CHARS};
use mt_formula::{
    evaluate, evaluate_to, evaluate_with, BinaryOp, Comparison, Computed, ErrorValue, Expr,
    Function, LogicFn, NumberFn, UnaryOp, Val,
};

#[test]
fn std_arithmetic_on_numbers() {
    check(&[
        ("1 + 2", "3"),
        ("{Qty} * 2", "6"),
        ("7 / 2", "3.5"),
        ("10 - 4 - 3", "3"),
        ("2 + 3 * 4", "14"),
        ("(2 + 3) * 4", "20"),
        ("-{Qty}", "-3"),
        ("--{Qty}", "3"),
        ("50%", "0.5"),
        ("{Qty}%", "0.03"),
        ("200 * 15%", "30"),
        ("0.1 + 0.2", "0.3"),
        ("ROUND(1 / 3, 4)", "0.3333"),
        ("1 / 3 * 3 = 1", "FALSE"),
        // Text written as a number is a number; true is 1.
        (r#""12.5" + 1"#, "13.5"),
        (r#"" 7 " * 2"#, "14"),
        ("TRUE + 1", "2"),
        ("FALSE * 5", "0"),
        // Empty counts as zero.
        ("{Empty number} + 1", "1"),
        ("{Empty number} * 5", "0"),
        ("{Empty number} - 1", "-1"),
        ("-{Empty number}", "0"),
        ("{Empty text} + 1", "1"),
    ]);
}

#[test]
fn invalid_arithmetic_gives_error_values() {
    check(&[
        ("{Qty} / 0", "#DIV/0!"),
        ("{Qty} / {Empty number}", "#DIV/0!"),
        ("0 / 0", "#DIV/0!"),
        (r#""abc" + 1"#, "#VALUE!"),
        (r#"1 - "abc""#, "#VALUE!"),
        (r#""abc" * 2"#, "#VALUE!"),
        (r#"2 / "abc""#, "#VALUE!"),
        (r#"-"abc""#, "#VALUE!"),
        (r#""abc"%"#, "#VALUE!"),
        ("{Status} + 1", "#VALUE!"),
        ("{Tags} + 1", "#VALUE!"),
        ("{Lines}.{Qty} + 1", "#VALUE!"),
        // Results that do not fit are errors, not wrong numbers.
        ("79228162514264337593543950335 + 1", "#NUM!"),
        ("-79228162514264337593543950335 - 1", "#NUM!"),
        ("79228162514264337593543950335 * 2", "#NUM!"),
        ("79228162514264337593543950335 / 0.1", "#NUM!"),
    ]);
}

#[test]
fn std_money_keeps_its_currency() {
    check(&[
        ("{Price} * {Qty}", "59.97 USD"),
        ("{Qty} * {Price}", "59.97 USD"),
        ("{Price} * {Qty} * (1 - {Discount})", "53.973 USD"),
        ("{Price} + {Shipping}", "24.49 USD"),
        ("{Price} - {Shipping}", "15.49 USD"),
        ("{Price} / 2", "9.995 USD"),
        ("ROUND({Price} / {Shipping}, 2)", "4.44"),
        ("-{Price}", "-19.99 USD"),
        ("{Price} * 20%", "3.998 USD"),
        ("{Price} + {Empty money}", "19.99 USD"),
        ("{Empty money} + {Price}", "19.99 USD"),
        ("{Price} - {Empty money}", "19.99 USD"),
        ("{Empty money} - {Price}", "-19.99 USD"),
        ("{Empty money} - {Empty money}", "0"),
        (r#"{Price} + PARAM("ship.flat")"#, "24.99 USD"),
        ("{Price} * {Empty number}", "0 USD"),
        // What cannot be meant.
        ("{Price} + 1", "#VALUE!"),
        ("1 - {Price}", "#VALUE!"),
        ("{Price} * {Price}", "#VALUE!"),
        ("2 / {Price}", "#VALUE!"),
        ("{Price} / 0", "#DIV/0!"),
        ("{Price} / {Empty money}", "#DIV/0!"),
        (r#"{Price} * "x""#, "#VALUE!"),
        ("{Price} * 79228162514264337593543950335", "#NUM!"),
        ("{Price} + {Due}", "#VALUE!"),
        // Different currencies never mix.
        (r#"{Price} + PARAM("ship.eur")"#, "#CURRENCY!"),
        (r#"{Price} - PARAM("ship.eur")"#, "#CURRENCY!"),
        (r#"{Price} / PARAM("ship.eur")"#, "#CURRENCY!"),
        (r#"{Price} > PARAM("ship.eur")"#, "#CURRENCY!"),
        (r#"{Price} = PARAM("ship.eur")"#, "#CURRENCY!"),
    ]);
}

#[test]
fn std_dates_times_and_durations() {
    check(&[
        ("{Due} + 7", "2026-05-27"),
        ("7 + {Due}", "2026-05-27"),
        ("{Due} - 20", "2026-04-30"),
        ("{Due} - TODAY()", "7"),
        ("TODAY() - {Due}", "-7"),
        ("{Due} + {Empty number}", "2026-05-20"),
        ("{Due} - {Empty number}", "2026-05-20"),
        ("{Empty number} + {Due}", "2026-05-20"),
        ("NOW() - {Created}", "1044900000ms"),
        ("{Created} - NOW()", "-1044900000ms"),
        ("{Created} + {Effort}", "2026-05-01T09:45:00.000Z"),
        ("{Effort} + {Created}", "2026-05-01T09:45:00.000Z"),
        ("{Created} - {Effort}", "2026-05-01T06:45:00.000Z"),
        ("{Created} + {Empty number}", "2026-05-01T08:15:00.000Z"),
        ("{Empty number} + {Created}", "2026-05-01T08:15:00.000Z"),
        ("{Created} - {Empty number}", "2026-05-01T08:15:00.000Z"),
        ("{Effort} + {Effort}", "10800000ms"),
        ("{Effort} - {Effort}", "0ms"),
        ("{Effort} + {Empty number}", "5400000ms"),
        ("{Empty number} + {Effort}", "5400000ms"),
        ("{Effort} - {Empty number}", "5400000ms"),
        ("{Empty number} - {Effort}", "-5400000ms"),
        ("-{Effort}", "-5400000ms"),
        ("{Effort} * 2", "10800000ms"),
        ("2 * {Effort}", "10800000ms"),
        ("{Effort} / 2", "2700000ms"),
        ("{Effort} / {Effort}", "1"),
        ("{Effort} / DURATION(1)", "1.5"),
        // Scaled durations round half-even to whole milliseconds.
        ("DURATION(0, 0, 0.001) * 2.5", "2ms"),
        ("DURATION(0, 0, 0.001) * 3.5", "4ms"),
        ("DURATION(0, 0, 0.005) / 2", "2ms"),
    ]);
}

#[test]
fn edge_calculating_with_an_empty_date_gives_nothing() {
    check(&[
        // An empty date is not day zero.
        ("{Empty date} + 7", "(blank)"),
        ("7 + {Empty date}", "(blank)"),
        ("{Empty date} - 7", "(blank)"),
        ("{Due} - {Empty date}", "(blank)"),
        ("{Empty date} - {Due}", "(blank)"),
        ("{Empty date} - {Empty date}", "(blank)"),
        ("TODAY() - {Empty date}", "(blank)"),
        // Also when the date comes out of a calculation.
        ("TODAY() - MAX({Empty date})", "(blank)"),
        ("{Due} - IF({Done}, TODAY())", "(blank)"),
        ("{Due} - IF({Done}, TODAY(), BLANK())", "(blank)"),
        ("({Empty date} + 7) - TODAY()", "(blank)"),
        ("{Due} - EDATE({Empty date}, 1)", "(blank)"),
        // So what follows is empty too, never a wrong number or a date shown as days.
        (r#""Days: " & ({Due} - {Empty date})"#, r#""Days: ""#),
        ("IF({Due} - {Empty date} > 30, 1, 2)", "2"),
        ("ISBLANK({Due} - {Empty date})", "TRUE"),
        (r#"IFBLANK({Due} - {Empty date}, "open")"#, r#""open""#),
        // An empty number of days is still zero days.
        ("{Due} + {Empty number}", "2026-05-20"),
        ("{Due} - {Empty number}", "2026-05-20"),
        // Errors come first.
        ("{Empty date} + 1 / 0", "#DIV/0!"),
        ("1 / 0 - {Empty date}", "#DIV/0!"),
        // Text and true/false count as numbers of days, like everywhere else.
        (r#"{Due} + "7""#, "2026-05-27"),
        ("{Due} + TRUE", "2026-05-21"),
        (r#"{Due} - "20""#, "2026-04-30"),
        (r#"{Due} + "soon""#, "#VALUE!"),
    ]);
    // The same for date-times: created is empty here.
    let mut record = order();
    record.cells.remove(&fid(CREATED));
    check_in(
        &[
            ("NOW() - {Created}", "(blank)"),
            ("{Created} - NOW()", "(blank)"),
            ("{Created} + {Effort}", "(blank)"),
            ("{Effort} + {Created}", "(blank)"),
            ("{Created} - {Effort}", "(blank)"),
            ("TO_HOURS(NOW() - {Created})", "(blank)"),
            ("YEAR({Created})", "(blank)"),
            (r#"TEXT({Created}, "hh:mm")"#, r#""""#),
        ],
        &record,
        &UTC,
    );
    // An empty amount or duration is zero of its kind.
    check(&[
        ("{Empty money} / {Price}", "0"),
        ("{Empty number} / {Effort}", "0"),
        ("{Empty money} / ({Price} - {Price})", "#DIV/0!"),
        ("{Empty number} / (DURATION(1) - DURATION(1))", "#DIV/0!"),
    ]);
}

#[test]
fn invalid_date_and_duration_arithmetic() {
    check(&[
        ("{Due} + 1.5", "#VALUE!"),
        ("{Due} - 0.5", "#VALUE!"),
        ("{Due} * 2", "#VALUE!"),
        ("{Due} / 2", "#VALUE!"),
        ("{Due} + {Due}", "#VALUE!"),
        ("5 - {Due}", "#VALUE!"),
        ("-{Due}", "#VALUE!"),
        ("{Due} + 9999999999", "#NUM!"),
        ("{Due} - 9999999999", "#NUM!"),
        ("{Due} + 99999999", "#NUM!"),
        ("{Due} - 99999999", "#NUM!"),
        ("{Created} + 1", "#VALUE!"),
        ("{Created} - {Due}", "#VALUE!"),
        ("{Created} + {Created}", "#VALUE!"),
        ("{Effort} - {Created}", "#VALUE!"),
        ("{Effort} * {Effort}", "#VALUE!"),
        ("{Effort} / 0", "#DIV/0!"),
        ("{Effort} / {Empty number}", "#DIV/0!"),
        ("{Effort} / (DURATION(1) - DURATION(1))", "#DIV/0!"),
        ("2 / {Effort}", "#VALUE!"),
        ("{Effort} + 1", "#VALUE!"),
        (r#"{Effort} * "x""#, "#VALUE!"),
        ("{Effort} * 79228162514264337593543950335", "#NUM!"),
        ("{Effort} / 0.0000000000000001", "#NUM!"),
        // Far beyond the supported range of instants.
        ("NOW() + DURATION(99999999999999)", "#NUM!"),
        ("NOW() - DURATION(99999999999999)", "#NUM!"),
        ("DURATION(2000000000000) + DURATION(2000000000000)", "#NUM!"),
    ]);
}

#[test]
fn std_joining_text() {
    check(&[
        (r#""a" & "b""#, r#""ab""#),
        (r#"{Title} & " x" & {Qty}"#, r#""Spring order x3""#),
        (r#""Total: " & {Price}"#, r#""Total: 19.99 USD""#),
        (r#""Total: " & {Price} * 3"#, r#""Total: 59.97 USD""#),
        (r#""Half: " & {Price} / 2"#, r#""Half: 10.00 USD""#),
        (r#""Due " & {Due}"#, r#""Due 2026-05-20""#),
        (r#"{Status} & """#, r#""Open""#),
        (r#"{Tags} & """#, r#""Open, Closed""#),
        (r#"{Empty number} & "x""#, r#""x""#),
        (r#"{Done} & """#, r#""FALSE""#),
        (r#""" & 1.50"#, r#""1.5""#),
        (r#""" & {Effort}"#, r#""PT1H30M""#),
        (r#""" & {Created}"#, r#""2026-05-01T08:15:00.000Z""#),
        (r#""" & {Discount}"#, r#""0.1""#),
        (r#""" & {Lines}.{Qty}"#, r#""1, 2, ""#),
        (r#""" & {Notes}"#, "\"Fragile.\nHandle with care\""),
        (r#"1 / 0 & "x""#, "#DIV/0!"),
        (r#""x" & 1 / 0"#, "#DIV/0!"),
    ]);
}

#[test]
fn std_comparisons() {
    check(&[
        ("{Qty} = 3", "TRUE"),
        ("{Qty} <> 3", "FALSE"),
        ("{Qty} > 2", "TRUE"),
        ("{Qty} >= 3", "TRUE"),
        ("{Qty} < 3", "FALSE"),
        ("{Qty} <= 3", "TRUE"),
        ("3.0 = 3.00", "TRUE"),
        // Text compares without regard to case.
        (r#""abc" = "ABC""#, "TRUE"),
        (r#""a" < "B""#, "TRUE"),
        (r#""b" > "A""#, "TRUE"),
        (r#""straße" = "STRASSE""#, "TRUE"),
        (r#"{Title} = "spring order""#, "TRUE"),
        (r#"{Title} <> "Spring order""#, "FALSE"),
        ("{Done} = FALSE", "TRUE"),
        ("TRUE > FALSE", "TRUE"),
        ("{Due} > TODAY()", "TRUE"),
        ("{Due} = DATE(2026, 5, 20)", "TRUE"),
        ("{Due} <= DATE(2026, 5, 20)", "TRUE"),
        ("{Created} < NOW()", "TRUE"),
        ("{Effort} > DURATION(1)", "TRUE"),
        ("{Effort} = DURATION(1, 30)", "TRUE"),
        ("{Price} > {Shipping}", "TRUE"),
        ("{Price} = {Price}", "TRUE"),
        ("{Price} <> {Shipping}", "TRUE"),
        // References are equal or not; they have no order.
        ("{Status} = {Status}", "TRUE"),
        ("{Status} <> {Status}", "FALSE"),
        ("{Status} = FIRST({Tags})", "TRUE"),
        ("{Status} = LAST({Tags})", "FALSE"),
    ]);
}

#[test]
fn edge_empty_values_in_comparisons() {
    check(&[
        // Empty equals zero, empty text and false.
        ("{Empty number} = 0", "TRUE"),
        (r#"{Empty number} = """#, "TRUE"),
        ("{Empty number} = FALSE", "TRUE"),
        ("{Empty number} = {Empty date}", "TRUE"),
        ("{Empty number} < 1", "TRUE"),
        ("{Empty number} > -1", "TRUE"),
        ("0 >= {Empty number}", "TRUE"),
        ("1 > {Empty number}", "TRUE"),
        (r#"{Empty number} < "a""#, "TRUE"),
        (r#""a" > {Empty number}"#, "TRUE"),
        ("{Empty number} < TRUE", "TRUE"),
        (r#"{Empty text} = """#, "TRUE"),
        ("{Empty number} <> 0", "FALSE"),
        // An empty date, amount, duration or reference never satisfies an ordering, so
        // "no due date" is not "overdue".
        ("{Empty date} < TODAY()", "FALSE"),
        ("{Empty date} > TODAY()", "FALSE"),
        ("{Empty date} <= TODAY()", "FALSE"),
        ("{Empty date} >= TODAY()", "FALSE"),
        ("TODAY() > {Empty date}", "FALSE"),
        ("TODAY() < {Empty date}", "FALSE"),
        ("{Empty date} = TODAY()", "FALSE"),
        ("{Empty date} <> TODAY()", "TRUE"),
        ("{Empty money} < {Price}", "FALSE"),
        ("{Empty money} > {Price}", "FALSE"),
        ("{Empty money} = {Price}", "FALSE"),
        ("{Empty number} < {Effort}", "FALSE"),
        ("{Empty number} < NOW()", "FALSE"),
        ("{Empty number} = {Status}", "FALSE"),
        ("{Empty number} < {Status}", "FALSE"),
    ]);
}

#[test]
fn invalid_comparisons_of_different_kinds() {
    check(&[
        // `=` is false and `<>` is true; an ordering has no answer.
        ("{Due} = 5", "FALSE"),
        ("{Due} <> 5", "TRUE"),
        ("{Due} > 5", "#VALUE!"),
        (r#"{Qty} = "3""#, "FALSE"),
        (r#"{Qty} < "5""#, "#VALUE!"),
        ("{Price} = 19.99", "FALSE"),
        ("{Price} > 5", "#VALUE!"),
        ("{Due} < {Created}", "#VALUE!"),
        ("{Done} < 1", "#VALUE!"),
        ("{Status} < {Status}", "#VALUE!"),
        ("{Status} >= {Status}", "#VALUE!"),
        // Lists are not compared as a whole.
        ("{Tags} = {Tags}", "#VALUE!"),
        ("{Lines}.{Qty} = 1", "#VALUE!"),
        ("1 < {Lines}.{Qty}", "#VALUE!"),
        ("{Empty number} = {Tags}", "#VALUE!"),
        // Errors go through.
        ("1 / 0 = 1", "#DIV/0!"),
        ("1 < 1 / 0", "#DIV/0!"),
        ("1 / 0 <> 1 / 0", "#DIV/0!"),
    ]);
}

#[test]
fn std_logic() {
    check(&[
        (r#"IF({Qty} > 2, "many", "few")"#, r#""many""#),
        (r#"IF({Qty} > 5, "many", "few")"#, r#""few""#),
        (r#"IF({Qty} > 5, "many")"#, "(blank)"),
        (r#"IF(1, "a", "b")"#, r#""a""#),
        (r#"IF(0, "a", "b")"#, r#""b""#),
        (r#"IF("true", "a", "b")"#, r#""a""#),
        (r#"IF(" False ", "a", "b")"#, r#""b""#),
        (r#"IF("", "a", "b")"#, r#""b""#),
        (r#"IF({Empty number}, "a", "b")"#, r#""b""#),
        (
            r#"IFS({Qty} > 5, "big", {Qty} > 2, "medium", TRUE, "small")"#,
            r#""medium""#,
        ),
        (r#"IFS({Qty} > 5, "big")"#, "#N/A"),
        (
            r#"SWITCH({Qty}, 1, "one", 3, "three", "other")"#,
            r#""three""#,
        ),
        (r#"SWITCH({Qty}, 1, "one", "other")"#, r#""other""#),
        (r#"SWITCH({Qty}, 1, "one", 2, "two")"#, "#N/A"),
        (r#"SWITCH({Title}, "spring ORDER", 1, 2)"#, "1"),
        (
            r#"SWITCH({Status}, {Status}, "same", "different")"#,
            r#""same""#,
        ),
        (r#"SWITCH({Empty number}, 0, "zero", "other")"#, r#""zero""#),
        ("AND(TRUE, {Qty} > 2)", "TRUE"),
        ("AND(TRUE, {Qty} > 5)", "FALSE"),
        ("AND({Qty})", "TRUE"),
        ("OR(FALSE, {Qty} > 2)", "TRUE"),
        ("OR(FALSE, FALSE)", "FALSE"),
        ("NOT({Done})", "TRUE"),
        ("NOT(1)", "FALSE"),
        ("NOT({Empty number})", "TRUE"),
        ("XOR(TRUE, TRUE)", "FALSE"),
        ("XOR(TRUE, FALSE, FALSE)", "TRUE"),
        ("XOR(TRUE, TRUE, TRUE)", "TRUE"),
        (r#"IFBLANK({Empty number}, "none")"#, r#""none""#),
        (r#"IFBLANK({Qty}, "none")"#, "3"),
        (r#"IFBLANK("  ", "none")"#, r#""none""#),
        (r#"IFBLANK({Empty text}, "none")"#, r#""none""#),
        ("ISBLANK({Empty date})", "TRUE"),
        ("ISBLANK({Qty})", "FALSE"),
        (r#"ISBLANK("")"#, "TRUE"),
        ("ISBLANK(0)", "FALSE"),
        ("ISBLANK(FALSE)", "FALSE"),
        ("ISBLANK(FILTER({Lines}.{Qty}, CurrentValue > 5))", "TRUE"),
        ("ISBLANK({Tags})", "FALSE"),
        ("ISNUMBER({Qty})", "TRUE"),
        (r#"ISNUMBER("3")"#, "FALSE"),
        ("ISNUMBER({Price})", "FALSE"),
        ("ISNUMBER({Empty number})", "FALSE"),
        ("ISTEXT({Title})", "TRUE"),
        (r#"ISTEXT("")"#, "FALSE"),
        ("ISTEXT(3)", "FALSE"),
        ("BLANK()", "(blank)"),
        (r#"BLANK() = """#, "TRUE"),
        ("ISBLANK(BLANK())", "TRUE"),
    ]);
}

#[test]
fn std_logic_evaluates_only_what_it_needs() {
    check(&[
        ("IF({Empty number} = 0, 0, 10 / {Empty number})", "0"),
        ("IF(TRUE, 1, 1 / 0)", "1"),
        ("IF(FALSE, 1 / 0, 2)", "2"),
        ("IF(FALSE, 1 / 0)", "(blank)"),
        ("IFS(TRUE, 1, 1 / 0, 2)", "1"),
        ("IFS(FALSE, 1 / 0, TRUE, 2)", "2"),
        (r#"SWITCH(1, 1, "a", 1 / 0, "b")"#, r#""a""#),
        (r#"SWITCH(1, 1, "a", 2, 1 / 0)"#, r#""a""#),
        (r#"SWITCH(1, 1, "a", 1 / 0)"#, r#""a""#),
        ("AND({Qty} > 5, 1 / 0)", "FALSE"),
        ("OR(TRUE, 1 / 0)", "TRUE"),
        ("IFERROR({Qty}, 1 / 0)", "3"),
        ("IFBLANK({Qty}, 1 / 0)", "3"),
    ]);
}

#[test]
fn anomalous_errors_are_values() {
    check(&[
        // They pass through operators and ordinary functions …
        ("1 / 0 + 1", "#DIV/0!"),
        ("-(1 / 0)", "#DIV/0!"),
        ("(1 / 0)%", "#DIV/0!"),
        ("SUM(1, 1 / 0)", "#DIV/0!"),
        ("LEN(1 / 0)", "#DIV/0!"),
        ("YEAR(1 / 0)", "#DIV/0!"),
        ("LABEL(1 / 0)", "#DIV/0!"),
        ("FIRST(1 / 0)", "#DIV/0!"),
        ("COUNTIF(1 / 0, 1)", "#DIV/0!"),
        ("COUNTIF({Lines}.{Qty}, 1 / 0)", "#DIV/0!"),
        ("SUMIF({Lines}.{Qty}, 1, 1 / 0)", "#DIV/0!"),
        ("FILTER(1 / 0, TRUE)", "#DIV/0!"),
        ("IF(1 / 0, 1, 2)", "#DIV/0!"),
        ("IFS(1 / 0, 1, TRUE, 2)", "#DIV/0!"),
        ("SWITCH(1 / 0, 1, 2)", "#DIV/0!"),
        (r#"SWITCH(2, 1 / 0, "a", "b")"#, "#DIV/0!"),
        ("AND(TRUE, 1 / 0)", "#DIV/0!"),
        ("OR(FALSE, 1 / 0)", "#DIV/0!"),
        ("NOT(1 / 0)", "#DIV/0!"),
        ("XOR(TRUE, 1 / 0)", "#DIV/0!"),
        ("IFBLANK(1 / 0, 1)", "#DIV/0!"),
        ("IFBLANK({Empty number}, 1 / 0)", "#DIV/0!"),
        // … the first error wins …
        (r#"1 / 0 + "abc" * 2"#, "#DIV/0!"),
        (r#""abc" * 2 + 1 / 0"#, "#VALUE!"),
        // … and they can be caught and asked about.
        (r#"IFERROR(1 / 0, "n/a")"#, r#""n/a""#),
        ("IFERROR({Qty} / {Empty number}, 0)", "0"),
        ("IFERROR(1 / 0, 2 / 0)", "#DIV/0!"),
        ("ISERROR(1 / 0)", "TRUE"),
        ("ISERROR({Qty})", "FALSE"),
        ("ISERROR({Empty number})", "FALSE"),
        ("ISBLANK(1 / 0)", "FALSE"),
        ("ISNUMBER(1 / 0)", "FALSE"),
        ("ISTEXT(1 / 0)", "FALSE"),
        // Conditions must be true or false.
        (r#"IF("yes", 1, 2)"#, "#VALUE!"),
        ("IF({Due}, 1, 2)", "#VALUE!"),
        ("IF({Tags}, 1, 2)", "#VALUE!"),
        ("AND({Price})", "#VALUE!"),
        ("NOT({Status})", "#VALUE!"),
        // There is no current item outside a list function.
        ("CurrentValue", "#VALUE!"),
        ("CurrentValue + 1", "#VALUE!"),
        ("ISERROR(CurrentValue)", "TRUE"),
    ]);
}

#[test]
fn std_later_formulas_see_earlier_results() {
    let record = order();
    let formula = compile("{Qty} * 2");
    let mut computed = Computed::new();
    assert_eq!(
        show(&evaluate_with(
            &formula,
            &Orders,
            &record,
            &UTC,
            Some(&computed)
        )),
        "6"
    );
    // A computed value replaces the stored one …
    computed.insert(fid(QTY), Ok(num("10")));
    assert_eq!(
        show(&evaluate_with(
            &formula,
            &Orders,
            &record,
            &UTC,
            Some(&computed)
        )),
        "20"
    );
    assert_eq!(show(&evaluate(&formula, &Orders, &record, &UTC)), "6");
    // … an empty result is empty …
    computed.insert(fid(QTY), Ok(Value::Null));
    assert_eq!(
        show(&evaluate_with(
            &formula,
            &Orders,
            &record,
            &UTC,
            Some(&computed)
        )),
        "0"
    );
    // … and an error flows on, where it can be caught.
    computed.insert(fid(QTY), Err(ErrorValue::Cycle));
    assert_eq!(
        show(&evaluate_with(
            &formula,
            &Orders,
            &record,
            &UTC,
            Some(&computed)
        )),
        "#CYCLE!"
    );
    let caught = compile("IFERROR({Qty}, 0) + 1");
    assert_eq!(
        show(&evaluate_with(
            &caught,
            &Orders,
            &record,
            &UTC,
            Some(&computed)
        )),
        "1"
    );
    // Fields that are not computed still come from the record.
    let other = compile("{Shipping}");
    assert_eq!(
        show(&evaluate_with(
            &other,
            &Orders,
            &record,
            &UTC,
            Some(&computed)
        )),
        "4.5 USD"
    );
    // A field the record does not have is empty.
    let empty = Rec::default();
    assert_eq!(
        show(&evaluate(&compile("{Qty}"), &Orders, &empty, &UTC)),
        "(blank)"
    );
    assert_eq!(
        show(&evaluate(&compile("{Lines}.{Qty}"), &Orders, &empty, &UTC)),
        "[]"
    );
    assert_eq!(
        show(&evaluate(
            &compile("SUM({Lines}.{Qty})"),
            &Orders,
            &empty,
            &UTC
        )),
        "0"
    );
}

#[test]
fn std_cell_values_from_other_kinds_of_fields() {
    check(&[
        // Links, people and attachments are lists of ids; they can be counted.
        ("COUNTA({Lines})", "3"),
        ("COUNTA({Owner})", "1"),
        ("COUNTA({Files})", "0"),
        ("ISBLANK({Files})", "TRUE"),
        ("ISBLANK({Owner})", "FALSE"),
        ("{Tags}", "[ref:1, ref:2]"),
        ("{Status}", "ref:1"),
        ("{Discount}", "0.1"),
        ("{Notes} = \"fragile.\nhandle with care\"", "TRUE"),
        ("{Extra}", "(blank)"),
    ]);
    let mut record = order();
    record
        .cells
        .insert(fid(EXTRA), Value::Json(serde_json::json!({"a": 1})));
    record
        .cells
        .insert(fid(TAGS), Value::MultiReference(vec![]));
    record.cells.insert(fid(OWNER), Value::Principal(vec![]));
    check_in(
        &[
            ("{Extra}", r#""{"a":1}""#),
            ("{Tags}", "[]"),
            ("ISBLANK({Tags})", "TRUE"),
            ("COUNTA({Owner})", "0"),
        ],
        &record,
        &UTC,
    );
}

fn to(source: &str, target: DataType) -> Result<Value, ErrorValue> {
    evaluate_to(&compile(source), &Orders, &order(), &UTC, None, target)
}

#[test]
fn std_results_become_cell_values() {
    // Amounts are rounded half-even to the currency's minor units, once, at the end.
    assert_eq!(
        to("{Price} * {Qty} * (1 - {Discount})", DataType::Money),
        Ok(money("53.97", "USD"))
    );
    assert_eq!(
        to("{Price} / 2", DataType::Money),
        Ok(money("10.00", "USD"))
    );
    assert_eq!(
        to("{Price} * 0.5 + {Price} * 0.5", DataType::Money),
        Ok(money("19.99", "USD"))
    );
    assert_eq!(
        to("{Shipping} / 8", DataType::Money),
        Ok(money("0.56", "USD"))
    );
    assert_eq!(to("{Qty} * 2", DataType::Number), Ok(num("6")));
    assert_eq!(to("1.50", DataType::Number), Ok(num("1.5")));
    assert_eq!(
        to("{Discount} * 2", DataType::Percent),
        Ok(Value::Percent(dec("0.2")))
    );
    assert_eq!(to("{Qty} > 1", DataType::Bool), Ok(Value::Bool(true)));
    assert_eq!(
        to("{Due} + 1", DataType::Date),
        Ok(Value::Date(day(2026, 5, 21)))
    );
    assert_eq!(
        to("NOW()", DataType::DateTime),
        Ok(Value::DateTime(at(2026, 5, 13, 10, 30)))
    );
    assert_eq!(
        to("{Effort} * 2", DataType::Duration),
        Ok(Value::Duration(DurationMs::from_millis(10_800_000)))
    );
    assert_eq!(
        to("{Status}", DataType::Reference),
        Ok(Value::Reference(item(1)))
    );
    assert_eq!(
        to("{Status}", DataType::MultiReference),
        Ok(Value::MultiReference(vec![item(1)]))
    );
    assert_eq!(
        to("{Tags}", DataType::MultiReference),
        Ok(Value::MultiReference(vec![item(1), item(2)]))
    );
    assert_eq!(
        to(
            "FILTER({Tags}, CurrentValue = {Status})",
            DataType::MultiReference
        ),
        Ok(Value::MultiReference(vec![item(1)]))
    );
    assert_eq!(
        to("UNIQUE({Lines}.{Labels})", DataType::MultiReference),
        Ok(Value::MultiReference(vec![item(1), item(2)]))
    );
    assert_eq!(
        to("FILTER({Tags}, FALSE)", DataType::MultiReference),
        Ok(Value::Null)
    );
    // Anything can be shown as text: references by their labels.
    assert_eq!(to("{Qty} * 2", DataType::Text), Ok(Value::Text("6".into())));
    assert_eq!(
        to("{Status}", DataType::Text),
        Ok(Value::Text("Open".into()))
    );
    assert_eq!(
        to("{Tags}", DataType::Text),
        Ok(Value::Text("Open, Closed".into()))
    );
    assert_eq!(
        to("{Price} / 2", DataType::Text),
        Ok(Value::Text("10.00 USD".into()))
    );
    assert_eq!(
        to("{Title}", DataType::LongText),
        Ok(Value::LongText("Spring order".into()))
    );
    assert_eq!(to(r#""  ""#, DataType::Text), Ok(Value::Null));
    // Nothing but empty amounts leaves no amount (there is no currency to state it in);
    // nothing but empty durations is a duration of zero.
    assert_eq!(to("{Empty money} * 2", DataType::Money), Ok(Value::Null));
    assert_eq!(
        to("{Empty money} + {Empty money}", DataType::Money),
        Ok(Value::Null)
    );
    assert_eq!(to("-{Empty money}", DataType::Money), Ok(Value::Null));
    assert_eq!(to("SUM({Empty money})", DataType::Money), Ok(Value::Null));
    assert_eq!(
        to("ROUND({Empty money}, 2)", DataType::Money),
        Ok(Value::Null)
    );
    assert_eq!(
        to("{Empty number} * 2", DataType::Duration),
        Ok(Value::Duration(DurationMs::from_millis(0)))
    );
    assert_eq!(to("{Empty date} + 7", DataType::Date), Ok(Value::Null));
    assert_eq!(
        to("{Due} - {Empty date}", DataType::Number),
        Ok(Value::Null)
    );
    // Empty stays empty, for every kind of field.
    for target in DataType::ALL.iter().copied() {
        assert_eq!(to("BLANK()", target), Ok(Value::Null), "{target:?}");
        assert_eq!(to("{Empty number}", target), Ok(Value::Null), "{target:?}");
        assert_eq!(to("1 / 0", target), Err(ErrorValue::DivZero), "{target:?}");
    }
}

#[test]
fn invalid_results_of_the_wrong_kind() {
    let wrong = Err(ErrorValue::Value);
    assert_eq!(to(r#""abc""#, DataType::Number), wrong);
    assert_eq!(to("{Due}", DataType::Number), wrong);
    assert_eq!(to("{Qty}", DataType::Bool), wrong);
    assert_eq!(to("{Qty}", DataType::Money), wrong);
    assert_eq!(to("{Qty} * 0.5", DataType::Duration), wrong);
    assert_eq!(to("{Price}", DataType::Number), wrong);
    assert_eq!(to("{Price}", DataType::Percent), wrong);
    assert_eq!(to("{Due}", DataType::DateTime), wrong);
    assert_eq!(to("{Created}", DataType::Date), wrong);
    assert_eq!(to("{Qty}", DataType::Duration), wrong);
    assert_eq!(to("{Qty}", DataType::Reference), wrong);
    assert_eq!(to("{Tags}", DataType::Reference), wrong);
    assert_eq!(to("{Lines}.{Qty}", DataType::MultiReference), wrong);
    assert_eq!(to("{Lines}.{Qty}", DataType::Number), wrong);
    assert_eq!(to("{Done}", DataType::Date), wrong);
    assert_eq!(to("{Effort}", DataType::Money), wrong);
    assert_eq!(to("{Title}", DataType::Bool), wrong);
    // Computed fields never produce links, people, attachments or structured data.
    for target in [
        DataType::Link,
        DataType::Principal,
        DataType::Attachment,
        DataType::Json,
    ] {
        assert_eq!(to("{Qty}", target), wrong, "{target:?}");
        assert_eq!(to("{Lines}", target), wrong, "{target:?}");
    }
    assert_eq!(
        to(r#"{Price} + PARAM("ship.eur")"#, DataType::Money),
        Err(ErrorValue::Currency)
    );
    assert_eq!(
        to(r#""x" & 1 / 0"#, DataType::Text),
        Err(ErrorValue::DivZero)
    );

    // Lists that are converted directly.
    let labels = |_| None;
    let list = Val::List(vec![Val::Ref(item(2)), Val::Blank, Val::Ref(item(1))]);
    assert_eq!(
        list.into_value(DataType::MultiReference, &labels),
        Ok(Value::MultiReference(vec![item(2), item(1)]))
    );
    let broken = Val::List(vec![Val::Ref(item(2)), Val::Error(ErrorValue::Num)]);
    assert_eq!(
        broken.clone().into_value(DataType::MultiReference, &labels),
        Err(ErrorValue::Num)
    );
    assert_eq!(
        broken.into_value(DataType::Text, &labels),
        Err(ErrorValue::Num)
    );
    // Without a label, a reference is shown by its id.
    assert_eq!(
        Val::Ref(item(2)).into_value(DataType::Text, &labels),
        Ok(Value::Text(item(2).to_string()))
    );
}

#[test]
fn std_value_conversions() {
    let labels = |_| None;
    // Numbers.
    assert_eq!(Val::Blank.number(), Ok(dec("0")));
    assert_eq!(Val::Bool(true).number(), Ok(dec("1")));
    assert_eq!(Val::Bool(false).number(), Ok(dec("0")));
    assert_eq!(Val::Text(" 2.50 ".into()).number(), Ok(dec("2.5")));
    assert_eq!(Val::Text("  ".into()).number(), Ok(dec("0")));
    assert_eq!(Val::Text("1e3".into()).number(), Err(ErrorValue::Value));
    assert_eq!(Val::Text("1,000".into()).number(), Err(ErrorValue::Value));
    assert_eq!(Val::Date(day(2026, 1, 1)).number(), Err(ErrorValue::Value));
    assert_eq!(Val::Error(ErrorValue::Ref).number(), Err(ErrorValue::Ref));
    assert_eq!(Val::Number(dec("7")).integer(), Ok(7));
    assert_eq!(Val::Number(dec("7.0")).integer(), Ok(7));
    assert_eq!(Val::Number(dec("7.5")).integer(), Err(ErrorValue::Value));
    assert_eq!(Val::Number(dec("-3")).integer(), Ok(-3));
    assert_eq!(
        Val::Number(dec("79228162514264337593543950335")).integer(),
        Err(ErrorValue::Value)
    );
    // True and false.
    assert_eq!(Val::Blank.boolean(), Ok(false));
    assert_eq!(Val::Number(dec("-1")).boolean(), Ok(true));
    assert_eq!(Val::Number(dec("0.0")).boolean(), Ok(false));
    assert_eq!(Val::Text("tRuE".into()).boolean(), Ok(true));
    assert_eq!(Val::Text("no".into()).boolean(), Err(ErrorValue::Value));
    assert_eq!(Val::Ref(item(1)).boolean(), Err(ErrorValue::Value));
    assert_eq!(Val::Error(ErrorValue::Num).boolean(), Err(ErrorValue::Num));
    // Text.
    assert_eq!(Val::Blank.text(&labels), Ok(String::new()));
    assert_eq!(Val::Bool(true).text(&labels), Ok("TRUE".into()));
    assert_eq!(Val::Number(dec("1.500")).text(&labels), Ok("1.5".into()));
    assert_eq!(
        Val::Error(ErrorValue::Num).text(&labels),
        Err(ErrorValue::Num)
    );
    // Emptiness and errors.
    assert!(Val::Blank.is_blank());
    assert!(Val::Text(" \n".into()).is_blank());
    assert!(Val::List(vec![]).is_blank());
    assert!(!Val::List(vec![Val::Blank]).is_blank());
    assert!(!Val::Number(dec("0")).is_blank());
    assert!(!Val::Error(ErrorValue::Num).is_blank());
    assert_eq!(Val::Error(ErrorValue::Num).error(), Some(ErrorValue::Num));
    assert_eq!(Val::Blank.error(), None);
    // Comparison results.
    use core::cmp::Ordering::{Equal, Greater, Less};
    assert_eq!(
        Val::Number(dec("1")).compare(&Val::Number(dec("2"))),
        Ok(Comparison::Ordered(Less))
    );
    assert_eq!(
        Val::Blank.compare(&Val::Blank),
        Ok(Comparison::Ordered(Equal))
    );
    assert_eq!(
        Val::Number(dec("1")).compare(&Val::Blank),
        Ok(Comparison::Ordered(Greater))
    );
    assert_eq!(
        Val::Blank.compare(&Val::Date(day(2026, 1, 1))),
        Ok(Comparison::Unknown)
    );
    assert_eq!(
        Val::Date(day(2026, 1, 1)).compare(&Val::Blank),
        Ok(Comparison::Unknown)
    );
    assert_eq!(
        Val::Ref(item(1)).compare(&Val::Ref(item(2))),
        Ok(Comparison::Unordered(false))
    );
    assert_eq!(
        Val::Ref(item(1)).compare(&Val::Number(dec("1"))),
        Ok(Comparison::Mismatch)
    );
    assert_eq!(
        Val::List(vec![]).compare(&Val::Blank),
        Err(ErrorValue::Value)
    );
    assert_eq!(
        Val::Blank.compare(&Val::Error(ErrorValue::Ref)),
        Err(ErrorValue::Ref)
    );
    // Every cell value has an evaluation value, and what is stored empty is blank.
    assert_eq!(Val::from_value(&Value::Null), Val::Blank);
    assert_eq!(Val::from_value(&Value::Text(String::new())), Val::Blank);
    assert_eq!(
        Val::from_value(&Value::Percent(dec("0.25"))),
        Val::Number(dec("0.25"))
    );
    assert_eq!(
        Val::from_value(&Value::LongText("a".into())),
        Val::Text("a".into())
    );
    assert_eq!(Val::from_value(&Value::Link(vec![])), Val::List(vec![]));
}

#[test]
fn anomalous_hand_built_trees_cannot_break_evaluation() {
    let record = order();
    // A wrong number of arguments (the parser would refuse it).
    for (function, count) in [
        (Function::Logic(LogicFn::If), 0),
        (Function::Logic(LogicFn::If), 5),
        (Function::Logic(LogicFn::Ifs), 3),
        (Function::Logic(LogicFn::Not), 0),
        (Function::Number(NumberFn::Round), 0),
        (Function::Number(NumberFn::Sum), 0),
    ] {
        let tree = Expr::Call {
            function,
            args: vec![Expr::Bool(true); count],
        };
        assert_eq!(
            show(&evaluate(&tree, &Orders, &record, &UTC)),
            "#VALUE!",
            "{function:?} with {count}"
        );
    }
    // Deeper than any parsed formula.
    let mut deep = Expr::Field(fid(QTY));
    for _ in 0..100 {
        deep = Expr::Unary {
            op: UnaryOp::Negate,
            operand: Box::new(deep),
        };
    }
    assert_eq!(show(&evaluate(&deep, &Orders, &record, &UTC)), "#LIMIT!");
    assert_eq!(
        evaluate_to(&deep, &Orders, &record, &UTC, None, DataType::Number),
        Err(ErrorValue::Limit)
    );
    // A field that no longer exists is `#REF!`, which can be caught like any error.
    let ghost = Expr::Binary {
        op: BinaryOp::Add,
        left: Box::new(Expr::Field(fid(999))),
        right: Box::new(Expr::Number(dec("1"))),
    };
    assert_eq!(show(&evaluate(&ghost, &Orders, &record, &UTC)), "#REF!");
    let ghost_link = Expr::Through {
        link: fid(998),
        field: fid(999),
    };
    assert_eq!(
        show(&evaluate(&ghost_link, &Orders, &record, &UTC)),
        "#REF!"
    );
    let ghost_linked_field = Expr::Through {
        link: fid(LINES),
        field: fid(999),
    };
    assert_eq!(
        show(&evaluate(&ghost_linked_field, &Orders, &record, &UTC)),
        "#REF!"
    );
    let caught = Expr::Call {
        function: Function::Logic(LogicFn::IfError),
        args: vec![ghost, Expr::Number(dec("0"))],
    };
    assert_eq!(show(&evaluate(&caught, &Orders, &record, &UTC)), "0");
}

/// A record with `count` linked lines whose quantities are 1, 2, 3, ….
fn with_lines(count: usize) -> Rec {
    let mut record = order();
    let quantities = (1..=count).map(|n| num(&n.to_string())).collect();
    record
        .linked
        .insert((fid(LINES), fid(LINE_QTY)), quantities);
    record
}

#[test]
fn edge_evaluation_is_bounded() {
    // Lists: at most MAX_LIST_ITEMS values of linked records.
    let full = with_lines(MAX_LIST_ITEMS);
    assert_eq!(run_in("COUNT({Lines}.{Qty})", &full, &UTC), "10000");
    assert_eq!(run_in("SUM({Lines}.{Qty})", &full, &UTC), "50005000");
    assert_eq!(
        run_in("COUNTIF({Lines}.{Qty}, CurrentValue > 9990)", &full, &UTC),
        "10"
    );
    let over = with_lines(MAX_LIST_ITEMS + 1);
    assert_eq!(run_in("COUNT({Lines}.{Qty})", &over, &UTC), "#LIMIT!");
    assert_eq!(run_in("IFERROR(SUM({Lines}.{Qty}), -1)", &over, &UTC), "-1");

    // Steps: a condition evaluated for every item of a long list adds up.
    let heavy = format!(
        "COUNTIF({{Lines}}.{{Qty}}, CurrentValue{} > 0)",
        " + 1".repeat(12)
    );
    assert_eq!(run_in(&heavy, &with_lines(1_000), &UTC), "1000");
    assert_eq!(run_in(&heavy, &full, &UTC), "#LIMIT!");
    let nested = "COUNTIF({Lines}.{Qty}, COUNTIF({Lines}.{Qty}, CurrentValue > 0) > 0)";
    assert_eq!(run_in(nested, &with_lines(100), &UTC), "100");
    assert_eq!(run_in(nested, &with_lines(1_000), &UTC), "#LIMIT!");

    // Texts: at most MAX_TEXT_CHARS characters.
    assert_eq!(run(r#"LEN(REPT("x", 100000))"#), "100000");
    assert_eq!(run(r#"REPT("x", 100001)"#), "#LIMIT!");
    assert_eq!(run(r#"REPT("ab", 50001)"#), "#LIMIT!");
    assert_eq!(run(r#"LEN(REPT("x", 100000) & "y")"#), "#LIMIT!");
    assert_eq!(
        run(r#"LEN(CONCAT(REPT("x", 60000), REPT("y", 40000)))"#),
        "100000"
    );
    assert_eq!(
        run(r#"CONCAT(REPT("x", 60000), REPT("y", 40001))"#),
        "#LIMIT!"
    );
    assert_eq!(
        run(r#"LEN(TEXTJOIN(REPT("-", 49998), FALSE, "a", "b", "c"))"#),
        "99999"
    );
    assert_eq!(
        run(r#"TEXTJOIN(REPT("-", 50000), FALSE, "a", "b", "c")"#),
        "#LIMIT!"
    );
    assert_eq!(
        run(r#"SUBSTITUTE(REPT("x", 100000), "x", "yy")"#),
        "#LIMIT!"
    );
    assert_eq!(run(r#"REPLACE(REPT("x", 100000), 1, 0, "y")"#), "#LIMIT!");
    assert_eq!(run(r#"UPPER(REPT("ß", 60000))"#), "#LIMIT!");
    // Characters are counted, not bytes.
    assert_eq!(run(r#"LEN(REPT("é", 100000))"#), "100000");
    assert_eq!(run(r#"LEN(REPT("é", 50000) & REPT("ü", 50000))"#), "100000");
    assert_eq!(MAX_TEXT_CHARS, 100_000);
}

//! The function library: numbers, text, `TEXT` formats, dates and working days, time
//! zones, lists of linked values, reference data and parameters.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_core::value::Value;

#[test]
fn std_totals_of_values() {
    check(&[
        ("SUM(1, 2, 3)", "6"),
        ("SUM({Lines}.{Amount})", "39.75 USD"),
        ("SUM({Lines}.{Qty})", "3"),
        ("SUM({Lines}.{Qty}, 10, {Qty})", "16"),
        ("SUM({Price}, {Shipping})", "24.49 USD"),
        ("SUM({Effort}, {Effort})", "10800000ms"),
        ("SUM({Lines}.{Qty}) * 2", "6"),
        ("AVERAGE(1, 2, 6)", "3"),
        ("AVERAGE({Lines}.{Amount})", "13.25 USD"),
        ("AVERAGE({Lines}.{Qty})", "1.5"),
        ("AVERAGE({Effort}, DURATION(0, 30))", "3600000ms"),
        (
            "AVERAGE(DURATION(0, 0, 0.001), DURATION(0, 0, 0.002))",
            "2ms",
        ),
        ("MEDIAN(3, 1, 2)", "2"),
        ("MEDIAN(4, 1, 3, 2)", "2.5"),
        ("MEDIAN(5)", "5"),
        ("MEDIAN({Lines}.{Amount})", "10 USD"),
        ("MEDIAN({Effort}, DURATION(1))", "4500000ms"),
        ("MIN(3, 1, 2)", "1"),
        ("MAX(3, 1, 2)", "3"),
        ("MIN({Lines}.{Amount})", "4.25 USD"),
        ("MAX({Lines}.{Amount})", "25.5 USD"),
        ("MIN({Lines}.{Shipped})", "2026-05-03"),
        ("MAX({Lines}.{Shipped})", "2026-05-10"),
        ("MAX({Due}, TODAY())", "2026-05-20"),
        ("MAX({Created}, NOW())", "2026-05-13T10:30:00.000Z"),
        ("MIN({Created}, NOW())", "2026-05-01T08:15:00.000Z"),
        ("MIN({Effort}, DURATION(1))", "3600000ms"),
        ("MAX({Effort}, DURATION(1))", "5400000ms"),
        ("COUNT({Lines}.{Qty})", "2"),
        ("COUNT({Lines}.{Name})", "0"),
        (
            r#"COUNT(1, "a", {Due}, {Price}, {Effort}, {Created}, TRUE)"#,
            "5",
        ),
        ("COUNT({Lines})", "0"),
        ("COUNTA({Lines}.{Qty})", "2"),
        ("COUNTA({Lines}.{Name})", "3"),
        (r#"COUNTA("", " ", 0)"#, "1"),
        ("COUNTA({Lines})", "3"),
    ]);
}

#[test]
fn edge_totals_skip_what_cannot_be_added() {
    check(&[
        // Empty values, text, true/false and references are skipped, as in a spreadsheet
        // range; nothing at all gives 0 for a sum and no value for the others.
        ("SUM({Lines}.{Name})", "0"),
        ("SUM({Empty number})", "0"),
        (r#"SUM("5", 1)"#, "1"),
        ("SUM(TRUE, 1)", "1"),
        ("SUM({Status}, 1)", "1"),
        ("AVERAGE({Empty number})", "(blank)"),
        ("AVERAGE({Lines}.{Name})", "(blank)"),
        ("MEDIAN({Empty number})", "(blank)"),
        ("MIN({Empty number})", "(blank)"),
        ("MAX({Lines}.{Name})", "(blank)"),
        ("COUNT({Empty number})", "0"),
        ("COUNTA({Empty number})", "0"),
        // An average ignores empty values instead of counting them as zero.
        ("AVERAGE({Lines}.{Qty}, {Empty number})", "1.5"),
    ]);
}

#[test]
fn invalid_totals_of_mixed_kinds() {
    check(&[
        ("SUM({Price}, 1)", "#VALUE!"),
        ("SUM(1, {Price})", "#VALUE!"),
        ("SUM({Effort}, 1)", "#VALUE!"),
        ("SUM({Price}, {Effort})", "#VALUE!"),
        (r#"SUM({Price}, PARAM("ship.eur"))"#, "#CURRENCY!"),
        (r#"MAX({Price}, PARAM("ship.eur"))"#, "#CURRENCY!"),
        // Dates have a smallest and a largest, but no sum or average.
        ("SUM({Due})", "#VALUE!"),
        ("SUM({Created})", "#VALUE!"),
        ("AVERAGE({Due})", "#VALUE!"),
        ("MEDIAN({Due})", "#VALUE!"),
        ("MAX({Due}, 5)", "#VALUE!"),
        ("MAX(5, {Due})", "#VALUE!"),
        ("MAX({Due}, {Created})", "#VALUE!"),
        ("MIN({Created}, {Due})", "#VALUE!"),
        ("SUM(79228162514264337593543950335, 1)", "#NUM!"),
        (
            "AVERAGE(79228162514264337593543950335, 79228162514264337593543950335)",
            "#NUM!",
        ),
    ]);
}

#[test]
fn std_rounding() {
    check(&[
        // ROUND rounds halves away from zero, like every spreadsheet.
        ("ROUND(2.5)", "3"),
        ("ROUND(-2.5)", "-3"),
        ("ROUND(2.4)", "2"),
        ("ROUND(2.345, 2)", "2.35"),
        ("ROUND(1.005, 2)", "1.01"),
        ("ROUND(1234.5, -2)", "1200"),
        ("ROUND(1250, -2)", "1300"),
        ("ROUND(-1250, -2)", "-1300"),
        ("ROUND(1.23456, 40)", "1.23456"),
        ("ROUND({Price}, 1)", "20 USD"),
        ("ROUND({Price} * {Qty} / 7, 2)", "8.57 USD"),
        ("ROUND({Empty number}, 2)", "0"),
        ("ROUNDUP(2.1)", "3"),
        ("ROUNDUP(-2.1)", "-3"),
        ("ROUNDUP(2.341, 2)", "2.35"),
        ("ROUNDUP(1201, -2)", "1300"),
        ("ROUNDUP(2)", "2"),
        ("ROUNDDOWN(2.9)", "2"),
        ("ROUNDDOWN(-2.9)", "-2"),
        ("ROUNDDOWN(2.349, 2)", "2.34"),
        ("ROUNDDOWN(1299, -2)", "1200"),
        ("ROUNDDOWN({Price})", "19 USD"),
        ("INT(2.9)", "2"),
        ("INT(-2.1)", "-3"),
        ("INT(5)", "5"),
        ("CEILING(2.1)", "3"),
        ("FLOOR(2.9)", "2"),
        ("CEILING(7, 5)", "10"),
        ("FLOOR(7, 5)", "5"),
        ("CEILING(-7, 5)", "-5"),
        ("FLOOR(-7, 5)", "-10"),
        ("CEILING(10, 5)", "10"),
        ("CEILING(1.01, 0.25)", "1.25"),
        ("CEILING(7, 0)", "0"),
        ("FLOOR(7, 0)", "0"),
        ("CEILING({Price}, 0.05)", "20 USD"),
        ("FLOOR({Price}, 0.5)", "19.5 USD"),
    ]);
}

#[test]
fn std_number_functions() {
    check(&[
        ("ABS(-3.5)", "3.5"),
        ("ABS(3.5)", "3.5"),
        ("ABS(-{Price})", "19.99 USD"),
        ("ABS({Empty number})", "0"),
        ("SIGN(-2)", "-1"),
        ("SIGN(0)", "0"),
        ("SIGN(0.001)", "1"),
        ("SIGN({Price})", "1"),
        ("SIGN(-{Price})", "-1"),
        ("SIGN({Empty number})", "0"),
        ("MOD(7, 3)", "1"),
        // The result has the sign of the divisor.
        ("MOD(-7, 3)", "2"),
        ("MOD(7, -3)", "-2"),
        ("MOD(7.5, 2)", "1.5"),
        ("MOD(6, 3)", "0"),
        ("POWER(2, 10)", "1024"),
        ("POWER(2, -2)", "0.25"),
        ("POWER(1.5, 2)", "2.25"),
        ("POWER(0, 0)", "1"),
        ("POWER(-2, 3)", "-8"),
        ("POWER(7, 1)", "7"),
        ("POWER(1, 10000)", "1"),
    ]);
}

#[test]
fn invalid_number_arguments() {
    check(&[
        (r#"ABS("x")"#, "#VALUE!"),
        ("ABS({Due})", "#VALUE!"),
        (r#"SIGN("x")"#, "#VALUE!"),
        ("ROUND(2.5, 1.5)", "#VALUE!"),
        (r#"ROUND("x")"#, "#VALUE!"),
        (r#"ROUND(1, "x")"#, "#VALUE!"),
        ("ROUND(1, -29)", "#NUM!"),
        ("ROUNDUP(1, 0.5)", "#VALUE!"),
        ("ROUNDDOWN(1, 0.5)", "#VALUE!"),
        ("ROUND(79228162514264337593543950335, -28)", "#NUM!"),
        ("INT({Price})", "#VALUE!"),
        ("MOD(7, 0)", "#DIV/0!"),
        ("MOD(7, {Empty number})", "#DIV/0!"),
        (r#"MOD("x", 2)"#, "#VALUE!"),
        ("MOD(79228162514264337593543950335, 0.1)", "#NUM!"),
        ("POWER(0, -1)", "#DIV/0!"),
        ("POWER(2, 0.5)", "#VALUE!"),
        ("POWER(10, 40)", "#NUM!"),
        ("POWER(2, 10001)", "#NUM!"),
        ("POWER(2, -10001)", "#NUM!"),
        (r#"POWER("x", 2)"#, "#VALUE!"),
        (r#"CEILING(1, "x")"#, "#VALUE!"),
        ("CEILING(79228162514264337593543950335, 0.1)", "#NUM!"),
        // The next multiple of 2 above the largest number does not exist.
        ("CEILING(79228162514264337593543950335, 2)", "#NUM!"),
    ]);
}

#[test]
fn std_text_functions() {
    check(&[
        (r#"CONCAT("a", 1, TRUE)"#, r#""a1TRUE""#),
        ("CONCAT({Lines}.{Name})", r#""BoltnutWasher""#),
        (r#"CONCAT({Status}, "-", {Tags})"#, r#""Open-OpenClosed""#),
        (r#"CONCAT({Empty number})"#, r#""""#),
        (
            r#"TEXTJOIN(", ", TRUE, {Lines}.{Name})"#,
            r#""Bolt, nut, Washer""#,
        ),
        (r#"TEXTJOIN("-", TRUE, {Lines}.{Qty})"#, r#""1-2""#),
        (r#"TEXTJOIN("-", FALSE, {Lines}.{Qty})"#, r#""1-2-""#),
        (r#"TEXTJOIN("/", TRUE, "a", "", "b")"#, r#""a/b""#),
        (r#"TEXTJOIN(", ", TRUE, {Tags})"#, r#""Open, Closed""#),
        (
            r#"TEXTJOIN(" + ", TRUE, {Lines}.{Amount})"#,
            r#""10.00 USD + 25.50 USD + 4.25 USD""#,
        ),
        (r#"TEXTJOIN("", TRUE, {Empty number})"#, r#""""#),
        ("LEN({Title})", "12"),
        (r#"LEN("")"#, "0"),
        (r#"LEN("héllo")"#, "5"),
        ("LEN({Qty})", "1"),
        ("LEN({Empty number})", "0"),
        ("LEFT({Title}, 6)", r#""Spring""#),
        ("LEFT({Title})", r#""S""#),
        ("RIGHT({Title}, 5)", r#""order""#),
        ("RIGHT({Title})", r#""r""#),
        (r#"LEFT("abc", 10)"#, r#""abc""#),
        (r#"RIGHT("abc", 10)"#, r#""abc""#),
        (r#"LEFT("abc", 0)"#, r#""""#),
        (r#"RIGHT("héllo", 4)"#, r#""éllo""#),
        ("LEFT(12345, 2)", r#""12""#),
        ("MID({Title}, 8, 3)", r#""ord""#),
        (r#"MID("abc", 5, 2)"#, r#""""#),
        (r#"MID("abc", 2, 100)"#, r#""bc""#),
        (r#"MID("abc", 2, 0)"#, r#""""#),
        ("UPPER({Title})", r#""SPRING ORDER""#),
        ("LOWER({Title})", r#""spring order""#),
        (r#"UPPER("straße")"#, r#""STRASSE""#),
        (r#"TRIM("  a   b  ")"#, r#""a b""#),
        ("TRIM({Empty text})", r#""""#),
        (r#"SUBSTITUTE("a-b-c", "-", "+")"#, r#""a+b+c""#),
        (r#"SUBSTITUTE("a-b-c", "-", "+", 2)"#, r#""a-b+c""#),
        (r#"SUBSTITUTE("a-b-c", "-", "+", 5)"#, r#""a-b-c""#),
        (r#"SUBSTITUTE("abc", "", "x")"#, r#""abc""#),
        (r#"SUBSTITUTE("abc", "b", "")"#, r#""ac""#),
        (r#"SUBSTITUTE("abc", "B", "x")"#, r#""abc""#),
        (r#"REPLACE("abcdef", 2, 3, "X")"#, r#""aXef""#),
        (r#"REPLACE("abc", 10, 1, "X")"#, r#""abcX""#),
        (r#"REPLACE("abc", 1, 0, "X")"#, r#""Xabc""#),
        (r#"REPLACE("héllo", 2, 1, "e")"#, r#""hello""#),
        (r#"FIND("o", {Title})"#, "8"),
        (r#"FIND("r", {Title})"#, "3"),
        (r#"FIND("r", {Title}, 4)"#, "9"),
        (r#"FIND("", "abc")"#, "1"),
        (r#"FIND("c", "abc", 3)"#, "3"),
        (r#"FIND("llo", "héllo")"#, "3"),
        (r#"SEARCH("O", {Title})"#, "8"),
        (r#"SEARCH("É", "café")"#, "4"),
        (r#"SEARCH("ORDER", {Title}, 2)"#, "8"),
        (r#"REPT("ab", 3)"#, r#""ababab""#),
        (r#"REPT("ab", 0)"#, r#""""#),
        (r#"VALUE("12.50")"#, "12.5"),
        (r#"VALUE("")"#, "0"),
        (r#"VALUE(" 7 ")"#, "7"),
        ("VALUE(TRUE)", "1"),
        ("VALUE(5)", "5"),
        (r#"EXACT("a", "A")"#, "FALSE"),
        (r#"EXACT("a", "a")"#, "TRUE"),
        (r#"EXACT(1, "1")"#, "TRUE"),
        (r#"CONTAINS({Title}, "ORDER")"#, "TRUE"),
        (r#"CONTAINS({Title}, "xyz")"#, "FALSE"),
        (r#"CONTAINS({Title}, "")"#, "FALSE"),
        ("CONTAINS({Title}, {Empty number})", "FALSE"),
        ("CONTAINS(12345, 34)", "TRUE"),
        // For a list, the question is whether an item equals the value.
        ("CONTAINS({Tags}, {Status})", "TRUE"),
        (r#"CONTAINS({Lines}.{Name}, "NUT")"#, "TRUE"),
        (r#"CONTAINS({Lines}.{Name}, "nu")"#, "FALSE"),
        ("CONTAINS({Lines}.{Qty}, 2)", "TRUE"),
        ("CONTAINS({Lines}.{Qty}, 5)", "FALSE"),
        ("CONTAINS({Lines}.{Amount}, 2)", "FALSE"),
    ]);
}

#[test]
fn invalid_text_arguments() {
    check(&[
        (r#"LEFT("abc", -1)"#, "#VALUE!"),
        (r#"RIGHT("abc", 1.5)"#, "#VALUE!"),
        (r#"MID("abc", 0, 2)"#, "#VALUE!"),
        (r#"MID("abc", 1, -1)"#, "#VALUE!"),
        (r#"MID("abc", "x", 1)"#, "#VALUE!"),
        (r#"SUBSTITUTE("a-b", "-", "+", 0)"#, "#VALUE!"),
        (r#"REPLACE("abc", 0, 1, "X")"#, "#VALUE!"),
        (r#"REPLACE("abc", 1, -1, "X")"#, "#VALUE!"),
        (r#"FIND("O", {Title})"#, "#VALUE!"),
        (r#"FIND("x", "abc")"#, "#VALUE!"),
        (r#"FIND("a", "abc", 0)"#, "#VALUE!"),
        (r#"FIND("c", "abc", 4)"#, "#VALUE!"),
        (r#"FIND("c", "abc", 9)"#, "#VALUE!"),
        (r#"SEARCH("x", "abc")"#, "#VALUE!"),
        (r#"REPT("ab", -1)"#, "#VALUE!"),
        (r#"VALUE("abc")"#, "#VALUE!"),
        ("VALUE({Due})", "#VALUE!"),
        (r#"TEXTJOIN("", "x", "a")"#, "#VALUE!"),
        (
            r#"CONTAINS({Lines}.{Amount}, PARAM("ship.eur"))"#,
            "#CURRENCY!",
        ),
        // Errors inside a list argument are found.
        ("CONCAT({Lines}.{Qty}, 1 / 0)", "#DIV/0!"),
    ]);
}

#[test]
fn std_text_of_numbers() {
    check(&[
        (r##"TEXT(1234.5, "#,##0.00")"##, r#""1,234.50""#),
        (r##"TEXT(1234567, "#,##0")"##, r#""1,234,567""#),
        (r##"TEXT(123, "#,##0")"##, r#""123""#),
        (r##"TEXT(1000, "#,##0")"##, r#""1,000""#),
        (r##"TEXT(-1234.567, "#,##0.0")"##, r#""-1,234.6""#),
        (r#"TEXT(0.256, "0%")"#, r#""26%""#),
        (r#"TEXT(0.256, "0.0%")"#, r#""25.6%""#),
        (r#"TEXT({Discount}, "0%")"#, r#""10%""#),
        (r#"TEXT(5, "000")"#, r#""005""#),
        (r#"TEXT(12345, "000")"#, r#""12345""#),
        (r#"TEXT(2.5, "0")"#, r#""3""#),
        (r#"TEXT(-2.5, "0")"#, r#""-3""#),
        (r#"TEXT(0.5, "0.00")"#, r#""0.50""#),
        (r#"TEXT(1.005, "0.00")"#, r#""1.01""#),
        (r##"TEXT(0.05, "#.00")"##, r#""0.05""#),
        (r##"TEXT(7, "#")"##, r#""7""#),
        // A value that rounds to zero has no sign.
        (r#"TEXT(-0.4, "0")"#, r#""0""#),
        (r#"TEXT(-0.004, "0.00")"#, r#""0.00""#),
        (r#"TEXT({Price}, "0.0")"#, r#""20.0""#),
        (r##"TEXT({Price} * {Qty}, "#,##0.00")"##, r#""59.97""#),
        (r#"TEXT({Empty number}, "0.00")"#, r#""""#),
        (
            r##""Total: " & TEXT({Price} * 1000, "#,##0.00") & " USD""##,
            r#""Total: 19,990.00 USD""#,
        ),
    ]);
}

#[test]
fn std_text_of_dates() {
    check(&[
        (r#"TEXT({Due}, "yyyy-mm-dd")"#, r#""2026-05-20""#),
        (r#"TEXT({Due}, "d mmm yyyy")"#, r#""20 May 2026""#),
        (
            r#"TEXT({Due}, "dddd, mmmm d, yy")"#,
            r#""Wednesday, May 20, 26""#,
        ),
        (r#"TEXT({Due}, "ddd")"#, r#""Wed""#),
        (r#"TEXT({Due}, "DD/MM/YYYY")"#, r#""20/05/2026""#),
        (r#"TEXT({Due}, "m/d/yy")"#, r#""5/20/26""#),
        (r#"TEXT(DATE(2026, 1, 5), "d.m.yyyy")"#, r#""5.1.2026""#),
        (r#"TEXT(DATE(2026, 9, 5), "mmm")"#, r#""Sep""#),
        (r#"TEXT(DATE(2026, 9, 5), "mmmmm")"#, r#""September""#),
        (r#"TEXT(DATE(2026, 9, 6), "ddddd")"#, r#""Sunday""#),
        (r#"TEXT({Due}, "yyyymmdd")"#, r#""20260520""#),
        (r#"TEXT(DATE(2026, 5, 18), "dddd")"#, r#""Monday""#),
        (r#"TEXT(DATE(2026, 5, 19), "dddd")"#, r#""Tuesday""#),
        (r#"TEXT(DATE(2026, 5, 20), "dddd")"#, r#""Wednesday""#),
        (r#"TEXT(DATE(2026, 5, 21), "dddd")"#, r#""Thursday""#),
        (r#"TEXT(DATE(2026, 5, 22), "dddd")"#, r#""Friday""#),
        (r#"TEXT(DATE(2026, 5, 23), "dddd")"#, r#""Saturday""#),
        (r#"TEXT(DATE(2026, 5, 24), "ddd")"#, r#""Sun""#),
        (r#"TEXT({Due}, "hh:mm")"#, r#""00:00""#),
        (
            r#"TEXT({Created}, "yyyy-mm-dd hh:mm:ss")"#,
            r#""2026-05-01 08:15:00""#,
        ),
        (r#"TEXT({Created}, "h:m")"#, r#""8:15""#),
        // `mm` next to hours or seconds is minutes; anywhere else it is the month.
        (r#"TEXT({Created}, "mm:ss")"#, r#""15:00""#),
        (r#"TEXT({Created}, "mm")"#, r#""05""#),
        (r#"TEXT({Created}, "hh mm, mm dd")"#, r#""08 15, 05 01""#),
        (
            r#"TEXT(NOW() + DURATION(0, 0, 7), "s") & TEXT(NOW() + DURATION(0, 0, 7), " ss")"#,
            r#""7 07""#,
        ),
        (r#"TEXT({Empty date}, "yyyy")"#, r#""""#),
        (
            r#"TEXT(DATE(5, 3, 4), "yyyy-mm-dd / yy")"#,
            r#""0005-03-04 / 05""#,
        ),
    ]);
}

#[test]
fn invalid_text_formats() {
    check(&[
        (r#"TEXT(1, "abc")"#, "#VALUE!"),
        (r#"TEXT(1, "")"#, "#VALUE!"),
        (r#"TEXT(1, "0.0.0")"#, "#VALUE!"),
        (r#"TEXT(1, "0.#")"#, "#VALUE!"),
        (r#"TEXT(1, ".00")"#, "#VALUE!"),
        (r#"TEXT(1, ",")"#, "#VALUE!"),
        (r#"TEXT(1, "%")"#, "#VALUE!"),
        (r#"TEXT(1, "0.00000000000000000000000000000")"#, "#VALUE!"),
        (r#"TEXT(1, "yyyy")"#, "#VALUE!"),
        (r#"TEXT({Due}, "0.00")"#, "#VALUE!"),
        (r#"TEXT({Due}, "!!")"#, "#VALUE!"),
        (r#"TEXT({Due}, "")"#, "#VALUE!"),
        (r#"TEXT("abc", "0")"#, "#VALUE!"),
        (r#"TEXT(TRUE, "0")"#, "#VALUE!"),
        (r#"TEXT({Effort}, "0")"#, "#VALUE!"),
        (r#"TEXT({Status}, "0")"#, "#VALUE!"),
        (r#"TEXT({Tags}, "0")"#, "#VALUE!"),
        (r#"TEXT(79228162514264337593543950335, "0%")"#, "#NUM!"),
    ]);
}

#[test]
fn std_parts_of_dates() {
    check(&[
        ("TODAY()", "2026-05-13"),
        ("NOW()", "2026-05-13T10:30:00.000Z"),
        ("DATE(2026, 2, 28)", "2026-02-28"),
        ("DATE(2024, 2, 29)", "2024-02-29"),
        ("YEAR({Due})", "2026"),
        ("MONTH({Due})", "5"),
        ("DAY({Due})", "20"),
        (r#"YEAR("2024-02-29")"#, "2024"),
        (r#"DAY(" 2024-02-29 ")"#, "29"),
        ("YEAR({Created})", "2026"),
        ("HOUR({Created})", "8"),
        ("MINUTE({Created})", "15"),
        ("HOUR({Due})", "0"),
        ("MINUTE({Due})", "0"),
        (r#"HOUR("2026-01-01")"#, "0"),
        // Sunday is 1 by default; with 2, Monday is 1.
        ("WEEKDAY({Due})", "4"),
        ("WEEKDAY({Due}, 2)", "3"),
        ("WEEKDAY(DATE(2026, 5, 17))", "1"),
        ("WEEKDAY(DATE(2026, 5, 17), 2)", "7"),
        ("WEEKDAY(DATE(2026, 5, 16))", "7"),
        ("WEEKDAY(DATE(2026, 5, 16), 2)", "6"),
        ("WEEKDAY(DATE(2026, 5, 18))", "2"),
        ("WEEKDAY(DATE(2026, 5, 18), 2)", "1"),
        // ISO weeks: the week with the year's first Thursday is week 1.
        ("ISOWEEKNUM({Due})", "21"),
        ("ISOWEEKNUM(DATE(2026, 1, 1))", "1"),
        ("ISOWEEKNUM(DATE(2027, 1, 1))", "53"),
        ("ISOWEEKNUM(DATE(2024, 12, 30))", "1"),
        ("ISOWEEKNUM(DATE(2023, 1, 1))", "52"),
        ("ISOWEEKNUM(DATE(2026, 12, 28))", "53"),
    ]);
}

#[test]
fn std_moving_and_measuring_dates() {
    check(&[
        (r#"DATEDIF(DATE(2026, 1, 31), DATE(2026, 3, 1), "D")"#, "29"),
        (r#"DATEDIF(DATE(2026, 1, 31), DATE(2026, 3, 1), "M")"#, "1"),
        (r#"DATEDIF(DATE(2026, 1, 31), DATE(2026, 3, 1), "Y")"#, "0"),
        (r#"DATEDIF(DATE(2026, 1, 15), DATE(2026, 3, 15), "M")"#, "2"),
        (r#"DATEDIF(DATE(2024, 2, 29), DATE(2026, 2, 28), "Y")"#, "1"),
        (r#"DATEDIF(DATE(2024, 2, 29), DATE(2026, 3, 1), "y")"#, "2"),
        (r#"DATEDIF(TODAY(), {Due}, " d ")"#, "7"),
        (r#"DATEDIF({Due}, {Due}, "D")"#, "0"),
        (r#"DATEDIF({Created}, {Due}, "D")"#, "19"),
        (r#"DATEADD({Due}, 1, "day")"#, "2026-05-21"),
        (r#"DATEADD({Due}, 2, "weeks")"#, "2026-06-03"),
        (r#"DATEADD({Due}, 1, "month")"#, "2026-06-20"),
        (r#"DATEADD({Due}, -1, "y")"#, "2025-05-20"),
        (r#"DATEADD({Due}, 0, "D")"#, "2026-05-20"),
        (r#"DATEADD({Due}, -3, " W ")"#, "2026-04-29"),
        // The end of a month stays in the month.
        (r#"DATEADD(DATE(2026, 1, 31), 1, "m")"#, "2026-02-28"),
        (r#"DATEADD(DATE(2024, 2, 29), 1, "year")"#, "2025-02-28"),
        (r#"DATEADD(DATE(2024, 2, 29), 4, "years")"#, "2028-02-29"),
        // A date-time keeps its time of day.
        (
            r#"DATEADD({Created}, 1, "day")"#,
            "2026-05-02T08:15:00.000Z",
        ),
        (
            r#"DATEADD({Created}, 1, "months")"#,
            "2026-06-01T08:15:00.000Z",
        ),
        (r#"DATEADD("2026-05-20", 1, "days")"#, "2026-05-21"),
        ("EDATE({Due}, 1)", "2026-06-20"),
        ("EDATE({Due}, -5)", "2025-12-20"),
        ("EDATE({Due}, 19)", "2027-12-20"),
        ("EDATE(DATE(2026, 3, 31), -1)", "2026-02-28"),
        ("EDATE({Created}, 1)", "2026-06-01"),
        ("EOMONTH({Due}, 0)", "2026-05-31"),
        ("EOMONTH({Due}, 1)", "2026-06-30"),
        ("EOMONTH({Due}, -5)", "2025-12-31"),
        ("EOMONTH(DATE(2024, 1, 15), 1)", "2024-02-29"),
        ("DAYS({Due}, TODAY())", "7"),
        ("DAYS(TODAY(), {Due})", "-7"),
        ("DAYS({Created}, {Due})", "-19"),
        ("DURATION(1, 30)", "5400000ms"),
        ("DURATION(1)", "3600000ms"),
        ("DURATION(0.5)", "1800000ms"),
        ("DURATION(0, 0, 1.5)", "1500ms"),
        ("DURATION(1, -30)", "1800000ms"),
        ("DURATION(0, 0, 0.0004)", "0ms"),
        ("DURATION({Empty number})", "0ms"),
        ("TO_HOURS({Effort})", "1.5"),
        ("TO_DAYS(DURATION(36))", "1.5"),
        ("TO_HOURS({Empty number})", "(blank)"),
        ("TO_HOURS(NOW() - {Created})", "290.25"),
    ]);
}

#[test]
fn edge_date_functions_without_a_date_return_nothing() {
    check(&[
        ("YEAR({Empty date})", "(blank)"),
        ("MONTH({Empty date})", "(blank)"),
        ("DAY({Empty date})", "(blank)"),
        ("HOUR({Empty date})", "(blank)"),
        ("MINUTE({Empty date})", "(blank)"),
        ("WEEKDAY({Empty date})", "(blank)"),
        ("ISOWEEKNUM({Empty date})", "(blank)"),
        (r#"DATEADD({Empty date}, 1, "month")"#, "(blank)"),
        ("EDATE({Empty date}, 6)", "(blank)"),
        ("EOMONTH({Empty date}, 0)", "(blank)"),
        ("WORKDAY({Empty date}, 5)", "(blank)"),
        (r#"DATEDIF({Empty date}, {Due}, "D")"#, "(blank)"),
        (r#"DATEDIF({Due}, {Empty date}, "D")"#, "(blank)"),
        ("DAYS({Due}, {Empty date})", "(blank)"),
        ("DAYS({Empty date}, {Due})", "(blank)"),
        ("NETWORKDAYS({Empty date}, {Due})", "(blank)"),
        ("NETWORKDAYS({Due}, {Empty date})", "(blank)"),
        // Blank text is no date either.
        (r#"YEAR("")"#, "(blank)"),
        ("YEAR({Empty text})", "(blank)"),
        // The other arguments are still looked at when there is a date.
        (r#"DATEADD({Due}, {Empty number}, "day")"#, "2026-05-20"),
        ("EDATE({Due}, {Empty number})", "2026-05-20"),
        // A whole formula over an optional date stays empty without any IF.
        (
            r#"IFBLANK(TEXT(EDATE({Empty date}, 6), "d mmm yyyy"), "not planned")"#,
            r#""not planned""#,
        ),
        ("YEAR({Empty date}) = 2026", "FALSE"),
    ]);
}

#[test]
fn std_working_days() {
    check(&[
        // Wednesday 13 May to Wednesday 20 May, both counted.
        ("NETWORKDAYS(TODAY(), {Due})", "6"),
        ("NETWORKDAYS({Due}, TODAY())", "-6"),
        ("NETWORKDAYS(TODAY(), TODAY())", "1"),
        // Friday to Tuesday over a weekend and the holiday on Monday 25 May.
        ("NETWORKDAYS(DATE(2026, 5, 22), DATE(2026, 5, 26))", "2"),
        ("NETWORKDAYS(DATE(2026, 5, 16), DATE(2026, 5, 17))", "0"),
        ("NETWORKDAYS(DATE(2026, 1, 1), DATE(2026, 12, 31))", "260"),
        ("WORKDAY(TODAY(), 1)", "2026-05-14"),
        ("WORKDAY(TODAY(), 0)", "2026-05-13"),
        ("WORKDAY(DATE(2026, 5, 15), 1)", "2026-05-18"),
        ("WORKDAY(DATE(2026, 5, 22), 1)", "2026-05-26"),
        ("WORKDAY(DATE(2026, 5, 18), -1)", "2026-05-15"),
        ("WORKDAY(DATE(2026, 5, 26), -1)", "2026-05-22"),
        ("WORKDAY(DATE(2026, 5, 16), 1)", "2026-05-18"),
        ("WORKDAY(TODAY(), 10)", "2026-05-28"),
        // The two functions agree with each other.
        ("NETWORKDAYS(TODAY(), WORKDAY(TODAY(), 10))", "11"),
        ("WORKDAY({Created}, 1)", "2026-05-04"),
    ]);
    // A calendar without working days gives an error instead of looping.
    check_in(
        &[
            ("WORKDAY(TODAY(), 1)", "#LIMIT!"),
            ("WORKDAY(TODAY(), -3)", "#LIMIT!"),
            ("WORKDAY(TODAY(), 0)", "2026-05-13"),
            ("NETWORKDAYS(TODAY(), {Due})", "0"),
        ],
        &order(),
        &IDLE,
    );
}

#[test]
fn invalid_date_arguments() {
    check(&[
        ("DATE(2026, 2, 30)", "#NUM!"),
        ("DATE(2026, 13, 1)", "#NUM!"),
        ("DATE(2026, -1, 1)", "#NUM!"),
        ("DATE(2026, 1, -1)", "#NUM!"),
        ("DATE(99999999999, 1, 1)", "#NUM!"),
        ("DATE(2026.5, 1, 1)", "#VALUE!"),
        (r#"DATE("x", 1, 1)"#, "#VALUE!"),
        (r#"YEAR("nope")"#, "#VALUE!"),
        (r#"YEAR("2026-02-30")"#, "#VALUE!"),
        ("YEAR(5)", "#VALUE!"),
        ("MONTH({Effort})", "#VALUE!"),
        ("DAY({Status})", "#VALUE!"),
        ("DAY({Tags})", "#VALUE!"),
        ("DAY(TRUE)", "#VALUE!"),
        ("DAY({Price})", "#VALUE!"),
        (r#"HOUR("x")"#, "#VALUE!"),
        ("MINUTE(5)", "#VALUE!"),
        ("WEEKDAY({Due}, 3)", "#NUM!"),
        ("WEEKDAY({Due}, 1.5)", "#VALUE!"),
        ("ISOWEEKNUM(5)", "#VALUE!"),
        (r#"DATEDIF({Due}, TODAY(), "D")"#, "#NUM!"),
        (r#"DATEDIF(TODAY(), {Due}, "X")"#, "#VALUE!"),
        (r#"DATEDIF(5, {Due}, "D")"#, "#VALUE!"),
        (r#"DATEDIF({Due}, 5, "D")"#, "#VALUE!"),
        (r#"DATEADD({Due}, 1, "hour")"#, "#VALUE!"),
        (r#"DATEADD({Due}, 1.5, "day")"#, "#VALUE!"),
        (r#"DATEADD(5, 1, "day")"#, "#VALUE!"),
        (r#"DATEADD({Due}, 99999999999, "day")"#, "#NUM!"),
        (r#"DATEADD({Due}, 9999999999999999, "weeks")"#, "#NUM!"),
        (r#"DATEADD({Due}, 9999999999999999, "years")"#, "#NUM!"),
        (r#"DATEADD({Due}, 9223372036854775807, "months")"#, "#NUM!"),
        (r#"DATEADD({Due}, 99999999, "years")"#, "#NUM!"),
        (r#"DATEADD({Created}, 99999999, "years")"#, "#NUM!"),
        ("EDATE({Due}, 0.5)", "#VALUE!"),
        ("EDATE(5, 1)", "#VALUE!"),
        ("EDATE({Due}, 999999999)", "#NUM!"),
        ("EOMONTH({Due}, 999999999)", "#NUM!"),
        ("EOMONTH(5, 1)", "#VALUE!"),
        ("DAYS(5, {Due})", "#VALUE!"),
        ("DAYS({Due}, 5)", "#VALUE!"),
        ("NETWORKDAYS(5, {Due})", "#VALUE!"),
        ("NETWORKDAYS({Due}, 5)", "#VALUE!"),
        ("NETWORKDAYS(DATE(1900, 1, 1), DATE(2026, 1, 1))", "#LIMIT!"),
        ("WORKDAY(5, 1)", "#VALUE!"),
        ("WORKDAY(TODAY(), 0.5)", "#VALUE!"),
        ("WORKDAY(TODAY(), 10001)", "#LIMIT!"),
        ("WORKDAY(TODAY(), -10001)", "#LIMIT!"),
        (r#"DURATION("x")"#, "#VALUE!"),
        (r#"DURATION(0, "x")"#, "#VALUE!"),
        (r#"DURATION(0, 0, "x")"#, "#VALUE!"),
        ("DURATION(79228162514264337593543950335)", "#NUM!"),
        ("DURATION(0, 79228162514264337593543950335)", "#NUM!"),
        ("DURATION(0, 0, 79228162514264337593543950335)", "#NUM!"),
        ("DURATION(99999999999999999999)", "#NUM!"),
        ("TO_HOURS(5)", "#VALUE!"),
        ("TO_DAYS({Due})", "#VALUE!"),
    ]);
}

#[test]
fn edge_days_follow_the_time_zone() {
    // 08:15 UTC is 17:15 in Tokyo and 01:15 in Los Angeles, on the same day.
    let record = order();
    check_in(
        &[
            ("HOUR({Created})", "17"),
            ("MINUTE({Created})", "15"),
            ("DAY({Created})", "1"),
            (
                r#"TEXT({Created}, "yyyy-mm-dd hh:mm")"#,
                r#""2026-05-01 17:15""#,
            ),
        ],
        &record,
        &TOKYO,
    );
    check_in(
        &[
            ("HOUR({Created})", "1"),
            ("DAY({Created})", "1"),
            (r#"TEXT({Created}, "hh:mm")"#, r#""01:15""#),
        ],
        &record,
        &LOS_ANGELES,
    );

    // 20:00 UTC on 30 April is already 1 May, 05:00, in Tokyo …
    let mut late = order();
    late.cells
        .insert(fid(CREATED), Value::DateTime(at(2026, 4, 30, 20, 0)));
    check_in(
        &[
            ("DAY({Created})", "1"),
            ("MONTH({Created})", "5"),
            ("HOUR({Created})", "5"),
            (r#"TEXT({Created}, "d mmm hh:mm")"#, r#""1 May 05:00""#),
            ("WEEKDAY({Created}, 2)", "5"),
            ("EOMONTH({Created}, 0)", "2026-05-31"),
            // Adding a month keeps the local time of day: 1 June, 05:00 in Tokyo.
            (
                r#"DATEADD({Created}, 1, "month")"#,
                "2026-05-31T20:00:00.000Z",
            ),
            ("DAYS({Due}, {Created})", "19"),
        ],
        &late,
        &TOKYO,
    );
    check_in(
        &[
            ("DAY({Created})", "30"),
            ("MONTH({Created})", "4"),
            ("HOUR({Created})", "20"),
        ],
        &late,
        &UTC,
    );

    // … and 03:00 UTC on 1 May is still 30 April, 20:00, in Los Angeles.
    let mut early = order();
    early
        .cells
        .insert(fid(CREATED), Value::DateTime(at(2026, 5, 1, 3, 0)));
    check_in(
        &[
            ("DAY({Created})", "30"),
            ("MONTH({Created})", "4"),
            ("HOUR({Created})", "20"),
            (
                r#"TEXT({Created}, "yyyy-mm-dd hh:mm")"#,
                r#""2026-04-30 20:00""#,
            ),
            (
                r#"DATEADD({Created}, 1, "day")"#,
                "2026-05-02T03:00:00.000Z",
            ),
            ("EDATE({Created}, 1)", "2026-05-30"),
        ],
        &early,
        &LOS_ANGELES,
    );
}

#[test]
fn std_values_of_linked_records() {
    check(&[
        ("{Lines}.{Qty}", "[1, 2, (blank)]"),
        ("{Lines}.{Name}", r#"["Bolt", "nut", "Washer"]"#),
        // A multi-value field of a linked record contributes its items.
        ("{Lines}.{Labels}", "[ref:1, ref:2, ref:1]"),
        ("COUNTIF({Lines}.{Qty}, CurrentValue > 1)", "1"),
        ("COUNTIF({Lines}.{Qty}, 2)", "1"),
        ("COUNTIF({Lines}.{Done}, TRUE)", "2"),
        ("COUNTIF({Lines}.{Labels}, {Status})", "2"),
        ("COUNTIF({Lines}.{Amount}, CurrentValue > {Shipping})", "2"),
        ("COUNTIF({Lines}.{Shipped}, CurrentValue < TODAY())", "2"),
        // A single value is a list of one, an empty value a list of none.
        ("COUNTIF({Qty}, 3)", "1"),
        ("COUNTIF({Empty number}, 0)", "0"),
        ("SUMIF({Lines}.{Qty}, CurrentValue > 0)", "3"),
        ("SUMIF({Lines}.{Done}, TRUE, {Lines}.{Amount})", "14.25 USD"),
        (
            "SUMIF({Lines}.{Qty}, CurrentValue >= 2, {Lines}.{Amount})",
            "25.5 USD",
        ),
        ("SUMIF({Lines}.{Qty}, CurrentValue > 5)", "0"),
        (
            "AVERAGEIF({Lines}.{Amount}, CurrentValue > {Shipping})",
            "17.75 USD",
        ),
        (
            "AVERAGEIF({Lines}.{Done}, FALSE, {Lines}.{Amount})",
            "25.5 USD",
        ),
        ("AVERAGEIF({Lines}.{Qty}, CurrentValue > 5)", "(blank)"),
        (
            r#"FILTER({Lines}.{Name}, CurrentValue <> "nut")"#,
            r#"["Bolt", "Washer"]"#,
        ),
        ("FILTER({Lines}.{Qty}, CurrentValue > 5)", "[]"),
        // A condition without `CurrentValue` is a value to equal.
        ("FILTER({Lines}.{Done}, TRUE)", "[TRUE, TRUE]"),
        ("FILTER({Lines}.{Qty}, TRUE)", "[]"),
        (
            "SUM(FILTER({Lines}.{Amount}, CurrentValue > {Shipping}))",
            "35.5 USD",
        ),
        (
            r#"TEXTJOIN(", ", TRUE, FILTER({Lines}.{Name}, LEN(CurrentValue) > 3))"#,
            r#""Bolt, Washer""#,
        ),
        ("UNIQUE({Lines}.{Labels})", "[ref:1, ref:2]"),
        ("UNIQUE({Lines}.{Qty})", "[1, 2]"),
        ("UNIQUE({Qty})", "[3]"),
        ("UNIQUE({Empty number})", "[]"),
        ("COUNTA(UNIQUE({Lines}.{Labels}))", "2"),
        ("FIRST({Lines}.{Name})", r#""Bolt""#),
        ("LAST({Lines}.{Name})", r#""Washer""#),
        ("FIRST({Empty number})", "(blank)"),
        ("LAST({Empty number})", "(blank)"),
        ("LAST({Qty})", "3"),
        ("INDEX({Lines}.{Name}, 2)", r#""nut""#),
        ("INDEX({Qty}, 1)", "3"),
        ("LAST({Lines}.{Qty})", "(blank)"),
    ]);
}

#[test]
fn std_conditions_written_as_text() {
    check(&[
        (r#"COUNTIF({Lines}.{Qty}, ">=1")"#, "2"),
        (r#"COUNTIF({Lines}.{Qty}, ">1")"#, "1"),
        (r#"COUNTIF({Lines}.{Qty}, "<2")"#, "2"),
        (r#"COUNTIF({Lines}.{Qty}, "<=2")"#, "3"),
        (r#"COUNTIF({Lines}.{Qty}, "=2")"#, "1"),
        (r#"COUNTIF({Lines}.{Qty}, "<>2")"#, "2"),
        (r#"COUNTIF({Lines}.{Qty}, "> 1")"#, "1"),
        (r#"COUNTIF({Lines}.{Name}, "bolt")"#, "1"),
        (r#"COUNTIF({Lines}.{Name}, "<>bolt")"#, "2"),
        (r#"COUNTIF({Lines}.{Name}, ">c")"#, "2"),
        // An amount is compared with a number by its amount.
        (r#"COUNTIF({Lines}.{Amount}, ">5")"#, "2"),
        (r#"SUMIF({Lines}.{Amount}, ">5")"#, "35.5 USD"),
        // Dates are written as YYYY-MM-DD; a line without a date matches neither side.
        (r#"COUNTIF({Lines}.{Shipped}, "<=2026-05-05")"#, "1"),
        (r#"COUNTIF({Lines}.{Shipped}, ">2026-05-05")"#, "1"),
        // Empty and not empty.
        (r#"COUNTIF({Lines}.{Qty}, "")"#, "1"),
        (r#"COUNTIF({Lines}.{Qty}, "=")"#, "1"),
        (r#"COUNTIF({Lines}.{Qty}, "<>")"#, "2"),
        (
            r#"SUMIF({Lines}.{Qty}, ">1", {Lines}.{Amount})"#,
            "25.5 USD",
        ),
        (r#"FILTER({Lines}.{Qty}, ">=2")"#, "[2]"),
        // Items of a kind that cannot be compared with the operand do not match.
        (r#"COUNTIF({Lines}.{Name}, ">5")"#, "0"),
        (r#"COUNTIF({Lines}.{Labels}, ">5")"#, "0"),
        (r#"COUNTIF({Lines}.{Labels}, "<>5")"#, "3"),
    ]);
}

#[test]
fn edge_the_current_item_belongs_to_the_innermost_list() {
    check(&[
        // The inner COUNTIF has its own current item; afterwards the outer one is back.
        ("COUNTIF({Lines}.{Qty}, COUNTIF({Lines}.{Qty}, CurrentValue >= 1) = 2)", "3"),
        (
            "FILTER({Lines}.{Qty}, COUNTIF({Lines}.{Amount}, CurrentValue > {Shipping}) = CurrentValue)",
            "[2]",
        ),
        // Also after the inner condition failed.
        (
            "FILTER({Lines}.{Qty}, IFERROR(COUNTIF({Lines}.{Name}, CurrentValue > 1) > 0, CurrentValue = 2))",
            "[2]",
        ),
        // Outside the list function there is no current item again.
        ("IFERROR(COUNTIF({Lines}.{Qty}, CurrentValue > 0) + CurrentValue, -1)", "-1"),
    ]);
    // Texts that are equal without regard to case are one item.
    let mut record = order();
    record.linked.insert(
        (fid(LINES), fid(LINE_NAME)),
        ["a", "A", "b", "", "B", "ß", "SS"]
            .into_iter()
            .map(|t| Value::Text(t.into()))
            .collect(),
    );
    record.linked.insert(
        (fid(LINES), fid(LINE_QTY)),
        vec![num("1"), num("1.0"), num("2"), Value::Null, num("1")],
    );
    record.linked.insert(
        (fid(LINES), fid(LINE_AMOUNT)),
        vec![money("1", "USD"), money("1", "EUR"), money("1.00", "USD")],
    );
    check_in(
        &[
            ("UNIQUE({Lines}.{Name})", r#"["a", "b", "ß"]"#),
            ("UNIQUE({Lines}.{Qty})", "[1, 2]"),
            ("UNIQUE({Lines}.{Amount})", "[1 USD, 1 EUR]"),
            ("COUNTA(UNIQUE({Lines}.{Done}))", "2"),
            ("COUNTA(UNIQUE({Lines}.{Shipped}))", "2"),
            ("UNIQUE(NOW())", "[2026-05-13T10:30:00.000Z]"),
            ("UNIQUE({Effort})", "[5400000ms]"),
            // Amounts in several currencies can be listed and counted, not added.
            ("SUM({Lines}.{Amount})", "#CURRENCY!"),
            ("COUNT({Lines}.{Amount})", "3"),
            (r#"COUNTIF({Lines}.{Amount}, ">0")"#, "3"),
        ],
        &record,
        &UTC,
    );
}

#[test]
fn invalid_list_arguments() {
    check(&[
        ("INDEX({Lines}.{Name}, 4)", "#N/A"),
        ("INDEX({Empty number}, 1)", "#N/A"),
        ("INDEX({Lines}.{Name}, 0)", "#VALUE!"),
        ("INDEX({Lines}.{Name}, -1)", "#VALUE!"),
        ("INDEX({Lines}.{Name}, 1.5)", "#VALUE!"),
        (r#"INDEX({Lines}.{Name}, "x")"#, "#VALUE!"),
        // A condition that cannot be decided for an item is an error, not "no".
        (r#"COUNTIF({Lines}.{Qty}, CurrentValue > "x")"#, "#VALUE!"),
        (r#"FILTER({Lines}.{Qty}, CurrentValue & "x")"#, "#VALUE!"),
        ("SUMIF({Lines}.{Name}, CurrentValue > 1)", "#VALUE!"),
        // The third argument must have one value per item.
        (r#"SUMIF({Lines}.{Qty}, ">0", {Tags})"#, "#VALUE!"),
        (r#"AVERAGEIF({Lines}.{Qty}, ">0", {Qty})"#, "#VALUE!"),
        (
            r#"SUMIF({Lines}.{Qty}, ">0", {Lines}.{Shipped})"#,
            "#VALUE!",
        ),
        (
            r#"COUNTIF({Lines}.{Amount}, PARAM("ship.eur"))"#,
            "#CURRENCY!",
        ),
        ("COUNTIF({Lines}.{Qty}, {Tags})", "#VALUE!"),
    ]);
}

#[test]
fn std_reference_data_and_parameters() {
    check(&[
        ("LABEL({Status})", r#""Open""#),
        ("CODE({Status})", r#""open""#),
        ("LABEL({Tags})", r#"["Open", "Closed"]"#),
        ("CODE({Tags})", r#"["open", "closed"]"#),
        (r#"ATTR({Status}, "weight")"#, "5"),
        (r#"ATTR({Status}, "colour")"#, r#""green""#),
        (r#"ATTR({Tags}, "weight")"#, "[5, 0.5]"),
        (r#"SUM(ATTR({Tags}, "weight"))"#, "5.5"),
        (r#"SUM(ATTR({Lines}.{Labels}, "weight"))"#, "10.5"),
        ("LABEL({Empty number})", "(blank)"),
        (r#"ATTR({Empty number}, "weight")"#, "(blank)"),
        (r#"PARAM("tax.rate")"#, "0.19"),
        (r#"PARAM("app.name")"#, r#""Demo""#),
        (r#"{Price} * (1 + PARAM("tax.rate"))"#, "23.7881 USD"),
        (r#"PARAM("app." & "name")"#, r#""Demo""#),
        // Unknown things are "not available", which can be caught.
        (r#"ATTR({Status}, "nope")"#, "#N/A"),
        (r#"ATTR({Tags}, "colour")"#, "#N/A"),
        (r#"PARAM("nope")"#, "#N/A"),
        (r#"IFERROR(PARAM("nope"), 0)"#, "0"),
        // Only references have labels.
        ("LABEL({Qty})", "#VALUE!"),
        ("CODE({Title})", "#VALUE!"),
        ("LABEL({Lines}.{Qty})", "#VALUE!"),
        (r#"ATTR({Due}, "weight")"#, "#VALUE!"),
        (r#"ATTR({Status}, 1 / 0)"#, "#DIV/0!"),
    ]);
    // An item that no longer exists.
    let mut record = order();
    record.cells.insert(fid(STATUS), Value::Reference(item(3)));
    let id = item(3).to_string();
    check_in(
        &[
            ("LABEL({Status})", "#N/A"),
            ("CODE({Status})", "#N/A"),
            (r#"IFERROR(LABEL({Status}), "(deleted)")"#, r#""(deleted)""#),
            (r#"{Status} & """#, &format!("\"{id}\"")),
        ],
        &record,
        &UTC,
    );
}

//! The type checker: the type of every kind of expression, what is refused when a formula
//! is written, and what fits a computed field.
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
use mt_formula::{
    check, check_for_field, ErrorValue, Expr, FormulaError, Function, LogicFn, NumberFn, Type,
    UnaryOp,
};

fn list(element: Type) -> Type {
    Type::List(Box::new(element))
}

fn typed(cases: &[(&str, Type)]) {
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|(source, expected)| {
            let found = check(&compile(source), &Orders);
            (found.as_ref() != Ok(expected))
                .then(|| format!("{source}\n     expected {expected:?}\n     found    {found:?}"))
        })
        .collect();
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

fn refused(cases: &[(&str, &str)]) {
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|(source, needle)| match check(&compile(source), &Orders) {
            Err(FormulaError::Type(message)) if message.contains(needle) => None,
            other => Some(format!(
                "{source}\n     expected a type error with `{needle}`\n     found    {other:?}"
            )),
        })
        .collect();
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

#[test]
fn std_types_of_values_and_fields() {
    typed(&[
        ("1", Type::Number),
        (r#""a""#, Type::Text),
        ("TRUE", Type::Bool),
        ("{Title}", Type::Text),
        ("{Notes}", Type::Text),
        ("{Extra}", Type::Text),
        ("{Qty}", Type::Number),
        ("{Discount}", Type::Number),
        ("{Price}", Type::Money),
        ("{Due}", Type::Date),
        ("{Created}", Type::DateTime),
        ("{Effort}", Type::Duration),
        ("{Done}", Type::Bool),
        ("{Status}", Type::Reference),
        ("{Tags}", list(Type::Reference)),
        ("{Lines}", list(Type::Text)),
        ("{Owner}", list(Type::Text)),
        ("{Files}", list(Type::Text)),
        ("{Lines}.{Amount}", list(Type::Money)),
        ("{Lines}.{Qty}", list(Type::Number)),
        // Lists are flat.
        ("{Lines}.{Labels}", list(Type::Reference)),
    ]);
    for data_type in DataType::ALL.iter().copied() {
        let found = Type::of_field(data_type);
        assert_eq!(found.is_list(), data_type.is_list(), "{data_type:?}");
        assert!(!found.element().is_list());
    }
    assert_eq!(list(Type::Money).element(), &Type::Money);
    assert_eq!(Type::Money.element(), &Type::Money);
    // The field type a result would naturally have.
    assert_eq!(Type::Number.data_type(), Some(DataType::Number));
    assert_eq!(Type::Text.data_type(), Some(DataType::Text));
    assert_eq!(Type::Bool.data_type(), Some(DataType::Bool));
    assert_eq!(Type::Date.data_type(), Some(DataType::Date));
    assert_eq!(Type::DateTime.data_type(), Some(DataType::DateTime));
    assert_eq!(Type::Duration.data_type(), Some(DataType::Duration));
    assert_eq!(Type::Money.data_type(), Some(DataType::Money));
    assert_eq!(Type::Reference.data_type(), Some(DataType::Reference));
    assert_eq!(
        list(Type::Reference).data_type(),
        Some(DataType::MultiReference)
    );
    assert_eq!(list(Type::Text).data_type(), None);
    assert_eq!(Type::Any.data_type(), None);
}

#[test]
fn std_types_of_arithmetic() {
    typed(&[
        ("{Qty} + 1", Type::Number),
        ("{Qty} - 1", Type::Number),
        ("{Qty} * 2", Type::Number),
        ("{Qty} / 2", Type::Number),
        (r#""5" + 1"#, Type::Number),
        ("TRUE + 1", Type::Number),
        ("-{Qty}", Type::Number),
        ("{Qty}%", Type::Number),
        ("{Price} * {Qty}", Type::Money),
        ("{Qty} * {Price}", Type::Money),
        ("{Price} + {Shipping}", Type::Money),
        ("{Price} - {Shipping}", Type::Money),
        ("{Price} / 2", Type::Money),
        ("{Price} / {Shipping}", Type::Number),
        ("-{Price}", Type::Money),
        ("{Price}%", Type::Money),
        ("{Due} + 7", Type::Date),
        ("7 + {Due}", Type::Date),
        ("{Due} - 7", Type::Date),
        ("{Due} - TODAY()", Type::Number),
        ("{Done} + {Due}", Type::Date),
        ("NOW() - {Created}", Type::Duration),
        ("{Created} + {Effort}", Type::DateTime),
        ("{Effort} + {Created}", Type::DateTime),
        ("{Created} - {Effort}", Type::DateTime),
        ("{Effort} + {Effort}", Type::Duration),
        ("{Effort} - {Effort}", Type::Duration),
        ("{Effort} * 2", Type::Duration),
        ("2 * {Effort}", Type::Duration),
        ("{Effort} / 2", Type::Duration),
        ("{Effort} / {Effort}", Type::Number),
        ("-{Effort}", Type::Duration),
        ("{Qty} & {Due}", Type::Text),
        (r#"{Tags} & """#, Type::Text),
    ]);
}

#[test]
fn edge_values_of_unknown_kind_fit_anywhere() {
    // PARAM, ATTR and BLANK() are not known until evaluation. Where the other operand
    // settles the result it is typed; where it does not, the result is unknown too.
    typed(&[
        (r#"PARAM("x")"#, Type::Any),
        ("BLANK()", Type::Any),
        (r#"PARAM("x") + PARAM("y")"#, Type::Any),
        (r#"PARAM("x") + 1"#, Type::Any),
        (r#"1 + PARAM("x")"#, Type::Any),
        (r#"{Price} + PARAM("x")"#, Type::Money),
        (r#"PARAM("x") + {Price}"#, Type::Money),
        (r#"{Due} + PARAM("x")"#, Type::Date),
        (r#"PARAM("x") + {Due}"#, Type::Date),
        (r#"{Created} + PARAM("x")"#, Type::DateTime),
        (r#"PARAM("x") + {Created}"#, Type::DateTime),
        (r#"{Effort} + PARAM("x")"#, Type::Any),
        (r#"PARAM("x") + {Effort}"#, Type::Any),
        (r#"PARAM("x") - PARAM("y")"#, Type::Any),
        (r#"PARAM("x") - {Due}"#, Type::Number),
        (r#"{Due} - PARAM("x")"#, Type::Any),
        (r#"PARAM("x") - {Created}"#, Type::Duration),
        (r#"{Created} - PARAM("x")"#, Type::Any),
        (r#"{Effort} - PARAM("x")"#, Type::Duration),
        (r#"PARAM("x") - {Effort}"#, Type::Any),
        (r#"1 - PARAM("x")"#, Type::Number),
        (r#"PARAM("x") - 1"#, Type::Any),
        (r#"{Price} - PARAM("x")"#, Type::Money),
        (r#"PARAM("x") - {Price}"#, Type::Money),
        (r#"PARAM("x") * PARAM("y")"#, Type::Any),
        (r#"PARAM("x") * 2"#, Type::Any),
        (r#"2 * PARAM("x")"#, Type::Any),
        (r#"{Price} * PARAM("x")"#, Type::Money),
        (r#"PARAM("x") * {Effort}"#, Type::Duration),
        (r#"PARAM("x") / PARAM("y")"#, Type::Any),
        (r#"PARAM("x") / {Price}"#, Type::Number),
        (r#"PARAM("x") / {Effort}"#, Type::Number),
        (r#"{Price} / PARAM("x")"#, Type::Any),
        (r#"{Effort} / PARAM("x")"#, Type::Any),
        (r#"1 / PARAM("x")"#, Type::Number),
        (r#"PARAM("x") / 2"#, Type::Any),
        (r#"-PARAM("x")"#, Type::Any),
        (r#"PARAM("x")%"#, Type::Any),
        (r#"PARAM("x") > 1"#, Type::Bool),
        (r#"PARAM("x") = {Status}"#, Type::Bool),
        (r#"SUM(PARAM("x"))"#, Type::Any),
        (r#"SUM(PARAM("x"), 1)"#, Type::Number),
        (r#"SUM(PARAM("x"), {Price})"#, Type::Money),
        (r#"ABS(PARAM("x"))"#, Type::Any),
        (r#"ROUND(PARAM("x"), 2)"#, Type::Any),
        (r#"TEXT(PARAM("x"), "0")"#, Type::Text),
        (r#"YEAR(PARAM("x"))"#, Type::Number),
        (r#"DATEADD(PARAM("x"), 1, "day")"#, Type::Any),
        (r#"TO_DAYS(PARAM("x"))"#, Type::Number),
        (r#"LABEL(PARAM("x"))"#, Type::Any),
        (r#"ATTR(PARAM("x"), "y")"#, Type::Any),
        (r#"IF(PARAM("x"), 1, 2)"#, Type::Number),
    ]);
}

#[test]
fn std_types_of_comparisons_and_logic() {
    typed(&[
        ("{Qty} > 1", Type::Bool),
        (r#"{Title} = "x""#, Type::Bool),
        ("{Due} < TODAY()", Type::Bool),
        ("{Status} = {Status}", Type::Bool),
        ("{Status} <> {Status}", Type::Bool),
        ("{Price} > {Shipping}", Type::Bool),
        ("{Effort} >= DURATION(1)", Type::Bool),
        // Asking whether there is a value, the spreadsheet way.
        (r#"{Due} = """#, Type::Bool),
        (r#""" <> {Price}"#, Type::Bool),
        (r#"{Status} = " ""#, Type::Bool),
        ("IF({Done}, 1, 2)", Type::Number),
        ("IF({Done}, 1)", Type::Number),
        ("IF({Qty}, {Due}, TODAY())", Type::Date),
        (r#"IF("true", {Price}, {Shipping})"#, Type::Money),
        // An empty alternative says nothing about the kind of the others.
        ("IF({Done}, BLANK(), {Due})", Type::Date),
        (
            "IF({Done}, {Due}, IF({Qty} > 1, BLANK(), BLANK()))",
            Type::Date,
        ),
        ("IFS({Done}, BLANK(), TRUE, {Price})", Type::Money),
        ("IF({Done}, BLANK(), BLANK())", Type::Any),
        ("IF({Done}, BLANK())", Type::Any),
        // An alternative of unknown kind makes the whole unknown.
        (r#"IF({Done}, IF({Qty} > 1, "a", 1), 2)"#, Type::Any),
        (r#"IF({Done}, PARAM("x"), 2)"#, Type::Any),
        (r#"IFERROR(PARAM("x"), 0)"#, Type::Any),
        (
            "IF({Done}, {Tags}, {Lines}.{Labels})",
            list(Type::Reference),
        ),
        // Alternatives of different kinds are allowed; the kind is then unknown.
        (r#"IF({Done}, "a", 2)"#, Type::Any),
        ("IF({Done}, {Tags}, {Lines}.{Qty})", list(Type::Any)),
        ("IF({Done}, {Tags}, {Status})", Type::Any),
        (r#"IFS({Qty} > 1, "a", TRUE, "b")"#, Type::Text),
        (r#"IFS({Qty} > 1, "a", TRUE, 2)"#, Type::Any),
        (r#"SWITCH({Qty}, 1, "a", 2, "b")"#, Type::Text),
        (r#"SWITCH({Qty}, 1, "a", 2, "b", "c")"#, Type::Text),
        (r#"SWITCH({Qty}, 1, "a", 2, "b", 3)"#, Type::Any),
        ("SWITCH({Qty}, 1, {Due}, {Due} + 1)", Type::Date),
        // The cases of a SWITCH are not results.
        (r#"SWITCH({Title}, "a", 1, "b", 2)"#, Type::Number),
        ("AND(TRUE, {Qty})", Type::Bool),
        (r#"OR({Done}, "true")"#, Type::Bool),
        ("NOT({Done})", Type::Bool),
        ("XOR({Done}, TRUE)", Type::Bool),
        ("IFERROR({Qty} / 0, 0)", Type::Number),
        (r#"IFERROR({Qty}, "x")"#, Type::Any),
        (r#"IFBLANK({Title}, "none")"#, Type::Text),
        ("ISBLANK({Due})", Type::Bool),
        ("ISERROR({Tags})", Type::Bool),
        ("ISNUMBER({Price})", Type::Bool),
        ("ISTEXT({Due})", Type::Bool),
    ]);
}

#[test]
fn std_types_of_functions() {
    typed(&[
        ("SUM({Lines}.{Amount})", Type::Money),
        ("SUM({Lines}.{Qty})", Type::Number),
        ("SUM({Effort}, {Effort})", Type::Duration),
        (r#"SUM(1, "a", TRUE, {Status})"#, Type::Number),
        ("SUM({Lines}.{Name})", Type::Number),
        ("AVERAGE({Lines}.{Amount})", Type::Money),
        ("MEDIAN({Qty})", Type::Number),
        ("MIN({Lines}.{Shipped})", Type::Date),
        ("MAX({Created}, NOW())", Type::DateTime),
        ("MAX({Lines}.{Qty})", Type::Number),
        ("COUNT({Lines})", Type::Number),
        ("COUNTA({Tags})", Type::Number),
        ("ABS({Price})", Type::Money),
        ("ABS({Qty})", Type::Number),
        ("ROUND({Price}, 2)", Type::Money),
        ("ROUND({Qty})", Type::Number),
        (r#"ROUNDUP("1.5")"#, Type::Number),
        ("ROUNDDOWN({Done})", Type::Number),
        ("CEILING({Price}, 0.05)", Type::Money),
        ("FLOOR(1.5)", Type::Number),
        ("SIGN({Price})", Type::Number),
        ("SIGN({Qty})", Type::Number),
        ("INT({Qty})", Type::Number),
        ("MOD({Qty}, 2)", Type::Number),
        ("POWER({Qty}, 2)", Type::Number),
        ("CONCAT({Lines}.{Name})", Type::Text),
        (r#"TEXTJOIN(", ", TRUE, {Tags})"#, Type::Text),
        ("LEN({Title})", Type::Number),
        ("LEFT({Title}, 2)", Type::Text),
        ("LEFT({Tags})", Type::Text),
        ("RIGHT({Title})", Type::Text),
        ("MID({Title}, 1, 2)", Type::Text),
        ("UPPER({Title})", Type::Text),
        ("LOWER({Qty})", Type::Text),
        ("TRIM({Title})", Type::Text),
        (r#"SUBSTITUTE({Title}, "a", "b", 1)"#, Type::Text),
        (r#"REPLACE({Title}, 1, 2, "x")"#, Type::Text),
        (r#"FIND("a", {Title}, 2)"#, Type::Number),
        (r#"SEARCH("a", {Title})"#, Type::Number),
        (r#"REPT("a", {Qty})"#, Type::Text),
        (r#"VALUE("5")"#, Type::Number),
        (r#"TEXT({Qty}, "0")"#, Type::Text),
        (r#"TEXT({Price}, "0.00")"#, Type::Text),
        (r#"TEXT({Due}, "yyyy")"#, Type::Text),
        (r#"TEXT({Created}, "hh:mm")"#, Type::Text),
        (r#"EXACT({Title}, "a")"#, Type::Bool),
        ("CONTAINS({Tags}, {Status})", Type::Bool),
        ("TODAY()", Type::Date),
        ("NOW()", Type::DateTime),
        ("DATE(2026, 1, 1)", Type::Date),
        ("YEAR({Due})", Type::Number),
        ("MONTH({Created})", Type::Number),
        (r#"DAY("2026-01-01")"#, Type::Number),
        ("HOUR({Created})", Type::Number),
        ("MINUTE({Created})", Type::Number),
        ("WEEKDAY({Due}, 2)", Type::Number),
        ("ISOWEEKNUM({Due})", Type::Number),
        (r#"DATEDIF({Created}, {Due}, "d")"#, Type::Number),
        (r#"DATEADD({Due}, 1, "day")"#, Type::Date),
        (r#"DATEADD({Created}, 1, "day")"#, Type::DateTime),
        (r#"DATEADD("2026-01-01", 1, " Days ")"#, Type::Date),
        // A unit that is not written out cannot be checked.
        ("DATEADD({Due}, 1, {Title})", Type::Date),
        ("EDATE({Due}, 1)", Type::Date),
        ("EOMONTH({Created}, 0)", Type::Date),
        ("WORKDAY({Due}, 5)", Type::Date),
        ("DAYS({Due}, {Created})", Type::Number),
        ("NETWORKDAYS({Due}, TODAY())", Type::Number),
        ("DURATION(1, 30)", Type::Duration),
        ("TO_HOURS({Effort})", Type::Number),
        ("TO_DAYS(NOW() - {Created})", Type::Number),
        ("COUNTIF({Lines}.{Qty}, CurrentValue > 1)", Type::Number),
        (
            r#"SUMIF({Lines}.{Qty}, ">1", {Lines}.{Amount})"#,
            Type::Money,
        ),
        (r#"SUMIF({Lines}.{Qty}, ">1")"#, Type::Number),
        (
            "AVERAGEIF({Lines}.{Amount}, CurrentValue > {Shipping})",
            Type::Money,
        ),
        (
            r#"FILTER({Lines}.{Name}, CurrentValue <> "x")"#,
            list(Type::Text),
        ),
        ("UNIQUE({Tags})", list(Type::Reference)),
        ("UNIQUE({Qty})", list(Type::Number)),
        ("FIRST({Lines}.{Amount})", Type::Money),
        ("LAST({Tags})", Type::Reference),
        ("FIRST({Qty})", Type::Number),
        ("INDEX({Lines}.{Name}, 1)", Type::Text),
        // The current item may be used anywhere inside the condition.
        (
            "COUNTIF({Lines}.{Qty}, IF(CurrentValue > 1, TRUE, FALSE))",
            Type::Number,
        ),
        (
            "FILTER({Lines}.{Qty}, SUM(CurrentValue, 1) > 2)",
            list(Type::Number),
        ),
        (
            "COUNTIF({Lines}.{Qty}, COUNTIF({Tags}, CurrentValue = {Status}) > CurrentValue)",
            Type::Number,
        ),
        ("LABEL({Status})", Type::Text),
        ("CODE({Status})", Type::Text),
        ("LABEL({Tags})", list(Type::Text)),
        (
            "LABEL(FILTER({Tags}, CurrentValue = {Status}))",
            list(Type::Text),
        ),
        (r#"ATTR({Status}, "w")"#, Type::Any),
        (r#"ATTR({Tags}, "w")"#, list(Type::Any)),
        (r#"LABEL(ATTR({Tags}, "parent"))"#, list(Type::Text)),
        (r#"SUM(ATTR({Tags}, "w"))"#, Type::Any),
    ]);
}

#[test]
fn invalid_operands() {
    refused(&[
        ("{Price} + 1", "`+` cannot combine an amount and a number"),
        ("1 - {Price}", "`-` cannot combine a number and an amount"),
        (
            "{Price} * {Price}",
            "`*` cannot combine an amount and an amount",
        ),
        ("2 / {Price}", "`/` cannot combine a number and an amount"),
        (
            "{Price} + {Effort}",
            "cannot combine an amount and a duration",
        ),
        (
            "{Price} * {Effort}",
            "cannot combine an amount and a duration",
        ),
        ("{Due} + {Due}", "cannot combine a date and a date"),
        ("{Due} * 2", "cannot combine a date and a number"),
        ("{Due} / 2", "cannot combine a date and a number"),
        ("1 - {Due}", "cannot combine a number and a date"),
        (
            "{Created} + 1",
            "cannot combine a date and time and a number",
        ),
        (
            "{Created} - {Due}",
            "cannot combine a date and time and a date",
        ),
        ("{Created} + {Created}", "cannot combine"),
        ("{Effort} + 1", "cannot combine a duration and a number"),
        (
            "{Effort} - {Created}",
            "cannot combine a duration and a date and time",
        ),
        (
            "{Effort} * {Effort}",
            "cannot combine a duration and a duration",
        ),
        ("2 / {Effort}", "cannot combine a number and a duration"),
        ("{Status} + 1", "cannot combine a reference and a number"),
        (
            "{Status} * {Status}",
            "cannot combine a reference and a reference",
        ),
        ("{Tags} + 1", "`+` cannot calculate with a list"),
        (
            "{Lines}.{Qty} * 2",
            "`*` cannot calculate with a list; use SUM",
        ),
        ("1 - {Lines}.{Qty}", "cannot calculate with a list"),
        ("-{Due}", "`-` cannot negate a date"),
        ("-{Tags}", "`-` cannot negate a list"),
        ("-{Status}", "cannot negate a reference"),
        ("{Status}%", "`%` cannot take a percentage of a reference"),
        ("{Created}%", "cannot take a percentage of a date and time"),
    ]);
}

#[test]
fn invalid_comparisons() {
    refused(&[
        (r#"{Qty} = "3""#, "`=` compares a number with text"),
        ("{Due} > 5", "`>` compares a date with a number"),
        ("{Price} > 5", "compares an amount with a number"),
        ("{Due} < {Created}", "compares a date with a date and time"),
        ("{Done} = 1", "compares true/false with a number"),
        ("{Status} = {Title}", "compares a reference with text"),
        (
            "{Status} < {Status}",
            "`<` cannot order references; compare their LABEL or CODE",
        ),
        ("{Status} >= {Status}", "cannot order references"),
        (r#"{Status} > PARAM("x")"#, "cannot order references"),
        // Only `=` and `<>` ask whether there is a value.
        (r#"{Qty} < """#, "compares a number with text"),
        (
            "{Tags} = {Status}",
            "`=` cannot compare a list; use CONTAINS",
        ),
        ("{Lines}.{Qty} > 1", "cannot compare a list"),
        (r#"{Tags} = """#, "cannot compare a list"),
    ]);
}

#[test]
fn invalid_use_of_the_current_item() {
    let message =
        "CurrentValue can only be used in the condition of FILTER, COUNTIF, SUMIF or AVERAGEIF";
    refused(&[
        ("CurrentValue + 1", message),
        ("IF(CurrentValue, 1, 2)", message),
        ("COUNTIF(CurrentValue, 1)", message),
        // The third argument of SUMIF is a list, not a condition.
        (r#"SUMIF({Lines}.{Qty}, ">1", CurrentValue)"#, message),
        ("SUM({Lines}.{Qty}, CurrentValue)", message),
    ]);
}

#[test]
fn invalid_arguments() {
    refused(&[
        (
            "IF({Due}, 1, 2)",
            "IF needs a condition as argument 1, not a date",
        ),
        (
            "IF({Tags}, 1, 2)",
            "IF needs a condition as argument 1, not a list",
        ),
        (
            "IFS({Qty} > 1, 1, {Price}, 2)",
            "IFS needs a condition as argument 3, not an amount",
        ),
        (
            "AND(TRUE, {Status})",
            "AND needs a condition as argument 2, not a reference",
        ),
        (
            "OR({Effort})",
            "OR needs a condition as argument 1, not a duration",
        ),
        (
            "NOT({Tags})",
            "NOT needs a condition as argument 1, not a list",
        ),
        (
            "XOR(TRUE, FALSE, {Created})",
            "XOR needs a condition as argument 3, not a date and time",
        ),
        ("SUM({Due})", "SUM cannot work with a date"),
        (
            "AVERAGE({Lines}.{Shipped})",
            "AVERAGE cannot work with a date",
        ),
        (
            "MEDIAN({Created})",
            "MEDIAN cannot work with a date and time",
        ),
        (
            "SUM({Price}, 1)",
            "SUM cannot combine an amount and plain numbers",
        ),
        (
            "SUM({Price}, {Effort})",
            "SUM cannot combine an amount and a duration",
        ),
        (
            "MAX({Due}, {Created})",
            "MAX cannot combine a date and a date and time",
        ),
        (
            "MIN({Due}, 5)",
            "MIN cannot combine a date and plain numbers",
        ),
        (
            r#"SUMIF({Lines}.{Qty}, ">1", {Lines}.{Shipped})"#,
            "SUMIF cannot work with a date",
        ),
        (
            "AVERAGEIF({Lines}.{Shipped}, TRUE)",
            "AVERAGEIF cannot work with a date",
        ),
        ("ABS({Due})", "ABS needs a number or an amount, not a date"),
        (
            "ROUND({Tags})",
            "ROUND needs a number or an amount, not a list",
        ),
        (
            "ROUND({Qty}, {Due})",
            "ROUND needs a number as argument 2, not a date",
        ),
        (
            "CEILING({Qty}, {Price})",
            "CEILING needs a number as argument 2, not an amount",
        ),
        (
            "FLOOR({Effort})",
            "FLOOR needs a number or an amount, not a duration",
        ),
        (
            "SIGN({Due})",
            "SIGN needs a number or an amount as argument 1, not a date",
        ),
        (
            "INT({Price})",
            "INT needs a number as argument 1, not an amount",
        ),
        (
            "MOD({Qty}, {Due})",
            "MOD needs a number as argument 2, not a date",
        ),
        ("MOD({Due}, 2)", "MOD needs a number as argument 1"),
        (
            "POWER({Due}, 2)",
            "POWER needs a number as argument 1, not a date",
        ),
        (
            "POWER(2, {Tags})",
            "POWER needs a number as argument 2, not a list",
        ),
        ("LEFT({Title}, {Due})", "LEFT needs a number as argument 2"),
        (
            "RIGHT({Title}, {Status})",
            "RIGHT needs a number as argument 2",
        ),
        ("MID({Title}, {Due}, 1)", "MID needs a number as argument 2"),
        ("MID({Title}, 1, {Due})", "MID needs a number as argument 3"),
        (
            r#"REPT("a", {Tags})"#,
            "REPT needs a number as argument 2, not a list",
        ),
        (
            r#"REPLACE("a", {Due}, 1, "b")"#,
            "REPLACE needs a number as argument 2",
        ),
        (
            r#"REPLACE("a", 1, {Due}, "b")"#,
            "REPLACE needs a number as argument 3",
        ),
        (
            r#"SUBSTITUTE("a", "b", "c", {Due})"#,
            "SUBSTITUTE needs a number as argument 4",
        ),
        (
            r#"FIND("a", "b", {Due})"#,
            "FIND needs a number as argument 3",
        ),
        (
            r#"SEARCH("a", "b", {Price})"#,
            "SEARCH needs a number as argument 3",
        ),
        (
            "VALUE({Due})",
            "VALUE needs a number as argument 1, not a date",
        ),
        (
            r#"TEXT({Title}, "0")"#,
            "TEXT formats numbers, amounts and dates, not text",
        ),
        (r#"TEXT({Effort}, "0")"#, "not a duration"),
        (r#"TEXT({Tags}, "0")"#, "not a list"),
        (r#"TEXT({Done}, "0")"#, "not true/false"),
        (r#"TEXT({Status}, "0")"#, "not a reference"),
        (
            "DATE({Due}, 1, 1)",
            "DATE needs a number as argument 1, not a date",
        ),
        ("DATE(2026, {Due}, 1)", "DATE needs a number as argument 2"),
        ("DATE(2026, 1, {Due})", "DATE needs a number as argument 3"),
        (
            "YEAR({Qty})",
            "YEAR needs a date as argument 1, not a number",
        ),
        (
            "MONTH({Price})",
            "MONTH needs a date as argument 1, not an amount",
        ),
        ("DAY({Tags})", "DAY needs a date as argument 1, not a list"),
        (
            "HOUR({Effort})",
            "HOUR needs a date as argument 1, not a duration",
        ),
        (
            "MINUTE({Done})",
            "MINUTE needs a date as argument 1, not true/false",
        ),
        ("WEEKDAY({Qty})", "WEEKDAY needs a date as argument 1"),
        (
            "WEEKDAY({Due}, {Due})",
            "WEEKDAY needs a number as argument 2",
        ),
        (
            "ISOWEEKNUM({Status})",
            "ISOWEEKNUM needs a date as argument 1, not a reference",
        ),
        (
            r#"DATEDIF({Qty}, {Due}, "d")"#,
            "DATEDIF needs a date as argument 1",
        ),
        (
            r#"DATEDIF({Due}, {Qty}, "d")"#,
            "DATEDIF needs a date as argument 2",
        ),
        (
            r#"DATEDIF({Due}, {Due}, "weeks")"#,
            r#"DATEDIF does not know the unit "weeks"; use one of: d, m, y"#,
        ),
        (
            r#"DATEADD({Due}, 1, "hour")"#,
            r#"DATEADD does not know the unit "hour""#,
        ),
        (
            r#"DATEADD({Qty}, 1, "day")"#,
            "DATEADD needs a date as argument 1, not a number",
        ),
        (
            r#"DATEADD({Tags}, 1, "day")"#,
            "DATEADD needs a date as argument 1, not a list",
        ),
        (
            r#"DATEADD({Due}, {Due}, "day")"#,
            "DATEADD needs a number as argument 2",
        ),
        ("EDATE({Qty}, 1)", "EDATE needs a date as argument 1"),
        (
            "EOMONTH({Due}, {Due})",
            "EOMONTH needs a number as argument 2",
        ),
        ("WORKDAY({Price}, 1)", "WORKDAY needs a date as argument 1"),
        ("DAYS({Qty}, {Due})", "DAYS needs a date as argument 1"),
        (
            "NETWORKDAYS({Due}, {Status})",
            "NETWORKDAYS needs a date as argument 2",
        ),
        ("DURATION({Due})", "DURATION needs a number as argument 1"),
        (
            "DURATION(1, {Due})",
            "DURATION needs a number as argument 2",
        ),
        (
            "DURATION(1, 2, {Due})",
            "DURATION needs a number as argument 3",
        ),
        (
            "TO_HOURS({Qty})",
            "TO_HOURS needs a duration as argument 1, not a number",
        ),
        (
            "TO_DAYS({Due})",
            "TO_DAYS needs a duration as argument 1, not a date",
        ),
        ("INDEX({Tags}, {Due})", "INDEX needs a number as argument 2"),
        ("LABEL({Qty})", "LABEL needs a reference, not a number"),
        ("CODE({Lines}.{Qty})", "CODE needs a reference, not a list"),
        (r#"ATTR({Title}, "x")"#, "ATTR needs a reference, not text"),
        (
            r#"ATTR({Lines}.{Qty}, "x")"#,
            "ATTR needs a reference, not a list",
        ),
        (r#"PARAM("")"#, "PARAM needs the key of a parameter"),
        (r#"PARAM("  ")"#, "PARAM needs the key of a parameter"),
    ]);
}

#[test]
fn anomalous_trees_that_did_not_come_from_the_parser() {
    let unknown = |name: String| Err(FormulaError::UnknownField { name, position: 0 });
    // A field that was deleted after the formula was written.
    assert_eq!(
        check(&Expr::Field(fid(999)), &Orders),
        unknown(fid(999).to_string())
    );
    let through = |link: u128, field: u128| Expr::Through {
        link: fid(link),
        field: fid(field),
    };
    assert_eq!(
        check(&through(999, LINE_QTY), &Orders),
        unknown(fid(999).to_string())
    );
    assert_eq!(
        check(&through(LINES, 999), &Orders),
        unknown(format!("{}.{}", fid(LINES), fid(999)))
    );
    // A field that is no longer a link.
    match check(&through(QTY, LINE_QTY), &Orders) {
        Err(FormulaError::Type(message)) => {
            assert!(
                message.contains("only a link field leads to another table"),
                "{message}"
            );
            assert!(message.ends_with("is a number field"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    // Wrong argument counts and sizes.
    let call = |function: Function, count: usize| Expr::Call {
        function,
        args: vec![Expr::Bool(true); count],
    };
    assert!(matches!(
        check(&call(Function::Logic(LogicFn::If), 1), &Orders),
        Err(FormulaError::Arity {
            function: "IF",
            found: 1,
            ..
        })
    ));
    assert_eq!(
        check(&call(Function::Number(NumberFn::Sum), 2_001), &Orders),
        Err(FormulaError::Limit("more than 2,000 parts"))
    );
    let mut deep = Expr::Field(fid(QTY));
    for _ in 0..70 {
        deep = Expr::Unary {
            op: UnaryOp::Negate,
            operand: Box::new(deep),
        };
    }
    assert_eq!(
        check(&deep, &Orders),
        Err(FormulaError::Limit("nested more than 64 levels deep"))
    );
    assert_eq!(
        check_for_field(&deep, &Orders, DataType::Number),
        Err(FormulaError::Limit("nested more than 64 levels deep"))
    );
    // Errors inside arguments are found wherever they are.
    refused(&[
        (
            "IF({Done}, {Price} + 1, 2)",
            "cannot combine an amount and a number",
        ),
        (
            "SUM({Qty}, LEN({Due} * 2))",
            "cannot combine a date and a number",
        ),
        (
            "COUNTIF({Lines}.{Qty}, CurrentValue > {Tags})",
            "cannot compare a list",
        ),
        (
            r#""x" & ({Status} + 1)"#,
            "cannot combine a reference and a number",
        ),
        ("NOT(-{Due} = 1)", "cannot negate a date"),
    ]);
}

fn for_field(source: &str, field: DataType) -> Result<Type, FormulaError> {
    check_for_field(&compile(source), &Orders, field)
}

#[test]
fn std_results_must_fit_the_field() {
    let fits: &[(&str, DataType)] = &[
        ("{Qty} * 2", DataType::Number),
        ("{Qty} / 100", DataType::Percent),
        ("{Price} * 2", DataType::Money),
        ("{Qty} > 1", DataType::Bool),
        ("{Due} + 1", DataType::Date),
        ("NOW()", DataType::DateTime),
        ("{Effort} * 2", DataType::Duration),
        ("{Status}", DataType::Reference),
        ("{Status}", DataType::MultiReference),
        ("{Tags}", DataType::MultiReference),
        ("UNIQUE({Lines}.{Labels})", DataType::MultiReference),
        (r#"ATTR({Tags}, "parent")"#, DataType::MultiReference),
        // Anything can be shown as text.
        ("{Qty} * 2", DataType::Text),
        ("{Tags}", DataType::Text),
        ("{Lines}.{Amount}", DataType::LongText),
        (r#"IF({Qty} = 0, "n/a", {Price} / {Qty})"#, DataType::Text),
        // A value of unknown kind is trusted.
        (r#"PARAM("x")"#, DataType::Money),
        (r#"PARAM("x")"#, DataType::Date),
        (r#"ATTR({Status}, "w")"#, DataType::Number),
        // Every alternative fits.
        ("IF({Done}, BLANK(), {Due})", DataType::Date),
        ("IF({Done}, {Due})", DataType::Date),
        ("IFERROR({Price} / {Qty}, {Price})", DataType::Money),
        ("IFS({Qty} > 5, 1, {Qty} > 2, 2, TRUE, 3)", DataType::Number),
        (
            "SWITCH({Qty}, 1, {Due}, 2, {Due} + 1, TODAY())",
            DataType::Date,
        ),
        ("IFBLANK({Due}, TODAY())", DataType::Date),
        (
            "IF({Done}, IF({Qty} > 1, 1, 2), IFERROR(3 / {Qty}, 0))",
            DataType::Number,
        ),
    ];
    for (source, field) in fits {
        assert!(
            for_field(source, *field).is_ok(),
            "{source} for {field:?}: {:?}",
            for_field(source, *field)
        );
    }
    // The type is reported as by `check`.
    assert_eq!(for_field("{Qty} * 2", DataType::Text), Ok(Type::Number));
    assert_eq!(
        for_field(r#"IF({Qty} = 0, "n/a", {Qty})"#, DataType::Text),
        Ok(Type::Any)
    );
}

#[test]
fn invalid_results_for_the_field() {
    let cases: &[(&str, DataType, &str)] = &[
        (
            "{Qty}",
            DataType::Money,
            "the formula can return a number, but the field is a money field",
        ),
        (
            "{Due}",
            DataType::Number,
            "can return a date, but the field is a number field",
        ),
        (
            "{Price}",
            DataType::Percent,
            "can return an amount, but the field is a percent field",
        ),
        ("{Qty}", DataType::Bool, "can return a number"),
        (
            "{Created}",
            DataType::Date,
            "can return a date and time, but the field is a date field",
        ),
        ("{Due}", DataType::DateTime, "can return a date"),
        ("{Qty}", DataType::Duration, "can return a number"),
        ("{Title}", DataType::Reference, "can return text"),
        ("{Tags}", DataType::Reference, "can return a list"),
        (
            "{Lines}.{Qty}",
            DataType::MultiReference,
            "can return a list",
        ),
        ("{Qty}", DataType::MultiReference, "can return a number"),
        // One alternative that does not fit is enough: those records would show an error.
        (
            r#"IF({Qty} = 0, "n/a", {Qty})"#,
            DataType::Number,
            "can return text, but the field is a number field",
        ),
        (
            r#"IFERROR({Qty} / 2, "error")"#,
            DataType::Number,
            "can return text",
        ),
        (
            "IFS({Done}, 1, TRUE, {Due})",
            DataType::Number,
            "can return a date",
        ),
        (
            "SWITCH({Qty}, 1, {Price}, 2, {Qty})",
            DataType::Money,
            "can return a number",
        ),
        (
            "SWITCH({Qty}, 1, {Price}, {Qty})",
            DataType::Money,
            "can return a number",
        ),
        (
            r#"IF({Done}, IF({Qty} > 1, 1, "x"), 2)"#,
            DataType::Number,
            "can return text",
        ),
        ("IFBLANK({Title}, 0)", DataType::Number, "can return text"),
        // What a computed field can never be.
        (
            "{Qty}",
            DataType::Link,
            "a computed field cannot be a link field",
        ),
        (
            "{Owner}",
            DataType::Principal,
            "a computed field cannot be a principal field",
        ),
        (
            "{Files}",
            DataType::Attachment,
            "a computed field cannot be a attachment field",
        ),
        (
            "{Extra}",
            DataType::Json,
            "a computed field cannot be a json field",
        ),
        // Problems inside the formula come first.
        (
            "{Price} + 1",
            DataType::Text,
            "cannot combine an amount and a number",
        ),
    ];
    for (source, field, needle) in cases {
        match for_field(source, *field) {
            Err(FormulaError::Type(message)) => {
                assert!(
                    message.contains(needle),
                    "{source} for {field:?}: {message}"
                );
            }
            other => panic!("{source} for {field:?}: {other:?}"),
        }
    }
}

#[test]
fn std_error_values_and_messages() {
    let codes: Vec<&str> = ErrorValue::ALL.iter().map(|error| error.code()).collect();
    assert_eq!(
        codes,
        [
            "#DIV/0!",
            "#VALUE!",
            "#REF!",
            "#NUM!",
            "#CURRENCY!",
            "#N/A",
            "#LIMIT!",
            "#CYCLE!"
        ]
    );
    for error in ErrorValue::ALL.iter().copied() {
        assert_eq!(error.to_string(), error.code());
        assert!(error.explanation().ends_with('.'), "{error}");
        // Stored by its code.
        let json = serde_json::to_string(&error).unwrap();
        assert_eq!(json, format!("\"{}\"", error.code()));
        assert_eq!(serde_json::from_str::<ErrorValue>(&json).unwrap(), error);
    }
    assert!(serde_json::from_str::<ErrorValue>("\"#NOPE!\"").is_err());
    assert_eq!(
        FormulaError::Cycle {
            path: "A → B → A".into()
        }
        .to_string(),
        "circular reference: A → B → A"
    );
    assert_eq!(
        FormulaError::Limit("more than 2,000 parts").to_string(),
        "the formula is too large: more than 2,000 parts"
    );
    assert_eq!(
        FormulaError::Type("x cannot be y".into()).to_string(),
        "x cannot be y"
    );
}

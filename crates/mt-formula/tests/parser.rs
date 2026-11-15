//! Parsing and printing. Categories are marked in the test names (`std_`, `edge_`,
//! `invalid_`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_core::id::FieldId;
use mt_formula::ast::{MAX_DEPTH, MAX_SOURCE_CHARS};
use mt_formula::{
    parse, BinaryOp, DateFn, Expr, FormulaError, Function, LogicFn, Names, NumberFn, TextFn,
    UnaryOp,
};

fn n(text: &str) -> Expr {
    Expr::Number(dec(text))
}

fn field(id: u128) -> Expr {
    Expr::Field(fid(id))
}

fn binary(op: BinaryOp, left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
    }
}

fn call(function: Function, args: Vec<Expr>) -> Expr {
    Expr::Call { function, args }
}

fn error(source: &str) -> FormulaError {
    parse(source, &Orders).unwrap_err()
}

fn syntax(source: &str) -> (usize, String) {
    let found = error(source);
    let FormulaError::Syntax { position, message } = found else {
        panic!("`{source}`: expected a syntax error, got {found:?}");
    };
    (position, message)
}

/// Parses, prints and parses again: the text and the tree must both be stable.
fn round_trip(source: &str) -> String {
    let tree = compile(source);
    let printed = tree.to_text(&Orders);
    assert_eq!(compile(&printed), tree, "`{source}` printed as `{printed}`");
    assert_eq!(compile(&printed).to_text(&Orders), printed);
    printed
}

#[test]
fn std_literals_and_references() {
    assert_eq!(compile("42"), n("42"));
    assert_eq!(compile("3.50"), n("3.50"));
    assert_eq!(compile("\"hello\""), Expr::Text("hello".into()));
    assert_eq!(
        compile("\"say \"\"hi\"\"\""),
        Expr::Text("say \"hi\"".into())
    );
    assert_eq!(compile("\"\""), Expr::Text(String::new()));
    assert_eq!(compile("TRUE"), Expr::Bool(true));
    assert_eq!(compile("false"), Expr::Bool(false));
    assert_eq!(compile("currentvalue"), Expr::CurrentValue);
    assert_eq!(compile("{Qty}"), field(QTY));
    assert_eq!(
        compile("{qty}"),
        field(QTY),
        "names are resolved by the schema"
    );
    assert_eq!(compile("{Empty number}"), field(EMPTY_NUMBER));
    // A brace or a backslash in a field name is escaped.
    assert_eq!(compile("{Odd \\}name\\\\}"), field(ODD_NAME));
    assert_eq!(
        compile("{Lines}.{Amount}"),
        Expr::Through {
            link: fid(LINES),
            field: fid(LINE_AMOUNT)
        }
    );
    assert_eq!(
        compile("  {Lines} . {Amount}  "),
        Expr::Through {
            link: fid(LINES),
            field: fid(LINE_AMOUNT)
        }
    );
    // Whitespace and line breaks do not matter.
    assert_eq!(compile(" 1\n+\t2 "), compile("1+2"));
}

#[test]
fn std_operator_precedence_and_grouping() {
    use BinaryOp::{Add, Concat, Divide, Equal, Less, Multiply, Subtract};
    assert_eq!(
        compile("1 + 2 * 3"),
        binary(Add, n("1"), binary(Multiply, n("2"), n("3")))
    );
    assert_eq!(
        compile("(1 + 2) * 3"),
        binary(Multiply, binary(Add, n("1"), n("2")), n("3"))
    );
    // Same level: left to right.
    assert_eq!(
        compile("10 - 4 - 3"),
        binary(Subtract, binary(Subtract, n("10"), n("4")), n("3"))
    );
    assert_eq!(
        compile("8 / 4 / 2"),
        binary(Divide, binary(Divide, n("8"), n("4")), n("2"))
    );
    // `&` binds looser than arithmetic and tighter than comparison.
    assert_eq!(
        compile("\"a\" & 1 + 2 = \"a3\""),
        binary(
            Equal,
            binary(Concat, Expr::Text("a".into()), binary(Add, n("1"), n("2"))),
            Expr::Text("a3".into())
        )
    );
    assert_eq!(
        compile("1 + 2 < 4"),
        binary(Less, binary(Add, n("1"), n("2")), n("4"))
    );
    for (source, op) in [
        ("1 = 2", BinaryOp::Equal),
        ("1 <> 2", BinaryOp::NotEqual),
        ("1 < 2", BinaryOp::Less),
        ("1 <= 2", BinaryOp::LessOrEqual),
        ("1 > 2", BinaryOp::Greater),
        ("1 >= 2", BinaryOp::GreaterOrEqual),
    ] {
        assert_eq!(compile(source), binary(op, n("1"), n("2")), "{source}");
    }
}

#[test]
fn std_signs_and_percent() {
    let negate = |operand: Expr| Expr::Unary {
        op: UnaryOp::Negate,
        operand: Box::new(operand),
    };
    let percent = |operand: Expr| Expr::Unary {
        op: UnaryOp::Percent,
        operand: Box::new(operand),
    };
    // A minus directly before a number is part of the number.
    assert_eq!(compile("-5"), n("-5"));
    assert_eq!(compile("- 5"), n("-5"));
    assert_eq!(compile("+5"), n("5"));
    assert_eq!(compile("-0"), n("0"));
    assert_eq!(compile("-{Qty}"), negate(field(QTY)));
    assert_eq!(compile("--{Qty}"), negate(negate(field(QTY))));
    assert_eq!(compile("-(5)"), negate(n("5")));
    assert_eq!(
        compile("3 - -2"),
        binary(BinaryOp::Subtract, n("3"), n("-2"))
    );
    assert_eq!(
        compile("-2 * 3"),
        binary(BinaryOp::Multiply, n("-2"), n("3"))
    );
    assert_eq!(
        compile("-{Qty} * 3"),
        binary(BinaryOp::Multiply, negate(field(QTY)), n("3"))
    );
    // Percent binds tighter than the sign.
    assert_eq!(compile("5%"), percent(n("5")));
    assert_eq!(compile("-5%"), negate(percent(n("5"))));
    assert_eq!(compile("(-5)%"), percent(n("-5")));
    assert_eq!(compile("{Qty}%%"), percent(percent(field(QTY))));
    assert_eq!(
        compile("{Price} * 20%"),
        binary(BinaryOp::Multiply, field(PRICE), percent(n("20")))
    );
}

#[test]
fn std_function_calls() {
    assert_eq!(
        compile("if({Qty} > 1, \"many\", \"one\")"),
        call(
            Function::Logic(LogicFn::If),
            vec![
                binary(BinaryOp::Greater, field(QTY), n("1")),
                Expr::Text("many".into()),
                Expr::Text("one".into())
            ]
        )
    );
    assert_eq!(
        compile("TODAY()"),
        call(Function::Date(DateFn::Today), vec![])
    );
    assert_eq!(
        compile("Sum( 1 , 2 )"),
        call(Function::Number(NumberFn::Sum), vec![n("1"), n("2")])
    );
    // CONCATENATE is another name for CONCAT.
    assert_eq!(compile("CONCATENATE(\"a\")"), compile("CONCAT(\"a\")"));
    assert_eq!(
        compile("concat(\"a\")"),
        call(Function::Text(TextFn::Concat), vec![Expr::Text("a".into())])
    );
    // Every function is found by its own name, in any case, and takes its minimum arguments.
    for function in Function::all() {
        assert_eq!(Function::by_name(function.name()), Some(function));
        assert_eq!(
            Function::by_name(&function.name().to_lowercase()),
            Some(function)
        );
        let pairs = function == Function::Logic(LogicFn::Ifs);
        let count = if pairs { 2 } else { function.min_args() };
        let source = format!("{}({})", function.name(), vec!["1"; count].join(", "));
        assert!(matches!(compile(&source), Expr::Call { .. }), "{source}");
    }
    assert_eq!(Function::all().len(), 81);
    assert_eq!(Function::by_name("NOPE"), None);
}

#[test]
fn std_printing_uses_current_names_and_minimal_parentheses() {
    let cases = [
        ("1+2*3", "1 + 2 * 3"),
        ("(1+2)*3", "(1 + 2) * 3"),
        ("1-(2-3)", "1 - (2 - 3)"),
        ("(1-2)-3", "1 - 2 - 3"),
        ("1/(2*3)", "1 / (2 * 3)"),
        ("-({Qty}+1)", "-({Qty} + 1)"),
        ("-{Qty}*2", "-{Qty} * 2"),
        ("2*-{Qty}", "2 * -{Qty}"),
        ("3 - -2", "3 - (-2)"),
        ("-5", "-5"),
        ("-5%", "-5%"),
        ("(-5)%", "(-5)%"),
        ("({Qty}+1)%", "({Qty} + 1)%"),
        ("({Qty}=1)=TRUE", "({Qty} = 1) = TRUE"),
        ("{Qty}=(1=1)", "{Qty} = (1 = 1)"),
        ("\"a\"&\"b\"&\"c\"", "\"a\" & \"b\" & \"c\""),
        ("\"he said \"\"x\"\"\"", "\"he said \"\"x\"\"\""),
        ("if({done},1,2)", "IF({Done}, 1, 2)"),
        ("today( )", "TODAY()"),
        ("sum({lines}.{amount})", "SUM({Lines}.{Amount})"),
        ("{odd \\}name\\\\}", "{Odd \\}name\\\\}"),
        (
            "COUNTIF({Lines}.{Qty}, CurrentValue>1)",
            "COUNTIF({Lines}.{Qty}, CurrentValue > 1)",
        ),
        ("1.50", "1.5"),
        ("TRUE", "TRUE"),
    ];
    for (source, expected) in cases {
        assert_eq!(round_trip(source), expected, "{source}");
    }

    // After a rename the same tree prints with the new name; a deleted field shows as an
    // id that cannot be mistaken for a name.
    struct Renamed;
    impl Names for Renamed {
        fn field(&self, field: FieldId) -> Option<String> {
            (field == fid(QTY)).then(|| "Quantity".to_owned())
        }
        fn linked_field(&self, _link: FieldId, _field: FieldId) -> Option<String> {
            None
        }
    }
    let tree = compile("{Qty} * {Price} + SUM({Lines}.{Amount})");
    assert_eq!(
        tree.to_text(&Renamed),
        "{Quantity} * {#fld_00000000000000000000000003} + SUM({#fld_0000000000000000000000000a}.{#fld_00000000000000000000000035})"
    );
}

#[test]
fn std_trees_survive_storage() {
    for source in [
        "IF({Qty} > 1, {Price} * {Qty} * (1 - {Discount}), 0)",
        "SUMIF({Lines}.{Qty}, CurrentValue > 1, {Lines}.{Amount})",
        "\"a\" & -5% & TRUE",
    ] {
        let tree = compile(source);
        let json = serde_json::to_string(&tree).unwrap();
        assert_eq!(serde_json::from_str::<Expr>(&json).unwrap(), tree, "{json}");
    }
    // The stored form names functions by their formula names and fields by id.
    let json = serde_json::to_string(&compile("SUM({Qty}, 1.50)")).unwrap();
    assert_eq!(
        json,
        r#"{"t":"call","v":{"function":"SUM","args":[{"t":"field","v":"fld_00000000000000000000000002"},{"t":"number","v":"1.5"}]}}"#
    );
    assert!(
        serde_json::from_str::<Expr>(r#"{"t":"call","v":{"function":"NOPE","args":[]}}"#).is_err()
    );
    // A function name in another case still loads.
    let lower: Function = serde_json::from_str("\"sum\"").unwrap();
    assert_eq!(lower, Function::Number(NumberFn::Sum));
}

#[test]
fn invalid_syntax_is_reported_with_a_position() {
    let cases: &[(&str, usize, &str)] = &[
        ("", 0, "the formula is empty"),
        ("   ", 0, "the formula is empty"),
        ("1 +", 3, "expected a value, found the end of the formula"),
        ("* 2", 0, "expected a value, found `*`"),
        (
            "1 2",
            2,
            "expected an operator or the end of the formula, found the number 2",
        ),
        ("(1 + 2", 6, "expected `)`"),
        (
            "1 + 2)",
            5,
            "expected an operator or the end of the formula, found `)`",
        ),
        ("\"open", 0, "not closed with `\"`"),
        ("{Qty", 0, "not closed with `}`"),
        ("{}", 0, "a field name is missing"),
        ("{  }", 0, "a field name is missing"),
        ("{a\\b}", 2, "must be followed by"),
        ("1 $ 2", 2, "unexpected character `$`"),
        ("1 ^ 2", 2, "unexpected character `^`"),
        (
            "Qty + 1",
            0,
            "`Qty` is not a value; put field names in braces",
        ),
        ("{Qty}.", 6, "expected a field of the linked table"),
        ("{Qty}.5", 6, "expected a field of the linked table"),
        ("SUM(1 2)", 6, "expected `,` or `)`"),
        ("SUM(1,", 6, "expected a value"),
        ("SUM(,1)", 4, "expected a value, found `,`"),
        ("1 < {Qty} < 10", 10, "comparisons cannot be chained"),
        ("1 = 1 = 1", 6, "comparisons cannot be chained"),
        ("1.", 1, "expected an operator"),
        ("99999999999999999999999999999999999", 0, "too many digits"),
    ];
    for (source, position, message) in cases {
        let (found_position, found_message) = syntax(source);
        assert!(
            found_message.contains(message),
            "`{source}`: {found_message}"
        );
        assert_eq!(found_position, *position, "`{source}`: {found_message}");
    }
    // A comparison inside parentheses or a function is not a chain.
    compile("(1 < {Qty}) = TRUE");
    compile("AND(1 < {Qty}, {Qty} < 10)");
    compile("IF(1 < 2, 3 < 4, 5 < 6)");
}

#[test]
fn invalid_names_and_argument_counts() {
    assert_eq!(
        error("{Nope} + 1"),
        FormulaError::UnknownField {
            name: "Nope".into(),
            position: 0
        }
    );
    assert_eq!(
        error("1 + {Lines}.{Nope}"),
        FormulaError::UnknownField {
            name: "Lines.Nope".into(),
            position: 4
        }
    );
    // Only link fields lead somewhere.
    assert_eq!(
        error("{Qty}.{Amount}"),
        FormulaError::UnknownField {
            name: "Qty.Amount".into(),
            position: 0
        }
    );
    assert_eq!(
        error("1 + NOPE(2)"),
        FormulaError::UnknownFunction {
            name: "NOPE".into(),
            position: 4
        }
    );
    let arity = |source: &str| {
        let problem = error(source);
        let FormulaError::Arity {
            function,
            expected,
            found,
        } = problem
        else {
            panic!("`{source}`: {problem:?}");
        };
        (function, expected, found)
    };
    assert_eq!(arity("IF(1)"), ("IF", "2 to 3".to_owned(), 1));
    assert_eq!(arity("IF(1, 2, 3, 4)"), ("IF", "2 to 3".to_owned(), 4));
    assert_eq!(arity("NOT()"), ("NOT", "1".to_owned(), 0));
    assert_eq!(arity("TODAY(1)"), ("TODAY", "0".to_owned(), 1));
    assert_eq!(arity("SUM()"), ("SUM", "at least 1".to_owned(), 0));
    assert_eq!(
        arity("IFS(1, 2, 3)"),
        ("IFS", "an even number of".to_owned(), 3)
    );
    assert_eq!(arity("MID(\"a\", 1)"), ("MID", "3".to_owned(), 2));
    assert_eq!(
        error("IF(1)").to_string(),
        "IF takes 2 to 3 argument(s), but 1 were given"
    );
    assert_eq!(
        error("{Nope}").to_string(),
        "there is no field named `Nope` (character 0)"
    );
    assert_eq!(
        error("NOPE()").to_string(),
        "there is no function named `NOPE` (character 0)"
    );
    assert_eq!(
        error("1 +").to_string(),
        "syntax error at character 3: expected a value, found the end of the formula"
    );
}

#[test]
fn edge_size_limits() {
    // Nesting.
    let nested = |levels: usize| format!("{}1{}", "(".repeat(levels), ")".repeat(levels));
    compile(&nested(MAX_DEPTH - 2));
    assert_eq!(
        error(&nested(MAX_DEPTH + 1)),
        FormulaError::Limit("nested more than 64 levels deep")
    );
    let negated = format!("{}{{Qty}}", "-".repeat(MAX_DEPTH + 1));
    assert_eq!(
        error(&negated),
        FormulaError::Limit("nested more than 64 levels deep")
    );
    // Length.
    let long = format!("\"{}\"", "x".repeat(MAX_SOURCE_CHARS));
    assert_eq!(
        error(&long),
        FormulaError::Limit("more than 10,000 characters")
    );
    let fits = format!("\"{}\"", "x".repeat(MAX_SOURCE_CHARS - 2));
    compile(&fits);
    // Number of parts: 1 + 1 + 1 + … has two parts per term.
    let sum = vec!["1"; 1_200].join("+");
    assert_eq!(error(&sum), FormulaError::Limit("more than 2,000 parts"));
    let wide = format!("SUM({})", vec!["1"; 1_500].join(","));
    let tree = compile(&wide);
    assert_eq!(tree.node_count(), 1_501);
    assert_eq!(tree.depth(), 2);
    assert_eq!(compile("1").depth(), 1);
    assert_eq!(compile("-{Qty} + 1").depth(), 3);
}

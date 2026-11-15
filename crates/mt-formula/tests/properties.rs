//! Properties that hold for every formula: printing keeps the meaning, evaluation is total
//! and deterministic, the type checker tells the truth about results, the graph orders and
//! recomputes correctly, and the parser survives any text. Also thread-safety.
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
use mt_core::id::{FieldId, TableId};
use mt_core::value::Value;
use mt_formula::{
    analyse, check, check_for_field, dependencies, evaluate, evaluate_to, parse, BinaryOp,
    ComputedField, Expr, Function, Graph, LogicFn, LookupFn, Schema, Type, UnaryOp, Val,
};
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

const ORDER_FIELDS: &[u128] = &[
    TITLE,
    QTY,
    PRICE,
    DISCOUNT,
    DUE,
    CREATED,
    DONE,
    STATUS,
    TAGS,
    LINES,
    EFFORT,
    NOTES,
    OWNER,
    EMPTY_NUMBER,
    EMPTY_DATE,
    EMPTY_TEXT,
    ODD_NAME,
    EMPTY_MONEY,
    SHIPPING,
    FILES,
    EXTRA,
];
const LINE_FIELDS: &[u128] = &[
    LINE_AMOUNT,
    LINE_QTY,
    LINE_NAME,
    LINE_SHIPPED,
    LINE_LABELS,
    LINE_DONE,
];

const TEXTS: &[&str] = &[
    "",
    " ",
    "a",
    "Bolt",
    "nut",
    "12.5",
    "-3",
    "TRUE",
    "false",
    "2026-05-20",
    "day",
    "month",
    "d",
    "m",
    "y",
    "0.00",
    "#,##0",
    "yyyy-mm-dd",
    "hh:mm",
    ">1",
    "<>nut",
    "<=2026-05-05",
    "weight",
    "tax.rate",
    "say \"hi\"",
    "{x}",
    "line\nbreak",
    "ß",
    "é",
    ", ",
];

fn leaf() -> impl Strategy<Value = Expr> {
    prop_oneof![
        4 => (-50_i64..200, 0_u32..3).prop_map(|(n, scale)| Expr::Number(Decimal::new(n, scale))),
        3 => prop::sample::select(TEXTS).prop_map(|text| Expr::Text(text.to_owned())),
        1 => any::<bool>().prop_map(Expr::Bool),
        8 => prop::sample::select(ORDER_FIELDS).prop_map(|field| Expr::Field(fid(field))),
        3 => prop::sample::select(LINE_FIELDS)
            .prop_map(|field| Expr::Through { link: fid(LINES), field: fid(field) }),
        1 => Just(Expr::CurrentValue),
    ]
}

const BINARY: &[BinaryOp] = &[
    BinaryOp::Add,
    BinaryOp::Subtract,
    BinaryOp::Multiply,
    BinaryOp::Divide,
    BinaryOp::Concat,
    BinaryOp::Equal,
    BinaryOp::NotEqual,
    BinaryOp::Less,
    BinaryOp::LessOrEqual,
    BinaryOp::Greater,
    BinaryOp::GreaterOrEqual,
];

/// A call with a valid number of arguments: missing ones are filled in, extra ones dropped.
fn call(function: Function, mut args: Vec<Expr>) -> Expr {
    let min = function.min_args();
    while args.len() < min {
        args.push(Expr::Number(Decimal::ONE));
    }
    if let Some(max) = function.max_args() {
        args.truncate(max);
    }
    if function == Function::Logic(LogicFn::Ifs) && args.len() % 2 == 1 {
        args.push(Expr::Text("odd".into()));
    }
    Expr::Call { function, args }
}

/// Any well-formed formula tree over the fixture's fields, typed or not.
fn expr() -> impl Strategy<Value = Expr> {
    let functions: Vec<Function> = Function::all().into_iter().collect();
    leaf().prop_recursive(5, 40, 4, move |inner| {
        prop_oneof![
            2 => (prop::sample::select(BINARY), inner.clone(), inner.clone()).prop_map(
                |(op, left, right)| Expr::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right)
                }
            ),
            1 => (any::<bool>(), inner.clone()).prop_map(|(negate, operand)| Expr::Unary {
                op: if negate { UnaryOp::Negate } else { UnaryOp::Percent },
                operand: Box::new(operand),
            }),
            4 => (prop::sample::select(functions.clone()), prop::collection::vec(inner, 0..4))
                .prop_map(|(function, args)| call(function, args)),
        ]
    })
}

/// The records formulas are evaluated for: the sample order, an empty record, and the
/// order with some fields emptied.
fn records() -> Vec<Rec> {
    let mut sparse = order();
    for field in [QTY, PRICE, DUE, CREATED, STATUS, TAGS, EFFORT, TITLE] {
        sparse.cells.remove(&fid(field));
    }
    sparse.linked.clear();
    vec![order(), Rec::default(), sparse]
}

fn uses_unknown_values(expr: &Expr) -> bool {
    let mut found = false;
    expr.walk(&mut |node| {
        if let Expr::Call { function, .. } = node {
            found |= matches!(function, Function::Lookup(LookupFn::Param | LookupFn::Attr));
        }
    });
    found
}

/// Whether a value is of the kind the checker promised. Empty values and errors fit any
/// type; a plain zero is what calculating with nothing but empty amounts or durations
/// leaves.
fn is_of(value: &Val, promised: &Type) -> bool {
    let zero = |value: &Val| matches!(value, Val::Number(n) if n.is_zero());
    match (value, promised) {
        (Val::Blank | Val::Error(_), _) | (_, Type::Any) => true,
        (Val::Number(_), Type::Number)
        | (Val::Text(_), Type::Text)
        | (Val::Bool(_), Type::Bool)
        | (Val::Date(_), Type::Date)
        | (Val::DateTime(_), Type::DateTime)
        | (Val::Duration(_), Type::Duration)
        | (Val::Money(..), Type::Money)
        | (Val::Ref(_), Type::Reference) => true,
        (value, Type::Money | Type::Duration) if zero(value) => true,
        (Val::List(items), Type::List(element)) => items.iter().all(|item| is_of(item, element)),
        _ => false,
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 600, ..ProptestConfig::default() })]

    /// Printing a formula and reading it back never changes what is calculated, and a
    /// formula that was read prints to text that reads back as the very same formula — so
    /// renaming a field or reformatting is always safe.
    #[test]
    fn prop_printing_keeps_the_meaning(tree in expr()) {
        let text = tree.to_text(&Orders);
        let parsed = parse(&text, &Orders);
        prop_assert!(parsed.is_ok(), "`{}` does not parse: {:?}", text, parsed);
        let parsed = parsed.unwrap();
        for record in records() {
            let original = evaluate(&tree, &Orders, &record, &UTC);
            let reread = evaluate(&parsed, &Orders, &record, &UTC);
            prop_assert_eq!(original, reread, "`{}`", text);
        }
        // From here on the text is stable (a hand-built tree may print `-0` once).
        let normal = parsed.to_text(&Orders);
        let again = parse(&normal, &Orders);
        prop_assert!(again.is_ok(), "`{}` does not parse: {:?}", normal, again);
        let again = again.unwrap();
        prop_assert_eq!(&again, &parsed, "`{}` reread from `{}`", text, normal);
        prop_assert_eq!(again.to_text(&Orders), normal);
    }

    /// Evaluation always ends with a value, the same one every time, in every time zone
    /// setting, and analysis never fails.
    #[test]
    fn prop_evaluation_is_total_and_repeatable(tree in expr()) {
        for record in records() {
            for world in [UTC, TOKYO, LOS_ANGELES, IDLE] {
                let first = evaluate(&tree, &Orders, &record, &world);
                let second = evaluate(&tree, &Orders, &record, &world);
                prop_assert_eq!(&first, &second);
                // Lists are flat.
                if let Val::List(items) = &first {
                    prop_assert!(items.iter().all(|item| !matches!(item, Val::List(_))));
                }
                // Every result can be offered to every kind of field without a crash.
                for target in DataType::ALL.iter().copied() {
                    let _ = evaluate_to(&tree, &Orders, &record, &world, None, target);
                }
            }
        }
        let analysis = analyse(&tree);
        prop_assert_eq!(analysis.nodes, tree.node_count());
        prop_assert_eq!(&analysis.dependencies, &dependencies(&tree));
        let _ = check(&tree, &Orders);
    }

    /// What the checker accepts evaluates to the kind of value it announced (or to nothing,
    /// or to an error value) — for every record.
    #[test]
    fn prop_the_checker_tells_the_truth(tree in expr()) {
        prop_assume!(!uses_unknown_values(&tree));
        let Ok(promised) = check(&tree, &Orders) else { return Ok(()); };
        for record in records() {
            let value = evaluate(&tree, &Orders, &record, &UTC);
            prop_assert!(
                is_of(&value, &promised),
                "`{}` was announced as {:?} but evaluated to {}",
                tree.to_text(&Orders),
                promised,
                show(&value)
            );
        }
        // And a formula accepted for a field never stores a value of another kind there.
        for target in DataType::ALL.iter().copied() {
            if check_for_field(&tree, &Orders, target).is_err() {
                continue;
            }
            for record in records() {
                let stored = evaluate_to(&tree, &Orders, &record, &UTC, None, target);
                if let Ok(value) = stored {
                    prop_assert!(
                        value.is_empty() || value.data_type() == Some(target),
                        "`{}` stored {:?} in a {:?} field",
                        tree.to_text(&Orders),
                        value,
                        target
                    );
                }
            }
        }
    }

    /// A formula's stored form is read back as the same formula.
    #[test]
    fn prop_trees_survive_storage(tree in expr()) {
        let json = serde_json::to_string(&tree).unwrap();
        prop_assert_eq!(serde_json::from_str::<Expr>(&json).unwrap(), tree);
    }

    /// The parser answers every text with a formula or an error, never with a crash, and
    /// what it accepts can be printed and read again.
    #[test]
    fn prop_the_parser_survives_any_text(
        parts in prop::collection::vec(
            prop_oneof![
                prop::sample::select(&[
                    "{Qty}", "{Price}", "{Lines}", ".", "{Due}", "(", ")", ",", "+", "-", "*",
                    "/", "&", "=", "<>", "<", "<=", ">", ">=", "%", "1", "2.5", "\"a\"", "\"",
                    "IF", "SUM", "TODAY", "CurrentValue", "TRUE", " ", "{", "}", "\\", "{Nope}",
                    "NOPE", "\n", "é", "$",
                ][..]).prop_map(str::to_owned),
                "\\PC{0,3}",
            ],
            0..14,
        )
    ) {
        let text = parts.concat();
        if let Ok(tree) = parse(&text, &Orders) {
            let printed = tree.to_text(&Orders);
            let reread = parse(&printed, &Orders);
            prop_assert!(reread.is_ok(), "`{}` printed as `{}`: {:?}", text, printed, reread);
            prop_assert_eq!(reread.unwrap().to_text(&Orders), printed);
        }
    }
}

// ---------------------------------------------------------------------------------------
// Formulas that are well typed by construction.
// ---------------------------------------------------------------------------------------

/// The kinds of expression the generator can build.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Number,
    Money,
    Date,
    Instant,
    Span,
    Text,
    Bool,
    Ref,
    Numbers,
    Amounts,
    Dates,
    Texts,
    Refs,
}

const SCALARS: &[Kind] = &[
    Kind::Number,
    Kind::Money,
    Kind::Date,
    Kind::Instant,
    Kind::Span,
    Kind::Text,
    Kind::Bool,
    Kind::Ref,
];

impl Kind {
    fn of_placeholder(name: &str, rng: &mut u64) -> Kind {
        match name {
            "N" => Kind::Number,
            "M" => Kind::Money,
            "D" => Kind::Date,
            "I" => Kind::Instant,
            "U" => Kind::Span,
            "T" => Kind::Text,
            "B" => Kind::Bool,
            "R" => Kind::Ref,
            "LN" => Kind::Numbers,
            "LM" => Kind::Amounts,
            "LD" => Kind::Dates,
            "LT" => Kind::Texts,
            "LR" => Kind::Refs,
            "X" => SCALARS[next(rng) % SCALARS.len()],
            other => panic!("unknown placeholder ${other}"),
        }
    }

    fn promised(self) -> Type {
        let list = |element| Type::List(Box::new(element));
        match self {
            Kind::Number => Type::Number,
            Kind::Money => Type::Money,
            Kind::Date => Type::Date,
            Kind::Instant => Type::DateTime,
            Kind::Span => Type::Duration,
            Kind::Text => Type::Text,
            Kind::Bool => Type::Bool,
            Kind::Ref => Type::Reference,
            Kind::Numbers => list(Type::Number),
            Kind::Amounts => list(Type::Money),
            Kind::Dates => list(Type::Date),
            Kind::Texts => list(Type::Text),
            Kind::Refs => list(Type::Reference),
        }
    }

    /// Ways to write an expression of this kind: without and with parts (`$N` is a number,
    /// `$M` an amount, `$D` a date, `$I` an instant, `$U` a duration, `$T` text, `$B` a
    /// condition, `$R` a reference, `$L…` lists of those, `$X` any single value).
    fn templates(self) -> (&'static [&'static str], &'static [&'static str]) {
        match self {
            Kind::Number => (
                &[
                    "0",
                    "1",
                    "2.5",
                    "-3",
                    "100",
                    "{Qty}",
                    "{Discount}",
                    "{Empty number}",
                ],
                &[
                    "($N + $N)",
                    "($N - $N)",
                    "($N * $N)",
                    "($N / $N)",
                    "-$N",
                    "$N%",
                    "($D - $D)",
                    "($M / $M)",
                    "($U / $U)",
                    "SUM($LN)",
                    "SUM($N, $LN)",
                    "AVERAGE($LN)",
                    "MEDIAN($N, $N)",
                    "MIN($LN)",
                    "MAX($N, $N)",
                    "COUNT($LM)",
                    "COUNTA($LT)",
                    "LEN($T)",
                    "YEAR($D)",
                    "MONTH($I)",
                    "DAY($D)",
                    "HOUR($I)",
                    "MINUTE($I)",
                    "WEEKDAY($D)",
                    "ISOWEEKNUM($D)",
                    "DAYS($D, $D)",
                    "NETWORKDAYS($D, $D)",
                    "DATEDIF($D, $D, \"d\")",
                    "IF($B, $N, $N)",
                    "IF($B, $N)",
                    "IFS($B, $N, $B, $N)",
                    "SWITCH($N, 1, $N, $N)",
                    "IFERROR($N, $N)",
                    "IFBLANK($N, $N)",
                    "ROUND($N, 1)",
                    "ROUNDUP($N)",
                    "ROUNDDOWN($N, 2)",
                    "INT($N)",
                    "ABS($N)",
                    "SIGN($M)",
                    "SIGN($N)",
                    "MOD($N, $N)",
                    "POWER($N, 2)",
                    "CEILING($N, 0.5)",
                    "FLOOR($N)",
                    "COUNTIF($LN, CurrentValue > $N)",
                    "COUNTIF($LM, \">5\")",
                    "COUNTIF($LR, $R)",
                    "SUMIF($LN, CurrentValue >= $N)",
                    "AVERAGEIF($LN, \">0\")",
                    "FIRST($LN)",
                    "LAST($LN)",
                    "INDEX($LN, 1)",
                    "TO_HOURS($U)",
                    "TO_DAYS($U)",
                    "VALUE($T)",
                    "FIND(\"o\", $T)",
                    "SEARCH(\"O\", $T)",
                ],
            ),
            Kind::Money => (
                &["{Price}", "{Shipping}", "{Empty money}"],
                &[
                    "($M + $M)",
                    "($M - $M)",
                    "($M * $N)",
                    "($N * $M)",
                    "($M / $N)",
                    "-$M",
                    "$M%",
                    "SUM($LM)",
                    "SUM($M, $M)",
                    "AVERAGE($LM)",
                    "MEDIAN($LM)",
                    "MIN($LM)",
                    "MAX($M, $M)",
                    "ROUND($M, 1)",
                    "ROUNDUP($M)",
                    "CEILING($M, 0.05)",
                    "FLOOR($M)",
                    "ABS($M)",
                    "IF($B, $M, $M)",
                    "IF($B, BLANK(), $M)",
                    "IFERROR($M, $M)",
                    "IFBLANK($M, $M)",
                    "FIRST($LM)",
                    "INDEX($LM, 2)",
                    "SUMIF($LN, \">1\", $LM)",
                    "SUMIF($LM, CurrentValue > $M)",
                    "AVERAGEIF($LM, \">5\")",
                    "SWITCH($N, 1, $M, $M)",
                ],
            ),
            Kind::Date => (
                &[
                    "{Due}",
                    "{Empty date}",
                    "TODAY()",
                    "DATE(2026, 2, 28)",
                    "DATE(2024, 2, 29)",
                ],
                &[
                    "($D + $N)",
                    "($N + $D)",
                    "($D - $N)",
                    "($D + 3)",
                    "EDATE($D, 1)",
                    "EDATE($I, -2)",
                    "EOMONTH($D, 0)",
                    "WORKDAY($D, 2)",
                    "WORKDAY($D, -3)",
                    "DATEADD($D, 1, \"month\")",
                    "DATEADD($D, -1, \"week\")",
                    "IF($B, $D, $D)",
                    "IF($B, $D)",
                    "IF($B, BLANK(), $D)",
                    "MIN($LD)",
                    "MAX($D, $D)",
                    "FIRST($LD)",
                    "LAST($LD)",
                    "IFBLANK($D, $D)",
                    "IFERROR($D, $D)",
                ],
            ),
            Kind::Instant => (
                &["{Created}", "NOW()"],
                &[
                    "($I + $U)",
                    "($U + $I)",
                    "($I - $U)",
                    "DATEADD($I, 1, \"day\")",
                    "DATEADD($I, 2, \"months\")",
                    "IF($B, $I, $I)",
                    "IF($B, $I)",
                    "MAX($I, $I)",
                    "MIN($I, $I)",
                    "IFBLANK($I, $I)",
                ],
            ),
            Kind::Span => (
                &["{Effort}", "DURATION(1, 30)", "DURATION(0, 45)"],
                &[
                    "($I - $I)",
                    "($U + $U)",
                    "($U - $U)",
                    "($U * $N)",
                    "($N * $U)",
                    "($U / $N)",
                    "-$U",
                    "SUM($U, $U)",
                    "AVERAGE($U, $U)",
                    "MAX($U, $U)",
                    "IF($B, $U, $U)",
                    "IF($B, $U)",
                    "DURATION($N)",
                    "DURATION($N, $N, $N)",
                    "IFERROR($U, $U)",
                ],
            ),
            Kind::Text => (
                &[
                    "\"\"",
                    "\"abc\"",
                    "\"Bolt\"",
                    "\"12.5\"",
                    "{Title}",
                    "{Notes}",
                    "{Empty text}",
                ],
                &[
                    "($T & $X)",
                    "($X & $T)",
                    "UPPER($T)",
                    "LOWER($T)",
                    "TRIM($T)",
                    "LEFT($T, 2)",
                    "RIGHT($T)",
                    "MID($T, 2, 3)",
                    "REPT($T, 2)",
                    "SUBSTITUTE($T, \"o\", \"0\")",
                    "REPLACE($T, 1, 1, $T)",
                    "TEXT($N, \"0.00\")",
                    "TEXT($M, \"#,##0.0\")",
                    "TEXT($N, \"0%\")",
                    "TEXT($D, \"yyyy-mm-dd\")",
                    "TEXT($I, \"d mmm hh:mm\")",
                    "LABEL($R)",
                    "CODE($R)",
                    "CONCAT($LT)",
                    "CONCAT($T, $X)",
                    "TEXTJOIN(\", \", TRUE, $LT)",
                    "TEXTJOIN(\"/\", FALSE, $LN)",
                    "IF($B, $T, $T)",
                    "IF($B, $T)",
                    "IFS($B, $T, TRUE, $T)",
                    "SWITCH($T, \"abc\", $T, $T)",
                    "FIRST($LT)",
                    "IFBLANK($T, $T)",
                    "IFERROR($T, $T)",
                ],
            ),
            Kind::Bool => (
                &["TRUE", "FALSE", "{Done}"],
                &[
                    "($N > $N)",
                    "($N = $N)",
                    "($N <> $N)",
                    "($T = $T)",
                    "($T < $T)",
                    "($D < $D)",
                    "($D >= $D)",
                    "($M >= $M)",
                    "($M = $M)",
                    "($U <> $U)",
                    "($U < $U)",
                    "($I > $I)",
                    "($R = $R)",
                    "($R <> $R)",
                    "($B = $B)",
                    "($D = \"\")",
                    "($M <> \"\")",
                    "AND($B, $B)",
                    "OR($B, $B)",
                    "NOT($B)",
                    "XOR($B, $B)",
                    "ISBLANK($X)",
                    "ISERROR($X)",
                    "ISNUMBER($X)",
                    "ISTEXT($X)",
                    "CONTAINS($T, $T)",
                    "CONTAINS($LR, $R)",
                    "CONTAINS($LN, $N)",
                    "EXACT($T, $T)",
                    "IF($B, $B, $B)",
                    "AND($B, $N)",
                ],
            ),
            Kind::Ref => (
                &["{Status}"],
                &[
                    "FIRST($LR)",
                    "LAST($LR)",
                    "INDEX($LR, 1)",
                    "IF($B, $R, $R)",
                    "IF($B, $R)",
                ],
            ),
            Kind::Numbers => (
                &["{Lines}.{Qty}"],
                &[
                    "FILTER($LN, CurrentValue > $N)",
                    "FILTER($LN, \">=2\")",
                    "UNIQUE($LN)",
                    "IF($B, $LN, $LN)",
                    "UNIQUE($N)",
                ],
            ),
            Kind::Amounts => (
                &["{Lines}.{Amount}"],
                &[
                    "FILTER($LM, CurrentValue > $M)",
                    "UNIQUE($LM)",
                    "IF($B, $LM, $LM)",
                ],
            ),
            Kind::Dates => (
                &["{Lines}.{Shipped}"],
                &[
                    "FILTER($LD, CurrentValue < $D)",
                    "UNIQUE($LD)",
                    "FILTER($LD, $D)",
                ],
            ),
            Kind::Texts => (
                &["{Lines}.{Name}", "{Lines}", "{Owner}", "{Files}"],
                &[
                    "LABEL($LR)",
                    "CODE($LR)",
                    "FILTER($LT, LEN(CurrentValue) > $N)",
                    "UNIQUE($LT)",
                    "FILTER($LT, \"<>nut\")",
                ],
            ),
            Kind::Refs => (
                &["{Tags}", "{Lines}.{Labels}"],
                &[
                    "UNIQUE($LR)",
                    "FILTER($LR, CurrentValue = $R)",
                    "IF($B, $LR, $LR)",
                ],
            ),
        }
    }
}

fn next(rng: &mut u64) -> usize {
    *rng = rng
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*rng >> 33) as usize
}

/// Writes a formula of the given kind, at most `depth` levels deep.
fn write(kind: Kind, depth: u32, rng: &mut u64) -> String {
    let (simple, composed) = kind.templates();
    let template = if depth == 0 || next(rng) % 5 == 0 {
        simple[next(rng) % simple.len()]
    } else {
        composed[next(rng) % composed.len()]
    };
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('$') {
        out.push_str(&rest[..start]);
        let name: String = rest[start + 1..]
            .chars()
            .take_while(char::is_ascii_uppercase)
            .collect();
        out.push_str(&write(
            Kind::of_placeholder(&name, rng),
            depth.saturating_sub(1),
            rng,
        ));
        rest = &rest[start + 1 + name.len()..];
    }
    out.push_str(rest);
    out
}

const KINDS: &[Kind] = &[
    Kind::Number,
    Kind::Money,
    Kind::Date,
    Kind::Instant,
    Kind::Span,
    Kind::Text,
    Kind::Bool,
    Kind::Ref,
    Kind::Numbers,
    Kind::Amounts,
    Kind::Dates,
    Kind::Texts,
    Kind::Refs,
];

proptest! {
    #![proptest_config(ProptestConfig { cases: 1500, ..ProptestConfig::default() })]

    /// A formula written to be of one kind is accepted as that kind, evaluates to that
    /// kind (or to nothing, or to an error value) for every record in every time zone, and
    /// stores a value of the field's type in a field of that kind.
    #[test]
    fn prop_well_typed_formulas_keep_their_promise(
        kind in prop::sample::select(KINDS),
        depth in 1_u32..5,
        seed in any::<u64>(),
    ) {
        let mut rng = seed;
        let source = write(kind, depth, &mut rng);
        let tree = parse(&source, &Orders);
        prop_assert!(tree.is_ok(), "`{}`: {:?}", source, tree);
        let tree = tree.unwrap();
        let promised = kind.promised();
        prop_assert_eq!(check(&tree, &Orders), Ok(promised.clone()), "`{}`", source);
        for record in records() {
            for world in [UTC, TOKYO, LOS_ANGELES] {
                let value = evaluate(&tree, &Orders, &record, &world);
                prop_assert!(
                    is_of(&value, &promised),
                    "`{}` is {:?} but evaluated to {}",
                    source,
                    promised,
                    show(&value)
                );
            }
        }
        // Stored in a field of its natural type, or shown as text.
        if let Some(field) = promised.data_type() {
            prop_assert!(check_for_field(&tree, &Orders, field).is_ok(), "`{}`", source);
            for record in records() {
                if let Ok(value) = evaluate_to(&tree, &Orders, &record, &UTC, None, field) {
                    prop_assert!(
                        value.is_empty() || value.data_type() == Some(field),
                        "`{}` stored {:?} in a {:?} field",
                        source,
                        value,
                        field
                    );
                }
            }
        }
        prop_assert!(check_for_field(&tree, &Orders, DataType::Text).is_ok());
        // Printing keeps the type and the meaning.
        let reread = parse(&tree.to_text(&Orders), &Orders).unwrap();
        prop_assert_eq!(&reread, &tree, "`{}`", source);
    }
}

// ---------------------------------------------------------------------------------------
// The graph: random acyclic definitions over one table.
// ---------------------------------------------------------------------------------------

const TABLE: u128 = 1;
/// Plain fields are 1..=4, computed fields 101...
const PLAIN: u128 = 4;

struct Numbers;

impl Schema for Numbers {
    fn field_by_name(&self, _name: &str) -> Option<FieldId> {
        None
    }
    fn field_type(&self, _field: FieldId) -> Option<DataType> {
        Some(DataType::Number)
    }
    fn linked_field_by_name(&self, _link: FieldId, _name: &str) -> Option<FieldId> {
        None
    }
    fn linked_field_type(&self, _link: FieldId, _field: FieldId) -> Option<DataType> {
        None
    }
}

/// Computed field `101 + i` is the sum of the fields it reads (plain ones, and computed
/// ones with a smaller index) plus one.
fn definitions(reads: &[Vec<u128>]) -> Vec<ComputedField> {
    reads
        .iter()
        .enumerate()
        .map(|(index, inputs)| {
            let mut expr = Expr::Number(Decimal::ONE);
            for input in inputs {
                expr = Expr::Binary {
                    op: BinaryOp::Add,
                    left: Box::new(expr),
                    right: Box::new(Expr::Field(fid(*input))),
                };
            }
            ComputedField {
                table: TableId::from_u128(TABLE),
                field: fid(101 + index as u128),
                expr,
            }
        })
        .collect()
}

/// For each computed field, which plain fields and earlier computed fields it reads.
fn acyclic_reads() -> impl Strategy<Value = Vec<Vec<u128>>> {
    (1_usize..14).prop_flat_map(|count| {
        (0..count)
            .map(|index| {
                let candidates: Vec<u128> = (1..=PLAIN)
                    .chain((0..index as u128).map(|i| 101 + i))
                    .collect();
                prop::collection::vec(prop::sample::select(candidates), 0..4)
            })
            .collect::<Vec<_>>()
    })
}

fn build(fields: Vec<ComputedField>) -> Graph {
    Graph::build(fields, &|_, _| None, &|_, field| field.to_string()).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 300, ..ProptestConfig::default() })]

    /// The evaluation order contains every computed field once, each after everything it
    /// reads, and does not depend on the order the definitions arrive in.
    #[test]
    fn prop_the_order_respects_every_dependency(reads in acyclic_reads(), seed in any::<u64>()) {
        let graph = build(definitions(&reads));
        let order = graph.order_in(TableId::from_u128(TABLE));
        prop_assert_eq!(order.len(), reads.len());
        let position: BTreeMap<FieldId, usize> =
            order.iter().enumerate().map(|(index, field)| (*field, index)).collect();
        prop_assert_eq!(position.len(), reads.len());
        for (index, inputs) in reads.iter().enumerate() {
            for input in inputs.iter().filter(|input| **input > 100) {
                prop_assert!(position[&fid(*input)] < position[&fid(101 + index as u128)]);
            }
        }
        // Shuffle the definitions with the seed.
        let mut shuffled = definitions(&reads);
        let mut state = seed | 1;
        for index in (1..shuffled.len()).rev() {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            shuffled.swap(index, (state >> 33) as usize % (index + 1));
        }
        prop_assert_eq!(build(shuffled), graph);
    }

    /// After a change, exactly the computed fields that read a changed field — directly or
    /// through other computed fields — are recomputed, in evaluation order; and recomputing
    /// only those gives the same record as recomputing everything.
    #[test]
    fn prop_a_change_recomputes_exactly_what_depends_on_it(
        reads in acyclic_reads(),
        changed in prop::collection::btree_set(1..=PLAIN, 0..3),
        values in prop::collection::vec(-20_i64..20, 4),
        new_values in prop::collection::vec(-20_i64..20, 4),
    ) {
        let table = TableId::from_u128(TABLE);
        let graph = build(definitions(&reads));
        let changed_ids: BTreeSet<FieldId> = changed.iter().map(|f| fid(*f)).collect();

        // The model: mark and sweep over the definitions, which are in dependency order.
        let mut dirty: BTreeSet<u128> = changed.clone();
        let mut expected = BTreeSet::new();
        for (index, inputs) in reads.iter().enumerate() {
            if inputs.iter().any(|input| dirty.contains(input)) {
                dirty.insert(101 + index as u128);
                expected.insert(fid(101 + index as u128));
            }
        }
        let affected = graph.affected(table, &changed_ids);
        prop_assert!(affected.remote.is_empty());
        let local: BTreeSet<FieldId> = affected.local.iter().copied().collect();
        prop_assert_eq!(&local, &expected);
        prop_assert_eq!(local.len(), affected.local.len(), "a field is listed twice");
        let order = graph.order_in(table);
        let positions: Vec<usize> = affected
            .local
            .iter()
            .map(|field| order.iter().position(|o| o == field).unwrap())
            .collect();
        prop_assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));

        // Compute everything, store it, change the plain fields, recompute only what is
        // affected: the record must equal a full recompute.
        let mut record = Rec::default();
        for (index, value) in values.iter().enumerate() {
            record.cells.insert(fid(1 + index as u128), Value::Number(Decimal::from(*value)));
        }
        let store = |record: &mut Rec, results: &mt_formula::Computed| {
            for (field, result) in results {
                record.cells.insert(*field, result.clone().unwrap());
            }
        };
        let all = graph.compute(table, &Numbers, &record, &UTC, None);
        prop_assert_eq!(all.len(), reads.len());
        store(&mut record, &all);
        for field in &changed {
            let value = new_values[(*field - 1) as usize];
            record.cells.insert(fid(*field), Value::Number(Decimal::from(value)));
        }
        let mut partial = record.clone();
        let results = graph.compute(table, &Numbers, &record, &UTC, Some(&expected));
        prop_assert_eq!(results.len(), expected.len());
        store(&mut partial, &results);
        let mut full = record.clone();
        let results = graph.compute(table, &Numbers, &record, &UTC, None);
        store(&mut full, &results);
        prop_assert_eq!(partial.cells, full.cells);
    }
}

// ---------------------------------------------------------------------------------------
// Sharing between threads.
// ---------------------------------------------------------------------------------------

#[test]
fn concurrency_formulas_and_graphs_are_shared_between_threads() {
    fn shareable<T: Send + Sync>() {}
    shareable::<Expr>();
    shareable::<Graph>();
    shareable::<Val>();
    shareable::<Type>();
    shareable::<mt_formula::FormulaError>();
    shareable::<mt_formula::Dependencies>();

    let sources = [
        "{Price} * {Qty} * (1 - {Discount})",
        "SUMIF({Lines}.{Done}, TRUE, {Lines}.{Amount})",
        r#"TEXTJOIN(", ", TRUE, FILTER({Lines}.{Name}, LEN(CurrentValue) > 3))"#,
        "NETWORKDAYS(TODAY(), {Due})",
        r#"IF({Due} < TODAY(), "late", TEXT({Due}, "d mmm yyyy"))"#,
        "COUNTIF({Lines}.{Qty}, COUNTIF({Lines}.{Qty}, CurrentValue >= 1) = 2)",
    ];
    let trees: Vec<Expr> = sources.iter().map(|source| compile(source)).collect();
    let record = order();
    let expected: Vec<Val> = trees
        .iter()
        .map(|tree| evaluate(tree, &Orders, &record, &UTC))
        .collect();
    // Eight threads evaluate the same trees for the same record at the same time.
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    (0..200)
                        .map(|round| {
                            let tree = &trees[round % trees.len()];
                            (round % trees.len(), evaluate(tree, &Orders, &record, &UTC))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for handle in handles {
            for (index, value) in handle.join().unwrap() {
                assert_eq!(value, expected[index], "{}", sources[index]);
            }
        }
    });
}

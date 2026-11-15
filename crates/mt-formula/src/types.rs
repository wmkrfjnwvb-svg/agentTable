//! The type checker: what a formula returns, and whether its parts can fit together.
//!
//! Evaluation never fails (a misfit is an error *value* in the cell), but most misfits can
//! be seen when the formula is written: adding text to a date, summing dates, comparing a
//! number with text, using `CurrentValue` outside a list condition. Those are reported here,
//! at definition time, so that nobody saves a formula that can only ever show `#VALUE!`.
//!
//! The checker is deliberately lenient where the evaluator is: text is accepted where a
//! number is needed (it may be written as a number), and a value of unknown kind
//! (`PARAM`, `ATTR`, `BLANK()`, `CurrentValue`) fits anywhere.
//!
//! Alternatives of different kinds — `IF({n} = 0, "n/a", {total} / {n})` — are fine inside
//! a formula (the result is of unknown kind, [`Type::Any`]). What a computed *field* stores
//! is checked separately by [`check_for_field`]: every value the formula can return must fit
//! the field, because a field holds one kind of value.

use crate::ast::{
    BinaryOp, DateFn, Expr, Function, ListFn, LogicFn, LookupFn, NumberFn, TextFn, UnaryOp,
    MAX_DEPTH, MAX_NODES,
};
use crate::error::FormulaError;
use crate::schema::Schema;
use mt_core::field::DataType;

/// The kind of value an expression produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    /// A number (percentages are numbers).
    Number,
    /// Text.
    Text,
    /// True or false.
    Bool,
    /// A calendar day.
    Date,
    /// An instant.
    DateTime,
    /// A length of time.
    Duration,
    /// An amount with a currency.
    Money,
    /// An item of a reference set.
    Reference,
    /// Several values of one kind.
    List(Box<Type>),
    /// Not known until evaluation; fits anywhere.
    Any,
}

impl Type {
    /// The type of the values of a field.
    #[must_use]
    pub fn of_field(data_type: DataType) -> Type {
        let list = |element: Type| Type::List(Box::new(element));
        match data_type {
            DataType::Text | DataType::LongText | DataType::Json => Type::Text,
            DataType::Number | DataType::Percent => Type::Number,
            DataType::Money => Type::Money,
            DataType::Bool => Type::Bool,
            DataType::Date => Type::Date,
            DataType::DateTime => Type::DateTime,
            DataType::Duration => Type::Duration,
            DataType::Reference => Type::Reference,
            DataType::MultiReference => list(Type::Reference),
            DataType::Link | DataType::Principal | DataType::Attachment => list(Type::Text),
        }
    }

    /// The type of one item: the element of a list, the type itself otherwise.
    #[must_use]
    pub fn element(&self) -> &Type {
        match self {
            Type::List(element) => element,
            Type::Number
            | Type::Text
            | Type::Bool
            | Type::Date
            | Type::DateTime
            | Type::Duration
            | Type::Money
            | Type::Reference
            | Type::Any => self,
        }
    }

    /// Whether this is a list.
    #[must_use]
    pub const fn is_list(&self) -> bool {
        matches!(self, Type::List(_))
    }

    /// The data type a computed field with this result would naturally have, if the type
    /// is definite and a single field type can hold it.
    #[must_use]
    pub fn data_type(&self) -> Option<DataType> {
        match self {
            Type::Number => Some(DataType::Number),
            Type::Text => Some(DataType::Text),
            Type::Bool => Some(DataType::Bool),
            Type::Date => Some(DataType::Date),
            Type::DateTime => Some(DataType::DateTime),
            Type::Duration => Some(DataType::Duration),
            Type::Money => Some(DataType::Money),
            Type::Reference => Some(DataType::Reference),
            Type::List(element) if **element == Type::Reference => Some(DataType::MultiReference),
            Type::List(_) | Type::Any => None,
        }
    }

    /// The type in words, for messages.
    fn words(&self) -> String {
        match self {
            Type::Number => "a number".to_owned(),
            Type::Text => "text".to_owned(),
            Type::Bool => "true/false".to_owned(),
            Type::Date => "a date".to_owned(),
            Type::DateTime => "a date and time".to_owned(),
            Type::Duration => "a duration".to_owned(),
            Type::Money => "an amount".to_owned(),
            Type::Reference => "a reference".to_owned(),
            Type::List(_) => "a list".to_owned(),
            Type::Any => "a value".to_owned(),
        }
    }

    /// Whether the type can stand where a number is needed: numbers, text (which may be
    /// written as a number) and true/false.
    const fn numeric(&self) -> bool {
        matches!(self, Type::Number | Type::Text | Type::Bool)
    }

    /// As [`Type::numeric`], or of unknown kind.
    const fn number_like(&self) -> bool {
        matches!(self, Type::Number | Type::Text | Type::Bool | Type::Any)
    }

    /// Whether the type can stand where a date is needed: dates, date-times, text (which
    /// may be written as `YYYY-MM-DD`) and unknown values.
    const fn date_like(&self) -> bool {
        matches!(self, Type::Date | Type::DateTime | Type::Text | Type::Any)
    }

    /// Whether the type can be a condition: true/false, numbers (zero is false), text
    /// (`"TRUE"`, `"FALSE"`) and unknown values.
    const fn condition_like(&self) -> bool {
        self.number_like()
    }
}

fn mismatch(message: String) -> FormulaError {
    FormulaError::typing(message)
}

/// The common type of two alternatives (the branches of an `IF`): the type both have, or
/// [`Type::Any`] when they differ or one of them is not known.
fn unify(a: Type, b: Type) -> Type {
    match (a, b) {
        (Type::List(a), Type::List(b)) => Type::List(Box::new(unify(*a, *b))),
        (a, b) if a == b => a,
        _ => Type::Any,
    }
}

/// The common type of several alternatives; unknown when there is none.
fn unify_all(types: impl IntoIterator<Item = Type>) -> Type {
    types.into_iter().reduce(unify).unwrap_or(Type::Any)
}

/// The result type of an arithmetic operator. Where an operand of unknown kind leaves more
/// than one possibility (a number plus an unknown value is a number or a date), the result
/// is of unknown kind too.
fn arithmetic(op: BinaryOp, a: &Type, b: &Type) -> Result<Type, FormulaError> {
    use Type::{Any, Date, DateTime, Duration, Money, Number};
    if a.is_list() || b.is_list() {
        return Err(mismatch(format!(
            "`{}` cannot calculate with a list; use SUM, FIRST or another list function",
            op.symbol()
        )));
    }
    let result = match op {
        BinaryOp::Add => match (a, b) {
            (Any, Any) => Some(Any),
            (Money, Money | Any) | (Any, Money) => Some(Money),
            (Date, other) | (other, Date) if other.number_like() => Some(Date),
            (DateTime, Duration | Any) | (Duration | Any, DateTime) => Some(DateTime),
            (Duration, Duration) => Some(Duration),
            // A duration plus an unknown value: a duration or a date-time.
            (Duration, Any) | (Any, Duration) => Some(Any),
            (x, y) if x.numeric() && y.numeric() => Some(Number),
            // A number plus an unknown value: a number or a date.
            (x, Any) | (Any, x) if x.numeric() => Some(Any),
            _ => None,
        },
        BinaryOp::Subtract => match (a, b) {
            (Any, Any) => Some(Any),
            (Money, Money | Any) | (Any, Money) => Some(Money),
            (Date, Date) | (Any, Date) => Some(Number),
            // A date minus an unknown value: a date (minus days) or days (minus a date).
            (Date, Any) => Some(Any),
            (Date, other) if other.numeric() => Some(Date),
            (DateTime, DateTime) | (Any, DateTime) => Some(Duration),
            (DateTime, Duration) => Some(DateTime),
            (DateTime, Any) => Some(Any),
            (Duration, Duration | Any) => Some(Duration),
            // An unknown value minus a duration: a duration or a date-time.
            (Any, Duration) => Some(Any),
            (x, y) if x.numeric() && y.number_like() => Some(Number),
            // An unknown value minus a number: a number or a date.
            (Any, y) if y.numeric() => Some(Any),
            _ => None,
        },
        BinaryOp::Multiply => match (a, b) {
            (Any, Any) => Some(Any),
            (Money, Money) | (Duration, Duration) => None,
            (Money, other) | (other, Money) if other.number_like() => Some(Money),
            (Duration, other) | (other, Duration) if other.number_like() => Some(Duration),
            (x, y) if x.numeric() && y.numeric() => Some(Number),
            // A number times an unknown value: a number, an amount or a duration.
            (x, Any) | (Any, x) if x.numeric() => Some(Any),
            _ => None,
        },
        BinaryOp::Divide => match (a, b) {
            (Any, Any) => Some(Any),
            (Money, Money) | (Duration, Duration) | (Any, Money | Duration) => Some(Number),
            // An amount divided by an unknown value: a ratio or an amount.
            (Money | Duration, Any) => Some(Any),
            (Money, other) if other.numeric() => Some(Money),
            (Duration, other) if other.numeric() => Some(Duration),
            (x, y) if x.numeric() && y.number_like() => Some(Number),
            // An unknown value divided by a number: a number, an amount or a duration.
            (Any, y) if y.numeric() => Some(Any),
            _ => None,
        },
        // Not arithmetic; handled by the caller.
        BinaryOp::Concat
        | BinaryOp::Equal
        | BinaryOp::NotEqual
        | BinaryOp::Less
        | BinaryOp::LessOrEqual
        | BinaryOp::Greater
        | BinaryOp::GreaterOrEqual => None,
    };
    result.ok_or_else(|| {
        mismatch(format!(
            "`{}` cannot combine {} and {}",
            op.symbol(),
            a.words(),
            b.words()
        ))
    })
}

/// The result type of a comparison. `with_empty_text` says that one side is the literal
/// `""`: `{Due} = ""` is the spreadsheet way of asking whether there is a value.
fn comparison(
    op: BinaryOp,
    a: &Type,
    b: &Type,
    with_empty_text: bool,
) -> Result<Type, FormulaError> {
    if a.is_list() || b.is_list() {
        return Err(mismatch(format!(
            "`{}` cannot compare a list; use CONTAINS, COUNTIF or FILTER",
            op.symbol()
        )));
    }
    let ordering = !matches!(op, BinaryOp::Equal | BinaryOp::NotEqual);
    if with_empty_text && !ordering {
        return Ok(Type::Bool);
    }
    let known_and_different = *a != Type::Any && *b != Type::Any && a != b;
    if known_and_different {
        return Err(mismatch(format!(
            "`{}` compares {} with {}; values of different kinds are never equal or ordered",
            op.symbol(),
            a.words(),
            b.words()
        )));
    }
    if ordering && (*a == Type::Reference || *b == Type::Reference) {
        return Err(mismatch(format!(
            "`{}` cannot order references; compare their LABEL or CODE",
            op.symbol()
        )));
    }
    Ok(Type::Bool)
}

/// The kind of result an aggregate over these argument types has.
fn aggregate(name: &str, args: &[Type], with_dates: bool) -> Result<Type, FormulaError> {
    let mut kind: Option<Type> = None;
    let mut plain_number = false;
    let mut unknown = false;
    for arg in args {
        let element = arg.element();
        let found = match element {
            Type::Money => Type::Money,
            Type::Duration => Type::Duration,
            Type::Number => {
                plain_number = true;
                continue;
            }
            Type::Any => {
                unknown = true;
                continue;
            }
            // Skipped at evaluation.
            Type::Text | Type::Bool | Type::Reference => continue,
            Type::Date | Type::DateTime if with_dates => element.clone(),
            Type::Date | Type::DateTime | Type::List(_) => {
                return Err(mismatch(format!(
                    "{name} cannot work with {}",
                    element.words()
                )));
            }
        };
        match &kind {
            Some(existing) if *existing != found => {
                return Err(mismatch(format!(
                    "{name} cannot combine {} and {}",
                    existing.words(),
                    found.words()
                )));
            }
            Some(_) | None => kind = Some(found),
        }
    }
    match kind {
        Some(kind) if plain_number => Err(mismatch(format!(
            "{name} cannot combine {} and plain numbers",
            kind.words()
        ))),
        Some(kind) => Ok(kind),
        None if unknown && !plain_number => Ok(Type::Any),
        None => Ok(Type::Number),
    }
}

/// A text literal argument, if the argument is one.
fn literal(args: &[Expr], index: usize) -> Option<&str> {
    match args.get(index) {
        Some(Expr::Text(text)) => Some(text),
        Some(
            Expr::Number(_)
            | Expr::Bool(_)
            | Expr::Field(_)
            | Expr::Through { .. }
            | Expr::CurrentValue
            | Expr::Unary { .. }
            | Expr::Binary { .. }
            | Expr::Call { .. },
        )
        | None => None,
    }
}

fn is_empty_text(expr: &Expr) -> bool {
    matches!(expr, Expr::Text(text) if text.trim().is_empty())
}

/// The arguments of a logic function that are possible results of it (empty for the
/// functions that return true or false).
fn alternatives(function: LogicFn, args: &[Expr]) -> Vec<&Expr> {
    match function {
        LogicFn::If => args.iter().skip(1).collect(),
        LogicFn::Ifs => args.iter().skip(1).step_by(2).collect(),
        LogicFn::Switch => {
            // Every second argument from the third, plus a final default.
            let count = args.len();
            let has_default = count.checked_rem(2) == Some(0);
            args.iter()
                .enumerate()
                .filter_map(|(index, arg)| {
                    let is_result = index >= 2 && index.checked_rem(2) == Some(0);
                    let is_default = has_default && index.saturating_add(1) == count;
                    (is_result || is_default).then_some(arg)
                })
                .collect()
        }
        LogicFn::IfError | LogicFn::IfBlank => args.iter().collect(),
        LogicFn::And
        | LogicFn::Or
        | LogicFn::Not
        | LogicFn::Xor
        | LogicFn::IsBlank
        | LogicFn::IsError
        | LogicFn::IsNumber
        | LogicFn::IsText
        | LogicFn::Blank => Vec::new(),
    }
}

/// Whether an expression can only ever be empty: `BLANK()`, or a choice between such.
/// An empty alternative says nothing about the kind of the others.
fn is_nothing(expr: &Expr) -> bool {
    let Expr::Call {
        function: Function::Logic(function),
        args,
    } = expr
    else {
        return false;
    };
    if *function == LogicFn::Blank {
        return true;
    }
    let alternatives = alternatives(*function, args);
    !alternatives.is_empty() && alternatives.into_iter().all(is_nothing)
}

/// The expressions whose value the formula can return: the formula itself, or the
/// alternatives of the `IF`, `IFS`, `SWITCH`, `IFERROR` or `IFBLANK` at its top.
fn outcomes<'e>(expr: &'e Expr, out: &mut Vec<&'e Expr>) {
    if let Expr::Call {
        function: Function::Logic(function),
        args,
    } = expr
    {
        let alternatives = alternatives(*function, args);
        if !alternatives.is_empty() {
            for alternative in alternatives {
                outcomes(alternative, out);
            }
            return;
        }
    }
    out.push(expr);
}

struct Checker<'s> {
    schema: &'s dyn Schema,
}

impl Checker<'_> {
    fn infer(&self, expr: &Expr, in_condition: bool) -> Result<Type, FormulaError> {
        match expr {
            Expr::Number(_) => Ok(Type::Number),
            Expr::Text(_) => Ok(Type::Text),
            Expr::Bool(_) => Ok(Type::Bool),
            Expr::Field(field) => self
                .schema
                .field_type(*field)
                .map(Type::of_field)
                .ok_or_else(|| FormulaError::UnknownField {
                    name: field.to_string(),
                    position: 0,
                }),
            Expr::Through { link, field } => {
                match self.schema.field_type(*link) {
                    Some(DataType::Link) => {}
                    Some(other) => {
                        return Err(mismatch(format!(
                            "only a link field leads to another table; {link} is a {other} field"
                        )));
                    }
                    None => {
                        return Err(FormulaError::UnknownField {
                            name: link.to_string(),
                            position: 0,
                        });
                    }
                }
                let linked = self
                    .schema
                    .linked_field_type(*link, *field)
                    .ok_or_else(|| FormulaError::UnknownField {
                        name: format!("{link}.{field}"),
                        position: 0,
                    })?;
                // Lists are flat: a multi-value field of the linked records adds its items.
                Ok(Type::List(Box::new(
                    Type::of_field(linked).element().clone(),
                )))
            }
            Expr::CurrentValue => {
                if in_condition {
                    Ok(Type::Any)
                } else {
                    Err(mismatch(
                        "CurrentValue can only be used in the condition of FILTER, COUNTIF, SUMIF or AVERAGEIF"
                            .to_owned(),
                    ))
                }
            }
            Expr::Unary { op, operand } => {
                let operand = self.infer(operand, in_condition)?;
                match (&operand, op) {
                    (Type::Money, _) => Ok(Type::Money),
                    (Type::Duration, _) => Ok(Type::Duration),
                    (Type::Any, _) => Ok(Type::Any),
                    (other, _) if other.numeric() => Ok(Type::Number),
                    (other, UnaryOp::Negate) => {
                        Err(mismatch(format!("`-` cannot negate {}", other.words())))
                    }
                    (other, UnaryOp::Percent) => Err(mismatch(format!(
                        "`%` cannot take a percentage of {}",
                        other.words()
                    ))),
                }
            }
            Expr::Binary { op, left, right } => {
                let a = self.infer(left, in_condition)?;
                let b = self.infer(right, in_condition)?;
                match op {
                    BinaryOp::Add | BinaryOp::Subtract | BinaryOp::Multiply | BinaryOp::Divide => {
                        arithmetic(*op, &a, &b)
                    }
                    BinaryOp::Concat => Ok(Type::Text),
                    BinaryOp::Equal
                    | BinaryOp::NotEqual
                    | BinaryOp::Less
                    | BinaryOp::LessOrEqual
                    | BinaryOp::Greater
                    | BinaryOp::GreaterOrEqual => {
                        let with_empty_text = is_empty_text(left) || is_empty_text(right);
                        comparison(*op, &a, &b, with_empty_text)
                    }
                }
            }
            Expr::Call { function, args } => self.call(*function, args, in_condition),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn call(
        &self,
        function: Function,
        args: &[Expr],
        in_condition: bool,
    ) -> Result<Type, FormulaError> {
        crate::parser::check_arity(function, args.len())?;
        // The second argument of these functions is a condition about each item.
        let takes_condition = matches!(
            function,
            Function::List(ListFn::Filter | ListFn::CountIf | ListFn::SumIf | ListFn::AverageIf)
        );
        let mut types = Vec::with_capacity(args.len());
        for (index, arg) in args.iter().enumerate() {
            let condition = in_condition || (takes_condition && index == 1);
            types.push(self.infer(arg, condition)?);
        }
        let first = types.first().cloned().unwrap_or(Type::Any);
        let name = function.name();
        // An argument, when it is given, must be of a kind the function can use.
        let need = |index: usize, what: &str, fits: fn(&Type) -> bool| match types.get(index) {
            Some(found) if !fits(found) => Err(mismatch(format!(
                "{name} needs {what} as argument {}, not {}",
                index.saturating_add(1),
                found.words()
            ))),
            Some(_) | None => Ok(()),
        };
        let number = |index: usize| need(index, "a number", Type::number_like);
        let date = |index: usize| need(index, "a date", Type::date_like);
        let condition = |index: usize| need(index, "a condition", Type::condition_like);
        match function {
            Function::Logic(function) => match function {
                LogicFn::If
                | LogicFn::Ifs
                | LogicFn::Switch
                | LogicFn::IfError
                | LogicFn::IfBlank => {
                    match function {
                        LogicFn::If => condition(0)?,
                        LogicFn::Ifs => {
                            for index in (0..types.len()).step_by(2) {
                                condition(index)?;
                            }
                        }
                        LogicFn::Switch
                        | LogicFn::IfError
                        | LogicFn::IfBlank
                        | LogicFn::And
                        | LogicFn::Or
                        | LogicFn::Not
                        | LogicFn::Xor
                        | LogicFn::IsBlank
                        | LogicFn::IsError
                        | LogicFn::IsNumber
                        | LogicFn::IsText
                        | LogicFn::Blank => {}
                    }
                    // The arguments were all checked above; here only the possible
                    // results count, and an empty one fits any kind.
                    let mut results = Vec::new();
                    for alternative in alternatives(function, args) {
                        if !is_nothing(alternative) {
                            results.push(self.infer(alternative, in_condition)?);
                        }
                    }
                    Ok(unify_all(results))
                }
                LogicFn::And | LogicFn::Or | LogicFn::Not | LogicFn::Xor => {
                    for index in 0..types.len() {
                        condition(index)?;
                    }
                    Ok(Type::Bool)
                }
                LogicFn::IsBlank | LogicFn::IsError | LogicFn::IsNumber | LogicFn::IsText => {
                    Ok(Type::Bool)
                }
                LogicFn::Blank => Ok(Type::Any),
            },
            Function::Number(function) => match function {
                NumberFn::Sum | NumberFn::Average | NumberFn::Median => {
                    aggregate(name, &types, false)
                }
                NumberFn::Min | NumberFn::Max => aggregate(name, &types, true),
                NumberFn::Count | NumberFn::CountA => Ok(Type::Number),
                NumberFn::Abs
                | NumberFn::Round
                | NumberFn::RoundUp
                | NumberFn::RoundDown
                | NumberFn::Ceiling
                | NumberFn::Floor => {
                    number(1)?;
                    match first {
                        Type::Money => Ok(Type::Money),
                        Type::Any => Ok(Type::Any),
                        Type::Number | Type::Text | Type::Bool => Ok(Type::Number),
                        other @ (Type::Date
                        | Type::DateTime
                        | Type::Duration
                        | Type::Reference
                        | Type::List(_)) => Err(mismatch(format!(
                            "{name} needs a number or an amount, not {}",
                            other.words()
                        ))),
                    }
                }
                NumberFn::Sign => {
                    need(0, "a number or an amount", |t| {
                        t.number_like() || *t == Type::Money
                    })?;
                    Ok(Type::Number)
                }
                NumberFn::Int => {
                    number(0)?;
                    Ok(Type::Number)
                }
                NumberFn::Mod | NumberFn::Power => {
                    number(0)?;
                    number(1)?;
                    Ok(Type::Number)
                }
            },
            Function::Text(function) => match function {
                TextFn::Concat
                | TextFn::TextJoin
                | TextFn::Upper
                | TextFn::Lower
                | TextFn::Trim => Ok(Type::Text),
                TextFn::Left | TextFn::Right | TextFn::Rept => {
                    number(1)?;
                    Ok(Type::Text)
                }
                TextFn::Mid | TextFn::Replace => {
                    number(1)?;
                    number(2)?;
                    Ok(Type::Text)
                }
                TextFn::Substitute => {
                    number(3)?;
                    Ok(Type::Text)
                }
                TextFn::Text => match first {
                    Type::Number | Type::Money | Type::Date | Type::DateTime | Type::Any => {
                        Ok(Type::Text)
                    }
                    other @ (Type::Text
                    | Type::Bool
                    | Type::Duration
                    | Type::Reference
                    | Type::List(_)) => Err(mismatch(format!(
                        "TEXT formats numbers, amounts and dates, not {}",
                        other.words()
                    ))),
                },
                TextFn::Len => Ok(Type::Number),
                TextFn::Find | TextFn::Search => {
                    number(2)?;
                    Ok(Type::Number)
                }
                TextFn::Value => {
                    number(0)?;
                    Ok(Type::Number)
                }
                TextFn::Exact | TextFn::Contains => Ok(Type::Bool),
            },
            Function::Date(function) => {
                let unit_ok = |index: usize, allowed: &[&str]| match literal(args, index) {
                    Some(unit) if !allowed.contains(&unit.trim().to_ascii_lowercase().as_str()) => {
                        Err(mismatch(format!(
                            "{name} does not know the unit \"{unit}\"; use one of: {}",
                            allowed.join(", ")
                        )))
                    }
                    Some(_) | None => Ok(()),
                };
                match function {
                    DateFn::Today => Ok(Type::Date),
                    DateFn::Now => Ok(Type::DateTime),
                    DateFn::Date => {
                        number(0)?;
                        number(1)?;
                        number(2)?;
                        Ok(Type::Date)
                    }
                    DateFn::EDate | DateFn::EoMonth | DateFn::Workday => {
                        date(0)?;
                        number(1)?;
                        Ok(Type::Date)
                    }
                    DateFn::DateAdd => {
                        number(1)?;
                        unit_ok(
                            2,
                            &[
                                "d", "day", "days", "w", "week", "weeks", "m", "month", "months",
                                "y", "year", "years",
                            ],
                        )?;
                        match first {
                            Type::DateTime => Ok(Type::DateTime),
                            Type::Date | Type::Text => Ok(Type::Date),
                            Type::Any => Ok(Type::Any),
                            other @ (Type::Number
                            | Type::Bool
                            | Type::Duration
                            | Type::Money
                            | Type::Reference
                            | Type::List(_)) => Err(mismatch(format!(
                                "DATEADD needs a date as argument 1, not {}",
                                other.words()
                            ))),
                        }
                    }
                    DateFn::DateDif => {
                        date(0)?;
                        date(1)?;
                        unit_ok(2, &["d", "m", "y"])?;
                        Ok(Type::Number)
                    }
                    DateFn::Days | DateFn::NetworkDays => {
                        date(0)?;
                        date(1)?;
                        Ok(Type::Number)
                    }
                    DateFn::Year
                    | DateFn::Month
                    | DateFn::Day
                    | DateFn::Hour
                    | DateFn::Minute
                    | DateFn::IsoWeekNum => {
                        date(0)?;
                        Ok(Type::Number)
                    }
                    DateFn::Weekday => {
                        date(0)?;
                        number(1)?;
                        Ok(Type::Number)
                    }
                    DateFn::ToHours | DateFn::ToDays => {
                        need(0, "a duration", |t| matches!(t, Type::Duration | Type::Any))?;
                        Ok(Type::Number)
                    }
                    DateFn::Duration => {
                        number(0)?;
                        number(1)?;
                        number(2)?;
                        Ok(Type::Duration)
                    }
                }
            }
            Function::List(function) => match function {
                ListFn::CountIf => Ok(Type::Number),
                ListFn::SumIf | ListFn::AverageIf => {
                    let source = types.get(2).cloned().unwrap_or(first);
                    aggregate(name, &[source], false)
                }
                ListFn::Filter | ListFn::Unique => {
                    Ok(Type::List(Box::new(first.element().clone())))
                }
                ListFn::First | ListFn::Last => Ok(first.element().clone()),
                ListFn::Index => {
                    number(1)?;
                    Ok(first.element().clone())
                }
            },
            Function::Lookup(function) => match function {
                LookupFn::Label | LookupFn::Code => match first {
                    Type::Reference => Ok(Type::Text),
                    Type::Any => Ok(Type::Any),
                    Type::List(element) if matches!(*element, Type::Reference | Type::Any) => {
                        Ok(Type::List(Box::new(Type::Text)))
                    }
                    other @ (Type::Number
                    | Type::Text
                    | Type::Bool
                    | Type::Date
                    | Type::DateTime
                    | Type::Duration
                    | Type::Money
                    | Type::List(_)) => Err(mismatch(format!(
                        "{name} needs a reference, not {}",
                        other.words()
                    ))),
                },
                LookupFn::Attr => match &first {
                    Type::Reference | Type::Any => Ok(Type::Any),
                    Type::List(element) if matches!(**element, Type::Reference | Type::Any) => {
                        Ok(Type::List(Box::new(Type::Any)))
                    }
                    other @ (Type::Number
                    | Type::Text
                    | Type::Bool
                    | Type::Date
                    | Type::DateTime
                    | Type::Duration
                    | Type::Money
                    | Type::List(_)) => Err(mismatch(format!(
                        "ATTR needs a reference, not {}",
                        other.words()
                    ))),
                },
                LookupFn::Param => {
                    if literal(args, 0).is_some_and(|key| key.trim().is_empty()) {
                        return Err(mismatch("PARAM needs the key of a parameter".to_owned()));
                    }
                    Ok(Type::Any)
                }
            },
        }
    }
}

/// The type of an expression as far as it can be told without evaluating it; `None` for an
/// expression that does not pass the checker. Used by the evaluator to tell an empty date
/// from an empty number.
pub(crate) fn static_type(expr: &Expr, schema: &dyn Schema) -> Option<Type> {
    Checker { schema }.infer(expr, true).ok()
}

/// Checks a formula against the schema and returns the type of its result.
///
/// # Errors
/// [`FormulaError::Type`] for parts that cannot fit together, [`FormulaError::UnknownField`]
/// for a field that no longer exists, [`FormulaError::Arity`] and [`FormulaError::Limit`]
/// for trees that did not come from the parser.
pub fn check(expr: &Expr, schema: &dyn Schema) -> Result<Type, FormulaError> {
    if expr.depth() > MAX_DEPTH {
        return Err(FormulaError::Limit("nested more than 64 levels deep"));
    }
    if expr.node_count() > MAX_NODES {
        return Err(FormulaError::Limit("more than 2,000 parts"));
    }
    Checker { schema }.infer(expr, false)
}

/// Whether a value of type `found` can be stored in a computed field of type `field`.
fn fits(field: DataType, found: &Type) -> bool {
    match (field, found) {
        (_, Type::Any) | (DataType::Text | DataType::LongText, _) => true,
        (DataType::Number | DataType::Percent, Type::Number)
        | (DataType::Money, Type::Money)
        | (DataType::Bool, Type::Bool)
        | (DataType::Date, Type::Date)
        | (DataType::DateTime, Type::DateTime)
        | (DataType::Duration, Type::Duration)
        | (DataType::Reference | DataType::MultiReference, Type::Reference) => true,
        (DataType::MultiReference, Type::List(element)) => {
            matches!(**element, Type::Reference | Type::Any)
        }
        _ => false,
    }
}

/// Checks a formula and that everything it can return can be stored in a computed field of
/// the given data type. `IF({n} = 0, "n/a", {total} / {n})` is fine for a text field and
/// refused for a number field, where the `"n/a"` records would show an error.
///
/// # Errors
/// As [`check`], and [`FormulaError::Type`] when a result does not fit the field.
pub fn check_for_field(
    expr: &Expr,
    schema: &dyn Schema,
    field: DataType,
) -> Result<Type, FormulaError> {
    let found = check(expr, schema)?;
    if matches!(
        field,
        DataType::Link | DataType::Principal | DataType::Attachment | DataType::Json
    ) {
        return Err(mismatch(format!(
            "a computed field cannot be a {field} field"
        )));
    }
    let mut results = Vec::new();
    outcomes(expr, &mut results);
    let checker = Checker { schema };
    for result in results {
        let result = checker.infer(result, false)?;
        if !fits(field, &result) {
            return Err(mismatch(format!(
                "the formula can return {}, but the field is a {field} field",
                result.words()
            )));
        }
    }
    Ok(found)
}

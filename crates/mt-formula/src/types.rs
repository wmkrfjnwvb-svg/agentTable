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
    /// written as a number), true/false and unknown values.
    const fn number_like(&self) -> bool {
        matches!(self, Type::Number | Type::Text | Type::Bool | Type::Any)
    }
}

fn mismatch(message: String) -> FormulaError {
    FormulaError::typing(message)
}

/// The common type of two alternatives (the branches of an `IF`).
fn unify(a: Type, b: Type) -> Result<Type, FormulaError> {
    match (a, b) {
        (Type::Any, other) | (other, Type::Any) => Ok(other),
        (Type::List(a), Type::List(b)) => Ok(Type::List(Box::new(unify(*a, *b)?))),
        (a, b) if a == b => Ok(a),
        (a, b) => Err(mismatch(format!(
            "the formula returns {} in one case and {} in another; a field holds one kind of value",
            a.words(),
            b.words()
        ))),
    }
}

fn unify_all(types: impl IntoIterator<Item = Type>) -> Result<Type, FormulaError> {
    types.into_iter().try_fold(Type::Any, unify)
}

/// The result type of an arithmetic operator.
fn arithmetic(op: BinaryOp, a: &Type, b: &Type) -> Result<Type, FormulaError> {
    use Type::{Any, Date, DateTime, Duration, Money, Number};
    if a.is_list() || b.is_list() {
        return Err(mismatch(format!(
            "`{}` cannot calculate with a list; use SUM, FIRST or another list function",
            op.symbol()
        )));
    }
    let both_numbers = a.number_like() && b.number_like();
    let result = match op {
        BinaryOp::Add => match (a, b) {
            (Money, Money | Any) | (Any, Money) => Some(Money),
            (Date, other) | (other, Date) if other.number_like() => Some(Date),
            (DateTime, Duration | Any) | (Duration | Any, DateTime) => Some(DateTime),
            (Duration, Duration | Any) | (Any, Duration) => Some(Duration),
            _ => both_numbers.then_some(Number),
        },
        BinaryOp::Subtract => match (a, b) {
            (Money, Money | Any) | (Any, Money) => Some(Money),
            (Date, Date) => Some(Number),
            (Date, other) if other.number_like() => Some(Date),
            (DateTime, DateTime) => Some(Duration),
            (DateTime, Duration) => Some(DateTime),
            (DateTime, Any) => Some(Any),
            (Duration, Duration | Any) | (Any, Duration) => Some(Duration),
            _ => both_numbers.then_some(Number),
        },
        BinaryOp::Multiply => match (a, b) {
            (Money, Money) | (Duration, Duration) => None,
            (Money, other) | (other, Money) if other.number_like() => Some(Money),
            (Duration, other) | (other, Duration) if other.number_like() => Some(Duration),
            _ => both_numbers.then_some(Number),
        },
        BinaryOp::Divide => match (a, b) {
            (Money, Money) | (Duration, Duration) => Some(Number),
            (Money, other) if other.number_like() => Some(Money),
            (Duration, other) if other.number_like() => Some(Duration),
            _ => both_numbers.then_some(Number),
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

fn comparison(op: BinaryOp, a: &Type, b: &Type) -> Result<Type, FormulaError> {
    if a.is_list() || b.is_list() {
        return Err(mismatch(format!(
            "`{}` cannot compare a list; use CONTAINS, COUNTIF or FILTER",
            op.symbol()
        )));
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
    let ordering = !matches!(op, BinaryOp::Equal | BinaryOp::NotEqual);
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
    for arg in args {
        let element = arg.element();
        let found = match element {
            Type::Money => Type::Money,
            Type::Duration => Type::Duration,
            Type::Number => {
                plain_number = true;
                continue;
            }
            // Skipped at evaluation, or possibly numbers.
            Type::Text | Type::Bool | Type::Any | Type::Reference => continue,
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
                    (other, _) if other.number_like() => Ok(Type::Number),
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
                    | BinaryOp::GreaterOrEqual => comparison(*op, &a, &b),
                }
            }
            Expr::Call { function, args } => self.call(*function, args, in_condition),
        }
    }

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
        match function {
            Function::Logic(function) => match function {
                LogicFn::If => unify(
                    types.get(1).cloned().unwrap_or(Type::Any),
                    types.get(2).cloned().unwrap_or(Type::Any),
                ),
                LogicFn::Ifs => unify_all(types.into_iter().skip(1).step_by(2)),
                LogicFn::Switch => {
                    // Results are every second argument from the third, plus a final default.
                    let count = types.len();
                    let has_default = count.checked_rem(2) == Some(0);
                    let results = types.into_iter().enumerate().filter_map(|(index, t)| {
                        let is_result = index >= 2 && index.checked_rem(2) == Some(0);
                        let is_default = has_default && index.saturating_add(1) == count;
                        (is_result || is_default).then_some(t)
                    });
                    unify_all(results)
                }
                LogicFn::And
                | LogicFn::Or
                | LogicFn::Not
                | LogicFn::Xor
                | LogicFn::IsBlank
                | LogicFn::IsError
                | LogicFn::IsNumber
                | LogicFn::IsText => Ok(Type::Bool),
                LogicFn::IfError | LogicFn::IfBlank => {
                    unify(first, types.get(1).cloned().unwrap_or(Type::Any))
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
                    if first == Type::Money {
                        Ok(Type::Money)
                    } else if first.number_like() {
                        Ok(Type::Number)
                    } else {
                        Err(mismatch(format!(
                            "{name} needs a number, not {}",
                            first.words()
                        )))
                    }
                }
                NumberFn::Sign | NumberFn::Int | NumberFn::Mod | NumberFn::Power => {
                    Ok(Type::Number)
                }
            },
            Function::Text(function) => match function {
                TextFn::Concat
                | TextFn::TextJoin
                | TextFn::Left
                | TextFn::Right
                | TextFn::Mid
                | TextFn::Upper
                | TextFn::Lower
                | TextFn::Trim
                | TextFn::Substitute
                | TextFn::Replace
                | TextFn::Rept => Ok(Type::Text),
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
                TextFn::Len | TextFn::Find | TextFn::Search | TextFn::Value => Ok(Type::Number),
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
                    DateFn::Today
                    | DateFn::Date
                    | DateFn::EDate
                    | DateFn::EoMonth
                    | DateFn::Workday => Ok(Type::Date),
                    DateFn::Now => Ok(Type::DateTime),
                    DateFn::DateAdd => {
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
                                "DATEADD needs a date, not {}",
                                other.words()
                            ))),
                        }
                    }
                    DateFn::DateDif => {
                        unit_ok(2, &["d", "m", "y"])?;
                        Ok(Type::Number)
                    }
                    DateFn::Year
                    | DateFn::Month
                    | DateFn::Day
                    | DateFn::Hour
                    | DateFn::Minute
                    | DateFn::Weekday
                    | DateFn::IsoWeekNum
                    | DateFn::Days
                    | DateFn::NetworkDays
                    | DateFn::ToHours
                    | DateFn::ToDays => Ok(Type::Number),
                    DateFn::Duration => Ok(Type::Duration),
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
                ListFn::First | ListFn::Last | ListFn::Index => Ok(first.element().clone()),
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
                LookupFn::Attr => match first.element() {
                    Type::Reference | Type::Any => Ok(Type::Any),
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

/// Checks a formula and that its result can be stored in a computed field of the given
/// data type.
///
/// # Errors
/// As [`check`], and [`FormulaError::Type`] when the result does not fit the field.
pub fn check_for_field(
    expr: &Expr,
    schema: &dyn Schema,
    field: DataType,
) -> Result<Type, FormulaError> {
    let found = check(expr, schema)?;
    let fits = match (field, &found) {
        (DataType::Link | DataType::Principal | DataType::Attachment | DataType::Json, _) => {
            return Err(mismatch(format!(
                "a computed field cannot be a {field} field"
            )));
        }
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
    };
    if fits {
        Ok(found)
    } else {
        Err(mismatch(format!(
            "the formula returns {}, but the field is a {field} field",
            found.words()
        )))
    }
}

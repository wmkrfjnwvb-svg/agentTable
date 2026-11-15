//! The evaluator.
//!
//! Evaluation is a pure function of the formula, the record, the linked records' values and
//! the [`Env`] (today, now, working days, parameters, reference labels). Nothing reads a
//! clock, so the browser and the server compute the same cell.
//!
//! It is also **bounded**: every step is counted ([`MAX_STEPS`]), lists are capped
//! ([`MAX_LIST_ITEMS`]) and texts are capped ([`MAX_TEXT_CHARS`]). A formula that grows
//! beyond the bounds evaluates to `#LIMIT!` instead of slowing the system down.
//!
//! `IF`, `IFS`, `SWITCH`, `AND`, `OR`, `IFERROR` and `IFBLANK` evaluate only the arguments
//! they need, so `IF({n} = 0, 0, {total} / {n})` never divides by zero.

use crate::ast::{BinaryOp, Expr, Function, ListFn, LogicFn, UnaryOp, MAX_DEPTH};
use crate::error::ErrorValue;
use crate::functions::{dates, lists, numbers, texts};
use crate::value::{Comparison, Val};
use core::cmp::Ordering;
use mt_core::decimal::Decimal;
use mt_core::field::DataType;
use mt_core::id::{FieldId, RefItemId};
use mt_core::time::{Date, Timestamp};
use mt_core::value::Value;
use std::collections::BTreeMap;

/// Maximum number of evaluation steps for one cell.
pub const MAX_STEPS: u32 = 200_000;
/// Maximum number of items in a list (for example the values of linked records).
pub const MAX_LIST_ITEMS: usize = 10_000;
/// Maximum number of characters in a text result.
pub const MAX_TEXT_CHARS: usize = 100_000;

/// The record a formula is evaluated for.
pub trait Record {
    /// The value of a field of the record.
    fn value(&self, field: FieldId) -> Option<&Value>;

    /// The values of `field` in the records linked through `link`, in link order, one per
    /// linked record (`Value::Null` for a linked record without a value).
    fn linked_values(&self, link: FieldId, field: FieldId) -> Vec<Value>;
}

/// Everything a formula can ask the outside world.
pub trait Env {
    /// Today's date in the relevant time zone.
    fn today(&self) -> Date;

    /// The current instant.
    fn now(&self) -> Timestamp;

    /// The instant at which a day starts in the relevant time zone.
    fn start_of_day(&self, date: Date) -> Timestamp;

    /// Whether a day is a working day in the applicable business calendar.
    fn is_workday(&self, date: Date) -> bool;

    /// The effective value of a parameter.
    fn param(&self, key: &str) -> Option<Value>;

    /// The label of a reference item.
    fn label(&self, item: RefItemId) -> Option<String>;

    /// The code of a reference item.
    fn code(&self, item: RefItemId) -> Option<String>;

    /// A custom attribute of a reference item, by the attribute's name.
    fn attribute(&self, item: RefItemId, name: &str) -> Option<Value>;
}

/// The results of computed fields evaluated earlier for the same record. A later formula
/// sees them instead of the stored values, including their errors.
pub type Computed = BTreeMap<FieldId, Result<Value, ErrorValue>>;

pub(crate) fn lift(result: Result<Val, ErrorValue>) -> Val {
    result.unwrap_or_else(Val::Error)
}

/// The first error among the values, looking into lists.
fn first_error(values: &[Val]) -> Option<ErrorValue> {
    values.iter().find_map(|value| match value {
        Val::Error(error) => Some(*error),
        Val::List(items) => items.iter().find_map(Val::error),
        Val::Blank
        | Val::Number(_)
        | Val::Text(_)
        | Val::Bool(_)
        | Val::Date(_)
        | Val::DateTime(_)
        | Val::Duration(_)
        | Val::Money(..)
        | Val::Ref(_) => None,
    })
}

struct Machine<'a> {
    record: &'a dyn Record,
    env: &'a dyn Env,
    computed: Option<&'a Computed>,
    steps: u32,
    current: Option<Val>,
}

impl Machine<'_> {
    fn field(&self, field: FieldId) -> Val {
        if let Some(result) = self.computed.and_then(|computed| computed.get(&field)) {
            return match result {
                Ok(value) => Val::from_value(value),
                Err(error) => Val::Error(*error),
            };
        }
        self.record.value(field).map_or(Val::Blank, Val::from_value)
    }

    fn through(&self, link: FieldId, field: FieldId) -> Val {
        let values = self.record.linked_values(link, field);
        let mut items = Vec::with_capacity(values.len());
        for value in &values {
            // A multi-value field of a linked record contributes its items.
            let converted = Val::from_value(value);
            if let Val::List(inner) = converted {
                items.extend(inner);
            } else {
                items.push(converted);
            }
            if items.len() > MAX_LIST_ITEMS {
                return Val::Error(ErrorValue::Limit);
            }
        }
        Val::List(items)
    }

    fn eval(&mut self, expr: &Expr) -> Val {
        self.steps = self.steps.saturating_add(1);
        if self.steps > MAX_STEPS {
            return Val::Error(ErrorValue::Limit);
        }
        match expr {
            Expr::Number(n) => Val::Number(*n),
            Expr::Text(text) => Val::Text(text.clone()),
            Expr::Bool(b) => Val::Bool(*b),
            Expr::Field(field) => self.field(*field),
            Expr::Through { link, field } => self.through(*link, *field),
            // Outside a list function there is no current item.
            Expr::CurrentValue => self
                .current
                .clone()
                .unwrap_or(Val::Error(ErrorValue::Value)),
            Expr::Unary { op, operand } => {
                let value = self.eval(operand);
                lift(match op {
                    UnaryOp::Negate => Val::Blank.subtract(&value),
                    UnaryOp::Percent => value.divide(&Val::Number(Decimal::ONE_HUNDRED)),
                })
            }
            Expr::Binary { op, left, right } => {
                let a = self.eval(left);
                let b = self.eval(right);
                lift(self.binary(*op, &a, &b))
            }
            Expr::Call { function, args } => self.call(*function, args),
        }
    }

    fn binary(&self, op: BinaryOp, a: &Val, b: &Val) -> Result<Val, ErrorValue> {
        let ordering = |wanted: &[Ordering]| -> Result<Val, ErrorValue> {
            Ok(Val::Bool(match a.compare(b)? {
                Comparison::Ordered(found) => wanted.contains(&found),
                Comparison::Unknown => false,
                Comparison::Unordered(_) | Comparison::Mismatch => return Err(ErrorValue::Value),
            }))
        };
        match op {
            BinaryOp::Add => a.add(b),
            BinaryOp::Subtract => a.subtract(b),
            BinaryOp::Multiply => a.multiply(b),
            BinaryOp::Divide => a.divide(b),
            BinaryOp::Concat => {
                let mut text = a.text(&|item| self.env.label(item))?;
                text.push_str(&b.text(&|item| self.env.label(item))?);
                texts::bounded(text)
            }
            BinaryOp::Equal => a.equals(b).map(Val::Bool),
            BinaryOp::NotEqual => a.equals(b).map(|equal| Val::Bool(!equal)),
            BinaryOp::Less => ordering(&[Ordering::Less]),
            BinaryOp::LessOrEqual => ordering(&[Ordering::Less, Ordering::Equal]),
            BinaryOp::Greater => ordering(&[Ordering::Greater]),
            BinaryOp::GreaterOrEqual => ordering(&[Ordering::Greater, Ordering::Equal]),
        }
    }

    /// Evaluates an argument that must be true or false.
    fn truth(&mut self, expr: &Expr) -> Result<bool, ErrorValue> {
        self.eval(expr).boolean()
    }

    /// The items of a list argument; a single value is a list of one, an empty value a list
    /// of none.
    fn list(&mut self, expr: &Expr) -> Result<Vec<Val>, ErrorValue> {
        match self.eval(expr) {
            Val::Error(error) => Err(error),
            Val::List(items) => Ok(items),
            Val::Blank => Ok(Vec::new()),
            single @ (Val::Number(_)
            | Val::Text(_)
            | Val::Bool(_)
            | Val::Date(_)
            | Val::DateTime(_)
            | Val::Duration(_)
            | Val::Money(..)
            | Val::Ref(_)) => Ok(vec![single]),
        }
    }

    /// Which items of `items` satisfy the criterion written as `criterion`: an expression
    /// using `CurrentValue`, a value to equal, or a text such as `">10"`.
    fn matching(&mut self, items: &[Val], criterion: &Expr) -> Result<Vec<bool>, ErrorValue> {
        let mut uses_current = false;
        criterion.walk(&mut |node| uses_current |= matches!(node, Expr::CurrentValue));
        if uses_current {
            let outer = self.current.take();
            let mut flags = Vec::with_capacity(items.len());
            for item in items {
                self.current = Some(item.clone());
                match self.truth(criterion) {
                    Ok(flag) => flags.push(flag),
                    Err(error) => {
                        self.current = outer;
                        return Err(error);
                    }
                }
            }
            self.current = outer;
            return Ok(flags);
        }
        let value = self.eval(criterion);
        if let Some(error) = value.error() {
            return Err(error);
        }
        let criterion = lists::Criterion::new(value);
        items.iter().map(|item| criterion.accepts(item)).collect()
    }

    fn call(&mut self, function: Function, args: &[Expr]) -> Val {
        // The parser guarantees the argument counts; a hand-built tree may not.
        if crate::parser::check_arity(function, args.len()).is_err() {
            return Val::Error(ErrorValue::Value);
        }
        lift(match function {
            Function::Logic(function) => self.logic(function, args),
            Function::List(function) => self.list_function(function, args),
            Function::Number(function) => self
                .arguments(args)
                .and_then(|values| numbers::call(function, &values)),
            Function::Text(function) => self
                .arguments(args)
                .and_then(|values| texts::call(function, &values, self.env)),
            Function::Date(function) => self
                .arguments(args)
                .and_then(|values| dates::call(function, &values, self.env)),
            Function::Lookup(function) => self
                .arguments(args)
                .and_then(|values| lists::lookup(function, &values, self.env)),
        })
    }

    /// Evaluates every argument; an error in any of them is the result.
    fn arguments(&mut self, args: &[Expr]) -> Result<Vec<Val>, ErrorValue> {
        let values: Vec<Val> = args.iter().map(|expr| self.eval(expr)).collect();
        match first_error(&values) {
            Some(error) => Err(error),
            None => Ok(values),
        }
    }

    /// The logic functions decide which of their arguments to evaluate.
    fn logic(&mut self, function: LogicFn, args: &[Expr]) -> Result<Val, ErrorValue> {
        let arg = |index: usize| args.get(index).ok_or(ErrorValue::Value);
        match function {
            LogicFn::If => Ok(if self.truth(arg(0)?)? {
                self.eval(arg(1)?)
            } else {
                args.get(2)
                    .map_or(Val::Blank, |otherwise| self.eval(otherwise))
            }),
            LogicFn::Ifs => {
                for pair in args.chunks(2) {
                    if let [condition, value] = pair {
                        if self.truth(condition)? {
                            return Ok(self.eval(value));
                        }
                    }
                }
                Err(ErrorValue::NotAvailable)
            }
            LogicFn::Switch => {
                let subject = self.eval(arg(0)?);
                if let Some(error) = subject.error() {
                    return Err(error);
                }
                let rest = args.get(1..).unwrap_or(&[]);
                let mut pairs = rest.chunks_exact(2);
                for pair in pairs.by_ref() {
                    if let [case, result] = pair {
                        let case = self.eval(case);
                        if subject.equals(&case)? {
                            return Ok(self.eval(result));
                        }
                    }
                }
                match pairs.remainder() {
                    [default] => Ok(self.eval(default)),
                    [] | [_, _, ..] => Err(ErrorValue::NotAvailable),
                }
            }
            LogicFn::And => {
                for condition in args {
                    if !self.truth(condition)? {
                        return Ok(Val::Bool(false));
                    }
                }
                Ok(Val::Bool(true))
            }
            LogicFn::Or => {
                for condition in args {
                    if self.truth(condition)? {
                        return Ok(Val::Bool(true));
                    }
                }
                Ok(Val::Bool(false))
            }
            LogicFn::Not => Ok(Val::Bool(!self.truth(arg(0)?)?)),
            LogicFn::Xor => {
                let mut odd = false;
                for condition in args {
                    odd ^= self.truth(condition)?;
                }
                Ok(Val::Bool(odd))
            }
            LogicFn::IfError => {
                let value = self.eval(arg(0)?);
                Ok(if value.error().is_some() {
                    self.eval(arg(1)?)
                } else {
                    value
                })
            }
            LogicFn::IfBlank => {
                let value = self.eval(arg(0)?);
                Ok(if value.is_blank() {
                    self.eval(arg(1)?)
                } else {
                    value
                })
            }
            LogicFn::IsError => Ok(Val::Bool(self.eval(arg(0)?).error().is_some())),
            LogicFn::IsBlank => Ok(Val::Bool(self.eval(arg(0)?).is_blank())),
            LogicFn::IsNumber => Ok(Val::Bool(matches!(self.eval(arg(0)?), Val::Number(_)))),
            LogicFn::IsText => {
                let value = self.eval(arg(0)?);
                Ok(Val::Bool(
                    matches!(value, Val::Text(_)) && !value.is_blank(),
                ))
            }
            LogicFn::Blank => Ok(Val::Blank),
        }
    }

    fn list_function(&mut self, function: ListFn, args: &[Expr]) -> Result<Val, ErrorValue> {
        let arg = |index: usize| args.get(index).ok_or(ErrorValue::Value);
        match function {
            ListFn::Filter => {
                let items = self.list(arg(0)?)?;
                let keep = self.matching(&items, arg(1)?)?;
                let kept = items
                    .into_iter()
                    .zip(keep)
                    .filter_map(|(item, keep)| keep.then_some(item))
                    .collect();
                Ok(Val::List(kept))
            }
            ListFn::CountIf => {
                let items = self.list(arg(0)?)?;
                let count = self
                    .matching(&items, arg(1)?)?
                    .into_iter()
                    .filter(|flag| *flag)
                    .count();
                Ok(Val::Number(Decimal::from(
                    u64::try_from(count).unwrap_or(u64::MAX),
                )))
            }
            ListFn::SumIf | ListFn::AverageIf => {
                let items = self.list(arg(0)?)?;
                let keep = self.matching(&items, arg(1)?)?;
                // With a third argument the matching positions select from that list.
                let source = match args.get(2) {
                    Some(other) => {
                        let other = self.list(other)?;
                        if other.len() != items.len() {
                            return Err(ErrorValue::Value);
                        }
                        other
                    }
                    None => items,
                };
                let selected: Vec<Val> = source
                    .into_iter()
                    .zip(keep)
                    .filter_map(|(item, keep)| keep.then_some(item))
                    .collect();
                let series = numbers::Series::of(selected.iter(), false)?;
                if function == ListFn::SumIf {
                    series.sum()
                } else {
                    series.average()
                }
            }
            ListFn::Unique | ListFn::First | ListFn::Last | ListFn::Index => {
                let values = self.arguments(args)?;
                lists::call(function, &values)
            }
        }
    }
}

/// Evaluates a formula for a record.
#[must_use]
pub fn evaluate(expr: &Expr, record: &dyn Record, env: &dyn Env) -> Val {
    evaluate_with(expr, record, env, None)
}

/// Evaluates a formula for a record, seeing the results of computed fields evaluated
/// earlier for the same record.
#[must_use]
pub fn evaluate_with(
    expr: &Expr,
    record: &dyn Record,
    env: &dyn Env,
    computed: Option<&Computed>,
) -> Val {
    if expr.depth() > MAX_DEPTH {
        return Val::Error(ErrorValue::Limit);
    }
    let mut machine = Machine {
        record,
        env,
        computed,
        steps: 0,
        current: None,
    };
    machine.eval(expr)
}

/// Evaluates a formula to the cell value of a computed field of the given data type.
///
/// # Errors
/// The error value the formula evaluated to, or [`ErrorValue::Value`] when the result is
/// not of the field's kind.
pub fn evaluate_to(
    expr: &Expr,
    record: &dyn Record,
    env: &dyn Env,
    computed: Option<&Computed>,
    target: DataType,
) -> Result<Value, ErrorValue> {
    evaluate_with(expr, record, env, computed).into_value(target, &|item| env.label(item))
}

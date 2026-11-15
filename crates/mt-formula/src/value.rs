//! The values formulas calculate with, and the rules for combining them.
//!
//! # Empty values
//!
//! As in spreadsheets, an empty value counts as `0` in arithmetic, as `""` when joining text
//! and as `FALSE` in logic, and it equals `0`, `""` and `FALSE`. One deliberate difference:
//! an empty **date, time, duration, amount or reference never satisfies an ordering
//! comparison**. `{Due} < TODAY()` is false for a record without a due date, so "no date"
//! is never mistaken for "overdue".
//!
//! # Kinds do not mix silently
//!
//! * Money plus money needs the same currency (`#CURRENCY!` otherwise); money times a number
//!   is money; money divided by money is a number.
//! * A date plus a whole number of days is a date; a date minus a date is a number of days;
//!   a date-time minus a date-time is a duration.
//! * Calculating with an **empty date** gives an empty result, not a wrong one:
//!   `{Due} - {Start}` is empty while there is no start. The evaluator decides this (it
//!   knows which operand is a date); the operations in this module see an empty value as
//!   zero.
//! * Text is not a number unless it is written as one (`"12.5"`); anything else is `#VALUE!`.
//! * Comparing values of different kinds: `=` is false, `<>` is true, an ordering is
//!   `#VALUE!`.
//! * Text compares without regard to case, like everywhere else in the product.
//!
//! Errors are values: an operation on an error gives that error.

use crate::error::ErrorValue;
use core::cmp::Ordering;
use mt_core::condition::fold_text;
use mt_core::decimal::{canonical_string, Decimal, Rounding};
use mt_core::field::DataType;
use mt_core::id::RefItemId;
use mt_core::money::{Currency, Money};
use mt_core::time::{Date, DurationMs, Timestamp};
use mt_core::value::Value;

/// A value during evaluation.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    /// No value.
    Blank,
    /// An exact decimal number (percentages are fractions: 25 % is `0.25`).
    Number(Decimal),
    /// Text.
    Text(String),
    /// True or false.
    Bool(bool),
    /// A calendar day.
    Date(Date),
    /// An instant.
    DateTime(Timestamp),
    /// A length of time.
    Duration(DurationMs),
    /// An amount in a currency, not yet rounded to the currency's minor units.
    Money(Decimal, Currency),
    /// An item of a reference set.
    Ref(RefItemId),
    /// Several values (of linked records, or of a multi-value field). Lists are flat.
    List(Vec<Val>),
    /// An error.
    Error(ErrorValue),
}

/// The result of comparing two values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comparison {
    /// The values are of comparable kinds and stand in this order.
    Ordered(Ordering),
    /// The values are of the same kind but have no order (references), and are equal or not.
    Unordered(bool),
    /// An empty value against a kind that has no "zero" (dates, amounts, references).
    Unknown,
    /// Different kinds.
    Mismatch,
}

/// The whole number in a decimal, if it is one and fits.
pub(crate) fn whole(n: Decimal) -> Option<i64> {
    if n.fract().is_zero() {
        i64::try_from(n.trunc().mantissa()).ok()
    } else {
        None
    }
}

impl Val {
    /// The evaluation value of a cell value.
    #[must_use]
    pub fn from_value(value: &Value) -> Val {
        let ids = |texts: Vec<String>| Val::List(texts.into_iter().map(Val::Text).collect());
        match value {
            Value::Null => Val::Blank,
            Value::Text(text) | Value::LongText(text) => {
                if text.trim().is_empty() {
                    Val::Blank
                } else {
                    Val::Text(text.clone())
                }
            }
            Value::Number(n) | Value::Percent(n) => Val::Number(*n),
            Value::Money(m) => Val::Money(m.amount(), m.currency()),
            Value::Bool(b) => Val::Bool(*b),
            Value::Date(d) => Val::Date(*d),
            Value::DateTime(t) => Val::DateTime(*t),
            Value::Duration(d) => Val::Duration(*d),
            Value::Reference(item) => Val::Ref(*item),
            Value::MultiReference(items) => Val::List(items.iter().map(|i| Val::Ref(*i)).collect()),
            Value::Link(items) => ids(items.iter().map(ToString::to_string).collect()),
            Value::Principal(items) => ids(items.iter().map(ToString::to_string).collect()),
            Value::Attachment(items) => ids(items.iter().map(ToString::to_string).collect()),
            Value::Json(json) => {
                if json.is_null() {
                    Val::Blank
                } else {
                    Val::Text(json.to_string())
                }
            }
        }
    }

    /// The cell value to store for a computed field of the given data type. Amounts are
    /// rounded half-even to the currency's minor units here, once, at the end.
    ///
    /// # Errors
    /// The error the formula evaluated to, or [`ErrorValue::Value`] when the result is not
    /// of the field's kind.
    pub fn into_value(
        self,
        target: DataType,
        labels: &dyn Fn(RefItemId) -> Option<String>,
    ) -> Result<Value, ErrorValue> {
        let wrong = Err(ErrorValue::Value);
        match (self, target) {
            (Val::Error(error), _) => Err(error),
            (Val::Blank, _) => Ok(Value::Null),
            (value, DataType::Text | DataType::LongText) => {
                let text = value.text(labels)?;
                Ok(if target == DataType::Text {
                    Value::Text(text)
                } else {
                    Value::LongText(text)
                }
                .normalized())
            }
            (Val::Number(n), DataType::Number) => Ok(Value::Number(n.normalize())),
            // Calculating with nothing but empty amounts or durations leaves a plain zero:
            // there is no currency to state an amount in, so the cell is empty.
            (Val::Number(n), DataType::Money) if n.is_zero() => Ok(Value::Null),
            (Val::Number(n), DataType::Duration) if n.is_zero() => {
                Ok(Value::Duration(DurationMs::from_millis(0)))
            }
            (Val::Number(n), DataType::Percent) => Ok(Value::Percent(n.normalize())),
            (Val::Money(amount, currency), DataType::Money) => Ok(Value::Money(Money::rounded(
                amount,
                currency,
                Rounding::HalfEven,
            ))),
            (Val::Bool(b), DataType::Bool) => Ok(Value::Bool(b)),
            (Val::Date(d), DataType::Date) => Ok(Value::Date(d)),
            (Val::DateTime(t), DataType::DateTime) => Ok(Value::DateTime(t)),
            (Val::Duration(d), DataType::Duration) => Ok(Value::Duration(d)),
            (Val::Ref(item), DataType::Reference) => Ok(Value::Reference(item)),
            (Val::Ref(item), DataType::MultiReference) => Ok(Value::MultiReference(vec![item])),
            (Val::List(items), DataType::MultiReference) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    match item {
                        Val::Ref(id) => out.push(id),
                        Val::Blank => {}
                        Val::Error(error) => return Err(error),
                        Val::Number(_)
                        | Val::Text(_)
                        | Val::Bool(_)
                        | Val::Date(_)
                        | Val::DateTime(_)
                        | Val::Duration(_)
                        | Val::Money(..)
                        | Val::List(_) => return wrong,
                    }
                }
                Ok(Value::MultiReference(out).normalized())
            }
            (
                Val::Number(_)
                | Val::Text(_)
                | Val::Bool(_)
                | Val::Date(_)
                | Val::DateTime(_)
                | Val::Duration(_)
                | Val::Money(..)
                | Val::Ref(_)
                | Val::List(_),
                _,
            ) => wrong,
        }
    }

    /// Whether the value is empty: no value, blank text or an empty list.
    #[must_use]
    pub fn is_blank(&self) -> bool {
        match self {
            Val::Blank => true,
            Val::Text(text) => text.trim().is_empty(),
            Val::List(items) => items.is_empty(),
            Val::Number(_)
            | Val::Bool(_)
            | Val::Date(_)
            | Val::DateTime(_)
            | Val::Duration(_)
            | Val::Money(..)
            | Val::Ref(_)
            | Val::Error(_) => false,
        }
    }

    /// The error, if the value is one.
    #[must_use]
    pub const fn error(&self) -> Option<ErrorValue> {
        match self {
            Val::Error(error) => Some(*error),
            Val::Blank
            | Val::Number(_)
            | Val::Text(_)
            | Val::Bool(_)
            | Val::Date(_)
            | Val::DateTime(_)
            | Val::Duration(_)
            | Val::Money(..)
            | Val::Ref(_)
            | Val::List(_) => None,
        }
    }

    /// The value as a number: empty is 0, true is 1, text must be written as a number.
    ///
    /// # Errors
    /// The value's own error, or [`ErrorValue::Value`].
    pub fn number(&self) -> Result<Decimal, ErrorValue> {
        match self {
            Val::Blank => Ok(Decimal::ZERO),
            Val::Number(n) => Ok(*n),
            Val::Bool(b) => Ok(if *b { Decimal::ONE } else { Decimal::ZERO }),
            Val::Text(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    return Ok(Decimal::ZERO);
                }
                mt_core::decimal::parse_decimal(trimmed).map_err(|_| ErrorValue::Value)
            }
            Val::Error(error) => Err(*error),
            Val::Date(_)
            | Val::DateTime(_)
            | Val::Duration(_)
            | Val::Money(..)
            | Val::Ref(_)
            | Val::List(_) => Err(ErrorValue::Value),
        }
    }

    /// The value as a whole number.
    ///
    /// # Errors
    /// As [`Val::number`], and [`ErrorValue::Value`] for a number with decimals.
    pub fn integer(&self) -> Result<i64, ErrorValue> {
        whole(self.number()?).ok_or(ErrorValue::Value)
    }

    /// The value as true or false: empty is false, a number is true unless zero, text must
    /// be `TRUE` or `FALSE`.
    ///
    /// # Errors
    /// The value's own error, or [`ErrorValue::Value`].
    pub fn boolean(&self) -> Result<bool, ErrorValue> {
        match self {
            Val::Blank => Ok(false),
            Val::Bool(b) => Ok(*b),
            Val::Number(n) => Ok(!n.is_zero()),
            Val::Text(text) => match text.trim().to_ascii_uppercase().as_str() {
                "TRUE" => Ok(true),
                "FALSE" | "" => Ok(false),
                _ => Err(ErrorValue::Value),
            },
            Val::Error(error) => Err(*error),
            Val::Date(_)
            | Val::DateTime(_)
            | Val::Duration(_)
            | Val::Money(..)
            | Val::Ref(_)
            | Val::List(_) => Err(ErrorValue::Value),
        }
    }

    /// The value as text. `labels` gives the label of a reference item; without one the
    /// item's id is used. The items of a list are joined with `", "`.
    ///
    /// # Errors
    /// The value's own error (or the first error in a list).
    pub fn text(&self, labels: &dyn Fn(RefItemId) -> Option<String>) -> Result<String, ErrorValue> {
        Ok(match self {
            Val::Blank => String::new(),
            Val::Number(n) => canonical_string(n),
            Val::Text(text) => text.clone(),
            Val::Bool(true) => "TRUE".to_owned(),
            Val::Bool(false) => "FALSE".to_owned(),
            Val::Date(d) => d.to_string(),
            Val::DateTime(t) => t.to_string(),
            Val::Duration(d) => d.to_string(),
            Val::Money(amount, currency) => {
                Money::rounded(*amount, *currency, Rounding::HalfEven).to_string()
            }
            Val::Ref(item) => labels(*item).unwrap_or_else(|| item.to_string()),
            Val::List(items) => {
                let mut parts = Vec::with_capacity(items.len());
                for item in items {
                    parts.push(item.text(labels)?);
                }
                parts.join(", ")
            }
            Val::Error(error) => return Err(*error),
        })
    }

    /// Compares two scalar values (see the module documentation for the rules).
    ///
    /// # Errors
    /// An operand's own error; [`ErrorValue::Currency`] for amounts in different currencies;
    /// [`ErrorValue::Value`] for a list.
    pub fn compare(&self, other: &Val) -> Result<Comparison, ErrorValue> {
        use Comparison::{Mismatch, Ordered, Unknown, Unordered};
        if let Some(error) = self.error().or(other.error()) {
            return Err(error);
        }
        // An empty value compared with `value` (never a list or an error: see above and
        // the first arm below).
        let zero_like = |value: &Val| -> Comparison {
            match value {
                Val::Blank => Ordered(Ordering::Equal),
                Val::Number(n) => Ordered(Decimal::ZERO.cmp(n)),
                Val::Text(text) => Ordered("".cmp(fold_text(text).as_str())),
                Val::Bool(b) => Ordered(false.cmp(b)),
                Val::Date(_)
                | Val::DateTime(_)
                | Val::Duration(_)
                | Val::Money(..)
                | Val::Ref(_)
                | Val::List(_)
                | Val::Error(_) => Unknown,
            }
        };
        Ok(match (self, other) {
            (Val::List(_), _) | (_, Val::List(_)) => return Err(ErrorValue::Value),
            (Val::Blank, value) => zero_like(value),
            (value, Val::Blank) => match zero_like(value) {
                Ordered(ordering) => Ordered(ordering.reverse()),
                other @ (Unordered(_) | Unknown | Mismatch) => other,
            },
            (Val::Number(a), Val::Number(b)) => Ordered(a.cmp(b)),
            (Val::Text(a), Val::Text(b)) => Ordered(fold_text(a).cmp(&fold_text(b))),
            (Val::Bool(a), Val::Bool(b)) => Ordered(a.cmp(b)),
            (Val::Date(a), Val::Date(b)) => Ordered(a.cmp(b)),
            (Val::DateTime(a), Val::DateTime(b)) => Ordered(a.cmp(b)),
            (Val::Duration(a), Val::Duration(b)) => Ordered(a.cmp(b)),
            (Val::Money(a, currency_a), Val::Money(b, currency_b)) => {
                if currency_a != currency_b {
                    return Err(ErrorValue::Currency);
                }
                Ordered(a.cmp(b))
            }
            (Val::Ref(a), Val::Ref(b)) => Unordered(a == b),
            _ => Mismatch,
        })
    }

    /// Whether two values are equal as `=` defines it.
    ///
    /// # Errors
    /// As [`Val::compare`].
    pub fn equals(&self, other: &Val) -> Result<bool, ErrorValue> {
        Ok(match self.compare(other)? {
            Comparison::Ordered(ordering) => ordering == Ordering::Equal,
            Comparison::Unordered(equal) => equal,
            Comparison::Unknown | Comparison::Mismatch => false,
        })
    }

    /// Adds two values.
    ///
    /// # Errors
    /// See the module documentation; [`ErrorValue::Num`] when the result does not fit.
    pub fn add(&self, other: &Val) -> Result<Val, ErrorValue> {
        if let Some(error) = self.error().or(other.error()) {
            return Err(error);
        }
        let days = |date: &Date, n: &Val| {
            let count = i32::try_from(n.integer()?).map_err(|_| ErrorValue::Num)?;
            date.checked_add_days(count)
                .map(Val::Date)
                .ok_or(ErrorValue::Num)
        };
        match (self, other) {
            (Val::Money(a, ca), Val::Money(b, cb)) => {
                if ca != cb {
                    return Err(ErrorValue::Currency);
                }
                a.checked_add(*b)
                    .map(|sum| Val::Money(sum, *ca))
                    .ok_or(ErrorValue::Num)
            }
            (Val::Money(a, c), Val::Blank) | (Val::Blank, Val::Money(a, c)) => {
                Ok(Val::Money(*a, *c))
            }
            (Val::Date(d), n @ (Val::Number(_) | Val::Blank | Val::Text(_) | Val::Bool(_)))
            | (n @ (Val::Number(_) | Val::Blank | Val::Text(_) | Val::Bool(_)), Val::Date(d)) => {
                days(d, n)
            }
            (Val::DateTime(t), Val::Duration(d)) | (Val::Duration(d), Val::DateTime(t)) => {
                t.checked_add(*d).map(Val::DateTime).ok_or(ErrorValue::Num)
            }
            (Val::DateTime(t), Val::Blank) | (Val::Blank, Val::DateTime(t)) => {
                Ok(Val::DateTime(*t))
            }
            (Val::Duration(a), Val::Duration(b)) => {
                a.checked_add(*b).map(Val::Duration).ok_or(ErrorValue::Num)
            }
            (Val::Duration(d), Val::Blank) | (Val::Blank, Val::Duration(d)) => {
                Ok(Val::Duration(*d))
            }
            _ => self
                .number()?
                .checked_add(other.number()?)
                .map(Val::Number)
                .ok_or(ErrorValue::Num),
        }
    }

    /// Subtracts `other` from this value.
    ///
    /// # Errors
    /// See the module documentation; [`ErrorValue::Num`] when the result does not fit.
    pub fn subtract(&self, other: &Val) -> Result<Val, ErrorValue> {
        if let Some(error) = self.error().or(other.error()) {
            return Err(error);
        }
        let span = |millis: Option<i64>| {
            millis
                .map(|ms| Val::Duration(DurationMs::from_millis(ms)))
                .ok_or(ErrorValue::Num)
        };
        match (self, other) {
            (Val::Money(a, ca), Val::Money(b, cb)) => {
                if ca != cb {
                    return Err(ErrorValue::Currency);
                }
                a.checked_sub(*b)
                    .map(|difference| Val::Money(difference, *ca))
                    .ok_or(ErrorValue::Num)
            }
            (Val::Money(a, c), Val::Blank) => Ok(Val::Money(*a, *c)),
            (Val::Blank, Val::Money(a, c)) => Ok(Val::Money(a.checked_neg_or_zero(), *c)),
            (Val::Date(a), Val::Date(b)) => Ok(Val::Number(Decimal::from(
                i64::from(a.days()).saturating_sub(i64::from(b.days())),
            ))),
            (Val::Date(d), n @ (Val::Number(_) | Val::Blank | Val::Text(_) | Val::Bool(_))) => {
                let count = n.integer()?.checked_neg().ok_or(ErrorValue::Num)?;
                let count = i32::try_from(count).map_err(|_| ErrorValue::Num)?;
                d.checked_add_days(count)
                    .map(Val::Date)
                    .ok_or(ErrorValue::Num)
            }
            (Val::DateTime(a), Val::DateTime(b)) => span(a.millis().checked_sub(b.millis())),
            (Val::DateTime(t), Val::Duration(d)) => {
                let back =
                    DurationMs::from_millis(d.millis().checked_neg().ok_or(ErrorValue::Num)?);
                t.checked_add(back)
                    .map(Val::DateTime)
                    .ok_or(ErrorValue::Num)
            }
            (Val::DateTime(t), Val::Blank) => Ok(Val::DateTime(*t)),
            (Val::Duration(a), Val::Duration(b)) => span(a.millis().checked_sub(b.millis())),
            (Val::Duration(d), Val::Blank) => Ok(Val::Duration(*d)),
            (Val::Blank, Val::Duration(d)) => span(d.millis().checked_neg()),
            _ => self
                .number()?
                .checked_sub(other.number()?)
                .map(Val::Number)
                .ok_or(ErrorValue::Num),
        }
    }

    /// Multiplies two values.
    ///
    /// # Errors
    /// See the module documentation; [`ErrorValue::Num`] when the result does not fit.
    pub fn multiply(&self, other: &Val) -> Result<Val, ErrorValue> {
        if let Some(error) = self.error().or(other.error()) {
            return Err(error);
        }
        match (self, other) {
            (Val::Money(..), Val::Money(..)) => Err(ErrorValue::Value),
            (Val::Money(amount, currency), factor) | (factor, Val::Money(amount, currency)) => {
                amount
                    .checked_mul(factor.number()?)
                    .map(|product| Val::Money(product, *currency))
                    .ok_or(ErrorValue::Num)
            }
            (Val::Duration(_), Val::Duration(_)) => Err(ErrorValue::Value),
            (Val::Duration(d), factor) | (factor, Val::Duration(d)) => {
                scale_duration(*d, factor.number()?, true)
            }
            _ => self
                .number()?
                .checked_mul(other.number()?)
                .map(Val::Number)
                .ok_or(ErrorValue::Num),
        }
    }

    /// Divides this value by `other`.
    ///
    /// # Errors
    /// [`ErrorValue::DivZero`] for a zero or empty divisor; otherwise as for multiplication.
    pub fn divide(&self, other: &Val) -> Result<Val, ErrorValue> {
        if let Some(error) = self.error().or(other.error()) {
            return Err(error);
        }
        let ratio = |a: Decimal, b: Decimal| {
            if b.is_zero() {
                Err(ErrorValue::DivZero)
            } else {
                a.checked_div(b).ok_or(ErrorValue::Num)
            }
        };
        match (self, other) {
            (Val::Money(a, ca), Val::Money(b, cb)) => {
                if ca != cb {
                    return Err(ErrorValue::Currency);
                }
                ratio(*a, *b).map(Val::Number)
            }
            (Val::Money(amount, currency), divisor) => {
                ratio(*amount, divisor.number()?).map(|quotient| Val::Money(quotient, *currency))
            }
            (Val::Duration(a), Val::Duration(b)) => {
                ratio(Decimal::from(a.millis()), Decimal::from(b.millis())).map(Val::Number)
            }
            (Val::Duration(d), divisor) => scale_duration(*d, divisor.number()?, false),
            // An empty amount or duration is zero of its kind.
            (Val::Blank, Val::Money(b, _)) => ratio(Decimal::ZERO, *b).map(Val::Number),
            (Val::Blank, Val::Duration(b)) => {
                ratio(Decimal::ZERO, Decimal::from(b.millis())).map(Val::Number)
            }
            (_, Val::Money(..) | Val::Duration(_)) => Err(ErrorValue::Value),
            _ => ratio(self.number()?, other.number()?).map(Val::Number),
        }
    }
}

/// A duration times (or divided by) a number, rounded half-even to whole milliseconds.
fn scale_duration(
    duration: DurationMs,
    factor: Decimal,
    multiply: bool,
) -> Result<Val, ErrorValue> {
    let millis = Decimal::from(duration.millis());
    let scaled = if multiply {
        millis.checked_mul(factor).ok_or(ErrorValue::Num)?
    } else if factor.is_zero() {
        return Err(ErrorValue::DivZero);
    } else {
        millis.checked_div(factor).ok_or(ErrorValue::Num)?
    };
    whole(Rounding::HalfEven.round(scaled, 0))
        .map(|ms| Val::Duration(DurationMs::from_millis(ms)))
        .ok_or(ErrorValue::Num)
}

trait NegOrZero {
    fn checked_neg_or_zero(self) -> Self;
}

impl NegOrZero for Decimal {
    /// The negated number (zero stays a positive zero).
    fn checked_neg_or_zero(mut self) -> Self {
        let negative = !self.is_sign_negative() && !self.is_zero();
        self.set_sign_negative(negative);
        self
    }
}

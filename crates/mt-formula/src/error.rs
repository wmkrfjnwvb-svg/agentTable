//! Errors of the crate, and the error values a formula can evaluate to.
//!
//! There are two kinds of failure:
//!
//! * A [`FormulaError`] means the formula *text* is not acceptable: it does not parse, names
//!   an unknown field or function, has the wrong number of arguments, or cannot be typed.
//!   It is reported when the formula is saved, with a position where one is known.
//! * An [`ErrorValue`] is the *result* of evaluating an acceptable formula for one record
//!   (`#DIV/0!` when a divisor happens to be zero). It is a value like any other: it shows in
//!   the cell, propagates through later calculations, and can be caught with `IFERROR`.

use serde::{Deserialize, Serialize};

/// Why a formula text is not acceptable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FormulaError {
    /// The text is not a well-formed formula.
    #[error("syntax error at character {position}: {message}")]
    Syntax {
        /// Character position (0-based) where the problem was noticed.
        position: usize,
        /// What is wrong, in plain English.
        message: String,
    },
    /// A `{name}` does not name a field.
    #[error("there is no field named `{name}` (character {position})")]
    UnknownField {
        /// The name as written.
        name: String,
        /// Character position of the reference.
        position: usize,
    },
    /// A name followed by `(` is not a function.
    #[error("there is no function named `{name}` (character {position})")]
    UnknownFunction {
        /// The name as written.
        name: String,
        /// Character position of the call.
        position: usize,
    },
    /// A function was called with too few or too many arguments.
    #[error("{function} takes {expected} argument(s), but {found} were given")]
    Arity {
        /// The function.
        function: &'static str,
        /// What it accepts, in words (`"2"`, `"1 to 3"`, `"at least 1"`).
        expected: String,
        /// How many were given.
        found: usize,
    },
    /// The parts of the formula do not fit together.
    #[error("{0}")]
    Type(String),
    /// Computed fields depend on each other in a circle.
    #[error("circular reference: {path}")]
    Cycle {
        /// The fields on the circle, in order, as text.
        path: String,
    },
    /// A size limit was exceeded.
    #[error("the formula is too large: {0}")]
    Limit(&'static str),
    /// The stored formula of a computed field cannot be read or written.
    #[error("the formula of `{field}` cannot be read: {reason}")]
    Stored {
        /// The field's name.
        field: String,
        /// What is wrong with the stored form.
        reason: String,
    },
}

impl FormulaError {
    pub(crate) fn syntax(position: usize, message: impl Into<String>) -> Self {
        FormulaError::Syntax {
            position,
            message: message.into(),
        }
    }

    pub(crate) fn typing(message: impl Into<String>) -> Self {
        FormulaError::Type(message.into())
    }
}

/// An error a formula evaluated to. Shown in the cell by its code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ErrorValue {
    /// Division by zero.
    #[serde(rename = "#DIV/0!")]
    DivZero,
    /// A value of the wrong kind (text where a number is needed, …).
    #[serde(rename = "#VALUE!")]
    Value,
    /// A field the formula refers to no longer exists.
    #[serde(rename = "#REF!")]
    Ref,
    /// A number outside the representable range, or an impossible date.
    #[serde(rename = "#NUM!")]
    Num,
    /// Amounts in different currencies were combined.
    #[serde(rename = "#CURRENCY!")]
    Currency,
    /// Nothing matched (`IFS` without a true condition, `SWITCH` without a case, a missing
    /// parameter).
    #[serde(rename = "#N/A")]
    NotAvailable,
    /// The calculation was stopped because it grew too large.
    #[serde(rename = "#LIMIT!")]
    Limit,
    /// The field depends on itself.
    #[serde(rename = "#CYCLE!")]
    Cycle,
}

impl ErrorValue {
    /// Every error value.
    pub const ALL: &'static [ErrorValue] = &[
        ErrorValue::DivZero,
        ErrorValue::Value,
        ErrorValue::Ref,
        ErrorValue::Num,
        ErrorValue::Currency,
        ErrorValue::NotAvailable,
        ErrorValue::Limit,
        ErrorValue::Cycle,
    ];

    /// The code shown in a cell.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            ErrorValue::DivZero => "#DIV/0!",
            ErrorValue::Value => "#VALUE!",
            ErrorValue::Ref => "#REF!",
            ErrorValue::Num => "#NUM!",
            ErrorValue::Currency => "#CURRENCY!",
            ErrorValue::NotAvailable => "#N/A",
            ErrorValue::Limit => "#LIMIT!",
            ErrorValue::Cycle => "#CYCLE!",
        }
    }

    /// What the code means, for a tooltip.
    #[must_use]
    pub const fn explanation(self) -> &'static str {
        match self {
            ErrorValue::DivZero => "The formula divides by zero or by an empty value.",
            ErrorValue::Value => {
                "A value has the wrong kind, for example text where a number is needed."
            }
            ErrorValue::Ref => "The formula refers to a field that no longer exists.",
            ErrorValue::Num => "The result is too large, or the date does not exist.",
            ErrorValue::Currency => "Amounts in different currencies cannot be combined.",
            ErrorValue::NotAvailable => "No case matched, or the value asked for is not available.",
            ErrorValue::Limit => "The calculation was stopped because it grew too large.",
            ErrorValue::Cycle => "The field depends on itself.",
        }
    }
}

impl core::fmt::Display for ErrorValue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.code())
    }
}

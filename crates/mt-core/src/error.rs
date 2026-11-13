//! The crate error type.
//!
//! `mt-core` has exactly one error enum, [`CoreError`]. Errors are plain values: nothing in this
//! crate panics. Structured *validation findings* (a value that breaks a field rule) are not
//! errors of this kind; they are reported as [`crate::validate::ValidationIssue`] data.

use crate::field::DataType;

/// Maximum number of characters of user input echoed back inside an error.
const ECHO_LIMIT: usize = 64;

/// Shortens user input before it is stored in an error, so that errors stay small and never
/// carry megabytes of pasted text.
pub(crate) fn clip(input: &str) -> String {
    if input.chars().count() <= ECHO_LIMIT {
        return input.to_owned();
    }
    let mut out: String = input.chars().take(ECHO_LIMIT).collect();
    out.push('…');
    out
}

/// Every failure `mt-core` can return.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoreError {
    /// A textual ID could not be parsed, or ID generation got an out-of-range time.
    #[error("invalid {kind} id `{input}`: {reason}")]
    InvalidId {
        /// The expected prefix of the ID kind (for example `rec`).
        kind: &'static str,
        /// The offending input (clipped).
        input: String,
        /// Why it was rejected.
        reason: &'static str,
    },
    /// A date, timestamp or duration is outside the supported range.
    #[error("time value {0} is out of the supported range")]
    TimeOutOfRange(i64),
    /// A date, timestamp or duration string is malformed.
    #[error("invalid date/time `{input}`: {reason}")]
    InvalidTime {
        /// The offending input (clipped).
        input: String,
        /// Why it was rejected.
        reason: &'static str,
    },
    /// A decimal string is malformed or does not fit the exact decimal type.
    #[error("invalid decimal `{input}`: {reason}")]
    InvalidDecimal {
        /// The offending input (clipped).
        input: String,
        /// Why it was rejected.
        reason: &'static str,
    },
    /// A code is not a member of an embedded code list (ISO 4217, ISO 3166, …) or of an enum's
    /// textual names.
    #[error("unknown {kind} `{code}`")]
    UnknownCode {
        /// The code list or enum name.
        kind: &'static str,
        /// The offending input (clipped).
        code: String,
    },
    /// A money amount has more decimal places than its currency allows.
    #[error("{currency} allows {max_decimals} decimal places, amount has {actual}")]
    MoneyScale {
        /// ISO 4217 code.
        currency: &'static str,
        /// Minor-unit digits of the currency.
        max_decimals: u8,
        /// Decimal places of the offending amount.
        actual: u32,
    },
    /// User input could not be parsed as a value of the given data type.
    #[error("cannot read `{input}` as {data_type}: {reason}")]
    Parse {
        /// The target data type.
        data_type: DataType,
        /// The offending input (clipped).
        input: String,
        /// Why it was rejected.
        reason: &'static str,
    },
    /// A text pattern or numbering pattern is malformed.
    #[error("invalid pattern `{pattern}` at character {position}: {reason}")]
    InvalidPattern {
        /// The offending pattern (clipped).
        pattern: String,
        /// Zero-based character position of the problem.
        position: usize,
        /// Why it was rejected.
        reason: &'static str,
    },
    /// A field definition (or one of its parts) is inconsistent.
    #[error("invalid definition: {reason}")]
    InvalidDefinition {
        /// Why it was rejected.
        reason: String,
    },
    /// A state transition is not allowed.
    #[error("transition `{action}` is not allowed from `{from}`")]
    InvalidTransition {
        /// Current state name.
        from: &'static str,
        /// Requested action name.
        action: &'static str,
    },
    /// A value cannot be converted to another data type.
    #[error("cannot convert {from} to {to}: {reason}")]
    Conversion {
        /// Source data type.
        from: DataType,
        /// Target data type.
        to: DataType,
        /// Why it failed.
        reason: &'static str,
    },
    /// A numeric result does not fit the exact decimal or integer type.
    #[error("arithmetic overflow while {0}")]
    Overflow(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn std_clip_keeps_short_input() {
        assert_eq!(clip("abc"), "abc");
    }

    #[test]
    fn edge_clip_cuts_long_input_on_char_boundary() {
        let long = "é".repeat(200);
        let c = clip(&long);
        assert_eq!(c.chars().count(), ECHO_LIMIT + 1);
        assert!(c.ends_with('…'));
    }

    #[test]
    fn std_errors_display() {
        let e = CoreError::InvalidId { kind: "rec", input: "x".into(), reason: "bad" };
        assert_eq!(e.to_string(), "invalid rec id `x`: bad");
        assert_eq!(CoreError::TimeOutOfRange(5).to_string(), "time value 5 is out of the supported range");
        let e = CoreError::MoneyScale { currency: "JPY", max_decimals: 0, actual: 2 };
        assert!(e.to_string().contains("JPY allows 0"));
        let e = CoreError::Conversion { from: DataType::Bool, to: DataType::Date, reason: "no" };
        assert_eq!(e.to_string(), "cannot convert bool to date: no");
        assert!(CoreError::Overflow("adding").to_string().contains("adding"));
        let e = CoreError::InvalidTransition { from: "accepted", action: "reject" };
        assert!(e.to_string().contains("accepted"));
    }
}

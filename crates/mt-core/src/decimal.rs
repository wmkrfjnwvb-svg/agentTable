//! Exact decimal helpers.
//!
//! All numbers and money amounts in the platform are exact decimals ([`Decimal`], re-exported
//! from `rust_decimal`: 96-bit mantissa, scale 0–28). Floats are never used.
//!
//! **Canonical text form** (used in JSON, in database `TEXT` columns and in the journal):
//! an optional `-`, at least one digit, and optionally `.` followed by at least one digit;
//! no exponent, no `+`, no thousands separators, no trailing fractional zeros, and zero is `0`
//! (never `-0`). [`canonical_string`] writes it; [`parse_decimal`] reads it (and also accepts
//! trailing zeros and leading zeros, which it normalises away).

use crate::error::{clip, CoreError};
pub use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;

/// How to round when digits must be dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rounding {
    /// Round half to even ("banker's rounding"): 0.5 → 0, 1.5 → 2, 2.5 → 2.
    HalfEven,
    /// Round half away from zero ("commercial rounding"): 0.5 → 1, -0.5 → -1.
    HalfUp,
    /// Truncate toward zero.
    Down,
    /// Round away from zero.
    Up,
}

impl Rounding {
    /// Rounds `value` to `decimals` decimal places with this mode.
    #[must_use]
    pub fn round(self, value: Decimal, decimals: u32) -> Decimal {
        let strategy = match self {
            Rounding::HalfEven => RoundingStrategy::MidpointNearestEven,
            Rounding::HalfUp => RoundingStrategy::MidpointAwayFromZero,
            Rounding::Down => RoundingStrategy::ToZero,
            Rounding::Up => RoundingStrategy::AwayFromZero,
        };
        value.round_dp_with_strategy(decimals, strategy)
    }
}

/// Writes the canonical text form of a decimal (see the module documentation).
#[must_use]
pub fn canonical_string(value: &Decimal) -> String {
    value.normalize().to_string()
}

/// Number of decimal places of the value after trailing zeros are removed.
#[must_use]
pub fn decimal_places(value: &Decimal) -> u32 {
    value.normalize().scale()
}

/// Parses a plain decimal: `-?digits(.digits)?`.
///
/// No exponent, sign `+`, spaces or separators are accepted; use
/// [`crate::coerce::parse_input`] for forgiving user input.
///
/// # Errors
/// [`CoreError::InvalidDecimal`] if the text is malformed, has more than 28 decimal places, or
/// does not fit 96 bits.
pub fn parse_decimal(input: &str) -> Result<Decimal, CoreError> {
    let err = |reason| CoreError::InvalidDecimal {
        input: clip(input),
        reason,
    };
    let unsigned = input.strip_prefix('-').unwrap_or(input);
    let (int_part, frac_part) = match unsigned.split_once('.') {
        Some((int_part, frac_part)) => (int_part, Some(frac_part)),
        None => (unsigned, None),
    };
    let all_digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !all_digits(int_part) {
        return Err(err("expected digits"));
    }
    if let Some(frac) = frac_part {
        if !all_digits(frac) {
            return Err(err("expected digits after the decimal point"));
        }
        if frac.len() > 28 {
            return Err(err("more than 28 decimal places"));
        }
    }
    Decimal::from_str_exact(input)
        .map(|d| d.normalize())
        .map_err(|_| err("number is too large"))
}

/// `#[serde(with = "mt_core::decimal::serde_canonical")]`: a decimal as a canonical string.
///
/// Deserialisation accepts **only strings**, so a JSON float can never sneak in.
pub mod serde_canonical {
    use super::{canonical_string, parse_decimal, Decimal};
    use serde::Deserialize;

    /// Serialises the decimal as its canonical string.
    ///
    /// # Errors
    /// Only those of the serializer.
    pub fn serialize<S: serde::Serializer>(
        value: &Decimal,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&canonical_string(value))
    }

    /// Reads a decimal from a string.
    ///
    /// # Errors
    /// If the input is not a string holding a plain decimal.
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Decimal, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        parse_decimal(&text).map_err(serde::de::Error::custom)
    }
}

/// Like [`serde_canonical`], for `Option<Decimal>` (`null` ↔ `None`).
pub mod serde_canonical_opt {
    use super::{canonical_string, parse_decimal, Decimal};
    use serde::Deserialize;

    /// Serialises `Some` as a canonical string and `None` as `null`.
    ///
    /// # Errors
    /// Only those of the serializer.
    pub fn serialize<S: serde::Serializer>(
        value: &Option<Decimal>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(d) => serializer.serialize_some(&canonical_string(d)),
            None => serializer.serialize_none(),
        }
    }

    /// Reads an optional decimal from a string or `null`.
    ///
    /// # Errors
    /// If the input is neither `null` nor a string holding a plain decimal.
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Decimal>, D::Error> {
        let text = <Option<std::borrow::Cow<'de, str>>>::deserialize(deserializer)?;
        match text {
            Some(t) => parse_decimal(&t)
                .map(Some)
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        parse_decimal(s).unwrap()
    }

    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Holder {
        #[serde(with = "serde_canonical")]
        a: Decimal,
        #[serde(with = "serde_canonical_opt")]
        b: Option<Decimal>,
    }

    #[test]
    fn std_parse_and_canonical() {
        assert_eq!(canonical_string(&d("1.50")), "1.5");
        assert_eq!(canonical_string(&d("-0")), "0");
        assert_eq!(canonical_string(&d("-0.00")), "0");
        assert_eq!(canonical_string(&d("007")), "7");
        assert_eq!(canonical_string(&d("100")), "100");
        assert_eq!(canonical_string(&d("-12.345")), "-12.345");
        assert_eq!(decimal_places(&d("1.500")), 1);
        assert_eq!(decimal_places(&d("100")), 0);
    }

    #[test]
    fn std_rounding_modes() {
        assert_eq!(Rounding::HalfEven.round(d("2.5"), 0), d("2"));
        assert_eq!(Rounding::HalfEven.round(d("1.5"), 0), d("2"));
        assert_eq!(Rounding::HalfUp.round(d("2.5"), 0), d("3"));
        assert_eq!(Rounding::HalfUp.round(d("-2.5"), 0), d("-3"));
        assert_eq!(Rounding::Down.round(d("2.99"), 1), d("2.9"));
        assert_eq!(Rounding::Down.round(d("-2.99"), 1), d("-2.9"));
        assert_eq!(Rounding::Up.round(d("2.01"), 1), d("2.1"));
        assert_eq!(Rounding::Up.round(d("-2.01"), 1), d("-2.1"));
    }

    #[test]
    fn std_serde_strings_only() {
        let h = Holder {
            a: d("1.50"),
            b: Some(d("2")),
        };
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(json, r#"{"a":"1.5","b":"2"}"#);
        assert_eq!(serde_json::from_str::<Holder>(&json).unwrap(), h);
        let none = Holder { a: d("0"), b: None };
        let json = serde_json::to_string(&none).unwrap();
        assert_eq!(json, r#"{"a":"0","b":null}"#);
        assert_eq!(serde_json::from_str::<Holder>(&json).unwrap(), none);
    }

    #[test]
    fn invalid_serde_rejects_numbers() {
        assert!(serde_json::from_str::<Holder>(r#"{"a":1.5,"b":null}"#).is_err());
        assert!(serde_json::from_str::<Holder>(r#"{"a":"1","b":2}"#).is_err());
        assert!(serde_json::from_str::<Holder>(r#"{"a":"x","b":null}"#).is_err());
        assert!(serde_json::from_str::<Holder>(r#"{"a":"1","b":"x"}"#).is_err());
    }

    #[test]
    fn invalid_parse() {
        for bad in [
            "", "-", ".", "1.", ".5", "+1", "1e3", "1,000", " 1", "1 ", "--1", "1.2.3", "0x10",
            "１",
        ] {
            assert!(parse_decimal(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn edge_limits() {
        // 28 decimal places are fine, 29 are not.
        assert!(parse_decimal("0.0000000000000000000000000001").is_ok());
        assert!(parse_decimal("0.00000000000000000000000000001").is_err());
        // 96-bit mantissa limit.
        assert!(parse_decimal("79228162514264337593543950335").is_ok());
        assert!(parse_decimal("79228162514264337593543950336").is_err());
        assert!(parse_decimal("-79228162514264337593543950335").is_ok());
        // Too many significant digits for the mantissa even though each part is plain.
        assert!(parse_decimal("123456789012345678901234567890.5").is_err());
    }
}

//! Currencies (ISO 4217) and money amounts.
//!
//! A [`Money`] is an exact decimal amount plus a [`Currency`]. Each currency knows its number
//! of minor-unit digits (USD 2, JPY 0, KRW 0, BHD 3, …) and [`Money::new`] refuses amounts with
//! more decimal places than the currency has, so "¥10.5" cannot be stored by accident.

use crate::decimal::{decimal_places, Decimal, Rounding};
use crate::error::{clip, CoreError};
use core::fmt;
use core::str::FromStr;

/// The embedded ISO 4217 list: `(code, minor-unit digits)`, sorted by code.
///
/// `None` means the standard defines no minor unit (precious metals, test and "no currency"
/// codes); such codes accept any number of decimal places. The list holds the active codes
/// plus `ANG` and `BGN`, which were withdrawn only recently and still appear in live data.
pub const ISO_4217: &[(&str, Option<u8>)] = &[
    ("AED", Some(2)),
    ("AFN", Some(2)),
    ("ALL", Some(2)),
    ("AMD", Some(2)),
    ("ANG", Some(2)),
    ("AOA", Some(2)),
    ("ARS", Some(2)),
    ("AUD", Some(2)),
    ("AWG", Some(2)),
    ("AZN", Some(2)),
    ("BAM", Some(2)),
    ("BBD", Some(2)),
    ("BDT", Some(2)),
    ("BGN", Some(2)),
    ("BHD", Some(3)),
    ("BIF", Some(0)),
    ("BMD", Some(2)),
    ("BND", Some(2)),
    ("BOB", Some(2)),
    ("BOV", Some(2)),
    ("BRL", Some(2)),
    ("BSD", Some(2)),
    ("BTN", Some(2)),
    ("BWP", Some(2)),
    ("BYN", Some(2)),
    ("BZD", Some(2)),
    ("CAD", Some(2)),
    ("CDF", Some(2)),
    ("CHE", Some(2)),
    ("CHF", Some(2)),
    ("CHW", Some(2)),
    ("CLF", Some(4)),
    ("CLP", Some(0)),
    ("CNY", Some(2)),
    ("COP", Some(2)),
    ("COU", Some(2)),
    ("CRC", Some(2)),
    ("CUP", Some(2)),
    ("CVE", Some(2)),
    ("CZK", Some(2)),
    ("DJF", Some(0)),
    ("DKK", Some(2)),
    ("DOP", Some(2)),
    ("DZD", Some(2)),
    ("EGP", Some(2)),
    ("ERN", Some(2)),
    ("ETB", Some(2)),
    ("EUR", Some(2)),
    ("FJD", Some(2)),
    ("FKP", Some(2)),
    ("GBP", Some(2)),
    ("GEL", Some(2)),
    ("GHS", Some(2)),
    ("GIP", Some(2)),
    ("GMD", Some(2)),
    ("GNF", Some(0)),
    ("GTQ", Some(2)),
    ("GYD", Some(2)),
    ("HKD", Some(2)),
    ("HNL", Some(2)),
    ("HTG", Some(2)),
    ("HUF", Some(2)),
    ("IDR", Some(2)),
    ("ILS", Some(2)),
    ("INR", Some(2)),
    ("IQD", Some(3)),
    ("IRR", Some(2)),
    ("ISK", Some(0)),
    ("JMD", Some(2)),
    ("JOD", Some(3)),
    ("JPY", Some(0)),
    ("KES", Some(2)),
    ("KGS", Some(2)),
    ("KHR", Some(2)),
    ("KMF", Some(0)),
    ("KPW", Some(2)),
    ("KRW", Some(0)),
    ("KWD", Some(3)),
    ("KYD", Some(2)),
    ("KZT", Some(2)),
    ("LAK", Some(2)),
    ("LBP", Some(2)),
    ("LKR", Some(2)),
    ("LRD", Some(2)),
    ("LSL", Some(2)),
    ("LYD", Some(3)),
    ("MAD", Some(2)),
    ("MDL", Some(2)),
    ("MGA", Some(2)),
    ("MKD", Some(2)),
    ("MMK", Some(2)),
    ("MNT", Some(2)),
    ("MOP", Some(2)),
    ("MRU", Some(2)),
    ("MUR", Some(2)),
    ("MVR", Some(2)),
    ("MWK", Some(2)),
    ("MXN", Some(2)),
    ("MXV", Some(2)),
    ("MYR", Some(2)),
    ("MZN", Some(2)),
    ("NAD", Some(2)),
    ("NGN", Some(2)),
    ("NIO", Some(2)),
    ("NOK", Some(2)),
    ("NPR", Some(2)),
    ("NZD", Some(2)),
    ("OMR", Some(3)),
    ("PAB", Some(2)),
    ("PEN", Some(2)),
    ("PGK", Some(2)),
    ("PHP", Some(2)),
    ("PKR", Some(2)),
    ("PLN", Some(2)),
    ("PYG", Some(0)),
    ("QAR", Some(2)),
    ("RON", Some(2)),
    ("RSD", Some(2)),
    ("RUB", Some(2)),
    ("RWF", Some(0)),
    ("SAR", Some(2)),
    ("SBD", Some(2)),
    ("SCR", Some(2)),
    ("SDG", Some(2)),
    ("SEK", Some(2)),
    ("SGD", Some(2)),
    ("SHP", Some(2)),
    ("SLE", Some(2)),
    ("SOS", Some(2)),
    ("SRD", Some(2)),
    ("SSP", Some(2)),
    ("STN", Some(2)),
    ("SVC", Some(2)),
    ("SYP", Some(2)),
    ("SZL", Some(2)),
    ("THB", Some(2)),
    ("TJS", Some(2)),
    ("TMT", Some(2)),
    ("TND", Some(3)),
    ("TOP", Some(2)),
    ("TRY", Some(2)),
    ("TTD", Some(2)),
    ("TWD", Some(2)),
    ("TZS", Some(2)),
    ("UAH", Some(2)),
    ("UGX", Some(0)),
    ("USD", Some(2)),
    ("USN", Some(2)),
    ("UYI", Some(0)),
    ("UYU", Some(2)),
    ("UYW", Some(4)),
    ("UZS", Some(2)),
    ("VED", Some(2)),
    ("VES", Some(2)),
    ("VND", Some(0)),
    ("VUV", Some(0)),
    ("WST", Some(2)),
    ("XAF", Some(0)),
    ("XAG", None),
    ("XAU", None),
    ("XBA", None),
    ("XBB", None),
    ("XBC", None),
    ("XBD", None),
    ("XCD", Some(2)),
    ("XCG", Some(2)),
    ("XDR", None),
    ("XOF", Some(0)),
    ("XPD", None),
    ("XPF", Some(0)),
    ("XPT", None),
    ("XSU", None),
    ("XTS", None),
    ("XUA", None),
    ("XXX", None),
    ("YER", Some(2)),
    ("ZAR", Some(2)),
    ("ZMW", Some(2)),
    ("ZWG", Some(2)),
];

/// An ISO 4217 currency code that is a member of [`ISO_4217`]. Text and serde form: the
/// three upper-case letters.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Currency {
    code: &'static str,
    minor_units: Option<u8>,
}

impl Currency {
    /// Looks up a currency by its exact upper-case code.
    ///
    /// # Errors
    /// [`CoreError::UnknownCode`] if the code is not in the embedded list.
    pub fn new(code: &str) -> Result<Self, CoreError> {
        ISO_4217
            .binary_search_by(|(c, _)| (*c).cmp(code))
            .ok()
            .and_then(|index| ISO_4217.get(index))
            .map(|(code, minor_units)| Currency {
                code,
                minor_units: *minor_units,
            })
            .ok_or_else(|| CoreError::UnknownCode {
                kind: "ISO 4217 currency",
                code: clip(code),
            })
    }

    /// The three-letter code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        self.code
    }

    /// Number of minor-unit digits (USD 2, JPY 0, BHD 3); `None` if the standard defines none.
    #[must_use]
    pub const fn minor_units(self) -> Option<u8> {
        self.minor_units
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code)
    }
}

impl fmt::Debug for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code)
    }
}

impl FromStr for Currency {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Currency::new(s)
    }
}

impl serde::Serialize for Currency {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.code)
    }
}

impl<'de> serde::Deserialize<'de> for Currency {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str> as serde::Deserialize>::deserialize(deserializer)?;
        Currency::new(&text).map_err(serde::de::Error::custom)
    }
}

/// An exact amount in one currency.
///
/// Invariant (enforced by every constructor and by deserialisation): the amount has no more
/// decimal places than the currency's minor unit. Serde form:
/// `{"amount":"12.5","currency":"USD"}` (amount as a canonical decimal string).
#[derive(Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "MoneyWire", into = "MoneyWire")]
pub struct Money {
    amount: Decimal,
    currency: Currency,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MoneyWire {
    #[serde(with = "crate::decimal::serde_canonical")]
    amount: Decimal,
    currency: Currency,
}

impl TryFrom<MoneyWire> for Money {
    type Error = CoreError;
    fn try_from(wire: MoneyWire) -> Result<Self, Self::Error> {
        Money::new(wire.amount, wire.currency)
    }
}

impl From<Money> for MoneyWire {
    fn from(money: Money) -> Self {
        MoneyWire {
            amount: money.amount,
            currency: money.currency,
        }
    }
}

impl Money {
    /// Creates an amount, checking the currency's minor-unit rule.
    ///
    /// # Errors
    /// [`CoreError::MoneyScale`] if the amount has more decimal places than the currency
    /// allows (for example `10.5` JPY). Trailing zeros do not count.
    pub fn new(amount: Decimal, currency: Currency) -> Result<Self, CoreError> {
        let amount = amount.normalize();
        if let Some(max) = currency.minor_units() {
            let actual = amount.scale();
            if actual > u32::from(max) {
                return Err(CoreError::MoneyScale {
                    currency: currency.code(),
                    max_decimals: max,
                    actual,
                });
            }
        }
        Ok(Money { amount, currency })
    }

    /// Creates an amount, rounding to the currency's minor unit with the given mode.
    /// Currencies without a minor unit are not rounded.
    #[must_use]
    pub fn rounded(amount: Decimal, currency: Currency, rounding: Rounding) -> Self {
        let amount = match currency.minor_units() {
            Some(max) => rounding.round(amount, u32::from(max)),
            None => amount,
        };
        Money {
            amount: amount.normalize(),
            currency,
        }
    }

    /// The amount (normalised: no trailing zeros).
    #[must_use]
    pub const fn amount(&self) -> Decimal {
        self.amount
    }

    /// The currency.
    #[must_use]
    pub const fn currency(&self) -> Currency {
        self.currency
    }

    /// The amount as a whole number of minor units (cents for USD, yen for JPY).
    /// `None` for currencies without a minor unit or if the number does not fit.
    #[must_use]
    pub fn minor_amount(&self) -> Option<i128> {
        let digits = u32::from(self.currency.minor_units()?);
        let mut scaled = self.amount;
        scaled.rescale(digits);
        (scaled.scale() == digits && scaled == self.amount).then(|| scaled.mantissa())
    }

    /// The amount written with exactly the currency's minor-unit digits, then the code:
    /// `12.50 USD`, `1000 JPY`.
    #[must_use]
    pub fn to_plain_string(&self) -> String {
        let places = self
            .currency
            .minor_units()
            .map_or_else(|| decimal_places(&self.amount), u32::from);
        format!("{} {}", fixed_places(&self.amount, places), self.currency)
    }
}

/// Writes `value` with exactly `places` decimal places (padding with zeros). The value must
/// already have at most `places` places; otherwise it is written unchanged.
pub(crate) fn fixed_places(value: &Decimal, places: u32) -> String {
    let mut scaled = value.normalize();
    if scaled.scale() < places {
        scaled.rescale(places);
    }
    scaled.to_string()
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_plain_string())
    }
}

impl fmt::Debug for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_plain_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decimal::parse_decimal;

    fn d(s: &str) -> Decimal {
        parse_decimal(s).unwrap()
    }
    fn cur(s: &str) -> Currency {
        Currency::new(s).unwrap()
    }

    #[test]
    fn std_table_is_sorted_and_well_formed() {
        assert!(ISO_4217.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(ISO_4217
            .iter()
            .all(|(c, _)| c.len() == 3 && c.bytes().all(|b| b.is_ascii_uppercase())));
        assert!(ISO_4217.iter().all(|(_, m)| m.is_none_or(|m| m <= 4)));
        assert_eq!(ISO_4217.len(), 179);
    }

    #[test]
    fn std_minor_units() {
        for (code, units) in [
            ("USD", 2),
            ("EUR", 2),
            ("INR", 2),
            ("JPY", 0),
            ("KRW", 0),
            ("VND", 0),
            ("BHD", 3),
            ("KWD", 3),
            ("CLF", 4),
        ] {
            assert_eq!(cur(code).minor_units(), Some(units), "{code}");
        }
        assert_eq!(cur("XAU").minor_units(), None);
        assert_eq!(cur("USD").code(), "USD");
        assert_eq!(cur("USD").to_string(), "USD");
        assert_eq!(format!("{:?}", cur("USD")), "USD");
        assert_eq!("JPY".parse::<Currency>().unwrap(), cur("JPY"));
        assert!(cur("EUR") < cur("USD"));
    }

    #[test]
    fn std_money_new_and_display() {
        let m = Money::new(d("12.50"), cur("USD")).unwrap();
        assert_eq!(m.amount(), d("12.5"));
        assert_eq!(m.currency(), cur("USD"));
        assert_eq!(m.to_string(), "12.50 USD");
        assert_eq!(format!("{m:?}"), "12.50 USD");
        assert_eq!(m.minor_amount(), Some(1250));
        let yen = Money::new(d("1000"), cur("JPY")).unwrap();
        assert_eq!(yen.to_string(), "1000 JPY");
        assert_eq!(yen.minor_amount(), Some(1000));
        // Trailing zeros do not violate the minor-unit rule.
        assert!(Money::new(d("1000.00"), cur("JPY")).is_ok());
        assert_eq!(
            Money::new(d("-0.005"), cur("BHD")).unwrap().minor_amount(),
            Some(-5)
        );
        let gold = Money::new(d("1.23456"), cur("XAU")).unwrap();
        assert_eq!(gold.to_string(), "1.23456 XAU");
        assert_eq!(gold.minor_amount(), None);
    }

    #[test]
    fn std_money_rounded() {
        assert_eq!(
            Money::rounded(d("10.5"), cur("JPY"), Rounding::HalfEven).amount(),
            d("10")
        );
        assert_eq!(
            Money::rounded(d("10.5"), cur("JPY"), Rounding::HalfUp).amount(),
            d("11")
        );
        assert_eq!(
            Money::rounded(d("1.005"), cur("USD"), Rounding::HalfUp).amount(),
            d("1.01")
        );
        assert_eq!(
            Money::rounded(d("1.23456"), cur("XAU"), Rounding::Down).amount(),
            d("1.23456")
        );
    }

    #[test]
    fn std_money_serde() {
        let m = Money::new(d("12.50"), cur("USD")).unwrap();
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(json, r#"{"amount":"12.5","currency":"USD"}"#);
        assert_eq!(serde_json::from_str::<Money>(&json).unwrap(), m);
    }

    #[test]
    fn invalid_currency_and_money() {
        for bad in ["", "usd", "US", "USDX", "ABC", "ZZZ", "U$D", "€"] {
            assert!(Currency::new(bad).is_err(), "{bad}");
        }
        let err = Money::new(d("10.5"), cur("JPY")).unwrap_err();
        assert_eq!(
            err,
            CoreError::MoneyScale {
                currency: "JPY",
                max_decimals: 0,
                actual: 1
            }
        );
        assert!(Money::new(d("0.001"), cur("USD")).is_err());
        assert!(serde_json::from_str::<Money>(r#"{"amount":"10.5","currency":"JPY"}"#).is_err());
        assert!(serde_json::from_str::<Money>(r#"{"amount":10,"currency":"JPY"}"#).is_err());
        assert!(serde_json::from_str::<Money>(r#"{"amount":"10","currency":"jpy"}"#).is_err());
        assert!(
            serde_json::from_str::<Money>(r#"{"amount":"10","currency":"JPY","x":1}"#).is_err()
        );
        assert!(serde_json::from_str::<Currency>("5").is_err());
    }

    #[test]
    fn edge_money_extremes() {
        let max = Decimal::MAX;
        let m = Money::new(max, cur("JPY")).unwrap();
        assert_eq!(
            m.minor_amount(),
            Some(79_228_162_514_264_337_593_543_950_335)
        );
        // Cannot scale MAX by 100: no minor amount, but no panic either.
        assert_eq!(Money::new(max, cur("USD")).unwrap().minor_amount(), None);
        assert_eq!(fixed_places(&d("1.234"), 2), "1.234");
        assert_eq!(fixed_places(&d("1"), 3), "1.000");
    }
}

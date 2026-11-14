//! The standard format validation library.
//!
//! Each validator is a pure function `fn(&str) -> Result<(), FormatViolation>` that documents
//! the rule it implements. Validators check the **canonical written form** (no surrounding
//! spaces); [`FormatId::normalize`] turns common user spellings into that form first
//! (for example an IBAN typed with spaces, or a Korean business number without hyphens).
//!
//! [`FormatId`] names every format so that a field definition can refer to one
//! (`validation.format`), and [`FormatId::validate`] dispatches to the function.
//!
//! Sensitive personal identifiers (for example resident registration numbers) are
//! deliberately not offered. See the crate README for the confidence level of each rule.

mod contact;
mod ids;

pub use contact::{validate_email, validate_phone_e164, validate_url};
pub use ids::{
    validate_bic, validate_de_vat_id, validate_iban, validate_in_gstin, validate_in_pan,
    validate_jp_corporate_number, validate_jp_invoice_number, validate_kr_business_number,
    validate_postal_de, validate_postal_in, validate_postal_jp, validate_postal_kr,
    validate_postal_us, validate_us_aba_routing, validate_us_ein, validate_uuid,
};

use crate::codes::{is_country_code, is_language_code};
use crate::error::{clip, CoreError};
use crate::money::Currency;
use core::fmt;
use core::str::FromStr;

/// Why a text does not satisfy a format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormatViolation {
    /// The text is empty.
    Empty,
    /// The text (or one of its parts) is too short or too long.
    Length,
    /// The text contains a character the format does not allow.
    Character,
    /// The parts of the text are not arranged as the format requires.
    Structure,
    /// The check digit or checksum is wrong.
    Checksum,
    /// A code inside the text (country, currency, prefix…) is not a known one.
    UnknownCode,
    /// A numeric part is outside its allowed range.
    Range,
}

impl FormatViolation {
    /// A short human-readable explanation.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            FormatViolation::Empty => "is empty",
            FormatViolation::Length => "has the wrong length",
            FormatViolation::Character => "contains a character that is not allowed",
            FormatViolation::Structure => "is not structured as the format requires",
            FormatViolation::Checksum => "has a wrong check digit",
            FormatViolation::UnknownCode => "contains an unknown code",
            FormatViolation::Range => "has a part outside its allowed range",
        }
    }
}

impl fmt::Display for FormatViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

macro_rules! define_formats {
    ($( $(#[$meta:meta])* $variant:ident => $name:literal; )+) => {
        /// A named standard format. Text and serde form: the dotted name (for example
        /// `"tax.in_gstin"`).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum FormatId {
            $( $(#[$meta])* $variant, )+
        }

        impl FormatId {
            /// Every format, in declaration order.
            pub const ALL: &'static [FormatId] = &[ $( FormatId::$variant, )+ ];

            /// The stable dotted name.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $( FormatId::$variant => $name, )+
                }
            }
        }
    };
}

define_formats! {
    /// Email address (practical subset of RFC 5321): [`validate_email`].
    Email => "contact.email";
    /// Web address with scheme `http` or `https`: [`validate_url`].
    Url => "contact.url";
    /// International phone number in E.164 form: [`validate_phone_e164`].
    PhoneE164 => "contact.phone_e164";
    /// US ZIP or ZIP+4: [`validate_postal_us`].
    PostalUs => "postal.us";
    /// Japanese postal code: [`validate_postal_jp`].
    PostalJp => "postal.jp";
    /// Korean postal code: [`validate_postal_kr`].
    PostalKr => "postal.kr";
    /// German postal code: [`validate_postal_de`].
    PostalDe => "postal.de";
    /// Indian PIN code: [`validate_postal_in`].
    PostalIn => "postal.in";
    /// IBAN with mod-97 check: [`validate_iban`].
    Iban => "bank.iban";
    /// SWIFT/BIC code: [`validate_bic`].
    Bic => "bank.bic";
    /// US ABA routing number with checksum: [`validate_us_aba_routing`].
    UsAbaRouting => "bank.us_aba";
    /// US Employer Identification Number (format only): [`validate_us_ein`].
    UsEin => "tax.us_ein";
    /// German VAT ID (USt-IdNr): [`validate_de_vat_id`].
    DeVatId => "tax.de_vat";
    /// Japanese corporate number with check digit: [`validate_jp_corporate_number`].
    JpCorporateNumber => "tax.jp_corporate";
    /// Japanese qualified invoice issuer number (`T` + 13 digits): [`validate_jp_invoice_number`].
    JpInvoiceNumber => "tax.jp_invoice";
    /// Korean business registration number with checksum: [`validate_kr_business_number`].
    KrBusinessNumber => "tax.kr_brn";
    /// Indian GSTIN with checksum: [`validate_in_gstin`].
    InGstin => "tax.in_gstin";
    /// Indian PAN (format only): [`validate_in_pan`].
    InPan => "tax.in_pan";
    /// ISO 3166-1 alpha-2 country code (upper case).
    CountryCode => "code.country";
    /// ISO 4217 currency code (upper case).
    CurrencyCode => "code.currency";
    /// ISO 639-1 language code (lower case).
    LanguageCode => "code.language";
    /// UUID in the 8-4-4-4-12 hexadecimal form: [`validate_uuid`].
    Uuid => "code.uuid";
}

/// How [`FormatId::normalize`] rewrites input before validation.
enum Tidy {
    /// Trim only.
    Trim,
    /// Trim and upper-case.
    Upper,
    /// Trim and lower-case.
    Lower,
    /// Remove spaces, then upper-case.
    CompactUpper,
    /// Remove spaces, hyphens, dots and parentheses.
    Digits,
}

impl FormatId {
    /// Checks `text` (in canonical written form) against this format.
    ///
    /// # Errors
    /// The [`FormatViolation`] describing the first problem found.
    pub fn validate(self, text: &str) -> Result<(), FormatViolation> {
        match self {
            FormatId::Email => validate_email(text),
            FormatId::Url => validate_url(text),
            FormatId::PhoneE164 => validate_phone_e164(text),
            FormatId::PostalUs => validate_postal_us(text),
            FormatId::PostalJp => validate_postal_jp(text),
            FormatId::PostalKr => validate_postal_kr(text),
            FormatId::PostalDe => validate_postal_de(text),
            FormatId::PostalIn => validate_postal_in(text),
            FormatId::Iban => validate_iban(text),
            FormatId::Bic => validate_bic(text),
            FormatId::UsAbaRouting => validate_us_aba_routing(text),
            FormatId::UsEin => validate_us_ein(text),
            FormatId::DeVatId => validate_de_vat_id(text),
            FormatId::JpCorporateNumber => validate_jp_corporate_number(text),
            FormatId::JpInvoiceNumber => validate_jp_invoice_number(text),
            FormatId::KrBusinessNumber => validate_kr_business_number(text),
            FormatId::InGstin => validate_in_gstin(text),
            FormatId::InPan => validate_in_pan(text),
            FormatId::CountryCode => membership(text, is_country_code(text)),
            FormatId::CurrencyCode => membership(text, Currency::new(text).is_ok()),
            FormatId::LanguageCode => membership(text, is_language_code(text)),
            FormatId::Uuid => validate_uuid(text),
        }
    }

    /// Rewrites common user spellings into the canonical written form. It never makes an
    /// invalid value valid by guessing; it only removes harmless decoration:
    ///
    /// * all formats: surrounding white space is removed;
    /// * IBAN, BIC, German VAT ID: inner spaces removed, upper-cased;
    /// * GSTIN, PAN, invoice number, country and currency codes: upper-cased;
    /// * UUID and language codes: lower-cased; email: the domain is lower-cased;
    /// * phone: spaces, hyphens, dots and parentheses removed;
    /// * EIN → `NN-NNNNNNN`, Korean business number → `NNN-NN-NNNNN`, Japanese postal code
    ///   → `NNN-NNNN` (a leading `〒` is dropped), ZIP+4 → `NNNNN-NNNN`.
    #[must_use]
    pub fn normalize(self, input: &str) -> String {
        let tidy = match self {
            FormatId::Url
            | FormatId::Email
            | FormatId::PostalKr
            | FormatId::PostalDe
            | FormatId::PostalIn
            | FormatId::UsAbaRouting
            | FormatId::JpCorporateNumber => Tidy::Trim,
            FormatId::InGstin
            | FormatId::InPan
            | FormatId::JpInvoiceNumber
            | FormatId::CountryCode
            | FormatId::CurrencyCode => Tidy::Upper,
            FormatId::Uuid | FormatId::LanguageCode => Tidy::Lower,
            FormatId::Iban | FormatId::Bic | FormatId::DeVatId => Tidy::CompactUpper,
            FormatId::PhoneE164
            | FormatId::PostalUs
            | FormatId::PostalJp
            | FormatId::UsEin
            | FormatId::KrBusinessNumber => Tidy::Digits,
        };
        let trimmed = input.trim();
        let tidied: String = match tidy {
            Tidy::Trim => trimmed.to_owned(),
            Tidy::Upper => trimmed.to_ascii_uppercase(),
            Tidy::Lower => trimmed.to_ascii_lowercase(),
            Tidy::CompactUpper => trimmed
                .chars()
                .filter(|c| *c != ' ')
                .collect::<String>()
                .to_ascii_uppercase(),
            Tidy::Digits => trimmed
                .chars()
                .filter(|c| !matches!(c, ' ' | '-' | '.' | '(' | ')'))
                .collect(),
        };
        // Re-insert the canonical separators where the digit count is unambiguous.
        let all_digits = tidied.bytes().all(|b| b.is_ascii_digit());
        let regroup = |groups: &[usize]| -> String {
            let mut out = String::new();
            let mut rest = tidied.as_str();
            for (index, size) in groups.iter().enumerate() {
                if index > 0 {
                    out.push('-');
                }
                out.push_str(rest.get(..*size).unwrap_or(""));
                rest = rest.get(*size..).unwrap_or("");
            }
            out
        };
        match self {
            FormatId::Email => match trimmed.rsplit_once('@') {
                Some((local, domain)) => format!("{local}@{}", domain.to_ascii_lowercase()),
                None => tidied,
            },
            FormatId::UsEin if all_digits && tidied.len() == 9 => regroup(&[2, 7]),
            FormatId::KrBusinessNumber if all_digits && tidied.len() == 10 => regroup(&[3, 2, 5]),
            FormatId::PostalUs if all_digits && tidied.len() == 9 => regroup(&[5, 4]),
            FormatId::PostalJp => {
                let digits = tidied.strip_prefix('〒').unwrap_or(&tidied).trim_start();
                if digits.len() == 7 && digits.bytes().all(|b| b.is_ascii_digit()) {
                    format!(
                        "{}-{}",
                        digits.get(..3).unwrap_or(""),
                        digits.get(3..).unwrap_or("")
                    )
                } else {
                    tidied
                }
            }
            FormatId::Url
            | FormatId::PhoneE164
            | FormatId::PostalUs
            | FormatId::PostalKr
            | FormatId::PostalDe
            | FormatId::PostalIn
            | FormatId::Iban
            | FormatId::Bic
            | FormatId::UsAbaRouting
            | FormatId::UsEin
            | FormatId::DeVatId
            | FormatId::JpCorporateNumber
            | FormatId::JpInvoiceNumber
            | FormatId::KrBusinessNumber
            | FormatId::InGstin
            | FormatId::InPan
            | FormatId::CountryCode
            | FormatId::CurrencyCode
            | FormatId::LanguageCode
            | FormatId::Uuid => tidied,
        }
    }

    /// Normalises, then validates; returns the canonical text on success.
    ///
    /// # Errors
    /// The [`FormatViolation`] of the normalised text.
    pub fn normalize_and_validate(self, input: &str) -> Result<String, FormatViolation> {
        let text = self.normalize(input);
        self.validate(&text).map(|()| text)
    }
}

fn membership(text: &str, is_member: bool) -> Result<(), FormatViolation> {
    if text.is_empty() {
        Err(FormatViolation::Empty)
    } else if is_member {
        Ok(())
    } else {
        Err(FormatViolation::UnknownCode)
    }
}

impl fmt::Display for FormatId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for FormatId {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        FormatId::ALL
            .iter()
            .copied()
            .find(|format| format.as_str() == s)
            .ok_or_else(|| CoreError::UnknownCode {
                kind: "format",
                code: clip(s),
            })
    }
}

impl serde::Serialize for FormatId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for FormatId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str> as serde::Deserialize>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One valid and one invalid vector per format: proves the dispatch table is wired.
    const VECTORS: &[(FormatId, &str, &str)] = &[
        (FormatId::Email, "a@example.com", "a@"),
        (FormatId::Url, "https://example.com", "example.com"),
        (FormatId::PhoneE164, "+819012345678", "09012345678"),
        (FormatId::PostalUs, "94105", "9410"),
        (FormatId::PostalJp, "100-0001", "100-001"),
        (FormatId::PostalKr, "04524", "4524"),
        (FormatId::PostalDe, "10115", "1011"),
        (FormatId::PostalIn, "110001", "010001"),
        (
            FormatId::Iban,
            "GB82WEST12345698765432",
            "GB82WEST12345698765431",
        ),
        (FormatId::Bic, "DEUTDEFF", "DEUTDEF"),
        (FormatId::UsAbaRouting, "021000021", "021000022"),
        (FormatId::UsEin, "12-3456789", "123-456789"),
        (FormatId::DeVatId, "DE136695976", "DE136695977"),
        (
            FormatId::JpCorporateNumber,
            "7000012050002",
            "7000012050003",
        ),
        (FormatId::JpInvoiceNumber, "T7000012050002", "7000012050002"),
        (FormatId::KrBusinessNumber, "124-81-00998", "124-81-00999"),
        (FormatId::InGstin, "27AAPFU0939F1ZV", "27AAPFU0939F1ZW"),
        (FormatId::InPan, "AAPFU0939F", "AAPFU0939"),
        (FormatId::CountryCode, "JP", "JA"),
        (FormatId::CurrencyCode, "JPY", "JPN"),
        (FormatId::LanguageCode, "ja", "jp"),
        (
            FormatId::Uuid,
            "123e4567-e89b-12d3-a456-426614174000",
            "123e4567e89b12d3a456426614174000",
        ),
    ];

    #[test]
    fn std_every_format_has_positive_and_negative_vectors() {
        assert_eq!(VECTORS.len(), FormatId::ALL.len());
        for (index, (format, good, bad)) in VECTORS.iter().enumerate() {
            assert_eq!(FormatId::ALL[index], *format);
            assert_eq!(format.validate(good), Ok(()), "{format} {good}");
            assert!(format.validate(bad).is_err(), "{format} {bad}");
            assert_eq!(format.validate(""), Err(FormatViolation::Empty), "{format}");
            assert_eq!(
                format.normalize_and_validate(good).as_deref(),
                Ok(*good),
                "{format}"
            );
        }
    }

    #[test]
    fn std_names_round_trip() {
        let mut seen = std::collections::BTreeSet::new();
        for format in FormatId::ALL {
            assert!(seen.insert(format.as_str()));
            assert_eq!(format.as_str().parse::<FormatId>().unwrap(), *format);
            assert_eq!(format.to_string(), format.as_str());
            let json = serde_json::to_string(format).unwrap();
            assert_eq!(serde_json::from_str::<FormatId>(&json).unwrap(), *format);
        }
        assert!("nope".parse::<FormatId>().is_err());
        assert!(serde_json::from_str::<FormatId>("\"nope\"").is_err());
    }

    #[test]
    fn std_normalize() {
        let cases: &[(FormatId, &str, &str)] = &[
            (
                FormatId::Iban,
                " gb82 west 1234 5698 7654 32 ",
                "GB82WEST12345698765432",
            ),
            (FormatId::Bic, "deut de ff", "DEUTDEFF"),
            (FormatId::DeVatId, "de 136 695 976", "DE136695976"),
            (FormatId::Email, " User@Example.COM ", "User@example.com"),
            (FormatId::PhoneE164, "+81 (90) 1234-5678", "+819012345678"),
            (FormatId::PhoneE164, "+1 415.555.2671", "+14155552671"),
            (FormatId::UsEin, "123456789", "12-3456789"),
            (FormatId::UsEin, "12-3456789", "12-3456789"),
            (FormatId::KrBusinessNumber, "1248100998", "124-81-00998"),
            (FormatId::KrBusinessNumber, "124 81 00998", "124-81-00998"),
            (FormatId::PostalJp, "〒100-0001", "100-0001"),
            (FormatId::PostalJp, "〒 1000001", "100-0001"),
            (FormatId::PostalJp, "1000001", "100-0001"),
            (FormatId::PostalUs, "941051234", "94105-1234"),
            (FormatId::PostalUs, " 94105 ", "94105"),
            (FormatId::InGstin, "27aapfu0939f1zv", "27AAPFU0939F1ZV"),
            (FormatId::InPan, "aapfu0939f", "AAPFU0939F"),
            (
                FormatId::JpInvoiceNumber,
                "t7000012050002",
                "T7000012050002",
            ),
            (FormatId::CountryCode, " jp ", "JP"),
            (FormatId::CurrencyCode, "jpy", "JPY"),
            (FormatId::LanguageCode, "JA", "ja"),
            (
                FormatId::Uuid,
                "123E4567-E89B-12D3-A456-426614174000",
                "123e4567-e89b-12d3-a456-426614174000",
            ),
            (
                FormatId::Url,
                " https://Example.com/A ",
                "https://Example.com/A",
            ),
            (FormatId::PostalKr, " 04524 ", "04524"),
            (FormatId::PostalDe, "10115", "10115"),
            (FormatId::PostalIn, "110001", "110001"),
            (FormatId::UsAbaRouting, " 021000021", "021000021"),
            (
                FormatId::JpCorporateNumber,
                "7000012050002 ",
                "7000012050002",
            ),
        ];
        for (format, input, expected) in cases {
            assert_eq!(format.normalize(input), *expected, "{format} {input:?}");
            assert_eq!(
                format.normalize_and_validate(input).as_deref(),
                Ok(*expected),
                "{format} {input:?}"
            );
        }
    }

    #[test]
    fn anomalous_normalize_does_not_repair_bad_input() {
        // Wrong digit counts are left alone and then fail validation.
        assert_eq!(FormatId::UsEin.normalize("12345678"), "12345678");
        assert!(FormatId::UsEin.normalize_and_validate("12345678").is_err());
        assert_eq!(FormatId::KrBusinessNumber.normalize("12481009"), "12481009");
        assert_eq!(FormatId::PostalJp.normalize("〒10-0001"), "〒100001");
        assert!(FormatId::PostalJp
            .normalize_and_validate("〒10-0001")
            .is_err());
        assert_eq!(FormatId::Email.normalize("no-at-sign"), "no-at-sign");
        assert_eq!(FormatId::PostalUs.normalize("9410-51234"), "94105-1234");
        assert_eq!(
            FormatId::Iban.normalize_and_validate("GB82 WEST 1234 5698 7654 31"),
            Err(FormatViolation::Checksum)
        );
    }

    #[test]
    fn std_violation_messages() {
        let all = [
            FormatViolation::Empty,
            FormatViolation::Length,
            FormatViolation::Character,
            FormatViolation::Structure,
            FormatViolation::Checksum,
            FormatViolation::UnknownCode,
            FormatViolation::Range,
        ];
        let mut seen = std::collections::BTreeSet::new();
        for v in all {
            assert!(seen.insert(v.to_string()));
            assert_eq!(v.to_string(), v.message());
        }
        assert_eq!(
            serde_json::to_string(&FormatViolation::UnknownCode).unwrap(),
            "\"unknown_code\""
        );
    }
}

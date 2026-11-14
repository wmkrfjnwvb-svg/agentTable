//! Postal codes, bank identifiers, business/tax identifiers and UUIDs.

use super::FormatViolation;
use crate::codes::is_country_code;

/// The decimal digits of `text`, or the right violation if it is empty or has other characters.
fn digits(text: &str) -> Result<Vec<u32>, FormatViolation> {
    if text.is_empty() {
        return Err(FormatViolation::Empty);
    }
    text.chars()
        .map(|c| {
            if c.is_ascii_digit() {
                c.to_digit(10)
            } else {
                None
            }
            .ok_or(FormatViolation::Character)
        })
        .collect()
}

/// Exactly `len` decimal digits.
fn fixed_digits(text: &str, len: usize) -> Result<Vec<u32>, FormatViolation> {
    let found = digits(text)?;
    if found.len() == len {
        Ok(found)
    } else {
        Err(FormatViolation::Length)
    }
}

/// Sum of `digit × weight` over the paired prefix of both slices.
fn weighted_sum(digits: &[u32], weights: &[u32]) -> u32 {
    digits.iter().zip(weights).fold(0_u32, |sum, (d, w)| {
        sum.saturating_add(d.saturating_mul(*w))
    })
}

/// Value of a character in base 36 (`0-9` then `A-Z`), upper case only.
fn base36(c: char) -> Option<u32> {
    if c.is_ascii_digit() || c.is_ascii_uppercase() {
        c.to_digit(36)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------------------------
// Postal codes
// ---------------------------------------------------------------------------------------------

/// US ZIP code: five digits (`94105`) or ZIP+4 (`94105-1234`).
///
/// # Errors
/// The first [`FormatViolation`] found.
pub fn validate_postal_us(text: &str) -> Result<(), FormatViolation> {
    match text.split_once('-') {
        None => fixed_digits(text, 5).map(|_| ()),
        Some((zip, plus4)) => {
            fixed_digits(zip, 5)?;
            fixed_digits(plus4, 4).map(|_| ()).map_err(|v| match v {
                FormatViolation::Empty => FormatViolation::Length,
                FormatViolation::Length
                | FormatViolation::Character
                | FormatViolation::Structure
                | FormatViolation::Checksum
                | FormatViolation::UnknownCode
                | FormatViolation::Range => v,
            })
        }
    }
}

/// Japanese postal code: `NNN-NNNN` (seven digits with a hyphen after the third). The `〒`
/// mark and the unhyphenated form are handled by [`super::FormatId::normalize`].
///
/// # Errors
/// The first [`FormatViolation`] found.
pub fn validate_postal_jp(text: &str) -> Result<(), FormatViolation> {
    if text.is_empty() {
        return Err(FormatViolation::Empty);
    }
    let (head, tail) = text.split_once('-').ok_or(FormatViolation::Structure)?;
    if head.len() != 3 || tail.len() != 4 {
        return Err(FormatViolation::Length);
    }
    digits(head)?;
    digits(tail).map(|_| ())
}

/// Korean postal code: five digits (the system in force since August 2015).
///
/// # Errors
/// The first [`FormatViolation`] found.
pub fn validate_postal_kr(text: &str) -> Result<(), FormatViolation> {
    fixed_digits(text, 5).map(|_| ())
}

/// German postal code (PLZ): five digits, from `01001` to `99998`.
///
/// # Errors
/// The first [`FormatViolation`] found.
pub fn validate_postal_de(text: &str) -> Result<(), FormatViolation> {
    fixed_digits(text, 5)?;
    if ("01001"..="99998").contains(&text) {
        Ok(())
    } else {
        Err(FormatViolation::Range)
    }
}

/// Indian PIN code: six digits, the first of which is `1`–`9`.
///
/// # Errors
/// The first [`FormatViolation`] found.
pub fn validate_postal_in(text: &str) -> Result<(), FormatViolation> {
    let found = fixed_digits(text, 6)?;
    if found.first() == Some(&0) {
        Err(FormatViolation::Range)
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Banking
// ---------------------------------------------------------------------------------------------

/// IBAN lengths per country, from the SWIFT IBAN registry (sorted by country code).
const IBAN_LENGTHS: &[(&str, usize)] = &[
    ("AD", 24),
    ("AE", 23),
    ("AL", 28),
    ("AT", 20),
    ("AZ", 28),
    ("BA", 20),
    ("BE", 16),
    ("BG", 22),
    ("BH", 22),
    ("BR", 29),
    ("BY", 28),
    ("CH", 21),
    ("CR", 22),
    ("CY", 28),
    ("CZ", 24),
    ("DE", 22),
    ("DK", 18),
    ("DO", 28),
    ("EE", 20),
    ("EG", 29),
    ("ES", 24),
    ("FI", 18),
    ("FO", 18),
    ("FR", 27),
    ("GB", 22),
    ("GE", 22),
    ("GI", 23),
    ("GL", 18),
    ("GR", 27),
    ("GT", 28),
    ("HR", 21),
    ("HU", 28),
    ("IE", 22),
    ("IL", 23),
    ("IQ", 23),
    ("IS", 26),
    ("IT", 27),
    ("JO", 30),
    ("KW", 30),
    ("KZ", 20),
    ("LB", 28),
    ("LC", 32),
    ("LI", 21),
    ("LT", 20),
    ("LU", 20),
    ("LV", 21),
    ("MC", 27),
    ("MD", 24),
    ("ME", 22),
    ("MK", 19),
    ("MR", 27),
    ("MT", 31),
    ("MU", 30),
    ("NL", 18),
    ("NO", 15),
    ("PK", 24),
    ("PL", 28),
    ("PS", 29),
    ("PT", 25),
    ("QA", 29),
    ("RO", 24),
    ("RS", 22),
    ("SA", 24),
    ("SC", 31),
    ("SE", 24),
    ("SI", 19),
    ("SK", 24),
    ("SM", 27),
    ("ST", 25),
    ("SV", 28),
    ("TL", 23),
    ("TN", 24),
    ("TR", 26),
    ("UA", 29),
    ("VA", 22),
    ("VG", 24),
    ("XK", 20),
];

/// IBAN (ISO 13616) in electronic form: no spaces, upper case.
///
/// Rule:
/// 1. 15–34 characters: two letters (country), two digits (check digits), then letters and
///    digits (the BBAN);
/// 2. the country must be an ISO 3166 code (or `XK`); for the countries in the embedded
///    registry table the total length must be the registered one;
/// 3. check digits must be `02`–`98`;
/// 4. **mod-97**: move the first four characters to the end, replace each letter by its
///    number (`A`=10 … `Z`=35) and read the result as one decimal number; its remainder
///    modulo 97 must be 1.
///
/// The national BBAN layout and national check digits are not checked.
///
/// # Errors
/// The first [`FormatViolation`] found.
pub fn validate_iban(text: &str) -> Result<(), FormatViolation> {
    if text.is_empty() {
        return Err(FormatViolation::Empty);
    }
    if !text
        .bytes()
        .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase())
    {
        return Err(FormatViolation::Character);
    }
    if !(15..=34).contains(&text.len()) {
        return Err(FormatViolation::Length);
    }
    let country = text.get(..2).unwrap_or("");
    let check = text.get(2..4).unwrap_or("");
    if !country.bytes().all(|b| b.is_ascii_uppercase())
        || !check.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(FormatViolation::Structure);
    }
    match IBAN_LENGTHS.binary_search_by(|(c, _)| (*c).cmp(country)) {
        Ok(index) => {
            if IBAN_LENGTHS
                .get(index)
                .is_some_and(|(_, len)| *len != text.len())
            {
                return Err(FormatViolation::Length);
            }
        }
        Err(_) => {
            if !is_country_code(country) {
                return Err(FormatViolation::UnknownCode);
            }
        }
    }
    if !("02"..="98").contains(&check) {
        return Err(FormatViolation::Checksum);
    }
    let rearranged = text.chars().skip(4).chain(text.chars().take(4));
    let mut remainder = 0_u32;
    for c in rearranged {
        let value = base36(c).unwrap_or(0);
        let shift = if value < 10 { 10 } else { 100 };
        remainder = remainder.saturating_mul(shift).saturating_add(value) % 97;
    }
    if remainder == 1 {
        Ok(())
    } else {
        Err(FormatViolation::Checksum)
    }
}

/// SWIFT/BIC business identifier code (ISO 9362).
///
/// Rule: 8 or 11 upper-case characters: four letters or digits (party prefix; ISO 9362:2014
/// allows digits, earlier editions letters only), two letters (ISO 3166 country code, or
/// `XK`), two letters or digits (location), and optionally three letters or digits (branch).
/// Whether the code is actually registered with SWIFT is not checked.
///
/// # Errors
/// The first [`FormatViolation`] found.
pub fn validate_bic(text: &str) -> Result<(), FormatViolation> {
    if text.is_empty() {
        return Err(FormatViolation::Empty);
    }
    if !text
        .bytes()
        .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase())
    {
        return Err(FormatViolation::Character);
    }
    if text.len() != 8 && text.len() != 11 {
        return Err(FormatViolation::Length);
    }
    let country = text.get(4..6).unwrap_or("");
    if is_country_code(country) || country == "XK" {
        Ok(())
    } else {
        Err(FormatViolation::UnknownCode)
    }
}

/// US ABA routing transit number.
///
/// Rule: nine digits `d1…d9`; the first two digits must be 00–12, 21–32, 61–72 or 80
/// (Federal Reserve routing symbol ranges); checksum
/// `3·(d1+d4+d7) + 7·(d2+d5+d8) + (d3+d6+d9) ≡ 0 (mod 10)`.
///
/// # Errors
/// The first [`FormatViolation`] found.
pub fn validate_us_aba_routing(text: &str) -> Result<(), FormatViolation> {
    let found = fixed_digits(text, 9)?;
    let prefix = text
        .get(..2)
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(99);
    if !matches!(prefix, 0..=12 | 21..=32 | 61..=72 | 80) {
        return Err(FormatViolation::Range);
    }
    if weighted_sum(&found, &[3, 7, 1, 3, 7, 1, 3, 7, 1]) % 10 == 0 {
        Ok(())
    } else {
        Err(FormatViolation::Checksum)
    }
}

// ---------------------------------------------------------------------------------------------
// Business and tax identifiers
// ---------------------------------------------------------------------------------------------

/// US Employer Identification Number (EIN), **format only**.
///
/// Rule: `NN-NNNNNNN` (two digits, hyphen, seven digits); the prefix `00` is never issued.
/// The EIN has no check digit. The list of valid IRS campus prefixes changes over time and is
/// deliberately not enforced.
///
/// # Errors
/// The first [`FormatViolation`] found.
pub fn validate_us_ein(text: &str) -> Result<(), FormatViolation> {
    if text.is_empty() {
        return Err(FormatViolation::Empty);
    }
    let (prefix, serial) = text.split_once('-').ok_or(FormatViolation::Structure)?;
    if prefix.len() != 2 || serial.len() != 7 {
        return Err(FormatViolation::Structure);
    }
    digits(prefix)?;
    digits(serial)?;
    if prefix == "00" {
        Err(FormatViolation::Range)
    } else {
        Ok(())
    }
}

/// German VAT identification number (Umsatzsteuer-Identifikationsnummer, USt-IdNr).
///
/// Rule: `DE` followed by nine digits, the first of which is not `0`; the ninth digit is a
/// check digit computed with ISO 7064 MOD 11,10 over the first eight: start with `p = 10`;
/// for each digit `d`: `s = (d + p) mod 10`, if `s = 0` then `s = 10`, `p = (2·s) mod 11`;
/// the check digit is `11 − p`, with 10 written as 0.
///
/// # Errors
/// The first [`FormatViolation`] found.
pub fn validate_de_vat_id(text: &str) -> Result<(), FormatViolation> {
    if text.is_empty() {
        return Err(FormatViolation::Empty);
    }
    let number = text.strip_prefix("DE").ok_or(FormatViolation::Structure)?;
    let found = fixed_digits(number, 9)?;
    if found.first() == Some(&0) {
        return Err(FormatViolation::Range);
    }
    let mut product = 10_u32;
    for digit in found.iter().take(8) {
        let mut sum = digit.saturating_add(product) % 10;
        if sum == 0 {
            sum = 10;
        }
        product = sum.saturating_mul(2) % 11;
    }
    let check = 11_u32.saturating_sub(product) % 10;
    if found.last() == Some(&check) {
        Ok(())
    } else {
        Err(FormatViolation::Checksum)
    }
}

/// Japanese corporate number (法人番号), 13 digits with a leading check digit.
///
/// Rule (National Tax Agency): the first digit is the check digit for the following twelve.
/// Number the twelve base digits from the **right** as `P1…P12`; with weight `Q = 1` for odd
/// positions and `2` for even positions, the check digit is
/// `9 − ((Σ Pn·Qn) mod 9)`. The check digit is therefore always 1–9.
///
/// # Errors
/// The first [`FormatViolation`] found.
pub fn validate_jp_corporate_number(text: &str) -> Result<(), FormatViolation> {
    let found = fixed_digits(text, 13)?;
    let sum = found
        .iter()
        .skip(1)
        .rev()
        .zip([1_u32, 2].iter().cycle())
        .fold(0_u32, |sum, (d, w)| {
            sum.saturating_add(d.saturating_mul(*w))
        });
    let check = 9_u32.saturating_sub(sum % 9);
    if found.first() == Some(&check) {
        Ok(())
    } else {
        Err(FormatViolation::Checksum)
    }
}

/// Japanese qualified invoice issuer registration number (適格請求書発行事業者登録番号),
/// **format only**.
///
/// Rule: the letter `T` followed by 13 digits. For corporations the digits are the corporate
/// number; for sole proprietors they are a separately assigned number, for which this crate
/// could not confirm a check-digit rule. So only the shape is checked here; to also check a
/// corporation's number, pass the digits to [`validate_jp_corporate_number`].
///
/// # Errors
/// The first [`FormatViolation`] found.
pub fn validate_jp_invoice_number(text: &str) -> Result<(), FormatViolation> {
    if text.is_empty() {
        return Err(FormatViolation::Empty);
    }
    let number = text.strip_prefix('T').ok_or(FormatViolation::Structure)?;
    fixed_digits(number, 13).map(|_| ()).map_err(|v| match v {
        FormatViolation::Empty => FormatViolation::Length,
        FormatViolation::Length
        | FormatViolation::Character
        | FormatViolation::Structure
        | FormatViolation::Checksum
        | FormatViolation::UnknownCode
        | FormatViolation::Range => v,
    })
}

/// Korean business registration number (사업자등록번호).
///
/// Rule: `NNN-NN-NNNNN` (ten digits `d1…d10`). Multiply `d1…d9` by the weights
/// `1 3 7 1 3 7 1 3 5` and add the products; add `⌊(d9·5)/10⌋`; the check digit `d10` is
/// `(10 − (sum mod 10)) mod 10`.
///
/// # Errors
/// The first [`FormatViolation`] found.
pub fn validate_kr_business_number(text: &str) -> Result<(), FormatViolation> {
    if text.is_empty() {
        return Err(FormatViolation::Empty);
    }
    let mut parts = text.split('-');
    let (a, b, c) = (
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
    );
    if parts.next().is_some() || a.len() != 3 || b.len() != 2 || c.len() != 5 {
        return Err(FormatViolation::Structure);
    }
    let mut found = digits(a)?;
    found.extend(digits(b)?);
    found.extend(digits(c)?);
    let ninth = found.get(8).copied().unwrap_or(0);
    let sum = weighted_sum(&found, &[1, 3, 7, 1, 3, 7, 1, 3, 5])
        .saturating_add(ninth.saturating_mul(5) / 10);
    let check = 10_u32.saturating_sub(sum % 10) % 10;
    if found.last() == Some(&check) {
        Ok(())
    } else {
        Err(FormatViolation::Checksum)
    }
}

/// Indian Goods and Services Tax Identification Number (GSTIN).
///
/// Rule: 15 upper-case letters or digits; the first two are a numeric state code (not `00`);
/// the last is a check character computed over the first fourteen with the "Luhn mod 36"
/// scheme: take each character's base-36 value (`0-9`, `A-Z`), multiply by 1, 2, 1, 2 …
/// from the left, replace each product by `⌊p/36⌋ + (p mod 36)`, add them; the check value is
/// `(36 − (sum mod 36)) mod 36`, written in base 36.
///
/// The inner layout (PAN, entity number, the usual `Z`) is not enforced, because special
/// registrations (UN bodies, non-resident and tax-deductor IDs) use other layouts.
///
/// # Errors
/// The first [`FormatViolation`] found.
pub fn validate_in_gstin(text: &str) -> Result<(), FormatViolation> {
    if text.is_empty() {
        return Err(FormatViolation::Empty);
    }
    let values: Vec<u32> = text
        .chars()
        .map(base36)
        .collect::<Option<_>>()
        .ok_or(FormatViolation::Character)?;
    if values.len() != 15 {
        return Err(FormatViolation::Length);
    }
    let state = text.get(..2).unwrap_or("");
    if !state.bytes().all(|b| b.is_ascii_digit()) {
        return Err(FormatViolation::Structure);
    }
    if state == "00" {
        return Err(FormatViolation::Range);
    }
    let sum =
        values
            .iter()
            .take(14)
            .zip([1_u32, 2].iter().cycle())
            .fold(0_u32, |sum, (v, factor)| {
                let product = v.saturating_mul(*factor);
                sum.saturating_add(product / 36)
                    .saturating_add(product % 36)
            });
    let check = 36_u32.saturating_sub(sum % 36) % 36;
    if values.last() == Some(&check) {
        Ok(())
    } else {
        Err(FormatViolation::Checksum)
    }
}

/// Indian Permanent Account Number (PAN), **format only** (the PAN check letter algorithm is
/// not published).
///
/// Rule: ten characters: five upper-case letters, four digits, one upper-case letter; the
/// fourth letter is the holder type and must be one of `A B C F G H J L P T`.
///
/// # Errors
/// The first [`FormatViolation`] found.
pub fn validate_in_pan(text: &str) -> Result<(), FormatViolation> {
    if text.is_empty() {
        return Err(FormatViolation::Empty);
    }
    if !text
        .bytes()
        .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase())
    {
        return Err(FormatViolation::Character);
    }
    if text.len() != 10 {
        return Err(FormatViolation::Length);
    }
    let shape_ok = text.bytes().enumerate().all(|(i, b)| {
        if (5..9).contains(&i) {
            b.is_ascii_digit()
        } else {
            b.is_ascii_uppercase()
        }
    });
    if !shape_ok {
        return Err(FormatViolation::Structure);
    }
    if text
        .chars()
        .nth(3)
        .is_some_and(|c| "ABCFGHJLPT".contains(c))
    {
        Ok(())
    } else {
        Err(FormatViolation::UnknownCode)
    }
}

// ---------------------------------------------------------------------------------------------
// UUID
// ---------------------------------------------------------------------------------------------

/// UUID (RFC 9562) in the canonical text form.
///
/// Rule: 36 characters: hexadecimal digits in groups of 8-4-4-4-12 separated by hyphens.
/// Both letter cases are accepted; any version and variant is accepted.
///
/// # Errors
/// The first [`FormatViolation`] found.
pub fn validate_uuid(text: &str) -> Result<(), FormatViolation> {
    if text.is_empty() {
        return Err(FormatViolation::Empty);
    }
    if text.len() != 36 {
        return Err(FormatViolation::Length);
    }
    for (index, byte) in text.bytes().enumerate() {
        let hyphen_expected = matches!(index, 8 | 13 | 18 | 23);
        if hyphen_expected && byte != b'-' {
            return Err(FormatViolation::Structure);
        }
        if !hyphen_expected && !byte.is_ascii_hexdigit() {
            return Err(if byte == b'-' {
                FormatViolation::Structure
            } else {
                FormatViolation::Character
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use FormatViolation as V;

    fn check(f: fn(&str) -> Result<(), V>, good: &[&str], bad: &[(&str, V)]) {
        for g in good {
            assert_eq!(f(g), Ok(()), "{g:?} should be valid");
        }
        for (b, why) in bad {
            assert_eq!(f(b), Err(*why), "{b:?}");
        }
    }

    #[test]
    fn std_postal_us() {
        check(
            validate_postal_us,
            &["94105", "00501", "94105-1234", "99950-0000"],
            &[
                ("", V::Empty),
                ("9410", V::Length),
                ("941051", V::Length),
                ("941051234", V::Length),
                ("94105-123", V::Length),
                ("94105-12345", V::Length),
                ("94105-", V::Length),
                ("-1234", V::Empty),
                ("9410a", V::Character),
                ("94105-12a4", V::Character),
                ("94105 1234", V::Character),
                ("94105-1234-1", V::Character),
            ],
        );
    }

    #[test]
    fn std_postal_jp_kr_de_in() {
        check(
            validate_postal_jp,
            &["100-0001", "060-0000", "999-9999"],
            &[
                ("", V::Empty),
                ("1000001", V::Structure),
                ("100-001", V::Length),
                ("10-00001", V::Length),
                ("100-00a1", V::Character),
                ("1a0-0001", V::Character),
                ("〒100-0001", V::Length),
                ("１００-０００１", V::Length),
                ("100-0001-", V::Length),
            ],
        );
        check(
            validate_postal_kr,
            &["04524", "63309", "00000"],
            &[
                ("", V::Empty),
                ("4524", V::Length),
                ("045240", V::Length),
                ("0452a", V::Character),
                ("045-24", V::Character),
            ],
        );
        check(
            validate_postal_de,
            &["10115", "01001", "99998", "80331"],
            &[
                ("", V::Empty),
                ("1011", V::Length),
                ("101155", V::Length),
                ("1011a", V::Character),
                ("00000", V::Range),
                ("01000", V::Range),
                ("99999", V::Range),
            ],
        );
        check(
            validate_postal_in,
            &["110001", "560001", "999999"],
            &[
                ("", V::Empty),
                ("11000", V::Length),
                ("1100011", V::Length),
                ("010001", V::Range),
                ("11000a", V::Character),
                ("110 001", V::Character),
            ],
        );
    }

    #[test]
    fn std_iban() {
        check(
            validate_iban,
            &[
                "GB82WEST12345698765432",
                "DE89370400440532013000",
                "FR1420041010050500013M02606",
                "NL91ABNA0417164300",
                "CH9300762011623852957",
                "NO9386011117947",
                "BE68539007547034",
                "ES9121000418450200051332",
                "IT60X0542811101000000123456",
                "SE4550000000058398257466",
                "AT611904300234573201",
                "MT84MALT011000012345MTLCAST001S",
                "SA0380000000608010167519",
            ],
            &[
                ("", V::Empty),
                ("GB82WEST12345698765431", V::Checksum),
                ("GB28WEST12345698765432", V::Checksum),
                ("GB82WEST1234569876543", V::Length),
                ("GB82WEST123456987654321", V::Length),
                ("GB82 WEST 1234 5698 7654 32", V::Character),
                ("gb82west12345698765432", V::Character),
                ("GB8", V::Length),
                ("GB82WEST1234", V::Length),
                ("GB82WEST12345698765432GB82WEST123456", V::Length),
                ("1282WEST12345698765432", V::Structure),
                ("GBXXWEST12345698765432", V::Structure),
                ("ZZ82WEST12345698765432", V::UnknownCode),
                ("GB00WEST12345698765432", V::Checksum),
                ("GB01WEST12345698765432", V::Checksum),
                ("GB99WEST12345698765432", V::Checksum),
                ("DE89370400440532013001", V::Checksum),
                // Transposition of two adjacent digits is always caught by mod-97.
                ("DE89370400440532010300", V::Checksum),
            ],
        );
    }

    #[test]
    fn edge_iban_country_without_registered_length() {
        // US is an ISO country without an IBAN registry entry: only shape and mod-97 apply.
        // Check digits computed for BBAN "0000000000000" + "US00".
        let bban = "1234567890123";
        let valid = (2..=98)
            .map(|c| format!("US{c:02}{bban}"))
            .find(|s| validate_iban(s).is_ok());
        let valid = valid.expect("exactly one check-digit pair is valid");
        assert_eq!(
            (2..=98)
                .filter(|c| validate_iban(&format!("US{c:02}{bban}")).is_ok())
                .count(),
            1
        );
        assert_eq!(validate_iban(&valid), Ok(()));
        assert!(IBAN_LENGTHS.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(IBAN_LENGTHS
            .iter()
            .all(|(c, len)| (is_country_code(c) || *c == "XK") && (15..=34).contains(len)));
    }

    #[test]
    fn std_bic() {
        check(
            validate_bic,
            &[
                "DEUTDEFF",
                "DEUTDEFF500",
                "NEDSZAJJ",
                "CHASUS33",
                "BOFAUS3NXXX",
                "MHCBJPJT",
                "E097AEXXXXX",
                "ABCDXKPR",
            ],
            &[
                ("", V::Empty),
                ("DEUTDEF", V::Length),
                ("DEUTDEFF5", V::Length),
                ("DEUTDEFF50", V::Length),
                ("DEUTDEFF5000", V::Length),
                ("deutdeff", V::Character),
                ("DEUT DEFF", V::Character),
                ("DEUTZZFF", V::UnknownCode),
                ("DEUT12FF", V::UnknownCode),
            ],
        );
    }

    #[test]
    fn std_aba() {
        check(
            validate_us_aba_routing,
            &[
                "021000021",
                "011000015",
                "026009593",
                "121000248",
                "322271627",
                "071000013",
                "061000104",
                "000000000",
            ],
            &[
                ("", V::Empty),
                ("021000022", V::Checksum),
                ("120100021", V::Checksum),
                ("02100002", V::Length),
                ("0210000211", V::Length),
                ("02100002a", V::Character),
                ("021-000-021", V::Character),
                ("131000000", V::Range),
                ("201000001", V::Range),
                ("901000000", V::Range),
            ],
        );
    }

    #[test]
    fn std_ein() {
        check(
            validate_us_ein,
            &["12-3456789", "01-0000000", "99-9999999"],
            &[
                ("", V::Empty),
                ("123456789", V::Structure),
                ("123-456789", V::Structure),
                ("1-23456789", V::Structure),
                ("12-345678", V::Structure),
                ("12-34567890", V::Structure),
                ("12-34-56789", V::Structure),
                ("1a-3456789", V::Character),
                ("12-345678a", V::Character),
                ("00-1234567", V::Range),
            ],
        );
    }

    #[test]
    fn std_de_vat() {
        check(
            validate_de_vat_id,
            &[
                "DE136695976",
                "DE143454214",
                "DE114103379",
                "DE115235681",
                "DE129274202",
                "DE811569869",
            ],
            &[
                ("", V::Empty),
                ("DE136695977", V::Checksum),
                ("DE163695976", V::Checksum),
                ("DE123456789", V::Checksum),
                ("136695976", V::Structure),
                ("de136695976", V::Structure),
                ("AT136695976", V::Structure),
                ("DE13669597", V::Length),
                ("DE1366959766", V::Length),
                ("DE", V::Empty),
                ("DE13669597a", V::Character),
                ("DE 136695976", V::Character),
                ("DE036695976", V::Range),
            ],
        );
    }

    #[test]
    fn edge_de_vat_check_digit_ten_is_written_as_zero() {
        // Find a base whose MOD 11,10 result is 10 and confirm 0 is the accepted check digit.
        let mut seen_zero = false;
        for base in 10_000_000_u32..10_000_200 {
            let valid: Vec<u32> = (0..10)
                .filter(|c| validate_de_vat_id(&format!("DE{base}{c}")).is_ok())
                .collect();
            assert_eq!(valid.len(), 1, "exactly one check digit per base");
            seen_zero |= valid == [0];
        }
        assert!(seen_zero);
    }

    #[test]
    fn std_jp_corporate_and_invoice() {
        check(
            validate_jp_corporate_number,
            // National Tax Agency; Toyota Motor Corporation.
            &["7000012050002", "1180301018771"],
            &[
                ("", V::Empty),
                ("7000012050003", V::Checksum),
                ("8000012050002", V::Checksum),
                ("7000012500002", V::Checksum),
                ("0000000000000", V::Checksum),
                ("700001205000", V::Length),
                ("70000120500022", V::Length),
                ("700001205000a", V::Character),
                ("７０００0１２０５０００２", V::Character),
            ],
        );
        check(
            validate_jp_invoice_number,
            &["T7000012050002", "T1180301018771", "T1234567890123"],
            &[
                ("", V::Empty),
                ("7000012050002", V::Structure),
                ("t7000012050002", V::Structure),
                ("T", V::Length),
                ("T700001205000", V::Length),
                ("T70000120500022", V::Length),
                ("T700001205000a", V::Character),
                ("T-7000012050002", V::Character),
            ],
        );
    }

    #[test]
    fn edge_jp_corporate_every_base_has_one_check_digit() {
        for base in [0_u64, 1, 12_050_002, 999_999_999_999, 180_301_018_771] {
            let valid = (0..10)
                .filter(|c| validate_jp_corporate_number(&format!("{c}{base:012}")).is_ok())
                .count();
            assert_eq!(valid, 1, "{base}");
        }
    }

    #[test]
    fn std_kr_business_number() {
        check(
            validate_kr_business_number,
            // Samsung Electronics.
            &["124-81-00998"],
            &[
                ("", V::Empty),
                ("124-81-00999", V::Checksum),
                ("142-81-00998", V::Checksum),
                ("1248100998", V::Structure),
                ("124-8100998", V::Structure),
                ("12-481-00998", V::Structure),
                ("124-81-0099", V::Structure),
                ("124-81-00998-1", V::Structure),
                ("12a-81-00998", V::Character),
                ("124-8a-00998", V::Character),
                ("124-81-0099a", V::Character),
            ],
        );
        // Every nine-digit base has exactly one valid check digit.
        for base in ["000-00-0000", "999-99-9999", "101-81-5000", "220-88-1234"] {
            assert_eq!(
                (0..10)
                    .filter(|c| validate_kr_business_number(&format!("{base}{c}")).is_ok())
                    .count(),
                1
            );
        }
    }

    #[test]
    fn std_in_gstin_and_pan() {
        check(
            validate_in_gstin,
            &[
                "27AAPFU0939F1ZV",
                "29AAGCB7383J1Z4",
                "33AAACH7409R1Z8",
                "24AAACC1206D1ZM",
                "29AABCU9603R1ZJ",
            ],
            &[
                ("", V::Empty),
                ("27AAPFU0939F1ZW", V::Checksum),
                ("27AAPFU0939F1Z0", V::Checksum),
                ("72AAPFU0939F1ZV", V::Checksum),
                ("27AAPFU0939F1Z", V::Length),
                ("27AAPFU0939F1ZVV", V::Length),
                ("27aapfu0939f1zv", V::Character),
                ("27AAPFU0939F1Z-", V::Character),
                ("A7AAPFU0939F1ZV", V::Structure),
                ("00AAPFU0939F1ZV", V::Range),
            ],
        );
        check(
            validate_in_pan,
            &["AAPFU0939F", "ABCPD1234E", "AAACR5055K", "AAATT0001A"],
            &[
                ("", V::Empty),
                ("AAPFU0939", V::Length),
                ("AAPFU0939FF", V::Length),
                ("aapfu0939f", V::Character),
                ("AAPFU-939F", V::Character),
                ("AAPF10939F", V::Structure),
                ("AAPFU09A9F", V::Structure),
                ("AAPFU09391", V::Structure),
                ("AAPXU0939F", V::UnknownCode),
            ],
        );
    }

    #[test]
    fn std_uuid() {
        check(
            validate_uuid,
            &[
                "123e4567-e89b-12d3-a456-426614174000",
                "00000000-0000-0000-0000-000000000000",
                "FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF",
                "018f4e6a-7b2c-7def-8abc-0123456789ab",
            ],
            &[
                ("", V::Empty),
                ("123e4567e89b12d3a456426614174000", V::Length),
                ("123e4567-e89b-12d3-a456-42661417400", V::Length),
                ("123e4567-e89b-12d3-a456-4266141740000", V::Length),
                ("123e4567-e89b-12d3-a456-42661417400g", V::Character),
                ("123e4567+e89b-12d3-a456-426614174000", V::Structure),
                ("123e-4567e89b-12d3-a456-426614174000", V::Structure),
                ("{23e4567-e89b-12d3-a456-42661417400}", V::Character),
            ],
        );
    }

    #[test]
    fn anomalous_non_ascii_digits_are_rejected() {
        // Arabic-Indic and full-width digits are `char::is_numeric` but not ASCII digits.
        assert_eq!(validate_postal_kr("٠٤٥٢٤"), Err(V::Character));
        assert_eq!(
            validate_us_aba_routing("０２１００００２１"),
            Err(V::Character)
        );
        assert_eq!(validate_in_gstin("27AAPFU0939F1Zé"), Err(V::Character));
    }
}

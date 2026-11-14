//! Embedded ISO code lists: countries (ISO 3166-1 alpha-2) and languages (ISO 639-1).
//!
//! The lists are compact, sorted, upper/lower-case exact, and searched by binary search.
//! Currency codes (ISO 4217) live in [`crate::money`] because they carry minor-unit rules.

/// ISO 3166-1 alpha-2 country codes (officially assigned, 249 entries), sorted.
pub const ISO_3166_ALPHA2: &[&str] = &[
    "AD", "AE", "AF", "AG", "AI", "AL", "AM", "AO", "AQ", "AR", "AS", "AT", "AU", "AW", "AX", "AZ",
    "BA", "BB", "BD", "BE", "BF", "BG", "BH", "BI", "BJ", "BL", "BM", "BN", "BO", "BQ", "BR", "BS",
    "BT", "BV", "BW", "BY", "BZ", "CA", "CC", "CD", "CF", "CG", "CH", "CI", "CK", "CL", "CM", "CN",
    "CO", "CR", "CU", "CV", "CW", "CX", "CY", "CZ", "DE", "DJ", "DK", "DM", "DO", "DZ", "EC", "EE",
    "EG", "EH", "ER", "ES", "ET", "FI", "FJ", "FK", "FM", "FO", "FR", "GA", "GB", "GD", "GE", "GF",
    "GG", "GH", "GI", "GL", "GM", "GN", "GP", "GQ", "GR", "GS", "GT", "GU", "GW", "GY", "HK", "HM",
    "HN", "HR", "HT", "HU", "ID", "IE", "IL", "IM", "IN", "IO", "IQ", "IR", "IS", "IT", "JE", "JM",
    "JO", "JP", "KE", "KG", "KH", "KI", "KM", "KN", "KP", "KR", "KW", "KY", "KZ", "LA", "LB", "LC",
    "LI", "LK", "LR", "LS", "LT", "LU", "LV", "LY", "MA", "MC", "MD", "ME", "MF", "MG", "MH", "MK",
    "ML", "MM", "MN", "MO", "MP", "MQ", "MR", "MS", "MT", "MU", "MV", "MW", "MX", "MY", "MZ", "NA",
    "NC", "NE", "NF", "NG", "NI", "NL", "NO", "NP", "NR", "NU", "NZ", "OM", "PA", "PE", "PF", "PG",
    "PH", "PK", "PL", "PM", "PN", "PR", "PS", "PT", "PW", "PY", "QA", "RE", "RO", "RS", "RU", "RW",
    "SA", "SB", "SC", "SD", "SE", "SG", "SH", "SI", "SJ", "SK", "SL", "SM", "SN", "SO", "SR", "SS",
    "ST", "SV", "SX", "SY", "SZ", "TC", "TD", "TF", "TG", "TH", "TJ", "TK", "TL", "TM", "TN", "TO",
    "TR", "TT", "TV", "TW", "TZ", "UA", "UG", "UM", "US", "UY", "UZ", "VA", "VC", "VE", "VG", "VI",
    "VN", "VU", "WF", "WS", "YE", "YT", "ZA", "ZM", "ZW",
];

/// ISO 639-1 two-letter language codes (183 entries), sorted, lower case.
pub const ISO_639_1: &[&str] = &[
    "aa", "ab", "ae", "af", "ak", "am", "an", "ar", "as", "av", "ay", "az", "ba", "be", "bg", "bi",
    "bm", "bn", "bo", "br", "bs", "ca", "ce", "ch", "co", "cr", "cs", "cu", "cv", "cy", "da", "de",
    "dv", "dz", "ee", "el", "en", "eo", "es", "et", "eu", "fa", "ff", "fi", "fj", "fo", "fr", "fy",
    "ga", "gd", "gl", "gn", "gu", "gv", "ha", "he", "hi", "ho", "hr", "ht", "hu", "hy", "hz", "ia",
    "id", "ie", "ig", "ii", "ik", "io", "is", "it", "iu", "ja", "jv", "ka", "kg", "ki", "kj", "kk",
    "kl", "km", "kn", "ko", "kr", "ks", "ku", "kv", "kw", "ky", "la", "lb", "lg", "li", "ln", "lo",
    "lt", "lu", "lv", "mg", "mh", "mi", "mk", "ml", "mn", "mr", "ms", "mt", "my", "na", "nb", "nd",
    "ne", "ng", "nl", "nn", "no", "nr", "nv", "ny", "oc", "oj", "om", "or", "os", "pa", "pi", "pl",
    "ps", "pt", "qu", "rm", "rn", "ro", "ru", "rw", "sa", "sc", "sd", "se", "sg", "si", "sk", "sl",
    "sm", "sn", "so", "sq", "sr", "ss", "st", "su", "sv", "sw", "ta", "te", "tg", "th", "ti", "tk",
    "tl", "tn", "to", "tr", "ts", "tt", "tw", "ty", "ug", "uk", "ur", "uz", "ve", "vi", "vo", "wa",
    "wo", "xh", "yi", "yo", "za", "zh", "zu",
];

/// Whether `code` is an officially assigned ISO 3166-1 alpha-2 country code (upper case).
#[must_use]
pub fn is_country_code(code: &str) -> bool {
    ISO_3166_ALPHA2.binary_search(&code).is_ok()
}

/// Whether `code` is an ISO 639-1 language code (lower case).
#[must_use]
pub fn is_language_code(code: &str) -> bool {
    ISO_639_1.binary_search(&code).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted_unique(list: &[&str]) -> bool {
        list.windows(2).all(|w| w[0] < w[1])
    }

    #[test]
    fn std_lists_are_sorted_and_sized() {
        assert!(sorted_unique(ISO_3166_ALPHA2));
        assert!(sorted_unique(ISO_639_1));
        assert_eq!(ISO_3166_ALPHA2.len(), 249);
        assert_eq!(ISO_639_1.len(), 183);
        assert!(ISO_3166_ALPHA2
            .iter()
            .all(|c| c.len() == 2 && c.bytes().all(|b| b.is_ascii_uppercase())));
        assert!(ISO_639_1
            .iter()
            .all(|c| c.len() == 2 && c.bytes().all(|b| b.is_ascii_lowercase())));
    }

    #[test]
    fn std_membership() {
        for c in ["US", "JP", "KR", "DE", "IN", "GB", "AD", "ZW", "SS"] {
            assert!(is_country_code(c), "{c}");
        }
        for c in ["en", "ja", "ko", "de", "hi", "aa", "zu"] {
            assert!(is_language_code(c), "{c}");
        }
    }

    #[test]
    fn invalid_membership() {
        for c in ["", "us", "USA", "XK", "UK", "EU", "ZZ", "U", "U S", "AA"] {
            assert!(!is_country_code(c), "{c}");
        }
        for c in ["", "EN", "eng", "xx", "e", "bh"] {
            assert!(!is_language_code(c), "{c}");
        }
    }
}

//! Auto-number patterns such as `PO-{YYYY}-{seq:5}`.
//!
//! A [`NumberingPattern`] turns a date and a sequence number into a human-readable identifier.
//! This module only parses and renders; allocating the next sequence value (and keeping it
//! gap-free where the law requires) is the job of the storage layer.
//!
//! # Syntax
//!
//! | Token | Output |
//! |---|---|
//! | `{YYYY}` | four-digit year |
//! | `{YY}` | two-digit year |
//! | `{MM}` | two-digit month |
//! | `{DD}` | two-digit day |
//! | `{seq}` | the sequence number |
//! | `{seq:N}` | the sequence number, zero-padded to `N` digits (1–18); longer numbers are **not** cut |
//! | `{{`, `}}` | a literal brace |
//! | anything else | copied literally |
//!
//! Exactly one `{seq}` token is required.

use crate::error::{clip, CoreError};
use crate::time::Date;
use core::fmt;
use core::str::FromStr;

/// Maximum number of characters in a numbering pattern.
pub const MAX_NUMBERING_CHARS: usize = 128;

/// One part of a numbering pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NumberingToken {
    /// Text copied as is.
    Literal(String),
    /// `{YYYY}`.
    Year4,
    /// `{YY}`.
    Year2,
    /// `{MM}`.
    Month,
    /// `{DD}`.
    Day,
    /// `{seq}` or `{seq:N}`; the payload is the zero-padding width (0 = no padding).
    Seq(u8),
}

/// How often the sequence restarts, derived from the date tokens in the pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SequenceReset {
    /// No date token: one sequence forever.
    Never,
    /// The pattern shows a year: restart each year.
    Yearly,
    /// The pattern shows a month: restart each month.
    Monthly,
    /// The pattern shows a day: restart each day.
    Daily,
}

/// A parsed numbering pattern. Text and serde form: the pattern source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberingPattern {
    source: String,
    tokens: Vec<NumberingToken>,
}

impl NumberingPattern {
    /// Parses a pattern.
    ///
    /// # Errors
    /// [`CoreError::InvalidPattern`] with the character position of the problem.
    pub fn new(source: &str) -> Result<Self, CoreError> {
        let fail = |position: usize, reason: &'static str| CoreError::InvalidPattern {
            pattern: clip(source),
            position,
            reason,
        };
        let chars: Vec<char> = source.chars().collect();
        if chars.len() > MAX_NUMBERING_CHARS {
            return Err(fail(
                MAX_NUMBERING_CHARS,
                "pattern is too long (max 128 characters)",
            ));
        }
        let mut tokens: Vec<NumberingToken> = Vec::new();
        let mut literal = String::new();
        let mut seq_count = 0_u32;
        let mut iter = chars.iter().copied().enumerate().peekable();
        while let Some((position, c)) = iter.next() {
            match c {
                '{' if iter.peek().is_some_and(|(_, n)| *n == '{') => {
                    iter.next();
                    literal.push('{');
                }
                '}' if iter.peek().is_some_and(|(_, n)| *n == '}') => {
                    iter.next();
                    literal.push('}');
                }
                '}' => {
                    return Err(fail(
                        position,
                        "`}` without `{`; write `}}` for a literal brace",
                    ))
                }
                '{' => {
                    let mut name = String::new();
                    let mut closed = false;
                    for (_, n) in iter.by_ref() {
                        if n == '}' {
                            closed = true;
                            break;
                        }
                        name.push(n);
                    }
                    if !closed {
                        return Err(fail(position, "`{` is not closed"));
                    }
                    let token = match name.as_str() {
                        "YYYY" => NumberingToken::Year4,
                        "YY" => NumberingToken::Year2,
                        "MM" => NumberingToken::Month,
                        "DD" => NumberingToken::Day,
                        "seq" => NumberingToken::Seq(0),
                        other => {
                            let width = other
                                .strip_prefix("seq:")
                                .filter(|w| !w.is_empty() && w.len() <= 2 && w.bytes().all(|b| b.is_ascii_digit()))
                                .and_then(|w| w.parse::<u8>().ok())
                                .filter(|w| (1..=18).contains(w))
                                .ok_or_else(|| fail(position, "unknown token; use {YYYY} {YY} {MM} {DD} {seq} {seq:N} with N from 1 to 18"))?;
                            NumberingToken::Seq(width)
                        }
                    };
                    if matches!(token, NumberingToken::Seq(_)) {
                        seq_count = seq_count.saturating_add(1);
                        if seq_count > 1 {
                            return Err(fail(position, "only one {seq} token is allowed"));
                        }
                    }
                    if !literal.is_empty() {
                        tokens.push(NumberingToken::Literal(core::mem::take(&mut literal)));
                    }
                    tokens.push(token);
                }
                other => literal.push(other),
            }
        }
        if !literal.is_empty() {
            tokens.push(NumberingToken::Literal(literal));
        }
        if seq_count == 0 {
            return Err(fail(chars.len(), "a {seq} token is required"));
        }
        Ok(NumberingPattern {
            source: source.to_owned(),
            tokens,
        })
    }

    /// The pattern source.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.source
    }

    /// The parsed tokens, in order.
    #[must_use]
    pub fn tokens(&self) -> &[NumberingToken] {
        &self.tokens
    }

    /// Renders the identifier for a date (normally the record's creation day in the base
    /// time zone, decided by the caller) and a sequence number.
    #[must_use]
    pub fn render(&self, date: Date, seq: u64) -> String {
        let (year, month, day) = date.ymd();
        let mut out = String::new();
        for token in &self.tokens {
            match token {
                NumberingToken::Literal(text) => out.push_str(text),
                NumberingToken::Year4 => out.push_str(&format!("{year:04}")),
                NumberingToken::Year2 => out.push_str(&format!("{:02}", year % 100)),
                NumberingToken::Month => out.push_str(&format!("{month:02}")),
                NumberingToken::Day => out.push_str(&format!("{day:02}")),
                NumberingToken::Seq(width) => {
                    out.push_str(&format!("{seq:0width$}", width = usize::from(*width)))
                }
            }
        }
        out
    }

    /// How often the sequence restarts: the finest date token present decides.
    #[must_use]
    pub fn reset(&self) -> SequenceReset {
        let mut reset = SequenceReset::Never;
        for token in &self.tokens {
            let candidate = match token {
                NumberingToken::Year4 | NumberingToken::Year2 => SequenceReset::Yearly,
                NumberingToken::Month => SequenceReset::Monthly,
                NumberingToken::Day => SequenceReset::Daily,
                NumberingToken::Literal(_) | NumberingToken::Seq(_) => SequenceReset::Never,
            };
            let rank = |r: SequenceReset| match r {
                SequenceReset::Never => 0,
                SequenceReset::Yearly => 1,
                SequenceReset::Monthly => 2,
                SequenceReset::Daily => 3,
            };
            if rank(candidate) > rank(reset) {
                reset = candidate;
            }
        }
        reset
    }

    /// The key of the sequence period a date falls in (`""`, `"2024"`, `"2024-05"` or
    /// `"2024-05-06"`), so storage can keep one counter per period.
    #[must_use]
    pub fn period_key(&self, date: Date) -> String {
        let (year, month, day) = date.ymd();
        match self.reset() {
            SequenceReset::Never => String::new(),
            SequenceReset::Yearly => format!("{year:04}"),
            SequenceReset::Monthly => format!("{year:04}-{month:02}"),
            SequenceReset::Daily => format!("{year:04}-{month:02}-{day:02}"),
        }
    }
}

impl fmt::Display for NumberingPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.source)
    }
}

impl FromStr for NumberingPattern {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        NumberingPattern::new(s)
    }
}

impl serde::Serialize for NumberingPattern {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.source)
    }
}

impl<'de> serde::Deserialize<'de> for NumberingPattern {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str> as serde::Deserialize>::deserialize(deserializer)?;
        NumberingPattern::new(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date() -> Date {
        Date::from_ymd(2024, 5, 6).unwrap()
    }
    fn render(p: &str, seq: u64) -> String {
        NumberingPattern::new(p).unwrap().render(date(), seq)
    }

    #[test]
    fn std_render_spec_examples() {
        assert_eq!(render("PO-{YYYY}-{seq:5}", 42), "PO-2024-00042");
        assert_eq!(render("PO-{YYYY}{MM}-{seq:5}", 7), "PO-202405-00007");
        assert_eq!(render("{YY}{MM}{DD}/{seq}", 3), "240506/3");
        assert_eq!(render("{seq:3}", 1), "001");
        assert_eq!(render("INV{{{seq}}}", 9), "INV{9}");
        assert_eq!(render("{seq}-請求", 9), "9-請求");
    }

    #[test]
    fn std_tokens_reset_and_period() {
        let p = NumberingPattern::new("PO-{YYYY}-{seq:5}").unwrap();
        assert_eq!(
            p.tokens(),
            &[
                NumberingToken::Literal("PO-".into()),
                NumberingToken::Year4,
                NumberingToken::Literal("-".into()),
                NumberingToken::Seq(5)
            ]
        );
        assert_eq!(p.reset(), SequenceReset::Yearly);
        assert_eq!(p.period_key(date()), "2024");
        let monthly = NumberingPattern::new("{YY}{MM}-{seq}").unwrap();
        assert_eq!(monthly.reset(), SequenceReset::Monthly);
        assert_eq!(monthly.period_key(date()), "2024-05");
        let daily = NumberingPattern::new("{DD}{MM}{YYYY}-{seq}").unwrap();
        assert_eq!(daily.reset(), SequenceReset::Daily);
        assert_eq!(daily.period_key(date()), "2024-05-06");
        let never = NumberingPattern::new("T{seq}").unwrap();
        assert_eq!(never.reset(), SequenceReset::Never);
        assert_eq!(never.period_key(date()), "");
    }

    #[test]
    fn std_display_serde() {
        let p: NumberingPattern = "PO-{YYYY}-{seq:5}".parse().unwrap();
        assert_eq!(p.to_string(), "PO-{YYYY}-{seq:5}");
        assert_eq!(p.as_str(), "PO-{YYYY}-{seq:5}");
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<NumberingPattern>(&json).unwrap(), p);
        assert!(serde_json::from_str::<NumberingPattern>("\"PO\"").is_err());
        assert_eq!(
            serde_json::to_string(&SequenceReset::Yearly).unwrap(),
            "\"yearly\""
        );
    }

    #[test]
    fn edge_wide_sequences_and_years() {
        // A sequence wider than the padding is never cut.
        assert_eq!(render("{seq:2}", 12_345), "12345");
        assert_eq!(render("{seq:18}", u64::MAX), "18446744073709551615");
        assert_eq!(render("{seq:18}", 0), "000000000000000000");
        let p = NumberingPattern::new("{YYYY}/{YY}-{seq}").unwrap();
        assert_eq!(p.render(Date::MIN, 1), "0001/01-1");
        assert_eq!(p.render(Date::MAX, 1), "9999/99-1");
        assert_eq!(
            p.render(Date::from_ymd(2000, 1, 1).unwrap(), 1),
            "2000/00-1"
        );
        assert!(
            NumberingPattern::new(&format!("{}{{seq}}", "x".repeat(MAX_NUMBERING_CHARS - 5)))
                .is_ok()
        );
    }

    #[test]
    fn invalid_patterns() {
        let cases: &[(&str, usize)] = &[
            ("", 0),
            ("PO-{YYYY}", 9),
            ("{seq}{seq}", 5),
            ("{seq:0}", 0),
            ("{seq:19}", 0),
            ("{seq:}", 0),
            ("{seq:a}", 0),
            ("{seq:005}", 0),
            ("{seq:+5}", 0),
            ("{yyyy}{seq}", 0),
            ("{}{seq}", 0),
            ("{seq", 0),
            ("{seq}}", 5),
            ("a}{seq}", 1),
            ("{seq}{", 5),
        ];
        for (pattern, position) in cases {
            match NumberingPattern::new(pattern) {
                Err(CoreError::InvalidPattern { position: p, .. }) => {
                    assert_eq!(p, *position, "{pattern}")
                }
                other => panic!("{pattern}: expected an error, got {other:?}"),
            }
        }
        let long = format!("{}{{seq}}", "x".repeat(MAX_NUMBERING_CHARS));
        assert!(NumberingPattern::new(&long).is_err());
    }
}

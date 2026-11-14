//! Regex-free text patterns for field validation.
//!
//! A [`TextPattern`] is a small, safe subset of regular-expression syntax that always matches
//! the **whole** text and runs in time proportional to `pattern length × text length` (no
//! backtracking blow-up, no `regex` dependency, tiny in WASM).
//!
//! # Syntax
//!
//! | Element | Meaning |
//! |---|---|
//! | `a`, `-`, `é` … | that literal character |
//! | `.` | any one character |
//! | `\d` | ASCII digit `0-9` |
//! | `\a` | ASCII letter `A-Z a-z` |
//! | `\u` / `\l` | ASCII upper-case / lower-case letter |
//! | `\w` | ASCII letter, digit or `_` |
//! | `\.` `\\` `\[` … | an escaped special character, taken literally |
//! | `[abc]`, `[A-Z0-9]`, `[^,;]` | one character from a set (ranges, `^` negation, `\d`-style classes) |
//! | `{n}`, `{m,n}`, `{m,}`, `?`, `*`, `+` | repetition of the element before it |
//! | leading `^`, trailing `$` | accepted and ignored (the match is always anchored) |
//!
//! Alternation (`|`), groups (`(…)`), back-references and look-around are **not** supported and
//! are rejected when the pattern is parsed. Example: employee ID `E\d{6}`, licence plate
//! `[A-Z]{2}-\d{2,4}`.

use crate::error::{clip, CoreError};
use core::fmt;
use core::str::FromStr;

/// Maximum number of characters in a pattern.
pub const MAX_PATTERN_CHARS: usize = 256;
/// Maximum repetition count in `{m,n}`.
pub const MAX_REPEAT: u32 = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
enum CharClass {
    Any,
    /// Inclusive ranges; `negated` flips the result.
    Set {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

impl CharClass {
    fn matches(&self, c: char) -> bool {
        match self {
            CharClass::Any => true,
            CharClass::Set { negated, ranges } => {
                ranges.iter().any(|(lo, hi)| (*lo..=*hi).contains(&c)) != *negated
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Element {
    class: CharClass,
    min: u32,
    /// `None` means unbounded.
    max: Option<u32>,
}

/// A parsed, validated text pattern. Text and serde form: the pattern source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPattern {
    source: String,
    elements: Vec<Element>,
}

const DIGIT: &[(char, char)] = &[('0', '9')];
const ALPHA: &[(char, char)] = &[('A', 'Z'), ('a', 'z')];
const UPPER: &[(char, char)] = &[('A', 'Z')];
const LOWER: &[(char, char)] = &[('a', 'z')];
const WORD: &[(char, char)] = &[('0', '9'), ('A', 'Z'), ('_', '_'), ('a', 'z')];

struct Parser<'a> {
    source: &'a str,
    chars: Vec<char>,
    pos: usize,
}

impl Parser<'_> {
    fn fail<T>(&self, reason: &'static str) -> Result<T, CoreError> {
        Err(CoreError::InvalidPattern {
            pattern: clip(self.source),
            position: self.pos,
            reason,
        })
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos = self.pos.saturating_add(1);
        }
        c
    }

    /// Reads what follows a backslash: a class shorthand or an escaped literal.
    fn escape(&mut self) -> Result<Vec<(char, char)>, CoreError> {
        match self.bump() {
            None => self.fail("pattern ends after `\\`"),
            Some('d') => Ok(DIGIT.to_vec()),
            Some('a') => Ok(ALPHA.to_vec()),
            Some('u') => Ok(UPPER.to_vec()),
            Some('l') => Ok(LOWER.to_vec()),
            Some('w') => Ok(WORD.to_vec()),
            Some(c) if c.is_alphanumeric() => {
                self.pos = self.pos.saturating_sub(1);
                self.fail("unknown escape; use \\d \\a \\u \\l \\w or escape a symbol")
            }
            Some(c) => Ok(vec![(c, c)]),
        }
    }

    fn set(&mut self) -> Result<CharClass, CoreError> {
        let negated = self.peek() == Some('^');
        if negated {
            self.bump();
        }
        let mut ranges: Vec<(char, char)> = Vec::new();
        loop {
            let c = match self.bump() {
                None => return self.fail("`[` is not closed"),
                Some(']') => break,
                Some(c) => c,
            };
            let low = if c == '\\' {
                let escaped = self.escape()?;
                match escaped.as_slice() {
                    [(lo, hi)] if lo == hi => *lo,
                    [..] => {
                        // A class shorthand such as \d inside a set.
                        ranges.extend(escaped);
                        continue;
                    }
                }
            } else {
                c
            };
            // A range `a-z`? A `-` just before `]` is a literal.
            let is_range = self.peek() == Some('-')
                && self
                    .chars
                    .get(self.pos.saturating_add(1))
                    .is_some_and(|n| *n != ']');
            if is_range {
                self.bump();
                let high = match self.bump() {
                    Some('\\') => match self.escape()?.as_slice() {
                        [(lo, hi)] if lo == hi => *lo,
                        [..] => return self.fail("a class cannot end a range"),
                    },
                    Some(h) => h,
                    None => return self.fail("`[` is not closed"),
                };
                if high < low {
                    return self.fail("range is reversed");
                }
                ranges.push((low, high));
            } else {
                ranges.push((low, low));
            }
        }
        if ranges.is_empty() {
            return self.fail("empty set `[]`");
        }
        Ok(CharClass::Set { negated, ranges })
    }

    fn number(&mut self) -> Result<Option<u32>, CoreError> {
        let mut value: Option<u32> = None;
        while let Some(digit) = self.peek().and_then(|c| c.to_digit(10)) {
            self.bump();
            let next = value.unwrap_or(0).saturating_mul(10).saturating_add(digit);
            if next > MAX_REPEAT {
                return self.fail("repetition count is too large (max 1000)");
            }
            value = Some(next);
        }
        Ok(value)
    }

    fn quantifier(&mut self) -> Result<(u32, Option<u32>), CoreError> {
        match self.peek() {
            Some('?') => {
                self.bump();
                Ok((0, Some(1)))
            }
            Some('*') => {
                self.bump();
                Ok((0, None))
            }
            Some('+') => {
                self.bump();
                Ok((1, None))
            }
            Some('{') => {
                self.bump();
                let Some(min) = self.number()? else {
                    return self.fail("expected a number after `{`");
                };
                let max = match self.bump() {
                    Some('}') => Some(min),
                    Some(',') => {
                        let max = self.number()?;
                        match self.bump() {
                            Some('}') => {}
                            Some(_) => {
                                // Point at the unexpected character.
                                self.pos = self.pos.saturating_sub(1);
                                return self.fail("expected `}`");
                            }
                            None => return self.fail("expected `}`"),
                        }
                        max
                    }
                    Some(_) => {
                        // Point at the unexpected character.
                        self.pos = self.pos.saturating_sub(1);
                        return self.fail("expected `}` or `,`");
                    }
                    None => return self.fail("expected `}` or `,`"),
                };
                if max.is_some_and(|max| max < min) {
                    return self.fail("repetition maximum is below the minimum");
                }
                Ok((min, max))
            }
            Some(_) | None => Ok((1, Some(1))),
        }
    }

    fn parse(&mut self) -> Result<Vec<Element>, CoreError> {
        if self.chars.len() > MAX_PATTERN_CHARS {
            return self.fail("pattern is too long (max 256 characters)");
        }
        let mut elements = Vec::new();
        if self.peek() == Some('^') {
            self.bump();
        }
        while let Some(c) = self.bump() {
            let class = match c {
                '.' => CharClass::Any,
                '\\' => CharClass::Set {
                    negated: false,
                    ranges: self.escape()?,
                },
                '[' => self.set()?,
                '$' if self.peek().is_none() => break,
                '|' | '(' | ')' => {
                    self.pos = self.pos.saturating_sub(1);
                    return self.fail("alternation and groups are not supported");
                }
                '?' | '*' | '+' | '{' => {
                    self.pos = self.pos.saturating_sub(1);
                    return self.fail("nothing to repeat");
                }
                ']' | '}' | '^' | '$' => {
                    self.pos = self.pos.saturating_sub(1);
                    return self.fail("special character must be escaped with `\\`");
                }
                literal => CharClass::Set {
                    negated: false,
                    ranges: vec![(literal, literal)],
                },
            };
            let (min, max) = self.quantifier()?;
            elements.push(Element { class, min, max });
        }
        Ok(elements)
    }
}

impl TextPattern {
    /// Parses a pattern.
    ///
    /// # Errors
    /// [`CoreError::InvalidPattern`] with the character position of the problem.
    pub fn new(source: &str) -> Result<Self, CoreError> {
        let mut parser = Parser {
            source,
            chars: source.chars().collect(),
            pos: 0,
        };
        let elements = parser.parse()?;
        Ok(TextPattern {
            source: source.to_owned(),
            elements,
        })
    }

    /// The pattern source as given to [`TextPattern::new`].
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.source
    }

    /// Whether the whole of `text` matches the pattern.
    ///
    /// Works on Unicode scalar values. Runs in `O(elements × characters)`.
    #[must_use]
    pub fn matches(&self, text: &str) -> bool {
        let chars: Vec<char> = text.chars().collect();
        let n = chars.len();
        // reach[p] is true when the elements processed so far can consume exactly p characters.
        let mut reach = vec![false; n.saturating_add(1)];
        if let Some(first) = reach.first_mut() {
            *first = true;
        }
        for element in &self.elements {
            // run[p] = number of consecutive characters from p that match the class.
            let mut run = vec![0_usize; n.saturating_add(1)];
            let mut streak = 0_usize;
            for (p, c) in chars.iter().enumerate().rev() {
                streak = if element.class.matches(*c) {
                    streak.saturating_add(1)
                } else {
                    0
                };
                if let Some(slot) = run.get_mut(p) {
                    *slot = streak;
                }
            }
            // From each reachable p the element can end anywhere in [p+min, p+min(max, run)].
            // Mark interval starts/ends in a difference array, then sweep.
            let min = usize::try_from(element.min).unwrap_or(usize::MAX);
            let max = element
                .max
                .map_or(usize::MAX, |m| usize::try_from(m).unwrap_or(usize::MAX));
            let mut delta = vec![0_i64; n.saturating_add(2)];
            for (p, reachable) in reach.iter().enumerate() {
                let available = run.get(p).copied().unwrap_or(0);
                if !*reachable || available < min {
                    continue;
                }
                let low = p.saturating_add(min);
                let high = p.saturating_add(available.min(max));
                if let Some(slot) = delta.get_mut(low) {
                    *slot = slot.saturating_add(1);
                }
                if let Some(slot) = delta.get_mut(high.saturating_add(1)) {
                    *slot = slot.saturating_sub(1);
                }
            }
            let mut open = 0_i64;
            for (slot, change) in reach.iter_mut().zip(&delta) {
                open = open.saturating_add(*change);
                *slot = open > 0;
            }
            if !reach.contains(&true) {
                return false;
            }
        }
        reach.last().copied().unwrap_or(false)
    }
}

impl fmt::Display for TextPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.source)
    }
}

impl FromStr for TextPattern {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        TextPattern::new(s)
    }
}

impl serde::Serialize for TextPattern {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.source)
    }
}

impl<'de> serde::Deserialize<'de> for TextPattern {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str> as serde::Deserialize>::deserialize(deserializer)?;
        TextPattern::new(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pattern: &str, text: &str) -> bool {
        TextPattern::new(pattern).unwrap().matches(text)
    }

    #[test]
    fn std_employee_id() {
        assert!(m(r"E\d{6}", "E123456"));
        assert!(m(r"^E\d{6}$", "E123456"));
        assert!(!m(r"E\d{6}", "E12345"));
        assert!(!m(r"E\d{6}", "E1234567"));
        assert!(!m(r"E\d{6}", "e123456"));
        assert!(!m(r"E\d{6}", "E12345a"));
    }

    #[test]
    fn std_classes_and_sets() {
        assert!(m(r"\a\u\l\w\d", "xQz_7"));
        assert!(!m(r"\u", "a"));
        assert!(!m(r"\l", "A"));
        assert!(!m(r"\a", "1"));
        assert!(!m(r"\w", "-"));
        assert!(m(r"[A-Z]{2}-\d{2,4}", "AB-123"));
        assert!(!m(r"[A-Z]{2}-\d{2,4}", "AB-1"));
        assert!(!m(r"[A-Z]{2}-\d{2,4}", "AB-12345"));
        assert!(m("[abc]+", "abcabc"));
        assert!(!m("[abc]+", "abd"));
        assert!(m("[^,;]*", "no commas"));
        assert!(!m("[^,;]*", "a,b"));
        assert!(m(r"[\d.]+", "1.2.3"));
        assert!(m(r"[a\-z]+", "a-z"));
        assert!(!m(r"[a\-z]+", "b"));
        assert!(m("[a-]+", "a-a"));
        assert!(m(r"[\]]", "]"));
        assert!(m(r"[!-\/]", "#"));
        assert!(m(".", "é"));
        assert!(m("é{2}", "éé"));
        assert!(m(r"\.\\\[", r".\["));
    }

    #[test]
    fn std_quantifiers() {
        assert!(m("ab?c", "ac"));
        assert!(m("ab?c", "abc"));
        assert!(!m("ab?c", "abbc"));
        assert!(m("a*", ""));
        assert!(m("a*", "aaaa"));
        assert!(!m("a+", ""));
        assert!(m("a{2,}", "aaa"));
        assert!(!m("a{2,}", "a"));
        assert!(m("a{0}", ""));
        assert!(m("a{0}b", "b"));
        assert!(m("", ""));
        assert!(!m("", "x"));
        assert!(m("^$", ""));
    }

    #[test]
    fn std_ambiguous_splits_need_no_backtracking() {
        // The split between the two runs is ambiguous; the matcher tracks all of them.
        assert!(m(r"\d*\d{3}", "12345"));
        assert!(!m(r"\d*\d{3}", "12"));
        assert!(m(r".*@.*\..*", "a@b.c"));
        assert!(m("a*a*a*b", "aaaaab"));
        assert!(!m("a*a*a*b", "aaaaa"));
        assert!(m(r"\w+-?\w+", "ab"));
    }

    #[test]
    fn std_display_serde() {
        let p: TextPattern = r"E\d{6}".parse().unwrap();
        assert_eq!(p.to_string(), r"E\d{6}");
        assert_eq!(p.as_str(), r"E\d{6}");
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, r#""E\\d{6}""#);
        assert_eq!(serde_json::from_str::<TextPattern>(&json).unwrap(), p);
        assert!(serde_json::from_str::<TextPattern>("\"(a)\"").is_err());
    }

    #[test]
    fn invalid_patterns_report_position() {
        let cases: &[(&str, usize)] = &[
            ("a|b", 1),
            ("(a)", 0),
            ("a)", 1),
            ("*a", 0),
            ("a**", 2),
            ("+", 0),
            ("?", 0),
            ("{2}", 0),
            ("a]", 1),
            ("a}", 1),
            ("a^", 1),
            ("a$b", 1),
            ("\\", 1),
            (r"\x", 1),
            (r"\9", 1),
            ("[abc", 4),
            ("[", 1),
            ("[]", 2),
            ("[^]", 3),
            ("[z-a]", 4),
            (r"[a-\d]", 5),
            ("[a-", 3),
            (r"[\", 2),
            (r"[a-\", 4),
            ("a{", 2),
            ("a{}", 2),
            ("a{x}", 2),
            ("a{2", 3),
            ("a{2,", 4),
            ("a{2,x}", 4),
            ("a{2x}", 3),
            ("a{3,2}", 6),
            ("a{1001}", 6),
            ("a{1,1001}", 8),
        ];
        for (pattern, position) in cases {
            match TextPattern::new(pattern) {
                Err(CoreError::InvalidPattern { position: p, .. }) => {
                    assert_eq!(p, *position, "{pattern}")
                }
                other => panic!("{pattern}: expected an error, got {other:?}"),
            }
        }
    }

    #[test]
    fn edge_limits() {
        assert!(TextPattern::new(&"a".repeat(MAX_PATTERN_CHARS)).is_ok());
        assert!(TextPattern::new(&"a".repeat(MAX_PATTERN_CHARS + 1)).is_err());
        assert!(m("a{1000}", &"a".repeat(1000)));
        assert!(!m("a{1000}", &"a".repeat(999)));
    }

    #[test]
    fn anomalous_pathological_input_is_fast() {
        // A classic catastrophic-backtracking shape; linear here.
        let pattern = TextPattern::new(&"a*".repeat(100)).unwrap();
        let text = "a".repeat(5_000) + "b";
        assert!(!pattern.matches(&text));
        assert!(pattern.matches(&"a".repeat(5_000)));
    }
}

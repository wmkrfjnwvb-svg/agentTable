//! Canonical JSON: the single byte representation of audit details that is hashed and stored.
//!
//! Rules (a strict subset of RFC 8785 / JCS):
//! 1. No whitespace anywhere.
//! 2. Object members are sorted by key, comparing the keys' UTF-8 bytes. (Keys are unique in a
//!    `serde_json::Value`.)
//! 3. Arrays keep their order.
//! 4. Numbers must be integers that fit `i64` or `u64` and are written in plain decimal. Floats are
//!    rejected: exact decimals must be carried as strings (platform rule: never floats).
//! 5. Strings are written as UTF-8. Only `"` and `\` and control characters below U+0020 are escaped:
//!    `\"`, `\\`, `\b`, `\t`, `\n`, `\f`, `\r`, and `\u00xx` (lowercase hex) for the rest. No Unicode
//!    normalisation is applied; callers must not rely on equivalence of different code point sequences.
//! 6. `null`, `true`, `false` are written literally.
//! 7. Nesting deeper than [`MAX_DEPTH`] is rejected.

use serde_json::Value;

use crate::error::AuditError;

/// Maximum nesting depth of arrays/objects accepted by [`canonical_json`].
pub const MAX_DEPTH: usize = 64;

/// Serialises `value` to its canonical JSON text.
pub fn canonical_json(value: &Value) -> Result<String, AuditError> {
    let mut out = String::new();
    write_value(&mut out, value, 0)?;
    Ok(out)
}

/// Whether `text` is already in canonical form (parses as JSON and re-serialises to the same bytes).
#[must_use]
pub fn is_canonical(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| canonical_json(&v).ok())
        .is_some_and(|c| c == text)
}

fn write_value(out: &mut String, value: &Value, depth: usize) -> Result<(), AuditError> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                out.push_str(&u.to_string());
            } else if let Some(i) = n.as_i64() {
                out.push_str(&i.to_string());
            } else {
                return Err(AuditError::NotCanonical("non-integer number"));
            }
        }
        Value::String(s) => write_string(out, s),
        Value::Array(items) => {
            let depth = descend(depth)?;
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, item, depth)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            let depth = descend(depth)?;
            // Sort explicitly: `serde_json` may be built with `preserve_order` by another crate.
            let mut members: Vec<(&String, &Value)> = map.iter().collect();
            members.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
            out.push('{');
            for (i, (key, item)) in members.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(out, key);
                out.push(':');
                write_value(out, item, depth)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn descend(depth: usize) -> Result<usize, AuditError> {
    if depth >= MAX_DEPTH {
        return Err(AuditError::NotCanonical("nested too deeply"));
    }
    Ok(depth.saturating_add(1))
}

fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{0c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if c < '\u{20}' => {
                out.push_str("\\u00");
                out.push_str(&crate::hex::to_hex(&[
                    u8::try_from(u32::from(c)).unwrap_or(0)
                ]));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

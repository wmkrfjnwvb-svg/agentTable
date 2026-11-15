//! Working with text. Positions and lengths count characters, starting at 1.

use crate::ast::TextFn;
use crate::error::ErrorValue;
use crate::eval::{Env, MAX_TEXT_CHARS};
use crate::functions::format;
use crate::functions::numbers::{flatten, value};
use crate::value::Val;
use mt_core::condition::fold_text;
use mt_core::decimal::Decimal;

/// A text result, unless it is longer than the limit.
pub(crate) fn bounded(text: String) -> Result<Val, ErrorValue> {
    if text.len() > MAX_TEXT_CHARS && text.chars().count() > MAX_TEXT_CHARS {
        return Err(ErrorValue::Limit);
    }
    Ok(Val::Text(text))
}

fn count(n: usize) -> Val {
    Val::Number(Decimal::from(u64::try_from(n).unwrap_or(u64::MAX)))
}

/// A count or position argument: a whole number that is at least `least`.
fn at_least(value: &Val, least: i64) -> Result<usize, ErrorValue> {
    let n = value.integer()?;
    if n < least {
        return Err(ErrorValue::Value);
    }
    usize::try_from(n).map_err(|_| ErrorValue::Value)
}

/// The position (from 1) of `needle` in `haystack` at or after position `start`.
fn position(haystack: &str, needle: &str, start: usize) -> Option<usize> {
    let skipped = start.checked_sub(1)?;
    let offset = haystack
        .char_indices()
        .map(|(index, _)| index)
        .chain([haystack.len()])
        .nth(skipped)?;
    let found = haystack.get(offset..)?.find(needle)?;
    let before = haystack.get(..offset.checked_add(found)?)?;
    before.chars().count().checked_add(1)
}

/// Lower case, character by character, keeping the number of characters (so positions in
/// the folded text are positions in the original).
fn lower_in_place(text: &str) -> String {
    text.chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect()
}

/// Evaluates a text function on evaluated, error-free arguments.
pub(crate) fn call(function: TextFn, values: &[Val], env: &dyn Env) -> Result<Val, ErrorValue> {
    let labels = |item| env.label(item);
    let text = |index: usize| value(values, index).text(&labels);
    match function {
        TextFn::Concat => {
            let mut out = String::new();
            for item in flatten(values) {
                out.push_str(&item.text(&labels)?);
                if out.len() > MAX_TEXT_CHARS.saturating_mul(4) {
                    return Err(ErrorValue::Limit);
                }
            }
            bounded(out)
        }
        TextFn::TextJoin => {
            let separator = text(0)?;
            let ignore_empty = value(values, 1).boolean()?;
            let mut parts = Vec::new();
            for item in flatten(values.get(2..).unwrap_or(&[])) {
                if ignore_empty && item.is_blank() {
                    continue;
                }
                parts.push(item.text(&labels)?);
            }
            bounded(parts.join(&separator))
        }
        TextFn::Len => Ok(count(text(0)?.chars().count())),
        TextFn::Left | TextFn::Right => {
            let source = text(0)?;
            let wanted = match values.get(1) {
                Some(n) => at_least(n, 0)?,
                None => 1,
            };
            let total = source.chars().count();
            let skip = if function == TextFn::Left {
                0
            } else {
                total.saturating_sub(wanted)
            };
            Ok(Val::Text(source.chars().skip(skip).take(wanted).collect()))
        }
        TextFn::Mid => {
            let start = at_least(value(values, 1), 1)?;
            let length = at_least(value(values, 2), 0)?;
            Ok(Val::Text(
                text(0)?
                    .chars()
                    .skip(start.saturating_sub(1))
                    .take(length)
                    .collect(),
            ))
        }
        TextFn::Upper => bounded(text(0)?.to_uppercase()),
        TextFn::Lower => bounded(text(0)?.to_lowercase()),
        TextFn::Trim => Ok(Val::Text(
            text(0)?.split_whitespace().collect::<Vec<_>>().join(" "),
        )),
        TextFn::Substitute => {
            let (source, old, new) = (text(0)?, text(1)?, text(2)?);
            if old.is_empty() {
                return Ok(Val::Text(source));
            }
            let Some(occurrence) = values.get(3) else {
                return bounded(source.replace(&old, &new));
            };
            let wanted = at_least(occurrence, 1)?;
            let Some((offset, _)) = source.match_indices(&old).nth(wanted.saturating_sub(1)) else {
                return Ok(Val::Text(source));
            };
            let (before, rest) = source.split_at(offset);
            let after = rest.get(old.len()..).unwrap_or("");
            bounded(format!("{before}{new}{after}"))
        }
        TextFn::Replace => {
            let source = text(0)?;
            let start = at_least(value(values, 1), 1)?;
            let length = at_least(value(values, 2), 0)?;
            let kept_before: String = source.chars().take(start.saturating_sub(1)).collect();
            let kept_after: String = source
                .chars()
                .skip(start.saturating_sub(1).saturating_add(length))
                .collect();
            bounded(format!("{kept_before}{}{kept_after}", text(3)?))
        }
        TextFn::Find | TextFn::Search => {
            let (mut needle, mut haystack) = (text(0)?, text(1)?);
            if function == TextFn::Search {
                needle = lower_in_place(&needle);
                haystack = lower_in_place(&haystack);
            }
            let start = match values.get(2) {
                Some(n) => at_least(n, 1)?,
                None => 1,
            };
            position(&haystack, &needle, start)
                .map(count)
                .ok_or(ErrorValue::Value)
        }
        TextFn::Rept => {
            let source = text(0)?;
            let times = at_least(value(values, 1), 0)?;
            if source.chars().count().saturating_mul(times) > MAX_TEXT_CHARS {
                return Err(ErrorValue::Limit);
            }
            Ok(Val::Text(source.repeat(times)))
        }
        TextFn::Value => value(values, 0).number().map(Val::Number),
        TextFn::Text => format::format(value(values, 0), &text(1)?, env).map(Val::Text),
        TextFn::Exact => Ok(Val::Bool(text(0)? == text(1)?)),
        TextFn::Contains => {
            let wanted = value(values, 1);
            if let Val::List(items) = value(values, 0) {
                for item in items {
                    if item.equals(wanted)? {
                        return Ok(Val::Bool(true));
                    }
                }
                return Ok(Val::Bool(false));
            }
            let needle = fold_text(&wanted.text(&labels)?);
            Ok(Val::Bool(
                !needle.is_empty() && fold_text(&text(0)?).contains(&needle),
            ))
        }
    }
}

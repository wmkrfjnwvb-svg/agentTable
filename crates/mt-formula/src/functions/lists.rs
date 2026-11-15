//! Lists (the values of linked records and of multi-value fields), reference data and
//! parameters.

use crate::ast::LookupFn;
use crate::error::ErrorValue;
use crate::eval::Env;
use crate::functions::numbers::value;
use crate::value::{Comparison, Val};
use core::cmp::Ordering;
use mt_core::condition::fold_text;
use mt_core::decimal::canonical_string;
use mt_core::time::Date;
use std::collections::BTreeSet;

/// The test of `COUNTIF`, `SUMIF` and `AVERAGEIF` when it is not written with
/// `CurrentValue`: a value to equal, or a text such as `">10"`, `"<>done"`, `"<=2026-01-31"`.
pub(crate) struct Criterion {
    accepted: &'static [Ordering],
    negate: bool,
    operand: Val,
}

impl Criterion {
    pub(crate) fn new(value: Val) -> Self {
        const LESS: &[Ordering] = &[Ordering::Less];
        const GREATER: &[Ordering] = &[Ordering::Greater];
        const EQUAL: &[Ordering] = &[Ordering::Equal];
        const AT_MOST: &[Ordering] = &[Ordering::Less, Ordering::Equal];
        const AT_LEAST: &[Ordering] = &[Ordering::Greater, Ordering::Equal];
        let equals = |operand: Val| Criterion {
            accepted: EQUAL,
            negate: false,
            operand,
        };
        let Val::Text(text) = &value else {
            return equals(value);
        };
        let rules: [(&str, &'static [Ordering], bool); 6] = [
            (">=", AT_LEAST, false),
            ("<=", AT_MOST, false),
            ("<>", EQUAL, true),
            (">", GREATER, false),
            ("<", LESS, false),
            ("=", EQUAL, false),
        ];
        for (prefix, accepted, negate) in rules {
            if let Some(rest) = text.strip_prefix(prefix) {
                let rest = rest.trim();
                // The operand is a number or a date when it is written as one.
                let operand = if let Ok(number) = mt_core::decimal::parse_decimal(rest) {
                    Val::Number(number)
                } else if let Ok(day) = rest.parse::<Date>() {
                    Val::Date(day)
                } else if rest.is_empty() {
                    Val::Blank
                } else {
                    Val::Text(rest.to_owned())
                };
                return Criterion {
                    accepted,
                    negate,
                    operand,
                };
            }
        }
        equals(value)
    }

    /// Whether an item satisfies the criterion. Items of a kind that cannot be compared
    /// with the operand simply do not match.
    pub(crate) fn accepts(&self, item: &Val) -> Result<bool, ErrorValue> {
        // An amount is compared with a plain number by its amount.
        let amount;
        let item = match (item, &self.operand) {
            (Val::Money(value, _), Val::Number(_)) => {
                amount = Val::Number(*value);
                &amount
            }
            _ => item,
        };
        let matched = match item.compare(&self.operand)? {
            Comparison::Ordered(ordering) => self.accepted.contains(&ordering),
            Comparison::Unordered(equal) => equal && self.accepted.contains(&Ordering::Equal),
            Comparison::Unknown | Comparison::Mismatch => false,
        };
        Ok(matched != self.negate)
    }
}

/// The items of an evaluated list argument.
fn items(value: &Val) -> Vec<Val> {
    if let Val::List(items) = value {
        items.clone()
    } else if value.is_blank() {
        Vec::new()
    } else {
        vec![value.clone()]
    }
}

/// A key under which equal values (as `=` sees them) are equal texts.
fn identity(value: &Val) -> String {
    match value {
        Val::Number(n) => format!("n:{}", canonical_string(n)),
        Val::Text(text) => format!("t:{}", fold_text(text)),
        Val::Bool(b) => format!("b:{b}"),
        Val::Date(d) => format!("d:{d}"),
        Val::DateTime(t) => format!("i:{}", t.millis()),
        Val::Duration(d) => format!("u:{}", d.millis()),
        Val::Money(amount, currency) => {
            format!("m:{}:{}", currency.code(), canonical_string(amount))
        }
        Val::Ref(item) => format!("r:{item}"),
        // Not items of a list: empty values are dropped first, lists are flat, and errors
        // were returned before the list got here.
        Val::Blank | Val::List(_) | Val::Error(_) => String::new(),
    }
}

/// `UNIQUE`: the distinct values, in order of first appearance, without empty ones.
pub(crate) fn unique(values: &[Val]) -> Val {
    let mut seen = BTreeSet::new();
    Val::List(
        items(value(values, 0))
            .into_iter()
            .filter(|item| !item.is_blank() && seen.insert(identity(item)))
            .collect(),
    )
}

/// `FIRST`: the first value, or nothing.
pub(crate) fn first(values: &[Val]) -> Val {
    items(value(values, 0))
        .into_iter()
        .next()
        .unwrap_or(Val::Blank)
}

/// `LAST`: the last value, or nothing.
pub(crate) fn last(values: &[Val]) -> Val {
    items(value(values, 0))
        .into_iter()
        .next_back()
        .unwrap_or(Val::Blank)
}

/// `INDEX`: the value at a position, counting from 1.
pub(crate) fn index(values: &[Val]) -> Result<Val, ErrorValue> {
    let position = value(values, 1).integer()?;
    let index = usize::try_from(position)
        .ok()
        .and_then(|p| p.checked_sub(1))
        .ok_or(ErrorValue::Value)?;
    items(value(values, 0))
        .into_iter()
        .nth(index)
        .ok_or(ErrorValue::NotAvailable)
}

/// Evaluates a reference-data or parameter function.
pub(crate) fn lookup(function: LookupFn, values: &[Val], env: &dyn Env) -> Result<Val, ErrorValue> {
    let subject = value(values, 0);
    // Applies `find` to a reference, or to every reference of a list.
    let for_each =
        |find: &dyn Fn(mt_core::id::RefItemId) -> Option<Val>| -> Result<Val, ErrorValue> {
            let one = |item: &Val| match item {
                Val::Ref(id) => find(*id).ok_or(ErrorValue::NotAvailable),
                Val::Blank => Ok(Val::Blank),
                Val::Number(_)
                | Val::Text(_)
                | Val::Bool(_)
                | Val::Date(_)
                | Val::DateTime(_)
                | Val::Duration(_)
                | Val::Money(..)
                | Val::List(_)
                | Val::Error(_) => Err(ErrorValue::Value),
            };
            if let Val::List(list) = subject {
                list.iter()
                    .map(one)
                    .collect::<Result<Vec<_>, _>>()
                    .map(Val::List)
            } else {
                one(subject)
            }
        };
    match function {
        LookupFn::Label => for_each(&|id| env.label(id).map(Val::Text)),
        LookupFn::Code => for_each(&|id| env.code(id).map(Val::Text)),
        LookupFn::Attr => {
            let name = value(values, 1).text(&|item| env.label(item))?;
            for_each(&|id| env.attribute(id, &name).map(|v| Val::from_value(&v)))
        }
        LookupFn::Param => {
            let key = subject.text(&|item| env.label(item))?;
            env.param(&key)
                .map(|v| Val::from_value(&v))
                .ok_or(ErrorValue::NotAvailable)
        }
    }
}

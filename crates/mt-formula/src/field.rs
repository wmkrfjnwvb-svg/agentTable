//! Formulas on field definitions.
//!
//! A computed field keeps its formula in the definition's computed facet
//! ([`mt_core::field::Computed`]) as a stored tree. `mt-core` does not interpret it; this
//! module writes it ([`attach`]), reads it ([`formula_of`]), checks it against the table
//! ([`check_field`]) and collects the computed fields of a table for the dependency graph
//! ([`computed_fields`]).
//!
//! The three kinds of computed field share one formula language. The kind only says which
//! editor the designer used, and what shape the formula therefore has:
//!
//! * **lookup** — exactly `{Link}.{Field}`: the values of a field in linked records;
//! * **roll-up** — a function over linked values: `SUM({Lines}.{Amount})`;
//! * **formula** — anything.

use crate::ast::Expr;
use crate::deps::ComputedField;
use crate::error::FormulaError;
use crate::schema::Schema;
use crate::types::{check_for_field, Type};
use mt_core::field::{Computed, ComputedKind, FieldDef};
use mt_core::id::TableId;

fn stored(field: &FieldDef, reason: impl core::fmt::Display) -> FormulaError {
    FormulaError::Stored {
        field: field.name.clone(),
        reason: reason.to_string(),
    }
}

/// Makes `field` a computed field with this formula. The formula is not checked here; call
/// [`check_field`] before saving the definition.
pub fn attach(field: &mut FieldDef, kind: ComputedKind, expr: &Expr) {
    // Writing a tree cannot fail (it has only text keys); should that ever change, the
    // empty formula left behind is refused by `FieldDef::check`.
    let formula = serde_json::to_value(expr).unwrap_or(serde_json::Value::Null);
    field.computed = Some(Computed { kind, formula });
}

/// The formula of a field: `None` for a field that is not computed.
///
/// # Errors
/// [`FormulaError::Stored`] if the stored tree cannot be read (it was written by a newer
/// version, or damaged).
pub fn formula_of(field: &FieldDef) -> Result<Option<Expr>, FormulaError> {
    let Some(computed) = &field.computed else {
        return Ok(None);
    };
    serde_json::from_value(computed.formula.clone())
        .map(Some)
        .map_err(|error| stored(field, error))
}

/// Whether the tree reads linked records anywhere.
fn reads_links(expr: &Expr) -> bool {
    let mut found = false;
    expr.walk(&mut |node| found |= matches!(node, Expr::Through { .. }));
    found
}

/// Checks the formula of a computed field against its table: the stored tree can be read,
/// it has the shape of its kind, its parts fit together, and everything it can return fits
/// the field's data type. Returns the type of the result; `None` for a field that is not
/// computed.
///
/// # Errors
/// [`FormulaError::Stored`] for an unreadable tree; [`FormulaError::Type`] for a formula
/// that does not have the shape of its kind or whose result does not fit the field; the
/// other errors of [`check_for_field`].
pub fn check_field(field: &FieldDef, schema: &dyn Schema) -> Result<Option<Type>, FormulaError> {
    let (Some(computed), Some(expr)) = (&field.computed, formula_of(field)?) else {
        return Ok(None);
    };
    let shaped = match computed.kind {
        ComputedKind::Formula => true,
        ComputedKind::Lookup => matches!(expr, Expr::Through { .. }),
        ComputedKind::Rollup => matches!(expr, Expr::Call { .. }) && reads_links(&expr),
    };
    if !shaped {
        let wanted = match computed.kind {
            ComputedKind::Lookup => "a lookup is a field of linked records: {Link}.{Field}",
            ComputedKind::Rollup | ComputedKind::Formula => {
                "a roll-up is a function over a field of linked records, such as SUM({Link}.{Field})"
            }
        };
        return Err(FormulaError::typing(format!("`{}`: {wanted}", field.name)));
    }
    check_for_field(&expr, schema, field.data_type).map(Some)
}

/// The computed fields among the fields of a table, ready for
/// [`crate::deps::Graph::build`]. Disabled and deprecated fields are included: their stored
/// values are still kept up to date.
///
/// # Errors
/// [`FormulaError::Stored`] for the first field whose stored tree cannot be read.
pub fn computed_fields(
    table: TableId,
    fields: &[FieldDef],
) -> Result<Vec<ComputedField>, FormulaError> {
    let mut out = Vec::new();
    for field in fields {
        if let Some(expr) = formula_of(field)? {
            out.push(ComputedField {
                table,
                field: field.id,
                expr,
            });
        }
    }
    Ok(out)
}

//! What the web app receives: values as people read them, fields with what the viewer may
//! do with them, and decisions explained in plain words.
//!
//! Display formatting lives here until the shared formatting reaches the browser through
//! WebAssembly (wave 7).

use crate::model::{Record, Table, Workspace};
use mt_core::decimal::{canonical_string, Decimal, Rounding};
use mt_core::field::{DataType, FieldDef, OptionsSource};
use mt_core::fieldtype::BuiltinType;
use mt_core::time::Timestamp;
use mt_core::value::Value;
use mt_perm::{Decision, FieldPermission, Grant, Granted, Reason, Resource, Scope, Visibility};
use serde_json::{json, Value as Json};

/// Groups the digits of a non-negative integer text in threes.
fn grouped(digits: &str) -> String {
    let count = digits.chars().count();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        let remaining = count.saturating_sub(index);
        if index > 0 && remaining.checked_rem(3) == Some(0) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// A decimal with thousands separators and at least `min_places` decimals.
#[must_use]
pub fn number(value: Decimal, min_places: u32) -> String {
    let text = canonical_string(&value);
    let (sign, body) = text
        .strip_prefix('-')
        .map_or(("", text.as_str()), |rest| ("-", rest));
    let (whole, fraction) = body.split_once('.').unwrap_or((body, ""));
    let mut fraction = fraction.to_owned();
    while u32::try_from(fraction.len()).unwrap_or(u32::MAX) < min_places {
        fraction.push('0');
    }
    if fraction.is_empty() {
        format!("{sign}{}", grouped(whole))
    } else {
        format!("{sign}{}.{fraction}", grouped(whole))
    }
}

fn instant(at: Timestamp) -> String {
    let text = at.to_string();
    // 2026-05-01T08:15:00.000Z → 2026-05-01 08:15
    text.get(..16)
        .map_or(text.clone(), |head| head.replacen('T', " ", 1))
}

/// A value as people read it.
#[must_use]
pub fn display(ws: &Workspace, field: &FieldDef, value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Text(text) | Value::LongText(text) => text.clone(),
        Value::Number(n) => {
            let places = field.format.decimals.map_or(0, u32::from);
            let shown = field
                .format
                .decimals
                .map_or(*n, |d| Rounding::HalfUp.round(*n, u32::from(d)));
            number(shown, places)
        }
        Value::Money(money) => {
            let places = money.currency().minor_units().map_or(0, u32::from);
            format!(
                "{} {}",
                number(Rounding::HalfEven.round(money.amount(), places), places),
                money.currency().code()
            )
        }
        Value::Percent(n) => {
            let percent = n.checked_mul(Decimal::ONE_HUNDRED).unwrap_or(*n);
            format!("{}%", number(Rounding::HalfUp.round(percent, 2), 0))
        }
        Value::Bool(true) => "Yes".to_owned(),
        Value::Bool(false) => "No".to_owned(),
        Value::Date(day) => day.to_string(),
        Value::DateTime(at) => instant(*at),
        Value::Duration(duration) => duration.to_string(),
        Value::Reference(item) => ws.label(*item).unwrap_or_else(|| item.to_string()),
        Value::MultiReference(items) => items
            .iter()
            .map(|item| ws.label(*item).unwrap_or_else(|| item.to_string()))
            .collect::<Vec<_>>()
            .join(", "),
        Value::Link(ids) => {
            let target = match field.options {
                OptionsSource::Table { table } => Some(table),
                OptionsSource::None
                | OptionsSource::Inline { .. }
                | OptionsSource::RefSet { .. }
                | OptionsSource::Parameter { .. } => None,
            };
            ids.iter()
                .map(|id| target.map_or_else(|| id.to_string(), |t| ws.title(t, *id)))
                .collect::<Vec<_>>()
                .join(", ")
        }
        Value::Principal(people) => people
            .iter()
            .map(|p| ws.name_of(*p))
            .collect::<Vec<_>>()
            .join(", "),
        Value::Attachment(files) => format!("{} file(s)", files.len()),
        Value::Json(json) => json.to_string(),
    }
}

/// One cell for the grid: the display text, the stored value (unless masked) and an error.
#[must_use]
pub fn cell(
    ws: &Workspace,
    field: &FieldDef,
    record: &Record,
    permission: FieldPermission,
) -> Json {
    if let Some(error) = record.errors.get(&field.id) {
        return json!({"d": error.code(), "error": error.explanation()});
    }
    let value = record.cells.get(&field.id).cloned().unwrap_or(Value::Null);
    let text = display(ws, field, &value);
    match permission.visibility {
        Visibility::Visible => match (&value, &field.options) {
            // Each linked record's title on its own: a title may contain commas.
            (Value::Link(ids), OptionsSource::Table { table }) => {
                let items: Vec<String> = ids.iter().map(|id| ws.title(*table, *id)).collect();
                json!({"d": text, "v": value, "items": items})
            }
            _ => json!({"d": text, "v": value}),
        },
        Visibility::Masked => {
            let masked = field
                .format
                .mask
                .map_or_else(|| "••••".to_owned(), |mask| mask.apply(&text));
            json!({"d": masked, "masked": true})
        }
        Visibility::Hidden => json!({"d": "", "hidden": true}),
    }
}

/// The description of a field for the web app.
#[must_use]
pub fn field(
    ws: &Workspace,
    table: &Table,
    field: &FieldDef,
    permission: Option<FieldPermission>,
) -> Json {
    let builtin = BuiltinType::of(field);
    let formula = mt_formula::field::formula_of(field)
        .ok()
        .flatten()
        .map(|expr| {
            let schema = crate::formula::TableSchema { ws, table };
            expr.to_text(&schema)
        });
    let target = match &field.options {
        OptionsSource::Table { table } => Some(json!(table)),
        OptionsSource::None
        | OptionsSource::Inline { .. }
        | OptionsSource::RefSet { .. }
        | OptionsSource::Parameter { .. } => None,
    };
    let numeric = matches!(
        field.data_type,
        DataType::Number | DataType::Money | DataType::Percent | DataType::Duration
    );
    json!({
        "id": field.id,
        "name": field.name,
        "type": builtin.as_str(),
        "typeLabel": builtin.label(),
        "group": builtin.group().as_str(),
        "dataType": field.data_type.as_str(),
        "computed": field.computed.as_ref().map(|c| c.kind.as_str()),
        "formula": formula,
        "generated": field.is_generated(),
        "required": field.validation.required,
        "classification": field.classification.as_str(),
        "masked": field.format.mask.is_some(),
        "numeric": numeric,
        "primary": table.primary == field.id,
        "linkTarget": target,
        "fieldGroup": table.group_of(field.id).map(|g| g.name.clone()),
        "help": field.help_text,
        "options": field.inline_options().map(|options| options
            .iter()
            .map(|o| json!({"id": o.id, "label": o.label, "enabled": o.enabled}))
            .collect::<Vec<_>>()),
        "maxItems": field.validation.max_items,
        "requiredWhen": field.validation.required_when.as_ref().and_then(crate::design::simple_of),
        "requiredWhenText": field.validation.required_when.as_ref().map(|c| crate::process::condition_text(ws, table, c)),
        "visibility": permission.map(|p| match p.visibility {
            Visibility::Visible => "visible",
            Visibility::Masked => "masked",
            Visibility::Hidden => "hidden",
        }),
        "editable": permission.is_some_and(|p| p.editable),
    })
}

/// Where a grant applies, in words.
#[must_use]
pub fn place_of(ws: &Workspace, on: &Resource) -> String {
    place(ws, on)
}

fn place(ws: &Workspace, on: &Resource) -> String {
    match on {
        Resource::Tenant => "the whole organisation".to_owned(),
        Resource::Workspace { .. } => "the workspace".to_owned(),
        Resource::Base { base, .. } => ws
            .base(*base)
            .map_or_else(|_| "a base".to_owned(), |b| format!("the {} base", b.name)),
        Resource::Table { table, .. } => ws
            .table(*table)
            .map_or_else(|_| table.to_string(), |t| format!("the {} table", t.name)),
    }
}

/// Which records a grant covers, in words.
#[must_use]
pub fn scope_of(ws: &Workspace, grant: &Grant) -> String {
    scope_text(ws, grant)
}

fn scope_text(ws: &Workspace, grant: &Grant) -> String {
    let scope = match &grant.scope {
        Scope::All => "all records".to_owned(),
        Scope::Own => "records they own".to_owned(),
        Scope::Team => "records of their team".to_owned(),
        Scope::Where { .. } => "records matching a condition".to_owned(),
        Scope::RefSubtree { root, .. } => format!(
            "records under {}",
            ws.label(*root).unwrap_or_else(|| root.to_string())
        ),
        Scope::Records { records } => format!("{} listed record(s)", records.len()),
    };
    if grant.condition.is_some() {
        format!("{scope}, with a limit")
    } else {
        scope
    }
}

/// A decision explained: the grant chain, or why each grant did not apply.
#[must_use]
pub fn explain(ws: &Workspace, decision: &Decision) -> Json {
    let policy = &ws.data.policy;
    let chain: Vec<Json> = decision
        .chain
        .iter()
        .map(|link| {
            let grant = policy.grant(link.grant);
            json!({
                "grant": link.grant,
                "principal": ws.name_of(link.principal),
                "holder": ws.name_of(link.holder),
                "viaGroup": link.holder != link.principal,
                "capability": link.capability.key(),
                "set": link.through_set.and_then(|s| policy.set(s)).map(|s| s.name.clone()),
                "on": place(ws, &link.on),
                "scope": grant.map(|g| scope_text(ws, g)),
                "reason": grant.map(|g| g.reason.clone()),
                "grantedBy": grant.map(|g| ws.name_of(g.granted_by)),
            })
        })
        .collect();
    let refusals: Vec<Json> = decision
        .refusals
        .iter()
        .map(|refusal| {
            let grant = refusal.grant.and_then(|g| policy.grant(g));
            let what = grant.map(|g| match &g.what {
                Granted::Capability(c) => c.key().to_owned(),
                Granted::Set(s) => policy
                    .set(*s)
                    .map_or_else(|| s.to_string(), |set| format!("set “{}”", set.name)),
            });
            json!({
                "principal": ws.name_of(refusal.principal),
                "grant": refusal.grant,
                "gives": what,
                "on": grant.map(|g| place(ws, &g.on)),
                "scope": grant.map(|g| scope_text(ws, g)),
                "reason": refusal.reason.to_string(),
                "code": reason_code(&refusal.reason),
            })
        })
        .collect();
    json!({"allowed": decision.allowed, "chain": chain, "refusals": refusals})
}

fn reason_code(reason: &Reason) -> &'static str {
    match reason {
        Reason::UnknownPrincipal => "unknown_principal",
        Reason::NoGrant => "no_grant",
        Reason::Inactive { .. } => "inactive",
        Reason::Elsewhere => "elsewhere",
        Reason::OutOfScope => "out_of_scope",
        Reason::ConditionNotMet => "condition_not_met",
        Reason::Administrative => "administrative",
        Reason::SeparationOfDuties { .. } => "separation_of_duties",
    }
}

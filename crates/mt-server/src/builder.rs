//! Building: the configuration users assemble from the components — bases, tables, fields
//! (two-way links included), code lists, settings, people and groups, and templates.
//!
//! Every operation checks the capability it needs (`schema.manage`, `refdata.manage`,
//! `settings.manage`, `members.manage`, `template.manage`), verifies the result with the engine
//! that owns it (field framework, formula checker, reference data), and is written to the audit
//! chain. A refused operation changes nothing.

use crate::error::{ApiError, ApiResult};
use crate::model::{Base, LinkPair, Person, Table, Workspace};
use mt_audit::ObjectRef;
use mt_core::field::{DataType, FieldDef, FieldRole, OptionsSource};
use mt_core::fieldtype::BuiltinType;
use mt_core::id::{BaseId, FieldId, PrincipalId, RefItemId, RefSetId, TableId};
use mt_core::time::Timestamp;
use mt_core::value::Value;
use mt_perm::{Action, Actor, PrincipalKind, Resource};
use mt_refdata::{Change, RefItem, RefSet, SetScope, State, Structure};
use serde_json::json;
use std::collections::BTreeSet;

/// Longest name of a base, table, field, code list or person.
pub const MAX_NAME_CHARS: usize = 80;

fn name_of(text: &str, what: &str) -> ApiResult<String> {
    let name = text.trim();
    if name.is_empty() {
        return Err(ApiError::invalid(
            format!("give the {what} a name"),
            json!(null),
        ));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(ApiError::invalid(
            format!("a {what} name has at most {MAX_NAME_CHARS} characters"),
            json!(null),
        ));
    }
    Ok(name.to_owned())
}

fn tenant_require(ws: &Workspace, actor: Actor, action: Action, now: Timestamp) -> ApiResult<()> {
    ws.require(actor, &action.into(), Resource::Tenant, None, now)
}

// ------------------------------------------------------------------- bases

/// Creates a base.
///
/// # Errors
/// 403 without `schema.manage`; 422 without a name; 409 for a name in use.
pub fn create_base(
    ws: &mut Workspace,
    actor: Actor,
    name: &str,
    description: &str,
    now: Timestamp,
) -> ApiResult<BaseId> {
    tenant_require(ws, actor, Action::SchemaManage, now)?;
    let name = name_of(name, "base")?;
    if ws
        .data
        .bases
        .iter()
        .any(|b| b.name.eq_ignore_ascii_case(&name))
    {
        return Err(ApiError::conflict(format!(
            "there is a base named “{name}” already"
        )));
    }
    let id = ws.new_id(now, BaseId::new)?;
    ws.data.bases.push(Base {
        id,
        name: name.clone(),
        description: description.trim().to_owned(),
    });
    ws.audit_object(
        actor,
        "base.create",
        ObjectRef::new("base", id.to_string()),
        json!({"name": name}),
        now,
    )?;
    Ok(id)
}

/// Renames a base or changes its description.
///
/// # Errors
/// 404, 403, 422, 409 as for creation.
pub fn update_base(
    ws: &mut Workspace,
    actor: Actor,
    id: BaseId,
    name: Option<&str>,
    description: Option<&str>,
    now: Timestamp,
) -> ApiResult<()> {
    let base = ws.base(id)?;
    ws.require(
        actor,
        &Action::SchemaManage.into(),
        base.resource(),
        None,
        now,
    )?;
    let new_name = name.map(|n| name_of(n, "base")).transpose()?;
    if let Some(n) = &new_name {
        if ws
            .data
            .bases
            .iter()
            .any(|b| b.id != id && b.name.eq_ignore_ascii_case(n))
        {
            return Err(ApiError::conflict(format!(
                "there is a base named “{n}” already"
            )));
        }
    }
    let base = ws
        .data
        .bases
        .iter_mut()
        .find(|b| b.id == id)
        .ok_or_else(|| ApiError::not_found(format!("base {id}")))?;
    if let Some(n) = new_name {
        base.name = n;
    }
    if let Some(d) = description {
        base.description = d.trim().to_owned();
    }
    let details = json!({"name": base.name, "description": base.description});
    ws.audit_object(
        actor,
        "base.update",
        ObjectRef::new("base", id.to_string()),
        details,
        now,
    )
}

/// Deletes an empty base.
///
/// # Errors
/// 404, 403, 409 while it has tables.
pub fn delete_base(ws: &mut Workspace, actor: Actor, id: BaseId, now: Timestamp) -> ApiResult<()> {
    let base = ws.base(id)?;
    ws.require(
        actor,
        &Action::SchemaManage.into(),
        base.resource(),
        None,
        now,
    )?;
    if ws.data.tables.iter().any(|t| t.base == id) {
        return Err(ApiError::conflict(format!(
            "“{}” still has tables: delete them first",
            base.name
        )));
    }
    let name = base.name.clone();
    ws.data.bases.retain(|b| b.id != id);
    ws.audit_object(
        actor,
        "base.delete",
        ObjectRef::new("base", id.to_string()),
        json!({"name": name}),
        now,
    )
}

// ------------------------------------------------------------------- tables

/// Creates a table with its primary field (a required text field that names the records).
///
/// # Errors
/// 404 for an unknown base, 403, 422, 409 for a name in use in the base.
pub fn create_table(
    ws: &mut Workspace,
    actor: Actor,
    base: BaseId,
    name: &str,
    description: &str,
    primary: &str,
    now: Timestamp,
) -> ApiResult<TableId> {
    let owner = ws.base(base)?;
    ws.require(
        actor,
        &Action::SchemaManage.into(),
        owner.resource(),
        None,
        now,
    )?;
    let name = name_of(name, "table")?;
    let primary_name = name_of(
        if primary.trim().is_empty() {
            "Name"
        } else {
            primary
        },
        "field",
    )?;
    if ws
        .data
        .tables
        .iter()
        .any(|t| t.base == base && t.name.eq_ignore_ascii_case(&name))
    {
        return Err(ApiError::conflict(format!(
            "the base has a table named “{name}” already"
        )));
    }
    let id = ws.new_id(now, TableId::new)?;
    let field_id = ws.new_field_id(now)?;
    let mut field = BuiltinType::Text.template(field_id, primary_name);
    field.validation.required = true;
    field
        .check()
        .map_err(|e| ApiError::invalid(e.to_string(), json!(null)))?;
    ws.data.tables.push(Table {
        id,
        base,
        name: name.clone(),
        description: description.trim().to_owned(),
        primary: field_id,
        fields: vec![field],
        groups: Vec::new(),
        records: Vec::new(),
    });
    ws.audit(actor, "table.create", id, None, json!({"name": name}), now)?;
    Ok(id)
}

/// Renames a table or changes its description.
///
/// # Errors
/// 404, 403, 422, 409.
pub fn update_table(
    ws: &mut Workspace,
    actor: Actor,
    id: TableId,
    name: Option<&str>,
    description: Option<&str>,
    now: Timestamp,
) -> ApiResult<()> {
    let table = ws.table(id)?;
    ws.require(
        actor,
        &Action::SchemaManage.into(),
        table.resource(),
        None,
        now,
    )?;
    let base = table.base;
    let new_name = name.map(|n| name_of(n, "table")).transpose()?;
    if let Some(n) = &new_name {
        if ws
            .data
            .tables
            .iter()
            .any(|t| t.id != id && t.base == base && t.name.eq_ignore_ascii_case(n))
        {
            return Err(ApiError::conflict(format!(
                "the base has a table named “{n}” already"
            )));
        }
    }
    let table = ws.table_mut(id)?;
    if let Some(n) = new_name {
        table.name = n;
    }
    if let Some(d) = description {
        table.description = d.trim().to_owned();
    }
    let details = json!({"name": table.name, "description": table.description});
    ws.audit(actor, "table.update", id, None, details, now)
}

/// Deletes a table with its records. Refused while other tables link to it or a process
/// runs on it.
///
/// # Errors
/// 404, 403, 409 naming what still uses it.
pub fn delete_table(
    ws: &mut Workspace,
    actor: Actor,
    id: TableId,
    now: Timestamp,
) -> ApiResult<()> {
    let table = ws.table(id)?;
    ws.require(
        actor,
        &Action::SchemaManage.into(),
        table.resource(),
        None,
        now,
    )?;
    let inverse: BTreeSet<(TableId, FieldId)> = ws
        .data
        .links
        .iter()
        .filter(|p| p.table == id || p.inverse_table == id)
        .flat_map(|p| [(p.table, p.field), (p.inverse_table, p.inverse_field)])
        .collect();
    let mut users = Vec::new();
    for other in ws.data.tables.iter().filter(|t| t.id != id) {
        for field in &other.fields {
            let links_here = matches!(field.options, OptionsSource::Table { table } if table == id);
            if links_here && !inverse.contains(&(other.id, field.id)) {
                users.push(format!("“{}” in “{}”", field.name, other.name));
            }
        }
    }
    // Mirrored link fields in other tables go with the table, unless formulas read them.
    for (other, field) in &inverse {
        if *other != id {
            users.extend(usages(ws, *other, *field, Some(id)));
        }
    }
    if crate::process::binding(ws, id).is_some() {
        users.push("its lifecycle".to_owned());
    }
    if !users.is_empty() {
        return Err(ApiError::conflict(format!(
            "“{}” is still used by {}",
            table.name,
            users.join(", ")
        )));
    }
    let name = table.name.clone();
    let count = table.records.len();
    ws.data.tables.retain(|t| t.id != id);
    for (other, field) in inverse.iter().filter(|(t, _)| *t != id) {
        if let Ok(t) = ws.table_mut(*other) {
            t.fields.retain(|f| f.id != *field);
            for record in &mut t.records {
                record.cells.remove(field);
            }
        }
    }
    ws.data
        .links
        .retain(|p| p.table != id && p.inverse_table != id);
    ws.rebuild_graph()?;
    ws.recompute(now);
    ws.audit(
        actor,
        "table.delete",
        id,
        None,
        json!({"name": name, "records": count}),
        now,
    )
}

// ------------------------------------------------------------------- fields

/// What else refers to a field: formulas, lookups and roll-ups (in any table), lifecycles,
/// approvals and access rules. `ignore` skips the fields of a table that is going away.
#[must_use]
pub fn usages(
    ws: &Workspace,
    table: TableId,
    field: FieldId,
    ignore: Option<TableId>,
) -> Vec<String> {
    let mut found = Vec::new();
    for other in ws.data.tables.iter().filter(|t| Some(t.id) != ignore) {
        for candidate in &other.fields {
            let Ok(Some(expr)) = mt_formula::field::formula_of(candidate) else {
                continue;
            };
            let deps = mt_formula::deps::dependencies(&expr);
            let reads = (other.id == table && deps.fields.contains(&field))
                || deps.through.iter().any(|(link, target)| {
                    *target == field && ws.link_target(other.id, *link) == Some(table)
                        || (other.id == table && *link == field)
                });
            if reads && !(other.id == table && candidate.id == field) {
                found.push(format!(
                    "the formula “{}” in “{}”",
                    candidate.name, other.name
                ));
            }
        }
    }
    let id = field.to_string();
    for binding in &ws.data.process.bindings {
        let text = serde_json::to_string(binding).unwrap_or_default();
        if text.contains(&id) {
            found.push(format!("the lifecycle “{}”", binding.lifecycle.name));
        }
    }
    for def in &ws.data.process.definitions {
        let text = serde_json::to_string(def).unwrap_or_default();
        if text.contains(&id) {
            found.push(format!("the approval “{} v{}”", def.name, def.version));
        }
    }
    let policy = serde_json::to_string(&ws.data.policy).unwrap_or_default();
    if policy.contains(&id) {
        found.push("access rules".to_owned());
    }
    found.sort();
    found.dedup();
    found
}

/// Adds a link field and, for a two-way link, its mirror in the target table.
///
/// # Errors
/// As [`Workspace::add_field`]; nothing is kept when the mirror cannot be added.
#[allow(clippy::too_many_arguments)]
pub fn add_link(
    ws: &mut Workspace,
    actor: Actor,
    table: TableId,
    mut field: FieldDef,
    target: TableId,
    inverse: Option<(&str, bool)>,
    now: Timestamp,
) -> ApiResult<FieldId> {
    ws.table(target)?;
    field.options = OptionsSource::Table { table: target };
    let id = ws.add_field(actor, table, field, now)?;
    let Some((inverse_name, inverse_single)) = inverse else {
        return Ok(id);
    };
    let inverse_id = ws.new_field_id(now)?;
    let mut mirror = BuiltinType::Link.template(inverse_id, inverse_name.trim());
    mirror.options = OptionsSource::Table { table };
    if inverse_single {
        mirror.validation.max_items = Some(1);
    }
    if let Err(error) = ws.add_field(actor, target, mirror, now) {
        // Undo the first half: nothing of a refused operation stays.
        ws.table_mut(table)?.fields.retain(|f| f.id != id);
        ws.rebuild_graph()?;
        return Err(error);
    }
    ws.data.links.push(LinkPair {
        table,
        field: id,
        inverse_table: target,
        inverse_field: inverse_id,
    });
    Ok(id)
}

/// What can change about a field after it was created.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldPatch {
    /// A new name.
    #[serde(default)]
    pub name: Option<String>,
    /// Required or not.
    #[serde(default)]
    pub required: Option<bool>,
    /// Help text ("" removes it).
    #[serde(default)]
    pub help: Option<String>,
    /// Choices of a single or multiple select, in order: existing ones by id (renamed if the
    /// label differs), new ones without. Choices left out are disabled, never deleted, so
    /// records keep their values.
    #[serde(default)]
    pub options: Option<Vec<OptionPatch>>,
    /// Required only when a condition holds (`null` removes the condition).
    #[serde(default, rename = "requiredWhen", deserialize_with = "present")]
    pub required_when: Option<serde_json::Value>,
}

/// Keeps an explicit `null` (as `Some(Value::Null)`), so it can be told apart from "left out".
fn present<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<serde_json::Value>, D::Error> {
    serde::Deserialize::deserialize(deserializer).map(Some)
}

/// One choice in a [`FieldPatch`].
#[derive(Debug, Clone, serde::Deserialize)]
pub struct OptionPatch {
    /// The choice, if it exists.
    #[serde(default)]
    pub id: Option<RefItemId>,
    /// Its label.
    pub label: String,
}

/// Changes a field's name, required flag, help text or choices.
///
/// # Errors
/// 404, 403, 422 for a change the field framework refuses, 409 for a name in use.
pub fn update_field(
    ws: &mut Workspace,
    actor: Actor,
    table_id: TableId,
    field_id: FieldId,
    patch: &FieldPatch,
    now: Timestamp,
) -> ApiResult<()> {
    let table = ws.table(table_id)?;
    ws.require(
        actor,
        &Action::SchemaManage.into(),
        table.resource(),
        None,
        now,
    )?;
    let mut field = table
        .field(field_id)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("field {field_id}")))?;
    let mut changes = Vec::new();
    if let Some(name) = &patch.name {
        let name = name_of(name, "field")?;
        if table
            .fields
            .iter()
            .any(|f| f.id != field_id && f.name.eq_ignore_ascii_case(&name))
        {
            return Err(ApiError::conflict(format!(
                "{} already has a field named “{name}”",
                table.name
            )));
        }
        if name != field.name {
            changes.push(json!({"what": "name", "from": field.name, "to": name}));
            field.name = name;
        }
    }
    if let Some(required) = patch.required {
        if required && field.is_generated() {
            return Err(ApiError::invalid(
                "computed and system fields fill themselves",
                json!(null),
            ));
        }
        if required != field.validation.required {
            changes.push(json!({"what": "required", "to": required}));
            field.validation.required = required;
        }
    }
    if let Some(help) = &patch.help {
        let help = help.trim();
        field.help_text = (!help.is_empty()).then(|| help.to_owned());
        changes.push(json!({"what": "help"}));
    }
    if let Some(wanted) = &patch.options {
        let OptionsSource::Inline { options } = &field.options else {
            return Err(ApiError::invalid(
                "only select fields have their own choices",
                json!(null),
            ));
        };
        let mut next = Vec::new();
        for (index, choice) in wanted.iter().enumerate() {
            let label = name_of(&choice.label, "choice")?;
            let id = match choice.id {
                Some(id) => {
                    if !options.iter().any(|o| o.id == id) {
                        return Err(ApiError::not_found(format!("choice {id}")));
                    }
                    id
                }
                None => RefItemId::from_u128(
                    u128::from(ws.new_field_id(now)?.timestamp_ms())
                        .saturating_mul(1000)
                        .saturating_add(u128::try_from(index).unwrap_or(0))
                        .saturating_add(u128::from(ws.data.next_id)),
                ),
            };
            let color = options
                .iter()
                .find(|o| o.id == id)
                .and_then(|o| o.color.clone());
            next.push(mt_core::field::InlineOption {
                id,
                label,
                color,
                enabled: true,
            });
        }
        for old in options {
            if !next.iter().any(|o| o.id == old.id) {
                next.push(mt_core::field::InlineOption {
                    enabled: false,
                    ..old.clone()
                });
            }
        }
        if !next.iter().any(|o| o.enabled) {
            return Err(ApiError::invalid("keep at least one choice", json!(null)));
        }
        changes.push(json!({"what": "choices", "to": next.iter().filter(|o| o.enabled).map(|o| o.label.clone()).collect::<Vec<_>>()}));
        field.options = OptionsSource::Inline { options: next };
    }
    if let Some(wanted) = &patch.required_when {
        if wanted.is_null() {
            if field.validation.required_when.take().is_some() {
                changes.push(json!({"what": "required when", "to": null}));
            }
        } else {
            if field.is_generated() {
                return Err(ApiError::invalid(
                    "computed and system fields fill themselves",
                    json!(null),
                ));
            }
            let simple: crate::design::SimpleCondition = serde_json::from_value(wanted.clone())
                .map_err(|e| ApiError::invalid(format!("required when: {e}"), json!(null)))?;
            if simple.field == field_id {
                return Err(ApiError::invalid(
                    "a field cannot be required because of its own value",
                    json!(null),
                ));
            }
            let table = ws.table(table_id)?;
            let condition = crate::design::condition(table, &simple)?;
            changes.push(json!({"what": "required when", "to": crate::process::condition_text(ws, table, &condition)}));
            field.validation.required_when = Some(condition);
        }
    }
    field
        .check()
        .map_err(|e| ApiError::invalid(e.to_string(), json!(null)))?;
    let name = field.name.clone();
    let table = ws.table_mut(table_id)?;
    if let Some(slot) = table.fields.iter_mut().find(|f| f.id == field_id) {
        *slot = field;
    }
    ws.rebuild_graph()?;
    ws.audit(
        actor,
        "field.update",
        table_id,
        None,
        json!({"field": name, "changes": changes}),
        now,
    )
}

/// Deletes a field with its values (and the mirror of a two-way link). Refused for the
/// primary field and while formulas, lifecycles, approvals or access rules use it.
///
/// # Errors
/// 404, 403, 409 naming what uses it.
pub fn delete_field(
    ws: &mut Workspace,
    actor: Actor,
    table_id: TableId,
    field_id: FieldId,
    now: Timestamp,
) -> ApiResult<()> {
    let table = ws.table(table_id)?;
    ws.require(
        actor,
        &Action::SchemaManage.into(),
        table.resource(),
        None,
        now,
    )?;
    let field = table
        .field(field_id)
        .ok_or_else(|| ApiError::not_found(format!("field {field_id}")))?;
    if table.primary == field_id {
        return Err(ApiError::conflict(format!(
            "“{}” names the records of {}: it cannot be deleted",
            field.name, table.name
        )));
    }
    let mirror: Option<(TableId, FieldId)> = ws.data.links.iter().find_map(|p| {
        if p.table == table_id && p.field == field_id {
            Some((p.inverse_table, p.inverse_field))
        } else if p.inverse_table == table_id && p.inverse_field == field_id {
            Some((p.table, p.field))
        } else {
            None
        }
    });
    let mut users = usages(ws, table_id, field_id, None);
    if let Some((t, f)) = mirror {
        users.extend(
            usages(ws, t, f, None)
                .into_iter()
                .filter(|u| !u.contains(&field.name)),
        );
    }
    if !users.is_empty() {
        return Err(ApiError::conflict(format!(
            "“{}” is used by {}",
            field.name,
            users.join(", ")
        )));
    }
    let name = field.name.clone();
    for (t, f) in [(table_id, field_id)].into_iter().chain(mirror) {
        let table = ws.table_mut(t)?;
        table.fields.retain(|x| x.id != f);
        for group in &mut table.groups {
            group.fields.retain(|x| *x != f);
        }
        for record in &mut table.records {
            record.cells.remove(&f);
            record.errors.remove(&f);
        }
    }
    ws.data.links.retain(|p| {
        !(p.table == table_id && p.field == field_id)
            && !(p.inverse_table == table_id && p.inverse_field == field_id)
    });
    ws.rebuild_graph()?;
    ws.recompute(now);
    ws.audit(
        actor,
        "field.delete",
        table_id,
        None,
        json!({"field": name}),
        now,
    )
}

/// The role a person field plays, by key (`requester`, `owner`, `assignee`).
///
/// # Errors
/// 422 for an unknown role.
pub fn person_role(key: &str) -> ApiResult<Option<FieldRole>> {
    match key {
        "" | "none" => Ok(None),
        "requester" => Ok(Some(FieldRole::PartyRequester)),
        "owner" => Ok(Some(FieldRole::PersonOwner)),
        "assignee" => Ok(Some(FieldRole::PersonAssignee)),
        other => Err(ApiError::invalid(
            format!("`{other}` is not a person role"),
            json!(null),
        )),
    }
}

// ------------------------------------------------------------------- code lists

fn code_of(text: &str, what: &str) -> ApiResult<String> {
    let code = text.trim();
    let ok = !code.is_empty()
        && code.len() <= 64
        && code
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if ok {
        Ok(code.to_owned())
    } else {
        Err(ApiError::invalid(
            format!("a {what} code is 1–64 letters, digits, `_`, `-` or `.`"),
            json!(null),
        ))
    }
}

pub(crate) fn refset_require(ws: &Workspace, actor: Actor, now: Timestamp) -> ApiResult<()> {
    tenant_require(ws, actor, Action::RefDataManage, now)
}

/// Creates a code list (a tenant reference set, flat or a tree).
///
/// # Errors
/// 403 without `refdata.manage`; 422; 409 for a code in use.
pub fn create_refset(
    ws: &mut Workspace,
    actor: Actor,
    code: &str,
    name: &str,
    tree: bool,
    now: Timestamp,
) -> ApiResult<RefSetId> {
    refset_require(ws, actor, now)?;
    let code = code_of(code, "code list")?.to_lowercase();
    let name = name_of(name, "code list")?;
    if ws
        .data
        .refsets
        .iter()
        .any(|s| s.set.code.eq_ignore_ascii_case(&code))
    {
        return Err(ApiError::conflict(format!(
            "there is a code list `{code}` already"
        )));
    }
    let id = ws.new_id(now, RefSetId::new)?;
    let structure = if tree {
        Structure::Tree { max_depth: 6 }
    } else {
        Structure::Flat
    };
    let state = State::new(
        RefSet::new(id, code.clone(), name.clone(), structure, SetScope::Tenant),
        Vec::new(),
    );
    state
        .effective()
        .map_err(|e| ApiError::invalid(e.to_string(), json!(null)))?;
    ws.data.refsets.push(state);
    ws.rebuild_refdata()?;
    ws.audit_object(
        actor,
        "refset.create",
        ObjectRef::new("refset", id.to_string()),
        json!({"code": code, "name": name, "tree": tree}),
        now,
    )?;
    Ok(id)
}

fn change_refset(
    ws: &mut Workspace,
    actor: Actor,
    set: RefSetId,
    change: Change,
    what: &str,
    now: Timestamp,
) -> ApiResult<crate::codelist::Outcome> {
    crate::codelist::change(ws, actor, set, change, what, now)
}

/// Adds an item to a code list (under a parent, in a tree) — at once, or as a proposal
/// when the list needs a second person's approval.
///
/// # Errors
/// 404, 403, 422 for a code in use, an unknown parent or a parent in a flat list.
pub fn add_item(
    ws: &mut Workspace,
    actor: Actor,
    set: RefSetId,
    code: &str,
    name: &str,
    parent: Option<RefItemId>,
    now: Timestamp,
) -> ApiResult<(RefItemId, crate::codelist::Outcome)> {
    let code = code_of(code, "item")?;
    let name = name_of(name, "item")?;
    let id = ws.new_id(now, RefItemId::new)?;
    let mut item = RefItem::new(id, code, name);
    item.parent = parent;
    item.sort = i64::try_from(
        ws.data
            .refsets
            .iter()
            .find(|s| s.set.id == set)
            .map_or(0, |s| s.items.len()),
    )
    .unwrap_or(0);
    let outcome = change_refset(ws, actor, set, Change::Add { item }, "refset.item.add", now)?;
    Ok((id, outcome))
}

/// Enables or disables an item (disabled items stay on records, but cannot be chosen) — at
/// once, or as a proposal.
///
/// # Errors
/// 404, 403, 422.
pub fn set_item_enabled(
    ws: &mut Workspace,
    actor: Actor,
    set: RefSetId,
    item: RefItemId,
    enabled: bool,
    now: Timestamp,
) -> ApiResult<crate::codelist::Outcome> {
    change_refset(
        ws,
        actor,
        set,
        Change::SetEnabled { id: item, enabled },
        "refset.item.enable",
        now,
    )
}

// ------------------------------------------------------------------- settings

fn param_key(key: &str) -> ApiResult<String> {
    let key = key.trim();
    let ok = key.len() <= 64
        && key.split('.').all(|part| {
            let mut chars = part.chars();
            chars.next().is_some_and(|c| c.is_ascii_lowercase())
                && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        });
    if ok {
        Ok(key.to_owned())
    } else {
        Err(ApiError::invalid(
            "a setting key is lower-case words joined by dots, like `expense.manager_limit`",
            json!(null),
        ))
    }
}

/// Sets a parameter formulas read with `PARAM("key")`; recomputes what depends on it.
///
/// # Errors
/// 403 without `settings.manage`; 422 for a bad key or an empty value.
pub fn set_param(
    ws: &mut Workspace,
    actor: Actor,
    key: &str,
    value: Value,
    now: Timestamp,
) -> ApiResult<()> {
    tenant_require(ws, actor, Action::SettingsManage, now)?;
    let key = param_key(key)?;
    if value.is_empty() {
        return Err(ApiError::invalid("give the setting a value", json!(null)));
    }
    let shown = serde_json::to_value(&value).unwrap_or(json!(null));
    ws.data.params.insert(key.clone(), value);
    ws.recompute(now);
    ws.audit_object(
        actor,
        "setting.update",
        ObjectRef::new("setting", key.clone()),
        json!({"key": key, "value": shown}),
        now,
    )
}

/// Reads a setting value typed as text: `1000 USD`, `8%`, `12.5`, or plain text.
///
/// # Errors
/// 422 when the text does not fit the type.
pub fn param_value(kind: &str, text: &str) -> ApiResult<Value> {
    let data_type = match kind {
        "money" => DataType::Money,
        "number" => DataType::Number,
        "percent" => DataType::Percent,
        "text" => DataType::Text,
        "bool" => DataType::Bool,
        other => {
            return Err(ApiError::invalid(
                format!("`{other}` is not a setting type"),
                json!(null),
            ))
        }
    };
    mt_core::coerce::parse_input(data_type, text, &mt_core::coerce::ParseContext::default())
        .map_err(|e| ApiError::invalid(e.to_string(), json!(null)))
}

// ------------------------------------------------------------------- people

/// Adds a person, guest, agent or group.
///
/// # Errors
/// 403 without `members.manage`; 422; 404 for an unknown member.
pub fn create_person(
    ws: &mut Workspace,
    actor: Actor,
    name: &str,
    kind: PrincipalKind,
    title: &str,
    members: &[PrincipalId],
    now: Timestamp,
) -> ApiResult<PrincipalId> {
    tenant_require(ws, actor, Action::MembersManage, now)?;
    let name = name_of(name, "person")?;
    if kind != PrincipalKind::Group && !members.is_empty() {
        return Err(ApiError::invalid("only groups have members", json!(null)));
    }
    for member in members {
        ws.person(*member)
            .ok_or_else(|| ApiError::not_found(format!("person {member}")))?;
    }
    let id = ws.new_id(now, PrincipalId::new)?;
    ws.data.people.push(Person {
        id,
        name: name.clone(),
        kind,
        title: title.trim().to_owned(),
        members: members.to_vec(),
    });
    ws.audit_object(
        actor,
        "member.create",
        ObjectRef::new("principal", id.to_string()),
        json!({"name": name, "kind": kind}),
        now,
    )?;
    Ok(id)
}

/// Renames someone, changes their title, or a group's members.
///
/// # Errors
/// 404, 403, 422 (members of a non-group, a group inside itself).
pub fn update_person(
    ws: &mut Workspace,
    actor: Actor,
    id: PrincipalId,
    name: Option<&str>,
    title: Option<&str>,
    members: Option<&[PrincipalId]>,
    now: Timestamp,
) -> ApiResult<()> {
    tenant_require(ws, actor, Action::MembersManage, now)?;
    let person = ws
        .person(id)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("person {id}")))?;
    let mut next = person.clone();
    if let Some(n) = name {
        next.name = name_of(n, "person")?;
    }
    if let Some(t) = title {
        next.title = t.trim().to_owned();
    }
    if let Some(list) = members {
        if person.kind != PrincipalKind::Group {
            return Err(ApiError::invalid("only groups have members", json!(null)));
        }
        for member in list {
            ws.person(*member)
                .ok_or_else(|| ApiError::not_found(format!("person {member}")))?;
            if *member == id || mt_perm::Directory::groups_of(ws, id).contains(member) {
                return Err(ApiError::invalid(
                    "a group cannot contain itself",
                    json!(null),
                ));
            }
        }
        next.members = list.to_vec();
    }
    let details = json!({"name": next.name, "members": next.members.iter().map(|m| ws.name_of(*m)).collect::<Vec<_>>()});
    if let Some(slot) = ws.data.people.iter_mut().find(|p| p.id == id) {
        *slot = next;
    }
    ws.audit_object(
        actor,
        "member.update",
        ObjectRef::new("principal", id.to_string()),
        details,
        now,
    )
}

// ------------------------------------------------------------------- templates

/// Installs a template by key.
///
/// # Errors
/// 403 without `template.manage`; 404 for an unknown template; 409 when installed already.
pub fn install_template(
    ws: &mut Workspace,
    actor: Actor,
    key: &str,
    now: Timestamp,
) -> ApiResult<()> {
    tenant_require(ws, actor, Action::TemplateManage, now)?;
    match key {
        "purchasing" => crate::seed::install_purchasing(ws, actor, now),
        other => Err(ApiError::not_found(format!("template `{other}`"))),
    }
}

/// Whether a template is installed.
#[must_use]
pub fn installed(ws: &Workspace, key: &str) -> bool {
    match key {
        "purchasing" => ws.data.bases.iter().any(|b| b.id == crate::model::BASE),
        _ => false,
    }
}

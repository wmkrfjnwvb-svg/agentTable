//! Settings: typed parameters formulas read with `PARAM("key")`, set for the whole workspace
//! and overridden per base, each with its history and an explanation of where a value comes
//! from.
//!
//! The rules are `mt-refdata`'s parameter registry: a parameter has one type for good (a
//! value of another type is refused), resolution takes the base's override before the
//! workspace value, and every change — clearing an override included — is kept. A formula
//! resolves parameters with the base of the record it computes, so the same formula gives
//! each base its own limit.

use crate::error::{ApiError, ApiResult};
use crate::model::Workspace;
use crate::view;
use mt_audit::ObjectRef;
use mt_core::field::{Classification, DataType, FieldDef};
use mt_core::id::{BaseId, FieldId, PrincipalId};
use mt_core::time::Timestamp;
use mt_core::validate::AcceptAllOptions;
use mt_core::value::Value;
use mt_perm::{Action, Actor, Resource};
use mt_refdata::{
    ParamContext, ParamDef, ParamKind, RefDataError, Registry, Scope, ScopeLevel, Setting, Source,
};
use serde_json::{json, Value as Json};
use std::collections::BTreeSet;

/// Who made changes that no person made: settings carried over from data saved before
/// settings had scopes and a history.
pub const SYSTEM: PrincipalId = PrincipalId::from_u128(0);

fn error(error: RefDataError) -> ApiError {
    let text = error.to_string();
    match error {
        RefDataError::UnknownParameter { .. } => ApiError::not_found(text),
        RefDataError::Invalid { .. }
        | RefDataError::DuplicateCode { .. }
        | RefDataError::DuplicateItem { .. }
        | RefDataError::UnknownItem { .. }
        | RefDataError::UnknownSet { .. }
        | RefDataError::UnknownParent { .. }
        | RefDataError::FlatSetHasParent { .. }
        | RefDataError::Cycle { .. }
        | RefDataError::TooDeep { .. }
        | RefDataError::AliasConflict { .. }
        | RefDataError::SystemItem { .. }
        | RefDataError::OverlayNotAllowed
        | RefDataError::HasChildren { .. }
        | RefDataError::VersionConflict { .. }
        | RefDataError::ApprovalRequired
        | RefDataError::SelfApproval
        | RefDataError::NotPending { .. }
        | RefDataError::NotProposer
        | RefDataError::EmptyChange
        | RefDataError::Limit(_)
        | RefDataError::UnknownVersion { .. }
        | RefDataError::InvalidParameter { .. }
        | RefDataError::ScopeNotAllowed { .. }
        | RefDataError::Core(_) => ApiError::invalid(text, json!(null)),
    }
}

/// The name of a setting type, as the settings page offers them.
#[must_use]
pub fn kind_of(def: &ParamDef) -> &'static str {
    match &def.kind {
        ParamKind::Secret => "secret",
        ParamKind::Value { data_type, .. } => match data_type {
            DataType::Money => "money",
            DataType::Number => "number",
            DataType::Percent => "percent",
            DataType::Bool => "bool",
            DataType::Text
            | DataType::LongText
            | DataType::Date
            | DataType::DateTime
            | DataType::Duration
            | DataType::Reference
            | DataType::MultiReference
            | DataType::Link
            | DataType::Principal
            | DataType::Attachment
            | DataType::Json => "text",
        },
    }
}

/// A new parameter of the value's type, set for the workspace and overridable per base.
fn definition(key: &str, value: &Value) -> ApiResult<ParamDef> {
    let data_type = value
        .data_type()
        .ok_or_else(|| ApiError::invalid("give the setting a value", json!(null)))?;
    Ok(ParamDef {
        key: key.to_owned(),
        description: format!("Read by formulas with PARAM(\"{key}\")"),
        kind: ParamKind::Value {
            data_type,
            set: None,
            validation: Box::default(),
        },
        default: None,
        overridable_at: BTreeSet::from([ScopeLevel::Tenant, ScopeLevel::Base]),
        sensitivity: Classification::Internal,
        critical: false,
        owner: None,
    })
}

/// Sets a value in a registry, defining the parameter on first use.
///
/// # Errors
/// 422 for a bad key, a value of another type than the parameter's, or a scope the
/// parameter cannot be overridden at.
pub fn put(
    registry: &mut Registry,
    key: &str,
    value: Value,
    scope: Scope,
    (by, at): (PrincipalId, Timestamp),
) -> ApiResult<()> {
    if registry.definition(key).is_none() {
        registry
            .define(definition(key, &value)?, &AcceptAllOptions)
            .map_err(error)?;
    }
    registry
        .set(key, scope, Setting::Value(value), by, at, &AcceptAllOptions)
        .map_err(error)
}

fn scope_of(base: Option<BaseId>) -> Scope {
    base.map_or(Scope::Tenant, |base| Scope::Base { base })
}

/// Settings of a base may be changed by whoever manages settings there.
fn require(ws: &Workspace, actor: Actor, base: Option<BaseId>, now: Timestamp) -> ApiResult<()> {
    let on = match base {
        Some(id) => ws.base(id)?.resource(),
        None => Resource::Tenant,
    };
    ws.require(actor, &Action::SettingsManage.into(), on, None, now)
}

fn person(actor: Actor) -> PrincipalId {
    actor.on_behalf_of.unwrap_or(actor.principal)
}

/// Sets a setting for the workspace, or overrides it for one base; recomputes what reads it.
///
/// # Errors
/// 403 without `settings.manage` (on the base, for a base override); 404 for an unknown
/// base; 422 for a bad key or a value of another type than the setting's.
pub fn set(
    ws: &mut Workspace,
    actor: Actor,
    key: &str,
    value: Value,
    base: Option<BaseId>,
    now: Timestamp,
) -> ApiResult<()> {
    require(ws, actor, base, now)?;
    let key = key.trim();
    if value.is_empty() {
        return Err(ApiError::invalid("give the setting a value", json!(null)));
    }
    let shown = display(ws, key, &value);
    put(
        ws.registry_mut(),
        key,
        value,
        scope_of(base),
        (person(actor), now),
    )?;
    ws.sync_settings();
    ws.recompute(now);
    ws.audit_object(
        actor,
        "setting.update",
        ObjectRef::new("setting", key.to_owned()),
        json!({"key": key, "value": shown, "for": place(ws, scope_of(base))}),
        now,
    )
}

/// Removes a base's override (or the workspace value), so the next level applies again.
///
/// # Errors
/// 403; 404 for an unknown setting or base, or when nothing is set there.
pub fn clear(
    ws: &mut Workspace,
    actor: Actor,
    key: &str,
    base: Option<BaseId>,
    now: Timestamp,
) -> ApiResult<()> {
    require(ws, actor, base, now)?;
    let scope = scope_of(base);
    let removed = ws
        .registry_mut()
        .clear(key, scope, person(actor), now)
        .map_err(error)?;
    if !removed {
        return Err(ApiError::not_found(format!(
            "a value of `{key}` for {}",
            place(ws, scope)
        )));
    }
    ws.sync_settings();
    ws.recompute(now);
    ws.audit_object(
        actor,
        "setting.clear",
        ObjectRef::new("setting", key.to_owned()),
        json!({"key": key, "for": place(ws, scope)}),
        now,
    )
}

/// Where a value is set, in words.
#[must_use]
pub fn place(ws: &Workspace, scope: Scope) -> String {
    match scope {
        Scope::Tenant => "the whole workspace".to_owned(),
        Scope::Base { base } => ws
            .base(base)
            .map_or_else(|_| base.to_string(), |b| format!("the base {}", b.name)),
        Scope::User { user } => ws.name_of(user),
    }
}

/// A value as people read it.
#[must_use]
pub fn display(ws: &Workspace, key: &str, value: &Value) -> String {
    value.data_type().map_or_else(String::new, |data_type| {
        view::display(
            ws,
            &FieldDef::new(FieldId::from_u128(1), key, data_type),
            value,
        )
    })
}

fn setting_text(ws: &Workspace, key: &str, setting: Option<&Setting>) -> String {
    match setting {
        Some(Setting::Value(value)) => display(ws, key, value),
        Some(Setting::Secret(_)) => "a secret".to_owned(),
        None => "cleared".to_owned(),
    }
}

/// The settings page: each setting with its workspace value, its base overrides, and its
/// history (newest first).
#[must_use]
pub fn list(ws: &Workspace) -> Vec<Json> {
    let registry = ws.registry();
    registry
        .definitions()
        .map(|def| {
            let key = def.key.as_str();
            let overrides: Vec<Json> = registry
                .overrides(key)
                .into_iter()
                .map(|(scope, setting, entry)| {
                    json!({
                        "base": match scope {
                            Scope::Base { base } => Some(base),
                            Scope::Tenant | Scope::User { .. } => None,
                        },
                        "for": place(ws, scope),
                        "value": setting_text(ws, key, Some(setting)),
                        "by": ws.name_of(entry.by),
                        "at": entry.at,
                    })
                })
                .collect();
            let mut scopes: Vec<Scope> = registry
                .overrides(key)
                .into_iter()
                .map(|(scope, _, _)| scope)
                .collect();
            for base in &ws.data.bases {
                let scope = Scope::Base { base: base.id };
                if !scopes.contains(&scope) {
                    scopes.push(scope);
                }
            }
            if !scopes.contains(&Scope::Tenant) {
                scopes.push(Scope::Tenant);
            }
            // Newest first; within one scope, later entries are newer at the same instant.
            let mut history: Vec<((Timestamp, usize), Json)> =
                scopes
                    .iter()
                    .flat_map(|scope| {
                        registry.history(key, *scope).iter().enumerate().map(
                            move |(index, entry)| {
                                (
                                    (entry.at, index),
                                    json!({
                                        "for": place(ws, *scope),
                                        "value": setting_text(ws, key, entry.setting.as_ref()),
                                        "by": ws.name_of(entry.by),
                                        "at": entry.at,
                                    }),
                                )
                            },
                        )
                    })
                    .collect();
            history.sort_by_key(|entry| std::cmp::Reverse(entry.0));
            let tenant = registry
                .overrides(key)
                .into_iter()
                .find(|(scope, _, _)| *scope == Scope::Tenant)
                .map(|(_, setting, _)| setting_text(ws, key, Some(setting)));
            json!({
                "key": key,
                "type": kind_of(def),
                "description": def.description,
                "value": tenant.unwrap_or_default(),
                "overrides": overrides,
                "history": history.into_iter().map(|(_, entry)| entry).take(50).collect::<Vec<_>>(),
            })
        })
        .collect()
}

/// The value a setting has for a base (or the workspace), and why.
///
/// # Errors
/// 404 for an unknown setting or base.
pub fn explain(ws: &Workspace, key: &str, base: Option<BaseId>) -> ApiResult<Json> {
    if let Some(id) = base {
        ws.base(id)?;
    }
    let resolved = ws
        .registry()
        .resolve(key, &ParamContext { base, user: None })
        .map_err(error)?;
    let shown = resolved.value.map(|v| display(ws, key, v));
    let asked = place(ws, scope_of(base));
    let text = match (&shown, resolved.source) {
        (Some(value), Source::Override { scope, by, at }) => {
            let when = format!("by {} on {} UTC", ws.name_of(by), view::instant(at));
            if scope == scope_of(base) {
                format!("`{key}` is {value} for {asked}: set for {asked} {when}.")
            } else {
                format!(
                    "`{key}` is {value} for {asked}, which has no value of its own: the value for {} applies (set {when}).",
                    place(ws, scope)
                )
            }
        }
        (Some(value), Source::Default) => format!("`{key}` is {value} for {asked}: the default."),
        (Some(value), Source::Unset) => format!("`{key}` is {value} for {asked}."),
        (None, Source::Override { .. } | Source::Default | Source::Unset) => {
            format!("`{key}` has no value for {asked}.")
        }
    };
    Ok(json!({"key": key, "for": asked, "value": shown, "text": text}))
}

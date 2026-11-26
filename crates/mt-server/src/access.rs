//! Building access: who may do what, where, to which records, up to which limit — grants,
//! capability sets and separation-of-duties rules, given and revoked on screen.
//!
//! The rules themselves are `mt-perm`'s: a grant that would let someone hold two
//! capabilities a rule keeps apart is refused, a guest never receives administration, and a
//! revoked grant stays on record. Every change is audited.

use crate::error::{ApiError, ApiResult};
use crate::model::Workspace;
use mt_audit::ObjectRef;
use mt_core::id::{BaseId, CapabilitySetId, GrantId, PrincipalId, SodRuleId, TableId};
use mt_core::time::Timestamp;
use mt_lifecycle::approval::{Approver, Node};
use mt_perm::{
    Action, Actor, Capability, CapabilitySet, Grant, Granted, PermError, Resource, Scope, SodKind,
    SodRule,
};
use serde::Deserialize;
use serde_json::{json, Value as Json};

fn perm_error(error: PermError) -> ApiError {
    match error {
        PermError::SodConflict { .. } | PermError::Duplicate { .. } => {
            ApiError::conflict(error.to_string())
        }
        PermError::Invalid(_)
        | PermError::UnknownSet(_)
        | PermError::UnknownGrant(_)
        | PermError::AlreadyRevoked(_)
        | PermError::SetInUse { .. }
        | PermError::NotAllowed(_)
        | PermError::Review(_) => ApiError::invalid(error.to_string(), json!(null)),
    }
}

fn require(ws: &Workspace, actor: Actor, now: Timestamp) -> ApiResult<()> {
    ws.require(
        actor,
        &Action::AccessManage.into(),
        Resource::Tenant,
        None,
        now,
    )
}

/// Every capability a grant can give: the built-in actions and the custom ones the
/// configuration names (lifecycle buttons, approval steps, sets, grants).
#[must_use]
pub fn catalogue(ws: &Workspace) -> Vec<String> {
    let mut keys: Vec<String> = Action::ALL.iter().map(|a| a.key().to_owned()).collect();
    for set in ws.data.policy.sets() {
        keys.extend(set.capabilities.iter().map(|c| c.key().to_owned()));
    }
    for grant in ws.data.policy.grants() {
        if let Granted::Capability(capability) = &grant.what {
            keys.push(capability.key().to_owned());
        }
    }
    for binding in &ws.data.process.bindings {
        for transition in &binding.lifecycle.transitions {
            if let Some(key) = &transition.capability {
                if key != crate::process::SYSTEM {
                    keys.push(key.clone());
                }
            }
        }
    }
    for def in &ws.data.process.definitions {
        for node in def.nodes() {
            let approvers: &[Approver] = match node {
                Node::Approval { approvers, .. } => approvers,
                Node::Notify { to, .. } => to,
                Node::Branch { .. } | Node::Parallel { .. } | Node::FillIn { .. } => &[],
            };
            for approver in approvers {
                if let Approver::Capability { capability } = approver {
                    keys.push(capability.clone());
                }
            }
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

/// Where a grant applies.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Place {
    /// Everywhere.
    Workspace,
    /// One base.
    Base {
        /// The base.
        id: BaseId,
    },
    /// One table.
    Table {
        /// The table.
        id: TableId,
    },
}

/// A grant to give.
#[derive(Debug, Clone, Deserialize)]
pub struct GrantSpec {
    /// To whom (a person, group, guest or agent).
    pub to: PrincipalId,
    /// A capability key …
    #[serde(default)]
    pub capability: Option<String>,
    /// … or a capability set.
    #[serde(default)]
    pub set: Option<CapabilitySetId>,
    /// Where.
    pub on: Place,
    /// Which records: `all`, `own`, `team`.
    #[serde(default = "all")]
    pub scope: String,
    /// A limit on the record (tables only), e.g. `Total ≤ 1000 USD`.
    #[serde(default)]
    pub limit: Option<crate::design::SimpleCondition>,
    /// Why.
    #[serde(default)]
    pub reason: String,
}

fn all() -> String {
    "all".to_owned()
}

/// Gives a grant.
///
/// # Errors
/// 403 without `access.manage`; 404 for an unknown place; 422 for a malformed grant;
/// 409 when it breaks a separation-of-duties rule.
pub fn give(
    ws: &mut Workspace,
    actor: Actor,
    spec: &GrantSpec,
    now: Timestamp,
) -> ApiResult<GrantId> {
    require(ws, actor, now)?;
    let what = match (&spec.capability, spec.set) {
        (Some(key), None) => Granted::Capability(
            key.trim()
                .parse::<Capability>()
                .map_err(|e| ApiError::invalid(e.to_string(), json!(null)))?,
        ),
        (None, Some(set)) => Granted::Set(set),
        (Some(_), Some(_)) | (None, None) => {
            return Err(ApiError::invalid(
                "give either a capability or a capability set",
                json!(null),
            ))
        }
    };
    let (on, table) = match &spec.on {
        Place::Workspace => (Resource::Tenant, None),
        Place::Base { id } => (ws.base(*id)?.resource(), None),
        Place::Table { id } => {
            let table = ws.table(*id)?;
            (table.resource(), Some(table.clone()))
        }
    };
    let scope = match spec.scope.as_str() {
        "all" => Scope::All,
        "own" => Scope::Own,
        "team" => Scope::Team,
        other => {
            return Err(ApiError::invalid(
                format!("`{other}` is not a scope: all, own or team"),
                json!(null),
            ))
        }
    };
    let condition = match (&spec.limit, &table) {
        (Some(limit), Some(table)) => Some(crate::design::condition(table, limit)?),
        (Some(_), None) => {
            return Err(ApiError::invalid(
                "a limit compares a field of a table: grant on a table",
                json!(null),
            ))
        }
        (None, _) => None,
    };
    let reason = spec.reason.trim();
    let id = ws.new_id(now, GrantId::new)?;
    let mut grant = Grant::new(
        id,
        spec.to,
        what,
        on,
        actor.on_behalf_of.unwrap_or(actor.principal),
        now,
        if reason.is_empty() {
            "Given on the access page"
        } else {
            reason
        },
    );
    grant.scope = scope;
    grant.condition = condition;
    let summary = grant_json(ws, &grant);
    let mut policy = ws.data.policy.clone();
    policy.give(grant, ws).map_err(perm_error)?;
    ws.data.policy = policy;
    ws.audit_object(
        actor,
        "access.grant",
        ObjectRef::new("grant", id.to_string()),
        summary,
        now,
    )?;
    Ok(id)
}

/// Revokes a grant (it stays on record).
///
/// # Errors
/// 403, 404 for an unknown grant, 409 when revoked already.
pub fn revoke(ws: &mut Workspace, actor: Actor, id: GrantId, now: Timestamp) -> ApiResult<()> {
    require(ws, actor, now)?;
    let grant = ws
        .data
        .policy
        .grant(id)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("grant {id}")))?;
    if grant.revoked_at.is_some() {
        return Err(ApiError::conflict("the grant is revoked already"));
    }
    ws.data
        .policy
        .revoke(id, actor.on_behalf_of.unwrap_or(actor.principal), now)
        .map_err(perm_error)?;
    let summary = grant_json(ws, &grant);
    ws.audit_object(
        actor,
        "access.revoke",
        ObjectRef::new("grant", id.to_string()),
        summary,
        now,
    )
}

/// Creates a capability set.
///
/// # Errors
/// 403; 422 for an empty name or an unknown capability key.
pub fn create_set(
    ws: &mut Workspace,
    actor: Actor,
    name: &str,
    capabilities: &[String],
    now: Timestamp,
) -> ApiResult<CapabilitySetId> {
    require(ws, actor, now)?;
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::invalid("give the set a name", json!(null)));
    }
    let capabilities = capabilities
        .iter()
        .map(|k| {
            k.trim()
                .parse::<Capability>()
                .map_err(|e| ApiError::invalid(e.to_string(), json!(null)))
        })
        .collect::<ApiResult<std::collections::BTreeSet<Capability>>>()?;
    if capabilities.is_empty() {
        return Err(ApiError::invalid(
            "a set holds at least one capability",
            json!(null),
        ));
    }
    let id = ws.new_id(now, CapabilitySetId::new)?;
    ws.data
        .policy
        .add_set(CapabilitySet {
            id,
            name: name.to_owned(),
            description: None,
            capabilities: capabilities.clone(),
        })
        .map_err(perm_error)?;
    ws.audit_object(
        actor,
        "access.set.create",
        ObjectRef::new("capability_set", id.to_string()),
        json!({"name": name, "capabilities": capabilities.iter().map(|c| c.key().to_owned()).collect::<Vec<_>>()}),
        now,
    )?;
    Ok(id)
}

/// Adds a separation-of-duties rule.
///
/// # Errors
/// 403; 422 for unknown keys or a rule the policy already breaks.
pub fn add_rule(
    ws: &mut Workspace,
    actor: Actor,
    name: &str,
    first: &str,
    second: &str,
    kind: SodKind,
    now: Timestamp,
) -> ApiResult<SodRuleId> {
    require(ws, actor, now)?;
    let parse = |k: &str| {
        k.trim()
            .parse::<Capability>()
            .map_err(|e| ApiError::invalid(e.to_string(), json!(null)))
    };
    let id = ws.new_id(now, SodRuleId::new)?;
    let rule = SodRule {
        id,
        name: name.trim().to_owned(),
        first: parse(first)?,
        second: parse(second)?,
        kind,
    };
    ws.data.policy.add_rule(rule).map_err(perm_error)?;
    ws.audit_object(
        actor,
        "access.rule.create",
        ObjectRef::new("sod_rule", id.to_string()),
        json!({"name": name, "first": first, "second": second}),
        now,
    )?;
    Ok(id)
}

/// A grant as people read it.
#[must_use]
pub fn grant_json(ws: &Workspace, grant: &Grant) -> Json {
    let policy = &ws.data.policy;
    let what = match &grant.what {
        Granted::Capability(c) => c.key().to_owned(),
        Granted::Set(s) => policy
            .set(*s)
            .map_or_else(|| s.to_string(), |set| format!("set “{}”", set.name)),
    };
    let limit = grant
        .condition
        .as_ref()
        .and_then(|condition| match grant.on {
            Resource::Table { table, .. } => ws
                .table(table)
                .ok()
                .map(|t| crate::process::condition_text(ws, t, condition)),
            Resource::Tenant | Resource::Workspace { .. } | Resource::Base { .. } => None,
        });
    json!({
        "id": grant.id,
        "to": ws.name_of(grant.principal),
        "toId": grant.principal,
        "what": what,
        "on": crate::view::place_of(ws, &grant.on),
        "scope": crate::view::scope_of(ws, grant),
        "limit": limit,
        "reason": grant.reason,
        "grantedBy": ws.name_of(grant.granted_by),
        "active": grant.revoked_at.is_none(),
    })
}

/// The access page: grants, sets, rules and the capability catalogue.
#[must_use]
pub fn overview(ws: &Workspace) -> Json {
    let grants: Vec<Json> = ws.data.policy.grants().map(|g| grant_json(ws, g)).collect();
    let sets: Vec<Json> = ws
        .data
        .policy
        .sets()
        .map(|s| json!({"id": s.id, "name": s.name, "capabilities": s.capabilities.iter().map(|c| c.key().to_owned()).collect::<Vec<_>>()}))
        .collect();
    let rules: Vec<Json> = ws
        .data
        .policy
        .rules()
        .map(|r| json!({"name": r.name, "first": r.first.key(), "second": r.second.key(), "kind": r.kind}))
        .collect();
    json!({"grants": grants, "sets": sets, "rules": rules, "capabilities": catalogue(ws)})
}

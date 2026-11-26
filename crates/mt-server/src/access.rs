//! Building access: who may do what, where, to which records, up to which limit — grants,
//! capability sets and separation-of-duties rules, given and revoked on screen.
//!
//! The rules themselves are `mt-perm`'s: a grant that would let someone hold two
//! capabilities a rule keeps apart is refused, a guest never receives administration, and a
//! revoked grant stays on record. Every change is audited.

use crate::error::{ApiError, ApiResult};
use crate::model::{Table, Workspace};
use mt_audit::ObjectRef;
use mt_core::field::{DataType, OptionsSource};
use mt_core::fieldtype::BuiltinType;
use mt_core::id::{
    AccessReviewId, BaseId, CapabilitySetId, FieldId, GrantId, PrincipalId, RecordId, ShareLinkId,
    SodRuleId, TableId,
};
use mt_core::time::Timestamp;
use mt_lifecycle::approval::{Approver, Node};
use mt_perm::{
    AccessReview, Action, Actor, Capability, CapabilitySet, FieldAccess, Grant, Granted, Inactive,
    PermError, Resource, ReviewState, Scope, ShareLink, ShareMode, SodKind, SodRule, Verdict,
    Visibility,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use std::collections::BTreeSet;

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
    /// Only some fields, or all but some (tables only).
    #[serde(default)]
    pub fields: Option<FieldsSpec>,
    /// In force from (inclusive).
    #[serde(default, rename = "validFrom")]
    pub valid_from: Option<Timestamp>,
    /// In force until (exclusive).
    #[serde(default, rename = "validTo")]
    pub valid_to: Option<Timestamp>,
}

/// Which fields a grant or a share link covers: `{"only": [..]}` or `{"except": [..]}`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldsSpec {
    /// Only these fields.
    Only(Vec<FieldId>),
    /// Every field but these.
    Except(Vec<FieldId>),
}

fn field_access(table: &Table, spec: &FieldsSpec) -> ApiResult<FieldAccess> {
    let (ids, only) = match spec {
        FieldsSpec::Only(ids) => (ids, true),
        FieldsSpec::Except(ids) => (ids, false),
    };
    if ids.is_empty() {
        return Err(ApiError::invalid("name at least one field", json!(null)));
    }
    if let Some(unknown) = ids.iter().find(|id| table.field(**id).is_none()) {
        return Err(ApiError::invalid(
            format!("{unknown} is not a field of “{}”", table.name),
            json!(null),
        ));
    }
    let set: BTreeSet<FieldId> = ids.iter().copied().collect();
    Ok(if only {
        FieldAccess::Only { only: set }
    } else {
        FieldAccess::Except { except: set }
    })
}

/// The fields a grant or link covers, in words; `None` for every field.
fn fields_text(table: Option<&Table>, access: &FieldAccess) -> Option<String> {
    let names = |ids: &BTreeSet<FieldId>| {
        ids.iter()
            .map(|id| {
                table
                    .and_then(|t| t.field(*id))
                    .map_or_else(|| id.to_string(), |f| format!("“{}”", f.name))
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    match access {
        FieldAccess::All => None,
        FieldAccess::Only { only } => Some(format!("only {}", names(only))),
        FieldAccess::Except { except } => Some(format!("all but {}", names(except))),
    }
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
    let fields = match (&spec.fields, &table) {
        (Some(fields), Some(table)) => field_access(table, fields)?,
        (Some(_), None) => {
            return Err(ApiError::invalid(
                "a grant limited to some fields must be given on one table",
                json!(null),
            ))
        }
        (None, _) => FieldAccess::All,
    };
    if spec.valid_to.is_some_and(|to| to <= now) {
        return Err(ApiError::invalid(
            "the grant would already have ended",
            json!(null),
        ));
    }
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
    grant.fields = fields;
    grant.valid_from = spec.valid_from;
    grant.valid_to = spec.valid_to;
    let summary = grant_json(ws, &grant, now);
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
    let summary = grant_json(ws, &grant, now);
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

/// A grant as people read it, and whether it is in force at `now`.
#[must_use]
pub fn grant_json(ws: &Workspace, grant: &Grant, now: Timestamp) -> Json {
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
    let table = grant.on.table().and_then(|id| ws.table(id).ok());
    let status = match grant.status(now) {
        Ok(()) => "in_force",
        Err(Inactive::Revoked) => "revoked",
        Err(Inactive::NotYetValid) => "not_yet",
        Err(Inactive::Expired) => "ended",
    };
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
        "fields": fields_text(table, &grant.fields),
        "validFrom": grant.valid_from,
        "validTo": grant.valid_to,
        "status": status,
    })
}

/// The access page: grants, sets, rules, reviews, share links and the capability catalogue.
#[must_use]
pub fn overview(ws: &Workspace, now: Timestamp, manage: bool) -> Json {
    let grants: Vec<Json> = ws
        .data
        .policy
        .grants()
        .map(|g| grant_json(ws, g, now))
        .collect();
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
    let reviews: Vec<Json> = ws
        .data
        .reviews
        .iter()
        .rev()
        .map(|r| review_json(ws, r, now))
        .collect();
    let shares: Vec<Json> = ws
        .data
        .shares
        .iter()
        .rev()
        .map(|s| share_json(ws, s, now, manage))
        .collect();
    json!({"grants": grants, "sets": sets, "rules": rules, "reviews": reviews, "shares": shares, "capabilities": catalogue(ws)})
}

/// The person an actor stands for (the person an agent acts for, or the actor).
const fn person(actor: Actor) -> PrincipalId {
    match actor.on_behalf_of {
        Some(person) => person,
        None => actor.principal,
    }
}

// ------------------------------------------------------------------------------- reviews

/// An access review as stored: the engine's review and what it covers, in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    /// The review.
    pub review: AccessReview,
    /// What it covers ("Expenses", "the whole workspace").
    pub covers: String,
}

/// A review to open.
#[derive(Debug, Clone, Deserialize)]
pub struct ReviewSpec {
    /// Its name ("Q4 review of the expense base").
    pub name: String,
    /// Which grants: those given there (a base includes its tables).
    #[serde(default = "everywhere")]
    pub on: Place,
    /// When it should be completed.
    #[serde(default, rename = "dueAt")]
    pub due_at: Option<Timestamp>,
}

const fn everywhere() -> Place {
    Place::Workspace
}

/// Whether a grant given on `on` falls under `place`.
fn covers(place: &Place, on: &Resource) -> bool {
    match place {
        Place::Workspace => true,
        Place::Base { id } => match on {
            Resource::Base { base, .. } | Resource::Table { base, .. } => base == id,
            Resource::Tenant | Resource::Workspace { .. } => false,
        },
        Place::Table { id } => on.table() == Some(*id),
    }
}

fn review_index(ws: &Workspace, id: AccessReviewId) -> ApiResult<usize> {
    ws.data
        .reviews
        .iter()
        .position(|r| r.review.id == id)
        .ok_or_else(|| ApiError::not_found(format!("access review {id}")))
}

/// Opens a review of the grants in force on a place.
///
/// # Errors
/// 403 without `access.manage`; 404 for an unknown place; 422 without a name, without a
/// grant in force there, or due before now.
pub fn open_review(
    ws: &mut Workspace,
    actor: Actor,
    spec: &ReviewSpec,
    now: Timestamp,
) -> ApiResult<AccessReviewId> {
    require(ws, actor, now)?;
    let covers_text = match &spec.on {
        Place::Workspace => "the whole workspace".to_owned(),
        Place::Base { id } => ws.base(*id)?.name.clone(),
        Place::Table { id } => ws.table(*id)?.name.clone(),
    };
    let id = ws.new_id(now, AccessReviewId::new)?;
    let review = AccessReview::open(
        id,
        spec.name.trim(),
        person(actor),
        now,
        spec.due_at,
        ws.data.policy.grants().filter(|g| covers(&spec.on, &g.on)),
    )
    .map_err(perm_error)?;
    let (_, total) = review.progress();
    ws.data.reviews.push(Review {
        review,
        covers: covers_text.clone(),
    });
    ws.audit_object(
        actor,
        "access.review.open",
        ObjectRef::new("access_review", id.to_string()),
        json!({"name": spec.name.trim(), "covers": covers_text, "grants": total, "dueAt": spec.due_at}),
        now,
    )?;
    Ok(id)
}

/// Records a verdict on one grant of a review.
///
/// # Errors
/// 403; 404 for an unknown review; 422 when the review is closed, the grant is not part of
/// it, the reviewer holds the grant, or a revocation has no note.
pub fn decide_review(
    ws: &mut Workspace,
    actor: Actor,
    id: AccessReviewId,
    (grant, verdict): (GrantId, Verdict),
    note: Option<String>,
    now: Timestamp,
) -> ApiResult<()> {
    require(ws, actor, now)?;
    let index = review_index(ws, id)?;
    let mut stored = ws
        .data
        .reviews
        .get(index)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("access review {id}")))?;
    stored
        .review
        .decide(grant, verdict, person(actor), now, note.clone(), ws)
        .map_err(perm_error)?;
    let name = stored.review.name.clone();
    if let Some(slot) = ws.data.reviews.get_mut(index) {
        *slot = stored;
    }
    ws.audit_object(
        actor,
        "access.review.decide",
        ObjectRef::new("access_review", id.to_string()),
        json!({"review": name, "grant": grant, "verdict": verdict, "note": note}),
        now,
    )
}

/// Completes a review: every grant marked for revocation is revoked, with the reviewer's
/// note in the audit trail. Returns how many were revoked.
///
/// # Errors
/// 403; 404; 422 while grants are undecided or when the review is closed.
pub fn complete_review(
    ws: &mut Workspace,
    actor: Actor,
    id: AccessReviewId,
    now: Timestamp,
) -> ApiResult<usize> {
    require(ws, actor, now)?;
    let index = review_index(ws, id)?;
    let mut stored = ws
        .data
        .reviews
        .get(index)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("access review {id}")))?;
    let to_revoke = stored
        .review
        .complete(person(actor), now)
        .map_err(perm_error)?;
    let name = stored.review.name.clone();
    let mut revoked = 0_usize;
    for grant_id in &to_revoke {
        let Some(grant) = ws.data.policy.grant(*grant_id).cloned() else {
            continue;
        };
        if grant.revoked_at.is_some() {
            continue;
        }
        let note = stored
            .review
            .items()
            .find(|item| item.grant == *grant_id)
            .and_then(|item| item.note.clone());
        let summary = grant_json(ws, &grant, now);
        ws.data
            .policy
            .revoke(*grant_id, person(actor), now)
            .map_err(perm_error)?;
        revoked = revoked.saturating_add(1);
        ws.audit_object(
            actor,
            "access.revoke",
            ObjectRef::new("grant", grant_id.to_string()),
            json!({"grant": summary, "review": name, "note": note}),
            now,
        )?;
    }
    let (_, total) = stored.review.progress();
    if let Some(slot) = ws.data.reviews.get_mut(index) {
        *slot = stored;
    }
    ws.audit_object(
        actor,
        "access.review.complete",
        ObjectRef::new("access_review", id.to_string()),
        json!({"review": name, "grants": total, "revoked": revoked}),
        now,
    )?;
    Ok(revoked)
}

/// Abandons a review; nothing is revoked.
///
/// # Errors
/// 403; 404; 422 when the review is closed already.
pub fn cancel_review(
    ws: &mut Workspace,
    actor: Actor,
    id: AccessReviewId,
    now: Timestamp,
) -> ApiResult<()> {
    require(ws, actor, now)?;
    let index = review_index(ws, id)?;
    let mut stored = ws
        .data
        .reviews
        .get(index)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("access review {id}")))?;
    stored
        .review
        .cancel(person(actor), now)
        .map_err(perm_error)?;
    let name = stored.review.name.clone();
    if let Some(slot) = ws.data.reviews.get_mut(index) {
        *slot = stored;
    }
    ws.audit_object(
        actor,
        "access.review.cancel",
        ObjectRef::new("access_review", id.to_string()),
        json!({"review": name}),
        now,
    )
}

/// A review as people read it.
#[must_use]
pub fn review_json(ws: &Workspace, stored: &Review, now: Timestamp) -> Json {
    let review = &stored.review;
    let (done, total) = review.progress();
    let items: Vec<Json> = review
        .items()
        .map(|item| {
            json!({
                "grant": item.grant,
                "holder": ws.name_of(item.principal),
                "holderId": item.principal,
                "what": ws.data.policy.grant(item.grant).map(|g| grant_json(ws, g, now)),
                "verdict": item.verdict,
                "note": item.note,
                "reviewer": item.reviewer.map(|p| ws.name_of(p)),
                "decidedAt": item.decided_at,
            })
        })
        .collect();
    json!({
        "id": review.id,
        "name": review.name,
        "covers": stored.covers,
        "state": match review.state {
            ReviewState::Open => "open",
            ReviewState::Completed => "completed",
            ReviewState::Cancelled => "cancelled",
        },
        "openedBy": ws.name_of(review.opened_by),
        "openedAt": review.opened_at,
        "dueAt": review.due_at,
        "overdue": review.is_overdue(now),
        "closedAt": review.closed_at,
        "closedBy": review.closed_by.map(|p| ws.name_of(p)),
        "done": done,
        "total": total,
        "items": items,
    })
}

// ------------------------------------------------------------------------- share links

/// The shortest token a link may have (the API makes 48 hexadecimal characters).
pub const MIN_TOKEN_CHARS: usize = 32;

/// A share link as stored: the engine's link and the secret token in its address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Share {
    /// The link.
    pub link: ShareLink,
    /// The secret part of its address; whoever has it may open the link.
    pub token: String,
}

/// A link to make: a table (every record its creator may read) or one record, read only.
#[derive(Debug, Clone, Deserialize)]
pub struct ShareSpec {
    /// One record instead of the table.
    #[serde(default)]
    pub record: Option<RecordId>,
    /// Only some fields, or all but some.
    #[serde(default)]
    pub fields: Option<FieldsSpec>,
    /// When it stops working.
    #[serde(default, rename = "expiresAt")]
    pub expires_at: Option<Timestamp>,
}

/// Makes a read-only share link. `token` is the secret the caller generated.
///
/// # Errors
/// 403 without `share.manage` on the table; 404 for an unknown table, or a record the actor
/// cannot read; 422 for unknown fields, no field at all, or an expiry in the past.
pub fn create_share(
    ws: &mut Workspace,
    actor: Actor,
    table_id: TableId,
    spec: &ShareSpec,
    token: String,
    now: Timestamp,
) -> ApiResult<ShareLinkId> {
    let table = ws.table(table_id)?.clone();
    ws.require(
        actor,
        &Action::ShareManage.into(),
        table.resource(),
        None,
        now,
    )?;
    if token.len() < MIN_TOKEN_CHARS {
        return Err(ApiError::internal("the link's token is too short"));
    }
    if let Some(record) = spec.record {
        if !ws
            .readable(actor, &table, now)
            .iter()
            .any(|r| r.id == record)
        {
            return Err(ApiError::not_found(format!("record {record}")));
        }
    }
    if spec.expires_at.is_some_and(|at| at <= now) {
        return Err(ApiError::invalid(
            "the link would already have expired",
            json!(null),
        ));
    }
    let fields = match &spec.fields {
        Some(fields) => field_access(&table, fields)?,
        None => FieldAccess::All,
    };
    let id = ws.new_id(now, ShareLinkId::new)?;
    let link = ShareLink {
        id,
        on: table.resource(),
        record: spec.record,
        view: None,
        mode: ShareMode::View,
        fields,
        created_by: person(actor),
        created_at: now,
        expires_at: spec.expires_at,
        revoked_at: None,
    };
    link.check().map_err(perm_error)?;
    let share = Share { link, token };
    // The audit trail never holds the token.
    let summary = share_json(ws, &share, now, false);
    ws.data.shares.push(share);
    ws.audit_object(
        actor,
        "share.create",
        ObjectRef::new("share_link", id.to_string()),
        summary,
        now,
    )?;
    Ok(id)
}

/// Withdraws a share link; it stops working at once and stays on record.
///
/// # Errors
/// 403 without `share.manage` on its table; 404; 409 when withdrawn already.
pub fn revoke_share(
    ws: &mut Workspace,
    actor: Actor,
    id: ShareLinkId,
    now: Timestamp,
) -> ApiResult<()> {
    let index = ws
        .data
        .shares
        .iter()
        .position(|s| s.link.id == id)
        .ok_or_else(|| ApiError::not_found(format!("share link {id}")))?;
    let on = ws
        .data
        .shares
        .get(index)
        .map(|s| s.link.on)
        .ok_or_else(|| ApiError::not_found(format!("share link {id}")))?;
    ws.require(actor, &Action::ShareManage.into(), on, None, now)?;
    let Some(share) = ws.data.shares.get_mut(index) else {
        return Err(ApiError::not_found(format!("share link {id}")));
    };
    if share.link.revoked_at.is_some() {
        return Err(ApiError::conflict("the link is withdrawn already"));
    }
    share.link.revoked_at = Some(now);
    let share = share.clone();
    let summary = share_json(ws, &share, now, false);
    ws.audit_object(
        actor,
        "share.revoke",
        ObjectRef::new("share_link", id.to_string()),
        summary,
        now,
    )
}

/// A share link as people read it; the token only for those who may hand it out.
#[must_use]
pub fn share_json(ws: &Workspace, share: &Share, now: Timestamp, with_token: bool) -> Json {
    let link = &share.link;
    let table = link.on.table().and_then(|id| ws.table(id).ok());
    let status = match link.status(now) {
        Ok(()) => "active",
        Err(Inactive::Revoked) => "withdrawn",
        Err(Inactive::Expired) => "expired",
        Err(Inactive::NotYetValid) => "not_yet",
    };
    json!({
        "id": link.id,
        "table": table.map(|t| t.name.clone()),
        "tableId": link.on.table(),
        "record": link.record.and_then(|r| table.map(|t| ws.title(t.id, r))),
        "fields": fields_text(table, &link.fields),
        "expiresAt": link.expires_at,
        "createdBy": ws.name_of(link.created_by),
        "createdAt": link.created_at,
        "status": status,
        "token": with_token.then(|| share.token.clone()),
    })
}

/// Compares two tokens in time that does not depend on where they differ.
fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0_u8, |diff, (x, y)| diff | (x ^ y))
            == 0
}

const fn lower(a: Visibility, b: Visibility) -> Visibility {
    match (a, b) {
        (Visibility::Hidden, _) | (_, Visibility::Hidden) => Visibility::Hidden,
        (Visibility::Masked, _) | (_, Visibility::Masked) => Visibility::Masked,
        (Visibility::Visible, Visibility::Visible) => Visibility::Visible,
    }
}

/// What the holder of a link sees, without an account. A link never shows more than the
/// person who made it may still see: their row filter and field permissions apply on top
/// of the link's own narrowing (no classified or protected fields, masks kept). Links to
/// other tables are left out: a link opens one table.
///
/// # Errors
/// 404 for an unknown token; 410 when the link expired or was withdrawn, its maker may no
/// longer share or read it, or its record is gone.
pub fn open_share(ws: &Workspace, token: &str, now: Timestamp) -> ApiResult<Json> {
    let share = ws
        .data
        .shares
        .iter()
        .find(|s| same_token(&s.token, token))
        .ok_or_else(|| ApiError::not_found("this link"))?;
    let link = &share.link;
    link.status(now).map_err(|why| {
        ApiError::gone(match why {
            Inactive::Revoked => "this link was withdrawn",
            Inactive::Expired => "this link has expired",
            Inactive::NotYetValid => "this link does not work yet",
        })
    })?;
    let table = link
        .on
        .table()
        .and_then(|id| ws.table(id).ok())
        .ok_or_else(|| ApiError::gone("the table of this link no longer exists"))?;
    let maker = Actor {
        principal: link.created_by,
        on_behalf_of: None,
    };
    if ws
        .require(
            maker,
            &Action::ShareManage.into(),
            table.resource(),
            None,
            now,
        )
        .is_err()
    {
        return Err(ApiError::gone(
            "the person who shared this link may no longer share it",
        ));
    }
    let records: Vec<_> = ws
        .readable(maker, table, now)
        .into_iter()
        .filter(|r| link.record.is_none_or(|id| r.id == id))
        .collect();
    if link.record.is_some() && records.is_empty() {
        return Err(ApiError::gone(
            "the record of this link is no longer available",
        ));
    }
    let metas = ws.field_meta(table);
    let link_permission = |field: FieldId| {
        metas
            .iter()
            .find(|m| m.id == field)
            .map_or(Visibility::Hidden, |meta| {
                link.field_permission(meta).visibility
            })
    };
    let for_table = ws.field_permissions(maker, table, None, now);
    let shown: Vec<_> = table
        .fields
        .iter()
        .filter(|field| !matches!(field.options, OptionsSource::Table { .. }))
        .filter(|field| {
            let maker_sees = for_table
                .get(&field.id)
                .map_or(Visibility::Hidden, |p| p.visibility);
            match lower(link_permission(field.id), maker_sees) {
                Visibility::Hidden => false,
                Visibility::Visible | Visibility::Masked => true,
            }
        })
        .collect();
    let columns: Vec<Json> = shown
        .iter()
        .map(|field| {
            json!({
                "id": field.id,
                "name": field.name,
                "typeLabel": BuiltinType::of(field).label(),
                "numeric": matches!(field.data_type, DataType::Number | DataType::Money | DataType::Percent | DataType::Duration),
                "primary": table.primary == field.id,
            })
        })
        .collect();
    let rows: Vec<Json> = records
        .iter()
        .map(|record| {
            let permissions = ws.field_permissions(maker, table, Some(record), now);
            let cells: serde_json::Map<String, Json> = shown
                .iter()
                .map(|field| {
                    let maker_sees = permissions
                        .get(&field.id)
                        .map_or(Visibility::Hidden, |p| p.visibility);
                    let permission = mt_perm::FieldPermission {
                        visibility: lower(link_permission(field.id), maker_sees),
                        editable: false,
                    };
                    (
                        field.id.to_string(),
                        crate::view::cell(ws, field, record, permission),
                    )
                })
                .collect();
            json!({"id": record.id, "title": ws.title(table.id, record.id), "cells": cells})
        })
        .collect();
    Ok(json!({
        "table": table.name,
        "record": link.record.map(|r| ws.title(table.id, r)),
        "sharedBy": ws.name_of(link.created_by),
        "expiresAt": link.expires_at,
        "columns": columns,
        "records": rows,
    }))
}

//! Code lists with a history: every change is a numbered version, and a list can require a
//! second person's approval for its changes (four eyes).
//!
//! The rules are `mt-refdata`'s [`Ledger`]: a change applies at once or is proposed,
//! depending on the list's policy; nobody approves their own proposal; a proposal applies
//! to the list as it is when approved, or not at all; switching four eyes off is itself a
//! change that needs approval. Any past version can be shown. The workspace keeps each
//! list's current state (what records and formulas use) and its history next to it; on
//! load the history must replay to exactly that state.

use crate::error::{ApiError, ApiResult};
use crate::model::Workspace;
use mt_audit::ObjectRef;
use mt_core::id::{PrincipalId, RefItemId, RefSetId};
use mt_core::time::Timestamp;
use mt_perm::Actor;
use mt_refdata::{
    Change, ChangePolicy, EffectiveSet, Ledger, Proposal, ProposalStatus, RefDataError, Revision,
    State,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};

/// The history of one code list as stored: where it started, every accepted version, and
/// every proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    /// The list.
    pub set: RefSetId,
    /// The list at version 0.
    pub genesis: State,
    /// Versions 1, 2, 3, …
    pub revisions: Vec<Revision>,
    /// Proposals, numbered from 1.
    pub proposals: Vec<Proposal>,
}

/// What became of a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// It applied, giving this version.
    Applied {
        /// The new version.
        version: u64,
    },
    /// It waits for a second person.
    Proposed {
        /// The proposal number.
        proposal: u64,
    },
}

impl Outcome {
    /// As the API returns it.
    #[must_use]
    pub fn json(self) -> Json {
        match self {
            Outcome::Applied { version } => json!({"applied": true, "version": version}),
            Outcome::Proposed { proposal } => json!({"applied": false, "proposal": proposal}),
        }
    }
}

fn person(actor: Actor) -> PrincipalId {
    actor.on_behalf_of.unwrap_or(actor.principal)
}

/// The engine's refusal as an API error.
fn refdata_error(error: RefDataError) -> ApiError {
    let text = error.to_string();
    match error {
        RefDataError::SelfApproval => ApiError::forbidden(
            "you proposed this change: someone else must approve or reject it",
            json!(null),
        ),
        RefDataError::NotProposer => ApiError::forbidden(text, json!(null)),
        RefDataError::NotPending { .. }
        | RefDataError::VersionConflict { .. }
        | RefDataError::ApprovalRequired => ApiError::conflict(text),
        RefDataError::UnknownVersion { .. } => ApiError::not_found(text),
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
        | RefDataError::EmptyChange
        | RefDataError::Limit(_)
        | RefDataError::UnknownParameter { .. }
        | RefDataError::InvalidParameter { .. }
        | RefDataError::ScopeNotAllowed { .. }
        | RefDataError::Core(_) => ApiError::invalid(text, json!(null)),
    }
}

/// A list's ledger, rebuilt from its history (or started now, for a list without one), and
/// the list's position in the workspace.
///
/// # Errors
/// 404 for an unknown list; 500 when the stored history does not replay to the list.
pub fn ledger(ws: &Workspace, set: RefSetId) -> ApiResult<(usize, Ledger)> {
    let index = ws
        .data
        .refsets
        .iter()
        .position(|s| s.set.id == set)
        .ok_or_else(|| ApiError::not_found(format!("code list {set}")))?;
    let current = ws
        .data
        .refsets
        .get(index)
        .ok_or_else(|| ApiError::not_found(format!("code list {set}")))?;
    let ledger = match ws.data.refset_history.iter().find(|h| h.set == set) {
        Some(history) => Ledger::restore(
            history.genesis.clone(),
            history.revisions.clone(),
            history.proposals.clone(),
        )
        .map_err(|e| ApiError::internal(format!("the history of `{}`: {e}", current.set.code)))?,
        None => Ledger::new(current.clone()).map_err(refdata_error)?,
    };
    if ledger.state() != current {
        return Err(ApiError::internal(format!(
            "the history of `{}` does not lead to the list as stored",
            current.set.code
        )));
    }
    Ok((index, ledger))
}

/// Checks every stored history against its list (on load).
///
/// # Errors
/// 500 for the first history that does not replay to its list, or names no list.
pub fn verify(ws: &Workspace) -> ApiResult<()> {
    for history in &ws.data.refset_history {
        ledger(ws, history.set)?;
    }
    Ok(())
}

fn store(ws: &mut Workspace, index: usize, ledger: &Ledger) -> ApiResult<()> {
    let set = ledger.state().set.id;
    if let Some(slot) = ws.data.refsets.get_mut(index) {
        *slot = ledger.state().clone();
    }
    let history = History {
        set,
        genesis: ledger.genesis().clone(),
        revisions: ledger.revisions().to_vec(),
        proposals: ledger.proposals().to_vec(),
    };
    match ws.data.refset_history.iter_mut().find(|h| h.set == set) {
        Some(slot) => *slot = history,
        None => ws.data.refset_history.push(history),
    }
    ws.rebuild_refdata()
}

/// Makes a change to a list: at once, or as a proposal when the list needs approval.
///
/// # Errors
/// 403 without `refdata.manage`; 404; 422 when the change does not apply.
pub fn change(
    ws: &mut Workspace,
    actor: Actor,
    set: RefSetId,
    change: Change,
    what: &str,
    now: Timestamp,
) -> ApiResult<Outcome> {
    crate::builder::refset_require(ws, actor, now)?;
    let (index, mut ledger) = ledger(ws, set)?;
    let summary = describe(ledger.state(), &change);
    let code = ledger.state().set.code.clone();
    let outcome = match ledger.state().set.change_policy {
        ChangePolicy::Direct => Outcome::Applied {
            version: ledger
                .commit(person(actor), now, ledger.version(), vec![change.clone()])
                .map_err(refdata_error)?,
        },
        ChangePolicy::Approval => Outcome::Proposed {
            proposal: ledger
                .propose(person(actor), now, vec![change.clone()], None)
                .map_err(refdata_error)?,
        },
    };
    store(ws, index, &ledger)?;
    match outcome {
        Outcome::Applied { version } => ws.audit_object(
            actor,
            what,
            ObjectRef::new("refset", set.to_string()),
            json!({"set": code, "change": change, "summary": summary, "version": version}),
            now,
        )?,
        Outcome::Proposed { proposal } => ws.audit_object(
            actor,
            "refset.change.propose",
            ObjectRef::new("refset", set.to_string()),
            json!({"set": code, "change": change, "summary": summary, "proposal": proposal}),
            now,
        )?,
    }
    Ok(outcome)
}

/// Requires (or stops requiring) a second person's approval for a list's changes. Switching
/// it off is itself proposed when the list needs approval.
///
/// # Errors
/// As [`change`]; 422 when the list already has that policy.
pub fn set_policy(
    ws: &mut Workspace,
    actor: Actor,
    set: RefSetId,
    approval: bool,
    now: Timestamp,
) -> ApiResult<Outcome> {
    let policy = if approval {
        ChangePolicy::Approval
    } else {
        ChangePolicy::Direct
    };
    change(
        ws,
        actor,
        set,
        Change::SetPolicy { policy },
        "refset.policy",
        now,
    )
}

/// A decision on a proposal.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    /// Apply it (someone other than its author).
    Approve,
    /// Refuse it, with a reason (someone other than its author).
    Reject {
        /// Why.
        reason: String,
    },
    /// Take it back (its author).
    Withdraw,
}

/// Decides on a proposal. Returns the version an approval produced.
///
/// # Errors
/// 403 without `refdata.manage`, for approving or rejecting one's own proposal, or for
/// withdrawing someone else's; 404; 409 when it is decided already or no longer applies;
/// 422 for a rejection without a reason.
pub fn decide(
    ws: &mut Workspace,
    actor: Actor,
    set: RefSetId,
    proposal: u64,
    decision: &Decision,
    now: Timestamp,
) -> ApiResult<Option<u64>> {
    crate::builder::refset_require(ws, actor, now)?;
    let (index, mut ledger) = ledger(ws, set)?;
    if !ledger.proposals().iter().any(|p| p.id == proposal) {
        return Err(ApiError::not_found(format!("proposal {proposal}")));
    }
    let who = person(actor);
    let (version, action) = match decision {
        Decision::Approve => (
            Some(ledger.approve(proposal, who, now).map_err(refdata_error)?),
            "refset.change.approve",
        ),
        Decision::Reject { reason } => {
            ledger
                .reject(proposal, who, now, reason.trim())
                .map_err(refdata_error)?;
            (None, "refset.change.reject")
        }
        Decision::Withdraw => {
            ledger.withdraw(proposal, who, now).map_err(refdata_error)?;
            (None, "refset.change.withdraw")
        }
    };
    let code = ledger.state().set.code.clone();
    store(ws, index, &ledger)?;
    let reason = match decision {
        Decision::Reject { reason } => Some(reason.trim().to_owned()),
        Decision::Approve | Decision::Withdraw => None,
    };
    ws.audit_object(
        actor,
        action,
        ObjectRef::new("refset", set.to_string()),
        json!({"set": code, "proposal": proposal, "version": version, "reason": reason}),
        now,
    )?;
    Ok(version)
}

fn item_name(state: &State, id: RefItemId) -> String {
    state
        .items
        .iter()
        .chain(&state.overlay.added)
        .find(|item| item.id == id)
        .map_or_else(|| id.to_string(), |item| format!("“{}”", item.name))
}

/// A change in words, naming items as they were in `state`.
#[must_use]
pub fn describe(state: &State, change: &Change) -> String {
    match change {
        Change::Add { item } => match item.parent {
            Some(parent) => format!(
                "added “{}” ({}) under {}",
                item.name,
                item.code,
                item_name(state, parent)
            ),
            None => format!("added “{}” ({})", item.name, item.code),
        },
        Change::Update { id, patch } => match &patch.name {
            Some(name) => format!("renamed {} to “{name}”", item_name(state, *id)),
            None => format!("changed {}", item_name(state, *id)),
        },
        Change::Move { id, parent } => match parent {
            Some(parent) => format!(
                "moved {} under {}",
                item_name(state, *id),
                item_name(state, *parent)
            ),
            None => format!("moved {} to the top", item_name(state, *id)),
        },
        Change::SetEnabled { id, enabled } => format!(
            "{} {}",
            if *enabled { "enabled" } else { "disabled" },
            item_name(state, *id)
        ),
        Change::SetAliases { id, system, .. } => {
            format!("changed the {system} codes of {}", item_name(state, *id))
        }
        Change::Remove { id } => format!("removed {}", item_name(state, *id)),
        Change::SetPolicy { policy } => match policy {
            ChangePolicy::Approval => "changes now need a second person's approval".to_owned(),
            ChangePolicy::Direct => "changes apply at once again".to_owned(),
        },
    }
}

/// A list's items as the code-list page shows them.
#[must_use]
pub fn items_json(set: &EffectiveSet, query: &str, now: Timestamp) -> Vec<Json> {
    let ids = if query.trim().is_empty() {
        set.walk()
    } else {
        set.search(query, "en", 200)
    };
    let today = now.date();
    ids.iter()
        .filter_map(|item| set.get(*item))
        .map(|item| {
            json!({
                "id": item.id,
                "code": item.code,
                "name": item.name,
                "depth": set.depth(item.id),
                "path": set.path_label(item.id, "en", " › "),
                "active": set.is_active(item.id, today),
                "leaf": set.is_leaf(item.id),
                "enabled": item.enabled,
            })
        })
        .collect()
}

/// The versions and proposals of a list, newest first.
///
/// # Errors
/// As [`ledger`].
pub fn history_json(ws: &Workspace, set: RefSetId) -> ApiResult<Json> {
    let (_, ledger) = ledger(ws, set)?;
    let mut state = ledger.genesis().clone();
    let mut versions = Vec::new();
    for revision in ledger.revisions() {
        let changes: Vec<String> = revision
            .changes
            .iter()
            .map(|c| describe(&state, c))
            .collect();
        versions.push(json!({
            "version": revision.version,
            "at": revision.at,
            "by": ws.name_of(revision.by),
            "approvedBy": revision.approved_by.map(|p| ws.name_of(p)),
            "changes": changes,
        }));
        state = state
            .apply(&revision.changes)
            .map_err(|e| ApiError::internal(e.to_string()))?
            .0;
    }
    versions.reverse();
    let proposals: Vec<Json> = ledger
        .proposals()
        .iter()
        .rev()
        .map(|p| {
            let (status, decided_by, reason, version) = match &p.status {
                ProposalStatus::Pending => ("pending", None, None, None),
                ProposalStatus::Approved { by, version, .. } => {
                    ("approved", Some(ws.name_of(*by)), None, Some(*version))
                }
                ProposalStatus::Rejected { by, reason, .. } => (
                    "rejected",
                    Some(ws.name_of(*by)),
                    Some(reason.clone()),
                    None,
                ),
                ProposalStatus::Withdrawn { .. } => ("withdrawn", None, None, None),
            };
            json!({
                "id": p.id,
                "by": ws.name_of(p.by),
                "byId": p.by,
                "at": p.at,
                "changes": p.changes.iter().map(|c| describe(ledger.state(), c)).collect::<Vec<_>>(),
                "note": p.note,
                "status": status,
                "decidedBy": decided_by,
                "reason": reason,
                "version": version,
            })
        })
        .collect();
    Ok(json!({
        "version": ledger.version(),
        "approval": ledger.state().set.change_policy == ChangePolicy::Approval,
        "versions": versions,
        "proposals": proposals,
    }))
}

/// A list's items as they were at a version.
///
/// # Errors
/// As [`ledger`]; 404 for a version that does not exist yet.
pub fn items_as_of(
    ws: &Workspace,
    set: RefSetId,
    version: u64,
    query: &str,
    now: Timestamp,
) -> ApiResult<Vec<Json>> {
    let (_, ledger) = ledger(ws, set)?;
    let state = ledger.as_of(version).map_err(refdata_error)?;
    let effective = state.effective().map_err(refdata_error)?;
    Ok(items_json(&effective, query, now))
}

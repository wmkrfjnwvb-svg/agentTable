//! Process: record lifecycles and approvals (`mt-lifecycle`) on top of the workspace.
//!
//! A table can be bound to a [`Lifecycle`]: one single-choice field holds the state, and only
//! transitions change it. A transition may start an approval; the approval's outcome takes
//! the record on (approved, rejected, sent back). Who is asked comes from the access policy:
//! an approver "whoever may `request.approve` this record" resolves to the people whose grants
//! (limits, separation of duties) allow it for this record at this moment. The requester is
//! never asked.
//!
//! While a record is in a state that freezes fields, they cannot change, and neither can the
//! records composed into it (the lines of a request under approval).

use crate::error::{ApiError, ApiResult};
use crate::model::{context, set_cell, Record, Table, Workspace};
use mt_core::condition::{Condition, Operand, Operator};
use mt_core::id::{FieldId, PrincipalId, RecordId, RefItemId, TableId};
use mt_core::state::StateCategory;
use mt_core::time::Timestamp;
use mt_core::value::Value;
use mt_lifecycle::approval::{
    Act, Approval, ApprovalDef, Approver, Env, Event, Join, Mode, Node, Output, People, Status,
    TaskKind, TaskStatus,
};
use mt_lifecycle::lifecycle::{Blocker, Lifecycle, Move, Transition};
use mt_lifecycle::LifecycleError;
use mt_perm::{Action, Actor, Capability, PrincipalKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};

/// The capability of transitions only the approval engine takes.
pub const SYSTEM: &str = "process.system";

/// A state and the option of the state field that shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateOption {
    /// The state key.
    pub state: String,
    /// The option.
    pub option: RefItemId,
}

/// Records of another table that are part of a bound record (lines of a request).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Child {
    /// The child table.
    pub table: TableId,
    /// The child's link to its parent.
    pub link: FieldId,
    /// The parent's field that lists the children (its editability decides).
    pub parent_field: FieldId,
}

/// A lifecycle bound to a table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    /// The lifecycle.
    pub lifecycle: Lifecycle,
    /// The field that holds the state.
    pub field: FieldId,
    /// State keys and their options.
    pub options: Vec<StateOption>,
    /// Composed records.
    #[serde(default)]
    pub children: Vec<Child>,
    /// The transition taken when the approval approves.
    pub on_approved: String,
    /// The transition taken when the approval rejects.
    pub on_rejected: String,
    /// The transition taken when an approver sends the request back.
    pub on_returned: String,
    /// The design it was published from (the designer edits it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub design: Option<crate::design::ProcessDesign>,
}

/// One approval of one record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    /// Number, from 1.
    pub id: u64,
    /// The table.
    pub table: TableId,
    /// The record.
    pub record: RecordId,
    /// The approval.
    pub approval: Approval,
}

/// A message in someone's inbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    /// Number, from 1.
    pub id: u64,
    /// To whom.
    pub to: PrincipalId,
    /// When.
    pub at: Timestamp,
    /// What.
    pub text: String,
    /// About which table.
    pub table: TableId,
    /// About which record.
    pub record: RecordId,
}

/// Lifecycles, approval definitions, running and finished approvals, and the inbox.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Process {
    /// Lifecycles bound to tables.
    pub bindings: Vec<Binding>,
    /// Approval definitions (every published version).
    pub definitions: Vec<ApprovalDef>,
    /// Approvals.
    pub runs: Vec<Run>,
    /// Notices.
    pub inbox: Vec<Notice>,
}

fn internal(error: impl core::fmt::Display) -> ApiError {
    ApiError::internal(error.to_string())
}

fn lifecycle_error(error: LifecycleError) -> ApiError {
    match error {
        LifecycleError::Invalid(message) => ApiError::invalid(message, Json::Null),
        LifecycleError::NotAllowed(message) => ApiError::forbidden(message, Json::Null),
        LifecycleError::State(_) | LifecycleError::Time(_) | LifecycleError::Version { .. } => {
            ApiError::conflict(error.to_string())
        }
    }
}

// ------------------------------------------------------------------- states

/// The lifecycle bound to a table.
#[must_use]
pub fn binding(ws: &Workspace, table: TableId) -> Option<&Binding> {
    ws.data
        .process
        .bindings
        .iter()
        .find(|b| b.lifecycle.table == table)
}

/// The state of a record (the initial state when the field is empty).
#[must_use]
pub fn state_of(binding: &Binding, record: &Record) -> String {
    match record.cells.get(&binding.field) {
        Some(Value::Reference(option)) => binding
            .options
            .iter()
            .find(|o| o.option == *option)
            .map_or_else(|| binding.lifecycle.initial.clone(), |o| o.state.clone()),
        Some(_) | None => binding.lifecycle.initial.clone(),
    }
}

fn option_of(binding: &Binding, state: &str) -> Option<RefItemId> {
    binding
        .options
        .iter()
        .find(|o| o.state == state)
        .map(|o| o.option)
}

fn state_name(binding: &Binding, state: &str) -> String {
    binding
        .lifecycle
        .state(state)
        .map_or_else(|| state.to_owned(), |s| s.name.clone())
}

fn field_name(table: &Table, field: FieldId) -> String {
    table
        .field(field)
        .map_or_else(|| field.to_string(), |f| f.name.clone())
}

/// Puts a new record in the initial state.
pub fn initialise(ws: &Workspace, table: TableId, record: &mut Record) {
    if let Some(binding) = binding(ws, table) {
        if let Some(option) = option_of(binding, &binding.lifecycle.initial) {
            set_cell(record, binding.field, Value::Reference(option));
        }
    }
}

/// The parents a child record belongs to, by its link value.
fn parents(value: Option<&Value>) -> Vec<RecordId> {
    match value {
        Some(Value::Link(ids)) => ids.clone(),
        Some(_) | None => Vec::new(),
    }
}

/// Refuses when one of the parents is in a state that freezes its children.
fn parents_editable(ws: &Workspace, child: &Child, parent_ids: &[RecordId]) -> ApiResult<()> {
    let Some(binding) = ws
        .data
        .process
        .bindings
        .iter()
        .find(|b| b.children.contains(child))
    else {
        return Ok(());
    };
    let parent_table = ws.table(binding.lifecycle.table)?;
    for id in parent_ids {
        let Some(parent) = parent_table.record(*id) else {
            continue;
        };
        let state = state_of(binding, parent);
        if !binding.lifecycle.may_edit(&state, child.parent_field) {
            return Err(ApiError::conflict(format!(
                "“{}” is {}: its {} cannot change",
                ws.title(parent_table.id, *id),
                state_name(binding, &state),
                field_name(parent_table, child.parent_field).to_lowercase()
            )));
        }
    }
    Ok(())
}

fn children_of(ws: &Workspace, table: TableId) -> Vec<Child> {
    ws.data
        .process
        .bindings
        .iter()
        .flat_map(|b| b.children.iter().filter(|c| c.table == table).cloned())
        .collect()
}

/// Checks a new record: it starts in the initial state; a child may not join a frozen parent.
///
/// # Errors
/// 409 when the record would start elsewhere or join a frozen parent.
pub fn guard_create(ws: &Workspace, table: &Table, values: &[(FieldId, Value)]) -> ApiResult<()> {
    if let Some(binding) = binding(ws, table.id) {
        let initial = option_of(binding, &binding.lifecycle.initial);
        for (field, value) in values {
            if *field == binding.field
                && !value.is_empty()
                && Some(value) != initial.map(Value::Reference).as_ref()
            {
                return Err(ApiError::conflict(format!(
                    "new records start as {}; “{}” changes through its buttons",
                    state_name(binding, &binding.lifecycle.initial),
                    field_name(table, binding.field)
                )));
            }
        }
    }
    for child in children_of(ws, table.id) {
        if let Some((_, value)) = values.iter().find(|(f, _)| *f == child.link) {
            parents_editable(ws, &child, &parents(Some(value)))?;
        }
    }
    Ok(())
}

/// Checks a change: the state changes only through transitions, frozen fields stay, and
/// frozen parents keep their children.
///
/// # Errors
/// 409 naming the field and the state.
pub fn guard_update(
    ws: &Workspace,
    table: &Table,
    record: &Record,
    changes: &[(FieldId, Value)],
) -> ApiResult<()> {
    if let Some(binding) = binding(ws, table.id) {
        let state = state_of(binding, record);
        for (field, value) in changes {
            if *field == binding.field {
                return Err(ApiError::conflict(format!(
                    "“{}” changes through its buttons, not by editing",
                    field_name(table, binding.field)
                )));
            }
            if !binding.lifecycle.may_edit(&state, *field) {
                return Err(ApiError::conflict(format!(
                    "“{}” cannot change while “{}” is {}",
                    field_name(table, *field),
                    ws.title(table.id, record.id),
                    state_name(binding, &state)
                )));
            }
            // Children taken from another parent leave it: that parent must allow it.
            for child in binding.children.iter().filter(|c| c.parent_field == *field) {
                let child_table = ws.table(child.table)?;
                let taken: Vec<RecordId> = parents(Some(value));
                for id in taken {
                    let others: Vec<RecordId> = child_table
                        .record(id)
                        .map(|r| parents(r.cells.get(&child.link)))
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|p| *p != record.id)
                        .collect();
                    parents_editable(ws, child, &others)?;
                }
            }
        }
    }
    for child in children_of(ws, table.id) {
        let mut affected = parents(record.cells.get(&child.link));
        if let Some((_, value)) = changes.iter().find(|(f, _)| *f == child.link) {
            affected.extend(parents(Some(value)));
        }
        if !changes.is_empty() {
            parents_editable(ws, &child, &affected)?;
        }
    }
    Ok(())
}

/// Checks a deletion: only records in their initial or a cancelled state go, and children of
/// frozen parents stay.
///
/// # Errors
/// 409.
pub fn guard_delete(ws: &Workspace, table: &Table, record: &Record) -> ApiResult<()> {
    if let Some(binding) = binding(ws, table.id) {
        let state = state_of(binding, record);
        let category = binding.lifecycle.state(&state).map(|s| s.category);
        let deletable =
            state == binding.lifecycle.initial || category == Some(StateCategory::Cancelled);
        if !deletable {
            return Err(ApiError::conflict(format!(
                "“{}” is {}: only drafts and cancelled records can be deleted",
                ws.title(table.id, record.id),
                state_name(binding, &state)
            )));
        }
    }
    for child in children_of(ws, table.id) {
        parents_editable(ws, &child, &parents(record.cells.get(&child.link)))?;
    }
    Ok(())
}

// ------------------------------------------------------------------- people

/// Capability holders for one record, asked of the access policy.
struct Holders<'a> {
    ws: &'a Workspace,
    table: &'a Table,
    record: &'a Record,
    now: Timestamp,
}

impl People for Holders<'_> {
    fn holders(&self, capability: &str) -> Vec<PrincipalId> {
        let Ok(capability) = capability.parse::<Capability>() else {
            return Vec::new();
        };
        self.ws
            .data
            .people
            .iter()
            .filter(|p| p.kind == PrincipalKind::Member)
            .filter(|p| {
                let actor = Actor::direct(p.id);
                let prior = self.ws.prior(actor, Some(self.record));
                self.ws
                    .decide(
                        actor,
                        &capability,
                        self.table.resource(),
                        Some(self.record),
                        &prior,
                        self.now,
                    )
                    .allowed
            })
            .map(|p| p.id)
            .collect()
    }
}

fn with_env<R>(
    ws: &Workspace,
    table: &Table,
    record: &Record,
    now: Timestamp,
    f: impl FnOnce(Env<'_>) -> R,
) -> R {
    let holders = Holders {
        ws,
        table,
        record,
        now,
    };
    let ctx = context(now);
    let lookup = |field: FieldId| record.cells.get(&field);
    f(Env {
        people: &holders,
        value_of: &lookup,
        context: &ctx,
        now,
    })
}

// ------------------------------------------------------------------- transitions

fn record_of(ws: &Workspace, table: TableId, record: RecordId) -> ApiResult<(&Table, &Record)> {
    let table = ws.table(table)?;
    let record = table
        .record(record)
        .ok_or_else(|| ApiError::not_found(format!("record {record}")))?;
    Ok((table, record))
}

/// A condition in words, with the table's field names: `“Total” > 0.00 USD`.
#[must_use]
pub fn condition_text(ws: &Workspace, table: &Table, condition: &Condition) -> String {
    match condition {
        Condition::All(parts) if parts.is_empty() => "always".to_owned(),
        Condition::Any(parts) if parts.is_empty() => "never".to_owned(),
        Condition::All(parts) => parts
            .iter()
            .map(|p| condition_text(ws, table, p))
            .collect::<Vec<_>>()
            .join(" and "),
        Condition::Any(parts) => format!(
            "({})",
            parts
                .iter()
                .map(|p| condition_text(ws, table, p))
                .collect::<Vec<_>>()
                .join(" or ")
        ),
        Condition::Not(inner) => format!("not ({})", condition_text(ws, table, inner)),
        Condition::Test(test) => {
            let field = table.field(test.field);
            let name = field.map_or_else(|| test.field.to_string(), |f| f.name.clone());
            let show = |value: &Value| {
                field.map_or_else(
                    || format!("{value:?}"),
                    |f| crate::view::display(ws, f, value),
                )
            };
            let operand = match &test.operand {
                Operand::None => String::new(),
                Operand::Value(value) => show(value),
                Operand::List(values) => values.iter().map(show).collect::<Vec<_>>().join(", "),
                Operand::Range(low, high) => format!("{} and {}", show(low), show(high)),
                Operand::Field(other) => format!("“{}”", field_name(table, *other)),
                Operand::Me => "me".to_owned(),
                Operand::Period(period) => format!("{period:?}").to_lowercase(),
            };
            let op = match test.op {
                Operator::IsEmpty => "is empty",
                Operator::IsNotEmpty => "is filled in",
                Operator::Eq => "is",
                Operator::Ne => "is not",
                Operator::Lt => "<",
                Operator::Le => "≤",
                Operator::Gt => ">",
                Operator::Ge => "≥",
                Operator::Between => "is between",
                Operator::Contains => "contains",
                Operator::NotContains => "does not contain",
                Operator::StartsWith => "starts with",
                Operator::EndsWith => "ends with",
                Operator::In => "is one of",
                Operator::NotIn => "is none of",
                Operator::HasAny => "has any of",
                Operator::HasAll => "has all of",
                Operator::HasNone => "has none of",
            };
            if operand.is_empty() {
                format!("“{name}” {op}")
            } else {
                format!("“{name}” {op} {operand}")
            }
        }
    }
}

fn blocker_text(
    ws: &Workspace,
    binding: &Binding,
    table: &Table,
    transition: &Transition,
    blocker: &Blocker,
) -> String {
    match blocker {
        Blocker::WrongState { current } => {
            format!("not possible while {}", state_name(binding, current))
        }
        Blocker::Capability { capability } => format!("needs “{capability}”"),
        Blocker::Guard => transition.guard.as_ref().map_or_else(
            || "the record does not meet this step's condition".to_owned(),
            |guard| format!("needs {}", condition_text(ws, table, guard)),
        ),
        Blocker::Required { field } => format!("“{}” is required", field_name(table, *field)),
    }
}

/// Sets the state, bumps the version and records the change.
fn apply_move(
    ws: &mut Workspace,
    actor: Actor,
    table_id: TableId,
    record_id: RecordId,
    moved: &Move,
    label: &str,
    now: Timestamp,
) -> ApiResult<()> {
    let (from, to) = (moved.from.as_str(), moved.to.as_str());
    let binding = binding(ws, table_id)
        .cloned()
        .ok_or_else(|| internal("no lifecycle"))?;
    let option =
        option_of(&binding, to).ok_or_else(|| internal(format!("state {to} has no option")))?;
    {
        let record = ws
            .table_mut(table_id)?
            .records
            .iter_mut()
            .find(|r| r.id == record_id)
            .ok_or_else(|| ApiError::not_found(format!("record {record_id}")))?;
        set_cell(record, binding.field, Value::Reference(option));
        record.modified_by = actor.on_behalf_of.unwrap_or(actor.principal);
        record.modified_at = now;
        record.version = record.version.saturating_add(1);
    }
    let title = ws.title(table_id, record_id);
    ws.audit(
        actor,
        "record.transition",
        table_id,
        Some(record_id),
        json!({
            "title": title,
            "transition": label,
            "from": state_name(&binding, from),
            "to": state_name(&binding, to),
        }),
        now,
    )
}

fn running_run(ws: &Workspace, table: TableId, record: RecordId) -> Option<usize> {
    ws.data
        .process
        .runs
        .iter()
        .position(|r| r.table == table && r.record == record && !r.approval.status.is_final())
}

/// Takes a transition for a person.
///
/// # Errors
/// 404, 403 without the right, 409 with every blocker.
pub fn transition(
    ws: &mut Workspace,
    actor: Actor,
    table_id: TableId,
    record_id: RecordId,
    key: &str,
    now: Timestamp,
) -> ApiResult<()> {
    let (table, record) = record_of(ws, table_id, record_id)?;
    let binding = binding(ws, table_id)
        .ok_or_else(|| ApiError::conflict(format!("“{}” has no lifecycle", table.name)))?;
    ws.require(
        actor,
        &Action::RecordRead.into(),
        table.resource(),
        Some(record),
        now,
    )?;
    let transition = binding
        .lifecycle
        .transition(key)
        .ok_or_else(|| ApiError::not_found(format!("transition `{key}`")))?;
    if transition.capability.as_deref() == Some(SYSTEM) {
        return Err(ApiError::forbidden(
            format!("“{}” is decided by the approval", transition.name),
            Json::Null,
        ));
    }
    let capability: Capability = match &transition.capability {
        Some(key) => key.parse().map_err(internal)?,
        None => Action::RecordUpdate.into(),
    };
    ws.require(actor, &capability, table.resource(), Some(record), now)?;
    let current = state_of(binding, record);
    let ctx = context(now);
    let lookup = |field: FieldId| record.cells.get(&field);
    let moved = binding
        .lifecycle
        .take(key, &current, &lookup, &ctx, &|_| true)
        .map_err(|blockers| {
            let reasons: Vec<String> = blockers
                .iter()
                .map(|b| blocker_text(ws, binding, table, transition, b))
                .collect();
            ApiError {
                details: json!(reasons),
                ..ApiError::conflict(format!(
                    "“{}” is not possible now: {}",
                    transition.name,
                    reasons.join("; ")
                ))
            }
        })?;
    let label = transition.name.clone();
    let requester = record.created_by;
    let leaving_approval = binding
        .lifecycle
        .state(&moved.from)
        .is_some_and(|s| s.category == StateCategory::PendingApproval);
    apply_move(ws, actor, table_id, record_id, &moved, &label, now)?;
    if leaving_approval {
        if let Some(index) = running_run(ws, table_id, record_id) {
            cancel_run(ws, index, actor, now)?;
        }
    }
    if let Some(def) = moved.approval {
        start(ws, table_id, record_id, &def, requester, now)?;
    }
    Ok(())
}

fn definition(ws: &Workspace, key: &str, version: Option<u32>) -> ApiResult<ApprovalDef> {
    ws.data
        .process
        .definitions
        .iter()
        .filter(|d| d.key == key && version.is_none_or(|v| v == d.version))
        .max_by_key(|d| d.version)
        .cloned()
        .ok_or_else(|| internal(format!("approval `{key}` is not defined")))
}

fn start(
    ws: &mut Workspace,
    table_id: TableId,
    record_id: RecordId,
    key: &str,
    requester: PrincipalId,
    now: Timestamp,
) -> ApiResult<()> {
    let def = definition(ws, key, None)?;
    let (table, record) = record_of(ws, table_id, record_id)?;
    let (approval, outputs) = with_env(ws, table, record, now, |env| {
        Approval::start(&def, requester, env)
    })
    .map_err(lifecycle_error)?;
    let id = ws
        .data
        .process
        .runs
        .iter()
        .map(|r| r.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    ws.data.process.runs.push(Run {
        id,
        table: table_id,
        record: record_id,
        approval,
    });
    let title = ws.title(table_id, record_id);
    ws.audit(
        Actor::direct(requester),
        "approval.start",
        table_id,
        Some(record_id),
        json!({"title": title, "approval": def.name, "version": def.version}),
        now,
    )?;
    after(ws, id, &def, &outputs, requester, now)
}

fn cancel_run(ws: &mut Workspace, index: usize, actor: Actor, now: Timestamp) -> ApiResult<()> {
    let Some(run) = ws.data.process.runs.get(index).cloned() else {
        return Ok(());
    };
    let def = definition(ws, &run.approval.def, Some(run.approval.version))?;
    let (table, record) = record_of(ws, run.table, run.record)?;
    let mut approval = run.approval.clone();
    let by = actor.on_behalf_of.unwrap_or(actor.principal);
    let outputs = with_env(ws, table, record, now, |env| {
        approval.step(&def, Event::Cancel { by }, env)
    })
    .map_err(lifecycle_error)?;
    if let Some(slot) = ws.data.process.runs.get_mut(index) {
        slot.approval = approval;
    }
    after(ws, run.id, &def, &outputs, by, now)
}

/// Someone acts on one of their tasks.
///
/// # Errors
/// 404, 403 (not their task, an agent acting for someone), 409, 422.
pub fn act(
    ws: &mut Workspace,
    actor: Actor,
    run_id: u64,
    task: u32,
    act: Act,
    now: Timestamp,
) -> ApiResult<(TableId, RecordId)> {
    if actor.on_behalf_of.is_some() {
        return Err(ApiError::forbidden(
            "approvals are decided by people themselves, not by an agent acting for them",
            Json::Null,
        ));
    }
    let index = ws
        .data
        .process
        .runs
        .iter()
        .position(|r| r.id == run_id)
        .ok_or_else(|| ApiError::not_found(format!("approval {run_id}")))?;
    let run = ws
        .data
        .process
        .runs
        .get(index)
        .cloned()
        .ok_or_else(|| internal("run vanished"))?;
    let def = definition(ws, &run.approval.def, Some(run.approval.version))?;
    let (table, record) = record_of(ws, run.table, run.record)?;
    ws.require(
        actor,
        &Action::RecordRead.into(),
        table.resource(),
        Some(record),
        now,
    )?;
    let step = run
        .approval
        .tasks
        .iter()
        .find(|t| t.id == task)
        .and_then(|t| def.node(&t.node))
        .map(|n| n.name().to_owned())
        .unwrap_or_default();
    // An approver named by a capability must still hold it when deciding: revoking a right
    // takes effect on tasks already open, not only on the next approval.
    let deciding = matches!(
        act,
        Act::Approve { .. } | Act::Reject { .. } | Act::Return { .. }
    );
    let opened = run.approval.tasks.iter().find(|t| t.id == task);
    if let (true, Some(opened)) = (deciding, opened) {
        let by_capability: Vec<&String> = match def.node(&opened.node) {
            Some(Node::Approval { approvers, .. }) => {
                let keys: Vec<&String> = approvers
                    .iter()
                    .filter_map(|a| match a {
                        Approver::Capability { capability } => Some(capability),
                        Approver::Person { .. } | Approver::Field { .. } => None,
                    })
                    .collect();
                if keys.len() == approvers.len() {
                    keys
                } else {
                    Vec::new()
                }
            }
            Some(_) | None => Vec::new(),
        };
        if opened.delegated_from.is_none()
            && opened.assignee == actor.principal
            && !by_capability.is_empty()
        {
            let prior = ws.prior(actor, Some(record));
            let holds = by_capability.iter().any(|key| {
                key.parse::<Capability>().is_ok_and(|capability| {
                    ws.decide(
                        actor,
                        &capability,
                        table.resource(),
                        Some(record),
                        &prior,
                        now,
                    )
                    .allowed
                })
            });
            if !holds {
                return Err(ApiError::forbidden(
                    format!(
                        "{} no longer may “{}” this record: ask an administrator to reassign the task",
                        ws.name_of(actor.principal),
                        by_capability.iter().map(|k| k.as_str()).collect::<Vec<_>>().join("” or “")
                    ),
                    Json::Null,
                ));
            }
        }
    }
    let mut approval = run.approval.clone();
    let event = Event::Act {
        task,
        actor: actor.principal,
        act: act.clone(),
    };
    let outputs = with_env(ws, table, record, now, |env| {
        approval.step(&def, event, env)
    })
    .map_err(lifecycle_error)?;
    if let Some(slot) = ws.data.process.runs.get_mut(index) {
        slot.approval = approval;
    }
    let (verb, comment) = match &act {
        Act::Approve { comment } => ("approve", comment.clone()),
        Act::Reject { comment } => ("reject", Some(comment.clone())),
        Act::Return { comment } => ("return", Some(comment.clone())),
        Act::Submit => ("submit", None),
        Act::Delegate { to } => ("delegate", Some(ws.name_of(*to))),
        Act::AddApprover { who } => ("add_approver", Some(ws.name_of(*who))),
        Act::Comment { text } => ("comment", Some(text.clone())),
    };
    let title = ws.title(run.table, run.record);
    ws.audit(
        actor,
        &format!("approval.{verb}"),
        run.table,
        Some(run.record),
        json!({"title": title, "step": step, "task": task, "comment": comment}),
        now,
    )?;
    after(ws, run_id, &def, &outputs, actor.principal, now)?;
    Ok((run.table, run.record))
}

fn notify(
    ws: &mut Workspace,
    to: PrincipalId,
    table: TableId,
    record: RecordId,
    text: String,
    now: Timestamp,
) {
    let id = ws
        .data
        .process
        .inbox
        .iter()
        .map(|n| n.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    ws.data.process.inbox.push(Notice {
        id,
        to,
        at: now,
        text,
        table,
        record,
    });
}

/// Turns what an approval reported into notices and record moves.
fn after(
    ws: &mut Workspace,
    run_id: u64,
    def: &ApprovalDef,
    outputs: &[Output],
    by: PrincipalId,
    now: Timestamp,
) -> ApiResult<()> {
    let Some(run) = ws
        .data
        .process
        .runs
        .iter()
        .find(|r| r.id == run_id)
        .cloned()
    else {
        return Ok(());
    };
    let (table_id, record_id) = (run.table, run.record);
    let title = ws.title(table_id, record_id);
    let requester = run.approval.requester;
    let step_name = |node: &str| {
        def.node(node)
            .map_or_else(|| node.to_owned(), |n| n.name().to_owned())
    };
    for output in outputs {
        match output {
            Output::Opened {
                task,
                assignee,
                node,
            } => {
                let kind = run
                    .approval
                    .tasks
                    .iter()
                    .find(|t| t.id == *task)
                    .map_or(TaskKind::Approve, |t| t.kind);
                let verb = match kind {
                    TaskKind::Approve => "approve",
                    TaskKind::FillIn => "complete",
                };
                let text = format!(
                    "{} asks you to {verb} “{title}” ({})",
                    ws.name_of(requester),
                    step_name(node)
                );
                notify(ws, *assignee, table_id, record_id, text, now);
            }
            Output::Reassigned {
                task: _,
                from,
                to,
                escalated,
            } => {
                let how = if *escalated {
                    "is overdue and now yours"
                } else {
                    "was handed to you"
                };
                let text = format!("“{title}” {how} (from {})", ws.name_of(*from));
                notify(ws, *to, table_id, record_id, text, now);
            }
            Output::Notified { node, to } => {
                for person in to {
                    let text = format!("“{title}”: {}", step_name(node));
                    notify(ws, *person, table_id, record_id, text, now);
                }
            }
            Output::Stuck { node } => {
                let (table, record) = record_of(ws, table_id, record_id)?;
                let admins = with_env(ws, table, record, now, |env| {
                    env.people.holders("access.manage")
                });
                for admin in admins {
                    let text = format!(
                        "“{title}” is stuck at “{}”: nobody may approve it. Change the access rules or cancel it.",
                        step_name(node)
                    );
                    notify(ws, admin, table_id, record_id, text, now);
                }
            }
            Output::Finished { status } => {
                finish(ws, table_id, record_id, *status, by, requester, now)?
            }
            Output::Closed { .. } | Output::Overdue { .. } => {}
        }
    }
    Ok(())
}

fn finish(
    ws: &mut Workspace,
    table_id: TableId,
    record_id: RecordId,
    status: Status,
    by: PrincipalId,
    requester: PrincipalId,
    now: Timestamp,
) -> ApiResult<()> {
    let Some(binding) = binding(ws, table_id).cloned() else {
        return Ok(());
    };
    let title = ws.title(table_id, record_id);
    let (key, text) = match status {
        Status::Approved => (&binding.on_approved, format!("“{title}” was approved")),
        Status::Rejected => (
            &binding.on_rejected,
            format!("“{title}” was rejected by {}", ws.name_of(by)),
        ),
        Status::Returned => (
            &binding.on_returned,
            format!("“{title}” was sent back to you by {}", ws.name_of(by)),
        ),
        Status::Cancelled | Status::Running | Status::Stuck => return Ok(()),
    };
    let (_, record) = record_of(ws, table_id, record_id)?;
    let current = state_of(&binding, record);
    let ctx = context(now);
    let lookup = |field: FieldId| record.cells.get(&field);
    let moved = binding
        .lifecycle
        .take(key, &current, &lookup, &ctx, &|_| true)
        .map_err(|_| {
            internal(format!(
                "the approval finished but “{key}” cannot be taken from {current}"
            ))
        })?;
    let label = binding
        .lifecycle
        .transition(key)
        .map_or_else(|| key.clone(), |t| t.name.clone());
    apply_move(
        ws,
        Actor::direct(by),
        table_id,
        record_id,
        &moved,
        &label,
        now,
    )?;
    notify(ws, requester, table_id, record_id, text, now);
    Ok(())
}

// ------------------------------------------------------------------- views

fn approver_text(ws: &Workspace, table: Option<&Table>, approver: &Approver) -> String {
    match approver {
        Approver::Person { id } => ws.name_of(*id),
        Approver::Field { field } => format!(
            "the people in “{}”",
            table.map_or_else(|| field.to_string(), |t| field_name(t, *field))
        ),
        Approver::Capability { capability } => format!("whoever may “{capability}” it"),
    }
}

fn node_json(ws: &Workspace, table: Option<&Table>, node: &Node) -> Json {
    let list =
        |nodes: &[Node]| -> Vec<Json> { nodes.iter().map(|n| node_json(ws, table, n)).collect() };
    match node {
        Node::Approval {
            key,
            name,
            approvers,
            mode,
            allow_return,
            ..
        } => json!({
            "kind": "approval",
            "key": key,
            "name": name,
            "who": approvers.iter().map(|a| approver_text(ws, table, a)).collect::<Vec<_>>(),
            "mode": match mode {
                Mode::Sequential => "one after another".to_owned(),
                Mode::All => "all of them".to_owned(),
                Mode::Any => "any one of them".to_owned(),
                Mode::Quorum { required } => format!("{required} of them"),
            },
            "canSendBack": allow_return,
        }),
        Node::Branch {
            key,
            name,
            cases,
            otherwise,
        } => json!({
            "kind": "branch",
            "key": key,
            "name": name,
            "cases": cases.iter().map(|c| json!({"name": c.name, "then": list(&c.then)})).collect::<Vec<_>>(),
            "otherwise": list(otherwise),
        }),
        Node::Parallel {
            key,
            name,
            branches,
            join,
        } => json!({
            "kind": "parallel",
            "key": key,
            "name": name,
            "branches": branches.iter().map(|b| list(b)).collect::<Vec<_>>(),
            "join": match join {
                Join::All => "all branches".to_owned(),
                Join::Any => "any branch".to_owned(),
                Join::AtLeast { count } => format!("{count} branches"),
            },
        }),
        Node::FillIn {
            key,
            name,
            assignee,
            fields,
        } => json!({
            "kind": "fill_in",
            "key": key,
            "name": name,
            "who": [approver_text(ws, table, assignee)],
            "fields": fields.iter().map(|f| table.map_or_else(|| f.to_string(), |t| field_name(t, *f))).collect::<Vec<_>>(),
        }),
        Node::Notify { key, name, to } => json!({
            "kind": "notify",
            "key": key,
            "name": name,
            "who": to.iter().map(|a| approver_text(ws, table, a)).collect::<Vec<_>>(),
        }),
    }
}

/// Replaces principal ids in engine texts ("delegated to prn_…") with names.
fn named(ws: &Workspace, text: &str) -> String {
    let mut out = text.to_owned();
    for person in &ws.data.people {
        let id = person.id.to_string();
        if out.contains(&id) {
            out = out.replace(&id, &person.name);
        }
    }
    out
}

fn task_status(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Open => "open",
        TaskStatus::Approved => "approved",
        TaskStatus::Rejected => "rejected",
        TaskStatus::Returned => "sent back",
        TaskStatus::Done => "done",
        TaskStatus::Cancelled => "no longer needed",
    }
}

fn status_text(status: Status) -> &'static str {
    match status {
        Status::Running => "running",
        Status::Approved => "approved",
        Status::Rejected => "rejected",
        Status::Returned => "sent back",
        Status::Cancelled => "cancelled",
        Status::Stuck => "stuck",
    }
}

fn run_json(ws: &Workspace, actor: Actor, run: &Run) -> Json {
    let def = definition(ws, &run.approval.def, Some(run.approval.version)).ok();
    let step = |node: &str| {
        def.as_ref()
            .and_then(|d| d.node(node))
            .map_or_else(|| node.to_owned(), |n| n.name().to_owned())
    };
    let deciding = actor.on_behalf_of.is_none();
    json!({
        "id": run.id,
        "status": status_text(run.approval.status),
        "definition": def.as_ref().map(|d| d.name.clone()),
        "version": run.approval.version,
        "requester": ws.name_of(run.approval.requester),
        "startedAt": run.approval.started_at,
        "endedAt": run.approval.ended_at,
        "tasks": run.approval.tasks.iter().map(|t| json!({
            "id": t.id,
            "step": step(&t.node),
            "kind": match t.kind { TaskKind::Approve => "approve", TaskKind::FillIn => "fill_in" },
            "assignee": ws.name_of(t.assignee),
            "assigneeId": t.assignee,
            "status": task_status(t.status),
            "comment": t.comment,
            "openedAt": t.opened_at,
            "actedAt": t.acted_at,
            "delegatedFrom": t.delegated_from.map(|p| ws.name_of(p)),
            "mine": deciding && t.status == TaskStatus::Open && t.assignee == actor.principal,
            "canSendBack": def.as_ref().and_then(|d| d.node(&t.node)).is_some_and(|n| matches!(n, Node::Approval { allow_return: true, .. })),
        })).collect::<Vec<_>>(),
        "history": run.approval.history.iter().map(|e| json!({
            "at": e.at,
            "actor": ws.name_of(e.actor),
            "what": named(ws, &e.what),
        })).collect::<Vec<_>>(),
    })
}

/// The process part of a record: its state, the buttons with what blocks them, the latest
/// approval, and for a draft who would be asked.
#[must_use]
pub fn record_json(
    ws: &Workspace,
    actor: Actor,
    table: &Table,
    record: &Record,
    now: Timestamp,
) -> Option<Json> {
    let binding = binding(ws, table.id)?;
    let current = state_of(binding, record);
    let ctx = context(now);
    let lookup = |field: FieldId| record.cells.get(&field);
    let prior = ws.prior(actor, Some(record));
    let offers: Vec<Json> = binding
        .lifecycle
        .offers(&current, &lookup, &ctx, &|_| true)
        .into_iter()
        .filter_map(|offer| {
            let transition = binding.lifecycle.transition(&offer.key)?;
            if transition.capability.as_deref() == Some(SYSTEM) {
                return None;
            }
            let capability: Capability = match &transition.capability {
                Some(key) => key.parse().ok()?,
                None => Action::RecordUpdate.into(),
            };
            let decision = ws.decide(
                actor,
                &capability,
                table.resource(),
                Some(record),
                &prior,
                now,
            );
            let mut reasons: Vec<String> = offer
                .blockers
                .iter()
                .map(|b| blocker_text(ws, binding, table, transition, b))
                .collect();
            if !decision.allowed {
                reasons.push(match &transition.capability {
                    Some(key) => format!("{} may not “{key}” here", ws.name_of(actor.principal)),
                    None => format!("{} may not change this record", ws.name_of(actor.principal)),
                });
            }
            Some(json!({
                "key": offer.key,
                "name": offer.name,
                "to": state_name(binding, &offer.to),
                "allowed": reasons.is_empty(),
                "reasons": reasons,
                "startsApproval": transition.approval.is_some(),
            }))
        })
        .collect();
    let run = ws
        .data
        .process
        .runs
        .iter()
        .filter(|r| r.table == table.id && r.record == record.id)
        .max_by_key(|r| r.id);
    // Who would be asked if the record were submitted now.
    let would_ask = binding
        .lifecycle
        .offers(&current, &lookup, &ctx, &|_| true)
        .iter()
        .filter_map(|o| binding.lifecycle.transition(&o.key)?.approval.clone())
        .next()
        .and_then(|key| definition(ws, &key, None).ok())
        .and_then(|def| {
            with_env(ws, table, record, now, |env| {
                mt_lifecycle::approval::simulate(&def, record.created_by, env)
            })
            .ok()
            .map(|(steps, outcome)| {
                json!({
                    "steps": steps.iter().map(|s| json!({
                        "step": def.node(&s.node).map_or_else(|| s.node.clone(), |n| n.name().to_owned()),
                        "people": s.people.iter().map(|p| ws.name_of(*p)).collect::<Vec<_>>(),
                    })).collect::<Vec<_>>(),
                    "outcome": status_text(outcome),
                })
            })
        });
    let state = binding.lifecycle.state(&current);
    Some(json!({
        "state": {
            "key": current,
            "name": state_name(binding, &current),
            "category": state.map(|s| s.category.to_string()),
        },
        "states": binding.lifecycle.states.iter().map(|s| json!({"key": s.key, "name": s.name, "category": s.category.to_string()})).collect::<Vec<_>>(),
        "transitions": offers,
        "approval": run.map(|r| run_json(ws, actor, r)),
        "wouldAsk": would_ask,
    }))
}

/// Which fields are frozen in the record's current state.
#[must_use]
pub fn frozen(
    ws: &Workspace,
    table: &Table,
    record: &Record,
) -> Option<(FieldId, Vec<FieldId>, String)> {
    let binding = binding(ws, table.id)?;
    let state = state_of(binding, record);
    let frozen = table
        .fields
        .iter()
        .filter(|f| !binding.lifecycle.may_edit(&state, f.id))
        .map(|f| f.id)
        .collect();
    Some((binding.field, frozen, state_name(binding, &state)))
}

/// The amount that sums up a record for an approver: the last computed money field (a
/// total), else the first money field.
#[must_use]
pub fn headline_amount(table: &Table) -> Option<&mt_core::field::FieldDef> {
    let money = |f: &&mt_core::field::FieldDef| f.data_type == mt_core::field::DataType::Money;
    table
        .fields
        .iter()
        .rev()
        .filter(money)
        .find(|f| f.is_computed())
        .or_else(|| table.fields.iter().find(money))
}

/// The capabilities an approval of the table asks its approvers to hold.
#[must_use]
pub fn approval_capabilities(ws: &Workspace, table: TableId) -> Vec<String> {
    let Some(binding) = binding(ws, table) else {
        return Vec::new();
    };
    let mut keys = Vec::new();
    for transition in &binding.lifecycle.transitions {
        let Some(def) = transition
            .approval
            .as_ref()
            .and_then(|k| definition(ws, k, None).ok())
        else {
            continue;
        };
        for node in def.nodes() {
            if let Node::Approval { approvers, .. } = node {
                for approver in approvers {
                    if let Approver::Capability { capability } = approver {
                        if !keys.contains(capability) {
                            keys.push(capability.clone());
                        }
                    }
                }
            }
        }
    }
    keys
}

/// The viewer's open tasks.
#[must_use]
pub fn tasks_json(ws: &Workspace, actor: Actor) -> Json {
    let mut out = Vec::new();
    if actor.on_behalf_of.is_some() {
        return json!(out);
    }
    for run in ws.data.process.runs.iter().rev() {
        let def = definition(ws, &run.approval.def, Some(run.approval.version)).ok();
        for task in run.approval.tasks_of(actor.principal) {
            let table = ws.table(run.table).ok();
            let total = table.and_then(|t| {
                let field = headline_amount(t)?;
                let value = t.record(run.record)?.cells.get(&field.id)?;
                Some(crate::view::display(ws, field, value))
            });
            out.push(json!({
                "run": run.id,
                "task": task.id,
                "kind": match task.kind { TaskKind::Approve => "approve", TaskKind::FillIn => "fill_in" },
                "step": def.as_ref().and_then(|d| d.node(&task.node)).map_or_else(|| task.node.clone(), |n| n.name().to_owned()),
                "table": run.table,
                "tableName": table.map(|t| t.name.clone()),
                "record": run.record,
                "title": ws.title(run.table, run.record),
                "requester": ws.name_of(run.approval.requester),
                "total": total,
                "openedAt": task.opened_at,
                "delegatedFrom": task.delegated_from.map(|p| ws.name_of(p)),
                "canSendBack": def.as_ref().and_then(|d| d.node(&task.node)).is_some_and(|n| matches!(n, Node::Approval { allow_return: true, .. })),
            }));
        }
    }
    json!(out)
}

/// The approvals of records the viewer can read, newest first.
#[must_use]
pub fn runs_json(ws: &Workspace, actor: Actor, now: Timestamp) -> Json {
    let list: Vec<Json> = ws
        .data
        .process
        .runs
        .iter()
        .rev()
        .filter(|run| {
            record_of(ws, run.table, run.record).is_ok_and(|(table, record)| {
                ws.decide(
                    actor,
                    &Action::RecordRead.into(),
                    table.resource(),
                    Some(record),
                    &ws.prior(actor, Some(record)),
                    now,
                )
                .allowed
            })
        })
        .map(|run| {
            let waiting: Vec<String> = run
                .approval
                .open_tasks()
                .iter()
                .map(|t| ws.name_of(t.assignee))
                .collect();
            json!({
                "id": run.id,
                "table": run.table,
                "record": run.record,
                "title": ws.title(run.table, run.record),
                "status": status_text(run.approval.status),
                "requester": ws.name_of(run.approval.requester),
                "waitingFor": waiting,
                "startedAt": run.approval.started_at,
                "endedAt": run.approval.ended_at,
            })
        })
        .collect();
    json!(list)
}

/// The viewer's notices, newest first.
#[must_use]
pub fn inbox_json(ws: &Workspace, actor: Actor) -> Json {
    let me = actor.on_behalf_of.unwrap_or(actor.principal);
    let list: Vec<Json> = ws
        .data
        .process
        .inbox
        .iter()
        .rev()
        .filter(|n| n.to == me)
        .map(|n| json!({"id": n.id, "at": n.at, "text": n.text, "table": n.table, "record": n.record}))
        .collect();
    json!(list)
}

/// The lifecycles and approval definitions, for the process page.
#[must_use]
pub fn definitions_json(ws: &Workspace) -> Json {
    let lifecycles: Vec<Json> = ws
        .data
        .process
        .bindings
        .iter()
        .map(|b| {
            let table = ws.table(b.lifecycle.table).ok();
            json!({
                "name": b.lifecycle.name,
                "table": table.map(|t| t.name.clone()),
                "initial": state_name(b, &b.lifecycle.initial),
                "states": b.lifecycle.states.iter().map(|s| json!({
                    "key": s.key,
                    "name": s.name,
                    "category": s.category.to_string(),
                    "required": s.required.iter().map(|f| table.map_or_else(|| f.to_string(), |t| field_name(t, *f))).collect::<Vec<_>>(),
                    "frozen": s.editable.as_ref().map(|e| e.is_empty()),
                })).collect::<Vec<_>>(),
                "transitions": b.lifecycle.transitions.iter().map(|t| json!({
                    "key": t.key,
                    "name": t.name,
                    "from": if t.from.is_empty() { vec!["any open state".to_owned()] } else { t.from.iter().map(|s| state_name(b, s)).collect() },
                    "to": state_name(b, &t.to),
                    "who": match t.capability.as_deref() {
                        Some(SYSTEM) => "the approval".to_owned(),
                        Some(key) => format!("whoever may “{key}”"),
                        None => "whoever may change the record".to_owned(),
                    },
                    "approval": t.approval.as_ref().and_then(|k| definition(ws, k, None).ok()).map(|d| d.name),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let approvals: Vec<Json> = ws
        .data
        .process
        .definitions
        .iter()
        .map(|d| {
            let table = ws
                .data
                .process
                .bindings
                .iter()
                .find(|b| {
                    b.lifecycle
                        .transitions
                        .iter()
                        .any(|t| t.approval.as_deref() == Some(d.key.as_str()))
                })
                .and_then(|b| ws.table(b.lifecycle.table).ok());
            json!({
                "key": d.key,
                "name": d.name,
                "version": d.version,
                "steps": d.steps.iter().map(|n| node_json(ws, table, n)).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({"lifecycles": lifecycles, "approvals": approvals})
}

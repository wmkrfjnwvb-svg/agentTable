//! Designing a table's process on screen: its lifecycle (states, buttons, who may press
//! them, locks, required fields) and its approval (steps, approvers, conditions, outcomes).
//!
//! The web app edits a [`ProcessDesign`]; [`publish`] compiles it into the engines'
//! definitions (`mt-lifecycle`), checks them (the closed-loop rule, field references,
//! approval structure), creates or reuses the state field, and binds everything to the table.
//! An approval gets a new version only when it changes; running approvals keep theirs.

use crate::error::{ApiError, ApiResult};
use crate::model::{set_cell, Workspace};
use crate::process::{Binding, Child, StateOption, SYSTEM};
use mt_core::condition::{Condition, Operand, Operator, Test};
use mt_core::field::{DataType, FieldDef, FieldRole, InlineOption, OptionsSource, Widget};
use mt_core::id::{FieldId, LifecycleId, RefItemId, TableId};
use mt_core::state::StateCategory;
use mt_core::time::Timestamp;
use mt_core::value::Value;
use mt_lifecycle::approval::{ApprovalDef, Approver, Case, Mode, Node};
use mt_lifecycle::lifecycle::{Lifecycle, StateDef, Transition};
use mt_perm::{Action, Actor};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;

/// A comparison of one field with a typed-in value: `Total > 1000 USD`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimpleCondition {
    /// The field.
    pub field: FieldId,
    /// The comparison.
    pub op: Operator,
    /// The value as typed (`1000 USD`, `yes`, an option id); empty for "is empty".
    #[serde(default)]
    pub text: String,
}

/// One state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateDesign {
    /// Stable key; generated from the name when empty.
    #[serde(default)]
    pub key: String,
    /// Name.
    pub name: String,
    /// Category.
    pub category: StateCategory,
    /// Fields that must have a value to enter it.
    #[serde(default)]
    pub required: Vec<FieldId>,
    /// Whether the record's fields are locked in this state.
    #[serde(default)]
    pub locked: bool,
    /// Fields that stay editable while locked.
    #[serde(default)]
    pub editable: Vec<FieldId>,
}

/// One button.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionDesign {
    /// Stable key; generated from the name when empty.
    #[serde(default)]
    pub key: String,
    /// The button label.
    pub name: String,
    /// The states it leaves from (any open state when empty).
    #[serde(default)]
    pub from: Vec<String>,
    /// The state it leads to.
    pub to: String,
    /// The capability needed (whoever may change the record when absent).
    #[serde(default)]
    pub capability: Option<String>,
    /// A condition the record must meet.
    #[serde(default)]
    pub when: Option<SimpleCondition>,
}

/// One approval step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StepDesign {
    /// People approve.
    Approval {
        /// Name.
        name: String,
        /// Who.
        approvers: Vec<Approver>,
        /// How they decide.
        mode: Mode,
        /// Whether they may send the request back.
        #[serde(default, rename = "canSendBack")]
        can_send_back: bool,
        /// Only when this holds.
        #[serde(default)]
        when: Option<SimpleCondition>,
    },
    /// People are told.
    Notify {
        /// Name.
        name: String,
        /// Whom.
        to: Vec<Approver>,
        /// Only when this holds.
        #[serde(default)]
        when: Option<SimpleCondition>,
    },
}

/// The approval of a table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalDesign {
    /// Name.
    pub name: String,
    /// The transition that starts it (its target state waits for the decision).
    #[serde(rename = "startedBy")]
    pub started_by: String,
    /// Where the record goes when approved.
    pub approved: String,
    /// Where it goes when rejected.
    pub rejected: String,
    /// Where it goes when sent back.
    pub returned: String,
    /// The steps, in order.
    pub steps: Vec<StepDesign>,
}

/// A table's process as the designer edits it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessDesign {
    /// Name of the lifecycle.
    pub name: String,
    /// The states.
    pub states: Vec<StateDesign>,
    /// The state new records start in.
    pub initial: String,
    /// The buttons.
    pub transitions: Vec<TransitionDesign>,
    /// The approval, if any.
    #[serde(default)]
    pub approval: Option<ApprovalDesign>,
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::invalid(message.into(), json!(null))
}

/// A key from a name: `Send back` → `send_back`.
fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    let trimmed = out.trim_end_matches('_');
    let mut key: String = trimmed.chars().take(40).collect();
    if !key.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
        key.insert_str(0, "s_");
    }
    key
}

fn unique(base: String, taken: &mut BTreeSet<String>) -> String {
    let mut key = base.clone();
    let mut n = 2_u32;
    while taken.contains(&key) {
        key = format!("{base}_{n}");
        n = n.saturating_add(1);
    }
    taken.insert(key.clone());
    key
}

/// The typed value of a simple condition.
/// Compiles a simple condition against a table's fields.
///
/// # Errors
/// 422 for an unknown field or a value that does not fit it.
pub fn condition(table: &crate::model::Table, simple: &SimpleCondition) -> ApiResult<Condition> {
    let field = table
        .field(simple.field)
        .ok_or_else(|| invalid(format!("{} is not a field of {}", simple.field, table.name)))?;
    let operand = match simple.op {
        Operator::IsEmpty | Operator::IsNotEmpty => Operand::None,
        Operator::Eq
        | Operator::Ne
        | Operator::Lt
        | Operator::Le
        | Operator::Gt
        | Operator::Ge
        | Operator::Between
        | Operator::Contains
        | Operator::NotContains
        | Operator::StartsWith
        | Operator::EndsWith
        | Operator::In
        | Operator::NotIn
        | Operator::HasAny
        | Operator::HasAll
        | Operator::HasNone => {
            let text = simple.text.trim();
            let value = match field.data_type {
                DataType::Reference => Value::Reference(
                    text.parse::<RefItemId>()
                        .map_err(|_| invalid(format!("choose a value of “{}”", field.name)))?,
                ),
                DataType::Text | DataType::LongText
                    if matches!(
                        simple.op,
                        Operator::Contains
                            | Operator::NotContains
                            | Operator::StartsWith
                            | Operator::EndsWith
                            | Operator::Eq
                            | Operator::Ne
                    ) =>
                {
                    Value::Text(text.to_owned())
                }
                DataType::Text
                | DataType::LongText
                | DataType::Number
                | DataType::Money
                | DataType::Percent
                | DataType::Bool
                | DataType::Date
                | DataType::DateTime
                | DataType::Duration
                | DataType::MultiReference
                | DataType::Link
                | DataType::Principal
                | DataType::Attachment
                | DataType::Json => {
                    let currency = field.validation.currencies.first().copied();
                    mt_core::coerce::parse_input(
                        field.data_type,
                        text,
                        &mt_core::coerce::ParseContext {
                            default_currency: currency,
                        },
                    )
                    .map_err(|e| invalid(format!("“{}”: {e}", field.name)))?
                }
            };
            Operand::Value(value)
        }
    };
    let built = Condition::Test(Test {
        field: simple.field,
        op: simple.op,
        operand,
    });
    built
        .check(&|id| table.field(id).map(|f| f.data_type))
        .map_err(|e| invalid(format!("condition on “{}”: {e}", field.name)))?;
    Ok(built)
}

/// Compiles the approval steps into engine nodes.
fn steps(
    ws: &Workspace,
    table: &crate::model::Table,
    design: &[StepDesign],
) -> ApiResult<Vec<Node>> {
    let mut taken = BTreeSet::new();
    let mut nodes = Vec::new();
    for step in design {
        let (name, when) = match step {
            StepDesign::Approval { name, when, .. } | StepDesign::Notify { name, when, .. } => {
                (name.trim().to_owned(), when.as_ref())
            }
        };
        if name.is_empty() {
            return Err(invalid("every step needs a name"));
        }
        let key = unique(slug(&name), &mut taken);
        let node = match step {
            StepDesign::Approval {
                approvers,
                mode,
                can_send_back,
                ..
            } => Node::Approval {
                key: key.clone(),
                name: name.clone(),
                approvers: approvers.clone(),
                mode: *mode,
                allow_return: *can_send_back,
                timeout: None,
                escalate_to: None,
            },
            StepDesign::Notify { to, .. } => Node::Notify {
                key: key.clone(),
                name: name.clone(),
                to: to.clone(),
            },
        };
        match when {
            None => nodes.push(node),
            Some(simple) => {
                let when = condition(table, simple)?;
                let text = crate::process::condition_text(ws, table, &when);
                nodes.push(Node::Branch {
                    key: unique(format!("{key}_when"), &mut taken),
                    name: format!("Only when {text}"),
                    cases: vec![Case {
                        name: text,
                        when,
                        then: vec![node],
                    }],
                    otherwise: Vec::new(),
                });
            }
        }
    }
    Ok(nodes)
}

/// The design of a table's process (`None` without a lifecycle).
#[must_use]
pub fn design_of(ws: &Workspace, table: TableId) -> Option<ProcessDesign> {
    let binding = crate::process::binding(ws, table)?;
    if let Some(design) = &binding.design {
        return Some(design.clone());
    }
    // Saved before designs were kept: derive what can be derived.
    let lifecycle = &binding.lifecycle;
    Some(ProcessDesign {
        name: lifecycle.name.clone(),
        states: lifecycle
            .states
            .iter()
            .map(|s| StateDesign {
                key: s.key.clone(),
                name: s.name.clone(),
                category: s.category,
                required: s.required.clone(),
                locked: s.editable.is_some(),
                editable: s.editable.clone().unwrap_or_default(),
            })
            .collect(),
        initial: lifecycle.initial.clone(),
        transitions: lifecycle
            .transitions
            .iter()
            .filter(|t| t.capability.as_deref() != Some(SYSTEM))
            .map(|t| TransitionDesign {
                key: t.key.clone(),
                name: t.name.clone(),
                from: t.from.clone(),
                to: t.to.clone(),
                capability: t.capability.clone(),
                when: None,
            })
            .collect(),
        approval: None,
    })
}

/// The approval definition key of a table.
fn approval_key(table: TableId) -> String {
    format!("{}_approval", table.to_string().to_lowercase())
}

/// Publishes a table's process design.
///
/// # Errors
/// 403 without `schema.manage`; 422 for a design the engines refuse (with their reason);
/// 409 when records are in a state the design removes.
#[allow(clippy::too_many_lines)]
pub fn publish(
    ws: &mut Workspace,
    actor: Actor,
    table_id: TableId,
    mut design: ProcessDesign,
    now: Timestamp,
) -> ApiResult<()> {
    let table = ws.table(table_id)?.clone();
    ws.require(
        actor,
        &Action::SchemaManage.into(),
        table.resource(),
        None,
        now,
    )?;
    if design.name.trim().is_empty() {
        design.name = table.name.clone();
    }
    // Keys.
    let mut taken = BTreeSet::new();
    for state in &mut design.states {
        state.name = state.name.trim().to_owned();
        if state.name.is_empty() {
            return Err(invalid("every state needs a name"));
        }
        let base = if state.key.is_empty() {
            slug(&state.name)
        } else {
            state.key.clone()
        };
        state.key = unique(base, &mut taken);
    }
    let mut transition_keys = BTreeSet::new();
    for transition in &mut design.transitions {
        transition.name = transition.name.trim().to_owned();
        let base = if transition.key.is_empty() {
            slug(&transition.name)
        } else {
            transition.key.clone()
        };
        transition.key = unique(base, &mut transition_keys);
        transition.capability = transition
            .capability
            .take()
            .map(|c| c.trim().to_owned())
            .filter(|c| !c.is_empty());
    }
    let existing = crate::process::binding(ws, table_id).cloned();
    // The state field: the bound one, an unbound state field of the table, or a new one.
    let state_field: Option<FieldDef> = existing
        .as_ref()
        .and_then(|b| table.field(b.field).cloned())
        .or_else(|| {
            table
                .fields
                .iter()
                .find(|f| f.role == Some(FieldRole::State) && f.data_type == DataType::Reference)
                .cloned()
        });
    let old_options: Vec<InlineOption> = state_field
        .as_ref()
        .and_then(|f| f.inline_options().map(<[InlineOption]>::to_vec))
        .unwrap_or_default();
    let mut options = Vec::new();
    let mut mapping = Vec::new();
    for state in &design.states {
        let known = existing
            .as_ref()
            .and_then(|b| {
                b.options
                    .iter()
                    .find(|o| o.state == state.key)
                    .map(|o| o.option)
            })
            .or_else(|| {
                old_options
                    .iter()
                    .find(|o| o.label.eq_ignore_ascii_case(&state.name))
                    .map(|o| o.id)
            });
        let id = match known {
            Some(id) => id,
            None => ws.new_id(now, RefItemId::new)?,
        };
        let color = old_options
            .iter()
            .find(|o| o.id == id)
            .and_then(|o| o.color.clone());
        options.push(InlineOption {
            id,
            label: state.name.clone(),
            color,
            enabled: true,
        });
        mapping.push(StateOption {
            state: state.key.clone(),
            option: id,
        });
    }
    // Records in a state the design no longer has.
    if let Some(field) = &state_field {
        let kept: BTreeSet<RefItemId> = options.iter().map(|o| o.id).collect();
        for option in &old_options {
            if kept.contains(&option.id) {
                continue;
            }
            let count = table
                .records
                .iter()
                .filter(|r| r.cells.get(&field.id) == Some(&Value::Reference(option.id)))
                .count();
            if count > 0 {
                return Err(ApiError::conflict(format!(
                    "{count} record(s) are in “{}”: keep that state or move them first",
                    option.label
                )));
            }
        }
    }
    // The lifecycle.
    let states: Vec<StateDef> = design
        .states
        .iter()
        .map(|s| StateDef {
            key: s.key.clone(),
            name: s.name.clone(),
            category: s.category,
            required: s.required.clone(),
            editable: s.locked.then(|| s.editable.clone()),
        })
        .collect();
    let mut transitions = Vec::new();
    for t in &design.transitions {
        transitions.push(Transition {
            key: t.key.clone(),
            name: t.name.clone(),
            from: t.from.clone(),
            to: t.to.clone(),
            capability: t.capability.clone(),
            guard: t.when.as_ref().map(|w| condition(&table, w)).transpose()?,
            approval: None,
        });
    }
    let key = existing
        .as_ref()
        .and_then(|b| {
            b.lifecycle
                .transitions
                .iter()
                .find_map(|t| t.approval.clone())
        })
        .unwrap_or_else(|| approval_key(table_id));
    let mut outcomes = (String::new(), String::new(), String::new());
    let mut def: Option<ApprovalDef> = None;
    if let Some(approval) = &design.approval {
        let starter = transitions
            .iter_mut()
            .find(|t| t.key == approval.started_by)
            .ok_or_else(|| invalid("choose the button that starts the approval"))?;
        starter.approval = Some(key.clone());
        let waiting = starter.to.clone();
        let category = design
            .states
            .iter()
            .find(|s| s.key == waiting)
            .map(|s| s.category);
        if category != Some(StateCategory::PendingApproval) {
            return Err(invalid(format!(
                "“{}” leads to a state that does not wait for approval: give that state the category “pending approval”",
                starter.name
            )));
        }
        let mut system = |suffix: &str, label: &str, to: &str| -> ApiResult<String> {
            if !design.states.iter().any(|s| s.key == to) {
                return Err(invalid(format!(
                    "choose where the record goes when {label}"
                )));
            }
            let key = unique(
                format!("{}_{suffix}", approval.started_by),
                &mut transition_keys,
            );
            transitions.push(Transition {
                key: key.clone(),
                name: label[..1].to_uppercase() + &label[1..],
                from: vec![waiting.clone()],
                to: to.to_owned(),
                capability: Some(SYSTEM.to_owned()),
                guard: None,
                approval: None,
            });
            Ok(key)
        };
        let approved = system("approved", "approved", &approval.approved)?;
        let rejected = system("rejected", "rejected", &approval.rejected)?;
        let returned = system("returned", "sent back", &approval.returned)?;
        outcomes = (approved, rejected, returned);
        let compiled = ApprovalDef {
            key: key.clone(),
            name: if approval.name.trim().is_empty() {
                "Approval".to_owned()
            } else {
                approval.name.trim().to_owned()
            },
            version: 1,
            steps: steps(ws, &table, &approval.steps)?,
        };
        let latest = ws
            .data
            .process
            .definitions
            .iter()
            .filter(|d| d.key == key)
            .max_by_key(|d| d.version)
            .cloned();
        def = Some(match latest {
            Some(latest) if latest.name == compiled.name && latest.steps == compiled.steps => {
                latest
            }
            Some(latest) => ApprovalDef {
                version: latest.version.saturating_add(1),
                ..compiled
            },
            None => compiled,
        });
    }
    let lifecycle = Lifecycle {
        id: existing
            .as_ref()
            .map_or(ws.new_id(now, LifecycleId::new)?, |b| b.lifecycle.id),
        table: table_id,
        name: design.name.trim().to_owned(),
        states,
        initial: design.initial.clone(),
        transitions,
    };
    // Engine messages name states and buttons by key; people know them by name.
    let names: Vec<(String, String)> = design
        .states
        .iter()
        .map(|st| (st.key.clone(), st.name.clone()))
        .chain(
            design
                .transitions
                .iter()
                .map(|t| (t.key.clone(), t.name.clone())),
        )
        .collect();
    let refuse = |e: mt_lifecycle::LifecycleError| {
        let mut text = e.to_string();
        for (key, name) in &names {
            text = text.replace(&format!("`{key}`"), &format!("“{name}”"));
        }
        invalid(text)
    };
    lifecycle.check().map_err(refuse)?;
    let type_of = |field: FieldId| table.field(field).map(|f| f.data_type);
    lifecycle.check_in_table(&type_of).map_err(refuse)?;
    if let Some(def) = &def {
        def.check().map_err(refuse)?;
        def.check_in_table(&type_of).map_err(refuse)?;
    }
    // Write: the state field, the binding, the definition, the records' states.
    let field_id = match &state_field {
        Some(field) => field.id,
        None => ws.new_field_id(now)?,
    };
    let mut field = state_field.unwrap_or_else(|| {
        let name = if table
            .fields
            .iter()
            .any(|f| f.name.eq_ignore_ascii_case("Status"))
        {
            "Lifecycle state"
        } else {
            "Status"
        };
        FieldDef::new(field_id, name, DataType::Reference)
    });
    field.role = Some(FieldRole::State);
    field.widget = Widget::Segmented;
    let disabled: Vec<InlineOption> = old_options
        .iter()
        .filter(|o| !options.iter().any(|n| n.id == o.id))
        .map(|o| InlineOption {
            enabled: false,
            ..o.clone()
        })
        .collect();
    options.extend(disabled);
    field.options = OptionsSource::Inline { options };
    field.check().map_err(|e| invalid(e.to_string()))?;
    let initial_option = mapping
        .iter()
        .find(|m| m.state == design.initial)
        .map(|m| m.option)
        .ok_or_else(|| invalid("choose the state new records start in"))?;
    {
        let target = ws.table_mut(table_id)?;
        match target.fields.iter_mut().find(|f| f.id == field_id) {
            Some(slot) => *slot = field,
            None => {
                let at = target.fields.len().min(1);
                target.fields.insert(at, field);
            }
        }
        for record in &mut target.records {
            if !record.cells.contains_key(&field_id) {
                set_cell(record, field_id, Value::Reference(initial_option));
            }
        }
    }
    let children: Vec<Child> = existing
        .as_ref()
        .map(|b| b.children.clone())
        .unwrap_or_else(|| {
            // Lines linked to this table through a single-record mirror are part of its records.
            ws.data
                .links
                .iter()
                .filter_map(|p| {
                    let (parent_field, child_table, link) = if p.table == table_id {
                        (p.field, p.inverse_table, p.inverse_field)
                    } else if p.inverse_table == table_id {
                        (p.inverse_field, p.table, p.field)
                    } else {
                        return None;
                    };
                    let single = ws
                        .table(child_table)
                        .ok()
                        .and_then(|t| t.field(link))
                        .is_some_and(|f| f.validation.max_items == Some(1));
                    single.then_some(Child {
                        table: child_table,
                        link,
                        parent_field,
                    })
                })
                .collect()
        });
    let binding = Binding {
        lifecycle,
        field: field_id,
        options: mapping,
        children,
        on_approved: outcomes.0,
        on_rejected: outcomes.1,
        on_returned: outcomes.2,
        design: Some(design.clone()),
    };
    let summary = json!({
        "lifecycle": binding.lifecycle.name,
        "states": binding.lifecycle.states.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
        "approval": def.as_ref().map(|d| format!("{} v{}", d.name, d.version)),
    });
    ws.data
        .process
        .bindings
        .retain(|b| b.lifecycle.table != table_id);
    ws.data.process.bindings.push(binding);
    if let Some(def) = def {
        if !ws.data.process.definitions.contains(&def) {
            ws.data.process.definitions.push(def);
        }
    }
    ws.rebuild_graph()?;
    ws.audit(actor, "process.publish", table_id, None, summary, now)
}

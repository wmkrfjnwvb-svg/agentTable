//! The HTTP API the web app uses. A preview of the Public API (`mt-api`, wave 4): the same
//! resources, without OpenAPI documents, versioning or real sign-in yet.
//!
//! Who is asking comes from the `x-user` header (a principal id), and `x-on-behalf-of` when
//! an agent acts for a person. Real identity (passwords, SSO, sessions) arrives with
//! `mt-identity` in wave 4; until then the web app offers a user switcher.

use crate::error::{ApiError, ApiResult};
use crate::model::{context, CellInput, Table, Workspace};
use crate::seed::ALICE;
use crate::view;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use mt_core::condition::{Condition, Operand, Operator};
use mt_core::field::{Classification, DataType, FieldDef, OptionsSource};
use mt_core::fieldtype::{search as search_types, suggest, BuiltinType, PickerGroup, Requirement};
use mt_core::id::{FieldId, PrincipalId, RecordId, RefSetId, TableId};
use mt_core::time::Timestamp;
use mt_core::value::Value;
use mt_formula::Type;
use mt_perm::{Action, Actor, Capability, PrincipalKind, Resource, Visibility};
use mt_query::{Aggregate, AggregateFn, GroupKey, SortKey};
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Where the time comes from.
#[derive(Debug, Clone, Copy)]
pub enum Clock {
    /// The system clock.
    System,
    /// A fixed instant (tests).
    Fixed(Timestamp),
}

impl Clock {
    /// The current instant.
    #[must_use]
    pub fn now(self) -> Timestamp {
        match self {
            Clock::Fixed(at) => at,
            Clock::System => {
                let millis = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0));
                Timestamp::from_millis(millis).unwrap_or(Timestamp::MIN)
            }
        }
    }
}

/// The server's state.
#[derive(Debug)]
pub struct App {
    /// The workspace.
    pub ws: Mutex<Workspace>,
    /// Where it is saved (not saved, if absent).
    pub data_file: Option<PathBuf>,
    /// The clock.
    pub clock: Clock,
}

type Shared = Arc<App>;
type Reply = ApiResult<Json<JsonValue>>;

impl App {
    /// Escalates or reports overdue approval tasks (the server calls this every minute).
    ///
    /// # Errors
    /// When the workspace cannot be saved.
    pub async fn tick(&self) -> ApiResult<usize> {
        let mut ws = self.ws.lock().await;
        let changed = crate::process::tick_all(&mut ws, self.clock.now())?;
        if changed > 0 {
            self.save(&ws)?;
        }
        Ok(changed)
    }

    /// Saves the workspace after a change.
    fn save(&self, ws: &Workspace) -> ApiResult<()> {
        let Some(path) = &self.data_file else {
            return Ok(());
        };
        let text =
            serde_json::to_vec_pretty(&ws.data).map_err(|e| ApiError::internal(e.to_string()))?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, text).map_err(|e| ApiError::internal(e.to_string()))?;
        std::fs::rename(&temporary, path).map_err(|e| ApiError::internal(e.to_string()))
    }
}

fn parse<T: FromStr>(text: &str, what: &str) -> ApiResult<T> {
    text.parse::<T>()
        .map_err(|_| ApiError::bad_request(format!("`{text}` is not a {what} id")))
}

/// Who is asking.
fn actor(ws: &Workspace, headers: &HeaderMap) -> ApiResult<Actor> {
    let header = |name: &str| -> ApiResult<Option<PrincipalId>> {
        match headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty())
        {
            Some(text) => {
                let id: PrincipalId = parse(text, "principal")?;
                match ws.person(id).map(|p| p.kind) {
                    Some(PrincipalKind::Group) | None => {
                        Err(ApiError::bad_request(format!("{text} cannot sign in")))
                    }
                    Some(
                        PrincipalKind::Member
                        | PrincipalKind::Guest
                        | PrincipalKind::Agent
                        | PrincipalKind::App,
                    ) => Ok(Some(id)),
                }
            }
            None => Ok(None),
        }
    };
    let principal = header("x-user")?.unwrap_or(ALICE);
    Ok(Actor {
        principal,
        on_behalf_of: header("x-on-behalf-of")?.filter(|p| *p != principal),
    })
}

/// The routes.
pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/status", get(status))
        .route("/api/people", get(people).post(create_person))
        .route("/api/tables", get(tables).post(create_table))
        .route(
            "/api/tables/{table}",
            get(table).patch(update_table).delete(delete_table),
        )
        .route("/api/bases", get(bases).post(create_base))
        .route("/api/bases/{base}", patch(update_base).delete(delete_base))
        .route(
            "/api/tables/{table}/fields/{field}",
            patch(update_field).delete(delete_field),
        )
        .route("/api/refdata/{set}/items", post(add_item))
        .route("/api/refdata/{set}/items/{item}", patch(update_item))
        .route("/api/people/{person}", patch(update_person))
        .route("/api/settings", get(settings))
        .route(
            "/api/settings/{key}",
            put(set_setting).delete(clear_setting),
        )
        .route("/api/settings/{key}/explain", get(explain_setting))
        .route(
            "/api/tables/{table}/process",
            get(process_design).put(publish_process),
        )
        .route("/api/access", get(access_overview))
        .route("/api/access/grants", post(give_grant))
        .route(
            "/api/access/grants/{grant}",
            axum::routing::delete(revoke_grant),
        )
        .route("/api/access/reviews", post(open_review))
        .route(
            "/api/access/reviews/{review}/decisions",
            post(decide_review),
        )
        .route(
            "/api/access/reviews/{review}/complete",
            post(complete_review),
        )
        .route("/api/access/reviews/{review}/cancel", post(cancel_review))
        .route("/api/tables/{table}/shares", post(create_share))
        .route("/api/shares/{share}", axum::routing::delete(revoke_share))
        .route("/api/share/{token}", get(open_share))
        .route("/api/access/sets", post(create_set))
        .route("/api/access/rules", post(add_rule))
        .route("/api/templates", get(templates))
        .route("/api/templates/{key}/install", post(install_template))
        .route("/api/tables/{table}/query", post(query))
        .route("/api/tables/{table}/records", post(create))
        .route(
            "/api/tables/{table}/records/{record}",
            get(record).patch(update).delete(delete),
        )
        .route("/api/tables/{table}/fields", post(add_field))
        .route("/api/tables/{table}/fields/{field}/options", get(options))
        .route("/api/tables/{table}/formula", post(check_formula))
        .route("/api/field-types", get(field_types))
        .route("/api/refdata", get(refsets).post(create_refset))
        .route("/api/refdata/{set}", get(refset).patch(update_refset))
        .route(
            "/api/refdata/{set}/proposals/{proposal}",
            post(decide_proposal),
        )
        .route("/api/capabilities", get(capabilities))
        .route("/api/explain", get(explain))
        .route("/api/audit", get(audit))
        .route("/api/audit/verify", get(verify))
        .route("/api/audit/tamper-check", post(tamper_check))
        .route(
            "/api/tables/{table}/records/{record}/transitions/{key}",
            post(transition),
        )
        .route("/api/approvals", get(approvals))
        .route("/api/approvals/{run}/tasks/{task}", post(act))
        .route("/api/tasks", get(tasks))
        .route("/api/process/tick", post(tick))
        .route("/api/inbox", get(inbox))
        .route("/api/process", get(process_definitions))
}

// ------------------------------------------------------------------- status

async fn status() -> Json<JsonValue> {
    let component = |name: &str, wave: u8, state: &str, summary: &str| json!({"crate": name, "wave": wave, "state": state, "summary": summary});
    Json(json!({
        "product": "agenttable",
        "version": env!("CARGO_PKG_VERSION"),
        "components": [
            component("mt-core", 1, "live", "Field types and validation (email, IBAN, phone …), auto-numbering, conditions, the field picker"),
            component("mt-audit", 1, "live", "Every change is recorded in a hash chain you can verify"),
            component("mt-cluster", 1, "engine", "Cluster membership, placement and fencing — used once the server runs as a cluster (wave 4)"),
            component("mt-refdata", 2, "live", "Cost centres (a tree), ISO countries and currencies as governed reference data"),
            component("mt-query", 2, "live", "Search, filter, sort, group and totals of the grid"),
            component("mt-formula", 2, "live", "Formula, lookup and roll-up fields; recompute across tables; the formula editor"),
            component("mt-perm", 2, "live", "Who sees and changes what: scopes, limits, field protection, separation of duties, explain"),
            component("mt-lifecycle", 2, "live", "Request lifecycle, approvals routed by the access rules (limits, CFO step), my tasks, inbox; SLA clocks and business calendars"),
            component("mt-store", 3, "placeholder", "SQLite, MySQL and PostgreSQL storage (now: one JSON file)"),
            component("mt-ops", 3, "placeholder", "Operation pipeline, real-time collaboration, undo"),
            component("mt-identity", 4, "placeholder", "Sign-in, SSO, sessions (now: a user switcher)"),
            component("mt-api", 4, "placeholder", "Versioned Public and Admin APIs with OpenAPI"),
            component("mt-rules", 5, "placeholder", "Automations and workflows"),
            component("mt-notify", 5, "placeholder", "Notifications by email and chat (now: the in-app inbox)"),
            component("mt-metrics", 6, "placeholder", "Dashboards and metrics"),
            component("mt-ai", 6, "placeholder", "AI skills, agents and the assistant"),
        ]
    }))
}

async fn people(State(app): State<Shared>) -> Json<JsonValue> {
    let ws = app.ws.lock().await;
    let list: Vec<JsonValue> = ws
        .data
        .people
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "name": p.name,
                "kind": p.kind,
                "title": p.title,
                "members": p.members.iter().map(|m| ws.name_of(*m)).collect::<Vec<_>>(),
            })
        })
        .collect();
    Json(json!(list))
}

// ------------------------------------------------------------------- tables

async fn tables(State(app): State<Shared>, headers: HeaderMap) -> Reply {
    let ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let now = app.clock.now();
    let list: Vec<JsonValue> = ws
        .data
        .tables
        .iter()
        .map(|t| {
            let readable = ws.readable(actor, t, now).len();
            let can_read = ws
                .decide(actor, &Action::RecordRead.into(), t.resource(), None, &[], now)
                .allowed;
            json!({"id": t.id, "base": t.base, "name": t.name, "description": t.description, "records": readable, "canRead": can_read})
        })
        .collect();
    Ok(Json(json!(list)))
}

fn hidden_reason(
    ws: &Workspace,
    actor: Actor,
    table: &Table,
    field: &FieldDef,
    now: Timestamp,
) -> String {
    let allowed = |capability: Capability| {
        ws.decide(actor, &capability, table.resource(), None, &[], now)
            .allowed
    };
    let read = ws.decide(
        actor,
        &Action::RecordRead.into(),
        table.resource(),
        None,
        &[],
        now,
    );
    if !read.allowed {
        return "you cannot read this table".to_owned();
    }
    if !read.fields.allows(field.id) {
        return "not included in your grants".to_owned();
    }
    let needed = match field.classification {
        Classification::Confidential => Some(Action::DataConfidential),
        Classification::Personal => Some(Action::DataPersonal),
        Classification::SensitivePersonal => Some(Action::DataSensitive),
        Classification::Public | Classification::Internal => None,
    };
    if let Some(action) = needed.filter(|a| !allowed((*a).into())) {
        return format!("{} data — needs `{}`", field.classification, action.key());
    }
    if let Some(group) = table.group_of(field.id) {
        if let Some(key) = &group.view_capability {
            return format!("in “{}” — needs `{key}`", group.name);
        }
    }
    "no access".to_owned()
}

fn table_json(ws: &Workspace, actor: Actor, table: &Table, now: Timestamp) -> JsonValue {
    let permissions = ws.field_permissions(actor, table, None, now);
    let mut columns = Vec::new();
    let mut hidden = Vec::new();
    for field in &table.fields {
        let permission = permissions.get(&field.id).copied();
        if permission.is_some_and(|p| p.visibility != Visibility::Hidden) {
            columns.push(view::field(ws, table, field, permission));
        } else {
            hidden.push(json!({"id": field.id, "name": field.name, "reason": hidden_reason(ws, actor, table, field, now)}));
        }
    }
    let allowed = |action: Action| {
        ws.decide(actor, &action.into(), table.resource(), None, &[], now)
            .allowed
    };
    json!({
        "id": table.id,
        "base": table.base,
        "name": table.name,
        "description": table.description,
        "primary": table.primary,
        "columns": columns,
        "hidden": hidden,
        "canRead": allowed(Action::RecordRead),
        "canCreate": allowed(Action::RecordCreate),
        "canManageSchema": allowed(Action::SchemaManage),
    })
}

async fn table(State(app): State<Shared>, headers: HeaderMap, Path(table): Path<String>) -> Reply {
    let ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let table = ws.table(parse(&table, "table")?)?;
    Ok(Json(table_json(&ws, actor, table, app.clock.now())))
}

// ------------------------------------------------------------------- query

#[derive(Debug, Deserialize)]
struct FilterSpec {
    field: FieldId,
    op: String,
    #[serde(default)]
    value: Option<CellInput>,
    #[serde(default)]
    me: bool,
}

#[derive(Debug, Deserialize)]
struct SortSpec {
    field: FieldId,
    #[serde(default)]
    desc: bool,
}

#[derive(Debug, Default, Deserialize)]
struct QuerySpec {
    #[serde(default)]
    search: String,
    #[serde(default)]
    filters: Vec<FilterSpec>,
    #[serde(default)]
    sort: Vec<SortSpec>,
    #[serde(default)]
    group: Option<FieldId>,
}

/// A record as the query evaluator sees it.
struct Row<'a>(&'a crate::model::Record);

impl mt_query::Row for Row<'_> {
    fn id(&self) -> RecordId {
        self.0.id
    }

    fn value(&self, field: FieldId) -> Option<&Value> {
        self.0.cells.get(&field)
    }
}

/// What the query evaluator asks about the table.
struct QueryEnv<'a> {
    ws: &'a Workspace,
    table: &'a Table,
}

impl mt_query::Env for QueryEnv<'_> {
    fn data_type(&self, field: FieldId) -> Option<DataType> {
        self.table.field(field).map(|f| f.data_type)
    }

    fn order_value(&self, field: FieldId, value: &Value) -> Option<Value> {
        let def = self.table.field(field)?;
        match value {
            Value::Reference(item) => {
                // Inline options by their position, reference items by their place in the tree.
                if let Some(options) = def.inline_options() {
                    let index = options.iter().position(|o| o.id == *item)?;
                    return Some(Value::Number(u64::try_from(index).ok()?.into()));
                }
                let set = self.ws.refdata().get(Workspace::refset_of(def)?)?;
                let index = set.walk().iter().position(|i| i == item)?;
                Some(Value::Number(u64::try_from(index).ok()?.into()))
            }
            Value::Principal(_) | Value::Link(_) | Value::MultiReference(_) => {
                Some(Value::Text(view::display(self.ws, def, value)))
            }
            Value::Null
            | Value::Text(_)
            | Value::LongText(_)
            | Value::Number(_)
            | Value::Money(_)
            | Value::Percent(_)
            | Value::Bool(_)
            | Value::Date(_)
            | Value::DateTime(_)
            | Value::Duration(_)
            | Value::Attachment(_)
            | Value::Json(_) => None,
        }
    }

    fn display_text(&self, field: FieldId, value: &Value) -> Option<String> {
        let def = self.table.field(field)?;
        Some(view::display(self.ws, def, value))
    }
}

fn condition(ws: &Workspace, table: &Table, spec: &FilterSpec) -> ApiResult<Condition> {
    let field = table
        .field(spec.field)
        .ok_or_else(|| ApiError::not_found(format!("field {}", spec.field)))?;
    let op: Operator = spec
        .op
        .parse()
        .map_err(|_| ApiError::bad_request(format!("`{}` is not an operator", spec.op)))?;
    let operand = if spec.me {
        Operand::Me
    } else {
        match &spec.value {
            None => Operand::None,
            Some(CellInput::Value { value }) => Operand::Value(value.clone()),
            Some(CellInput::Text { text }) => {
                let ctx = mt_core::coerce::ParseContext {
                    default_currency: mt_core::money::Currency::new("USD").ok(),
                };
                let value =
                    mt_core::coerce::parse_input(field.data_type, text, &ctx).or_else(|_| {
                        // A text operator on a field that is not text compares the display.
                        Ok::<Value, ApiError>(Value::Text(text.clone()))
                    })?;
                Operand::Value(value)
            }
        }
    };
    let _ = ws;
    Ok(Condition::test(field.id, op, operand))
}

async fn query(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(table): Path<String>,
    body: Option<Json<QuerySpec>>,
) -> Reply {
    let spec = body.map(|Json(spec)| spec).unwrap_or_default();
    let ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let now = app.clock.now();
    let table = ws.table(parse(&table, "table")?)?;
    ws.require(
        actor,
        &Action::RecordRead.into(),
        table.resource(),
        None,
        now,
    )?;
    let permissions = ws.field_permissions(actor, table, None, now);
    let visible = |field: FieldId| {
        permissions
            .get(&field)
            .is_some_and(|p| p.visibility == Visibility::Visible)
    };
    // Nobody filters, sorts or groups by what they cannot see.
    let used = spec
        .filters
        .iter()
        .map(|f| f.field)
        .chain(spec.sort.iter().map(|s| s.field))
        .chain(spec.group);
    for field in used {
        if !visible(field) {
            return Err(ApiError::forbidden(
                "you cannot filter, sort or group by a field you cannot see",
                json!({"field": field}),
            ));
        }
    }
    let mut parts = Vec::new();
    for filter in &spec.filters {
        parts.push(condition(&ws, table, filter)?);
    }
    let searchable: Vec<FieldId> = table
        .fields
        .iter()
        .map(|f| f.id)
        .filter(|f| visible(*f))
        .collect();
    // Totals of amounts and of plain quantities (not of ratings or computed day counts).
    let numeric: Vec<FieldId> = table
        .fields
        .iter()
        .filter(|f| {
            let summable = match f.data_type {
                DataType::Money => true,
                DataType::Number => {
                    f.computed.is_none() && f.widget != mt_core::field::Widget::Rating
                }
                DataType::Text
                | DataType::LongText
                | DataType::Percent
                | DataType::Bool
                | DataType::Date
                | DataType::DateTime
                | DataType::Duration
                | DataType::Reference
                | DataType::MultiReference
                | DataType::Link
                | DataType::Principal
                | DataType::Attachment
                | DataType::Json => false,
            };
            summable && visible(f.id)
        })
        .map(|f| f.id)
        .collect();
    let q = mt_query::Query {
        filter: (!parts.is_empty()).then_some(Condition::All(parts)),
        search: (!spec.search.trim().is_empty()).then(|| mt_query::Search {
            text: spec.search.clone(),
            fields: searchable,
        }),
        group: spec.group.map(GroupKey::by).into_iter().collect(),
        sort: spec
            .sort
            .iter()
            .map(|s| {
                if s.desc {
                    SortKey::desc(s.field)
                } else {
                    SortKey::asc(s.field)
                }
            })
            .collect(),
        aggregates: numeric
            .iter()
            .map(|field| Aggregate {
                field: *field,
                function: AggregateFn::Sum,
            })
            .collect(),
    };
    let types: BTreeMap<FieldId, DataType> =
        table.fields.iter().map(|f| (f.id, f.data_type)).collect();
    q.check(&|f| types.get(&f).copied())
        .map_err(|e| ApiError::invalid(e.to_string(), JsonValue::Null))?;
    let readable = ws.readable(actor, table, now);
    let rows: Vec<Row<'_>> = readable.iter().map(|r| Row(r)).collect();
    let mut ctx = context(now);
    ctx.me = Some(actor.on_behalf_of.unwrap_or(actor.principal));
    let env = QueryEnv { ws: &ws, table };
    let result = mt_query::run(&q, &rows, &ctx, &env);
    let totals = |values: &[mt_query::AggregateValue]| -> JsonValue {
        let mut out = serde_json::Map::new();
        for (field, value) in numeric.iter().zip(values) {
            let shown = match (&value.value, table.field(*field)) {
                (Some(v), Some(def)) => view::display(&ws, def, v),
                (None, _) | (Some(_), None) => match value.problem {
                    Some(mt_query::AggregateProblem::MixedCurrencies) => {
                        "mixed currencies".to_owned()
                    }
                    Some(mt_query::AggregateProblem::Overflow) => "too large".to_owned(),
                    None => String::new(),
                },
            };
            out.insert(field.to_string(), json!(shown));
        }
        JsonValue::Object(out)
    };
    let mut out_rows = Vec::new();
    for id in &result.rows {
        let Some(record) = table.record(*id) else {
            continue;
        };
        let per_record = ws.field_permissions(actor, table, Some(record), now);
        let mut cells = serde_json::Map::new();
        for field in &table.fields {
            if let Some(permission) = per_record.get(&field.id) {
                if permission.visibility != Visibility::Hidden {
                    cells.insert(
                        field.id.to_string(),
                        view::cell(&ws, field, record, *permission),
                    );
                }
            }
        }
        let can = |action: Action| {
            let prior = ws.prior(actor, Some(record));
            ws.decide(
                actor,
                &action.into(),
                table.resource(),
                Some(record),
                &prior,
                now,
            )
            .allowed
        };
        out_rows.push(json!({
            "id": record.id,
            "title": ws.title(table.id, record.id),
            "cells": cells,
            "canEdit": can(Action::RecordUpdate),
            "canDelete": can(Action::RecordDelete),
        }));
    }
    let groups: Vec<JsonValue> = result
        .groups
        .iter()
        .map(|group| {
            let key = table
                .field(group.field)
                .map_or_else(String::new, |def| view::display(&ws, def, &group.key));
            json!({
                "key": if key.is_empty() { "(empty)".to_owned() } else { key },
                "count": group.count,
                "totals": totals(&group.aggregates),
                "rows": group.rows,
            })
        })
        .collect();
    Ok(Json(json!({
        "table": table_json(&ws, actor, table, now),
        "rows": out_rows,
        "count": result.rows.len(),
        "totals": totals(&result.totals),
        "groups": groups,
    })))
}

// ------------------------------------------------------------------- records

fn record_json(
    ws: &Workspace,
    actor: Actor,
    table: &Table,
    record_id: RecordId,
    now: Timestamp,
) -> ApiResult<JsonValue> {
    let record = table
        .record(record_id)
        .ok_or_else(|| ApiError::not_found(format!("record {record_id}")))?;
    ws.require(
        actor,
        &Action::RecordRead.into(),
        table.resource(),
        Some(record),
        now,
    )?;
    let permissions = ws.field_permissions(actor, table, Some(record), now);
    let frozen = crate::process::frozen(ws, table, record);
    let mut fields = Vec::new();
    for field in &table.fields {
        let permission = permissions.get(&field.id).copied();
        if let Some(p) = permission.filter(|p| p.visibility != Visibility::Hidden) {
            let mut described = view::field(ws, table, field, Some(p));
            if let JsonValue::Object(map) = &mut described {
                map.insert("cell".to_owned(), view::cell(ws, field, record, p));
                if let Some((state_field, list, state)) = &frozen {
                    if *state_field == field.id {
                        map.insert("editable".to_owned(), json!(false));
                        map.insert(
                            "lockedReason".to_owned(),
                            json!("changes through the buttons above"),
                        );
                    } else if list.contains(&field.id) {
                        map.insert("editable".to_owned(), json!(false));
                        map.insert(
                            "lockedReason".to_owned(),
                            json!(format!("locked while {state}")),
                        );
                    }
                }
            }
            fields.push(described);
        }
    }
    let prior = ws.prior(actor, Some(record));
    let check = |capability: Capability| {
        let decision = ws.decide(
            actor,
            &capability,
            table.resource(),
            Some(record),
            &prior,
            now,
        );
        view::explain(ws, &decision)
    };
    let mut actions = serde_json::Map::new();
    actions.insert("update".to_owned(), check(Action::RecordUpdate.into()));
    actions.insert("delete".to_owned(), check(Action::RecordDelete.into()));
    if let Some(key) = crate::process::approval_capabilities(ws, table.id).first() {
        if let Ok(approve) = Capability::custom(key) {
            actions.insert("approve".to_owned(), check(approve));
        }
    }
    Ok(json!({
        "id": record.id,
        "title": ws.title(table.id, record.id),
        "table": table.id,
        "fields": fields,
        "actions": actions,
        "createdBy": ws.name_of(record.created_by),
        "createdAt": record.created_at,
        "modifiedBy": ws.name_of(record.modified_by),
        "modifiedAt": record.modified_at,
        "version": record.version,
        "process": crate::process::record_json(ws, actor, table, record, now),
        "canShare": ws
            .decide(
                actor,
                &Action::ShareManage.into(),
                table.resource(),
                None,
                &[],
                now,
            )
            .allowed,
    }))
}

// ------------------------------------------------------------------- process

async fn transition(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path((table, record, key)): Path<(String, String, String)>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let now = app.clock.now();
    let table_id: TableId = parse(&table, "table")?;
    let record_id: RecordId = parse(&record, "record")?;
    crate::process::transition(&mut ws, actor, table_id, record_id, &key, now)?;
    app.save(&ws)?;
    let table = ws.table(table_id)?;
    Ok(Json(record_json(&ws, actor, table, record_id, now)?))
}

async fn act(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path((run, task)): Path<(u64, u32)>,
    Json(mut body): Json<JsonValue>,
) -> Reply {
    // `{"act": "submit", "values": {field: input}}` for a fill-in task; other acts as usual.
    let values: BTreeMap<FieldId, CellInput> =
        match body.as_object_mut().and_then(|o| o.remove("values")) {
            Some(values) => serde_json::from_value(values)
                .map_err(|e| ApiError::bad_request(format!("values: {e}")))?,
            None => BTreeMap::new(),
        };
    let act: mt_lifecycle::approval::Act =
        serde_json::from_value(body).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let now = app.clock.now();
    let (table_id, record_id) = crate::process::act(&mut ws, actor, run, task, act, &values, now)?;
    app.save(&ws)?;
    let table = ws.table(table_id)?;
    Ok(Json(record_json(&ws, actor, table, record_id, now)?))
}

async fn tick(State(app): State<Shared>, headers: HeaderMap) -> Reply {
    {
        let ws = app.ws.lock().await;
        let actor = actor(&ws, &headers)?;
        ws.require(
            actor,
            &Action::SettingsManage.into(),
            Resource::Tenant,
            None,
            app.clock.now(),
        )?;
    }
    Ok(Json(json!({"changed": app.tick().await?})))
}

async fn tasks(State(app): State<Shared>, headers: HeaderMap) -> Reply {
    let ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    Ok(Json(crate::process::tasks_json(&ws, actor)))
}

async fn approvals(State(app): State<Shared>, headers: HeaderMap) -> Reply {
    let ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    Ok(Json(crate::process::runs_json(&ws, actor, app.clock.now())))
}

async fn inbox(State(app): State<Shared>, headers: HeaderMap) -> Reply {
    let ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    Ok(Json(crate::process::inbox_json(&ws, actor)))
}

async fn process_definitions(State(app): State<Shared>) -> Json<JsonValue> {
    let ws = app.ws.lock().await;
    Json(crate::process::definitions_json(&ws))
}

async fn record(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path((table, record)): Path<(String, String)>,
) -> Reply {
    let ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let table = ws.table(parse(&table, "table")?)?;
    Ok(Json(record_json(
        &ws,
        actor,
        table,
        parse(&record, "record")?,
        app.clock.now(),
    )?))
}

#[derive(Debug, Deserialize)]
struct Cells {
    cells: BTreeMap<FieldId, CellInput>,
}

async fn create(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(table): Path<String>,
    Json(body): Json<Cells>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let now = app.clock.now();
    let table_id: TableId = parse(&table, "table")?;
    let id = ws.create_record(actor, table_id, &body.cells, now)?;
    app.save(&ws)?;
    let table = ws.table(table_id)?;
    Ok(Json(record_json(&ws, actor, table, id, now)?))
}

async fn update(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path((table, record)): Path<(String, String)>,
    Json(body): Json<Cells>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let now = app.clock.now();
    let table_id: TableId = parse(&table, "table")?;
    let record_id: RecordId = parse(&record, "record")?;
    ws.update_record(actor, table_id, record_id, &body.cells, now)?;
    app.save(&ws)?;
    let table = ws.table(table_id)?;
    Ok(Json(record_json(&ws, actor, table, record_id, now)?))
}

async fn delete(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path((table, record)): Path<(String, String)>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    ws.delete_record(
        actor,
        parse(&table, "table")?,
        parse(&record, "record")?,
        app.clock.now(),
    )?;
    app.save(&ws)?;
    Ok(Json(json!({"deleted": true})))
}

async fn options(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path((table, field)): Path<(String, String)>,
) -> Reply {
    let ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let now = app.clock.now();
    let table = ws.table(parse(&table, "table")?)?;
    let field = table
        .field(parse(&field, "field")?)
        .ok_or_else(|| ApiError::not_found("field"))?;
    let list: Vec<JsonValue> = match &field.options {
        OptionsSource::Inline { options } => options
            .iter()
            .filter(|o| o.enabled)
            .map(|o| json!({"value": {"type": "reference", "value": o.id}, "label": o.label, "color": o.color}))
            .collect(),
        OptionsSource::RefSet { .. } => {
            let scope = field.options.ref_scope().ok_or_else(|| ApiError::internal("no scope"))?;
            let set = ws
                .refdata()
                .get(scope.set)
                .ok_or_else(|| ApiError::not_found("reference set"))?;
            set.options(&scope, now.date())
                .into_iter()
                .map(|id| {
                    json!({
                        "value": {"type": "reference", "value": id},
                        "label": set.label(id, "en"),
                        "path": set.path_label(id, "en", " › "),
                        "depth": set.depth(id),
                    })
                })
                .collect()
        }
        OptionsSource::Table { table: target } => {
            let target = ws.table(*target)?;
            ws.readable(actor, target, now)
                .into_iter()
                .map(|r| json!({"value": r.id, "label": ws.title(target.id, r.id)}))
                .collect()
        }
        OptionsSource::None | OptionsSource::Parameter { .. } => match field.data_type {
            DataType::Principal => ws
                .data
                .people
                .iter()
                .filter(|p| p.kind != PrincipalKind::Group)
                .map(|p| json!({"value": {"type": "principal", "value": [p.id]}, "label": p.name}))
                .collect(),
            DataType::Text
            | DataType::LongText
            | DataType::Number
            | DataType::Money
            | DataType::Percent
            | DataType::Bool
            | DataType::Date
            | DataType::DateTime
            | DataType::Duration
            | DataType::Reference
            | DataType::MultiReference
            | DataType::Link
            | DataType::Attachment
            | DataType::Json => Vec::new(),
        },
    };
    Ok(Json(json!(list)))
}

// ------------------------------------------------------------------- schema

#[derive(Debug, Deserialize)]
struct FormulaSpec {
    text: String,
}

async fn check_formula(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(table): Path<String>,
    Json(spec): Json<FormulaSpec>,
) -> Reply {
    let ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let now = app.clock.now();
    let table = ws.table(parse(&table, "table")?)?;
    let schema = crate::formula::TableSchema { ws: &ws, table };
    let expr = match mt_formula::parse(&spec.text, &schema) {
        Ok(expr) => expr,
        Err(error) => {
            let position = match &error {
                mt_formula::FormulaError::Syntax { position, .. }
                | mt_formula::FormulaError::UnknownField { position, .. }
                | mt_formula::FormulaError::UnknownFunction { position, .. } => Some(*position),
                mt_formula::FormulaError::Arity { .. }
                | mt_formula::FormulaError::Type(_)
                | mt_formula::FormulaError::Cycle { .. }
                | mt_formula::FormulaError::Limit(_)
                | mt_formula::FormulaError::Stored { .. } => None,
            };
            return Ok(Json(
                json!({"ok": false, "error": error.to_string(), "position": position}),
            ));
        }
    };
    let checked = mt_formula::check(&expr, &schema);
    let analysis = mt_formula::analyse(&expr);
    let warnings: Vec<String> = analysis
        .warnings
        .iter()
        .map(|w| match w {
            mt_formula::Warning::Volatile => "Uses TODAY() or NOW(): recomputed daily".to_owned(),
            mt_formula::Warning::VolatileOverLinks => {
                "Uses the date and linked records: every record is re-read daily".to_owned()
            }
            mt_formula::Warning::ManyLinkedLookups { count } => {
                format!("Reads linked records {count} times")
            }
            mt_formula::Warning::NestedListCondition => {
                "A list inside a list condition is read once per item".to_owned()
            }
        })
        .collect();
    let result_type = match &checked {
        Ok(t) => t.clone(),
        Err(error) => {
            return Ok(Json(
                json!({"ok": false, "error": error.to_string(), "position": null}),
            ));
        }
    };
    let data_type = formula_type(&result_type);
    let preview: Vec<JsonValue> = ws
        .readable(actor, table, now)
        .into_iter()
        .take(3)
        .map(|record| {
            let row = crate::formula::RecordCtx {
                ws: &ws,
                table,
                record,
            };
            let env = crate::formula::FormulaEnv {
                ws: &ws,
                now,
                base: Some(table.base),
            };
            let value = mt_formula::evaluate_to(&expr, &schema, &row, &env, None, data_type);
            let mut def = FieldDef::new(FieldId::from_u128(0), "preview", data_type);
            def.data_type = data_type;
            json!({
                "record": ws.title(table.id, record.id),
                "value": match value {
                    Ok(v) => view::display(&ws, &def, &v),
                    Err(e) => e.code().to_owned(),
                },
            })
        })
        .collect();
    Ok(Json(json!({
        "ok": true,
        "type": data_type.as_str(),
        "normalized": expr.to_text(&schema),
        "warnings": warnings,
        "reads": analysis.dependencies.fields.iter().filter_map(|f| table.field(*f)).map(|f| f.name.clone()).collect::<Vec<_>>(),
        "preview": preview,
    })))
}

/// The data type a computed field with this result gets.
fn formula_type(found: &Type) -> DataType {
    match found {
        Type::List(element) if **element == Type::Reference => DataType::MultiReference,
        Type::Number
        | Type::Text
        | Type::Bool
        | Type::Date
        | Type::DateTime
        | Type::Duration
        | Type::Money
        | Type::Reference
        | Type::List(_)
        | Type::Any => found.data_type().unwrap_or(DataType::Text),
    }
}

#[derive(Debug, Deserialize)]
struct FieldSpec {
    #[serde(rename = "type")]
    kind: String,
    name: String,
    #[serde(default)]
    options: Vec<String>,
    #[serde(default)]
    target: Option<TableId>,
    #[serde(default)]
    refset: Option<RefSetId>,
    #[serde(default)]
    formula: Option<String>,
    #[serde(default)]
    pattern: Option<String>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    help: Option<String>,
    /// Person fields: `requester`, `owner`, `assignee`.
    #[serde(default)]
    role: Option<String>,
    /// Money fields: the currency (ISO code).
    #[serde(default)]
    currency: Option<String>,
    /// Links: at most one linked record.
    #[serde(default)]
    single: bool,
    /// Links: the name of the mirror field in the target table (two-way link).
    #[serde(default, rename = "inverseName")]
    inverse_name: Option<String>,
    /// Links: the mirror holds at most one record.
    #[serde(default, rename = "inverseSingle")]
    inverse_single: bool,
}

fn not_yet(builtin: BuiltinType) -> Option<&'static str> {
    match builtin {
        BuiltinType::AiField => Some("AI fields arrive with the AI runtime (wave 6)"),
        BuiltinType::Attachment | BuiltinType::Signature => {
            Some("files need object storage, which arrives with the storage layer (wave 3)")
        }
        BuiltinType::Location | BuiltinType::Address => {
            Some("the map and address editors arrive with the web grid (wave 7)")
        }
        BuiltinType::RecordSource | BuiltinType::TraceLink => {
            Some("sources and traces arrive with the operation pipeline (wave 3)")
        }
        BuiltinType::Text
        | BuiltinType::LongText
        | BuiltinType::Number
        | BuiltinType::Currency
        | BuiltinType::Percent
        | BuiltinType::Date
        | BuiltinType::DateTime
        | BuiltinType::Duration
        | BuiltinType::Checkbox
        | BuiltinType::SingleSelect
        | BuiltinType::MultiSelect
        | BuiltinType::Url
        | BuiltinType::Person
        | BuiltinType::Party
        | BuiltinType::OrgUnit
        | BuiltinType::Phone
        | BuiltinType::Email
        | BuiltinType::Barcode
        | BuiltinType::Rating
        | BuiltinType::Progress
        | BuiltinType::AutoNumber
        | BuiltinType::TaxId
        | BuiltinType::CodeList
        | BuiltinType::CodeListMulti
        | BuiltinType::Cascade
        | BuiltinType::CatalogueItem
        | BuiltinType::Link
        | BuiltinType::Lookup
        | BuiltinType::Rollup
        | BuiltinType::Formula
        | BuiltinType::LineItems
        | BuiltinType::LifecycleState
        | BuiltinType::Assignment
        | BuiltinType::DueDate
        | BuiltinType::CreatedAt
        | BuiltinType::CreatedBy
        | BuiltinType::ModifiedAt
        | BuiltinType::ModifiedBy
        | BuiltinType::RecordId
        | BuiltinType::RecordVersion => None,
    }
}

#[derive(Debug, Deserialize)]
struct TypeQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    sample: String,
}

async fn field_types(Query(params): Query<TypeQuery>) -> Json<JsonValue> {
    let describe = |t: BuiltinType| {
        json!({
            "key": t.as_str(),
            "label": t.label(),
            "group": t.group().as_str(),
            "synonyms": t.synonyms(),
            "requirements": t.requirements().iter().map(|r| r.as_str()).collect::<Vec<_>>(),
            "unavailable": not_yet(t),
        })
    };
    let groups: Vec<JsonValue> = PickerGroup::ALL
        .iter()
        .map(|g| json!({"key": g.as_str(), "types": g.types().into_iter().map(describe).collect::<Vec<_>>()}))
        .collect();
    let found: Vec<&str> = if params.q.trim().is_empty() {
        Vec::new()
    } else {
        search_types(&params.q)
            .into_iter()
            .map(BuiltinType::as_str)
            .collect()
    };
    let suggestion = (!params.sample.trim().is_empty()).then(|| suggest(&params.sample).as_str());
    Json(json!({"groups": groups, "search": found, "suggestion": suggestion}))
}

async fn add_field(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(table): Path<String>,
    Json(spec): Json<FieldSpec>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let now = app.clock.now();
    let table_id: TableId = parse(&table, "table")?;
    let builtin: BuiltinType = spec
        .kind
        .parse()
        .map_err(|_| ApiError::bad_request(format!("`{}` is not a field type", spec.kind)))?;
    if let Some(reason) = not_yet(builtin) {
        return Err(ApiError::not_yet(format!(
            "{} fields are not available yet: {reason}",
            builtin.label()
        )));
    }
    let id = ws.new_field_id(now)?;
    let mut field = builtin.template(id, spec.name.trim());
    for requirement in builtin.requirements() {
        match requirement {
            Requirement::Options => {
                let options: Vec<mt_core::field::InlineOption> = spec
                    .options
                    .iter()
                    .filter(|label| !label.trim().is_empty())
                    .enumerate()
                    .map(|(index, label)| mt_core::field::InlineOption {
                        id: mt_core::id::RefItemId::from_u128(
                            u128::from(id.timestamp_ms())
                                .saturating_mul(1000)
                                .saturating_add(u128::try_from(index).unwrap_or(0)),
                        ),
                        label: label.trim().to_owned(),
                        color: None,
                        enabled: true,
                    })
                    .collect();
                if options.is_empty() {
                    return Err(ApiError::invalid(
                        "give at least one option",
                        JsonValue::Null,
                    ));
                }
                field.options = OptionsSource::Inline { options };
            }
            Requirement::RefSet => {
                let set = spec
                    .refset
                    .ok_or_else(|| ApiError::invalid("choose a reference set", JsonValue::Null))?;
                ws.refdata()
                    .get(set)
                    .ok_or_else(|| ApiError::not_found("reference set"))?;
                field.options = OptionsSource::of_set(set);
            }
            Requirement::LinkTarget => {
                let target = spec.target.ok_or_else(|| {
                    ApiError::invalid("choose the table to link to", JsonValue::Null)
                })?;
                ws.table(target)?;
                field.options = OptionsSource::Table { table: target };
            }
            Requirement::Formula => {
                let text = spec.formula.clone().unwrap_or_default();
                let table = ws.table(table_id)?;
                let schema = crate::formula::TableSchema { ws: &ws, table };
                let expr = mt_formula::parse(&text, &schema)
                    .map_err(|e| ApiError::invalid(e.to_string(), JsonValue::Null))?;
                let found = mt_formula::check(&expr, &schema)
                    .map_err(|e| ApiError::invalid(e.to_string(), JsonValue::Null))?;
                let data_type = formula_type(&found);
                field.data_type = data_type;
                field.widget = mt_core::field::Widget::default_for(data_type);
                let kind = builtin
                    .computed_kind()
                    .unwrap_or(mt_core::field::ComputedKind::Formula);
                mt_formula::field::attach(&mut field, kind, &expr);
            }
            Requirement::Numbering => {
                let pattern = spec.pattern.clone().unwrap_or_else(|| "{seq:4}".to_owned());
                field.format.numbering = Some(
                    mt_core::numbering::NumberingPattern::new(&pattern)
                        .map_err(|e| ApiError::invalid(e.to_string(), JsonValue::Null))?,
                );
            }
            Requirement::Format => {
                let key = spec
                    .format
                    .clone()
                    .unwrap_or_else(|| "tax.de_vat".to_owned());
                field.validation.format = Some(key.parse().map_err(|_| {
                    ApiError::invalid(format!("`{key}` is not a format"), JsonValue::Null)
                })?);
            }
            Requirement::Skill => {
                return Err(ApiError::not_yet(
                    "AI fields arrive with the AI runtime (wave 6)",
                ));
            }
        }
    }
    field.validation.required = spec.required && !field.is_generated();
    field.help_text = spec
        .help
        .as_deref()
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .map(str::to_owned);
    if field.data_type == DataType::Principal {
        if let Some(role) = &spec.role {
            field.role = crate::builder::person_role(role)?;
        }
    }
    if field.data_type == DataType::Money {
        if let Some(code) = spec.currency.as_deref().filter(|c| !c.is_empty()) {
            let currency = mt_core::money::Currency::new(code)
                .map_err(|e| ApiError::invalid(e.to_string(), JsonValue::Null))?;
            field.validation.currencies = vec![currency];
        }
    }
    let id = match field.options {
        OptionsSource::Table { table: target } => {
            if spec.single {
                field.validation.max_items = Some(1);
            }
            let inverse = spec
                .inverse_name
                .as_deref()
                .filter(|n| !n.trim().is_empty())
                .map(|n| (n, spec.inverse_single));
            crate::builder::add_link(&mut ws, actor, table_id, field, target, inverse, now)?
        }
        OptionsSource::None
        | OptionsSource::Inline { .. }
        | OptionsSource::RefSet { .. }
        | OptionsSource::Parameter { .. } => ws.add_field(actor, table_id, field, now)?,
    };
    app.save(&ws)?;
    let table = ws.table(table_id)?;
    Ok(Json(
        json!({"field": id, "table": table_json(&ws, actor, table, now)}),
    ))
}

// ------------------------------------------------------------------- building

#[derive(Debug, Deserialize)]
struct NameBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

async fn bases(State(app): State<Shared>, headers: HeaderMap) -> Reply {
    let ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let now = app.clock.now();
    let list: Vec<JsonValue> = ws
        .data
        .bases
        .iter()
        .map(|b| {
            let tables: Vec<JsonValue> = ws
                .data
                .tables
                .iter()
                .filter(|t| t.base == b.id)
                .map(|t| {
                    let can_read = ws
                        .decide(actor, &Action::RecordRead.into(), t.resource(), None, &[], now)
                        .allowed;
                    json!({"id": t.id, "name": t.name, "records": ws.readable(actor, t, now).len(), "canRead": can_read})
                })
                .collect();
            let manage = ws
                .decide(actor, &Action::SchemaManage.into(), b.resource(), None, &[], now)
                .allowed;
            json!({"id": b.id, "name": b.name, "description": b.description, "tables": tables, "canManageSchema": manage})
        })
        .collect();
    let can_create = ws
        .decide(
            actor,
            &Action::SchemaManage.into(),
            Resource::Tenant,
            None,
            &[],
            now,
        )
        .allowed;
    Ok(Json(json!({"bases": list, "canCreate": can_create})))
}

async fn create_base(
    State(app): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<NameBody>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let id = crate::builder::create_base(
        &mut ws,
        actor,
        body.name.as_deref().unwrap_or_default(),
        body.description.as_deref().unwrap_or_default(),
        app.clock.now(),
    )?;
    app.save(&ws)?;
    Ok(Json(json!({"id": id})))
}

async fn update_base(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(base): Path<String>,
    Json(body): Json<NameBody>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    crate::builder::update_base(
        &mut ws,
        actor,
        parse(&base, "base")?,
        body.name.as_deref(),
        body.description.as_deref(),
        app.clock.now(),
    )?;
    app.save(&ws)?;
    Ok(Json(json!({"updated": true})))
}

async fn delete_base(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(base): Path<String>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    crate::builder::delete_base(&mut ws, actor, parse(&base, "base")?, app.clock.now())?;
    app.save(&ws)?;
    Ok(Json(json!({"deleted": true})))
}

#[derive(Debug, Deserialize)]
struct NewTable {
    base: mt_core::id::BaseId,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    primary: String,
}

async fn create_table(
    State(app): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<NewTable>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let now = app.clock.now();
    let id = crate::builder::create_table(
        &mut ws,
        actor,
        body.base,
        &body.name,
        &body.description,
        &body.primary,
        now,
    )?;
    app.save(&ws)?;
    let table = ws.table(id)?;
    Ok(Json(table_json(&ws, actor, table, now)))
}

async fn update_table(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(table): Path<String>,
    Json(body): Json<NameBody>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let now = app.clock.now();
    let id: TableId = parse(&table, "table")?;
    crate::builder::update_table(
        &mut ws,
        actor,
        id,
        body.name.as_deref(),
        body.description.as_deref(),
        now,
    )?;
    app.save(&ws)?;
    let table = ws.table(id)?;
    Ok(Json(table_json(&ws, actor, table, now)))
}

async fn delete_table(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(table): Path<String>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    crate::builder::delete_table(&mut ws, actor, parse(&table, "table")?, app.clock.now())?;
    app.save(&ws)?;
    Ok(Json(json!({"deleted": true})))
}

async fn update_field(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path((table, field)): Path<(String, String)>,
    Json(body): Json<crate::builder::FieldPatch>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let now = app.clock.now();
    let table_id: TableId = parse(&table, "table")?;
    crate::builder::update_field(
        &mut ws,
        actor,
        table_id,
        parse(&field, "field")?,
        &body,
        now,
    )?;
    app.save(&ws)?;
    let table = ws.table(table_id)?;
    Ok(Json(table_json(&ws, actor, table, now)))
}

async fn delete_field(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path((table, field)): Path<(String, String)>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let now = app.clock.now();
    let table_id: TableId = parse(&table, "table")?;
    crate::builder::delete_field(&mut ws, actor, table_id, parse(&field, "field")?, now)?;
    app.save(&ws)?;
    let table = ws.table(table_id)?;
    Ok(Json(table_json(&ws, actor, table, now)))
}

#[derive(Debug, Deserialize)]
struct NewRefSet {
    code: String,
    name: String,
    #[serde(default)]
    tree: bool,
}

async fn create_refset(
    State(app): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<NewRefSet>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let id = crate::builder::create_refset(
        &mut ws,
        actor,
        &body.code,
        &body.name,
        body.tree,
        app.clock.now(),
    )?;
    app.save(&ws)?;
    Ok(Json(json!({"id": id})))
}

#[derive(Debug, Deserialize)]
struct NewItem {
    code: String,
    name: String,
    #[serde(default)]
    parent: Option<mt_core::id::RefItemId>,
}

async fn add_item(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(set): Path<String>,
    Json(body): Json<NewItem>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let (id, outcome) = crate::builder::add_item(
        &mut ws,
        actor,
        parse(&set, "reference set")?,
        &body.code,
        &body.name,
        body.parent,
        app.clock.now(),
    )?;
    app.save(&ws)?;
    let mut answer = outcome.json();
    if let JsonValue::Object(map) = &mut answer {
        map.insert("id".to_owned(), json!(id));
    }
    Ok(Json(answer))
}

#[derive(Debug, Deserialize)]
struct ItemPatchBody {
    enabled: bool,
}

async fn update_item(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path((set, item)): Path<(String, String)>,
    Json(body): Json<ItemPatchBody>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let outcome = crate::builder::set_item_enabled(
        &mut ws,
        actor,
        parse(&set, "reference set")?,
        parse(&item, "item")?,
        body.enabled,
        app.clock.now(),
    )?;
    app.save(&ws)?;
    let mut answer = outcome.json();
    if let JsonValue::Object(map) = &mut answer {
        map.insert("updated".to_owned(), json!(true));
    }
    Ok(Json(answer))
}

#[derive(Debug, Deserialize)]
struct RefSetPatch {
    approval: bool,
}

async fn update_refset(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(set): Path<String>,
    Json(body): Json<RefSetPatch>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let outcome = crate::codelist::set_policy(
        &mut ws,
        actor,
        parse(&set, "reference set")?,
        body.approval,
        app.clock.now(),
    )?;
    app.save(&ws)?;
    Ok(Json(outcome.json()))
}

async fn decide_proposal(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path((set, proposal)): Path<(String, u64)>,
    Json(decision): Json<crate::codelist::Decision>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let version = crate::codelist::decide(
        &mut ws,
        actor,
        parse(&set, "reference set")?,
        proposal,
        &decision,
        app.clock.now(),
    )?;
    app.save(&ws)?;
    Ok(Json(json!({"decided": true, "version": version})))
}

#[derive(Debug, Deserialize)]
struct NewPerson {
    name: String,
    kind: PrincipalKind,
    #[serde(default)]
    title: String,
    #[serde(default)]
    members: Vec<PrincipalId>,
}

async fn create_person(
    State(app): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<NewPerson>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let id = crate::builder::create_person(
        &mut ws,
        actor,
        &body.name,
        body.kind,
        &body.title,
        &body.members,
        app.clock.now(),
    )?;
    app.save(&ws)?;
    Ok(Json(json!({"id": id})))
}

#[derive(Debug, Deserialize)]
struct PersonPatch {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    members: Option<Vec<PrincipalId>>,
}

async fn update_person(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(person): Path<String>,
    Json(body): Json<PersonPatch>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    crate::builder::update_person(
        &mut ws,
        actor,
        parse(&person, "principal")?,
        body.name.as_deref(),
        body.title.as_deref(),
        body.members.as_deref(),
        app.clock.now(),
    )?;
    app.save(&ws)?;
    Ok(Json(json!({"updated": true})))
}

async fn settings(State(app): State<Shared>) -> Json<JsonValue> {
    let ws = app.ws.lock().await;
    Json(json!(crate::settings::list(&ws)))
}

#[derive(Debug, Deserialize)]
struct SettingScope {
    /// A base, for its override; the whole workspace without.
    #[serde(default)]
    base: Option<mt_core::id::BaseId>,
}

async fn clear_setting(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Query(scope): Query<SettingScope>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    crate::settings::clear(&mut ws, actor, &key, scope.base, app.clock.now())?;
    app.save(&ws)?;
    Ok(Json(json!({"cleared": true})))
}

async fn explain_setting(
    State(app): State<Shared>,
    Path(key): Path<String>,
    Query(scope): Query<SettingScope>,
) -> Reply {
    let ws = app.ws.lock().await;
    Ok(Json(crate::settings::explain(&ws, &key, scope.base)?))
}

#[derive(Debug, Deserialize)]
struct SettingBody {
    #[serde(rename = "type")]
    kind: String,
    text: String,
    /// A base, to override the workspace value there.
    #[serde(default)]
    base: Option<mt_core::id::BaseId>,
}

async fn set_setting(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(body): Json<SettingBody>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let value = crate::builder::param_value(&body.kind, &body.text)?;
    crate::builder::set_param(&mut ws, actor, &key, value, body.base, app.clock.now())?;
    app.save(&ws)?;
    Ok(Json(json!({"updated": true})))
}

async fn process_design(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(table): Path<String>,
) -> Reply {
    let ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let now = app.clock.now();
    let table_id: TableId = parse(&table, "table")?;
    let table = ws.table(table_id)?;
    let fields: Vec<JsonValue> = table
        .fields
        .iter()
        .map(|f| {
            json!({
                "id": f.id,
                "name": f.name,
                "dataType": f.data_type.as_str(),
                "type": mt_core::fieldtype::BuiltinType::of(f).as_str(),
                "generated": f.is_generated(),
                "options": f.inline_options().map(|o| o.iter().filter(|x| x.enabled).map(|x| json!({"id": x.id, "label": x.label})).collect::<Vec<_>>()),
            })
        })
        .collect();
    let binding = crate::process::binding(&ws, table_id);
    let approval = binding
        .and_then(|b| {
            b.lifecycle
                .transitions
                .iter()
                .find_map(|t| t.approval.clone())
        })
        .and_then(|key| {
            ws.data
                .process
                .definitions
                .iter()
                .filter(|d| d.key == key)
                .max_by_key(|d| d.version)
                .map(|d| json!({"name": d.name, "version": d.version}))
        });
    let running = ws
        .data
        .process
        .runs
        .iter()
        .filter(|r| r.table == table_id && !r.approval.status.is_final())
        .count();
    let manage = ws
        .decide(
            actor,
            &Action::SchemaManage.into(),
            table.resource(),
            None,
            &[],
            now,
        )
        .allowed;
    Ok(Json(json!({
        "table": {"id": table.id, "name": table.name},
        "design": crate::design::design_of(&ws, table_id),
        "published": approval,
        "running": running,
        "fields": fields,
        "canDesign": manage,
    })))
}

async fn publish_process(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(table): Path<String>,
    Json(design): Json<crate::design::ProcessDesign>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let table_id: TableId = parse(&table, "table")?;
    crate::design::publish(&mut ws, actor, table_id, design, app.clock.now())?;
    app.save(&ws)?;
    Ok(Json(
        json!({"design": crate::design::design_of(&ws, table_id)}),
    ))
}

async fn access_overview(State(app): State<Shared>, headers: HeaderMap) -> Reply {
    let ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let manage = ws
        .decide(
            actor,
            &Action::AccessManage.into(),
            Resource::Tenant,
            None,
            &[],
            app.clock.now(),
        )
        .allowed;
    let mut overview = crate::access::overview(&ws, app.clock.now(), manage);
    if let JsonValue::Object(map) = &mut overview {
        map.insert("canManage".to_owned(), json!(manage));
    }
    Ok(Json(overview))
}

async fn give_grant(
    State(app): State<Shared>,
    headers: HeaderMap,
    Json(spec): Json<crate::access::GrantSpec>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let id = crate::access::give(&mut ws, actor, &spec, app.clock.now())?;
    app.save(&ws)?;
    Ok(Json(json!({"id": id})))
}

async fn revoke_grant(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(grant): Path<String>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    crate::access::revoke(&mut ws, actor, parse(&grant, "grant")?, app.clock.now())?;
    app.save(&ws)?;
    Ok(Json(json!({"revoked": true})))
}

async fn open_review(
    State(app): State<Shared>,
    headers: HeaderMap,
    Json(spec): Json<crate::access::ReviewSpec>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let id = crate::access::open_review(&mut ws, actor, &spec, app.clock.now())?;
    app.save(&ws)?;
    Ok(Json(json!({"id": id})))
}

#[derive(Debug, Deserialize)]
struct ReviewDecision {
    grant: String,
    verdict: mt_perm::Verdict,
    #[serde(default)]
    note: Option<String>,
}

async fn decide_review(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(review): Path<String>,
    Json(body): Json<ReviewDecision>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    crate::access::decide_review(
        &mut ws,
        actor,
        parse(&review, "access review")?,
        (parse(&body.grant, "grant")?, body.verdict),
        body.note,
        app.clock.now(),
    )?;
    app.save(&ws)?;
    Ok(Json(json!({"decided": true})))
}

async fn complete_review(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(review): Path<String>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let revoked = crate::access::complete_review(
        &mut ws,
        actor,
        parse(&review, "access review")?,
        app.clock.now(),
    )?;
    app.save(&ws)?;
    Ok(Json(json!({"completed": true, "revoked": revoked})))
}

async fn cancel_review(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(review): Path<String>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    crate::access::cancel_review(
        &mut ws,
        actor,
        parse(&review, "access review")?,
        app.clock.now(),
    )?;
    app.save(&ws)?;
    Ok(Json(json!({"cancelled": true})))
}

/// A secret for a share link's address: 24 random bytes from the operating system.
fn share_token() -> ApiResult<String> {
    let mut bytes = [0_u8; 24];
    getrandom::fill(&mut bytes)
        .map_err(|e| ApiError::internal(format!("no randomness for the link: {e}")))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

async fn create_share(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(table): Path<String>,
    Json(spec): Json<crate::access::ShareSpec>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let token = share_token()?;
    let id = crate::access::create_share(
        &mut ws,
        actor,
        parse(&table, "table")?,
        &spec,
        token.clone(),
        app.clock.now(),
    )?;
    app.save(&ws)?;
    Ok(Json(json!({"id": id, "token": token})))
}

async fn revoke_share(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(share): Path<String>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    crate::access::revoke_share(
        &mut ws,
        actor,
        parse(&share, "share link")?,
        app.clock.now(),
    )?;
    app.save(&ws)?;
    Ok(Json(json!({"withdrawn": true})))
}

/// Opened by whoever holds the link: no identity is read.
async fn open_share(State(app): State<Shared>, Path(token): Path<String>) -> Reply {
    let ws = app.ws.lock().await;
    Ok(Json(crate::access::open_share(
        &ws,
        &token,
        app.clock.now(),
    )?))
}

#[derive(Debug, Deserialize)]
struct NewSet {
    name: String,
    capabilities: Vec<String>,
}

async fn create_set(
    State(app): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<NewSet>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let id = crate::access::create_set(
        &mut ws,
        actor,
        &body.name,
        &body.capabilities,
        app.clock.now(),
    )?;
    app.save(&ws)?;
    Ok(Json(json!({"id": id})))
}

#[derive(Debug, Deserialize)]
struct NewRule {
    name: String,
    first: String,
    second: String,
    kind: mt_perm::SodKind,
}

async fn add_rule(
    State(app): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<NewRule>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    let id = crate::access::add_rule(
        &mut ws,
        actor,
        &body.name,
        &body.first,
        &body.second,
        body.kind,
        app.clock.now(),
    )?;
    app.save(&ws)?;
    Ok(Json(json!({"id": id})))
}

async fn templates(State(app): State<Shared>) -> Json<JsonValue> {
    let ws = app.ws.lock().await;
    let list: Vec<JsonValue> = crate::seed::templates()
        .into_iter()
        .map(|(key, name, description)| {
            json!({"key": key, "name": name, "description": description, "installed": crate::builder::installed(&ws, key)})
        })
        .collect();
    Json(json!(list))
}

async fn install_template(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Reply {
    let mut ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    crate::builder::install_template(&mut ws, actor, &key, app.clock.now())?;
    app.save(&ws)?;
    Ok(Json(json!({"installed": key})))
}

// ------------------------------------------------------------------- reference data

async fn refsets(State(app): State<Shared>) -> Json<JsonValue> {
    let ws = app.ws.lock().await;
    let list: Vec<JsonValue> = ws
        .data
        .refsets
        .iter()
        .map(|state| {
            let count = ws
                .refdata()
                .get(state.set.id)
                .map_or(0, mt_refdata::EffectiveSet::len);
            json!({
                "id": state.set.id,
                "code": state.set.code,
                "name": state.set.name,
                "description": state.set.description,
                "tree": matches!(state.set.structure, mt_refdata::Structure::Tree { .. }),
                "system": matches!(state.set.scope, mt_refdata::SetScope::System),
                "items": count,
            })
        })
        .collect();
    Json(json!(list))
}

#[derive(Debug, Deserialize)]
struct SetQuery {
    #[serde(default)]
    q: String,
    /// Show the list as it was at this version.
    #[serde(default, rename = "asOf")]
    as_of: Option<u64>,
}

async fn refset(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(set): Path<String>,
    Query(params): Query<SetQuery>,
) -> Reply {
    let ws = app.ws.lock().await;
    let id: RefSetId = parse(&set, "reference set")?;
    let state = ws
        .data
        .refsets
        .iter()
        .find(|s| s.set.id == id)
        .ok_or_else(|| ApiError::not_found("reference set"))?;
    let now = app.clock.now();
    let items = match params.as_of {
        Some(version) => crate::codelist::items_as_of(&ws, id, version, &params.q, now)?,
        None => {
            let set = ws
                .refdata()
                .get(id)
                .ok_or_else(|| ApiError::not_found("reference set"))?;
            crate::codelist::items_json(set, &params.q, now)
        }
    };
    let manage = actor(&ws, &headers).is_ok_and(|actor| {
        ws.decide(
            actor,
            &Action::RefDataManage.into(),
            Resource::Tenant,
            None,
            &[],
            now,
        )
        .allowed
    });
    Ok(Json(json!({
        "id": state.set.id,
        "code": state.set.code,
        "name": state.set.name,
        "description": state.set.description,
        "tree": matches!(state.set.structure, mt_refdata::Structure::Tree { .. }),
        "system": matches!(state.set.scope, mt_refdata::SetScope::System),
        "items": items,
        "asOf": params.as_of,
        "canManage": manage,
        "history": crate::codelist::history_json(&ws, id)?,
    })))
}

// ------------------------------------------------------------------- access

async fn capabilities(State(app): State<Shared>) -> Json<JsonValue> {
    let ws = app.ws.lock().await;
    let keys = crate::access::catalogue(&ws);
    let rules: Vec<JsonValue> = ws
        .data
        .policy
        .rules()
        .map(|r| json!({"name": r.name, "first": r.first.key(), "second": r.second.key(), "kind": r.kind}))
        .collect();
    Json(json!({"capabilities": keys, "rules": rules}))
}

#[derive(Debug, Deserialize)]
struct ExplainQuery {
    table: TableId,
    #[serde(default)]
    record: Option<RecordId>,
    capability: String,
    #[serde(rename = "as")]
    who: PrincipalId,
    #[serde(default, rename = "for")]
    for_whom: Option<PrincipalId>,
}

async fn explain(State(app): State<Shared>, Query(q): Query<ExplainQuery>) -> Reply {
    let ws = app.ws.lock().await;
    let now = app.clock.now();
    let table = ws.table(q.table)?;
    let record = match q.record {
        Some(id) => Some(
            table
                .record(id)
                .ok_or_else(|| ApiError::not_found(format!("record {id}")))?,
        ),
        None => None,
    };
    let capability: Capability = q
        .capability
        .parse()
        .map_err(|e: mt_perm::PermError| ApiError::bad_request(e.to_string()))?;
    let actor = Actor {
        principal: q.who,
        on_behalf_of: q.for_whom.filter(|p| *p != q.who),
    };
    let prior = ws.prior(actor, record);
    let decision = ws.decide(actor, &capability, table.resource(), record, &prior, now);
    let mut out = view::explain(&ws, &decision);
    if let JsonValue::Object(map) = &mut out {
        map.insert(
            "question".to_owned(),
            json!({
                "who": ws.name_of(actor.principal),
                "for": actor.on_behalf_of.map(|p| ws.name_of(p)),
                "capability": capability.key(),
                "table": table.name,
                "record": record.map(|r| ws.title(table.id, r.id)),
            }),
        );
        let fields: Vec<String> = table
            .fields
            .iter()
            .filter(|f| decision.fields.allows(f.id))
            .map(|f| f.name.clone())
            .collect();
        map.insert("fields".to_owned(), json!(fields));
    }
    Ok(Json(out))
}

// ------------------------------------------------------------------- audit

fn require_audit(ws: &Workspace, actor: Actor, now: Timestamp) -> ApiResult<()> {
    ws.require(
        actor,
        &Action::AuditRead.into(),
        Resource::Tenant,
        None,
        now,
    )
}

async fn audit(State(app): State<Shared>, headers: HeaderMap) -> Reply {
    let ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    require_audit(&ws, actor, app.clock.now())?;
    let who = |id: &str| {
        id.parse::<PrincipalId>()
            .map_or_else(|_| id.to_owned(), |p| ws.name_of(p))
    };
    let entries: Vec<JsonValue> = ws
        .data
        .audit
        .iter()
        .rev()
        .map(|e| {
            json!({
                "seq": e.seq,
                "hash": e.hash.to_string(),
                "prev": e.prev_hash.to_string(),
                "actor": who(&e.actor.id),
                "initiator": who(&e.initiator.id),
                "action": e.action,
                "object": format!("{} {}", e.object.kind, e.object.id),
                "details": serde_json::from_str::<JsonValue>(&e.details).unwrap_or(JsonValue::Null),
                "at": Timestamp::from_millis(e.at_ms).map(|t| t.to_string()).unwrap_or_default(),
            })
        })
        .collect();
    Ok(Json(json!(entries)))
}

fn verdict(v: &mt_audit::ChainVerdict) -> JsonValue {
    match v {
        mt_audit::ChainVerdict::Intact { head, entries } => {
            json!({"intact": true, "entries": entries, "head": head.hash.to_string()})
        }
        mt_audit::ChainVerdict::Tampered(finding) => json!({
            "intact": false,
            "seq": finding.seq,
            "kind": format!("{:?}", finding.kind),
            "lastGood": finding.last_good.seq,
        }),
    }
}

async fn verify(State(app): State<Shared>, headers: HeaderMap) -> Reply {
    let ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    require_audit(&ws, actor, app.clock.now())?;
    Ok(Json(verdict(&ws.verify_audit(&ws.data.audit))))
}

#[derive(Debug, Deserialize)]
struct TamperSpec {
    seq: u64,
}

/// Shows what verification reports when an entry is altered — on a copy; the real log is
/// never changed.
async fn tamper_check(
    State(app): State<Shared>,
    headers: HeaderMap,
    Json(spec): Json<TamperSpec>,
) -> Reply {
    let ws = app.ws.lock().await;
    let actor = actor(&ws, &headers)?;
    require_audit(&ws, actor, app.clock.now())?;
    let mut copy = ws.data.audit.clone();
    let entry = copy
        .iter_mut()
        .find(|e| e.seq == spec.seq)
        .ok_or_else(|| ApiError::not_found(format!("audit entry {}", spec.seq)))?;
    entry.details = entry.details.replacen('}', ",\"tampered\":true}", 1);
    Ok(Json(verdict(&ws.verify_audit(&copy))))
}

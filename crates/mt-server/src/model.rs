//! The workspace: tables, records, reference data, people, the access policy and the audit
//! log, kept in memory and saved as one JSON document.
//!
//! This is a stand-in for the storage layer (`mt-store`, wave 3) and the operation pipeline
//! (`mt-ops`, wave 3). The *rules* are the real ones: every change is checked by `mt-perm`,
//! validated by `mt-core`, recomputed by `mt-formula`, and recorded in the `mt-audit` hash
//! chain. Formulas are recomputed in full after each change (the incremental planner,
//! `Graph::affected`, is used once the operation pipeline exists).

use crate::error::{ApiError, ApiResult};
use mt_audit::{AuditDraft, AuditEntry, AuditLog, InMemoryAuditStore, ObjectRef};
use mt_core::coerce::{parse_input, ParseContext};
use mt_core::condition::UtcContext;
use mt_core::field::{DataType, FieldDef, FieldGroup, FieldRole, OptionsSource};
use mt_core::id::{
    BaseId, FieldId, PrincipalId, RecordId, RefItemId, RefSetId, TableId, WorkspaceId,
};
use mt_core::money::Currency;
use mt_core::time::{Date, Timestamp, Weekday};
use mt_core::validate::{check_writable, validate_value};
use mt_core::value::Value;
use mt_formula::{ErrorValue, Graph};
use mt_perm::{
    Action, Actor, Capability, Decision, Directory, Facts, FieldMeta, FieldPermission, Policy,
    PrincipalKind, RecordFacts, Request, Resource,
};
use mt_refdata::{NoParameters, RefData};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

/// The workspace (one per installation until tenants arrive with `mt-identity`).
pub const WORKSPACE: WorkspaceId = WorkspaceId::from_u128(1);
/// The base of the purchasing sample (and of tables saved before bases existed).
pub const BASE: BaseId = BaseId::from_u128(1);

const fn default_base() -> BaseId {
    BASE
}

/// A base: a set of tables, with their processes and access, that belong together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Base {
    /// Stable id.
    pub id: BaseId,
    /// Display name.
    pub name: String,
    /// What the base is for.
    #[serde(default)]
    pub description: String,
}

impl Base {
    /// The resource the base is for the access policy.
    #[must_use]
    pub const fn resource(&self) -> Resource {
        Resource::Base {
            workspace: WORKSPACE,
            base: self.id,
        }
    }
}

/// A principal: a person, guest, agent or group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    /// Stable id.
    pub id: PrincipalId,
    /// Display name.
    pub name: String,
    /// Kind of principal.
    pub kind: PrincipalKind,
    /// What they do, for the user switcher.
    pub title: String,
    /// Members (groups only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<PrincipalId>,
}

/// One record of a table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// Stable id.
    pub id: RecordId,
    /// Cell values.
    pub cells: BTreeMap<FieldId, Value>,
    /// Errors of computed fields (`#DIV/0!` …) by field.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub errors: BTreeMap<FieldId, ErrorValue>,
    /// Who created it.
    pub created_by: PrincipalId,
    /// When.
    pub created_at: Timestamp,
    /// Who changed it last.
    pub modified_by: PrincipalId,
    /// When.
    pub modified_at: Timestamp,
    /// Number of committed changes.
    pub version: u64,
}

impl RecordFacts for Record {
    fn id(&self) -> RecordId {
        self.id
    }

    fn value(&self, field: FieldId) -> Option<&Value> {
        self.cells.get(&field)
    }
}

/// A table: its fields, field groups and records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    /// Stable id.
    pub id: TableId,
    /// The base it belongs to.
    #[serde(default = "default_base")]
    pub base: BaseId,
    /// Display name.
    pub name: String,
    /// What the table holds.
    pub description: String,
    /// The field that names a record.
    pub primary: FieldId,
    /// The fields, in display order.
    pub fields: Vec<FieldDef>,
    /// Field groups.
    #[serde(default)]
    pub groups: Vec<FieldGroup>,
    /// The records, in creation order.
    pub records: Vec<Record>,
}

impl Table {
    /// The resource the table is for the access policy.
    #[must_use]
    pub const fn resource(&self) -> Resource {
        Resource::Table {
            workspace: WORKSPACE,
            base: self.base,
            table: self.id,
        }
    }

    /// A field by id.
    #[must_use]
    pub fn field(&self, id: FieldId) -> Option<&FieldDef> {
        self.fields.iter().find(|field| field.id == id)
    }

    /// A record by id.
    #[must_use]
    pub fn record(&self, id: RecordId) -> Option<&Record> {
        self.records.iter().find(|record| record.id == id)
    }

    /// The group a field belongs to.
    #[must_use]
    pub fn group_of(&self, field: FieldId) -> Option<&FieldGroup> {
        self.groups
            .iter()
            .find(|group| group.fields.contains(&field))
    }
}

/// Two link fields that mirror each other (a request's lines, a line's request).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPair {
    /// One side's table.
    pub table: TableId,
    /// Its link field.
    pub field: FieldId,
    /// The other side's table.
    pub inverse_table: TableId,
    /// Its link field.
    pub inverse_field: FieldId,
}

/// Everything that is saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Data {
    /// Bases.
    #[serde(default)]
    pub bases: Vec<Base>,
    /// Tables.
    pub tables: Vec<Table>,
    /// Reference sets (definitions, items, overlays).
    pub refsets: Vec<mt_refdata::State>,
    /// People, guests, agents and groups.
    pub people: Vec<Person>,
    /// Grants, capability sets and separation-of-duties rules.
    pub policy: Policy,
    /// Parameters formulas can read with `PARAM`.
    pub params: BTreeMap<String, Value>,
    /// Numbering counters by field and period.
    pub counters: BTreeMap<String, u64>,
    /// The next id number.
    pub next_id: u64,
    /// The audit log.
    pub audit: Vec<AuditEntry>,
    /// Mirrored link fields.
    pub links: Vec<LinkPair>,
    /// Lifecycles, approvals and the inbox.
    #[serde(default)]
    pub process: crate::process::Process,
}

/// A cell as a client sends it: text to read as the field's type, or a structured value
/// (references, links, people).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CellInput {
    /// Text, read like typed input (`12.50`, `2026-05-20`, `19.99 USD`).
    Text {
        /// The text.
        text: String,
    },
    /// A value in its stored form.
    Value {
        /// The value.
        value: Value,
    },
}

/// The live workspace: the saved data and what is derived from it.
#[derive(Debug)]
pub struct Workspace {
    /// The saved data.
    pub data: Data,
    refdata: RefData,
    graph: Graph,
    log: AuditLog,
}

/// The chain id of the audit log.
pub const AUDIT_CHAIN: &str = "workspace";

fn internal(error: impl core::fmt::Display) -> ApiError {
    ApiError::internal(error.to_string())
}

/// UTC day and context for conditions.
#[must_use]
pub fn context(now: Timestamp) -> UtcContext {
    UtcContext::on(now.date())
}

impl Workspace {
    /// Loads a workspace, deriving reference data and the formula graph.
    ///
    /// # Errors
    /// When the saved data is inconsistent (a reference set or formula that does not load).
    pub fn new(data: Data) -> ApiResult<Workspace> {
        let log = AuditLog::new(AUDIT_CHAIN).map_err(internal)?;
        let mut workspace = Workspace {
            data,
            refdata: RefData::new(),
            graph: Graph::default(),
            log,
        };
        workspace.ensure_bases();
        workspace.rebuild_refdata()?;
        workspace.rebuild_graph()?;
        Ok(workspace)
    }

    /// Data saved before bases existed: its tables belong to the purchasing base.
    fn ensure_bases(&mut self) {
        let missing: BTreeSet<BaseId> = self
            .data
            .tables
            .iter()
            .map(|t| t.base)
            .filter(|id| !self.data.bases.iter().any(|b| b.id == *id))
            .collect();
        for id in missing {
            self.data.bases.push(Base {
                id,
                name: if id == BASE {
                    "Purchasing".to_owned()
                } else {
                    "Base".to_owned()
                },
                description: String::new(),
            });
        }
    }

    /// A base.
    ///
    /// # Errors
    /// 404 for an unknown base.
    pub fn base(&self, id: BaseId) -> ApiResult<&Base> {
        self.data
            .bases
            .iter()
            .find(|b| b.id == id)
            .ok_or_else(|| ApiError::not_found(format!("base {id}")))
    }

    /// Rebuilds the reference-data catalogue from the saved sets.
    pub(crate) fn rebuild_refdata(&mut self) -> ApiResult<()> {
        let mut catalogue = RefData::new();
        for state in &self.data.refsets {
            catalogue
                .insert(state.effective().map_err(internal)?)
                .map_err(internal)?;
        }
        self.refdata = catalogue;
        Ok(())
    }

    /// Rebuilds the formula graph; refuses circular definitions.
    pub(crate) fn rebuild_graph(&mut self) -> ApiResult<()> {
        let mut computed = Vec::new();
        for table in &self.data.tables {
            computed.extend(
                mt_formula::field::computed_fields(table.id, &table.fields)
                    .map_err(|e| ApiError::invalid(e.to_string(), json!(null)))?,
            );
        }
        let graph = Graph::build(
            computed,
            &|table, link| self.link_target(table, link),
            &|table, field| {
                self.table(table)
                    .ok()
                    .and_then(|t| t.field(field))
                    .map_or_else(|| field.to_string(), |f| f.name.clone())
            },
        )
        .map_err(|e| ApiError::invalid(e.to_string(), json!(null)))?;
        self.graph = graph;
        Ok(())
    }

    /// The reference-data catalogue.
    #[must_use]
    pub const fn refdata(&self) -> &RefData {
        &self.refdata
    }

    /// The formula graph.
    #[must_use]
    pub const fn graph(&self) -> &Graph {
        &self.graph
    }

    /// A table.
    ///
    /// # Errors
    /// 404 for an unknown table.
    pub fn table(&self, id: TableId) -> ApiResult<&Table> {
        self.data
            .tables
            .iter()
            .find(|table| table.id == id)
            .ok_or_else(|| ApiError::not_found(format!("table {id}")))
    }

    pub(crate) fn table_mut(&mut self, id: TableId) -> ApiResult<&mut Table> {
        self.data
            .tables
            .iter_mut()
            .find(|table| table.id == id)
            .ok_or_else(|| ApiError::not_found(format!("table {id}")))
    }

    /// The table a link field leads to.
    #[must_use]
    pub fn link_target(&self, table: TableId, field: FieldId) -> Option<TableId> {
        let field = self.table(table).ok()?.field(field)?;
        match field.options {
            OptionsSource::Table { table } => Some(table),
            OptionsSource::None
            | OptionsSource::Inline { .. }
            | OptionsSource::RefSet { .. }
            | OptionsSource::Parameter { .. } => None,
        }
    }

    /// A person by id.
    #[must_use]
    pub fn person(&self, id: PrincipalId) -> Option<&Person> {
        self.data.people.iter().find(|person| person.id == id)
    }

    /// The name of a principal (its id if unknown).
    #[must_use]
    pub fn name_of(&self, id: PrincipalId) -> String {
        self.person(id)
            .map_or_else(|| id.to_string(), |person| person.name.clone())
    }

    /// The label of a reference item: from the reference sets, or from an inline option of
    /// any field.
    #[must_use]
    pub fn label(&self, item: RefItemId) -> Option<String> {
        for state in &self.data.refsets {
            if let Some(set) = self.refdata.get(state.set.id) {
                if let Some(label) = set.label(item, "en") {
                    return Some(label.to_owned());
                }
            }
        }
        self.data
            .tables
            .iter()
            .flat_map(|table| table.fields.iter())
            .filter_map(|field| field.inline_options())
            .flatten()
            .find(|option| option.id == item)
            .map(|option| option.label.clone())
    }

    /// The next fresh id number, as the random part of a time-ordered id.
    fn fresh(&mut self, now: Timestamp) -> (u64, [u8; 10]) {
        self.data.next_id = self.data.next_id.saturating_add(1);
        let mut random = [0_u8; 10];
        for (slot, byte) in random
            .iter_mut()
            .rev()
            .zip(self.data.next_id.to_be_bytes().iter().rev())
        {
            *slot = *byte;
        }
        (u64::try_from(now.millis()).unwrap_or(0), random)
    }

    /// A new record id.
    ///
    /// # Errors
    /// When the clock is outside the id range.
    pub fn new_record_id(&mut self, now: Timestamp) -> ApiResult<RecordId> {
        let (ms, random) = self.fresh(now);
        RecordId::new(ms, random).map_err(internal)
    }

    /// A new field id.
    ///
    /// # Errors
    /// When the clock is outside the id range.
    pub fn new_field_id(&mut self, now: Timestamp) -> ApiResult<FieldId> {
        let (ms, random) = self.fresh(now);
        FieldId::new(ms, random).map_err(internal)
    }

    /// A new id of any kind (`TableId::new`, `BaseId::new` …).
    ///
    /// # Errors
    /// When the clock is outside the id range.
    pub fn new_id<T>(
        &mut self,
        now: Timestamp,
        make: impl Fn(u64, [u8; 10]) -> Result<T, mt_core::CoreError>,
    ) -> ApiResult<T> {
        let (ms, random) = self.fresh(now);
        make(ms, random).map_err(internal)
    }

    // ---------------------------------------------------------------- access

    /// Asks the access policy.
    #[must_use]
    pub fn decide(
        &self,
        actor: Actor,
        capability: &Capability,
        on: Resource,
        record: Option<&Record>,
        prior: &[Capability],
        now: Timestamp,
    ) -> Decision {
        let ctx = context(now);
        let env = mt_perm::Env {
            directory: self,
            facts: self,
            context: &ctx,
        };
        self.data.policy.decide(
            &Request {
                actor,
                capability,
                on,
                record: record.map(|r| r as &dyn RecordFacts),
                prior,
                now,
            },
            &env,
        )
    }

    /// Refuses with the explanation unless the policy allows.
    ///
    /// # Errors
    /// 403 with the decision as details.
    pub fn require(
        &self,
        actor: Actor,
        capability: &Capability,
        on: Resource,
        record: Option<&Record>,
        now: Timestamp,
    ) -> ApiResult<()> {
        let prior = self.prior(actor, record);
        let decision = self.decide(actor, capability, on, record, &prior, now);
        if decision.allowed {
            Ok(())
        } else {
            Err(ApiError::forbidden(
                format!(
                    "{} may not {capability} here",
                    self.name_of(actor.principal)
                ),
                crate::view::explain(self, &decision),
            ))
        }
    }

    /// What the actor's principals already did to the record (separation of duties).
    #[must_use]
    pub fn prior(&self, actor: Actor, record: Option<&Record>) -> Vec<Capability> {
        let created = record.is_some_and(|record| {
            record.created_by == actor.principal || Some(record.created_by) == actor.on_behalf_of
        });
        if created {
            vec![Action::RecordCreate.into()]
        } else {
            Vec::new()
        }
    }

    /// The engine's view of a table's fields.
    #[must_use]
    pub fn field_meta(&self, table: &Table) -> Vec<FieldMeta> {
        table
            .fields
            .iter()
            .map(|field| {
                let group = table.group_of(field.id);
                let capability =
                    |key: Option<&String>| key.and_then(|key| key.parse::<Capability>().ok());
                FieldMeta {
                    id: field.id,
                    classification: field.classification,
                    masked: field.format.mask.is_some(),
                    generated: field.is_generated(),
                    view_capability: capability(group.and_then(|g| g.view_capability.as_ref())),
                    edit_capability: capability(group.and_then(|g| g.edit_capability.as_ref())),
                }
            })
            .collect()
    }

    /// What the actor may do with each field of a record (or of the table).
    #[must_use]
    pub fn field_permissions(
        &self,
        actor: Actor,
        table: &Table,
        record: Option<&Record>,
        now: Timestamp,
    ) -> BTreeMap<FieldId, FieldPermission> {
        let ctx = context(now);
        let env = mt_perm::Env {
            directory: self,
            facts: self,
            context: &ctx,
        };
        self.data.policy.field_permissions(
            actor,
            table.resource(),
            record.map(|r| r as &dyn RecordFacts),
            now,
            &env,
            &self.field_meta(table),
        )
    }

    /// The records of a table the actor may read.
    #[must_use]
    pub fn readable<'t>(&self, actor: Actor, table: &'t Table, now: Timestamp) -> Vec<&'t Record> {
        let ctx = context(now);
        let env = mt_perm::Env {
            directory: self,
            facts: self,
            context: &ctx,
        };
        let read: Capability = Action::RecordRead.into();
        let filter = self
            .data
            .policy
            .row_filter(actor, &read, table.resource(), now, &env);
        if filter.admits_nothing() {
            return Vec::new();
        }
        table
            .records
            .iter()
            .filter(|record| filter.admits(*record, &env))
            .collect()
    }

    // ---------------------------------------------------------------- formulas

    /// Recomputes every computed and system field of every record, in dependency order.
    pub fn recompute(&mut self, now: Timestamp) {
        let order: Vec<(TableId, FieldId)> = self.graph.order().to_vec();
        for (table_id, field_id) in order {
            let Ok(table) = self.table(table_id) else {
                continue;
            };
            let (Some(field), Some(expr)) =
                (table.field(field_id), self.graph.expr(table_id, field_id))
            else {
                continue;
            };
            let schema = crate::formula::TableSchema { ws: self, table };
            let env = crate::formula::FormulaEnv { ws: self, now };
            let results: Vec<(RecordId, Result<Value, ErrorValue>)> = table
                .records
                .iter()
                .map(|record| {
                    let row = crate::formula::RecordCtx {
                        ws: self,
                        table,
                        record,
                    };
                    let result =
                        mt_formula::evaluate_to(expr, &schema, &row, &env, None, field.data_type);
                    (record.id, result)
                })
                .collect();
            let Ok(table) = self.table_mut(table_id) else {
                continue;
            };
            for (record_id, result) in results {
                if let Some(record) = table.records.iter_mut().find(|r| r.id == record_id) {
                    match result {
                        Ok(value) => {
                            record.errors.remove(&field_id);
                            set_cell(record, field_id, value);
                        }
                        Err(error) => {
                            record.cells.remove(&field_id);
                            record.errors.insert(field_id, error);
                        }
                    }
                }
            }
        }
        self.refresh_system_cells();
    }

    /// Fills the system fields (created at/by, modified at/by, id, version) from the
    /// records' metadata.
    fn refresh_system_cells(&mut self) {
        use mt_core::field::SystemField;
        for table in &mut self.data.tables {
            let system: Vec<(FieldId, SystemField)> = table
                .fields
                .iter()
                .filter_map(|field| field.system.map(|s| (field.id, s)))
                .collect();
            for record in &mut table.records {
                for (field, kind) in &system {
                    let value = match kind {
                        SystemField::CreatedAt => Value::DateTime(record.created_at),
                        SystemField::ModifiedAt => Value::DateTime(record.modified_at),
                        SystemField::CreatedBy => Value::Principal(vec![record.created_by]),
                        SystemField::ModifiedBy => Value::Principal(vec![record.modified_by]),
                        SystemField::RecordId => Value::Text(record.id.to_string()),
                        SystemField::Version => Value::Number(record.version.into()),
                        SystemField::Source => Value::Text("entered in the app".to_owned()),
                        SystemField::Trace => Value::Null,
                    };
                    set_cell(record, *field, value);
                }
            }
        }
    }

    // ---------------------------------------------------------------- changes

    /// Reads and validates the inputs for some fields of a record of `table`.
    fn read_inputs(
        &self,
        actor: Actor,
        table: &Table,
        record: Option<&Record>,
        inputs: &BTreeMap<FieldId, CellInput>,
        now: Timestamp,
    ) -> ApiResult<Vec<(FieldId, Value)>> {
        let permissions = self.field_permissions(actor, table, record, now);
        let resolver = self.refdata.resolver(now.date(), &NoParameters);
        let currency = Currency::new("USD").ok();
        let ctx = ParseContext {
            default_currency: currency,
        };
        let mut issues = Vec::new();
        let mut values = Vec::new();
        for (field_id, input) in inputs {
            let field = table
                .field(*field_id)
                .ok_or_else(|| ApiError::not_found(format!("field {field_id}")))?;
            if let Some(issue) = check_writable(field) {
                issues.push(json!({"field": field.id, "message": issue.message}));
                continue;
            }
            if !permissions.get(field_id).is_some_and(|p| p.editable) {
                issues.push(json!({
                    "field": field.id,
                    "message": format!("you may not change `{}`", field.name)
                }));
                continue;
            }
            let value = match input {
                CellInput::Text { text } => match parse_input(field.data_type, text, &ctx) {
                    Ok(value) => value,
                    Err(error) => {
                        issues.push(json!({"field": field.id, "message": error.to_string()}));
                        continue;
                    }
                },
                CellInput::Value { value } => value.clone().normalized(),
            };
            let previous = record.and_then(|r| r.cells.get(field_id));
            for issue in validate_value(field, &value, previous, &resolver) {
                issues.push(json!({"field": field.id, "message": issue.message}));
            }
            if let Value::Link(ids) = &value {
                if let Some(target) = self.link_target(table.id, field.id) {
                    let target = self.table(target)?;
                    if let Some(missing) = ids.iter().find(|id| target.record(**id).is_none()) {
                        issues.push(json!({
                            "field": field.id,
                            "message": format!("record {missing} does not exist in {}", target.name)
                        }));
                    }
                }
            }
            values.push((*field_id, value));
        }
        if issues.is_empty() {
            Ok(values)
        } else {
            Err(ApiError::invalid(
                "some values are not acceptable",
                json!(issues),
            ))
        }
    }

    /// Creates a record.
    ///
    /// # Errors
    /// 403 without the right to create; 422 for unacceptable values.
    pub fn create_record(
        &mut self,
        actor: Actor,
        table_id: TableId,
        inputs: &BTreeMap<FieldId, CellInput>,
        now: Timestamp,
    ) -> ApiResult<RecordId> {
        let table = self.table(table_id)?;
        self.require(
            actor,
            &Action::RecordCreate.into(),
            table.resource(),
            None,
            now,
        )?;
        let values = self.read_inputs(actor, table, None, inputs, now)?;
        crate::process::guard_create(self, table, &values)?;
        // Required fields that were not given.
        let mut issues = Vec::new();
        for field in &table.fields {
            let given = values
                .iter()
                .any(|(id, value)| *id == field.id && !value.is_empty());
            let defaulted = field.data_type == DataType::Principal
                && field.role == Some(FieldRole::PartyRequester);
            if field.validation.required && !given && !field.is_generated() && !defaulted {
                issues.push(
                    json!({"field": field.id, "message": format!("`{}` is required", field.name)}),
                );
            }
        }
        if !issues.is_empty() {
            return Err(ApiError::invalid("some values are missing", json!(issues)));
        }
        let requester: Vec<FieldId> = table
            .fields
            .iter()
            .filter(|f| {
                f.data_type == DataType::Principal && f.role == Some(FieldRole::PartyRequester)
            })
            .map(|f| f.id)
            .collect();
        let numbered: Vec<(FieldId, mt_core::numbering::NumberingPattern)> = table
            .fields
            .iter()
            .filter_map(|f| f.format.numbering.clone().map(|p| (f.id, p)))
            .collect();
        let id = self.new_record_id(now)?;
        let mut record = Record {
            id,
            cells: BTreeMap::new(),
            errors: BTreeMap::new(),
            created_by: actor.on_behalf_of.unwrap_or(actor.principal),
            created_at: now,
            modified_by: actor.on_behalf_of.unwrap_or(actor.principal),
            modified_at: now,
            version: 1,
        };
        let creator = record.created_by;
        for field in requester {
            set_cell(&mut record, field, Value::Principal(vec![creator]));
        }
        for (field, pattern) in numbered {
            let key = format!("{field}:{}", pattern.period_key(now.date()));
            let next = self.data.counters.get(&key).copied();
            let (text, following) = pattern.allocate(next, now.date()).map_err(internal)?;
            self.data.counters.insert(key, following);
            set_cell(&mut record, field, Value::Text(text));
        }
        let mut links = Vec::new();
        for (field, value) in values {
            if let Value::Link(ids) = &value {
                links.push((field, ids.clone()));
            }
            set_cell(&mut record, field, value);
        }
        crate::process::initialise(self, table_id, &mut record);
        self.table_mut(table_id)?.records.push(record);
        for (field, ids) in links {
            self.sync_inverse(table_id, field, id, &ids)?;
        }
        self.recompute(now);
        let title = self.title(table_id, id);
        self.audit(
            actor,
            "record.create",
            table_id,
            Some(id),
            json!({"title": title}),
            now,
        )?;
        Ok(id)
    }

    /// Changes some fields of a record.
    ///
    /// # Errors
    /// 404, 403 without the right to change it, 422 for unacceptable values.
    pub fn update_record(
        &mut self,
        actor: Actor,
        table_id: TableId,
        record_id: RecordId,
        inputs: &BTreeMap<FieldId, CellInput>,
        now: Timestamp,
    ) -> ApiResult<()> {
        let table = self.table(table_id)?;
        let record = table
            .record(record_id)
            .ok_or_else(|| ApiError::not_found(format!("record {record_id}")))?;
        self.require(
            actor,
            &Action::RecordUpdate.into(),
            table.resource(),
            Some(record),
            now,
        )?;
        let values = self.read_inputs(actor, table, Some(record), inputs, now)?;
        let before: Vec<(FieldId, String, String)> = values
            .iter()
            .filter_map(|(field, value)| {
                let def = table.field(*field)?;
                let old = record.cells.get(field).cloned().unwrap_or(Value::Null);
                (old != *value).then(|| {
                    (
                        *field,
                        def.name.clone(),
                        crate::view::display(self, def, &old),
                    )
                })
            })
            .collect();
        if before.is_empty() {
            return Ok(());
        }
        let changed: Vec<(FieldId, Value)> = values
            .iter()
            .filter(|(field, _)| before.iter().any(|(changed, _, _)| changed == field))
            .cloned()
            .collect();
        crate::process::guard_update(self, table, record, &changed)?;
        let mut links = Vec::new();
        {
            let table = self.table_mut(table_id)?;
            let record = table
                .records
                .iter_mut()
                .find(|r| r.id == record_id)
                .ok_or_else(|| ApiError::not_found(format!("record {record_id}")))?;
            for (field, value) in values {
                if before.iter().any(|(changed, _, _)| *changed == field) {
                    if let Value::Link(ids) = &value {
                        links.push((field, ids.clone()));
                    }
                    set_cell(record, field, value);
                }
            }
            record.modified_by = actor.on_behalf_of.unwrap_or(actor.principal);
            record.modified_at = now;
            record.version = record.version.saturating_add(1);
        }
        for (field, ids) in links {
            self.sync_inverse(table_id, field, record_id, &ids)?;
        }
        self.recompute(now);
        let table = self.table(table_id)?;
        let changes: Vec<serde_json::Value> = before
            .iter()
            .map(|(field, name, old)| {
                let new = table
                    .record(record_id)
                    .and_then(|r| r.cells.get(field))
                    .zip(table.field(*field))
                    .map_or_else(String::new, |(value, def)| {
                        crate::view::display(self, def, value)
                    });
                json!({"field": name, "from": old, "to": new})
            })
            .collect();
        let title = self.title(table_id, record_id);
        self.audit(
            actor,
            "record.update",
            table_id,
            Some(record_id),
            json!({"title": title, "changes": changes}),
            now,
        )
    }

    /// Deletes a record and removes it from the links that point to it.
    ///
    /// # Errors
    /// 404, 403 without the right to delete it.
    pub fn delete_record(
        &mut self,
        actor: Actor,
        table_id: TableId,
        record_id: RecordId,
        now: Timestamp,
    ) -> ApiResult<()> {
        let table = self.table(table_id)?;
        let record = table
            .record(record_id)
            .ok_or_else(|| ApiError::not_found(format!("record {record_id}")))?;
        self.require(
            actor,
            &Action::RecordDelete.into(),
            table.resource(),
            Some(record),
            now,
        )?;
        crate::process::guard_delete(self, table, record)?;
        let title = self.title(table_id, record_id);
        self.table_mut(table_id)?
            .records
            .retain(|r| r.id != record_id);
        // Remove dangling links everywhere.
        let link_fields: Vec<(TableId, FieldId)> = self
            .data
            .tables
            .iter()
            .flat_map(|t| {
                t.fields
                    .iter()
                    .filter(|f| matches!(f.options, OptionsSource::Table { table } if table == table_id))
                    .map(move |f| (t.id, f.id))
            })
            .collect();
        for (other, field) in link_fields {
            for record in &mut self.table_mut(other)?.records {
                if let Some(Value::Link(ids)) = record.cells.get(&field) {
                    let kept: Vec<RecordId> =
                        ids.iter().copied().filter(|id| *id != record_id).collect();
                    set_cell(record, field, Value::Link(kept));
                }
            }
        }
        self.recompute(now);
        self.audit(
            actor,
            "record.delete",
            table_id,
            Some(record_id),
            json!({"title": title}),
            now,
        )
    }

    /// Keeps the mirrored link field in step after `field` of `record` was set to `ids`.
    fn sync_inverse(
        &mut self,
        table_id: TableId,
        field: FieldId,
        record: RecordId,
        ids: &[RecordId],
    ) -> ApiResult<()> {
        let pairs: Vec<(TableId, FieldId)> = self
            .data
            .links
            .iter()
            .filter_map(|pair| {
                if pair.table == table_id && pair.field == field {
                    Some((pair.inverse_table, pair.inverse_field))
                } else if pair.inverse_table == table_id && pair.inverse_field == field {
                    Some((pair.table, pair.field))
                } else {
                    None
                }
            })
            .collect();
        for (inverse_table, inverse_field) in pairs {
            let single = self
                .table(inverse_table)?
                .field(inverse_field)
                .is_some_and(|f| f.validation.max_items == Some(1));
            // Records that now point to a new owner leave their previous one.
            let mut moved: Vec<(RecordId, Vec<RecordId>)> = Vec::new();
            for other in &mut self.table_mut(inverse_table)?.records {
                let current = match other.cells.get(&inverse_field) {
                    Some(Value::Link(list)) => list.clone(),
                    Some(_) | None => Vec::new(),
                };
                let wanted = ids.contains(&other.id);
                let has = current.contains(&record);
                if wanted && !has {
                    let next = if single {
                        moved.push((other.id, current.clone()));
                        vec![record]
                    } else {
                        let mut next = current.clone();
                        next.push(record);
                        next
                    };
                    set_cell(other, inverse_field, Value::Link(next));
                } else if !wanted && has {
                    let next: Vec<RecordId> =
                        current.into_iter().filter(|id| *id != record).collect();
                    set_cell(other, inverse_field, Value::Link(next));
                }
            }
            for (child, previous_owners) in moved {
                for owner in &mut self.table_mut(table_id)?.records {
                    if owner.id != record && previous_owners.contains(&owner.id) {
                        if let Some(Value::Link(list)) = owner.cells.get(&field) {
                            let next: Vec<RecordId> =
                                list.iter().copied().filter(|id| *id != child).collect();
                            set_cell(owner, field, Value::Link(next));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Adds a field to a table (schema change).
    ///
    /// # Errors
    /// 403 without the right to change the schema; 422 for a definition that does not fit.
    pub fn add_field(
        &mut self,
        actor: Actor,
        table_id: TableId,
        field: FieldDef,
        now: Timestamp,
    ) -> ApiResult<FieldId> {
        let table = self.table(table_id)?;
        self.require(
            actor,
            &Action::SchemaManage.into(),
            table.resource(),
            None,
            now,
        )?;
        if table
            .fields
            .iter()
            .any(|f| f.name.eq_ignore_ascii_case(&field.name))
        {
            return Err(ApiError::conflict(format!(
                "{} already has a field named `{}`",
                table.name, field.name
            )));
        }
        let invalid = |e: mt_core::CoreError| ApiError::invalid(e.to_string(), json!(null));
        field.check().map_err(invalid)?;
        let types: BTreeMap<FieldId, DataType> =
            table.fields.iter().map(|f| (f.id, f.data_type)).collect();
        field
            .check_in_table(&|id| types.get(&id).copied())
            .map_err(invalid)?;
        let schema = crate::formula::TableSchema { ws: self, table };
        mt_formula::field::check_field(&field, &schema)
            .map_err(|e| ApiError::invalid(e.to_string(), json!(null)))?;
        let id = field.id;
        let name = field.name.clone();
        let kind = mt_core::fieldtype::BuiltinType::of(&field);
        self.table_mut(table_id)?.fields.push(field);
        if let Err(error) = self.rebuild_graph() {
            self.table_mut(table_id)?.fields.retain(|f| f.id != id);
            self.rebuild_graph()?;
            return Err(error);
        }
        self.recompute(now);
        self.audit(
            actor,
            "field.create",
            table_id,
            None,
            json!({"field": name, "type": kind.label()}),
            now,
        )?;
        Ok(id)
    }

    /// The display title of a record (its primary field).
    #[must_use]
    pub fn title(&self, table: TableId, record: RecordId) -> String {
        let Ok(table) = self.table(table) else {
            return record.to_string();
        };
        let Some(found) = table.record(record) else {
            return record.to_string();
        };
        table
            .field(table.primary)
            .and_then(|field| {
                found
                    .cells
                    .get(&field.id)
                    .map(|v| crate::view::display(self, field, v))
            })
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "(untitled)".to_owned())
    }

    // ---------------------------------------------------------------- audit

    /// Appends an entry to the audit chain.
    ///
    /// # Errors
    /// When the entry cannot be prepared.
    pub fn audit(
        &mut self,
        actor: Actor,
        action: &str,
        table: TableId,
        record: Option<RecordId>,
        details: serde_json::Value,
        now: Timestamp,
    ) -> ApiResult<()> {
        let object = match record {
            Some(record) => ObjectRef::new("record", record.to_string()),
            None => ObjectRef::new("table", table.to_string()),
        };
        self.audit_object(actor, action, object, details, now)
    }

    /// Appends an entry about any object (a base, a person, a code list) to the audit chain.
    ///
    /// # Errors
    /// When the entry cannot be prepared.
    pub fn audit_object(
        &mut self,
        actor: Actor,
        action: &str,
        object: ObjectRef,
        details: serde_json::Value,
        now: Timestamp,
    ) -> ApiResult<()> {
        let principal = |id: PrincipalId| {
            let kind = match self.person(id).map(|p| p.kind) {
                Some(PrincipalKind::Agent) => mt_audit::PrincipalKind::AiAgent,
                Some(PrincipalKind::App) => mt_audit::PrincipalKind::Service,
                Some(PrincipalKind::Member | PrincipalKind::Guest | PrincipalKind::Group)
                | None => mt_audit::PrincipalKind::User,
            };
            mt_audit::Principal::new(kind, id.to_string())
        };
        let mut draft =
            AuditDraft::direct(principal(actor.principal), action, object, now.millis())
                .with_details(details);
        if let Some(initiator) = actor.on_behalf_of {
            draft = draft.on_behalf_of(principal(initiator));
        }
        let mut store = InMemoryAuditStore::new();
        *store.entries_mut() = core::mem::take(&mut self.data.audit);
        let result = self.log.append(&mut store, draft);
        self.data.audit = core::mem::take(store.entries_mut());
        result.map(|_| ()).map_err(internal)
    }

    /// Verifies the audit chain.
    #[must_use]
    pub fn verify_audit(&self, entries: &[AuditEntry]) -> mt_audit::ChainVerdict {
        mt_audit::verify_entries(self.log.genesis(), entries)
    }

    /// The reference-set id behind a field's options, if any.
    #[must_use]
    pub fn refset_of(field: &FieldDef) -> Option<RefSetId> {
        field.options.ref_scope().map(|scope| scope.set)
    }
}

/// Sets a cell; an empty value removes it.
pub fn set_cell(record: &mut Record, field: FieldId, value: Value) {
    if value.is_empty() {
        record.cells.remove(&field);
    } else {
        record.cells.insert(field, value);
    }
}

impl Directory for Workspace {
    fn kind(&self, principal: PrincipalId) -> Option<PrincipalKind> {
        self.person(principal).map(|person| person.kind)
    }

    fn groups_of(&self, principal: PrincipalId) -> BTreeSet<PrincipalId> {
        let mut found = BTreeSet::new();
        let mut frontier = vec![principal];
        while let Some(current) = frontier.pop() {
            for group in &self.data.people {
                if group.members.contains(&current) && found.insert(group.id) {
                    frontier.push(group.id);
                }
            }
        }
        found
    }

    fn members_of(&self, group: PrincipalId) -> BTreeSet<PrincipalId> {
        let mut found = BTreeSet::new();
        let mut frontier = vec![group];
        while let Some(current) = frontier.pop() {
            for member in self
                .person(current)
                .map(|p| p.members.as_slice())
                .unwrap_or(&[])
            {
                if found.insert(*member) {
                    frontier.push(*member);
                }
            }
        }
        found
    }
}

impl Facts for Workspace {
    fn owner_fields(&self, table: TableId) -> Vec<FieldId> {
        self.table(table).map_or_else(
            |_| Vec::new(),
            |table| {
                table
                    .fields
                    .iter()
                    .filter(|f| {
                        f.data_type == DataType::Principal
                            && matches!(
                                f.role,
                                Some(
                                    FieldRole::PersonOwner
                                        | FieldRole::PersonAssignee
                                        | FieldRole::PartyRequester
                                )
                            )
                    })
                    .map(|f| f.id)
                    .collect()
            },
        )
    }

    fn is_within(&self, set: RefSetId, root: RefItemId, item: RefItemId) -> bool {
        self.refdata
            .get(set)
            .is_some_and(|set| set.is_within(root, item))
    }
}

/// Whether a day is a working day (Monday to Friday) — the business calendar is a
/// placeholder until `mt-lifecycle`.
#[must_use]
pub fn is_workday(day: Date) -> bool {
    !matches!(day.weekday(), Weekday::Saturday | Weekday::Sunday)
}

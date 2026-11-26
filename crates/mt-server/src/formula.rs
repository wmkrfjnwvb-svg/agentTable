//! How formulas see the workspace: a table's fields by name, a record and the values of its
//! linked records, and the outside world (today, working days, parameters, labels).

use crate::model::{Record, Table, Workspace};
use mt_core::field::DataType;
use mt_core::id::{FieldId, RefItemId};
use mt_core::time::{Date, Timestamp};
use mt_core::value::Value;
use mt_formula::{Env, Names, Schema};

/// A table as the formula parser and type checker see it.
pub struct TableSchema<'a> {
    /// The workspace.
    pub ws: &'a Workspace,
    /// The formula's table.
    pub table: &'a Table,
}

impl TableSchema<'_> {
    fn linked(&self, link: FieldId) -> Option<&Table> {
        let target = self.ws.link_target(self.table.id, link)?;
        self.ws.table(target).ok()
    }
}

fn by_name(table: &Table, name: &str) -> Option<FieldId> {
    table
        .fields
        .iter()
        .find(|field| field.name.eq_ignore_ascii_case(name.trim()))
        .map(|field| field.id)
}

impl Schema for TableSchema<'_> {
    fn field_by_name(&self, name: &str) -> Option<FieldId> {
        by_name(self.table, name)
    }

    fn field_type(&self, field: FieldId) -> Option<DataType> {
        self.table.field(field).map(|f| f.data_type)
    }

    fn linked_field_by_name(&self, link: FieldId, name: &str) -> Option<FieldId> {
        self.linked(link).and_then(|table| by_name(table, name))
    }

    fn linked_field_type(&self, link: FieldId, field: FieldId) -> Option<DataType> {
        self.linked(link)
            .and_then(|table| table.field(field))
            .map(|f| f.data_type)
    }
}

impl Names for TableSchema<'_> {
    fn field(&self, field: FieldId) -> Option<String> {
        self.table.field(field).map(|f| f.name.clone())
    }

    fn linked_field(&self, link: FieldId, field: FieldId) -> Option<String> {
        self.linked(link)
            .and_then(|table| table.field(field))
            .map(|f| f.name.clone())
    }
}

/// A record and its linked records.
pub struct RecordCtx<'a> {
    /// The workspace.
    pub ws: &'a Workspace,
    /// The record's table.
    pub table: &'a Table,
    /// The record.
    pub record: &'a Record,
}

impl mt_formula::Record for RecordCtx<'_> {
    fn value(&self, field: FieldId) -> Option<&Value> {
        self.record.cells.get(&field)
    }

    fn linked_values(&self, link: FieldId, field: FieldId) -> Vec<Value> {
        let Some(Value::Link(ids)) = self.record.cells.get(&link) else {
            return Vec::new();
        };
        let Some(target) = self
            .ws
            .link_target(self.table.id, link)
            .and_then(|t| self.ws.table(t).ok())
        else {
            return Vec::new();
        };
        ids.iter()
            .map(|id| {
                target
                    .record(*id)
                    .and_then(|r| r.cells.get(&field).cloned())
                    .unwrap_or(Value::Null)
            })
            .collect()
    }
}

/// The outside world of a formula.
pub struct FormulaEnv<'a> {
    /// The workspace (parameters, labels).
    pub ws: &'a Workspace,
    /// The current instant.
    pub now: Timestamp,
    /// The base of the record being computed: settings resolve with its overrides.
    pub base: Option<mt_core::id::BaseId>,
}

impl Env for FormulaEnv<'_> {
    fn today(&self) -> Date {
        self.now.date()
    }

    fn now(&self) -> Timestamp {
        self.now
    }

    fn start_of_day(&self, date: Date) -> Timestamp {
        date.to_timestamp()
    }

    fn is_workday(&self, date: Date) -> bool {
        crate::model::is_workday(date)
    }

    fn param(&self, key: &str) -> Option<Value> {
        self.ws
            .registry()
            .value(
                key,
                &mt_refdata::ParamContext {
                    base: self.base,
                    user: None,
                },
            )
            .cloned()
    }

    fn label(&self, item: RefItemId) -> Option<String> {
        self.ws.label(item)
    }

    fn code(&self, item: RefItemId) -> Option<String> {
        self.ws
            .data
            .refsets
            .iter()
            .find_map(|state| self.ws.refdata().get(state.set.id)?.get(item))
            .map(|found| found.code.clone())
    }

    fn attribute(&self, item: RefItemId, _name: &str) -> Option<Value> {
        self.ws
            .data
            .refsets
            .iter()
            .find_map(|state| self.ws.refdata().get(state.set.id)?.get(item))
            .and_then(|found| found.value.clone())
    }
}

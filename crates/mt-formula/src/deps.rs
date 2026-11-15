//! What a formula depends on, and the order in which computed fields are recomputed.
//!
//! * [`dependencies`] lists what one formula reads: fields of its own record, fields of
//!   linked records, parameters, and whether it reads the clock (`TODAY`, `NOW`).
//! * [`analyse`] adds design-time warnings about formulas that will be expensive.
//! * A [`Graph`] holds every computed field of a base. Building it refuses circular
//!   definitions (also across tables, through links) and fixes an evaluation order in which
//!   every formula comes after the computed fields it reads.
//! * [`Graph::affected`] answers "these fields of a record changed; what must be
//!   recomputed?" — for the same record, and for the records of other tables that link to
//!   it.
//! * [`Graph::compute`] evaluates the computed fields of one record in order, each seeing
//!   the results before it.

use crate::ast::{DateFn, Expr, Function, ListFn, LookupFn};
use crate::error::{ErrorValue, FormulaError};
use crate::eval::{evaluate_to, Computed, Env, Record};
use crate::schema::Schema;
use mt_core::id::{FieldId, TableId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Maximum number of computed fields in one graph.
pub const MAX_COMPUTED_FIELDS: usize = 10_000;
/// Number of linked-record references above which a formula gets a warning.
pub const MANY_LINKED_LOOKUPS: usize = 5;

/// What one formula reads.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Dependencies {
    /// Fields of the formula's own record (including the link fields it goes through).
    pub fields: BTreeSet<FieldId>,
    /// Fields of linked records: `(link field, field of the linked table)`.
    pub through: BTreeSet<(FieldId, FieldId)>,
    /// Parameters named by a literal key.
    pub params: BTreeSet<String>,
    /// A parameter key is computed, so any parameter may be read.
    pub any_param: bool,
    /// The result changes with time (`TODAY`, `NOW`), without any record changing.
    pub volatile: bool,
}

/// Lists what a formula reads.
#[must_use]
pub fn dependencies(expr: &Expr) -> Dependencies {
    let mut found = Dependencies::default();
    expr.walk(&mut |node| match node {
        Expr::Field(field) => {
            found.fields.insert(*field);
        }
        Expr::Through { link, field } => {
            found.fields.insert(*link);
            found.through.insert((*link, *field));
        }
        Expr::Call { function, args } => match function {
            Function::Date(DateFn::Today | DateFn::Now) => found.volatile = true,
            Function::Lookup(LookupFn::Param) => match args.first() {
                Some(Expr::Text(key)) => {
                    found.params.insert(key.clone());
                }
                Some(
                    Expr::Number(_)
                    | Expr::Bool(_)
                    | Expr::Field(_)
                    | Expr::Through { .. }
                    | Expr::CurrentValue
                    | Expr::Unary { .. }
                    | Expr::Binary { .. }
                    | Expr::Call { .. },
                )
                | None => found.any_param = true,
            },
            Function::Logic(_)
            | Function::Number(_)
            | Function::Text(_)
            | Function::Date(_)
            | Function::List(_)
            | Function::Lookup(_) => {}
        },
        Expr::Number(_)
        | Expr::Text(_)
        | Expr::Bool(_)
        | Expr::CurrentValue
        | Expr::Unary { .. }
        | Expr::Binary { .. } => {}
    });
    found
}

/// A design-time warning about the cost of a formula.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "warning", rename_all = "snake_case")]
pub enum Warning {
    /// The formula reads the clock, so it is recomputed when read and once a day rather
    /// than only when its record changes.
    Volatile,
    /// The formula reads the clock *and* linked records: every record's linked records are
    /// read again every day.
    VolatileOverLinks,
    /// The formula goes through links many times; each is a read of all linked records.
    ManyLinkedLookups {
        /// How many times.
        count: usize,
    },
    /// A list condition contains another list function: the inner list is processed once
    /// per item of the outer list.
    NestedListCondition,
}

/// Size, dependencies and warnings of one formula.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Analysis {
    /// Number of parts.
    pub nodes: usize,
    /// Nesting depth.
    pub depth: usize,
    /// What it reads.
    pub dependencies: Dependencies,
    /// What to tell the person writing it.
    pub warnings: Vec<Warning>,
}

fn is_list_condition_function(function: Function) -> bool {
    matches!(
        function,
        Function::List(ListFn::Filter | ListFn::CountIf | ListFn::SumIf | ListFn::AverageIf)
    )
}

/// Analyses a formula for the person writing it.
#[must_use]
pub fn analyse(expr: &Expr) -> Analysis {
    let dependencies = dependencies(expr);
    let mut lookups = 0_usize;
    let mut nested = false;
    expr.walk(&mut |node| match node {
        Expr::Through { .. } => lookups = lookups.saturating_add(1),
        Expr::Call { function, args } if is_list_condition_function(*function) => {
            if let Some(condition) = args.get(1) {
                condition.walk(&mut |inner| {
                    if let Expr::Call { function, .. } = inner {
                        nested |= matches!(function, Function::List(_));
                    }
                    nested |= matches!(inner, Expr::Through { .. });
                });
            }
        }
        Expr::Number(_)
        | Expr::Text(_)
        | Expr::Bool(_)
        | Expr::Field(_)
        | Expr::CurrentValue
        | Expr::Unary { .. }
        | Expr::Binary { .. }
        | Expr::Call { .. } => {}
    });
    let mut warnings = Vec::new();
    if dependencies.volatile {
        warnings.push(if dependencies.through.is_empty() {
            Warning::Volatile
        } else {
            Warning::VolatileOverLinks
        });
    }
    if lookups > MANY_LINKED_LOOKUPS {
        warnings.push(Warning::ManyLinkedLookups { count: lookups });
    }
    if nested {
        warnings.push(Warning::NestedListCondition);
    }
    Analysis {
        nodes: expr.node_count(),
        depth: expr.depth(),
        dependencies,
        warnings,
    }
}

/// A computed field: its place and its formula.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputedField {
    /// The table.
    pub table: TableId,
    /// The field.
    pub field: FieldId,
    /// The formula.
    pub expr: Expr,
}

/// Computed fields of another table that read the changed record through a link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    /// The table whose records must be refreshed.
    pub table: TableId,
    /// Its link field: the records to refresh are those whose link contains the changed
    /// record.
    pub link: FieldId,
    /// The computed fields to recompute for those records, in evaluation order.
    pub fields: Vec<FieldId>,
}

/// What must be recomputed after fields of a record changed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Affected {
    /// Computed fields of the same record, in evaluation order.
    pub local: Vec<FieldId>,
    /// Computed fields of records in tables that link to the changed record. After those
    /// are recomputed, ask again for that table to follow longer chains.
    pub remote: Vec<Remote>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Node {
    expr: Expr,
    deps: Dependencies,
}

type Key = (TableId, FieldId);

/// Every computed field of a base, in a consistent evaluation order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Graph {
    nodes: BTreeMap<Key, Node>,
    /// For every computed field that reads linked records: the table each link leads to.
    targets: BTreeMap<(TableId, FieldId), TableId>,
    order: Vec<Key>,
}

impl Graph {
    /// Builds the graph. `link_target` tells which table a link field leads to; `name` gives
    /// a field's display name for the error message.
    ///
    /// # Errors
    /// [`FormulaError::Cycle`] when computed fields depend on each other in a circle,
    /// [`FormulaError::Type`] when a field is given twice or a formula goes through a field
    /// that is not a link, [`FormulaError::Limit`] for too many fields.
    pub fn build(
        fields: Vec<ComputedField>,
        link_target: &dyn Fn(TableId, FieldId) -> Option<TableId>,
        name: &dyn Fn(TableId, FieldId) -> String,
    ) -> Result<Graph, FormulaError> {
        if fields.len() > MAX_COMPUTED_FIELDS {
            return Err(FormulaError::Limit("more than 10,000 computed fields"));
        }
        let mut nodes: BTreeMap<Key, Node> = BTreeMap::new();
        let mut targets = BTreeMap::new();
        for computed in fields {
            let key = (computed.table, computed.field);
            let deps = dependencies(&computed.expr);
            for (link, _) in &deps.through {
                let target = link_target(computed.table, *link).ok_or_else(|| {
                    FormulaError::typing(format!(
                        "`{}` goes through `{}`, which is not a link field",
                        name(computed.table, computed.field),
                        name(computed.table, *link)
                    ))
                })?;
                targets.insert((computed.table, *link), target);
            }
            let node = Node {
                expr: computed.expr,
                deps,
            };
            if nodes.insert(key, node).is_some() {
                return Err(FormulaError::typing(format!(
                    "`{}` is defined twice",
                    name(key.0, key.1)
                )));
            }
        }
        let mut graph = Graph {
            nodes,
            targets,
            order: Vec::new(),
        };
        graph.order = graph.sorted(name)?;
        Ok(graph)
    }

    /// The computed fields a computed field reads.
    fn inputs(&self, key: Key) -> Vec<Key> {
        let Some(node) = self.nodes.get(&key) else {
            return Vec::new();
        };
        let local = node.deps.fields.iter().map(|field| (key.0, *field));
        let remote = node.deps.through.iter().filter_map(|(link, field)| {
            self.targets
                .get(&(key.0, *link))
                .map(|table| (*table, *field))
        });
        local
            .chain(remote)
            .filter(|input| self.nodes.contains_key(input))
            .collect()
    }

    /// Orders the fields so that each comes after the computed fields it reads.
    fn sorted(&self, name: &dyn Fn(TableId, FieldId) -> String) -> Result<Vec<Key>, FormulaError> {
        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            InProgress,
            Done,
        }
        let mut marks: BTreeMap<Key, Mark> = BTreeMap::new();
        let mut order = Vec::with_capacity(self.nodes.len());
        for start in self.nodes.keys() {
            if marks.contains_key(start) {
                continue;
            }
            // Depth-first search with an explicit stack: (field, its inputs, next input).
            let mut stack: Vec<(Key, Vec<Key>, usize)> = vec![(*start, self.inputs(*start), 0)];
            marks.insert(*start, Mark::InProgress);
            while let Some((key, inputs, next)) = stack.last_mut() {
                let Some(input) = inputs.get(*next).copied() else {
                    marks.insert(*key, Mark::Done);
                    order.push(*key);
                    stack.pop();
                    continue;
                };
                *next = next.saturating_add(1);
                match marks.get(&input) {
                    Some(Mark::Done) => {}
                    Some(Mark::InProgress) => {
                        // The circle is the part of the stack from `input` on.
                        let mut path: Vec<String> = stack
                            .iter()
                            .map(|(key, _, _)| *key)
                            .skip_while(|key| *key != input)
                            .map(|key| name(key.0, key.1))
                            .collect();
                        path.push(name(input.0, input.1));
                        return Err(FormulaError::Cycle {
                            path: path.join(" → "),
                        });
                    }
                    None => {
                        marks.insert(input, Mark::InProgress);
                        let inputs = self.inputs(input);
                        stack.push((input, inputs, 0));
                    }
                }
            }
        }
        Ok(order)
    }

    /// Number of computed fields.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether there is no computed field.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The formula of a computed field.
    #[must_use]
    pub fn expr(&self, table: TableId, field: FieldId) -> Option<&Expr> {
        self.nodes.get(&(table, field)).map(|node| &node.expr)
    }

    /// What a computed field reads.
    #[must_use]
    pub fn dependencies_of(&self, table: TableId, field: FieldId) -> Option<&Dependencies> {
        self.nodes.get(&(table, field)).map(|node| &node.deps)
    }

    /// Every computed field, each after the computed fields it reads.
    #[must_use]
    pub fn order(&self) -> &[(TableId, FieldId)] {
        &self.order
    }

    /// The computed fields of one table in evaluation order.
    #[must_use]
    pub fn order_in(&self, table: TableId) -> Vec<FieldId> {
        self.order
            .iter()
            .filter(|(t, _)| *t == table)
            .map(|(_, field)| *field)
            .collect()
    }

    /// The computed fields of `table` that read any of `changed`, directly or through other
    /// computed fields of the same record, in evaluation order. Returns them together with
    /// the set of all changed fields (the given ones and the recomputed ones).
    fn local_closure(
        &self,
        table: TableId,
        changed: &BTreeSet<FieldId>,
    ) -> (Vec<FieldId>, BTreeSet<FieldId>) {
        let mut dirty = changed.clone();
        let mut local = Vec::new();
        for field in self.order_in(table) {
            let reads_dirty = self
                .nodes
                .get(&(table, field))
                .is_some_and(|node| !node.deps.fields.is_disjoint(&dirty));
            if reads_dirty && !changed.contains(&field) {
                dirty.insert(field);
                local.push(field);
            }
        }
        (local, dirty)
    }

    /// What must be recomputed after the fields `changed` of a record of `table` changed.
    #[must_use]
    pub fn affected(&self, table: TableId, changed: &BTreeSet<FieldId>) -> Affected {
        let (local, dirty) = self.local_closure(table, changed);
        // Other records read this one through links: group their fields by table and link.
        let mut readers: BTreeMap<(TableId, FieldId), BTreeSet<FieldId>> = BTreeMap::new();
        for ((reader_table, reader_field), node) in &self.nodes {
            for (link, field) in &node.deps.through {
                let leads_here = self.targets.get(&(*reader_table, *link)) == Some(&table);
                if leads_here && dirty.contains(field) {
                    readers
                        .entry((*reader_table, *link))
                        .or_default()
                        .insert(*reader_field);
                }
            }
        }
        let remote = readers
            .into_iter()
            .map(|((reader_table, link), direct)| {
                // The directly affected fields and what depends on them in the same record.
                let (dependents, _) = self.local_closure(reader_table, &direct);
                let wanted: BTreeSet<FieldId> = direct.into_iter().chain(dependents).collect();
                Remote {
                    table: reader_table,
                    link,
                    fields: self
                        .order_in(reader_table)
                        .into_iter()
                        .filter(|field| wanted.contains(field))
                        .collect(),
                }
            })
            .collect();
        Affected { local, remote }
    }

    /// Evaluates computed fields of one record in order; each formula sees the results
    /// before it. `schema` describes the record's table; a computed field it does not know
    /// is skipped. `only` restricts the evaluation to some fields (a partial recompute; the
    /// others are read from the record as stored).
    #[must_use]
    pub fn compute(
        &self,
        table: TableId,
        schema: &dyn Schema,
        record: &dyn Record,
        env: &dyn Env,
        only: Option<&BTreeSet<FieldId>>,
    ) -> Computed {
        let mut computed = Computed::new();
        for field in self.order_in(table) {
            if only.is_some_and(|wanted| !wanted.contains(&field)) {
                continue;
            }
            let (Some(expr), Some(data_type)) = (self.expr(table, field), schema.field_type(field))
            else {
                continue;
            };
            let result: Result<_, ErrorValue> =
                evaluate_to(expr, schema, record, env, Some(&computed), data_type);
            computed.insert(field, result);
        }
        computed
    }
}

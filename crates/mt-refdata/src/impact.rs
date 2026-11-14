//! Impact analysis: what a change to a reference item touches, before it is made.
//!
//! Disabling, moving or removing an item affects records that carry it, filters and rules
//! that mention it and integrations that map to it. This module computes *which items* a
//! change touches (the item, and for branch-wide changes its descendants) and asks a
//! [`Usage`] source — implemented by storage — how much each of them is used.

use crate::change::Change;
use crate::effective::EffectiveSet;
use mt_core::id::RefItemId;
use serde::{Deserialize, Serialize};

/// The kind of thing that refers to a reference item besides records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    /// A field's default value or option subtree.
    Field,
    /// A saved view filter.
    Filter,
    /// An automation rule.
    Rule,
    /// A workflow or approval definition.
    Workflow,
    /// An integration mapping.
    Mapping,
    /// A parameter value.
    Parameter,
    /// A dashboard or report.
    Dashboard,
}

/// Something that refers to a reference item.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Reference {
    /// What kind of thing it is.
    pub kind: ReferenceKind,
    /// Its name, for people.
    pub name: String,
}

/// How much reference items are used. Implemented by storage.
pub trait Usage {
    /// Number of records that carry the item.
    fn records(&self, item: RefItemId) -> u64;

    /// Filters, rules, mappings and so on that mention the item.
    fn references(&self, item: RefItemId) -> Vec<Reference>;
}

/// A [`Usage`] source that knows of no use at all.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoUsage;

impl Usage for NoUsage {
    fn records(&self, _item: RefItemId) -> u64 {
        0
    }

    fn references(&self, _item: RefItemId) -> Vec<Reference> {
        Vec::new()
    }
}

/// The use of one touched item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemImpact {
    /// The item.
    pub item: RefItemId,
    /// Records that carry it.
    pub records: u64,
    /// Other things that mention it.
    pub references: Vec<Reference>,
}

/// What a change touches.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Impact {
    /// Every touched item that exists in the set, the changed item first.
    pub items: Vec<ItemImpact>,
    /// Why the change must not be made as it is, if so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<String>,
}

impl Impact {
    /// Total number of records carrying a touched item.
    #[must_use]
    pub fn records(&self) -> u64 {
        self.items
            .iter()
            .fold(0_u64, |sum, item| sum.saturating_add(item.records))
    }

    /// Total number of filters, rules, mappings and so on that mention a touched item.
    #[must_use]
    pub fn reference_count(&self) -> usize {
        self.items.iter().fold(0_usize, |sum, item| {
            sum.saturating_add(item.references.len())
        })
    }

    /// Whether nothing in use is touched.
    #[must_use]
    pub fn is_harmless(&self) -> bool {
        self.blocked.is_none() && self.records() == 0 && self.reference_count() == 0
    }
}

/// Computes what `change` would touch in `set`.
///
/// * Adding or enabling an item touches nothing in use.
/// * Disabling or moving an item touches the item and its whole branch (its descendants
///   become unselectable, or change what "is under X" means).
/// * Updating an item touches the item; if the validity period changes, its branch too.
/// * Changing aliases touches the item (integrations map through them).
/// * Removing an item touches the item and is **blocked** while anything still uses it:
///   disable it instead, so that existing records keep a meaningful value.
#[must_use]
pub fn impact(set: &EffectiveSet, change: &Change, usage: &dyn Usage) -> Impact {
    let with_branch = |id: RefItemId| {
        let mut ids = vec![id];
        ids.extend(set.descendants(id));
        ids
    };
    let touched = match change {
        Change::Add { .. } | Change::SetEnabled { enabled: true, .. } => Vec::new(),
        Change::SetEnabled { id, enabled: false } | Change::Move { id, .. } => with_branch(*id),
        Change::Update { id, patch } => {
            if patch.valid_from.is_some() || patch.valid_to.is_some() {
                with_branch(*id)
            } else {
                vec![*id]
            }
        }
        Change::SetAliases { id, .. } | Change::Remove { id } => vec![*id],
    };
    let items: Vec<ItemImpact> = touched
        .into_iter()
        .filter(|id| set.get(*id).is_some())
        .map(|item| ItemImpact {
            item,
            records: usage.records(item),
            references: usage.references(item),
        })
        .collect();
    let mut result = Impact {
        items,
        blocked: None,
    };
    if matches!(change, Change::Remove { .. }) && !result.is_harmless() {
        result.blocked = Some(format!(
            "the item is still used by {} record(s) and {} other reference(s); disable it instead",
            result.records(),
            result.reference_count()
        ));
    }
    result
}

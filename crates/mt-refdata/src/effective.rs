//! The effective set: what one tenant actually sees of a reference set.
//!
//! [`EffectiveSet::build`] merges the set's items with the tenant's [`Overlay`], verifies
//! everything that needs the whole set, and indexes the result for fast questions:
//!
//! * unique ids and codes (codes compared without regard to ASCII case);
//! * a well-formed tree: parents exist, no cycles, no item deeper than the set allows, and
//!   no parents at all in a flat set;
//! * unambiguous crosswalks: one external value maps to one item per external system;
//! * overlay rules: only system sets take an overlay; it may relabel, disable and add
//!   aliases to system items and add its own items, nothing else.
//!
//! A built set is immutable. Changes go through [`crate::Ledger`], which builds a new one.
//!
//! # When may an item be selected?
//!
//! An item can be newly selected on a day when it **and every ancestor** is enabled and
//! valid on that day (switching off "Japan" switches off the whole branch), and it fits the
//! field's scope (subtree, level, leaves only). Items already on a record are never removed
//! by this rule; that is the caller's `previous` value in `mt_core::validate`.

use crate::error::RefDataError;
use crate::model::{
    check_aliases, check_label, check_names, localised, Overlay, RefItem, RefSet, SetScope,
    Structure, MAX_ITEMS_PER_SET,
};
use mt_core::condition::fold_text;
use mt_core::field::RefScope;
use mt_core::id::{RefItemId, RefSetId};
use mt_core::time::Date;
use mt_core::validate::OptionResolver;
use std::collections::{BTreeMap, BTreeSet};

/// Who an item of an effective set belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Part of a system set; the tenant may relabel, disable and alias it.
    System,
    /// Owned by the tenant (an item of a tenant or base set, or an overlay addition).
    Tenant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Node {
    item: RefItem,
    origin: Origin,
    depth: u8,
    children: Vec<RefItemId>,
}

/// A validated, indexed reference set as one tenant sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveSet {
    set: RefSet,
    nodes: BTreeMap<RefItemId, Node>,
    by_code: BTreeMap<String, RefItemId>,
    roots: Vec<RefItemId>,
    crosswalk: BTreeMap<String, BTreeMap<String, RefItemId>>,
}

impl EffectiveSet {
    /// Merges `items` with the tenant `overlay` (system sets only) and verifies the result.
    ///
    /// # Errors
    /// The first problem found; see the module documentation for the rules.
    pub fn build(
        set: &RefSet,
        items: &[RefItem],
        overlay: Option<&Overlay>,
    ) -> Result<Self, RefDataError> {
        set.check()?;
        let overlay = overlay.filter(|o| !o.is_empty());
        if overlay.is_some() && set.scope != SetScope::System {
            return Err(RefDataError::OverlayNotAllowed);
        }
        let base_origin = match set.scope {
            SetScope::System => Origin::System,
            SetScope::Tenant | SetScope::Base { .. } => Origin::Tenant,
        };
        let mut nodes: BTreeMap<RefItemId, Node> = BTreeMap::new();
        let mut add = |item: &RefItem, origin: Origin| -> Result<(), RefDataError> {
            item.check(set)?;
            let node = Node {
                item: item.clone(),
                origin,
                depth: 0,
                children: Vec::new(),
            };
            if nodes.insert(item.id, node).is_some() {
                return Err(RefDataError::DuplicateItem { id: item.id });
            }
            Ok(())
        };
        for item in items {
            add(item, base_origin)?;
        }
        if let Some(overlay) = overlay {
            for item in &overlay.added {
                add(item, Origin::Tenant)?;
            }
            Self::apply_overlay(&mut nodes, overlay)?;
        }
        if nodes.len() > MAX_ITEMS_PER_SET {
            return Err(RefDataError::Limit("too many items in one reference set"));
        }

        let mut by_code: BTreeMap<String, RefItemId> = BTreeMap::new();
        for (id, node) in &nodes {
            if by_code
                .insert(node.item.code.to_ascii_lowercase(), *id)
                .is_some()
            {
                return Err(RefDataError::DuplicateCode {
                    code: node.item.code.clone(),
                });
            }
        }

        let depths = Self::depths(set, &nodes)?;
        let mut children: BTreeMap<RefItemId, Vec<RefItemId>> = BTreeMap::new();
        let mut roots = Vec::new();
        for (id, node) in &nodes {
            match node.item.parent {
                Some(parent) => children.entry(parent).or_default().push(*id),
                None => roots.push(*id),
            }
        }
        let order = |ids: &mut Vec<RefItemId>| {
            ids.sort_by_cached_key(|id| {
                let item = nodes.get(id).map(|n| &n.item);
                (
                    item.map_or(0, |i| i.sort),
                    item.map(|i| fold_text(&i.name)).unwrap_or_default(),
                    *id,
                )
            });
        };
        order(&mut roots);
        for list in children.values_mut() {
            order(list);
        }

        let mut crosswalk: BTreeMap<String, BTreeMap<String, RefItemId>> = BTreeMap::new();
        for (id, node) in &nodes {
            for (system, values) in &node.item.aliases {
                let table = crosswalk.entry(system.clone()).or_default();
                for value in values {
                    if table.insert(value.clone(), *id).is_some() {
                        return Err(RefDataError::AliasConflict {
                            system: system.clone(),
                            value: value.clone(),
                        });
                    }
                }
            }
        }

        for (id, node) in &mut nodes {
            node.depth = depths.get(id).copied().unwrap_or(1);
            node.children = children.remove(id).unwrap_or_default();
        }
        Ok(EffectiveSet {
            set: set.clone(),
            nodes,
            by_code,
            roots,
            crosswalk,
        })
    }

    /// The system item an overlay entry refers to. Relabelling, disabling and aliasing are
    /// for system items; the tenant edits its own additions directly.
    fn system_node(
        nodes: &mut BTreeMap<RefItemId, Node>,
        id: RefItemId,
    ) -> Result<&mut Node, RefDataError> {
        match nodes.get_mut(&id) {
            None => Err(RefDataError::UnknownItem { id }),
            Some(node) if node.origin == Origin::Tenant => Err(RefDataError::invalid(
                "overlay",
                format!("{id} is the tenant's own item; change the item itself"),
            )),
            Some(node) => Ok(node),
        }
    }

    fn apply_overlay(
        nodes: &mut BTreeMap<RefItemId, Node>,
        overlay: &Overlay,
    ) -> Result<(), RefDataError> {
        for id in &overlay.disabled {
            Self::system_node(nodes, *id)?.item.enabled = false;
        }
        for (id, relabel) in &overlay.relabel {
            let node = Self::system_node(nodes, *id)?;
            if let Some(name) = &relabel.name {
                check_label("item name", name)?;
                node.item.name.clone_from(name);
            }
            check_names("item name", &relabel.names)?;
            for (locale, name) in &relabel.names {
                node.item.names.insert(locale.clone(), name.clone());
            }
        }
        for (id, aliases) in &overlay.aliases {
            let node = Self::system_node(nodes, *id)?;
            check_aliases(aliases)?;
            for (system, values) in aliases {
                node.item
                    .aliases
                    .entry(system.clone())
                    .or_default()
                    .extend(values.iter().cloned());
            }
            check_aliases(&node.item.aliases)?;
        }
        Ok(())
    }

    /// The level of every item (1 = top), verifying parents, cycles and the depth limit.
    fn depths(
        set: &RefSet,
        nodes: &BTreeMap<RefItemId, Node>,
    ) -> Result<BTreeMap<RefItemId, u8>, RefDataError> {
        let max = set.structure.max_depth();
        let mut resolved: BTreeMap<RefItemId, usize> = BTreeMap::new();
        for (start, node) in nodes {
            if let Some(parent) = node.item.parent {
                if set.structure == Structure::Flat {
                    return Err(RefDataError::FlatSetHasParent { item: *start });
                }
                if !nodes.contains_key(&parent) {
                    return Err(RefDataError::UnknownParent {
                        item: *start,
                        parent,
                    });
                }
            }
            if resolved.contains_key(start) {
                continue;
            }
            // Walk up to a root or to an item whose level is known, then come back down.
            let mut trail = vec![*start];
            let mut on_trail: BTreeSet<RefItemId> = BTreeSet::from([*start]);
            let mut level = 0_usize;
            let mut current = node.item.parent;
            while let Some(parent) = current {
                if let Some(known) = resolved.get(&parent) {
                    level = *known;
                    break;
                }
                if !on_trail.insert(parent) {
                    return Err(RefDataError::Cycle { item: parent });
                }
                trail.push(parent);
                current = nodes.get(&parent).and_then(|n| n.item.parent);
            }
            while let Some(id) = trail.pop() {
                level = level.saturating_add(1);
                if level > usize::from(max) {
                    return Err(RefDataError::TooDeep {
                        item: id,
                        depth: level,
                        max,
                    });
                }
                resolved.insert(id, level);
            }
        }
        Ok(resolved
            .into_iter()
            .map(|(id, level)| (id, u8::try_from(level).unwrap_or(u8::MAX)))
            .collect())
    }

    /// The set definition.
    #[must_use]
    pub const fn set(&self) -> &RefSet {
        &self.set
    }

    /// Number of items.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the set has no items.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// An item by id, with the tenant's labels, aliases and disabling applied.
    #[must_use]
    pub fn get(&self, id: RefItemId) -> Option<&RefItem> {
        self.nodes.get(&id).map(|node| &node.item)
    }

    /// An item by code (ASCII case ignored).
    #[must_use]
    pub fn by_code(&self, code: &str) -> Option<&RefItem> {
        self.by_code
            .get(&code.to_ascii_lowercase())
            .and_then(|id| self.get(*id))
    }

    /// Who the item belongs to.
    #[must_use]
    pub fn origin(&self, id: RefItemId) -> Option<Origin> {
        self.nodes.get(&id).map(|node| node.origin)
    }

    /// Every item id, in id order.
    pub fn ids(&self) -> impl Iterator<Item = RefItemId> + '_ {
        self.nodes.keys().copied()
    }

    /// Top-level items in display order.
    #[must_use]
    pub fn roots(&self) -> &[RefItemId] {
        &self.roots
    }

    /// The children of an item in display order (empty for a leaf or an unknown item).
    #[must_use]
    pub fn children(&self, id: RefItemId) -> &[RefItemId] {
        self.nodes.get(&id).map_or(&[], |node| &node.children)
    }

    /// The parent of an item.
    #[must_use]
    pub fn parent(&self, id: RefItemId) -> Option<RefItemId> {
        self.get(id).and_then(|item| item.parent)
    }

    /// The level of an item (1 = top level).
    #[must_use]
    pub fn depth(&self, id: RefItemId) -> Option<u8> {
        self.nodes.get(&id).map(|node| node.depth)
    }

    /// Whether the item exists and has no children.
    #[must_use]
    pub fn is_leaf(&self, id: RefItemId) -> bool {
        self.nodes
            .get(&id)
            .is_some_and(|node| node.children.is_empty())
    }

    /// The ancestors of an item, nearest first (empty for a top-level or unknown item).
    #[must_use]
    pub fn ancestors(&self, id: RefItemId) -> Vec<RefItemId> {
        let mut out = Vec::new();
        let mut current = self.parent(id);
        while let Some(parent) = current {
            out.push(parent);
            current = self.parent(parent);
        }
        out
    }

    /// The path from the top level down to the item itself (empty for an unknown item).
    #[must_use]
    pub fn path(&self, id: RefItemId) -> Vec<RefItemId> {
        if !self.nodes.contains_key(&id) {
            return Vec::new();
        }
        let mut path = self.ancestors(id);
        path.reverse();
        path.push(id);
        path
    }

    /// Whether `item` is a descendant of `ancestor` (an item is not within itself).
    #[must_use]
    pub fn is_within(&self, ancestor: RefItemId, item: RefItemId) -> bool {
        let mut current = self.parent(item);
        while let Some(parent) = current {
            if parent == ancestor {
                return true;
            }
            current = self.parent(parent);
        }
        false
    }

    /// Every descendant of an item, parents before children, siblings in display order.
    #[must_use]
    pub fn descendants(&self, id: RefItemId) -> Vec<RefItemId> {
        self.walk_from(self.children(id))
    }

    /// Every item, parents before children, siblings in display order.
    #[must_use]
    pub fn walk(&self) -> Vec<RefItemId> {
        self.walk_from(&self.roots)
    }

    fn walk_from(&self, start: &[RefItemId]) -> Vec<RefItemId> {
        let mut out = Vec::new();
        let mut stack: Vec<RefItemId> = start.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            out.push(id);
            stack.extend(self.children(id).iter().rev().copied());
        }
        out
    }

    /// Whether the item and all its ancestors are enabled and valid on `day`.
    #[must_use]
    pub fn is_active(&self, id: RefItemId, day: Date) -> bool {
        let Some(item) = self.get(id) else {
            return false;
        };
        let usable = |item: &RefItem| item.enabled && item.is_valid_on(day);
        usable(item)
            && self
                .ancestors(id)
                .into_iter()
                .all(|ancestor| self.get(ancestor).is_some_and(usable))
    }

    /// Whether the item may be newly selected on `day` in a field with the given scope.
    /// False for a scope that names another set.
    #[must_use]
    pub fn is_selectable(&self, scope: &RefScope, item: RefItemId, day: Date) -> bool {
        scope.set == self.set.id
            && self.is_active(item, day)
            && scope.subtree.is_none_or(|root| self.is_within(root, item))
            && scope
                .level
                .is_none_or(|level| self.depth(item) == Some(level))
            && (!scope.leaves_only || self.is_leaf(item))
    }

    /// The items a field with the given scope offers on `day`, parents before children.
    #[must_use]
    pub fn options(&self, scope: &RefScope, day: Date) -> Vec<RefItemId> {
        let candidates = match scope.subtree {
            Some(root) => self.descendants(root),
            None => self.walk(),
        };
        candidates
            .into_iter()
            .filter(|id| self.is_selectable(scope, *id, day))
            .collect()
    }

    /// The label of an item for a locale.
    #[must_use]
    pub fn label(&self, id: RefItemId, locale: &str) -> Option<&str> {
        self.get(id)
            .map(|item| localised(&item.name, &item.names, locale))
    }

    /// The labels along the path, joined: `Asia / Japan / Kanto / Tokyo`.
    #[must_use]
    pub fn path_label(&self, id: RefItemId, locale: &str, separator: &str) -> Option<String> {
        let labels: Vec<&str> = self
            .path(id)
            .into_iter()
            .filter_map(|step| self.label(step, locale))
            .collect();
        (!labels.is_empty()).then(|| labels.join(separator))
    }

    /// Items whose label (in `locale`) or code contains `query`, case-insensitively, in
    /// display order, at most `limit`. A blank query finds nothing.
    #[must_use]
    pub fn search(&self, query: &str, locale: &str, limit: usize) -> Vec<RefItemId> {
        let needle = fold_text(query.trim());
        if needle.is_empty() {
            return Vec::new();
        }
        self.walk()
            .into_iter()
            .filter(|id| {
                self.get(*id).is_some_and(|item| {
                    fold_text(localised(&item.name, &item.names, locale)).contains(&needle)
                        || fold_text(&item.code).contains(&needle)
                })
            })
            .take(limit)
            .collect()
    }

    /// The item an external system means by `external_value` (exact match).
    #[must_use]
    pub fn crosswalk(&self, system: &str, external_value: &str) -> Option<RefItemId> {
        self.crosswalk
            .get(system)
            .and_then(|table| table.get(external_value))
            .copied()
    }

    /// The value to send to an external system for an item: its first alias there.
    #[must_use]
    pub fn alias(&self, system: &str, item: RefItemId) -> Option<&str> {
        self.get(item)
            .and_then(|item| item.aliases.get(system))
            .and_then(|values| values.iter().next())
            .map(String::as_str)
    }

    /// The external systems that have at least one alias in this set.
    pub fn systems(&self) -> impl Iterator<Item = &str> {
        self.crosswalk.keys().map(String::as_str)
    }
}

/// Answers whether a parameter currently lists an item as an option
/// (`OptionsSource::Parameter` in `mt-core`). Implemented by the parameter registry.
pub trait ParameterOptions {
    /// Whether the parameter `key` offers `item`.
    fn offers(&self, key: &str, item: RefItemId) -> bool;
}

/// A [`ParameterOptions`] without any parameter: nothing is offered.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoParameters;

impl ParameterOptions for NoParameters {
    fn offers(&self, _key: &str, _item: RefItemId) -> bool {
        false
    }
}

/// The reference sets of one tenant, ready for lookups.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RefData {
    sets: BTreeMap<RefSetId, EffectiveSet>,
    by_code: BTreeMap<String, RefSetId>,
}

impl RefData {
    /// An empty catalogue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a set or replaces the set with the same id (after a change).
    ///
    /// # Errors
    /// [`RefDataError::DuplicateCode`] when another set already uses the code.
    pub fn insert(&mut self, set: EffectiveSet) -> Result<(), RefDataError> {
        let id = set.set().id;
        let code = set.set().code.clone();
        if self.by_code.get(&code).is_some_and(|other| *other != id) {
            return Err(RefDataError::DuplicateCode { code });
        }
        if let Some(previous) = self.sets.insert(id, set) {
            self.by_code.remove(&previous.set().code);
        }
        self.by_code.insert(code, id);
        Ok(())
    }

    /// Removes a set.
    pub fn remove(&mut self, id: RefSetId) -> Option<EffectiveSet> {
        let removed = self.sets.remove(&id)?;
        self.by_code.remove(&removed.set().code);
        Some(removed)
    }

    /// A set by id.
    #[must_use]
    pub fn get(&self, id: RefSetId) -> Option<&EffectiveSet> {
        self.sets.get(&id)
    }

    /// A set by code.
    #[must_use]
    pub fn by_code(&self, code: &str) -> Option<&EffectiveSet> {
        self.by_code.get(code).and_then(|id| self.sets.get(id))
    }

    /// Number of sets.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sets.len()
    }

    /// Whether there is no set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    /// The option resolver for writes whose effective date is `day`: what
    /// `mt_core::validate` asks to decide whether a reference value is acceptable.
    #[must_use]
    pub fn resolver<'a>(&'a self, day: Date, parameters: &'a dyn ParameterOptions) -> Resolver<'a> {
        Resolver {
            data: self,
            day,
            parameters,
        }
    }
}

/// [`OptionResolver`] over a tenant's reference data for one effective date.
#[derive(Clone, Copy)]
pub struct Resolver<'a> {
    data: &'a RefData,
    day: Date,
    parameters: &'a dyn ParameterOptions,
}

impl OptionResolver for Resolver<'_> {
    fn is_selectable(&self, scope: &RefScope, item: RefItemId) -> bool {
        self.data
            .get(scope.set)
            .is_some_and(|set| set.is_selectable(scope, item, self.day))
    }

    fn is_within(&self, set: RefSetId, ancestor: RefItemId, item: RefItemId) -> bool {
        self.data
            .get(set)
            .is_some_and(|set| set.is_within(ancestor, item))
    }

    fn parameter_offers(&self, key: &str, item: RefItemId) -> bool {
        self.parameters.offers(key, item)
    }
}

//! Changes to a reference set and the state they apply to.
//!
//! A [`State`] is what storage keeps for one tenant's view of a set: the set definition, the
//! set's own items and (for a system set) the tenant's overlay. A [`Change`] says what
//! someone wants to modify. [`State::apply`] routes it:
//!
//! | Change | Tenant or base set | System set (through the tenant's overlay) |
//! |---|---|---|
//! | `add` | new item | new tenant item |
//! | `update` | any patch | tenant items: any patch · system items: name and localised names only |
//! | `move` | new parent | tenant items only |
//! | `set_enabled` | the item's flag | tenant items: the flag · system items: the overlay's disabled list |
//! | `set_aliases` | the item's aliases | tenant items: the item · system items: the tenant's own aliases |
//! | `remove` | only without children | tenant items only, only without children |
//!
//! Codes never change: no change can touch them. Every application is verified by building
//! the [`EffectiveSet`] of the result, so an accepted change always leaves a valid set.

use crate::effective::EffectiveSet;
use crate::error::RefDataError;
use crate::model::{Overlay, RefItem, RefSet, Relabel, SetScope};
use mt_core::id::{FieldId, RefItemId};
use mt_core::time::Date;
use mt_core::value::Value;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// (De)serialises `Option<Option<T>>` so that a missing key (`None`: leave as is), an
/// explicit `null` (`Some(None)`: clear) and a value (`Some(Some(v))`: set) stay distinct.
mod double_option {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[allow(clippy::option_option)]
    pub fn serialize<T: Serialize, S: Serializer>(
        value: &Option<Option<T>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value
            .as_ref()
            .and_then(Option::as_ref)
            .serialize(serializer)
    }

    #[allow(clippy::option_option)]
    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Option<T>>, D::Error> {
        Option::<T>::deserialize(deserializer).map(Some)
    }
}

/// The parts of an item to change. Absent parts stay as they are. The code is not here:
/// codes are immutable.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[allow(clippy::option_option)]
pub struct ItemPatch {
    /// New default name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Localised names to set (per locale; other locales stay).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, String>,
    /// New typed value (`null` clears it).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub value: Option<Option<Value>>,
    /// New sort position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<i64>,
    /// New first valid day (`null` removes the limit).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub valid_from: Option<Option<Date>>,
    /// New last valid day (`null` removes the limit).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub valid_to: Option<Option<Date>>,
    /// Custom attribute values to set; a null or empty value removes the attribute.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<FieldId, Value>,
}

impl ItemPatch {
    /// Whether the patch touches anything other than the default and localised names.
    fn goes_beyond_labels(&self) -> bool {
        self.value.is_some()
            || self.sort.is_some()
            || self.valid_from.is_some()
            || self.valid_to.is_some()
            || !self.attributes.is_empty()
    }

    fn is_empty(&self) -> bool {
        self.name.is_none() && self.names.is_empty() && !self.goes_beyond_labels()
    }

    fn apply_to(&self, item: &mut RefItem) {
        if let Some(name) = &self.name {
            item.name.clone_from(name);
        }
        for (locale, name) in &self.names {
            item.names.insert(locale.clone(), name.clone());
        }
        if let Some(value) = &self.value {
            item.value.clone_from(value);
        }
        if let Some(sort) = self.sort {
            item.sort = sort;
        }
        if let Some(valid_from) = self.valid_from {
            item.valid_from = valid_from;
        }
        if let Some(valid_to) = self.valid_to {
            item.valid_to = valid_to;
        }
        for (field, value) in &self.attributes {
            if value.is_empty() {
                item.attributes.remove(field);
            } else {
                item.attributes.insert(*field, value.clone());
            }
        }
    }
}

/// One requested modification of a set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Change {
    /// Add an item.
    Add {
        /// The new item.
        item: RefItem,
    },
    /// Change parts of an item.
    Update {
        /// The item.
        id: RefItemId,
        /// What to change.
        patch: ItemPatch,
    },
    /// Give an item another parent (`None`: make it a top-level item).
    Move {
        /// The item.
        id: RefItemId,
        /// The new parent.
        parent: Option<RefItemId>,
    },
    /// Enable or disable an item. A disabled item stays on existing records.
    SetEnabled {
        /// The item.
        id: RefItemId,
        /// The new state.
        enabled: bool,
    },
    /// Replace the aliases of an item in one external system (an empty set removes them).
    SetAliases {
        /// The item.
        id: RefItemId,
        /// The external system.
        system: String,
        /// The values that system uses for the item.
        values: BTreeSet<String>,
    },
    /// Remove an item for good. Only for items without children; whether records still use
    /// it is the caller's check (see [`crate::impact`]). Disabling is the normal way.
    Remove {
        /// The item.
        id: RefItemId,
    },
}

impl Change {
    /// The item the change is about.
    #[must_use]
    pub const fn item(&self) -> RefItemId {
        match self {
            Change::Add { item } => item.id,
            Change::Update { id, .. }
            | Change::Move { id, .. }
            | Change::SetEnabled { id, .. }
            | Change::SetAliases { id, .. }
            | Change::Remove { id } => *id,
        }
    }
}

/// One tenant's stored view of a set: the definition, the set's own items and, for a system
/// set, the tenant's overlay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// The set definition.
    pub set: RefSet,
    /// The set's own items (for a system set: the system items, which tenants cannot edit).
    pub items: Vec<RefItem>,
    /// The tenant's overlay (always empty unless the set is a system set).
    #[serde(default, skip_serializing_if = "Overlay::is_empty")]
    pub overlay: Overlay,
}

impl State {
    /// A state without an overlay.
    #[must_use]
    pub fn new(set: RefSet, items: Vec<RefItem>) -> Self {
        State {
            set,
            items,
            overlay: Overlay::default(),
        }
    }

    /// The validated, indexed set this state describes.
    ///
    /// # Errors
    /// See [`EffectiveSet::build`].
    pub fn effective(&self) -> Result<EffectiveSet, RefDataError> {
        EffectiveSet::build(&self.set, &self.items, Some(&self.overlay))
    }

    fn is_system(&self) -> bool {
        self.set.scope == SetScope::System
    }

    /// The editable item with this id: one of the set's own items, or for a system set one
    /// of the tenant's additions.
    fn editable(&mut self, id: RefItemId) -> Option<&mut RefItem> {
        let list = if self.is_system() {
            &mut self.overlay.added
        } else {
            &mut self.items
        };
        list.iter_mut().find(|item| item.id == id)
    }

    fn is_system_item(&self, id: RefItemId) -> bool {
        self.is_system() && self.items.iter().any(|item| item.id == id)
    }

    fn has_children(&self, id: RefItemId) -> bool {
        self.items
            .iter()
            .chain(&self.overlay.added)
            .any(|item| item.parent == Some(id))
    }

    /// Applies one change without verifying the resulting set.
    fn apply_unchecked(&mut self, change: &Change) -> Result<(), RefDataError> {
        let system_item =
            |action: &'static str, id: RefItemId| RefDataError::SystemItem { id, action };
        match change {
            Change::Add { item } => {
                if self.is_system() {
                    self.overlay.added.push(item.clone());
                } else {
                    self.items.push(item.clone());
                }
                Ok(())
            }
            Change::Update { id, patch } => {
                if patch.is_empty() {
                    return Err(RefDataError::EmptyChange);
                }
                if self.is_system_item(*id) {
                    if patch.goes_beyond_labels() {
                        return Err(system_item("change anything but their labels", *id));
                    }
                    let relabel: &mut Relabel = self.overlay.relabel.entry(*id).or_default();
                    if let Some(name) = &patch.name {
                        relabel.name = Some(name.clone());
                    }
                    for (locale, name) in &patch.names {
                        relabel.names.insert(locale.clone(), name.clone());
                    }
                    return Ok(());
                }
                let item = self
                    .editable(*id)
                    .ok_or(RefDataError::UnknownItem { id: *id })?;
                patch.apply_to(item);
                Ok(())
            }
            Change::Move { id, parent } => {
                if self.is_system_item(*id) {
                    return Err(system_item("move them", *id));
                }
                let item = self
                    .editable(*id)
                    .ok_or(RefDataError::UnknownItem { id: *id })?;
                item.parent = *parent;
                Ok(())
            }
            Change::SetEnabled { id, enabled } => {
                if self.is_system_item(*id) {
                    if *enabled {
                        self.overlay.disabled.remove(id);
                    } else {
                        self.overlay.disabled.insert(*id);
                    }
                    return Ok(());
                }
                let item = self
                    .editable(*id)
                    .ok_or(RefDataError::UnknownItem { id: *id })?;
                item.enabled = *enabled;
                Ok(())
            }
            Change::SetAliases { id, system, values } => {
                let aliases = if self.is_system_item(*id) {
                    self.overlay.aliases.entry(*id).or_default()
                } else {
                    &mut self
                        .editable(*id)
                        .ok_or(RefDataError::UnknownItem { id: *id })?
                        .aliases
                };
                if values.is_empty() {
                    aliases.remove(system);
                } else {
                    aliases.insert(system.clone(), values.clone());
                }
                // An overlay entry without any alias left is dropped again.
                if self.overlay.aliases.get(id).is_some_and(BTreeMap::is_empty) {
                    self.overlay.aliases.remove(id);
                }
                Ok(())
            }
            Change::Remove { id } => {
                if self.is_system_item(*id) {
                    return Err(system_item("remove them", *id));
                }
                if self.editable(*id).is_none() {
                    return Err(RefDataError::UnknownItem { id: *id });
                }
                if self.has_children(*id) {
                    return Err(RefDataError::HasChildren { id: *id });
                }
                if self.is_system() {
                    self.overlay.added.retain(|item| item.id != *id);
                } else {
                    self.items.retain(|item| item.id != *id);
                }
                Ok(())
            }
        }
    }

    /// Applies the changes in order and verifies the result. Either all of them apply and
    /// the new state with its effective set is returned, or nothing changes.
    ///
    /// # Errors
    /// [`RefDataError::EmptyChange`] for an empty list, the error of the first change that
    /// cannot be applied, or the first rule the resulting set would break.
    pub fn apply(&self, changes: &[Change]) -> Result<(State, EffectiveSet), RefDataError> {
        if changes.is_empty() {
            return Err(RefDataError::EmptyChange);
        }
        let mut next = self.clone();
        for change in changes {
            next.apply_unchecked(change)?;
        }
        let effective = next.effective()?;
        Ok((next, effective))
    }
}

//! Lifecycles: the states a record moves through and the transitions between them.
//!
//! Every state has a [`StateCategory`] (new, in progress, waiting, pending approval, done,
//! failed, cancelled), so generic features — SLA pauses, "my open work", dashboards — work on
//! any lifecycle without knowing its names. A transition can require a capability, a guard
//! condition on the record, the target state's required fields, and an approval.
//!
//! [`Lifecycle::check`] enforces the closed-loop rule of the product: there is a terminal
//! state, every state is reachable from the initial one, and from every open state a terminal
//! state can be reached — no record can get stuck by design.

use crate::error::LifecycleError;
use mt_core::condition::{Condition, Context};
use mt_core::field::DataType;
use mt_core::id::{FieldId, LifecycleId, TableId};
use mt_core::state::StateCategory;
use mt_core::value::Value;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Most states in one lifecycle.
pub const MAX_STATES: usize = 50;
/// Most transitions in one lifecycle.
pub const MAX_TRANSITIONS: usize = 200;

/// One state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateDef {
    /// Stable key (`submitted`).
    pub key: String,
    /// Display name.
    pub name: String,
    /// What kind of state it is.
    pub category: StateCategory,
    /// Fields that must have a value to enter the state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<FieldId>,
    /// The only fields that may change while in the state (all, if absent; none, if empty).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editable: Option<Vec<FieldId>>,
}

/// One transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    /// Stable key (`submit`).
    pub key: String,
    /// The button label.
    pub name: String,
    /// The states it leaves from (any open state, if empty).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub from: Vec<String>,
    /// The state it leads to.
    pub to: String,
    /// The capability needed to take it (anyone who may update the record, if absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    /// A condition the record must satisfy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<Condition>,
    /// The approval that must succeed; the record waits in the target state's category
    /// "pending approval" meanwhile (the caller starts the approval).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<String>,
}

/// A lifecycle bound to a table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lifecycle {
    /// Stable id.
    pub id: LifecycleId,
    /// The table.
    pub table: TableId,
    /// Name.
    pub name: String,
    /// The states.
    pub states: Vec<StateDef>,
    /// The state new records start in.
    pub initial: String,
    /// The transitions.
    pub transitions: Vec<Transition>,
}

/// Why a transition cannot be taken now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "blocked", rename_all = "snake_case")]
pub enum Blocker {
    /// The record is not in a state the transition leaves from.
    WrongState {
        /// The record's state.
        current: String,
    },
    /// The actor lacks the capability.
    Capability {
        /// The capability needed.
        capability: String,
    },
    /// The guard condition does not hold.
    Guard,
    /// A field the target state requires is empty.
    Required {
        /// The field.
        field: FieldId,
    },
}

/// A transition as offered for a record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    /// The transition's key.
    pub key: String,
    /// The button label.
    pub name: String,
    /// Where it leads.
    pub to: String,
    /// Why it cannot be taken; empty when it can.
    pub blockers: Vec<Blocker>,
}

/// A state change to apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Move {
    /// The state left.
    pub from: String,
    /// The state entered.
    pub to: String,
    /// The approval to start, if the transition has one.
    pub approval: Option<String>,
    /// The category of the state entered.
    pub category: StateCategory,
}

fn invalid(message: impl Into<String>) -> LifecycleError {
    LifecycleError::invalid(message)
}

fn is_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && key.len() <= 64
}

impl Lifecycle {
    /// A state by key.
    #[must_use]
    pub fn state(&self, key: &str) -> Option<&StateDef> {
        self.states.iter().find(|state| state.key == key)
    }

    /// A transition by key.
    #[must_use]
    pub fn transition(&self, key: &str) -> Option<&Transition> {
        self.transitions.iter().find(|t| t.key == key)
    }

    fn leaves_from(&self, transition: &Transition, state: &StateDef) -> bool {
        if transition.from.is_empty() {
            state.category.is_open()
        } else {
            transition.from.contains(&state.key)
        }
    }

    /// Verifies the lifecycle (see the module documentation).
    ///
    /// # Errors
    /// [`LifecycleError::Invalid`] describing the first problem found.
    pub fn check(&self) -> Result<(), LifecycleError> {
        if self.name.trim().is_empty() {
            return Err(invalid("a lifecycle needs a name"));
        }
        if self.states.is_empty() || self.states.len() > MAX_STATES {
            return Err(invalid(format!("a lifecycle has 1 to {MAX_STATES} states")));
        }
        if self.transitions.len() > MAX_TRANSITIONS {
            return Err(invalid(format!(
                "a lifecycle has at most {MAX_TRANSITIONS} transitions"
            )));
        }
        let mut keys = BTreeSet::new();
        for state in &self.states {
            if !is_key(&state.key) {
                return Err(invalid(format!(
                    "`{}` is not a state key: lower-case letters, digits and `_`",
                    state.key
                )));
            }
            if !keys.insert(state.key.as_str()) {
                return Err(invalid(format!(
                    "the state `{}` is defined twice",
                    state.key
                )));
            }
            if state.name.trim().is_empty() {
                return Err(invalid(format!("the state `{}` needs a name", state.key)));
            }
        }
        let initial = self.state(&self.initial).ok_or_else(|| {
            invalid(format!(
                "the initial state `{}` does not exist",
                self.initial
            ))
        })?;
        if initial.category.is_terminal() {
            return Err(invalid("the initial state cannot be a final state"));
        }
        if !self.states.iter().any(|s| s.category.is_terminal()) {
            return Err(invalid(
                "a lifecycle needs a final state (done, failed or cancelled)",
            ));
        }
        let mut transition_keys = BTreeSet::new();
        for transition in &self.transitions {
            if !is_key(&transition.key) {
                return Err(invalid(format!(
                    "`{}` is not a transition key",
                    transition.key
                )));
            }
            if !transition_keys.insert(transition.key.as_str()) {
                return Err(invalid(format!(
                    "the transition `{}` is defined twice",
                    transition.key
                )));
            }
            if transition.name.trim().is_empty() {
                return Err(invalid(format!(
                    "the transition `{}` needs a button label",
                    transition.key
                )));
            }
            for state in transition.from.iter().chain([&transition.to]) {
                if self.state(state).is_none() {
                    return Err(invalid(format!(
                        "the transition `{}` refers to the unknown state `{state}`",
                        transition.key
                    )));
                }
            }
            if transition.from.contains(&transition.to) {
                return Err(invalid(format!(
                    "the transition `{}` leads back to where it starts",
                    transition.key
                )));
            }
            if let Some(key) = &transition.capability {
                if !mt_core::field::is_capability_key(key) {
                    return Err(invalid(format!("`{key}` is not a capability key")));
                }
            }
        }
        // Reachability from the initial state.
        let next = |state: &StateDef| -> Vec<&str> {
            self.transitions
                .iter()
                .filter(|t| self.leaves_from(t, state))
                .map(|t| t.to.as_str())
                .collect()
        };
        let mut reached: BTreeSet<&str> = BTreeSet::from([self.initial.as_str()]);
        let mut frontier = vec![self.initial.as_str()];
        while let Some(key) = frontier.pop() {
            if let Some(state) = self.state(key) {
                for to in next(state) {
                    if reached.insert(to) {
                        frontier.push(to);
                    }
                }
            }
        }
        if let Some(unreached) = self
            .states
            .iter()
            .find(|s| !reached.contains(s.key.as_str()))
        {
            return Err(invalid(format!(
                "the state `{}` can never be reached",
                unreached.key
            )));
        }
        // From every open state some final state can be reached.
        let mut finishing: BTreeSet<&str> = self
            .states
            .iter()
            .filter(|s| s.category.is_terminal())
            .map(|s| s.key.as_str())
            .collect();
        loop {
            let before = finishing.len();
            for state in &self.states {
                if next(state).iter().any(|to| finishing.contains(to)) {
                    finishing.insert(state.key.as_str());
                }
            }
            if finishing.len() == before {
                break;
            }
        }
        if let Some(stuck) = self
            .states
            .iter()
            .find(|s| !finishing.contains(s.key.as_str()))
        {
            return Err(invalid(format!(
                "a record in `{}` could never finish: add a transition towards a final state",
                stuck.key
            )));
        }
        Ok(())
    }

    /// Verifies the parts that name fields against the table. `type_of` returns the data type
    /// of a field of the table.
    ///
    /// # Errors
    /// [`LifecycleError::Invalid`].
    pub fn check_in_table(
        &self,
        type_of: &dyn Fn(FieldId) -> Option<DataType>,
    ) -> Result<(), LifecycleError> {
        for state in &self.states {
            for field in state.required.iter().chain(state.editable.iter().flatten()) {
                if type_of(*field).is_none() {
                    return Err(invalid(format!(
                        "the state `{}` names the unknown field {field}",
                        state.key
                    )));
                }
            }
        }
        for transition in &self.transitions {
            if let Some(guard) = &transition.guard {
                guard
                    .check(type_of)
                    .map_err(|e| invalid(format!("the guard of `{}`: {e}", transition.key)))?;
            }
        }
        Ok(())
    }

    /// What stops a transition from being taken now (nothing, if it can be).
    #[must_use]
    pub fn blockers<'a>(
        &self,
        transition: &Transition,
        current: &str,
        value_of: &dyn Fn(FieldId) -> Option<&'a Value>,
        ctx: &dyn Context,
        may: &dyn Fn(&str) -> bool,
    ) -> Vec<Blocker> {
        let mut blockers = Vec::new();
        let Some(state) = self.state(current) else {
            return vec![Blocker::WrongState {
                current: current.to_owned(),
            }];
        };
        if !self.leaves_from(transition, state) {
            blockers.push(Blocker::WrongState {
                current: current.to_owned(),
            });
        }
        if let Some(capability) = &transition.capability {
            if !may(capability) {
                blockers.push(Blocker::Capability {
                    capability: capability.clone(),
                });
            }
        }
        if transition
            .guard
            .as_ref()
            .is_some_and(|guard| !guard.evaluate(value_of, ctx))
        {
            blockers.push(Blocker::Guard);
        }
        if let Some(target) = self.state(&transition.to) {
            for field in &target.required {
                if value_of(*field).is_none_or(Value::is_empty) {
                    blockers.push(Blocker::Required { field: *field });
                }
            }
        }
        blockers
    }

    /// The transitions that leave the current state, each with what blocks it.
    #[must_use]
    pub fn offers<'a>(
        &self,
        current: &str,
        value_of: &dyn Fn(FieldId) -> Option<&'a Value>,
        ctx: &dyn Context,
        may: &dyn Fn(&str) -> bool,
    ) -> Vec<Offer> {
        let Some(state) = self.state(current) else {
            return Vec::new();
        };
        self.transitions
            .iter()
            .filter(|t| self.leaves_from(t, state))
            .map(|t| Offer {
                key: t.key.clone(),
                name: t.name.clone(),
                to: t.to.clone(),
                blockers: self.blockers(t, current, value_of, ctx, may),
            })
            .collect()
    }

    /// Takes a transition: the move to apply, or everything that blocks it. The caller applies
    /// the move and the record change in one transaction (a transition is atomic).
    ///
    /// # Errors
    /// [`LifecycleError::State`] for an unknown transition or with the blockers listed.
    pub fn take<'a>(
        &self,
        key: &str,
        current: &str,
        value_of: &dyn Fn(FieldId) -> Option<&'a Value>,
        ctx: &dyn Context,
        may: &dyn Fn(&str) -> bool,
    ) -> Result<Move, Vec<Blocker>> {
        let Some(transition) = self.transition(key) else {
            return Err(vec![Blocker::WrongState {
                current: current.to_owned(),
            }]);
        };
        let blockers = self.blockers(transition, current, value_of, ctx, may);
        if !blockers.is_empty() {
            return Err(blockers);
        }
        let category = self
            .state(&transition.to)
            .map_or(StateCategory::InProgress, |s| s.category);
        Ok(Move {
            from: current.to_owned(),
            to: transition.to.clone(),
            approval: transition.approval.clone(),
            category,
        })
    }

    /// Whether a field may change while the record is in a state.
    #[must_use]
    pub fn may_edit(&self, state: &str, field: FieldId) -> bool {
        self.state(state).is_some_and(|s| {
            !s.category.is_terminal()
                && s.editable.as_ref().is_none_or(|list| list.contains(&field))
        })
    }

    /// How many records are in each state, for a board or a dashboard.
    #[must_use]
    pub fn tally<'s>(&self, states: impl IntoIterator<Item = &'s str>) -> BTreeMap<String, usize> {
        let mut counts: BTreeMap<String, usize> =
            self.states.iter().map(|s| (s.key.clone(), 0)).collect();
        for state in states {
            if let Some(count) = counts.get_mut(state) {
                *count = count.saturating_add(1);
            }
        }
        counts
    }
}

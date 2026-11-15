//! The access policy of a tenant and the engine that decides.
//!
//! One engine answers every question about access — for the UI, the API, exports, search,
//! notifications, dashboards and AI alike:
//!
//! * [`Policy::decide`] — may this actor do this, here, to this record? The [`Decision`]
//!   carries the grants that allow it (the *grant chain*) or, for each grant that came
//!   close, why it did not apply. That is the "explain" function; "preview as user" is
//!   simply asking for another actor.
//! * [`Policy::row_filter`] — which records of a table may the actor do this to? Used for
//!   lists, so that a query never returns a record `decide` would refuse.
//! * [`Policy::field_permissions`] — which fields are hidden, masked, visible, editable.
//!
//! **Dual identity.** An AI agent acts *for* someone. An [`Actor`] with `on_behalf_of` is
//! allowed only what both the agent and that person are allowed: grants are checked for
//! each, rows and fields are intersected. An agent can never do more than the person who
//! started it, and never more than it was granted itself.
//!
//! The engine is pure: who is in which group, which fields make a principal an owner, and
//! the shape of reference trees are asked from the caller ([`Directory`], [`Facts`]).

use crate::capability::{Action, Capability};
use crate::error::PermError;
use crate::grant::{
    CapabilitySet, FieldAccess, Grant, Granted, Inactive, Resource, Scope, SodKind, SodRule,
};
use core::fmt;
use mt_core::condition::{Condition, Context};
use mt_core::field::Classification;
use mt_core::id::{
    CapabilitySetId, FieldId, GrantId, PrincipalId, RecordId, RefItemId, RefSetId, SodRuleId,
    TableId,
};
use mt_core::time::{Date, Timestamp, Weekday};
use mt_core::value::Value;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// What kind of principal something is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    /// A member of the tenant.
    Member,
    /// An invited outsider: never an administrator.
    Guest,
    /// An AI agent.
    Agent,
    /// An API application.
    App,
    /// A group of principals.
    Group,
}

/// Who is who: asked from the identity layer.
pub trait Directory {
    /// The kind of a principal; `None` for one that does not exist (or was removed).
    fn kind(&self, principal: PrincipalId) -> Option<PrincipalKind>;

    /// Every group the principal belongs to, directly or through other groups.
    fn groups_of(&self, principal: PrincipalId) -> BTreeSet<PrincipalId>;

    /// Every principal in the group, directly or through other groups.
    fn members_of(&self, group: PrincipalId) -> BTreeSet<PrincipalId>;
}

/// Facts about tables and reference data the engine needs.
pub trait Facts {
    /// The fields of a table that make the principals in them owners of the record: the
    /// fields with an owner, assignee or requester role, and the creator.
    fn owner_fields(&self, table: TableId) -> Vec<FieldId>;

    /// Whether `item` is `root` or below it in the reference set.
    fn is_within(&self, set: RefSetId, root: RefItemId, item: RefItemId) -> bool;
}

/// The record an action is about.
pub trait RecordFacts {
    /// The record's id.
    fn id(&self) -> RecordId;

    /// The value of a field (for a record being created: the proposed value).
    fn value(&self, field: FieldId) -> Option<&Value>;
}

/// Everything the engine asks the outside world.
#[derive(Clone, Copy)]
pub struct Env<'a> {
    /// Principals and groups.
    pub directory: &'a dyn Directory,
    /// Tables and reference data.
    pub facts: &'a dyn Facts,
    /// Today, week start and time zone for conditions with relative dates. Its `me` is not
    /// used: conditions are evaluated for the principal being checked.
    pub context: &'a dyn Context,
}

/// Who acts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Actor {
    /// The principal performing the action.
    pub principal: PrincipalId,
    /// The person an agent (or app) acts for, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_behalf_of: Option<PrincipalId>,
}

impl Actor {
    /// Someone acting for themselves.
    #[must_use]
    pub const fn direct(principal: PrincipalId) -> Actor {
        Actor {
            principal,
            on_behalf_of: None,
        }
    }

    /// An agent acting for the person who started it.
    #[must_use]
    pub const fn agent(agent: PrincipalId, initiator: PrincipalId) -> Actor {
        Actor {
            principal: agent,
            on_behalf_of: Some(initiator),
        }
    }

    /// Every principal whose grants must allow the action.
    fn principals(&self) -> Vec<PrincipalId> {
        let mut all = vec![self.principal];
        if let Some(initiator) = self.on_behalf_of.filter(|p| *p != self.principal) {
            all.push(initiator);
        }
        all
    }
}

/// A question to the engine.
#[derive(Clone, Copy)]
pub struct Request<'a> {
    /// Who acts.
    pub actor: Actor,
    /// What they want to do.
    pub capability: &'a Capability,
    /// Where.
    pub on: Resource,
    /// To which record, if the action is about one. Without a record the answer is about
    /// the place as a whole ("may they see this table at all?").
    pub record: Option<&'a dyn RecordFacts>,
    /// What the actor's principals already did to this record (for separation of duties).
    pub prior: &'a [Capability],
    /// When.
    pub now: Timestamp,
}

/// A grant that allows the action: one link of the grant chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applied {
    /// The grant.
    pub grant: GrantId,
    /// The principal that was checked (the actor, or the person it acts for).
    pub principal: PrincipalId,
    /// Who holds the grant: that principal, or a group it belongs to.
    pub holder: PrincipalId,
    /// The capability set the capability comes from, if the grant gives a set.
    pub through_set: Option<CapabilitySetId>,
    /// The granted capability that covers the requested one (itself, or one that includes
    /// it, such as "update" for "read").
    pub capability: Capability,
    /// Where the grant was given.
    pub on: Resource,
}

/// Why a grant, or the absence of one, does not allow the action.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Reason {
    /// The principal does not exist (any more).
    UnknownPrincipal,
    /// Nobody gave the principal, or any of its groups, this capability.
    NoGrant,
    /// The grant is not in force.
    Inactive {
        /// Why not.
        why: Inactive,
    },
    /// The grant was given somewhere else (another workspace, base or table).
    Elsewhere,
    /// The record is outside the grant's scope.
    OutOfScope,
    /// The record does not satisfy the grant's condition (for example it exceeds a limit).
    ConditionNotMet,
    /// Guests cannot administer.
    Administrative,
    /// A separation-of-duties rule forbids it: the same principal already did the paired
    /// action on this record.
    SeparationOfDuties {
        /// The rule.
        rule: SodRuleId,
    },
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Reason::UnknownPrincipal => "the principal does not exist",
            Reason::NoGrant => "no grant gives this capability",
            Reason::Inactive {
                why: Inactive::Revoked,
            } => "the grant was revoked",
            Reason::Inactive {
                why: Inactive::NotYetValid,
            } => "the grant is not valid yet",
            Reason::Inactive {
                why: Inactive::Expired,
            } => "the grant has expired",
            Reason::Elsewhere => "the grant was given somewhere else",
            Reason::OutOfScope => "the record is outside the grant's scope",
            Reason::ConditionNotMet => "the record does not satisfy the grant's condition",
            Reason::Administrative => "guests cannot administer",
            Reason::SeparationOfDuties { .. } => {
                "separation of duties: the same principal already did the paired action on this record"
            }
        })
    }
}

/// One reason an action is not allowed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    /// The principal that was checked.
    pub principal: PrincipalId,
    /// The grant that came close, if there was one.
    pub grant: Option<GrantId>,
    /// Why it does not allow the action.
    pub reason: Reason,
}

/// The answer to a [`Request`], with its explanation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    /// Whether the action is allowed.
    pub allowed: bool,
    /// The grants that allow it (for an agent acting for a person: of both).
    pub chain: Vec<Applied>,
    /// Why grants that give the capability did not apply. Listed also when the action is
    /// allowed through another grant.
    pub refusals: Vec<Refusal>,
    /// The fields the action may touch (no field when it is not allowed).
    pub fields: FieldAccess,
}

/// A [`Context`] whose `me` is the principal being checked.
struct As<'a> {
    inner: &'a dyn Context,
    me: PrincipalId,
}

impl Context for As<'_> {
    fn me(&self) -> Option<PrincipalId> {
        Some(self.me)
    }

    fn today(&self) -> Date {
        self.inner.today()
    }

    fn week_start(&self) -> Weekday {
        self.inner.week_start()
    }

    fn fiscal_year_start_month(&self) -> u32 {
        self.inner.fiscal_year_start_month()
    }

    fn start_of_day(&self, date: Date) -> Timestamp {
        self.inner.start_of_day(date)
    }
}

/// Whether a record is within a scope for a principal.
fn scope_admits(
    scope: &Scope,
    principal: PrincipalId,
    table: Option<TableId>,
    record: &dyn RecordFacts,
    env: &Env<'_>,
) -> bool {
    let owned_by = |owners: &BTreeSet<PrincipalId>| {
        table.is_some_and(|table| {
            env.facts.owner_fields(table).into_iter().any(|field| {
                matches!(record.value(field), Some(Value::Principal(people))
                    if people.iter().any(|p| owners.contains(p)))
            })
        })
    };
    match scope {
        Scope::All => true,
        Scope::Own => owned_by(&BTreeSet::from([principal])),
        Scope::Team => {
            let mut owners = env.directory.groups_of(principal);
            owners.insert(principal);
            owned_by(&owners)
        }
        Scope::Where { condition } => holds(condition, principal, record, env),
        Scope::RefSubtree { field, set, root } => match record.value(*field) {
            Some(Value::Reference(item)) => env.facts.is_within(*set, *root, *item),
            Some(Value::MultiReference(items)) => items
                .iter()
                .any(|item| env.facts.is_within(*set, *root, *item)),
            Some(
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
                | Value::Link(_)
                | Value::Principal(_)
                | Value::Attachment(_)
                | Value::Json(_),
            )
            | None => false,
        },
        Scope::Records { records } => records.contains(&record.id()),
    }
}

/// Whether a condition holds for a record, with `me` being the principal.
fn holds(
    condition: &Condition,
    principal: PrincipalId,
    record: &dyn RecordFacts,
    env: &Env<'_>,
) -> bool {
    let context = As {
        inner: env.context,
        me: principal,
    };
    condition.evaluate(&|field| record.value(field), &context)
}

/// One way a record can be admitted: a grant's scope and condition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowRule {
    /// The grant the rule comes from.
    pub grant: GrantId,
    /// Which records.
    pub scope: Scope,
    /// What they must also satisfy.
    pub condition: Option<Condition>,
}

/// The records one principal may act on: any of the rules, or everything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowFilterPart {
    /// The principal.
    pub principal: PrincipalId,
    /// A grant without scope or condition applies: every record is admitted.
    pub everything: bool,
    /// Otherwise a record is admitted if any rule admits it (none: no record).
    pub rules: Vec<RowRule>,
}

/// Which records of a table an actor may act on. A record is admitted if every part admits
/// it (one part per principal: the actor, and the person it acts for).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowFilter {
    /// The table.
    pub table: Option<TableId>,
    /// One part per principal.
    pub parts: Vec<RowFilterPart>,
}

impl RowFilter {
    /// Whether no record can be admitted, so the query need not run.
    #[must_use]
    pub fn admits_nothing(&self) -> bool {
        self.parts
            .iter()
            .any(|part| !part.everything && part.rules.is_empty())
    }

    /// Whether every record is admitted, so the query needs no extra filter.
    #[must_use]
    pub fn admits_everything(&self) -> bool {
        self.parts.iter().all(|part| part.everything)
    }

    /// Whether a record is admitted.
    #[must_use]
    pub fn admits(&self, record: &dyn RecordFacts, env: &Env<'_>) -> bool {
        self.parts.iter().all(|part| {
            part.everything
                || part.rules.iter().any(|rule| {
                    scope_admits(&rule.scope, part.principal, self.table, record, env)
                        && rule
                            .condition
                            .as_ref()
                            .is_none_or(|c| holds(c, part.principal, record, env))
                })
        })
    }
}

/// What the engine needs to know about a field to decide who sees and edits it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldMeta {
    /// The field.
    pub id: FieldId,
    /// Its privacy classification.
    pub classification: Classification,
    /// Whether it has a display mask.
    pub masked: bool,
    /// Whether its values are generated (numbered, calculated, system): nobody edits it.
    pub generated: bool,
    /// The capability its field group requires to see it.
    pub view_capability: Option<Capability>,
    /// The capability its field group requires to change it.
    pub edit_capability: Option<Capability>,
}

/// How much of a field's value the viewer gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// Nothing: the value is not sent at all.
    Hidden,
    /// The masked form only.
    Masked,
    /// The value.
    Visible,
}

/// What an actor may do with one field of a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldPermission {
    /// How much of the value they get.
    pub visibility: Visibility,
    /// Whether they may change it.
    pub editable: bool,
}

/// A principal that holds two capabilities which a rule says must be held by different
/// people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The rule.
    pub rule: SodRuleId,
    /// The principal.
    pub principal: PrincipalId,
    /// The grant that gives the rule's first capability.
    pub first: GrantId,
    /// The grant that gives the rule's second capability.
    pub second: GrantId,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    grants: Vec<Grant>,
    sets: Vec<CapabilitySet>,
    rules: Vec<SodRule>,
}

/// The access policy of a tenant: its grants, capability sets and separation-of-duties
/// rules.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(from = "Stored", into = "Stored")]
pub struct Policy {
    grants: BTreeMap<GrantId, Grant>,
    by_principal: BTreeMap<PrincipalId, BTreeSet<GrantId>>,
    sets: BTreeMap<CapabilitySetId, CapabilitySet>,
    rules: BTreeMap<SodRuleId, SodRule>,
}

impl From<Stored> for Policy {
    fn from(stored: Stored) -> Self {
        let mut policy = Policy::default();
        for grant in stored.grants {
            policy.index(grant);
        }
        policy.sets = stored.sets.into_iter().map(|set| (set.id, set)).collect();
        policy.rules = stored
            .rules
            .into_iter()
            .map(|rule| (rule.id, rule))
            .collect();
        policy
    }
}

impl From<Policy> for Stored {
    fn from(policy: Policy) -> Self {
        Stored {
            grants: policy.grants.into_values().collect(),
            sets: policy.sets.into_values().collect(),
            rules: policy.rules.into_values().collect(),
        }
    }
}

impl Policy {
    /// An empty policy: nobody may do anything.
    #[must_use]
    pub fn new() -> Policy {
        Policy::default()
    }

    fn index(&mut self, grant: Grant) {
        self.by_principal
            .entry(grant.principal)
            .or_default()
            .insert(grant.id);
        self.grants.insert(grant.id, grant);
    }

    /// Every grant, revoked ones included, by id.
    pub fn grants(&self) -> impl Iterator<Item = &Grant> {
        self.grants.values()
    }

    /// A grant.
    #[must_use]
    pub fn grant(&self, id: GrantId) -> Option<&Grant> {
        self.grants.get(&id)
    }

    /// The grants given to a principal directly (not through its groups).
    pub fn grants_of(&self, principal: PrincipalId) -> impl Iterator<Item = &Grant> {
        self.by_principal
            .get(&principal)
            .into_iter()
            .flatten()
            .filter_map(|id| self.grants.get(id))
    }

    /// Every capability set.
    pub fn sets(&self) -> impl Iterator<Item = &CapabilitySet> {
        self.sets.values()
    }

    /// A capability set.
    #[must_use]
    pub fn set(&self, id: CapabilitySetId) -> Option<&CapabilitySet> {
        self.sets.get(&id)
    }

    /// Every separation-of-duties rule.
    pub fn rules(&self) -> impl Iterator<Item = &SodRule> {
        self.rules.values()
    }

    /// The capabilities a grant gives, each with the set it comes from.
    fn given<'a>(&'a self, grant: &'a Grant) -> Vec<(&'a Capability, Option<CapabilitySetId>)> {
        match &grant.what {
            Granted::Capability(capability) => vec![(capability, None)],
            Granted::Set(id) => self
                .sets
                .get(id)
                .into_iter()
                .flat_map(|set| set.capabilities.iter().map(|c| (c, Some(set.id))))
                .collect(),
        }
    }

    /// The capability by which a grant gives `wanted` (itself, or one that includes it).
    fn gives(
        &self,
        grant: &Grant,
        wanted: &Capability,
    ) -> Option<(Capability, Option<CapabilitySetId>)> {
        let given = self.given(grant);
        given
            .iter()
            .find(|(capability, _)| *capability == wanted)
            .or_else(|| {
                given
                    .iter()
                    .find(|(capability, _)| capability.includes(wanted))
            })
            .map(|(capability, set)| ((*capability).clone(), *set))
    }

    /// Adds a capability set.
    ///
    /// # Errors
    /// [`PermError::Invalid`] for a malformed set, [`PermError::Duplicate`] for a used id.
    pub fn add_set(&mut self, set: CapabilitySet) -> Result<(), PermError> {
        set.check()?;
        if self.sets.contains_key(&set.id) {
            return Err(PermError::Duplicate {
                what: "capability set",
                id: set.id.to_string(),
            });
        }
        self.sets.insert(set.id, set);
        Ok(())
    }

    /// Changes a capability set. Every grant of the set gives the new capabilities from
    /// now on; the change is refused if it would make a principal hold two capabilities a
    /// rule keeps apart.
    ///
    /// # Errors
    /// [`PermError::Invalid`], [`PermError::UnknownSet`], [`PermError::SodConflict`].
    pub fn update_set(
        &mut self,
        set: CapabilitySet,
        now: Timestamp,
        directory: &dyn Directory,
    ) -> Result<(), PermError> {
        set.check()?;
        let Some(previous) = self.sets.insert(set.id, set.clone()) else {
            self.sets.remove(&set.id);
            return Err(PermError::UnknownSet(set.id));
        };
        let users: Vec<&Grant> = self
            .grants
            .values()
            .filter(|grant| grant.what == Granted::Set(set.id) && grant.revoked_at.is_none())
            .collect();
        let conflict = users
            .into_iter()
            .find_map(|grant| self.hold_conflict(grant, now, directory));
        if let Some(conflict) = conflict {
            self.sets.insert(previous.id, previous);
            return Err(conflict);
        }
        Ok(())
    }

    /// Removes a capability set that no grant in force uses.
    ///
    /// # Errors
    /// [`PermError::UnknownSet`], [`PermError::SetInUse`].
    pub fn remove_set(&mut self, id: CapabilitySetId) -> Result<(), PermError> {
        if !self.sets.contains_key(&id) {
            return Err(PermError::UnknownSet(id));
        }
        let grants = self
            .grants
            .values()
            .filter(|grant| grant.what == Granted::Set(id) && grant.revoked_at.is_none())
            .count();
        if grants > 0 {
            return Err(PermError::SetInUse { set: id, grants });
        }
        self.sets.remove(&id);
        Ok(())
    }

    /// Adds a separation-of-duties rule. Grants that already break it stay (see
    /// [`Policy::violations`]); new ones are refused.
    ///
    /// # Errors
    /// [`PermError::Invalid`], [`PermError::Duplicate`].
    pub fn add_rule(&mut self, rule: SodRule) -> Result<(), PermError> {
        rule.check()?;
        if self.rules.contains_key(&rule.id) {
            return Err(PermError::Duplicate {
                what: "separation-of-duties rule",
                id: rule.id.to_string(),
            });
        }
        self.rules.insert(rule.id, rule);
        Ok(())
    }

    /// Removes a separation-of-duties rule; `false` if there was none.
    pub fn remove_rule(&mut self, id: SodRuleId) -> bool {
        self.rules.remove(&id).is_some()
    }

    /// The principals a grant reaches: the grantee, and for a group its members.
    fn reached(grant: &Grant, directory: &dyn Directory) -> BTreeSet<PrincipalId> {
        let mut reached = BTreeSet::from([grant.principal]);
        if directory.kind(grant.principal) == Some(PrincipalKind::Group) {
            reached.extend(directory.members_of(grant.principal));
        }
        reached
    }

    /// The grants that give a principal — directly or through its groups — a capability
    /// covering `wanted` and that are not revoked or expired at `now`.
    fn holdings<'p>(
        &'p self,
        principal: PrincipalId,
        wanted: &'p Capability,
        now: Timestamp,
        directory: &dyn Directory,
    ) -> Vec<&'p Grant> {
        let mut holders = directory.groups_of(principal);
        holders.insert(principal);
        holders
            .into_iter()
            .flat_map(|holder| self.grants_of(holder))
            .filter(|grant| {
                !matches!(
                    grant.status(now),
                    Err(Inactive::Revoked | Inactive::Expired)
                ) && self.gives(grant, wanted).is_some()
            })
            .collect()
    }

    /// The first "hold" rule that `candidate` (already in the policy, or about to be) would
    /// break for a principal it reaches.
    fn hold_conflict(
        &self,
        candidate: &Grant,
        now: Timestamp,
        directory: &dyn Directory,
    ) -> Option<PermError> {
        for rule in self
            .rules
            .values()
            .filter(|rule| rule.kind == SodKind::Hold)
        {
            for (mine, other) in [(&rule.first, &rule.second), (&rule.second, &rule.first)] {
                if self.gives(candidate, mine).is_none() {
                    continue;
                }
                // The candidate gives one side; does anyone it reaches hold the other side
                // on an overlapping resource — possibly through the candidate itself?
                let both_in_one = self.gives(candidate, other).is_some();
                let held_elsewhere = Self::reached(candidate, directory).into_iter().any(|p| {
                    self.holdings(p, other, now, directory)
                        .into_iter()
                        .any(|grant| grant.id != candidate.id && grant.on.overlaps(&candidate.on))
                });
                if both_in_one || held_elsewhere {
                    return Some(PermError::SodConflict {
                        rule: rule.id,
                        first: rule.first.to_string(),
                        second: rule.second.to_string(),
                    });
                }
            }
        }
        None
    }

    /// Gives a grant.
    ///
    /// # Errors
    /// [`PermError::Invalid`] for a malformed grant, an unknown grantee, or an
    /// administrative capability for a guest; [`PermError::Duplicate`];
    /// [`PermError::UnknownSet`]; [`PermError::SodConflict`] when the grantee (or a member
    /// of the grantee group) would hold two capabilities a rule keeps apart.
    pub fn give(&mut self, grant: Grant, directory: &dyn Directory) -> Result<(), PermError> {
        grant.check()?;
        if self.grants.contains_key(&grant.id) {
            return Err(PermError::Duplicate {
                what: "grant",
                id: grant.id.to_string(),
            });
        }
        if let Granted::Set(set) = &grant.what {
            if !self.sets.contains_key(set) {
                return Err(PermError::UnknownSet(*set));
            }
        }
        let Some(kind) = directory.kind(grant.principal) else {
            return Err(PermError::invalid(format!(
                "{} is not a principal of this tenant",
                grant.principal
            )));
        };
        let administrative = self
            .given(&grant)
            .into_iter()
            .find(|(capability, _)| capability.is_administrative());
        if let (PrincipalKind::Guest, Some((capability, _))) = (kind, administrative) {
            return Err(PermError::invalid(format!(
                "a guest cannot be given `{capability}`"
            )));
        }
        if let Some(conflict) = self.hold_conflict(&grant, grant.granted_at, directory) {
            return Err(conflict);
        }
        self.index(grant);
        Ok(())
    }

    /// Gives a grant on behalf of `giver`, who must be allowed to manage access where the
    /// grant applies and cannot give anything to themselves.
    ///
    /// # Errors
    /// [`PermError::NotAllowed`], and the errors of [`Policy::give`].
    pub fn give_as(
        &mut self,
        giver: PrincipalId,
        grant: Grant,
        env: &Env<'_>,
    ) -> Result<(), PermError> {
        if grant.granted_by != giver {
            return Err(PermError::NotAllowed(
                "the grant must name the person giving it".to_owned(),
            ));
        }
        if grant.principal == giver || env.directory.members_of(grant.principal).contains(&giver) {
            return Err(PermError::NotAllowed(
                "nobody can give access to themselves".to_owned(),
            ));
        }
        let manage = Capability::Builtin(Action::AccessManage);
        let decision = self.decide(
            &Request {
                actor: Actor::direct(giver),
                capability: &manage,
                on: grant.on,
                record: None,
                prior: &[],
                now: grant.granted_at,
            },
            env,
        );
        if !decision.allowed {
            return Err(PermError::NotAllowed(format!(
                "{giver} may not manage access there"
            )));
        }
        self.give(grant, env.directory)
    }

    /// Revokes a grant. It stays in the policy as a record of what was given.
    ///
    /// # Errors
    /// [`PermError::UnknownGrant`], [`PermError::AlreadyRevoked`].
    pub fn revoke(&mut self, id: GrantId, by: PrincipalId, at: Timestamp) -> Result<(), PermError> {
        let grant = self
            .grants
            .get_mut(&id)
            .ok_or(PermError::UnknownGrant(id))?;
        if grant.revoked_at.is_some() {
            return Err(PermError::AlreadyRevoked(id));
        }
        grant.revoked_at = Some(at);
        grant.revoked_by = Some(by);
        Ok(())
    }

    /// Principals that hold both capabilities of a "hold" rule on overlapping resources:
    /// what an access review must resolve after a rule was added.
    #[must_use]
    pub fn violations(&self, now: Timestamp, directory: &dyn Directory) -> Vec<Violation> {
        let mut found = Vec::new();
        let mut principals: BTreeSet<PrincipalId> = BTreeSet::new();
        for grant in self.grants.values() {
            principals.extend(Self::reached(grant, directory));
        }
        for rule in self
            .rules
            .values()
            .filter(|rule| rule.kind == SodKind::Hold)
        {
            for principal in &principals {
                let firsts = self.holdings(*principal, &rule.first, now, directory);
                let seconds = self.holdings(*principal, &rule.second, now, directory);
                let pair = firsts.iter().find_map(|first| {
                    seconds
                        .iter()
                        .find(|second| first.on.overlaps(&second.on))
                        .map(|second| (first.id, second.id))
                });
                if let Some((first, second)) = pair {
                    found.push(Violation {
                        rule: rule.id,
                        principal: *principal,
                        first,
                        second,
                    });
                }
            }
        }
        found
    }

    /// The grants of one principal (and its groups) that give the capability, each either
    /// applied or refused with a reason.
    fn decide_for(
        &self,
        principal: PrincipalId,
        request: &Request<'_>,
        env: &Env<'_>,
    ) -> (Vec<Applied>, Vec<Refusal>, FieldAccess) {
        let refusal = |grant: Option<GrantId>, reason: Reason| Refusal {
            principal,
            grant,
            reason,
        };
        let Some(kind) = env.directory.kind(principal) else {
            return (
                Vec::new(),
                vec![refusal(None, Reason::UnknownPrincipal)],
                FieldAccess::none(),
            );
        };
        let mut holders = env.directory.groups_of(principal);
        holders.insert(principal);
        let mut applied = Vec::new();
        let mut refusals = Vec::new();
        let mut fields = FieldAccess::none();
        let mut considered = false;
        for holder in holders {
            for grant in self.grants_of(holder) {
                let Some((capability, through_set)) = self.gives(grant, request.capability) else {
                    continue;
                };
                considered = true;
                let reason = if let Err(why) = grant.status(request.now) {
                    Some(Reason::Inactive { why })
                } else if !grant.on.covers(&request.on) {
                    Some(Reason::Elsewhere)
                } else if kind == PrincipalKind::Guest && request.capability.is_administrative() {
                    Some(Reason::Administrative)
                } else {
                    match request
                        .record
                        .filter(|_| request.capability.is_record_scoped())
                    {
                        Some(record) => {
                            let table = request.on.table();
                            if !scope_admits(&grant.scope, principal, table, record, env) {
                                Some(Reason::OutOfScope)
                            } else if grant
                                .condition
                                .as_ref()
                                .is_some_and(|c| !holds(c, principal, record, env))
                            {
                                Some(Reason::ConditionNotMet)
                            } else {
                                None
                            }
                        }
                        None => None,
                    }
                };
                match reason {
                    Some(reason) => refusals.push(refusal(Some(grant.id), reason)),
                    None => {
                        fields = fields.union(&grant.fields);
                        applied.push(Applied {
                            grant: grant.id,
                            principal,
                            holder,
                            through_set,
                            capability,
                            on: grant.on,
                        });
                    }
                }
            }
        }
        if !considered {
            refusals.push(refusal(None, Reason::NoGrant));
        }
        (applied, refusals, fields)
    }

    /// Decides whether an actor may do something, and explains the answer.
    #[must_use]
    pub fn decide(&self, request: &Request<'_>, env: &Env<'_>) -> Decision {
        let mut decision = Decision {
            allowed: true,
            chain: Vec::new(),
            refusals: Vec::new(),
            fields: FieldAccess::All,
        };
        for principal in request.actor.principals() {
            let (applied, refusals, fields) = self.decide_for(principal, request, env);
            decision.allowed &= !applied.is_empty();
            decision.fields = decision.fields.intersection(&fields);
            decision.chain.extend(applied);
            decision.refusals.extend(refusals);
        }
        // What the same hands already did to this record.
        for rule in self
            .rules
            .values()
            .filter(|rule| rule.kind == SodKind::SameRecord)
        {
            let paired = rule
                .counterpart(request.capability)
                .is_some_and(|other| request.prior.contains(other));
            if paired {
                decision.allowed = false;
                decision.refusals.push(Refusal {
                    principal: request.actor.principal,
                    grant: None,
                    reason: Reason::SeparationOfDuties { rule: rule.id },
                });
            }
        }
        if !decision.allowed {
            decision.fields = FieldAccess::none();
        }
        decision
    }

    /// The records of a table an actor may do something to: the filter a list query must
    /// apply. It agrees with [`Policy::decide`] for every record (separation of duties
    /// aside, which depends on what was done to each record).
    #[must_use]
    pub fn row_filter(
        &self,
        actor: Actor,
        capability: &Capability,
        on: Resource,
        now: Timestamp,
        env: &Env<'_>,
    ) -> RowFilter {
        let parts = actor
            .principals()
            .into_iter()
            .map(|principal| {
                let request = Request {
                    actor: Actor::direct(principal),
                    capability,
                    on,
                    record: None,
                    prior: &[],
                    now,
                };
                let (applied, _, _) = self.decide_for(principal, &request, env);
                let rules: Vec<RowRule> = applied
                    .iter()
                    .filter_map(|link| self.grants.get(&link.grant))
                    .map(|grant| RowRule {
                        grant: grant.id,
                        scope: grant.scope.clone(),
                        condition: grant.condition.clone(),
                    })
                    .collect();
                let everything = !capability.is_record_scoped() && !rules.is_empty()
                    || rules
                        .iter()
                        .any(|rule| rule.scope == Scope::All && rule.condition.is_none());
                RowFilterPart {
                    principal,
                    everything,
                    rules: if everything { Vec::new() } else { rules },
                }
            })
            .collect();
        RowFilter {
            table: on.table(),
            parts,
        }
    }

    /// What an actor may do with each field of a record (or of the table, without one).
    ///
    /// A field's value is sent only if the actor may read the record, the grants that
    /// allow reading include the field, the actor is cleared for the field's
    /// classification, and holds the capability its field group requires. A masked field
    /// is sent in clear only to those who may unmask. Editing additionally needs the right
    /// to update the record and the field, and the group's edit capability; generated
    /// values are never editable.
    #[must_use]
    pub fn field_permissions(
        &self,
        actor: Actor,
        on: Resource,
        record: Option<&dyn RecordFacts>,
        now: Timestamp,
        env: &Env<'_>,
        fields: &[FieldMeta],
    ) -> BTreeMap<FieldId, FieldPermission> {
        let mut decisions: BTreeMap<Capability, Decision> = BTreeMap::new();
        let mut decide = |capability: Capability| -> Decision {
            decisions
                .entry(capability.clone())
                .or_insert_with(|| {
                    self.decide(
                        &Request {
                            actor,
                            capability: &capability,
                            on,
                            record,
                            prior: &[],
                            now,
                        },
                        env,
                    )
                })
                .clone()
        };
        let read = decide(Action::RecordRead.into());
        let update = decide(Action::RecordUpdate.into());
        fields
            .iter()
            .map(|field| {
                let cleared = match field.classification {
                    Classification::Public | Classification::Internal => true,
                    Classification::Confidential => decide(Action::DataConfidential.into()).allowed,
                    Classification::Personal => decide(Action::DataPersonal.into()).allowed,
                    Classification::SensitivePersonal => {
                        decide(Action::DataSensitive.into()).allowed
                    }
                };
                let in_group =
                    |capability: &Option<Capability>,
                     decide: &mut dyn FnMut(Capability) -> Decision| {
                        capability
                            .as_ref()
                            .is_none_or(|capability| decide(capability.clone()).allowed)
                    };
                let readable = read.allowed
                    && read.fields.allows(field.id)
                    && cleared
                    && in_group(&field.view_capability, &mut decide);
                let visibility = if !readable {
                    Visibility::Hidden
                } else if field.masked && !decide(Action::DataUnmask.into()).allowed {
                    Visibility::Masked
                } else {
                    Visibility::Visible
                };
                let editable = visibility == Visibility::Visible
                    && !field.generated
                    && update.allowed
                    && update.fields.allows(field.id)
                    && in_group(&field.edit_capability, &mut decide);
                (
                    field.id,
                    FieldPermission {
                        visibility,
                        editable,
                    },
                )
            })
            .collect()
    }

    /// Every capability a principal holds somewhere at or above a place through grants in
    /// force — what the place offers them, whatever the record. For menus, and for access
    /// reviews.
    #[must_use]
    pub fn capabilities(
        &self,
        principal: PrincipalId,
        on: Resource,
        now: Timestamp,
        directory: &dyn Directory,
    ) -> BTreeSet<Capability> {
        let Some(kind) = directory.kind(principal) else {
            return BTreeSet::new();
        };
        let mut holders = directory.groups_of(principal);
        holders.insert(principal);
        let mut held = BTreeSet::new();
        for grant in holders
            .into_iter()
            .flat_map(|holder| self.grants_of(holder))
        {
            if grant.status(now).is_err() || !grant.on.covers(&on) {
                continue;
            }
            for (capability, _) in self.given(grant) {
                held.insert(capability.clone());
                if let Capability::Builtin(action) = capability {
                    held.extend(action.includes().iter().map(|a| Capability::Builtin(*a)));
                }
            }
        }
        if kind == PrincipalKind::Guest {
            held.retain(|capability| !capability.is_administrative());
        }
        held
    }
}

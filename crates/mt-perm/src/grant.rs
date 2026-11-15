//! Grants: who may do what, where, to which records, under which condition, and until when.
//!
//! A [`Grant`] gives a principal (a person, guest, agent, API app, or a group of them) a
//! capability or a whole [`CapabilitySet`] on a [`Resource`]. For actions on records it can
//! be narrowed by a [`Scope`] (which records), a condition (for example an amount limit)
//! and a [`FieldAccess`] (which fields). It always records who gave it and why.
//!
//! There are no "deny" rules: access is the sum of what was granted, which is what makes
//! every decision explainable. What must never be combined is expressed by a [`SodRule`].

use crate::capability::Capability;
use crate::error::PermError;
use mt_core::condition::{Condition, MAX_CONDITION_DEPTH};
use mt_core::field::DataType;
use mt_core::id::{
    BaseId, CapabilitySetId, FieldId, GrantId, PrincipalId, RecordId, RefItemId, RefSetId,
    SodRuleId, TableId, WorkspaceId,
};
use mt_core::limits::MAX_FILTER_CONDITIONS;
use mt_core::time::Timestamp;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Maximum number of characters in the reason of a grant.
pub const MAX_REASON_CHARS: usize = 500;
/// Maximum number of records in a [`Scope::Records`].
pub const MAX_SCOPE_RECORDS: usize = 10_000;
/// Maximum number of capabilities in a capability set.
pub const MAX_SET_CAPABILITIES: usize = 200;
/// Maximum number of characters in the name of a capability set or rule.
pub const MAX_NAME_CHARS: usize = 255;

/// Where a grant applies, or where an action happens: the tenant, a workspace, a base or a
/// table. Each level names its ancestors, so that "does this cover that" needs no lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "level", rename_all = "snake_case")]
pub enum Resource {
    /// The whole tenant.
    Tenant,
    /// A workspace and everything in it.
    Workspace {
        /// The workspace.
        workspace: WorkspaceId,
    },
    /// A base and its tables.
    Base {
        /// The workspace the base is in.
        workspace: WorkspaceId,
        /// The base.
        base: BaseId,
    },
    /// One table.
    Table {
        /// The workspace the base is in.
        workspace: WorkspaceId,
        /// The base the table is in.
        base: BaseId,
        /// The table.
        table: TableId,
    },
}

impl Resource {
    /// Whether this resource is `other` or contains it.
    #[must_use]
    pub fn covers(&self, other: &Resource) -> bool {
        match (*self, *other) {
            (Resource::Tenant, _) => true,
            (Resource::Workspace { workspace }, inner) => inner.workspace() == Some(workspace),
            (Resource::Base { workspace, base }, inner) => {
                inner.workspace() == Some(workspace) && inner.base() == Some(base)
            }
            (Resource::Table { .. }, inner) => *self == inner,
        }
    }

    /// Whether one of the two resources contains the other.
    #[must_use]
    pub fn overlaps(&self, other: &Resource) -> bool {
        self.covers(other) || other.covers(self)
    }

    /// The workspace of the resource, unless it is the tenant.
    #[must_use]
    pub const fn workspace(&self) -> Option<WorkspaceId> {
        match self {
            Resource::Tenant => None,
            Resource::Workspace { workspace }
            | Resource::Base { workspace, .. }
            | Resource::Table { workspace, .. } => Some(*workspace),
        }
    }

    /// The base of the resource, if it is a base or a table.
    #[must_use]
    pub const fn base(&self) -> Option<BaseId> {
        match self {
            Resource::Tenant | Resource::Workspace { .. } => None,
            Resource::Base { base, .. } | Resource::Table { base, .. } => Some(*base),
        }
    }

    /// The table, if the resource is one.
    #[must_use]
    pub const fn table(&self) -> Option<TableId> {
        match self {
            Resource::Tenant | Resource::Workspace { .. } | Resource::Base { .. } => None,
            Resource::Table { table, .. } => Some(*table),
        }
    }
}

/// Which records a grant applies to.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum Scope {
    /// Every record.
    #[default]
    All,
    /// Records the principal owns: named in one of the table's ownership fields (owner,
    /// assignee, requester, creator).
    Own,
    /// Records owned by the principal or by one of the principal's groups.
    Team,
    /// Records for which a condition holds; `me` in the condition is the principal.
    Where {
        /// The condition.
        condition: Condition,
    },
    /// Records whose reference field holds an item within a subtree of a reference set
    /// ("region is in EMEA").
    RefSubtree {
        /// The reference field of the table.
        field: FieldId,
        /// The reference set.
        set: RefSetId,
        /// The root of the subtree (included).
        root: RefItemId,
    },
    /// These records.
    Records {
        /// The records.
        records: BTreeSet<RecordId>,
    },
}

/// Which fields a grant applies to.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "fields", rename_all = "snake_case")]
pub enum FieldAccess {
    /// Every field.
    #[default]
    All,
    /// Only these fields.
    Only {
        /// The fields.
        only: BTreeSet<FieldId>,
    },
    /// Every field except these.
    Except {
        /// The fields.
        except: BTreeSet<FieldId>,
    },
}

impl FieldAccess {
    /// No field at all.
    #[must_use]
    pub const fn none() -> FieldAccess {
        FieldAccess::Only {
            only: BTreeSet::new(),
        }
    }

    /// Only these fields.
    #[must_use]
    pub fn only(fields: impl IntoIterator<Item = FieldId>) -> FieldAccess {
        FieldAccess::Only {
            only: fields.into_iter().collect(),
        }
    }

    /// Every field except these.
    #[must_use]
    pub fn except(fields: impl IntoIterator<Item = FieldId>) -> FieldAccess {
        FieldAccess::Except {
            except: fields.into_iter().collect(),
        }
        .simplified()
    }

    /// "Every field except none" is every field.
    fn simplified(self) -> FieldAccess {
        match self {
            FieldAccess::Except { except } if except.is_empty() => FieldAccess::All,
            FieldAccess::All | FieldAccess::Only { .. } | FieldAccess::Except { .. } => self,
        }
    }

    /// Whether the field is included.
    #[must_use]
    pub fn allows(&self, field: FieldId) -> bool {
        match self {
            FieldAccess::All => true,
            FieldAccess::Only { only } => only.contains(&field),
            FieldAccess::Except { except } => !except.contains(&field),
        }
    }

    /// Whether no field is included.
    #[must_use]
    pub fn is_none(&self) -> bool {
        matches!(self, FieldAccess::Only { only } if only.is_empty())
    }

    /// The fields included in either (two grants add up).
    #[must_use]
    pub fn union(&self, other: &FieldAccess) -> FieldAccess {
        match (self, other) {
            (FieldAccess::All, _) | (_, FieldAccess::All) => FieldAccess::All,
            (FieldAccess::Only { only: a }, FieldAccess::Only { only: b }) => FieldAccess::Only {
                only: a.union(b).copied().collect(),
            },
            (FieldAccess::Except { except: a }, FieldAccess::Except { except: b }) => {
                FieldAccess::Except {
                    except: a.intersection(b).copied().collect(),
                }
            }
            (FieldAccess::Only { only }, FieldAccess::Except { except })
            | (FieldAccess::Except { except }, FieldAccess::Only { only }) => FieldAccess::Except {
                except: except.difference(only).copied().collect(),
            },
        }
        .simplified()
    }

    /// The fields included in both (an agent acting for a person sees what both may see).
    #[must_use]
    pub fn intersection(&self, other: &FieldAccess) -> FieldAccess {
        match (self, other) {
            (FieldAccess::All, x) | (x, FieldAccess::All) => x.clone(),
            (FieldAccess::Only { only: a }, FieldAccess::Only { only: b }) => FieldAccess::Only {
                only: a.intersection(b).copied().collect(),
            },
            (FieldAccess::Except { except: a }, FieldAccess::Except { except: b }) => {
                FieldAccess::Except {
                    except: a.union(b).copied().collect(),
                }
            }
            (FieldAccess::Only { only }, FieldAccess::Except { except })
            | (FieldAccess::Except { except }, FieldAccess::Only { only }) => FieldAccess::Only {
                only: only.difference(except).copied().collect(),
            },
        }
    }
}

/// What a grant gives: one capability, or every capability of a set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Granted {
    /// One capability.
    Capability(Capability),
    /// Every capability of a capability set, as the set is defined at decision time.
    Set(CapabilitySetId),
}

/// Why a grant is not in force at some instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Inactive {
    /// It was revoked.
    Revoked,
    /// Its validity has not started.
    NotYetValid,
    /// Its validity has ended.
    Expired,
}

/// A capability (or capability set) given to a principal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// Stable ID.
    pub id: GrantId,
    /// Who receives it: a person, guest, agent, API app, or a group.
    pub principal: PrincipalId,
    /// What is given.
    pub what: Granted,
    /// Where it applies.
    pub on: Resource,
    /// Which records it applies to (actions on records only).
    #[serde(default)]
    pub scope: Scope,
    /// A condition the record must also satisfy, typically a limit ("amount ≤ 10 000").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<Condition>,
    /// Which fields it applies to (reading and changing records only).
    #[serde(default)]
    pub fields: FieldAccess,
    /// In force from this instant (inclusive); from the start, if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<Timestamp>,
    /// In force until this instant (exclusive); without end, if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<Timestamp>,
    /// Who gave it.
    pub granted_by: PrincipalId,
    /// When it was given.
    pub granted_at: Timestamp,
    /// Why it was given.
    pub reason: String,
    /// When it was revoked, if it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<Timestamp>,
    /// Who revoked it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_by: Option<PrincipalId>,
}

impl Grant {
    /// A grant for every record and field, without condition or end.
    #[must_use]
    pub fn new(
        id: GrantId,
        principal: PrincipalId,
        what: Granted,
        on: Resource,
        granted_by: PrincipalId,
        granted_at: Timestamp,
        reason: impl Into<String>,
    ) -> Grant {
        Grant {
            id,
            principal,
            what,
            on,
            scope: Scope::All,
            condition: None,
            fields: FieldAccess::All,
            valid_from: None,
            valid_to: None,
            granted_by,
            granted_at,
            reason: reason.into(),
            revoked_at: None,
            revoked_by: None,
        }
    }

    /// Whether the grant is in force at `now`.
    ///
    /// # Errors
    /// Why it is not.
    pub fn status(&self, now: Timestamp) -> Result<(), Inactive> {
        if self.revoked_at.is_some_and(|at| at <= now) {
            return Err(Inactive::Revoked);
        }
        if self.valid_from.is_some_and(|from| now < from) {
            return Err(Inactive::NotYetValid);
        }
        if self.valid_to.is_some_and(|to| now >= to) {
            return Err(Inactive::Expired);
        }
        Ok(())
    }

    /// Whether the grant narrows its capability to some records or fields.
    #[must_use]
    pub fn is_narrowed(&self) -> bool {
        self.scope != Scope::All || self.condition.is_some() || self.fields != FieldAccess::All
    }

    /// Verifies the grant by itself.
    ///
    /// # Errors
    /// [`PermError::Invalid`] describing the first problem found.
    pub fn check(&self) -> Result<(), PermError> {
        if self.reason.trim().is_empty() {
            return Err(PermError::invalid("a grant needs a reason"));
        }
        if self.reason.chars().count() > MAX_REASON_CHARS {
            return Err(PermError::invalid(format!(
                "the reason is longer than {MAX_REASON_CHARS} characters"
            )));
        }
        if let (Some(from), Some(to)) = (self.valid_from, self.valid_to) {
            if from >= to {
                return Err(PermError::invalid("the grant ends before it starts"));
            }
        }
        if let Granted::Capability(capability) = &self.what {
            if !capability.is_record_scoped() && self.is_narrowed() {
                return Err(PermError::invalid(format!(
                    "`{capability}` is not about individual records; it cannot be limited to some records or fields"
                )));
            }
        }
        // Conditions and field lists name fields of one table.
        let names_fields = match &self.scope {
            Scope::All | Scope::Own | Scope::Team => false,
            Scope::Where { .. } | Scope::RefSubtree { .. } | Scope::Records { .. } => true,
        } || self.condition.is_some()
            || self.fields != FieldAccess::All;
        if names_fields && self.on.table().is_none() {
            return Err(PermError::invalid(
                "a grant limited to some records or fields must be given on one table",
            ));
        }
        if let Scope::Records { records } = &self.scope {
            if records.is_empty() || records.len() > MAX_SCOPE_RECORDS {
                return Err(PermError::invalid(format!(
                    "a record list has between 1 and {MAX_SCOPE_RECORDS} records"
                )));
            }
        }
        if self.fields.is_none() {
            return Err(PermError::invalid("the grant includes no field at all"));
        }
        let conditions = [
            match &self.scope {
                Scope::Where { condition } => Some(condition),
                Scope::All
                | Scope::Own
                | Scope::Team
                | Scope::RefSubtree { .. }
                | Scope::Records { .. } => None,
            },
            self.condition.as_ref(),
        ];
        for condition in conditions.into_iter().flatten() {
            if condition.depth() > MAX_CONDITION_DEPTH
                || condition.test_count() > MAX_FILTER_CONDITIONS
            {
                return Err(PermError::invalid(format!(
                    "a condition has at most {MAX_FILTER_CONDITIONS} tests and {MAX_CONDITION_DEPTH} levels"
                )));
            }
        }
        Ok(())
    }

    /// Verifies the parts of the grant that name fields against the table it is given on.
    /// `type_of` returns the data type of a field of that table.
    ///
    /// # Errors
    /// [`PermError::Invalid`] for a condition that does not fit the fields, a subtree scope
    /// on a field that is not a reference, or a field list naming unknown fields.
    pub fn check_in_table(
        &self,
        type_of: &dyn Fn(FieldId) -> Option<DataType>,
    ) -> Result<(), PermError> {
        let invalid = |error: mt_core::CoreError| PermError::invalid(error.to_string());
        match &self.scope {
            Scope::Where { condition } => condition.check(type_of).map_err(invalid)?,
            Scope::RefSubtree { field, .. } => match type_of(*field) {
                Some(DataType::Reference | DataType::MultiReference) => {}
                Some(other) => {
                    return Err(PermError::invalid(format!(
                        "a subtree scope needs a reference field; {field} is a {other} field"
                    )));
                }
                None => {
                    return Err(PermError::invalid(format!(
                        "{field} is not a field of the table"
                    )));
                }
            },
            Scope::All | Scope::Own | Scope::Team | Scope::Records { .. } => {}
        }
        if let Some(condition) = &self.condition {
            condition.check(type_of).map_err(invalid)?;
        }
        let listed = match &self.fields {
            FieldAccess::All => None,
            FieldAccess::Only { only } => Some(only),
            FieldAccess::Except { except } => Some(except),
        };
        for field in listed.into_iter().flatten() {
            if type_of(*field).is_none() {
                return Err(PermError::invalid(format!(
                    "{field} is not a field of the table"
                )));
            }
        }
        Ok(())
    }
}

fn check_name(what: &str, name: &str) -> Result<(), PermError> {
    if name.trim().is_empty() {
        return Err(PermError::invalid(format!("{what} needs a name")));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(PermError::invalid(format!(
            "the name of {what} is longer than {MAX_NAME_CHARS} characters"
        )));
    }
    Ok(())
}

/// A named set of capabilities that are given together ("Requester", "Approver up to
/// 10 000", "Base builder"). Named after what the holder can do, not after a job title.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySet {
    /// Stable ID.
    pub id: CapabilitySetId,
    /// Name.
    pub name: String,
    /// What the set is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The capabilities.
    pub capabilities: BTreeSet<Capability>,
}

impl CapabilitySet {
    /// Verifies the set: a name and between 1 and [`MAX_SET_CAPABILITIES`] capabilities.
    ///
    /// # Errors
    /// [`PermError::Invalid`].
    pub fn check(&self) -> Result<(), PermError> {
        check_name("a capability set", &self.name)?;
        if self.capabilities.is_empty() || self.capabilities.len() > MAX_SET_CAPABILITIES {
            return Err(PermError::invalid(format!(
                "a capability set has between 1 and {MAX_SET_CAPABILITIES} capabilities"
            )));
        }
        Ok(())
    }
}

/// How two capabilities must be kept apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SodKind {
    /// No principal may hold both (on overlapping resources): whoever maintains suppliers
    /// cannot also release payments.
    Hold,
    /// A principal may hold both, but not use both on the same record: whoever created a
    /// request cannot approve it.
    SameRecord,
}

/// A separation-of-duties rule: two capabilities that must not come together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SodRule {
    /// Stable ID.
    pub id: SodRuleId,
    /// Name ("Four eyes on purchase requests").
    pub name: String,
    /// One capability.
    pub first: Capability,
    /// The other capability.
    pub second: Capability,
    /// How they must be kept apart.
    pub kind: SodKind,
}

impl SodRule {
    /// Verifies the rule: a name and two different capabilities.
    ///
    /// # Errors
    /// [`PermError::Invalid`].
    pub fn check(&self) -> Result<(), PermError> {
        check_name("a separation-of-duties rule", &self.name)?;
        if self.first == self.second {
            return Err(PermError::invalid(
                "a separation-of-duties rule needs two different capabilities",
            ));
        }
        Ok(())
    }

    /// The capability the rule pairs with `capability`, if the rule mentions it.
    #[must_use]
    pub fn counterpart(&self, capability: &Capability) -> Option<&Capability> {
        if *capability == self.first {
            Some(&self.second)
        } else if *capability == self.second {
            Some(&self.first)
        } else {
            None
        }
    }
}

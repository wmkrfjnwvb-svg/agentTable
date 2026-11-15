//! Capabilities: the actions that can be allowed.
//!
//! A capability is *an allowed action on a business object* — "read records", "approve",
//! "manage the schema". The platform has a fixed list of built-in [`Action`]s; a base adds
//! its own [`Capability::Custom`] ones for lifecycle transitions, field groups, buttons and
//! API operations (`transition:approve_po`, `salary.view`).
//!
//! Capabilities name what can be done, never who does it: there are no job titles here.

use crate::error::PermError;
use core::fmt;
use core::str::FromStr;
use mt_core::field::is_capability_key;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

macro_rules! actions {
    ( $( $(#[$doc:meta])* $variant:ident => $key:literal, )+ ) => {
        /// A built-in action.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Action {
            $( $(#[$doc])* $variant, )+
        }

        impl Action {
            /// Every built-in action.
            pub const ALL: &'static [Action] = &[ $( Action::$variant, )+ ];

            /// The stable key (also the stored form).
            #[must_use]
            pub const fn key(self) -> &'static str {
                match self {
                    $( Action::$variant => $key, )+
                }
            }
        }
    };
}

actions! {
    /// See records.
    RecordRead => "record.read",
    /// Add records.
    RecordCreate => "record.create",
    /// Change records.
    RecordUpdate => "record.update",
    /// Delete records.
    RecordDelete => "record.delete",
    /// Comment on records.
    RecordComment => "record.comment",
    /// Export records to a file.
    RecordExport => "record.export",
    /// Import records from a file.
    RecordImport => "record.import",
    /// See fields classified as confidential.
    DataConfidential => "data.confidential",
    /// See fields classified as personal data.
    DataPersonal => "data.personal",
    /// See fields classified as sensitive personal data.
    DataSensitive => "data.sensitive",
    /// See masked values in clear.
    DataUnmask => "data.unmask",
    /// Decide approvals.
    ProcessApprove => "process.approve",
    /// Create and change shared views.
    ViewManage => "view.manage",
    /// Change tables, fields, lifecycles and forms.
    SchemaManage => "schema.manage",
    /// Create and change rules and workflows.
    AutomationManage => "automation.manage",
    /// Run AI skills.
    SkillRun => "skill.run",
    /// Start agents.
    AgentRun => "agent.run",
    /// Create share links and invite guests.
    ShareManage => "share.manage",
    /// Give and revoke grants, define capability sets and separation-of-duties rules.
    AccessManage => "access.manage",
    /// Add and remove members and groups.
    MembersManage => "members.manage",
    /// Change reference data.
    RefDataManage => "refdata.manage",
    /// Change settings and parameters.
    SettingsManage => "settings.manage",
    /// Install and upgrade templates.
    TemplateManage => "template.manage",
    /// Read the audit trail.
    AuditRead => "audit.read",
}

impl Action {
    /// Whether the action is done to individual records, so that a grant's scope and
    /// condition decide *which* records. The other actions are done to a table, base,
    /// workspace or the tenant as a whole.
    #[must_use]
    pub const fn is_record_scoped(self) -> bool {
        match self {
            Action::RecordRead
            | Action::RecordCreate
            | Action::RecordUpdate
            | Action::RecordDelete
            | Action::RecordComment
            | Action::RecordExport
            | Action::DataConfidential
            | Action::DataPersonal
            | Action::DataSensitive
            | Action::DataUnmask
            | Action::ProcessApprove
            | Action::SkillRun => true,
            Action::RecordImport
            | Action::ViewManage
            | Action::SchemaManage
            | Action::AutomationManage
            | Action::AgentRun
            | Action::ShareManage
            | Action::AccessManage
            | Action::MembersManage
            | Action::RefDataManage
            | Action::SettingsManage
            | Action::TemplateManage
            | Action::AuditRead => false,
        }
    }

    /// Whether the action administers the platform. Guests and share links can never be
    /// given such an action.
    #[must_use]
    pub const fn is_administrative(self) -> bool {
        match self {
            Action::RecordImport
            | Action::DataUnmask
            | Action::ViewManage
            | Action::SchemaManage
            | Action::AutomationManage
            | Action::ShareManage
            | Action::AccessManage
            | Action::MembersManage
            | Action::RefDataManage
            | Action::SettingsManage
            | Action::TemplateManage
            | Action::AuditRead => true,
            Action::RecordRead
            | Action::RecordCreate
            | Action::RecordUpdate
            | Action::RecordDelete
            | Action::RecordComment
            | Action::RecordExport
            | Action::DataConfidential
            | Action::DataPersonal
            | Action::DataSensitive
            | Action::ProcessApprove
            | Action::SkillRun
            | Action::AgentRun => false,
        }
    }

    /// The actions that having this one includes: whoever may change, delete, comment on,
    /// export or approve a record may also see it.
    #[must_use]
    pub const fn includes(self) -> &'static [Action] {
        match self {
            Action::RecordUpdate
            | Action::RecordDelete
            | Action::RecordComment
            | Action::RecordExport
            | Action::ProcessApprove => &[Action::RecordRead],
            Action::SchemaManage => &[Action::ViewManage],
            Action::RecordRead
            | Action::RecordCreate
            | Action::RecordImport
            | Action::DataConfidential
            | Action::DataPersonal
            | Action::DataSensitive
            | Action::DataUnmask
            | Action::ViewManage
            | Action::AutomationManage
            | Action::SkillRun
            | Action::AgentRun
            | Action::ShareManage
            | Action::AccessManage
            | Action::MembersManage
            | Action::RefDataManage
            | Action::SettingsManage
            | Action::TemplateManage
            | Action::AuditRead => &[],
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

/// An action that can be allowed: a built-in one, or one a base defines.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    /// A built-in action.
    Builtin(Action),
    /// A capability defined by a base, by its key.
    Custom(String),
}

impl Capability {
    /// A custom capability.
    ///
    /// # Errors
    /// [`PermError::Invalid`] if the key is not well formed or is the key of a built-in
    /// action.
    pub fn custom(key: &str) -> Result<Capability, PermError> {
        match key.parse::<Capability>()? {
            Capability::Builtin(action) => Err(PermError::invalid(format!(
                "`{action}` is a built-in capability"
            ))),
            custom @ Capability::Custom(_) => Ok(custom),
        }
    }

    /// The stable key.
    #[must_use]
    pub fn key(&self) -> &str {
        match self {
            Capability::Builtin(action) => action.key(),
            Capability::Custom(key) => key,
        }
    }

    /// Whether a grant's scope and condition decide which records the capability applies
    /// to. Custom capabilities (transitions, buttons, field groups) always act on records.
    #[must_use]
    pub const fn is_record_scoped(&self) -> bool {
        match self {
            Capability::Builtin(action) => action.is_record_scoped(),
            Capability::Custom(_) => true,
        }
    }

    /// Whether the capability administers the platform.
    #[must_use]
    pub const fn is_administrative(&self) -> bool {
        match self {
            Capability::Builtin(action) => action.is_administrative(),
            Capability::Custom(_) => false,
        }
    }

    /// Whether holding this capability includes `other` (itself, or a built-in inclusion
    /// such as "update includes read").
    #[must_use]
    pub fn includes(&self, other: &Capability) -> bool {
        if self == other {
            return true;
        }
        match (self, other) {
            (Capability::Builtin(held), Capability::Builtin(wanted)) => {
                held.includes().contains(wanted)
            }
            (Capability::Builtin(_) | Capability::Custom(_), _) => false,
        }
    }
}

impl From<Action> for Capability {
    fn from(action: Action) -> Self {
        Capability::Builtin(action)
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

impl FromStr for Capability {
    type Err = PermError;

    fn from_str(key: &str) -> Result<Self, Self::Err> {
        if let Some(action) = Action::ALL.iter().find(|action| action.key() == key) {
            return Ok(Capability::Builtin(*action));
        }
        if is_capability_key(key) {
            Ok(Capability::Custom(key.to_owned()))
        } else {
            Err(PermError::invalid(format!(
                "`{}` is not a capability key: use lower-case letters, digits, `.`, `_`, `-` and `:`, starting with a letter",
                key.chars().take(64).collect::<String>()
            )))
        }
    }
}

impl Serialize for Capability {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.key())
    }
}

impl<'de> Deserialize<'de> for Capability {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let key = <std::borrow::Cow<'de, str> as Deserialize>::deserialize(deserializer)?;
        key.parse().map_err(serde::de::Error::custom)
    }
}

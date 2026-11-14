//! The parameter registry: typed settings with a scope hierarchy and an explanation of where
//! each value comes from (product design doc 07 §3).
//!
//! A parameter is defined once ([`ParamDef`]): a namespaced key, a type with validation, an
//! optional default and the levels at which it may be overridden. Values are then set at a
//! [`Scope`]. Resolution takes the most specific value that applies:
//!
//! ```text
//! user override  →  base override  →  tenant override  →  default  →  (unset)
//! ```
//!
//! and [`Resolved::explain`] says which one it took, who set it and when.
//!
//! * **Typing.** A parameter's type is a field data type with the field framework's
//!   validation (`mt-core`), so a setting is checked exactly like a cell: ranges, patterns,
//!   and membership of a reference set for choice parameters.
//! * **Secrets** are write-only. The registry never holds a secret's content, only an
//!   opaque [`SecretRef`] to the encrypted store, and never puts even that into an
//!   explanation.
//! * **Feature flags** are boolean parameters ([`Registry::flag`]).
//! * **Option lists.** A multi-choice parameter can feed the options of a field
//!   (`OptionsSource::Parameter` in `mt-core`); [`Registry::bind`] gives the
//!   [`ParameterOptions`] for that.
//! * Every change is kept as history per key and scope; clearing an override is a change too.

use crate::effective::ParameterOptions;
use crate::error::RefDataError;
use crate::model::{clip, MAX_TEXT_CHARS};
use mt_core::decimal::Decimal;
use mt_core::field::{Classification, DataType, FieldDef, OptionsSource, ValidationSpec};
use mt_core::id::{BaseId, FieldId, PrincipalId, RefItemId, RefSetId};
use mt_core::time::{DurationMs, Timestamp};
use mt_core::validate::{validate_value, OptionResolver};
use mt_core::value::Value;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Maximum number of characters in a parameter key.
pub const MAX_KEY_CHARS: usize = 128;
/// Maximum number of registered parameters.
pub const MAX_PARAMETERS: usize = 10_000;
/// Maximum size of a JSON parameter value, serialised, in bytes.
pub const MAX_JSON_BYTES: usize = 65_536;
/// Number of past entries kept per key and scope (the audit log keeps everything).
pub const MAX_HISTORY: usize = 1_000;

/// A level of the scope hierarchy at which a parameter may be overridden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeLevel {
    /// For the whole tenant.
    Tenant,
    /// For one base.
    Base,
    /// For one person (preferences).
    User,
}

impl ScopeLevel {
    /// The level's name in messages.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            ScopeLevel::Tenant => "tenant",
            ScopeLevel::Base => "base",
            ScopeLevel::User => "user",
        }
    }
}

/// Where an override lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "level", rename_all = "snake_case")]
pub enum Scope {
    /// The whole tenant.
    Tenant,
    /// One base.
    Base {
        /// The base.
        base: BaseId,
    },
    /// One person.
    User {
        /// The person.
        user: PrincipalId,
    },
}

impl Scope {
    /// The level of this scope.
    #[must_use]
    pub const fn level(self) -> ScopeLevel {
        match self {
            Scope::Tenant => ScopeLevel::Tenant,
            Scope::Base { .. } => ScopeLevel::Base,
            Scope::User { .. } => ScopeLevel::User,
        }
    }
}

/// For whom a parameter is resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ParamContext {
    /// The base the request is about, if any.
    pub base: Option<BaseId>,
    /// The person the request is for, if any.
    pub user: Option<PrincipalId>,
}

/// An opaque handle to a secret kept in the encrypted secret store. It is not the secret.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretRef(String);

impl SecretRef {
    /// Wraps a handle issued by the secret store.
    ///
    /// # Errors
    /// [`RefDataError::Invalid`] for an empty or over-long handle.
    pub fn new(handle: impl Into<String>) -> Result<Self, RefDataError> {
        let handle = handle.into();
        if handle.is_empty() || handle.len() > MAX_TEXT_CHARS {
            return Err(RefDataError::invalid(
                "secret handle",
                "must be 1 to 255 bytes",
            ));
        }
        Ok(SecretRef(handle))
    }

    /// The handle, for the secret store only.
    #[must_use]
    pub fn handle(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for SecretRef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SecretRef(<redacted>)")
    }
}

/// What kind of value a parameter holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParamKind {
    /// A typed value, validated like a field.
    Value {
        /// The data type.
        data_type: DataType,
        /// For choice parameters: the reference set the value comes from.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        set: Option<RefSetId>,
        /// Validation rules (range, length, pattern, decimals, item counts).
        #[serde(default)]
        validation: Box<ValidationSpec>,
    },
    /// A secret: write-only, held in the secret store.
    Secret,
}

/// The definition of a parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamDef {
    /// Namespaced, immutable key: `approval.default_timeout_hours`.
    pub key: String,
    /// What the parameter controls, for the admin console.
    pub description: String,
    /// The kind of value.
    pub kind: ParamKind,
    /// The value when nothing overrides it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// The levels at which it may be overridden.
    pub overridable_at: BTreeSet<ScopeLevel>,
    /// How sensitive the value is.
    pub sensitivity: Classification,
    /// Whether a change needs approval (enforced by the caller's change process).
    #[serde(default)]
    pub critical: bool,
    /// The person accountable for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<PrincipalId>,
}

impl ParamDef {
    fn of(key: &str, description: &str, data_type: DataType, set: Option<RefSetId>) -> Self {
        ParamDef {
            key: key.to_owned(),
            description: description.to_owned(),
            kind: ParamKind::Value {
                data_type,
                set,
                validation: Box::default(),
            },
            default: None,
            overridable_at: BTreeSet::from([ScopeLevel::Tenant]),
            sensitivity: Classification::Internal,
            critical: false,
            owner: None,
        }
    }

    /// A text parameter, overridable per tenant.
    #[must_use]
    pub fn text(key: &str, description: &str) -> Self {
        Self::of(key, description, DataType::Text, None)
    }

    /// A whole-number parameter.
    #[must_use]
    pub fn integer(key: &str, description: &str) -> Self {
        let mut def = Self::of(key, description, DataType::Number, None);
        if let ParamKind::Value { validation, .. } = &mut def.kind {
            validation.max_decimals = Some(0);
        }
        def
    }

    /// A decimal-number parameter.
    #[must_use]
    pub fn decimal(key: &str, description: &str) -> Self {
        Self::of(key, description, DataType::Number, None)
    }

    /// A yes/no parameter (a feature flag), off unless set.
    #[must_use]
    pub fn flag(key: &str, description: &str) -> Self {
        Self::of(key, description, DataType::Bool, None)
    }

    /// A duration parameter.
    #[must_use]
    pub fn duration(key: &str, description: &str) -> Self {
        Self::of(key, description, DataType::Duration, None)
    }

    /// A parameter whose value is one item of a reference set.
    #[must_use]
    pub fn choice(key: &str, description: &str, set: RefSetId) -> Self {
        Self::of(key, description, DataType::Reference, Some(set))
    }

    /// A parameter whose value is a list of items of a reference set. It can feed the
    /// options of a field.
    #[must_use]
    pub fn choices(key: &str, description: &str, set: RefSetId) -> Self {
        Self::of(key, description, DataType::MultiReference, Some(set))
    }

    /// A free-form JSON parameter.
    #[must_use]
    pub fn json(key: &str, description: &str) -> Self {
        Self::of(key, description, DataType::Json, None)
    }

    /// A secret, overridable per tenant.
    #[must_use]
    pub fn secret(key: &str, description: &str) -> Self {
        ParamDef {
            kind: ParamKind::Secret,
            sensitivity: Classification::Confidential,
            ..Self::of(key, description, DataType::Text, None)
        }
    }

    /// Sets the default value.
    #[must_use]
    pub fn with_default(mut self, value: Value) -> Self {
        self.default = Some(value);
        self
    }

    /// Sets the inclusive numeric range (numbers; durations in milliseconds).
    #[must_use]
    pub fn with_range(mut self, min: Option<Decimal>, max: Option<Decimal>) -> Self {
        if let ParamKind::Value { validation, .. } = &mut self.kind {
            validation.min = min;
            validation.max = max;
        }
        self
    }

    /// Sets the levels at which the parameter may be overridden.
    #[must_use]
    pub fn overridable(mut self, levels: &[ScopeLevel]) -> Self {
        self.overridable_at = levels.iter().copied().collect();
        self
    }

    /// Whether the parameter is a secret.
    #[must_use]
    pub const fn is_secret(&self) -> bool {
        matches!(self.kind, ParamKind::Secret)
    }

    /// The field definition that validates values of a typed parameter.
    fn field_for(
        key: &str,
        data_type: DataType,
        set: Option<RefSetId>,
        validation: &ValidationSpec,
    ) -> FieldDef {
        let mut field = FieldDef::new(FieldId::from_u128(0), key, data_type);
        field.validation = validation.clone();
        if let Some(set) = set {
            field.options = OptionsSource::of_set(set);
        }
        field
    }

    /// The field definition that validates this parameter's values (`None` for a secret).
    fn field(&self) -> Option<FieldDef> {
        match &self.kind {
            ParamKind::Secret => None,
            ParamKind::Value {
                data_type,
                set,
                validation,
            } => Some(Self::field_for(&self.key, *data_type, *set, validation)),
        }
    }

    fn invalid(&self, reason: impl Into<String>) -> RefDataError {
        RefDataError::InvalidParameter {
            key: clip(&self.key),
            reason: reason.into(),
        }
    }

    /// Verifies a value against the definition.
    fn check_value(
        &self,
        value: &Value,
        resolver: &dyn OptionResolver,
    ) -> Result<(), RefDataError> {
        let Some(field) = self.field() else {
            return Err(self.invalid("a secret takes a secret handle, not a value"));
        };
        if value.is_empty() {
            return Err(self.invalid("an empty value is not a setting; clear the override instead"));
        }
        if let Value::Json(json) = value {
            let size = serde_json::to_string(json).map_or(usize::MAX, |text| text.len());
            if size > MAX_JSON_BYTES {
                return Err(self.invalid(format!("JSON larger than {MAX_JSON_BYTES} bytes")));
            }
        }
        match validate_value(&field, value, None, resolver)
            .into_iter()
            .next()
        {
            Some(issue) => Err(self.invalid(issue.message)),
            None => Ok(()),
        }
    }

    /// Verifies the definition itself.
    ///
    /// # Errors
    /// [`RefDataError::Invalid`], [`RefDataError::InvalidParameter`], or a wrapped
    /// [`mt_core::CoreError`] from the validation rules.
    pub fn check(&self, resolver: &dyn OptionResolver) -> Result<(), RefDataError> {
        check_key(&self.key)?;
        if self.description.trim().is_empty() || self.description.chars().count() > 1_000 {
            return Err(RefDataError::invalid(
                "parameter description",
                "must be 1 to 1000 characters",
            ));
        }
        match &self.kind {
            ParamKind::Secret => {
                if self.default.is_some() {
                    return Err(self.invalid("a secret cannot have a default"));
                }
            }
            ParamKind::Value {
                data_type,
                set,
                validation,
            } => {
                if matches!(data_type, DataType::Link | DataType::Attachment) {
                    return Err(self.invalid(format!("a parameter cannot be of type {data_type}")));
                }
                if data_type.is_reference() != set.is_some() {
                    return Err(self.invalid(
                        "choice parameters name a reference set; other parameters do not",
                    ));
                }
                Self::field_for(&self.key, *data_type, *set, validation).check()?;
                if let Some(default) = &self.default {
                    self.check_value(default, resolver)?;
                }
            }
        }
        Ok(())
    }
}

/// A parameter key: 1–128 characters, dot-separated segments of lowercase letters, digits
/// and `_`, each starting with a letter, with at least two segments
/// (`import.max_rows`).
pub fn check_key(key: &str) -> Result<(), RefDataError> {
    let segment_ok = |segment: &str| {
        let mut chars = segment.chars();
        chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };
    let ok = key.len() <= MAX_KEY_CHARS && key.contains('.') && key.split('.').all(segment_ok);
    if ok {
        Ok(())
    } else {
        Err(RefDataError::invalid(
            "parameter key",
            format!(
                "`{}` must be namespaced lowercase segments such as `import.max_rows`",
                clip(key)
            ),
        ))
    }
}

/// What is stored for a parameter at a scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Setting {
    /// A typed value.
    Value(Value),
    /// A handle to a secret.
    Secret(SecretRef),
}

/// One change of a parameter at a scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The new setting; `None` means the override was cleared.
    pub setting: Option<Setting>,
    /// Who changed it.
    pub by: PrincipalId,
    /// When.
    pub at: Timestamp,
}

/// Where a resolved value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Nothing sets the parameter.
    Unset,
    /// The definition's default.
    Default,
    /// An override at a scope.
    Override {
        /// The scope.
        scope: Scope,
        /// Who set it.
        by: PrincipalId,
        /// When.
        at: Timestamp,
    },
}

/// The effective value of a parameter for a context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved<'a> {
    /// The parameter key.
    pub key: &'a str,
    /// The value (`None` when unset, and always `None` for a secret).
    pub value: Option<&'a Value>,
    /// The secret handle, for secrets that are set.
    pub secret: Option<&'a SecretRef>,
    /// Where it comes from.
    pub source: Source,
}

impl Resolved<'_> {
    /// Whether the parameter has a value or a secret.
    #[must_use]
    pub const fn is_set(&self) -> bool {
        self.value.is_some() || self.secret.is_some()
    }

    /// A sentence saying what the value is and where it comes from. Never shows a secret
    /// or its handle.
    #[must_use]
    pub fn explain(&self) -> String {
        let what = match (self.value, self.secret) {
            (Some(value), _) => format!("`{}` is {value}", self.key),
            (None, Some(_)) => format!("`{}` is a secret that is set", self.key),
            (None, None) => format!("`{}` has no value", self.key),
        };
        match self.source {
            Source::Unset => format!("{what}: there is no default and no override"),
            Source::Default => format!("{what}: the default"),
            Source::Override { scope, by, at } => {
                let place = match scope {
                    Scope::Tenant => "for the tenant".to_owned(),
                    Scope::Base { base } => format!("for the base {base}"),
                    Scope::User { user } => format!("for the user {user}"),
                };
                format!("{what}: set {place} by {by} at {at}")
            }
        }
    }
}

/// The parameters of one tenant: definitions and overrides with their history.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Registry {
    defs: BTreeMap<String, ParamDef>,
    values: BTreeMap<String, BTreeMap<Scope, Vec<Entry>>>,
}

impl Registry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a parameter or replaces its definition. A replacement must keep the kind
    /// and data type, and every current override must still be valid under it.
    ///
    /// # Errors
    /// The definition's own problem, [`RefDataError::Limit`], or
    /// [`RefDataError::InvalidParameter`] naming what the replacement would break.
    pub fn define(
        &mut self,
        def: ParamDef,
        resolver: &dyn OptionResolver,
    ) -> Result<(), RefDataError> {
        def.check(resolver)?;
        if let Some(existing) = self.defs.get(&def.key) {
            let shape = |d: &ParamDef| match &d.kind {
                ParamKind::Secret => None,
                ParamKind::Value { data_type, .. } => Some(*data_type),
            };
            if shape(existing) != shape(&def) {
                return Err(def.invalid("the type of a parameter cannot change"));
            }
            for (scope, entries) in self.values.get(&def.key).into_iter().flatten() {
                if !def.overridable_at.contains(&scope.level()) {
                    if entries.last().is_some_and(|e| e.setting.is_some()) {
                        return Err(def.invalid(format!(
                            "an override exists at the {} level; clear it first",
                            scope.level().name()
                        )));
                    }
                    continue;
                }
                if let Some(Entry {
                    setting: Some(Setting::Value(value)),
                    ..
                }) = entries.last()
                {
                    def.check_value(value, resolver)?;
                }
            }
        } else if self.defs.len() >= MAX_PARAMETERS {
            return Err(RefDataError::Limit("too many parameters"));
        }
        self.defs.insert(def.key.clone(), def);
        Ok(())
    }

    /// The definition of a parameter.
    #[must_use]
    pub fn definition(&self, key: &str) -> Option<&ParamDef> {
        self.defs.get(key)
    }

    /// Every definition, in key order.
    pub fn definitions(&self) -> impl Iterator<Item = &ParamDef> {
        self.defs.values()
    }

    fn def(&self, key: &str) -> Result<&ParamDef, RefDataError> {
        self.defs
            .get(key)
            .ok_or_else(|| RefDataError::UnknownParameter { key: clip(key) })
    }

    fn push(&mut self, key: &str, scope: Scope, entry: Entry) {
        let history = self
            .values
            .entry(key.to_owned())
            .or_default()
            .entry(scope)
            .or_default();
        history.push(entry);
        let excess = history.len().saturating_sub(MAX_HISTORY);
        history.drain(..excess);
    }

    /// Sets a parameter at a scope.
    ///
    /// # Errors
    /// [`RefDataError::UnknownParameter`], [`RefDataError::ScopeNotAllowed`] or
    /// [`RefDataError::InvalidParameter`]. On error nothing changes.
    pub fn set(
        &mut self,
        key: &str,
        scope: Scope,
        setting: Setting,
        by: PrincipalId,
        at: Timestamp,
        resolver: &dyn OptionResolver,
    ) -> Result<(), RefDataError> {
        let def = self.def(key)?;
        if !def.overridable_at.contains(&scope.level()) {
            return Err(RefDataError::ScopeNotAllowed {
                key: clip(key),
                scope: scope.level().name(),
            });
        }
        match &setting {
            Setting::Value(value) => def.check_value(value, resolver)?,
            Setting::Secret(_) => {
                if !def.is_secret() {
                    return Err(def.invalid("only secret parameters take a secret handle"));
                }
            }
        }
        self.push(
            key,
            scope,
            Entry {
                setting: Some(setting),
                by,
                at,
            },
        );
        Ok(())
    }

    /// Removes the override at a scope, so that the next level applies again. Returns
    /// whether there was an override to remove.
    ///
    /// # Errors
    /// [`RefDataError::UnknownParameter`].
    pub fn clear(
        &mut self,
        key: &str,
        scope: Scope,
        by: PrincipalId,
        at: Timestamp,
    ) -> Result<bool, RefDataError> {
        self.def(key)?;
        if self.current(key, scope).is_none() {
            return Ok(false);
        }
        self.push(
            key,
            scope,
            Entry {
                setting: None,
                by,
                at,
            },
        );
        Ok(true)
    }

    /// The entry that currently sets the parameter at a scope, if any.
    fn current(&self, key: &str, scope: Scope) -> Option<(&Setting, &Entry)> {
        let entry = self.values.get(key)?.get(&scope)?.last()?;
        entry.setting.as_ref().map(|setting| (setting, entry))
    }

    /// The changes made to a parameter at a scope, oldest first.
    #[must_use]
    pub fn history(&self, key: &str, scope: Scope) -> &[Entry] {
        self.values
            .get(key)
            .and_then(|scopes| scopes.get(&scope))
            .map_or(&[], Vec::as_slice)
    }

    /// The effective value of a parameter for a context.
    ///
    /// # Errors
    /// [`RefDataError::UnknownParameter`].
    pub fn resolve(&self, key: &str, ctx: &ParamContext) -> Result<Resolved<'_>, RefDataError> {
        let def = self.def(key)?;
        let candidates = [
            ctx.user.map(|user| Scope::User { user }),
            ctx.base.map(|base| Scope::Base { base }),
            Some(Scope::Tenant),
        ];
        for scope in candidates.into_iter().flatten() {
            if !def.overridable_at.contains(&scope.level()) {
                continue;
            }
            if let Some((setting, entry)) = self.current(key, scope) {
                let source = Source::Override {
                    scope,
                    by: entry.by,
                    at: entry.at,
                };
                return Ok(match setting {
                    Setting::Value(value) => Resolved {
                        key: &def.key,
                        value: Some(value),
                        secret: None,
                        source,
                    },
                    Setting::Secret(secret) => Resolved {
                        key: &def.key,
                        value: None,
                        secret: Some(secret),
                        source,
                    },
                });
            }
        }
        Ok(Resolved {
            key: &def.key,
            value: def.default.as_ref(),
            secret: None,
            source: if def.default.is_some() {
                Source::Default
            } else {
                Source::Unset
            },
        })
    }

    /// The effective value, or `None` when the parameter is unknown, unset or a secret.
    #[must_use]
    pub fn value(&self, key: &str, ctx: &ParamContext) -> Option<&Value> {
        self.resolve(key, ctx)
            .ok()
            .and_then(|resolved| resolved.value)
    }

    /// Whether a feature flag is on. Unknown, unset and non-boolean parameters are off.
    #[must_use]
    pub fn flag(&self, key: &str, ctx: &ParamContext) -> bool {
        matches!(self.value(key, ctx), Some(Value::Bool(true)))
    }

    /// A numeric parameter's value.
    #[must_use]
    pub fn number(&self, key: &str, ctx: &ParamContext) -> Option<Decimal> {
        match self.value(key, ctx) {
            Some(Value::Number(n)) => Some(*n),
            Some(_) | None => None,
        }
    }

    /// A text parameter's value.
    #[must_use]
    pub fn text(&self, key: &str, ctx: &ParamContext) -> Option<&str> {
        self.value(key, ctx).and_then(Value::as_text)
    }

    /// A duration parameter's value.
    #[must_use]
    pub fn duration(&self, key: &str, ctx: &ParamContext) -> Option<DurationMs> {
        match self.value(key, ctx) {
            Some(Value::Duration(d)) => Some(*d),
            Some(_) | None => None,
        }
    }

    /// The registry seen from one context, as a source of field options.
    #[must_use]
    pub const fn bind(&self, ctx: ParamContext) -> Bound<'_> {
        Bound {
            registry: self,
            ctx,
        }
    }
}

/// A [`Registry`] bound to a context: answers which items a choice parameter offers.
#[derive(Debug, Clone, Copy)]
pub struct Bound<'a> {
    registry: &'a Registry,
    ctx: ParamContext,
}

impl ParameterOptions for Bound<'_> {
    fn offers(&self, key: &str, item: RefItemId) -> bool {
        match self.registry.value(key, &self.ctx) {
            Some(Value::MultiReference(items)) => items.contains(&item),
            Some(Value::Reference(only)) => *only == item,
            Some(_) | None => false,
        }
    }
}

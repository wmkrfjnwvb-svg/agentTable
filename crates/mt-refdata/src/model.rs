//! The stored shape of reference data: sets, items and tenant overlays.
//!
//! * A [`RefSet`] is a governed list of permitted values (a "data dictionary" when flat, a
//!   hierarchy when a tree): regions, cost centres, return reasons, ISO countries.
//! * A [`RefItem`] is one value. Cells store its stable [`RefItemId`], never its label, so
//!   relabelling or translating an item never touches records.
//! * An [`Overlay`] holds one tenant's changes to a *system* set: extra items, other labels,
//!   switched-off items and aliases. System codes cannot be changed by tenants, which keeps
//!   one meaning per code across the installation.
//!
//! The types here are plain data with local checks. Everything that needs the whole set
//! (unique codes, a well-formed tree, unambiguous aliases) is verified when an
//! [`crate::EffectiveSet`] is built.

use crate::error::RefDataError;
use mt_core::field::{is_locale_tag, DataType, FieldDef};
use mt_core::id::{BaseId, FieldId, PrincipalId, RefItemId, RefSetId};
use mt_core::time::Date;
use mt_core::validate::{validate_value, AcceptAllOptions};
use mt_core::value::Value;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Deepest tree a set may have (the same bound fields use for levels).
pub const MAX_TREE_DEPTH: u8 = mt_core::field::MAX_REF_LEVEL;
/// Maximum number of items in one set, tenant extensions included.
pub const MAX_ITEMS_PER_SET: usize = 100_000;
/// Maximum number of custom attributes of a set.
pub const MAX_ATTRIBUTES: usize = 64;
/// Maximum number of characters in a set or item code.
pub const MAX_CODE_CHARS: usize = 64;
/// Maximum number of characters in a name, description or alias.
pub const MAX_TEXT_CHARS: usize = 255;
/// Maximum number of external systems one item may have aliases for.
pub const MAX_ALIAS_SYSTEMS: usize = 32;
/// Maximum number of aliases of one item in one external system.
pub const MAX_ALIASES_PER_SYSTEM: usize = 32;

/// Shortens text that is echoed in an error message.
pub(crate) fn clip(input: &str) -> String {
    const ECHO_LIMIT: usize = 40;
    if input.chars().count() <= ECHO_LIMIT {
        return input.to_owned();
    }
    let mut out: String = input.chars().take(ECHO_LIMIT).collect();
    out.push('…');
    out
}

/// A display text: not blank, at most [`MAX_TEXT_CHARS`] characters, no control characters.
pub(crate) fn check_label(what: &'static str, text: &str) -> Result<(), RefDataError> {
    if text.trim().is_empty() {
        return Err(RefDataError::invalid(what, "must not be empty"));
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(RefDataError::invalid(
            what,
            format!("is longer than {MAX_TEXT_CHARS} characters"),
        ));
    }
    if text.chars().any(char::is_control) {
        return Err(RefDataError::invalid(what, "contains a control character"));
    }
    Ok(())
}

/// Localised names: every key a locale tag, every value a label.
pub(crate) fn check_names(
    what: &'static str,
    names: &BTreeMap<String, String>,
) -> Result<(), RefDataError> {
    for (locale, name) in names {
        if !is_locale_tag(locale) {
            return Err(RefDataError::invalid(
                what,
                format!(
                    "`{}` is not a locale tag such as `de` or `en-IN`",
                    clip(locale)
                ),
            ));
        }
        check_label(what, name)?;
    }
    Ok(())
}

/// The name to show for a locale: the exact tag, else its language, else the default.
pub(crate) fn localised<'a>(
    default: &'a str,
    names: &'a BTreeMap<String, String>,
    locale: &str,
) -> &'a str {
    let language = locale
        .split_once('-')
        .map_or(locale, |(language, _)| language);
    names
        .get(locale)
        .or_else(|| names.get(language))
        .map_or(default, String::as_str)
}

/// An item code: 1–64 characters, starting with a letter or digit, then letters, digits,
/// `_`, `.` or `-`. Codes are what integrations and formulas use; they never change.
pub fn check_item_code(code: &str) -> Result<(), RefDataError> {
    let mut chars = code.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphanumeric());
    let rest_ok = chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    if !first_ok || !rest_ok || code.len() > MAX_CODE_CHARS {
        return Err(RefDataError::invalid(
            "item code",
            format!(
                "`{}` must be 1 to {MAX_CODE_CHARS} letters, digits, `_`, `.` or `-`, starting with a letter or digit",
                clip(code)
            ),
        ));
    }
    Ok(())
}

/// A set code: 1–64 characters, a lowercase letter followed by lowercase letters, digits or
/// `_` (`cost_center`, `iso_3166_1`).
pub fn check_set_code(code: &str) -> Result<(), RefDataError> {
    let mut chars = code.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase());
    let rest_ok = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !first_ok || !rest_ok || code.len() > MAX_CODE_CHARS {
        return Err(RefDataError::invalid(
            "set code",
            format!(
                "`{}` must be 1 to {MAX_CODE_CHARS} lowercase letters, digits or `_`, starting with a letter",
                clip(code)
            ),
        ));
    }
    Ok(())
}

/// The name of an external system in a crosswalk: 1–64 lowercase letters, digits, `_`, `.`
/// or `-` (`sap`, `hris_a`, `shop.eu`).
pub fn check_system_name(system: &str) -> Result<(), RefDataError> {
    let ok = !system.is_empty()
        && system.len() <= MAX_CODE_CHARS
        && system
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-'));
    if ok {
        Ok(())
    } else {
        Err(RefDataError::invalid(
            "external system name",
            format!(
                "`{}` must be 1 to {MAX_CODE_CHARS} lowercase letters, digits, `_`, `.` or `-`",
                clip(system)
            ),
        ))
    }
}

/// Aliases of an item: external system → the values that system uses for the item. The first
/// value (in sort order) is the one sent back to that system.
pub type Aliases = BTreeMap<String, BTreeSet<String>>;

pub(crate) fn check_aliases(aliases: &Aliases) -> Result<(), RefDataError> {
    if aliases.len() > MAX_ALIAS_SYSTEMS {
        return Err(RefDataError::Limit("aliases for too many external systems"));
    }
    for (system, values) in aliases {
        check_system_name(system)?;
        if values.is_empty() {
            return Err(RefDataError::invalid(
                "alias",
                format!("system `{system}` is listed without a value"),
            ));
        }
        if values.len() > MAX_ALIASES_PER_SYSTEM {
            return Err(RefDataError::Limit(
                "too many aliases in one external system",
            ));
        }
        for value in values {
            check_label("alias", value)?;
            if value.trim() != value {
                return Err(RefDataError::invalid(
                    "alias",
                    "must not start or end with a space",
                ));
            }
        }
    }
    Ok(())
}

/// Whether a set is a plain list or a hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Structure {
    /// A plain list: no item has a parent.
    Flat,
    /// A hierarchy.
    Tree {
        /// The deepest level allowed (1 = only top-level items), at most [`MAX_TREE_DEPTH`].
        max_depth: u8,
    },
}

impl Structure {
    /// A tree of the greatest supported depth.
    #[must_use]
    pub const fn tree() -> Self {
        Structure::Tree {
            max_depth: MAX_TREE_DEPTH,
        }
    }

    /// The deepest level an item may be at (1 for a flat set).
    #[must_use]
    pub const fn max_depth(self) -> u8 {
        match self {
            Structure::Flat => 1,
            Structure::Tree { max_depth } => max_depth,
        }
    }
}

/// Who a set belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SetScope {
    /// Built in (ISO countries, currencies, …). Tenants adapt it through an [`Overlay`].
    System,
    /// Owned by the tenant, usable in every base.
    Tenant,
    /// Owned by one base.
    Base {
        /// The base.
        base: BaseId,
    },
}

/// How changes to a set take effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangePolicy {
    /// A change by someone who may manage the set applies at once.
    Direct,
    /// A change is proposed by one person and applied when another approves it.
    Approval,
}

/// A reference set: the definition, without its items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefSet {
    /// Stable id.
    pub id: RefSetId,
    /// Immutable code, unique per tenant; what integrations and formulas use.
    pub code: String,
    /// Display name.
    pub name: String,
    /// Localised display names by locale tag.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, String>,
    /// What the set is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Flat or tree.
    pub structure: Structure,
    /// Who it belongs to.
    pub scope: SetScope,
    /// Custom attributes every item may carry (a region's time zone, a category's tax
    /// rate), defined with the field framework.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub item_schema: Vec<FieldDef>,
    /// The person accountable for the set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<PrincipalId>,
    /// How changes take effect.
    pub change_policy: ChangePolicy,
}

impl RefSet {
    /// A set with the given structure and scope, no custom attributes and direct changes.
    pub fn new(
        id: RefSetId,
        code: impl Into<String>,
        name: impl Into<String>,
        structure: Structure,
        scope: SetScope,
    ) -> Self {
        RefSet {
            id,
            code: code.into(),
            name: name.into(),
            names: BTreeMap::new(),
            description: None,
            structure,
            scope,
            item_schema: Vec::new(),
            owner: None,
            change_policy: ChangePolicy::Direct,
        }
    }

    /// The name to show for a locale.
    #[must_use]
    pub fn display_name(&self, locale: &str) -> &str {
        localised(&self.name, &self.names, locale)
    }

    /// The definition of a custom attribute.
    #[must_use]
    pub fn attribute(&self, field: FieldId) -> Option<&FieldDef> {
        self.item_schema.iter().find(|def| def.id == field)
    }

    /// Verifies the definition on its own: code, names, depth and custom attributes.
    ///
    /// # Errors
    /// [`RefDataError::Invalid`], [`RefDataError::Limit`] or a wrapped
    /// [`mt_core::CoreError`] from an attribute definition.
    pub fn check(&self) -> Result<(), RefDataError> {
        check_set_code(&self.code)?;
        check_label("set name", &self.name)?;
        check_names("set name", &self.names)?;
        if let Some(description) = &self.description {
            if description.chars().count() > MAX_TEXT_CHARS.saturating_mul(4) {
                return Err(RefDataError::invalid("set description", "is too long"));
            }
        }
        if let Structure::Tree { max_depth } = self.structure {
            if max_depth == 0 || max_depth > MAX_TREE_DEPTH {
                return Err(RefDataError::invalid(
                    "tree depth",
                    format!("must be between 1 and {MAX_TREE_DEPTH}"),
                ));
            }
        }
        if self.item_schema.len() > MAX_ATTRIBUTES {
            return Err(RefDataError::Limit("too many custom attributes"));
        }
        let type_of =
            |field: FieldId| -> Option<DataType> { self.attribute(field).map(|def| def.data_type) };
        for (index, def) in self.item_schema.iter().enumerate() {
            def.check()?;
            def.check_in_table(&type_of)?;
            let earlier = self.item_schema.iter().take(index);
            if earlier.clone().any(|other| other.id == def.id) {
                return Err(RefDataError::invalid(
                    "custom attribute",
                    format!("the attribute {} is defined twice", def.id),
                ));
            }
            if earlier
                .clone()
                .any(|other| other.name.eq_ignore_ascii_case(&def.name))
            {
                return Err(RefDataError::invalid(
                    "custom attribute",
                    format!("the name `{}` is used twice", clip(&def.name)),
                ));
            }
            if def.is_generated() {
                return Err(RefDataError::invalid(
                    "custom attribute",
                    "auto-numbering is not available for attributes of reference items",
                ));
            }
        }
        Ok(())
    }
}

/// One permitted value of a set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefItem {
    /// Stable id; this is what cells store.
    pub id: RefItemId,
    /// Immutable code, unique within the set (ASCII case ignored).
    pub code: String,
    /// Display name.
    pub name: String,
    /// Localised display names by locale tag.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, String>,
    /// An optional typed value carried by the item (a weight, a rate, a date).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// The parent item (trees only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<RefItemId>,
    /// Position among its siblings; ties are ordered by name.
    #[serde(default)]
    pub sort: i64,
    /// Disabled items stay on existing records but cannot be newly selected.
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    /// First day on which the item may be selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<Date>,
    /// Last day on which the item may be selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<Date>,
    /// Values of the set's custom attributes.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<FieldId, Value>,
    /// What external systems call this item.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub aliases: Aliases,
}

const fn enabled_by_default() -> bool {
    true
}

impl RefItem {
    /// An enabled top-level item without validity limits, attributes or aliases.
    pub fn new(id: RefItemId, code: impl Into<String>, name: impl Into<String>) -> Self {
        RefItem {
            id,
            code: code.into(),
            name: name.into(),
            names: BTreeMap::new(),
            value: None,
            parent: None,
            sort: 0,
            enabled: true,
            valid_from: None,
            valid_to: None,
            attributes: BTreeMap::new(),
            aliases: Aliases::new(),
        }
    }

    /// The same item under a parent.
    #[must_use]
    pub const fn under(mut self, parent: RefItemId) -> Self {
        self.parent = Some(parent);
        self
    }

    /// The name to show for a locale.
    #[must_use]
    pub fn display_name(&self, locale: &str) -> &str {
        localised(&self.name, &self.names, locale)
    }

    /// Whether `day` lies within the item's own validity period.
    #[must_use]
    pub fn is_valid_on(&self, day: Date) -> bool {
        self.valid_from.is_none_or(|from| from <= day) && self.valid_to.is_none_or(|to| day <= to)
    }

    /// Verifies the item on its own against the set definition: code, names, validity
    /// period, custom attribute values and aliases.
    ///
    /// # Errors
    /// [`RefDataError::Invalid`] or [`RefDataError::Limit`].
    pub fn check(&self, set: &RefSet) -> Result<(), RefDataError> {
        check_item_code(&self.code)?;
        check_label("item name", &self.name)?;
        check_names("item name", &self.names)?;
        if let (Some(from), Some(to)) = (self.valid_from, self.valid_to) {
            if from > to {
                return Err(RefDataError::invalid(
                    "validity period",
                    format!("item `{}` ends before it starts", self.code),
                ));
            }
        }
        if self.value.as_ref().is_some_and(Value::is_empty) {
            return Err(RefDataError::invalid(
                "item value",
                "an empty value must be left out",
            ));
        }
        for (field, value) in &self.attributes {
            let Some(def) = set.attribute(*field) else {
                return Err(RefDataError::invalid(
                    "custom attribute",
                    format!(
                        "item `{}` has a value for the undefined attribute {field}",
                        self.code
                    ),
                ));
            };
            if let Some(issue) = validate_value(def, value, None, &AcceptAllOptions)
                .into_iter()
                .next()
            {
                return Err(RefDataError::invalid(
                    "custom attribute",
                    format!("item `{}`: {}", self.code, issue.message),
                ));
            }
        }
        for def in &set.item_schema {
            let missing = self.attributes.get(&def.id).is_none_or(Value::is_empty);
            if def.validation.required && missing {
                return Err(RefDataError::invalid(
                    "custom attribute",
                    format!(
                        "item `{}` needs a value for `{}`",
                        self.code,
                        clip(&def.name)
                    ),
                ));
            }
        }
        check_aliases(&self.aliases)
    }
}

/// Another label for a system item, chosen by a tenant.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Relabel {
    /// Replaces the default name, if given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Replaces or adds localised names, per locale.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, String>,
}

/// One tenant's changes to a system set. The effective set of the tenant is
/// *system items ∪ added − disabled*, with the tenant's labels and aliases.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Overlay {
    /// Items the tenant added. They may hang under system items or under each other.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<RefItem>,
    /// Other labels for system items.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub relabel: BTreeMap<RefItemId, Relabel>,
    /// System items the tenant switched off.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub disabled: BTreeSet<RefItemId>,
    /// The tenant's aliases for system items (added to the system's own).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub aliases: BTreeMap<RefItemId, Aliases>,
}

impl Overlay {
    /// Whether the overlay changes nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.relabel.is_empty()
            && self.disabled.is_empty()
            && self.aliases.is_empty()
    }
}

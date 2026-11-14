//! Field definitions: the schema of one column.
//!
//! A field has six independent facets (product design, doc 07 §1):
//!
//! 1. [`DataType`] — how the value is stored and what it means;
//! 2. [`Widget`] — how a person enters it;
//! 3. [`OptionsSource`] — where selectable options come from;
//! 4. [`DisplayFormat`] — how it is shown (prefix, suffix, decimals, mask, numbering);
//! 5. [`ValidationSpec`] — which values are acceptable;
//! 6. governance — [`FieldStatus`], [`Classification`], [`FieldRole`], group and help text.
//!
//! [`FieldDef::check`] verifies that the facets fit together (for example that a numeric range
//! is not attached to a text field). Checking a *value* against a definition is done by
//! [`crate::validate`].

use crate::decimal::Decimal;
use crate::error::{clip, CoreError};
use crate::format::FormatId;
use crate::id::{FieldGroupId, FieldId, FieldLibraryItemId, RefItemId, RefSetId, TableId};
use crate::money::Currency;
use crate::numbering::NumberingPattern;
use crate::pattern::TextPattern;
use core::fmt;
use core::str::FromStr;

/// Maximum number of characters in a field or group name.
pub const MAX_NAME_CHARS: usize = 255;
/// Maximum number of characters in a prefix or suffix.
pub const MAX_AFFIX_CHARS: usize = 32;
/// Maximum number of decimal places a field may display or accept.
pub const MAX_DECIMALS: u8 = 28;
/// Maximum number of inline options of one field.
pub const MAX_INLINE_OPTIONS: usize = 10_000;

macro_rules! string_enum {
    (
        $(#[$meta:meta])* $name:ident, $kind:literal;
        $( $(#[$vmeta:meta])* $variant:ident => $text:literal, )+
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $( $(#[$vmeta])* $variant, )+
        }

        impl $name {
            /// Every variant, in declaration order.
            pub const ALL: &'static [$name] = &[ $( $name::$variant, )+ ];

            /// The stable textual name (also the serde form).
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $( $name::$variant => $text, )+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = CoreError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                $name::ALL
                    .iter()
                    .copied()
                    .find(|v| v.as_str() == s)
                    .ok_or_else(|| CoreError::UnknownCode { kind: $kind, code: clip(s) })
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = <std::borrow::Cow<'de, str> as serde::Deserialize>::deserialize(deserializer)?;
                text.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}
pub(crate) use string_enum;

string_enum! {
    /// The storage type of a field. One per [`crate::value::Value`] variant (except null).
    DataType, "data type";
    /// Single-line text.
    Text => "text",
    /// Multi-line text.
    LongText => "long_text",
    /// Exact decimal number.
    Number => "number",
    /// Exact amount with a currency.
    Money => "money",
    /// Percentage stored as a fraction.
    Percent => "percent",
    /// True or false.
    Bool => "bool",
    /// Calendar day.
    Date => "date",
    /// Instant in UTC.
    DateTime => "datetime",
    /// Length of time.
    Duration => "duration",
    /// One item of a reference set or inline option list.
    Reference => "reference",
    /// Several reference items.
    MultiReference => "multi_reference",
    /// Links to records of another table.
    Link => "link",
    /// People, groups or agents.
    Principal => "principal",
    /// Stored files.
    Attachment => "attachment",
    /// Free-form JSON.
    Json => "json",
}

impl DataType {
    /// Text-like types (`text`, `long_text`).
    #[must_use]
    pub const fn is_text(self) -> bool {
        matches!(self, DataType::Text | DataType::LongText)
    }

    /// Types with a numeric magnitude that `min`/`max` rules apply to
    /// (`number`, `money`, `percent`, `duration`).
    #[must_use]
    pub const fn is_numeric(self) -> bool {
        matches!(
            self,
            DataType::Number | DataType::Money | DataType::Percent | DataType::Duration
        )
    }

    /// Types holding a list of IDs (`multi_reference`, `link`, `principal`, `attachment`).
    #[must_use]
    pub const fn is_list(self) -> bool {
        matches!(
            self,
            DataType::MultiReference | DataType::Link | DataType::Principal | DataType::Attachment
        )
    }

    /// Types whose values come from options (`reference`, `multi_reference`).
    #[must_use]
    pub const fn is_reference(self) -> bool {
        matches!(self, DataType::Reference | DataType::MultiReference)
    }
}

string_enum! {
    /// The input control used to edit a field.
    Widget, "widget";
    /// Single-line text box.
    TextBox => "text_box",
    /// Multi-line text area.
    TextArea => "text_area",
    /// Rich text (markdown) editor.
    RichText => "rich_text",
    /// Numeric input.
    NumberInput => "number_input",
    /// Slider between the field's min and max.
    Slider => "slider",
    /// Star (or other icon) rating.
    Rating => "rating",
    /// Progress bar.
    Progress => "progress",
    /// Amount input with a currency selector.
    MoneyInput => "money_input",
    /// Checkbox.
    Checkbox => "checkbox",
    /// On/off toggle.
    Toggle => "toggle",
    /// Date picker.
    DatePicker => "date_picker",
    /// Date and time picker.
    DateTimePicker => "datetime_picker",
    /// Duration input.
    DurationInput => "duration_input",
    /// Drop-down list (one choice).
    Dropdown => "dropdown",
    /// Radio buttons (one choice).
    Radio => "radio",
    /// Segmented control (one choice).
    Segmented => "segmented",
    /// Multi-select drop-down.
    MultiSelect => "multi_select",
    /// Group of checkboxes (several choices).
    CheckboxGroup => "checkbox_group",
    /// Tag input (several choices).
    TagInput => "tag_input",
    /// Cascading selector along a tree path.
    Cascader => "cascader",
    /// Tree picker.
    TreePicker => "tree_picker",
    /// Searchable picker for large sets.
    SearchPicker => "search_picker",
    /// Record picker for link fields.
    LinkPicker => "link_picker",
    /// Line-item editor for child records.
    LineItems => "line_items",
    /// People/group picker.
    PrincipalPicker => "principal_picker",
    /// File uploader.
    FileUpload => "file_upload",
    /// Signature pad (stored as an attachment).
    Signature => "signature",
    /// Barcode / QR scanner filling a text field.
    Barcode => "barcode",
    /// Location picker (stored as JSON).
    Location => "location",
    /// Raw JSON editor.
    JsonEditor => "json_editor",
}

impl Widget {
    /// The widgets that can edit a data type. The first entry is the default.
    #[must_use]
    pub const fn allowed_for(data_type: DataType) -> &'static [Widget] {
        match data_type {
            DataType::Text => &[Widget::TextBox, Widget::Barcode],
            DataType::LongText => &[Widget::TextArea, Widget::RichText],
            DataType::Number => &[
                Widget::NumberInput,
                Widget::Slider,
                Widget::Rating,
                Widget::Progress,
            ],
            DataType::Money => &[Widget::MoneyInput],
            DataType::Percent => &[Widget::NumberInput, Widget::Slider, Widget::Progress],
            DataType::Bool => &[Widget::Checkbox, Widget::Toggle],
            DataType::Date => &[Widget::DatePicker],
            DataType::DateTime => &[Widget::DateTimePicker],
            DataType::Duration => &[Widget::DurationInput],
            DataType::Reference => &[
                Widget::Dropdown,
                Widget::Radio,
                Widget::Segmented,
                Widget::Cascader,
                Widget::TreePicker,
                Widget::SearchPicker,
            ],
            DataType::MultiReference => &[
                Widget::MultiSelect,
                Widget::CheckboxGroup,
                Widget::TagInput,
                Widget::TreePicker,
                Widget::SearchPicker,
            ],
            DataType::Link => &[Widget::LinkPicker, Widget::LineItems],
            DataType::Principal => &[Widget::PrincipalPicker],
            DataType::Attachment => &[Widget::FileUpload, Widget::Signature],
            DataType::Json => &[Widget::JsonEditor, Widget::Location],
        }
    }

    /// The default widget of a data type.
    #[must_use]
    pub fn default_for(data_type: DataType) -> Widget {
        Widget::allowed_for(data_type)
            .first()
            .copied()
            .unwrap_or(Widget::TextBox)
    }

    /// Whether this widget can edit the data type.
    #[must_use]
    pub fn supports(self, data_type: DataType) -> bool {
        Widget::allowed_for(data_type).contains(&self)
    }
}

string_enum! {
    /// Privacy classification of a field. Ordered from least to most sensitive.
    Classification, "classification";
    /// May be shown to anyone.
    Public => "public",
    /// Internal business data (the default).
    Internal => "internal",
    /// Restricted business data.
    Confidential => "confidential",
    /// Personal data.
    Personal => "personal",
    /// Sensitive personal data.
    SensitivePersonal => "sensitive_personal",
}

impl Classification {
    /// Whether the field holds personal data (`personal` or `sensitive_personal`).
    #[must_use]
    pub const fn is_personal(self) -> bool {
        matches!(
            self,
            Classification::Personal | Classification::SensitivePersonal
        )
    }
}

string_enum! {
    /// Lifecycle status of a field definition.
    FieldStatus, "field status";
    /// Normal.
    Enabled => "enabled",
    /// Hidden from entry and default views; existing data kept; writes rejected.
    Disabled => "disabled",
    /// Read-only everywhere and flagged for removal.
    Deprecated => "deprecated",
}

impl FieldStatus {
    /// Whether new values may be written to the field.
    #[must_use]
    pub const fn accepts_writes(self) -> bool {
        matches!(self, FieldStatus::Enabled)
    }

    /// Whether the field is shown in forms and default views.
    #[must_use]
    pub const fn visible_by_default(self) -> bool {
        matches!(self, FieldStatus::Enabled | FieldStatus::Deprecated)
    }
}

string_enum! {
    /// The role a field plays for generic platform features (doc 13 §3). Features such as SLA
    /// reports, approval thresholds and budgets find fields by role, never by name.
    FieldRole, "field role";
    /// The record's title / primary field.
    Title => "title",
    /// Who asked.
    PartyRequester => "party.requester",
    /// The other side (customer, supplier, candidate…).
    PartyCounterparty => "party.counterparty",
    /// Who is responsible for the record.
    PersonOwner => "person.owner",
    /// Who is doing the work.
    PersonAssignee => "person.assignee",
    /// The lifecycle state.
    State => "state",
    /// When the record was opened.
    TimeOpened => "time.opened",
    /// When it is due.
    TimeDue => "time.due",
    /// When it was resolved.
    TimeResolved => "time.resolved",
    /// The amount used for thresholds and budgets.
    MoneyAmount => "money.amount",
    /// The currency of the amount (when stored separately).
    MoneyCurrency => "money.currency",
    /// A business dimension (cost centre, region, entity, project, brand).
    Dimension => "dimension",
    /// The category classification.
    ClassificationCategory => "classification.category",
    /// The reason classification.
    ClassificationReason => "classification.reason",
    /// Evidence attachments.
    EvidenceAttachments => "evidence.attachments",
    /// A quantity of goods.
    Quantity => "quantity",
    /// The item (catalogue entry) concerned.
    Item => "item",
}

impl FieldRole {
    /// The data types a field with this role may have.
    #[must_use]
    pub const fn allowed_types(self) -> &'static [DataType] {
        match self {
            FieldRole::Title => &[DataType::Text],
            FieldRole::PartyRequester | FieldRole::PartyCounterparty => {
                &[DataType::Link, DataType::Principal]
            }
            FieldRole::PersonOwner | FieldRole::PersonAssignee => &[DataType::Principal],
            FieldRole::State
            | FieldRole::ClassificationCategory
            | FieldRole::ClassificationReason
            | FieldRole::MoneyCurrency => &[DataType::Reference],
            FieldRole::TimeOpened | FieldRole::TimeDue | FieldRole::TimeResolved => {
                &[DataType::DateTime, DataType::Date]
            }
            FieldRole::MoneyAmount => &[DataType::Money, DataType::Number],
            FieldRole::Dimension => &[DataType::Reference, DataType::Link],
            FieldRole::EvidenceAttachments => &[DataType::Attachment],
            FieldRole::Quantity => &[DataType::Number],
            FieldRole::Item => &[DataType::Link, DataType::Reference],
        }
    }
}

/// One option of an inline option list.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InlineOption {
    /// Stable ID stored in cells.
    pub id: RefItemId,
    /// Label shown to people.
    pub label: String,
    /// Optional colour token (for example `"red"`); semantics belong to the UI theme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Disabled options stay on existing records but cannot be newly selected.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

const fn default_true() -> bool {
    true
}

/// Where the selectable options of a field come from.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OptionsSource {
    /// The field has no options.
    #[default]
    None,
    /// Options defined on the field itself.
    Inline {
        /// The options, in display order.
        options: Vec<InlineOption>,
    },
    /// Items of a governed reference set.
    RefSet {
        /// The reference set.
        set: RefSetId,
        /// Restrict to the descendants of this item.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subtree: Option<RefItemId>,
        /// Only leaf items may be selected.
        #[serde(default)]
        leaves_only: bool,
    },
    /// Records of a table (for link fields).
    Table {
        /// The linked table.
        table: TableId,
    },
}

/// How much of a value a mask leaves readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Mask {
    /// Number of trailing characters left visible.
    pub visible_tail: u8,
}

impl Mask {
    /// Applies the mask: every character except the last `visible_tail` becomes `*`.
    /// White space and the separators `-`, `.`, `@`, `+` are kept so the shape stays readable.
    #[must_use]
    pub fn apply(self, text: &str) -> String {
        let total = text.chars().count();
        let keep_from = total.saturating_sub(usize::from(self.visible_tail));
        text.chars()
            .enumerate()
            .map(|(index, c)| {
                if index >= keep_from || c.is_whitespace() || matches!(c, '-' | '.' | '@' | '+') {
                    c
                } else {
                    '*'
                }
            })
            .collect()
    }
}

/// How a value is displayed. Purely presentational: it never changes stored data.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct DisplayFormat {
    /// Text shown before the value (for example `¥`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    /// Text shown after the value (for example `kg`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
    /// Fixed number of decimal places to show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u8>,
    /// Show thousands separators.
    #[serde(default)]
    pub thousands: bool,
    /// Mask applied for viewers without the unmask capability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<Mask>,
    /// Auto-numbering pattern (text fields only); the value is generated, not typed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numbering: Option<NumberingPattern>,
}

string_enum! {
    /// The scope within which a value must be unique.
    UniqueScope, "unique scope";
    /// Unique among the records of the table.
    Table => "table",
    /// Unique among the line items of the same parent record.
    Parent => "parent",
}

string_enum! {
    /// Comparison used by a cross-field rule.
    Comparison, "comparison";
    /// Less than.
    Lt => "lt",
    /// Less than or equal.
    Le => "le",
    /// Greater than.
    Gt => "gt",
    /// Greater than or equal.
    Ge => "ge",
    /// Equal.
    Eq => "eq",
    /// Not equal.
    Ne => "ne",
}

impl Comparison {
    /// Whether `ordering` (of this field's value relative to the other field's value)
    /// satisfies the comparison.
    #[must_use]
    pub const fn holds(self, ordering: core::cmp::Ordering) -> bool {
        use core::cmp::Ordering::{Equal, Greater, Less};
        match (self, ordering) {
            (Comparison::Lt, Less)
            | (Comparison::Le, Less | Equal)
            | (Comparison::Gt, Greater)
            | (Comparison::Ge, Greater | Equal)
            | (Comparison::Eq, Equal)
            | (Comparison::Ne, Less | Greater) => true,
            (Comparison::Lt, Equal | Greater)
            | (Comparison::Le, Greater)
            | (Comparison::Gt, Less | Equal)
            | (Comparison::Ge, Less)
            | (Comparison::Eq, Less | Greater)
            | (Comparison::Ne, Equal) => false,
        }
    }
}

/// "This field must compare like so with that field" (for example `end >= start`).
/// The rule is skipped while either value is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CrossFieldRule {
    /// The comparison this field's value must satisfy.
    pub comparison: Comparison,
    /// The other field (same table, same data type).
    pub other: FieldId,
}

/// Which values a field accepts. Every part is optional; parts that do not apply to the
/// field's data type are rejected by [`FieldDef::check`].
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct ValidationSpec {
    /// The value must not be empty.
    #[serde(default)]
    pub required: bool,
    /// Minimum number of characters (text types).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_chars: Option<u32>,
    /// Maximum number of characters (text types).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<u32>,
    /// Smallest allowed magnitude (numeric types; percent as a fraction, duration in ms).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::decimal::serde_canonical_opt"
    )]
    pub min: Option<Decimal>,
    /// Largest allowed magnitude (numeric types).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::decimal::serde_canonical_opt"
    )]
    pub max: Option<Decimal>,
    /// Maximum number of decimal places accepted (number, percent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_decimals: Option<u8>,
    /// Currencies accepted (money). Empty means any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub currencies: Vec<Currency>,
    /// Minimum number of items (list types).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_items: Option<u32>,
    /// Maximum number of items (list types).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u32>,
    /// The whole text must match this pattern (text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<TextPattern>,
    /// The text must satisfy this standard format (text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<FormatId>,
    /// The value must be unique within the scope. Enforced by the store; recorded here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unique: Option<UniqueScope>,
    /// Comparisons with other fields of the same record.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cross_field: Vec<CrossFieldRule>,
}

/// The definition of one field.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FieldDef {
    /// Stable ID; physical column names derive from it, so renames never touch data.
    pub id: FieldId,
    /// Display name.
    pub name: String,
    /// Storage type.
    pub data_type: DataType,
    /// Input control.
    pub widget: Widget,
    /// Option source.
    #[serde(default)]
    pub options: OptionsSource,
    /// Display format.
    #[serde(default)]
    pub format: DisplayFormat,
    /// Validation rules.
    #[serde(default)]
    pub validation: ValidationSpec,
    /// Role for generic features.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<FieldRole>,
    /// Privacy classification.
    pub classification: Classification,
    /// Lifecycle status.
    pub status: FieldStatus,
    /// The group this field belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<FieldGroupId>,
    /// The field-library item this field was created from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_item: Option<FieldLibraryItemId>,
    /// Help text shown next to the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help_text: Option<String>,
}

fn invalid(reason: impl Into<String>) -> CoreError {
    CoreError::InvalidDefinition {
        reason: reason.into(),
    }
}

fn check_name(what: &str, name: &str) -> Result<(), CoreError> {
    if name.trim().is_empty() {
        return Err(invalid(format!("{what} name must not be empty")));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(invalid(format!(
            "{what} name is longer than {MAX_NAME_CHARS} characters"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(invalid(format!("{what} name contains a control character")));
    }
    Ok(())
}

impl FieldDef {
    /// A minimal enabled, internal field of the given type with its default widget and no
    /// options, format or validation.
    #[must_use]
    pub fn new(id: FieldId, name: impl Into<String>, data_type: DataType) -> Self {
        FieldDef {
            id,
            name: name.into(),
            data_type,
            widget: Widget::default_for(data_type),
            options: OptionsSource::None,
            format: DisplayFormat::default(),
            validation: ValidationSpec::default(),
            role: None,
            classification: Classification::Internal,
            status: FieldStatus::Enabled,
            group: None,
            library_item: None,
            help_text: None,
        }
    }

    /// The inline options of the field, if it has an inline option list.
    #[must_use]
    pub fn inline_options(&self) -> Option<&[InlineOption]> {
        match &self.options {
            OptionsSource::Inline { options } => Some(options),
            OptionsSource::None | OptionsSource::RefSet { .. } | OptionsSource::Table { .. } => {
                None
            }
        }
    }

    /// Whether values of this field are generated by the platform (auto-numbering) rather
    /// than entered.
    #[must_use]
    pub const fn is_generated(&self) -> bool {
        self.format.numbering.is_some()
    }

    /// Verifies that the facets of the definition fit together.
    ///
    /// # Errors
    /// [`CoreError::InvalidDefinition`] describing the first inconsistency found.
    pub fn check(&self) -> Result<(), CoreError> {
        check_name("field", &self.name)?;
        if !self.widget.supports(self.data_type) {
            return Err(invalid(format!(
                "widget {} cannot edit a {} field",
                self.widget, self.data_type
            )));
        }
        self.check_options()?;
        self.check_format()?;
        self.check_validation()?;
        if let Some(role) = self.role {
            if !role.allowed_types().contains(&self.data_type) {
                return Err(invalid(format!(
                    "role {role} cannot be given to a {} field",
                    self.data_type
                )));
            }
        }
        Ok(())
    }

    fn check_options(&self) -> Result<(), CoreError> {
        let t = self.data_type;
        match &self.options {
            OptionsSource::None => {
                if t.is_reference() {
                    return Err(invalid(format!(
                        "a {t} field needs inline options or a reference set"
                    )));
                }
                if matches!(t, DataType::Link) {
                    return Err(invalid("a link field needs a target table"));
                }
            }
            OptionsSource::Inline { options } => {
                if !t.is_reference() {
                    return Err(invalid(format!(
                        "inline options cannot be attached to a {t} field"
                    )));
                }
                if options.len() > MAX_INLINE_OPTIONS {
                    return Err(invalid(format!(
                        "more than {MAX_INLINE_OPTIONS} inline options"
                    )));
                }
                let mut ids: Vec<RefItemId> = Vec::with_capacity(options.len());
                let mut labels: Vec<&str> = Vec::with_capacity(options.len());
                for option in options {
                    check_name("option", &option.label)?;
                    if ids.contains(&option.id) {
                        return Err(invalid(format!("option id {} is used twice", option.id)));
                    }
                    if labels.contains(&option.label.as_str()) {
                        return Err(invalid(format!(
                            "option label `{}` is used twice",
                            clip(&option.label)
                        )));
                    }
                    ids.push(option.id);
                    labels.push(&option.label);
                }
            }
            OptionsSource::RefSet { .. } => {
                if !t.is_reference() {
                    return Err(invalid(format!(
                        "a reference set cannot be attached to a {t} field"
                    )));
                }
            }
            OptionsSource::Table { .. } => {
                if !matches!(t, DataType::Link) {
                    return Err(invalid(format!(
                        "a target table cannot be attached to a {t} field"
                    )));
                }
            }
        }
        Ok(())
    }

    fn check_format(&self) -> Result<(), CoreError> {
        let t = self.data_type;
        let f = &self.format;
        for (what, affix) in [("prefix", &f.prefix), ("suffix", &f.suffix)] {
            if let Some(text) = affix {
                if text.chars().count() > MAX_AFFIX_CHARS {
                    return Err(invalid(format!(
                        "{what} is longer than {MAX_AFFIX_CHARS} characters"
                    )));
                }
                if text.chars().any(char::is_control) {
                    return Err(invalid(format!("{what} contains a control character")));
                }
            }
        }
        if let Some(decimals) = f.decimals {
            if !matches!(t, DataType::Number | DataType::Money | DataType::Percent) {
                return Err(invalid(format!(
                    "display decimals do not apply to a {t} field"
                )));
            }
            if decimals > MAX_DECIMALS {
                return Err(invalid(format!("display decimals above {MAX_DECIMALS}")));
            }
        }
        if f.thousands && !matches!(t, DataType::Number | DataType::Money | DataType::Percent) {
            return Err(invalid(format!(
                "thousands separators do not apply to a {t} field"
            )));
        }
        if f.mask.is_some() && !matches!(t, DataType::Text) {
            return Err(invalid(format!("a mask does not apply to a {t} field")));
        }
        if f.numbering.is_some() {
            if !matches!(t, DataType::Text) {
                return Err(invalid(format!(
                    "auto-numbering does not apply to a {t} field"
                )));
            }
            let v = &self.validation;
            if v.pattern.is_some()
                || v.format.is_some()
                || v.min_chars.is_some()
                || v.max_chars.is_some()
            {
                return Err(invalid(
                    "an auto-numbered field cannot also have text validation rules",
                ));
            }
        }
        Ok(())
    }

    fn check_validation(&self) -> Result<(), CoreError> {
        let t = self.data_type;
        let v = &self.validation;
        if (v.min_chars.is_some() || v.max_chars.is_some()) && !t.is_text() {
            return Err(invalid(format!(
                "character limits do not apply to a {t} field"
            )));
        }
        if let (Some(min), Some(max)) = (v.min_chars, v.max_chars) {
            if min > max {
                return Err(invalid("min_chars is greater than max_chars"));
            }
        }
        if (v.min.is_some() || v.max.is_some()) && !t.is_numeric() {
            return Err(invalid(format!(
                "a numeric range does not apply to a {t} field"
            )));
        }
        if let (Some(min), Some(max)) = (v.min, v.max) {
            if min > max {
                return Err(invalid("min is greater than max"));
            }
        }
        if let Some(decimals) = v.max_decimals {
            if !matches!(t, DataType::Number | DataType::Percent) {
                return Err(invalid(format!(
                    "max_decimals does not apply to a {t} field"
                )));
            }
            if decimals > MAX_DECIMALS {
                return Err(invalid(format!("max_decimals above {MAX_DECIMALS}")));
            }
        }
        if !v.currencies.is_empty() && !matches!(t, DataType::Money) {
            return Err(invalid(format!(
                "a currency list does not apply to a {t} field"
            )));
        }
        if (v.min_items.is_some() || v.max_items.is_some()) && !t.is_list() {
            return Err(invalid(format!("item limits do not apply to a {t} field")));
        }
        if let (Some(min), Some(max)) = (v.min_items, v.max_items) {
            if min > max {
                return Err(invalid("min_items is greater than max_items"));
            }
        }
        if (v.pattern.is_some() || v.format.is_some()) && !matches!(t, DataType::Text) {
            return Err(invalid(format!(
                "pattern and format rules do not apply to a {t} field"
            )));
        }
        if v.unique.is_some()
            && matches!(
                t,
                DataType::LongText | DataType::Json | DataType::Attachment | DataType::Bool
            )
        {
            return Err(invalid(format!("a {t} field cannot be unique")));
        }
        for rule in &v.cross_field {
            if rule.other == self.id {
                return Err(invalid("a cross-field rule compares the field with itself"));
            }
            if t.is_list() || matches!(t, DataType::Json | DataType::Reference) {
                return Err(invalid(format!(
                    "cross-field comparisons do not apply to a {t} field"
                )));
            }
        }
        Ok(())
    }
}

/// A named, ordered group of fields. Rendered as a section in record pages and forms and as a
/// two-level header in the grid.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FieldGroup {
    /// Stable ID.
    pub id: FieldGroupId,
    /// Display name.
    pub name: String,
    /// Member fields, in order.
    pub fields: Vec<FieldId>,
    /// Optional description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl FieldGroup {
    /// Verifies the group: a valid name and no field listed twice.
    ///
    /// # Errors
    /// [`CoreError::InvalidDefinition`].
    pub fn check(&self) -> Result<(), CoreError> {
        check_name("group", &self.name)?;
        for (index, field) in self.fields.iter().enumerate() {
            if self
                .fields
                .iter()
                .take(index)
                .any(|earlier| earlier == field)
            {
                return Err(invalid(format!(
                    "field {field} is listed twice in group `{}`",
                    clip(&self.name)
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod testkit {
    use super::*;

    pub(crate) fn fid(n: u128) -> FieldId {
        FieldId::from_u128(n)
    }

    pub(crate) fn opt(n: u128, label: &str, enabled: bool) -> InlineOption {
        InlineOption {
            id: RefItemId::from_u128(n),
            label: label.to_owned(),
            color: None,
            enabled,
        }
    }

    /// A valid field of every data type.
    pub(crate) fn field_of(data_type: DataType) -> FieldDef {
        let mut field = FieldDef::new(fid(1), "Field", data_type);
        if data_type.is_reference() {
            field.options = OptionsSource::Inline {
                options: vec![opt(1, "A", true), opt(2, "B", true), opt(9, "Old", false)],
            };
        }
        if matches!(data_type, DataType::Link) {
            field.options = OptionsSource::Table {
                table: TableId::from_u128(7),
            };
        }
        field
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::*;
    use super::*;
    use crate::value::testkit::dec;

    #[test]
    fn std_every_data_type_has_valid_default_field() {
        for data_type in DataType::ALL {
            let field = field_of(*data_type);
            field.check().unwrap();
            assert!(field.widget.supports(*data_type));
            assert_eq!(field.widget, Widget::default_for(*data_type));
            for widget in Widget::allowed_for(*data_type) {
                let mut f = field.clone();
                f.widget = *widget;
                f.check().unwrap();
            }
        }
    }

    #[test]
    fn std_string_enums_round_trip_text_and_serde() {
        for data_type in DataType::ALL {
            assert_eq!(data_type.as_str().parse::<DataType>().unwrap(), *data_type);
            let json = serde_json::to_string(data_type).unwrap();
            assert_eq!(json, format!("\"{data_type}\""));
            assert_eq!(serde_json::from_str::<DataType>(&json).unwrap(), *data_type);
        }
        for widget in Widget::ALL {
            assert_eq!(widget.to_string().parse::<Widget>().unwrap(), *widget);
        }
        for role in FieldRole::ALL {
            assert_eq!(role.as_str().parse::<FieldRole>().unwrap(), *role);
            assert!(!role.allowed_types().is_empty());
        }
        for c in Classification::ALL {
            assert_eq!(c.as_str().parse::<Classification>().unwrap(), *c);
        }
        for s in FieldStatus::ALL {
            assert_eq!(s.as_str().parse::<FieldStatus>().unwrap(), *s);
        }
        for s in UniqueScope::ALL {
            assert_eq!(s.as_str().parse::<UniqueScope>().unwrap(), *s);
        }
        for c in Comparison::ALL {
            assert_eq!(c.as_str().parse::<Comparison>().unwrap(), *c);
        }
    }

    #[test]
    fn invalid_unknown_enum_text_is_rejected() {
        assert!(matches!(
            "blob".parse::<DataType>(),
            Err(CoreError::UnknownCode {
                kind: "data type",
                ..
            })
        ));
        assert!("".parse::<Widget>().is_err());
        assert!(serde_json::from_str::<FieldStatus>("\"gone\"").is_err());
        assert!(serde_json::from_str::<DataType>("7").is_err());
    }

    #[test]
    fn std_every_widget_is_reachable_from_some_type() {
        for widget in Widget::ALL {
            assert!(
                DataType::ALL.iter().any(|t| widget.supports(*t)),
                "{widget} is orphaned"
            );
        }
    }

    #[test]
    fn std_type_predicates_partition_as_documented() {
        assert!(
            DataType::Text.is_text() && DataType::LongText.is_text() && !DataType::Json.is_text()
        );
        assert!(DataType::Duration.is_numeric() && !DataType::Date.is_numeric());
        assert!(DataType::Link.is_list() && !DataType::Reference.is_list());
        assert!(DataType::MultiReference.is_reference() && !DataType::Link.is_reference());
    }

    #[test]
    fn std_classification_orders_by_sensitivity() {
        assert!(Classification::Public < Classification::Internal);
        assert!(Classification::Confidential < Classification::Personal);
        assert!(Classification::Personal < Classification::SensitivePersonal);
        assert!(
            Classification::Personal.is_personal()
                && Classification::SensitivePersonal.is_personal()
        );
        assert!(!Classification::Confidential.is_personal());
    }

    #[test]
    fn std_field_status_semantics() {
        assert!(FieldStatus::Enabled.accepts_writes());
        assert!(!FieldStatus::Disabled.accepts_writes());
        assert!(!FieldStatus::Deprecated.accepts_writes());
        assert!(FieldStatus::Enabled.visible_by_default());
        assert!(!FieldStatus::Disabled.visible_by_default());
        assert!(FieldStatus::Deprecated.visible_by_default());
    }

    #[test]
    fn std_comparison_truth_table() {
        use core::cmp::Ordering::{Equal, Greater, Less};
        let table = [
            (Comparison::Lt, [true, false, false]),
            (Comparison::Le, [true, true, false]),
            (Comparison::Gt, [false, false, true]),
            (Comparison::Ge, [false, true, true]),
            (Comparison::Eq, [false, true, false]),
            (Comparison::Ne, [true, false, true]),
        ];
        for (comparison, expected) in table {
            for (ordering, want) in [Less, Equal, Greater].into_iter().zip(expected) {
                assert_eq!(
                    comparison.holds(ordering),
                    want,
                    "{comparison} {ordering:?}"
                );
            }
        }
    }

    #[test]
    fn std_mask_keeps_tail_and_shape() {
        let mask = Mask { visible_tail: 4 };
        assert_eq!(mask.apply("4111 1111 1111 1234"), "**** **** **** 1234");
        assert_eq!(mask.apply("jo@example.com"), "**@*******.com");
        assert_eq!(mask.apply("abc"), "abc");
        assert_eq!(mask.apply(""), "");
        assert_eq!(Mask { visible_tail: 0 }.apply("秘密"), "**");
    }

    #[test]
    fn std_serde_round_trip_full_definition() {
        let mut field = field_of(DataType::Text);
        field.format.prefix = Some("#".into());
        field.format.mask = Some(Mask { visible_tail: 2 });
        field.validation.required = true;
        field.validation.max_chars = Some(20);
        field.validation.pattern = Some(TextPattern::new(r"E\d{6}").unwrap());
        field.validation.unique = Some(UniqueScope::Table);
        field.role = Some(FieldRole::Title);
        field.group = Some(FieldGroupId::from_u128(3));
        field.help_text = Some("Employee number".into());
        field.check().unwrap();
        let json = serde_json::to_string(&field).unwrap();
        assert_eq!(serde_json::from_str::<FieldDef>(&json).unwrap(), field);

        let mut number = field_of(DataType::Number);
        number.validation.min = Some(dec("0"));
        number.validation.max = Some(dec("99.5"));
        number.validation.max_decimals = Some(1);
        number.format.decimals = Some(1);
        number.format.thousands = true;
        number.check().unwrap();
        let json = serde_json::to_string(&number).unwrap();
        assert!(json.contains("\"max\":\"99.5\""), "{json}");
        assert_eq!(serde_json::from_str::<FieldDef>(&json).unwrap(), number);
    }

    #[test]
    fn edge_minimal_json_uses_defaults() {
        let json = format!(
            r#"{{"id":"{}","name":"N","data_type":"bool","widget":"checkbox","classification":"internal","status":"enabled"}}"#,
            fid(5)
        );
        let field: FieldDef = serde_json::from_str(&json).unwrap();
        assert_eq!(field, FieldDef::new(fid(5), "N", DataType::Bool));
        let option: InlineOption = serde_json::from_str(&format!(
            r#"{{"id":"{}","label":"x"}}"#,
            RefItemId::from_u128(1)
        ))
        .unwrap();
        assert!(option.enabled);
    }

    fn reason(result: Result<(), CoreError>) -> String {
        match result {
            Err(CoreError::InvalidDefinition { reason }) => reason,
            other => panic!("expected InvalidDefinition, got {other:?}"),
        }
    }

    #[test]
    fn invalid_names() {
        let mut f = field_of(DataType::Text);
        f.name = "  ".into();
        assert!(reason(f.check()).contains("must not be empty"));
        f.name = "x".repeat(MAX_NAME_CHARS + 1);
        assert!(reason(f.check()).contains("longer than"));
        f.name = "a\nb".into();
        assert!(reason(f.check()).contains("control character"));
        f.name = "é".repeat(MAX_NAME_CHARS);
        f.check().unwrap();
    }

    #[test]
    fn invalid_widget_for_type() {
        let mut f = field_of(DataType::Bool);
        f.widget = Widget::Dropdown;
        assert!(reason(f.check()).contains("cannot edit"));
    }

    #[test]
    fn invalid_options() {
        let mut f = field_of(DataType::Reference);
        f.options = OptionsSource::None;
        assert!(reason(f.check()).contains("needs inline options"));
        let mut f = field_of(DataType::Link);
        f.options = OptionsSource::None;
        assert!(reason(f.check()).contains("target table"));
        let mut f = field_of(DataType::Text);
        f.options = OptionsSource::Inline { options: vec![] };
        assert!(reason(f.check()).contains("inline options cannot"));
        f.options = OptionsSource::RefSet {
            set: RefSetId::from_u128(1),
            subtree: None,
            leaves_only: false,
        };
        assert!(reason(f.check()).contains("reference set cannot"));
        f.options = OptionsSource::Table {
            table: TableId::from_u128(1),
        };
        assert!(reason(f.check()).contains("target table cannot"));

        let mut f = field_of(DataType::Reference);
        f.options = OptionsSource::Inline {
            options: vec![opt(1, "A", true), opt(1, "B", true)],
        };
        assert!(reason(f.check()).contains("used twice"));
        f.options = OptionsSource::Inline {
            options: vec![opt(1, "A", true), opt(2, "A", true)],
        };
        assert!(reason(f.check()).contains("label `A` is used twice"));
        f.options = OptionsSource::Inline {
            options: vec![opt(1, "", true)],
        };
        assert!(reason(f.check()).contains("option name"));
        f.options = OptionsSource::RefSet {
            set: RefSetId::from_u128(1),
            subtree: Some(RefItemId::from_u128(2)),
            leaves_only: true,
        };
        f.check().unwrap();
        assert!(f.inline_options().is_none());
        assert_eq!(
            field_of(DataType::Reference)
                .inline_options()
                .map(<[InlineOption]>::len),
            Some(3)
        );
    }

    #[test]
    fn edge_too_many_inline_options() {
        let mut f = field_of(DataType::MultiReference);
        let options = (0..=MAX_INLINE_OPTIONS)
            .map(|i| opt(i as u128, &format!("o{i}"), true))
            .collect();
        f.options = OptionsSource::Inline { options };
        assert!(reason(f.check()).contains("more than"));
    }

    #[test]
    fn invalid_format_parts() {
        let mut f = field_of(DataType::Text);
        f.format.prefix = Some("x".repeat(MAX_AFFIX_CHARS + 1));
        assert!(reason(f.check()).contains("prefix is longer"));
        f.format.prefix = None;
        f.format.suffix = Some("\t".into());
        assert!(reason(f.check()).contains("suffix contains"));
        f.format.suffix = None;
        f.format.decimals = Some(2);
        assert!(reason(f.check()).contains("display decimals do not apply"));
        f.format.decimals = None;
        f.format.thousands = true;
        assert!(reason(f.check()).contains("thousands"));

        let mut n = field_of(DataType::Number);
        n.format.decimals = Some(MAX_DECIMALS + 1);
        assert!(reason(n.check()).contains("display decimals above"));
        n.format.decimals = None;
        n.format.mask = Some(Mask { visible_tail: 1 });
        assert!(reason(n.check()).contains("mask"));
        n.format.mask = None;
        n.format.numbering = Some(NumberingPattern::new("N-{seq:3}").unwrap());
        assert!(reason(n.check()).contains("auto-numbering"));
    }

    #[test]
    fn std_auto_numbered_field() {
        let mut f = field_of(DataType::Text);
        assert!(!f.is_generated());
        f.format.numbering = Some(NumberingPattern::new("PO-{YYYY}-{seq:5}").unwrap());
        f.check().unwrap();
        assert!(f.is_generated());
        f.validation.max_chars = Some(5);
        assert!(reason(f.check()).contains("auto-numbered"));
    }

    #[test]
    fn invalid_validation_parts() {
        let mut f = field_of(DataType::Number);
        f.validation.max_chars = Some(3);
        assert!(reason(f.check()).contains("character limits"));
        let mut f = field_of(DataType::Text);
        f.validation.min_chars = Some(5);
        f.validation.max_chars = Some(3);
        assert!(reason(f.check()).contains("min_chars is greater"));
        let mut f = field_of(DataType::Text);
        f.validation.min = Some(dec("1"));
        assert!(reason(f.check()).contains("numeric range"));
        let mut f = field_of(DataType::Money);
        f.validation.min = Some(dec("5"));
        f.validation.max = Some(dec("1"));
        assert!(reason(f.check()).contains("min is greater than max"));
        let mut f = field_of(DataType::Money);
        f.validation.max_decimals = Some(2);
        assert!(reason(f.check()).contains("max_decimals does not apply"));
        let mut f = field_of(DataType::Percent);
        f.validation.max_decimals = Some(MAX_DECIMALS + 1);
        assert!(reason(f.check()).contains("max_decimals above"));
        let mut f = field_of(DataType::Number);
        f.validation.currencies = vec![Currency::new("USD").unwrap()];
        assert!(reason(f.check()).contains("currency list"));
        let mut f = field_of(DataType::Reference);
        f.validation.max_items = Some(2);
        assert!(reason(f.check()).contains("item limits"));
        let mut f = field_of(DataType::Link);
        f.validation.min_items = Some(3);
        f.validation.max_items = Some(2);
        assert!(reason(f.check()).contains("min_items is greater"));
        let mut f = field_of(DataType::LongText);
        f.validation.format = Some(FormatId::Email);
        assert!(reason(f.check()).contains("pattern and format"));
        let mut f = field_of(DataType::Json);
        f.validation.unique = Some(UniqueScope::Table);
        assert!(reason(f.check()).contains("cannot be unique"));
        let mut f = field_of(DataType::Date);
        f.validation.cross_field = vec![CrossFieldRule {
            comparison: Comparison::Ge,
            other: f.id,
        }];
        assert!(reason(f.check()).contains("with itself"));
        let mut f = field_of(DataType::Link);
        f.validation.cross_field = vec![CrossFieldRule {
            comparison: Comparison::Eq,
            other: fid(2),
        }];
        assert!(reason(f.check()).contains("cross-field comparisons"));
        let mut f = field_of(DataType::Date);
        f.validation.cross_field = vec![CrossFieldRule {
            comparison: Comparison::Ge,
            other: fid(2),
        }];
        f.check().unwrap();
    }

    #[test]
    fn invalid_role_for_type() {
        let mut f = field_of(DataType::Text);
        f.role = Some(FieldRole::TimeDue);
        assert!(reason(f.check()).contains("role time.due"));
        let mut f = field_of(DataType::Money);
        f.role = Some(FieldRole::MoneyAmount);
        f.check().unwrap();
    }

    #[test]
    fn std_field_group_checks() {
        let mut group = FieldGroup {
            id: FieldGroupId::from_u128(1),
            name: "Contact".into(),
            fields: vec![fid(1), fid(2)],
            description: None,
        };
        group.check().unwrap();
        let json = serde_json::to_string(&group).unwrap();
        assert_eq!(serde_json::from_str::<FieldGroup>(&json).unwrap(), group);
        group.fields.push(fid(1));
        assert!(reason(group.check()).contains("listed twice"));
        group.fields.clear();
        group.name = String::new();
        assert!(reason(group.check()).contains("group name"));
    }
}

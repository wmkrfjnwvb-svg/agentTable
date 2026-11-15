//! The field types a person picks from when adding a field, in eight groups (product design
//! doc 16 §3), plus custom field types and the field library.
//!
//! A *field type* is not a storage type. Storage has fifteen [`DataType`]s; the picker offers
//! about forty things people recognise — "Email", "Rating", "Auto number", "Lifecycle state" —
//! each of which is a [`FieldDef`] with some facets preset. [`BuiltinType::template`] writes
//! that definition, [`BuiltinType::requirements`] says what the designer still has to supply
//! (options, a target table, a formula…), and [`BuiltinType::of`] recognises the type of an
//! existing definition, so the grid can show the right name and icon.
//!
//! * [`search`] finds types by name or synonym ("dropdown" → Single select).
//! * [`suggest`] proposes a type for a sample value ("a@b.co" → Email).
//! * A [`FieldTypeDef`] is a tenant's own named type ("Employee number").
//! * A [`FieldLibraryItem`] is a complete, reusable definition ("Cost centre").

use crate::coerce;
use crate::decimal::Decimal;
use crate::error::{clip, CoreError};
use crate::field::{
    string_enum, Classification, Computed, ComputedKind, DataType, DisplayFormat, FieldDef,
    FieldRole, OptionsSource, SystemField, ValidationSpec, Widget,
};
use crate::format::FormatId;
use crate::id::{FieldId, FieldLibraryItemId, FieldTypeId};
use core::fmt;
use core::str::FromStr;

string_enum! {
    /// The groups of the field picker, in the order they are shown.
    PickerGroup, "picker group";
    /// Everyday data.
    Basic => "basic",
    /// People and organisation: these carry permissions and notifications.
    People => "people",
    /// Validated business formats.
    Business => "business",
    /// Governed values from reference data.
    Reference => "reference",
    /// Links to other records and values calculated from them.
    Relational => "relational",
    /// Fields that drive the process.
    Process => "process",
    /// Fields filled by an AI skill.
    Ai => "ai",
    /// Read-only fields the platform maintains.
    System => "system",
}

string_enum! {
    /// A field type offered by the field picker.
    BuiltinType, "field type";
    /// Single-line text.
    Text => "text",
    /// Multi-line rich text.
    LongText => "long_text",
    /// A number.
    Number => "number",
    /// An amount with its currency.
    Currency => "currency",
    /// A percentage.
    Percent => "percent",
    /// A calendar day.
    Date => "date",
    /// A date and time.
    DateTime => "datetime",
    /// A length of time.
    Duration => "duration",
    /// Yes or no.
    Checkbox => "checkbox",
    /// One choice from options defined on the field.
    SingleSelect => "single_select",
    /// Several choices from options defined on the field.
    MultiSelect => "multi_select",
    /// Files.
    Attachment => "attachment",
    /// A web address.
    Url => "url",
    /// People or teams.
    Person => "person",
    /// The other side of a business relationship: customer, supplier, candidate.
    Party => "party",
    /// A unit of the organisation.
    OrgUnit => "org_unit",
    /// A phone number.
    Phone => "phone",
    /// An email address.
    Email => "email",
    /// A postal address.
    Address => "address",
    /// A place on the map.
    Location => "location",
    /// A barcode or QR code, scanned or typed.
    Barcode => "barcode",
    /// A rating from 1 to 5.
    Rating => "rating",
    /// Progress from 0 to 100 %.
    Progress => "progress",
    /// A handwritten signature.
    Signature => "signature",
    /// A number assigned automatically (`PO-2026-00042`).
    AutoNumber => "auto_number",
    /// A tax or business registration number in a market's format.
    TaxId => "tax_id",
    /// One value of a code list.
    CodeList => "code_list",
    /// Several values of a code list.
    CodeListMulti => "code_list_multi",
    /// One value of a tree-shaped code list, chosen level by level.
    Cascade => "cascade",
    /// An item of a catalogue.
    CatalogueItem => "catalogue_item",
    /// Links to records of another table.
    Link => "link",
    /// The values of a field in linked records.
    Lookup => "lookup",
    /// A total, count or other summary over linked records.
    Rollup => "rollup",
    /// A value calculated by a formula.
    Formula => "formula",
    /// Child records edited inline, with totals.
    LineItems => "line_items",
    /// The record's state in its lifecycle.
    LifecycleState => "lifecycle_state",
    /// Who is doing the work.
    Assignment => "assignment",
    /// When the work is due.
    DueDate => "due_date",
    /// A typed value produced by an AI skill.
    AiField => "ai_field",
    /// When the record was created.
    CreatedAt => "created_at",
    /// Who created the record.
    CreatedBy => "created_by",
    /// When the record was last changed.
    ModifiedAt => "modified_at",
    /// Who changed the record last.
    ModifiedBy => "modified_by",
    /// The record's id.
    RecordId => "record_id",
    /// The record's version.
    RecordVersion => "record_version",
    /// The data source the record came from.
    RecordSource => "record_source",
    /// The trace of the last change.
    TraceLink => "trace_link",
}

string_enum! {
    /// Something the designer must supply before a definition written by
    /// [`BuiltinType::template`] is complete.
    Requirement, "field requirement";
    /// The options of the field.
    Options => "options",
    /// The reference set the values come from.
    RefSet => "ref_set",
    /// The table the field links to.
    LinkTarget => "link_target",
    /// The formula.
    Formula => "formula",
    /// The numbering pattern.
    Numbering => "numbering",
    /// The AI skill that fills the field.
    Skill => "skill",
    /// Which standard format applies (for example which country's tax number).
    Format => "format",
}

impl BuiltinType {
    /// The picker group the type is offered in.
    #[must_use]
    pub const fn group(self) -> PickerGroup {
        match self {
            BuiltinType::Text
            | BuiltinType::LongText
            | BuiltinType::Number
            | BuiltinType::Currency
            | BuiltinType::Percent
            | BuiltinType::Date
            | BuiltinType::DateTime
            | BuiltinType::Duration
            | BuiltinType::Checkbox
            | BuiltinType::SingleSelect
            | BuiltinType::MultiSelect
            | BuiltinType::Attachment
            | BuiltinType::Url => PickerGroup::Basic,
            BuiltinType::Person | BuiltinType::Party | BuiltinType::OrgUnit => PickerGroup::People,
            BuiltinType::Phone
            | BuiltinType::Email
            | BuiltinType::Address
            | BuiltinType::Location
            | BuiltinType::Barcode
            | BuiltinType::Rating
            | BuiltinType::Progress
            | BuiltinType::Signature
            | BuiltinType::AutoNumber
            | BuiltinType::TaxId => PickerGroup::Business,
            BuiltinType::CodeList
            | BuiltinType::CodeListMulti
            | BuiltinType::Cascade
            | BuiltinType::CatalogueItem => PickerGroup::Reference,
            BuiltinType::Link
            | BuiltinType::Lookup
            | BuiltinType::Rollup
            | BuiltinType::Formula
            | BuiltinType::LineItems => PickerGroup::Relational,
            BuiltinType::LifecycleState | BuiltinType::Assignment | BuiltinType::DueDate => {
                PickerGroup::Process
            }
            BuiltinType::AiField => PickerGroup::Ai,
            BuiltinType::CreatedAt
            | BuiltinType::CreatedBy
            | BuiltinType::ModifiedAt
            | BuiltinType::ModifiedBy
            | BuiltinType::RecordId
            | BuiltinType::RecordVersion
            | BuiltinType::RecordSource
            | BuiltinType::TraceLink => PickerGroup::System,
        }
    }

    /// The name shown in the picker (English; the UI translates by the stable key).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            BuiltinType::Text => "Text",
            BuiltinType::LongText => "Long text",
            BuiltinType::Number => "Number",
            BuiltinType::Currency => "Currency",
            BuiltinType::Percent => "Percent",
            BuiltinType::Date => "Date",
            BuiltinType::DateTime => "Date and time",
            BuiltinType::Duration => "Duration",
            BuiltinType::Checkbox => "Checkbox",
            BuiltinType::SingleSelect => "Single select",
            BuiltinType::MultiSelect => "Multiple select",
            BuiltinType::Attachment => "Attachment",
            BuiltinType::Url => "URL",
            BuiltinType::Person => "Person",
            BuiltinType::Party => "Party",
            BuiltinType::OrgUnit => "Organisation unit",
            BuiltinType::Phone => "Phone",
            BuiltinType::Email => "Email",
            BuiltinType::Address => "Address",
            BuiltinType::Location => "Location",
            BuiltinType::Barcode => "Barcode",
            BuiltinType::Rating => "Rating",
            BuiltinType::Progress => "Progress",
            BuiltinType::Signature => "Signature",
            BuiltinType::AutoNumber => "Auto number",
            BuiltinType::TaxId => "Tax or business ID",
            BuiltinType::CodeList => "Code list",
            BuiltinType::CodeListMulti => "Code list (multiple)",
            BuiltinType::Cascade => "Cascading code list",
            BuiltinType::CatalogueItem => "Catalogue item",
            BuiltinType::Link => "Link to records",
            BuiltinType::Lookup => "Lookup",
            BuiltinType::Rollup => "Roll-up",
            BuiltinType::Formula => "Formula",
            BuiltinType::LineItems => "Line items",
            BuiltinType::LifecycleState => "Lifecycle state",
            BuiltinType::Assignment => "Assignment",
            BuiltinType::DueDate => "Due date",
            BuiltinType::AiField => "AI field",
            BuiltinType::CreatedAt => "Created at",
            BuiltinType::CreatedBy => "Created by",
            BuiltinType::ModifiedAt => "Modified at",
            BuiltinType::ModifiedBy => "Modified by",
            BuiltinType::RecordId => "Record ID",
            BuiltinType::RecordVersion => "Version",
            BuiltinType::RecordSource => "Source",
            BuiltinType::TraceLink => "Trace",
        }
    }

    /// Other words people search for (lower case).
    #[must_use]
    pub const fn synonyms(self) -> &'static [&'static str] {
        match self {
            BuiltinType::Text => &["string", "single line", "name", "title"],
            BuiltinType::LongText => &["notes", "description", "rich text", "paragraph", "memo"],
            BuiltinType::Number => &["integer", "decimal", "quantity", "count"],
            BuiltinType::Currency => &["money", "amount", "price", "cost", "budget"],
            BuiltinType::Percent => &["percentage", "rate", "ratio"],
            BuiltinType::Date => &["day", "deadline", "calendar"],
            BuiltinType::DateTime => &["time", "timestamp", "datetime"],
            BuiltinType::Duration => &["time spent", "hours", "effort", "elapsed"],
            BuiltinType::Checkbox => &["boolean", "yes/no", "toggle", "flag", "tick"],
            BuiltinType::SingleSelect => &["dropdown", "choice", "option", "radio", "category"],
            BuiltinType::MultiSelect => &["tags", "labels", "multi select", "checkboxes"],
            BuiltinType::Attachment => &["file", "upload", "document", "image", "photo"],
            BuiltinType::Url => &["link", "website", "web address", "hyperlink"],
            BuiltinType::Person => &["user", "member", "people", "team", "group", "owner"],
            BuiltinType::Party => &["customer", "supplier", "vendor", "candidate", "contact"],
            BuiltinType::OrgUnit => &["department", "division", "cost centre", "cost center"],
            BuiltinType::Phone => &["telephone", "mobile", "cell"],
            BuiltinType::Email => &["e-mail", "mail"],
            BuiltinType::Address => &["street", "postal address", "shipping address"],
            BuiltinType::Location => &["map", "gps", "coordinates", "place", "geo"],
            BuiltinType::Barcode => &["qr", "qr code", "scan", "sku"],
            BuiltinType::Rating => &["stars", "score", "priority"],
            BuiltinType::Progress => &["completion", "percent done", "progress bar"],
            BuiltinType::Signature => &["sign", "autograph", "sign-off"],
            BuiltinType::AutoNumber => &["sequence", "serial", "numbering", "id", "order number"],
            BuiltinType::TaxId => &[
                "vat",
                "gstin",
                "ein",
                "pan",
                "business number",
                "tax number",
            ],
            BuiltinType::CodeList => &["reference data", "master data", "lookup list", "enum"],
            BuiltinType::CodeListMulti => &["reference data", "master data", "multiple codes"],
            BuiltinType::Cascade => &["cascading", "hierarchy", "tree", "country region city"],
            BuiltinType::CatalogueItem => &["product", "item", "material", "service", "catalog"],
            BuiltinType::Link => &["relation", "linked record", "foreign key", "reference"],
            BuiltinType::Lookup => &["linked value", "pull value", "vlookup"],
            BuiltinType::Rollup => &["rollup", "sum of linked", "aggregate", "total", "count"],
            BuiltinType::Formula => &["calculation", "computed", "expression", "calculated"],
            BuiltinType::LineItems => &["lines", "child table", "sub-table", "order lines"],
            BuiltinType::LifecycleState => &["status", "state", "stage", "workflow", "process"],
            BuiltinType::Assignment => &["assignee", "assigned to", "responsible", "handler"],
            BuiltinType::DueDate => &["due", "deadline", "sla", "target date"],
            BuiltinType::AiField => &[
                "ai",
                "classify",
                "extract",
                "summarise",
                "summarize",
                "translate",
            ],
            BuiltinType::CreatedAt => &["created time", "creation date"],
            BuiltinType::CreatedBy => &["creator", "author"],
            BuiltinType::ModifiedAt => &["updated at", "last modified", "last edited time"],
            BuiltinType::ModifiedBy => &["updated by", "last editor"],
            BuiltinType::RecordId => &["id", "identifier", "key"],
            BuiltinType::RecordVersion => &["revision"],
            BuiltinType::RecordSource => &["data source", "origin", "imported from"],
            BuiltinType::TraceLink => &["audit", "history", "trace id"],
        }
    }

    /// What the designer must still supply after [`BuiltinType::template`].
    #[must_use]
    pub const fn requirements(self) -> &'static [Requirement] {
        match self {
            BuiltinType::SingleSelect | BuiltinType::MultiSelect => &[Requirement::Options],
            BuiltinType::OrgUnit
            | BuiltinType::CodeList
            | BuiltinType::CodeListMulti
            | BuiltinType::Cascade
            | BuiltinType::LifecycleState => &[Requirement::RefSet],
            BuiltinType::Party
            | BuiltinType::CatalogueItem
            | BuiltinType::Link
            | BuiltinType::LineItems => &[Requirement::LinkTarget],
            BuiltinType::Lookup | BuiltinType::Rollup | BuiltinType::Formula => {
                &[Requirement::Formula]
            }
            BuiltinType::AutoNumber => &[Requirement::Numbering],
            BuiltinType::TaxId => &[Requirement::Format],
            BuiltinType::AiField => &[Requirement::Skill],
            BuiltinType::Text
            | BuiltinType::LongText
            | BuiltinType::Number
            | BuiltinType::Currency
            | BuiltinType::Percent
            | BuiltinType::Date
            | BuiltinType::DateTime
            | BuiltinType::Duration
            | BuiltinType::Checkbox
            | BuiltinType::Attachment
            | BuiltinType::Url
            | BuiltinType::Person
            | BuiltinType::Phone
            | BuiltinType::Email
            | BuiltinType::Address
            | BuiltinType::Location
            | BuiltinType::Barcode
            | BuiltinType::Rating
            | BuiltinType::Progress
            | BuiltinType::Signature
            | BuiltinType::Assignment
            | BuiltinType::DueDate
            | BuiltinType::CreatedAt
            | BuiltinType::CreatedBy
            | BuiltinType::ModifiedAt
            | BuiltinType::ModifiedBy
            | BuiltinType::RecordId
            | BuiltinType::RecordVersion
            | BuiltinType::RecordSource
            | BuiltinType::TraceLink => &[],
        }
    }

    /// The system field behind a system type.
    const fn system(self) -> Option<SystemField> {
        match self {
            BuiltinType::CreatedAt => Some(SystemField::CreatedAt),
            BuiltinType::CreatedBy => Some(SystemField::CreatedBy),
            BuiltinType::ModifiedAt => Some(SystemField::ModifiedAt),
            BuiltinType::ModifiedBy => Some(SystemField::ModifiedBy),
            BuiltinType::RecordId => Some(SystemField::RecordId),
            BuiltinType::RecordVersion => Some(SystemField::Version),
            BuiltinType::RecordSource => Some(SystemField::Source),
            BuiltinType::TraceLink => Some(SystemField::Trace),
            BuiltinType::Text
            | BuiltinType::LongText
            | BuiltinType::Number
            | BuiltinType::Currency
            | BuiltinType::Percent
            | BuiltinType::Date
            | BuiltinType::DateTime
            | BuiltinType::Duration
            | BuiltinType::Checkbox
            | BuiltinType::SingleSelect
            | BuiltinType::MultiSelect
            | BuiltinType::Attachment
            | BuiltinType::Url
            | BuiltinType::Person
            | BuiltinType::Party
            | BuiltinType::OrgUnit
            | BuiltinType::Phone
            | BuiltinType::Email
            | BuiltinType::Address
            | BuiltinType::Location
            | BuiltinType::Barcode
            | BuiltinType::Rating
            | BuiltinType::Progress
            | BuiltinType::Signature
            | BuiltinType::AutoNumber
            | BuiltinType::TaxId
            | BuiltinType::CodeList
            | BuiltinType::CodeListMulti
            | BuiltinType::Cascade
            | BuiltinType::CatalogueItem
            | BuiltinType::Link
            | BuiltinType::Lookup
            | BuiltinType::Rollup
            | BuiltinType::Formula
            | BuiltinType::LineItems
            | BuiltinType::LifecycleState
            | BuiltinType::Assignment
            | BuiltinType::DueDate
            | BuiltinType::AiField => None,
        }
    }

    /// The storage type and input control the template starts with. For computed and AI
    /// fields the storage type follows the formula or the skill's output; the template
    /// starts as text.
    #[must_use]
    pub const fn storage(self) -> (DataType, Widget) {
        match self {
            BuiltinType::Text
            | BuiltinType::Url
            | BuiltinType::Phone
            | BuiltinType::Email
            | BuiltinType::AutoNumber
            | BuiltinType::TaxId
            | BuiltinType::Lookup
            | BuiltinType::Rollup
            | BuiltinType::Formula
            | BuiltinType::AiField
            | BuiltinType::RecordId
            | BuiltinType::RecordSource
            | BuiltinType::TraceLink => (DataType::Text, Widget::TextBox),
            BuiltinType::Barcode => (DataType::Text, Widget::Barcode),
            BuiltinType::LongText => (DataType::LongText, Widget::RichText),
            BuiltinType::Address => (DataType::LongText, Widget::Address),
            BuiltinType::Number | BuiltinType::RecordVersion => {
                (DataType::Number, Widget::NumberInput)
            }
            BuiltinType::Rating => (DataType::Number, Widget::Rating),
            BuiltinType::Currency => (DataType::Money, Widget::MoneyInput),
            BuiltinType::Percent => (DataType::Percent, Widget::NumberInput),
            BuiltinType::Progress => (DataType::Percent, Widget::Progress),
            BuiltinType::Date => (DataType::Date, Widget::DatePicker),
            BuiltinType::DateTime
            | BuiltinType::DueDate
            | BuiltinType::CreatedAt
            | BuiltinType::ModifiedAt => (DataType::DateTime, Widget::DateTimePicker),
            BuiltinType::Duration => (DataType::Duration, Widget::DurationInput),
            BuiltinType::Checkbox => (DataType::Bool, Widget::Checkbox),
            BuiltinType::SingleSelect | BuiltinType::CodeList => {
                (DataType::Reference, Widget::Dropdown)
            }
            BuiltinType::LifecycleState => (DataType::Reference, Widget::Segmented),
            BuiltinType::Cascade => (DataType::Reference, Widget::Cascader),
            BuiltinType::OrgUnit => (DataType::Reference, Widget::TreePicker),
            BuiltinType::MultiSelect => (DataType::MultiReference, Widget::MultiSelect),
            BuiltinType::CodeListMulti => (DataType::MultiReference, Widget::SearchPicker),
            BuiltinType::Attachment => (DataType::Attachment, Widget::FileUpload),
            BuiltinType::Signature => (DataType::Attachment, Widget::Signature),
            BuiltinType::Person
            | BuiltinType::Assignment
            | BuiltinType::CreatedBy
            | BuiltinType::ModifiedBy => (DataType::Principal, Widget::PrincipalPicker),
            BuiltinType::Party | BuiltinType::CatalogueItem | BuiltinType::Link => {
                (DataType::Link, Widget::LinkPicker)
            }
            BuiltinType::LineItems => (DataType::Link, Widget::LineItems),
            BuiltinType::Location => (DataType::Json, Widget::Location),
        }
    }

    /// The role the type gives the field, if it has one by nature.
    const fn role(self) -> Option<FieldRole> {
        match self {
            BuiltinType::Party => Some(FieldRole::PartyCounterparty),
            BuiltinType::OrgUnit => Some(FieldRole::Dimension),
            BuiltinType::CatalogueItem => Some(FieldRole::Item),
            BuiltinType::LifecycleState => Some(FieldRole::State),
            BuiltinType::Assignment => Some(FieldRole::PersonAssignee),
            BuiltinType::DueDate => Some(FieldRole::TimeDue),
            BuiltinType::Text
            | BuiltinType::LongText
            | BuiltinType::Number
            | BuiltinType::Currency
            | BuiltinType::Percent
            | BuiltinType::Date
            | BuiltinType::DateTime
            | BuiltinType::Duration
            | BuiltinType::Checkbox
            | BuiltinType::SingleSelect
            | BuiltinType::MultiSelect
            | BuiltinType::Attachment
            | BuiltinType::Url
            | BuiltinType::Person
            | BuiltinType::Phone
            | BuiltinType::Email
            | BuiltinType::Address
            | BuiltinType::Location
            | BuiltinType::Barcode
            | BuiltinType::Rating
            | BuiltinType::Progress
            | BuiltinType::Signature
            | BuiltinType::AutoNumber
            | BuiltinType::TaxId
            | BuiltinType::CodeList
            | BuiltinType::CodeListMulti
            | BuiltinType::Cascade
            | BuiltinType::Link
            | BuiltinType::Lookup
            | BuiltinType::Rollup
            | BuiltinType::Formula
            | BuiltinType::LineItems
            | BuiltinType::AiField
            | BuiltinType::CreatedAt
            | BuiltinType::CreatedBy
            | BuiltinType::ModifiedAt
            | BuiltinType::ModifiedBy
            | BuiltinType::RecordId
            | BuiltinType::RecordVersion
            | BuiltinType::RecordSource
            | BuiltinType::TraceLink => None,
        }
    }

    /// The kind of computed field the type creates, if it creates one. The computed facet
    /// itself is attached together with the formula (by `mt-formula`, which checks it).
    #[must_use]
    pub const fn computed_kind(self) -> Option<ComputedKind> {
        match self {
            BuiltinType::Lookup => Some(ComputedKind::Lookup),
            BuiltinType::Rollup => Some(ComputedKind::Rollup),
            BuiltinType::Formula => Some(ComputedKind::Formula),
            BuiltinType::Text
            | BuiltinType::LongText
            | BuiltinType::Number
            | BuiltinType::Currency
            | BuiltinType::Percent
            | BuiltinType::Date
            | BuiltinType::DateTime
            | BuiltinType::Duration
            | BuiltinType::Checkbox
            | BuiltinType::SingleSelect
            | BuiltinType::MultiSelect
            | BuiltinType::Attachment
            | BuiltinType::Url
            | BuiltinType::Person
            | BuiltinType::Party
            | BuiltinType::OrgUnit
            | BuiltinType::Phone
            | BuiltinType::Email
            | BuiltinType::Address
            | BuiltinType::Location
            | BuiltinType::Barcode
            | BuiltinType::Rating
            | BuiltinType::Progress
            | BuiltinType::Signature
            | BuiltinType::AutoNumber
            | BuiltinType::TaxId
            | BuiltinType::CodeList
            | BuiltinType::CodeListMulti
            | BuiltinType::Cascade
            | BuiltinType::CatalogueItem
            | BuiltinType::Link
            | BuiltinType::LineItems
            | BuiltinType::LifecycleState
            | BuiltinType::Assignment
            | BuiltinType::DueDate
            | BuiltinType::AiField
            | BuiltinType::CreatedAt
            | BuiltinType::CreatedBy
            | BuiltinType::ModifiedAt
            | BuiltinType::ModifiedBy
            | BuiltinType::RecordId
            | BuiltinType::RecordVersion
            | BuiltinType::RecordSource
            | BuiltinType::TraceLink => None,
        }
    }

    /// Writes the definition of a new field of this type: storage type, input control,
    /// standard format, value range, role, privacy classification and the system facet are
    /// preset. What [`BuiltinType::requirements`] lists is left for the designer; until
    /// then [`FieldDef::check`] refuses the definition and says what is missing.
    #[must_use]
    pub fn template(self, id: FieldId, name: impl Into<String>) -> FieldDef {
        let (data_type, widget) = self.storage();
        let mut field = FieldDef::new(id, name, data_type);
        field.widget = widget;
        field.role = self.role();
        field.system = self.system();
        let zero_to = |max: i64| (Some(Decimal::ZERO), Some(Decimal::from(max)));
        match self {
            BuiltinType::Url => field.validation.format = Some(FormatId::Url),
            BuiltinType::Email => {
                field.validation.format = Some(FormatId::Email);
                field.classification = Classification::Personal;
            }
            BuiltinType::Phone => {
                field.validation.format = Some(FormatId::PhoneE164);
                field.classification = Classification::Personal;
            }
            BuiltinType::Address | BuiltinType::Location | BuiltinType::Signature => {
                field.classification = Classification::Personal;
                if self == BuiltinType::Signature {
                    field.validation.max_items = Some(1);
                }
            }
            BuiltinType::Rating => {
                (field.validation.min, field.validation.max) = zero_to(5);
                field.validation.max_decimals = Some(0);
            }
            BuiltinType::Progress => {
                (field.validation.min, field.validation.max) = zero_to(1);
            }
            BuiltinType::Text
            | BuiltinType::LongText
            | BuiltinType::Number
            | BuiltinType::Currency
            | BuiltinType::Percent
            | BuiltinType::Date
            | BuiltinType::DateTime
            | BuiltinType::Duration
            | BuiltinType::Checkbox
            | BuiltinType::SingleSelect
            | BuiltinType::MultiSelect
            | BuiltinType::Attachment
            | BuiltinType::Person
            | BuiltinType::Party
            | BuiltinType::OrgUnit
            | BuiltinType::Barcode
            | BuiltinType::AutoNumber
            | BuiltinType::TaxId
            | BuiltinType::CodeList
            | BuiltinType::CodeListMulti
            | BuiltinType::Cascade
            | BuiltinType::CatalogueItem
            | BuiltinType::Link
            | BuiltinType::Lookup
            | BuiltinType::Rollup
            | BuiltinType::Formula
            | BuiltinType::LineItems
            | BuiltinType::LifecycleState
            | BuiltinType::Assignment
            | BuiltinType::DueDate
            | BuiltinType::AiField
            | BuiltinType::CreatedAt
            | BuiltinType::CreatedBy
            | BuiltinType::ModifiedAt
            | BuiltinType::ModifiedBy
            | BuiltinType::RecordId
            | BuiltinType::RecordVersion
            | BuiltinType::RecordSource
            | BuiltinType::TraceLink => {}
        }
        field
    }

    /// The picker type of an existing definition: what the grid shows as the field's type.
    /// The most specific type that fits is chosen, so a text field with the email format is
    /// "Email" and a reference field with the state role is "Lifecycle state".
    #[must_use]
    pub fn of(field: &FieldDef) -> BuiltinType {
        if let Some(system) = field.system {
            return match system {
                SystemField::CreatedAt => BuiltinType::CreatedAt,
                SystemField::CreatedBy => BuiltinType::CreatedBy,
                SystemField::ModifiedAt => BuiltinType::ModifiedAt,
                SystemField::ModifiedBy => BuiltinType::ModifiedBy,
                SystemField::RecordId => BuiltinType::RecordId,
                SystemField::Version => BuiltinType::RecordVersion,
                SystemField::Source => BuiltinType::RecordSource,
                SystemField::Trace => BuiltinType::TraceLink,
            };
        }
        if let Some(Computed { kind, .. }) = &field.computed {
            return match kind {
                ComputedKind::Formula => BuiltinType::Formula,
                ComputedKind::Lookup => BuiltinType::Lookup,
                ComputedKind::Rollup => BuiltinType::Rollup,
            };
        }
        if field.ai_skill.is_some() {
            return BuiltinType::AiField;
        }
        let from_set = matches!(
            field.options,
            OptionsSource::RefSet { .. } | OptionsSource::Parameter { .. }
        );
        match field.data_type {
            DataType::Text => {
                if field.format.numbering.is_some() {
                    return BuiltinType::AutoNumber;
                }
                if field.widget == Widget::Barcode {
                    return BuiltinType::Barcode;
                }
                match field.validation.format {
                    Some(FormatId::Email) => BuiltinType::Email,
                    Some(FormatId::Url) => BuiltinType::Url,
                    Some(FormatId::PhoneE164) => BuiltinType::Phone,
                    Some(
                        FormatId::UsEin
                        | FormatId::DeVatId
                        | FormatId::JpCorporateNumber
                        | FormatId::JpInvoiceNumber
                        | FormatId::KrBusinessNumber
                        | FormatId::InGstin
                        | FormatId::InPan,
                    ) => BuiltinType::TaxId,
                    Some(
                        FormatId::PostalUs
                        | FormatId::PostalJp
                        | FormatId::PostalKr
                        | FormatId::PostalDe
                        | FormatId::PostalIn
                        | FormatId::Iban
                        | FormatId::Bic
                        | FormatId::UsAbaRouting
                        | FormatId::CountryCode
                        | FormatId::CurrencyCode
                        | FormatId::LanguageCode
                        | FormatId::Uuid,
                    )
                    | None => BuiltinType::Text,
                }
            }
            DataType::LongText => {
                if field.widget == Widget::Address {
                    BuiltinType::Address
                } else {
                    BuiltinType::LongText
                }
            }
            DataType::Number => {
                if field.widget == Widget::Rating {
                    BuiltinType::Rating
                } else {
                    BuiltinType::Number
                }
            }
            DataType::Money => BuiltinType::Currency,
            DataType::Percent => {
                if field.widget == Widget::Progress {
                    BuiltinType::Progress
                } else {
                    BuiltinType::Percent
                }
            }
            DataType::Bool => BuiltinType::Checkbox,
            DataType::Date => BuiltinType::Date,
            DataType::DateTime => {
                if field.role == Some(FieldRole::TimeDue) {
                    BuiltinType::DueDate
                } else {
                    BuiltinType::DateTime
                }
            }
            DataType::Duration => BuiltinType::Duration,
            DataType::Reference => {
                if field.role == Some(FieldRole::State) {
                    BuiltinType::LifecycleState
                } else if field.role == Some(FieldRole::Dimension)
                    && field.widget == Widget::TreePicker
                {
                    BuiltinType::OrgUnit
                } else if field.widget == Widget::Cascader || field.options.cascade_from().is_some()
                {
                    BuiltinType::Cascade
                } else if from_set {
                    BuiltinType::CodeList
                } else {
                    BuiltinType::SingleSelect
                }
            }
            DataType::MultiReference => {
                if from_set {
                    BuiltinType::CodeListMulti
                } else {
                    BuiltinType::MultiSelect
                }
            }
            DataType::Link => {
                if field.widget == Widget::LineItems {
                    BuiltinType::LineItems
                } else if field.role == Some(FieldRole::Item) {
                    BuiltinType::CatalogueItem
                } else if matches!(
                    field.role,
                    Some(FieldRole::PartyCounterparty | FieldRole::PartyRequester)
                ) {
                    BuiltinType::Party
                } else {
                    BuiltinType::Link
                }
            }
            DataType::Principal => {
                if field.role == Some(FieldRole::PersonAssignee) {
                    BuiltinType::Assignment
                } else {
                    BuiltinType::Person
                }
            }
            DataType::Attachment => {
                if field.widget == Widget::Signature {
                    BuiltinType::Signature
                } else {
                    BuiltinType::Attachment
                }
            }
            DataType::Json => {
                if field.widget == Widget::Location {
                    BuiltinType::Location
                } else {
                    // Free-form structured data has no picker type of its own.
                    BuiltinType::LongText
                }
            }
        }
    }
}

impl PickerGroup {
    /// The types offered in this group, in picker order.
    #[must_use]
    pub fn types(self) -> Vec<BuiltinType> {
        BuiltinType::ALL
            .iter()
            .copied()
            .filter(|t| t.group() == self)
            .collect()
    }
}

fn fold(text: &str) -> String {
    text.trim().to_lowercase()
}

/// Finds field types by what a person types into the picker's search box: part of the name,
/// of the stable key or of a synonym, ignoring case. Types whose name starts with the query
/// come first, then other name matches, then synonym matches; within each, picker order. An
/// empty query lists every type.
#[must_use]
pub fn search(query: &str) -> Vec<BuiltinType> {
    let wanted = fold(query);
    let rank = |t: &BuiltinType| -> Option<u8> {
        let label = fold(t.label());
        if wanted.is_empty() || label.starts_with(&wanted) {
            Some(0)
        } else if label.contains(&wanted) || t.as_str().contains(&wanted) {
            Some(1)
        } else if t.synonyms().iter().any(|synonym| synonym.contains(&wanted)) {
            Some(2)
        } else {
            None
        }
    };
    let mut found: Vec<(u8, BuiltinType)> = BuiltinType::ALL
        .iter()
        .filter_map(|t| rank(t).map(|rank| (rank, *t)))
        .collect();
    found.sort_by_key(|(rank, t)| (*rank, *t));
    found.into_iter().map(|(_, t)| t).collect()
}

/// Proposes a field type for a sample value, by what the text is written as. This is the
/// deterministic first guess; an AI skill may refine it with the column's name and more
/// samples.
#[must_use]
pub fn suggest(sample: &str) -> BuiltinType {
    let text = sample.trim();
    if text.is_empty() {
        return BuiltinType::Text;
    }
    if text.contains('\n') {
        return BuiltinType::LongText;
    }
    let lower = text.to_lowercase();
    if matches!(lower.as_str(), "true" | "false" | "yes" | "no") {
        return BuiltinType::Checkbox;
    }
    if FormatId::Email.validate(text).is_ok() {
        return BuiltinType::Email;
    }
    if (lower.starts_with("http://") || lower.starts_with("https://"))
        && FormatId::Url.validate(text).is_ok()
    {
        return BuiltinType::Url;
    }
    // A phone number is usually written with separators; a number field would read the
    // same text as one large number.
    if text.starts_with('+') {
        let digits: String = text
            .chars()
            .filter(|c| !matches!(c, ' ' | '-' | '(' | ')' | '.' | '\u{00A0}'))
            .collect();
        if FormatId::PhoneE164.validate(&digits).is_ok() {
            return BuiltinType::Phone;
        }
    }
    let reads_as = |data_type: DataType| {
        coerce::parse_input(data_type, text, &coerce::ParseContext::default()).is_ok()
    };
    if text.ends_with('%') && reads_as(DataType::Percent) {
        return BuiltinType::Percent;
    }
    for (data_type, builtin) in [
        (DataType::Number, BuiltinType::Number),
        (DataType::Date, BuiltinType::Date),
        (DataType::DateTime, BuiltinType::DateTime),
        (DataType::Money, BuiltinType::Currency),
    ] {
        if reads_as(data_type) {
            return builtin;
        }
    }
    if text.chars().count() > 120 {
        BuiltinType::LongText
    } else {
        BuiltinType::Text
    }
}

/// Maximum number of characters in the key of a custom field type.
pub const MAX_TYPE_KEY_CHARS: usize = 64;

fn invalid(reason: impl Into<String>) -> CoreError {
    CoreError::InvalidDefinition {
        reason: reason.into(),
    }
}

/// A tenant's own named field type: a storage type with an input control, validation,
/// format and default classification, offered in the picker next to the built-in ones
/// ("Employee number", "Batch code").
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FieldTypeDef {
    /// Stable ID.
    pub id: FieldTypeId,
    /// Stable key: lower-case letters, digits and `_`, starting with a letter. Must not be
    /// the key of a built-in type.
    pub key: String,
    /// Display name.
    pub name: String,
    /// The picker group it is offered in.
    pub group: PickerGroup,
    /// Storage type.
    pub base: DataType,
    /// Input control.
    pub widget: Widget,
    /// Validation rules every field of this type starts with.
    #[serde(default)]
    pub validation: ValidationSpec,
    /// Display format every field of this type starts with.
    #[serde(default)]
    pub format: DisplayFormat,
    /// Privacy classification every field of this type starts with.
    pub classification: Classification,
}

impl FieldTypeDef {
    /// Writes the definition of a new field of this type.
    #[must_use]
    pub fn create(&self, id: FieldId, name: impl Into<String>) -> FieldDef {
        let mut field = FieldDef::new(id, name, self.base);
        field.widget = self.widget;
        field.validation = self.validation.clone();
        field.format = self.format.clone();
        field.classification = self.classification;
        field
    }

    /// Verifies the type: a well-formed key that no built-in type uses, a name, and facets
    /// that fit together (as for a field of this type). Types that need options or a target
    /// table are not custom types; those are library items.
    ///
    /// # Errors
    /// [`CoreError::InvalidDefinition`] describing the first problem found.
    pub fn check(&self) -> Result<(), CoreError> {
        let mut chars = self.key.chars();
        let well_formed = chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            && self.key.len() <= MAX_TYPE_KEY_CHARS;
        if !well_formed {
            return Err(invalid(format!(
                "`{}` is not a field type key: use lower-case letters, digits and `_`, starting with a letter",
                clip(&self.key)
            )));
        }
        if BuiltinType::from_str(&self.key).is_ok() {
            return Err(invalid(format!(
                "`{}` is the key of a built-in field type",
                self.key
            )));
        }
        if matches!(self.group, PickerGroup::System | PickerGroup::Ai) {
            return Err(invalid(format!(
                "a custom field type cannot be offered in the {} group",
                self.group
            )));
        }
        self.create(FieldId::from_u128(0), self.name.as_str())
            .check()
    }
}

/// A part of a field definition, as named when a field is compared with its library item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Facet {
    /// The storage type.
    DataType,
    /// The input control.
    Widget,
    /// The option source.
    Options,
    /// The display format.
    Format,
    /// The validation rules.
    Validation,
    /// The role.
    Role,
    /// The privacy classification.
    Classification,
    /// The help text.
    HelpText,
    /// The computed facet.
    Computed,
    /// The AI skill.
    AiSkill,
}

impl fmt::Display for Facet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Facet::DataType => "data type",
            Facet::Widget => "widget",
            Facet::Options => "options",
            Facet::Format => "format",
            Facet::Validation => "validation",
            Facet::Role => "role",
            Facet::Classification => "classification",
            Facet::HelpText => "help text",
            Facet::Computed => "formula",
            Facet::AiSkill => "AI skill",
        })
    }
}

/// A complete, reusable field definition kept in the field library ("Cost centre",
/// "Customer email"). Fields created from it remember the item, so a governance view can
/// show where it is used and which fields have drifted from it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FieldLibraryItem {
    /// Stable ID.
    pub id: FieldLibraryItemId,
    /// Name in the library.
    pub name: String,
    /// What the item is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The definition new fields start from. Its own id, group and status are not used.
    pub definition: FieldDef,
    /// Version of the item: starts at 1, raised by every change to the definition.
    pub version: u32,
}

impl FieldLibraryItem {
    /// Verifies the item: a name, a version, and a complete definition. Definitions that
    /// depend on one table — computed fields, cascades from another field, cross-field
    /// rules, conditional requirements — cannot be library items.
    ///
    /// # Errors
    /// [`CoreError::InvalidDefinition`] describing the first problem found.
    pub fn check(&self) -> Result<(), CoreError> {
        if self.name.trim().is_empty() {
            return Err(invalid("a library item needs a name"));
        }
        if self.version == 0 {
            return Err(invalid("library item version starts at 1"));
        }
        let definition = &self.definition;
        definition.check()?;
        let bound_to_a_table = definition.computed.is_some()
            || definition.system.is_some()
            || definition.options.cascade_from().is_some()
            || !definition.validation.cross_field.is_empty()
            || definition.validation.required_when.is_some();
        if bound_to_a_table {
            return Err(invalid(
                "a library item cannot refer to other fields of a table or be a system field",
            ));
        }
        Ok(())
    }

    /// Writes the definition of a new field created from this item: the item's definition
    /// with the new id, linked back to the item, enabled, ungrouped, at version 1.
    #[must_use]
    pub fn instantiate(&self, id: FieldId) -> FieldDef {
        let mut field = self.definition.clone();
        field.id = id;
        field.library_item = Some(self.id);
        field.group = None;
        field.status = crate::field::FieldStatus::Enabled;
        field.version = 1;
        field
    }

    /// The facets in which a field differs from this item's definition. Empty for a field
    /// that still follows the library; names may differ freely.
    #[must_use]
    pub fn differences(&self, field: &FieldDef) -> Vec<Facet> {
        let own = &self.definition;
        let checks = [
            (Facet::DataType, own.data_type == field.data_type),
            (Facet::Widget, own.widget == field.widget),
            (Facet::Options, own.options == field.options),
            (Facet::Format, own.format == field.format),
            (Facet::Validation, own.validation == field.validation),
            (Facet::Role, own.role == field.role),
            (
                Facet::Classification,
                own.classification == field.classification,
            ),
            (Facet::HelpText, own.help_text == field.help_text),
            (Facet::Computed, own.computed == field.computed),
            (Facet::AiSkill, own.ai_skill == field.ai_skill),
        ];
        checks
            .into_iter()
            .filter_map(|(facet, same)| (!same).then_some(facet))
            .collect()
    }
}

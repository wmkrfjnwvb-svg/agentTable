//! The field picker: built-in field types in eight groups, their templates, recognising the
//! type of a definition, search, suggestions, custom field types, the field library, and the
//! computed and system facets of field definitions.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use mt_core::condition::Condition;
use mt_core::field::{
    Classification, Computed, ComputedKind, DataType, FieldDef, FieldRole, FieldStatus,
    InlineOption, OptionsSource, SystemField, Widget,
};
use mt_core::fieldtype::{
    search, suggest, BuiltinType, Facet, FieldLibraryItem, FieldTypeDef, PickerGroup, Requirement,
};
use mt_core::format::FormatId;
use mt_core::id::{
    FieldGroupId, FieldId, FieldLibraryItemId, FieldTypeId, RefItemId, RefSetId, SkillId, TableId,
};
use mt_core::numbering::NumberingPattern;
use mt_core::pattern::TextPattern;
use mt_core::validate::{check_writable, IssueCode};
use mt_core::CoreError;
use std::collections::BTreeSet;
use std::str::FromStr;

fn fid(n: u128) -> FieldId {
    FieldId::from_u128(n)
}

fn reason(result: Result<(), CoreError>) -> String {
    match result {
        Err(CoreError::InvalidDefinition { reason }) => reason,
        Ok(()) => panic!("expected the definition to be refused"),
        Err(other) => panic!("unexpected error {other:?}"),
    }
}

fn option(id: u128, label: &str) -> InlineOption {
    InlineOption {
        id: RefItemId::from_u128(id),
        label: label.to_owned(),
        color: None,
        enabled: true,
    }
}

fn formula() -> serde_json::Value {
    serde_json::json!({"t": "number", "v": "1"})
}

/// Supplies what a template still needs, the way a designer would.
fn complete(builtin: BuiltinType, field: &mut FieldDef) {
    for requirement in builtin.requirements() {
        match requirement {
            Requirement::Options => {
                field.options = OptionsSource::Inline {
                    options: vec![option(1, "High"), option(2, "Low")],
                };
            }
            Requirement::RefSet => field.options = OptionsSource::of_set(RefSetId::from_u128(7)),
            Requirement::LinkTarget => {
                field.options = OptionsSource::Table {
                    table: TableId::from_u128(3),
                };
            }
            Requirement::Formula => {
                field.computed = Some(Computed {
                    kind: builtin.computed_kind().unwrap(),
                    formula: formula(),
                });
            }
            Requirement::Numbering => {
                field.format.numbering = Some(NumberingPattern::new("PO-{YYYY}-{seq:5}").unwrap());
            }
            Requirement::Skill => field.ai_skill = Some(SkillId::from_u128(9)),
            Requirement::Format => field.validation.format = Some(FormatId::DeVatId),
        }
    }
}

#[test]
fn std_eight_groups_in_picker_order() {
    let groups: Vec<&str> = PickerGroup::ALL
        .iter()
        .map(|group| group.as_str())
        .collect();
    assert_eq!(
        groups,
        [
            "basic",
            "people",
            "business",
            "reference",
            "relational",
            "process",
            "ai",
            "system"
        ]
    );
    let sizes: Vec<usize> = PickerGroup::ALL
        .iter()
        .map(|group| group.types().len())
        .collect();
    assert_eq!(sizes, [13, 3, 10, 4, 5, 3, 1, 8]);
    assert_eq!(BuiltinType::ALL.len(), 47);
    // Every type is offered in exactly one group, and the picker lists them group by group.
    let listed: Vec<BuiltinType> = PickerGroup::ALL
        .iter()
        .flat_map(|group| group.types())
        .collect();
    assert_eq!(listed, BuiltinType::ALL);
    assert_eq!(
        PickerGroup::Relational.types(),
        [
            BuiltinType::Link,
            BuiltinType::Lookup,
            BuiltinType::Rollup,
            BuiltinType::Formula,
            BuiltinType::LineItems
        ]
    );
    assert_eq!(PickerGroup::Ai.types(), [BuiltinType::AiField]);
    // Keys, names and synonyms.
    let keys: BTreeSet<&str> = BuiltinType::ALL.iter().map(|t| t.as_str()).collect();
    let labels: BTreeSet<&str> = BuiltinType::ALL.iter().map(|t| t.label()).collect();
    assert_eq!(keys.len(), 47);
    assert_eq!(labels.len(), 47);
    for builtin in BuiltinType::ALL.iter().copied() {
        assert_eq!(BuiltinType::from_str(builtin.as_str()).unwrap(), builtin);
        assert!(!builtin.label().is_empty());
        assert!(!builtin.synonyms().is_empty(), "{builtin}");
        for synonym in builtin.synonyms() {
            assert_eq!(*synonym, synonym.to_lowercase().trim(), "{builtin}");
        }
        let json = serde_json::to_string(&builtin).unwrap();
        assert_eq!(serde_json::from_str::<BuiltinType>(&json).unwrap(), builtin);
    }
    assert!(BuiltinType::from_str("button").is_err());
}

#[test]
fn std_every_template_becomes_a_valid_field_of_its_type() {
    for builtin in BuiltinType::ALL.iter().copied() {
        let mut field = builtin.template(fid(1), builtin.label());
        let (data_type, widget) = builtin.storage();
        assert_eq!(
            (field.data_type, field.widget),
            (data_type, widget),
            "{builtin}"
        );
        assert!(widget.supports(data_type), "{builtin}");
        assert_eq!(field.name, builtin.label());
        assert_eq!(field.status, FieldStatus::Enabled);
        // Without what the designer must supply, the definition is either refused with a
        // reason or not yet recognisable as the type.
        if !builtin.requirements().is_empty() {
            let refused = field.check().is_err();
            let unrecognised = BuiltinType::of(&field) != builtin;
            assert!(
                refused || unrecognised,
                "{builtin} is complete without {:?}",
                builtin.requirements()
            );
        }
        complete(builtin, &mut field);
        field
            .check()
            .unwrap_or_else(|error| panic!("{builtin}: {error}"));
        // The grid shows the type the designer picked.
        assert_eq!(BuiltinType::of(&field), builtin, "{field:?}");
        // System and computed fields cannot be typed into; the others can.
        let generated = matches!(builtin.group(), PickerGroup::System)
            || builtin.computed_kind().is_some()
            || builtin == BuiltinType::AutoNumber;
        assert_eq!(field.is_generated(), generated, "{builtin}");
        assert_eq!(check_writable(&field).is_some(), generated, "{builtin}");
        // The definition survives storage.
        let json = serde_json::to_string(&field).unwrap();
        assert_eq!(
            serde_json::from_str::<FieldDef>(&json).unwrap(),
            field,
            "{builtin}"
        );
    }
}

#[test]
fn std_templates_preset_what_the_type_means() {
    let template = |builtin: BuiltinType| builtin.template(fid(1), "x");
    assert_eq!(
        template(BuiltinType::Email).validation.format,
        Some(FormatId::Email)
    );
    assert_eq!(
        template(BuiltinType::Url).validation.format,
        Some(FormatId::Url)
    );
    assert_eq!(
        template(BuiltinType::Phone).validation.format,
        Some(FormatId::PhoneE164)
    );
    // Contact details, addresses, places and signatures are personal data by default.
    for builtin in [
        BuiltinType::Email,
        BuiltinType::Phone,
        BuiltinType::Address,
        BuiltinType::Location,
        BuiltinType::Signature,
    ] {
        assert_eq!(
            template(builtin).classification,
            Classification::Personal,
            "{builtin}"
        );
    }
    assert_eq!(
        template(BuiltinType::Url).classification,
        Classification::Internal
    );
    let rating = template(BuiltinType::Rating);
    assert_eq!(rating.validation.min, Some(0.into()));
    assert_eq!(rating.validation.max, Some(5.into()));
    assert_eq!(rating.validation.max_decimals, Some(0));
    let progress = template(BuiltinType::Progress);
    assert_eq!(
        (progress.validation.min, progress.validation.max),
        (Some(0.into()), Some(1.into()))
    );
    assert_eq!(
        template(BuiltinType::Signature).validation.max_items,
        Some(1)
    );
    // Roles let generic features find the field.
    assert_eq!(
        template(BuiltinType::LifecycleState).role,
        Some(FieldRole::State)
    );
    assert_eq!(
        template(BuiltinType::Assignment).role,
        Some(FieldRole::PersonAssignee)
    );
    assert_eq!(
        template(BuiltinType::DueDate).role,
        Some(FieldRole::TimeDue)
    );
    assert_eq!(
        template(BuiltinType::Party).role,
        Some(FieldRole::PartyCounterparty)
    );
    assert_eq!(
        template(BuiltinType::CatalogueItem).role,
        Some(FieldRole::Item)
    );
    assert_eq!(
        template(BuiltinType::OrgUnit).role,
        Some(FieldRole::Dimension)
    );
    assert_eq!(template(BuiltinType::Text).role, None);
    // System types.
    assert_eq!(
        template(BuiltinType::CreatedBy).system,
        Some(SystemField::CreatedBy)
    );
    assert_eq!(
        template(BuiltinType::RecordVersion).system,
        Some(SystemField::Version)
    );
    assert_eq!(template(BuiltinType::Text).system, None);
    // Computed kinds.
    assert_eq!(
        BuiltinType::Lookup.computed_kind(),
        Some(ComputedKind::Lookup)
    );
    assert_eq!(
        BuiltinType::Rollup.computed_kind(),
        Some(ComputedKind::Rollup)
    );
    assert_eq!(
        BuiltinType::Formula.computed_kind(),
        Some(ComputedKind::Formula)
    );
    assert_eq!(BuiltinType::Number.computed_kind(), None);
}

#[test]
fn edge_the_most_specific_type_is_recognised() {
    let of = |edit: &dyn Fn(&mut FieldDef), data_type: DataType| {
        let mut field = FieldDef::new(fid(1), "x", data_type);
        edit(&mut field);
        BuiltinType::of(&field)
    };
    // Text with a format that has no type of its own stays text.
    assert_eq!(
        of(
            &|f| f.validation.format = Some(FormatId::PostalDe),
            DataType::Text
        ),
        BuiltinType::Text
    );
    assert_eq!(
        of(
            &|f| f.validation.format = Some(FormatId::Iban),
            DataType::Text
        ),
        BuiltinType::Text
    );
    for format in [
        FormatId::UsEin,
        FormatId::DeVatId,
        FormatId::JpCorporateNumber,
        FormatId::JpInvoiceNumber,
        FormatId::KrBusinessNumber,
        FormatId::InGstin,
        FormatId::InPan,
    ] {
        assert_eq!(
            of(&|f| f.validation.format = Some(format), DataType::Text),
            BuiltinType::TaxId
        );
    }
    // Reference fields: role and option source decide.
    let set = OptionsSource::of_set(RefSetId::from_u128(1));
    assert_eq!(
        of(&|f| f.options = set.clone(), DataType::Reference),
        BuiltinType::CodeList
    );
    assert_eq!(
        of(
            &|f| f.options = OptionsSource::Parameter {
                key: "regions".into()
            },
            DataType::Reference
        ),
        BuiltinType::CodeList
    );
    assert_eq!(
        of(
            &|f| {
                f.options = OptionsSource::RefSet {
                    set: RefSetId::from_u128(1),
                    subtree: None,
                    level: None,
                    leaves_only: false,
                    cascade_from: Some(fid(2)),
                };
            },
            DataType::Reference
        ),
        BuiltinType::Cascade
    );
    assert_eq!(
        of(
            &|f| {
                f.options = set.clone();
                f.role = Some(FieldRole::State);
            },
            DataType::Reference
        ),
        BuiltinType::LifecycleState
    );
    // A dimension is an organisation unit only with the tree picker.
    assert_eq!(
        of(
            &|f| {
                f.options = set.clone();
                f.role = Some(FieldRole::Dimension);
            },
            DataType::Reference
        ),
        BuiltinType::CodeList
    );
    assert_eq!(
        of(&|f| f.options = set.clone(), DataType::MultiReference),
        BuiltinType::CodeListMulti
    );
    // Links.
    assert_eq!(
        of(
            &|f| f.role = Some(FieldRole::PartyRequester),
            DataType::Link
        ),
        BuiltinType::Party
    );
    assert_eq!(
        of(&|f| f.role = Some(FieldRole::Dimension), DataType::Link),
        BuiltinType::Link
    );
    // Other widgets of the same storage type.
    assert_eq!(
        of(&|f| f.widget = Widget::Slider, DataType::Number),
        BuiltinType::Number
    );
    assert_eq!(
        of(&|f| f.widget = Widget::Slider, DataType::Percent),
        BuiltinType::Percent
    );
    assert_eq!(
        of(&|f| f.widget = Widget::Toggle, DataType::Bool),
        BuiltinType::Checkbox
    );
    assert_eq!(
        of(
            &|f| f.role = Some(FieldRole::TimeOpened),
            DataType::DateTime
        ),
        BuiltinType::DateTime
    );
    assert_eq!(
        of(
            &|f| f.role = Some(FieldRole::PersonOwner),
            DataType::Principal
        ),
        BuiltinType::Person
    );
    assert_eq!(
        of(&|f| f.widget = Widget::TextArea, DataType::LongText),
        BuiltinType::LongText
    );
    // Free-form structured data has no picker type of its own.
    assert_eq!(of(&|_| {}, DataType::Json), BuiltinType::LongText);
    // Generated values win over everything else.
    assert_eq!(
        of(
            &|f| {
                f.validation.format = Some(FormatId::Email);
                f.ai_skill = Some(SkillId::from_u128(1));
            },
            DataType::Text
        ),
        BuiltinType::AiField
    );
    assert_eq!(
        of(
            &|f| f.computed = Some(Computed {
                kind: ComputedKind::Rollup,
                formula: formula()
            }),
            DataType::Money
        ),
        BuiltinType::Rollup
    );
    for system in SystemField::ALL.iter().copied() {
        let found = of(&|f| f.system = Some(system), system.data_type());
        assert_eq!(found.group(), PickerGroup::System, "{system}");
    }
}

#[test]
fn std_search_finds_types_by_name_and_synonym() {
    assert_eq!(search(""), BuiltinType::ALL);
    assert_eq!(search("   "), BuiltinType::ALL);
    // Synonyms from other products.
    assert_eq!(search("dropdown"), [BuiltinType::SingleSelect]);
    assert_eq!(search("status"), [BuiltinType::LifecycleState]);
    assert_eq!(search("MONEY"), [BuiltinType::Currency]);
    assert_eq!(search(" vlookup "), [BuiltinType::Lookup]);
    assert_eq!(search("qr"), [BuiltinType::Barcode]);
    assert_eq!(search("tags"), [BuiltinType::MultiSelect]);
    assert_eq!(search("department"), [BuiltinType::OrgUnit]);
    assert_eq!(search("zzz"), []);
    // Names that start with the query come first, then other name matches, then synonyms.
    let dates = search("date");
    assert_eq!(dates[..2], [BuiltinType::Date, BuiltinType::DateTime]);
    assert!(dates.contains(&BuiltinType::DueDate));
    assert!(
        dates.contains(&BuiltinType::CreatedAt),
        "by the synonym `creation date`"
    );
    let position = |wanted: BuiltinType| dates.iter().position(|t| *t == wanted).unwrap();
    assert!(position(BuiltinType::DueDate) < position(BuiltinType::CreatedAt));
    let links = search("link");
    assert_eq!(links[0], BuiltinType::Link);
    assert!(links.contains(&BuiltinType::Url), "by the synonym `link`");
    assert!(links.contains(&BuiltinType::TraceLink), "by the key");
    // Searching for a type's own name finds it first.
    for builtin in BuiltinType::ALL.iter().copied() {
        assert_eq!(search(builtin.label()).first(), Some(&builtin), "{builtin}");
        assert!(search(builtin.as_str()).contains(&builtin), "{builtin}");
        for synonym in builtin.synonyms() {
            assert!(
                search(synonym).contains(&builtin),
                "{builtin} by `{synonym}`"
            );
        }
    }
    // No type is listed twice.
    for query in ["a", "e", "id", "date", "list"] {
        let found = search(query);
        let unique: BTreeSet<BuiltinType> = found.iter().copied().collect();
        assert_eq!(unique.len(), found.len(), "{query}");
    }
}

#[test]
fn std_a_type_is_suggested_from_a_sample_value() {
    let cases = [
        ("ada@example.com", BuiltinType::Email),
        ("https://example.com/a?b=1", BuiltinType::Url),
        ("+81 90 1234 5678", BuiltinType::Phone),
        ("+4915112345678", BuiltinType::Phone),
        ("42", BuiltinType::Number),
        ("-1,234.50", BuiltinType::Number),
        ("12.5%", BuiltinType::Percent),
        ("2026-05-20", BuiltinType::Date),
        ("2026-05-20T10:30:00Z", BuiltinType::DateTime),
        ("19.99 USD", BuiltinType::Currency),
        ("true", BuiltinType::Checkbox),
        ("No", BuiltinType::Checkbox),
        ("Spring order", BuiltinType::Text),
        ("example.com", BuiltinType::Text),
        ("not an email@", BuiltinType::Text),
        ("first line\nsecond line", BuiltinType::LongText),
        ("", BuiltinType::Text),
        ("   ", BuiltinType::Text),
    ];
    for (sample, expected) in cases {
        assert_eq!(suggest(sample), expected, "`{sample}`");
    }
    assert_eq!(suggest(&"word ".repeat(40)), BuiltinType::LongText);
    // Whatever the sample, a suggestion is a type a field can be created from.
    for sample in [
        "\u{0}", "١٢٣", "🙂", "1e9", "+", "%", "http://", "@", "0x1F",
    ] {
        let builtin = suggest(sample);
        builtin
            .template(fid(1), "x")
            .check()
            .unwrap_or_else(|error| panic!("`{sample}`: {error}"));
    }
}

fn employee_number() -> FieldTypeDef {
    let mut field_type = FieldTypeDef {
        id: FieldTypeId::from_u128(1),
        key: "employee_number".into(),
        name: "Employee number".into(),
        group: PickerGroup::Business,
        base: DataType::Text,
        widget: Widget::TextBox,
        validation: Default::default(),
        format: Default::default(),
        classification: Classification::Personal,
    };
    field_type.validation.pattern = Some(TextPattern::new(r"E-\d{4}").unwrap());
    field_type.format.prefix = Some("#".into());
    field_type
}

#[test]
fn std_custom_field_types() {
    let field_type = employee_number();
    field_type.check().unwrap();
    let field = field_type.create(fid(5), "Staff no.");
    field.check().unwrap();
    assert_eq!(field.id, fid(5));
    assert_eq!(field.name, "Staff no.");
    assert_eq!(field.data_type, DataType::Text);
    assert_eq!(field.validation, field_type.validation);
    assert_eq!(field.format, field_type.format);
    assert_eq!(field.classification, Classification::Personal);
    let json = serde_json::to_string(&field_type).unwrap();
    assert_eq!(
        serde_json::from_str::<FieldTypeDef>(&json).unwrap(),
        field_type
    );
    // A numeric type with a range.
    let mut score = employee_number();
    score.key = "score_0_100".into();
    score.base = DataType::Number;
    score.widget = Widget::Slider;
    score.validation = Default::default();
    score.validation.min = Some(0.into());
    score.validation.max = Some(100.into());
    score.format = Default::default();
    score.check().unwrap();
}

#[test]
fn invalid_custom_field_types() {
    let with = |edit: &dyn Fn(&mut FieldTypeDef)| {
        let mut field_type = employee_number();
        edit(&mut field_type);
        reason(field_type.check())
    };
    for key in [
        "",
        "Employee",
        "9lives",
        "employee number",
        "employee-number",
        "é",
        &"a".repeat(65),
    ] {
        assert!(
            with(&|t| t.key = key.to_owned()).contains("is not a field type key"),
            "{key}"
        );
    }
    // The longest key that is accepted.
    let mut longest = employee_number();
    longest.key = "a".repeat(64);
    longest.check().unwrap();
    // The keys of built-in types are taken.
    assert_eq!(
        with(&|t| t.key = "email".into()),
        "`email` is the key of a built-in field type"
    );
    assert!(with(&|t| t.key = "auto_number".into()).contains("built-in"));
    // Groups reserved for the platform.
    assert_eq!(
        with(&|t| t.group = PickerGroup::System),
        "a custom field type cannot be offered in the system group"
    );
    assert!(with(&|t| t.group = PickerGroup::Ai).contains("the ai group"));
    // The facets must fit together, as for a field.
    assert!(with(&|t| t.name = "  ".into()).contains("name must not be empty"));
    assert!(with(&|t| t.widget = Widget::Checkbox).contains("cannot edit a text field"));
    assert!(with(&|t| t.base = DataType::Number).contains("widget"));
    assert!(with(&|t| t.format.decimals = Some(2)).contains("decimals do not apply"));
    // Types that need options or a target table are library items, not custom types.
    assert!(with(&|t| {
        t.base = DataType::Reference;
        t.widget = Widget::Dropdown;
        t.validation = Default::default();
        t.format = Default::default();
    })
    .contains("needs inline options or a reference set"));
}

fn cost_centre() -> FieldLibraryItem {
    let mut definition = FieldDef::new(fid(99), "Cost centre", DataType::Reference);
    definition.options = OptionsSource::of_set(RefSetId::from_u128(4));
    definition.widget = Widget::SearchPicker;
    definition.role = Some(FieldRole::Dimension);
    definition.help_text = Some("Who pays".into());
    definition.group = Some(FieldGroupId::from_u128(8));
    definition.status = FieldStatus::Deprecated;
    definition.version = 6;
    FieldLibraryItem {
        id: FieldLibraryItemId::from_u128(2),
        name: "Cost centre".into(),
        description: Some("The finance dimension every spend record carries".into()),
        definition,
        version: 3,
    }
}

#[test]
fn std_the_field_library() {
    let item = cost_centre();
    item.check().unwrap();
    let field = item.instantiate(fid(10));
    field.check().unwrap();
    // A new field: its own id, linked to the item, enabled, ungrouped, version 1.
    assert_eq!(field.id, fid(10));
    assert_eq!(field.library_item, Some(item.id));
    assert_eq!(field.status, FieldStatus::Enabled);
    assert_eq!(field.group, None);
    assert_eq!(field.version, 1);
    assert_eq!(field.options, item.definition.options);
    assert_eq!(field.role, Some(FieldRole::Dimension));
    assert_eq!(item.differences(&field), []);
    // Renaming, regrouping or disabling a field is not drift.
    let mut renamed = field.clone();
    renamed.name = "Kostenstelle".into();
    renamed.names.insert("de".into(), "Kostenstelle".into());
    renamed.group = Some(FieldGroupId::from_u128(1));
    renamed.status = FieldStatus::Disabled;
    assert_eq!(item.differences(&renamed), []);
    // Changing what the field accepts or shows is.
    let mut drifted = field.clone();
    drifted.widget = Widget::Dropdown;
    drifted.validation.required = true;
    drifted.classification = Classification::Confidential;
    drifted.help_text = None;
    assert_eq!(
        item.differences(&drifted),
        [
            Facet::Widget,
            Facet::Validation,
            Facet::Classification,
            Facet::HelpText
        ]
    );
    let mut other = field.clone();
    other.data_type = DataType::MultiReference;
    other.options = OptionsSource::of_set(RefSetId::from_u128(5));
    other.format.prefix = Some("CC ".into());
    other.role = None;
    other.ai_skill = Some(SkillId::from_u128(1));
    other.computed = Some(Computed {
        kind: ComputedKind::Formula,
        formula: formula(),
    });
    let found = item.differences(&other);
    assert_eq!(
        found,
        [
            Facet::DataType,
            Facet::Options,
            Facet::Format,
            Facet::Role,
            Facet::Computed,
            Facet::AiSkill
        ]
    );
    let words: Vec<String> = found.iter().map(ToString::to_string).collect();
    assert_eq!(
        words,
        [
            "data type",
            "options",
            "format",
            "role",
            "formula",
            "AI skill"
        ]
    );
    assert_eq!(Facet::Widget.to_string(), "widget");
    assert_eq!(Facet::Validation.to_string(), "validation");
    assert_eq!(Facet::Classification.to_string(), "classification");
    assert_eq!(Facet::HelpText.to_string(), "help text");
    // The item survives storage.
    let json = serde_json::to_string(&item).unwrap();
    assert_eq!(
        serde_json::from_str::<FieldLibraryItem>(&json).unwrap(),
        item
    );
}

#[test]
fn invalid_library_items() {
    let with = |edit: &dyn Fn(&mut FieldLibraryItem)| {
        let mut item = cost_centre();
        edit(&mut item);
        reason(item.check())
    };
    assert_eq!(
        with(&|i| i.name = " ".into()),
        "a library item needs a name"
    );
    assert_eq!(with(&|i| i.version = 0), "library item version starts at 1");
    // The definition itself must be complete.
    assert!(with(&|i| i.definition.options = OptionsSource::None).contains("needs inline options"));
    // What depends on the other fields of one table cannot be shared between tables.
    let bound = "a library item cannot refer to other fields of a table or be a system field";
    assert_eq!(
        with(&|i| {
            i.definition.options = OptionsSource::RefSet {
                set: RefSetId::from_u128(4),
                subtree: None,
                level: None,
                leaves_only: false,
                cascade_from: Some(fid(3)),
            };
        }),
        bound
    );
    assert_eq!(
        with(&|i| i.definition.validation.required_when = Some(Condition::is_empty(fid(3)))),
        bound
    );
    assert_eq!(
        with(&|i| {
            i.definition = FieldDef::new(fid(1), "Total", DataType::Number);
            i.definition.computed = Some(Computed {
                kind: ComputedKind::Formula,
                formula: formula(),
            });
        }),
        bound
    );
    assert_eq!(
        with(&|i| i.definition = BuiltinType::CreatedAt.template(fid(1), "Created")),
        bound
    );
    assert_eq!(
        with(&|i| {
            i.definition = FieldDef::new(fid(1), "End", DataType::Date);
            i.definition
                .validation
                .cross_field
                .push(mt_core::field::CrossFieldRule {
                    comparison: mt_core::field::Comparison::Ge,
                    other: fid(2),
                });
        }),
        bound
    );
}

#[test]
fn std_computed_and_system_fields() {
    // A computed field: calculated, never typed.
    let mut total = FieldDef::new(fid(1), "Total", DataType::Money);
    assert!(!total.is_computed() && !total.is_generated());
    assert_eq!(total.value_source(), None);
    total.computed = Some(Computed {
        kind: ComputedKind::Formula,
        formula: formula(),
    });
    total.check().unwrap();
    assert!(total.is_computed() && total.is_generated());
    assert_eq!(total.value_source(), Some("calculated"));
    let issue = check_writable(&total).unwrap();
    assert_eq!(issue.code, IssueCode::Generated);
    assert_eq!(issue.message, "`Total` is calculated");
    // The stored form.
    let json = serde_json::to_value(&total).unwrap();
    assert_eq!(
        json["computed"],
        serde_json::json!({"kind": "formula", "formula": {"t": "number", "v": "1"}})
    );
    assert_eq!(serde_json::from_value::<FieldDef>(json).unwrap(), total);
    for kind in ComputedKind::ALL.iter().copied() {
        assert_eq!(ComputedKind::from_str(kind.as_str()).unwrap(), kind);
    }
    // Definitions stored before these facets existed still load.
    let mut old = serde_json::to_value(FieldDef::new(fid(2), "Name", DataType::Text)).unwrap();
    let object = old.as_object_mut().unwrap();
    assert!(!object.contains_key("computed") && !object.contains_key("system"));
    let loaded: FieldDef = serde_json::from_value(old).unwrap();
    assert_eq!((loaded.computed, loaded.system), (None, None));

    // System fields.
    for system in SystemField::ALL.iter().copied() {
        let mut field = FieldDef::new(fid(3), "System", system.data_type());
        field.system = Some(system);
        field.check().unwrap();
        assert!(field.is_generated() && !field.is_computed());
        assert_eq!(field.value_source(), Some("maintained by the system"));
        assert_eq!(
            check_writable(&field).unwrap().message,
            "`System` is maintained by the system"
        );
        let json = serde_json::to_string(&field).unwrap();
        assert_eq!(serde_json::from_str::<FieldDef>(&json).unwrap(), field);
    }
    assert_eq!(SystemField::CreatedAt.data_type(), DataType::DateTime);
    assert_eq!(SystemField::ModifiedBy.data_type(), DataType::Principal);
    assert_eq!(SystemField::Version.data_type(), DataType::Number);
    assert_eq!(SystemField::Trace.data_type(), DataType::Text);

    // The other sources of values, in words.
    let mut numbered = FieldDef::new(fid(4), "No.", DataType::Text);
    numbered.format.numbering = Some(NumberingPattern::new("T-{seq}").unwrap());
    assert_eq!(numbered.value_source(), Some("numbered automatically"));
    assert_eq!(
        check_writable(&numbered).unwrap().message,
        "`No.` is numbered automatically"
    );
    let mut ai = FieldDef::new(fid(5), "Summary", DataType::LongText);
    ai.ai_skill = Some(SkillId::from_u128(1));
    assert_eq!(ai.value_source(), Some("filled by an AI skill"));
    // An AI field can still be corrected by a person.
    assert!(!ai.is_generated());
    assert_eq!(check_writable(&ai), None);
}

#[test]
fn invalid_computed_and_system_fields() {
    let computed = || {
        Some(Computed {
            kind: ComputedKind::Formula,
            formula: formula(),
        })
    };
    // What a computed field can never be.
    for data_type in [
        DataType::Link,
        DataType::Principal,
        DataType::Attachment,
        DataType::Json,
    ] {
        let mut field = FieldDef::new(fid(1), "x", data_type);
        if data_type == DataType::Link {
            field.options = OptionsSource::Table {
                table: TableId::from_u128(1),
            };
        }
        field.computed = computed();
        assert_eq!(
            reason(field.check()),
            format!("a computed field cannot be a {data_type} field")
        );
    }
    // A formula is needed.
    for formula in [
        serde_json::Value::Null,
        serde_json::json!("1 + 1"),
        serde_json::json!([]),
    ] {
        let mut field = FieldDef::new(fid(1), "x", DataType::Number);
        field.computed = Some(Computed {
            kind: ComputedKind::Formula,
            formula,
        });
        assert_eq!(reason(field.check()), "a computed field needs a formula");
    }
    // One source of values only.
    let one_way =
        "a field gets its values in one way: auto-numbering, a formula, the system or an AI skill";
    let mut field = FieldDef::new(fid(1), "x", DataType::Text);
    field.computed = computed();
    field.format.numbering = Some(NumberingPattern::new("T-{seq}").unwrap());
    assert_eq!(reason(field.check()), one_way);
    let mut field = FieldDef::new(fid(1), "x", DataType::Text);
    field.computed = computed();
    field.ai_skill = Some(SkillId::from_u128(1));
    assert_eq!(reason(field.check()), one_way);
    let mut field = FieldDef::new(fid(1), "x", DataType::Text);
    field.computed = computed();
    field.system = Some(SystemField::RecordId);
    assert_eq!(reason(field.check()), one_way);
    let mut field = FieldDef::new(fid(1), "x", DataType::Text);
    field.system = Some(SystemField::RecordId);
    field.ai_skill = Some(SkillId::from_u128(1));
    assert_eq!(reason(field.check()), one_way);
    // A system field has the type of what the system records.
    let mut field = FieldDef::new(fid(1), "x", DataType::Text);
    field.system = Some(SystemField::CreatedAt);
    assert_eq!(
        reason(field.check()),
        "the system field created_at is a datetime field"
    );
    // Nobody can be asked to fill in a generated value.
    let mut field = FieldDef::new(fid(1), "x", DataType::Number);
    field.computed = computed();
    field.validation.required = true;
    assert_eq!(
        reason(field.check()),
        "a field whose values are generated cannot be required"
    );
    let mut field = FieldDef::new(fid(1), "x", DataType::DateTime);
    field.system = Some(SystemField::ModifiedAt);
    field.validation.required = true;
    assert_eq!(
        reason(field.check()),
        "a field whose values are generated cannot be required"
    );
    let mut field = FieldDef::new(fid(1), "x", DataType::Number);
    field.computed = computed();
    field.validation.required_when = Some(Condition::is_empty(fid(2)));
    assert_eq!(
        reason(field.check()),
        "a field whose values are generated cannot be conditionally required"
    );
}

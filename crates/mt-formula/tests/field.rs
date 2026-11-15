//! Formulas on field definitions: attaching, reading back, checking the shape of each kind
//! of computed field and its result against the field's type, and building the graph from
//! a table's definitions.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_core::field::{Computed, ComputedKind, DataType, FieldDef, FieldStatus};
use mt_core::fieldtype::BuiltinType;
use mt_core::id::TableId;
use mt_core::validate::check_writable;
use mt_formula::field::{attach, check_field, computed_fields, formula_of};
use mt_formula::{FormulaError, Graph, Type};

fn computed(
    id: u128,
    name: &str,
    data_type: DataType,
    kind: ComputedKind,
    source: &str,
) -> FieldDef {
    let mut field = FieldDef::new(fid(id), name, data_type);
    attach(&mut field, kind, &compile(source));
    field
}

#[test]
fn std_a_formula_is_kept_on_the_definition() {
    let mut total = BuiltinType::Formula.template(fid(50), "Total");
    assert_eq!(formula_of(&total), Ok(None));
    assert_eq!(check_field(&total, &Orders), Ok(None));
    total.data_type = DataType::Money;
    total.widget = mt_core::field::Widget::MoneyInput;
    let formula = compile("{Price} * {Qty} + {Shipping}");
    attach(
        &mut total,
        BuiltinType::Formula.computed_kind().unwrap(),
        &formula,
    );
    // The definition is valid, recognised as a formula field, and cannot be typed into.
    total.check().unwrap();
    assert_eq!(BuiltinType::of(&total), BuiltinType::Formula);
    assert!(check_writable(&total).is_some());
    assert_eq!(formula_of(&total), Ok(Some(formula.clone())));
    assert_eq!(check_field(&total, &Orders), Ok(Some(Type::Money)));
    // It survives storage with the definition, and still names fields by id.
    let json = serde_json::to_string(&total).unwrap();
    assert!(json.contains("fld_"), "{json}");
    assert!(!json.contains("Price"), "{json}");
    let loaded: FieldDef = serde_json::from_str(&json).unwrap();
    assert_eq!(formula_of(&loaded), Ok(Some(formula)));
    // Attaching again replaces the formula and the kind.
    attach(
        &mut total,
        ComputedKind::Rollup,
        &compile("SUM({Lines}.{Amount})"),
    );
    assert_eq!(
        total.computed.as_ref().map(|c| c.kind),
        Some(ComputedKind::Rollup)
    );
    assert_eq!(check_field(&total, &Orders), Ok(Some(Type::Money)));
}

#[test]
fn std_each_kind_has_its_shape() {
    let lookup = computed(
        50,
        "Line names",
        DataType::Text,
        ComputedKind::Lookup,
        "{Lines}.{Name}",
    );
    assert_eq!(
        check_field(&lookup, &Orders),
        Ok(Some(Type::List(Box::new(Type::Text))))
    );
    let labels = computed(
        51,
        "Line labels",
        DataType::MultiReference,
        ComputedKind::Lookup,
        "{Lines}.{Labels}",
    );
    assert!(check_field(&labels, &Orders).is_ok());
    for (source, data_type) in [
        ("SUM({Lines}.{Amount})", DataType::Money),
        ("COUNTIF({Lines}.{Done}, TRUE)", DataType::Number),
        ("MAX({Lines}.{Shipped})", DataType::Date),
        (r#"TEXTJOIN(", ", TRUE, {Lines}.{Name})"#, DataType::Text),
        (
            "IF(COUNTA({Lines}) = 0, 0, SUM({Lines}.{Qty}))",
            DataType::Number,
        ),
    ] {
        let rollup = computed(52, "Roll-up", data_type, ComputedKind::Rollup, source);
        assert!(
            check_field(&rollup, &Orders).is_ok(),
            "{source}: {:?}",
            check_field(&rollup, &Orders)
        );
    }
    // A formula may be anything, including what a lookup or a roll-up is.
    for source in [
        "{Lines}.{Name}",
        "SUM({Lines}.{Qty})",
        "{Qty} * 2",
        r#""x""#,
    ] {
        let formula = computed(53, "Formula", DataType::Text, ComputedKind::Formula, source);
        assert!(check_field(&formula, &Orders).is_ok(), "{source}");
    }
}

#[test]
fn invalid_shapes_and_results() {
    let refused = |field: &FieldDef| match check_field(field, &Orders) {
        Err(FormulaError::Type(message)) => message,
        other => panic!("{other:?}"),
    };
    // A lookup is exactly a field of linked records.
    for source in [
        "{Qty}",
        "SUM({Lines}.{Qty})",
        "{Lines}.{Qty} & \"\"",
        "FIRST({Lines}.{Name})",
    ] {
        let lookup = computed(50, "Names", DataType::Text, ComputedKind::Lookup, source);
        assert_eq!(
            refused(&lookup),
            "`Names`: a lookup is a field of linked records: {Link}.{Field}",
            "{source}"
        );
    }
    // A roll-up is a function over linked records.
    for source in [
        "{Lines}.{Qty}",
        "{Qty} + 1",
        "SUM({Qty})",
        "SUM({Lines}.{Qty}) + 1",
    ] {
        let rollup = computed(51, "Sum", DataType::Number, ComputedKind::Rollup, source);
        assert!(
            refused(&rollup)
                .starts_with("`Sum`: a roll-up is a function over a field of linked records"),
            "{source}"
        );
    }
    // The result must fit the field.
    let wrong = computed(
        52,
        "Total",
        DataType::Number,
        ComputedKind::Formula,
        "{Price} * 2",
    );
    assert_eq!(
        refused(&wrong),
        "the formula can return an amount, but the field is a number field"
    );
    let wrong = computed(
        53,
        "Names",
        DataType::Number,
        ComputedKind::Lookup,
        "{Lines}.{Name}",
    );
    assert_eq!(
        refused(&wrong),
        "the formula can return a list, but the field is a number field"
    );
    let wrong = computed(
        54,
        "Sum",
        DataType::Money,
        ComputedKind::Rollup,
        "SUM({Lines}.{Qty})",
    );
    assert_eq!(
        refused(&wrong),
        "the formula can return a number, but the field is a money field"
    );
    // Problems inside the formula.
    let wrong = computed(
        55,
        "Bad",
        DataType::Text,
        ComputedKind::Formula,
        "{Price} + 1",
    );
    assert!(refused(&wrong).contains("cannot combine an amount and a number"));
}

#[test]
fn anomalous_stored_formulas_that_cannot_be_read() {
    for formula in [
        serde_json::json!({"t": "call", "v": {"function": "FROM_THE_FUTURE", "args": []}}),
        serde_json::json!({"t": "hologram", "v": 1}),
        serde_json::json!({}),
        serde_json::json!("1 + 1"),
        serde_json::Value::Null,
    ] {
        let mut field = FieldDef::new(fid(50), "Total", DataType::Number);
        field.computed = Some(Computed {
            kind: ComputedKind::Formula,
            formula,
        });
        for result in [
            formula_of(&field).map(|_| ()),
            check_field(&field, &Orders).map(|_| ()),
            computed_fields(TableId::from_u128(1), std::slice::from_ref(&field)).map(|_| ()),
        ] {
            let Err(FormulaError::Stored {
                field: name,
                reason,
            }) = result
            else {
                panic!("{result:?}");
            };
            assert_eq!(name, "Total");
            assert!(!reason.is_empty());
        }
    }
    let error = FormulaError::Stored {
        field: "Total".into(),
        reason: "unknown variant".into(),
    };
    assert_eq!(
        error.to_string(),
        "the formula of `Total` cannot be read: unknown variant"
    );
}

#[test]
fn std_the_graph_is_built_from_the_definitions_of_a_table() {
    let table = TableId::from_u128(1);
    let mut disabled = computed(
        52,
        "Old total",
        DataType::Money,
        ComputedKind::Formula,
        "{Price} * 2",
    );
    disabled.status = FieldStatus::Disabled;
    let fields = vec![
        FieldDef::new(fid(QTY), "Qty", DataType::Number),
        computed(
            50,
            "Subtotal",
            DataType::Money,
            ComputedKind::Rollup,
            "SUM({Lines}.{Amount})",
        ),
        FieldDef::new(fid(PRICE), "Price", DataType::Money),
        computed(
            51,
            "Average",
            DataType::Money,
            ComputedKind::Formula,
            "{Price} / {Qty}",
        ),
        disabled,
    ];
    let found = computed_fields(table, &fields).unwrap();
    let ids: Vec<_> = found.iter().map(|computed| computed.field).collect();
    assert_eq!(ids, [fid(50), fid(51), fid(52)]);
    assert!(found.iter().all(|computed| computed.table == table));
    let graph = Graph::build(
        found,
        &|_, link| (link == fid(LINES)).then_some(TableId::from_u128(2)),
        &|_, field| field.to_string(),
    )
    .unwrap();
    assert_eq!(graph.len(), 3);
    let changed = [fid(QTY)].into_iter().collect();
    assert_eq!(graph.affected(table, &changed).local, [fid(51)]);
    assert_eq!(computed_fields(table, &[]), Ok(vec![]));
}

//! What formulas read, design-time warnings, the graph of computed fields (cycles, order),
//! what a change affects (in the same record and across links) and computing a record.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::{at, day, dec, fid, money, num, show, Rec, UTC};
use mt_core::field::DataType;
use mt_core::id::{FieldId, TableId};
use mt_core::value::Value;
use mt_formula::deps::{MANY_LINKED_LOOKUPS, MAX_COMPUTED_FIELDS};
use mt_formula::{
    analyse, dependencies, evaluate, parse, Affected, Analysis, ComputedField, Dependencies,
    ErrorValue, Expr, FormulaError, Graph, Remote, Schema, Val, Warning,
};
use std::collections::BTreeSet;

// A small base: customers have orders, orders have lines, lines point back to their order.
const CUSTOMERS: u128 = 1;
const ORDERS: u128 = 2;
const LINES: u128 = 3;

// Customers.
const C_NAME: u128 = 10;
const C_ORDERS: u128 = 11; // link → orders
const C_REVENUE: u128 = 12; // computed
const C_TIER: u128 = 13; // computed

// Orders.
const O_TITLE: u128 = 20;
const O_LINES: u128 = 21; // link → lines
const O_SHIPPING: u128 = 22;
const O_DUE: u128 = 23;
const O_DONE: u128 = 24;
const O_SUBTOTAL: u128 = 25; // computed
const O_TAX: u128 = 26; // computed
const O_TOTAL: u128 = 27; // computed
const O_LATE: u128 = 28; // computed
const O_SUMMARY: u128 = 29; // computed
const O_COUNT: u128 = 30;
const O_AVERAGE: u128 = 31; // computed
const O_SAFE: u128 = 32; // computed

// Lines.
const L_NAME: u128 = 40;
const L_QTY: u128 = 41;
const L_PRICE: u128 = 42;
const L_ORDER: u128 = 43; // link → orders
const L_AMOUNT: u128 = 44; // computed

type Field = (u128, &'static str, DataType);
/// A table: its id, its fields, and where its link fields lead.
type Table = (u128, &'static [Field], &'static [(u128, u128)]);

const TABLES: &[Table] = &[
    (
        CUSTOMERS,
        &[
            (C_NAME, "Name", DataType::Text),
            (C_ORDERS, "Orders", DataType::Link),
            (C_REVENUE, "Revenue", DataType::Money),
            (C_TIER, "Tier", DataType::Text),
        ],
        &[(C_ORDERS, ORDERS)],
    ),
    (
        ORDERS,
        &[
            (O_TITLE, "Title", DataType::Text),
            (O_LINES, "Lines", DataType::Link),
            (O_SHIPPING, "Shipping", DataType::Money),
            (O_DUE, "Due", DataType::Date),
            (O_DONE, "Done", DataType::Bool),
            (O_SUBTOTAL, "Subtotal", DataType::Money),
            (O_TAX, "Tax", DataType::Money),
            (O_TOTAL, "Total", DataType::Money),
            (O_LATE, "Late", DataType::Bool),
            (O_SUMMARY, "Summary", DataType::Text),
            (O_COUNT, "Count", DataType::Number),
            (O_AVERAGE, "Average", DataType::Money),
            (O_SAFE, "Safe", DataType::Money),
        ],
        &[(O_LINES, LINES)],
    ),
    (
        LINES,
        &[
            (L_NAME, "Name", DataType::Text),
            (L_QTY, "Qty", DataType::Number),
            (L_PRICE, "Price", DataType::Money),
            (L_ORDER, "Order", DataType::Link),
            (L_AMOUNT, "Amount", DataType::Money),
        ],
        &[(L_ORDER, ORDERS)],
    ),
];

fn table(id: u128) -> TableId {
    TableId::from_u128(id)
}

fn fields_of(table: u128) -> &'static [Field] {
    TABLES
        .iter()
        .find(|(id, _, _)| *id == table)
        .map_or(&[], |(_, fields, _)| fields)
}

fn target_of(table: u128, link: FieldId) -> Option<u128> {
    let (_, _, links) = TABLES.iter().find(|(id, _, _)| *id == table)?;
    links
        .iter()
        .find(|(field, _)| fid(*field) == link)
        .map(|(_, target)| *target)
}

/// One table of the base as the formulas of that table see it.
struct In(u128);

impl Schema for In {
    fn field_by_name(&self, name: &str) -> Option<FieldId> {
        fields_of(self.0)
            .iter()
            .find(|(_, n, _)| n.eq_ignore_ascii_case(name))
            .map(|(id, _, _)| fid(*id))
    }

    fn field_type(&self, field: FieldId) -> Option<DataType> {
        fields_of(self.0)
            .iter()
            .find(|(id, _, _)| fid(*id) == field)
            .map(|(_, _, t)| *t)
    }

    fn linked_field_by_name(&self, link: FieldId, name: &str) -> Option<FieldId> {
        In(target_of(self.0, link)?).field_by_name(name)
    }

    fn linked_field_type(&self, link: FieldId, field: FieldId) -> Option<DataType> {
        In(target_of(self.0, link)?).field_type(field)
    }
}

fn formula(table_id: u128, source: &str) -> Expr {
    parse(source, &In(table_id)).unwrap_or_else(|error| panic!("`{source}`: {error}"))
}

fn computed(table_id: u128, field: u128, source: &str) -> ComputedField {
    ComputedField {
        table: table(table_id),
        field: fid(field),
        expr: formula(table_id, source),
    }
}

fn link_target(from: TableId, link: FieldId) -> Option<TableId> {
    TABLES
        .iter()
        .find(|(id, _, _)| table(*id) == from)
        .and_then(|(id, _, _)| target_of(*id, link))
        .map(table)
}

fn name(of: TableId, field: FieldId) -> String {
    TABLES
        .iter()
        .find(|(id, _, _)| table(*id) == of)
        .and_then(|(_, fields, _)| fields.iter().find(|(id, _, _)| fid(*id) == field))
        .map_or_else(|| field.to_string(), |(_, name, _)| (*name).to_owned())
}

/// The computed fields of the base.
fn definitions() -> Vec<ComputedField> {
    vec![
        computed(
            CUSTOMERS,
            C_TIER,
            r#"IF({Revenue} > PARAM("tier.gold"), "gold", "standard")"#,
        ),
        computed(CUSTOMERS, C_REVENUE, "SUM({Orders}.{Total})"),
        computed(ORDERS, O_TOTAL, "{Subtotal} + {Tax} + {Shipping}"),
        computed(ORDERS, O_TAX, r#"{Subtotal} * PARAM("tax.rate")"#),
        computed(ORDERS, O_SUBTOTAL, "SUM({Lines}.{Amount})"),
        computed(ORDERS, O_LATE, "AND(NOT({Done}), {Due} < TODAY())"),
        computed(
            ORDERS,
            O_SUMMARY,
            r#"{Title} & ": " & COUNTA({Lines}) & " line(s)""#,
        ),
        computed(ORDERS, O_AVERAGE, "{Subtotal} / {Count}"),
        computed(ORDERS, O_SAFE, "IFERROR({Average}, {Subtotal})"),
        computed(LINES, L_AMOUNT, "{Price} * {Qty}"),
    ]
}

fn graph() -> Graph {
    Graph::build(definitions(), &link_target, &name).unwrap()
}

fn ids(fields: &[u128]) -> Vec<FieldId> {
    fields.iter().map(|f| fid(*f)).collect()
}

fn set(fields: &[u128]) -> BTreeSet<FieldId> {
    fields.iter().map(|f| fid(*f)).collect()
}

fn position(order: &[(TableId, FieldId)], table_id: u128, field: u128) -> usize {
    order
        .iter()
        .position(|key| *key == (table(table_id), fid(field)))
        .unwrap()
}

#[test]
fn std_what_a_formula_reads() {
    let found = dependencies(&formula(ORDERS, "{Subtotal} + {Tax} + {Shipping}"));
    assert_eq!(
        found,
        Dependencies {
            fields: set(&[O_SUBTOTAL, O_TAX, O_SHIPPING]),
            ..Dependencies::default()
        }
    );
    // Going through a link reads the link field itself and the linked field.
    let found = dependencies(&formula(
        ORDERS,
        "SUM({Lines}.{Amount}) + COUNTA({Lines}.{Qty})",
    ));
    assert_eq!(found.fields, set(&[O_LINES]));
    assert_eq!(
        found.through,
        [(fid(O_LINES), fid(L_AMOUNT)), (fid(O_LINES), fid(L_QTY))]
            .into_iter()
            .collect()
    );
    assert!(!found.volatile && !found.any_param && found.params.is_empty());
    // Parameters by name; a computed key means "any parameter".
    let found = dependencies(&formula(
        ORDERS,
        r#"PARAM("tax.rate") + PARAM("tax.rate") + PARAM("x")"#,
    ));
    assert_eq!(
        found.params,
        ["tax.rate".to_owned(), "x".to_owned()]
            .into_iter()
            .collect()
    );
    assert!(!found.any_param);
    assert!(dependencies(&formula(ORDERS, r#"PARAM("tax." & {Title})"#)).any_param);
    // The clock.
    assert!(dependencies(&formula(ORDERS, "{Due} < TODAY()")).volatile);
    assert!(dependencies(&formula(ORDERS, "IF({Done}, 1, HOUR(NOW()))")).volatile);
    assert!(!dependencies(&formula(ORDERS, "{Due} + 1")).volatile);
    // Fields inside conditions and nested calls are found too.
    let found = dependencies(&formula(
        ORDERS,
        "COUNTIF({Lines}.{Qty}, CurrentValue > {Count}) + IF({Done}, -{Shipping}, 0)",
    ));
    assert_eq!(found.fields, set(&[O_LINES, O_COUNT, O_DONE, O_SHIPPING]));
    // Constants read nothing.
    assert_eq!(
        dependencies(&formula(ORDERS, r#"1 + LEN("abc")"#)),
        Dependencies::default()
    );
    // What is read survives storage.
    let found = dependencies(&formula(
        ORDERS,
        r#"SUM({Lines}.{Amount}) * PARAM("tax.rate")"#,
    ));
    let json = serde_json::to_string(&found).unwrap();
    assert_eq!(serde_json::from_str::<Dependencies>(&json).unwrap(), found);
}

#[test]
fn std_warnings_for_the_person_writing_a_formula() {
    let warnings = |source: &str| analyse(&formula(ORDERS, source)).warnings;
    assert_eq!(warnings("{Subtotal} + {Tax}"), []);
    assert_eq!(warnings("SUM({Lines}.{Amount})"), []);
    assert_eq!(warnings("{Due} < TODAY()"), [Warning::Volatile]);
    assert_eq!(
        warnings("COUNTIF({Lines}.{Qty}, CurrentValue > 0) + DAYS({Due}, NOW())"),
        [Warning::VolatileOverLinks]
    );
    // Each walk through a link reads every linked record.
    let five = ["SUM({Lines}.{Qty})"; MANY_LINKED_LOOKUPS].join(" + ");
    assert_eq!(warnings(&five), []);
    let six = format!("{five} + MAX({{Lines}}.{{Qty}})");
    assert_eq!(warnings(&six), [Warning::ManyLinkedLookups { count: 6 }]);
    // A list inside the condition of a list function is read once per item.
    assert_eq!(
        warnings("COUNTIF({Lines}.{Qty}, CurrentValue > AVERAGE({Lines}.{Qty}))"),
        [Warning::NestedListCondition]
    );
    assert_eq!(
        warnings("FILTER({Lines}.{Qty}, COUNTIF({Lines}.{Qty}, CurrentValue) > 1)"),
        [Warning::NestedListCondition]
    );
    assert_eq!(warnings("SUMIF({Lines}.{Qty}, CurrentValue > {Count})"), []);
    assert_eq!(
        warnings(r#"SUMIF({Lines}.{Qty}, ">1", {Lines}.{Amount})"#),
        []
    );
    assert_eq!(
        warnings(&format!(
            "{six} + COUNTIF({{Lines}}.{{Qty}}, FIRST({{Lines}}.{{Qty}})) + HOUR(NOW())"
        )),
        [
            Warning::VolatileOverLinks,
            Warning::ManyLinkedLookups { count: 8 },
            Warning::NestedListCondition
        ]
    );
    // A hand-built call without its condition is analysed without complaint.
    let incomplete = Expr::Call {
        function: mt_formula::Function::List(mt_formula::ListFn::CountIf),
        args: vec![formula(ORDERS, "{Lines}.{Qty}")],
    };
    assert_eq!(analyse(&incomplete).warnings, []);
    // The whole analysis, as the formula editor receives it.
    let analysis = analyse(&formula(ORDERS, "IF({Done}, 0, {Due} - TODAY())"));
    assert_eq!(analysis.nodes, 6);
    assert_eq!(analysis.depth, 3);
    assert_eq!(analysis.dependencies.fields, set(&[O_DONE, O_DUE]));
    assert_eq!(analysis.warnings, [Warning::Volatile]);
    let json = serde_json::to_value(&analysis).unwrap();
    assert_eq!(
        json["warnings"],
        serde_json::json!([{"warning": "volatile"}])
    );
    assert_eq!(serde_json::from_value::<Analysis>(json).unwrap(), analysis);
    let many = serde_json::to_value(Warning::ManyLinkedLookups { count: 6 }).unwrap();
    assert_eq!(
        many,
        serde_json::json!({"warning": "many_linked_lookups", "count": 6})
    );
}

#[test]
fn std_every_formula_comes_after_what_it_reads() {
    let graph = graph();
    assert_eq!(graph.len(), 10);
    assert!(!graph.is_empty());
    assert!(Graph::default().is_empty());
    let order = graph.order();
    assert_eq!(order.len(), 10);
    let before = |a: (u128, u128), b: (u128, u128)| {
        assert!(
            position(order, a.0, a.1) < position(order, b.0, b.1),
            "{a:?} before {b:?}"
        );
    };
    // Within a record …
    before((ORDERS, O_SUBTOTAL), (ORDERS, O_TAX));
    before((ORDERS, O_SUBTOTAL), (ORDERS, O_TOTAL));
    before((ORDERS, O_TAX), (ORDERS, O_TOTAL));
    before((ORDERS, O_SUBTOTAL), (ORDERS, O_AVERAGE));
    before((ORDERS, O_AVERAGE), (ORDERS, O_SAFE));
    before((CUSTOMERS, C_REVENUE), (CUSTOMERS, C_TIER));
    // … and across links.
    before((LINES, L_AMOUNT), (ORDERS, O_SUBTOTAL));
    before((ORDERS, O_TOTAL), (CUSTOMERS, C_REVENUE));
    assert_eq!(
        graph.order_in(table(ORDERS)).len(),
        7,
        "{:?}",
        graph.order_in(table(ORDERS))
    );
    assert_eq!(graph.order_in(table(LINES)), ids(&[L_AMOUNT]));
    assert_eq!(graph.order_in(table(CUSTOMERS)), ids(&[C_REVENUE, C_TIER]));
    assert_eq!(graph.order_in(table(99)), []);

    // The order does not depend on the order the definitions were given in.
    let mut reversed = definitions();
    reversed.reverse();
    let other = Graph::build(reversed, &link_target, &name).unwrap();
    assert_eq!(other, graph);
    assert_eq!(other.order(), graph.order());

    // Looking a field up.
    assert_eq!(
        graph.expr(table(LINES), fid(L_AMOUNT)),
        Some(&formula(LINES, "{Price} * {Qty}"))
    );
    assert_eq!(graph.expr(table(LINES), fid(L_QTY)), None);
    assert_eq!(graph.expr(table(ORDERS), fid(L_AMOUNT)), None);
    assert_eq!(
        graph
            .dependencies_of(table(ORDERS), fid(O_TAX))
            .map(|deps| deps.params.len()),
        Some(1)
    );
    assert_eq!(graph.dependencies_of(table(ORDERS), fid(O_TITLE)), None);
}

fn cycle(fields: Vec<ComputedField>) -> String {
    match Graph::build(fields, &link_target, &name) {
        Err(FormulaError::Cycle { path }) => path,
        other => panic!("expected a cycle, got {other:?}"),
    }
}

#[test]
fn invalid_circular_definitions() {
    // A field that reads itself.
    assert_eq!(
        cycle(vec![computed(ORDERS, O_TOTAL, "{Total} + 1")]),
        "Total → Total"
    );
    // Two fields that read each other.
    assert_eq!(
        cycle(vec![
            computed(ORDERS, O_TAX, "{Total} * 0.1"),
            computed(ORDERS, O_TOTAL, "{Subtotal} + {Tax}"),
        ]),
        "Tax → Total → Tax"
    );
    // A longer circle; the fields that only lead into it are not part of the message.
    assert_eq!(
        cycle(vec![
            computed(ORDERS, O_SUMMARY, "{Title} & {Total}"),
            computed(ORDERS, O_SUBTOTAL, "{Average} * {Count}"),
            computed(ORDERS, O_AVERAGE, "{Total} / {Count}"),
            computed(ORDERS, O_TOTAL, "{Subtotal} + {Shipping}"),
        ]),
        "Subtotal → Average → Total → Subtotal"
    );
    // A circle through links: an order's subtotal sums its lines' amounts, and a line's
    // amount is a share of its order's subtotal.
    assert_eq!(
        cycle(vec![
            computed(ORDERS, O_SUBTOTAL, "SUM({Lines}.{Amount})"),
            computed(LINES, L_AMOUNT, "FIRST({Order}.{Subtotal}) * {Qty}"),
        ]),
        "Subtotal → Amount → Subtotal"
    );
    // A circle inside a condition is still a circle.
    assert_eq!(
        cycle(vec![computed(
            ORDERS,
            O_LATE,
            "IF({Done}, FALSE, NOT({Late}))"
        )]),
        "Late → Late"
    );
    // Reading the same field twice, or a diamond, is not a circle.
    Graph::build(
        vec![
            computed(ORDERS, O_SUBTOTAL, "SUM({Lines}.{Amount})"),
            computed(ORDERS, O_TAX, "{Subtotal} * 0.1 + {Subtotal} * 0.05"),
            computed(ORDERS, O_AVERAGE, "{Subtotal} / 2"),
            computed(ORDERS, O_TOTAL, "{Tax} + {Average}"),
        ],
        &link_target,
        &name,
    )
    .unwrap();
    // Reading a plain link back is fine as long as no computed field closes the circle.
    Graph::build(
        vec![
            computed(ORDERS, O_SUBTOTAL, "SUM({Lines}.{Amount})"),
            computed(
                LINES,
                L_AMOUNT,
                r#"{Price} * {Qty} + SUM({Order}.{Shipping})"#,
            ),
        ],
        &link_target,
        &name,
    )
    .unwrap();
}

#[test]
fn invalid_definitions() {
    // The same field twice.
    let twice = vec![computed(ORDERS, O_TAX, "1"), computed(ORDERS, O_TAX, "2")];
    assert_eq!(
        Graph::build(twice, &link_target, &name),
        Err(FormulaError::Type("`Tax` is defined twice".into()))
    );
    // Going through a field that is not (or no longer) a link.
    let through = ComputedField {
        table: table(ORDERS),
        field: fid(O_TAX),
        expr: Expr::Through {
            link: fid(O_TITLE),
            field: fid(L_QTY),
        },
    };
    assert_eq!(
        Graph::build(vec![through], &link_target, &name),
        Err(FormulaError::Type(
            "`Tax` goes through `Title`, which is not a link field".into()
        ))
    );
    // Too many computed fields.
    let many = |count: usize| -> Vec<ComputedField> {
        (0..count)
            .map(|n| ComputedField {
                table: table(ORDERS),
                field: fid(1_000 + n as u128),
                expr: Expr::Bool(true),
            })
            .collect()
    };
    assert_eq!(
        Graph::build(many(MAX_COMPUTED_FIELDS + 1), &link_target, &name),
        Err(FormulaError::Limit("more than 10,000 computed fields"))
    );
    assert_eq!(
        Graph::build(many(MAX_COMPUTED_FIELDS), &link_target, &name)
            .unwrap()
            .len(),
        10_000
    );
    // A long chain is ordered without deep recursion.
    let chain: Vec<ComputedField> = (0..5_000_u128)
        .map(|n| ComputedField {
            table: table(ORDERS),
            field: fid(1_000 + n),
            expr: if n == 0 {
                Expr::Bool(true)
            } else {
                Expr::Field(fid(999 + n))
            },
        })
        .rev()
        .collect();
    let graph = Graph::build(chain, &link_target, &name).unwrap();
    let order = graph.order_in(table(ORDERS));
    assert_eq!(order.first(), Some(&fid(1_000)));
    assert_eq!(order.last(), Some(&fid(5_999)));
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn std_what_a_change_affects() {
    let graph = graph();
    let affected =
        |table_id: u128, changed: &[u128]| graph.affected(table(table_id), &set(changed));
    let remote = |table_id: u128, link: u128, fields: &[u128]| Remote {
        table: table(table_id),
        link: fid(link),
        fields: ids(fields),
    };

    // A line's quantity: its amount, then the orders that contain the line.
    assert_eq!(
        affected(LINES, &[L_QTY]),
        Affected {
            local: ids(&[L_AMOUNT]),
            remote: vec![remote(
                ORDERS,
                O_LINES,
                &[O_SUBTOTAL, O_TAX, O_TOTAL, O_AVERAGE, O_SAFE]
            )],
        }
    );
    // Following the chain: those order fields changed, which customers read.
    let next = affected(ORDERS, &[O_SUBTOTAL, O_TAX, O_TOTAL, O_AVERAGE, O_SAFE]);
    assert_eq!(
        next,
        Affected {
            local: vec![],
            remote: vec![remote(CUSTOMERS, C_ORDERS, &[C_REVENUE, C_TIER])]
        }
    );
    assert_eq!(
        affected(CUSTOMERS, &[C_REVENUE, C_TIER]),
        Affected::default()
    );

    // An order's shipping: only the total, and the customer's revenue.
    assert_eq!(
        affected(ORDERS, &[O_SHIPPING]),
        Affected {
            local: ids(&[O_TOTAL]),
            remote: vec![remote(CUSTOMERS, C_ORDERS, &[C_REVENUE, C_TIER])],
        }
    );
    // Adding or removing lines changes the link field.
    let lines_changed = affected(ORDERS, &[O_LINES]);
    assert_eq!(lines_changed.local.len(), 6);
    assert!(lines_changed.local.contains(&fid(O_SUMMARY)));
    assert!(!lines_changed.local.contains(&fid(O_LATE)));
    assert_eq!(
        lines_changed.remote,
        vec![remote(CUSTOMERS, C_ORDERS, &[C_REVENUE, C_TIER])]
    );
    // Local changes that nobody else reads.
    assert_eq!(
        affected(ORDERS, &[O_TITLE]),
        Affected {
            local: ids(&[O_SUMMARY]),
            remote: vec![]
        }
    );
    assert_eq!(
        affected(ORDERS, &[O_DUE, O_DONE]),
        Affected {
            local: ids(&[O_LATE]),
            remote: vec![]
        }
    );
    assert_eq!(
        affected(ORDERS, &[O_COUNT]),
        Affected {
            local: ids(&[O_AVERAGE, O_SAFE]),
            remote: vec![]
        }
    );
    // A line's name is read by nothing; a customer's name neither.
    assert_eq!(affected(LINES, &[L_NAME]), Affected::default());
    assert_eq!(affected(CUSTOMERS, &[C_NAME]), Affected::default());
    assert_eq!(affected(ORDERS, &[]), Affected::default());
    assert_eq!(affected(99, &[O_TITLE]), Affected::default());
    // New links from a customer to orders: the customer's own fields.
    assert_eq!(
        affected(CUSTOMERS, &[C_ORDERS]),
        Affected {
            local: ids(&[C_REVENUE, C_TIER]),
            remote: vec![]
        }
    );
    // The local fields come in evaluation order.
    let local = affected(ORDERS, &[O_LINES, O_COUNT, O_TITLE]).local;
    let order = graph.order_in(table(ORDERS));
    let positions: Vec<usize> = local
        .iter()
        .map(|f| order.iter().position(|o| o == f).unwrap())
        .collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "{local:?}"
    );
}

#[test]
fn edge_two_links_to_the_same_table_are_told_apart() {
    // Lines read their order through `Order`; orders read their lines through `Lines`.
    let fields = vec![
        computed(ORDERS, O_SUBTOTAL, "SUM({Lines}.{Amount})"),
        computed(LINES, L_AMOUNT, "{Price} * {Qty} + SUM({Order}.{Shipping})"),
        computed(
            CUSTOMERS,
            C_REVENUE,
            "SUM({Orders}.{Subtotal}) + SUM({Orders}.{Shipping})",
        ),
    ];
    let graph = Graph::build(fields, &link_target, &name).unwrap();
    let affected = graph.affected(table(ORDERS), &set(&[O_SHIPPING]));
    assert_eq!(affected.local, []);
    assert_eq!(
        affected.remote,
        vec![
            Remote {
                table: table(CUSTOMERS),
                link: fid(C_ORDERS),
                fields: ids(&[C_REVENUE])
            },
            Remote {
                table: table(LINES),
                link: fid(L_ORDER),
                fields: ids(&[L_AMOUNT])
            },
        ]
    );
    // The lines' amounts changed, which the orders read: the chain goes on (and would go
    // round and round if a computed field closed the circle, which `build` refuses).
    let next = graph.affected(table(LINES), &set(&[L_AMOUNT]));
    assert_eq!(
        next.remote,
        vec![Remote {
            table: table(ORDERS),
            link: fid(O_LINES),
            fields: ids(&[O_SUBTOTAL])
        }]
    );
}

/// An order with two lines (amounts 20.00 and 10.50 USD, as computed for the lines).
fn order_record() -> Rec {
    let mut record = Rec::default();
    let cells = [
        (O_TITLE, Value::Text("Spring order".into())),
        (O_SHIPPING, money("4.50", "USD")),
        (O_DUE, Value::Date(day(2026, 5, 1))),
        (O_DONE, Value::Bool(false)),
        (O_COUNT, num("4")),
        (
            O_LINES,
            Value::Link(vec![
                mt_core::id::RecordId::from_u128(1),
                mt_core::id::RecordId::from_u128(2),
            ]),
        ),
        // Stale stored results from before the lines changed.
        (O_SUBTOTAL, money("1.00", "USD")),
        (O_TAX, money("0.19", "USD")),
    ];
    for (field, value) in cells {
        record.cells.insert(fid(field), value);
    }
    record.linked.insert(
        (fid(O_LINES), fid(L_AMOUNT)),
        vec![money("20.00", "USD"), money("10.50", "USD")],
    );
    record
}

#[test]
fn std_computing_a_record() {
    let graph = graph();
    let record = order_record();
    let results = graph.compute(table(ORDERS), &In(ORDERS), &record, &UTC, None);
    assert_eq!(results.len(), 7);
    assert_eq!(results[&fid(O_SUBTOTAL)], Ok(money("30.50", "USD")));
    // 30.50 × 19 % = 5.795, stored as 5.80 (half-even to cents) …
    assert_eq!(results[&fid(O_TAX)], Ok(money("5.80", "USD")));
    // … and the total adds what is stored: 30.50 + 5.80 + 4.50.
    assert_eq!(results[&fid(O_TOTAL)], Ok(money("40.80", "USD")));
    assert_eq!(results[&fid(O_LATE)], Ok(Value::Bool(true)));
    assert_eq!(
        results[&fid(O_SUMMARY)],
        Ok(Value::Text("Spring order: 2 line(s)".into()))
    );
    // 30.50 / 4 = 7.625, half-even to 7.62.
    assert_eq!(results[&fid(O_AVERAGE)], Ok(money("7.62", "USD")));
    assert_eq!(results[&fid(O_SAFE)], Ok(money("7.62", "USD")));

    // An error is a result like any other, and the next formula can catch it.
    let mut no_count = order_record();
    no_count.cells.remove(&fid(O_COUNT));
    let results = graph.compute(table(ORDERS), &In(ORDERS), &no_count, &UTC, None);
    assert_eq!(results[&fid(O_AVERAGE)], Err(ErrorValue::DivZero));
    assert_eq!(results[&fid(O_SAFE)], Ok(money("30.50", "USD")));
    assert_eq!(results[&fid(O_TOTAL)], Ok(money("40.80", "USD")));

    // A partial recompute: only what a change to shipping affects. The other computed
    // fields are read as stored (here: the stale subtotal and tax).
    let affected = graph.affected(table(ORDERS), &set(&[O_SHIPPING]));
    let only: BTreeSet<FieldId> = affected.local.iter().copied().collect();
    let results = graph.compute(table(ORDERS), &In(ORDERS), &record, &UTC, Some(&only));
    assert_eq!(results.len(), 1);
    assert_eq!(results[&fid(O_TOTAL)], Ok(money("5.69", "USD")));

    // Another table, an unknown table, and a record without anything.
    let mut line = Rec::default();
    line.cells.insert(fid(L_PRICE), money("3.50", "USD"));
    line.cells.insert(fid(L_QTY), num("3"));
    let results = graph.compute(table(LINES), &In(LINES), &line, &UTC, None);
    assert_eq!(results.len(), 1);
    assert_eq!(results[&fid(L_AMOUNT)], Ok(money("10.50", "USD")));
    assert!(graph
        .compute(table(99), &In(ORDERS), &record, &UTC, None)
        .is_empty());
    let blank = graph.compute(table(ORDERS), &In(ORDERS), &Rec::default(), &UTC, None);
    assert_eq!(blank[&fid(O_SUBTOTAL)], Ok(Value::Null));
    assert_eq!(blank[&fid(O_TOTAL)], Ok(Value::Null));
    assert_eq!(blank[&fid(O_LATE)], Ok(Value::Bool(false)));
    assert_eq!(
        blank[&fid(O_SUMMARY)],
        Ok(Value::Text(": 0 line(s)".into()))
    );
    assert_eq!(blank[&fid(O_AVERAGE)], Err(ErrorValue::DivZero));
    assert_eq!(blank[&fid(O_SAFE)], Ok(Value::Null));

    // A computed field the schema does not know (deleted meanwhile) is skipped.
    struct WithoutTax;
    impl Schema for WithoutTax {
        fn field_by_name(&self, name: &str) -> Option<FieldId> {
            In(ORDERS).field_by_name(name)
        }
        fn field_type(&self, field: FieldId) -> Option<DataType> {
            (field != fid(O_TAX))
                .then(|| In(ORDERS).field_type(field))
                .flatten()
        }
        fn linked_field_by_name(&self, link: FieldId, name: &str) -> Option<FieldId> {
            In(ORDERS).linked_field_by_name(link, name)
        }
        fn linked_field_type(&self, link: FieldId, field: FieldId) -> Option<DataType> {
            In(ORDERS).linked_field_type(link, field)
        }
    }
    let results = graph.compute(table(ORDERS), &WithoutTax, &record, &UTC, None);
    assert!(!results.contains_key(&fid(O_TAX)));
    // What still reads the deleted field shows that the reference is gone.
    assert_eq!(results[&fid(O_TOTAL)], Err(ErrorValue::Ref));
    assert_eq!(results[&fid(O_SUBTOTAL)], Ok(money("30.50", "USD")));
}

#[test]
fn std_a_computed_result_is_what_the_formula_alone_gives() {
    // Evaluating a formula by itself against a record that already holds the results of
    // the fields it reads gives the same value as computing the record in one go.
    let graph = graph();
    let mut record = order_record();
    let results = graph.compute(table(ORDERS), &In(ORDERS), &record, &UTC, None);
    for (field, result) in &results {
        if let Ok(value) = result {
            record.cells.insert(*field, value.clone());
        } else {
            record.cells.remove(field);
        }
    }
    let total = evaluate(
        &formula(ORDERS, "{Subtotal} + {Tax} + {Shipping}"),
        &In(ORDERS),
        &record,
        &UTC,
    );
    assert_eq!(show(&total), "40.8 USD");
    assert_eq!(
        total,
        Val::Money(dec("40.80"), mt_core::money::Currency::new("USD").unwrap())
    );
    let late = evaluate(
        &formula(ORDERS, "AND(NOT({Done}), {Due} < TODAY())"),
        &In(ORDERS),
        &record,
        &UTC,
    );
    assert_eq!(late, Val::Bool(true));
    let _ = at(2026, 1, 1, 0, 0);
}

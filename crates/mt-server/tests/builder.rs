//! Building a configuration from an empty workspace through the API, the way the web app
//! does it: a base, a code list, two tables, fields of many kinds (a two-way link, a roll-up,
//! a formula reading a setting), people and groups, editing and deleting, templates — and
//! everything that is refused.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use http_body_util as _;
use tower as _;

fn empty() -> Arc<App> {
    mt_server::open(None, Clock::Fixed(now()), mt_server::Start::Empty).unwrap()
}

async fn post(app: &Arc<App>, path: &str, body: Value) -> (StatusCode, Value) {
    call(app, Method::POST, path, Some(ALICE), Some(body)).await
}

async fn id(app: &Arc<App>, path: &str, body: Value) -> String {
    let (status, value) = post(app, path, body).await;
    assert_eq!(status, StatusCode::OK, "{path}: {value}");
    value["id"].as_str().unwrap().to_owned()
}

async fn field(app: &Arc<App>, table: &str, body: Value) -> String {
    let (status, value) = post(app, &format!("/api/tables/{table}/fields"), body).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    value["field"].as_str().unwrap().to_owned()
}

async fn columns(app: &Arc<App>, table: &str) -> Vec<String> {
    let (_, value) = call(
        app,
        Method::GET,
        &format!("/api/tables/{table}"),
        None,
        None,
    )
    .await;
    value["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_owned())
        .collect()
}

async fn rows(app: &Arc<App>, table: &str) -> Value {
    let (_, value) = call(
        app,
        Method::POST,
        &format!("/api/tables/{table}/query"),
        None,
        Some(json!({})),
    )
    .await;
    value
}

fn cell(result: &Value, title: &str, field: &str) -> String {
    result["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["title"] == title)
        .unwrap_or_else(|| panic!("no row {title}"))["cells"][field]["d"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

struct Expenses {
    base: String,
    claims: String,
    lines: String,
    categories: String,
    amount: String,
    total: String,
    needs_finance: String,
    claim_link: String,
    lines_link: String,
}

/// The S3 expense configuration of doc 14, built step by step.
async fn build(app: &Arc<App>) -> Expenses {
    let base = id(
        app,
        "/api/bases",
        json!({"name": "Expenses", "description": "Claims and their lines"}),
    )
    .await;
    let categories = id(
        app,
        "/api/refdata",
        json!({"code": "expense_category", "name": "Expense categories", "tree": true}),
    )
    .await;
    let travel = id(
        app,
        &format!("/api/refdata/{categories}/items"),
        json!({"code": "TRAVEL", "name": "Travel"}),
    )
    .await;
    for (code, name) in [("FLIGHT", "Flights"), ("HOTEL", "Hotels")] {
        id(
            app,
            &format!("/api/refdata/{categories}/items"),
            json!({"code": code, "name": name, "parent": travel}),
        )
        .await;
    }
    id(
        app,
        &format!("/api/refdata/{categories}/items"),
        json!({"code": "MEALS", "name": "Meals"}),
    )
    .await;

    let (status, claims) = post(
        app,
        "/api/tables",
        json!({"base": base, "name": "Expense claims", "primary": "Purpose"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claims}");
    let claims = claims["id"].as_str().unwrap().to_owned();
    let (_, lines) = post(
        app,
        "/api/tables",
        json!({"base": base, "name": "Expense lines", "primary": "Description"}),
    )
    .await;
    let lines = lines["id"].as_str().unwrap().to_owned();

    field(
        app,
        &claims,
        json!({"type": "auto_number", "name": "Claim no", "pattern": "EXP-{YYYY}-{seq:4}"}),
    )
    .await;
    field(
        app,
        &claims,
        json!({"type": "person", "name": "Claimant", "role": "requester"}),
    )
    .await;
    field(app, &lines, json!({"type": "date", "name": "Date"})).await;
    field(
        app,
        &lines,
        json!({"type": "code_list", "name": "Category", "refset": categories}),
    )
    .await;
    let amount = field(
        app,
        &lines,
        json!({"type": "currency", "name": "Amount", "currency": "USD", "required": true}),
    )
    .await;
    let claim_link = field(app, &lines, json!({"type": "link", "name": "Claim", "target": claims, "single": true, "inverseName": "Lines"})).await;
    let (status, _) = call(
        app,
        Method::PUT,
        "/api/settings/expense.manager_limit",
        Some(ALICE),
        Some(json!({"type": "money", "text": "1000 USD"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let total = field(
        app,
        &claims,
        json!({"type": "rollup", "name": "Total", "formula": "SUM({Lines}.{Amount})"}),
    )
    .await;
    let needs_finance = field(app, &claims, json!({"type": "formula", "name": "Needs finance", "formula": "{Total} > PARAM(\"expense.manager_limit\")"})).await;
    let lines_link = columns_id(app, &claims, "Lines").await;
    Expenses {
        base,
        claims,
        lines,
        categories,
        amount,
        total,
        needs_finance,
        claim_link,
        lines_link,
    }
}

async fn columns_id(app: &Arc<App>, table: &str, name: &str) -> String {
    let (_, value) = call(
        app,
        Method::GET,
        &format!("/api/tables/{table}"),
        None,
        None,
    )
    .await;
    value["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no column {name}"))["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn std_an_empty_workspace_has_one_administrator_and_a_template_gallery() {
    let app = empty();
    let (_, bases) = call(&app, Method::GET, "/api/bases", None, None).await;
    assert_eq!(bases["bases"], json!([]));
    assert_eq!(bases["canCreate"], true);
    let (_, people) = call(&app, Method::GET, "/api/people", None, None).await;
    assert_eq!(people.as_array().unwrap().len(), 1);
    assert_eq!(people[0]["name"], "Alice Admin");
    let (_, templates) = call(&app, Method::GET, "/api/templates", None, None).await;
    assert_eq!(templates[0]["key"], "purchasing");
    assert_eq!(templates[0]["installed"], false);
    let (_, sets) = call(&app, Method::GET, "/api/refdata", None, None).await;
    assert!(
        sets.as_array().unwrap().len() >= 2,
        "ISO countries and currencies are built in"
    );
}

#[tokio::test]
async fn std_the_expense_configuration_is_built_and_works() {
    let app = empty();
    let x = build(&app).await;
    assert_eq!(
        columns(&app, &x.claims).await,
        [
            "Purpose",
            "Claim no",
            "Claimant",
            "Lines",
            "Total",
            "Needs finance"
        ]
    );
    assert_eq!(
        columns(&app, &x.lines).await,
        ["Description", "Date", "Category", "Amount", "Claim"]
    );
    let (_, bases) = call(&app, Method::GET, "/api/bases", None, None).await;
    let tables: Vec<&str> = bases["bases"][0]["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(tables, ["Expense claims", "Expense lines"]);
    assert_eq!(bases["bases"][0]["id"], x.base.as_str());

    // Use it: a claim with three lines.
    let (status, claim) = post(&app, &format!("/api/tables/{}/records", x.claims), json!({"cells": {columns_id(&app, &x.claims, "Purpose").await: {"text": "Client visit Berlin"}}})).await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    let claim_id = claim["id"].as_str().unwrap().to_owned();
    let (_, options) = call(
        &app,
        Method::GET,
        &format!(
            "/api/tables/{}/fields/{}/options",
            x.lines,
            columns_id(&app, &x.lines, "Category").await
        ),
        None,
        None,
    )
    .await;
    let labels: Vec<&str> = options
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["Travel", "Flights", "Hotels", "Meals"]);
    let flights = options
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["label"] == "Flights")
        .unwrap()["value"]
        .clone();
    let description = columns_id(&app, &x.lines, "Description").await;
    let category = columns_id(&app, &x.lines, "Category").await;
    for (text, amount) in [
        ("Flight to Berlin", "420.00"),
        ("Hotel, two nights", "380"),
        ("Client dinner", "95.50 USD"),
    ] {
        let (status, body) = post(
            &app,
            &format!("/api/tables/{}/records", x.lines),
            json!({"cells": {
                &description: {"text": text},
                &x.amount: {"text": amount},
                &category: {"value": flights},
                &x.claim_link: {"value": {"type": "link", "value": [claim_id]}},
            }}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let claims = rows(&app, &x.claims).await;
    assert_eq!(cell(&claims, "Client visit Berlin", &x.total), "895.50 USD");
    assert_eq!(cell(&claims, "Client visit Berlin", &x.needs_finance), "No");
    assert_eq!(
        cell(
            &claims,
            "Client visit Berlin",
            &columns_id(&app, &x.claims, "Claimant").await
        ),
        "Alice Admin"
    );
    assert!(cell(
        &claims,
        "Client visit Berlin",
        &columns_id(&app, &x.claims, "Claim no").await
    )
    .starts_with("EXP-2026-0001"));
    assert_eq!(
        cell(&claims, "Client visit Berlin", &x.lines_link),
        "Flight to Berlin, Hotel, two nights, Client dinner",
        "the two-way link is mirrored"
    );
    let row = claims["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["title"] == "Client visit Berlin")
        .unwrap();
    assert_eq!(
        row["cells"][&x.lines_link]["items"],
        json!(["Flight to Berlin", "Hotel, two nights", "Client dinner"]),
        "titles are listed one by one: a title may contain a comma"
    );
    // A required amount is required; a lower limit recomputes the formula.
    let (status, _) = post(
        &app,
        &format!("/api/tables/{}/records", x.lines),
        json!({"cells": {&description: {"text": "Taxi"}}}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    call(
        &app,
        Method::PUT,
        "/api/settings/expense.manager_limit",
        Some(ALICE),
        Some(json!({"type": "money", "text": "500 USD"})),
    )
    .await;
    let claims = rows(&app, &x.claims).await;
    assert_eq!(
        cell(&claims, "Client visit Berlin", &x.needs_finance),
        "Yes"
    );
    let (_, settings) = call(&app, Method::GET, "/api/settings", None, None).await;
    assert_eq!(settings[0]["key"], "expense.manager_limit");
    assert_eq!(settings[0]["value"], "500.00 USD");
    // Everything that was built is in the audit chain.
    let (_, entries) = call(&app, Method::GET, "/api/audit", None, None).await;
    let actions: std::collections::BTreeSet<&str> = entries
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    for action in [
        "base.create",
        "refset.create",
        "refset.item.add",
        "table.create",
        "field.create",
        "setting.update",
        "record.create",
    ] {
        assert!(actions.contains(action), "{action}");
    }
    let (_, verdict) = call(&app, Method::GET, "/api/audit/verify", None, None).await;
    assert_eq!(verdict["intact"], true);
    let _ = (&x.categories, &x.lines);
}

#[tokio::test]
async fn std_fields_are_edited_and_deleted_safely() {
    let app = empty();
    let x = build(&app).await;
    let purpose = columns_id(&app, &x.claims, "Purpose").await;
    // Rename, require, help.
    let (status, table) = call(
        &app,
        Method::PATCH,
        &format!("/api/tables/{}/fields/{purpose}", x.claims),
        Some(ALICE),
        Some(json!({"name": "Trip purpose", "help": "Why the money was spent"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{table}");
    assert_eq!(table["columns"][0]["name"], "Trip purpose");
    assert_eq!(table["columns"][0]["help"], "Why the money was spent");
    // Names stay unique; formulas keep working after a rename (they store ids).
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("/api/tables/{}/fields/{}", x.lines, x.amount),
        Some(ALICE),
        Some(json!({"name": "date"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    call(
        &app,
        Method::PATCH,
        &format!("/api/tables/{}/fields/{}", x.lines, x.amount),
        Some(ALICE),
        Some(json!({"name": "Amount (USD)"})),
    )
    .await;
    let (_, check) = post(
        &app,
        &format!("/api/tables/{}/formula", x.claims),
        json!({"text": "SUM({Lines}.{Amount (USD)})"}),
    )
    .await;
    assert_eq!(check["ok"], true);
    // Deleting what others use is refused, with the users named.
    let (status, body) = call(
        &app,
        Method::DELETE,
        &format!("/api/tables/{}/fields/{}", x.lines, x.amount),
        Some(ALICE),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        body["message"],
        "“Amount (USD)” is used by the formula “Total” in “Expense claims”"
    );
    let (status, body) = call(
        &app,
        Method::DELETE,
        &format!("/api/tables/{}/fields/{}", x.claims, x.total),
        Some(ALICE),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("“Needs finance”"));
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/tables/{}/fields/{purpose}", x.claims),
        Some(ALICE),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "the primary field stays");
    // In order, they go: the formula, the roll-up, then the link with its mirror.
    for (table, field) in [
        (&x.claims, &x.needs_finance),
        (&x.claims, &x.total),
        (&x.lines, &x.claim_link),
    ] {
        let (status, body) = call(
            &app,
            Method::DELETE,
            &format!("/api/tables/{table}/fields/{field}"),
            Some(ALICE),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    assert_eq!(
        columns(&app, &x.claims).await,
        ["Trip purpose", "Claim no", "Claimant"],
        "the mirror went with the link"
    );
    // Choices of a select field: renamed, added, removed (disabled, values kept).
    let status_field = field(
        &app,
        &x.claims,
        json!({"type": "single_select", "name": "Kind", "options": ["Travel", "Office"]}),
    )
    .await;
    let (_, described) = call(
        &app,
        Method::GET,
        &format!("/api/tables/{}/fields/{status_field}/options", x.claims),
        None,
        None,
    )
    .await;
    let travel = described[0]["value"]["value"].as_str().unwrap().to_owned();
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("/api/tables/{}/fields/{status_field}", x.claims),
        Some(ALICE),
        Some(
            json!({"options": [{"id": travel, "label": "Travel & trips"}, {"label": "Training"}]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, described) = call(
        &app,
        Method::GET,
        &format!("/api/tables/{}/fields/{status_field}/options", x.claims),
        None,
        None,
    )
    .await;
    let labels: Vec<&str> = described
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["label"].as_str().unwrap())
        .collect();
    assert_eq!(
        labels,
        ["Travel & trips", "Training"],
        "Office is disabled, not offered"
    );
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("/api/tables/{}/fields/{status_field}", x.claims),
        Some(ALICE),
        Some(json!({"options": []})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!(
            "/api/tables/{}/fields/{}",
            x.claims,
            columns_id(&app, &x.claims, "Claim no").await
        ),
        Some(ALICE),
        Some(json!({"required": true})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "generated fields fill themselves"
    );
}

#[tokio::test]
async fn std_bases_tables_and_code_lists_are_managed() {
    let app = empty();
    let x = build(&app).await;
    // Rename and describe.
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("/api/bases/{}", x.base),
        Some(ALICE),
        Some(json!({"name": "Travel & expenses"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, table) = call(
        &app,
        Method::PATCH,
        &format!("/api/tables/{}", x.lines),
        Some(ALICE),
        Some(json!({"description": "One line per receipt"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(table["description"], "One line per receipt");
    // A base with tables is not deleted; a table is (its mirror links go with it).
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/bases/{}", x.base),
        Some(ALICE),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, body) = call(
        &app,
        Method::DELETE,
        &format!("/api/tables/{}", x.lines),
        Some(ALICE),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "the claims' Total reads its lines: {body}"
    );
    assert!(body["message"].as_str().unwrap().contains("“Total”"));
    let scratch = id(
        &app,
        "/api/tables",
        json!({"base": x.base, "name": "Scratch"}),
    )
    .await;
    assert_eq!(
        columns(&app, &scratch).await,
        ["Name"],
        "a primary field by default"
    );
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/tables/{scratch}"),
        Some(ALICE),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let other = id(&app, "/api/bases", json!({"name": "Spare"})).await;
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/bases/{other}"),
        Some(ALICE),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // Code list items are disabled, not deleted.
    let (_, set) = call(
        &app,
        Method::GET,
        &format!("/api/refdata/{}", x.categories),
        None,
        None,
    )
    .await;
    let meals = set["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["code"] == "MEALS")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("/api/refdata/{}/items/{meals}", x.categories),
        Some(ALICE),
        Some(json!({"enabled": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, set) = call(
        &app,
        Method::GET,
        &format!("/api/refdata/{}", x.categories),
        None,
        None,
    )
    .await;
    assert_eq!(
        set["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["code"] == "MEALS")
            .unwrap()["active"],
        false
    );
}

#[tokio::test]
async fn std_people_groups_and_templates() {
    let app = empty();
    let mia = id(
        &app,
        "/api/people",
        json!({"name": "Mia Miller", "kind": "member", "title": "Sales"}),
    )
    .await;
    let max = id(
        &app,
        "/api/people",
        json!({"name": "Max Manager", "kind": "member", "title": "Head of sales"}),
    )
    .await;
    let team = id(
        &app,
        "/api/people",
        json!({"name": "Sales team", "kind": "group", "members": [mia]}),
    )
    .await;
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("/api/people/{team}"),
        Some(ALICE),
        Some(json!({"members": [mia, max]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, people) = call(&app, Method::GET, "/api/people", None, None).await;
    let group = people
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "Sales team")
        .unwrap();
    assert_eq!(group["members"], json!(["Mia Miller", "Max Manager"]));
    // Refusals: members of a person, a group inside itself, unknown members, no name.
    let (status, _) = post(
        &app,
        "/api/people",
        json!({"name": "Odd", "kind": "member", "members": [mia]}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("/api/people/{team}"),
        Some(ALICE),
        Some(json!({"members": [team]})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("/api/people/{mia}"),
        Some(ALICE),
        Some(json!({"members": [max]})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = post(&app, "/api/people", json!({"name": " ", "kind": "member"})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    // Mia has no rights yet: she builds nothing.
    for (path, body) in [
        ("/api/bases", json!({"name": "Mine"})),
        ("/api/refdata", json!({"code": "x", "name": "X"})),
        ("/api/people", json!({"name": "Eve", "kind": "member"})),
        ("/api/templates/purchasing/install", json!({})),
    ] {
        let (status, _) = call(&app, Method::POST, path, Some(&mia), Some(body)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
    }
    let (status, _) = call(
        &app,
        Method::PUT,
        "/api/settings/x.y",
        Some(&mia),
        Some(json!({"type": "number", "text": "1"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // The purchasing sample installs once, next to what was built.
    let (status, _) = post(&app, "/api/templates/purchasing/install", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post(&app, "/api/templates/purchasing/install", json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = post(&app, "/api/templates/crm/install", json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, bases) = call(&app, Method::GET, "/api/bases", None, None).await;
    assert_eq!(bases["bases"][0]["name"], "Purchasing");
    assert_eq!(bases["bases"][0]["tables"].as_array().unwrap().len(), 3);
    let (_, people) = call(&app, Method::GET, "/api/people", None, None).await;
    assert!(
        people
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "Mia Miller"),
        "nobody is lost"
    );
}

#[tokio::test]
async fn invalid_building_blocks_are_refused() {
    let app = empty();
    let x = build(&app).await;
    let cases: Vec<(&str, String, Value, StatusCode)> = vec![
        (
            "empty base name",
            "/api/bases".to_owned(),
            json!({"name": "  "}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "duplicate base",
            "/api/bases".to_owned(),
            json!({"name": "expenses"}),
            StatusCode::CONFLICT,
        ),
        (
            "long base name",
            "/api/bases".to_owned(),
            json!({"name": "x".repeat(81)}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "unknown base",
            "/api/tables".to_owned(),
            json!({"base": "bas_0000000000000000000000000z", "name": "T"}),
            StatusCode::NOT_FOUND,
        ),
        (
            "duplicate table",
            "/api/tables".to_owned(),
            json!({"base": x.base, "name": "Expense Claims"}),
            StatusCode::CONFLICT,
        ),
        (
            "bad code list code",
            "/api/refdata".to_owned(),
            json!({"code": "has space", "name": "X"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "duplicate code list",
            "/api/refdata".to_owned(),
            json!({"code": "EXPENSE_CATEGORY", "name": "X"}),
            StatusCode::CONFLICT,
        ),
        (
            "duplicate item",
            format!("/api/refdata/{}/items", x.categories),
            json!({"code": "meals", "name": "Meals again"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "unknown parent",
            format!("/api/refdata/{}/items", x.categories),
            json!({"code": "X", "name": "X", "parent": "itm_0000000000000000000000000z"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "duplicate field",
            format!("/api/tables/{}/fields", x.claims),
            json!({"type": "text", "name": "total"}),
            StatusCode::CONFLICT,
        ),
        (
            "link to nowhere",
            format!("/api/tables/{}/fields", x.claims),
            json!({"type": "link", "name": "X", "target": "tbl_0000000000000000000000000z"}),
            StatusCode::NOT_FOUND,
        ),
        (
            "mirror name in use",
            format!("/api/tables/{}/fields", x.lines),
            json!({"type": "link", "name": "Claim 2", "target": x.claims, "inverseName": "Total"}),
            StatusCode::CONFLICT,
        ),
        (
            "unknown role",
            format!("/api/tables/{}/fields", x.claims),
            json!({"type": "person", "name": "X", "role": "boss"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "unknown currency",
            format!("/api/tables/{}/fields", x.claims),
            json!({"type": "currency", "name": "X", "currency": "XYZ"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ];
    for (what, path, body, expected) in cases {
        let (status, answer) = post(&app, &path, body).await;
        assert_eq!(status, expected, "{what}: {answer}");
    }
    // A refused mirror leaves nothing behind.
    assert!(!columns(&app, &x.lines)
        .await
        .contains(&"Claim 2".to_owned()));
    let (status, _) = call(
        &app,
        Method::PUT,
        "/api/settings/Bad-Key",
        Some(ALICE),
        Some(json!({"type": "number", "text": "1"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = call(
        &app,
        Method::PUT,
        "/api/settings/x.limit",
        Some(ALICE),
        Some(json!({"type": "money", "text": "lots"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = call(
        &app,
        Method::PUT,
        "/api/settings/x.limit",
        Some(ALICE),
        Some(json!({"type": "colour", "text": "red"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = call(
        &app,
        Method::PUT,
        "/api/settings/x.limit",
        Some(ALICE),
        Some(json!({"type": "text", "text": ""})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = call(
        &app,
        Method::PATCH,
        "/api/bases/bas_0000000000000000000000000z",
        Some(ALICE),
        Some(json!({"name": "X"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("/api/tables/{}/fields/{}", x.claims, x.total),
        Some(ALICE),
        Some(json!({"options": [{"label": "A"}]})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "only select fields have choices"
    );
}

#[tokio::test]
async fn std_a_built_workspace_survives_a_restart() {
    let dir = std::env::temp_dir().join(format!("agenttable-builder-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("data.json");
    let app = mt_server::open(
        Some(file.clone()),
        Clock::Fixed(now()),
        mt_server::Start::Empty,
    )
    .unwrap();
    let x = build(&app).await;
    let reopened = mt_server::open(
        Some(file.clone()),
        Clock::Fixed(now()),
        mt_server::Start::Empty,
    )
    .unwrap();
    assert_eq!(
        columns(&reopened, &x.claims).await,
        [
            "Purpose",
            "Claim no",
            "Claimant",
            "Lines",
            "Total",
            "Needs finance"
        ]
    );
    let (_, bases) = call(&reopened, Method::GET, "/api/bases", None, None).await;
    assert_eq!(bases["bases"][0]["name"], "Expenses");
    std::fs::remove_dir_all(&dir).unwrap();
}

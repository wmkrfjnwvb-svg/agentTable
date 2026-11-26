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

// ---- designing the process and giving access ----

struct People {
    mia: String,
    max: String,
    fiona: String,
    finance: String,
}

async fn people(app: &Arc<App>) -> People {
    let mia = id(
        app,
        "/api/people",
        json!({"name": "Mia Miller", "kind": "member"}),
    )
    .await;
    let max = id(
        app,
        "/api/people",
        json!({"name": "Max Manager", "kind": "member"}),
    )
    .await;
    let fiona = id(
        app,
        "/api/people",
        json!({"name": "Fiona Finance", "kind": "member"}),
    )
    .await;
    let finance = id(
        app,
        "/api/people",
        json!({"name": "Finance team", "kind": "group", "members": [fiona]}),
    )
    .await;
    People {
        mia,
        max,
        fiona,
        finance,
    }
}

fn state(key: &str, name: &str, category: &str) -> Value {
    json!({"key": key, "name": name, "category": category})
}

fn design(x: &Expenses, p: &People) -> Value {
    json!({
        "name": "Expense claim",
        "states": [
            state("draft", "Draft", "new"),
            {"key": "submitted", "name": "Submitted", "category": "pending_approval", "locked": true, "required": [x.lines_link]},
            {"key": "approved", "name": "Approved", "category": "in_progress", "locked": true},
            state("paid", "Paid", "done_success"),
            state("rejected", "Rejected", "done_failure"),
        ],
        "initial": "draft",
        "transitions": [
            {"key": "submit", "name": "Submit", "from": ["draft"], "to": "submitted"},
            {"key": "withdraw", "name": "Withdraw", "from": ["submitted"], "to": "draft"},
            {"key": "pay", "name": "Pay", "from": ["approved"], "to": "paid", "capability": "expense.pay"},
            {"key": "revise", "name": "Revise", "from": ["rejected"], "to": "draft"},
        ],
        "approval": {
            "name": "Expense approval",
            "startedBy": "submit",
            "approved": "approved",
            "rejected": "rejected",
            "returned": "draft",
            "steps": [
                {"kind": "approval", "name": "Manager", "approvers": [{"kind": "person", "id": p.max}], "mode": {"mode": "any"}, "canSendBack": true},
                {"kind": "approval", "name": "Finance review", "approvers": [{"kind": "capability", "capability": "expense.finance_review"}], "mode": {"mode": "any"}, "when": {"field": x.needs_finance, "op": "eq", "text": "yes"}},
                {"kind": "notify", "name": "Ready to pay", "to": [{"kind": "capability", "capability": "expense.pay"}]},
            ],
        },
    })
}

async fn grant(app: &Arc<App>, body: Value) -> (StatusCode, Value) {
    post(app, "/api/access/grants", body).await
}

async fn task(app: &Arc<App>, who: &str, title: &str) -> String {
    let (_, tasks) = call(app, Method::GET, "/api/tasks", Some(who), None).await;
    let task = tasks
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["title"] == title)
        .unwrap_or_else(|| panic!("{who} has no task for {title}: {tasks}"));
    format!("/api/approvals/{}/tasks/{}", task["run"], task["task"])
}

#[tokio::test]
async fn std_a_designed_process_runs_with_the_access_that_was_given() {
    let app = empty();
    let x = build(&app).await;
    let p = people(&app).await;
    // Design and publish the process.
    let (status, published) = call(
        &app,
        Method::PUT,
        &format!("/api/tables/{}/process", x.claims),
        Some(ALICE),
        Some(design(&x, &p)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{published}");
    let (_, info) = call(
        &app,
        Method::GET,
        &format!("/api/tables/{}/process", x.claims),
        None,
        None,
    )
    .await;
    assert_eq!(info["published"]["version"], 1);
    assert_eq!(
        info["design"]["approval"]["steps"][1]["when"]["text"],
        "yes"
    );
    assert!(
        columns(&app, &x.claims)
            .await
            .contains(&"Status".to_owned()),
        "a state field was added"
    );
    // Give access: employees their own claims; the manager reads all; finance reviews and pays.
    let (status, set) = post(&app, "/api/access/sets", json!({"name": "Employee", "capabilities": ["record.read", "record.create", "record.update"]})).await;
    assert_eq!(status, StatusCode::OK, "{set}");
    let employee = set["id"].as_str().unwrap().to_owned();
    for body in [
        json!({"to": p.mia, "set": employee, "on": {"kind": "base", "id": x.base}, "scope": "own", "reason": "Submits expenses"}),
        json!({"to": p.max, "capability": "record.read", "on": {"kind": "base", "id": x.base}, "reason": "Approves the team's expenses"}),
        json!({"to": p.finance, "capability": "record.read", "on": {"kind": "base", "id": x.base}}),
        json!({"to": p.finance, "capability": "expense.finance_review", "on": {"kind": "table", "id": x.claims}}),
        json!({"to": p.finance, "capability": "expense.pay", "on": {"kind": "table", "id": x.claims}, "limit": {"field": x.total, "op": "le", "text": "10000 USD"}}),
    ] {
        let (status, answer) = grant(&app, body).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
    }
    let (_, access) = call(&app, Method::GET, "/api/access", None, None).await;
    let limits: Vec<&Value> = access["grants"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|g| g["limit"].is_string())
        .collect();
    assert_eq!(limits[0]["limit"], "“Total” ≤ 10,000.00 USD");
    assert!(access["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == "expense.finance_review"));

    // Mia: a claim of 1,250 USD (above the 1,000 limit: finance reviews).
    let purpose = columns_id(&app, &x.claims, "Purpose").await;
    let (status, claim) = call(
        &app,
        Method::POST,
        &format!("/api/tables/{}/records", x.claims),
        Some(&p.mia),
        Some(json!({"cells": {&purpose: {"text": "Trade fair Paris"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    assert_eq!(claim["process"]["state"]["name"], "Draft");
    let claim_id = claim["id"].as_str().unwrap().to_owned();
    let description = columns_id(&app, &x.lines, "Description").await;
    let mut line_ids = Vec::new();
    for (text, amount) in [("Stand", "600"), ("Hotel", "650")] {
        let (status, line) = call(&app, Method::POST, &format!("/api/tables/{}/records", x.lines), Some(&p.mia), Some(json!({"cells": {
            &description: {"text": text}, &x.amount: {"text": amount}, &x.claim_link: {"value": {"type": "link", "value": [claim_id]}}
        }}))).await;
        assert_eq!(status, StatusCode::OK, "{line}");
        line_ids.push(line["id"].as_str().unwrap().to_owned());
    }
    // Mia sees her claim only.
    let (_, seen) = call(
        &app,
        Method::POST,
        &format!("/api/tables/{}/query", x.claims),
        Some(&p.mia),
        Some(json!({})),
    )
    .await;
    assert_eq!(seen["count"], 1);
    let path = format!("/api/tables/{}/records/{claim_id}", x.claims);
    let (_, mine) = call(&app, Method::GET, &path, Some(&p.mia), None).await;
    assert_eq!(
        mine["process"]["wouldAsk"]["steps"][0]["people"],
        json!(["Max Manager"])
    );
    assert_eq!(
        mine["process"]["wouldAsk"]["steps"][1]["people"],
        json!(["Fiona Finance"])
    );
    let (status, submitted) = call(
        &app,
        Method::POST,
        &format!("{path}/transitions/submit"),
        Some(&p.mia),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{submitted}");
    assert_eq!(submitted["process"]["state"]["name"], "Submitted");
    // Locked while submitted, its lines too.
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("/api/tables/{}/records/{}", x.lines, line_ids[0]),
        Some(&p.mia),
        Some(json!({"cells": {&x.amount: {"text": "700"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    // Max approves; Fiona reviews; Fiona pays.
    let (status, after) = call(
        &app,
        Method::POST,
        &task(&app, &p.max, "Trade fair Paris").await,
        Some(&p.max),
        Some(json!({"act": "approve"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(
        after["process"]["state"]["name"], "Submitted",
        "finance still reviews"
    );
    let (status, after) = call(
        &app,
        Method::POST,
        &task(&app, &p.fiona, "Trade fair Paris").await,
        Some(&p.fiona),
        Some(json!({"act": "approve"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(after["process"]["state"]["name"], "Approved");
    let (_, inbox) = call(&app, Method::GET, "/api/inbox", Some(&p.fiona), None).await;
    assert!(
        inbox
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["text"] == "“Trade fair Paris”: Ready to pay"),
        "{inbox}"
    );
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("{path}/transitions/pay"),
        Some(&p.max),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "Max may not pay");
    let (status, paid) = call(
        &app,
        Method::POST,
        &format!("{path}/transitions/pay"),
        Some(&p.fiona),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{paid}");
    assert_eq!(paid["process"]["state"]["name"], "Paid");
    let (_, inbox) = call(&app, Method::GET, "/api/inbox", Some(&p.mia), None).await;
    assert_eq!(inbox[0]["text"], "“Trade fair Paris” was approved");
}

#[tokio::test]
async fn invalid_designs_are_refused_with_the_reason() {
    let app = empty();
    let x = build(&app).await;
    let p = people(&app).await;
    let put = |body: Value| {
        let app = app.clone();
        let path = format!("/api/tables/{}/process", x.claims);
        async move { call(&app, Method::PUT, &path, Some(ALICE), Some(body)).await }
    };
    let mut d = design(&x, &p);
    d["states"] = json!([
        state("draft", "Draft", "new"),
        state("open", "Open", "in_progress")
    ]);
    d["transitions"] = json!([{"key": "go", "name": "Go", "from": ["draft"], "to": "open"}]);
    d["approval"] = json!(null);
    let (status, body) = put(d).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body["message"].as_str().unwrap().contains("final state"),
        "{body}"
    );
    let mut d = design(&x, &p);
    d["transitions"]
        .as_array_mut()
        .unwrap()
        .retain(|t| t["key"] != "revise");
    d["states"]
        .as_array_mut()
        .unwrap()
        .push(state("on_hold", "On hold", "waiting_internal"));
    let (status, body) = put(d).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("never be reached"),
        "{body}"
    );
    let mut d = design(&x, &p);
    d["states"][1]["category"] = json!("in_progress");
    let (status, body) = put(d).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("pending approval"),
        "{body}"
    );
    let mut d = design(&x, &p);
    d["approval"]["steps"][0]["approvers"] = json!([]);
    let (status, body) = put(d).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body["message"].as_str().unwrap().contains("no approver"),
        "{body}"
    );
    let mut d = design(&x, &p);
    d["approval"]["steps"][1]["when"]["text"] = json!("lots");
    let (status, _) = put(d).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let mut d = design(&x, &p);
    d["approval"]["approved"] = json!("nowhere");
    let (status, _) = put(d).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = call(
        &app,
        Method::PUT,
        &format!("/api/tables/{}/process", x.claims),
        Some(&p.mia),
        Some(design(&x, &p)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Mia does not design processes"
    );
    // Published; a record in Paid; the design may not drop Paid then.
    let (status, _) = put(design(&x, &p)).await;
    assert_eq!(status, StatusCode::OK);
    let mut d = design(&x, &p);
    d["states"]
        .as_array_mut()
        .unwrap()
        .retain(|s| s["key"] != "rejected");
    d["transitions"]
        .as_array_mut()
        .unwrap()
        .retain(|t| t["key"] != "revise");
    d["approval"]["rejected"] = json!("draft");
    let (status, body) = put(d).await;
    assert_eq!(status, StatusCode::OK, "nobody is in Rejected: {body}");
}

#[tokio::test]
async fn std_running_approvals_keep_their_version() {
    let app = empty();
    let x = build(&app).await;
    let p = people(&app).await;
    let path = format!("/api/tables/{}/process", x.claims);
    call(&app, Method::PUT, &path, Some(ALICE), Some(design(&x, &p))).await;
    // Alice submits her claim (she created it; Max is asked).
    grant(
        &app,
        json!({"to": p.max, "capability": "record.read", "on": {"kind": "base", "id": x.base}}),
    )
    .await;
    let purpose = columns_id(&app, &x.claims, "Purpose").await;
    let (_, claim) = post(
        &app,
        &format!("/api/tables/{}/records", x.claims),
        json!({"cells": {&purpose: {"text": "Taxi"}}}),
    )
    .await;
    let claim_id = claim["id"].as_str().unwrap().to_owned();
    let description = columns_id(&app, &x.lines, "Description").await;
    post(&app, &format!("/api/tables/{}/records", x.lines), json!({"cells": {&description: {"text": "Ride"}, &x.amount: {"text": "30"}, &x.claim_link: {"value": {"type": "link", "value": [claim_id]}}}})).await;
    let (status, body) = post(
        &app,
        &format!(
            "/api/tables/{}/records/{claim_id}/transitions/submit",
            x.claims
        ),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The same design again: no new version. A changed step: version 2.
    call(&app, Method::PUT, &path, Some(ALICE), Some(design(&x, &p))).await;
    let (_, info) = call(&app, Method::GET, &path, None, None).await;
    assert_eq!(info["published"]["version"], 1);
    assert_eq!(info["running"], 1);
    let mut d = design(&x, &p);
    d["approval"]["steps"][0]["name"] = json!("Line manager");
    call(&app, Method::PUT, &path, Some(ALICE), Some(d)).await;
    let (_, info) = call(&app, Method::GET, &path, None, None).await;
    assert_eq!(info["published"]["version"], 2);
    // The running approval still runs on version 1 and finishes.
    let (status, after) = call(
        &app,
        Method::POST,
        &task(&app, &p.max, "Taxi").await,
        Some(&p.max),
        Some(json!({"act": "approve"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(after["process"]["approval"]["version"], 1);
    assert_eq!(after["process"]["state"]["name"], "Approved");
}

#[tokio::test]
async fn std_access_is_given_revoked_and_kept_apart() {
    let app = empty();
    let x = build(&app).await;
    let p = people(&app).await;
    // Separation of duties: who reviews does not pay.
    let (status, body) = post(&app, "/api/access/rules", json!({"name": "Reviewers do not pay", "first": "expense.finance_review", "second": "expense.pay", "kind": "hold"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _) = grant(&app, json!({"to": p.fiona, "capability": "expense.finance_review", "on": {"kind": "table", "id": x.claims}})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = grant(&app, json!({"to": p.finance, "capability": "expense.pay", "on": {"kind": "table", "id": x.claims}})).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "Fiona is in the finance team: {body}"
    );
    // Revoked access is gone at once, and stays on record.
    let (_, given) = grant(
        &app,
        json!({"to": p.max, "capability": "record.read", "on": {"kind": "table", "id": x.claims}}),
    )
    .await;
    let grant_id = given["id"].as_str().unwrap().to_owned();
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/tables/{}/query", x.claims),
        Some(&p.max),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/access/grants/{grant_id}"),
        Some(ALICE),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/tables/{}/query", x.claims),
        Some(&p.max),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/access/grants/{grant_id}"),
        Some(ALICE),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "revoked already");
    let (_, access) = call(&app, Method::GET, "/api/access", None, None).await;
    assert!(access["grants"]
        .as_array()
        .unwrap()
        .iter()
        .any(|g| g["id"] == grant_id.as_str() && g["active"] == false));
    // Refusals.
    for body in [
        json!({"to": p.max, "on": {"kind": "workspace"}}),
        json!({"to": p.max, "capability": "Not a key", "on": {"kind": "workspace"}}),
        json!({"to": p.max, "capability": "record.read", "on": {"kind": "base", "id": x.base}, "scope": "everything"}),
        json!({"to": p.max, "capability": "record.read", "on": {"kind": "base", "id": x.base}, "limit": {"field": x.total, "op": "le", "text": "1 USD"}}),
    ] {
        let (status, answer) = grant(&app, body).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    }
    let (status, _) = grant(&app, json!({"to": p.max, "capability": "record.read", "on": {"kind": "table", "id": "tbl_0000000000000000000000000z"}})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/access/grants",
        Some(&p.max),
        Some(json!({"to": p.max, "capability": "access.manage", "on": {"kind": "workspace"}})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "nobody gives themselves access"
    );
    let (status, _) = post(
        &app,
        "/api/access/sets",
        json!({"name": "Empty", "capabilities": []}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = post(
        &app,
        "/api/access/sets",
        json!({"name": " ", "capabilities": ["record.read"]}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = call(
        &app,
        Method::DELETE,
        "/api/access/grants/grt_0000000000000000000000000z",
        Some(ALICE),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn std_a_withdrawn_right_no_longer_decides_open_tasks() {
    let app = empty();
    let x = build(&app).await;
    let p = people(&app).await;
    call(
        &app,
        Method::PUT,
        &format!("/api/tables/{}/process", x.claims),
        Some(ALICE),
        Some(design(&x, &p)),
    )
    .await;
    grant(
        &app,
        json!({"to": p.max, "capability": "record.read", "on": {"kind": "base", "id": x.base}}),
    )
    .await;
    grant(
        &app,
        json!({"to": p.finance, "capability": "record.read", "on": {"kind": "base", "id": x.base}}),
    )
    .await;
    let (_, review) = grant(&app, json!({"to": p.finance, "capability": "expense.finance_review", "on": {"kind": "table", "id": x.claims}})).await;
    // Alice's claim above the limit: Max, then finance.
    call(
        &app,
        Method::PUT,
        "/api/settings/expense.manager_limit",
        Some(ALICE),
        Some(json!({"type": "money", "text": "10 USD"})),
    )
    .await;
    let purpose = columns_id(&app, &x.claims, "Purpose").await;
    let (_, claim) = post(
        &app,
        &format!("/api/tables/{}/records", x.claims),
        json!({"cells": {&purpose: {"text": "Dinner"}}}),
    )
    .await;
    let claim_id = claim["id"].as_str().unwrap().to_owned();
    let description = columns_id(&app, &x.lines, "Description").await;
    post(&app, &format!("/api/tables/{}/records", x.lines), json!({"cells": {&description: {"text": "Dinner"}, &x.amount: {"text": "80"}, &x.claim_link: {"value": {"type": "link", "value": [claim_id]}}}})).await;
    post(
        &app,
        &format!(
            "/api/tables/{}/records/{claim_id}/transitions/submit",
            x.claims
        ),
        json!({}),
    )
    .await;
    call(
        &app,
        Method::POST,
        &task(&app, &p.max, "Dinner").await,
        Some(&p.max),
        Some(json!({"act": "approve"})),
    )
    .await;
    let fiona_task = task(&app, &p.fiona, "Dinner").await;
    // The review right is revoked while Fiona's task is open.
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/access/grants/{}", review["id"].as_str().unwrap()),
        Some(ALICE),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call(
        &app,
        Method::POST,
        &fiona_task,
        Some(&p.fiona),
        Some(json!({"act": "approve"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("no longer may “expense.finance_review”"));
    // Comments are still possible; the claim waits for an administrator.
    let (status, _) = call(
        &app,
        Method::POST,
        &fiona_task,
        Some(&p.fiona),
        Some(json!({"act": "comment", "text": "I lost the right"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

// ---- more step kinds: quorum, parallel branches, fill-in, timeouts ----

fn app_at(file: &std::path::Path, millis: i64) -> Arc<App> {
    mt_server::open(
        Some(file.to_path_buf()),
        Clock::Fixed(Timestamp::from_millis(millis).unwrap()),
        mt_server::Start::Empty,
    )
    .unwrap()
}

#[tokio::test]
async fn std_quorum_parallel_fill_in_and_timeouts() {
    let dir = std::env::temp_dir().join(format!("agenttable-steps-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("data.json");
    let start = now().millis();
    let app = app_at(&file, start);
    let x = build(&app).await;
    let p = people(&app).await;
    let ola = id(
        &app,
        "/api/people",
        json!({"name": "Ola Officer", "kind": "member"}),
    )
    .await;
    let paid_on = field(&app, &x.claims, json!({"type": "date", "name": "Paid on"})).await;
    for who in [&p.max, &p.fiona, &ola] {
        grant(
            &app,
            json!({"to": who, "capability": "record.read", "on": {"kind": "base", "id": x.base}}),
        )
        .await;
    }
    let mut d = design(&x, &p);
    d["approval"]["steps"] = json!([
        {"kind": "parallel", "name": "Checks", "join": {"join": "all"}, "branches": [
            [{"kind": "approval", "name": "Manager", "approvers": [{"kind": "person", "id": p.max}], "mode": {"mode": "any"}, "timeoutHours": 24, "escalateTo": {"kind": "person", "id": ola}}],
            [{"kind": "approval", "name": "Board", "approvers": [{"kind": "person", "id": p.fiona}, {"kind": "person", "id": ola}, {"kind": "person", "id": p.mia}], "mode": {"mode": "quorum", "required": 2}}],
        ]},
        {"kind": "fill_in", "name": "Payment date", "assignee": {"kind": "person", "id": p.fiona}, "fields": [paid_on]},
    ]);
    let (status, body) = call(
        &app,
        Method::PUT,
        &format!("/api/tables/{}/process", x.claims),
        Some(ALICE),
        Some(d),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, info) = call(
        &app,
        Method::GET,
        &format!("/api/tables/{}/process", x.claims),
        None,
        None,
    )
    .await;
    assert_eq!(
        info["design"]["approval"]["steps"][0]["branches"][1][0]["mode"]["required"], 2,
        "the design round-trips"
    );
    // Alice's claim; both branches start at once.
    let purpose = columns_id(&app, &x.claims, "Purpose").await;
    let (_, claim) = post(
        &app,
        &format!("/api/tables/{}/records", x.claims),
        json!({"cells": {&purpose: {"text": "Conference"}}}),
    )
    .await;
    let claim_id = claim["id"].as_str().unwrap().to_owned();
    let description = columns_id(&app, &x.lines, "Description").await;
    post(&app, &format!("/api/tables/{}/records", x.lines), json!({"cells": {&description: {"text": "Ticket"}, &x.amount: {"text": "300"}, &x.claim_link: {"value": {"type": "link", "value": [claim_id]}}}})).await;
    let (status, submitted) = post(
        &app,
        &format!(
            "/api/tables/{}/records/{claim_id}/transitions/submit",
            x.claims
        ),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{submitted}");
    let open: Vec<&str> = submitted["process"]["approval"]["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["status"] == "open")
        .map(|t| t["assignee"].as_str().unwrap())
        .collect();
    assert_eq!(
        open,
        ["Max Manager", "Fiona Finance", "Ola Officer", "Mia Miller"]
    );
    assert!(submitted["process"]["approval"]["tasks"][0]["dueAt"].is_number());
    // The quorum: Fiona and Ola approve; Mia's vote is no longer needed.
    for who in [&p.fiona, &ola] {
        let (status, _) = call(
            &app,
            Method::POST,
            &task(&app, who, "Conference").await,
            Some(who),
            Some(json!({"act": "approve"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
    let (_, mia_tasks) = call(&app, Method::GET, "/api/tasks", Some(&p.mia), None).await;
    assert!(
        mia_tasks.as_array().unwrap().is_empty(),
        "the quorum was reached"
    );
    // A day passes; the manager's task is escalated to Ola.
    let ticked = app_at(&file, start + 23 * 3_600_000);
    let (_, changed) = call(
        &ticked,
        Method::POST,
        "/api/process/tick",
        Some(ALICE),
        None,
    )
    .await;
    assert_eq!(changed["changed"], 0, "not overdue yet");
    let later = app_at(&file, start + 25 * 3_600_000);
    let (status, _) = call(
        &later,
        Method::POST,
        "/api/process/tick",
        Some(&p.mia),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "only administrators tick by hand"
    );
    let (_, changed) = call(&later, Method::POST, "/api/process/tick", Some(ALICE), None).await;
    assert_eq!(changed["changed"], 1);
    let (_, inbox) = call(&later, Method::GET, "/api/inbox", Some(&ola), None).await;
    assert_eq!(
        inbox[0]["text"],
        "“Conference” is overdue and now yours (from Max Manager)"
    );
    let (_, max_tasks) = call(&later, Method::GET, "/api/tasks", Some(&p.max), None).await;
    assert!(max_tasks.as_array().unwrap().is_empty());
    let (status, _) = call(
        &later,
        Method::POST,
        &task(&later, &ola, "Conference").await,
        Some(&ola),
        Some(json!({"act": "approve"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // Fill-in: Fiona (who may only read) enters the payment date while the claim is locked.
    let fill = task(&later, &p.fiona, "Conference").await;
    let (_, tasks) = call(&later, Method::GET, "/api/tasks", Some(&p.fiona), None).await;
    assert_eq!(tasks[0]["kind"], "fill_in");
    assert_eq!(tasks[0]["fields"][0]["name"], "Paid on");
    let (status, body) = call(
        &later,
        Method::POST,
        &fill,
        Some(&p.fiona),
        Some(json!({"act": "submit"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["message"], "fill in Paid on first");
    let (status, body) = call(
        &later,
        Method::POST,
        &fill,
        Some(&p.fiona),
        Some(json!({"act": "submit", "values": {&purpose: {"text": "Changed"}}})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "only the step's fields: {body}"
    );
    let (status, body) = call(
        &later,
        Method::POST,
        &fill,
        Some(&ola),
        Some(json!({"act": "submit", "values": {&paid_on: {"text": "2026-12-01"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "not Ola's task: {body}");
    let (status, done) = call(
        &later,
        Method::POST,
        &fill,
        Some(&p.fiona),
        Some(json!({"act": "submit", "values": {&paid_on: {"text": "2026-12-01"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["process"]["state"]["name"], "Approved");
    let paid = done["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "Paid on")
        .unwrap();
    assert_eq!(paid["cell"]["d"], "2026-12-01");
    let (_, entries) = call(&later, Method::GET, "/api/audit", Some(ALICE), None).await;
    assert!(entries
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["details"]["task"] == "Payment date"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn invalid_step_designs_are_refused() {
    let app = empty();
    let x = build(&app).await;
    let p = people(&app).await;
    let path = format!("/api/tables/{}/process", x.claims);
    let approver = json!([{"kind": "person", "id": p.max}]);
    for (what, steps) in [
        (
            "zero hours",
            json!([{"kind": "approval", "name": "A", "approvers": approver, "mode": {"mode": "any"}, "timeoutHours": 0}]),
        ),
        (
            "escalation without timeout",
            json!([{"kind": "approval", "name": "A", "approvers": approver, "mode": {"mode": "any"}, "escalateTo": {"kind": "person", "id": p.fiona}}]),
        ),
        (
            "quorum of nobody",
            json!([{"kind": "approval", "name": "A", "approvers": approver, "mode": {"mode": "quorum", "required": 0}}]),
        ),
        (
            "one branch",
            json!([{"kind": "parallel", "name": "P", "join": {"join": "all"}, "branches": [[{"kind": "approval", "name": "A", "approvers": approver, "mode": {"mode": "any"}}]]}]),
        ),
        (
            "fill in a computed field",
            json!([{"kind": "fill_in", "name": "F", "assignee": {"kind": "person", "id": p.fiona}, "fields": [x.total]}]),
        ),
        (
            "fill in nothing",
            json!([{"kind": "fill_in", "name": "F", "assignee": {"kind": "person", "id": p.fiona}, "fields": []}]),
        ),
        (
            "unnamed step",
            json!([{"kind": "notify", "name": " ", "to": approver}]),
        ),
    ] {
        let mut d = design(&x, &p);
        d["approval"]["steps"] = steps;
        let (status, body) = call(&app, Method::PUT, &path, Some(ALICE), Some(d)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{what}: {body}");
    }
}

#[tokio::test]
async fn std_fields_required_only_when_a_condition_holds() {
    let app = empty();
    let x = build(&app).await;
    let p = people(&app).await;
    let receipt = field(&app, &x.claims, json!({"type": "url", "name": "Receipt"})).await;
    let patch = |body: Value| {
        let app = app.clone();
        let path = format!("/api/tables/{}/fields/{receipt}", x.claims);
        async move { call(&app, Method::PATCH, &path, Some(ALICE), Some(body)).await }
    };
    let (status, table) =
        patch(json!({"requiredWhen": {"field": x.total, "op": "gt", "text": "500 USD"}})).await;
    assert_eq!(status, StatusCode::OK, "{table}");
    let column = table["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Receipt")
        .unwrap()
        .clone();
    assert_eq!(
        column["requiredWhen"]["text"], "500 USD",
        "it round-trips for editing"
    );
    assert_eq!(column["requiredWhenText"], "“Total” > 500.00 USD");
    // A claim without lines needs no receipt; with 600 USD of lines it does.
    let purpose = columns_id(&app, &x.claims, "Purpose").await;
    let (status, claim) = post(
        &app,
        &format!("/api/tables/{}/records", x.claims),
        json!({"cells": {&purpose: {"text": "Fair"}}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let claim_id = claim["id"].as_str().unwrap().to_owned();
    let description = columns_id(&app, &x.lines, "Description").await;
    post(&app, &format!("/api/tables/{}/records", x.lines), json!({"cells": {&description: {"text": "Stand"}, &x.amount: {"text": "600"}, &x.claim_link: {"value": {"type": "link", "value": [claim_id]}}}})).await;
    let path = format!("/api/tables/{}/records/{claim_id}", x.claims);
    let (status, body) = call(
        &app,
        Method::PATCH,
        &path,
        Some(ALICE),
        Some(json!({"cells": {&purpose: {"text": "Fair Paris"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["details"][0]["field"], receipt.as_str());
    assert_eq!(
        body["details"][0]["message"],
        "`Receipt` is required for this record"
    );
    // Moving forward needs it too.
    call(
        &app,
        Method::PUT,
        &format!("/api/tables/{}/process", x.claims),
        Some(ALICE),
        Some(design(&x, &p)),
    )
    .await;
    let (status, body) = call(
        &app,
        Method::POST,
        &format!("{path}/transitions/submit"),
        Some(ALICE),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["details"][0], "“Receipt” is required for this record");
    let (_, record) = call(&app, Method::GET, &path, Some(ALICE), None).await;
    let submit = record["process"]["transitions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["key"] == "submit")
        .unwrap()
        .clone();
    assert_eq!(submit["allowed"], false);
    let (status, _) = call(&app, Method::PATCH, &path, Some(ALICE), Some(json!({"cells": {&purpose: {"text": "Fair Paris"}, &receipt: {"text": "https://receipts.example/fair"}}}))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = call(
        &app,
        Method::POST,
        &format!("{path}/transitions/submit"),
        Some(ALICE),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Removing the condition; refusals.
    let (status, table) = patch(json!({"requiredWhen": null})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(table["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Receipt")
        .unwrap()["requiredWhen"]
        .is_null());
    let (status, _) =
        patch(json!({"requiredWhen": {"field": receipt, "op": "is_empty", "text": ""}})).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "not by its own value"
    );
    let (status, _) = patch(json!({"requiredWhen": {"field": "fld_0000000000000000000000000z", "op": "eq", "text": "1"}})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) =
        patch(json!({"requiredWhen": {"field": x.total, "op": "gt", "text": "lots"}})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("/api/tables/{}/fields/{}", x.claims, x.total),
        Some(ALICE),
        Some(json!({"requiredWhen": {"field": receipt, "op": "is_not_empty", "text": ""}})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "computed fields fill themselves"
    );
}

async fn query_as(app: &Arc<App>, table: &str, who: &str) -> Value {
    call(
        app,
        Method::POST,
        &format!("/api/tables/{table}/query"),
        Some(who),
        Some(json!({})),
    )
    .await
    .1
}

async fn access_as(app: &Arc<App>, who: &str) -> Value {
    call(app, Method::GET, "/api/access", Some(who), None)
        .await
        .1
}

async fn a_claim_of_600(app: &Arc<App>, x: &Expenses, purpose: &str) -> String {
    let purpose_field = columns_id(app, &x.claims, "Purpose").await;
    let (status, claim) = post(
        app,
        &format!("/api/tables/{}/records", x.claims),
        json!({"cells": {&purpose_field: {"text": purpose}}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    let claim = claim["id"].as_str().unwrap().to_owned();
    let description = columns_id(app, &x.lines, "Description").await;
    let (status, line) = post(app, &format!("/api/tables/{}/records", x.lines), json!({"cells": {&description: {"text": "Stand"}, &x.amount: {"text": "600"}, &x.claim_link: {"value": {"type": "link", "value": [claim]}}}})).await;
    assert_eq!(status, StatusCode::OK, "{line}");
    claim
}

const DAY: i64 = 86_400_000;

#[tokio::test]
async fn std_grants_limited_to_fields_and_to_a_period() {
    let app = empty();
    let x = build(&app).await;
    let p = people(&app).await;
    a_claim_of_600(&app, &x, "Fair").await;
    let at = |days: i64| now().millis() + days * DAY;
    // Max reads the claims but not their totals.
    let (status, body) = grant(&app, json!({"to": p.max, "capability": "record.read", "on": {"kind": "table", "id": x.claims}, "fields": {"except": [x.total]}, "reason": "Approves"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let seen = query_as(&app, &x.claims, &p.max).await;
    assert_eq!(seen["rows"].as_array().unwrap().len(), 1);
    assert!(
        seen["rows"][0]["cells"].get(&x.total).is_none(),
        "never sent: {}",
        seen["rows"][0]
    );
    assert_eq!(seen["rows"][0]["cells"][&x.needs_finance]["d"], "No");
    // Mia from tomorrow; Fiona until tomorrow.
    let (status, _) = grant(&app, json!({"to": p.mia, "capability": "record.read", "on": {"kind": "table", "id": x.claims}, "validFrom": at(1), "reason": "Starts tomorrow"})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        query_as(&app, &x.claims, &p.mia).await["rows"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "not yet"
    );
    let (status, _) = grant(&app, json!({"to": p.fiona, "capability": "record.read", "on": {"kind": "table", "id": x.claims}, "fields": {"only": [x.total, x.needs_finance]}, "validTo": at(1), "reason": "Audit this week"})).await;
    assert_eq!(status, StatusCode::OK);
    let fiona = query_as(&app, &x.claims, &p.fiona).await;
    assert_eq!(fiona["rows"][0]["cells"][&x.total]["d"], "600.00 USD");
    let purpose = columns_id(&app, &x.claims, "Purpose").await;
    assert!(fiona["rows"][0]["cells"].get(&purpose).is_none());
    let access = access_as(&app, ALICE).await;
    let find = |to: &str| {
        access["grants"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["toId"] == to)
            .unwrap()
            .clone()
    };
    assert_eq!(find(&p.max)["fields"], "all but “Total”");
    assert_eq!(find(&p.mia)["status"], "not_yet");
    assert_eq!(find(&p.mia)["validFrom"], at(1));
    assert_eq!(find(&p.fiona)["status"], "in_force");
    assert_eq!(find(&p.fiona)["fields"], "only “Total”, “Needs finance”");
    // Refused.
    for (body, why) in [
        (
            json!({"to": p.mia, "capability": "record.read", "on": {"kind": "table", "id": x.claims}, "validTo": at(0)}),
            "already have ended",
        ),
        (
            json!({"to": p.mia, "capability": "record.read", "on": {"kind": "table", "id": x.claims}, "validFrom": at(2), "validTo": at(1)}),
            "ends before it starts",
        ),
        (
            json!({"to": p.mia, "capability": "record.read", "on": {"kind": "base", "id": x.base}, "fields": {"only": [x.total]}}),
            "on one table",
        ),
        (
            json!({"to": p.mia, "capability": "record.read", "on": {"kind": "table", "id": x.claims}, "fields": {"only": []}}),
            "at least one field",
        ),
        (
            json!({"to": p.mia, "capability": "record.read", "on": {"kind": "table", "id": x.claims}, "fields": {"only": [x.amount]}}),
            "not a field of",
        ),
        (
            json!({"to": p.mia, "capability": "access.manage", "on": {"kind": "table", "id": x.claims}, "fields": {"only": [x.total]}}),
            "not about individual records",
        ),
    ] {
        let (status, body) = grant(&app, body).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{why}: {body}");
        assert!(
            body["message"].as_str().unwrap().contains(why),
            "{why}: {body}"
        );
    }
}

#[tokio::test]
async fn std_access_reviews_keep_or_revoke_with_a_note() {
    let app = empty();
    let x = build(&app).await;
    let p = people(&app).await;
    let base = json!({"kind": "base", "id": x.base});
    for body in [
        json!({"to": p.max, "capability": "record.read", "on": base, "reason": "Approves"}),
        json!({"to": p.finance, "capability": "record.read", "on": base, "reason": "Pays"}),
        json!({"to": p.mia, "capability": "record.read", "on": base, "validFrom": now().millis() + DAY, "reason": "Starts tomorrow"}),
    ] {
        assert_eq!(grant(&app, body).await.0, StatusCode::OK);
    }
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/access/reviews",
        Some(&p.max),
        Some(json!({"name": "Q4", "on": base})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "only access managers review");
    let (status, opened) = post(
        &app,
        "/api/access/reviews",
        json!({"name": "Q4 expenses", "on": base, "dueAt": now().millis() + 7 * DAY}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{opened}");
    let review = opened["id"].as_str().unwrap().to_owned();
    let access = access_as(&app, ALICE).await;
    let shown = access["reviews"][0].clone();
    assert_eq!(shown["covers"], "Expenses");
    assert_eq!(shown["state"], "open");
    assert_eq!(
        shown["total"], 2,
        "grants in force on the base only: {shown}"
    );
    assert_eq!(shown["overdue"], false);
    let item = |holder: &str| {
        shown["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["holderId"] == holder)
            .unwrap()["grant"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let (max_grant, finance_grant) = (item(&p.max), item(&p.finance));
    let decide = |body: Value| {
        let app = app.clone();
        let path = format!("/api/access/reviews/{review}/decisions");
        async move { post(&app, &path, body).await }
    };
    let complete = format!("/api/access/reviews/{review}/complete");
    assert_eq!(
        decide(json!({"grant": max_grant, "verdict": "keep"}))
            .await
            .0,
        StatusCode::OK
    );
    let (status, body) = post(&app, &complete, json!({})).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "undecided grants: {body}"
    );
    let (status, body) = decide(json!({"grant": finance_grant, "verdict": "revoke"})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["message"].as_str().unwrap().contains("needs a note"));
    assert_eq!(decide(json!({"grant": finance_grant, "verdict": "revoke", "note": "Finance moved to the new base"})).await.0, StatusCode::OK);
    let (status, body) = post(&app, &complete, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revoked"], 1);
    let access = access_as(&app, ALICE).await;
    let status_of = |id: &str| {
        access["grants"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["id"] == id)
            .unwrap()["status"]
            .clone()
    };
    assert_eq!(status_of(&finance_grant), "revoked");
    assert_eq!(status_of(&max_grant), "in_force");
    assert_eq!(access["reviews"][0]["state"], "completed");
    assert_eq!(access["reviews"][0]["closedBy"], "Alice Admin");
    let (status, _) = post(&app, &complete, json!({})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "closed");
    assert_eq!(
        decide(json!({"grant": max_grant, "verdict": "revoke", "note": "late"}))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Nobody reviews their own access; a review can be abandoned.
    let (_, opened) = post(&app, "/api/access/reviews", json!({"name": "Everything"})).await;
    let everything = opened["id"].as_str().unwrap().to_owned();
    let access = access_as(&app, ALICE).await;
    let own = access["reviews"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["holder"] == "Alice Admin")
        .unwrap()["grant"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, body) = post(
        &app,
        &format!("/api/access/reviews/{everything}/decisions"),
        json!({"grant": own, "verdict": "keep"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["message"].as_str().unwrap().contains("own access"));
    assert_eq!(
        post(
            &app,
            &format!("/api/access/reviews/{everything}/cancel"),
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        post(
            &app,
            &format!("/api/access/reviews/{everything}/cancel"),
            json!({})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Refused openings.
    assert_eq!(
        post(&app, "/api/access/reviews", json!({"name": " "}))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(
            &app,
            "/api/access/reviews",
            json!({"name": "Late", "dueAt": now().millis()})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(
            &app,
            "/api/access/reviews",
            json!({"name": "Lines", "on": {"kind": "table", "id": x.lines}})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY,
        "no grant there"
    );
    assert_eq!(
        post(
            &app,
            "/api/access/reviews/arv_00000000000000000000000001/complete",
            json!({})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (_, audit) = call(&app, Method::GET, "/api/audit", Some(ALICE), None).await;
    let text = audit.to_string();
    for action in [
        "access.review.open",
        "access.review.decide",
        "access.review.complete",
        "access.review.cancel",
    ] {
        assert!(text.contains(action), "{action}");
    }
    assert!(
        text.contains("Finance moved to the new base"),
        "the note is in the audit trail"
    );
}

#[tokio::test]
async fn std_share_links_show_one_table_and_never_more_than_their_maker_sees() {
    let app = empty();
    let x = build(&app).await;
    let p = people(&app).await;
    let fair = a_claim_of_600(&app, &x, "Fair").await;
    a_claim_of_600(&app, &x, "Expo").await;
    // A link to one claim, without the formula.
    let path = format!("/api/tables/{}/shares", x.claims);
    let (status, made) = post(&app, &path, json!({"record": fair, "fields": {"except": [x.needs_finance]}, "expiresAt": now().millis() + DAY})).await;
    assert_eq!(status, StatusCode::OK, "{made}");
    let token = made["token"].as_str().unwrap().to_owned();
    assert_eq!(token.len(), 48);
    let (status, opened) = call(
        &app,
        Method::GET,
        &format!("/api/share/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{opened}");
    assert_eq!(opened["table"], "Expense claims");
    assert_eq!(opened["record"], "Fair");
    assert_eq!(opened["sharedBy"], "Alice Admin");
    assert_eq!(opened["records"].as_array().unwrap().len(), 1);
    let names: Vec<&str> = opened["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"Total") && names.contains(&"Purpose"),
        "{names:?}"
    );
    assert!(!names.contains(&"Needs finance"), "left out by the link");
    assert!(!names.contains(&"Lines"), "a link opens one table");
    assert_eq!(opened["records"][0]["cells"][&x.total]["d"], "600.00 USD");
    assert!(opened["records"][0]["cells"]
        .get(&x.needs_finance)
        .is_none());
    // The token is shown to those who may hand it out, and never written to the audit trail.
    let access = access_as(&app, ALICE).await;
    assert_eq!(access["shares"][0]["token"], token.as_str());
    assert_eq!(access["shares"][0]["record"], "Fair");
    assert_eq!(access["shares"][0]["status"], "active");
    assert_eq!(access["shares"][0]["fields"], "all but “Needs finance”");
    assert!(access_as(&app, &p.max).await["shares"][0]["token"].is_null());
    let (_, audit) = call(&app, Method::GET, "/api/audit", Some(ALICE), None).await;
    assert!(audit.to_string().contains("share.create"));
    assert!(!audit.to_string().contains(&token));
    // Max may share the claims, but does not see their totals: neither does his link.
    for body in [
        json!({"to": p.max, "capability": "record.read", "on": {"kind": "table", "id": x.claims}, "fields": {"except": [x.total]}, "reason": "Approves"}),
        json!({"to": p.max, "capability": "share.manage", "on": {"kind": "table", "id": x.claims}, "reason": "Shares with auditors"}),
    ] {
        assert_eq!(grant(&app, body).await.0, StatusCode::OK);
    }
    let (status, made) = call(&app, Method::POST, &path, Some(&p.max), Some(json!({}))).await;
    assert_eq!(status, StatusCode::OK, "{made}");
    let max_token = made["token"].as_str().unwrap().to_owned();
    let max_share = made["id"].as_str().unwrap().to_owned();
    let (_, opened) = call(
        &app,
        Method::GET,
        &format!("/api/share/{max_token}"),
        None,
        None,
    )
    .await;
    assert_eq!(
        opened["records"].as_array().unwrap().len(),
        2,
        "the whole table"
    );
    assert!(opened["record"].is_null());
    let names: Vec<&str> = opened["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert!(
        !names.contains(&"Total"),
        "Max cannot see totals: {names:?}"
    );
    // When Max loses the right to share, his link stops working.
    let access = access_as(&app, ALICE).await;
    let share_grant = access["grants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["what"] == "share.manage")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/api/access/grants/{share_grant}"),
            Some(ALICE),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, body) = call(
        &app,
        Method::GET,
        &format!("/api/share/{max_token}"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::GONE);
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("may no longer share"));
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/shares/{max_share}"),
        Some(&p.max),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Withdrawn, unknown and refused links.
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/api/shares/{max_share}"),
            Some(ALICE),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/api/shares/{max_share}"),
            Some(ALICE),
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (status, body) = call(
        &app,
        Method::GET,
        &format!("/api/share/{max_token}"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::GONE);
    assert_eq!(body["message"], "this link was withdrawn");
    assert_eq!(
        call(
            &app,
            Method::GET,
            "/api/share/0123456789abcdef0123456789abcdef0123456789abcdef",
            None,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, Method::GET, "/api/share/short", None, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, Method::POST, &path, Some(&p.mia), Some(json!({})))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(&app, &path, json!({"expiresAt": now().millis()}))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(
            &app,
            &path,
            json!({"record": "rec_00000000000000000000000zzz"})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(post(&app, &path, json!({"fields": {"except": [x.total, x.needs_finance, columns_id(&app, &x.claims, "Purpose").await]}})).await.0, StatusCode::OK, "other fields remain");
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            "/api/shares/shl_00000000000000000000000001",
            Some(ALICE),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn edge_share_links_expire() {
    let dir = std::env::temp_dir().join(format!("agenttable-share-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("data.json");
    let _ = std::fs::remove_file(&file);
    let start = now().millis();
    let app = app_at(&file, start);
    let x = build(&app).await;
    a_claim_of_600(&app, &x, "Fair").await;
    let (_, made) = post(
        &app,
        &format!("/api/tables/{}/shares", x.claims),
        json!({"expiresAt": start + 3_600_000}),
    )
    .await;
    let token = made["token"].as_str().unwrap().to_owned();
    assert_eq!(
        call(
            &app,
            Method::GET,
            &format!("/api/share/{token}"),
            None,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    drop(app);
    let later = app_at(&file, start + 3_600_000);
    let (status, body) = call(
        &later,
        Method::GET,
        &format!("/api/share/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::GONE);
    assert_eq!(body["message"], "this link has expired");
    assert_eq!(
        access_as(&later, ALICE).await["shares"][0]["status"],
        "expired"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

async fn code_list(app: &Arc<App>, set: &str, who: &str, query: &str) -> Value {
    call(
        app,
        Method::GET,
        &format!("/api/refdata/{set}{query}"),
        Some(who),
        None,
    )
    .await
    .1
}

fn item_names(list: &Value) -> Vec<String> {
    list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn std_code_lists_keep_a_history_and_can_need_a_second_person() {
    let dir = std::env::temp_dir().join(format!("agenttable-codelist-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("data.json");
    let _ = std::fs::remove_file(&file);
    let app = app_at(&file, now().millis());
    let x = build(&app).await;
    let p = people(&app).await;
    let set = x.categories.clone();
    let list = code_list(&app, &set, ALICE, "").await;
    assert_eq!(list["history"]["version"], 4, "four items, four versions");
    assert_eq!(list["history"]["approval"], false);
    assert_eq!(list["canManage"], true);
    assert_eq!(
        list["history"]["versions"][0]["changes"][0],
        "added “Meals” (MEALS)"
    );
    assert_eq!(
        list["history"]["versions"][2]["changes"][0],
        "added “Flights” (FLIGHT) under “Travel”"
    );
    assert_eq!(list["history"]["versions"][0]["by"], "Alice Admin");
    assert!(list["history"]["versions"][0]["approvedBy"].is_null());
    // Max may manage code lists too: he is the second person.
    let (status, _) = grant(&app, json!({"to": p.max, "capability": "refdata.manage", "on": {"kind": "workspace"}, "reason": "Second pair of eyes"})).await;
    assert_eq!(status, StatusCode::OK);
    let path = format!("/api/refdata/{set}");
    let (status, body) = call(
        &app,
        Method::PATCH,
        &path,
        Some(ALICE),
        Some(json!({"approval": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({"applied": true, "version": 5}),
        "switching four eyes on applies at once"
    );
    // From now on, changes are proposed.
    let (status, body) = post(
        &app,
        &format!("{path}/items"),
        json!({"code": "TAXI", "name": "Taxis", "parent": item_id(&list, "Travel")}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["applied"], false);
    assert_eq!(body["proposal"], 1);
    let list = code_list(&app, &set, ALICE, "").await;
    assert!(
        !item_names(&list).contains(&"Taxis".to_owned()),
        "not before approval"
    );
    assert_eq!(list["history"]["proposals"][0]["status"], "pending");
    assert_eq!(
        list["history"]["proposals"][0]["changes"][0],
        "added “Taxis” (TAXI) under “Travel”"
    );
    let decide = |who: &str, n: u64, body: Value| {
        let app = app.clone();
        let path = format!("{path}/proposals/{n}");
        let who = who.to_owned();
        async move { call(&app, Method::POST, &path, Some(&who), Some(body)).await }
    };
    let (status, body) = decide(ALICE, 1, json!({"decision": "approve"})).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "nobody approves their own change"
    );
    assert!(body["message"].as_str().unwrap().contains("someone else"));
    let (status, body) = decide(&p.max, 1, json!({"decision": "approve"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["version"], 6);
    let list = code_list(&app, &set, ALICE, "").await;
    assert!(item_names(&list).contains(&"Taxis".to_owned()));
    assert_eq!(list["history"]["versions"][0]["by"], "Alice Admin");
    assert_eq!(list["history"]["versions"][0]["approvedBy"], "Max Manager");
    assert_eq!(list["history"]["proposals"][0]["status"], "approved");
    assert_eq!(list["history"]["proposals"][0]["decidedBy"], "Max Manager");
    assert_eq!(
        decide(&p.max, 1, json!({"decision": "approve"})).await.0,
        StatusCode::CONFLICT,
        "decided already"
    );
    assert_eq!(
        decide(&p.max, 9, json!({"decision": "approve"})).await.0,
        StatusCode::NOT_FOUND
    );
    // A rejection needs a reason; the item stays as it was.
    let meals = item_id(&list, "Meals");
    let (status, body) = call(
        &app,
        Method::PATCH,
        &format!("{path}/items/{meals}"),
        Some(ALICE),
        Some(json!({"enabled": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["proposal"], 2);
    assert_eq!(
        decide(&p.max, 2, json!({"decision": "reject", "reason": " "}))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        decide(
            &p.max,
            2,
            json!({"decision": "reject", "reason": "Meals are still claimed"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let list = code_list(&app, &set, ALICE, "").await;
    assert_eq!(
        list["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["name"] == "Meals")
            .unwrap()["enabled"],
        true
    );
    assert_eq!(
        list["history"]["proposals"][0]["reason"],
        "Meals are still claimed"
    );
    // Switching four eyes off needs four eyes; only its author withdraws a proposal.
    let (status, body) = call(
        &app,
        Method::PATCH,
        &path,
        Some(ALICE),
        Some(json!({"approval": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"applied": false, "proposal": 3}));
    assert_eq!(
        code_list(&app, &set, ALICE, "").await["history"]["proposals"][0]["changes"][0],
        "changes apply at once again"
    );
    assert_eq!(
        decide(&p.max, 3, json!({"decision": "withdraw"})).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        decide(ALICE, 3, json!({"decision": "withdraw"})).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &path,
            Some(ALICE),
            Some(json!({"approval": true}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY,
        "already so"
    );
    // Any version can be shown as it was.
    assert!(item_names(&code_list(&app, &set, ALICE, "?asOf=0").await).is_empty());
    let v4 = code_list(&app, &set, ALICE, "?asOf=4").await;
    assert_eq!(item_names(&v4), ["Travel", "Flights", "Hotels", "Meals"]);
    assert_eq!(v4["asOf"], 4);
    assert_eq!(
        item_names(&code_list(&app, &set, ALICE, "?asOf=6").await).len(),
        5
    );
    assert_eq!(
        call(
            &app,
            Method::GET,
            &format!("{path}?asOf=99"),
            Some(ALICE),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    // Without the right: read only.
    assert_eq!(code_list(&app, &set, &p.mia, "").await["canManage"], false);
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            &path,
            Some(&p.mia),
            Some(json!({"approval": false}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        decide(&p.mia, 3, json!({"decision": "approve"})).await.0,
        StatusCode::FORBIDDEN
    );
    // The audit trail has every step.
    let (_, audit) = call(&app, Method::GET, "/api/audit", Some(ALICE), None).await;
    let text = audit.to_string();
    for action in [
        "refset.policy",
        "refset.change.propose",
        "refset.change.approve",
        "refset.change.reject",
        "refset.change.withdraw",
    ] {
        assert!(text.contains(action), "{action}");
    }
    // The history survives a restart, and a history that does not lead to the list is refused.
    drop(app);
    let again = app_at(&file, now().millis());
    assert_eq!(
        code_list(&again, &set, ALICE, "").await["history"]["version"],
        6
    );
    drop(again);
    let mut stored: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    stored["refset_history"][0]["revisions"]
        .as_array_mut()
        .unwrap()
        .pop();
    std::fs::write(&file, stored.to_string()).unwrap();
    let refused = mt_server::open(
        Some(file.clone()),
        Clock::Fixed(now()),
        mt_server::Start::Empty,
    );
    assert!(refused.is_err(), "an inconsistent history is not loaded");
    let _ = std::fs::remove_dir_all(&dir);
}

fn item_id(list: &Value, name: &str) -> String {
    list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == name)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn setting_of(app: &Arc<App>, key: &str) -> Value {
    let (_, list) = call(app, Method::GET, "/api/settings", None, None).await;
    list.as_array()
        .unwrap()
        .iter()
        .find(|s| s["key"] == key)
        .cloned()
        .unwrap_or(Value::Null)
}

#[tokio::test]
async fn std_settings_are_overridden_per_base_explained_and_kept() {
    let dir = std::env::temp_dir().join(format!("agenttable-settings-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("data.json");
    let _ = std::fs::remove_file(&file);
    let app = app_at(&file, now().millis());
    let x = build(&app).await;
    let p = people(&app).await;
    a_claim_of_600(&app, &x, "Fair").await;
    let key = "expense.manager_limit";
    let path = format!("/api/settings/{key}");
    let needs_finance = |app: Arc<App>| {
        let claims = x.claims.clone();
        let field = x.needs_finance.clone();
        async move { cell(&rows(&app, &claims).await, "Fair", &field) }
    };
    let workspace_value = setting_of(&app, key).await["value"].clone();
    assert_eq!(
        needs_finance(app.clone()).await,
        if workspace_value == "1,000.00 USD" {
            "No"
        } else {
            "Yes"
        }
    );
    let (status, _) = call(
        &app,
        Method::PUT,
        &path,
        Some(ALICE),
        Some(json!({"type": "money", "text": "1000 USD"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(needs_finance(app.clone()).await, "No");
    // The Expenses base sets its own, lower limit: its formulas follow.
    let (status, body) = call(
        &app,
        Method::PUT,
        &path,
        Some(ALICE),
        Some(json!({"type": "money", "text": "300 USD", "base": x.base})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(needs_finance(app.clone()).await, "Yes");
    let setting = setting_of(&app, key).await;
    assert_eq!(setting["type"], "money");
    assert_eq!(
        setting["value"], "1,000.00 USD",
        "the workspace value is unchanged"
    );
    let overrides = setting["overrides"].as_array().unwrap();
    assert_eq!(overrides.len(), 2);
    assert_eq!(overrides[1]["for"], "the base Expenses");
    assert_eq!(overrides[1]["value"], "300.00 USD");
    assert_eq!(overrides[1]["by"], "Alice Admin");
    assert!(setting["history"]
        .as_array()
        .unwrap()
        .iter()
        .any(|h| h["for"] == "the base Expenses" && h["value"] == "300.00 USD"));
    // Explained for the base, for the workspace, and for a base without its own value.
    let explain = |base: Option<&str>| {
        let app = app.clone();
        let query = base.map_or_else(String::new, |b| format!("?base={b}"));
        let path = format!("{path}/explain{query}");
        async move { call(&app, Method::GET, &path, None, None).await }
    };
    let (_, here) = explain(Some(&x.base)).await;
    assert_eq!(here["value"], "300.00 USD");
    assert!(
        here["text"].as_str().unwrap().contains(
            "is 300.00 USD for the base Expenses: set for the base Expenses by Alice Admin on 2026-11-18 10:00 UTC."
        ),
        "{here}"
    );
    let (_, everywhere) = explain(None).await;
    assert!(everywhere["text"]
        .as_str()
        .unwrap()
        .contains("is 1,000.00 USD for the whole workspace: set for the whole workspace"));
    let other = id(&app, "/api/bases", json!({"name": "Travel desk"})).await;
    let (_, elsewhere) = explain(Some(&other)).await;
    assert_eq!(elsewhere["value"], "1,000.00 USD");
    assert!(elsewhere["text"].as_str().unwrap().contains("for the base Travel desk, which has no value of its own: the value for the whole workspace applies (set by Alice Admin"), "{elsewhere}");
    let (_, unset) = call(
        &app,
        Method::PUT,
        "/api/settings/travel.daily_allowance",
        Some(ALICE),
        Some(json!({"type": "money", "text": "80 EUR", "base": other})),
    )
    .await;
    assert_eq!(unset["updated"], true);
    let (_, none) = call(
        &app,
        Method::GET,
        "/api/settings/travel.daily_allowance/explain",
        None,
        None,
    )
    .await;
    assert_eq!(
        none["text"],
        "`travel.daily_allowance` has no value for the whole workspace."
    );
    assert!(none["value"].is_null());
    // Someone who manages the settings of one base sets its override, not the workspace's.
    assert_eq!(grant(&app, json!({"to": p.max, "capability": "settings.manage", "on": {"kind": "base", "id": x.base}, "reason": "Runs the expense base"})).await.0, StatusCode::OK);
    assert_eq!(
        call(
            &app,
            Method::PUT,
            &path,
            Some(&p.max),
            Some(json!({"type": "money", "text": "400 USD", "base": x.base}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            Method::PUT,
            &path,
            Some(&p.max),
            Some(json!({"type": "money", "text": "400 USD"}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            Method::PUT,
            &path,
            Some(&p.max),
            Some(json!({"type": "money", "text": "400 USD", "base": other}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            Method::PUT,
            &path,
            Some(&p.mia),
            Some(json!({"type": "money", "text": "1 USD", "base": x.base}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    // A setting keeps its type; removing the override brings the workspace value back.
    let (status, body) = call(
        &app,
        Method::PUT,
        &path,
        Some(ALICE),
        Some(json!({"type": "number", "text": "5", "base": x.base})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("{path}?base={}", x.base),
            Some(ALICE),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(needs_finance(app.clone()).await, "No");
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("{path}?base={}", x.base),
            Some(ALICE),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND,
        "nothing to remove"
    );
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("{path}?base=bas_0000000000000000000000000z"),
            Some(ALICE),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            "/api/settings/no.such",
            Some(ALICE),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("{path}?base={}", x.base),
            Some(&p.mia),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            Method::GET,
            "/api/settings/no.such/explain",
            None,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            Method::PUT,
            &path,
            Some(ALICE),
            Some(
                json!({"type": "money", "text": "1 USD", "base": "bas_0000000000000000000000000z"})
            )
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let history = setting_of(&app, key).await["history"].clone();
    let base_history: Vec<&Value> = history
        .as_array()
        .unwrap()
        .iter()
        .filter(|h| h["for"] == "the base Expenses")
        .collect();
    assert_eq!(base_history[0]["value"], "cleared", "newest first");
    assert_eq!(base_history[1]["by"], "Max Manager");
    assert_eq!(base_history.len(), 3);
    let (_, audit) = call(&app, Method::GET, "/api/audit", Some(ALICE), None).await;
    assert!(audit.to_string().contains("setting.clear"));
    // Saved and reloaded; settings saved the old way are carried over.
    drop(app);
    let mut stored: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert!(stored.get("params").is_none(), "only the new form is saved");
    let rate = stored["settings"]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["key"] == key)
        .unwrap()["entry"]["setting"]["value"]
        .clone();
    stored["params"] = json!({"legacy.limit": rate});
    std::fs::write(&file, stored.to_string()).unwrap();
    let again = app_at(&file, now().millis());
    let legacy = setting_of(&again, "legacy.limit").await;
    assert_eq!(legacy["value"], "1,000.00 USD");
    assert_eq!(legacy["history"][0]["by"], "the system");
    assert_eq!(
        setting_of(&again, key).await["overrides"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    drop(again);
    // Settings that do not hold are not loaded.
    let mut broken: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    broken["settings"]["definitions"][0]["key"] = json!("Bad Key");
    std::fs::write(&file, broken.to_string()).unwrap();
    assert!(mt_server::open(
        Some(file.clone()),
        Clock::Fixed(now()),
        mt_server::Start::Empty
    )
    .is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

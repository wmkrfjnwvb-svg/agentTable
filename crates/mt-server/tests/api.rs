//! The HTTP API, end to end in process: every endpoint, as different people, against the demo
//! workspace (fixed clock, no data file unless a test saves one).
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
use mt_server::seed::{self, line, request, supplier, LINES, REQUESTS, SUPPLIERS};
use tower as _;

#[tokio::test]
async fn std_status_people_and_tables() {
    let app = app();
    let (status, body) = call(&app, Method::GET, "/api/status", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["product"], "agenttable");
    let states: Vec<&str> = body["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["state"].as_str().unwrap())
        .collect();
    assert!(
        states.contains(&"live") && states.contains(&"placeholder") && states.contains(&"engine")
    );
    let (_, people) = call(&app, Method::GET, "/api/people", None, None).await;
    assert_eq!(people.as_array().unwrap().len(), 9);
    let team = people
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "Purchasing team")
        .unwrap();
    assert_eq!(team["members"], json!(["Bob Buyer", "Carol Controller"]));
    // What each person can see of the tables.
    let (_, tables) = call(&app, Method::GET, "/api/tables", Some(GINA), None).await;
    let summary: Vec<(String, u64, bool)> = tables
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap().to_owned(),
                t["records"].as_u64().unwrap(),
                t["canRead"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("Suppliers".to_owned(), 1, true),
            ("Purchase requests".to_owned(), 0, false),
            ("Request lines".to_owned(), 0, false)
        ]
    );
    let (_, tables) = call(&app, Method::GET, "/api/tables", None, None).await;
    let counts: Vec<u64> = tables
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["records"].as_u64().unwrap())
        .collect();
    assert_eq!(counts, [4, 5, 9], "Alice is the default user");
    let (status, table) = call(&app, Method::GET, &table_path(REQUESTS), Some(BOB), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(table["canCreate"], true);
    assert_eq!(table["canManageSchema"], false);
    let total = table["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Total")
        .unwrap();
    assert_eq!(total["formula"], "{Subtotal} + {Tax}");
    assert_eq!(total["computed"], "formula");
    assert_eq!(total["typeLabel"], "Formula");
}

#[tokio::test]
async fn invalid_identities_and_ids() {
    let app = app();
    for (user, message) in [
        ("nope", "is not a principal id"),
        (TEAM, "cannot sign in"),
        ("prn_000000000000000000000000zz", "cannot sign in"),
    ] {
        let (status, body) = call(&app, Method::GET, "/api/tables", Some(user), None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{user}");
        assert!(
            body["message"].as_str().unwrap().contains(message),
            "{body}"
        );
    }
    let (status, _) = call(&app, Method::GET, "/api/tables/nope", None, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = call(
        &app,
        Method::GET,
        "/api/tables/tbl_0000000000000000000000000z",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "not_found");
    let (status, _) = call(
        &app,
        Method::GET,
        &format!(
            "{}/records/rec_0000000000000000000000000z",
            table_path(REQUESTS)
        ),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn std_formulas_compute_across_tables() {
    let app = app();
    let (_, requests) = query(&app, ALICE, REQUESTS, json!({})).await;
    assert_eq!(requests["count"], 5);
    assert_eq!(
        cell(&requests, "Laptops for new hires", request::SUBTOTAL),
        "4,965.00 USD"
    );
    assert_eq!(
        cell(&requests, "Laptops for new hires", request::TAX),
        "397.20 USD"
    );
    assert_eq!(
        cell(&requests, "Laptops for new hires", request::TOTAL),
        "5,362.20 USD"
    );
    assert_eq!(
        cell(&requests, "Trade fair booth", request::NEEDS_CFO),
        "Yes"
    );
    assert_eq!(cell(&requests, "Office chairs", request::NEEDS_CFO), "No");
    assert_eq!(
        cell(&requests, "Design tool licences", request::DAYS_LEFT),
        "-5"
    );
    assert_eq!(
        cell(&requests, "Laptops for new hires", request::NUMBER),
        "PR-2026-0001"
    );
    assert_eq!(
        cell(&requests, "Laptops for new hires", request::CHANGED_BY),
        "Bob Buyer"
    );
    assert_eq!(
        cell(&requests, "Laptops for new hires", request::COST_CENTRE),
        "IT"
    );
    assert_eq!(
        requests["totals"][request::TOTAL.to_string()],
        "46,530.72 USD"
    );
    // Totals cover amounts, not computed day counts.
    assert!(requests["totals"]
        .get(request::DAYS_LEFT.to_string())
        .is_none());
    let (_, suppliers) = query(&app, ALICE, SUPPLIERS, json!({})).await;
    assert_eq!(
        cell(&suppliers, "Northwind Traders", supplier::SPEND),
        "6,333.12 USD"
    );
    assert_eq!(
        cell(&suppliers, "Northwind Traders", supplier::REQUESTS),
        "Laptops for new hires, Printer toner"
    );
    // Change a line of the draft: the request, and the supplier's spend, follow.
    let (_, lines) = query(&app, ALICE, LINES, json!({"search": "chair"})).await;
    assert_eq!(titles(&lines), ["Ergonomic chair"]);
    let chair = record_id(&lines, "Ergonomic chair");
    let path = format!("{}/records/{chair}", table_path(LINES));
    let (status, record) = call(
        &app,
        Method::PATCH,
        &path,
        None,
        Some(json!({"cells": {line::QUANTITY.to_string(): {"text": "8"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{record}");
    assert_eq!(record["version"], 2);
    let (_, requests) = query(&app, ALICE, REQUESTS, json!({})).await;
    assert_eq!(
        cell(&requests, "Office chairs", request::TOTAL),
        "2,764.80 USD"
    );
    let (_, suppliers) = query(&app, ALICE, SUPPLIERS, json!({})).await;
    assert_eq!(
        cell(&suppliers, "Contoso Supplies", supplier::SPEND),
        "2,764.80 USD"
    );
    // A line of a request under approval cannot change: its total is being approved.
    let (_, lines) = query(&app, ALICE, LINES, json!({"search": "laptop 14"})).await;
    let laptop = record_id(&lines, "Laptop 14\"");
    let (status, body) = call(
        &app,
        Method::PATCH,
        &format!("{}/records/{laptop}", table_path(LINES)),
        None,
        Some(json!({"cells": {line::QUANTITY.to_string(): {"text": "4"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        body["message"],
        "“Laptops for new hires” is Submitted: its lines cannot change"
    );
    // An unchanged value is not a change.
    let (status, record) = call(
        &app,
        Method::PATCH,
        &path,
        None,
        Some(json!({"cells": {line::QUANTITY.to_string(): {"text": "8"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(record["version"], 2);
}

#[tokio::test]
async fn std_search_filter_sort_group() {
    let app = app();
    let (_, sorted) = query(
        &app,
        ALICE,
        REQUESTS,
        json!({"sort": [{"field": request::TOTAL, "desc": true}]}),
    )
    .await;
    assert_eq!(titles(&sorted)[0], "Trade fair booth");
    let (_, filtered) = query(
        &app,
        ALICE,
        REQUESTS,
        json!({"filters": [{"field": request::TOTAL, "op": "gt", "value": {"text": "5000"}}]}),
    )
    .await;
    assert_eq!(filtered["count"], 3);
    let (_, mine) = query(
        &app,
        CAROL,
        REQUESTS,
        json!({"filters": [{"field": request::REQUESTER, "op": "eq", "me": true}]}),
    )
    .await;
    assert_eq!(titles(&mine), ["Design tool licences", "Trade fair booth"]);
    let (_, empty) = query(
        &app,
        ALICE,
        SUPPLIERS,
        json!({"filters": [{"field": supplier::EMAIL, "op": "is_empty"}]}),
    )
    .await;
    assert_eq!(titles(&empty), ["Tailspin Events"]);
    let status_id = seed::status::SUBMITTED.to_string();
    let (_, submitted) = query(&app, ALICE, REQUESTS, json!({"filters": [{"field": request::STATUS, "op": "eq", "value": {"value": {"type": "reference", "value": status_id}}}]})).await;
    assert_eq!(submitted["count"], 2);
    let (_, grouped) = query(&app, ALICE, REQUESTS, json!({"group": request::STATUS})).await;
    let keys: Vec<&str> = grouped["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["key"].as_str().unwrap())
        .collect();
    assert_eq!(
        keys,
        ["Draft", "Submitted", "Approved", "Ordered"],
        "in the order of the options"
    );
    let submitted = grouped["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["key"] == "Submitted")
        .unwrap();
    assert_eq!(submitted["count"], 2);
    assert_eq!(
        submitted["totals"][request::TOTAL.to_string()],
        "33,010.20 USD"
    );
    let (_, by_centre) = query(
        &app,
        ALICE,
        REQUESTS,
        json!({"group": request::COST_CENTRE, "sort": [{"field": request::TITLE}]}),
    )
    .await;
    assert_eq!(by_centre["groups"].as_array().unwrap().len(), 4);
    let (_, by_person) = query(&app, ALICE, REQUESTS, json!({"group": request::REQUESTER})).await;
    assert_eq!(by_person["groups"].as_array().unwrap().len(), 2);
    let (_, searched) = query(&app, ALICE, REQUESTS, json!({"search": "northwind"})).await;
    assert_eq!(searched["count"], 2, "links are searched by their titles");
    // A request without a body is a plain listing.
    let (status, all) = call(
        &app,
        Method::POST,
        &format!("{}/query", table_path(SUPPLIERS)),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(all["count"], 4);
}

#[tokio::test]
async fn invalid_queries() {
    let app = app();
    let (status, body) = query(
        &app,
        BOB,
        SUPPLIERS,
        json!({"sort": [{"field": supplier::IBAN}]}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body["message"].as_str().unwrap().contains("cannot see"));
    let (status, _) = query(
        &app,
        ALICE,
        REQUESTS,
        json!({"filters": [{"field": request::TITLE, "op": "nope"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = query(
        &app,
        ALICE,
        REQUESTS,
        json!({"filters": [{"field": "fld_0000000000000000000000000z", "op": "eq"}]}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "an unknown field is not visible"
    );
    let (status, _) = query(
        &app,
        ALICE,
        REQUESTS,
        json!({"filters": [{"field": request::TOTAL, "op": "contains", "value": {"text": "x"}}]}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, body) = query(&app, GINA, REQUESTS, json!({})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["details"]["allowed"], false);
}

#[tokio::test]
async fn std_each_person_sees_their_share() {
    let app = app();
    let (_, bob) = query(&app, BOB, REQUESTS, json!({})).await;
    assert_eq!(
        titles(&bob),
        ["Laptops for new hires", "Office chairs", "Printer toner"]
    );
    assert!(bob["rows"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["canEdit"] == true && r["canDelete"] == false));
    let (_, carol) = query(&app, CAROL, REQUESTS, json!({})).await;
    assert_eq!(carol["count"], 5);
    let editable: Vec<bool> = carol["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["canEdit"].as_bool().unwrap())
        .collect();
    assert_eq!(editable, [false, false, true, true, false]);
    // Bank details: hidden for the team, masked for nobody who is cleared, in clear for finance.
    let (_, team) = query(&app, BOB, SUPPLIERS, json!({})).await;
    let hidden: Vec<&str> = team["table"]["hidden"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["name"].as_str().unwrap())
        .collect();
    assert_eq!(hidden, ["IBAN"]);
    assert!(team["table"]["hidden"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("data.confidential"));
    let (_, finance) = query(&app, DAVE, SUPPLIERS, json!({})).await;
    assert_eq!(
        cell(&finance, "Contoso Supplies", supplier::IBAN),
        "DE89370400440532013000"
    );
    // The guest: her company only, five fields.
    let (_, guest) = query(&app, GINA, SUPPLIERS, json!({})).await;
    assert_eq!(titles(&guest), ["Northwind Traders"]);
    assert_eq!(guest["rows"][0]["cells"].as_object().unwrap().len(), 5);
    let reasons: Vec<&str> = guest["table"]["hidden"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["reason"].as_str().unwrap())
        .collect();
    assert!(
        reasons.iter().all(|r| *r == "not included in your grants"),
        "{reasons:?}"
    );
    // The assistant on its own sees everything readable; acting for Bob, only Bob's share.
    let (_, alone) = query(&app, ASSISTANT, REQUESTS, json!({})).await;
    assert_eq!(alone["count"], 5);
    let (_, for_bob) = call_as(
        &app,
        Method::POST,
        &format!("{}/query", table_path(REQUESTS)),
        Some(ASSISTANT),
        Some(BOB),
        Some(json!({})),
    )
    .await;
    assert_eq!(for_bob["count"], 3);
    // Acting for oneself is acting directly.
    let (_, same) = call_as(
        &app,
        Method::POST,
        &format!("{}/query", table_path(REQUESTS)),
        Some(BOB),
        Some(BOB),
        Some(json!({})),
    )
    .await;
    assert_eq!(same["count"], 3);
}

#[tokio::test]
async fn std_masked_values() {
    let app = app();
    // Alice may read confidential data but holds no unmask for… she does (administration
    // set). Take unmasking away from finance by asking as Carol with a temporary grant.
    {
        let mut ws = app.ws.lock().await;
        let mut grant = mt_perm::Grant::new(
            mt_core::id::GrantId::from_u128(900),
            mt_core::id::PrincipalId::from_u128(3),
            mt_perm::Granted::Capability(mt_perm::Action::DataConfidential.into()),
            mt_perm::Resource::Tenant,
            mt_core::id::PrincipalId::from_u128(1),
            now(),
            "test",
        );
        grant.fields = mt_perm::FieldAccess::All;
        let directory = DirectoryProbe(&ws);
        let mut policy = ws.data.policy.clone();
        policy.give(grant.clone(), &directory).unwrap();
        let grant2 = mt_perm::Grant {
            id: mt_core::id::GrantId::from_u128(901),
            what: mt_perm::Granted::Capability(
                mt_perm::Capability::custom("supplier.bank.view").unwrap(),
            ),
            ..grant
        };
        policy.give(grant2, &directory).unwrap();
        ws.data.policy = policy;
    }
    let (_, carol) = query(&app, CAROL, SUPPLIERS, json!({})).await;
    let iban = &carol["rows"][0]["cells"][supplier::IBAN.to_string()];
    assert_eq!(iban["masked"], true);
    assert_eq!(iban["d"], "******************5432");
    assert!(iban.get("v").is_none(), "the stored value is never sent");
}

struct DirectoryProbe<'a>(&'a mt_server::model::Workspace);

impl mt_perm::Directory for DirectoryProbe<'_> {
    fn kind(&self, principal: mt_core::id::PrincipalId) -> Option<mt_perm::PrincipalKind> {
        mt_perm::Directory::kind(self.0, principal)
    }
    fn groups_of(
        &self,
        principal: mt_core::id::PrincipalId,
    ) -> std::collections::BTreeSet<mt_core::id::PrincipalId> {
        self.0.groups_of(principal)
    }
    fn members_of(
        &self,
        group: mt_core::id::PrincipalId,
    ) -> std::collections::BTreeSet<mt_core::id::PrincipalId> {
        self.0.members_of(group)
    }
}

#[tokio::test]
async fn std_records_are_created_changed_and_deleted() {
    let app = app();
    let path = format!("{}/records", table_path(REQUESTS));
    let (status, created) = call(
        &app,
        Method::POST,
        &path,
        Some(BOB),
        Some(json!({"cells": {request::TITLE.to_string(): {"text": "Standing desks"}, request::NEEDED_BY.to_string(): {"text": "2026-12-24"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED.min(StatusCode::OK), "{created}");
    assert_eq!(created["title"], "Standing desks");
    let fields = created["fields"].as_array().unwrap();
    let value = |name: &str| {
        fields.iter().find(|f| f["name"] == name).unwrap()["cell"]["d"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(value("No."), "PR-2026-0006", "numbered automatically");
    assert_eq!(
        value("Requester"),
        "Bob Buyer",
        "the creator is the requester"
    );
    assert_eq!(value("Days left"), "36");
    assert_eq!(created["actions"]["update"]["allowed"], true);
    assert_eq!(created["actions"]["approve"]["allowed"], false);
    let id = created["id"].as_str().unwrap().to_owned();
    // Add a line to it: the request's lines and totals follow.
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("{}/records", table_path(LINES)),
        Some(BOB),
        Some(json!({"cells": {
            line::ITEM.to_string(): {"text": "Desk"},
            line::REQUEST.to_string(): {"value": {"type": "link", "value": [id]}},
            line::QUANTITY.to_string(): {"text": "2"},
            line::PRICE.to_string(): {"text": "650 USD"},
        }})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, record) = call(&app, Method::GET, &format!("{path}/{id}"), Some(BOB), None).await;
    let fields = record["fields"].as_array().unwrap();
    let total = fields.iter().find(|f| f["name"] == "Total").unwrap();
    assert_eq!(total["cell"]["d"], "1,404.00 USD");
    let lines = fields.iter().find(|f| f["name"] == "Lines").unwrap();
    assert_eq!(lines["cell"]["d"], "Desk");
    // Bob may not delete; Alice may.
    let (status, body) = call(
        &app,
        Method::DELETE,
        &format!("{path}/{id}"),
        Some(BOB),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["details"]["allowed"], false);
    let (status, _) = call(&app, Method::DELETE, &format!("{path}/{id}"), None, None).await;
    assert_eq!(status, StatusCode::OK);
    let (_, lines) = query(&app, ALICE, LINES, json!({"search": "desk"})).await;
    assert_eq!(
        cell(&lines, "Desk", line::REQUEST),
        "",
        "links to a deleted record are removed"
    );
    let (status, _) = call(&app, Method::GET, &format!("{path}/{id}"), None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn invalid_changes_are_refused_with_reasons() {
    let app = app();
    let (_, suppliers) = query(&app, ALICE, SUPPLIERS, json!({})).await;
    let northwind = record_id(&suppliers, "Northwind Traders");
    let path = format!("{}/records/{northwind}", table_path(SUPPLIERS));
    let issues = |body: &Value| -> Vec<String> {
        body["details"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["message"].as_str().unwrap().to_owned())
            .collect()
    };
    let (status, body) = call(&app, Method::PATCH, &path, None, Some(json!({"cells": {
        supplier::EMAIL.to_string(): {"text": "not-an-email"},
        supplier::RATING.to_string(): {"text": "9"},
        supplier::IBAN.to_string(): {"text": "DE00 1234"},
        supplier::REQUESTS.to_string(): {"value": {"type": "link", "value": ["rec_0000000000000000000000000z"]}},
    }}))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(issues(&body).len(), 4, "{body}");
    let (status, body) = call(
        &app,
        Method::PATCH,
        &path,
        None,
        Some(json!({"cells": {supplier::RATING.to_string(): {"text": "lots"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(issues(&body).len(), 1);
    // Generated values and fields the person may not change.
    let (_, requests) = query(&app, ALICE, REQUESTS, json!({})).await;
    let laptops = record_id(&requests, "Laptops for new hires");
    let request_path = format!("{}/records/{laptops}", table_path(REQUESTS));
    let (status, body) = call(
        &app,
        Method::PATCH,
        &request_path,
        Some(BOB),
        Some(json!({"cells": {request::TOTAL.to_string(): {"text": "1 USD"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(issues(&body)[0].contains("calculated"));
    let (status, body) = call(
        &app,
        Method::PATCH,
        &request_path,
        Some(CAROL),
        Some(json!({"cells": {request::TITLE.to_string(): {"text": "x"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["details"]["refusals"][0]["code"], "out_of_scope");
    let (status, _) = call(
        &app,
        Method::PATCH,
        &request_path,
        None,
        Some(json!({"cells": {"fld_0000000000000000000000000z": {"text": "x"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // A guest cannot change her company's record.
    let (status, _) = call(
        &app,
        Method::PATCH,
        &path,
        Some(GINA),
        Some(json!({"cells": {supplier::PHONE.to_string(): {"text": "+14155550199"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Required values on creation.
    let (status, body) = call(
        &app,
        Method::POST,
        &format!("{}/records", table_path(LINES)),
        None,
        Some(json!({"cells": {line::ITEM.to_string(): {"text": "Cable"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(issues(&body)[0].contains("Quantity"));
    // A line belongs to one request.
    let (status, body) = call(&app, Method::POST, &format!("{}/records", table_path(LINES)), None, Some(json!({"cells": {
        line::ITEM.to_string(): {"text": "Cable"},
        line::QUANTITY.to_string(): {"text": "1"},
        line::REQUEST.to_string(): {"value": {"type": "link", "value": [laptops, record_id(&requests, "Office chairs")]}},
    }}))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    // Nothing of this reached the audit log.
    let (_, entries) = call(&app, Method::GET, "/api/audit", None, None).await;
    assert_eq!(entries.as_array().unwrap().len(), 33);
}

#[tokio::test]
async fn std_links_stay_mirrored() {
    let app = app();
    let (_, requests) = query(&app, ALICE, REQUESTS, json!({})).await;
    let chairs = record_id(&requests, "Office chairs");
    let laptops = record_id(&requests, "Laptops for new hires");
    let (_, lines) = query(&app, ALICE, LINES, json!({})).await;
    let mouse = record_id(&lines, "Wireless mouse");
    // Lines of a request under approval stay where they are.
    let (status, _) = call(&app, Method::PATCH, &format!("{}/records/{mouse}", table_path(LINES)), None, Some(json!({"cells": {line::REQUEST.to_string(): {"value": {"type": "link", "value": [chairs]}}}}))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    // Once withdrawn, the laptops request is a draft again.
    let (status, body) = call(
        &app,
        Method::POST,
        &format!(
            "{}/records/{laptops}/transitions/withdraw",
            table_path(REQUESTS)
        ),
        Some(BOB),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["process"]["state"]["name"], "Draft");
    // Move the mouse to the chairs request: it leaves the laptops request.
    let (status, _) = call(&app, Method::PATCH, &format!("{}/records/{mouse}", table_path(LINES)), None, Some(json!({"cells": {line::REQUEST.to_string(): {"value": {"type": "link", "value": [chairs]}}}}))).await;
    assert_eq!(status, StatusCode::OK);
    let (_, requests) = query(&app, ALICE, REQUESTS, json!({})).await;
    assert_eq!(
        cell(&requests, "Office chairs", request::LINES),
        "Ergonomic chair, Wireless mouse"
    );
    assert_eq!(
        cell(&requests, "Laptops for new hires", request::LINES),
        "Laptop 14\", Docking station"
    );
    // And back, from the request's side: the line moves with it.
    let (_, record) = call(
        &app,
        Method::GET,
        &format!("{}/records/{laptops}", table_path(REQUESTS)),
        None,
        None,
    )
    .await;
    let current: Vec<String> = record["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "Lines")
        .unwrap()["cell"]["v"]["value"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    let mut wanted = current.clone();
    wanted.push(mouse.clone());
    let (status, _) = call(&app, Method::PATCH, &format!("{}/records/{laptops}", table_path(REQUESTS)), None, Some(json!({"cells": {request::LINES.to_string(): {"value": {"type": "link", "value": wanted}}}}))).await;
    assert_eq!(status, StatusCode::OK);
    let (_, requests) = query(&app, ALICE, REQUESTS, json!({})).await;
    assert_eq!(
        cell(&requests, "Office chairs", request::LINES),
        "Ergonomic chair"
    );
    assert_eq!(
        cell(&requests, "Laptops for new hires", request::TOTAL),
        "5,362.20 USD"
    );
    let (_, lines) = query(&app, ALICE, LINES, json!({})).await;
    assert_eq!(
        cell(&lines, "Wireless mouse", line::REQUEST),
        "Laptops for new hires"
    );
    // Removing a line from a request unlinks it.
    let (status, _) = call(&app, Method::PATCH, &format!("{}/records/{laptops}", table_path(REQUESTS)), None, Some(json!({"cells": {request::LINES.to_string(): {"value": {"type": "link", "value": current}}}}))).await;
    assert_eq!(status, StatusCode::OK);
    let (_, lines) = query(&app, ALICE, LINES, json!({})).await;
    assert_eq!(cell(&lines, "Wireless mouse", line::REQUEST), "");
}

#[tokio::test]
async fn std_options_for_every_kind_of_field() {
    let app = app();
    let options = |table, field: mt_core::id::FieldId| {
        format!("{}/fields/{field}/options", table_path(table))
    };
    let (_, statuses) = call(
        &app,
        Method::GET,
        &options(REQUESTS, request::STATUS),
        None,
        None,
    )
    .await;
    let labels: Vec<&str> = statuses
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["label"].as_str().unwrap())
        .collect();
    assert_eq!(
        labels,
        [
            "Draft",
            "Submitted",
            "Approved",
            "Rejected",
            "Ordered",
            "Cancelled"
        ]
    );
    let (_, centres) = call(
        &app,
        Method::GET,
        &options(REQUESTS, request::COST_CENTRE),
        None,
        None,
    )
    .await;
    assert_eq!(centres.as_array().unwrap().len(), 8);
    assert_eq!(centres[2]["path"], "Company › Operations › Facilities");
    let (_, suppliers) = call(
        &app,
        Method::GET,
        &options(REQUESTS, request::SUPPLIER),
        Some(GINA),
        None,
    )
    .await;
    assert!(suppliers.as_array().unwrap().is_empty() || suppliers.as_array().unwrap().len() == 1);
    let (_, people) = call(
        &app,
        Method::GET,
        &options(REQUESTS, request::REQUESTER),
        None,
        None,
    )
    .await;
    assert_eq!(people.as_array().unwrap().len(), 7, "people, not groups");
    let (_, none) = call(
        &app,
        Method::GET,
        &options(REQUESTS, request::TITLE),
        None,
        None,
    )
    .await;
    assert!(none.as_array().unwrap().is_empty());
    let (status, _) = call(
        &app,
        Method::GET,
        &options(REQUESTS, mt_core::id::FieldId::from_u128(999)),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn std_formula_editor_checks() {
    let app = app();
    let path = format!("{}/formula", table_path(REQUESTS));
    let (_, broken) = call(
        &app,
        Method::POST,
        &path,
        None,
        Some(json!({"text": "{Total} * "})),
    )
    .await;
    assert_eq!(broken["ok"], false);
    assert_eq!(broken["position"], 10);
    let (_, unknown) = call(
        &app,
        Method::POST,
        &path,
        None,
        Some(json!({"text": "{Nope} + 1"})),
    )
    .await;
    assert_eq!(unknown["position"], 0);
    let (_, mistyped) = call(
        &app,
        Method::POST,
        &path,
        None,
        Some(json!({"text": "{Total} + 1"})),
    )
    .await;
    assert_eq!(mistyped["ok"], false);
    assert!(mistyped["error"]
        .as_str()
        .unwrap()
        .contains("cannot combine"));
    assert!(mistyped["position"].is_null());
    let (_, arity) = call(
        &app,
        Method::POST,
        &path,
        None,
        Some(json!({"text": "ROUND()"})),
    )
    .await;
    assert!(arity["position"].is_null());
    let (_, ok) = call(
        &app,
        Method::POST,
        &path,
        Some(BOB),
        Some(json!({"text": "if({days left}>10,\"later\",\"soon\")"})),
    )
    .await;
    assert_eq!(ok["ok"], true, "{ok}");
    assert_eq!(ok["type"], "text");
    assert_eq!(
        ok["normalized"],
        "IF({Days left} > 10, \"later\", \"soon\")"
    );
    assert_eq!(
        ok["preview"].as_array().unwrap().len(),
        3,
        "previewed on records Bob can read"
    );
    let (_, warned) = call(&app, Method::POST, &path, None, Some(json!({"text": "COUNTIF({Lines}.{Quantity}, CurrentValue > AVERAGE({Lines}.{Quantity})) + DAYS({Needed by}, TODAY())"}))).await;
    let warnings = warned["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 2, "{warned}");
    let many = (0..6)
        .map(|_| "SUM({Lines}.{Quantity})")
        .collect::<Vec<_>>()
        .join(" + ");
    let (_, lookups) = call(&app, Method::POST, &path, None, Some(json!({"text": many}))).await;
    assert!(lookups["warnings"][0].as_str().unwrap().contains("6 times"));
    let (_, volatile) = call(
        &app,
        Method::POST,
        &path,
        None,
        Some(json!({"text": "TODAY()"})),
    )
    .await;
    assert!(volatile["warnings"][0].as_str().unwrap().contains("daily"));
    let (_, labels) = call(
        &app,
        Method::POST,
        &path,
        None,
        Some(json!({"text": "{Total} / {Subtotal}"})),
    )
    .await;
    assert_eq!(labels["type"], "number");
    let (_, error_value) = call(
        &app,
        Method::POST,
        &format!("{}/formula", table_path(LINES)),
        None,
        Some(json!({"text": "{Quantity} / 0"})),
    )
    .await;
    assert_eq!(error_value["preview"][0]["value"], "#DIV/0!");
}

#[tokio::test]
async fn std_fields_are_added_from_the_picker() {
    let app = app();
    let path = format!("{}/fields", table_path(REQUESTS));
    let add = |spec: Value| {
        let app = app.clone();
        let path = path.clone();
        async move { call(&app, Method::POST, &path, None, Some(spec)).await }
    };
    let (status, body) = add(
        json!({"type": "formula", "name": "Tax %", "formula": "ROUND({Tax} / {Total} * 100, 1)"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, requests) = query(&app, ALICE, REQUESTS, json!({})).await;
    let id: mt_core::id::FieldId = body["field"].as_str().unwrap().parse().unwrap();
    assert_eq!(cell(&requests, "Office chairs", id), "7.4");
    for (spec, label) in [
        (
            json!({"type": "single_select", "name": "Priority", "options": ["Low", " ", "High"]}),
            "Single select",
        ),
        (
            json!({"type": "code_list", "name": "Paid by", "refset": seed::COST_CENTRES}),
            "Code list",
        ),
        (
            json!({"type": "link", "name": "Related", "target": REQUESTS}),
            "Link to records",
        ),
        (
            json!({"type": "lookup", "name": "Line items", "formula": "{Lines}.{Item}"}),
            "Lookup",
        ),
        (
            json!({"type": "rollup", "name": "Units", "formula": "SUM({Lines}.{Quantity})"}),
            "Roll-up",
        ),
        (
            json!({"type": "auto_number", "name": "Ref", "pattern": "REF-{seq:3}"}),
            "Auto number",
        ),
        (
            json!({"type": "tax_id", "name": "VAT", "format": "tax.de_vat"}),
            "Tax or business ID",
        ),
        (json!({"type": "email", "name": "Contact"}), "Email"),
        (json!({"type": "rating", "name": "Urgency"}), "Rating"),
        (
            json!({"type": "created_by", "name": "Author"}),
            "Created by",
        ),
    ] {
        let (status, body) = add(spec.clone()).await;
        assert_eq!(status, StatusCode::OK, "{spec}: {body}");
        let columns = body["table"]["columns"].as_array().unwrap();
        let added = columns.iter().find(|c| c["id"] == body["field"]).unwrap();
        assert_eq!(added["typeLabel"], label, "{spec}");
    }
    let (_, requests) = query(&app, ALICE, REQUESTS, json!({})).await;
    let units = requests["table"]["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Units")
        .unwrap();
    let units: mt_core::id::FieldId = units["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(cell(&requests, "Laptops for new hires", units), "9");
    // The audit log records schema changes.
    let (_, entries) = call(&app, Method::GET, "/api/audit", None, None).await;
    assert_eq!(entries[0]["action"], "field.create");
}

#[tokio::test]
async fn invalid_fields() {
    let app = app();
    let path = format!("{}/fields", table_path(REQUESTS));
    let add = |spec: Value, user: &'static str| {
        let app = app.clone();
        let path = path.clone();
        async move { call(&app, Method::POST, &path, Some(user), Some(spec)).await }
    };
    let cases = [
        (
            json!({"type": "nope", "name": "x"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"type": "ai_field", "name": "x"}),
            StatusCode::NOT_IMPLEMENTED,
        ),
        (
            json!({"type": "attachment", "name": "x"}),
            StatusCode::NOT_IMPLEMENTED,
        ),
        (
            json!({"type": "number", "name": "Total"}),
            StatusCode::CONFLICT,
        ),
        (
            json!({"type": "number", "name": " "}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({"type": "single_select", "name": "x"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({"type": "code_list", "name": "x"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({"type": "code_list", "name": "x", "refset": "rfs_0000000000000000000000000z"}),
            StatusCode::NOT_FOUND,
        ),
        (
            json!({"type": "link", "name": "x"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({"type": "link", "name": "x", "target": "tbl_0000000000000000000000000z"}),
            StatusCode::NOT_FOUND,
        ),
        (
            json!({"type": "formula", "name": "x", "formula": "{Total} +"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({"type": "formula", "name": "x", "formula": "{Total} + 1"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({"type": "lookup", "name": "x", "formula": "{Total}"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({"type": "auto_number", "name": "x", "pattern": "no sequence"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({"type": "tax_id", "name": "x", "format": "nope"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ];
    for (spec, expected) in cases {
        let (status, body) = add(spec.clone(), ALICE).await;
        assert_eq!(status, expected, "{spec}: {body}");
    }
    let (status, _) = add(json!({"type": "number", "name": "Mine"}), BOB).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // A formula that would close a circle is refused and leaves the table as it was.
    let (status, body) = call(
        &app,
        Method::POST,
        &format!("{}/fields", table_path(LINES)),
        None,
        Some(json!({"type": "lookup", "name": "Request total", "formula": "{Request}.{Total}"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = call(
        &app,
        Method::POST,
        &format!("{}/fields", table_path(LINES)),
        None,
        Some(json!({"type": "rollup", "name": "Loop", "formula": "SUM({Request}.{Total})"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "reading the request total is no circle: {body}"
    );
}

#[tokio::test]
async fn std_field_types_reference_data_and_capabilities() {
    let app = app();
    let (_, types) = call(&app, Method::GET, "/api/field-types", None, None).await;
    assert_eq!(types["groups"].as_array().unwrap().len(), 8);
    assert!(types["search"].as_array().unwrap().is_empty());
    let (_, found) = call(
        &app,
        Method::GET,
        "/api/field-types?q=dropdown&sample=a%40b.example",
        None,
        None,
    )
    .await;
    assert_eq!(found["search"], json!(["single_select"]));
    assert_eq!(found["suggestion"], "email");
    let (_, sets) = call(&app, Method::GET, "/api/refdata", None, None).await;
    let names: Vec<&str> = sets
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 3);
    assert!(names.contains(&"Cost centres"));
    let path = format!("/api/refdata/{}", seed::COST_CENTRES);
    let (_, centres) = call(&app, Method::GET, &path, None, None).await;
    assert_eq!(centres["tree"], true);
    assert_eq!(centres["items"].as_array().unwrap().len(), 8);
    let (_, found) = call(&app, Method::GET, &format!("{path}?q=emea"), None, None).await;
    assert_eq!(found["items"][0]["path"], "Company › Sales › Sales EMEA");
    let (status, _) = call(
        &app,
        Method::GET,
        "/api/refdata/rfs_0000000000000000000000000z",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, catalogue) = call(&app, Method::GET, "/api/capabilities", None, None).await;
    let keys: Vec<&str> = catalogue["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert!(
        keys.contains(&"request.approve")
            && keys.contains(&"supplier.bank.view")
            && keys.contains(&"record.read")
    );
    assert_eq!(catalogue["rules"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn std_access_is_explained() {
    let app = app();
    let (_, requests) = query(&app, ALICE, REQUESTS, json!({})).await;
    let explain = |record: &str, capability: &str, who: &str, for_whom: Option<&str>| {
        let mut path = format!("/api/explain?table={REQUESTS}&capability={capability}&as={who}");
        if !record.is_empty() {
            path.push_str(&format!("&record={}", record_id(&requests, record)));
        }
        if let Some(f) = for_whom {
            path.push_str(&format!("&for={f}"));
        }
        let app = app.clone();
        async move { call(&app, Method::GET, &path, None, None).await.1 }
    };
    let allowed = explain("Laptops for new hires", "request.approve", CAROL, None).await;
    assert_eq!(allowed["allowed"], true, "{allowed}");
    assert_eq!(allowed["chain"][0]["reason"], "Approves up to 10,000");
    assert_eq!(allowed["chain"][0]["scope"], "all records, with a limit");
    assert_eq!(allowed["question"]["record"], "Laptops for new hires");
    let over = explain("Trade fair booth", "request.approve", CAROL, None).await;
    assert_eq!(over["allowed"], false);
    assert_eq!(over["refusals"][0]["code"], "condition_not_met");
    let own = explain("Design tool licences", "request.approve", CAROL, None).await;
    assert_eq!(own["allowed"], false);
    let codes: Vec<&str> = own["refusals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"separation_of_duties"), "{codes:?}");
    let finance = explain("Trade fair booth", "request.approve", DAVE, None).await;
    assert_eq!(finance["allowed"], true);
    assert_eq!(finance["chain"][0]["viaGroup"], true);
    assert_eq!(finance["chain"][0]["holder"], "Finance");
    let agent = explain("Office chairs", "record.read", ASSISTANT, Some(BOB)).await;
    assert_eq!(agent["allowed"], true);
    assert_eq!(agent["question"]["for"], "Bob Buyer");
    let agent = explain("Trade fair booth", "record.read", ASSISTANT, Some(BOB)).await;
    assert_eq!(agent["allowed"], false);
    let table_level = explain("", "record.create", BOB, None).await;
    assert_eq!(table_level["allowed"], true);
    assert_eq!(table_level["chain"][0]["set"], "Requester");
    let (status, _) = call(
        &app,
        Method::GET,
        &format!("/api/explain?table={REQUESTS}&capability=Bad&as={BOB}"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(&app, Method::GET, &format!("/api/explain?table={REQUESTS}&record=rec_0000000000000000000000000z&capability=record.read&as={BOB}"), None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn std_the_audit_chain() {
    let app = app();
    let (status, body) = call(&app, Method::GET, "/api/audit", Some(BOB), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["details"]["refusals"][0]["code"], "no_grant");
    for path in ["/api/audit/verify"] {
        let (status, _) = call(&app, Method::GET, path, Some(CAROL), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    let (_, entries) = call(&app, Method::GET, "/api/audit", Some(DAVE), None).await;
    let entries = entries.as_array().unwrap();
    assert_eq!(entries.len(), 33);
    assert_eq!(entries[0]["seq"], 33, "newest first");
    assert_eq!(
        entries[32]["action"], "template.install",
        "the sample was installed first"
    );
    assert_eq!(entries[0]["action"], "record.transition");
    assert_eq!(entries[0]["actor"], "Dave Finance");
    assert_eq!(entries[0]["details"]["to"], "Ordered");
    assert_eq!(entries[31]["actor"], "Alice Admin");
    assert_eq!(entries[31]["details"]["title"], "Northwind Traders");
    assert_eq!(entries[31]["prev"].as_str().unwrap().len(), 64);
    let (_, verdict) = call(&app, Method::GET, "/api/audit/verify", Some(DAVE), None).await;
    assert_eq!(verdict["intact"], true);
    assert_eq!(verdict["entries"], 33);
    let (_, tampered) = call(
        &app,
        Method::POST,
        "/api/audit/tamper-check",
        Some(DAVE),
        Some(json!({"seq": 7})),
    )
    .await;
    assert_eq!(tampered["intact"], false);
    assert_eq!(tampered["seq"], 7);
    assert_eq!(tampered["lastGood"], 6);
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/audit/tamper-check",
        Some(DAVE),
        Some(json!({"seq": 99})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The real chain is untouched; an agent's change records both identities.
    let (_, lines) = query(&app, ALICE, LINES, json!({})).await;
    let chair = record_id(&lines, "Ergonomic chair");
    let (status, body) = call_as(
        &app,
        Method::PATCH,
        &format!("{}/records/{chair}", table_path(LINES)),
        Some(ASSISTANT),
        Some(BOB),
        Some(json!({"cells": {line::QUANTITY.to_string(): {"text": "12"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, entries) = call(&app, Method::GET, "/api/audit", None, None).await;
    assert_eq!(entries[0]["actor"], "Purchasing assistant");
    assert_eq!(entries[0]["initiator"], "Bob Buyer");
    assert_eq!(entries[0]["details"]["changes"][0]["from"], "6");
    let (_, verdict) = call(&app, Method::GET, "/api/audit/verify", None, None).await;
    assert_eq!(verdict["entries"], 34);
}

#[tokio::test]
async fn std_the_workspace_is_saved_and_reopened() {
    let dir = std::env::temp_dir().join(format!("agenttable-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("data.json");
    let _ = std::fs::remove_file(&file);
    let app = mt_server::open(
        Some(file.clone()),
        Clock::Fixed(now()),
        mt_server::Start::Demo,
    )
    .unwrap();
    assert!(!file.exists(), "nothing is written before the first change");
    let (_, lines) = query(&app, ALICE, LINES, json!({})).await;
    let chair = record_id(&lines, "Ergonomic chair");
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("{}/records/{chair}", table_path(LINES)),
        None,
        Some(json!({"cells": {line::QUANTITY.to_string(): {"text": "8"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(file.exists());
    let reopened = mt_server::open(
        Some(file.clone()),
        Clock::Fixed(now()),
        mt_server::Start::Demo,
    )
    .unwrap();
    let (_, requests) = query(&reopened, ALICE, REQUESTS, json!({})).await;
    assert_eq!(
        cell(&requests, "Office chairs", request::TOTAL),
        "2,764.80 USD"
    );
    let (_, verdict) = call(&reopened, Method::GET, "/api/audit/verify", None, None).await;
    assert_eq!(verdict["entries"], 34);
    // A damaged file is reported, not ignored.
    std::fs::write(&file, b"{ not json").unwrap();
    assert!(mt_server::open(
        Some(file.clone()),
        Clock::Fixed(now()),
        mt_server::Start::Demo
    )
    .is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn std_the_web_app_is_served() {
    let app = app();
    let (status, page) = call(&app, Method::GET, "/", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.as_str().unwrap().contains("<"), "an HTML page");
    let (status, page) = call(&app, Method::GET, "/table/anything", None, None).await;
    assert_eq!(status, StatusCode::OK, "client-side routes get the app");
    assert!(page.as_str().unwrap().contains("<"));
    let (status, _) = call(&app, Method::GET, "/api/nope", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(&app, Method::GET, "/assets/missing.js", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(Clock::System.now() > Timestamp::from_millis(1_700_000_000_000).unwrap());
}

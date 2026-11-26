//! Lifecycles and approvals through the HTTP API, as the people who use them: submitting,
//! routing by the access rules, my tasks, approving, rejecting, sending back, withdrawing,
//! placing orders, locks while approving, the inbox, and what is refused.
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
use mt_server::seed::{line, request, LINES, REQUESTS, SUPPLIERS};
use tower as _;

const ERIN: &str = "prn_00000000000000000000000007";

fn table_path(id: mt_core::id::TableId) -> String {
    format!("/api/tables/{id}")
}

async fn id_of(app: &Arc<App>, title: &str) -> String {
    let (_, rows) = call(
        app,
        Method::POST,
        &format!("{}/query", table_path(REQUESTS)),
        Some(ALICE),
        Some(json!({})),
    )
    .await;
    rows["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["title"] == title)
        .unwrap_or_else(|| panic!("no request {title}"))["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn open(app: &Arc<App>, user: &str, title: &str) -> Value {
    let id = id_of(app, title).await;
    let (status, record) = call(
        app,
        Method::GET,
        &format!("{}/records/{id}", table_path(REQUESTS)),
        Some(user),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{record}");
    record
}

async fn go(app: &Arc<App>, user: &str, title: &str, key: &str) -> (StatusCode, Value) {
    let id = id_of(app, title).await;
    call(
        app,
        Method::POST,
        &format!("{}/records/{id}/transitions/{key}", table_path(REQUESTS)),
        Some(user),
        None,
    )
    .await
}

async fn tasks(app: &Arc<App>, user: &str) -> Vec<Value> {
    let (status, list) = call(app, Method::GET, "/api/tasks", Some(user), None).await;
    assert_eq!(status, StatusCode::OK);
    list.as_array().unwrap().clone()
}

async fn task_titles(app: &Arc<App>, user: &str) -> Vec<String> {
    let mut titles: Vec<String> = tasks(app, user)
        .await
        .iter()
        .map(|t| t["title"].as_str().unwrap().to_owned())
        .collect();
    titles.sort();
    titles
}

async fn decide(app: &Arc<App>, user: &str, title: &str, act: Value) -> (StatusCode, Value) {
    let task = tasks(app, user)
        .await
        .into_iter()
        .find(|t| t["title"] == title)
        .unwrap_or_else(|| panic!("{user} has no task for {title}"));
    call(
        app,
        Method::POST,
        &format!("/api/approvals/{}/tasks/{}", task["run"], task["task"]),
        Some(user),
        Some(act),
    )
    .await
}

async fn inbox(app: &Arc<App>, user: &str) -> Vec<String> {
    let (_, list) = call(app, Method::GET, "/api/inbox", Some(user), None).await;
    list.as_array()
        .unwrap()
        .iter()
        .map(|n| n["text"].as_str().unwrap().to_owned())
        .collect()
}

fn state(record: &Value) -> &str {
    record["process"]["state"]["name"].as_str().unwrap()
}

fn offer<'a>(record: &'a Value, key: &str) -> &'a Value {
    record["process"]["transitions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["key"] == key)
        .unwrap_or_else(|| panic!("no transition {key}: {}", record["process"]))
}

fn field<'a>(record: &'a Value, name: &str) -> &'a Value {
    record["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap()
}

// ---- the demo, as seeded ----

#[tokio::test]
async fn std_the_demo_requests_went_through_the_real_process() {
    let app = app();
    for (title, expected) in [
        ("Laptops for new hires", "Submitted"),
        ("Office chairs", "Draft"),
        ("Design tool licences", "Approved"),
        ("Trade fair booth", "Submitted"),
        ("Printer toner", "Ordered"),
    ] {
        assert_eq!(state(&open(&app, ALICE, title).await), expected, "{title}");
    }
    // Who waits for whom: within limits Carol or Dave; Carol's own request skips her.
    assert_eq!(
        task_titles(&app, CAROL).await,
        ["Laptops for new hires"],
        "Carol never approves her own requests"
    );
    assert_eq!(task_titles(&app, DAVE).await, ["Laptops for new hires"]);
    assert_eq!(task_titles(&app, ERIN).await, ["Trade fair booth"]);
    assert!(tasks(&app, BOB).await.is_empty());
    let booth = open(&app, ALICE, "Trade fair booth").await;
    let approval = &booth["process"]["approval"];
    assert_eq!(approval["status"], "running");
    let steps: Vec<(&str, &str, &str)> = approval["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["step"].as_str().unwrap(),
                t["assignee"].as_str().unwrap(),
                t["status"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        steps,
        [
            ("Approver within limits", "Dave Finance", "approved"),
            ("CFO", "Erin CFO", "open"),
        ]
    );
    // The inbox tells people what is waiting for them.
    assert!(inbox(&app, ERIN)
        .await
        .contains(&"Carol Controller asks you to approve “Trade fair booth” (CFO)".to_owned()));
    assert!(inbox(&app, BOB)
        .await
        .contains(&"“Printer toner” was approved".to_owned()));
    // The process is described for the process page.
    let (_, definitions) = call(&app, Method::GET, "/api/process", None, None).await;
    let lifecycle = &definitions["lifecycles"][0];
    assert_eq!(lifecycle["table"], "Purchase requests");
    assert_eq!(lifecycle["states"].as_array().unwrap().len(), 6);
    let steps = &definitions["approvals"][0]["steps"];
    assert_eq!(steps[0]["who"][0], "whoever may “request.approve” it");
    assert_eq!(steps[1]["cases"][0]["then"][0]["who"][0], "Erin CFO");
    assert_eq!(steps[2]["kind"], "notify");
}

// ---- standard flow ----

#[tokio::test]
async fn std_submit_approve_and_order() {
    let app = app();
    // Bob's draft: he may submit or cancel; it would go to Carol or Dave.
    let chairs = open(&app, BOB, "Office chairs").await;
    assert_eq!(state(&chairs), "Draft");
    assert_eq!(offer(&chairs, "submit")["allowed"], true);
    assert_eq!(offer(&chairs, "submit")["startsApproval"], true);
    assert_eq!(offer(&chairs, "cancel")["allowed"], true);
    assert_eq!(
        chairs["process"]["wouldAsk"]["steps"][0]["people"],
        json!(["Carol Controller", "Dave Finance"])
    );
    assert_eq!(chairs["process"]["wouldAsk"]["outcome"], "approved");
    // Carol may read it but not submit it.
    let seen_by_carol = open(&app, CAROL, "Office chairs").await;
    assert_eq!(offer(&seen_by_carol, "submit")["allowed"], false);
    assert_eq!(
        offer(&seen_by_carol, "submit")["reasons"][0],
        "Carol Controller may not change this record"
    );

    let (status, submitted) = go(&app, BOB, "Office chairs", "submit").await;
    assert_eq!(status, StatusCode::OK, "{submitted}");
    assert_eq!(state(&submitted), "Submitted");
    assert_eq!(submitted["process"]["approval"]["status"], "running");
    // While it is being approved, nothing of it changes.
    assert_eq!(field(&submitted, "Title")["editable"], false);
    assert_eq!(
        field(&submitted, "Title")["lockedReason"],
        "locked while Submitted"
    );
    assert_eq!(field(&submitted, "Status")["editable"], false);
    let id = id_of(&app, "Office chairs").await;
    let (status, body) = call(
        &app,
        Method::PATCH,
        &format!("{}/records/{id}", table_path(REQUESTS)),
        Some(BOB),
        Some(json!({"cells": {request::TITLE.to_string(): {"text": "Chairs!"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        body["message"],
        "“Title” cannot change while “Office chairs” is Submitted"
    );
    assert!(task_titles(&app, CAROL)
        .await
        .contains(&"Office chairs".to_owned()));
    assert!(task_titles(&app, DAVE)
        .await
        .contains(&"Office chairs".to_owned()));
    assert!(inbox(&app, CAROL).await.contains(
        &"Bob Buyer asks you to approve “Office chairs” (Approver within limits)".to_owned()
    ));

    // Carol approves: one approver is enough; Dave's task is no longer needed.
    let (status, approved) = decide(
        &app,
        CAROL,
        "Office chairs",
        json!({"act": "approve", "comment": "Fine"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    assert_eq!(state(&approved), "Approved");
    let approval = &approved["process"]["approval"];
    assert_eq!(approval["status"], "approved");
    let statuses: Vec<&str> = approval["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["approved", "no longer needed"]);
    assert_eq!(approval["tasks"][0]["comment"], "Fine");
    assert!(!task_titles(&app, DAVE)
        .await
        .contains(&"Office chairs".to_owned()));
    assert_eq!(inbox(&app, BOB).await[0], "“Office chairs” was approved");
    assert!(inbox(&app, DAVE)
        .await
        .contains(&"“Office chairs”: Approved — ready to order".to_owned()));
    // Approved: only the date may still change; Bob cannot place the order, Dave can.
    let bob_view = open(&app, BOB, "Office chairs").await;
    assert_eq!(field(&bob_view, "Needed by")["editable"], true);
    assert_eq!(field(&bob_view, "Title")["editable"], false);
    assert_eq!(offer(&bob_view, "order")["allowed"], false);
    let (status, _) = go(&app, BOB, "Office chairs", "order").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, ordered) = go(&app, DAVE, "Office chairs", "order").await;
    assert_eq!(status, StatusCode::OK, "{ordered}");
    assert_eq!(state(&ordered), "Ordered");
    assert!(
        ordered["process"]["transitions"]
            .as_array()
            .unwrap()
            .is_empty(),
        "final"
    );
    // The audit log tells the story.
    let (_, entries) = call(&app, Method::GET, "/api/audit", Some(DAVE), None).await;
    let actions: Vec<&str> = entries
        .as_array()
        .unwrap()
        .iter()
        .take(5)
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    assert_eq!(
        actions,
        [
            "record.transition",
            "record.transition",
            "approval.approve",
            "approval.start",
            "record.transition",
        ]
    );
    let (_, verdict) = call(&app, Method::GET, "/api/audit/verify", None, None).await;
    assert_eq!(verdict["intact"], true);
}

#[tokio::test]
async fn std_the_cfo_step_and_sending_back() {
    let app = app();
    // Erin sends the booth back: it is a draft again, Carol is told.
    let (status, body) = decide(
        &app,
        ERIN,
        "Trade fair booth",
        json!({"act": "return", "comment": "Get a second quote"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(state(&body), "Draft");
    assert_eq!(body["process"]["approval"]["status"], "sent back");
    assert_eq!(
        inbox(&app, CAROL).await[0],
        "“Trade fair booth” was sent back to you by Erin CFO"
    );
    // Resubmitted: Dave first again, then Erin.
    let (status, _) = go(&app, CAROL, "Trade fair booth", "submit").await;
    assert_eq!(status, StatusCode::OK);
    assert!(tasks(&app, ERIN).await.is_empty());
    decide(&app, DAVE, "Trade fair booth", json!({"act": "approve"})).await;
    assert_eq!(task_titles(&app, ERIN).await, ["Trade fair booth"]);
    let (status, body) = decide(&app, ERIN, "Trade fair booth", json!({"act": "approve"})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state(&body), "Approved");
    // Every approval of the request is listed, newest first.
    let (_, runs) = call(&app, Method::GET, "/api/approvals", Some(CAROL), None).await;
    let booth: Vec<&str> = runs
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["title"] == "Trade fair booth")
        .map(|r| r["status"].as_str().unwrap())
        .collect();
    assert_eq!(booth, ["approved", "sent back"]);
}

#[tokio::test]
async fn std_rejection_revision_withdrawal_and_cancellation() {
    let app = app();
    // A rejection needs a reason.
    let (status, body) = decide(
        &app,
        DAVE,
        "Laptops for new hires",
        json!({"act": "reject", "comment": " "}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["message"], "give a reason");
    let (status, body) = decide(
        &app,
        DAVE,
        "Laptops for new hires",
        json!({"act": "reject", "comment": "Buy refurbished"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state(&body), "Rejected");
    assert_eq!(
        inbox(&app, BOB).await[0],
        "“Laptops for new hires” was rejected by Dave Finance"
    );
    assert!(
        task_titles(&app, CAROL).await.is_empty(),
        "Carol's task closed"
    );
    // Bob revises it: a draft again, editable again (its lines too).
    let (status, body) = go(&app, BOB, "Laptops for new hires", "revise").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state(&body), "Draft");
    assert_eq!(field(&body, "Title")["editable"], true);
    // Submitted again, then withdrawn: the approval is cancelled and nobody waits.
    go(&app, BOB, "Laptops for new hires", "submit").await;
    assert_eq!(task_titles(&app, CAROL).await, ["Laptops for new hires"]);
    let (status, body) = go(&app, BOB, "Laptops for new hires", "withdraw").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state(&body), "Draft");
    assert_eq!(body["process"]["approval"]["status"], "cancelled");
    assert!(tasks(&app, CAROL).await.is_empty());
    // Cancelled from any open state; then it may be deleted, unlike an ordered one.
    let (status, body) = go(&app, BOB, "Laptops for new hires", "cancel").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state(&body), "Cancelled");
    let toner = id_of(&app, "Printer toner").await;
    let (status, body) = call(
        &app,
        Method::DELETE,
        &format!("{}/records/{toner}", table_path(REQUESTS)),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        body["message"],
        "“Printer toner” is Ordered: only drafts and cancelled records can be deleted"
    );
    let laptops = id_of(&app, "Laptops for new hires").await;
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("{}/records/{laptops}", table_path(REQUESTS)),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

// ---- invalid and anomalous ----

#[tokio::test]
async fn invalid_transitions_are_refused_with_every_reason() {
    let app = app();
    // An empty draft cannot be submitted: no supplier, no lines, no total.
    let (status, created) = call(
        &app,
        Method::POST,
        &format!("{}/records", table_path(REQUESTS)),
        Some(BOB),
        Some(json!({"cells": {request::TITLE.to_string(): {"text": "Monitors"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(state(&created), "Draft", "new records start as drafts");
    let submit = offer(&created, "submit");
    assert_eq!(submit["allowed"], false);
    assert_eq!(
        submit["reasons"],
        json!([
            "needs “Total” > 0.00 USD",
            "“Supplier” is required",
            "“Lines” is required",
        ])
    );
    let (status, body) = go(&app, BOB, "Monitors", "submit").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["details"].as_array().unwrap().len(), 3);
    // Steps the approval takes cannot be taken by hand.
    let (status, body) = go(&app, ALICE, "Laptops for new hires", "submit_approved").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["message"], "“Approved” is decided by the approval");
    // Unknown transitions, wrong states, tables without a lifecycle.
    let (status, _) = go(&app, ALICE, "Office chairs", "teleport").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = go(&app, ALICE, "Office chairs", "order").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("not possible while Draft"));
    let (_, suppliers) = call(
        &app,
        Method::POST,
        &format!("{}/query", table_path(SUPPLIERS)),
        None,
        Some(json!({})),
    )
    .await;
    let northwind = suppliers["rows"][0]["id"].as_str().unwrap().to_owned();
    let (status, _) = call(
        &app,
        Method::POST,
        &format!(
            "{}/records/{northwind}/transitions/submit",
            table_path(SUPPLIERS)
        ),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    // The state is not a field to edit, and records do not start elsewhere.
    let chairs = id_of(&app, "Office chairs").await;
    let approved = mt_server::seed::status::APPROVED.to_string();
    let (status, body) = call(
        &app,
        Method::PATCH,
        &format!("{}/records/{chairs}", table_path(REQUESTS)),
        None,
        Some(json!({"cells": {request::STATUS.to_string(): {"value": {"type": "reference", "value": approved}}}})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        body["message"],
        "“Status” changes through its buttons, not by editing"
    );
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("{}/records", table_path(REQUESTS)),
        None,
        Some(json!({"cells": {request::TITLE.to_string(): {"text": "Sneaky"}, request::STATUS.to_string(): {"value": {"type": "reference", "value": approved}}}})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    // Gina cannot even see requests.
    let (status, _) = go(&app, GINA, "Office chairs", "submit").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn invalid_acts_on_tasks_are_refused() {
    let app = app();
    let task = tasks(&app, CAROL).await.remove(0);
    let path = format!("/api/approvals/{}/tasks/{}", task["run"], task["task"]);
    // Not hers.
    let (status, body) = call(
        &app,
        Method::POST,
        &path,
        Some(BOB),
        Some(json!({"act": "approve"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body["message"].as_str().unwrap().contains("not yours"));
    // An agent acting for Carol does not decide for her.
    let (status, body) = call_as(
        &app,
        Method::POST,
        &path,
        Some(ASSISTANT),
        Some(CAROL),
        Some(json!({"act": "approve"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("not by an agent"));
    let (_, agent_tasks) = call_as(
        &app,
        Method::GET,
        "/api/tasks",
        Some(ASSISTANT),
        Some(CAROL),
        None,
    )
    .await;
    assert!(agent_tasks.as_array().unwrap().is_empty());
    // Unknown approvals and tasks; acts that do not fit.
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/approvals/999/tasks/1",
        Some(CAROL),
        Some(json!({"act": "approve"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let wrong_task = format!("/api/approvals/{}/tasks/99", task["run"]);
    let (status, _) = call(
        &app,
        Method::POST,
        &wrong_task,
        Some(CAROL),
        Some(json!({"act": "approve"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = call(
        &app,
        Method::POST,
        &path,
        Some(CAROL),
        Some(json!({"act": "submit"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &app,
        Method::POST,
        &path,
        Some(CAROL),
        Some(json!({"act": "dance"})),
    )
    .await;
    assert!(status.is_client_error());
    // Comments and delegation are recorded; the requester never receives the task.
    let (status, body) = call(
        &app,
        Method::POST,
        &path,
        Some(CAROL),
        Some(json!({"act": "comment", "text": "Which model?"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let history = body["process"]["approval"]["history"].as_array().unwrap();
    assert_eq!(history.last().unwrap()["what"], "commented: Which model?");
    let (status, _) = call(
        &app,
        Method::POST,
        &path,
        Some(CAROL),
        Some(json!({"act": "delegate", "to": BOB})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = call(
        &app,
        Method::POST,
        &path,
        Some(CAROL),
        Some(json!({"act": "delegate", "to": ERIN})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["process"]["approval"]["history"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["what"],
        "delegated to Erin CFO"
    );
    assert_eq!(
        task_titles(&app, ERIN).await,
        ["Laptops for new hires", "Trade fair booth"]
    );
    assert_eq!(
        inbox(&app, ERIN).await[0],
        "“Laptops for new hires” was handed to you (from Carol Controller)"
    );
    // Acting on a finished approval.
    decide(
        &app,
        ERIN,
        "Laptops for new hires",
        json!({"act": "approve"}),
    )
    .await;
    let (status, _) = call(
        &app,
        Method::POST,
        &path,
        Some(ERIN),
        Some(json!({"act": "approve"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn anomalous_nobody_may_approve_a_huge_request() {
    let app = app();
    // 200 chairs at 1,000: above every approval limit.
    let chairs = id_of(&app, "Office chairs").await;
    let (_, lines) = call(
        &app,
        Method::POST,
        &format!("{}/query", table_path(LINES)),
        None,
        Some(json!({"search": "chair"})),
    )
    .await;
    let line_id = lines["rows"][0]["id"].as_str().unwrap().to_owned();
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("{}/records/{line_id}", table_path(LINES)),
        Some(BOB),
        Some(json!({"cells": {line::QUANTITY.to_string(): {"text": "200"}, line::PRICE.to_string(): {"text": "1000 USD"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let record = open(&app, BOB, "Office chairs").await;
    assert_eq!(record["process"]["wouldAsk"]["outcome"], "stuck");
    let (status, body) = go(&app, BOB, "Office chairs", "submit").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["process"]["approval"]["status"], "stuck");
    assert!(inbox(&app, ALICE).await[0]
        .starts_with("“Office chairs” is stuck at “Approver within limits”"));
    // The requester can still withdraw it.
    let (status, body) = call(
        &app,
        Method::POST,
        &format!(
            "{}/records/{chairs}/transitions/withdraw",
            table_path(REQUESTS)
        ),
        Some(BOB),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["process"]["approval"]["status"], "cancelled");
}

#[tokio::test]
async fn edge_lines_of_frozen_requests_cannot_move_in_or_out() {
    let app = app();
    let (_, lines) = call(
        &app,
        Method::POST,
        &format!("{}/query", table_path(LINES)),
        None,
        Some(json!({})),
    )
    .await;
    let id = |title: &str| {
        lines["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["title"] == title)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let chairs = id_of(&app, "Office chairs").await;
    // A draft cannot take a line from a submitted request.
    let mut wanted = vec![id("Ergonomic chair")];
    wanted.push(id("Wireless mouse"));
    let (status, body) = call(
        &app,
        Method::PATCH,
        &format!("{}/records/{chairs}", table_path(REQUESTS)),
        None,
        Some(json!({"cells": {request::LINES.to_string(): {"value": {"type": "link", "value": wanted}}}})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("Laptops for new hires"));
    // New lines cannot join it, nor can its lines be deleted.
    let laptops = id_of(&app, "Laptops for new hires").await;
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("{}/records", table_path(LINES)),
        None,
        Some(json!({"cells": {line::ITEM.to_string(): {"text": "Bag"}, line::QUANTITY.to_string(): {"text": "1"}, line::REQUEST.to_string(): {"value": {"type": "link", "value": [laptops]}}}})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("{}/records/{}", table_path(LINES), id("Wireless mouse")),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    // Lines of drafts are free.
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("{}/records/{}", table_path(LINES), id("Ergonomic chair")),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn std_approvals_survive_a_restart() {
    let dir = std::env::temp_dir().join(format!("agenttable-process-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("data.json");
    let app = mt_server::open(
        Some(file.clone()),
        Clock::Fixed(now()),
        mt_server::Start::Demo,
    )
    .unwrap();
    go(&app, BOB, "Office chairs", "submit").await;
    let reopened = mt_server::open(
        Some(file.clone()),
        Clock::Fixed(now()),
        mt_server::Start::Demo,
    )
    .unwrap();
    assert_eq!(
        task_titles(&reopened, CAROL).await,
        ["Laptops for new hires", "Office chairs"]
    );
    let (status, body) = decide(&reopened, CAROL, "Office chairs", json!({"act": "approve"})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state(&body), "Approved");
    // Bob's view of the approvals: his own requests only; Gina sees none.
    let (_, mine) = call(&reopened, Method::GET, "/api/approvals", Some(BOB), None).await;
    let titles: Vec<&str> = mine
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["title"].as_str().unwrap())
        .collect();
    assert!(
        titles
            .iter()
            .all(|t| ["Office chairs", "Laptops for new hires", "Printer toner"].contains(t)),
        "{titles:?}"
    );
    let (_, none) = call(&reopened, Method::GET, "/api/approvals", Some(GINA), None).await;
    assert!(none.as_array().unwrap().is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

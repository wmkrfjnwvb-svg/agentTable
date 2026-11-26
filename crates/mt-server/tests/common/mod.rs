//! Shared helpers of the API tests: the demo app on a fixed clock, calls as someone, queries.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use axum::body::Body;
pub use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
pub use mt_core::time::Timestamp;
pub use mt_server::api::{App, Clock};
pub use serde_json::{json, Value};
pub use std::sync::Arc;
use tower::ServiceExt;

pub const ALICE: &str = "prn_00000000000000000000000001";
pub const BOB: &str = "prn_00000000000000000000000002";
pub const CAROL: &str = "prn_00000000000000000000000003";
pub const DAVE: &str = "prn_00000000000000000000000004";
pub const GINA: &str = "prn_00000000000000000000000005";
pub const ASSISTANT: &str = "prn_00000000000000000000000006";
pub const TEAM: &str = "prn_0000000000000000000000000m";

pub fn now() -> Timestamp {
    // 2026-11-18T10:00:00Z, a Wednesday (searching "laptop" alone also finds the lines of
    // "Laptops for new hires": links are searched by their titles).
    Timestamp::from_millis(1_794_996_000_000).unwrap()
}

pub fn app() -> Arc<App> {
    mt_server::open(None, Clock::Fixed(now()), mt_server::Start::Demo).unwrap()
}

pub async fn call(
    app: &Arc<App>,
    method: Method,
    path: &str,
    user: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    call_as(app, method, path, user, None, body).await
}

pub async fn call_as(
    app: &Arc<App>,
    method: Method,
    path: &str,
    user: Option<&str>,
    for_whom: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(user) = user {
        builder = builder.header("x-user", user);
    }
    if let Some(for_whom) = for_whom {
        builder = builder.header("x-on-behalf-of", for_whom);
    }
    let request = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let response = mt_server::router(app.clone())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, value)
}

pub fn table_path(id: mt_core::id::TableId) -> String {
    format!("/api/tables/{id}")
}

pub async fn query(
    app: &Arc<App>,
    user: &str,
    table: mt_core::id::TableId,
    spec: Value,
) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        &format!("{}/query", table_path(table)),
        Some(user),
        Some(spec),
    )
    .await
}

pub fn titles(result: &Value) -> Vec<String> {
    result["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["title"].as_str().unwrap().to_owned())
        .collect()
}

pub fn cell(result: &Value, title: &str, field: mt_core::id::FieldId) -> String {
    let row = result["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["title"] == title)
        .unwrap();
    row["cells"][field.to_string()]["d"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

pub fn record_id(result: &Value, title: &str) -> String {
    result["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["title"] == title)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

//! The product kept in a database — on SQLite, MySQL and PostgreSQL: restarts, imports,
//! fencing between two servers, and starting over.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_server::persist::{Opening, TENANT};
use mt_server::seed::{request, LINES, REQUESTS};
use mt_store::{Store, StoreConfig};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// SQLite always; MySQL and PostgreSQL when their URLs are set (required in CI).
fn configs() -> Vec<StoreConfig> {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let prefix = format!("mts{}_{n}_", std::process::id());
    let dir = std::env::temp_dir().join(format!("mt-server-db-tests/{prefix}"));
    let mut configs = vec![StoreConfig {
        prefix: prefix.clone(),
        ..StoreConfig::new(format!("sqlite://{}", dir.display()))
    }];
    let require = std::env::var("MT_TEST_REQUIRE_ALL_DBS").is_ok_and(|v| v == "1");
    for var in ["MT_TEST_MYSQL_URL", "MT_TEST_POSTGRES_URL"] {
        match std::env::var(var) {
            Ok(url) if !url.is_empty() => configs.push(StoreConfig {
                prefix: prefix.clone(),
                ..StoreConfig::new(url)
            }),
            Ok(_) | Err(_) => assert!(!require, "{var} is not set"),
        }
    }
    configs
}

async fn open(config: &StoreConfig, fresh: bool, start: mt_server::Start, millis: i64) -> Arc<App> {
    let opening = Opening {
        config: config.clone(),
        fresh,
        import: None,
        import_if_new: false,
    };
    mt_server::open_database(
        opening,
        Clock::Fixed(Timestamp::from_millis(millis).unwrap()),
        start,
    )
    .await
    .unwrap_or_else(|e| panic!("{}: {e}", config.url))
}

async fn clean(config: &StoreConfig) {
    Store::open(config, now().millis())
        .await
        .unwrap()
        .destroy()
        .await
        .unwrap();
}

async fn rows(app: &Arc<App>, table: mt_core::id::TableId) -> Value {
    query(app, ALICE, table, json!({})).await.1
}

#[tokio::test]
async fn std_the_workspace_survives_a_restart_in_every_database() {
    for config in configs() {
        let app = open(&config, true, mt_server::Start::Demo, now().millis()).await;
        assert!(app.persist.engine().is_some());
        let before = rows(&app, REQUESTS).await;
        let lines_before = rows(&app, LINES).await["rows"].as_array().unwrap().len();
        // A new request, an edit, a deletion and a submission through the API.
        let (status, made) = call(
            &app,
            Method::POST,
            &format!("{}/records", table_path(REQUESTS)),
            Some(BOB),
            Some(json!({"cells": {request::TITLE.to_string(): {"text": "Standing desks"}}})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{made}");
        let desks = made["id"].as_str().unwrap().to_owned();
        let (status, _) = call(
            &app,
            Method::PATCH,
            &format!("{}/records/{desks}", table_path(REQUESTS)),
            Some(BOB),
            Some(json!({"cells": {request::TITLE.to_string(): {"text": "Standing desks (4)"}}})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let chairs = record_id(&before, "Office chairs");
        let (status, submitted) = call(
            &app,
            Method::POST,
            &format!(
                "{}/records/{chairs}/transitions/submit",
                table_path(REQUESTS)
            ),
            Some(BOB),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{submitted}");
        let line = rows(&app, LINES).await["rows"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let (status, _) = call(
            &app,
            Method::DELETE,
            &format!("{}/records/{line}", table_path(LINES)),
            Some(ALICE),
            None,
        )
        .await;
        assert!(
            status == StatusCode::OK || status == StatusCode::CONFLICT,
            "{status}"
        );
        let deleted = status == StatusCode::OK;
        let after = rows(&app, REQUESTS).await;
        let (_, desk_detail) = call(
            &app,
            Method::GET,
            &format!("{}/records/{desks}", table_path(REQUESTS)),
            Some(ALICE),
            None,
        )
        .await;
        app.persist.close().await;
        drop(app);
        // A new server process on the same database: everything is there.
        let again = open(
            &config,
            false,
            mt_server::Start::Empty,
            now().millis() + 60_000,
        )
        .await;
        assert_eq!(rows(&again, REQUESTS).await, after, "{}", config.url);
        assert_eq!(
            rows(&again, LINES).await["rows"].as_array().unwrap().len(),
            lines_before - usize::from(deleted)
        );
        let (_, reloaded) = call(
            &again,
            Method::GET,
            &format!("{}/records/{desks}", table_path(REQUESTS)),
            Some(ALICE),
            None,
        )
        .await;
        for key in [
            "createdBy",
            "createdAt",
            "modifiedBy",
            "modifiedAt",
            "version",
            "title",
        ] {
            assert_eq!(reloaded[key], desk_detail[key], "{key}");
        }
        let (_, chairs_now) = call(
            &again,
            Method::GET,
            &format!("{}/records/{chairs}", table_path(REQUESTS)),
            Some(ALICE),
            None,
        )
        .await;
        assert_eq!(
            chairs_now["process"]["state"]["name"], "Submitted",
            "the approval survives"
        );
        let (_, tasks) = call(
            &again,
            Method::GET,
            "/api/tasks",
            Some(mt_server::seed::CAROL.to_string().as_str()),
            None,
        )
        .await;
        assert!(
            tasks.to_string().contains("Office chairs"),
            "the approver still has the task: {tasks}"
        );
        let (_, audit) = call(&again, Method::GET, "/api/audit/verify", Some(ALICE), None).await;
        assert_eq!(audit["intact"], true, "{audit}");
        // The changes are numbered batches in the journal.
        let store = Store::open(&config, now().millis()).await.unwrap();
        let tenant = store.tenant(TENANT, now().millis()).await.unwrap();
        let journal = tenant
            .journal(mt_server::model::BASE, 0, 1_000)
            .await
            .unwrap();
        assert!(journal.len() >= 4, "{}", journal.len());
        assert!(journal
            .iter()
            .enumerate()
            .all(|(i, a)| a.seq == i as u64 + 1));
        store.close().await;
        // Keeps working after the restart.
        let (status, _) = call(
            &again,
            Method::PATCH,
            &format!("{}/records/{desks}", table_path(REQUESTS)),
            Some(BOB),
            Some(json!({"cells": {request::TITLE.to_string(): {"text": "Standing desks (5)"}}})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        again.persist.close().await;
        clean(&config).await;
    }
}

#[tokio::test]
async fn std_a_workspace_saved_as_json_is_imported() {
    for config in configs() {
        let source = mt_server::open(None, Clock::Fixed(now()), mt_server::Start::Demo).unwrap();
        let data = source.ws.lock().await.data.clone();
        let expected = rows(&source, REQUESTS).await;
        let opening = Opening {
            config: config.clone(),
            fresh: true,
            import: Some(data.clone()),
            import_if_new: false,
        };
        let app = mt_server::open_database(opening, Clock::Fixed(now()), mt_server::Start::Empty)
            .await
            .unwrap();
        assert_eq!(rows(&app, REQUESTS).await, expected, "{}", config.url);
        app.persist.close().await;
        // Importing over an existing workspace is refused.
        let opening = Opening {
            config: config.clone(),
            fresh: false,
            import: Some(data.clone()),
            import_if_new: false,
        };
        let refused =
            mt_server::open_database(opening, Clock::Fixed(now()), mt_server::Start::Empty).await;
        assert!(
            matches!(&refused, Err(e) if e.status == 409),
            "{:?}",
            refused.err()
        );
        // Carried over only into a new database: an existing workspace is kept as it is.
        let opening = Opening {
            config: config.clone(),
            fresh: false,
            import: Some(data),
            import_if_new: true,
        };
        let kept = mt_server::open_database(opening, Clock::Fixed(now()), mt_server::Start::Empty)
            .await
            .unwrap();
        assert_eq!(rows(&kept, REQUESTS).await, expected);
        kept.persist.close().await;
        // Starting over gives a new, empty workspace.
        let fresh = open(&config, true, mt_server::Start::Empty, now().millis()).await;
        let (_, tables) = call(&fresh, Method::GET, "/api/tables", Some(ALICE), None).await;
        assert_eq!(tables.as_array().unwrap().len(), 0);
        fresh.persist.close().await;
        clean(&config).await;
    }
}

/// Two servers on one database: the one that started last owns it; the other's changes are
/// refused instead of overwriting.
#[tokio::test]
async fn edge_a_second_server_fences_the_first_out() {
    for config in configs() {
        let first = open(&config, true, mt_server::Start::Demo, now().millis()).await;
        let second = open(&config, false, mt_server::Start::Empty, now().millis() + 1).await;
        let requests = rows(&first, REQUESTS).await;
        let chairs = record_id(&requests, "Office chairs");
        let edit = |title: &str| json!({"cells": {request::TITLE.to_string(): {"text": title}}});
        let path = format!("{}/records/{chairs}", table_path(REQUESTS));
        let (status, body) = call(
            &first,
            Method::PATCH,
            &path,
            Some(BOB),
            Some(edit("From the first")),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{}: {body}",
            config.url
        );
        assert!(body["message"].as_str().unwrap().contains("another server"));
        let (status, _) = call(&first, Method::PATCH, &path, Some(BOB), Some(edit("Again"))).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "it stays fenced");
        let (status, _) = call(
            &second,
            Method::PATCH,
            &path,
            Some(BOB),
            Some(edit("From the second")),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        first.persist.close().await;
        second.persist.close().await;
        let third = open(&config, false, mt_server::Start::Empty, now().millis() + 2).await;
        let titles = titles(&rows(&third, REQUESTS).await);
        assert!(titles.contains(&"From the second".to_owned()));
        assert!(!titles
            .iter()
            .any(|t| t.starts_with("From the first") || t == "Again"));
        third.persist.close().await;
        clean(&config).await;
    }
}

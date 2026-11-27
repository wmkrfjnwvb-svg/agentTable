//! What the store does with damaged data, older schemas, unreachable servers and races —
//! on every engine.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_cluster::NodeId;
use mt_store::{NewJob, Store, StoreConfig, StoreError};
use std::sync::Arc;

#[tokio::test]
async fn anomalous_damaged_rows_are_reported_not_trusted() {
    on_every_db(|store| async move {
        let t = store.create_tenant(tenant(1), "Acme", NOW).await.unwrap();
        // An owner recorded half-way.
        t.acquire(b"base:1", &NodeId::new(1, 1, "a:1"), None)
            .await
            .unwrap();
        sqlx::query("UPDATE _epochs SET owner_addr = NULL")
            .execute(t.pool())
            .await
            .unwrap();
        assert!(
            matches!(t.epoch(b"base:1").await, Err(StoreError::Corrupt(_))),
            "{}",
            name(store.dialect())
        );
        // A negative epoch.
        sqlx::query("UPDATE _epochs SET epoch = -1, owner_id = NULL, owner_incarnation = NULL")
            .execute(t.pool())
            .await
            .unwrap();
        assert!(matches!(
            t.epoch(b"base:1").await,
            Err(StoreError::Corrupt(_))
        ));
        // A lease recorded half-way; a lease set to what it is (no change, still true).
        let lease = mt_cluster::LeaseRecord {
            term: 1,
            holder: Some(NodeId::new(1, 1, "a:1")),
            expires_at_ms: 5,
            cursor_ms: 0,
        };
        assert!(t.compare_and_set_lease(3, None, &lease).await.unwrap());
        assert!(t
            .compare_and_set_lease(3, Some(&lease), &lease)
            .await
            .unwrap());
        assert!(!t
            .compare_and_set_lease(
                3,
                Some(&mt_cluster::LeaseRecord {
                    term: 9,
                    ..lease.clone()
                }),
                &mt_cluster::LeaseRecord {
                    term: 9,
                    ..lease.clone()
                }
            )
            .await
            .unwrap());
        sqlx::query("UPDATE _leases SET holder_addr = NULL")
            .execute(t.pool())
            .await
            .unwrap();
        assert!(matches!(t.lease(3).await, Err(StoreError::Corrupt(_))));
        // A job in a state nobody knows.
        let job = NewJob {
            queue: "q".into(),
            payload: "{}".into(),
            run_at: NOW,
            max_attempts: 1,
            trace: None,
        };
        t.enqueue("job_1", &job, NOW).await.unwrap();
        sqlx::query("UPDATE _jobs SET state = 'lost'")
            .execute(t.pool())
            .await
            .unwrap();
        assert!(matches!(t.job("job_1").await, Err(StoreError::Corrupt(_))));
        // A document with a negative version.
        t.put_document("view", "a", "{}", 0, NOW).await.unwrap();
        sqlx::query("UPDATE _documents SET version = -5")
            .execute(t.pool())
            .await
            .unwrap();
        assert!(matches!(
            t.document("view", "a").await,
            Err(StoreError::Corrupt(_))
        ));
        store
    })
    .await;
}

#[tokio::test]
async fn std_a_tenant_on_an_older_schema_is_brought_up_to_date_when_opened() {
    for config in configs() {
        let store = Store::open(&config, NOW).await.unwrap();
        store.create_tenant(tenant(1), "Acme", NOW).await.unwrap();
        store.close().await;
        let store = Store::open(&config, NOW).await.unwrap();
        sqlx::query("UPDATE tenants SET schema_version = 0")
            .execute(store.control())
            .await
            .unwrap();
        assert_eq!(
            store.info(tenant(1)).await.unwrap().unwrap().schema_version,
            0
        );
        let t = store.tenant(tenant(1), NOW + 5).await.unwrap();
        assert_eq!(
            store.info(tenant(1)).await.unwrap().unwrap().schema_version,
            mt_store::schema::TENANT_VERSION
        );
        // Usable: the migration is idempotent over the existing tables.
        t.put_document("view", "a", "{}", 0, NOW).await.unwrap();
        // A tenant row that is not readable.
        sqlx::query("UPDATE tenants SET id = 'not-an-id'")
            .execute(store.control())
            .await
            .unwrap();
        assert!(matches!(store.tenants().await, Err(StoreError::Corrupt(_))));
        sqlx::query(
            "UPDATE tenants SET id = $1"
                .replace("$1", &format!("'{}'", tenant(1)))
                .as_str(),
        )
        .execute(store.control())
        .await
        .unwrap();
        store.destroy().await.unwrap();
    }
}

#[tokio::test]
async fn anomalous_an_unreachable_server_is_an_error_not_a_hang() {
    for url in [
        "postgres://mt@127.0.0.1:1/postgres",
        "mysql://root@127.0.0.1:1/mysql",
    ] {
        let config = StoreConfig {
            max_connections: 1,
            connect_timeout_ms: 1_000,
            ..StoreConfig::new(url)
        };
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            Store::open(&config, NOW),
        )
        .await;
        assert!(
            matches!(result, Ok(Err(StoreError::Db(_)))),
            "{url}: {result:?}"
        );
    }
    // A file where the directory should be.
    let file = std::env::temp_dir().join(format!("mt-store-file-{}", std::process::id()));
    std::fs::write(&file, b"x").unwrap();
    let result = Store::open(
        &StoreConfig::new(format!("sqlite://{}", file.display())),
        NOW,
    )
    .await;
    assert!(matches!(result, Err(StoreError::Invalid(_))), "{result:?}");
    std::fs::remove_file(&file).unwrap();
    for e in [
        StoreError::Db("x".into()),
        StoreError::Fenced { current: 3 },
        StoreError::LeaseLost("job_1".into()),
        StoreError::Corrupt("x".into()),
    ] {
        assert!(!e.to_string().is_empty());
    }
}

/// Nodes racing for one base: every winner gets its own epoch, the record ends with the
/// last winner, and the losers are told.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrency_nodes_racing_for_ownership_each_get_a_distinct_epoch() {
    on_every_db(|store| async move {
        let t = Arc::new(store.create_tenant(tenant(1), "Acme", NOW).await.unwrap());
        let mut handles = Vec::new();
        for n in 0..8_u64 {
            let t = t.clone();
            handles.push(tokio::spawn(async move {
                let mut won = Vec::new();
                let mut lost = 0;
                for _ in 0..5 {
                    match t
                        .acquire(b"base:race", &NodeId::new(n, 1, format!("n{n}:1")), None)
                        .await
                    {
                        Ok(token) => won.push(token.epoch),
                        Err(StoreError::Conflict(_)) => lost += 1,
                        Err(other) => panic!("{other}"),
                    }
                }
                (won, lost)
            }));
        }
        let mut epochs = Vec::new();
        let mut lost = 0;
        for h in handles {
            let (won, l) = h.await.unwrap();
            epochs.extend(won);
            lost += l;
        }
        epochs.sort_unstable();
        let total = epochs.len();
        epochs.dedup();
        assert_eq!(
            epochs.len(),
            total,
            "{}: two winners of one epoch",
            name(store.dialect())
        );
        assert_eq!(total + lost, 40);
        let record = t.epoch(b"base:race").await.unwrap().unwrap();
        assert_eq!(record.epoch, *epochs.last().unwrap());
        store
    })
    .await;
}

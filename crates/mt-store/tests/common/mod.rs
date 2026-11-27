//! Runs each storage test on SQLite, MySQL and PostgreSQL, each in databases of its own.
#![allow(dead_code)]

use mt_core::id::TenantId;
use mt_store::{Dialect, Store, StoreConfig};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

pub const NOW: i64 = 1_794_996_000_000;

/// The engines to test on: SQLite always; MySQL and PostgreSQL when their URLs are set
/// (`MT_TEST_MYSQL_URL`, `MT_TEST_POSTGRES_URL`). With `MT_TEST_REQUIRE_ALL_DBS=1` (CI) a
/// missing one fails the test instead of being left out.
pub fn configs() -> Vec<StoreConfig> {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let prefix = format!("mtt{}_{n}_", std::process::id());
    let dir = std::env::temp_dir().join(format!("mt-store-tests/{prefix}"));
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

/// Opens a fresh store on every engine, runs the test on each, and removes the databases.
pub async fn on_every_db<F, Fut>(test: F)
where
    F: Fn(Store) -> Fut,
    Fut: std::future::Future<Output = Store>,
{
    for config in configs() {
        let store = Store::open(&config, NOW)
            .await
            .unwrap_or_else(|e| panic!("{}: {e}", config.url));
        let dialect = store.dialect();
        eprintln!("-- {}", dialect.name());
        let store = test(store).await;
        store
            .destroy()
            .await
            .unwrap_or_else(|e| panic!("{}: cleaning up: {e}", dialect.name()));
    }
}

pub fn tenant(n: u128) -> TenantId {
    TenantId::from_u128(n)
}

pub fn name(d: Dialect) -> &'static str {
    d.name()
}

//! The control plane: tenants and their databases, on every engine.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_store::schema::TENANT_VERSION;
use mt_store::{Dialect, Store, StoreConfig, StoreError};

#[tokio::test]
async fn std_tenants_get_a_database_of_their_own() {
    on_every_db(|store| async move {
        let d = store.dialect();
        assert!(store.tenants().await.unwrap().is_empty());
        let acme = store.create_tenant(tenant(1), " Acme ", NOW).await.unwrap();
        assert_eq!(acme.id, tenant(1));
        assert_eq!(acme.dialect(), d);
        store
            .create_tenant(tenant(2), "Globex", NOW + 1)
            .await
            .unwrap();
        let list = store.tenants().await.unwrap();
        assert_eq!(list.len(), 2, "{}", name(d));
        assert_eq!(list[0].name, "Acme");
        assert_eq!(list[0].status, "active");
        assert_eq!(list[0].schema_version, TENANT_VERSION);
        assert_eq!(list[0].created_at, NOW);
        assert!(list[0].db_name.ends_with(&tenant(1).to_string()));
        // Opened again from the list.
        let again = store.tenant(tenant(1), NOW).await.unwrap();
        assert_eq!(again.id, tenant(1));
        assert!(matches!(
            store.create_tenant(tenant(1), "Again", NOW).await,
            Err(StoreError::Exists(_))
        ));
        assert!(matches!(
            store.create_tenant(tenant(3), "  ", NOW).await,
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            store.tenant(tenant(9), NOW).await,
            Err(StoreError::NotFound(_))
        ));
        // Erasure removes the database.
        store.drop_tenant(tenant(2)).await.unwrap();
        assert_eq!(store.tenants().await.unwrap().len(), 1);
        assert!(matches!(
            store.drop_tenant(tenant(2)).await,
            Err(StoreError::NotFound(_))
        ));
        store
    })
    .await;
}

#[tokio::test]
async fn std_a_store_reopens_with_its_tenants() {
    for config in configs() {
        let store = Store::open(&config, NOW).await.unwrap();
        store.create_tenant(tenant(1), "Acme", NOW).await.unwrap();
        store.close().await;
        let reopened = Store::open(&config, NOW + 10).await.unwrap();
        let t = reopened.tenant(tenant(1), NOW + 10).await.unwrap();
        assert_eq!(t.id, tenant(1));
        assert_eq!(
            reopened.info(tenant(1)).await.unwrap().unwrap().name,
            "Acme"
        );
        reopened.destroy().await.unwrap();
    }
}

#[tokio::test]
async fn invalid_configurations_are_refused() {
    for url in ["redis://localhost", "sqlite://", "nothing"] {
        let result = Store::open(&StoreConfig::new(url), NOW).await;
        assert!(
            matches!(result, Err(StoreError::Invalid(_))),
            "{url}: {result:?}"
        );
    }
    let bad_prefix = StoreConfig {
        prefix: "Bad-Prefix".to_owned(),
        ..StoreConfig::new("sqlite:///tmp/x")
    };
    assert!(matches!(
        Store::open(&bad_prefix, NOW).await,
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(
        Dialect::of_url("postgresql://x").unwrap(),
        Dialect::Postgres
    );
    assert_eq!(Dialect::of_url("mariadb://x").unwrap(), Dialect::MySql);
    assert!(Dialect::Sqlite.quote("Robert'); DROP TABLE x;--").is_err());
    assert_eq!(Dialect::MySql.quote("t_x").unwrap(), "`t_x`");
    assert_eq!(Dialect::Postgres.quote("t_x").unwrap(), "\"t_x\"");
    assert_eq!(Dialect::Postgres.params(2, 3), "$2, $3, $4");
    assert_eq!(Dialect::MySql.params(1, 2), "?, ?");
    assert!(mt_store::int(u64::MAX).is_err());
    assert!(mt_store::uint(-1).is_err());
    assert_eq!(mt_store::uint(5).unwrap(), 5);
}

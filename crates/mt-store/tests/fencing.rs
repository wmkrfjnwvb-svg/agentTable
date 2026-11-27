//! Documents, ownership epochs and scheduler leases, on every engine.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_cluster::{
    EpochRecord, EpochStore, LeaseRecord, LeaseStore, MemoryEpochStore, MemoryLeaseStore, NodeId,
};
use mt_store::StoreError;

fn node(id: u64, inc: u64) -> NodeId {
    NodeId::new(id, inc, format!("10.0.0.{id}:7000"))
}

#[tokio::test]
async fn std_documents_keep_an_optimistic_version() {
    on_every_db(|store| async move {
        let t = store.create_tenant(tenant(1), "Acme", NOW).await.unwrap();
        assert!(t.document("workspace", "main").await.unwrap().is_none());
        assert_eq!(
            t.put_document("workspace", "main", r#"{"a":1}"#, 0, NOW)
                .await
                .unwrap(),
            1
        );
        let body = "x".repeat(200_000) + "é漢字";
        assert_eq!(
            t.put_document("workspace", "main", &body, 1, NOW + 1)
                .await
                .unwrap(),
            2
        );
        let doc = t.document("workspace", "main").await.unwrap().unwrap();
        assert_eq!(
            (doc.version, doc.updated_at, doc.body.len()),
            (2, NOW + 1, body.len())
        );
        assert_eq!(doc.body, body, "long and non-ASCII text round-trips");
        // A writer who read version 1 cannot overwrite version 2.
        assert!(matches!(
            t.put_document("workspace", "main", "{}", 1, NOW).await,
            Err(StoreError::Conflict(_))
        ));
        assert!(matches!(
            t.put_document("workspace", "main", "{}", 0, NOW).await,
            Err(StoreError::Conflict(_))
        ));
        t.put_document("view", "b", "{}", 0, NOW).await.unwrap();
        t.put_document("view", "a", "{}", 0, NOW).await.unwrap();
        let views: Vec<String> = t
            .documents("view")
            .await
            .unwrap()
            .into_iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(views, ["a", "b"]);
        assert!(matches!(
            t.delete_document("view", "a", 2).await,
            Err(StoreError::Conflict(_))
        ));
        t.delete_document("view", "a", 1).await.unwrap();
        assert_eq!(t.documents("view").await.unwrap().len(), 1);
        for (kind, id) in [("Bad Kind", "x"), ("view", ""), ("view", "has space")] {
            assert!(
                matches!(t.document(kind, id).await, Err(StoreError::Invalid(_))),
                "{kind}/{id}"
            );
        }
        assert!(matches!(
            t.documents("Bad").await,
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            t.put_document("view", "x", "{}", u64::MAX, NOW).await,
            Err(StoreError::Invalid(_))
        ));
        store
    })
    .await;
}

#[tokio::test]
async fn std_ownership_moves_by_raising_the_epoch() {
    on_every_db(|store| async move {
        let t = store.create_tenant(tenant(1), "Acme", NOW).await.unwrap();
        let base = b"base:1".as_slice();
        assert!(t.epoch(base).await.unwrap().is_none());
        let a = t.acquire(base, &node(1, 1), None).await.unwrap();
        assert_eq!(a.epoch, 1);
        // Idempotent for the holder.
        assert_eq!(t.acquire(base, &node(1, 1), Some(1)).await.unwrap(), a);
        // Another node takes over: the epoch rises.
        let b = t.acquire(base, &node(2, 1), None).await.unwrap();
        assert_eq!(b.epoch, 2);
        assert!(!t.release(&a).await.unwrap(), "A was superseded");
        assert!(t.release(&b).await.unwrap());
        assert_eq!(
            t.epoch(base).await.unwrap(),
            Some(EpochRecord {
                epoch: 2,
                owner: None
            })
        );
        // A restarted node (new incarnation) is another owner.
        let c = t.acquire(base, &node(1, 2), None).await.unwrap();
        assert_eq!(c.epoch, 3);
        // A lost race.
        let stale = EpochRecord {
            epoch: 1,
            owner: Some(node(1, 1)),
        };
        assert!(!t
            .compare_and_set_epoch(
                base,
                Some(&stale),
                &EpochRecord {
                    epoch: 9,
                    owner: None
                }
            )
            .await
            .unwrap());
        assert!(
            !t.compare_and_set_epoch(
                base,
                None,
                &EpochRecord {
                    epoch: 1,
                    owner: None
                }
            )
            .await
            .unwrap(),
            "exists already"
        );
        assert!(matches!(t.epoch(&[]).await, Err(StoreError::Invalid(_))));
        assert!(matches!(
            t.epoch(&[0; 121]).await,
            Err(StoreError::Invalid(_))
        ));
        store
    })
    .await;
}

/// The same compare-and-set requests give the same answers and the same records as
/// `mt-cluster`'s in-memory stores (the reference the simulator uses).
#[tokio::test]
async fn model_epochs_and_leases_behave_like_the_reference() {
    on_every_db(|store| async move {
        let t = store.create_tenant(tenant(1), "Acme", NOW).await.unwrap();
        let mut epochs = MemoryEpochStore::new();
        let mut leases = MemoryLeaseStore::new();
        let mut rng = 0x2545_F491_u64;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        let owners = [None, Some(node(1, 1)), Some(node(2, 1)), Some(node(1, 2))];
        for _ in 0..120 {
            let resource = [b'r', (next() % 3) as u8];
            let current = epochs.load(&resource).unwrap();
            let expected = match next() % 3 {
                0 => current.clone(),
                1 => None,
                _ => Some(EpochRecord {
                    epoch: next() % 4,
                    owner: owners[(next() % 4) as usize].clone(),
                }),
            };
            let new = EpochRecord {
                epoch: next() % 6,
                owner: owners[(next() % 4) as usize].clone(),
            };
            let want = epochs
                .compare_and_set(&resource, expected.as_ref(), new.clone())
                .unwrap();
            let got = t
                .compare_and_set_epoch(&resource, expected.as_ref(), &new)
                .await
                .unwrap();
            assert_eq!(
                got,
                want,
                "{} epoch {resource:?} {expected:?} -> {new:?}",
                name(store.dialect())
            );
            assert_eq!(
                t.epoch(&resource).await.unwrap(),
                epochs.load(&resource).unwrap()
            );

            let shard = (next() % 3) as u32;
            let current = leases.load(shard).unwrap();
            let expected = match next() % 3 {
                0 => current.clone(),
                1 => None,
                _ => Some(LeaseRecord {
                    term: next() % 3,
                    holder: owners[(next() % 4) as usize].clone(),
                    expires_at_ms: next() % 3,
                    cursor_ms: next() % 2,
                }),
            };
            let new = LeaseRecord {
                term: next() % 5,
                holder: owners[(next() % 4) as usize].clone(),
                expires_at_ms: next() % 4,
                cursor_ms: next() % 3,
            };
            let want = leases
                .compare_and_set(shard, expected.as_ref(), new.clone())
                .unwrap();
            let got = t
                .compare_and_set_lease(shard, expected.as_ref(), &new)
                .await
                .unwrap();
            assert_eq!(
                got,
                want,
                "{} lease {shard} {expected:?} -> {new:?}",
                name(store.dialect())
            );
            assert_eq!(t.lease(shard).await.unwrap(), leases.load(shard).unwrap());
        }
        store
    })
    .await;
}

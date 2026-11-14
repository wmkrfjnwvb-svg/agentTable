//! Ownership tests: rendezvous placement, routing, fencing epochs and shard leases.
//! Categories are marked in the test names (`std_`, `anomalous_`, `edge_`, `invalid_`,
//! `concurrency_`, `prop_`, `model_`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use mt_cluster::codec::MAX_WEIGHT;
use mt_cluster::rng::Rng;
use mt_cluster::scheduler::{advance, claim, release, renew, shard_key, shard_of};
use mt_cluster::{
    check_token, moved_keys, ClusterError, EpochRecord, EpochStore, Fencer, FireWindow, KeyMove,
    LeaseRecord, LeaseStore, LeaseToken, MemoryEpochStore, MemoryLeaseStore, NodeId, Placement,
    Route, RoutingTable, SchedEvent, SchedulerConfig, ShardScheduler,
};
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::thread;

fn node(id: u64) -> NodeId {
    NodeId::new(id, 1, format!("n{id}"))
}

fn even(ids: &[u64]) -> Placement {
    Placement::new(ids.iter().map(|id| (node(*id), 1)))
}

fn keys(count: u32) -> Vec<Vec<u8>> {
    (0..count)
        .map(|i| format!("bas_{i:08}").into_bytes())
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Deterministic randomness
// ---------------------------------------------------------------------------------------------

#[test]
fn std_rng_is_deterministic_and_seed_dependent() {
    let mut a = Rng::new(42);
    let mut b = Rng::new(42);
    let mut c = Rng::new(43);
    let first: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
    assert_eq!(first, (0..8).map(|_| b.next_u64()).collect::<Vec<_>>());
    assert_ne!(first, (0..8).map(|_| c.next_u64()).collect::<Vec<_>>());
    // The sequence is part of the reproducibility contract of every simulation seed.
    assert_eq!(Rng::new(0).next_u64(), 0xe220_a839_7b1d_cdaf);
}

#[test]
fn edge_rng_degenerate_ranges() {
    let mut rng = Rng::new(7);
    assert_eq!(rng.below(0), 0);
    assert_eq!(rng.below(1), 0);
    assert_eq!(rng.index(0), 0);
    assert_eq!(rng.range(9, 3), 9);
    assert_eq!(rng.range(5, 5), 5);
    assert_eq!(rng.range(u64::MAX, u64::MAX), u64::MAX);
    for _ in 0..200 {
        assert!(!rng.chance(0));
        assert!(rng.chance(1000));
        assert!((3..=9).contains(&rng.range(3, 9)));
        assert!(rng.index(4) < 4);
    }
}

#[test]
fn std_rng_shuffle_is_a_reproducible_permutation() {
    let mut items: Vec<u32> = (0..50).collect();
    Rng::new(1).shuffle(&mut items);
    let mut again: Vec<u32> = (0..50).collect();
    Rng::new(1).shuffle(&mut again);
    assert_eq!(items, again);
    assert_ne!(items, (0..50).collect::<Vec<_>>());
    items.sort_unstable();
    assert_eq!(items, (0..50).collect::<Vec<_>>());
    let mut empty: Vec<u32> = vec![];
    Rng::new(1).shuffle(&mut empty);
    let mut single = vec![1];
    Rng::new(1).shuffle(&mut single);
    assert_eq!(single, vec![1]);
}

// ---------------------------------------------------------------------------------------------
// Placement
// ---------------------------------------------------------------------------------------------

#[test]
fn std_placement_does_not_depend_on_input_order() {
    let a = even(&[3, 1, 2]);
    let b = even(&[1, 2, 3]);
    assert_eq!(a, b);
    assert_eq!(a.len(), 3);
    assert!(!a.is_empty());
    assert_eq!(
        a.nodes().iter().map(|(n, _)| n.id).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    for key in keys(200) {
        assert_eq!(a.owner(&key), b.owner(&key));
    }
}

#[test]
fn std_duplicate_ids_keep_the_newest_incarnation() {
    let old = NodeId::new(1, 4, "old");
    let new = NodeId::new(1, 9, "new");
    let p = Placement::new([(old.clone(), 1), (new.clone(), 2), (node(2), 1)]);
    assert_eq!(p.len(), 2);
    assert!(p.contains(&new));
    assert!(!p.contains(&old));
    assert_eq!(p.nodes()[0], (new, 2));
}

#[test]
fn edge_empty_placement_and_zero_weights_own_nothing() {
    let empty = Placement::default();
    assert!(empty.is_empty());
    assert_eq!(empty.owner(b"k"), None);
    assert!(empty.ranked(b"k").is_empty());

    let all_zero = Placement::new([(node(1), 0), (node(2), 0)]);
    assert_eq!(all_zero.owner(b"k"), None);
    assert!(all_zero.ranked(b"k").is_empty());

    // A node of weight 0 is a member but never an owner (a draining or API-only node).
    let one_zero = Placement::new([(node(1), 0), (node(2), 1)]);
    for key in keys(100) {
        assert_eq!(one_zero.owner(&key), Some(&node(2)));
        assert_eq!(one_zero.ranked(&key), vec![&node(2)]);
    }
}

#[test]
fn edge_weights_are_clamped() {
    let p = Placement::new([(node(1), u16::MAX)]);
    assert_eq!(p.nodes()[0].1, MAX_WEIGHT);
    assert_eq!(p.owner(b"k"), Some(&node(1)));
}

#[test]
fn std_ranking_starts_with_the_owner_and_lists_every_node_once() {
    let p = Placement::new([(node(1), 1), (node(2), 3), (node(3), 2), (node(4), 1)]);
    for key in keys(300) {
        let ranked = p.ranked(&key);
        assert_eq!(ranked.first().copied(), p.owner(&key));
        let ids: BTreeSet<u64> = ranked.iter().map(|n| n.id).collect();
        assert_eq!(ids, BTreeSet::from([1, 2, 3, 4]));
        assert_eq!(ranked.len(), 4);
    }
}

#[test]
fn std_ranking_gives_the_failover_order() {
    // The second-ranked node is exactly the owner once the first is removed.
    let p = even(&[1, 2, 3, 4, 5]);
    for key in keys(300) {
        let ranked = p.ranked(&key);
        let without_owner = Placement::new(
            p.nodes()
                .iter()
                .filter(|(n, _)| n != ranked[0])
                .cloned()
                .collect::<Vec<_>>(),
        );
        assert_eq!(without_owner.owner(&key), Some(ranked[1]));
    }
}

#[test]
fn std_equal_weights_balance_and_weights_are_proportional() {
    let all = keys(10_000);
    let p = even(&[1, 2, 3, 4, 5]);
    let mut counts: BTreeMap<u64, u32> = BTreeMap::new();
    for key in &all {
        *counts.entry(p.owner(key).unwrap().id).or_default() += 1;
    }
    assert_eq!(counts.len(), 5);
    for (id, count) in &counts {
        assert!(
            (1_600..=2_400).contains(count),
            "node {id} owns {count} of 10000"
        );
    }

    let weighted = Placement::new([(node(1), 3), (node(2), 1)]);
    let heavy = all
        .iter()
        .filter(|key| weighted.owner(key).unwrap().id == 1)
        .count();
    assert!(
        (7_125..=7_875).contains(&heavy),
        "weight-3 node owns {heavy} of 10000"
    );
}

#[test]
fn std_restart_keeps_keys_on_the_same_stable_id_but_counts_as_a_move() {
    let before = even(&[1, 2, 3]);
    let restarted = NodeId::new(2, 2, "n2");
    let after = Placement::new([(node(1), 1), (restarted.clone(), 1), (node(3), 1)]);
    let all = keys(500);
    let moves = moved_keys(all.iter().map(Vec::as_slice), &before, &after);
    assert!(!moves.is_empty());
    for KeyMove { key, from, to } in &moves {
        assert_eq!(from.as_ref(), Some(&node(2)), "key {key:?}");
        assert_eq!(to.as_ref(), Some(&restarted));
    }
    // No key moved between different stable ids.
    for key in &all {
        assert_eq!(
            before.owner(key).map(|n| n.id),
            after.owner(key).map(|n| n.id)
        );
    }
}

#[test]
fn edge_moved_keys_between_empty_and_populated_placements() {
    let all = keys(20);
    let none = Placement::default();
    let some = even(&[1]);
    let moves = moved_keys(all.iter().map(Vec::as_slice), &none, &some);
    assert_eq!(moves.len(), 20);
    assert!(moves
        .iter()
        .all(|m| m.from.is_none() && m.to == Some(node(1))));
    let back = moved_keys(all.iter().map(Vec::as_slice), &some, &none);
    assert!(back
        .iter()
        .all(|m| m.from == Some(node(1)) && m.to.is_none()));
    assert!(moved_keys(all.iter().map(Vec::as_slice), &some, &some).is_empty());
}

fn arb_nodes() -> impl Strategy<Value = Vec<(NodeId, u16)>> {
    proptest::collection::btree_map(1_u64..10_000, 1_u16..5, 2..10)
        .prop_map(|m| m.into_iter().map(|(id, w)| (node(id), w)).collect())
}

fn arb_keys() -> impl Strategy<Value = Vec<Vec<u8>>> {
    proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..24), 1..120)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

    /// Removing a node moves only the keys it owned (minimal disruption).
    #[test]
    fn prop_removing_a_node_moves_only_its_keys(nodes in arb_nodes(), victim in any::<prop::sample::Index>(), keys in arb_keys()) {
        let gone = nodes[victim.index(nodes.len())].0.clone();
        let before = Placement::new(nodes.clone());
        let after = Placement::new(nodes.into_iter().filter(|(n, _)| *n != gone));
        for key in &keys {
            let was = before.owner(key).unwrap();
            if *was == gone {
                prop_assert_ne!(after.owner(key), Some(&gone));
            } else {
                prop_assert_eq!(after.owner(key), Some(was));
            }
        }
    }

    /// Adding a node moves keys only to the new node.
    #[test]
    fn prop_adding_a_node_moves_keys_only_to_it(nodes in arb_nodes(), weight in 1_u16..5, keys in arb_keys()) {
        let newcomer = node(20_000);
        let before = Placement::new(nodes.clone());
        let after = Placement::new(nodes.into_iter().chain([(newcomer.clone(), weight)]));
        for moved in moved_keys(keys.iter().map(Vec::as_slice), &before, &after) {
            prop_assert_eq!(moved.to, Some(newcomer.clone()));
        }
    }

    /// Raising one node's weight never takes keys away from it and never moves keys
    /// between the other nodes.
    #[test]
    fn prop_raising_a_weight_only_attracts_keys(nodes in arb_nodes(), pick in any::<prop::sample::Index>(), keys in arb_keys()) {
        let index = pick.index(nodes.len());
        let grown = nodes[index].0.clone();
        let before = Placement::new(nodes.clone());
        let mut heavier = nodes;
        heavier[index].1 += 3;
        let after = Placement::new(heavier);
        for moved in moved_keys(keys.iter().map(Vec::as_slice), &before, &after) {
            prop_assert_eq!(moved.to, Some(grown.clone()));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Routing
// ---------------------------------------------------------------------------------------------

/// A resource whose rendezvous owner in `placement` is `owner`.
fn key_owned_by(placement: &Placement, owner: u64) -> Vec<u8> {
    keys(10_000)
        .into_iter()
        .find(|key| placement.owner(key).is_some_and(|n| n.id == owner))
        .unwrap()
}

#[test]
fn std_route_follows_the_current_holder_then_the_rendezvous_owner() {
    let placement = even(&[1, 2, 3]);
    let table = RoutingTable::new(node(1), placement.clone());
    assert_eq!(table.placement(), &placement);
    let mine = key_owned_by(&placement, 1);
    let theirs = key_owned_by(&placement, 2);

    // No record known: rendezvous decides.
    assert_eq!(
        table.route(&mine, None, None),
        Route::Local { must_acquire: true }
    );
    assert_eq!(
        table.route(&theirs, None, None),
        Route::Forward { node: node(2) }
    );

    // We hold the current epoch: run locally without acquiring.
    let ours = EpochRecord {
        epoch: 5,
        owner: Some(node(1)),
    };
    assert_eq!(
        table.route(&theirs, Some(&ours), Some(5)),
        Route::Local {
            must_acquire: false
        }
    );

    // Another active node holds it, even though rendezvous would pick us: no stealing.
    let held_by_3 = EpochRecord {
        epoch: 2,
        owner: Some(node(3)),
    };
    assert_eq!(
        table.route(&mine, Some(&held_by_3), None),
        Route::Forward { node: node(3) }
    );
}

#[test]
fn anomalous_route_ignores_holders_that_are_gone_or_stale() {
    let placement = even(&[1, 2, 3]);
    let table = RoutingTable::new(node(1), placement.clone());
    let mine = key_owned_by(&placement, 1);
    let theirs = key_owned_by(&placement, 2);

    // The recorded holder is not an active member any more: rendezvous takes over.
    let held_by_dead = EpochRecord {
        epoch: 7,
        owner: Some(node(9)),
    };
    assert_eq!(
        table.route(&mine, Some(&held_by_dead), None),
        Route::Local { must_acquire: true }
    );
    assert_eq!(
        table.route(&theirs, Some(&held_by_dead), None),
        Route::Forward { node: node(2) }
    );

    // An older instance of an active node is not the active node.
    let held_by_old_instance = EpochRecord {
        epoch: 7,
        owner: Some(NodeId::new(3, 0, "n3")),
    };
    assert_eq!(
        table.route(&mine, Some(&held_by_old_instance), None),
        Route::Local { must_acquire: true }
    );

    // The record names us, but we do not hold that epoch locally (we were reset, or the
    // record is from an epoch we lost): acquire again or forward, never write blindly.
    let names_us = EpochRecord {
        epoch: 7,
        owner: Some(node(1)),
    };
    assert_eq!(
        table.route(&mine, Some(&names_us), Some(6)),
        Route::Local { must_acquire: true }
    );
    assert_eq!(
        table.route(&theirs, Some(&names_us), None),
        Route::Forward { node: node(2) }
    );

    // A released record (no owner) falls back to rendezvous.
    let released = EpochRecord {
        epoch: 7,
        owner: None,
    };
    assert_eq!(
        table.route(&theirs, Some(&released), None),
        Route::Forward { node: node(2) }
    );
}

#[test]
fn edge_route_is_unavailable_without_owner_candidates() {
    let table = RoutingTable::new(node(1), Placement::default());
    assert_eq!(table.route(b"k", None, None), Route::Unavailable);
    let zero = RoutingTable::new(node(1), Placement::new([(node(1), 0)]));
    assert_eq!(zero.route(b"k", None, None), Route::Unavailable);
}

#[test]
fn std_draining_node_forwards_everything_it_does_not_hold() {
    // While draining, the node builds its table from a placement without itself.
    let table = RoutingTable::new(node(1), even(&[2, 3]));
    for key in keys(50) {
        assert!(matches!(
            table.route(&key, None, None),
            Route::Forward { .. }
        ));
    }
}

#[test]
fn std_fan_out_splits_subscribers() {
    let table = RoutingTable::new(node(1), even(&[1, 2, 3]));
    let gone = node(8);
    let subscribers = [node(3), node(1), gone.clone(), node(2), node(3), gone.clone()];
    let fan_out = table.fan_out(&subscribers);
    assert!(fan_out.local);
    assert_eq!(fan_out.remote, vec![node(2), node(3)]);
    assert_eq!(fan_out.gone, vec![gone]);

    let none = table.fan_out(&[]);
    assert!(!none.local && none.remote.is_empty() && none.gone.is_empty());
}

// ---------------------------------------------------------------------------------------------
// In-memory store
// ---------------------------------------------------------------------------------------------

#[test]
fn std_memory_store_compare_and_set() {
    let mut store = MemoryEpochStore::new();
    assert!(store.is_empty());
    let first = EpochRecord {
        epoch: 1,
        owner: Some(node(1)),
    };
    let second = EpochRecord {
        epoch: 2,
        owner: Some(node(2)),
    };
    assert!(store.cas(b"r", None, first.clone()).unwrap());
    // Wrong expectation: nothing changes.
    assert!(!store.cas(b"r", None, second.clone()).unwrap());
    assert!(!store.cas(b"r", Some(&second), second.clone()).unwrap());
    assert_eq!(store.get(b"r").unwrap(), Some(first.clone()));
    assert!(store.cas(b"r", Some(&first), second.clone()).unwrap());
    assert_eq!(store.get(b"r").unwrap(), Some(second));
    assert_eq!(store.get(b"other").unwrap(), None);
    assert_eq!(store.len(), 1);
}

#[test]
fn anomalous_memory_store_fails_exactly_as_many_times_as_asked() {
    let mut store = MemoryEpochStore::new();
    store.fail_next(2);
    assert!(matches!(
        store.get(b"r"),
        Err(ClusterError::StoreUnavailable(_))
    ));
    let record = EpochRecord {
        epoch: 1,
        owner: None,
    };
    assert!(matches!(
        store.cas(b"r", None, record.clone()),
        Err(ClusterError::StoreUnavailable(_))
    ));
    assert!(store.is_empty());
    assert!(store.cas(b"r", None, record).unwrap());
}

// ---------------------------------------------------------------------------------------------
// Fencing epochs
// ---------------------------------------------------------------------------------------------

/// A store in which another node wins the race between `load` and `compare_and_set`.
struct RacyEpochs {
    inner: MemoryEpochStore,
    interloper: Option<EpochRecord>,
}

impl EpochStore for RacyEpochs {
    fn load(&mut self, resource: &[u8]) -> Result<Option<EpochRecord>, ClusterError> {
        self.inner.get(resource)
    }

    fn compare_and_set(
        &mut self,
        resource: &[u8],
        expected: Option<&EpochRecord>,
        new: EpochRecord,
    ) -> Result<bool, ClusterError> {
        if let Some(record) = self.interloper.take() {
            assert!(self.inner.cas(resource, expected, record)?);
        }
        self.inner.cas(resource, expected, new)
    }
}

#[test]
fn std_acquire_write_release_cycle() {
    let mut store = MemoryEpochStore::new();
    let mut fencer = Fencer::new(node(1));
    assert_eq!(fencer.me(), &node(1));
    assert_eq!(fencer.token(b"r"), None);

    let token = fencer.acquire(&mut store, b"r").unwrap();
    assert_eq!((token.epoch, &token.owner), (1, &node(1)));
    assert_eq!(token.resource, b"r".to_vec());
    assert_eq!(fencer.held_epoch(b"r"), Some(1));
    assert_eq!(fencer.token(b"r"), Some(token.clone()));
    assert_eq!(fencer.held().collect::<Vec<_>>(), vec![b"r".as_slice()]);
    assert_eq!(check_token(&mut store, &token), Ok(()));

    // Acquiring again is free: same epoch, no store write.
    assert_eq!(fencer.acquire(&mut store, b"r").unwrap(), token);

    assert!(fencer.release(&mut store, b"r").unwrap());
    assert_eq!(fencer.held_epoch(b"r"), None);
    assert_eq!(
        check_token(&mut store, &token),
        Err(ClusterError::Fenced { current: 1 })
    );
    // Releasing twice is harmless.
    assert!(!fencer.release(&mut store, b"r").unwrap());

    // Acquiring after a release raises the epoch: the old token can never come back.
    let again = fencer.acquire(&mut store, b"r").unwrap();
    assert_eq!(again.epoch, 2);
    assert_eq!(
        check_token(&mut store, &token),
        Err(ClusterError::Fenced { current: 2 })
    );
}

#[test]
fn std_takeover_fences_the_previous_owner() {
    let mut store = MemoryEpochStore::new();
    let mut old = Fencer::new(node(1));
    let mut new = Fencer::new(node(2));
    let old_token = old.acquire(&mut store, b"r").unwrap();
    let new_token = new.acquire(&mut store, b"r").unwrap();
    assert_eq!(new_token.epoch, 2);
    assert_eq!(check_token(&mut store, &new_token), Ok(()));
    let rejected = check_token(&mut store, &old_token).unwrap_err();
    assert!(rejected.is_fenced());
    assert_eq!(rejected, ClusterError::Fenced { current: 2 });

    // The old owner learns it from the rejected write and stops claiming ownership.
    old.on_fenced(b"r");
    assert_eq!(old.token(b"r"), None);
    // Its release is a no-op and cannot disturb the new owner.
    assert!(!old.release(&mut store, b"r").unwrap());
    assert_eq!(check_token(&mut store, &new_token), Ok(()));
}

#[test]
fn anomalous_superseded_owner_that_did_not_notice_cannot_release_or_reuse() {
    let mut store = MemoryEpochStore::new();
    let mut old = Fencer::new(node(1));
    let mut new = Fencer::new(node(2));
    old.acquire(&mut store, b"r").unwrap();
    let new_token = new.acquire(&mut store, b"r").unwrap();
    // `old` still believes it holds epoch 1. Its release does nothing to the record…
    assert!(!old.release(&mut store, b"r").unwrap());
    assert_eq!(check_token(&mut store, &new_token), Ok(()));
    assert_eq!(old.held_epoch(b"r"), None);
    // …and a re-acquire is a real takeover with a higher epoch, not a silent reuse.
    let mut stale = Fencer::new(node(1));
    stale.acquire(&mut store, b"q").unwrap();
    let other = new.acquire(&mut store, b"q").unwrap();
    assert_eq!(other.epoch, 2);
    let retaken = stale.acquire(&mut store, b"q").unwrap();
    assert_eq!(retaken.epoch, 3);
    assert!(check_token(&mut store, &other).is_err());
}

#[test]
fn anomalous_lost_race_is_contended_and_leaves_nothing_held() {
    let mut store = RacyEpochs {
        inner: MemoryEpochStore::new(),
        interloper: Some(EpochRecord {
            epoch: 1,
            owner: Some(node(2)),
        }),
    };
    let mut fencer = Fencer::new(node(1));
    assert_eq!(
        fencer.acquire(&mut store, b"r"),
        Err(ClusterError::Contended)
    );
    assert_eq!(fencer.held_epoch(b"r"), None);
    // The winner's record is intact.
    assert_eq!(
        store.inner.get(b"r").unwrap(),
        Some(EpochRecord {
            epoch: 1,
            owner: Some(node(2))
        })
    );
    // A retry takes over normally.
    assert_eq!(fencer.acquire(&mut store, b"r").unwrap().epoch, 2);
}

#[test]
fn anomalous_store_failures_are_reported_and_retriable() {
    let mut store = MemoryEpochStore::new();
    let mut fencer = Fencer::new(node(1));

    store.fail_next(1);
    assert!(matches!(
        fencer.acquire(&mut store, b"r"),
        Err(ClusterError::StoreUnavailable(_))
    ));
    assert_eq!(fencer.held_epoch(b"r"), None);
    let token = fencer.acquire(&mut store, b"r").unwrap();

    // Failure on the write half of an acquire (load succeeds, compare-and-set fails).
    let mut other = Fencer::new(node(2));
    let mut flaky = FailOnCas {
        inner: store,
        fail: true,
    };
    assert!(matches!(
        other.acquire(&mut flaky, b"r"),
        Err(ClusterError::StoreUnavailable(_))
    ));
    assert_eq!(other.held_epoch(b"r"), None);
    let mut store = flaky.inner;
    assert_eq!(check_token(&mut store, &token), Ok(()));

    // A failed release keeps the resource held so that it can be retried.
    store.fail_next(1);
    assert!(fencer.release(&mut store, b"r").is_err());
    assert_eq!(fencer.held_epoch(b"r"), Some(1));
    assert!(fencer.release(&mut store, b"r").unwrap());

    store.fail_next(1);
    assert!(matches!(
        check_token(&mut store, &token),
        Err(ClusterError::StoreUnavailable(_))
    ));
}

struct FailOnCas {
    inner: MemoryEpochStore,
    fail: bool,
}

impl EpochStore for FailOnCas {
    fn load(&mut self, resource: &[u8]) -> Result<Option<EpochRecord>, ClusterError> {
        self.inner.get(resource)
    }

    fn compare_and_set(
        &mut self,
        resource: &[u8],
        expected: Option<&EpochRecord>,
        new: EpochRecord,
    ) -> Result<bool, ClusterError> {
        if self.fail {
            self.fail = false;
            return Err(ClusterError::StoreUnavailable("write failed".into()));
        }
        self.inner.cas(resource, expected, new)
    }
}

#[test]
fn std_drain_stops_acquisitions_and_completes_when_everything_is_released() {
    let mut store = MemoryEpochStore::new();
    let mut fencer = Fencer::new(node(1));
    fencer.acquire(&mut store, b"a").unwrap();
    fencer.acquire(&mut store, b"b").unwrap();
    assert!(!fencer.is_draining() && !fencer.is_drained());

    fencer.begin_drain();
    assert!(fencer.is_draining() && !fencer.is_drained());
    assert_eq!(fencer.acquire(&mut store, b"c"), Err(ClusterError::Draining));
    // Even a resource it already holds is not re-confirmed during a drain.
    assert_eq!(fencer.acquire(&mut store, b"a"), Err(ClusterError::Draining));

    let held: Vec<Vec<u8>> = fencer.held().map(<[u8]>::to_vec).collect();
    assert_eq!(held, vec![b"a".to_vec(), b"b".to_vec()]);
    for resource in held {
        assert!(fencer.release(&mut store, &resource).unwrap());
    }
    assert!(fencer.is_drained());
}

#[test]
fn std_handoff_candidates_are_the_held_resources_placement_assigns_elsewhere() {
    let mut store = MemoryEpochStore::new();
    let mut fencer = Fencer::new(node(1));
    let all = keys(60);
    for key in &all {
        fencer.acquire(&mut store, key).unwrap();
    }
    let alone = even(&[1]);
    assert!(fencer.handoff_candidates(&alone).is_empty());

    let grown = even(&[1, 2]);
    let candidates = fencer.handoff_candidates(&grown);
    assert!(!candidates.is_empty() && candidates.len() < all.len());
    for key in &all {
        let moves = grown.owner(key) != Some(&node(1));
        assert_eq!(candidates.contains(key), moves);
    }
    // Without this node in the placement (drain) everything is handed off.
    assert_eq!(fencer.handoff_candidates(&even(&[2])).len(), all.len());
    assert_eq!(
        fencer.handoff_candidates(&Placement::default()).len(),
        all.len()
    );
}

#[test]
fn std_reset_forgets_epochs_of_the_old_identity() {
    let mut store = MemoryEpochStore::new();
    let mut fencer = Fencer::new(node(1));
    let old_token = fencer.acquire(&mut store, b"r").unwrap();
    let renewed = NodeId::new(1, 2, "n1");
    fencer.reset(renewed.clone());
    assert_eq!(fencer.me(), &renewed);
    assert_eq!(fencer.held().count(), 0);
    // The new instance must win a new epoch; the old instance's token dies with it.
    let new_token = fencer.acquire(&mut store, b"r").unwrap();
    assert_eq!((new_token.epoch, &new_token.owner), (2, &renewed));
    assert!(check_token(&mut store, &old_token).is_err());
}

#[test]
fn edge_epoch_overflow_is_an_error_not_a_wrap() {
    let mut store = MemoryEpochStore::new();
    let last = EpochRecord {
        epoch: u64::MAX,
        owner: Some(node(2)),
    };
    assert!(store.cas(b"r", None, last.clone()).unwrap());
    let mut fencer = Fencer::new(node(1));
    assert_eq!(
        fencer.acquire(&mut store, b"r"),
        Err(ClusterError::EpochOverflow)
    );
    assert_eq!(store.get(b"r").unwrap(), Some(last));
}

#[test]
fn edge_token_for_unknown_resource_is_fenced_at_epoch_zero() {
    let mut store = MemoryEpochStore::new();
    let mut fencer = Fencer::new(node(1));
    let token = fencer.acquire(&mut store, b"r").unwrap();
    let mut elsewhere = MemoryEpochStore::new();
    assert_eq!(
        check_token(&mut elsewhere, &token),
        Err(ClusterError::Fenced { current: 0 })
    );
    // Same epoch, different instance: rejected.
    let forged = mt_cluster::FencingToken {
        owner: NodeId::new(1, 2, "n1"),
        ..token
    };
    assert!(check_token(&mut store, &forged).is_err());
}

/// The accepted writes of a shared store: `(resource, epoch, owner id)`.
#[derive(Default)]
struct SharedState {
    epochs: MemoryEpochStore,
    accepted: Vec<(Vec<u8>, u64, u64)>,
}

/// Each call locks separately, so other threads interleave between `load` and
/// `compare_and_set` exactly like concurrent database sessions.
struct SharedEpochs(Arc<Mutex<SharedState>>);

impl EpochStore for SharedEpochs {
    fn load(&mut self, resource: &[u8]) -> Result<Option<EpochRecord>, ClusterError> {
        self.0.lock().unwrap().epochs.get(resource)
    }

    fn compare_and_set(
        &mut self,
        resource: &[u8],
        expected: Option<&EpochRecord>,
        new: EpochRecord,
    ) -> Result<bool, ClusterError> {
        self.0.lock().unwrap().epochs.cas(resource, expected, new)
    }
}

#[test]
fn concurrency_racing_nodes_never_share_an_epoch() {
    const THREADS: u64 = 8;
    const ROUNDS: u64 = 400;
    let shared = Arc::new(Mutex::new(SharedState::default()));
    let handles: Vec<_> = (1..=THREADS)
        .map(|id| {
            let shared = Arc::clone(&shared);
            thread::spawn(move || {
                let mut store = SharedEpochs(Arc::clone(&shared));
                let mut fencer = Fencer::new(node(id));
                let mut rng = Rng::new(id);
                let mut contended = 0_u64;
                for _ in 0..ROUNDS {
                    let resource = vec![b'r', u8::try_from(rng.below(3)).unwrap()];
                    match fencer.acquire(&mut store, &resource) {
                        Ok(token) => {
                            // The guarded write: token check and write in one critical
                            // section, as a database transaction would do.
                            let mut state = shared.lock().unwrap();
                            if check_token(&mut state.epochs, &token).is_ok() {
                                state.accepted.push((resource.clone(), token.epoch, id));
                            } else {
                                drop(state);
                                fencer.on_fenced(&resource);
                            }
                        }
                        Err(ClusterError::Contended) => contended += 1,
                        Err(other) => panic!("unexpected error {other}"),
                    }
                    if rng.chance(100) {
                        fencer.release(&mut store, &resource).unwrap();
                    }
                }
                contended
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }

    let state = shared.lock().unwrap();
    assert!(!state.accepted.is_empty());
    let mut owner_of: BTreeMap<(Vec<u8>, u64), u64> = BTreeMap::new();
    let mut last_epoch: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
    for (resource, epoch, owner) in &state.accepted {
        // One writer per (resource, epoch)…
        let seen = owner_of.entry((resource.clone(), *epoch)).or_insert(*owner);
        assert_eq!(seen, owner, "two owners wrote {resource:?} at epoch {epoch}");
        // …and accepted epochs never go backwards.
        let last = last_epoch.entry(resource.clone()).or_insert(0);
        assert!(*epoch >= *last);
        *last = *epoch;
    }
}

#[derive(Debug, Clone)]
enum FenceOp {
    Acquire(usize, u8),
    Release(usize, u8),
    Write(usize, u8),
    Reset(usize),
    StoreFails,
}

fn arb_fence_op() -> impl Strategy<Value = FenceOp> {
    prop_oneof![
        4 => (0_usize..3, 0_u8..2).prop_map(|(n, r)| FenceOp::Acquire(n, r)),
        2 => (0_usize..3, 0_u8..2).prop_map(|(n, r)| FenceOp::Release(n, r)),
        4 => (0_usize..3, 0_u8..2).prop_map(|(n, r)| FenceOp::Write(n, r)),
        1 => (0_usize..3).prop_map(FenceOp::Reset),
        1 => Just(FenceOp::StoreFails),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Model test: whatever nodes do, each (resource, epoch) has one accepted writer, the
    /// stored epoch never decreases, and a node's accepted write always carries the
    /// stored epoch.
    #[test]
    fn model_fencing_keeps_a_single_writer(ops in proptest::collection::vec(arb_fence_op(), 1..80)) {
        let mut store = MemoryEpochStore::new();
        let mut fencers: Vec<Fencer> = (1..=3).map(|id| Fencer::new(node(id))).collect();
        let mut incarnations = [1_u64; 3];
        let mut writers: BTreeMap<(u8, u64), NodeId> = BTreeMap::new();
        let mut stored: BTreeMap<u8, u64> = BTreeMap::new();
        for op in ops {
            match op {
                FenceOp::Acquire(n, r) => {
                    let _ = fencers[n].acquire(&mut store, &[r]);
                }
                FenceOp::Release(n, r) => {
                    let _ = fencers[n].release(&mut store, &[r]);
                }
                FenceOp::Write(n, r) => {
                    if let Some(token) = fencers[n].token(&[r]) {
                        match check_token(&mut store, &token) {
                            Ok(()) => {
                                let writer = writers.entry((r, token.epoch)).or_insert_with(|| token.owner.clone());
                                prop_assert_eq!(&*writer, &token.owner);
                                prop_assert_eq!(stored.get(&r).copied(), Some(token.epoch));
                            }
                            Err(e) if e.is_fenced() => fencers[n].on_fenced(&[r]),
                            Err(_) => {}
                        }
                    }
                }
                FenceOp::Reset(n) => {
                    incarnations[n] += 1;
                    let id = u64::try_from(n).unwrap() + 1;
                    fencers[n].reset(NodeId::new(id, incarnations[n], format!("n{id}")));
                }
                FenceOp::StoreFails => store.fail_next(1),
            }
            for r in 0_u8..2 {
                let mut probe = store.clone();
                probe.fail_next(0);
                let epoch = probe.get(&[r]).unwrap().map_or(0, |record| record.epoch);
                let previous = stored.insert(r, epoch).unwrap_or(0);
                prop_assert!(epoch >= previous, "epoch went backwards");
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Shard leases
// ---------------------------------------------------------------------------------------------

const TTL: u64 = 1_000;

fn lease(store: &mut MemoryLeaseStore, shard: u32) -> LeaseRecord {
    LeaseStore::load(store, shard).unwrap().unwrap()
}

#[test]
fn std_shard_keys_and_shard_assignment() {
    assert_eq!(&shard_key(0), b"shard:\0\0\0\0");
    assert_eq!(&shard_key(0x0102_0304), b"shard:\x01\x02\x03\x04");
    assert_eq!(shard_of(b"anything", 0), 0);
    assert_eq!(shard_of(b"anything", 1), 0);
    let mut used = BTreeSet::new();
    for key in keys(2_000) {
        let shard = shard_of(&key, 16);
        assert!(shard < 16);
        assert_eq!(shard, shard_of(&key, 16));
        used.insert(shard);
    }
    assert_eq!(used.len(), 16);
    // Golden value: nodes of different versions must agree on the shard of a key.
    assert_eq!(shard_of(b"a", 64), u32::try_from(0x02c0_bdbf_4814_20f8_u64 % 64).unwrap());
}

#[test]
fn std_claim_renew_advance_release() {
    let mut store = MemoryLeaseStore::new();
    let token = claim(&mut store, 3, &node(1), 100, TTL).unwrap();
    assert_eq!(
        token,
        LeaseToken {
            shard: 3,
            term: 1,
            holder: node(1)
        }
    );
    assert_eq!(
        lease(&mut store, 3),
        LeaseRecord {
            term: 1,
            holder: Some(node(1)),
            expires_at_ms: 1_100,
            cursor_ms: 0
        }
    );

    // Claiming again is idempotent: same term, expiry untouched.
    assert_eq!(claim(&mut store, 3, &node(1), 500, TTL).unwrap(), token);
    assert_eq!(lease(&mut store, 3).expires_at_ms, 1_100);

    assert_eq!(renew(&mut store, &token, 600, TTL), Ok(1_600));
    // A renewal never shortens the lease (a retry that arrives late, a clock that jumped back).
    assert_eq!(renew(&mut store, &token, 200, TTL), Ok(1_600));

    assert_eq!(
        advance(&mut store, &token, 700, 650),
        Ok(Some(FireWindow {
            shard: 3,
            term: 1,
            after_ms: 0,
            upto_ms: 650
        }))
    );
    assert_eq!(
        advance(&mut store, &token, 800, 800),
        Ok(Some(FireWindow {
            shard: 3,
            term: 1,
            after_ms: 650,
            upto_ms: 800
        }))
    );
    // Retrying, or asking for an older instant, fires nothing.
    assert_eq!(advance(&mut store, &token, 800, 800), Ok(None));
    assert_eq!(advance(&mut store, &token, 800, 10), Ok(None));

    assert_eq!(release(&mut store, &token), Ok(true));
    assert_eq!(release(&mut store, &token), Ok(false));
    let after = lease(&mut store, 3);
    assert_eq!((after.holder, after.expires_at_ms, after.cursor_ms), (None, 0, 800));
}

#[test]
fn std_lease_takeover_happens_only_after_expiry_and_continues_the_cursor() {
    let mut store = MemoryLeaseStore::new();
    let first = claim(&mut store, 0, &node(1), 0, TTL).unwrap();
    advance(&mut store, &first, 10, 400).unwrap();

    assert_eq!(
        claim(&mut store, 0, &node(2), 999, TTL),
        Err(ClusterError::LeaseHeld { until_ms: 1_000 })
    );
    // At the expiry instant the lease is free.
    let second = claim(&mut store, 0, &node(2), 1_000, TTL).unwrap();
    assert_eq!(second.term, 2);
    assert_eq!(lease(&mut store, 0).cursor_ms, 400);

    // The old holder is fenced on every operation.
    assert_eq!(
        renew(&mut store, &first, 1_001, TTL),
        Err(ClusterError::Fenced { current: 2 })
    );
    assert_eq!(
        advance(&mut store, &first, 1_001, 900),
        Err(ClusterError::Fenced { current: 2 })
    );
    assert_eq!(release(&mut store, &first), Ok(false));

    // The new holder's first window starts exactly where the old one stopped.
    assert_eq!(
        advance(&mut store, &second, 1_001, 900),
        Ok(Some(FireWindow {
            shard: 0,
            term: 2,
            after_ms: 400,
            upto_ms: 900
        }))
    );
}

#[test]
fn std_released_lease_can_be_claimed_at_once() {
    let mut store = MemoryLeaseStore::new();
    let first = claim(&mut store, 0, &node(1), 0, TTL).unwrap();
    release(&mut store, &first).unwrap();
    let second = claim(&mut store, 0, &node(2), 1, TTL).unwrap();
    assert_eq!(second.term, 2);
    // The same instance re-claiming after its own expiry also gets a new term.
    let third = claim(&mut store, 0, &node(2), 5_000, TTL).unwrap();
    assert_eq!(third.term, 3);
    assert!(renew(&mut store, &second, 5_001, TTL).unwrap_err().is_fenced());
}

#[test]
fn anomalous_expired_lease_cannot_fire_until_renewed() {
    let mut store = MemoryLeaseStore::new();
    let token = claim(&mut store, 0, &node(1), 0, TTL).unwrap();
    assert_eq!(
        advance(&mut store, &token, 1_000, 900),
        Err(ClusterError::LeaseExpired)
    );
    // Nobody took it meanwhile, so the holder may renew and carry on in the same term.
    assert_eq!(renew(&mut store, &token, 1_200, TTL), Ok(2_200));
    assert!(advance(&mut store, &token, 1_300, 900).unwrap().is_some());
}

#[test]
fn invalid_tokens_for_unknown_shards_are_fenced() {
    let mut store = MemoryLeaseStore::new();
    let ghost = LeaseToken {
        shard: 9,
        term: 1,
        holder: node(1),
    };
    assert_eq!(
        renew(&mut store, &ghost, 0, TTL),
        Err(ClusterError::Fenced { current: 0 })
    );
    assert_eq!(
        advance(&mut store, &ghost, 0, 10),
        Err(ClusterError::Fenced { current: 0 })
    );
    assert_eq!(release(&mut store, &ghost), Ok(false));
}

/// A lease store in which another node wins between `load` and `compare_and_set`.
struct RacyLeases {
    inner: MemoryLeaseStore,
    interloper: Option<LeaseRecord>,
}

impl LeaseStore for RacyLeases {
    fn load(&mut self, shard: u32) -> Result<Option<LeaseRecord>, ClusterError> {
        LeaseStore::load(&mut self.inner, shard)
    }

    fn compare_and_set(
        &mut self,
        shard: u32,
        expected: Option<&LeaseRecord>,
        new: LeaseRecord,
    ) -> Result<bool, ClusterError> {
        if let Some(record) = self.interloper.take() {
            assert!(LeaseStore::compare_and_set(
                &mut self.inner,
                shard,
                expected,
                record
            )?);
        }
        LeaseStore::compare_and_set(&mut self.inner, shard, expected, new)
    }
}

#[test]
fn anomalous_lost_lease_races_are_contended() {
    let stolen = |term| LeaseRecord {
        term,
        holder: Some(node(2)),
        expires_at_ms: 9_000,
        cursor_ms: 0,
    };
    let mut store = RacyLeases {
        inner: MemoryLeaseStore::new(),
        interloper: Some(stolen(1)),
    };
    assert_eq!(
        claim(&mut store, 0, &node(1), 0, TTL),
        Err(ClusterError::Contended)
    );
    assert_eq!(lease(&mut store.inner, 0).holder, Some(node(2)));

    let mut store = RacyLeases {
        inner: MemoryLeaseStore::new(),
        interloper: None,
    };
    let token = claim(&mut store, 0, &node(1), 0, TTL).unwrap();
    store.interloper = Some(stolen(2));
    assert_eq!(
        renew(&mut store, &token, 1, TTL),
        Err(ClusterError::Contended)
    );
    // After the lost race the token is plainly fenced.
    assert_eq!(
        advance(&mut store, &token, 2, 5),
        Err(ClusterError::Fenced { current: 2 })
    );

    let mut store = RacyLeases {
        inner: MemoryLeaseStore::new(),
        interloper: None,
    };
    let token = claim(&mut store, 0, &node(1), 0, TTL).unwrap();
    store.interloper = Some(stolen(2));
    assert_eq!(
        advance(&mut store, &token, 1, 5),
        Err(ClusterError::Contended)
    );
    assert_eq!(lease(&mut store.inner, 0).cursor_ms, 0);

    let mut store = RacyLeases {
        inner: MemoryLeaseStore::new(),
        interloper: None,
    };
    let token = claim(&mut store, 0, &node(1), 0, TTL).unwrap();
    store.interloper = Some(stolen(2));
    assert_eq!(release(&mut store, &token), Ok(false));
    assert_eq!(lease(&mut store.inner, 0).holder, Some(node(2)));
}

#[test]
fn edge_lease_term_overflow_and_saturating_expiry() {
    let mut store = MemoryLeaseStore::new();
    let last = LeaseRecord {
        term: u64::MAX,
        holder: None,
        expires_at_ms: 0,
        cursor_ms: 7,
    };
    assert!(LeaseStore::compare_and_set(&mut store, 0, None, last).unwrap());
    assert_eq!(
        claim(&mut store, 0, &node(1), 0, TTL),
        Err(ClusterError::EpochOverflow)
    );

    // Near the end of time the expiry saturates instead of wrapping to the past.
    let token = claim(&mut store, 1, &node(1), u64::MAX - 5, TTL).unwrap();
    assert_eq!(lease(&mut store, 1).expires_at_ms, u64::MAX);
    assert_eq!(renew(&mut store, &token, u64::MAX - 1, TTL), Ok(u64::MAX));
}

#[test]
fn anomalous_store_failures_surface_from_every_lease_operation() {
    let mut store = MemoryLeaseStore::new();
    let unavailable = |r: Result<(), ClusterError>| matches!(r, Err(ClusterError::StoreUnavailable(_)));
    store.fail_next(1);
    assert!(unavailable(claim(&mut store, 0, &node(1), 0, TTL).map(|_| ())));
    let token = claim(&mut store, 0, &node(1), 0, TTL).unwrap();
    store.fail_next(1);
    assert!(unavailable(renew(&mut store, &token, 1, TTL).map(|_| ())));
    store.fail_next(1);
    assert!(unavailable(advance(&mut store, &token, 1, 5).map(|_| ())));
    store.fail_next(1);
    assert!(unavailable(release(&mut store, &token).map(|_| ())));
    // Nothing was changed by the failed calls.
    assert_eq!(
        lease(&mut store, 0),
        LeaseRecord {
            term: 1,
            holder: Some(node(1)),
            expires_at_ms: TTL,
            cursor_ms: 0
        }
    );
}

fn sched_cfg() -> SchedulerConfig {
    SchedulerConfig {
        shard_count: 4,
        lease_ttl_ms: 1_500,
        renew_before_ms: 500,
    }
}

#[test]
fn std_scheduler_config_defaults_renew_well_before_expiry() {
    let cfg = SchedulerConfig::default();
    assert!(cfg.shard_count > 0);
    assert!(cfg.renew_before_ms < cfg.lease_ttl_ms);
}

#[test]
fn std_scheduler_claims_renews_and_releases_with_placement() {
    let mut store = MemoryLeaseStore::new();
    let mut scheduler = ShardScheduler::new(sched_cfg(), node(1));
    let alone = even(&[1]);

    let events = scheduler.tick(0, &alone, &mut store);
    assert_eq!(events.len(), 4);
    assert!(events
        .iter()
        .all(|e| matches!(e, SchedEvent::Claimed(t) if t.term == 1 && t.holder == node(1))));
    assert_eq!(scheduler.held_shards().collect::<Vec<_>>(), vec![0, 1, 2, 3]);
    assert_eq!(scheduler.token(2).map(|t| t.term), Some(1));

    // Far from expiry: nothing to do.
    assert!(scheduler.tick(900, &alone, &mut store).is_empty());
    // Within `renew_before_ms` of expiry: every lease is extended.
    let events = scheduler.tick(1_000, &alone, &mut store);
    assert_eq!(
        events,
        (0..4).map(SchedEvent::Renewed).collect::<Vec<_>>()
    );
    assert_eq!(lease(&mut store, 0).expires_at_ms, 2_500);

    // A second node joins: the shards placement gives it are released, the rest stay.
    let both = even(&[1, 2]);
    let moved: Vec<u32> = (0..4)
        .filter(|s| both.owner(&shard_key(*s)) != Some(&node(1)))
        .collect();
    let events = scheduler.tick(1_100, &both, &mut store);
    assert_eq!(
        events,
        moved.iter().copied().map(SchedEvent::Released).collect::<Vec<_>>()
    );
    for shard in &moved {
        assert_eq!(scheduler.token(*shard), None);
        assert_eq!(lease(&mut store, *shard).holder, None);
    }

    // Drain: a placement without this node releases everything.
    scheduler.tick(1_200, &even(&[2]), &mut store);
    assert_eq!(scheduler.held_shards().count(), 0);
    assert!(scheduler.tick(1_300, &Placement::default(), &mut store).is_empty());
}

#[test]
fn std_scheduler_waits_for_the_previous_holder_to_expire() {
    let mut store = MemoryLeaseStore::new();
    let mut first = ShardScheduler::new(sched_cfg(), node(1));
    let mut second = ShardScheduler::new(sched_cfg(), node(2));
    first.tick(0, &even(&[1]), &mut store);

    // Node 2 believes it owns everything (node 1 looks dead to it), but the leases are live.
    assert!(second.tick(100, &even(&[2]), &mut store).is_empty());
    assert_eq!(second.held_shards().count(), 0);
    assert_eq!(second.fire(100, &mut store, 0, 50), Err(ClusterError::NotHeld));

    // After expiry it takes over with a higher term.
    let events = second.tick(1_500, &even(&[2]), &mut store);
    assert_eq!(events.len(), 4);
    assert!(events
        .iter()
        .all(|e| matches!(e, SchedEvent::Claimed(t) if t.term == 2)));

    // Node 1 comes back from its pause: its next renewal is fenced and it drops the leases.
    let events = first.tick(1_600, &even(&[1]), &mut store);
    assert_eq!(events, (0..4).map(SchedEvent::Lost).collect::<Vec<_>>());
    assert_eq!(first.held_shards().count(), 0);
    // It may not fire in the meantime either.
    assert_eq!(first.fire(1_600, &mut store, 0, 1_000), Err(ClusterError::NotHeld));
    // And it cannot grab them back while node 2's leases are live.
    assert!(first.tick(1_700, &even(&[1]), &mut store).is_empty());
}

#[test]
fn std_scheduler_fire_commits_windows_and_drops_fenced_leases() {
    let mut store = MemoryLeaseStore::new();
    let mut first = ShardScheduler::new(sched_cfg(), node(1));
    first.tick(0, &even(&[1]), &mut store);
    assert_eq!(
        first.fire(10, &mut store, 1, 10),
        Ok(Some(FireWindow {
            shard: 1,
            term: 1,
            after_ms: 0,
            upto_ms: 10
        }))
    );
    assert_eq!(first.fire(20, &mut store, 1, 10), Ok(None));

    // The lease ran out locally unnoticed (a long pause): firing is refused and the lease dropped.
    assert_eq!(
        first.fire(1_500, &mut store, 1, 1_400),
        Err(ClusterError::LeaseExpired)
    );
    assert_eq!(first.token(1), None);

    // Another node takes shard 2 after expiry; the stale holder's fire is fenced.
    let other = claim(&mut store, 2, &node(2), 1_500, TTL).unwrap();
    assert_eq!(
        first.fire(1_501, &mut store, 2, 1_400),
        Err(ClusterError::Fenced { current: other.term })
    );
    assert_eq!(first.token(2), None);

    // A store outage is not a reason to drop a lease.
    store.fail_next(1);
    assert!(matches!(
        first.fire(100, &mut store, 3, 50),
        Err(ClusterError::StoreUnavailable(_))
    ));
    assert!(first.token(3).is_some());
}

#[test]
fn anomalous_scheduler_survives_store_outages() {
    let mut store = MemoryLeaseStore::new();
    let mut scheduler = ShardScheduler::new(sched_cfg(), node(1));
    let alone = even(&[1]);

    // Claims that fail are retried on the next tick.
    store.fail_next(2);
    let events = scheduler.tick(0, &alone, &mut store);
    assert_eq!(events.len(), 2);
    assert_eq!(scheduler.tick(1, &alone, &mut store).len(), 2);
    assert_eq!(scheduler.held_shards().count(), 4);

    // A failed renewal before expiry keeps the lease and retries.
    store.fail_next(100);
    assert!(scheduler.tick(1_100, &alone, &mut store).is_empty());
    assert_eq!(scheduler.held_shards().count(), 4);
    // Still failing at expiry: the leases are lost (someone else may have them now).
    let events = scheduler.tick(1_501, &alone, &mut store);
    assert_eq!(events, (0..4).map(SchedEvent::Lost).collect::<Vec<_>>());
    assert_eq!(scheduler.held_shards().count(), 0);

    // A failed release keeps the lease so the release is retried.
    store.fail_next(0);
    scheduler.tick(2_000, &alone, &mut store);
    assert_eq!(scheduler.held_shards().count(), 4);
    store.fail_next(100);
    assert!(scheduler.tick(2_001, &even(&[2]), &mut store).is_empty());
    assert_eq!(scheduler.held_shards().count(), 4);
    store.fail_next(0);
    assert_eq!(scheduler.tick(2_002, &even(&[2]), &mut store).len(), 4);
}

#[test]
fn std_scheduler_reset_forgets_leases_and_reclaims_under_the_new_identity() {
    let mut store = MemoryLeaseStore::new();
    let mut scheduler = ShardScheduler::new(sched_cfg(), node(1));
    scheduler.tick(0, &even(&[1]), &mut store);
    let renewed = NodeId::new(1, 2, "n1");
    scheduler.reset(renewed.clone());
    assert_eq!(scheduler.held_shards().count(), 0);

    let placement = Placement::new([(renewed.clone(), 1)]);
    // The old instance's leases are still live: the new instance waits like anyone else.
    assert!(scheduler.tick(100, &placement, &mut store).is_empty());
    let events = scheduler.tick(1_500, &placement, &mut store);
    assert!(events
        .iter()
        .all(|e| matches!(e, SchedEvent::Claimed(t) if t.term == 2 && t.holder == renewed)));
    assert_eq!(events.len(), 4);
}

#[test]
fn edge_scheduler_tracks_the_stored_expiry_after_an_idempotent_claim() {
    // The scheduler forgets a lease it still holds in the store (reset to the same
    // identity). Re-claiming is idempotent and does not extend the lease, so the scheduler
    // must renew by the *stored* expiry (1500), not by "now + ttl" (2100).
    let mut store = MemoryLeaseStore::new();
    let mut scheduler = ShardScheduler::new(sched_cfg(), node(1));
    let alone = even(&[1]);
    scheduler.tick(0, &alone, &mut store);
    scheduler.reset(node(1));
    let events = scheduler.tick(600, &alone, &mut store);
    assert!(events
        .iter()
        .all(|e| matches!(e, SchedEvent::Claimed(t) if t.term == 1)));
    assert_eq!(lease(&mut store, 0).expires_at_ms, 1_500);
    let events = scheduler.tick(1_000, &alone, &mut store);
    assert_eq!(events, (0..4).map(SchedEvent::Renewed).collect::<Vec<_>>());
    assert_eq!(lease(&mut store, 0).expires_at_ms, 2_500);
}

#[test]
fn edge_scheduler_with_zero_shards_does_nothing() {
    let mut store = MemoryLeaseStore::new();
    let cfg = SchedulerConfig {
        shard_count: 0,
        ..sched_cfg()
    };
    let mut scheduler = ShardScheduler::new(cfg, node(1));
    assert!(scheduler.tick(0, &even(&[1]), &mut store).is_empty());
    assert!(store.is_empty());
}

#[derive(Debug, Clone)]
enum LeaseOp {
    Claim(usize),
    Renew(usize),
    Release(usize),
    Advance(usize, u64),
    Sleep(u64),
    StoreFails,
}

fn arb_lease_op() -> impl Strategy<Value = LeaseOp> {
    prop_oneof![
        3 => (0_usize..3).prop_map(LeaseOp::Claim),
        2 => (0_usize..3).prop_map(LeaseOp::Renew),
        1 => (0_usize..3).prop_map(LeaseOp::Release),
        5 => (0_usize..3, 0_u64..400).prop_map(|(n, d)| LeaseOp::Advance(n, d)),
        3 => (0_u64..700).prop_map(LeaseOp::Sleep),
        1 => Just(LeaseOp::StoreFails),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Model test of one shard shared by three nodes that claim, renew, release and fire in
    /// any order, with time passing and the store failing now and then. Fire windows must
    /// tile the timeline: each starts where the previous one ended, terms never go back,
    /// and at any moment at most one node's token is current.
    #[test]
    fn model_fire_windows_never_overlap(ops in proptest::collection::vec(arb_lease_op(), 1..120)) {
        let mut store = MemoryLeaseStore::new();
        let nodes = [node(1), node(2), node(3)];
        let mut tokens: [Option<LeaseToken>; 3] = [None, None, None];
        let mut now = 0_u64;
        let mut windows: Vec<FireWindow> = Vec::new();
        for op in ops {
            match op {
                LeaseOp::Claim(n) => {
                    if let Ok(token) = claim(&mut store, 0, &nodes[n], now, 500) {
                        tokens[n] = Some(token);
                    }
                }
                LeaseOp::Renew(n) => {
                    if let Some(token) = &tokens[n] {
                        if renew(&mut store, token, now, 500).is_err_and(|e| e.is_fenced()) {
                            tokens[n] = None;
                        }
                    }
                }
                LeaseOp::Release(n) => {
                    if let Some(token) = tokens[n].take() {
                        if release(&mut store, &token).is_err() {
                            tokens[n] = Some(token);
                        }
                    }
                }
                LeaseOp::Advance(n, back) => {
                    if let Some(token) = &tokens[n] {
                        match advance(&mut store, token, now, now.saturating_sub(back)) {
                            Ok(Some(window)) => windows.push(window),
                            Ok(None) => {}
                            Err(e) if e.is_fenced() => tokens[n] = None,
                            Err(_) => {}
                        }
                    }
                }
                LeaseOp::Sleep(ms) => now += ms,
                LeaseOp::StoreFails => store.fail_next(1),
            }
            let mut probe = store.clone();
            probe.fail_next(0);
            let record = LeaseStore::load(&mut probe, 0).unwrap();
            let current = tokens
                .iter()
                .flatten()
                .filter(|t| record.as_ref().is_some_and(|r| r.term == t.term && r.holder.as_ref() == Some(&t.holder)))
                .count();
            prop_assert!(current <= 1, "two current lease tokens");
        }
        let mut cursor = 0_u64;
        let mut term = 0_u64;
        for window in &windows {
            prop_assert_eq!(window.after_ms, cursor, "gap or overlap between windows");
            prop_assert!(window.upto_ms > window.after_ms);
            prop_assert!(window.term >= term);
            cursor = window.upto_ms;
            term = window.term;
        }
    }
}

//! Property tests: tree queries agree with each other on arbitrary forests, selection is
//! monotone, storage round trips, and changes are atomic. Plus the concurrency checks.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_core::field::RefScope;
use mt_core::id::{RefItemId, RefSetId};
use mt_refdata::{
    Change, EffectiveSet, ItemPatch, Ledger, Overlay, RefData, RefDataError, RefItem, RefSet,
    SetScope, State, Structure,
};
use proptest::prelude::*;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::thread;

fn forest_set() -> RefSet {
    RefSet::new(
        RefSetId::from_u128(1),
        "forest",
        "Forest",
        Structure::tree(),
        SetScope::Tenant,
    )
}

/// A random forest: item `n` hangs under an earlier item or is a root, so there are no
/// cycles; depth is kept within the limit by construction of the test (rejected otherwise).
fn forest() -> impl Strategy<Value = Vec<RefItem>> {
    proptest::collection::vec(
        (
            any::<proptest::sample::Index>(),
            0_u8..4,
            -3_i64..4,
            any::<bool>(),
            0_u8..3,
        ),
        1..40,
    )
    .prop_map(|rows| {
        let mut items: Vec<RefItem> = Vec::new();
        for (n, (pick, root_bias, sort, enabled, validity)) in rows.into_iter().enumerate() {
            let id = u128::try_from(n).unwrap() + 1;
            let parent = if items.is_empty() || root_bias == 0 {
                None
            } else {
                Some(items[pick.index(items.len())].id.as_u128())
            };
            let mut entry = item(id, &format!("c{id}"), &format!("Item {}", id % 7), parent);
            entry.sort = sort;
            entry.enabled = enabled;
            match validity {
                1 => entry.valid_from = Some(day(2026, 1, 1)),
                2 => entry.valid_to = Some(day(2025, 12, 31)),
                _ => {}
            }
            items.push(entry);
        }
        items
    })
}

fn built(items: &[RefItem]) -> Option<EffectiveSet> {
    match EffectiveSet::build(&forest_set(), items, None) {
        Ok(set) => Some(set),
        // Random forests can exceed the depth limit; that is a correct refusal.
        Err(RefDataError::TooDeep { .. }) => None,
        Err(other) => panic!("unexpected error {other}"),
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// The tree queries describe one and the same tree.
    #[test]
    fn tree_queries_agree(items in forest()) {
        let Some(set) = built(&items) else { return Ok(()) };
        let all: BTreeSet<RefItemId> = set.ids().collect();
        prop_assert_eq!(all.len(), items.len());

        // The walk visits every item once, parents before children.
        let walk = set.walk();
        prop_assert_eq!(walk.len(), items.len());
        prop_assert_eq!(walk.iter().copied().collect::<BTreeSet<_>>(), all.clone());
        for (position, id) in walk.iter().enumerate() {
            if let Some(parent) = set.parent(*id) {
                let parent_position = walk.iter().position(|other| *other == parent).unwrap();
                prop_assert!(parent_position < position);
            }
        }
        for id in &all {
            let ancestors = set.ancestors(*id);
            // Depth, path and ancestors agree.
            prop_assert_eq!(usize::from(set.depth(*id).unwrap()), ancestors.len() + 1);
            let path = set.path(*id);
            prop_assert_eq!(path.last(), Some(id));
            prop_assert_eq!(path.len(), ancestors.len() + 1);
            prop_assert_eq!(set.parent(*id), ancestors.first().copied());
            // `is_within` is exactly "is one of my ancestors".
            for other in &all {
                prop_assert_eq!(set.is_within(*other, *id), ancestors.contains(other));
            }
            // Descendants are exactly the items within this one.
            let descendants: BTreeSet<RefItemId> = set.descendants(*id).into_iter().collect();
            let within: BTreeSet<RefItemId> = all.iter().copied().filter(|other| set.is_within(*id, *other)).collect();
            prop_assert_eq!(&descendants, &within);
            // Leaf means no children means no descendants.
            prop_assert_eq!(set.is_leaf(*id), set.children(*id).is_empty());
            prop_assert_eq!(set.is_leaf(*id), descendants.is_empty());
            // Every child names this item as its parent.
            for child in set.children(*id) {
                prop_assert_eq!(set.parent(*child), Some(*id));
            }
            // Codes resolve back to the item.
            prop_assert_eq!(set.by_code(&set.get(*id).unwrap().code).unwrap().id, *id);
        }
        // Roots are exactly the items without a parent.
        let roots: BTreeSet<RefItemId> = set.roots().iter().copied().collect();
        let parentless: BTreeSet<RefItemId> = all.iter().copied().filter(|id| set.parent(*id).is_none()).collect();
        prop_assert_eq!(roots, parentless);
    }

    /// Selection: an item is selectable only if every ancestor is active, narrowing a scope
    /// never adds options, and the option list is exactly the selectable items.
    #[test]
    fn selection_is_consistent(items in forest(), subtree in any::<proptest::sample::Index>(), level in proptest::option::of(1_u8..6), leaves_only in any::<bool>(), late in any::<bool>()) {
        let Some(set) = built(&items) else { return Ok(()) };
        let on = if late { day(2026, 6, 1) } else { day(2025, 6, 1) };
        let everything = RefScope { set: forest_set().id, subtree: None, level: None, leaves_only: false };
        let root = items[subtree.index(items.len())].id;
        let narrow = RefScope { subtree: Some(root), level, leaves_only, ..everything };

        let wide: BTreeSet<RefItemId> = set.options(&everything, on).into_iter().collect();
        let narrowed: BTreeSet<RefItemId> = set.options(&narrow, on).into_iter().collect();
        prop_assert!(narrowed.is_subset(&wide));
        for id in set.ids() {
            let selectable = set.is_selectable(&everything, id, on);
            prop_assert_eq!(selectable, wide.contains(&id));
            prop_assert_eq!(set.is_selectable(&narrow, id, on), narrowed.contains(&id));
            if selectable {
                // The whole path is enabled and valid.
                for step in set.path(id) {
                    let entry = set.get(step).unwrap();
                    prop_assert!(entry.enabled && entry.is_valid_on(on));
                }
            }
            if narrowed.contains(&id) {
                prop_assert!(set.is_within(root, id));
                prop_assert!(level.is_none_or(|l| set.depth(id) == Some(l)));
                prop_assert!(!leaves_only || set.is_leaf(id));
            }
        }
        // Options come in display order: a subsequence of the walk.
        let walk = set.walk();
        let ordered: Vec<RefItemId> = walk.into_iter().filter(|id| wide.contains(id)).collect();
        prop_assert_eq!(set.options(&everything, on), ordered);
    }

    /// What is stored reads back identically, and builds the same effective set.
    #[test]
    fn storage_round_trip(items in forest()) {
        let state = State::new(forest_set(), items);
        let json = serde_json::to_string(&state).unwrap();
        let back: State = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(&back, &state);
        match (state.effective(), back.effective()) {
            (Ok(a), Ok(b)) => prop_assert_eq!(a, b),
            (Err(a), Err(b)) => prop_assert_eq!(a, b),
            _ => prop_assert!(false, "round trip changed validity"),
        }
    }

    /// The order in which items are stored does not matter.
    #[test]
    fn item_order_does_not_matter(items in forest(), seed in any::<u64>()) {
        let Some(set) = built(&items) else { return Ok(()) };
        let mut shuffled = items;
        // A deterministic shuffle from the seed.
        let mut state = seed | 1;
        for i in (1..shuffled.len()).rev() {
            state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            let j = usize::try_from(state >> 33).unwrap() % (i + 1);
            shuffled.swap(i, j);
        }
        prop_assert_eq!(built(&shuffled).unwrap(), set);
    }

    /// A tenant overlay never changes a system item's code, parent or validity, and the
    /// effective set is exactly system ∪ added.
    #[test]
    fn overlays_only_adapt(disable in proptest::collection::btree_set(41_u128..44, 0..3), relabel in proptest::option::of("[A-Za-z]{1,8}"), extra in 0_u8..4) {
        let mut overlay = Overlay {
            disabled: disable.iter().copied().map(rid).collect(),
            ..Overlay::default()
        };
        if let Some(name) = &relabel {
            overlay.relabel.insert(rid(42), mt_refdata::Relabel { name: Some(name.clone()), names: Default::default() });
        }
        for n in 0..u128::from(extra) {
            overlay.added.push(item(60 + n, &format!("mine{n}"), "Mine", Some(41)));
        }
        let set = EffectiveSet::build(&system_set(), &system_items(), Some(&overlay)).unwrap();
        prop_assert_eq!(set.len(), 3 + usize::from(extra));
        for original in system_items() {
            let effective = set.get(original.id).unwrap();
            prop_assert_eq!(&effective.code, &original.code);
            prop_assert_eq!(effective.parent, original.parent);
            prop_assert_eq!((effective.valid_from, effective.valid_to), (original.valid_from, original.valid_to));
            prop_assert_eq!(effective.enabled, !disable.contains(&original.id.as_u128()));
            let expected_name = match (&relabel, original.id == rid(42)) {
                (Some(name), true) => name.as_str(),
                _ => original.name.as_str(),
            };
            prop_assert_eq!(effective.name.as_str(), expected_name);
        }
    }

    /// Applying changes is atomic: either the result is a valid set, or the state is
    /// untouched; and applying never panics.
    #[test]
    fn changes_are_atomic(renames in proptest::collection::vec((1_u128..30, "[a-z ]{0,5}"), 1..6)) {
        let state = region_state();
        let changes: Vec<Change> = renames
            .into_iter()
            .map(|(id, name)| Change::Update { id: rid(id), patch: ItemPatch { name: Some(name), ..ItemPatch::default() } })
            .collect();
        let before = state.clone();
        match state.apply(&changes) {
            Ok((next, effective)) => {
                prop_assert_eq!(next.effective().unwrap(), effective);
                prop_assert_eq!(next.items.len(), before.items.len());
            }
            Err(_) => prop_assert_eq!(&state, &before),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Concurrency and cross-platform
// ---------------------------------------------------------------------------------------------

fn assert_send_sync<T: Send + Sync + 'static>() {}

#[test]
fn concurrency_types_are_send_and_sync() {
    assert_send_sync::<EffectiveSet>();
    assert_send_sync::<RefData>();
    assert_send_sync::<Ledger>();
    assert_send_sync::<State>();
    assert_send_sync::<mt_refdata::Registry>();
    assert_send_sync::<RefDataError>();
}

#[test]
fn concurrency_readers_share_one_catalogue() {
    let mut data = RefData::new();
    data.insert(EffectiveSet::build(&region_set(), &region_items(), None).unwrap())
        .unwrap();
    let data = Arc::new(data);
    let answer = |data: &RefData| {
        let set = data.by_code("region").unwrap();
        (
            set.walk(),
            set.options(&scope(REGION), day(2025, 6, 1)),
            set.path_label(rid(1111), "ja", " / "),
            set.search("jp", "en", 100),
        )
    };
    let expected = answer(&data);
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let data = Arc::clone(&data);
            thread::spawn(move || (0..200).map(|_| answer(&data)).next_back().unwrap())
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().unwrap(), expected);
    }
}

#[test]
fn concurrency_racing_writers_each_change_lands_once() {
    // Eight editors add items to the same set through optimistic versioning: they read the
    // version, prepare a change, and retry when someone else was faster.
    const WRITERS: u128 = 8;
    const EACH: u128 = 25;
    let ledger = Arc::new(Mutex::new(Ledger::new(region_state()).unwrap()));
    let handles: Vec<_> = (0..WRITERS)
        .map(|writer| {
            let ledger = Arc::clone(&ledger);
            thread::spawn(move || {
                let mut conflicts = 0_u32;
                for n in 0..EACH {
                    let id = 10_000 + writer * 100 + n;
                    loop {
                        // Read outside the write lock, as a client would.
                        let seen = ledger.lock().unwrap().version();
                        thread::yield_now();
                        let change = Change::Add {
                            item: item(id, &format!("w{writer}-{n}"), "Added", Some(3)),
                        };
                        let result = ledger.lock().unwrap().commit(
                            person(writer + 1),
                            at(1),
                            seen,
                            vec![change],
                        );
                        match result {
                            Ok(_) => break,
                            Err(RefDataError::VersionConflict { .. }) => conflicts += 1,
                            Err(other) => panic!("unexpected error {other}"),
                        }
                    }
                }
                conflicts
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    let ledger = ledger.lock().unwrap();
    // Every change landed exactly once, each in its own version, and the set is valid.
    assert_eq!(ledger.version(), u64::try_from(WRITERS * EACH).unwrap());
    assert_eq!(
        ledger.effective().children(rid(3)).len(),
        usize::try_from(WRITERS * EACH).unwrap()
    );
    assert_eq!(
        ledger.effective().len(),
        14 + usize::try_from(WRITERS * EACH).unwrap()
    );
    let restored = Ledger::restore(
        ledger.genesis().clone(),
        ledger.revisions().to_vec(),
        vec![],
    )
    .unwrap();
    assert_eq!(restored.state(), ledger.state());
}

#[test]
fn edge_set_size_limit() {
    use mt_refdata::model::MAX_ITEMS_PER_SET;
    let set = RefSet::new(
        RefSetId::from_u128(9),
        "big",
        "Big",
        Structure::Flat,
        SetScope::Tenant,
    );
    let items: Vec<RefItem> = (0..=u128::try_from(MAX_ITEMS_PER_SET).unwrap())
        .map(|n| item(n + 1, &format!("c{n}"), "N", None))
        .collect();
    assert_eq!(
        EffectiveSet::build(&set, &items, None).unwrap_err(),
        RefDataError::Limit("too many items in one reference set")
    );
    assert_eq!(
        EffectiveSet::build(&set, &items[1..], None).unwrap().len(),
        MAX_ITEMS_PER_SET
    );
}

#[test]
fn edge_large_flat_set_builds_and_answers_quickly() {
    let set = RefSet::new(
        RefSetId::from_u128(9),
        "big",
        "Big",
        Structure::Flat,
        SetScope::Tenant,
    );
    let items: Vec<RefItem> = (0..20_000_u128)
        .map(|n| item(n + 1, &format!("code{n}"), &format!("Name {n}"), None))
        .collect();
    let built = EffectiveSet::build(&set, &items, None).unwrap();
    assert_eq!(built.len(), 20_000);
    assert_eq!(built.by_code("CODE19999").unwrap().id, rid(20_000));
    assert_eq!(built.search("name 1999", "en", 100).len(), 11);
    let all = RefScope {
        set: set.id,
        subtree: None,
        level: None,
        leaves_only: true,
    };
    assert_eq!(built.options(&all, day(2026, 1, 1)).len(), 20_000);
}

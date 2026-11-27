//! Seeded simulations of the whole system. A failure names its seed; failing seeds found
//! earlier stay here as regression tests.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use mt_sim::{run, Config, Report};

fn seeds() -> u64 {
    std::env::var("MT_SIM_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(150)
}

fn total(reports: &[Report]) -> Report {
    let mut t = Report::default();
    for r in reports {
        t.committed += r.committed;
        t.refused += r.refused;
        t.repeats += r.repeats;
        t.fenced += r.fenced;
        t.split_brains += r.split_brains;
        t.crashes += r.crashes;
        t.partitions += r.partitions;
        t.lost += r.lost;
        t.resends += r.resends;
        t.catch_ups += r.catch_ups;
        t.claims += r.claims;
        t.jobs_done += r.jobs_done;
        t.leases_lost += r.leases_lost;
        t.refused_then_applied += r.refused_then_applied;
    }
    t
}

#[test]
fn concurrency_many_seeds_keep_every_invariant() {
    let config = Config::default();
    let mut reports = Vec::new();
    for seed in 1..=seeds() {
        match run(seed, &config) {
            Ok(report) => reports.push(report),
            Err(violation) => panic!("{violation}"),
        }
    }
    let t = total(&reports);
    eprintln!("{t:?}");
    // The runs really exercise the hard cases.
    assert!(t.committed > 1_000, "{t:?}");
    assert!(t.split_brains > 0, "no split brain was simulated");
    assert!(t.fenced > 0, "no superseded owner tried to write");
    assert!(t.repeats > 0, "no repeated batch was recognised");
    assert!(t.crashes > 0 && t.partitions > 0 && t.lost > 0 && t.resends > 0);
    assert!(t.catch_ups > 0, "no client had to catch up");
    assert!(t.leases_lost > 0, "no job lease was taken over");
    assert_eq!(
        t.jobs_done, t.committed,
        "every committed batch's job ran once"
    );
}

#[test]
fn std_a_run_is_a_function_of_its_seed() {
    let config = Config::default();
    assert_eq!(run(42, &config).unwrap(), run(42, &config).unwrap());
    assert_ne!(run(42, &config).unwrap(), run(43, &config).unwrap());
}

#[test]
fn edge_a_calm_network_and_a_hostile_one() {
    // In order (one fixed delay), nothing lost or doubled, no faults.
    let calm = Config {
        faults: false,
        loss_per_mille: 0,
        duplicate_per_mille: 0,
        min_delay_ms: 5,
        max_delay_ms: 5,
        ..Config::default()
    };
    let hostile = Config {
        nodes: 5,
        clients: 6,
        loss_per_mille: 150,
        duplicate_per_mille: 150,
        fault_every_ms: 150,
        detect_ms: 300,
        ..Config::default()
    };
    for seed in 1..=20 {
        let quiet = run(seed, &calm).unwrap_or_else(|v| panic!("{v}"));
        assert_eq!(
            (quiet.fenced, quiet.crashes, quiet.catch_ups),
            (0, 0, 0),
            "calm: {quiet:?}"
        );
        run(seed, &hostile).unwrap_or_else(|v| panic!("{v}"));
    }
}

/// Without journaled refusals, an owner that never saw the first decision can apply a batch
/// whose client was already told it was refused (a resend or a duplicate crossing the
/// answer). The simulator finds it; journaling refusals prevents it (the default above).
#[test]
fn anomalous_unjournaled_refusals_let_a_refused_batch_be_applied_later() {
    // Copies that outlive crashes: long delays, many duplicates, frequent crashes, quick resends.
    let config = Config {
        persist_refusals: false,
        max_delay_ms: 400,
        loss_per_mille: 50,
        duplicate_per_mille: 300,
        fault_every_ms: 120,
        resend_ms: 60,
        ..Config::default()
    };
    let found: u64 = (1..=seeds())
        .filter_map(|seed| run(seed, &config).ok())
        .map(|r| r.refused_then_applied)
        .sum();
    assert!(found > 0, "the anomaly was not reproduced");
    // The same hostile conditions with refusals journaled: never.
    let journaled = Config {
        persist_refusals: true,
        ..config
    };
    for seed in 1..=seeds() {
        run(seed, &journaled).unwrap_or_else(|v| panic!("{v}"));
    }
}

/// The simulator catches a store that does not check fencing tokens: a superseded owner's
/// writes then land after the new owner's (stale state, lost changes, repeats).
#[test]
fn anomalous_a_store_without_fencing_is_caught() {
    let broken = Config {
        check_fences: false,
        ..Config::default()
    };
    let caught: Vec<mt_sim::Violation> = (1..=seeds())
        .filter_map(|seed| run(seed, &broken).err())
        .collect();
    assert!(!caught.is_empty(), "no seed caught the broken store");
    let first = caught[0].to_string();
    assert!(
        first.starts_with(&format!("seed {} at ", caught[0].seed)),
        "{first}"
    );
}

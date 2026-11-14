//! Shared helpers for seeded tests.
#![allow(dead_code)]

use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

/// Seeds to run: `MT_SIM_SEED=<n>` runs exactly one seed; otherwise `MT_SIM_SEEDS=<count>`
/// (default `default_count`) seeds starting at `base`.
pub fn seeds(base: u64, default_count: u64) -> Vec<u64> {
    if let Some(s) = std::env::var("MT_SIM_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        return vec![s];
    }
    let n = std::env::var("MT_SIM_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default_count);
    (base..base + n).collect()
}

/// Runs `f` for every seed; on failure prints the seed so the run can be replayed with
/// `MT_SIM_SEED=<seed>`.
pub fn for_seeds(name: &str, base: u64, default_count: u64, f: impl Fn(u64)) {
    for seed in seeds(base, default_count) {
        if let Err(e) = catch_unwind(AssertUnwindSafe(|| f(seed))) {
            eprintln!("\n*** {name}: FAILING SEED {seed} (replay with MT_SIM_SEED={seed}) ***\n");
            resume_unwind(e);
        }
    }
}

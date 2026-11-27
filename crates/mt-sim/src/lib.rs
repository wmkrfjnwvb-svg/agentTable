//! `mt-sim`: deterministic simulation of the whole system (doc 21 §4).
//!
//! A seeded scheduler drives nodes, clients, a shared database and job workers on a virtual
//! clock, over a network with delay, loss, duplication, reordering and partitions, with node
//! crashes and restarts. The parts are the real ones: `mt-cluster`'s placement and fencing,
//! `mt-ops`' base state (each owner's actor) and client replicas; the database follows
//! `mt-store`'s rules. Membership views lag behind the truth, so two nodes can believe they
//! own a base at once (split brain) — fencing must keep that harmless.
//!
//! Invariants checked at the end of every run (a failure names the seed):
//! 1. each base's journal is numbered without gaps, written by non-decreasing epochs;
//! 2. no batch is applied twice, across ownership changes, crashes and resends;
//! 3. no batch is applied after its client was told it was refused;
//! 4. every client ends with exactly the database's records and no unanswered batch;
//! 5. every job ends done, completed exactly once, despite lost leases and crashes.
#![forbid(unsafe_code)]

pub mod db;
pub mod world;

pub use world::World;

/// The shape of a run.
#[derive(Debug, Clone)]
pub struct Config {
    /// Nodes.
    pub nodes: usize,
    /// Clients.
    pub clients: usize,
    /// Bases.
    pub bases: usize,
    /// How long clients edit and faults happen (virtual ms).
    pub duration_ms: u64,
    /// Time allowed afterwards for everything to settle.
    pub settle_ms: u64,
    /// Fastest delivery.
    pub min_delay_ms: u64,
    /// Slowest delivery (random delays reorder messages).
    pub max_delay_ms: u64,
    /// Messages lost, per mille.
    pub loss_per_mille: u16,
    /// Messages delivered twice, per mille.
    pub duplicate_per_mille: u16,
    /// Crashes and partitions on, roughly this often.
    pub faults: bool,
    /// Mean time between faults.
    pub fault_every_ms: u64,
    /// How long nodes take to notice a change of membership.
    pub detect_ms: u64,
    /// How often a client edits.
    pub edit_every_ms: u64,
    /// When a client sends an unanswered batch again.
    pub resend_ms: u64,
    /// How often a node's worker claims jobs.
    pub worker_every_ms: u64,
    /// A job lease.
    pub lease_ms: u64,
    /// Owners journal refusals so every later owner answers a repeated batch the same way.
    pub persist_refusals: bool,
    /// The database checks the writer's fencing token (off only to show that the simulator
    /// catches a store that does not).
    pub check_fences: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            nodes: 3,
            clients: 4,
            bases: 2,
            duration_ms: 4_000,
            settle_ms: 120_000,
            min_delay_ms: 1,
            max_delay_ms: 40,
            loss_per_mille: 30,
            duplicate_per_mille: 30,
            faults: true,
            fault_every_ms: 400,
            detect_ms: 80,
            edit_every_ms: 60,
            resend_ms: 250,
            worker_every_ms: 50,
            lease_ms: 120,
            persist_refusals: true,
            check_fences: true,
        }
    }
}

/// What happened in a run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// The seed.
    pub seed: u64,
    /// Batches committed.
    pub committed: u64,
    /// Batches refused.
    pub refused: u64,
    /// Repeats recognised and answered without applying.
    pub repeats: u64,
    /// Commits refused because the writer had been superseded.
    pub fenced: u64,
    /// Moments when more than one node believed it owned a base.
    pub split_brains: u64,
    /// Node crashes.
    pub crashes: u64,
    /// Partitions.
    pub partitions: u64,
    /// Messages lost.
    pub lost: u64,
    /// Batches sent again.
    pub resends: u64,
    /// Clients catching up from the journal.
    pub catch_ups: u64,
    /// Job claims.
    pub claims: u64,
    /// Jobs completed.
    pub jobs_done: u64,
    /// Completions refused because the lease had been taken over.
    pub leases_lost: u64,
    /// Batches applied after their client was told they were refused (only possible when
    /// refusals are not journaled).
    pub refused_then_applied: u64,
    /// Each base's epoch at the end.
    pub final_epochs: Vec<u64>,
}

/// A broken invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The seed that reproduces it.
    pub seed: u64,
    /// When (virtual ms).
    pub at_ms: u64,
    /// What.
    pub what: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "seed {} at {} ms: {}", self.seed, self.at_ms, self.what)
    }
}

/// Runs one seed.
///
/// # Errors
/// The first broken invariant.
pub fn run(seed: u64, config: &Config) -> Result<Report, Violation> {
    World::new(seed, config.clone()).run()
}

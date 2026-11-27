//! The simulated database: what every node reads and writes, with the same rules as
//! `mt-store` — epochs by compare-and-set, a commit checks the writer's token and the
//! sequence atomically, decisions are journaled, and jobs are leased with a token.

use mt_cluster::{check_token, EpochStore, FencingToken, MemoryEpochStore};
use mt_core::id::BaseId;
use mt_ops::{Accepted, Outcome, Rejection};
use std::collections::BTreeMap;

/// Why a commit was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitError {
    /// The token is not the current owner record.
    Fenced,
    /// The batch is not the next in sequence.
    Gap,
}

/// One journaled batch and the epoch of the owner that wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The batch.
    pub accepted: Accepted,
    /// The writer's epoch.
    pub epoch: u64,
}

/// A job in the simulated queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// Runs again after this instant.
    pub run_at: u64,
    /// Done.
    pub done: bool,
    /// The current claim's token (raised on every claim).
    pub token: u64,
    /// The current lease's end, while held.
    pub until: Option<u64>,
    /// How many times a completion was accepted.
    pub completions: u32,
}

/// The shared database.
#[derive(Debug)]
pub struct Db {
    /// Ownership epochs per base resource.
    pub epochs: MemoryEpochStore,
    /// The journal per base.
    pub journal: BTreeMap<BaseId, Vec<Entry>>,
    /// Refusals, so every owner gives the same answer to a batch sent again.
    pub refusals: BTreeMap<BaseId, Vec<Rejection>>,
    /// Jobs by id.
    pub jobs: BTreeMap<String, Job>,
    /// Whether commits check the writer's token (a correct store always does).
    pub check_fences: bool,
}

impl Db {
    /// An empty database.
    #[must_use]
    pub fn new(check_fences: bool) -> Self {
        Db {
            epochs: MemoryEpochStore::new(),
            journal: BTreeMap::new(),
            refusals: BTreeMap::new(),
            jobs: BTreeMap::new(),
            check_fences,
        }
    }

    /// The resource name of a base.
    #[must_use]
    pub fn resource(base: BaseId) -> Vec<u8> {
        format!("base:{base}").into_bytes()
    }

    /// Commits a batch for its writer, atomically checking the token and the sequence.
    ///
    /// # Errors
    /// [`CommitError`].
    pub fn commit(
        &mut self,
        base: BaseId,
        accepted: &Accepted,
        token: &FencingToken,
    ) -> Result<(), CommitError> {
        let fenced =
            token.resource != Db::resource(base) || check_token(&mut self.epochs, token).is_err();
        if fenced && self.check_fences {
            return Err(CommitError::Fenced);
        }
        let journal = self.journal.entry(base).or_default();
        let last = journal.last().map_or(0, |e| e.accepted.seq);
        if last.checked_add(1) != Some(accepted.seq) {
            return Err(CommitError::Gap);
        }
        journal.push(Entry {
            accepted: accepted.clone(),
            epoch: token.epoch,
        });
        self.jobs.insert(
            format!("{base}/{}", accepted.seq),
            Job {
                run_at: 0,
                done: false,
                token: 0,
                until: None,
                completions: 0,
            },
        );
        Ok(())
    }

    /// Records a refusal, for the writer that holds the base.
    ///
    /// # Errors
    /// [`CommitError::Fenced`].
    pub fn refuse(
        &mut self,
        base: BaseId,
        rejection: &Rejection,
        token: &FencingToken,
    ) -> Result<(), CommitError> {
        if check_token(&mut self.epochs, token).is_err() {
            return Err(CommitError::Fenced);
        }
        self.refusals
            .entry(base)
            .or_default()
            .push(rejection.clone());
        Ok(())
    }

    /// The decisions an owner loads: every accepted batch in order, and the refusals.
    #[must_use]
    pub fn decisions(&self, base: BaseId) -> (Vec<Accepted>, Vec<Outcome>) {
        let accepted = self
            .journal
            .get(&base)
            .map(|j| j.iter().map(|e| e.accepted.clone()).collect())
            .unwrap_or_default();
        let refused = self
            .refusals
            .get(&base)
            .map(|r| r.iter().cloned().map(Outcome::Rejected).collect())
            .unwrap_or_default();
        (accepted, refused)
    }

    /// The accepted batches of a base after a sequence (a client catching up).
    #[must_use]
    pub fn after(&self, base: BaseId, seq: u64) -> Vec<Accepted> {
        self.journal
            .get(&base)
            .map(|j| {
                j.iter()
                    .filter(|e| e.accepted.seq > seq)
                    .map(|e| e.accepted.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Claims due jobs (as `mt-store`): queued and due, or held past their lease.
    pub fn claim(&mut self, now: u64, max: usize, lease_ms: u64) -> Vec<(String, u64)> {
        let mut claimed = Vec::new();
        for (id, job) in &mut self.jobs {
            if claimed.len() >= max {
                break;
            }
            let free = !job.done && job.run_at <= now && job.until.is_none_or(|until| until <= now);
            if free {
                job.token = job.token.saturating_add(1);
                job.until = Some(now.saturating_add(lease_ms));
                claimed.push((id.clone(), job.token));
            }
        }
        claimed
    }

    /// Completes a job for the holder of `token`; returns whether it was still held.
    pub fn complete(&mut self, id: &str, token: u64, now: u64) -> bool {
        match self.jobs.get_mut(id) {
            Some(job) if !job.done && job.token == token && job.until.is_some_and(|u| u > now) => {
                job.done = true;
                job.until = None;
                job.completions = job.completions.saturating_add(1);
                true
            }
            Some(_) | None => false,
        }
    }

    /// The epoch record of a base.
    pub fn epoch(&mut self, base: BaseId) -> u64 {
        self.epochs
            .load(&Db::resource(base))
            .ok()
            .flatten()
            .map_or(0, |r| r.epoch)
    }
}

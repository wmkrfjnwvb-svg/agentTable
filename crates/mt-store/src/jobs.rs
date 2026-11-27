//! A job queue with leases (doc 20 §4.2): any worker of any node claims due jobs; a claim is
//! a lease with a fencing token, so a worker that stalled past its lease cannot complete a
//! job another worker took over. Every job reaches a terminal state: `done`, or `dead` after
//! its last attempt (failed, or its worker stopped answering).
//!
//! Claims lock rows with `FOR UPDATE SKIP LOCKED` on MySQL and PostgreSQL, so workers do not
//! wait on each other; SQLite has a single writer. Either way, the claim itself is a
//! compare-and-set on the token, so a job is never held by two workers at once.

use crate::dialect::check_ident;
use crate::error::{StoreError, StoreResult};
use crate::store::{int, uint, Tenant};
use sqlx::any::AnyRow;

/// The longest wait between attempts (one hour).
pub const MAX_BACKOFF_MS: i64 = 3_600_000;
/// Most jobs claimed at once.
pub const MAX_CLAIM: usize = 100;

/// Where a job stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JobState {
    /// Waiting to run (at `run_at`).
    Queued,
    /// Held by a worker until `lease_until`.
    Running,
    /// Finished.
    Done,
    /// Gave up: every attempt failed or the worker of the last attempt stopped answering.
    Dead,
}

impl JobState {
    /// The stored form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Running => "running",
            JobState::Done => "done",
            JobState::Dead => "dead",
        }
    }

    fn parse(text: &str) -> StoreResult<JobState> {
        match text {
            "queued" => Ok(JobState::Queued),
            "running" => Ok(JobState::Running),
            "done" => Ok(JobState::Done),
            "dead" => Ok(JobState::Dead),
            other => Err(StoreError::Corrupt(format!("job state `{other}`"))),
        }
    }
}

/// The wait before attempt `attempts + 1` after a failure: 1 s, 2 s, 4 s, … up to an hour.
#[must_use]
pub fn backoff(attempts: u32) -> i64 {
    let exponent = attempts.saturating_sub(1).min(31);
    1_000_i64
        .checked_mul(1_i64 << exponent)
        .unwrap_or(MAX_BACKOFF_MS)
        .min(MAX_BACKOFF_MS)
}

/// A job to enqueue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewJob {
    /// The queue (`recompute`, `notify`, …).
    pub queue: String,
    /// What to do (JSON).
    pub payload: String,
    /// Not before this instant (ms).
    pub run_at: i64,
    /// Attempts before giving up (at least 1).
    pub max_attempts: u32,
    /// The trace it belongs to.
    pub trace: Option<String>,
}

/// A stored job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// Its id (the caller's: enqueueing the same id twice is refused, so retries of the
    /// enqueue itself are safe).
    pub id: String,
    /// The queue.
    pub queue: String,
    /// What to do.
    pub payload: String,
    /// Where it stands.
    pub state: JobState,
    /// Attempts started.
    pub attempts: u32,
    /// Attempts allowed.
    pub max_attempts: u32,
    /// Not before (ms).
    pub run_at: i64,
    /// The worker holding it.
    pub lease_owner: Option<String>,
    /// Until when (ms).
    pub lease_until: Option<i64>,
    /// Raised on every claim.
    pub token: u64,
    /// The last failure.
    pub last_error: Option<String>,
    /// When it was enqueued (ms).
    pub created_at: i64,
    /// When it reached `done` or `dead` (ms).
    pub finished_at: Option<i64>,
    /// Its trace.
    pub trace: Option<String>,
}

/// Proof that a worker holds a job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    /// The job.
    pub job: String,
    /// The token of this claim.
    pub token: u64,
    /// The worker.
    pub worker: String,
    /// Valid until (ms).
    pub until: i64,
}

fn job_of(row: &AnyRow) -> StoreResult<Job> {
    let count = |col: &str| -> StoreResult<u32> {
        u32::try_from(crate::row::int(row, col)?)
            .map_err(|_| StoreError::Corrupt(format!("`{col}` out of range")))
    };
    Ok(Job {
        id: crate::row::text(row, "id")?,
        queue: crate::row::text(row, "queue")?,
        payload: crate::row::text(row, "payload")?,
        state: JobState::parse(&crate::row::text(row, "state")?)?,
        attempts: count("attempts")?,
        max_attempts: count("max_attempts")?,
        run_at: crate::row::int(row, "run_at")?,
        lease_owner: crate::row::opt_text(row, "lease_owner")?,
        lease_until: crate::row::opt_int(row, "lease_until")?,
        token: uint(crate::row::int(row, "token")?)?,
        last_error: crate::row::opt_text(row, "last_error")?,
        created_at: crate::row::int(row, "created_at")?,
        finished_at: crate::row::opt_int(row, "finished_at")?,
        trace: crate::row::opt_text(row, "trace")?,
    })
}

const COLUMNS: &str = "id, queue, payload, state, attempts, max_attempts, run_at, lease_owner, \
    lease_until, token, last_error, created_at, finished_at, trace";

fn check_name(what: &str, text: &str) -> StoreResult<()> {
    if text.is_empty() || text.len() > 255 || !text.chars().all(|c| c.is_ascii_graphic()) {
        return Err(StoreError::Invalid(format!("`{text}` is not a {what}")));
    }
    Ok(())
}

impl Tenant {
    /// Adds a job.
    ///
    /// # Errors
    /// [`StoreError::Exists`] for an id in use; [`StoreError::Invalid`] for a bad queue,
    /// id or attempt count; the database's errors.
    pub async fn enqueue(&self, id: &str, job: &NewJob, now_ms: i64) -> StoreResult<()> {
        check_name("job id", id)?;
        check_ident(&job.queue)?;
        if job.max_attempts == 0 {
            return Err(StoreError::Invalid(
                "a job needs at least one attempt".to_owned(),
            ));
        }
        let d = self.dialect;
        let inserted = sqlx::query(&format!(
            "INSERT INTO _jobs ({COLUMNS}) VALUES ({})",
            d.params(1, 14)
        ))
        .bind(id.to_owned())
        .bind(job.queue.clone())
        .bind(job.payload.clone())
        .bind(JobState::Queued.as_str().to_owned())
        .bind(0_i64)
        .bind(i64::from(job.max_attempts))
        .bind(job.run_at)
        .bind(None::<String>)
        .bind(None::<i64>)
        .bind(0_i64)
        .bind(None::<String>)
        .bind(now_ms)
        .bind(None::<i64>)
        .bind(job.trace.clone())
        .execute(&self.pool)
        .await;
        match inserted {
            Ok(_) => Ok(()),
            Err(error) => match self.job(id).await? {
                Some(_) => Err(StoreError::Exists(format!("job {id}"))),
                None => Err(error.into()),
            },
        }
    }

    /// A job.
    ///
    /// # Errors
    /// The database's errors.
    pub async fn job(&self, id: &str) -> StoreResult<Option<Job>> {
        let row = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM _jobs WHERE id = {}",
            self.dialect.param(1)
        ))
        .bind(id.to_owned())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(job_of).transpose()
    }

    /// The jobs of a queue, by id.
    ///
    /// # Errors
    /// The database's errors.
    pub async fn jobs(&self, queue: &str) -> StoreResult<Vec<Job>> {
        check_ident(queue)?;
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM _jobs WHERE queue = {} ORDER BY id",
            self.dialect.param(1)
        ))
        .bind(queue.to_owned())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(job_of).collect()
    }

    /// Claims up to `max` due jobs of a queue for `worker`, each leased for `lease_ms`. A
    /// job whose worker stopped answering (its lease ran out) is due again — or, after its
    /// last attempt, given up (`dead`).
    ///
    /// # Errors
    /// [`StoreError::Invalid`]; the database's errors.
    pub async fn claim(
        &self,
        queue: &str,
        worker: &str,
        max: usize,
        now_ms: i64,
        lease_ms: i64,
    ) -> StoreResult<Vec<(Job, Lease)>> {
        check_ident(queue)?;
        check_name("worker", worker)?;
        if lease_ms <= 0 {
            return Err(StoreError::Invalid("a lease lasts some time".to_owned()));
        }
        let d = self.dialect;
        let until = now_ms
            .checked_add(lease_ms)
            .ok_or_else(|| StoreError::Invalid("lease overflow".to_owned()))?;
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM _jobs WHERE queue = {} AND ((state = 'queued' AND run_at <= {}) \
             OR (state = 'running' AND lease_until <= {})) ORDER BY run_at, id LIMIT {}{}",
            d.param(1),
            d.param(2),
            d.param(3),
            d.param(4),
            d.for_update_skip_locked()
        ))
        .bind(queue.to_owned())
        .bind(now_ms)
        .bind(now_ms)
        .bind(int(u64::try_from(max.min(MAX_CLAIM)).unwrap_or(0))?)
        .fetch_all(&mut *tx)
        .await?;
        let mut claimed = Vec::new();
        for row in &rows {
            let job = job_of(row)?;
            if job.state == JobState::Running && job.attempts >= job.max_attempts {
                sqlx::query(&format!(
                    "UPDATE _jobs SET state = 'dead', finished_at = {}, lease_owner = NULL, lease_until = NULL, \
                     last_error = {} WHERE id = {} AND token = {}",
                    d.param(1),
                    d.param(2),
                    d.param(3),
                    d.param(4)
                ))
                .bind(now_ms)
                .bind("the worker of the last attempt stopped answering".to_owned())
                .bind(job.id.clone())
                .bind(int(job.token)?)
                .execute(&mut *tx)
                .await?;
                continue;
            }
            let token = job
                .token
                .checked_add(1)
                .ok_or_else(|| StoreError::Invalid("token overflow".to_owned()))?;
            let updated = sqlx::query(&format!(
                "UPDATE _jobs SET state = 'running', lease_owner = {}, lease_until = {}, token = {}, \
                 attempts = attempts + 1 WHERE id = {} AND token = {}",
                d.param(1),
                d.param(2),
                d.param(3),
                d.param(4),
                d.param(5)
            ))
            .bind(worker.to_owned())
            .bind(until)
            .bind(int(token)?)
            .bind(job.id.clone())
            .bind(int(job.token)?)
            .execute(&mut *tx)
            .await?;
            if updated.rows_affected() == 1 {
                let lease = Lease {
                    job: job.id.clone(),
                    token,
                    worker: worker.to_owned(),
                    until,
                };
                let job = Job {
                    state: JobState::Running,
                    attempts: job.attempts.saturating_add(1),
                    lease_owner: Some(worker.to_owned()),
                    lease_until: Some(until),
                    token,
                    ..job
                };
                claimed.push((job, lease));
            }
        }
        tx.commit().await?;
        Ok(claimed)
    }

    /// Extends a lease the worker still holds.
    ///
    /// # Errors
    /// [`StoreError::LeaseLost`] when the job was taken over or finished.
    pub async fn heartbeat(&self, lease: &Lease, now_ms: i64, lease_ms: i64) -> StoreResult<Lease> {
        let d = self.dialect;
        let until = now_ms
            .checked_add(lease_ms.max(1))
            .ok_or_else(|| StoreError::Invalid("lease overflow".to_owned()))?;
        let updated = sqlx::query(&format!(
            "UPDATE _jobs SET lease_until = {} WHERE id = {} AND token = {} AND state = 'running' AND lease_owner = {}",
            d.param(1),
            d.param(2),
            d.param(3),
            d.param(4)
        ))
        .bind(until)
        .bind(lease.job.clone())
        .bind(int(lease.token)?)
        .bind(lease.worker.clone())
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() == 1 {
            Ok(Lease {
                until,
                ..lease.clone()
            })
        } else {
            Err(StoreError::LeaseLost(lease.job.clone()))
        }
    }

    /// Marks a held job done.
    ///
    /// # Errors
    /// [`StoreError::LeaseLost`] when the worker no longer holds it.
    pub async fn complete(&self, lease: &Lease, now_ms: i64) -> StoreResult<()> {
        let d = self.dialect;
        let updated = sqlx::query(&format!(
            "UPDATE _jobs SET state = 'done', finished_at = {}, lease_owner = NULL, lease_until = NULL \
             WHERE id = {} AND token = {} AND state = 'running'",
            d.param(1),
            d.param(2),
            d.param(3)
        ))
        .bind(now_ms)
        .bind(lease.job.clone())
        .bind(int(lease.token)?)
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() == 1 {
            Ok(())
        } else {
            Err(StoreError::LeaseLost(lease.job.clone()))
        }
    }

    /// Records a failed attempt: the job waits (doubling, up to an hour) and runs again, or
    /// is given up after its last attempt. Returns its new state.
    ///
    /// # Errors
    /// [`StoreError::LeaseLost`] when the worker no longer holds it.
    pub async fn fail(&self, lease: &Lease, error: &str, now_ms: i64) -> StoreResult<JobState> {
        let d = self.dialect;
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM _jobs WHERE id = {} AND token = {} AND state = 'running'{}",
            d.param(1),
            d.param(2),
            d.for_update()
        ))
        .bind(lease.job.clone())
        .bind(int(lease.token)?)
        .fetch_optional(&mut *tx)
        .await?;
        let job = row
            .as_ref()
            .map(job_of)
            .transpose()?
            .ok_or_else(|| StoreError::LeaseLost(lease.job.clone()))?;
        let error: String = error.chars().take(2_000).collect();
        let state = if job.attempts < job.max_attempts {
            sqlx::query(&format!(
                "UPDATE _jobs SET state = 'queued', run_at = {}, last_error = {}, lease_owner = NULL, lease_until = NULL \
                 WHERE id = {} AND token = {}",
                d.param(1),
                d.param(2),
                d.param(3),
                d.param(4)
            ))
            .bind(now_ms.saturating_add(backoff(job.attempts)))
            .bind(error)
            .bind(lease.job.clone())
            .bind(int(lease.token)?)
            .execute(&mut *tx)
            .await?;
            JobState::Queued
        } else {
            sqlx::query(&format!(
                "UPDATE _jobs SET state = 'dead', finished_at = {}, last_error = {}, lease_owner = NULL, lease_until = NULL \
                 WHERE id = {} AND token = {}",
                d.param(1),
                d.param(2),
                d.param(3),
                d.param(4)
            ))
            .bind(now_ms)
            .bind(error)
            .bind(lease.job.clone())
            .bind(int(lease.token)?)
            .execute(&mut *tx)
            .await?;
            JobState::Dead
        };
        tx.commit().await?;
        Ok(state)
    }

    /// Queues a dead job again for one more round of attempts (an administrator's retry).
    ///
    /// # Errors
    /// [`StoreError::Conflict`] when it is not dead.
    pub async fn revive(&self, id: &str, now_ms: i64) -> StoreResult<()> {
        let d = self.dialect;
        let updated = sqlx::query(&format!(
            "UPDATE _jobs SET state = 'queued', run_at = {}, attempts = 0, finished_at = NULL WHERE id = {} AND state = 'dead'",
            d.param(1),
            d.param(2)
        ))
        .bind(now_ms)
        .bind(id.to_owned())
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() == 1 {
            Ok(())
        } else {
            Err(StoreError::Conflict(format!("job {id} is not dead")))
        }
    }
}

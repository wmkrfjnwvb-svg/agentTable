//! The job queue with leases, on every engine.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_store::jobs::{backoff, MAX_BACKOFF_MS};
use mt_store::{JobState, NewJob, StoreError};
use std::collections::BTreeMap;
use std::sync::Arc;

fn job(queue: &str, run_at: i64, max_attempts: u32) -> NewJob {
    NewJob {
        queue: queue.to_owned(),
        payload: r#"{"record":"rec_1"}"#.to_owned(),
        run_at,
        max_attempts,
        trace: Some("trc_1".to_owned()),
    }
}

#[test]
fn std_backoff_doubles_up_to_an_hour() {
    assert_eq!(
        [backoff(0), backoff(1), backoff(2), backoff(3), backoff(10)],
        [1_000, 1_000, 2_000, 4_000, 512_000]
    );
    assert_eq!(backoff(13), MAX_BACKOFF_MS);
    assert_eq!(backoff(u32::MAX), MAX_BACKOFF_MS);
    let names: Vec<&str> = [
        JobState::Queued,
        JobState::Running,
        JobState::Done,
        JobState::Dead,
    ]
    .iter()
    .map(|s| s.as_str())
    .collect();
    assert_eq!(names, ["queued", "running", "done", "dead"]);
}

#[tokio::test]
async fn std_a_job_runs_fails_retries_and_finishes() {
    on_every_db(|store| async move {
        let t = store.create_tenant(tenant(1), "Acme", NOW).await.unwrap();
        t.enqueue("job_a", &job("recompute", NOW, 3), NOW)
            .await
            .unwrap();
        t.enqueue("job_later", &job("recompute", NOW + 60_000, 3), NOW)
            .await
            .unwrap();
        t.enqueue("job_other", &job("notify", NOW, 3), NOW)
            .await
            .unwrap();
        assert!(matches!(
            t.enqueue("job_a", &job("recompute", NOW, 3), NOW).await,
            Err(StoreError::Exists(_))
        ));
        // Only what is due, only from this queue.
        let claimed = t.claim("recompute", "w1", 10, NOW, 30_000).await.unwrap();
        assert_eq!(claimed.len(), 1, "{}", name(store.dialect()));
        let (running, lease) = &claimed[0];
        assert_eq!(
            (
                running.id.as_str(),
                running.state,
                running.attempts,
                running.token
            ),
            ("job_a", JobState::Running, 1, 1)
        );
        assert_eq!(running.lease_owner.as_deref(), Some("w1"));
        assert_eq!(lease.until, NOW + 30_000);
        assert!(
            t.claim("recompute", "w2", 10, NOW + 1, 30_000)
                .await
                .unwrap()
                .is_empty(),
            "held"
        );
        // Fails: back in the queue after the backoff.
        assert_eq!(
            t.fail(lease, "timeout talking to the mail server", NOW + 5)
                .await
                .unwrap(),
            JobState::Queued
        );
        let queued = t.job("job_a").await.unwrap().unwrap();
        assert_eq!(
            (queued.state, queued.run_at, queued.lease_owner),
            (JobState::Queued, NOW + 5 + 1_000, None)
        );
        assert_eq!(
            queued.last_error.as_deref(),
            Some("timeout talking to the mail server")
        );
        assert!(
            matches!(t.complete(lease, NOW).await, Err(StoreError::LeaseLost(_))),
            "that claim is over"
        );
        // Second attempt succeeds.
        let again = t
            .claim("recompute", "w2", 10, NOW + 2_000, 30_000)
            .await
            .unwrap();
        assert_eq!(again.len(), 1);
        let lease = &again[0].1;
        assert_eq!((again[0].0.attempts, lease.token), (2, 2));
        let longer = t.heartbeat(lease, NOW + 20_000, 30_000).await.unwrap();
        assert_eq!(longer.until, NOW + 50_000);
        t.complete(&longer, NOW + 21_000).await.unwrap();
        let done = t.job("job_a").await.unwrap().unwrap();
        assert_eq!(
            (done.state, done.finished_at, done.trace.as_deref()),
            (JobState::Done, Some(NOW + 21_000), Some("trc_1"))
        );
        assert!(matches!(
            t.heartbeat(&longer, NOW, 1).await,
            Err(StoreError::LeaseLost(_))
        ));
        assert!(matches!(
            t.fail(&longer, "x", NOW).await,
            Err(StoreError::LeaseLost(_))
        ));
        assert_eq!(t.jobs("recompute").await.unwrap().len(), 2);
        store
    })
    .await;
}

#[tokio::test]
async fn edge_a_stalled_worker_loses_its_job_and_the_last_attempt_ends_dead() {
    on_every_db(|store| async move {
        let t = store.create_tenant(tenant(1), "Acme", NOW).await.unwrap();
        t.enqueue("job_x", &job("sync", NOW, 2), NOW).await.unwrap();
        let first = t
            .claim("sync", "slow", 1, NOW, 1_000)
            .await
            .unwrap()
            .remove(0)
            .1;
        // The lease runs out; another worker takes the job over.
        let second = t
            .claim("sync", "fast", 1, NOW + 1_000, 1_000)
            .await
            .unwrap()
            .remove(0)
            .1;
        assert_eq!(second.token, 2);
        // The slow worker wakes up: it can neither finish, fail nor extend the job.
        assert!(matches!(
            t.complete(&first, NOW + 1_500).await,
            Err(StoreError::LeaseLost(_))
        ));
        assert!(matches!(
            t.fail(&first, "late", NOW + 1_500).await,
            Err(StoreError::LeaseLost(_))
        ));
        assert!(matches!(
            t.heartbeat(&first, NOW + 1_500, 1_000).await,
            Err(StoreError::LeaseLost(_))
        ));
        // The second (last) attempt's worker stops answering too: the job is given up.
        assert!(t
            .claim("sync", "w3", 1, NOW + 2_000, 1_000)
            .await
            .unwrap()
            .is_empty());
        let dead = t.job("job_x").await.unwrap().unwrap();
        assert_eq!(dead.state, JobState::Dead);
        assert_eq!(
            dead.last_error.as_deref(),
            Some("the worker of the last attempt stopped answering")
        );
        // An administrator retries it.
        assert!(matches!(t.revive("job_x", NOW).await, Ok(())));
        assert!(matches!(
            t.revive("job_x", NOW).await,
            Err(StoreError::Conflict(_))
        ));
        let revived = t
            .claim("sync", "w4", 1, NOW + 3_000, 1_000)
            .await
            .unwrap()
            .remove(0);
        assert_eq!(revived.0.attempts, 1);
        assert_eq!(
            t.fail(&revived.1, "still broken", NOW + 3_001)
                .await
                .unwrap(),
            JobState::Queued
        );
        let last = t
            .claim("sync", "w4", 1, NOW + 10_000, 1_000)
            .await
            .unwrap()
            .remove(0)
            .1;
        assert_eq!(
            t.fail(&last, "broken for good", NOW + 10_001)
                .await
                .unwrap(),
            JobState::Dead
        );
        assert_eq!(
            t.job("job_x").await.unwrap().unwrap().finished_at,
            Some(NOW + 10_001)
        );
        store
    })
    .await;
}

#[tokio::test]
async fn invalid_jobs_are_refused() {
    on_every_db(|store| async move {
        let t = store.create_tenant(tenant(1), "Acme", NOW).await.unwrap();
        assert!(matches!(
            t.enqueue("job_a", &job("Bad Queue", NOW, 1), NOW).await,
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            t.enqueue("", &job("q", NOW, 1), NOW).await,
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            t.enqueue("job_a", &job("q", NOW, 0), NOW).await,
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            t.claim("q", "has space", 1, NOW, 1).await,
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            t.claim("q", "w", 1, NOW, 0).await,
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            t.claim("q", "w", 1, i64::MAX, 10).await,
            Err(StoreError::Invalid(_))
        ));
        assert!(t.job("job_none").await.unwrap().is_none());
        store
    })
    .await;
}

/// Many workers claim at once: every job is done exactly once, and no job is held by two
/// workers at the same time.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrency_workers_claiming_together_run_each_job_once() {
    on_every_db(|store| async move {
        let t = Arc::new(store.create_tenant(tenant(1), "Acme", NOW).await.unwrap());
        let jobs = 60;
        for n in 0..jobs {
            t.enqueue(&format!("job_{n:03}"), &job("work", NOW, 3), NOW)
                .await
                .unwrap();
        }
        let mut handles = Vec::new();
        for w in 0..6 {
            let t = t.clone();
            handles.push(tokio::spawn(async move {
                let mut done = Vec::new();
                loop {
                    let claimed = t
                        .claim("work", &format!("worker{w}"), 4, NOW, 600_000)
                        .await
                        .unwrap();
                    if claimed.is_empty() {
                        break;
                    }
                    for (job, lease) in claimed {
                        t.complete(&lease, NOW + 1).await.unwrap();
                        done.push(job.id);
                    }
                }
                done
            }));
        }
        let mut count: BTreeMap<String, usize> = BTreeMap::new();
        for handle in handles {
            for id in handle.await.unwrap() {
                *count.entry(id).or_default() += 1;
            }
        }
        assert_eq!(count.len(), jobs, "{}", name(store.dialect()));
        assert!(
            count.values().all(|n| *n == 1),
            "a job ran twice: {count:?}"
        );
        let all = t.jobs("work").await.unwrap();
        assert!(all
            .iter()
            .all(|j| j.state == JobState::Done && j.attempts == 1 && j.token == 1));
        store
    })
    .await;
}

/// The reference: the queue's rules written plainly over a map.
#[derive(Debug, Clone, PartialEq)]
struct RefJob {
    state: JobState,
    attempts: u32,
    max: u32,
    run_at: i64,
    owner: Option<String>,
    until: Option<i64>,
    token: u64,
}

fn reference_claim(
    model: &mut BTreeMap<String, RefJob>,
    worker: &str,
    max: usize,
    now: i64,
    lease: i64,
) -> Vec<(String, u64)> {
    let mut due: Vec<(i64, String)> = model
        .iter()
        .filter(|(_, j)| {
            (j.state == JobState::Queued && j.run_at <= now)
                || (j.state == JobState::Running && j.until.unwrap_or(i64::MAX) <= now)
        })
        .map(|(id, j)| (j.run_at, id.clone()))
        .collect();
    due.sort();
    let mut claimed = Vec::new();
    for (_, id) in due.into_iter().take(max) {
        let j = model.get_mut(&id).unwrap();
        if j.state == JobState::Running && j.attempts >= j.max {
            *j = RefJob {
                state: JobState::Dead,
                owner: None,
                until: None,
                ..j.clone()
            };
            continue;
        }
        j.token += 1;
        j.attempts += 1;
        j.state = JobState::Running;
        j.owner = Some(worker.to_owned());
        j.until = Some(now + lease);
        claimed.push((id, j.token));
    }
    claimed
}

#[tokio::test]
async fn model_the_queue_follows_its_rules() {
    on_every_db(|store| async move {
        let t = store.create_tenant(tenant(1), "Acme", NOW).await.unwrap();
        let mut model: BTreeMap<String, RefJob> = BTreeMap::new();
        let mut leases: Vec<mt_store::Lease> = Vec::new();
        let mut now = NOW;
        let mut rng = 0x9E37_79B9_u64;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for step in 0..150 {
            match next() % 7 {
                0 => {
                    let id = format!("job_{step:03}");
                    let max = 1 + (next() % 3) as u32;
                    let at = now + (next() % 3) as i64 * 500;
                    t.enqueue(&id, &job("q", at, max), now).await.unwrap();
                    model.insert(
                        id,
                        RefJob {
                            state: JobState::Queued,
                            attempts: 0,
                            max,
                            run_at: at,
                            owner: None,
                            until: None,
                            token: 0,
                        },
                    );
                }
                1 | 2 => {
                    let worker = format!("w{}", next() % 3);
                    let max = 1 + (next() % 3) as usize;
                    let got = t.claim("q", &worker, max, now, 1_000).await.unwrap();
                    let want = reference_claim(&mut model, &worker, max, now, 1_000);
                    let got_ids: Vec<(String, u64)> =
                        got.iter().map(|(j, l)| (j.id.clone(), l.token)).collect();
                    assert_eq!(got_ids, want, "{} step {step}", name(store.dialect()));
                    leases.extend(got.into_iter().map(|(_, l)| l));
                }
                3 | 4 if !leases.is_empty() => {
                    let lease = leases.remove((next() % leases.len() as u64) as usize);
                    let holds = model
                        .get(&lease.job)
                        .is_some_and(|j| j.state == JobState::Running && j.token == lease.token);
                    let j = model.get_mut(&lease.job).unwrap();
                    if next() % 2 == 0 {
                        let result = t.complete(&lease, now).await;
                        assert_eq!(result.is_ok(), holds, "complete at step {step}");
                        if holds {
                            *j = RefJob {
                                state: JobState::Done,
                                owner: None,
                                until: None,
                                ..j.clone()
                            };
                        }
                    } else {
                        let result = t.fail(&lease, "boom", now).await;
                        assert_eq!(result.is_ok(), holds, "fail at step {step}");
                        if holds {
                            if j.attempts < j.max {
                                *j = RefJob {
                                    state: JobState::Queued,
                                    run_at: now + backoff(j.attempts),
                                    owner: None,
                                    until: None,
                                    ..j.clone()
                                };
                            } else {
                                *j = RefJob {
                                    state: JobState::Dead,
                                    owner: None,
                                    until: None,
                                    ..j.clone()
                                };
                            }
                            assert_eq!(result.unwrap(), j.state);
                        }
                    }
                }
                5 if !leases.is_empty() => {
                    let lease = leases[(next() % leases.len() as u64) as usize].clone();
                    let holds = model.get(&lease.job).is_some_and(|j| {
                        j.state == JobState::Running
                            && j.token == lease.token
                            && j.owner.as_deref() == Some(lease.worker.as_str())
                    });
                    let result = t.heartbeat(&lease, now, 1_000).await;
                    assert_eq!(result.is_ok(), holds, "heartbeat at step {step}");
                    if holds {
                        model.get_mut(&lease.job).unwrap().until = Some(now + 1_000);
                    }
                }
                _ => now += (next() % 4) as i64 * 400,
            }
            for (id, want) in &model {
                let got = t.job(id).await.unwrap().unwrap();
                let got = RefJob {
                    state: got.state,
                    attempts: got.attempts,
                    max: got.max_attempts,
                    run_at: got.run_at,
                    owner: got.lease_owner,
                    until: got.lease_until,
                    token: got.token,
                };
                assert_eq!(&got, want, "{} step {step}: {id}", name(store.dialect()));
            }
        }
        store
    })
    .await;
}

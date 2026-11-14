//! Lease-based shard scheduler for timers, SLA clocks and cron jobs.
//!
//! Work is split into `shard_count` shards ([`shard_of`] maps a tenant / job key to a shard).
//! Each shard has one lease record in a shared store:
//! `(term, holder, expires_at_ms, cursor_ms)`.
//!
//! * The rendezvous owner of a shard claims its lease. A lease held by another instance can
//!   only be taken after it expired; every take-over raises `term` (the fencing token).
//! * The holder fires due work by advancing `cursor_ms` with a compare-and-set
//!   ([`advance`]). The returned [`FireWindow`] `(after, upto]` is the only licence to fire
//!   timers due in that range. Because the cursor only moves forward under the fence,
//!   windows never overlap: no timer slot fires twice, within a term or across terms.
//! * `claim`, `renew`, `release` and `advance` are idempotent and safe to retry.
//!
//! `now_ms` should come from the store's clock (database time) so that expiry is judged
//! consistently; node clocks that disagree can cost availability but never cause a double
//! fire, because firing is fenced by the term and the cursor, not by time.

use crate::error::ClusterError;
use crate::hash::stable_hash64;
use crate::node::NodeId;
use crate::placement::Placement;
use std::collections::BTreeMap;

/// Stored lease of one shard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseRecord {
    /// Fencing term; raised on every change of holder.
    pub term: u64,
    /// Current holder, `None` after a release.
    pub holder: Option<NodeId>,
    /// When the lease expires (ms).
    pub expires_at_ms: u64,
    /// Everything due at or before this instant has been fired.
    pub cursor_ms: u64,
}

/// Proof of holding a shard lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseToken {
    /// The shard.
    pub shard: u32,
    /// The lease term.
    pub term: u64,
    /// The holder instance.
    pub holder: NodeId,
}

/// Licence to fire everything due in `(after_ms, upto_ms]` for a shard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FireWindow {
    /// The shard.
    pub shard: u32,
    /// Term under which the window was committed.
    pub term: u64,
    /// Exclusive lower bound (ms).
    pub after_ms: u64,
    /// Inclusive upper bound (ms).
    pub upto_ms: u64,
}

/// Compare-and-set storage of lease records (atomic and linearizable, like `EpochStore`).
pub trait LeaseStore {
    /// Reads the lease of a shard.
    fn load(&mut self, shard: u32) -> Result<Option<LeaseRecord>, ClusterError>;

    /// Replaces the record with `new` if it currently equals `expected`.
    fn compare_and_set(
        &mut self,
        shard: u32,
        expected: Option<&LeaseRecord>,
        new: LeaseRecord,
    ) -> Result<bool, ClusterError>;
}

/// The placement key of a shard (`"shard:"` + big-endian index).
#[must_use]
pub fn shard_key(shard: u32) -> [u8; 10] {
    let mut key = *b"shard:\0\0\0\0";
    for (dst, src) in key.iter_mut().skip(6).zip(shard.to_be_bytes()) {
        *dst = src;
    }
    key
}

/// The shard a key (tenant id, job id) belongs to. Returns 0 when `shard_count` is 0.
#[must_use]
pub fn shard_of(key: &[u8], shard_count: u32) -> u32 {
    let r = stable_hash64(key)
        .checked_rem(u64::from(shard_count))
        .unwrap_or(0);
    u32::try_from(r).unwrap_or(0)
}

fn holds(rec: &LeaseRecord, token: &LeaseToken) -> bool {
    rec.term == token.term && rec.holder.as_ref() == Some(&token.holder)
}

fn load_held<S: LeaseStore + ?Sized>(
    store: &mut S,
    token: &LeaseToken,
) -> Result<LeaseRecord, ClusterError> {
    let rec = store.load(token.shard)?;
    match rec {
        Some(r) if holds(&r, token) => Ok(r),
        Some(r) => Err(ClusterError::Fenced { current: r.term }),
        None => Err(ClusterError::Fenced { current: 0 }),
    }
}

/// Claims the lease of `shard` for `me`. Succeeds when the lease is free, expired, or
/// already held by `me` (idempotent, same term). Fails with `LeaseHeld` while another
/// instance holds an unexpired lease and with `Contended` on a lost race.
pub fn claim<S: LeaseStore + ?Sized>(
    store: &mut S,
    shard: u32,
    me: &NodeId,
    now_ms: u64,
    ttl_ms: u64,
) -> Result<LeaseToken, ClusterError> {
    claim_with_expiry(store, shard, me, now_ms, ttl_ms).map(|(token, _)| token)
}

/// [`claim`], also returning the expiry stored for the lease. An idempotent claim keeps the
/// existing expiry, which can be earlier than `now_ms + ttl_ms`.
fn claim_with_expiry<S: LeaseStore + ?Sized>(
    store: &mut S,
    shard: u32,
    me: &NodeId,
    now_ms: u64,
    ttl_ms: u64,
) -> Result<(LeaseToken, u64), ClusterError> {
    let current = store.load(shard)?;
    let (term, cursor_ms) = match &current {
        None => (1, 0),
        Some(r) => {
            // A holder counts only while its lease is unexpired.
            let live_holder = r.holder.as_ref().filter(|_| now_ms < r.expires_at_ms);
            match live_holder {
                Some(h) if h == me => {
                    let token = LeaseToken {
                        shard,
                        term: r.term,
                        holder: me.clone(),
                    };
                    return Ok((token, r.expires_at_ms));
                }
                Some(_) => {
                    return Err(ClusterError::LeaseHeld {
                        until_ms: r.expires_at_ms,
                    });
                }
                None => {}
            }
            (
                r.term.checked_add(1).ok_or(ClusterError::EpochOverflow)?,
                r.cursor_ms,
            )
        }
    };
    let expires_at_ms = now_ms.saturating_add(ttl_ms);
    let new = LeaseRecord {
        term,
        holder: Some(me.clone()),
        expires_at_ms,
        cursor_ms,
    };
    if !store.compare_and_set(shard, current.as_ref(), new)? {
        return Err(ClusterError::Contended);
    }
    let token = LeaseToken {
        shard,
        term,
        holder: me.clone(),
    };
    Ok((token, expires_at_ms))
}

/// Extends the lease to `now_ms + ttl_ms` (never shortens it). Fails with `Fenced` when the
/// token is no longer the current lease. Returns the new expiry.
pub fn renew<S: LeaseStore + ?Sized>(
    store: &mut S,
    token: &LeaseToken,
    now_ms: u64,
    ttl_ms: u64,
) -> Result<u64, ClusterError> {
    let rec = load_held(store, token)?;
    let expires_at_ms = now_ms.saturating_add(ttl_ms).max(rec.expires_at_ms);
    let new = LeaseRecord {
        expires_at_ms,
        ..rec.clone()
    };
    if !store.compare_and_set(token.shard, Some(&rec), new)? {
        return Err(ClusterError::Contended);
    }
    Ok(expires_at_ms)
}

/// Releases the lease so another node can claim it at once. Returns `false` when the token
/// was not the current lease any more (nothing changed).
pub fn release<S: LeaseStore + ?Sized>(
    store: &mut S,
    token: &LeaseToken,
) -> Result<bool, ClusterError> {
    let Some(rec) = store.load(token.shard)?.filter(|r| holds(r, token)) else {
        return Ok(false);
    };
    let new = LeaseRecord {
        holder: None,
        expires_at_ms: 0,
        ..rec.clone()
    };
    store.compare_and_set(token.shard, Some(&rec), new)
}

/// Commits the firing of everything due up to `upto_ms`. Returns the window to fire, or
/// `None` when the cursor is already there (idempotent retry). Fails with `Fenced` when the
/// token is stale and `LeaseExpired` when the lease ran out (renew first).
///
/// The cursor moves *before* the caller acts on the window, so on its own this is
/// at-most-once: a holder that crashes right after this call never fires the window. A
/// database-backed store must therefore make the work durable in the same transaction as
/// the cursor move (insert the due jobs into the outbox, then let workers run them); that
/// turns the window into exactly-once hand-over to a durable queue.
pub fn advance<S: LeaseStore + ?Sized>(
    store: &mut S,
    token: &LeaseToken,
    now_ms: u64,
    upto_ms: u64,
) -> Result<Option<FireWindow>, ClusterError> {
    let rec = load_held(store, token)?;
    if now_ms >= rec.expires_at_ms {
        return Err(ClusterError::LeaseExpired);
    }
    if upto_ms <= rec.cursor_ms {
        return Ok(None);
    }
    let new = LeaseRecord {
        cursor_ms: upto_ms,
        ..rec.clone()
    };
    if !store.compare_and_set(token.shard, Some(&rec), new)? {
        return Err(ClusterError::Contended);
    }
    Ok(Some(FireWindow {
        shard: token.shard,
        term: token.term,
        after_ms: rec.cursor_ms,
        upto_ms,
    }))
}

/// Tunables of the node-side scheduler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerConfig {
    /// Number of shards in the cluster (must be equal on all nodes).
    pub shard_count: u32,
    /// Lease duration (ms).
    pub lease_ttl_ms: u64,
    /// Renew when less than this remains (ms).
    pub renew_before_ms: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            shard_count: 64,
            lease_ttl_ms: 15_000,
            renew_before_ms: 10_000,
        }
    }
}

/// What the scheduler did during a tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedEvent {
    /// A lease was claimed.
    Claimed(LeaseToken),
    /// A lease was extended.
    Renewed(u32),
    /// A lease was released because the shard now belongs to another node.
    Released(u32),
    /// A lease was lost (fenced by another node, or expired while the store was down).
    Lost(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Held {
    term: u64,
    expires_at_ms: u64,
}

/// Node-side state machine: keeps the leases of the shards placement assigns to this node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardScheduler {
    cfg: SchedulerConfig,
    me: NodeId,
    held: BTreeMap<u32, Held>,
}

impl ShardScheduler {
    /// Creates a scheduler holding no leases.
    #[must_use]
    pub fn new(cfg: SchedulerConfig, me: NodeId) -> Self {
        Self {
            cfg,
            me,
            held: BTreeMap::new(),
        }
    }

    /// Switches to a new identity (restart or `Event::Renewed`) and forgets all leases.
    pub fn reset(&mut self, me: NodeId) {
        self.me = me;
        self.held.clear();
    }

    /// The lease token of a shard, if held.
    #[must_use]
    pub fn token(&self, shard: u32) -> Option<LeaseToken> {
        self.held.get(&shard).map(|h| LeaseToken {
            shard,
            term: h.term,
            holder: self.me.clone(),
        })
    }

    /// Shards currently held.
    pub fn held_shards(&self) -> impl Iterator<Item = u32> + '_ {
        self.held.keys().copied()
    }

    /// Reconciles held leases with `placement`: claims shards this node should own,
    /// renews those close to expiry, releases those that moved away. Pass a placement
    /// without this node to drain. Store errors are retried on the next tick.
    pub fn tick<S: LeaseStore + ?Sized>(
        &mut self,
        now_ms: u64,
        placement: &Placement,
        store: &mut S,
    ) -> Vec<SchedEvent> {
        let mut events = Vec::new();
        for shard in 0..self.cfg.shard_count {
            let desired = placement.owner(&shard_key(shard)) == Some(&self.me);
            match (self.token(shard), desired) {
                (None, false) => {}
                (None, true) => {
                    let claimed =
                        claim_with_expiry(store, shard, &self.me, now_ms, self.cfg.lease_ttl_ms);
                    if let Ok((t, expires_at_ms)) = claimed {
                        self.held.insert(
                            shard,
                            Held {
                                term: t.term,
                                expires_at_ms,
                            },
                        );
                        events.push(SchedEvent::Claimed(t));
                    }
                }
                (Some(t), false) => {
                    if release(store, &t).is_ok() {
                        self.held.remove(&shard);
                        events.push(SchedEvent::Released(shard));
                    }
                }
                (Some(t), true) => self.keep(now_ms, &t, store, &mut events),
            }
        }
        events
    }

    fn keep<S: LeaseStore + ?Sized>(
        &mut self,
        now_ms: u64,
        token: &LeaseToken,
        store: &mut S,
        events: &mut Vec<SchedEvent>,
    ) {
        let expires = self.held.get(&token.shard).map_or(0, |h| h.expires_at_ms);
        if now_ms.saturating_add(self.cfg.renew_before_ms) < expires {
            return;
        }
        match renew(store, token, now_ms, self.cfg.lease_ttl_ms) {
            Ok(expires_at_ms) => {
                self.held.insert(
                    token.shard,
                    Held {
                        term: token.term,
                        expires_at_ms,
                    },
                );
                events.push(SchedEvent::Renewed(token.shard));
            }
            Err(e) => {
                if e.is_fenced() || now_ms >= expires {
                    self.held.remove(&token.shard);
                    events.push(SchedEvent::Lost(token.shard));
                }
            }
        }
    }

    /// Commits firing for a held shard up to `upto_ms` (see [`advance`]). On `Fenced` or
    /// `LeaseExpired` the lease is dropped locally; `NotHeld` when the shard is not held.
    pub fn fire<S: LeaseStore + ?Sized>(
        &mut self,
        now_ms: u64,
        store: &mut S,
        shard: u32,
        upto_ms: u64,
    ) -> Result<Option<FireWindow>, ClusterError> {
        let token = self.token(shard).ok_or(ClusterError::NotHeld)?;
        let result = advance(store, &token, now_ms, upto_ms);
        if let Err(e) = &result {
            if e.is_fenced() || *e == ClusterError::LeaseExpired {
                self.held.remove(&shard);
            }
        }
        result
    }
}

//! Single-writer safety with fencing epochs.
//!
//! Each resource (a base) has a record `(epoch, owner)` in a store shared by all nodes
//! (the tenant database). The rules:
//!
//! 1. A node becomes owner only by winning a compare-and-set that raises the epoch by one.
//! 2. Every write carries a [`FencingToken`]; the store accepts it only while the record
//!    still equals `(token.epoch, token.owner)` ([`check_token`], which a real store runs
//!    inside the write transaction).
//! 3. Releasing keeps the epoch and clears the owner, so the old token is dead at once.
//!
//! Therefore at most one node instance can have writes accepted for a given
//! `(resource, epoch)`, whatever the membership views are: a partitioned old owner is
//! rejected as soon as someone else acquires. Membership and placement only decide *who
//! should try* to acquire; they are never trusted for safety.

use crate::error::ClusterError;
use crate::node::NodeId;
use crate::placement::Placement;
use std::collections::BTreeMap;

/// The stored ownership record of a resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpochRecord {
    /// Monotonic epoch; raised by one on every acquisition.
    pub epoch: u64,
    /// Current owner instance, or `None` after a release.
    pub owner: Option<NodeId>,
}

/// Proof of ownership attached to every write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FencingToken {
    /// The resource.
    pub resource: Vec<u8>,
    /// The epoch the owner acquired.
    pub epoch: u64,
    /// The owner instance.
    pub owner: NodeId,
}

/// Compare-and-set storage of epoch records. Implementations must make `compare_and_set`
/// atomic and linearizable (a database row update with a `WHERE epoch = ? AND owner = ?`).
pub trait EpochStore {
    /// Reads the record of a resource.
    fn load(&mut self, resource: &[u8]) -> Result<Option<EpochRecord>, ClusterError>;

    /// Replaces the record with `new` if it currently equals `expected`.
    /// Returns `Ok(false)` when it does not.
    fn compare_and_set(
        &mut self,
        resource: &[u8],
        expected: Option<&EpochRecord>,
        new: EpochRecord,
    ) -> Result<bool, ClusterError>;
}

/// Checks that `token` is still the current owner record. A real store must run this in the
/// same transaction as the write it guards.
pub fn check_token<S: EpochStore + ?Sized>(
    store: &mut S,
    token: &FencingToken,
) -> Result<(), ClusterError> {
    let current = store.load(&token.resource)?;
    let epoch = current.as_ref().map_or(0, |r| r.epoch);
    let ok =
        current.is_some_and(|r| r.epoch == token.epoch && r.owner.as_ref() == Some(&token.owner));
    if ok {
        Ok(())
    } else {
        Err(ClusterError::Fenced { current: epoch })
    }
}

/// Node-side ownership state machine: which epochs this node instance holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fencer {
    me: NodeId,
    held: BTreeMap<Vec<u8>, u64>,
    draining: bool,
}

impl Fencer {
    /// Creates the state for node instance `me`, holding nothing.
    #[must_use]
    pub fn new(me: NodeId) -> Self {
        Self {
            me,
            held: BTreeMap::new(),
            draining: false,
        }
    }

    /// The instance this state belongs to.
    #[must_use]
    pub fn me(&self) -> &NodeId {
        &self.me
    }

    /// Switches to a new identity (after `Event::Renewed` or a restart) and forgets every
    /// held epoch: they belonged to the old instance and others may have taken over.
    pub fn reset(&mut self, me: NodeId) {
        self.me = me;
        self.held.clear();
    }

    /// The epoch held for a resource, if any.
    #[must_use]
    pub fn held_epoch(&self, resource: &[u8]) -> Option<u64> {
        self.held.get(resource).copied()
    }

    /// The token to attach to writes, if the resource is held.
    #[must_use]
    pub fn token(&self, resource: &[u8]) -> Option<FencingToken> {
        self.held_epoch(resource).map(|epoch| FencingToken {
            resource: resource.to_vec(),
            epoch,
            owner: self.me.clone(),
        })
    }

    /// Resources currently held, in key order.
    pub fn held(&self) -> impl Iterator<Item = &[u8]> {
        self.held.keys().map(Vec::as_slice)
    }

    /// True once draining has begun.
    #[must_use]
    pub fn is_draining(&self) -> bool {
        self.draining
    }

    /// True when draining and nothing is held any more: the node may leave.
    #[must_use]
    pub fn is_drained(&self) -> bool {
        self.draining && self.held.is_empty()
    }

    /// Starts a graceful drain: no new acquisitions; the caller then releases every
    /// [`Fencer::held`] resource and finally leaves the cluster.
    pub fn begin_drain(&mut self) {
        self.draining = true;
    }

    /// Becomes the owner of `resource` by raising its epoch. Idempotent while the store
    /// still shows this instance at the held epoch. Fails with `Contended` when another
    /// node won the race, and with `Draining` during a drain.
    ///
    /// This takes ownership away from any previous owner; call it only when routing says
    /// this node should own the resource.
    pub fn acquire<S: EpochStore + ?Sized>(
        &mut self,
        store: &mut S,
        resource: &[u8],
    ) -> Result<FencingToken, ClusterError> {
        if self.draining {
            return Err(ClusterError::Draining);
        }
        let current = store.load(resource)?;
        let held = self.held_epoch(resource);
        let next = match &current {
            None => 1,
            Some(r) => {
                if held == Some(r.epoch) && r.owner.as_ref() == Some(&self.me) {
                    return Ok(FencingToken {
                        resource: resource.to_vec(),
                        epoch: r.epoch,
                        owner: self.me.clone(),
                    });
                }
                r.epoch.checked_add(1).ok_or(ClusterError::EpochOverflow)?
            }
        };
        self.held.remove(resource);
        let new = EpochRecord {
            epoch: next,
            owner: Some(self.me.clone()),
        };
        if !store.compare_and_set(resource, current.as_ref(), new)? {
            return Err(ClusterError::Contended);
        }
        self.held.insert(resource.to_vec(), next);
        Ok(FencingToken {
            resource: resource.to_vec(),
            epoch: next,
            owner: self.me.clone(),
        })
    }

    /// Gives up ownership (hand-off or drain). Returns `true` when the store record was
    /// cleared, `false` when this node did not hold the resource or had already been
    /// superseded. On a store error the resource stays held so the call can be retried.
    pub fn release<S: EpochStore + ?Sized>(
        &mut self,
        store: &mut S,
        resource: &[u8],
    ) -> Result<bool, ClusterError> {
        let Some(epoch) = self.held_epoch(resource) else {
            return Ok(false);
        };
        let expected = EpochRecord {
            epoch,
            owner: Some(self.me.clone()),
        };
        let cleared = EpochRecord { epoch, owner: None };
        let done = store.compare_and_set(resource, Some(&expected), cleared)?;
        self.held.remove(resource);
        Ok(done)
    }

    /// Records that a write with our token was rejected: we no longer own the resource.
    pub fn on_fenced(&mut self, resource: &[u8]) {
        self.held.remove(resource);
    }

    /// Held resources that `placement` assigns to another node (or to nobody): release
    /// these to hand ownership off after a membership change.
    #[must_use]
    pub fn handoff_candidates(&self, placement: &Placement) -> Vec<Vec<u8>> {
        self.held
            .keys()
            .filter(|k| placement.owner(k) != Some(&self.me))
            .cloned()
            .collect()
    }
}

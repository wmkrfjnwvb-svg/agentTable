//! Routing decisions: where an operation on a resource runs, and which nodes need the
//! fan-out of a committed change.

use crate::fencing::EpochRecord;
use crate::node::NodeId;
use crate::placement::Placement;

/// Where an operation on a resource must be executed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// This node executes it.
    Local {
        /// True when this node must first acquire the fencing epoch.
        must_acquire: bool,
    },
    /// Forward to this node.
    Forward {
        /// The node that owns (or should own) the resource.
        node: NodeId,
    },
    /// No node can own the resource right now.
    Unavailable,
}

/// Nodes that must receive a committed change because they hold subscribers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FanOut {
    /// This node has subscribers.
    pub local: bool,
    /// Other active nodes with subscribers (sorted, unique).
    pub remote: Vec<NodeId>,
    /// Subscriber nodes that are not active members any more (sorted, unique); their
    /// subscriptions should be dropped.
    pub gone: Vec<NodeId>,
}

/// A node's routing view: its identity plus the placement derived from its membership view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingTable {
    me: NodeId,
    placement: Placement,
}

impl RoutingTable {
    /// Builds the table. Pass a placement without `me` while draining or after leaving.
    #[must_use]
    pub fn new(me: NodeId, placement: Placement) -> Self {
        Self { me, placement }
    }

    /// The placement used for decisions.
    #[must_use]
    pub fn placement(&self) -> &Placement {
        &self.placement
    }

    /// Decides where an operation on `resource` runs.
    ///
    /// * `record` is the stored epoch record if the caller knows it (it may be stale or
    ///   absent; safety never depends on it).
    /// * `held_epoch` is the epoch this node holds locally ([`crate::Fencer::held_epoch`]).
    ///
    /// The current epoch holder keeps the resource while it is an active member (no
    /// stealing); otherwise the rendezvous owner takes it.
    #[must_use]
    pub fn route(
        &self,
        resource: &[u8],
        record: Option<&EpochRecord>,
        held_epoch: Option<u64>,
    ) -> Route {
        if let Some((epoch, owner)) = record.and_then(|r| r.owner.as_ref().map(|o| (r.epoch, o))) {
            if *owner == self.me {
                if held_epoch == Some(epoch) {
                    return Route::Local {
                        must_acquire: false,
                    };
                }
            } else if self.placement.contains(owner) {
                return Route::Forward {
                    node: owner.clone(),
                };
            }
        }
        match self.placement.owner(resource) {
            None => Route::Unavailable,
            Some(n) if *n == self.me => Route::Local { must_acquire: true },
            Some(n) => Route::Forward { node: n.clone() },
        }
    }

    /// Splits the nodes holding subscribers into local, remote-active and gone.
    #[must_use]
    pub fn fan_out<'a>(&self, subscriber_nodes: impl IntoIterator<Item = &'a NodeId>) -> FanOut {
        let mut f = FanOut::default();
        for n in subscriber_nodes {
            if *n == self.me {
                f.local = true;
            } else if self.placement.contains(n) {
                f.remote.push(n.clone());
            } else {
                f.gone.push(n.clone());
            }
        }
        f.remote.sort();
        f.remote.dedup();
        f.gone.sort();
        f.gone.dedup();
        f
    }
}

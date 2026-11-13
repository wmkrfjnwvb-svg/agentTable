//! Rendezvous (highest-random-weight) placement of resource keys on nodes.
//!
//! The owner of a key is the node with the highest [`rendezvous_score`]. Weights are
//! implemented exactly and without floats: a node of weight `w` competes with `w` replicas
//! and its score is the maximum over them, so it wins a share proportional to `w`. Ties
//! (practically impossible) go to the lower stable id. Only the stable id is hashed, so a
//! restart of a node does not move keys to other nodes.

use crate::codec::MAX_WEIGHT;
use crate::hash::{rendezvous_score, stable_hash64};
use crate::node::NodeId;

/// An immutable set of nodes that can own resources.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Placement {
    nodes: Vec<(NodeId, u16)>,
}

impl Placement {
    /// Builds a placement. Nodes are sorted by stable id; when an id appears more than once
    /// the highest incarnation is kept. Weights are clamped to [`MAX_WEIGHT`].
    #[must_use]
    pub fn new(nodes: impl IntoIterator<Item = (NodeId, u16)>) -> Self {
        let mut nodes: Vec<(NodeId, u16)> = nodes
            .into_iter()
            .map(|(n, w)| (n, w.min(MAX_WEIGHT)))
            .collect();
        nodes.sort();
        nodes.reverse();
        nodes.dedup_by(|later, kept| later.0.id == kept.0.id);
        nodes.reverse();
        Self { nodes }
    }

    /// The nodes with their weights, sorted by stable id.
    #[must_use]
    pub fn nodes(&self) -> &[(NodeId, u16)] {
        &self.nodes
    }

    /// Number of nodes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True when there is no node.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// True when exactly this instance is part of the placement.
    #[must_use]
    pub fn contains(&self, node: &NodeId) -> bool {
        self.nodes.iter().any(|(n, _)| n == node)
    }

    fn score(key_hash: u64, id: u64, weight: u16) -> Option<u64> {
        (0..u32::from(weight))
            .map(|r| rendezvous_score(key_hash, id, r))
            .max()
    }

    /// Nodes able to own `key`, best first. Nodes of weight 0 are excluded.
    #[must_use]
    pub fn ranked(&self, key: &[u8]) -> Vec<&NodeId> {
        let kh = stable_hash64(key);
        let mut scored: Vec<(u64, &NodeId)> = self
            .nodes
            .iter()
            .filter_map(|(n, w)| Self::score(kh, n.id, *w).map(|s| (s, n)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
        scored.into_iter().map(|(_, n)| n).collect()
    }

    /// The owner of `key`, or `None` when no node has a positive weight.
    #[must_use]
    pub fn owner(&self, key: &[u8]) -> Option<&NodeId> {
        let kh = stable_hash64(key);
        let mut best: Option<(u64, &NodeId)> = None;
        for (n, w) in &self.nodes {
            if let Some(s) = Self::score(kh, n.id, *w) {
                // Strictly greater: on a tie the earlier (lower id) node stays.
                if best.is_none_or(|(bs, _)| s > bs) {
                    best = Some((s, n));
                }
            }
        }
        best.map(|(_, n)| n)
    }
}

/// A key whose owner differs between two placements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyMove<'k> {
    /// The key.
    pub key: &'k [u8],
    /// Owner before (if any).
    pub from: Option<NodeId>,
    /// Owner after (if any).
    pub to: Option<NodeId>,
}

/// Which of `keys` change owner when the placement changes from `before` to `after`.
/// A restart (same stable id, new incarnation) counts as a move: the new instance must
/// acquire ownership again.
#[must_use]
pub fn moved_keys<'k>(
    keys: impl IntoIterator<Item = &'k [u8]>,
    before: &Placement,
    after: &Placement,
) -> Vec<KeyMove<'k>> {
    keys.into_iter()
        .filter_map(|key| {
            let from = before.owner(key);
            let to = after.owner(key);
            (from != to).then(|| KeyMove {
                key,
                from: from.cloned(),
                to: to.cloned(),
            })
        })
        .collect()
}

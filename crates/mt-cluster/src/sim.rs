//! Deterministic simulation building blocks (feature `sim`).
//!
//! * [`SimNet`]: a simulated network with delay, loss, duplication, reordering (random
//!   per-packet delays) and partitions. Packets are encoded frames, so the codec is
//!   exercised on every hop.
//! * [`SimCluster`]: a set of [`Membership`] nodes driven by a virtual clock, with crash,
//!   restart and graceful leave.
//!
//! Everything is a function of the seed: the same seed replays the same run.

use crate::codec::{decode, encode};
use crate::membership::{Config, Event, Input, Membership, Output};
use crate::node::NodeId;
use crate::rng::Rng;
use std::collections::BTreeMap;

/// Behaviour of the simulated network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetConfig {
    /// Minimum one-way delay (ms).
    pub min_delay_ms: u64,
    /// Maximum one-way delay (ms). Random delays reorder packets.
    pub max_delay_ms: u64,
    /// Probability of dropping a packet (per mille).
    pub loss_per_mille: u16,
    /// Probability of delivering a packet twice (per mille).
    pub duplicate_per_mille: u16,
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            min_delay_ms: 1,
            max_delay_ms: 20,
            loss_per_mille: 0,
            duplicate_per_mille: 0,
        }
    }
}

/// A packet in flight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    /// Sender identity (what the transport handshake established).
    pub from: NodeId,
    /// Destination address.
    pub to: String,
    /// Encoded frame.
    pub bytes: Vec<u8>,
}

/// Simulated network.
#[derive(Debug, Clone)]
pub struct SimNet {
    /// Current behaviour; may be changed at any time.
    pub cfg: NetConfig,
    queue: BTreeMap<(u64, u64), Packet>,
    seq: u64,
    groups: BTreeMap<String, u32>,
    /// Packets handed to `send`.
    pub sent: u64,
    /// Packets dropped by loss or partition.
    pub dropped: u64,
}

impl SimNet {
    /// Creates a network.
    #[must_use]
    pub fn new(cfg: NetConfig) -> Self {
        Self {
            cfg,
            queue: BTreeMap::new(),
            seq: 0,
            groups: BTreeMap::new(),
            sent: 0,
            dropped: 0,
        }
    }

    /// Puts an address into a partition group. Addresses in different groups cannot
    /// exchange packets. Unassigned addresses are in group 0.
    pub fn set_group(&mut self, addr: &str, group: u32) {
        self.groups.insert(addr.to_string(), group);
    }

    /// Removes all partitions.
    pub fn heal(&mut self) {
        self.groups.clear();
    }

    /// True when the two addresses can currently reach each other.
    #[must_use]
    pub fn connected(&self, a: &str, b: &str) -> bool {
        let g = |x: &str| self.groups.get(x).copied().unwrap_or(0);
        g(a) == g(b)
    }

    fn enqueue(&mut self, at: u64, p: Packet) {
        self.seq = self.seq.wrapping_add(1);
        self.queue.insert((at, self.seq), p);
    }

    /// Sends a packet at time `now_ms`; it may be lost, delayed or duplicated.
    pub fn send(&mut self, now_ms: u64, rng: &mut Rng, packet: Packet) {
        self.sent = self.sent.saturating_add(1);
        if !self.connected(&packet.from.addr, &packet.to) || rng.chance(self.cfg.loss_per_mille) {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        let delay = rng.range(self.cfg.min_delay_ms, self.cfg.max_delay_ms);
        if rng.chance(self.cfg.duplicate_per_mille) {
            let delay2 = rng.range(self.cfg.min_delay_ms, self.cfg.max_delay_ms);
            self.enqueue(now_ms.saturating_add(delay2), packet.clone());
        }
        self.enqueue(now_ms.saturating_add(delay), packet);
    }

    /// Takes the next packet due at or before `now_ms`. Packets whose endpoints are
    /// partitioned at delivery time are dropped.
    pub fn next_due(&mut self, now_ms: u64) -> Option<Packet> {
        loop {
            let key = *self.queue.keys().next().filter(|(at, _)| *at <= now_ms)?;
            let p = self.queue.remove(&key)?;
            if self.connected(&p.from.addr, &p.to) {
                return Some(p);
            }
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    /// Number of packets in flight.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.queue.len()
    }
}

/// One simulated node.
#[derive(Debug, Clone)]
pub struct SimNode {
    /// Its membership state machine.
    pub membership: Membership,
    /// False while crashed.
    pub up: bool,
    /// Every event it emitted, with the time.
    pub events: Vec<(u64, Event)>,
}

/// A simulated cluster of membership nodes. Node `i` has stable id `i + 1` and address `n{i}`.
#[derive(Debug, Clone)]
pub struct SimCluster {
    /// Virtual clock (ms).
    pub now_ms: u64,
    /// The run's random source.
    pub rng: Rng,
    /// The network.
    pub net: SimNet,
    /// The nodes.
    pub nodes: Vec<SimNode>,
    cfg: Config,
    /// Clock advance per [`SimCluster::step`] (ms).
    pub step_ms: u64,
}

impl SimCluster {
    /// Address of node `i`.
    #[must_use]
    pub fn addr(i: usize) -> String {
        format!("n{i}")
    }

    /// The seed addresses every node joins through (the first two nodes).
    #[must_use]
    pub fn seeds() -> Vec<String> {
        vec![Self::addr(0), Self::addr(1)]
    }

    /// Creates `n` nodes at time 1000 and makes them join through the seeds.
    #[must_use]
    pub fn new(seed: u64, n: usize, cfg: Config, net: NetConfig) -> Self {
        let mut c = Self {
            now_ms: 1_000,
            rng: Rng::new(seed),
            net: SimNet::new(net),
            nodes: Vec::new(),
            cfg,
            step_ms: 50,
        };
        for i in 0..n {
            let node = c.spawn(i, c.now_ms);
            c.nodes.push(node);
            c.input(
                i,
                Input::Join {
                    seeds: Self::seeds(),
                },
            );
        }
        c
    }

    fn spawn(&mut self, i: usize, incarnation: u64) -> SimNode {
        let id = u64::try_from(i).unwrap_or(u64::MAX).saturating_add(1);
        let me = NodeId::new(id, incarnation, Self::addr(i));
        SimNode {
            membership: Membership::new(self.cfg.clone(), me, self.rng.next_u64()),
            up: true,
            events: Vec::new(),
        }
    }

    /// Feeds an input to node `i` (ignored while it is down) and routes its outputs.
    pub fn input(&mut self, i: usize, input: Input) {
        let now = self.now_ms;
        let Some(node) = self.nodes.get_mut(i).filter(|n| n.up) else {
            return;
        };
        let outputs = node.membership.handle(now, input);
        let from = node.membership.me().clone();
        for o in outputs {
            match o {
                Output::Send { to, msg } => {
                    if let Ok(bytes) = encode(&msg) {
                        let from = from.clone();
                        self.net
                            .send(now, &mut self.rng, Packet { from, to, bytes });
                    }
                }
                Output::Event(e) => node.events.push((now, e)),
            }
        }
    }

    /// Advances the clock by `step_ms`, delivers due packets and ticks every node in a
    /// random order.
    pub fn step(&mut self) {
        self.now_ms = self.now_ms.saturating_add(self.step_ms);
        while let Some(p) = self.net.next_due(self.now_ms) {
            let target = self
                .nodes
                .iter()
                .position(|n| n.membership.me().addr == p.to);
            if let (Some(i), Ok(msg)) = (target, decode(&p.bytes)) {
                self.input(i, Input::Message { from: p.from, msg });
            }
        }
        let mut order: Vec<usize> = (0..self.nodes.len()).collect();
        self.rng.shuffle(&mut order);
        for i in order {
            self.input(i, Input::Tick);
        }
    }

    /// Runs [`SimCluster::step`] until `duration_ms` of virtual time has passed.
    pub fn run_for(&mut self, duration_ms: u64) {
        let end = self.now_ms.saturating_add(duration_ms);
        while self.now_ms < end {
            self.step();
        }
    }

    /// Crashes node `i`: it stops processing and loses all state.
    pub fn crash(&mut self, i: usize) {
        if let Some(n) = self.nodes.get_mut(i) {
            n.up = false;
        }
    }

    /// Restarts node `i` as a fresh instance (new incarnation, empty state) and joins.
    pub fn restart(&mut self, i: usize) {
        let Some(old) = self.nodes.get(i) else {
            return;
        };
        let incarnation = old
            .membership
            .me()
            .incarnation
            .saturating_add(1)
            .max(self.now_ms);
        let events = old.events.clone();
        let mut node = self.spawn(i, incarnation);
        node.events = events;
        if let Some(slot) = self.nodes.get_mut(i) {
            *slot = node;
        }
        self.input(
            i,
            Input::Join {
                seeds: Self::seeds(),
            },
        );
    }

    /// Makes node `i` leave gracefully; afterwards it counts as down.
    pub fn leave(&mut self, i: usize) {
        self.input(i, Input::Leave);
        self.crash(i);
    }

    /// Splits the nodes into partition groups: `group_of(i)` is the group of node `i`.
    pub fn partition(&mut self, group_of: impl Fn(usize) -> u32) {
        for i in 0..self.nodes.len() {
            self.net.set_group(&Self::addr(i), group_of(i));
        }
    }

    /// Identities of the nodes that are up, sorted.
    #[must_use]
    pub fn up_nodes(&self) -> Vec<NodeId> {
        let mut v: Vec<NodeId> = self
            .nodes
            .iter()
            .filter(|n| n.up)
            .map(|n| n.membership.me().clone())
            .collect();
        v.sort();
        v
    }

    /// True when every up node's set of active members is exactly the set of up nodes.
    #[must_use]
    pub fn converged(&self) -> bool {
        let truth = self.up_nodes();
        self.nodes.iter().filter(|n| n.up).all(|n| {
            let view: Vec<NodeId> = n
                .membership
                .active_nodes()
                .into_iter()
                .map(|(id, _)| id)
                .collect();
            view == truth
        })
    }
}

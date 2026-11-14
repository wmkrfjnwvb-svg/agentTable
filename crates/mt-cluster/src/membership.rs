//! SWIM-style membership: failure detection and dissemination as a sans-IO state machine.
//!
//! Drive it with [`Membership::handle`]: feed `Tick`s (as often as you like; all timers are
//! deadlines computed from `now_ms`), incoming messages, `Join` and `Leave`; send the returned
//! messages and react to the returned events.
//!
//! Protocol summary (see README for details and weaknesses):
//! * every `probe_interval_ms` one member (round-robin over a shuffled order) gets a `Ping`;
//!   without an `Ack` after `probe_timeout_ms`, `indirect_probes` other members are asked to
//!   probe it (`PingReq`); without any `Ack` by the end of the period the member is `Suspect`;
//! * a suspect that does not refute within `suspect_timeout_ms` is `Dead`;
//! * a node refutes suspicion by raising its `version`; a node that learns it was declared
//!   dead *renews* (raises its incarnation) and thereby rejoins as a new instance;
//! * facts are merged with a total order (see `Update::precedence`), so views converge;
//! * facts are piggybacked on probes with bounded retransmission, and a periodic full-state
//!   `Sync` (anti-entropy, also sent to dead / left members and seeds) repairs whatever gossip lost
//!   and heals partitions.

use crate::codec::{MAX_ADDR_BYTES, MAX_UPDATES, MAX_WEIGHT};
use crate::message::{MemberState, Message, Update};
use crate::node::NodeId;
use crate::placement::Placement;
use crate::rng::Rng;
use std::collections::BTreeMap;

/// Maximum number of seed addresses kept.
pub const MAX_SEEDS: usize = 64;
/// Maximum number of indirect probes relayed at once.
pub const MAX_FORWARDS: usize = 1024;

/// Tunables of the membership protocol. All durations are milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Protocol period: one probe per period.
    pub probe_interval_ms: u64,
    /// How long to wait for a direct `Ack` before asking for indirect probes.
    pub probe_timeout_ms: u64,
    /// Number of members asked to probe indirectly (SWIM's `k`).
    pub indirect_probes: usize,
    /// How long a suspect may stay silent before it is declared dead.
    pub suspect_timeout_ms: u64,
    /// Interval of full-state anti-entropy exchanges.
    pub sync_interval_ms: u64,
    /// Each fact is piggybacked `retransmit_mult * bit_length(member_count)` times.
    pub retransmit_mult: u32,
    /// Maximum number of queued facts piggybacked on one message.
    pub max_piggyback: usize,
    /// How long dead / left members are remembered (and dead ones re-contacted).
    pub dead_retention_ms: u64,
    /// Placement weight this node advertises (clamped to [`MAX_WEIGHT`]).
    pub weight: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            probe_interval_ms: 1_000,
            probe_timeout_ms: 300,
            indirect_probes: 3,
            suspect_timeout_ms: 5_000,
            sync_interval_ms: 10_000,
            retransmit_mult: 4,
            max_piggyback: 16,
            dead_retention_ms: 24 * 60 * 60 * 1_000,
            weight: 1,
        }
    }
}

/// What this node knows about another node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// Identity of the instance.
    pub node: NodeId,
    /// Its refutation counter.
    pub version: u64,
    /// Its state in this node's view.
    pub state: MemberState,
    /// Its placement weight.
    pub weight: u16,
    /// When this node last changed the record (ms).
    pub since_ms: u64,
}

impl Member {
    fn update(&self) -> Update {
        Update {
            node: self.node.clone(),
            version: self.version,
            state: self.state,
            weight: self.weight,
        }
    }
}

/// Input of the state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// Time passed; run timers.
    Tick,
    /// A message arrived from an authenticated peer.
    Message {
        /// The sender, as established by the transport handshake.
        from: NodeId,
        /// The message.
        msg: Message,
    },
    /// Join the cluster through these seed addresses (also remembered for later recovery).
    Join {
        /// Addresses of seed nodes.
        seeds: Vec<String>,
    },
    /// Leave the cluster gracefully. Afterwards the state machine is inert.
    Leave,
}

/// Output of the state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// Send `msg` to the node at address `to`.
    Send {
        /// Destination address.
        to: String,
        /// The message.
        msg: Message,
    },
    /// The local view changed.
    Event(Event),
}

/// Change of the local membership view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The node is (again) believed alive: newly seen, a new incarnation, or suspicion refuted.
    MemberUp(NodeId),
    /// The node is suspected of having failed (it still counts as a member).
    Suspect(NodeId),
    /// The node instance was declared dead, or was replaced by a newer incarnation.
    Down(NodeId),
    /// The node instance left gracefully (also emitted for this node on `Leave`).
    Left(NodeId),
    /// This node learned it had been declared dead and took a new incarnation. Everything
    /// tied to the old identity (epochs, leases) must be dropped.
    Renewed(NodeId),
}

#[derive(Debug, Clone)]
struct Queued {
    update: Update,
    remaining: u32,
}

#[derive(Debug, Clone)]
struct Probe {
    /// The exact instance that was pinged.
    target: NodeId,
    seq: u64,
    sent_at: u64,
    indirect_sent: bool,
}

#[derive(Debug, Clone)]
struct Forward {
    requester: String,
    seq: u64,
    expires_at: u64,
}

/// The membership state machine of one node.
#[derive(Debug, Clone)]
pub struct Membership {
    cfg: Config,
    me: NodeId,
    version: u64,
    left: bool,
    members: BTreeMap<u64, Member>,
    queue: Vec<Queued>,
    probe: Option<Probe>,
    probe_order: Vec<u64>,
    next_probe_at: u64,
    next_sync_at: u64,
    next_seq: u64,
    forwards: BTreeMap<u64, Forward>,
    seeds: Vec<String>,
    rng: Rng,
}

impl Membership {
    /// Creates the state machine for node `me`. `seed` drives every random choice.
    #[must_use]
    pub fn new(cfg: Config, me: NodeId, seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let next_seq = rng.next_u64();
        let mut cfg = cfg;
        cfg.weight = cfg.weight.min(MAX_WEIGHT);
        Self {
            cfg,
            me,
            version: 0,
            left: false,
            members: BTreeMap::new(),
            queue: Vec::new(),
            probe: None,
            probe_order: Vec::new(),
            next_probe_at: 0,
            next_sync_at: 0,
            next_seq,
            forwards: BTreeMap::new(),
            seeds: Vec::new(),
            rng,
        }
    }

    /// This node's current identity (the incarnation changes on renew).
    #[must_use]
    pub fn me(&self) -> &NodeId {
        &self.me
    }

    /// True after `Leave`.
    #[must_use]
    pub fn has_left(&self) -> bool {
        self.left
    }

    /// All other known nodes, including dead and left ones still remembered.
    pub fn members(&self) -> impl Iterator<Item = &Member> {
        self.members.values()
    }

    /// The record for a stable node id (not this node).
    #[must_use]
    pub fn member(&self, id: u64) -> Option<&Member> {
        self.members.get(&id)
    }

    /// Active (alive or suspect) instances with their weights, including this node unless
    /// it has left. Sorted by stable id.
    #[must_use]
    pub fn active_nodes(&self) -> Vec<(NodeId, u16)> {
        let mut v: Vec<(NodeId, u16)> = self
            .members
            .values()
            .filter(|m| m.state.is_active())
            .map(|m| (m.node.clone(), m.weight))
            .collect();
        if !self.left {
            v.push((self.me.clone(), self.cfg.weight));
        }
        v.sort();
        v
    }

    /// True when the active nodes (including this one) are a strict majority of all nodes
    /// this view remembers, not counting those that left gracefully.
    ///
    /// Optional guard against ownership ping-pong: a node on the minority side of a
    /// partition that can still reach the store should not acquire epochs. It is a
    /// heuristic, not a safety mechanism (fencing is), and it costs availability while
    /// more than half of the remembered nodes are dead (until `dead_retention_ms` passes).
    #[must_use]
    pub fn has_majority(&self) -> bool {
        let active = self.active_nodes().len();
        let dead = self
            .members
            .values()
            .filter(|m| m.state == MemberState::Dead)
            .count();
        !self.left && active > dead
    }

    /// Placement over the active nodes of this view.
    #[must_use]
    pub fn placement(&self) -> Placement {
        Placement::new(self.active_nodes())
    }

    /// Advances the state machine. `now_ms` must not go backwards.
    pub fn handle(&mut self, now_ms: u64, input: Input) -> Vec<Output> {
        let mut out = Vec::new();
        if self.left {
            return out;
        }
        match input {
            Input::Tick => self.tick(now_ms, &mut out),
            Input::Message { from, msg } => self.on_message(now_ms, &from, msg, &mut out),
            Input::Join { seeds } => self.join(seeds, &mut out),
            Input::Leave => self.leave(&mut out),
        }
        out
    }

    fn self_update(&self) -> Update {
        Update {
            node: self.me.clone(),
            version: self.version,
            state: MemberState::Alive,
            weight: self.cfg.weight,
        }
    }

    fn retransmit_limit(&self) -> u32 {
        let n = u64::try_from(self.members.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let bits = u64::BITS.saturating_sub(n.leading_zeros());
        self.cfg.retransmit_mult.saturating_mul(bits).max(1)
    }

    fn enqueue(&mut self, update: Update) {
        let id = update.node.id;
        self.queue.retain(|q| q.update.node.id != id);
        let remaining = self.retransmit_limit();
        self.queue.push(Queued { update, remaining });
    }

    /// Facts to piggyback on a message to `to`: our own state, what we think of the
    /// recipient if that is not "alive" (so it can refute or renew), and queued facts.
    fn gossip_for(&mut self, to: Option<u64>) -> Vec<Update> {
        let mut v = vec![self.self_update()];
        if let Some(m) = to.and_then(|id| self.members.get(&id)) {
            if m.state != MemberState::Alive {
                v.push(m.update());
            }
        }
        self.queue.sort_by_key(|q| core::cmp::Reverse(q.remaining));
        for q in self.queue.iter_mut().take(self.cfg.max_piggyback) {
            v.push(q.update.clone());
            q.remaining = q.remaining.saturating_sub(1);
        }
        self.queue.retain(|q| q.remaining > 0);
        v
    }

    /// Our whole view (bounded by the codec limit; a random window if it is larger).
    fn full_state(&mut self) -> Vec<Update> {
        let mut v = vec![self.self_update()];
        let room = MAX_UPDATES.saturating_sub(1);
        let skip = if self.members.len() > room {
            self.rng.index(self.members.len())
        } else {
            0
        };
        v.extend(
            self.members
                .values()
                .cycle()
                .skip(skip)
                .take(room.min(self.members.len()))
                .map(Member::update),
        );
        v
    }

    fn next_seq(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        seq
    }

    fn active_ids(&self, except: Option<u64>) -> Vec<u64> {
        self.members
            .values()
            .filter(|m| m.state.is_active() && Some(m.node.id) != except)
            .map(|m| m.node.id)
            .collect()
    }

    fn addr_of(&self, id: u64) -> Option<String> {
        self.members.get(&id).map(|m| m.node.addr.clone())
    }

    fn join(&mut self, seeds: Vec<String>, out: &mut Vec<Output>) {
        self.seeds = seeds
            .into_iter()
            .filter(|s| !s.is_empty() && s.len() <= MAX_ADDR_BYTES && *s != self.me.addr)
            .take(MAX_SEEDS)
            .collect();
        self.sync_seeds(out);
    }

    fn sync_seeds(&mut self, out: &mut Vec<Output>) {
        for to in self.seeds.clone() {
            let updates = self.full_state();
            out.push(Output::Send {
                to,
                msg: Message::Sync {
                    want_reply: true,
                    updates,
                },
            });
        }
    }

    fn leave(&mut self, out: &mut Vec<Output>) {
        self.left = true;
        let update = Update {
            node: self.me.clone(),
            version: self.version,
            state: MemberState::Left,
            weight: self.cfg.weight,
        };
        let mut ids = self.active_ids(None);
        self.rng.shuffle(&mut ids);
        let fanout = self.cfg.indirect_probes.max(3);
        let addrs: Vec<String> = ids
            .into_iter()
            .take(fanout)
            .filter_map(|i| self.addr_of(i))
            .collect();
        for to in addrs {
            out.push(Output::Send {
                to,
                msg: Message::Sync {
                    want_reply: false,
                    updates: vec![update.clone()],
                },
            });
        }
        out.push(Output::Event(Event::Left(self.me.clone())));
    }

    fn tick(&mut self, now: u64, out: &mut Vec<Output>) {
        // Suspects whose time is up become dead.
        let expired: Vec<Update> = self
            .members
            .values()
            .filter(|m| {
                m.state == MemberState::Suspect
                    && now.saturating_sub(m.since_ms) >= self.cfg.suspect_timeout_ms
            })
            .map(|m| Update {
                state: MemberState::Dead,
                ..m.update()
            })
            .collect();
        for u in expired {
            self.apply(now, u, out);
        }
        // Forget old tombstones and stale relays.
        let retention = self.cfg.dead_retention_ms;
        self.members
            .retain(|_, m| m.state.is_active() || now.saturating_sub(m.since_ms) < retention);
        self.forwards.retain(|_, f| now < f.expires_at);

        // Failure detector.
        if let Some(p) = self.probe.clone() {
            let elapsed = now.saturating_sub(p.sent_at);
            if elapsed >= self.cfg.probe_interval_ms {
                self.probe = None;
                self.suspect(now, &p.target, out);
            } else if elapsed >= self.cfg.probe_timeout_ms && !p.indirect_sent {
                self.indirect_probe(&p, out);
            }
        }
        if self.probe.is_none() && now >= self.next_probe_at {
            self.start_probe(now, out);
        }

        // Anti-entropy.
        if now >= self.next_sync_at {
            self.next_sync_at = now.saturating_add(self.cfg.sync_interval_ms);
            self.sync(out);
        }
    }

    /// Marks the probed instance as suspect if it is still the alive member of that id.
    fn suspect(&mut self, now: u64, target: &NodeId, out: &mut Vec<Output>) {
        let suspicion = self
            .members
            .get(&target.id)
            .filter(|m| m.node == *target && m.state == MemberState::Alive)
            .map(|m| Update {
                state: MemberState::Suspect,
                ..m.update()
            });
        if let Some(u) = suspicion {
            self.apply(now, u, out);
        }
    }

    fn indirect_probe(&mut self, p: &Probe, out: &mut Vec<Output>) {
        if let Some(pr) = self.probe.as_mut() {
            pr.indirect_sent = true;
        }
        let target = p.target.clone();
        let mut helpers = self.active_ids(Some(target.id));
        self.rng.shuffle(&mut helpers);
        for id in helpers.into_iter().take(self.cfg.indirect_probes) {
            if let Some(to) = self.addr_of(id) {
                let updates = self.gossip_for(Some(id));
                out.push(Output::Send {
                    to,
                    msg: Message::PingReq {
                        seq: p.seq,
                        target: target.clone(),
                        updates,
                    },
                });
            }
        }
    }

    fn start_probe(&mut self, now: u64, out: &mut Vec<Output>) {
        self.next_probe_at = now.saturating_add(self.cfg.probe_interval_ms);
        if self.probe_order.is_empty() {
            self.probe_order = self.active_ids(None);
            self.rng.shuffle(&mut self.probe_order);
        }
        while let Some(id) = self.probe_order.pop() {
            let Some(target) = self
                .members
                .get(&id)
                .filter(|m| m.state.is_active())
                .map(|m| m.node.clone())
            else {
                continue;
            };
            let seq = self.next_seq();
            let updates = self.gossip_for(Some(id));
            out.push(Output::Send {
                to: target.addr.clone(),
                msg: Message::Ping { seq, updates },
            });
            self.probe = Some(Probe {
                target,
                seq,
                sent_at: now,
                indirect_sent: false,
            });
            return;
        }
    }

    fn sync(&mut self, out: &mut Vec<Output>) {
        let active = self.active_ids(None);
        let dead: Vec<u64> = self
            .members
            .values()
            .filter(|m| !m.state.is_active())
            .map(|m| m.node.id)
            .collect();
        let mut targets = Vec::new();
        if let Some(id) = active.get(self.rng.index(active.len())) {
            targets.push(*id);
        }
        if let Some(id) = dead.get(self.rng.index(dead.len())) {
            targets.push(*id);
        }
        let addrs: Vec<String> = targets
            .into_iter()
            .filter_map(|id| self.addr_of(id))
            .collect();
        for to in addrs {
            let updates = self.full_state();
            out.push(Output::Send {
                to,
                msg: Message::Sync {
                    want_reply: true,
                    updates,
                },
            });
        }
        if active.is_empty() {
            self.sync_seeds(out);
        }
    }

    fn on_message(&mut self, now: u64, from: &NodeId, msg: Message, out: &mut Vec<Output>) {
        if *from == self.me {
            return;
        }
        match msg {
            Message::Hello(_) | Message::HelloAck(_) | Message::HelloConfirm { tag: _ } => {}
            Message::Ping { seq, updates } => {
                self.apply_all(now, updates, out);
                let updates = self.gossip_for(Some(from.id));
                out.push(Output::Send {
                    to: from.addr.clone(),
                    msg: Message::Ack { seq, updates },
                });
            }
            Message::Ack { seq, updates } => {
                self.apply_all(now, updates, out);
                if self.probe.as_ref().is_some_and(|p| p.seq == seq) {
                    self.probe = None;
                } else if let Some(f) = self.forwards.remove(&seq) {
                    let updates = self.gossip_for(None);
                    out.push(Output::Send {
                        to: f.requester,
                        msg: Message::Ack {
                            seq: f.seq,
                            updates,
                        },
                    });
                }
            }
            Message::PingReq {
                seq,
                target,
                updates,
            } => {
                self.apply_all(now, updates, out);
                if self.forwards.len() < MAX_FORWARDS {
                    let fseq = self.next_seq();
                    self.forwards.insert(
                        fseq,
                        Forward {
                            requester: from.addr.clone(),
                            seq,
                            expires_at: now.saturating_add(self.cfg.probe_interval_ms),
                        },
                    );
                    let updates = self.gossip_for(Some(target.id));
                    out.push(Output::Send {
                        to: target.addr,
                        msg: Message::Ping { seq: fseq, updates },
                    });
                }
            }
            Message::Sync {
                want_reply,
                updates,
            } => {
                self.apply_all(now, updates, out);
                if want_reply {
                    let updates = self.full_state();
                    out.push(Output::Send {
                        to: from.addr.clone(),
                        msg: Message::Sync {
                            want_reply: false,
                            updates,
                        },
                    });
                }
            }
        }
    }

    fn apply_all(&mut self, now: u64, updates: Vec<Update>, out: &mut Vec<Output>) {
        for u in updates {
            self.apply(now, u, out);
        }
    }

    fn renew(&mut self, incarnation: u64, out: &mut Vec<Output>) {
        self.me.incarnation = incarnation;
        self.version = 0;
        self.enqueue(self.self_update());
        out.push(Output::Event(Event::Renewed(self.me.clone())));
    }

    fn apply_self(&mut self, u: &Update, out: &mut Vec<Output>) {
        if u.node.incarnation < self.me.incarnation {
            return;
        }
        if u.node.incarnation > self.me.incarnation {
            // Someone knows a newer instance of us than we are: jump past it.
            self.renew(u.node.incarnation.saturating_add(1), out);
            return;
        }
        match u.state {
            MemberState::Alive => self.version = self.version.max(u.version),
            MemberState::Suspect => {
                if u.version >= self.version {
                    self.version = u.version.saturating_add(1);
                    self.enqueue(self.self_update());
                }
            }
            MemberState::Dead | MemberState::Left => {
                self.renew(self.me.incarnation.saturating_add(1), out);
            }
        }
    }

    /// Merges one fact into the view.
    fn apply(&mut self, now: u64, u: Update, out: &mut Vec<Output>) {
        if u.node.id == self.me.id {
            self.apply_self(&u, out);
            return;
        }
        let old = self.members.get(&u.node.id);
        if old.is_some_and(|m| m.update().precedence() >= u.precedence()) {
            return;
        }
        // State of the same instance before this fact, if we knew it.
        let same = old.filter(|m| m.node == u.node).map(|m| m.state);
        let was_active = same.is_some_and(MemberState::is_active);
        if let Some(m) = old.filter(|m| m.node != u.node && m.state.is_active()) {
            out.push(Output::Event(Event::Down(m.node.clone())));
        }
        match u.state {
            MemberState::Alive => {
                if same != Some(MemberState::Alive) {
                    out.push(Output::Event(Event::MemberUp(u.node.clone())));
                }
            }
            MemberState::Suspect => {
                if same.is_none() {
                    out.push(Output::Event(Event::MemberUp(u.node.clone())));
                }
                if same != Some(MemberState::Suspect) {
                    out.push(Output::Event(Event::Suspect(u.node.clone())));
                }
            }
            MemberState::Dead => {
                if was_active {
                    out.push(Output::Event(Event::Down(u.node.clone())));
                }
            }
            MemberState::Left => {
                if was_active {
                    out.push(Output::Event(Event::Left(u.node.clone())));
                }
            }
        }
        // A probe is about one instance: drop it when that instance is gone or was replaced
        // (the replacement was never pinged, so its silence would prove nothing).
        let probe_is_moot = self.probe.as_ref().is_some_and(|p| {
            p.target.id == u.node.id && (!u.state.is_active() || p.target != u.node)
        });
        if probe_is_moot {
            self.probe = None;
        }
        self.members.insert(
            u.node.id,
            Member {
                node: u.node.clone(),
                version: u.version,
                state: u.state,
                weight: u.weight.min(MAX_WEIGHT),
                since_ms: now,
            },
        );
        self.enqueue(u);
    }
}

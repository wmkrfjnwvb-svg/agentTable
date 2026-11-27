//! The simulated world and its event loop.

use crate::db::{CommitError, Db};
use crate::{Config, Report, Violation};
use mt_cluster::rng::Rng;
use mt_cluster::{Fencer, NodeId, Placement};
use mt_core::id::{BaseId, FieldId, RecordId, TableId};
use mt_core::value::Value;
use mt_ops::{BaseState, Batch, ClientId, ClientOpId, Op, OpsError, Outcome, Replica};
use std::collections::{BTreeMap, BTreeSet};

/// Who a message is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum To {
    Node(usize),
    Client(usize),
}

/// What travels.
#[derive(Debug, Clone)]
enum Body {
    /// A client's batch (forwarded at most a few times).
    Submit {
        batch: Batch,
        client: usize,
        hops: u8,
    },
    /// A decision about a base, for a client.
    Outcome(BaseId, Outcome),
}

#[derive(Debug, Clone)]
enum Event {
    Deliver {
        to: To,
        from: Option<usize>,
        body: Body,
    },
    ClientTick(usize),
    Crash(usize),
    Restart(usize),
    Partition(u64),
    Heal,
    View(usize),
    WorkerTick(usize),
    Reconnect(usize),
    JobDone {
        node: usize,
        incarnation: u64,
        job: String,
        token: u64,
    },
}

struct Node {
    id: NodeId,
    up: bool,
    view: BTreeSet<usize>,
    fencer: Fencer,
    actors: BTreeMap<BaseId, (u64, BaseState)>,
}

struct Client {
    replicas: BTreeMap<BaseId, Replica>,
    sent: BTreeMap<ClientOpId, u64>,
    made: u128,
    told_refused: BTreeSet<(BaseId, ClientOpId)>,
}

/// The world: nodes, clients, the database, the network and the clock.
pub struct World {
    config: Config,
    seed: u64,
    rng: Rng,
    now: u64,
    order: u64,
    events: BTreeMap<(u64, u64), Event>,
    nodes: Vec<Node>,
    clients: Vec<Client>,
    group: Vec<u64>,
    db: Db,
    bases: Vec<BaseId>,
    report: Report,
    stopped: bool,
}

const TABLE: TableId = TableId::from_u128(1);

fn field(n: u64) -> FieldId {
    FieldId::from_u128(u128::from(n).saturating_add(1))
}

impl World {
    /// Builds the world for a seed.
    #[must_use]
    pub fn new(seed: u64, config: Config) -> Self {
        let bases: Vec<BaseId> = (1..=config.bases.max(1))
            .map(|n| BaseId::from_u128(n as u128))
            .collect();
        let all: BTreeSet<usize> = (0..config.nodes.max(1)).collect();
        let nodes = (0..config.nodes.max(1))
            .map(|i| {
                let id = NodeId::new(i as u64, 1, format!("node-{i}"));
                Node {
                    fencer: Fencer::new(id.clone()),
                    id,
                    up: true,
                    view: all.clone(),
                    actors: BTreeMap::new(),
                }
            })
            .collect();
        let clients = (0..config.clients.max(1))
            .map(|c| Client {
                replicas: bases
                    .iter()
                    .map(|b| {
                        (
                            *b,
                            Replica::new(
                                ClientId((c as u64).saturating_add(1)),
                                0,
                                BaseState::new(*b),
                            ),
                        )
                    })
                    .collect(),
                sent: BTreeMap::new(),
                made: 0,
                told_refused: BTreeSet::new(),
            })
            .collect();
        World {
            rng: Rng::new(seed),
            seed,
            now: 0,
            order: 0,
            events: BTreeMap::new(),
            nodes,
            clients,
            group: vec![0; config.nodes.max(1)],
            db: Db::new(config.check_fences),
            bases,
            report: Report {
                seed,
                ..Report::default()
            },
            stopped: false,
            config,
        }
    }

    fn at(&mut self, delay: u64, event: Event) {
        self.order = self.order.saturating_add(1);
        self.events
            .insert((self.now.saturating_add(delay), self.order), event);
    }

    fn violation(&self, what: impl Into<String>) -> Violation {
        Violation {
            seed: self.seed,
            at_ms: self.now,
            what: what.into(),
        }
    }

    /// Sends over the network: delayed, maybe lost or duplicated; nodes in different
    /// partitions cannot reach each other (clients reach every node).
    fn send(&mut self, from: Option<usize>, to: To, body: Body) {
        if let (Some(a), To::Node(b)) = (from, to) {
            if self.group.get(a) != self.group.get(b) {
                return;
            }
        }
        if self.rng.chance(self.config.loss_per_mille) {
            self.report.lost = self.report.lost.saturating_add(1);
            // A lost message to a client is a dropped connection: it reconnects and catches up.
            if let To::Client(c) = to {
                self.at(self.config.resend_ms, Event::Reconnect(c));
            }
            return;
        }
        let copies = if self.rng.chance(self.config.duplicate_per_mille) {
            2
        } else {
            1
        };
        for _ in 0..copies {
            let delay = self.rng.range(
                self.config.min_delay_ms,
                self.config.max_delay_ms.max(self.config.min_delay_ms),
            );
            self.at(
                delay,
                Event::Deliver {
                    to,
                    from,
                    body: body.clone(),
                },
            );
        }
    }

    fn owner_in_view(&self, node: usize, base: BaseId) -> Option<usize> {
        let view = &self.nodes.get(node)?.view;
        let placement = Placement::new(view.iter().filter_map(|i| {
            self.nodes
                .get(*i)
                .map(|n| (NodeId::new(*i as u64, 1, n.id.addr.clone()), 1))
        }));
        placement
            .owner(&Db::resource(base))
            .and_then(|owner| usize::try_from(owner.id).ok())
    }

    /// Every node's view of who is up lags behind the truth.
    fn views_change(&mut self) {
        for i in 0..self.nodes.len() {
            let delay = self.rng.range(
                self.config.detect_ms / 2,
                self.config.detect_ms.saturating_mul(2),
            );
            self.at(delay, Event::View(i));
        }
    }

    /// Runs the world and checks every invariant.
    ///
    /// # Errors
    /// The first [`Violation`].
    pub fn run(mut self) -> Result<Report, Violation> {
        for c in 0..self.clients.len() {
            let delay = self.rng.below(self.config.edit_every_ms.max(1));
            self.at(delay, Event::ClientTick(c));
        }
        for n in 0..self.nodes.len() {
            self.at(self.config.worker_every_ms, Event::WorkerTick(n));
        }
        if self.config.faults {
            self.schedule_faults();
        }
        let limit = self
            .config
            .duration_ms
            .saturating_add(self.config.settle_ms);
        while let Some(((time, order), event)) = self.events.pop_first() {
            if time > limit {
                self.events.insert((time, order), event);
                break;
            }
            self.now = time;
            if !self.stopped && self.now >= self.config.duration_ms {
                self.stop();
            }
            self.handle(event)?;
            if self.stopped && self.quiet() {
                break;
            }
        }
        self.check()?;
        Ok(self.report)
    }

    fn schedule_faults(&mut self) {
        let mut t = 0;
        while t < self.config.duration_ms {
            t = t.saturating_add(self.rng.range(
                self.config.fault_every_ms / 2,
                self.config.fault_every_ms.saturating_mul(3) / 2,
            ));
            if t >= self.config.duration_ms {
                break;
            }
            let delay = t.saturating_sub(self.now);
            match self.rng.below(3) {
                0 => {
                    let n = self.rng.index(self.nodes.len());
                    self.at(delay, Event::Crash(n));
                    let back = self.rng.range(50, 600);
                    self.at(delay.saturating_add(back), Event::Restart(n));
                }
                1 => {
                    let split = self.rng.next_u64();
                    self.at(delay, Event::Partition(split));
                    let back = self.rng.range(100, 800);
                    self.at(delay.saturating_add(back), Event::Heal);
                }
                _ => {
                    // A quiet period.
                }
            }
        }
    }

    /// The end of the run: no more edits or faults; every node up and connected.
    fn stop(&mut self) {
        self.stopped = true;
        self.group.iter_mut().for_each(|g| *g = 0);
        for n in 0..self.nodes.len() {
            if self.nodes.get(n).is_some_and(|node| !node.up) {
                self.restart(n);
            }
        }
        self.views_change();
    }

    fn quiet(&self) -> bool {
        let messages = self
            .events
            .values()
            .any(|e| matches!(e, Event::Deliver { .. } | Event::Reconnect(_)));
        let pending = self
            .clients
            .iter()
            .any(|c| c.replicas.values().any(|r| r.pending().next().is_some()));
        let jobs = self.db.jobs.values().any(|j| !j.done);
        !messages && !pending && !jobs
    }

    fn handle(&mut self, event: Event) -> Result<(), Violation> {
        match event {
            Event::Deliver { to, from, body } => self.deliver(to, from, body),
            Event::ClientTick(c) => {
                self.client_tick(c)?;
                Ok(())
            }
            Event::Crash(n) => {
                if let Some(node) = self.nodes.get_mut(n) {
                    if node.up {
                        node.up = false;
                        node.actors.clear();
                        self.report.crashes = self.report.crashes.saturating_add(1);
                    }
                }
                self.views_change();
                Ok(())
            }
            Event::Restart(n) => {
                self.restart(n);
                self.views_change();
                Ok(())
            }
            Event::Partition(split) => {
                let count = self.nodes.len() as u64;
                for (i, g) in self.group.iter_mut().enumerate() {
                    *g = (split >> (i % 64)) & 1;
                }
                if count > 1 {
                    self.report.partitions = self.report.partitions.saturating_add(1);
                }
                self.views_change();
                Ok(())
            }
            Event::Heal => {
                self.group.iter_mut().for_each(|g| *g = 0);
                self.views_change();
                Ok(())
            }
            Event::View(n) => {
                let truth: BTreeSet<usize> = (0..self.nodes.len())
                    .filter(|j| {
                        self.nodes.get(*j).is_some_and(|node| node.up)
                            && self.group.get(*j) == self.group.get(n)
                    })
                    .collect();
                if let Some(node) = self.nodes.get_mut(n) {
                    node.view = truth;
                }
                self.count_split_brain();
                Ok(())
            }
            Event::WorkerTick(n) => {
                self.worker_tick(n);
                Ok(())
            }
            Event::Reconnect(c) => self.catch_up(c),
            Event::JobDone {
                node,
                incarnation,
                job,
                token,
            } => {
                let alive = self
                    .nodes
                    .get(node)
                    .is_some_and(|n| n.up && n.id.incarnation == incarnation);
                if alive {
                    if self.db.complete(&job, token, self.now) {
                        self.report.jobs_done = self.report.jobs_done.saturating_add(1);
                    } else {
                        self.report.leases_lost = self.report.leases_lost.saturating_add(1);
                    }
                }
                Ok(())
            }
        }
    }

    fn restart(&mut self, n: usize) {
        if let Some(node) = self.nodes.get_mut(n) {
            if !node.up {
                node.up = true;
                node.id = NodeId::new(
                    node.id.id,
                    node.id.incarnation.saturating_add(1),
                    node.id.addr.clone(),
                );
                node.fencer = Fencer::new(node.id.clone());
                node.actors.clear();
            }
        }
    }

    fn count_split_brain(&mut self) {
        for base in self.bases.clone() {
            let owners: BTreeSet<usize> = (0..self.nodes.len())
                .filter(|n| {
                    self.nodes.get(*n).is_some_and(|node| node.up)
                        && self.owner_in_view(*n, base) == Some(*n)
                })
                .collect();
            if owners.len() > 1 {
                self.report.split_brains = self.report.split_brains.saturating_add(1);
            }
        }
    }

    fn deliver(&mut self, to: To, _from: Option<usize>, body: Body) -> Result<(), Violation> {
        match (to, body) {
            (
                To::Node(n),
                Body::Submit {
                    batch,
                    client,
                    hops,
                },
            ) => self.node_submit(n, batch, client, hops),
            (To::Client(c), Body::Outcome(base, outcome)) => self.client_receive(c, base, &outcome),
            (To::Node(_), Body::Outcome(..)) | (To::Client(_), Body::Submit { .. }) => Ok(()),
        }
    }

    /// Loads a base from the database: every journaled batch replayed, refusals recalled.
    fn load(&self, base: BaseId) -> Result<BaseState, Violation> {
        let (accepted, refused) = self.db.decisions(base);
        let mut state = BaseState::new(base);
        for a in &accepted {
            state.replay(a).map_err(|e| {
                self.violation(format!("the journal of {base} does not replay: {e}"))
            })?;
        }
        if self.config.persist_refusals {
            for r in refused {
                state.recall(r);
            }
        }
        Ok(state)
    }

    fn node_submit(
        &mut self,
        n: usize,
        batch: Batch,
        client: usize,
        hops: u8,
    ) -> Result<(), Violation> {
        if !self.nodes.get(n).is_some_and(|node| node.up) {
            return Ok(());
        }
        let base = batch.base;
        let owner = self.owner_in_view(n, base);
        if owner != Some(n) {
            if let Some(o) = owner {
                if hops < 3 {
                    self.send(
                        Some(n),
                        To::Node(o),
                        Body::Submit {
                            batch,
                            client,
                            hops: hops.saturating_add(1),
                        },
                    );
                }
            }
            return Ok(());
        }
        // This node owns the base, as far as it knows. It writes with the token it holds —
        // which may have been superseded meanwhile — and takes ownership only when it holds
        // none (at first, or after it learnt it was fenced).
        let resource = Db::resource(base);
        let token = {
            let Some(node) = self.nodes.get_mut(n) else {
                return Ok(());
            };
            match node.fencer.token(&resource) {
                Some(token) => token,
                None => match node.fencer.acquire(&mut self.db.epochs, &resource) {
                    Ok(token) => token,
                    Err(_) => return Ok(()),
                },
            }
        };
        let stale = self
            .nodes
            .get(n)
            .and_then(|node| node.actors.get(&base))
            .is_none_or(|(epoch, _)| *epoch != token.epoch);
        if stale {
            let state = self.load(base)?;
            if let Some(node) = self.nodes.get_mut(n) {
                node.actors.insert(base, (token.epoch, state));
            }
        }
        let Some(decision) = self
            .nodes
            .get_mut(n)
            .and_then(|node| node.actors.get_mut(&base))
            .map(|(_, actor)| actor.step(&batch))
        else {
            return Ok(());
        };
        if decision.repeated {
            self.report.repeats = self.report.repeats.saturating_add(1);
            self.send(
                Some(n),
                To::Client(client),
                Body::Outcome(base, decision.outcome),
            );
            return Ok(());
        }
        let result = match &decision.outcome {
            Outcome::Accepted(accepted) => self.db.commit(base, accepted, &token),
            Outcome::Rejected(rejection) if self.config.persist_refusals => {
                self.db.refuse(base, rejection, &token)
            }
            Outcome::Rejected(_) => Ok(()),
        };
        match result {
            Ok(()) => match &decision.outcome {
                Outcome::Accepted(_) => {
                    self.report.committed = self.report.committed.saturating_add(1);
                    for c in 0..self.clients.len() {
                        self.send(
                            Some(n),
                            To::Client(c),
                            Body::Outcome(base, decision.outcome.clone()),
                        );
                    }
                }
                Outcome::Rejected(_) => {
                    self.report.refused = self.report.refused.saturating_add(1);
                    self.send(
                        Some(n),
                        To::Client(client),
                        Body::Outcome(base, decision.outcome),
                    );
                }
            },
            Err(CommitError::Fenced) => {
                self.report.fenced = self.report.fenced.saturating_add(1);
                if let Some(node) = self.nodes.get_mut(n) {
                    node.fencer.on_fenced(&resource);
                    node.actors.remove(&base);
                }
            }
            Err(CommitError::Gap) => {
                return Err(self.violation(format!(
                    "node {n} held a current token for {base} but a stale state"
                )));
            }
        }
        Ok(())
    }

    fn client_tick(&mut self, c: usize) -> Result<(), Violation> {
        let now = self.now;
        if !self.stopped {
            let base = *self
                .bases
                .get(self.rng.index(self.bases.len()))
                .unwrap_or(&BaseId::from_u128(1));
            let ops = self.edit(c, base);
            if let Some(batch) = self
                .clients
                .get_mut(c)
                .and_then(|client| client.replicas.get_mut(&base))
                .and_then(|r| r.submit(ops).ok())
            {
                if let Some(client) = self.clients.get_mut(c) {
                    client.sent.insert(batch.id, now);
                }
                let node = self.rng.index(self.nodes.len());
                self.send(
                    None,
                    To::Node(node),
                    Body::Submit {
                        batch,
                        client: c,
                        hops: 0,
                    },
                );
            }
        }
        // Batches without an answer are sent again, to any node.
        let due: Vec<Batch> = self
            .clients
            .get(c)
            .map(|client| {
                client
                    .replicas
                    .values()
                    .flat_map(Replica::pending)
                    .filter(|b| {
                        client
                            .sent
                            .get(&b.id)
                            .is_none_or(|t| now.saturating_sub(*t) >= self.config.resend_ms)
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        for batch in due {
            if let Some(client) = self.clients.get_mut(c) {
                client.sent.insert(batch.id, now);
            }
            self.report.resends = self.report.resends.saturating_add(1);
            let node = self.rng.index(self.nodes.len());
            self.send(
                None,
                To::Node(node),
                Body::Submit {
                    batch,
                    client: c,
                    hops: 0,
                },
            );
        }
        let pending = self.clients.get(c).is_some_and(|client| {
            client
                .replicas
                .values()
                .any(|r| r.pending().next().is_some())
        });
        if !self.stopped || pending {
            let every = if self.stopped {
                self.config.resend_ms
            } else {
                self.config.edit_every_ms
            };
            let delay = self
                .rng
                .range(every / 2, every.saturating_mul(3) / 2)
                .max(1);
            self.at(delay, Event::ClientTick(c));
        }
        Ok(())
    }

    /// A small random edit on what the client sees.
    fn edit(&mut self, c: usize, base: BaseId) -> Vec<Op> {
        let Some(client) = self.clients.get(c) else {
            return Vec::new();
        };
        let Some(replica) = client.replicas.get(&base) else {
            return Vec::new();
        };
        let view = replica.view();
        let confirmed = replica.confirmed().clone();
        let known: Vec<(RecordId, bool)> =
            view.records().map(|((_, r), s)| (*r, s.deleted)).collect();
        let made = client.made;
        let mut ops = Vec::new();
        let mut created = 0_u128;
        for _ in 0..self.rng.range(1, 3) {
            let pick = if known.is_empty() {
                None
            } else {
                known.get(self.rng.index(known.len())).copied()
            };
            let value = match self.rng.below(4) {
                0 => Value::Null,
                n => Value::Text(format!("c{c}-{n}-{}", self.rng.below(3))),
            };
            let op = match (self.rng.below(10), pick) {
                (0..=2, _) | (_, None) => {
                    created = created.saturating_add(1);
                    let record = RecordId::from_u128(
                        (c as u128)
                            .saturating_add(1)
                            .saturating_mul(1_000_000)
                            .saturating_add(made)
                            .saturating_add(created),
                    );
                    Op::Create {
                        table: TABLE,
                        record,
                        cells: BTreeMap::from([(field(0), value)]),
                    }
                }
                (3..=6, Some((r, _))) => {
                    let f = field(self.rng.below(3));
                    let seen = confirmed
                        .record(TABLE, r)
                        .map_or(0, |rec| rec.cell_version(f));
                    Op::Set {
                        table: TABLE,
                        record: r,
                        field: f,
                        value,
                        expected: seen,
                    }
                }
                (7, Some((r, _))) => {
                    let seen = confirmed.record(TABLE, r).map_or(0, |rec| rec.version);
                    Op::Transition {
                        table: TABLE,
                        record: r,
                        field: field(9),
                        value,
                        expected: seen,
                    }
                }
                (8, Some((r, true))) | (_, Some((r, true))) => Op::Restore {
                    table: TABLE,
                    record: r,
                },
                (_, Some((r, false))) => {
                    let seen = confirmed.record(TABLE, r).map_or(0, |rec| rec.version);
                    Op::Delete {
                        table: TABLE,
                        record: r,
                        expected: seen,
                    }
                }
            };
            ops.push(op);
        }
        if let Some(client) = self.clients.get_mut(c) {
            client.made = client.made.saturating_add(created);
        }
        ops
    }

    fn client_receive(
        &mut self,
        c: usize,
        base: BaseId,
        outcome: &Outcome,
    ) -> Result<(), Violation> {
        if let Outcome::Rejected(r) = outcome {
            if r.id.client == ClientId((c as u64).saturating_add(1)) {
                if let Some(client) = self.clients.get_mut(c) {
                    client.told_refused.insert((base, r.id));
                }
            }
        }
        let result = self
            .clients
            .get_mut(c)
            .and_then(|client| client.replicas.get_mut(&base))
            .map(|r| r.receive(outcome));
        match result {
            Some(Err(OpsError::Gap { .. })) => {
                // Missed changes: catch up from the journal (a node serves it from the database).
                self.report.catch_ups = self.report.catch_ups.saturating_add(1);
                let after = self
                    .clients
                    .get(c)
                    .and_then(|client| client.replicas.get(&base))
                    .map_or(0, |r| r.confirmed().seq());
                for accepted in self.db.after(base, after) {
                    let replay = self
                        .clients
                        .get_mut(c)
                        .and_then(|client| client.replicas.get_mut(&base))
                        .map(|r| r.receive(&Outcome::Accepted(accepted)));
                    if let Some(Err(e)) = replay {
                        return Err(self.violation(format!("client {c} could not catch up: {e}")));
                    }
                }
                if let Some(r) = self
                    .clients
                    .get_mut(c)
                    .and_then(|client| client.replicas.get_mut(&base))
                {
                    r.receive(outcome).map_err(|e| Violation {
                        seed: self.seed,
                        at_ms: self.now,
                        what: format!("client {c}: {e}"),
                    })?;
                }
                Ok(())
            }
            Some(Err(e)) => Err(self.violation(format!("client {c} could not take a change: {e}"))),
            Some(Ok(())) | None => Ok(()),
        }
    }

    /// After a reconnect: every accepted batch the client has not seen, from the journal.
    fn catch_up(&mut self, c: usize) -> Result<(), Violation> {
        self.report.catch_ups = self.report.catch_ups.saturating_add(1);
        for base in self.bases.clone() {
            let after = self
                .clients
                .get(c)
                .and_then(|client| client.replicas.get(&base))
                .map_or(0, |r| r.confirmed().seq());
            for accepted in self.db.after(base, after) {
                let result = self
                    .clients
                    .get_mut(c)
                    .and_then(|client| client.replicas.get_mut(&base))
                    .map(|r| r.receive(&Outcome::Accepted(accepted)));
                if let Some(Err(e)) = result {
                    return Err(self.violation(format!("client {c} could not catch up: {e}")));
                }
            }
        }
        Ok(())
    }

    fn worker_tick(&mut self, n: usize) {
        if self.nodes.get(n).is_some_and(|node| node.up) {
            let claimed = self.db.claim(self.now, 2, self.config.lease_ms);
            let incarnation = self.nodes.get(n).map_or(0, |node| node.id.incarnation);
            for (job, token) in claimed {
                self.report.claims = self.report.claims.saturating_add(1);
                let work = self
                    .rng
                    .range(5, self.config.lease_ms.saturating_mul(3) / 2);
                self.at(
                    work,
                    Event::JobDone {
                        node: n,
                        incarnation,
                        job,
                        token,
                    },
                );
            }
        }
        self.at(self.config.worker_every_ms, Event::WorkerTick(n));
    }

    /// The invariants, at the end.
    fn check(&mut self) -> Result<(), Violation> {
        for base in self.bases.clone() {
            let journal = self.db.journal.get(&base).cloned().unwrap_or_default();
            // 1. Numbered 1, 2, 3, … without gaps; written by non-decreasing epochs.
            for (i, entry) in journal.iter().enumerate() {
                if entry.accepted.seq != (i as u64).saturating_add(1) {
                    return Err(self.violation(format!(
                        "{base}: the journal has a gap at {}",
                        entry.accepted.seq
                    )));
                }
            }
            if journal
                .windows(2)
                .any(|w| w.first().map(|e| e.epoch) > w.get(1).map(|e| e.epoch))
            {
                return Err(self.violation(format!(
                    "{base}: a superseded owner wrote after a newer one"
                )));
            }
            // 2. No batch applied twice.
            let ids: BTreeSet<ClientOpId> = journal.iter().map(|e| e.accepted.id).collect();
            if ids.len() != journal.len() {
                return Err(self.violation(format!("{base}: a batch was applied twice")));
            }
            // 3. No batch applied after its client was told it was refused.
            for client in &self.clients {
                if let Some((_, id)) = client
                    .told_refused
                    .iter()
                    .find(|(b, id)| *b == base && ids.contains(id))
                {
                    self.report.refused_then_applied =
                        self.report.refused_then_applied.saturating_add(1);
                    if self.config.persist_refusals {
                        return Err(self.violation(format!(
                            "{base}: batch {id:?} was refused to its client, then applied"
                        )));
                    }
                }
            }
            // 4. Every client ends with exactly the database's records.
            let truth = self.load(base)?;
            let truth: Vec<_> = truth.records().map(|(k, v)| (*k, v.clone())).collect();
            for (c, client) in self.clients.iter().enumerate() {
                if let Some(replica) = client.replicas.get(&base) {
                    if replica.pending().next().is_some() {
                        return Err(
                            self.violation(format!("client {c} has unanswered batches for {base}"))
                        );
                    }
                    let view: Vec<_> = replica
                        .view()
                        .records()
                        .map(|(k, v)| (*k, v.clone()))
                        .collect();
                    if view != truth {
                        return Err(self.violation(format!(
                            "client {c} differs from the database for {base}"
                        )));
                    }
                }
            }
            self.report.final_epochs.push(self.db.epoch(base));
        }
        // 5. Every job ran to the end, and was completed exactly once.
        for (id, job) in &self.db.jobs {
            if !job.done || job.completions != 1 {
                return Err(self.violation(format!(
                    "job {id} ended done={} after {} completions",
                    job.done, job.completions
                )));
            }
        }
        Ok(())
    }
}

//! Deterministic simulation tests: many seeds, random faults, invariants checked
//! continuously and after healing. A failing seed is printed; replay it with
//! `MT_SIM_SEED=<seed>`; run more seeds with `MT_SIM_SEEDS=<count>`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::for_seeds;
use mt_cluster::membership::{Config, Event};
use mt_cluster::scheduler::shard_key;
use mt_cluster::sim::{NetConfig, SimCluster};
use mt_cluster::{
    check_token, ClusterError, EpochStore, Fencer, FireWindow, LeaseStore, MemberState,
    MemoryEpochStore, MemoryLeaseStore, NodeId, Route, RoutingTable, SchedulerConfig,
    ShardScheduler,
};
use std::collections::BTreeMap;

const PROBE_MS: u64 = 200;
const SUSPECT_MS: u64 = 1_000;
const RESOURCES: u8 = 8;
const SHARDS: u32 = 8;
const LEASE_TTL_MS: u64 = 1_500;

fn cfg() -> Config {
    Config {
        probe_interval_ms: PROBE_MS,
        probe_timeout_ms: 60,
        indirect_probes: 2,
        suspect_timeout_ms: SUSPECT_MS,
        sync_interval_ms: 2_000,
        retransmit_mult: 3,
        max_piggyback: 8,
        dead_retention_ms: 3_600_000,
        weight: 1,
    }
}

fn calm() -> NetConfig {
    NetConfig {
        min_delay_ms: 1,
        max_delay_ms: 30,
        loss_per_mille: 0,
        duplicate_per_mille: 50,
    }
}

fn stormy(c: &mut SimCluster) -> NetConfig {
    NetConfig {
        min_delay_ms: 1,
        max_delay_ms: c.rng.range(20, 150),
        loss_per_mille: c.rng.range(0, 200) as u16,
        duplicate_per_mille: c.rng.range(0, 100) as u16,
    }
}

/// Applies one random fault (partition, heal, crash, restart, leave).
fn random_fault(c: &mut SimCluster, on_restart: &mut dyn FnMut(&mut SimCluster, usize)) {
    let n = c.nodes.len();
    let i = c.rng.index(n);
    let up = c.nodes.iter().filter(|x| x.up).count();
    match c.rng.below(10) {
        0..=2 => {
            let cut = c.rng.range(1, n as u64 - 1) as usize;
            let rot = c.rng.index(n);
            c.partition(|k| u32::from((k + rot) % n < cut));
        }
        3 | 4 => c.net.heal(),
        5 | 6 => {
            if up > 1 {
                c.crash(i);
            }
        }
        7 | 8 => {
            if !c.nodes[i].up {
                c.restart(i);
                on_restart(c, i);
            }
        }
        _ => {
            if up > 1 && c.nodes[i].up {
                c.leave(i);
            }
        }
    }
}

fn heal(c: &mut SimCluster, on_restart: &mut dyn FnMut(&mut SimCluster, usize)) {
    c.net.heal();
    c.net.cfg = calm();
    for i in 0..c.nodes.len() {
        if !c.nodes[i].up && c.rng.chance(600) {
            c.restart(i);
            on_restart(c, i);
        }
    }
}

fn describe(c: &SimCluster) -> String {
    let mut s = String::new();
    for n in c.nodes.iter() {
        s.push_str(&format!(
            "\n  {} up={} sees {:?} / {:?}",
            n.membership.me(),
            n.up,
            n.membership
                .active_nodes()
                .iter()
                .map(|(x, _)| x.to_string())
                .collect::<Vec<_>>(),
            n.membership
                .members()
                .map(|m| format!("{}:{:?}", m.node, m.state))
                .collect::<Vec<_>>(),
        ));
    }
    s
}

#[test]
fn membership_converges_after_faults_heal() {
    for_seeds("membership_converges", 1, 500, |seed| {
        let n = 3 + (seed % 5) as usize;
        let mut c = SimCluster::new(seed, n, cfg(), calm());
        c.run_for(6_000);
        assert!(
            c.converged(),
            "initial join did not converge{}",
            describe(&c)
        );

        c.net.cfg = stormy(&mut c);
        for _ in 0..15 {
            random_fault(&mut c, &mut |_, _| {});
            let d = c.rng.range(200, 1_500);
            c.run_for(d);
        }
        heal(&mut c, &mut |_, _| {});
        c.run_for(30_000);
        assert!(c.converged(), "no convergence after heal{}", describe(&c));

        // No false permanent death: every up node is Alive (not merely suspect) everywhere.
        for a in c.nodes.iter().filter(|x| x.up) {
            for b in c.nodes.iter().filter(|x| x.up) {
                if a.membership.me() != b.membership.me() {
                    let m = a.membership.member(b.membership.me().id).unwrap();
                    assert_eq!(m.state, MemberState::Alive, "{}", describe(&c));
                    assert_eq!(&m.node, b.membership.me());
                }
            }
        }
    });
}

#[test]
fn dead_node_is_detected_within_bound() {
    for_seeds("dead_detected", 10_000, 500, |seed| {
        let n = 3 + (seed % 5) as usize;
        let mut c = SimCluster::new(seed, n, cfg(), calm());
        c.run_for(6_000);
        assert!(c.converged());
        // Lossy network: detection must still be bounded because every node probes the
        // victim itself within two rounds.
        c.net.cfg.loss_per_mille = (seed % 4) as u16 * 50;
        let victim = c.rng.index(n);
        let dead = c.nodes[victim].membership.me().clone();
        c.crash(victim);
        let t0 = c.now_ms;
        let bound = (2 * (n as u64 - 1) + 3) * PROBE_MS + SUSPECT_MS + 2 * c.step_ms;
        c.run_for(bound);
        for node in c.nodes.iter().filter(|x| x.up) {
            let m = node.membership.member(dead.id).unwrap();
            assert_eq!(
                m.state,
                MemberState::Dead,
                "bound {bound}ms{}",
                describe(&c)
            );
            let at = node
                .events
                .iter()
                .find(|(_, e)| *e == Event::Down(dead.clone()))
                .map(|(t, _)| *t)
                .unwrap();
            assert!(at - t0 <= bound);
        }
    });
}

#[test]
fn healthy_reachable_nodes_are_never_declared_dead_on_a_clean_network() {
    for_seeds("no_false_death", 20_000, 500, |seed| {
        let n = 3 + (seed % 5) as usize;
        let mut net = calm();
        net.max_delay_ms = 40; // below the probe timeout: every direct ack arrives in time
        let mut c = SimCluster::new(seed, n, cfg(), net);
        c.run_for(20_000);
        assert!(c.converged());
        for node in &c.nodes {
            for (_, e) in &node.events {
                match e {
                    Event::MemberUp(_) => {}
                    Event::Suspect(x) | Event::Down(x) | Event::Left(x) | Event::Renewed(x) => {
                        panic!("unexpected event about {x}: {e:?}")
                    }
                }
            }
        }
    });
}

/// Membership plus fencing, routing and the lease scheduler on shared stores.
struct World {
    c: SimCluster,
    fencers: Vec<Fencer>,
    scheds: Vec<ShardScheduler>,
    seen_events: Vec<usize>,
    skew: Vec<u64>,
    epochs: MemoryEpochStore,
    leases: MemoryLeaseStore,
    /// Accepted writes in commit order: (resource, epoch, owner).
    accepted: Vec<(Vec<u8>, u64, NodeId)>,
    /// Committed fire windows in commit order, with the holder.
    windows: Vec<(FireWindow, NodeId)>,
    rejected: u64,
    /// Only nodes that see a majority may acquire (optional guard, see README).
    gate_majority: bool,
}

fn sched_cfg() -> SchedulerConfig {
    SchedulerConfig {
        shard_count: SHARDS,
        lease_ttl_ms: LEASE_TTL_MS,
        renew_before_ms: 1_000,
    }
}

impl World {
    fn new(seed: u64, n: usize) -> Self {
        let mut c = SimCluster::new(seed, n, cfg(), calm());
        let ids: Vec<NodeId> = c.nodes.iter().map(|x| x.membership.me().clone()).collect();
        let skew = (0..n).map(|_| c.rng.range(0, 400)).collect();
        Self {
            fencers: ids.iter().cloned().map(Fencer::new).collect(),
            scheds: ids
                .iter()
                .cloned()
                .map(|id| ShardScheduler::new(sched_cfg(), id))
                .collect(),
            seen_events: vec![0; n],
            skew,
            c,
            epochs: MemoryEpochStore::new(),
            leases: MemoryLeaseStore::new(),
            accepted: Vec::new(),
            windows: Vec::new(),
            rejected: 0,
            gate_majority: false,
        }
    }

    fn on_restart(&mut self, i: usize) {
        let me = self.c.nodes[i].membership.me().clone();
        self.fencers[i] = Fencer::new(me.clone());
        self.scheds[i] = ShardScheduler::new(sched_cfg(), me);
        self.seen_events[i] = self.c.nodes[i].events.len();
    }

    fn table(&self, i: usize) -> RoutingTable {
        let m = &self.c.nodes[i].membership;
        RoutingTable::new(m.me().clone(), m.placement())
    }

    /// A write attempt with whatever token node `i` holds: accepted only if not fenced.
    fn write(&mut self, i: usize, r: &[u8]) {
        let Some(token) = self.fencers[i].token(r) else {
            return;
        };
        match check_token(&mut self.epochs, &token) {
            Ok(()) => self.accepted.push((r.to_vec(), token.epoch, token.owner)),
            Err(e) => {
                if e.is_fenced() {
                    self.rejected += 1;
                    self.fencers[i].on_fenced(r);
                }
            }
        }
    }

    /// A client operation arriving at node `i`; follows at most `hops` forwards.
    fn operation(&mut self, i: usize, r: &[u8], hops: u8) {
        let record = self.epochs.load(r).ok().flatten();
        let route = self
            .table(i)
            .route(r, record.as_ref(), self.fencers[i].held_epoch(r));
        match route {
            Route::Local { must_acquire } => {
                let gated = self.gate_majority && !self.c.nodes[i].membership.has_majority();
                if must_acquire && (gated || self.fencers[i].acquire(&mut self.epochs, r).is_err())
                {
                    return;
                }
                self.write(i, r);
            }
            Route::Forward { node } => {
                let target = self.c.nodes.iter().position(|x| {
                    x.up && *x.membership.me() == node
                        && self.c.net.connected(&SimCluster::addr(i), &node.addr)
                });
                if let (Some(t), true) = (target, hops > 0) {
                    self.operation(t, r, hops - 1);
                }
            }
            Route::Unavailable => {}
        }
    }

    fn step(&mut self) {
        self.c.step();
        let n = self.c.nodes.len();
        for i in 0..n {
            if !self.c.nodes[i].up {
                continue;
            }
            // React to a renewed identity: everything held belonged to the old instance.
            let events = self.c.nodes[i].events[self.seen_events[i]..].to_vec();
            self.seen_events[i] = self.c.nodes[i].events.len();
            for (_, e) in events {
                if let Event::Renewed(me) = e {
                    self.fencers[i].reset(me.clone());
                    self.scheds[i].reset(me);
                }
            }
            if self.c.rng.chance(20) {
                self.epochs.fail_next(1);
            }
            if self.c.rng.chance(20) {
                self.leases.fail_next(1);
            }
            let placement = self.c.nodes[i].membership.placement();
            // Hand off resources that moved away.
            for r in self.fencers[i].handoff_candidates(&placement) {
                let _ = self.fencers[i].release(&mut self.epochs, &r);
            }
            // Every node that believes it owns something keeps writing (a partitioned old
            // owner does exactly this).
            for r in 0..RESOURCES {
                self.write(i, &[r]);
            }
            // Random client operations.
            for _ in 0..2 {
                let r = [self.c.rng.below(u64::from(RESOURCES)) as u8];
                self.operation(i, &r, 2);
            }
            // Leases: node clocks are skewed against each other.
            let clock = self.c.now_ms + self.skew[i];
            self.scheds[i].tick(clock, &placement, &mut self.leases);
            let holder = self.c.nodes[i].membership.me().clone();
            for shard in 0..SHARDS {
                if let Ok(Some(w)) = self.scheds[i].fire(clock, &mut self.leases, shard, clock) {
                    self.windows.push((w, holder.clone()));
                }
            }
        }
    }

    fn run_for(&mut self, ms: u64) {
        let end = self.c.now_ms + ms;
        while self.c.now_ms < end {
            self.step();
        }
    }

    fn check_single_writer(&self) {
        let mut owner_of: BTreeMap<(Vec<u8>, u64), NodeId> = BTreeMap::new();
        let mut last_epoch: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
        for (r, epoch, owner) in &self.accepted {
            let o = owner_of
                .entry((r.clone(), *epoch))
                .or_insert_with(|| owner.clone());
            assert_eq!(o, owner, "two writers for resource {r:?} epoch {epoch}");
            let last = last_epoch.entry(r.clone()).or_insert(*epoch);
            assert!(
                *epoch >= *last,
                "stale epoch {epoch} accepted after {last} for {r:?}"
            );
            *last = *epoch;
        }
    }

    fn check_leases(&self) {
        let mut holder_of: BTreeMap<(u32, u64), NodeId> = BTreeMap::new();
        let mut cursor: BTreeMap<u32, (u64, u64)> = BTreeMap::new();
        for (w, holder) in &self.windows {
            let h = holder_of
                .entry((w.shard, w.term))
                .or_insert_with(|| holder.clone());
            assert_eq!(
                h, holder,
                "two holders fired shard {} in term {}",
                w.shard, w.term
            );
            assert!(w.after_ms < w.upto_ms);
            let (upto, term) = cursor.entry(w.shard).or_insert((0, 0));
            assert!(
                w.after_ms >= *upto,
                "shard {} fired ({}, {}] twice (already fired up to {upto})",
                w.shard,
                w.after_ms,
                w.upto_ms
            );
            assert!(w.term >= *term, "term went backwards on shard {}", w.shard);
            *upto = w.upto_ms;
            *term = w.term;
        }
    }
}

#[test]
fn single_writer_ownership_and_leases_hold_under_faults() {
    for_seeds("full_stack", 30_000, 500, |seed| {
        let n = 3 + (seed % 4) as usize;
        let mut w = World::new(seed, n);
        w.run_for(5_000);
        w.c.net.cfg = stormy(&mut w.c);
        for _ in 0..12 {
            let mut restarted = Vec::new();
            random_fault(&mut w.c, &mut |_, i| restarted.push(i));
            for i in restarted {
                w.on_restart(i);
            }
            let d = w.c.rng.range(200, 1_500);
            w.run_for(d);
            w.check_single_writer();
            w.check_leases();
        }
        let mut restarted = Vec::new();
        heal(&mut w.c, &mut |_, i| restarted.push(i));
        for i in restarted {
            w.on_restart(i);
        }
        w.run_for(30_000);
        w.check_single_writer();
        w.check_leases();
        assert!(
            w.c.converged(),
            "no convergence after heal{}",
            describe(&w.c)
        );
        assert!(!w.accepted.is_empty());
        assert!(!w.windows.is_empty());
        w.epochs.fail_next(0);
        w.leases.fail_next(0);

        // Every resource has exactly one owner, agreed by all alive nodes, and that owner
        // can write.
        let up: Vec<usize> = (0..n).filter(|i| w.c.nodes[*i].up).collect();
        for r in 0..RESOURCES {
            let r = [r];
            let entry = up[w.c.rng.index(up.len())];
            w.operation(entry, &r, 2);
            let record = w.epochs.load(&r).unwrap().unwrap();
            let owner = record
                .owner
                .clone()
                .expect("resource has an owner after healing");
            let mut local = 0;
            for &i in &up {
                let route = w
                    .table(i)
                    .route(&r, Some(&record), w.fencers[i].held_epoch(&r));
                match route {
                    Route::Local { must_acquire } => {
                        assert!(!must_acquire);
                        assert_eq!(w.c.nodes[i].membership.me(), &owner);
                        local += 1;
                    }
                    Route::Forward { node } => assert_eq!(node, owner),
                    Route::Unavailable => panic!("unavailable after healing"),
                }
            }
            assert_eq!(local, 1, "exactly one node owns {r:?}");
            assert_eq!(
                w.c.nodes[up[0]].membership.placement().owner(&r),
                Some(&owner),
                "owner is the rendezvous owner"
            );
        }
        // Every shard lease is held by its rendezvous owner.
        let placement = w.c.nodes[up[0]].membership.placement();
        for shard in 0..SHARDS {
            let rec = w.leases.load(shard).unwrap().unwrap();
            assert_eq!(rec.holder.as_ref(), placement.owner(&shard_key(shard)));
        }
        w.check_single_writer();
        w.check_leases();
    });
}

#[test]
fn partitioned_old_owner_is_fenced() {
    // Scripted: the owner of a resource is cut off; the other side takes over; the old
    // owner's writes are rejected from that moment on.
    let mut w = World::new(7, 3);
    w.gate_majority = true;
    w.run_for(5_000);
    let r = [3u8];
    w.operation(0, &r, 2);
    let old = w.epochs.load(&r).unwrap().unwrap().owner.unwrap();
    let old_i = (old.id - 1) as usize;
    w.c.partition(|k| u32::from(k == old_i));
    w.run_for(10_000);
    let new = w.epochs.load(&r).unwrap().unwrap();
    assert_ne!(
        new.owner.as_ref(),
        Some(&old),
        "ownership moved to the majority side"
    );
    assert!(w.rejected > 0, "the old owner was fenced at least once");
    assert_eq!(w.fencers[old_i].held_epoch(&r), None);
    w.check_single_writer();
    let stale = mt_cluster::FencingToken {
        resource: r.to_vec(),
        epoch: 1,
        owner: old,
    };
    assert!(matches!(
        check_token(&mut w.epochs, &stale),
        Err(ClusterError::Fenced { .. })
    ));
}

#[test]
fn same_seed_replays_identically() {
    let run = |seed| {
        let mut w = World::new(seed, 5);
        w.c.net.cfg = stormy(&mut w.c);
        for _ in 0..6 {
            let mut restarted = Vec::new();
            random_fault(&mut w.c, &mut |_, i| restarted.push(i));
            for i in restarted {
                w.on_restart(i);
            }
            w.run_for(1_000);
        }
        (
            w.accepted,
            w.windows,
            w.c.net.sent,
            w.c.net.dropped,
            w.c.up_nodes(),
        )
    };
    assert_eq!(run(42), run(42));
    assert_ne!(run(42).2, run(43).2);
}

// ---------------------------------------------------------------------------------------------
// The harness itself: a simulator that silently misbehaves would make every result above
// meaningless, so its own rules are tested too.
// ---------------------------------------------------------------------------------------------

fn packet(from: u64, to: &str) -> mt_cluster::sim::Packet {
    mt_cluster::sim::Packet {
        from: NodeId::new(from, 1, format!("n{from}")),
        to: to.to_owned(),
        bytes: vec![1, 2, 3],
    }
}

#[test]
fn harness_network_delays_partitions_and_heals() {
    use mt_cluster::rng::Rng;
    use mt_cluster::sim::SimNet;
    let mut rng = Rng::new(1);
    let mut net = SimNet::new(NetConfig::default());
    assert_eq!(net.in_flight(), 0);

    // Delivery happens after the configured delay (1..=20 ms), never before.
    net.send(100, &mut rng, packet(0, "n1"));
    assert_eq!((net.sent, net.dropped, net.in_flight()), (1, 0, 1));
    assert_eq!(net.next_due(100), None);
    assert_eq!(net.next_due(120), Some(packet(0, "n1")));
    assert_eq!(net.next_due(10_000), None);

    // A partition drops packets at send time…
    net.set_group("n1", 1);
    assert!(!net.connected("n0", "n1") && net.connected("n0", "n2"));
    net.send(200, &mut rng, packet(0, "n1"));
    assert_eq!((net.sent, net.dropped, net.in_flight()), (2, 1, 0));
    // …and packets already in flight when it starts are dropped at delivery time.
    net.heal();
    net.send(300, &mut rng, packet(0, "n1"));
    net.set_group("n0", 7);
    assert_eq!(net.next_due(400), None);
    assert_eq!((net.dropped, net.in_flight()), (2, 0));
    net.heal();
    assert!(net.connected("n0", "n1"));
}

#[test]
fn harness_network_loses_and_duplicates_as_configured() {
    use mt_cluster::rng::Rng;
    use mt_cluster::sim::SimNet;
    let mut rng = Rng::new(2);
    let mut lossy = SimNet::new(NetConfig {
        loss_per_mille: 1000,
        ..NetConfig::default()
    });
    for _ in 0..50 {
        lossy.send(0, &mut rng, packet(0, "n1"));
    }
    assert_eq!((lossy.sent, lossy.dropped, lossy.in_flight()), (50, 50, 0));

    let mut doubling = SimNet::new(NetConfig {
        duplicate_per_mille: 1000,
        ..NetConfig::default()
    });
    for _ in 0..50 {
        doubling.send(0, &mut rng, packet(0, "n1"));
    }
    assert_eq!(doubling.in_flight(), 100);
    // Packets come out in time order.
    let mut delivered = 0;
    for now in 0..=20 {
        while doubling.next_due(now).is_some() {
            delivered += 1;
        }
    }
    assert_eq!(delivered, 100);
}

#[test]
fn harness_cluster_controls_behave() {
    let mut c = SimCluster::new(5, 3, cfg(), calm());
    c.run_for(5_000);
    assert!(c.converged());
    assert_eq!(c.up_nodes().len(), 3);

    // Out-of-range indices are ignored rather than panicking mid-simulation.
    c.crash(99);
    c.restart(99);
    c.leave(99);
    c.input(99, mt_cluster::Input::Tick);
    assert_eq!(c.nodes.len(), 3);

    // A crashed node processes nothing; a restart is a new instance that keeps the
    // recorded event history for later assertions.
    let before = c.nodes[2].membership.me().clone();
    let history = c.nodes[2].events.len();
    assert!(history > 0);
    c.crash(2);
    assert_eq!(c.up_nodes().len(), 2);
    c.input(2, mt_cluster::Input::Tick);
    assert_eq!(c.nodes[2].events.len(), history);
    c.restart(2);
    let after = c.nodes[2].membership.me().clone();
    assert_eq!(after.id, before.id);
    assert!(after.incarnation > before.incarnation);
    assert_eq!(c.nodes[2].events.len(), history);
    c.run_for(20_000);
    assert!(c.converged());

    // A graceful leave takes the node out of everyone's view.
    c.leave(1);
    c.run_for(10_000);
    assert!(c.converged());
    assert_eq!(c.up_nodes().len(), 2);
}

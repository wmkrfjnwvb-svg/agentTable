//! Membership state machine tests: small, fully deterministic scenarios that drive one or two
//! machines by hand. Whole-cluster behaviour under faults is covered by `simulation.rs`.
//! Categories are marked in the test names (`std_`, `anomalous_`, `edge_`, `invalid_`, `prop_`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use mt_cluster::codec::{validate, MAX_ADDR_BYTES, MAX_UPDATES, MAX_WEIGHT};
use mt_cluster::membership::{MAX_FORWARDS, MAX_SEEDS};
use mt_cluster::message::Hello;
use mt_cluster::{
    encode, Config, Event, Input, MemberState, Membership, Message, NodeId, Output, Update,
    PROTOCOL_VERSION,
};
use proptest::prelude::*;
use std::collections::BTreeSet;

fn cfg() -> Config {
    Config {
        probe_interval_ms: 1_000,
        probe_timeout_ms: 300,
        indirect_probes: 2,
        suspect_timeout_ms: 3_000,
        sync_interval_ms: 10_000,
        retransmit_mult: 2,
        max_piggyback: 4,
        dead_retention_ms: 60_000,
        weight: 1,
    }
}

fn n(id: u64) -> NodeId {
    NodeId::new(id, 1, format!("n{id}"))
}

fn machine(id: u64) -> Membership {
    Membership::new(cfg(), n(id), id)
}

fn fact(id: u64, state: MemberState) -> Update {
    Update {
        node: n(id),
        version: 0,
        state,
        weight: 1,
    }
}

fn alive(id: u64) -> Update {
    fact(id, MemberState::Alive)
}

/// Delivers facts as a one-way sync from an unrelated peer.
fn teach(m: &mut Membership, now: u64, updates: Vec<Update>) -> Vec<Output> {
    let msg = Message::Sync {
        want_reply: false,
        updates,
    };
    m.handle(now, Input::Message { from: n(99), msg })
}

fn tick(m: &mut Membership, now: u64) -> Vec<Output> {
    m.handle(now, Input::Tick)
}

fn deliver(m: &mut Membership, now: u64, from: u64, msg: Message) -> Vec<Output> {
    m.handle(now, Input::Message { from: n(from), msg })
}

fn sends(out: &[Output]) -> Vec<(String, Message)> {
    out.iter()
        .filter_map(|o| match o {
            Output::Send { to, msg } => Some((to.clone(), msg.clone())),
            Output::Event(_) => None,
        })
        .collect()
}

fn events(out: &[Output]) -> Vec<Event> {
    out.iter()
        .filter_map(|o| match o {
            Output::Event(e) => Some(e.clone()),
            Output::Send { .. } => None,
        })
        .collect()
}

fn pings(out: &[Output]) -> Vec<(String, u64, Vec<Update>)> {
    sends(out)
        .into_iter()
        .filter_map(|(to, msg)| {
            if let Message::Ping { seq, updates } = msg {
                Some((to, seq, updates))
            } else {
                None
            }
        })
        .collect()
}

fn state_of(m: &Membership, id: u64) -> Option<MemberState> {
    m.member(id).map(|member| member.state)
}

fn active_ids(m: &Membership) -> Vec<u64> {
    m.active_nodes().iter().map(|(node, _)| node.id).collect()
}

// ---------------------------------------------------------------------------------------------
// Joining and anti-entropy
// ---------------------------------------------------------------------------------------------

#[test]
fn std_default_config_is_sane() {
    let c = Config::default();
    assert!(c.probe_timeout_ms < c.probe_interval_ms);
    assert!(c.suspect_timeout_ms >= c.probe_interval_ms);
    assert!(c.sync_interval_ms >= c.probe_interval_ms);
    assert!(c.indirect_probes > 0 && c.retransmit_mult > 0 && c.max_piggyback > 0);
    assert!((1..=MAX_WEIGHT).contains(&c.weight));
}

#[test]
fn std_new_machine_knows_only_itself() {
    let m = machine(1);
    assert_eq!(m.me(), &n(1));
    assert!(!m.has_left());
    assert_eq!(m.members().count(), 0);
    assert_eq!(m.active_nodes(), vec![(n(1), 1)]);
    assert!(m.has_majority());
    assert_eq!(m.placement().owner(b"k"), Some(&n(1)));
}

#[test]
fn std_join_sends_the_full_state_to_every_usable_seed() {
    let mut m = machine(1);
    let seeds = vec![
        "n2".to_owned(),
        String::new(),
        "x".repeat(MAX_ADDR_BYTES + 1),
        "n1".to_owned(), // our own address
        "n3".to_owned(),
    ];
    let out = m.handle(0, Input::Join { seeds });
    assert!(events(&out).is_empty());
    let expected = Message::Sync {
        want_reply: true,
        updates: vec![alive(1)],
    };
    assert_eq!(
        sends(&out),
        vec![
            ("n2".to_owned(), expected.clone()),
            ("n3".to_owned(), expected)
        ]
    );
}

#[test]
fn edge_seed_list_is_bounded() {
    let mut m = machine(1);
    let seeds: Vec<String> = (0..MAX_SEEDS + 6).map(|i| format!("seed{i}")).collect();
    let out = m.handle(0, Input::Join { seeds });
    assert_eq!(sends(&out).len(), MAX_SEEDS);
}

#[test]
fn std_lonely_node_keeps_asking_its_seeds() {
    let mut m = machine(1);
    m.handle(
        0,
        Input::Join {
            seeds: vec!["seed".to_owned()],
        },
    );
    // Nobody answered. At every sync interval the seeds are tried again.
    for now in [0, 10_000, 20_000] {
        let out = tick(&mut m, now);
        assert_eq!(sends(&out).len(), 1, "at {now}");
        assert_eq!(sends(&out)[0].0, "seed");
    }
    assert!(sends(&tick(&mut m, 25_000)).is_empty());
}

#[test]
fn std_two_nodes_learn_about_each_other_through_one_sync_exchange() {
    let mut a = machine(1);
    let mut b = machine(2);
    let out = a.handle(
        0,
        Input::Join {
            seeds: vec!["n2".to_owned()],
        },
    );
    let (to, hello) = sends(&out).remove(0);
    assert_eq!(to, "n2");

    let out = deliver(&mut b, 1, 1, hello);
    assert_eq!(events(&out), vec![Event::MemberUp(n(1))]);
    let (to, reply) = sends(&out).remove(0);
    assert_eq!(to, "n1");
    assert_eq!(
        reply,
        Message::Sync {
            want_reply: false,
            updates: vec![alive(2), alive(1)]
        }
    );

    let out = deliver(&mut a, 2, 2, reply);
    assert_eq!(events(&out), vec![Event::MemberUp(n(2))]);
    assert!(sends(&out).is_empty());

    for m in [&a, &b] {
        assert_eq!(m.active_nodes(), vec![(n(1), 1), (n(2), 1)]);
        assert_eq!(m.placement().len(), 2);
        assert!(m.has_majority());
    }
    let seen = a.member(2).unwrap();
    assert_eq!(
        (seen.state, seen.version, seen.weight, seen.since_ms),
        (MemberState::Alive, 0, 1, 2)
    );
    // Both compute the same owner for any key.
    assert_eq!(a.placement().owner(b"base"), b.placement().owner(b"base"));
}

#[test]
fn std_ping_is_answered_with_an_ack_carrying_our_state() {
    let mut b = machine(2);
    let out = deliver(
        &mut b,
        5,
        1,
        Message::Ping {
            seq: 9,
            updates: vec![alive(1)],
        },
    );
    assert_eq!(events(&out), vec![Event::MemberUp(n(1))]);
    let (to, msg) = sends(&out).remove(0);
    assert_eq!(to, "n1");
    let Message::Ack { seq, updates } = msg else {
        panic!("expected an ack")
    };
    assert_eq!(seq, 9);
    assert_eq!(updates[0], alive(2));
}

#[test]
fn invalid_messages_from_ourselves_and_handshake_messages_are_ignored() {
    let mut m = machine(1);
    let hello = Hello {
        protocol: PROTOCOL_VERSION,
        node: n(2),
        nonce: [0; 16],
        tag: [0; 32],
    };
    // Our own messages reflected back (a misrouted packet) change nothing.
    let reflected = Message::Sync {
        want_reply: true,
        updates: vec![alive(5)],
    };
    assert!(deliver(&mut m, 0, 1, reflected).is_empty());
    assert_eq!(m.members().count(), 0);
    // Handshake messages belong to the transport, not to membership.
    for msg in [
        Message::Hello(hello.clone()),
        Message::HelloAck(hello),
        Message::HelloConfirm { tag: [0; 32] },
    ] {
        assert!(deliver(&mut m, 0, 2, msg).is_empty());
    }
    assert_eq!(m.members().count(), 0);
}

// ---------------------------------------------------------------------------------------------
// Failure detection
// ---------------------------------------------------------------------------------------------

#[test]
fn std_answered_probes_keep_a_member_alive() {
    let mut m = machine(1);
    teach(&mut m, 0, vec![alive(2)]);
    for round in 0..5_u64 {
        let now = round * 1_000;
        let out = tick(&mut m, now);
        assert!(events(&out).is_empty(), "round {round}");
        let probes = pings(&out);
        assert_eq!(probes.len(), 1);
        let (to, seq, updates) = probes[0].clone();
        assert_eq!(to, "n2");
        assert_eq!(updates[0], alive(1));
        deliver(
            &mut m,
            now + 20,
            2,
            Message::Ack {
                seq,
                updates: vec![alive(2)],
            },
        );
    }
    assert_eq!(state_of(&m, 2), Some(MemberState::Alive));
}

#[test]
fn anomalous_silent_member_becomes_suspect_then_dead_then_forgotten() {
    let mut m = machine(1);
    assert_eq!(
        events(&teach(&mut m, 0, vec![alive(2)])),
        vec![Event::MemberUp(n(2))]
    );

    // t=0: probe, plus the first anti-entropy sync.
    let out = tick(&mut m, 0);
    assert_eq!(pings(&out).len(), 1);
    assert!(sends(&out).iter().any(|(to, msg)| to == "n2"
        && matches!(
            msg,
            Message::Sync {
                want_reply: true,
                ..
            }
        )));

    // No ack. With nobody to ask for an indirect probe, nothing is sent at the timeout.
    assert!(tick(&mut m, 299).is_empty());
    assert!(tick(&mut m, 300).is_empty());
    assert!(tick(&mut m, 999).is_empty());

    // End of the protocol period: suspect. A suspect is still a member and is still probed,
    // and the probe tells it about the suspicion so that it can refute.
    let out = tick(&mut m, 1_000);
    assert_eq!(events(&out), vec![Event::Suspect(n(2))]);
    assert_eq!(state_of(&m, 2), Some(MemberState::Suspect));
    assert_eq!(active_ids(&m), vec![1, 2]);
    let (_, _, gossip) = pings(&out).remove(0);
    assert!(gossip.contains(&fact(2, MemberState::Suspect)));

    assert!(events(&tick(&mut m, 2_000)).is_empty());
    assert!(events(&tick(&mut m, 3_000)).is_empty());
    assert!(events(&tick(&mut m, 3_999)).is_empty());

    // Suspect timeout: dead. Dead members own nothing and are not probed.
    let out = tick(&mut m, 4_000);
    assert_eq!(events(&out), vec![Event::Down(n(2))]);
    assert!(sends(&out).is_empty());
    assert_eq!(state_of(&m, 2), Some(MemberState::Dead));
    assert_eq!(active_ids(&m), vec![1]);
    assert!(m.placement().owner(b"k") == Some(&n(1)));
    // One of two remembered nodes is not a strict majority.
    assert!(!m.has_majority());
    assert!(pings(&tick(&mut m, 5_000)).is_empty());

    // Anti-entropy keeps contacting the dead member, so a healed partition is noticed.
    let out = tick(&mut m, 10_000);
    assert_eq!(sends(&out).len(), 1);
    assert!(matches!(&sends(&out)[0], (to, Message::Sync { want_reply: true, .. }) if to == "n2"));

    // The tombstone is kept for the retention period, then dropped.
    tick(&mut m, 63_999);
    assert_eq!(state_of(&m, 2), Some(MemberState::Dead));
    tick(&mut m, 64_000);
    assert_eq!(m.member(2), None);
    assert!(m.has_majority());
}

#[test]
fn std_indirect_probe_saves_a_member_whose_direct_path_is_broken() {
    let mut m = machine(1);
    teach(&mut m, 0, vec![alive(2), alive(3)]);
    let out = tick(&mut m, 0);
    let (target_addr, seq, _) = pings(&out).remove(0);
    let (target, helper) = if target_addr == "n2" { (2, 3) } else { (3, 2) };

    // No direct ack by the timeout: the other member is asked to probe on our behalf.
    let out = tick(&mut m, 300);
    let requests = sends(&out);
    assert_eq!(requests.len(), 1);
    let (to, msg) = requests[0].clone();
    assert_eq!(to, format!("n{helper}"));
    let Message::PingReq {
        seq: asked,
        target: asked_target,
        ..
    } = msg
    else {
        panic!("expected an indirect probe request")
    };
    assert_eq!((asked, asked_target), (seq, n(target)));
    // Asked only once per probe.
    assert!(tick(&mut m, 400).is_empty());

    // The helper relays the target's ack: the probe succeeded.
    deliver(
        &mut m,
        500,
        helper,
        Message::Ack {
            seq,
            updates: vec![],
        },
    );
    let out = tick(&mut m, 1_000);
    assert!(events(&out).is_empty());
    assert_eq!(state_of(&m, target), Some(MemberState::Alive));
    // The next period probes the other member (round-robin).
    assert_eq!(pings(&out).remove(0).0, format!("n{helper}"));
}

#[test]
fn anomalous_no_ack_by_either_path_means_suspect() {
    let mut m = machine(1);
    teach(&mut m, 0, vec![alive(2), alive(3)]);
    let (target_addr, _, _) = pings(&tick(&mut m, 0)).remove(0);
    tick(&mut m, 300);
    let out = tick(&mut m, 1_000);
    let suspected = if target_addr == "n2" { n(2) } else { n(3) };
    assert_eq!(events(&out), vec![Event::Suspect(suspected)]);
}

#[test]
fn anomalous_late_or_foreign_acks_do_not_clear_a_probe() {
    let mut m = machine(1);
    teach(&mut m, 0, vec![alive(2)]);
    let (_, seq, _) = pings(&tick(&mut m, 0)).remove(0);
    // An ack with another sequence number (a duplicate from an earlier round).
    deliver(
        &mut m,
        10,
        2,
        Message::Ack {
            seq: seq.wrapping_add(17),
            updates: vec![],
        },
    );
    assert_eq!(events(&tick(&mut m, 1_000)), vec![Event::Suspect(n(2))]);
    // The right ack after the period ended does not undo the suspicion by itself; the
    // member has to refute it with a higher version.
    deliver(
        &mut m,
        1_010,
        2,
        Message::Ack {
            seq,
            updates: vec![alive(2)],
        },
    );
    assert_eq!(state_of(&m, 2), Some(MemberState::Suspect));
}

#[test]
fn std_helper_relays_an_indirect_probe_and_its_ack() {
    let mut helper = machine(3);
    let out = deliver(
        &mut helper,
        0,
        1,
        Message::PingReq {
            seq: 77,
            target: n(2),
            updates: vec![],
        },
    );
    let (to, relay_seq, _) = pings(&out).remove(0);
    assert_eq!(to, "n2");

    let ack = Message::Ack {
        seq: relay_seq,
        updates: vec![],
    };
    let out = deliver(&mut helper, 10, 2, ack.clone());
    let (to, msg) = sends(&out).remove(0);
    assert_eq!(to, "n1");
    assert!(matches!(msg, Message::Ack { seq: 77, .. }));

    // A duplicate ack, or an ack nobody is waiting for, is dropped.
    assert!(deliver(&mut helper, 11, 2, ack).is_empty());
    let unknown = Message::Ack {
        seq: relay_seq.wrapping_add(1),
        updates: vec![],
    };
    assert!(deliver(&mut helper, 12, 2, unknown).is_empty());
}

#[test]
fn edge_helper_forgets_relays_after_one_protocol_period() {
    let mut helper = machine(3);
    let out = deliver(
        &mut helper,
        20,
        1,
        Message::PingReq {
            seq: 5,
            target: n(2),
            updates: vec![],
        },
    );
    let (_, relay_seq, _) = pings(&out).remove(0);
    tick(&mut helper, 1_020);
    let late = Message::Ack {
        seq: relay_seq,
        updates: vec![],
    };
    assert!(sends(&deliver(&mut helper, 1_021, 2, late)).is_empty());
}

#[test]
fn edge_relayed_probes_are_bounded() {
    let mut helper = machine(3);
    let mut relayed = 0;
    for seq in 0..u64::try_from(MAX_FORWARDS).unwrap() + 5 {
        let out = deliver(
            &mut helper,
            0,
            1,
            Message::PingReq {
                seq,
                target: n(2),
                updates: vec![],
            },
        );
        relayed += pings(&out).len();
    }
    assert_eq!(relayed, MAX_FORWARDS);
}

#[test]
fn edge_probe_is_dropped_when_its_target_is_declared_dead_meanwhile() {
    let mut m = machine(1);
    teach(&mut m, 0, vec![alive(2)]);
    tick(&mut m, 0);
    assert_eq!(
        events(&teach(&mut m, 100, vec![fact(2, MemberState::Dead)])),
        vec![Event::Down(n(2))]
    );
    // No suspicion of an already dead member at the end of the period.
    assert!(events(&tick(&mut m, 1_000)).is_empty());
    assert_eq!(state_of(&m, 2), Some(MemberState::Dead));
}

#[test]
fn edge_restarted_instance_is_not_blamed_for_the_silence_of_its_predecessor() {
    let mut m = machine(1);
    teach(&mut m, 0, vec![alive(2)]);
    assert_eq!(pings(&tick(&mut m, 0)).remove(0).0, "n2");
    // While the probe of the old instance is pending, the node restarts elsewhere.
    let restarted = NodeId::new(2, 2, "n2-new");
    let update = Update {
        node: restarted.clone(),
        ..alive(2)
    };
    assert_eq!(
        events(&teach(&mut m, 100, vec![update])),
        vec![Event::Down(n(2)), Event::MemberUp(restarted.clone())]
    );
    // The old probe times out. The new instance was never pinged, so it is not suspected;
    // it simply gets its own probe.
    assert!(sends(&tick(&mut m, 300)).is_empty());
    let out = tick(&mut m, 1_000);
    assert!(events(&out).is_empty());
    assert_eq!(pings(&out).remove(0).0, "n2-new");
    assert_eq!(state_of(&m, 2), Some(MemberState::Alive));
    // And if the new instance stays silent, it is suspected on its own account.
    assert_eq!(
        events(&tick(&mut m, 2_000)),
        vec![Event::Suspect(restarted)]
    );
}

// ---------------------------------------------------------------------------------------------
// Refutation, renewal and leaving
// ---------------------------------------------------------------------------------------------

fn self_update_of(m: &mut Membership) -> Update {
    let out = deliver(
        m,
        0,
        98,
        Message::Ping {
            seq: 0,
            updates: vec![],
        },
    );
    let Message::Ack { mut updates, .. } = sends(&out).remove(0).1 else {
        panic!("expected an ack")
    };
    updates.remove(0)
}

#[test]
fn std_suspected_node_refutes_by_raising_its_version() {
    let mut b = machine(2);
    let out = teach(&mut b, 0, vec![fact(2, MemberState::Suspect)]);
    assert!(out.is_empty());
    let refutation = self_update_of(&mut b);
    assert_eq!(
        refutation,
        Update {
            version: 1,
            ..alive(2)
        }
    );

    // A stale suspicion (older version) needs no new refutation.
    teach(&mut b, 1, vec![fact(2, MemberState::Suspect)]);
    assert_eq!(self_update_of(&mut b).version, 1);
    // A suspicion of the current version does.
    let current = Update {
        version: 1,
        ..fact(2, MemberState::Suspect)
    };
    teach(&mut b, 2, vec![current]);
    assert_eq!(self_update_of(&mut b).version, 2);
    // An echo of a higher alive version of ourselves is adopted (never go backwards).
    let echo = Update {
        version: 10,
        ..alive(2)
    };
    teach(&mut b, 3, vec![echo]);
    assert_eq!(self_update_of(&mut b).version, 10);
    assert_eq!(b.me(), &n(2));

    // The suspecting node accepts the refutation.
    let mut a = machine(1);
    teach(&mut a, 0, vec![fact(2, MemberState::Suspect)]);
    assert_eq!(
        events(&teach(&mut a, 1, vec![refutation])),
        vec![Event::MemberUp(n(2))]
    );
    assert_eq!(state_of(&a, 2), Some(MemberState::Alive));
}

#[test]
fn std_node_declared_dead_renews_its_incarnation() {
    let mut b = machine(2);
    let out = teach(&mut b, 0, vec![fact(2, MemberState::Dead)]);
    let renewed = NodeId::new(2, 2, "n2");
    assert_eq!(events(&out), vec![Event::Renewed(renewed.clone())]);
    assert_eq!(b.me(), &renewed);
    // The new identity is announced with a fresh version counter.
    let announced = self_update_of(&mut b);
    assert_eq!((announced.node, announced.version), (renewed, 0));

    // Facts about older incarnations of ourselves are history.
    assert!(teach(&mut b, 1, vec![fact(2, MemberState::Dead)]).is_empty());
    assert_eq!(b.me().incarnation, 2);

    // Someone knows a newer instance than we are (we restarted with a stale counter):
    // jump past it, whatever it says.
    let newer = Update {
        node: NodeId::new(2, 7, "n2"),
        version: 3,
        state: MemberState::Alive,
        weight: 1,
    };
    let out = teach(&mut b, 2, vec![newer]);
    assert_eq!(events(&out), vec![Event::Renewed(NodeId::new(2, 8, "n2"))]);

    // Being recorded as "left" while running is treated like being declared dead.
    let left = Update {
        node: NodeId::new(2, 8, "n2"),
        version: 0,
        state: MemberState::Left,
        weight: 1,
    };
    let out = teach(&mut b, 3, vec![left]);
    assert_eq!(events(&out), vec![Event::Renewed(NodeId::new(2, 9, "n2"))]);
}

#[test]
fn std_leaving_announces_and_makes_the_machine_inert() {
    let mut m = machine(1);
    teach(&mut m, 0, (2..=6).map(alive).collect());
    let out = m.handle(10, Input::Leave);
    assert_eq!(events(&out), vec![Event::Left(n(1))]);
    let goodbye = Message::Sync {
        want_reply: false,
        updates: vec![fact(1, MemberState::Left)],
    };
    let told = sends(&out);
    assert_eq!(told.len(), 3);
    assert!(told.iter().all(|(_, msg)| *msg == goodbye));
    let recipients: BTreeSet<&String> = told.iter().map(|(to, _)| to).collect();
    assert_eq!(recipients.len(), 3);

    assert!(m.has_left());
    assert!(!m.has_majority());
    assert_eq!(active_ids(&m), vec![2, 3, 4, 5, 6]);
    assert!(!m.placement().contains(&n(1)));
    // Nothing wakes it up again.
    assert!(tick(&mut m, 20).is_empty());
    assert!(m.handle(30, Input::Leave).is_empty());
    assert!(m
        .handle(
            40,
            Input::Join {
                seeds: vec!["n2".to_owned()]
            }
        )
        .is_empty());
    assert!(teach(&mut m, 50, vec![alive(7)]).is_empty());
    assert_eq!(m.member(7), None);
}

#[test]
fn edge_leaving_a_tiny_cluster_tells_whoever_is_there() {
    let mut alone = machine(1);
    let out = alone.handle(0, Input::Leave);
    assert!(sends(&out).is_empty());
    assert_eq!(events(&out), vec![Event::Left(n(1))]);

    let mut pair = machine(1);
    teach(&mut pair, 0, vec![alive(2)]);
    assert_eq!(sends(&pair.handle(1, Input::Leave)).len(), 1);
}

#[test]
fn std_departure_is_final_for_the_incarnation_and_a_restart_rejoins() {
    let mut b = machine(2);
    teach(&mut b, 0, vec![alive(1)]);
    assert_eq!(
        events(&teach(&mut b, 5, vec![fact(1, MemberState::Left)])),
        vec![Event::Left(n(1))]
    );
    assert_eq!(state_of(&b, 1), Some(MemberState::Left));
    assert_eq!(active_ids(&b), vec![2]);
    // Left members do not count against the majority (they are not coming back).
    assert!(b.has_majority());

    // Late gossip that the old instance is alive is ignored…
    let late = Update {
        version: 50,
        ..alive(1)
    };
    assert!(teach(&mut b, 6, vec![late]).is_empty());
    assert_eq!(state_of(&b, 1), Some(MemberState::Left));

    // …but the restarted node (new incarnation) is welcome.
    let restarted = Update {
        node: NodeId::new(1, 2, "n1"),
        ..alive(1)
    };
    assert_eq!(
        events(&teach(&mut b, 7, vec![restarted])),
        vec![Event::MemberUp(NodeId::new(1, 2, "n1"))]
    );
    assert_eq!(active_ids(&b), vec![1, 2]);
}

#[test]
fn std_newer_incarnation_replaces_the_running_instance() {
    let mut a = machine(1);
    teach(&mut a, 0, vec![alive(2)]);
    let restarted = NodeId::new(2, 2, "n2-new");
    let update = Update {
        node: restarted.clone(),
        ..alive(2)
    };
    assert_eq!(
        events(&teach(&mut a, 1, vec![update])),
        vec![Event::Down(n(2)), Event::MemberUp(restarted.clone())]
    );
    assert_eq!(a.member(2).unwrap().node, restarted);
    // Gossip about the old instance is ignored from now on.
    assert!(teach(&mut a, 2, vec![fact(2, MemberState::Dead)]).is_empty());
    assert_eq!(state_of(&a, 2), Some(MemberState::Alive));
}

// ---------------------------------------------------------------------------------------------
// Merge rules
// ---------------------------------------------------------------------------------------------

#[test]
fn std_merge_rules() {
    use MemberState::{Alive, Dead, Left, Suspect};
    let v = |version: u64, state: MemberState| Update {
        version,
        ..fact(2, state)
    };
    let up = Event::MemberUp(n(2));
    let suspect = Event::Suspect(n(2));
    let down = Event::Down(n(2));
    let left = Event::Left(n(2));
    // (facts applied in order, expected events of the last one, expected final state)
    let cases: Vec<(Vec<Update>, Vec<Event>, MemberState)> = vec![
        (vec![v(0, Alive)], vec![up.clone()], Alive),
        (
            vec![v(0, Suspect)],
            vec![up.clone(), suspect.clone()],
            Suspect,
        ),
        (vec![v(0, Dead)], vec![], Dead),
        (vec![v(0, Left)], vec![], Left),
        (vec![v(0, Alive), v(0, Alive)], vec![], Alive),
        (
            vec![v(0, Alive), v(0, Suspect)],
            vec![suspect.clone()],
            Suspect,
        ),
        (vec![v(0, Suspect), v(0, Suspect)], vec![], Suspect),
        // Alive does not override a suspicion of the same version…
        (vec![v(0, Suspect), v(0, Alive)], vec![], Suspect),
        // …only a refutation (higher version) does.
        (vec![v(0, Suspect), v(1, Alive)], vec![up.clone()], Alive),
        // An old suspicion does not hit a node that already refuted it.
        (vec![v(1, Alive), v(0, Suspect)], vec![], Alive),
        (vec![v(0, Alive), v(0, Dead)], vec![down.clone()], Dead),
        (vec![v(0, Suspect), v(0, Dead)], vec![down], Dead),
        (vec![v(0, Alive), v(0, Left)], vec![left], Left),
        // Terminal states are final for the incarnation, whatever the version.
        (vec![v(0, Dead), v(9, Alive)], vec![], Dead),
        (vec![v(0, Dead), v(9, Suspect)], vec![], Dead),
        (vec![v(0, Left), v(9, Alive)], vec![], Left),
        // "Left" outranks "dead" (it is the more precise fact); no second event.
        (vec![v(0, Dead), v(0, Left)], vec![], Left),
        (vec![v(0, Left), v(5, Dead)], vec![], Left),
    ];
    for (facts, expected_events, expected_state) in cases {
        let mut m = machine(1);
        let mut last = Vec::new();
        for (step, update) in facts.iter().enumerate() {
            last = teach(&mut m, u64::try_from(step).unwrap(), vec![update.clone()]);
        }
        assert_eq!(events(&last), expected_events, "facts {facts:?}");
        assert_eq!(state_of(&m, 2), Some(expected_state), "facts {facts:?}");
    }
}

#[test]
fn edge_weights_are_clamped_and_weight_changes_propagate() {
    let mut cfg = cfg();
    cfg.weight = 1_000;
    let m = Membership::new(cfg, n(1), 1);
    assert_eq!(m.active_nodes(), vec![(n(1), MAX_WEIGHT)]);

    let mut a = machine(1);
    teach(&mut a, 0, vec![alive(2)]);
    let heavier = Update {
        weight: 5,
        ..alive(2)
    };
    // Same instance, same version, new weight: applied silently (no membership event).
    assert!(events(&teach(&mut a, 1, vec![heavier])).is_empty());
    assert_eq!(a.member(2).unwrap().weight, 5);
    let absurd = Update {
        weight: u16::MAX,
        ..alive(3)
    };
    teach(&mut a, 2, vec![absurd]);
    assert_eq!(a.member(3).unwrap().weight, MAX_WEIGHT);
    // A weight of zero keeps the node a member that owns nothing.
    let observer = Update {
        weight: 0,
        ..alive(4)
    };
    teach(&mut a, 3, vec![observer]);
    assert!(active_ids(&a).contains(&4));
    assert!((0..200_u32).all(|i| a.placement().owner(&i.to_be_bytes()).unwrap().id != 4));
}

#[test]
fn edge_majority_counts_dead_but_not_departed_members() {
    use MemberState::{Dead, Left};
    let mut m = machine(1);
    teach(&mut m, 0, vec![alive(2), fact(3, Dead), fact(4, Dead)]);
    // 2 active (us and n2) against 2 dead: not a strict majority.
    assert!(!m.has_majority());
    teach(&mut m, 1, vec![alive(5)]);
    assert!(m.has_majority());
    teach(&mut m, 2, vec![fact(6, Left), fact(7, Left), fact(8, Left)]);
    assert!(m.has_majority());
    // A suspect still counts as active.
    teach(&mut m, 3, vec![fact(5, MemberState::Suspect)]);
    assert!(m.has_majority());
}

// ---------------------------------------------------------------------------------------------
// Dissemination
// ---------------------------------------------------------------------------------------------

#[test]
fn std_facts_are_piggybacked_a_bounded_number_of_times() {
    let mut m = machine(1);
    teach(&mut m, 0, vec![alive(2)]);
    // One member known: limit = retransmit_mult (2) x bit_length(2) (2) = 4 transmissions.
    let mut carried = 0;
    for seq in 0..10 {
        let out = deliver(
            &mut m,
            1,
            9,
            Message::Ping {
                seq,
                updates: vec![],
            },
        );
        let Message::Ack { updates, .. } = sends(&out).remove(0).1 else {
            panic!("expected an ack")
        };
        // Our own state is always first.
        assert_eq!(updates[0], alive(1));
        if updates.contains(&alive(2)) {
            carried += 1;
        }
    }
    assert_eq!(carried, 4);
}

#[test]
fn edge_piggyback_size_is_bounded_and_newest_facts_go_first() {
    let mut m = machine(1);
    teach(&mut m, 0, (2..=40).map(alive).collect());
    let out = deliver(
        &mut m,
        1,
        2,
        Message::Ping {
            seq: 1,
            updates: vec![],
        },
    );
    let Message::Ack { updates, .. } = sends(&out).remove(0).1 else {
        panic!("expected an ack")
    };
    // Self + at most `max_piggyback` queued facts.
    assert_eq!(updates.len(), 1 + 4);

    // A fresher fact about a node replaces the queued older one instead of piling up.
    let mut m = machine(1);
    teach(&mut m, 0, vec![alive(2)]);
    teach(&mut m, 1, vec![fact(2, MemberState::Suspect)]);
    let out = deliver(
        &mut m,
        2,
        9,
        Message::Ping {
            seq: 1,
            updates: vec![],
        },
    );
    let Message::Ack { updates, .. } = sends(&out).remove(0).1 else {
        panic!("expected an ack")
    };
    assert_eq!(updates, vec![alive(1), fact(2, MemberState::Suspect)]);
}

#[test]
fn edge_every_message_of_a_huge_view_fits_the_wire_limits() {
    let mut m = machine(1);
    let count = u64::try_from(MAX_UPDATES).unwrap() + 80;
    teach(&mut m, 0, (2..2 + count).map(alive).collect());
    assert_eq!(m.members().count(), MAX_UPDATES + 80);
    let mut synced = 0;
    for now in [0, 10_000, 20_000, 30_000] {
        for (_, msg) in sends(&tick(&mut m, now)) {
            assert_eq!(validate(&msg), Ok(()));
            assert!(encode(&msg).is_ok());
            if let Message::Sync { updates, .. } = &msg {
                assert_eq!(updates.len(), MAX_UPDATES);
                synced += 1;
            }
        }
    }
    // One sync per interval to an active member, plus one to a dead member once the
    // unanswered probes have produced some.
    assert!((4..=8).contains(&synced), "{synced} syncs");
    // A sync request is answered within the limit too.
    let out = deliver(
        &mut m,
        40_000,
        2,
        Message::Sync {
            want_reply: true,
            updates: vec![],
        },
    );
    let (_, reply) = sends(&out).remove(0);
    assert_eq!(validate(&reply), Ok(()));
}

#[test]
fn std_same_seed_gives_the_same_choices() {
    let run = |seed: u64| {
        let mut m = Membership::new(cfg(), n(1), seed);
        teach(&mut m, 0, (2..=9).map(alive).collect());
        let mut trace = Vec::new();
        for now in (0..20_000).step_by(250) {
            trace.extend(tick(&mut m, now));
        }
        trace
    };
    assert_eq!(run(7), run(7));
    assert_ne!(run(7), run(8));
}

// ---------------------------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------------------------

fn arb_fact() -> impl Strategy<Value = Update> {
    (2_u64..7, 1_u64..4, 0_u64..4, 0_u8..4, 0_u16..3, 0_u8..2).prop_map(
        |(id, incarnation, version, state, weight, addr)| Update {
            node: NodeId::new(id, incarnation, format!("n{id}-{addr}")),
            version,
            state: [
                MemberState::Alive,
                MemberState::Suspect,
                MemberState::Dead,
                MemberState::Left,
            ][usize::from(state)],
            weight,
        },
    )
}

fn view(m: &Membership) -> Vec<(NodeId, u64, MemberState, u16)> {
    m.members()
        .map(|member| {
            (
                member.node.clone(),
                member.version,
                member.state,
                member.weight,
            )
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    /// Convergence: the view depends only on the set of facts received, not on their order,
    /// their grouping into messages or how often they are repeated.
    #[test]
    fn prop_views_converge_whatever_the_delivery_order(
        facts in proptest::collection::vec(arb_fact(), 1..40),
        order in any::<u64>(),
    ) {
        let mut in_order = machine(1);
        for update in &facts {
            teach(&mut in_order, 0, vec![update.clone()]);
        }
        let mut shuffled = facts.clone();
        mt_cluster::rng::Rng::new(order).shuffle(&mut shuffled);
        shuffled.extend(facts.iter().take(5).cloned());
        let mut out_of_order = machine(1);
        for chunk in shuffled.chunks(3) {
            teach(&mut out_of_order, 0, chunk.to_vec());
        }
        prop_assert_eq!(view(&in_order), view(&out_of_order));
    }

    /// The events are a faithful change feed: replaying them reproduces the set of active
    /// members, so a consumer that only listens to events never drifts from the view.
    #[test]
    fn prop_events_reproduce_the_active_set(facts in proptest::collection::vec(arb_fact(), 1..60)) {
        let mut m = machine(1);
        let mut active: BTreeSet<NodeId> = BTreeSet::new();
        for (step, update) in facts.into_iter().enumerate() {
            for event in events(&teach(&mut m, u64::try_from(step).unwrap(), vec![update])) {
                match event {
                    Event::MemberUp(node) => {
                        active.insert(node);
                    }
                    Event::Down(node) | Event::Left(node) => {
                        prop_assert!(active.remove(&node), "removal of an unknown member");
                    }
                    Event::Suspect(node) => prop_assert!(active.contains(&node)),
                    Event::Renewed(_) => prop_assert!(false, "facts were not about this node"),
                }
            }
            let expected: BTreeSet<NodeId> = m
                .active_nodes()
                .into_iter()
                .map(|(node, _)| node)
                .filter(|node| node.id != 1)
                .collect();
            prop_assert_eq!(&active, &expected);
        }
    }

    /// A node never changes its mind about itself except by moving forward: its incarnation
    /// and (within an incarnation) its version never decrease, whatever it is told.
    #[test]
    fn prop_own_identity_only_moves_forward(
        claims in proptest::collection::vec((1_u64..5, 0_u64..5, 0_u8..4), 1..40),
    ) {
        let mut m = machine(1);
        let mut last = (1_u64, 0_u64);
        for (incarnation, version, state) in claims {
            let update = Update {
                node: NodeId::new(1, incarnation, "n1"),
                version,
                state: [
                    MemberState::Alive,
                    MemberState::Suspect,
                    MemberState::Dead,
                    MemberState::Left,
                ][usize::from(state)],
                weight: 1,
            };
            teach(&mut m, 0, vec![update.clone()]);
            let mine = self_update_of(&mut m);
            prop_assert_eq!(mine.state, MemberState::Alive);
            let now = (mine.node.incarnation, mine.version);
            prop_assert!(now >= last, "identity went backwards: {:?} -> {:?}", last, now);
            // Whatever was claimed about us is now strictly older than what we announce,
            // unless it was simply "alive" at our current version.
            if update.state != MemberState::Alive {
                prop_assert!(now > (update.node.incarnation, update.version));
            }
            last = now;
        }
    }
}

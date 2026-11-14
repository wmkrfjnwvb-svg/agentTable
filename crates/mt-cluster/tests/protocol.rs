//! Wire protocol tests: identities, stable hashing, the binary codec and the authentication
//! handshake. Categories are marked in the test names (`std_`, `edge_`, `invalid_`, `fuzz_`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use mt_cluster::auth::MIN_SECRET_BYTES;
use mt_cluster::codec::{validate, MAX_ADDR_BYTES, MAX_FRAME_BYTES, MAX_UPDATES, MAX_WEIGHT, WIRE_VERSION};
use mt_cluster::hash::{mix64, rendezvous_score, stable_hash64};
use mt_cluster::message::Hello;
use mt_cluster::{
    decode, encode, ClusterError, ClusterSecret, Handshake, MemberState, Message, NodeId, ProtocolVersion, Update,
    PROTOCOL_VERSION,
};
use proptest::prelude::*;

fn node(id: u64) -> NodeId {
    NodeId::new(id, 100 + id, format!("10.0.0.{id}:7000"))
}

fn update(id: u64, state: MemberState) -> Update {
    Update { node: node(id), version: id, state, weight: 1 }
}

fn secret() -> ClusterSecret {
    ClusterSecret::new(b"0123456789abcdef0123456789abcdef").unwrap()
}

fn other_secret() -> ClusterSecret {
    ClusterSecret::new(b"another-cluster-secret-000000000").unwrap()
}

fn every_message() -> Vec<Message> {
    let hello = Hello { protocol: PROTOCOL_VERSION, node: node(1), nonce: [7; 16], tag: [9; 32] };
    vec![
        Message::Hello(hello.clone()),
        Message::HelloAck(hello),
        Message::HelloConfirm { tag: [3; 32] },
        Message::Ping { seq: 1, updates: vec![] },
        Message::Ping { seq: u64::MAX, updates: vec![update(1, MemberState::Alive), update(2, MemberState::Suspect)] },
        Message::Ack { seq: 2, updates: vec![update(3, MemberState::Dead)] },
        Message::PingReq { seq: 3, target: node(4), updates: vec![update(5, MemberState::Left)] },
        Message::Sync { want_reply: true, updates: vec![update(6, MemberState::Alive)] },
        Message::Sync { want_reply: false, updates: vec![] },
    ]
}

// ---------------------------------------------------------------------------------------------
// Node identity and hashing
// ---------------------------------------------------------------------------------------------

#[test]
fn std_protocol_versions_are_compatible_within_a_major() {
    let v = |major, minor| ProtocolVersion { major, minor };
    assert!(v(1, 0).is_compatible(v(1, 7)));
    assert!(!v(1, 0).is_compatible(v(2, 0)));
    assert_eq!(v(1, 2).to_string(), "1.2");
    assert_eq!(PROTOCOL_VERSION, v(1, 0));
}

#[test]
fn std_node_ids_display_and_order_by_id_then_incarnation() {
    let a = NodeId::new(1, 5, "x");
    assert_eq!(a.to_string(), "1#5@x");
    let mut nodes = vec![NodeId::new(2, 1, "a"), NodeId::new(1, 9, "b"), NodeId::new(1, 2, "z")];
    nodes.sort();
    assert_eq!(nodes.iter().map(|n| (n.id, n.incarnation)).collect::<Vec<_>>(), vec![(1, 2), (1, 9), (2, 1)]);
}

#[test]
fn std_hash_golden_vectors_never_change() {
    // These values are part of the cluster contract: every node of every version must compute
    // them identically, or nodes would disagree about who owns what.
    assert_eq!(mix64(0), 0);
    assert_eq!(mix64(1), 0x5692_161d_100b_05e5);
    assert_eq!(stable_hash64(b""), 0xf52a_15e9_a9b5_e89b);
    assert_eq!(stable_hash64(b"a"), 0x02c0_bdbf_4814_20f8);
    assert_eq!(stable_hash64(b"bas_01j9z3k5q8abcdefghjkmnpqrs"), 0xbdfa_22a6_39a9_9d51);
    assert_eq!(rendezvous_score(stable_hash64(b"a"), 1, 0), 0xff21_dc13_4a13_a201);
    assert_eq!(rendezvous_score(stable_hash64(b"a"), 2, 3), 0x7631_c78c_d57b_5c81);
}

#[test]
fn std_member_state_activity() {
    assert!(MemberState::Alive.is_active() && MemberState::Suspect.is_active());
    assert!(!MemberState::Dead.is_active() && !MemberState::Left.is_active());
}

// ---------------------------------------------------------------------------------------------
// Codec
// ---------------------------------------------------------------------------------------------

#[test]
fn std_every_message_round_trips() {
    for message in every_message() {
        let frame = encode(&message).unwrap();
        assert_eq!(frame[0], WIRE_VERSION);
        assert_eq!(decode(&frame).unwrap(), message);
    }
}

#[test]
fn std_wire_format_golden_frame() {
    // Changing the layout of a message breaks rolling upgrades; this pins it.
    let message = Message::Ping {
        seq: 7,
        updates: vec![Update { node: NodeId::new(1, 2, "n1"), version: 3, state: MemberState::Suspect, weight: 4 }],
    };
    assert_eq!(encode(&message).unwrap(), vec![1, 3, 7, 1, 1, 2, 2, 110, 49, 3, 1, 4]);
    assert_eq!(encode(&Message::HelloConfirm { tag: [9; 32] }).unwrap().len(), 34);
}

fn raw_frame(message: &Message) -> Vec<u8> {
    postcard::to_extend(message, vec![WIRE_VERSION]).unwrap()
}

#[test]
fn invalid_structural_limits_are_enforced_on_encode_and_decode() {
    let long_addr = "x".repeat(MAX_ADDR_BYTES + 1);
    let heavy = Update { weight: MAX_WEIGHT + 1, ..update(1, MemberState::Alive) };
    let cases: Vec<(Message, &str)> = vec![
        (Message::Ping { seq: 1, updates: vec![update(1, MemberState::Alive); MAX_UPDATES + 1] }, "too many updates"),
        (Message::Ack { seq: 1, updates: vec![heavy] }, "weight too large"),
        (Message::Sync { want_reply: false, updates: vec![Update { node: NodeId::new(1, 1, ""), ..update(1, MemberState::Alive) }] }, "empty node address"),
        (Message::PingReq { seq: 1, target: NodeId::new(1, 1, long_addr.clone()), updates: vec![] }, "node address too long"),
        (
            Message::Hello(Hello { protocol: PROTOCOL_VERSION, node: NodeId::new(1, 1, long_addr), nonce: [0; 16], tag: [0; 32] }),
            "node address too long",
        ),
    ];
    for (message, reason) in cases {
        assert_eq!(validate(&message), Err(ClusterError::LimitExceeded(reason)));
        assert_eq!(encode(&message), Err(ClusterError::LimitExceeded(reason)));
        // A peer that skips the checks cannot get the message past the decoder either.
        assert_eq!(decode(&raw_frame(&message)), Err(ClusterError::LimitExceeded(reason)));
    }
}

#[test]
fn edge_limits_are_inclusive() {
    let at_limit = Message::Sync {
        want_reply: false,
        updates: vec![
            Update { node: NodeId::new(1, 1, "x".repeat(MAX_ADDR_BYTES)), version: 0, state: MemberState::Alive, weight: MAX_WEIGHT };
            MAX_UPDATES
        ],
    };
    let frame = encode(&at_limit).unwrap();
    assert!(frame.len() <= MAX_FRAME_BYTES);
    assert_eq!(decode(&frame).unwrap(), at_limit);
}

#[test]
fn invalid_frames_are_rejected_with_the_right_error() {
    assert_eq!(decode(&[]), Err(ClusterError::EmptyFrame));
    assert_eq!(decode(&[2, 0]), Err(ClusterError::UnsupportedWireVersion(2)));
    assert_eq!(decode(&[0]), Err(ClusterError::UnsupportedWireVersion(0)));
    assert!(matches!(decode(&[WIRE_VERSION]), Err(ClusterError::Malformed(_))));
    assert!(matches!(decode(&[WIRE_VERSION, 200]), Err(ClusterError::Malformed(_))));
    let mut frame = encode(&Message::Ping { seq: 1, updates: vec![] }).unwrap();
    frame.push(0);
    assert_eq!(decode(&frame), Err(ClusterError::TrailingBytes));
    let oversized = vec![WIRE_VERSION; MAX_FRAME_BYTES + 1];
    assert_eq!(decode(&oversized), Err(ClusterError::FrameTooLarge { len: MAX_FRAME_BYTES + 1, max: MAX_FRAME_BYTES }));
}

#[test]
fn edge_truncated_frames_never_decode() {
    for message in every_message() {
        let frame = encode(&message).unwrap();
        for cut in 0..frame.len() {
            assert!(decode(&frame[..cut]).is_err(), "prefix of length {cut} decoded");
        }
    }
}

fn arb_update() -> impl Strategy<Value = Update> {
    (any::<u64>(), any::<u64>(), "[a-z0-9.:]{1,20}", any::<u64>(), 0_u8..4, 0..=MAX_WEIGHT).prop_map(
        |(id, incarnation, addr, version, state, weight)| Update {
            node: NodeId::new(id, incarnation, addr),
            version,
            state: [MemberState::Alive, MemberState::Suspect, MemberState::Dead, MemberState::Left][usize::from(state)],
            weight,
        },
    )
}

fn arb_message() -> impl Strategy<Value = Message> {
    let updates = || proptest::collection::vec(arb_update(), 0..8);
    prop_oneof![
        (any::<u64>(), updates()).prop_map(|(seq, updates)| Message::Ping { seq, updates }),
        (any::<u64>(), updates()).prop_map(|(seq, updates)| Message::Ack { seq, updates }),
        (any::<u64>(), arb_update(), updates()).prop_map(|(seq, target, updates)| Message::PingReq { seq, target: target.node, updates }),
        (any::<bool>(), updates()).prop_map(|(want_reply, updates)| Message::Sync { want_reply, updates }),
        any::<[u8; 32]>().prop_map(|tag| Message::HelloConfirm { tag }),
        (arb_update(), any::<[u8; 16]>(), any::<[u8; 32]>(), any::<u16>(), any::<u16>()).prop_map(|(u, nonce, tag, major, minor)| {
            Message::Hello(Hello { protocol: ProtocolVersion { major, minor }, node: u.node, nonce, tag })
        }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn prop_codec_round_trips(message in arb_message()) {
        let frame = encode(&message).unwrap();
        prop_assert_eq!(decode(&frame).unwrap(), message);
    }

    #[test]
    fn fuzz_decoder_never_panics_on_random_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..300)) {
        let _ = decode(&bytes);
    }

    #[test]
    fn fuzz_decoder_never_panics_on_corrupted_frames(message in arb_message(), flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..6)) {
        let mut frame = encode(&message).unwrap();
        for (position, byte) in flips {
            let index = position % frame.len();
            frame[index] ^= byte;
        }
        // Whatever comes out is either an error or a message that passes every limit.
        if let Ok(decoded) = decode(&frame) {
            prop_assert!(validate(&decoded).is_ok());
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Authentication handshake
// ---------------------------------------------------------------------------------------------

/// Runs a full handshake through the codec; returns both sides.
fn handshake(initiator_secret: ClusterSecret, responder_secret: ClusterSecret) -> Result<(Handshake, Handshake), ClusterError> {
    let (mut initiator, hello) = Handshake::initiator(initiator_secret, node(1), [1; 16])?;
    let mut responder = Handshake::responder(responder_secret, node(2), [2; 16]);
    let hello = decode(&encode(&hello)?)?;
    let ack = responder.handle(&hello)?.ok_or(ClusterError::UnexpectedHandshake)?;
    let ack = decode(&encode(&ack)?)?;
    let confirm = initiator.handle(&ack)?.ok_or(ClusterError::UnexpectedHandshake)?;
    let confirm = decode(&encode(&confirm)?)?;
    assert_eq!(responder.handle(&confirm)?, None);
    Ok((initiator, responder))
}

#[test]
fn std_handshake_authenticates_both_sides() {
    let (initiator, responder) = handshake(secret(), secret()).unwrap();
    assert_eq!(initiator.peer(), Some(&node(2)));
    assert_eq!(responder.peer(), Some(&node(1)));
}

#[test]
fn std_peer_is_unknown_until_the_handshake_completes() {
    let (mut initiator, hello) = Handshake::initiator(secret(), node(1), [1; 16]).unwrap();
    let mut responder = Handshake::responder(secret(), node(2), [2; 16]);
    assert_eq!(initiator.peer(), None);
    assert_eq!(responder.peer(), None);
    let ack = responder.handle(&hello).unwrap().unwrap();
    // The responder has only seen a (replayable) Hello: not authenticated yet.
    assert_eq!(responder.peer(), None);
    let confirm = initiator.handle(&ack).unwrap().unwrap();
    assert_eq!(initiator.peer(), Some(&node(2)));
    responder.handle(&confirm).unwrap();
    assert_eq!(responder.peer(), Some(&node(1)));
}

#[test]
fn invalid_wrong_secret_fails_on_either_side() {
    assert_eq!(handshake(secret(), other_secret()).err(), Some(ClusterError::AuthFailed));
    // The initiator also rejects an ack made with another secret.
    let (mut initiator, hello) = Handshake::initiator(secret(), node(1), [1; 16]).unwrap();
    let (_, forged_hello) = Handshake::initiator(other_secret(), node(1), [1; 16]).unwrap();
    let mut impostor = Handshake::responder(other_secret(), node(2), [2; 16]);
    let forged_ack = impostor.handle(&forged_hello).unwrap().unwrap();
    assert_eq!(initiator.handle(&forged_ack), Err(ClusterError::AuthFailed));
    assert_eq!(initiator.peer(), None);
    let _ = hello;
}

#[test]
fn invalid_tampered_hello_fields_fail_authentication() {
    let (_, hello) = Handshake::initiator(secret(), node(1), [1; 16]).unwrap();
    let Message::Hello(original) = hello else { panic!("expected Hello") };
    let tamper: Vec<Box<dyn Fn(&mut Hello)>> = vec![
        Box::new(|h| h.tag[0] ^= 1),
        Box::new(|h| h.nonce[15] ^= 1),
        Box::new(|h| h.node.id = 3),
        Box::new(|h| h.node.incarnation += 1),
        Box::new(|h| h.node.addr.push('x')),
        Box::new(|h| h.protocol.minor += 1),
    ];
    for edit in tamper {
        let mut hello = original.clone();
        edit(&mut hello);
        let mut responder = Handshake::responder(secret(), node(2), [2; 16]);
        assert_eq!(responder.handle(&Message::Hello(hello)), Err(ClusterError::AuthFailed));
    }
}

#[test]
fn invalid_replayed_messages_do_not_authenticate() {
    // Record a complete, honest session.
    let (mut initiator, hello) = Handshake::initiator(secret(), node(1), [1; 16]).unwrap();
    let mut responder = Handshake::responder(secret(), node(2), [2; 16]);
    let ack = responder.handle(&hello).unwrap().unwrap();
    let confirm = initiator.handle(&ack).unwrap().unwrap();

    // An attacker replays the Hello and the Confirm to a responder with a fresh nonce.
    let mut fresh = Handshake::responder(secret(), node(2), [42; 16]);
    assert!(fresh.handle(&hello).unwrap().is_some());
    assert_eq!(fresh.handle(&confirm), Err(ClusterError::AuthFailed));
    assert_eq!(fresh.peer(), None);

    // An attacker replays the Ack to an initiator with a fresh nonce.
    let (mut fresh_initiator, _) = Handshake::initiator(secret(), node(1), [43; 16]).unwrap();
    assert_eq!(fresh_initiator.handle(&ack), Err(ClusterError::AuthFailed));
}

#[test]
fn invalid_out_of_order_messages_fail_and_the_handshake_stays_failed() {
    let (mut initiator, hello) = Handshake::initiator(secret(), node(1), [1; 16]).unwrap();
    assert_eq!(initiator.handle(&hello), Err(ClusterError::UnexpectedHandshake));
    // After any error the state machine is dead: even a valid ack is refused.
    let mut responder = Handshake::responder(secret(), node(2), [2; 16]);
    let ack = responder.handle(&hello).unwrap().unwrap();
    assert_eq!(initiator.handle(&ack), Err(ClusterError::UnexpectedHandshake));

    let mut responder = Handshake::responder(secret(), node(2), [2; 16]);
    assert_eq!(responder.handle(&Message::Ping { seq: 1, updates: vec![] }), Err(ClusterError::UnexpectedHandshake));
    let mut responder = Handshake::responder(secret(), node(2), [2; 16]);
    responder.handle(&hello).unwrap();
    assert_eq!(responder.handle(&hello), Err(ClusterError::UnexpectedHandshake));

    let (mut done_initiator, mut done_responder) = handshake(secret(), secret()).unwrap();
    assert_eq!(done_initiator.handle(&ack), Err(ClusterError::UnexpectedHandshake));
    assert_eq!(done_responder.handle(&hello), Err(ClusterError::UnexpectedHandshake));
    assert_eq!(done_responder.peer(), None);
}

#[test]
fn invalid_incompatible_protocol_and_self_connection() {
    let foreign = Hello { protocol: ProtocolVersion { major: 2, minor: 0 }, node: node(1), nonce: [1; 16], tag: [0; 32] };
    let mut responder = Handshake::responder(secret(), node(2), [2; 16]);
    assert_eq!(
        responder.handle(&Message::Hello(foreign)),
        Err(ClusterError::IncompatibleProtocol { ours: PROTOCOL_VERSION, theirs: ProtocolVersion { major: 2, minor: 0 } })
    );
    // A node that reaches itself (same stable id) refuses the connection.
    let (_, hello) = Handshake::initiator(secret(), node(2), [1; 16]).unwrap();
    let mut responder = Handshake::responder(secret(), node(2), [2; 16]);
    assert_eq!(responder.handle(&hello), Err(ClusterError::SelfConnection));
    let (mut initiator, _) = Handshake::initiator(secret(), node(1), [1; 16]).unwrap();
    let mut twin = Handshake::responder(secret(), node(1), [2; 16]);
    let (_, hello_from_other) = Handshake::initiator(secret(), node(3), [1; 16]).unwrap();
    let ack_from_twin = twin.handle(&hello_from_other).unwrap().unwrap();
    assert_eq!(initiator.handle(&ack_from_twin), Err(ClusterError::SelfConnection));
}

#[test]
fn invalid_short_secrets_are_refused_and_secrets_never_print() {
    assert_eq!(ClusterSecret::new(&[1; MIN_SECRET_BYTES - 1]).err(), Some(ClusterError::SecretTooShort));
    assert_eq!(ClusterSecret::new(b"").err(), Some(ClusterError::SecretTooShort));
    let secret = ClusterSecret::new(&[b'k'; MIN_SECRET_BYTES]).unwrap();
    let printed = format!("{secret:?}");
    assert_eq!(printed, "ClusterSecret(<redacted>)");
    assert!(!printed.contains('k'));
    let (handshake, _) = Handshake::initiator(secret, node(1), [1; 16]).unwrap();
    assert!(!format!("{handshake:?}").contains("kkkk"));
}

#[test]
fn std_errors_have_messages_and_fenced_is_detectable() {
    assert!(ClusterError::Fenced { current: 4 }.is_fenced());
    assert!(!ClusterError::Contended.is_fenced());
    let errors = [
        ClusterError::FrameTooLarge { len: 2, max: 1 },
        ClusterError::EmptyFrame,
        ClusterError::UnsupportedWireVersion(9),
        ClusterError::Malformed("x".into()),
        ClusterError::TrailingBytes,
        ClusterError::LimitExceeded("y"),
        ClusterError::IncompatibleProtocol { ours: PROTOCOL_VERSION, theirs: PROTOCOL_VERSION },
        ClusterError::AuthFailed,
        ClusterError::UnexpectedHandshake,
        ClusterError::SelfConnection,
        ClusterError::SecretTooShort,
        ClusterError::StoreUnavailable("z".into()),
        ClusterError::Contended,
        ClusterError::Fenced { current: 1 },
        ClusterError::EpochOverflow,
        ClusterError::Draining,
        ClusterError::LeaseHeld { until_ms: 5 },
        ClusterError::LeaseExpired,
        ClusterError::NotHeld,
    ];
    for error in errors {
        assert!(!error.to_string().is_empty());
    }
}

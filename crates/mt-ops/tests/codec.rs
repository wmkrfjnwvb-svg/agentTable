//! The binary codec: round trips, limits, and arbitrary input.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_core::value::Value;
use mt_ops::codec::{
    decode_client, decode_server, encode_client, encode_server, MAX_FRAME_BYTES, WIRE_VERSION,
};
use mt_ops::{
    Accepted, ClientMessage, Effect, Op, OpsError, Outcome, Reason, Rejection, ServerMessage,
    MAX_CELLS, MAX_OPS,
};
use proptest::prelude::*;
use std::collections::BTreeMap;

fn op_with_values() -> impl Strategy<Value = Op> {
    let cells = proptest::collection::btree_map(1_u128..20, any_value(), 0..4).prop_map(|m| {
        m.into_iter()
            .map(|(f, v)| (field(f), v))
            .collect::<BTreeMap<_, _>>()
    });
    prop_oneof![
        (any::<u128>(), any::<u128>(), cells).prop_map(|(t, r, cells)| Op::Create {
            table: table(t),
            record: record(r),
            cells
        }),
        (
            any::<u128>(),
            any::<u128>(),
            any::<u128>(),
            any_value(),
            any::<u64>()
        )
            .prop_map(|(t, r, f, v, e)| set(t, r, f, v, e)),
        (
            any::<u128>(),
            any::<u128>(),
            any::<u128>(),
            any_value(),
            any::<u64>()
        )
            .prop_map(|(t, r, f, v, e)| transition(t, r, f, v, e)),
        (any::<u128>(), any::<u128>(), any::<u64>()).prop_map(|(t, r, e)| delete(t, r, e)),
        (any::<u128>(), any::<u128>()).prop_map(|(t, r)| restore(t, r)),
    ]
}

fn effect() -> impl Strategy<Value = Effect> {
    prop_oneof![
        (
            any::<u128>(),
            any::<u128>(),
            proptest::collection::btree_map(1_u128..20, any_value(), 0..3)
        )
            .prop_map(|(t, r, cells)| Effect::Created {
                table: table(t),
                record: record(r),
                cells: cells.into_iter().map(|(f, v)| (field(f), v)).collect()
            }),
        (
            any::<u128>(),
            any::<u128>(),
            any::<u128>(),
            any_value(),
            any_value(),
            proptest::option::of(any_value())
        )
            .prop_map(|(t, r, f, before, after, overwritten)| Effect::Set {
                table: table(t),
                record: record(r),
                field: field(f),
                before,
                after,
                overwritten
            }),
        (any::<u128>(), any::<u128>()).prop_map(|(t, r)| Effect::Deleted {
            table: table(t),
            record: record(r)
        }),
        (any::<u128>(), any::<u128>()).prop_map(|(t, r)| Effect::Restored {
            table: table(t),
            record: record(r)
        }),
    ]
}

fn reason() -> impl Strategy<Value = Reason> {
    prop_oneof![
        Just(Reason::WrongBase),
        Just(Reason::EmptyBatch),
        Just(Reason::TooManyOps),
        Just(Reason::TooManyCells),
        any::<u64>().prop_map(|newest| Reason::TooOld { newest }),
        Just(Reason::RecordExists),
        Just(Reason::UnknownRecord),
        Just(Reason::RecordDeleted),
        Just(Reason::NotDeleted),
        (any::<u64>(), any::<u64>())
            .prop_map(|(expected, actual)| Reason::StaleRecord { expected, actual }),
        Just(Reason::SequenceExhausted),
    ]
}

fn client_message() -> impl Strategy<Value = ClientMessage> {
    prop_oneof![
        (
            any::<u64>(),
            any::<u64>(),
            proptest::collection::vec(op_with_values(), 0..6)
        )
            .prop_map(|(c, s, ops)| ClientMessage::Submit(batch(c, s, ops))),
        any::<u64>().prop_map(|after| ClientMessage::CatchUp { after }),
    ]
}

fn server_message() -> impl Strategy<Value = ServerMessage> {
    prop_oneof![
        (
            any::<u64>(),
            any::<u64>(),
            any::<u64>(),
            proptest::collection::vec(effect(), 0..6)
        )
            .prop_map(
                |(c, s, seq, effects)| ServerMessage::Outcome(Outcome::Accepted(Accepted {
                    id: id(c, s),
                    seq,
                    effects
                }))
            ),
        (
            any::<u64>(),
            any::<u64>(),
            proptest::option::of(any::<usize>()),
            reason()
        )
            .prop_map(
                |(c, s, op, reason)| ServerMessage::Outcome(Outcome::Rejected(Rejection {
                    id: id(c, s),
                    op,
                    reason
                }))
            ),
        any::<u64>().prop_map(|oldest| ServerMessage::Reload { oldest }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    /// Every message comes back exactly, in binary and in JSON.
    #[test]
    fn prop_client_messages_round_trip(message in client_message()) {
        let bytes = encode_client(&message).unwrap();
        prop_assert_eq!(bytes[0], WIRE_VERSION);
        prop_assert_eq!(decode_client(&bytes).unwrap(), message.clone());
        let json = serde_json::to_string(&message).unwrap();
        prop_assert_eq!(serde_json::from_str::<ClientMessage>(&json).unwrap(), message);
    }

    #[test]
    fn prop_server_messages_round_trip(message in server_message()) {
        let bytes = encode_server(&message).unwrap();
        prop_assert_eq!(decode_server(&bytes).unwrap(), message.clone());
        let json = serde_json::to_string(&message).unwrap();
        prop_assert_eq!(serde_json::from_str::<ServerMessage>(&json).unwrap(), message);
    }

    /// Arbitrary bytes never panic the decoder.
    #[test]
    fn prop_any_bytes_decode_or_fail_cleanly(bytes in proptest::collection::vec(any::<u8>(), 0..300)) {
        let _ = decode_client(&bytes);
        let _ = decode_server(&bytes);
        let mut framed = vec![WIRE_VERSION];
        framed.extend(&bytes);
        let _ = decode_client(&framed);
        let _ = decode_server(&framed);
    }

    /// A message cut short or extended is refused.
    #[test]
    fn prop_truncated_or_extended_frames_are_refused(message in client_message(), cut in 1_usize..8, extra in any::<u8>()) {
        let bytes = encode_client(&message).unwrap();
        if bytes.len() > cut {
            prop_assert!(decode_client(&bytes[..bytes.len() - cut]).is_err());
        }
        let mut longer = bytes;
        longer.push(extra);
        prop_assert!(decode_client(&longer).is_err());
    }
}

#[test]
fn invalid_frames_are_refused_with_the_reason() {
    assert!(matches!(decode_client(&[]), Err(OpsError::Malformed(_))));
    assert_eq!(decode_client(&[9, 0]), Err(OpsError::UnknownVersion(9)));
    assert_eq!(decode_server(&[0]), Err(OpsError::UnknownVersion(0)));
    let big = vec![WIRE_VERSION; MAX_FRAME_BYTES + 1];
    assert!(matches!(
        decode_client(&big),
        Err(OpsError::TooLarge { .. })
    ));
    assert!(matches!(
        decode_server(&big),
        Err(OpsError::TooLarge { .. })
    ));
    let ok = encode_client(&ClientMessage::CatchUp { after: 3 }).unwrap();
    let mut trailing = ok.clone();
    trailing.push(0);
    assert_eq!(
        decode_client(&trailing),
        Err(OpsError::Malformed("trailing bytes".to_owned()))
    );
    // Too many operations or cells: neither encoded nor accepted when decoded.
    let many = ClientMessage::Submit(batch(
        1,
        1,
        (0..=MAX_OPS as u128).map(|n| restore(1, n)).collect(),
    ));
    assert_eq!(
        encode_client(&many),
        Err(OpsError::Limit("too many operations"))
    );
    let forged = postcard::to_extend(&many, vec![WIRE_VERSION]).unwrap();
    assert_eq!(
        decode_client(&forged),
        Err(OpsError::Limit("too many operations"))
    );
    let wide: Vec<(u128, Value)> = (0..=MAX_CELLS as u128).map(|n| (n, text("x"))).collect();
    let wide_create = ClientMessage::Submit(batch(1, 1, vec![create(1, 1, &wide)]));
    assert_eq!(
        encode_client(&wide_create),
        Err(OpsError::Limit("too many cells"))
    );
    let wide_effect = ServerMessage::Outcome(Outcome::Accepted(Accepted {
        id: id(1, 1),
        seq: 1,
        effects: vec![Effect::Created {
            table: table(1),
            record: record(1),
            cells: wide.iter().map(|(f, v)| (field(*f), v.clone())).collect(),
        }],
    }));
    assert_eq!(
        encode_server(&wide_effect),
        Err(OpsError::Limit("too many cells"))
    );
    let forged = postcard::to_extend(&wide_effect, vec![WIRE_VERSION]).unwrap();
    assert_eq!(
        decode_server(&forged),
        Err(OpsError::Limit("too many cells"))
    );
    // A frame too large to send is refused when encoding.
    let huge = ClientMessage::Submit(batch(
        1,
        1,
        vec![set(
            1,
            1,
            1,
            Value::LongText("x".repeat(MAX_FRAME_BYTES)),
            0,
        )],
    ));
    assert!(matches!(
        encode_client(&huge),
        Err(OpsError::TooLarge { .. })
    ));
    let huge_out = ServerMessage::Outcome(Outcome::Accepted(Accepted {
        id: id(1, 1),
        seq: 1,
        effects: vec![
            Effect::Restored {
                table: table(1),
                record: record(1)
            };
            100_000
        ],
    }));
    assert!(matches!(
        encode_server(&huge_out),
        Err(OpsError::TooLarge { .. })
    ));
    // A value whose text is not JSON is refused.
    let mut bad = vec![WIRE_VERSION];
    bad.extend(postcard::to_allocvec(&(0_u32, (1_u64, 1_u64), "bas", 1_u8, 1_u32)).unwrap());
    assert!(decode_client(&bad).is_err());
    // Every error reads as a sentence.
    for e in [
        OpsError::Malformed("x".into()),
        OpsError::TooLarge { bytes: 2, max: 1 },
        OpsError::UnknownVersion(3),
        OpsError::Limit("x"),
        OpsError::Gap {
            expected: 1,
            got: 3,
        },
        OpsError::Replay {
            seq: 1,
            reason: "x".into(),
        },
    ] {
        assert!(!e.to_string().is_empty());
    }
}

#[test]
fn std_the_json_form_reads_well() {
    let message = ClientMessage::Submit(batch(4, 2, vec![set(1, 2, 3, text("hello"), 7)]));
    let json = serde_json::to_value(&message).unwrap();
    let set = &json["submit"]["ops"][0]["set"];
    assert_eq!(
        set["value"],
        serde_json::json!({"type": "text", "value": "hello"})
    );
    assert_eq!(set["expected"], 7);
    assert_eq!(
        json["submit"]["id"],
        serde_json::json!({"client": 4, "seq": 2})
    );
}

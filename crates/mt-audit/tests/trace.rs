//! Trace context tests: standard, edge, invalid, property.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use mt_audit::{AuditError, SpanId, TraceContext, TraceFlags, TraceId};
use proptest::prelude::*;

const SAMPLE: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

// ---- standard ----

#[test]
fn standard_parses_w3c_example() {
    let ctx = TraceContext::parse(SAMPLE).unwrap();
    assert_eq!(ctx.trace_id.to_string(), "4bf92f3577b34da6a3ce929d0e0e4736");
    assert_eq!(ctx.span_id.to_string(), "00f067aa0ba902b7");
    assert!(ctx.flags.is_sampled());
    assert_eq!(ctx.to_string(), SAMPLE);
    assert_eq!(SAMPLE.parse::<TraceContext>().unwrap(), ctx);
}

#[test]
fn standard_child_keeps_trace_and_flags() {
    let ctx = TraceContext::parse(SAMPLE).unwrap();
    let new_span = SpanId::from_bytes([1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
    let child = ctx.child(new_span).unwrap();
    assert_eq!(child.parent_span_id, ctx.span_id);
    assert_eq!(child.context.trace_id, ctx.trace_id);
    assert_eq!(child.context.span_id, new_span);
    assert_eq!(child.context.flags, ctx.flags);
    assert_eq!(
        child.context.to_string(),
        "00-4bf92f3577b34da6a3ce929d0e0e4736-0102030405060708-01"
    );
}

#[test]
fn standard_new_and_accessors() {
    let t = TraceId::from_bytes([9; 16]).unwrap();
    let s = SpanId::from_bytes([7; 8]).unwrap();
    let ctx = TraceContext::new(t, s, TraceFlags(0));
    assert!(!ctx.flags.is_sampled());
    assert_eq!(t.as_bytes(), &[9; 16]);
    assert_eq!(s.as_bytes(), &[7; 8]);
    assert_eq!(
        ctx.to_string(),
        format!("00-{}-{}-00", "09".repeat(16), "07".repeat(8))
    );
}

#[test]
fn standard_serde_round_trip() {
    let ctx = TraceContext::parse(SAMPLE).unwrap();
    let text = serde_json::to_string(&ctx).unwrap();
    assert_eq!(text, format!("\"{SAMPLE}\""));
    assert_eq!(serde_json::from_str::<TraceContext>(&text).unwrap(), ctx);
    let id: TraceId = serde_json::from_str("\"4bf92f3577b34da6a3ce929d0e0e4736\"").unwrap();
    assert_eq!(id, ctx.trace_id);
    let span: SpanId = serde_json::from_str("\"00f067aa0ba902b7\"").unwrap();
    assert_eq!(
        serde_json::to_string(&span).unwrap(),
        "\"00f067aa0ba902b7\""
    );
    assert!(serde_json::from_str::<TraceId>("\"zz\"").is_err());
    assert!(serde_json::from_str::<TraceId>("5").is_err());
}

// ---- edge ----

#[test]
fn edge_unknown_flag_bits_are_preserved() {
    let ctx =
        TraceContext::parse("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-fe").unwrap();
    assert_eq!(ctx.flags, TraceFlags(0xfe));
    assert!(!ctx.flags.is_sampled());
    assert!(ctx.to_string().ends_with("-fe"));
}

#[test]
fn edge_future_version_is_accepted_and_downgraded() {
    let exact = "01-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
    let extended = "cc-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-what-the-future-holds";
    assert_eq!(TraceContext::parse(exact).unwrap().to_string(), SAMPLE);
    assert_eq!(TraceContext::parse(extended).unwrap().to_string(), SAMPLE);
}

// ---- invalid ----

#[test]
fn invalid_headers_are_rejected() {
    let cases = [
        "",
        "00",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-0",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-011",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-x",
        "01-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01x",
        "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        "0g-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
        "00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00F067AA0BA902B7-01",
        "00-4bf92f3577b34da6a3ce929d0e0e473x-00f067aa0ba902b7-01",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-0G",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-zz",
        "00_4bf92f3577b34da6a3ce929d0e0e4736_00f067aa0ba902b7_01",
        "00-4bf92f3577b34da6a3ce929d0e0e47-36-00f067aa0ba902b7-01",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00-f067aa0ba902b-01",
        " 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01 ",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-0é",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902é-01",
    ];
    for case in cases {
        let result = TraceContext::parse(case);
        assert!(
            matches!(result, Err(AuditError::InvalidTraceparent(_))),
            "accepted {case:?}"
        );
    }
}

#[test]
fn invalid_ids() {
    assert!(TraceId::from_bytes([0; 16]).is_err());
    assert!(SpanId::from_bytes([0; 8]).is_err());
    assert!("abc".parse::<TraceId>().is_err());
    assert!("abc".parse::<SpanId>().is_err());
    let ctx = TraceContext::parse(SAMPLE).unwrap();
    assert!(ctx.child(ctx.span_id).is_err());
}

// ---- property ----

fn trace_id() -> impl Strategy<Value = TraceId> {
    any::<[u8; 16]>().prop_filter_map("zero", |b| TraceId::from_bytes(b).ok())
}

fn span_id() -> impl Strategy<Value = SpanId> {
    any::<[u8; 8]>().prop_filter_map("zero", |b| SpanId::from_bytes(b).ok())
}

proptest! {
    #[test]
    fn prop_format_parse_round_trip(t in trace_id(), s in span_id(), f in any::<u8>()) {
        let ctx = TraceContext::new(t, s, TraceFlags(f));
        let text = ctx.to_string();
        prop_assert_eq!(text.len(), 55);
        prop_assert_eq!(TraceContext::parse(&text).unwrap(), ctx);
    }

    #[test]
    fn prop_parse_never_panics_and_accepted_input_is_canonical(s in "\\PC{0,70}") {
        if let Ok(ctx) = TraceContext::parse(&s) {
            // Anything accepted re-formats to its own first 55 characters, with version 00.
            prop_assert_eq!(&ctx.to_string()[2..], &s[2..55]);
        }
    }

    #[test]
    fn prop_single_character_corruption_is_rejected_or_changes_meaning(
        t in trace_id(), s in span_id(), f in any::<u8>(), pos in 0usize..55, c in any::<char>()
    ) {
        let ctx = TraceContext::new(t, s, TraceFlags(f));
        let text = ctx.to_string();
        let original = text.chars().nth(pos).unwrap();
        prop_assume!(c != original);
        let mutated: String =
            text.chars().enumerate().map(|(i, ch)| if i == pos { c } else { ch }).collect();
        match TraceContext::parse(&mutated) {
            Err(_) => {}
            // Only a version change (positions 0-1) may leave the meaning intact.
            Ok(parsed) => prop_assert!(parsed != ctx || pos < 2),
        }
    }

    #[test]
    fn prop_child_chain_stays_in_trace(t in trace_id(), spans in proptest::collection::vec(span_id(), 1..20)) {
        let mut ctx = TraceContext::new(t, spans[0], TraceFlags(1));
        for span in &spans[1..] {
            match ctx.child(*span) {
                Ok(child) => {
                    prop_assert_eq!(child.parent_span_id, ctx.span_id);
                    prop_assert_eq!(child.context.trace_id, t);
                    ctx = child.context;
                }
                Err(_) => prop_assert_eq!(*span, ctx.span_id),
            }
        }
    }
}

//! W3C Trace Context (`traceparent`) — strict parsing, formatting and child-span derivation.
//!
//! Format (version `00`): `00-<32 hex trace id>-<16 hex parent/span id>-<2 hex flags>`.
//!
//! Validation rules (W3C Recommendation, 2020-02-06):
//! - only lowercase hex is accepted; no surrounding whitespace;
//! - version `ff` is invalid;
//! - an all-zero trace id or span id is invalid;
//! - version `00` must be exactly 55 characters;
//! - a higher version is accepted only if the first 55 characters are well formed and either the header
//!   ends there or the next character is `-` (forward compatibility); the extra data is ignored and the
//!   context is re-emitted as version `00`.
//!
//! No randomness lives here: new span ids are supplied by the caller.

use core::fmt;
use core::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::AuditError;
use crate::hex;

/// Length of a version-00 `traceparent` header.
const TRACEPARENT_LEN: usize = 55;

/// 16-byte trace identifier (one per user action). Never all zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TraceId([u8; 16]);

/// 8-byte span identifier. Never all zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpanId([u8; 8]);

impl TraceId {
    /// Builds a trace id from raw bytes; rejects the all-zero id.
    pub fn from_bytes(bytes: [u8; 16]) -> Result<Self, AuditError> {
        if bytes == [0u8; 16] {
            return Err(AuditError::InvalidTraceparent("trace id is all zero"));
        }
        Ok(Self(bytes))
    }

    /// The raw bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl SpanId {
    /// Builds a span id from raw bytes; rejects the all-zero id.
    pub fn from_bytes(bytes: [u8; 8]) -> Result<Self, AuditError> {
        if bytes == [0u8; 8] {
            return Err(AuditError::InvalidTraceparent("span id is all zero"));
        }
        Ok(Self(bytes))
    }

    /// The raw bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 8] {
        &self.0
    }
}

impl fmt::Display for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        hex::write_hex(f, &self.0)
    }
}

impl fmt::Display for SpanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        hex::write_hex(f, &self.0)
    }
}

impl FromStr for TraceId {
    type Err = AuditError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode::<16>(s, false).ok_or(AuditError::InvalidTraceparent(
            "trace id must be 32 lowercase hex characters",
        ))?;
        Self::from_bytes(bytes)
    }
}

impl FromStr for SpanId {
    type Err = AuditError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode::<8>(s, false).ok_or(AuditError::InvalidTraceparent(
            "span id must be 16 lowercase hex characters",
        ))?;
        Self::from_bytes(bytes)
    }
}

macro_rules! serde_via_str {
    ($t:ty) => {
        impl Serialize for $t {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }
        impl<'de> Deserialize<'de> for $t {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                text.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}
pub(crate) use serde_via_str;

serde_via_str!(TraceId);
serde_via_str!(SpanId);
serde_via_str!(TraceContext);

/// The 8-bit trace flags field. Unknown bits are preserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TraceFlags(pub u8);

impl TraceFlags {
    /// The `sampled` bit.
    pub const SAMPLED: u8 = 0x01;

    /// Whether the `sampled` bit is set.
    #[must_use]
    pub const fn is_sampled(self) -> bool {
        self.0 & Self::SAMPLED != 0
    }
}

/// A parsed trace context: which trace we are in and which span is the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TraceContext {
    /// The trace this span belongs to.
    pub trace_id: TraceId,
    /// The current span (the `parent-id` field of the header as seen by the receiver).
    pub span_id: SpanId,
    /// Trace flags.
    pub flags: TraceFlags,
}

/// A child span together with the span it was derived from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChildSpan {
    /// Context to propagate downstream (same trace id and flags, new span id).
    pub context: TraceContext,
    /// The span this child hangs under.
    pub parent_span_id: SpanId,
}

impl TraceContext {
    /// Starts a new trace from caller-supplied ids.
    #[must_use]
    pub const fn new(trace_id: TraceId, span_id: SpanId, flags: TraceFlags) -> Self {
        Self {
            trace_id,
            span_id,
            flags,
        }
    }

    /// Parses a `traceparent` header value with the strict rules in the module documentation.
    pub fn parse(header: &str) -> Result<Self, AuditError> {
        let err = AuditError::InvalidTraceparent;
        if !header.is_ascii() {
            return Err(err("non-ASCII character"));
        }
        if header.len() < TRACEPARENT_LEN {
            return Err(err("too short"));
        }
        // ASCII was checked, so byte offsets are character boundaries.
        let (head, rest) = header.split_at(TRACEPARENT_LEN);
        let mut parts = head.split('-');
        let (Some(version), Some(trace), Some(span), Some(flags), None) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            return Err(err("expected four dash-separated fields"));
        };
        let [version] = hex::decode::<1>(version, false)
            .ok_or(err("version must be 2 lowercase hex characters"))?;
        if version == 0xff {
            return Err(err("version ff is forbidden"));
        }
        if version == 0 && !rest.is_empty() {
            return Err(err("version 00 must be exactly 55 characters"));
        }
        if !(rest.is_empty() || rest.starts_with('-')) {
            return Err(err("unexpected data after flags"));
        }
        let trace_id: TraceId = trace.parse()?;
        let span_id: SpanId = span.parse()?;
        let [flags] = hex::decode::<1>(flags, false)
            .ok_or(err("flags must be 2 lowercase hex characters"))?;
        Ok(Self {
            trace_id,
            span_id,
            flags: TraceFlags(flags),
        })
    }

    /// Derives a child span. The new span id is an input (generated by the caller's id source).
    ///
    /// Fails if the new span id equals the current span id (a span cannot be its own parent).
    pub fn child(&self, new_span_id: SpanId) -> Result<ChildSpan, AuditError> {
        if new_span_id == self.span_id {
            return Err(AuditError::InvalidTraceparent(
                "child span id equals parent span id",
            ));
        }
        Ok(ChildSpan {
            context: Self {
                trace_id: self.trace_id,
                span_id: new_span_id,
                flags: self.flags,
            },
            parent_span_id: self.span_id,
        })
    }
}

impl fmt::Display for TraceContext {
    /// Formats as a version-00 `traceparent` header value.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "00-{}-{}-", self.trace_id, self.span_id)?;
        hex::write_hex(f, &[self.flags.0])
    }
}

impl FromStr for TraceContext {
    type Err = AuditError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

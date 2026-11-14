//! Typed, prefixed, stable identifiers.
//!
//! Every platform object has an ID of the form `<prefix>_<26 characters>`, for example
//! `rec_01j9z3k5q8abcdefghjkmnpqrs`. The body is a 128-bit, ULID-style number:
//!
//! * the top 48 bits are a UTC timestamp in milliseconds;
//! * the low 80 bits are random.
//!
//! The body is written in **lowercase Crockford base32** (`0-9 a-z` without `i l o u`), most
//! significant digit first, always 26 characters. Consequences:
//!
//! * there is exactly one textual form per ID (parsing is strict: no upper case, no aliases);
//! * the text sorts in the same order as the number, so IDs sort roughly by creation time;
//! * the body is safe as a SQL identifier suffix (`t_<body>`), which is how physical tables and
//!   columns are named.
//!
//! This crate is pure, so **generation takes the time and the random bytes as inputs**; nothing
//! here reads a clock or a random source.
//!
//! Each kind of object has its own Rust type ([`RecordId`], [`FieldId`], …), so a record ID can
//! never be passed where a field ID is expected.

use crate::error::{clip, CoreError};
use core::fmt;
use core::str::FromStr;

/// Number of characters in the body of an ID (after the prefix and the underscore).
pub const ID_BODY_LEN: usize = 26;

/// Largest timestamp (milliseconds since the Unix epoch) that fits the 48-bit time part.
pub const ID_MAX_TIME_MS: u64 = 0xFFFF_FFFF_FFFF;

/// Number of random bytes in an ID.
pub const ID_RANDOM_LEN: usize = 10;

const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// Packs time and randomness into the 128-bit ID number.
fn compose(unix_ms: u64, random: [u8; ID_RANDOM_LEN]) -> Option<u128> {
    if unix_ms > ID_MAX_TIME_MS {
        return None;
    }
    let mut value = u128::from(unix_ms);
    for byte in random {
        value = value.wrapping_shl(8) | u128::from(byte);
    }
    Some(value)
}

/// Writes the 26-character body of an ID.
fn encode_body(value: u128) -> String {
    let mut buf = [b'0'; ID_BODY_LEN];
    let mut rest = value;
    for slot in buf.iter_mut().rev() {
        let index = usize::try_from(rest & 31).unwrap_or(0);
        *slot = ALPHABET.get(index).copied().unwrap_or(b'0');
        rest = rest.wrapping_shr(5);
    }
    buf.iter().map(|b| char::from(*b)).collect()
}

/// Reads the 26-character body of an ID.
fn decode_body(body: &str) -> Result<u128, &'static str> {
    if body.len() != ID_BODY_LEN {
        return Err("the part after the prefix must be 26 characters");
    }
    let mut value: u128 = 0;
    for (position, byte) in body.bytes().enumerate() {
        let digit = ALPHABET
            .iter()
            .position(|a| *a == byte)
            .ok_or("only lowercase Crockford base32 characters are allowed")?;
        if position == 0 && digit > 7 {
            return Err("the value does not fit 128 bits");
        }
        value = value.wrapping_shl(5) | u128::try_from(digit).unwrap_or(0);
    }
    Ok(value)
}

fn parse_with_prefix(prefix: &'static str, input: &str) -> Result<u128, CoreError> {
    let err = |reason| CoreError::InvalidId {
        kind: prefix,
        input: clip(input),
        reason,
    };
    let body = input
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('_'))
        .ok_or_else(|| err("wrong or missing prefix"))?;
    decode_body(body).map_err(err)
}

macro_rules! define_ids {
    ($( $(#[$meta:meta])* $name:ident, $variant:ident => $prefix:literal; )+) => {
        /// The kind of object an ID refers to. One variant per typed ID.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum IdKind {
            $( $(#[$meta])* $variant, )+
        }

        impl IdKind {
            /// Every ID kind, in declaration order.
            pub const ALL: &'static [IdKind] = &[ $( IdKind::$variant, )+ ];

            /// The textual prefix of this kind (without the underscore).
            #[must_use]
            pub const fn prefix(self) -> &'static str {
                match self {
                    $( IdKind::$variant => $prefix, )+
                }
            }

            /// Finds the kind for a prefix such as `"rec"`.
            #[must_use]
            pub fn from_prefix(prefix: &str) -> Option<IdKind> {
                IdKind::ALL.iter().copied().find(|kind| kind.prefix() == prefix)
            }

            /// Reads the kind from a full textual ID (`"rec_…"`). The body is validated too.
            #[must_use]
            pub fn of(id: &str) -> Option<IdKind> {
                let (prefix, body) = id.split_once('_')?;
                let kind = IdKind::from_prefix(prefix)?;
                decode_body(body).ok().map(|_| kind)
            }
        }

        $(
            $(#[$meta])*
            ///
            #[doc = concat!("Textual form: `", $prefix, "_<26 lowercase Crockford base32 characters>`.")]
            #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
            pub struct $name(u128);

            impl $name {
                /// The textual prefix (without the underscore).
                pub const PREFIX: &'static str = $prefix;

                /// The kind of this ID type.
                pub const KIND: IdKind = IdKind::$variant;

                /// Creates a new ID from a timestamp (milliseconds since the Unix epoch, UTC)
                /// and ten random bytes. Both are inputs: this function is deterministic.
                ///
                /// # Errors
                /// [`CoreError::InvalidId`] if `unix_ms` does not fit 48 bits.
                pub fn new(unix_ms: u64, random: [u8; ID_RANDOM_LEN]) -> Result<Self, CoreError> {
                    compose(unix_ms, random).map(Self).ok_or_else(|| CoreError::InvalidId {
                        kind: $prefix,
                        input: unix_ms.to_string(),
                        reason: "timestamp does not fit 48 bits",
                    })
                }

                /// Wraps a raw 128-bit number.
                #[must_use]
                pub const fn from_u128(value: u128) -> Self {
                    Self(value)
                }

                /// The raw 128-bit number.
                #[must_use]
                pub const fn as_u128(self) -> u128 {
                    self.0
                }

                /// Big-endian bytes; bytewise order equals ID order.
                #[must_use]
                pub const fn to_bytes(self) -> [u8; 16] {
                    self.0.to_be_bytes()
                }

                /// Inverse of [`Self::to_bytes`].
                #[must_use]
                pub const fn from_bytes(bytes: [u8; 16]) -> Self {
                    Self(u128::from_be_bytes(bytes))
                }

                /// The creation time stored in the ID (milliseconds since the Unix epoch).
                #[must_use]
                pub fn timestamp_ms(self) -> u64 {
                    u64::try_from(self.0.wrapping_shr(80)).unwrap_or(0)
                }

                /// The 26-character body without the prefix; used for physical names.
                #[must_use]
                pub fn body(self) -> String {
                    encode_body(self.0)
                }
            }

            impl fmt::Display for $name {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    write!(f, "{}_{}", $prefix, encode_body(self.0))
                }
            }

            impl fmt::Debug for $name {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    fmt::Display::fmt(self, f)
                }
            }

            impl FromStr for $name {
                type Err = CoreError;
                fn from_str(s: &str) -> Result<Self, Self::Err> {
                    parse_with_prefix($prefix, s).map(Self)
                }
            }

            impl serde::Serialize for $name {
                fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                    serializer.collect_str(self)
                }
            }

            impl<'de> serde::Deserialize<'de> for $name {
                fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                    let text = <std::borrow::Cow<'de, str> as serde::Deserialize>::deserialize(deserializer)?;
                    text.parse().map_err(serde::de::Error::custom)
                }
            }
        )+
    };
}

define_ids! {
    /// A tenant (one customer organisation; one database).
    TenantId, Tenant => "ten";
    /// A login account (control plane).
    AccountId, Account => "acc";
    /// A membership of an account in a tenant.
    MemberId, Member => "mem";
    /// A principal: member, guest, agent, API app or rule owner.
    PrincipalId, Principal => "prn";
    /// A workspace.
    WorkspaceId, Workspace => "wsp";
    /// A base.
    BaseId, Base => "bas";
    /// A table.
    TableId, Table => "tbl";
    /// A field (column) of a table.
    FieldId, Field => "fld";
    /// A field group.
    FieldGroupId, FieldGroup => "fgr";
    /// A reusable field definition in the field library.
    FieldLibraryItemId, FieldLibraryItem => "fli";
    /// A named field type in the field-type registry.
    FieldTypeId, FieldType => "fty";
    /// A record (row).
    RecordId, Record => "rec";
    /// A view.
    ViewId, View => "viw";
    /// An operation (one committed change).
    OpId, Op => "op";
    /// A trace (one end-to-end request or run).
    TraceId, Trace => "trc";
    /// A reference data set (code list).
    RefSetId, RefSet => "rfs";
    /// An item of a reference data set.
    RefItemId, RefItem => "rfi";
    /// An attachment.
    AttachmentId, Attachment => "att";
    /// A comment.
    CommentId, Comment => "cmt";
    /// A numbering sequence.
    SequenceId, Sequence => "seq";
    /// A lifecycle definition.
    LifecycleId, Lifecycle => "lcy";
    /// A rule.
    RuleId, Rule => "rul";
    /// A workflow.
    WorkflowId, Workflow => "wfl";
    /// A run of a rule, workflow, skill or agent.
    RunId, Run => "run";
    /// An AI skill.
    SkillId, Skill => "skl";
    /// An AI agent.
    AgentId, Agent => "agt";
    /// A data source (integration).
    DataSourceId, DataSource => "dsr";
    /// An asynchronous job.
    JobId, Job => "job";
    /// An app.
    AppId, App => "app";
    /// A page.
    PageId, Page => "pag";
    /// A dashboard.
    DashboardId, Dashboard => "dsh";
    /// A template.
    TemplateId, Template => "tpl";
    /// A change set.
    ChangeSetId, ChangeSet => "chg";
}

#[cfg(test)]
mod tests {
    use super::*;

    const RND: [u8; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

    #[test]
    fn std_round_trip_display_parse() {
        let id = RecordId::new(1_700_000_000_000, RND).unwrap();
        let text = id.to_string();
        assert!(text.starts_with("rec_"));
        assert_eq!(text.len(), 4 + ID_BODY_LEN);
        assert_eq!(text.parse::<RecordId>().unwrap(), id);
        assert_eq!(id.timestamp_ms(), 1_700_000_000_000);
        assert_eq!(format!("{id:?}"), text);
        assert_eq!(id.body(), text.strip_prefix("rec_").unwrap());
    }

    #[test]
    fn std_golden_vector() {
        // 0 time, zero random -> all zeros; max -> 7zzz…
        assert_eq!(
            OpId::from_u128(0).to_string(),
            "op_00000000000000000000000000"
        );
        assert_eq!(
            OpId::from_u128(u128::MAX).to_string(),
            "op_7zzzzzzzzzzzzzzzzzzzzzzzzz"
        );
        assert_eq!(
            OpId::from_u128(31).to_string(),
            "op_0000000000000000000000000z"
        );
        assert_eq!(
            OpId::from_u128(32).to_string(),
            "op_00000000000000000000000010"
        );
    }

    #[test]
    fn std_bytes_round_trip_and_order() {
        let a = FieldId::new(1, [0; 10]).unwrap();
        let b = FieldId::new(2, [0; 10]).unwrap();
        assert!(a < b);
        assert!(a.to_bytes() < b.to_bytes());
        assert!(a.to_string() < b.to_string());
        assert_eq!(FieldId::from_bytes(a.to_bytes()), a);
        assert_eq!(a.as_u128(), 1u128 << 80);
    }

    #[test]
    fn std_serde_is_a_string() {
        let id = TableId::from_u128(5);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"tbl_00000000000000000000000005\"");
        assert_eq!(serde_json::from_str::<TableId>(&json).unwrap(), id);
        // Escaped JSON strings (owned Cow path) work too.
        let escaped = "\"tbl_0000000000000000000000000\\u0035\"";
        assert_eq!(serde_json::from_str::<TableId>(escaped).unwrap(), id);
    }

    #[test]
    fn std_kinds_and_prefixes_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for kind in IdKind::ALL {
            assert!(
                seen.insert(kind.prefix()),
                "duplicate prefix {}",
                kind.prefix()
            );
            assert!(kind.prefix().bytes().all(|b| b.is_ascii_lowercase()));
            assert_eq!(IdKind::from_prefix(kind.prefix()), Some(*kind));
        }
        assert_eq!(IdKind::ALL.len(), 33);
        assert_eq!(RecordId::KIND, IdKind::Record);
        assert_eq!(RecordId::PREFIX, "rec");
        assert_eq!(
            IdKind::of("rec_00000000000000000000000005"),
            Some(IdKind::Record)
        );
    }

    #[test]
    fn edge_time_boundaries() {
        let max = TenantId::new(ID_MAX_TIME_MS, [0xFF; 10]).unwrap();
        assert_eq!(max.as_u128(), u128::MAX);
        assert_eq!(max.timestamp_ms(), ID_MAX_TIME_MS);
        assert_eq!(TenantId::new(0, [0; 10]).unwrap().as_u128(), 0);
    }

    #[test]
    fn invalid_time_too_large() {
        let err = TenantId::new(ID_MAX_TIME_MS + 1, [0; 10]).unwrap_err();
        assert!(matches!(err, CoreError::InvalidId { kind: "ten", .. }));
    }

    #[test]
    fn invalid_parse_cases() {
        let good = "rec_00000000000000000000000005";
        assert!(good.parse::<RecordId>().is_ok());
        for bad in [
            "",
            "rec",
            "rec_",
            "fld_00000000000000000000000005",  // wrong prefix
            "rec-00000000000000000000000005",  // wrong separator
            "rec_0000000000000000000000005",   // 25 chars
            "rec_000000000000000000000000055", // 27 chars
            "rec_0000000000000000000000000Z",  // upper case
            "rec_0000000000000000000000000i",  // excluded letter
            "rec_0000000000000000000000000l",
            "rec_0000000000000000000000000o",
            "rec_0000000000000000000000000u",
            "rec_80000000000000000000000000", // overflow
            "rec_0000000000000000000000000é", // non-ASCII (27 bytes)
            " rec_00000000000000000000000005",
        ] {
            assert!(bad.parse::<RecordId>().is_err(), "{bad} should be rejected");
        }
        assert!(serde_json::from_str::<RecordId>("5").is_err());
        assert!(serde_json::from_str::<RecordId>("\"nope\"").is_err());
    }

    #[test]
    fn invalid_kind_lookup() {
        assert_eq!(IdKind::from_prefix("zzz"), None);
        assert_eq!(IdKind::of("zzz_00000000000000000000000005"), None);
        assert_eq!(IdKind::of("rec_bad"), None);
        assert_eq!(IdKind::of("rec"), None);
    }

    #[test]
    fn anomalous_op_prefix_does_not_shadow_longer_text() {
        // "op" is a prefix of no other kind followed by '_', and parsing needs an exact match.
        assert!("opx_00000000000000000000000005".parse::<OpId>().is_err());
        assert!("op_00000000000000000000000005".parse::<OpId>().is_ok());
    }
}

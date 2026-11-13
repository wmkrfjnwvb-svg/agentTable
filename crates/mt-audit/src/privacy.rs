//! Privacy mode for audit details (doc 03 §4.4).
//!
//! When a tenant chooses minimisation, the audit entry keeps the *field id* of a classified field but
//! replaces its value with a keyed digest. Full values stay in cell history, which is subject to
//! retention and data-subject requests; the audit chain itself then never needs to be rewritten.
//!
//! Under [`Privacy::HashClassified`], every object member whose key is in `fields` — at any nesting
//! depth — has its value replaced by `{"$hash":"hmac-sha256:<hex>"}` where the digest is
//! `HMAC-SHA256(key, "mt-audit/value/v1\0" ‖ lp(field) ‖ canonical_json(value))`. The key is a per-tenant
//! secret, which prevents dictionary attacks on low-entropy values by anyone who lacks it. Equal values
//! of the same field give equal digests, so "changed / unchanged" stays visible.

use core::fmt;
use std::collections::BTreeSet;

use hmac::{Hmac, Mac};
use serde_json::{Map, Value};
use sha2::Sha256;

use crate::canonical::{canonical_json, MAX_DEPTH};
use crate::chain::update_lp;
use crate::error::AuditError;
use crate::hex;

const VALUE_DOMAIN: &[u8] = b"mt-audit/value/v1\0";

/// The member name that marks a minimised value.
pub const HASH_MEMBER: &str = "$hash";

/// How audit details are stored.
#[derive(Clone, PartialEq, Eq)]
pub enum Privacy {
    /// Details are stored as given.
    Full,
    /// Values of classified fields are replaced by keyed digests.
    HashClassified {
        /// Field ids (object member names) that are classified.
        fields: BTreeSet<String>,
        /// Per-tenant secret used for the digests.
        key: Vec<u8>,
    },
}

impl fmt::Debug for Privacy {
    /// Never prints the key.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => f.write_str("Full"),
            Self::HashClassified { fields, .. } => f
                .debug_struct("HashClassified")
                .field("fields", fields)
                .finish_non_exhaustive(),
        }
    }
}

impl Privacy {
    /// Returns `details` with this privacy mode applied.
    pub fn apply(&self, details: &Value) -> Result<Value, AuditError> {
        match self {
            Self::Full => Ok(details.clone()),
            Self::HashClassified { fields, key } => minimise(details, fields, key, 0),
        }
    }
}

/// The digest text (`hmac-sha256:<hex>`) stored for `value` of field `field` under `key`.
///
/// Public so that an authorised party holding the key can check a claimed value against the audit log.
pub fn value_digest(key: &[u8], field: &str, value: &Value) -> Result<String, AuditError> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key).map_err(|_| AuditError::Signing("unusable key"))?;
    Mac::update(&mut mac, VALUE_DOMAIN);
    let mut field_part = Collector(Vec::new());
    update_lp(&mut field_part, field.as_bytes());
    Mac::update(&mut mac, &field_part.0);
    Mac::update(&mut mac, canonical_json(value)?.as_bytes());
    Ok(format!(
        "hmac-sha256:{}",
        hex::to_hex(&mac.finalize().into_bytes())
    ))
}

struct Collector(Vec<u8>);

impl sha2::digest::Update for Collector {
    fn update(&mut self, data: &[u8]) {
        self.0.extend_from_slice(data);
    }
}

fn minimise(
    value: &Value,
    fields: &BTreeSet<String>,
    key: &[u8],
    depth: usize,
) -> Result<Value, AuditError> {
    if depth > MAX_DEPTH {
        return Err(AuditError::NotCanonical("nested too deeply"));
    }
    let deeper = depth.saturating_add(1);
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Ok(value.clone()),
        Value::Array(items) => items
            .iter()
            .map(|item| minimise(item, fields, key, deeper))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(map) => {
            let mut out = Map::new();
            for (name, member) in map {
                let replaced = if fields.contains(name) {
                    let mut marker = Map::new();
                    marker.insert(
                        HASH_MEMBER.to_owned(),
                        Value::String(value_digest(key, name, member)?),
                    );
                    Value::Object(marker)
                } else {
                    minimise(member, fields, key, deeper)?
                };
                out.insert(name.clone(), replaced);
            }
            Ok(Value::Object(out))
        }
    }
}

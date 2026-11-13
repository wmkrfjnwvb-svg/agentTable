//! The hash-chained audit log (doc 17 §3.2 D15).
//!
//! # Chain definition
//!
//! A chain belongs to one `chain_id` (the tenant). Its *genesis hash* is
//! `SHA-256("mt-audit/genesis/v1\0" ‖ lp(chain_id))`, so entries cannot be spliced between tenants.
//! Entries are numbered from 1. Entry `n` stores `prev_hash` = hash of entry `n-1` (the genesis hash for
//! `n = 1`) and `hash` = SHA-256 over this exact byte string:
//!
//! ```text
//! "mt-audit/entry/v1\0"
//! seq            u64 big-endian
//! prev_hash      32 bytes
//! actor          kind tag (1 byte) ‖ lp(id)
//! initiator      kind tag (1 byte) ‖ lp(id)
//! lp(action) ‖ lp(object.kind) ‖ lp(object.id) ‖ lp(details)
//! at_ms          i64 big-endian
//! trace id       0x00, or 0x01 ‖ 16 bytes
//! ```
//!
//! `lp(x)` is the byte length of `x` as u64 big-endian followed by the UTF-8 bytes of `x`. Every
//! variable-length field is length-prefixed, so the encoding is injective. `details` is canonical JSON
//! text (see [`crate::canonical`]) and is stored as text so that its bytes never change.
//!
//! # Concurrency
//!
//! Appending is optimistic: [`AuditLog::prepare`] builds an entry on top of a head, and
//! [`AuditStore::append`] must atomically refuse an entry that does not extend the current head
//! ([`AuditError::AppendConflict`] / [`AuditError::BrokenLink`]). A writer that loses the race prepares
//! again on the new head, so the chain stays linear under any interleaving.

use core::fmt;
use core::num::NonZeroU64;
use core::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::canonical::canonical_json;
use crate::checkpoint::{Checkpoint, CheckpointSigner};
use crate::error::AuditError;
use crate::hex;
use crate::privacy::Privacy;
use crate::trace::{serde_via_str, TraceId};

/// Maximum byte length of a principal id, object kind or object id.
pub const MAX_ID_BYTES: usize = 256;
/// Maximum byte length of an action name.
pub const MAX_ACTION_BYTES: usize = 128;
/// Maximum byte length of the canonical details text.
pub const MAX_DETAILS_BYTES: usize = 262_144;

const ENTRY_DOMAIN: &[u8] = b"mt-audit/entry/v1\0";
const GENESIS_DOMAIN: &[u8] = b"mt-audit/genesis/v1\0";

/// A SHA-256 digest.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash32([u8; 32]);

impl Hash32 {
    /// Wraps raw digest bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Hash32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        hex::write_hex(f, &self.0)
    }
}

impl fmt::Debug for Hash32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash32({self})")
    }
}

impl FromStr for Hash32 {
    type Err = AuditError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        hex::decode::<32>(s, false)
            .map(Self)
            .ok_or(AuditError::InvalidHash)
    }
}

serde_via_str!(Hash32);

/// What kind of identity a [`Principal`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    /// A human account.
    User,
    /// An API client / integration.
    Service,
    /// An AI skill.
    AiSkill,
    /// An AI agent.
    AiAgent,
    /// An automation rule (acting for its owner).
    Rule,
    /// A schedule (acting for its owner).
    Schedule,
    /// The platform itself.
    System,
}

impl PrincipalKind {
    /// The tag byte used in the hash preimage. Stable forever.
    const fn tag(self) -> u8 {
        match self {
            Self::User => 1,
            Self::Service => 2,
            Self::AiSkill => 3,
            Self::AiAgent => 4,
            Self::Rule => 5,
            Self::Schedule => 6,
            Self::System => 7,
        }
    }
}

/// An identity that acts or on whose behalf something acts.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Principal {
    /// The kind of identity.
    pub kind: PrincipalKind,
    /// Its identifier (non-empty, at most [`MAX_ID_BYTES`] bytes).
    pub id: String,
}

impl Principal {
    /// Builds a principal.
    pub fn new(kind: PrincipalKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
        }
    }
}

/// A reference to the object an action touched.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ObjectRef {
    /// Object kind, e.g. `record`, `field`, `ai_run`.
    pub kind: String,
    /// Object identifier.
    pub id: String,
}

impl ObjectRef {
    /// Builds an object reference.
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
        }
    }
}

/// The caller-supplied part of an audit entry.
///
/// Dual identity (doc 18): `actor` is the principal that executed the action (for AI runs the skill or
/// agent); `initiator` is the principal on whose behalf it ran (user, rule owner, schedule owner). For a
/// direct action both are the same principal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditDraft {
    /// Who executed the action.
    pub actor: Principal,
    /// On whose behalf.
    pub initiator: Principal,
    /// Action name, e.g. `record.update`.
    pub action: String,
    /// The object acted on.
    pub object: ObjectRef,
    /// Structured details; canonicalised (and minimised under [`Privacy`]) on append.
    pub details: Value,
    /// When it happened, UTC milliseconds since the epoch.
    pub at_ms: i64,
    /// The trace of the user action that caused this, if any.
    pub trace_id: Option<TraceId>,
}

impl AuditDraft {
    /// A direct action: the actor is also the initiator. Details default to `{}`.
    pub fn direct(
        actor: Principal,
        action: impl Into<String>,
        object: ObjectRef,
        at_ms: i64,
    ) -> Self {
        Self {
            initiator: actor.clone(),
            actor,
            action: action.into(),
            object,
            details: Value::Object(serde_json::Map::new()),
            at_ms,
            trace_id: None,
        }
    }

    /// Sets a distinct initiator (dual identity).
    #[must_use]
    pub fn on_behalf_of(mut self, initiator: Principal) -> Self {
        self.initiator = initiator;
        self
    }

    /// Sets the details.
    #[must_use]
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }

    /// Sets the trace id.
    #[must_use]
    pub fn with_trace(mut self, trace_id: TraceId) -> Self {
        self.trace_id = Some(trace_id);
        self
    }
}

/// One link of the chain. See the module documentation for the hash definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Position in the chain, starting at 1.
    pub seq: u64,
    /// Hash of the previous entry (genesis hash for the first).
    pub prev_hash: Hash32,
    /// Hash of this entry.
    pub hash: Hash32,
    /// Who executed the action.
    pub actor: Principal,
    /// On whose behalf.
    pub initiator: Principal,
    /// Action name.
    pub action: String,
    /// The object acted on.
    pub object: ObjectRef,
    /// Canonical JSON text of the details.
    pub details: String,
    /// When it happened, UTC milliseconds since the epoch.
    pub at_ms: i64,
    /// Trace id of the causing user action.
    pub trace_id: Option<TraceId>,
}

impl AuditEntry {
    /// Recomputes the hash from the entry's content (ignores the stored `hash`).
    #[must_use]
    pub fn compute_hash(&self) -> Hash32 {
        let mut h = Sha256::new();
        h.update(ENTRY_DOMAIN);
        h.update(self.seq.to_be_bytes());
        h.update(self.prev_hash.0);
        for principal in [&self.actor, &self.initiator] {
            h.update([principal.kind.tag()]);
            update_lp(&mut h, principal.id.as_bytes());
        }
        update_lp(&mut h, self.action.as_bytes());
        update_lp(&mut h, self.object.kind.as_bytes());
        update_lp(&mut h, self.object.id.as_bytes());
        update_lp(&mut h, self.details.as_bytes());
        h.update(self.at_ms.to_be_bytes());
        match &self.trace_id {
            None => h.update([0u8]),
            Some(id) => {
                h.update([1u8]);
                h.update(id.as_bytes());
            }
        }
        Hash32(h.finalize().into())
    }

    /// Whether the stored `hash` matches the content.
    #[must_use]
    pub fn is_self_consistent(&self) -> bool {
        self.compute_hash() == self.hash
    }

    /// This entry as a chain head.
    #[must_use]
    pub const fn head(&self) -> ChainHead {
        ChainHead {
            seq: self.seq,
            hash: self.hash,
        }
    }
}

/// Feeds a length-prefixed byte string into the hasher.
pub(crate) fn update_lp(h: &mut impl sha2::digest::Update, bytes: &[u8]) {
    // usize is at most 64 bits on every supported target.
    h.update(&u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes());
    h.update(bytes);
}

/// A position in the chain: the sequence number and hash of its last entry (`seq` 0 = genesis).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainHead {
    /// Sequence number of the last entry (0 for an empty chain).
    pub seq: u64,
    /// Hash of the last entry (genesis hash for an empty chain).
    pub hash: Hash32,
}

/// Storage for one chain. Implementations do the I/O; this crate only defines the contract.
///
/// `append` **must** be atomic and must refuse any entry that does not extend the current head: this is
/// the compare-and-set that keeps the chain linear when several writers race.
pub trait AuditStore {
    /// The last stored entry's position, or `None` if the chain is empty.
    fn head(&self) -> Result<Option<ChainHead>, AuditError>;

    /// Appends `entry` if and only if `entry.seq` is the head's `seq + 1` (1 for an empty chain) and, for
    /// a non-empty chain, `entry.prev_hash` equals the head's hash.
    ///
    /// Errors: [`AuditError::AppendConflict`] on a sequence mismatch, [`AuditError::BrokenLink`] on a
    /// hash mismatch. On error nothing is stored.
    fn append(&mut self, entry: AuditEntry) -> Result<(), AuditError>;

    /// The entry with sequence number `seq`, if stored.
    fn get(&self, seq: u64) -> Result<Option<AuditEntry>, AuditError>;

    /// Up to `limit` entries in storage order, skipping the first `offset` stored entries.
    fn scan(&self, offset: usize, limit: usize) -> Result<Vec<AuditEntry>, AuditError>;

    /// Stores a checkpoint.
    fn put_checkpoint(&mut self, checkpoint: Checkpoint) -> Result<(), AuditError>;

    /// All checkpoints, ascending by `seq`.
    fn checkpoints(&self) -> Result<Vec<Checkpoint>, AuditError>;
}

/// An [`AuditStore`] kept in memory. Used in tests, the simulator and the browser.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InMemoryAuditStore {
    entries: Vec<AuditEntry>,
    checkpoints: Vec<Checkpoint>,
}

impl InMemoryAuditStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The stored entries in storage order.
    #[must_use]
    pub fn entries(&self) -> &[AuditEntry] {
        &self.entries
    }

    /// Raw mutable access to the stored entries, bypassing every check. Exists so that tests and the
    /// simulator can model an attacker or a corrupted disk.
    pub fn entries_mut(&mut self) -> &mut Vec<AuditEntry> {
        &mut self.entries
    }

    /// Raw mutable access to the stored checkpoints (same purpose as [`Self::entries_mut`]).
    pub fn checkpoints_mut(&mut self) -> &mut Vec<Checkpoint> {
        &mut self.checkpoints
    }
}

impl AuditStore for InMemoryAuditStore {
    fn head(&self) -> Result<Option<ChainHead>, AuditError> {
        Ok(self.entries.last().map(AuditEntry::head))
    }

    fn append(&mut self, entry: AuditEntry) -> Result<(), AuditError> {
        let (expected_seq, expected_prev) = match self.entries.last() {
            None => (1, None),
            Some(last) => (
                last.seq
                    .checked_add(1)
                    .ok_or(AuditError::SequenceOverflow)?,
                Some(last.hash),
            ),
        };
        if entry.seq != expected_seq {
            return Err(AuditError::AppendConflict {
                expected_seq,
                entry_seq: entry.seq,
            });
        }
        if expected_prev.is_some_and(|prev| prev != entry.prev_hash) {
            return Err(AuditError::BrokenLink { seq: entry.seq });
        }
        self.entries.push(entry);
        Ok(())
    }

    fn get(&self, seq: u64) -> Result<Option<AuditEntry>, AuditError> {
        // Fast path for an untampered store, then a scan so that tampered stores still answer.
        let direct = seq
            .checked_sub(1)
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| self.entries.get(i))
            .filter(|e| e.seq == seq);
        Ok(direct
            .or_else(|| self.entries.iter().find(|e| e.seq == seq))
            .cloned())
    }

    fn scan(&self, offset: usize, limit: usize) -> Result<Vec<AuditEntry>, AuditError> {
        Ok(self
            .entries
            .iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect())
    }

    fn put_checkpoint(&mut self, checkpoint: Checkpoint) -> Result<(), AuditError> {
        let at = self
            .checkpoints
            .partition_point(|c| c.seq <= checkpoint.seq);
        self.checkpoints.insert(at, checkpoint);
        Ok(())
    }

    fn checkpoints(&self) -> Result<Vec<Checkpoint>, AuditError> {
        Ok(self.checkpoints.clone())
    }
}

/// The append-side logic of one chain: canonicalisation, privacy, hashing, checkpoint cadence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditLog {
    chain_id: String,
    privacy: Privacy,
    checkpoint_every: Option<NonZeroU64>,
}

impl AuditLog {
    /// A log for the chain `chain_id` (normally the tenant id) with full details and no automatic
    /// checkpoints.
    pub fn new(chain_id: impl Into<String>) -> Result<Self, AuditError> {
        let chain_id = chain_id.into();
        check_text("chain_id", &chain_id, MAX_ID_BYTES)?;
        Ok(Self {
            chain_id,
            privacy: Privacy::Full,
            checkpoint_every: None,
        })
    }

    /// Sets the privacy mode applied to details on append.
    #[must_use]
    pub fn with_privacy(mut self, privacy: Privacy) -> Self {
        self.privacy = privacy;
        self
    }

    /// Makes [`Self::append_checkpointed`] sign a checkpoint whenever `seq` is a multiple of `every`.
    #[must_use]
    pub fn with_checkpoint_every(mut self, every: NonZeroU64) -> Self {
        self.checkpoint_every = Some(every);
        self
    }

    /// The chain identifier.
    #[must_use]
    pub fn chain_id(&self) -> &str {
        &self.chain_id
    }

    /// The head of the empty chain (`seq` 0, genesis hash).
    #[must_use]
    pub fn genesis(&self) -> ChainHead {
        let mut h = Sha256::new();
        h.update(GENESIS_DOMAIN);
        update_lp(&mut h, self.chain_id.as_bytes());
        ChainHead {
            seq: 0,
            hash: Hash32(h.finalize().into()),
        }
    }

    /// The store's head, or the genesis head if the store is empty.
    pub fn head<S: AuditStore + ?Sized>(&self, store: &S) -> Result<ChainHead, AuditError> {
        Ok(store.head()?.unwrap_or_else(|| self.genesis()))
    }

    /// Builds the entry that extends `head` with `draft`. Pure: nothing is stored.
    pub fn prepare(&self, head: &ChainHead, draft: AuditDraft) -> Result<AuditEntry, AuditError> {
        check_text("actor.id", &draft.actor.id, MAX_ID_BYTES)?;
        check_text("initiator.id", &draft.initiator.id, MAX_ID_BYTES)?;
        check_text("action", &draft.action, MAX_ACTION_BYTES)?;
        check_text("object.kind", &draft.object.kind, MAX_ID_BYTES)?;
        check_text("object.id", &draft.object.id, MAX_ID_BYTES)?;
        let details = canonical_json(&self.privacy.apply(&draft.details)?)?;
        if details.len() > MAX_DETAILS_BYTES {
            return Err(AuditError::InvalidField {
                field: "details",
                problem: "too long",
            });
        }
        let mut entry = AuditEntry {
            seq: head
                .seq
                .checked_add(1)
                .ok_or(AuditError::SequenceOverflow)?,
            prev_hash: head.hash,
            hash: head.hash,
            actor: draft.actor,
            initiator: draft.initiator,
            action: draft.action,
            object: draft.object,
            details,
            at_ms: draft.at_ms,
            trace_id: draft.trace_id,
        };
        entry.hash = entry.compute_hash();
        Ok(entry)
    }

    /// Prepares `draft` on the store's current head and appends it.
    ///
    /// With a shared store another writer may win between the two steps; the store then answers
    /// [`AuditError::AppendConflict`] and the caller retries.
    pub fn append<S: AuditStore + ?Sized>(
        &self,
        store: &mut S,
        draft: AuditDraft,
    ) -> Result<AuditEntry, AuditError> {
        let entry = self.prepare(&self.head(store)?, draft)?;
        store.append(entry.clone())?;
        Ok(entry)
    }

    /// Like [`Self::append`], and additionally signs and stores a checkpoint when the new entry's `seq`
    /// is a multiple of the configured cadence. The checkpoint time is the entry's `at_ms`.
    pub fn append_checkpointed<S: AuditStore + ?Sized>(
        &self,
        store: &mut S,
        draft: AuditDraft,
        signer: &dyn CheckpointSigner,
    ) -> Result<(AuditEntry, Option<Checkpoint>), AuditError> {
        let entry = self.append(store, draft)?;
        let due = self
            .checkpoint_every
            .is_some_and(|n| entry.seq.checked_rem(n.get()) == Some(0));
        if !due {
            return Ok((entry, None));
        }
        let checkpoint = Checkpoint::sign(&self.chain_id, entry.head(), entry.at_ms, signer)?;
        store.put_checkpoint(checkpoint.clone())?;
        Ok((entry, Some(checkpoint)))
    }

    /// Signs and stores a checkpoint over the store's current head.
    pub fn checkpoint<S: AuditStore + ?Sized>(
        &self,
        store: &mut S,
        signer: &dyn CheckpointSigner,
        at_ms: i64,
    ) -> Result<Checkpoint, AuditError> {
        let head = store
            .head()?
            .ok_or(AuditError::NothingToCheckpoint { seq: 0 })?;
        let checkpoint = Checkpoint::sign(&self.chain_id, head, at_ms, signer)?;
        store.put_checkpoint(checkpoint.clone())?;
        Ok(checkpoint)
    }
}

fn check_text(field: &'static str, text: &str, max: usize) -> Result<(), AuditError> {
    if text.is_empty() {
        return Err(AuditError::InvalidField {
            field,
            problem: "empty",
        });
    }
    if text.len() > max {
        return Err(AuditError::InvalidField {
            field,
            problem: "too long",
        });
    }
    Ok(())
}

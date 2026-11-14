//! Signed checkpoints over the audit chain.
//!
//! A checkpoint states "the chain `chain_id` had hash `hash` at `seq`", signed. Anyone holding a
//! checkpoint (ideally exported outside the database) can later prove that history up to `seq` was not
//! rewritten. The signed message is:
//!
//! ```text
//! "mt-audit/checkpoint/v1\0" ‖ lp(chain_id) ‖ seq (u64 BE) ‖ hash (32 bytes) ‖ at_ms (i64 BE)
//!   ‖ lp(algorithm) ‖ lp(key_id)
//! ```
//!
//! Signing is behind [`CheckpointSigner`] / [`CheckpointVerifier`], so an asymmetric scheme (for example
//! Ed25519 held in a KMS) can replace the built-in [`HmacSha256Signer`] without touching the chain.

use core::fmt;

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::chain::{update_lp, ChainHead, Hash32};
use crate::error::AuditError;

const CHECKPOINT_DOMAIN: &[u8] = b"mt-audit/checkpoint/v1\0";

/// Algorithm name of [`HmacSha256Signer`].
pub const HMAC_SHA256: &str = "hmac-sha256";

/// A signed statement about the chain head at some point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Sequence number covered by the checkpoint.
    pub seq: u64,
    /// Hash of the entry at `seq`.
    pub hash: Hash32,
    /// When the checkpoint was made, UTC milliseconds.
    pub at_ms: i64,
    /// Signature algorithm name.
    pub algorithm: String,
    /// Identifier of the signing key (for rotation).
    pub key_id: String,
    /// The signature bytes.
    pub signature: Vec<u8>,
}

/// Produces checkpoint signatures. Implement this for an HSM/KMS or an asymmetric key.
pub trait CheckpointSigner {
    /// Algorithm name recorded in the checkpoint.
    fn algorithm(&self) -> &str;
    /// Key identifier recorded in the checkpoint.
    fn key_id(&self) -> &str;
    /// Signs `message`.
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, AuditError>;
}

/// Checks checkpoint signatures. For asymmetric schemes this needs only the public key.
pub trait CheckpointVerifier {
    /// Returns `Ok(())` if `signature` is valid for `message` under the named key and algorithm,
    /// [`AuditError::BadSignature`] if it is not, and [`AuditError::UnknownKey`] if this verifier does
    /// not know the key or algorithm.
    fn verify(
        &self,
        algorithm: &str,
        key_id: &str,
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), AuditError>;
}

impl Checkpoint {
    /// Signs a checkpoint for `head` of chain `chain_id`.
    pub fn sign(
        chain_id: &str,
        head: ChainHead,
        at_ms: i64,
        signer: &dyn CheckpointSigner,
    ) -> Result<Self, AuditError> {
        let mut checkpoint = Self {
            seq: head.seq,
            hash: head.hash,
            at_ms,
            algorithm: signer.algorithm().to_owned(),
            key_id: signer.key_id().to_owned(),
            signature: Vec::new(),
        };
        checkpoint.signature = signer.sign(&checkpoint.message(chain_id))?;
        Ok(checkpoint)
    }

    /// The exact bytes that are signed (see the module documentation).
    #[must_use]
    pub fn message(&self, chain_id: &str) -> Vec<u8> {
        let mut out = Buffer(Vec::new());
        out.0.extend_from_slice(CHECKPOINT_DOMAIN);
        update_lp(&mut out, chain_id.as_bytes());
        out.0.extend_from_slice(&self.seq.to_be_bytes());
        out.0.extend_from_slice(self.hash.as_bytes());
        out.0.extend_from_slice(&self.at_ms.to_be_bytes());
        update_lp(&mut out, self.algorithm.as_bytes());
        update_lp(&mut out, self.key_id.as_bytes());
        out.0
    }

    /// Verifies the signature for chain `chain_id`.
    pub fn verify_signature(
        &self,
        chain_id: &str,
        verifier: &dyn CheckpointVerifier,
    ) -> Result<(), AuditError> {
        verifier.verify(
            &self.algorithm,
            &self.key_id,
            &self.message(chain_id),
            &self.signature,
        )
    }

    /// The chain position this checkpoint attests.
    #[must_use]
    pub const fn head(&self) -> ChainHead {
        ChainHead {
            seq: self.seq,
            hash: self.hash,
        }
    }
}

/// Byte buffer that accepts the same length-prefixed writes as a hasher.
struct Buffer(Vec<u8>);

impl sha2::digest::Update for Buffer {
    fn update(&mut self, data: &[u8]) {
        self.0.extend_from_slice(data);
    }
}

/// HMAC-SHA256 signer and verifier with a caller-provided secret key.
///
/// The key is held as a keyed HMAC state, not as bytes, and the type cannot be compared, so no
/// code path can leak the key through a non-constant-time comparison.
#[derive(Clone)]
pub struct HmacSha256Signer {
    key_id: String,
    keyed: Hmac<Sha256>,
}

impl HmacSha256Signer {
    /// Minimum accepted key length in bytes.
    pub const MIN_KEY_BYTES: usize = 16;

    /// Builds a signer. The key must be at least [`Self::MIN_KEY_BYTES`] long and `key_id` non-empty.
    pub fn new(key_id: impl Into<String>, key: &[u8]) -> Result<Self, AuditError> {
        let key_id = key_id.into();
        if key_id.is_empty() {
            return Err(AuditError::Signing("empty key id"));
        }
        if key.len() < Self::MIN_KEY_BYTES {
            return Err(AuditError::Signing("key too short"));
        }
        // HMAC accepts keys of any length, so this cannot fail for a long enough key.
        let keyed =
            Hmac::<Sha256>::new_from_slice(key).map_err(|_| AuditError::Signing("unusable key"))?;
        Ok(Self { key_id, keyed })
    }

    fn mac(&self, message: &[u8]) -> Hmac<Sha256> {
        let mut mac = self.keyed.clone();
        mac.update(message);
        mac
    }
}

impl fmt::Debug for HmacSha256Signer {
    /// Never prints the key.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HmacSha256Signer")
            .field("key_id", &self.key_id)
            .finish_non_exhaustive()
    }
}

impl CheckpointSigner for HmacSha256Signer {
    fn algorithm(&self) -> &str {
        HMAC_SHA256
    }

    fn key_id(&self) -> &str {
        &self.key_id
    }

    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, AuditError> {
        Ok(self.mac(message).finalize().into_bytes().to_vec())
    }
}

impl CheckpointVerifier for HmacSha256Signer {
    fn verify(
        &self,
        algorithm: &str,
        key_id: &str,
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), AuditError> {
        if algorithm != HMAC_SHA256 || key_id != self.key_id {
            return Err(AuditError::UnknownKey);
        }
        // `verify_slice` compares in constant time.
        self.mac(message)
            .verify_slice(signature)
            .map_err(|_| AuditError::BadSignature)
    }
}

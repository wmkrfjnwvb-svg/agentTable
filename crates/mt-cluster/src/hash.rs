//! Stable 64-bit hashing used for placement.
//!
//! The functions here are part of the cluster contract: every node, on every platform and
//! every version, must compute identical values, otherwise nodes disagree about owners.
//! They are therefore implemented here (not taken from `std`, whose hashers are unspecified)
//! and pinned by golden-vector tests.
//!
//! * [`stable_hash64`] = FNV-1a 64 over the bytes, then the SplitMix64 finalizer for avalanche.
//! * [`rendezvous_score`] combines a key hash with a node id and replica index.
//!
//! The hash is **not** cryptographic. Keys are internal identifiers, not attacker-chosen data.

/// SplitMix64 finalizer: a bijective 64-bit mixer with good avalanche.
#[must_use]
pub const fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Stable hash of a byte string (FNV-1a 64 + [`mix64`]).
#[must_use]
pub fn stable_hash64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    mix64(h)
}

/// Rendezvous score of `(key, node, replica)`. Higher wins.
#[must_use]
pub const fn rendezvous_score(key_hash: u64, node: u64, replica: u32) -> u64 {
    let salt = (replica as u64)
        .wrapping_add(1)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15);
    mix64(key_hash ^ mix64(node.wrapping_add(salt)))
}

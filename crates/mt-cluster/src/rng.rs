//! A tiny deterministic PRNG (SplitMix64). Randomness is always injected as a seed, never
//! drawn from the environment, so every run is reproducible.

/// SplitMix64 generator. Not cryptographic; used for probe order, gossip targets and the
/// simulator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Creates a generator from a seed. The same seed gives the same sequence on every platform.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        crate::hash::mix64(self.state)
    }

    /// Uniform-ish value in `0..n` (`0` when `n == 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64().checked_rem(n).unwrap_or(0)
    }

    /// Value in `lo..=hi` (returns `lo` when `hi <= lo`).
    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        let span = hi.saturating_sub(lo).saturating_add(1);
        lo.saturating_add(self.below(span))
    }

    /// True with probability `per_mille / 1000`.
    pub fn chance(&mut self, per_mille: u16) -> bool {
        self.below(1000) < u64::from(per_mille)
    }

    /// Index in `0..len` (`0` when `len == 0`).
    pub fn index(&mut self, len: usize) -> usize {
        let n = u64::try_from(len).unwrap_or(u64::MAX);
        usize::try_from(self.below(n)).unwrap_or(0)
    }

    /// Fisher–Yates shuffle.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        let mut i = items.len();
        while i > 1 {
            let j = self.index(i);
            i = i.saturating_sub(1);
            items.swap(i, j);
        }
    }
}

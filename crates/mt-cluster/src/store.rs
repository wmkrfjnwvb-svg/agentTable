//! In-memory compare-and-set store for tests and simulation. The production store is
//! database-backed and lives in another crate.

use crate::error::ClusterError;
use crate::fencing::{EpochRecord, EpochStore};
use crate::scheduler::{LeaseRecord, LeaseStore};
use std::collections::BTreeMap;

/// A map with compare-and-set and failure injection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryStore<V> {
    map: BTreeMap<Vec<u8>, V>,
    fail_next: u32,
}

/// In-memory [`EpochStore`].
pub type MemoryEpochStore = MemoryStore<EpochRecord>;
/// In-memory [`LeaseStore`].
pub type MemoryLeaseStore = MemoryStore<LeaseRecord>;

impl<V> Default for MemoryStore<V> {
    fn default() -> Self {
        Self {
            map: BTreeMap::new(),
            fail_next: 0,
        }
    }
}

impl<V: Clone + PartialEq> MemoryStore<V> {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes the next `n` operations fail with [`ClusterError::StoreUnavailable`].
    pub fn fail_next(&mut self, n: u32) {
        self.fail_next = n;
    }

    /// Number of stored records.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True when nothing is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    fn fault(&mut self) -> Result<(), ClusterError> {
        if self.fail_next > 0 {
            self.fail_next = self.fail_next.saturating_sub(1);
            return Err(ClusterError::StoreUnavailable("injected".into()));
        }
        Ok(())
    }

    /// Reads a record.
    pub fn get(&mut self, key: &[u8]) -> Result<Option<V>, ClusterError> {
        self.fault()?;
        Ok(self.map.get(key).cloned())
    }

    /// Writes `new` only if the current record equals `expected`. Returns whether it did.
    pub fn cas(&mut self, key: &[u8], expected: Option<&V>, new: V) -> Result<bool, ClusterError> {
        self.fault()?;
        if self.map.get(key) != expected {
            return Ok(false);
        }
        self.map.insert(key.to_vec(), new);
        Ok(true)
    }
}

impl EpochStore for MemoryStore<EpochRecord> {
    fn load(&mut self, resource: &[u8]) -> Result<Option<EpochRecord>, ClusterError> {
        self.get(resource)
    }

    fn compare_and_set(
        &mut self,
        resource: &[u8],
        expected: Option<&EpochRecord>,
        new: EpochRecord,
    ) -> Result<bool, ClusterError> {
        self.cas(resource, expected, new)
    }
}

impl LeaseStore for MemoryStore<LeaseRecord> {
    fn load(&mut self, shard: u32) -> Result<Option<LeaseRecord>, ClusterError> {
        self.get(&shard.to_be_bytes())
    }

    fn compare_and_set(
        &mut self,
        shard: u32,
        expected: Option<&LeaseRecord>,
        new: LeaseRecord,
    ) -> Result<bool, ClusterError> {
        self.cas(&shard.to_be_bytes(), expected, new)
    }
}

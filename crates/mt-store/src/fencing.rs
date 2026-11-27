//! Ownership epochs (fencing) and scheduler leases, stored as compare-and-set rows — the
//! database side of `mt-cluster`'s `EpochStore` and `LeaseStore`, with the same semantics.
//! A write by a base's owner checks its token inside the write's own transaction, so an
//! owner that lost ownership (a partition, a pause) can never commit.

use crate::dialect::Dialect;
use crate::error::{StoreError, StoreResult};
use crate::store::{int, uint, Tenant};
use mt_cluster::{EpochRecord, FencingToken, LeaseRecord, NodeId};
use sqlx::any::AnyRow;

/// Longest resource name in bytes.
pub const MAX_RESOURCE: usize = 120;

fn hex(bytes: &[u8]) -> StoreResult<String> {
    if bytes.is_empty() || bytes.len() > MAX_RESOURCE {
        return Err(StoreError::Invalid(format!(
            "a resource has 1 to {MAX_RESOURCE} bytes"
        )));
    }
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn node_of(
    id: Option<i64>,
    incarnation: Option<i64>,
    addr: Option<String>,
) -> StoreResult<Option<NodeId>> {
    match (id, incarnation, addr) {
        (Some(id), Some(incarnation), Some(addr)) => {
            Ok(Some(NodeId::new(uint(id)?, uint(incarnation)?, addr)))
        }
        (None, None, None) => Ok(None),
        (_, _, _) => Err(StoreError::Corrupt("a half-recorded owner".to_owned())),
    }
}

fn epoch_of(row: &AnyRow) -> StoreResult<EpochRecord> {
    Ok(EpochRecord {
        epoch: uint(crate::row::int(row, "epoch")?)?,
        owner: node_of(
            crate::row::opt_int(row, "owner_id")?,
            crate::row::opt_int(row, "owner_incarnation")?,
            crate::row::opt_text(row, "owner_addr")?,
        )?,
    })
}

fn lease_of(row: &AnyRow) -> StoreResult<LeaseRecord> {
    Ok(LeaseRecord {
        term: uint(crate::row::int(row, "term")?)?,
        holder: node_of(
            crate::row::opt_int(row, "holder_id")?,
            crate::row::opt_int(row, "holder_incarnation")?,
            crate::row::opt_text(row, "holder_addr")?,
        )?,
        expires_at_ms: uint(crate::row::int(row, "expires_at")?)?,
        cursor_ms: uint(crate::row::int(row, "cursor_at")?)?,
    })
}

type NodeCols = (Option<i64>, Option<i64>, Option<String>);

fn node_cols(node: Option<&NodeId>) -> StoreResult<NodeCols> {
    Ok(match node {
        Some(n) => (
            Some(int(n.id)?),
            Some(int(n.incarnation)?),
            Some(n.addr.clone()),
        ),
        None => (None, None, None),
    })
}

/// `col = ?` or `col IS NULL`, numbering placeholders from `*n`.
fn same(d: Dialect, col: &str, present: bool, n: &mut usize) -> String {
    if present {
        *n = n.saturating_add(1);
        format!("{col} = {}", d.param(*n))
    } else {
        format!("{col} IS NULL")
    }
}

impl Tenant {
    /// The epoch record of a resource.
    ///
    /// # Errors
    /// [`StoreError::Invalid`] for a bad resource; the database's errors.
    pub async fn epoch(&self, resource: &[u8]) -> StoreResult<Option<EpochRecord>> {
        let row = sqlx::query(&format!(
            "SELECT epoch, owner_id, owner_incarnation, owner_addr FROM _epochs WHERE resource = {}",
            self.dialect.param(1)
        ))
        .bind(hex(resource)?)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(epoch_of).transpose()
    }

    /// Replaces the epoch record with `new` if it currently is `expected`; returns whether
    /// it did.
    ///
    /// # Errors
    /// As [`Tenant::epoch`].
    pub async fn compare_and_set_epoch(
        &self,
        resource: &[u8],
        expected: Option<&EpochRecord>,
        new: &EpochRecord,
    ) -> StoreResult<bool> {
        let d = self.dialect;
        let key = hex(resource)?;
        let (id, inc, addr) = node_cols(new.owner.as_ref())?;
        let Some(expected) = expected else {
            let inserted = sqlx::query(&format!(
                "INSERT INTO _epochs (resource, epoch, owner_id, owner_incarnation, owner_addr) VALUES ({})",
                d.params(1, 5)
            ))
            .bind(key)
            .bind(int(new.epoch)?)
            .bind(id)
            .bind(inc)
            .bind(addr)
            .execute(&self.pool)
            .await;
            return match inserted {
                Ok(_) => Ok(true),
                Err(error) => match self.epoch(resource).await? {
                    Some(_) => Ok(false),
                    None => Err(error.into()),
                },
            };
        };
        if expected == new {
            return Ok(self.epoch(resource).await?.as_ref() == Some(new));
        }
        let (eid, einc, eaddr) = node_cols(expected.owner.as_ref())?;
        let mut n = 6;
        let owner = expected.owner.is_some();
        let sql = format!(
            "UPDATE _epochs SET epoch = {}, owner_id = {}, owner_incarnation = {}, owner_addr = {} \
             WHERE resource = {} AND epoch = {} AND {} AND {} AND {}",
            d.param(1),
            d.param(2),
            d.param(3),
            d.param(4),
            d.param(5),
            d.param(6),
            same(d, "owner_id", owner, &mut n),
            same(d, "owner_incarnation", owner, &mut n),
            same(d, "owner_addr", owner, &mut n),
        );
        let mut query = sqlx::query(&sql)
            .bind(int(new.epoch)?)
            .bind(id)
            .bind(inc)
            .bind(addr)
            .bind(key)
            .bind(int(expected.epoch)?);
        if owner {
            query = query.bind(eid).bind(einc).bind(eaddr);
        }
        Ok(query.execute(&self.pool).await?.rows_affected() == 1)
    }

    /// Becomes the owner of a resource by raising its epoch (as `mt-cluster`'s `Fencer`).
    /// Idempotent while the stored record shows `me` at `held`.
    ///
    /// # Errors
    /// [`StoreError::Conflict`] when another node won the race; the database's errors.
    pub async fn acquire(
        &self,
        resource: &[u8],
        me: &NodeId,
        held: Option<u64>,
    ) -> StoreResult<FencingToken> {
        let current = self.epoch(resource).await?;
        let next = match &current {
            None => 1,
            Some(r) => {
                if held == Some(r.epoch) && r.owner.as_ref() == Some(me) {
                    return Ok(FencingToken {
                        resource: resource.to_vec(),
                        epoch: r.epoch,
                        owner: me.clone(),
                    });
                }
                r.epoch
                    .checked_add(1)
                    .ok_or_else(|| StoreError::Invalid("epoch overflow".to_owned()))?
            }
        };
        let new = EpochRecord {
            epoch: next,
            owner: Some(me.clone()),
        };
        if self
            .compare_and_set_epoch(resource, current.as_ref(), &new)
            .await?
        {
            Ok(FencingToken {
                resource: resource.to_vec(),
                epoch: next,
                owner: me.clone(),
            })
        } else {
            Err(StoreError::Conflict(
                "another node acquired it first".to_owned(),
            ))
        }
    }

    /// Gives up ownership; returns whether the record was still ours.
    ///
    /// # Errors
    /// The database's errors.
    pub async fn release(&self, token: &FencingToken) -> StoreResult<bool> {
        let expected = EpochRecord {
            epoch: token.epoch,
            owner: Some(token.owner.clone()),
        };
        let cleared = EpochRecord {
            epoch: token.epoch,
            owner: None,
        };
        self.compare_and_set_epoch(&token.resource, Some(&expected), &cleared)
            .await
    }

    /// Checks a token inside a transaction, locking the epoch row until it ends.
    pub(crate) async fn check_fence(
        d: Dialect,
        tx: &mut sqlx::Transaction<'_, sqlx::Any>,
        token: &FencingToken,
    ) -> StoreResult<()> {
        let row = sqlx::query(&format!(
            "SELECT epoch, owner_id, owner_incarnation, owner_addr FROM _epochs WHERE resource = {}{}",
            d.param(1),
            d.for_update()
        ))
        .bind(hex(&token.resource)?)
        .fetch_optional(&mut **tx)
        .await?;
        let current = row.as_ref().map(epoch_of).transpose()?;
        let ok = current
            .as_ref()
            .is_some_and(|r| r.epoch == token.epoch && r.owner.as_ref() == Some(&token.owner));
        if ok {
            Ok(())
        } else {
            Err(StoreError::Fenced {
                current: current.map_or(0, |r| r.epoch),
            })
        }
    }

    /// The lease record of a scheduler shard.
    ///
    /// # Errors
    /// The database's errors.
    pub async fn lease(&self, shard: u32) -> StoreResult<Option<LeaseRecord>> {
        let row = sqlx::query(&format!(
            "SELECT term, holder_id, holder_incarnation, holder_addr, expires_at, cursor_at FROM _leases WHERE shard = {}",
            self.dialect.param(1)
        ))
        .bind(i64::from(shard))
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(lease_of).transpose()
    }

    /// Replaces a shard's lease record with `new` if it currently is `expected`.
    ///
    /// # Errors
    /// The database's errors.
    pub async fn compare_and_set_lease(
        &self,
        shard: u32,
        expected: Option<&LeaseRecord>,
        new: &LeaseRecord,
    ) -> StoreResult<bool> {
        let d = self.dialect;
        let (id, inc, addr) = node_cols(new.holder.as_ref())?;
        let Some(expected) = expected else {
            let inserted = sqlx::query(&format!(
                "INSERT INTO _leases (shard, term, holder_id, holder_incarnation, holder_addr, expires_at, cursor_at) VALUES ({})",
                d.params(1, 7)
            ))
            .bind(i64::from(shard))
            .bind(int(new.term)?)
            .bind(id)
            .bind(inc)
            .bind(addr)
            .bind(int(new.expires_at_ms)?)
            .bind(int(new.cursor_ms)?)
            .execute(&self.pool)
            .await;
            return match inserted {
                Ok(_) => Ok(true),
                Err(error) => match self.lease(shard).await? {
                    Some(_) => Ok(false),
                    None => Err(error.into()),
                },
            };
        };
        if expected == new {
            return Ok(self.lease(shard).await?.as_ref() == Some(new));
        }
        let (eid, einc, eaddr) = node_cols(expected.holder.as_ref())?;
        let holder = expected.holder.is_some();
        let mut n = 10;
        let sql = format!(
            "UPDATE _leases SET term = {}, holder_id = {}, holder_incarnation = {}, holder_addr = {}, \
             expires_at = {}, cursor_at = {} WHERE shard = {} AND term = {} AND expires_at = {} AND cursor_at = {} \
             AND {} AND {} AND {}",
            d.param(1),
            d.param(2),
            d.param(3),
            d.param(4),
            d.param(5),
            d.param(6),
            d.param(7),
            d.param(8),
            d.param(9),
            d.param(10),
            same(d, "holder_id", holder, &mut n),
            same(d, "holder_incarnation", holder, &mut n),
            same(d, "holder_addr", holder, &mut n),
        );
        let mut query = sqlx::query(&sql)
            .bind(int(new.term)?)
            .bind(id)
            .bind(inc)
            .bind(addr)
            .bind(int(new.expires_at_ms)?)
            .bind(int(new.cursor_ms)?)
            .bind(i64::from(shard))
            .bind(int(expected.term)?)
            .bind(int(expected.expires_at_ms)?)
            .bind(int(expected.cursor_ms)?);
        if holder {
            query = query.bind(eid).bind(einc).bind(eaddr);
        }
        Ok(query.execute(&self.pool).await?.rows_affected() == 1)
    }
}

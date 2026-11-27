//! The records of a base in physical tables, and the journal of accepted batches.
//!
//! One physical table per user table, named from ids only: `t_<table>` with `id`,
//! `_version`, `_created`, `_deleted`, and per field `f_<field>` (the value as canonical
//! JSON), `sk_<field>` (the `mt-core` sort key, so ordering is byte-identical on every
//! engine) and `cv_<field>` (the cell version). Tables and columns are added before a write
//! that needs them (DDL is not transactional on MySQL); the write itself — fence check,
//! sequence check, journal row and row changes — is one transaction.

use crate::dialect::Dialect;
use crate::error::{StoreError, StoreResult};
use crate::store::{int, uint, Tenant};
use mt_cluster::FencingToken;
use mt_core::id::{BaseId, FieldId, PrincipalId, RecordId, TableId};
use mt_core::sortkey::sort_key;
use mt_core::value::Value;
use mt_ops::{Accepted, CellState, Effect, RecordState};
use std::collections::{BTreeMap, BTreeSet};

/// A base's records as stored: key → record.
pub type Records = BTreeMap<(TableId, RecordId), RecordState>;

fn table_name(table: TableId) -> String {
    format!("t_{table}")
}

fn columns(field: FieldId) -> (String, String, String) {
    (
        format!("f_{field}"),
        format!("sk_{field}"),
        format!("cv_{field}"),
    )
}

/// A value as stored: canonical JSON and its sort key (`None` for an empty value).
fn stored(value: &Value) -> StoreResult<(Option<String>, Option<Vec<u8>>)> {
    if value.is_empty() {
        return Ok((None, None));
    }
    let json =
        serde_json::to_string(value).map_err(|e| StoreError::Invalid(format!("a value: {e}")))?;
    Ok((Some(json), Some(sort_key(value))))
}

fn effect_target(effect: &Effect) -> (TableId, RecordId) {
    match effect {
        Effect::Created { table, record, .. }
        | Effect::Set { table, record, .. }
        | Effect::Deleted { table, record }
        | Effect::Restored { table, record } => (*table, *record),
    }
}

impl Tenant {
    async fn known_tables(&self) -> StoreResult<BTreeSet<String>> {
        let rows = sqlx::query("SELECT id FROM _tables")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(|r| crate::row::text(r, "id")).collect()
    }

    async fn known_fields(&self, table: TableId) -> StoreResult<BTreeSet<String>> {
        let rows = sqlx::query(&format!(
            "SELECT id FROM _fields WHERE tbl = {}",
            self.dialect.param(1)
        ))
        .bind(table.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(|r| crate::row::text(r, "id")).collect()
    }

    /// The columns a physical table has, as the engine reports them.
    async fn columns_of(&self, table: &str) -> StoreResult<BTreeSet<String>> {
        crate::dialect::check_ident(table)?;
        let sql = match self.dialect {
            Dialect::Sqlite => format!("SELECT name FROM pragma_table_info('{table}')"),
            Dialect::MySql => "SELECT COLUMN_NAME AS name FROM information_schema.COLUMNS \
                 WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ?"
                .to_owned(),
            Dialect::Postgres => {
                "SELECT column_name::text AS name FROM information_schema.columns \
                 WHERE table_schema = current_schema() AND table_name = $1"
                    .to_owned()
            }
        };
        let query = sqlx::query(&sql);
        let query = match self.dialect {
            Dialect::Sqlite => query,
            Dialect::MySql | Dialect::Postgres => query.bind(table.to_owned()),
        };
        let rows = query.fetch_all(&self.pool).await?;
        rows.iter().map(|r| crate::row::text(r, "name")).collect()
    }

    /// Creates the physical table of a user table (once).
    ///
    /// # Errors
    /// The database's errors.
    pub async fn ensure_table(&self, base: BaseId, table: TableId) -> StoreResult<()> {
        if self.known_tables().await?.contains(&table.to_string()) {
            return Ok(());
        }
        let d = self.dialect;
        let name = d.quote(&table_name(table))?;
        sqlx::query(&format!(
            "CREATE TABLE IF NOT EXISTS {name} (id {key} NOT NULL PRIMARY KEY, _version {int} NOT NULL, \
             _created {int} NOT NULL, _deleted {int} NOT NULL){opts}",
            key = d.key(),
            int = d.int(),
            opts = d.table_options()
        ))
        .execute(&self.pool)
        .await?;
        sqlx::query(&format!(
            "INSERT INTO _tables (id, base) VALUES ({})",
            d.params(1, 2)
        ))
        .bind(table.to_string())
        .bind(base.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Adds the columns of a field to its table's physical table (once). Interrupted half
    /// way, it completes the next time.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] when the table has no physical table yet.
    pub async fn ensure_field(&self, table: TableId, field: FieldId) -> StoreResult<()> {
        if !self.known_tables().await?.contains(&table.to_string()) {
            return Err(StoreError::NotFound(format!(
                "the physical table of {table}"
            )));
        }
        if self.known_fields(table).await?.contains(&field.to_string()) {
            return Ok(());
        }
        let d = self.dialect;
        let name = d.quote(&table_name(table))?;
        let (f, sk, cv) = columns(field);
        let present = self.columns_of(&table_name(table)).await?;
        for (column, kind) in [(f, d.text()), (sk, d.blob()), (cv, d.int())] {
            if !present.contains(&column) {
                let quoted = d.quote(&column)?;
                sqlx::query(&format!("ALTER TABLE {name} ADD COLUMN {quoted} {kind}"))
                    .execute(&self.pool)
                    .await?;
            }
        }
        sqlx::query(&format!(
            "INSERT INTO _fields (tbl, id) VALUES ({})",
            d.params(1, 2)
        ))
        .bind(table.to_string())
        .bind(field.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Creates every physical table and column an accepted batch writes to.
    ///
    /// # Errors
    /// The database's errors.
    pub async fn prepare(&self, base: BaseId, accepted: &Accepted) -> StoreResult<()> {
        for effect in &accepted.effects {
            let (table, _) = effect_target(effect);
            self.ensure_table(base, table).await?;
            match effect {
                Effect::Created { cells, .. } => {
                    for field in cells.keys() {
                        self.ensure_field(table, *field).await?;
                    }
                }
                Effect::Set { field, .. } => self.ensure_field(table, *field).await?,
                Effect::Deleted { .. } | Effect::Restored { .. } => {}
            }
        }
        Ok(())
    }

    /// The last committed sequence of a base (0 before the first).
    ///
    /// # Errors
    /// The database's errors.
    pub async fn last_seq(&self, base: BaseId) -> StoreResult<u64> {
        let max: Option<i64> = sqlx::query_scalar(&format!(
            "SELECT MAX(seq) FROM _journal WHERE base = {}",
            self.dialect.param(1)
        ))
        .bind(base.to_string())
        .fetch_one(&self.pool)
        .await?;
        uint(max.unwrap_or(0))
    }

    /// Commits an accepted batch: its journal row and its record changes, in one
    /// transaction — after checking, in that transaction, that the writer still owns the
    /// base (when a token is given) and that the batch is the next in sequence.
    /// [`Tenant::prepare`] must have run for it.
    ///
    /// # Errors
    /// [`StoreError::Fenced`] when another node owns the base now; [`StoreError::Conflict`]
    /// for a batch that is not the next one; the database's errors. On any error nothing is
    /// written.
    pub async fn commit(
        &self,
        base: BaseId,
        accepted: &Accepted,
        actor: PrincipalId,
        at_ms: i64,
        fence: Option<&FencingToken>,
    ) -> StoreResult<()> {
        let d = self.dialect;
        let mut tx = self.pool.begin().await?;
        if let Some(token) = fence {
            Tenant::check_fence(d, &mut tx, token).await?;
        }
        let last: Option<i64> = sqlx::query_scalar(&format!(
            "SELECT MAX(seq) FROM _journal WHERE base = {}",
            d.param(1)
        ))
        .bind(base.to_string())
        .fetch_one(&mut *tx)
        .await?;
        let last = uint(last.unwrap_or(0))?;
        if last.checked_add(1) != Some(accepted.seq) {
            return Err(StoreError::Conflict(format!(
                "batch {} does not follow {last}",
                accepted.seq
            )));
        }
        let body = serde_json::to_string(accepted)
            .map_err(|e| StoreError::Invalid(format!("a batch: {e}")))?;
        sqlx::query(&format!(
            "INSERT INTO _journal (base, seq, client, client_seq, at, actor, body) VALUES ({})",
            d.params(1, 7)
        ))
        .bind(base.to_string())
        .bind(int(accepted.seq)?)
        .bind(int(accepted.id.client.0)?)
        .bind(int(accepted.id.seq)?)
        .bind(at_ms)
        .bind(actor.to_string())
        .bind(body)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            StoreError::Conflict(format!("the journal refused batch {}: {e}", accepted.seq))
        })?;
        let seq = int(accepted.seq)?;
        for effect in &accepted.effects {
            write_effect(d, &mut tx, effect, seq).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Accepted batches of a base after a sequence, in order (catching a client up).
    ///
    /// # Errors
    /// The database's errors; [`StoreError::Corrupt`] for an unreadable entry.
    pub async fn journal(
        &self,
        base: BaseId,
        after: u64,
        limit: usize,
    ) -> StoreResult<Vec<Accepted>> {
        let d = self.dialect;
        let rows = sqlx::query(&format!(
            "SELECT body FROM _journal WHERE base = {} AND seq > {} ORDER BY seq LIMIT {}",
            d.param(1),
            d.param(2),
            d.param(3)
        ))
        .bind(base.to_string())
        .bind(int(after)?)
        .bind(int(u64::try_from(limit).unwrap_or(u64::MAX))?)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                let body = crate::row::text(row, "body")?;
                serde_json::from_str(&body)
                    .map_err(|e| StoreError::Corrupt(format!("journal: {e}")))
            })
            .collect()
    }

    /// The last `count` accepted batches of a base, oldest first (to recognise repeats after
    /// a restart).
    ///
    /// # Errors
    /// As [`Tenant::journal`].
    pub async fn journal_tail(&self, base: BaseId, count: usize) -> StoreResult<Vec<Accepted>> {
        let last = self.last_seq(base).await?;
        let from = last.saturating_sub(u64::try_from(count).unwrap_or(u64::MAX));
        self.journal(base, from, count).await
    }

    /// Every record of a base and its last sequence.
    ///
    /// # Errors
    /// The database's errors; [`StoreError::Corrupt`] for unreadable rows.
    pub async fn load_base(&self, base: BaseId) -> StoreResult<(u64, Records)> {
        let d = self.dialect;
        let tables = sqlx::query(&format!(
            "SELECT id FROM _tables WHERE base = {} ORDER BY id",
            d.param(1)
        ))
        .bind(base.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut records = Records::new();
        for row in &tables {
            let table_text = crate::row::text(row, "id")?;
            let table: TableId = table_text
                .parse()
                .map_err(|_| StoreError::Corrupt(format!("table id `{table_text}`")))?;
            let fields: Vec<FieldId> = self
                .known_fields(table)
                .await?
                .iter()
                .map(|f| {
                    f.parse()
                        .map_err(|_| StoreError::Corrupt(format!("field id `{f}`")))
                })
                .collect::<StoreResult<_>>()?;
            let mut select = vec![
                "id".to_owned(),
                "_version".to_owned(),
                "_created".to_owned(),
                "_deleted".to_owned(),
            ];
            for field in &fields {
                let (f, _, cv) = columns(*field);
                select.push(d.quote(&f)?);
                select.push(d.quote(&cv)?);
            }
            let rows = sqlx::query(&format!(
                "SELECT {} FROM {} ORDER BY id",
                select.join(", "),
                d.quote(&table_name(table))?
            ))
            .fetch_all(&self.pool)
            .await?;
            for row in &rows {
                let id_text = crate::row::text(row, "id")?;
                let record: RecordId = id_text
                    .parse()
                    .map_err(|_| StoreError::Corrupt(format!("record id `{id_text}`")))?;
                let mut cells = BTreeMap::new();
                for field in &fields {
                    let (f, _, cv) = columns(*field);
                    if let Some(json) = crate::row::opt_text(row, &f)? {
                        let value: Value = serde_json::from_str(&json)
                            .map_err(|e| StoreError::Corrupt(format!("{table}.{field}: {e}")))?;
                        let version = uint(crate::row::opt_int(row, &cv)?.unwrap_or(0))?;
                        cells.insert(*field, CellState { value, version });
                    }
                }
                records.insert(
                    (table, record),
                    RecordState {
                        cells,
                        version: uint(crate::row::int(row, "_version")?)?,
                        created: uint(crate::row::int(row, "_created")?)?,
                        deleted: crate::row::int(row, "_deleted")? != 0,
                    },
                );
            }
        }
        Ok((self.last_seq(base).await?, records))
    }

    /// Record ids of a table in the order of a field's values (empty values last), as the
    /// database sorts the stored sort keys — the same order on every engine.
    ///
    /// # Errors
    /// The database's errors.
    pub async fn ids_by(&self, table: TableId, field: FieldId) -> StoreResult<Vec<RecordId>> {
        let d = self.dialect;
        let (_, sk, _) = columns(field);
        let sk = d.quote(&sk)?;
        let rows = sqlx::query(&format!(
            "SELECT id FROM {} WHERE _deleted = 0 ORDER BY CASE WHEN {sk} IS NULL THEN 1 ELSE 0 END, {sk}, id",
            d.quote(&table_name(table))?
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                let text = crate::row::text(row, "id")?;
                text.parse()
                    .map_err(|_| StoreError::Corrupt(format!("record id `{text}`")))
            })
            .collect()
    }
}

async fn write_effect(
    d: Dialect,
    tx: &mut sqlx::Transaction<'_, sqlx::Any>,
    effect: &Effect,
    seq: i64,
) -> StoreResult<()> {
    let (table, record) = effect_target(effect);
    let name = d.quote(&table_name(table))?;
    let changed = match effect {
        Effect::Created { cells, .. } => {
            let mut cols = vec![
                "id".to_owned(),
                "_version".to_owned(),
                "_created".to_owned(),
                "_deleted".to_owned(),
            ];
            let mut values = Vec::new();
            for (field, value) in cells {
                let (f, sk, cv) = columns(*field);
                cols.push(d.quote(&f)?);
                cols.push(d.quote(&sk)?);
                cols.push(d.quote(&cv)?);
                values.push(stored(value)?);
            }
            let sql = format!(
                "INSERT INTO {name} ({}) VALUES ({})",
                cols.join(", "),
                d.params(1, cols.len())
            );
            let mut query = sqlx::query(&sql)
                .bind(record.to_string())
                .bind(seq)
                .bind(seq)
                .bind(0_i64);
            for (json, key) in values {
                let present = json.is_some();
                query = query.bind(json).bind(key).bind(present.then_some(seq));
            }
            query.execute(&mut **tx).await?.rows_affected()
        }
        Effect::Set { field, after, .. } => {
            let (f, sk, cv) = columns(*field);
            let (json, key) = stored(after)?;
            let present = json.is_some();
            sqlx::query(&format!(
                "UPDATE {name} SET {} = {}, {} = {}, {} = {}, _version = {} WHERE id = {}",
                d.quote(&f)?,
                d.param(1),
                d.quote(&sk)?,
                d.param(2),
                d.quote(&cv)?,
                d.param(3),
                d.param(4),
                d.param(5)
            ))
            .bind(json)
            .bind(key)
            .bind(present.then_some(seq))
            .bind(seq)
            .bind(record.to_string())
            .execute(&mut **tx)
            .await?
            .rows_affected()
        }
        Effect::Deleted { .. } | Effect::Restored { .. } => {
            let deleted = i64::from(matches!(effect, Effect::Deleted { .. }));
            sqlx::query(&format!(
                "UPDATE {name} SET _deleted = {}, _version = {} WHERE id = {}",
                d.param(1),
                d.param(2),
                d.param(3)
            ))
            .bind(deleted)
            .bind(seq)
            .bind(record.to_string())
            .execute(&mut **tx)
            .await?
            .rows_affected()
        }
    };
    if changed == 1 {
        Ok(())
    } else {
        Err(StoreError::Conflict(format!(
            "{table}/{record} does not fit the stored rows"
        )))
    }
}

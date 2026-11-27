//! Schemas and their migrations. Each schema is a list of numbered steps; a database records
//! the steps it has in `_schema` and gets the missing ones, in order, when it is opened.
//! Steps are idempotent (`IF NOT EXISTS`), so a step interrupted half-way can run again.

use crate::dialect::Dialect;
use crate::error::StoreResult;
use sqlx::AnyPool;

/// The control schema's version.
pub const CONTROL_VERSION: i64 = 1;
/// The tenant schema's version.
pub const TENANT_VERSION: i64 = 2;

fn index(d: Dialect, name: &str, table: &str, columns: &str) -> String {
    match d {
        // MySQL has no CREATE INDEX IF NOT EXISTS: the index is created with its table.
        Dialect::MySql => String::new(),
        Dialect::Sqlite | Dialect::Postgres => {
            format!("CREATE INDEX IF NOT EXISTS {name} ON {table} ({columns})")
        }
    }
}

fn inline_index(d: Dialect, name: &str, columns: &str) -> String {
    match d {
        Dialect::MySql => format!(", INDEX {name} ({columns})"),
        Dialect::Sqlite | Dialect::Postgres => String::new(),
    }
}

fn schema_table(d: Dialect) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS _schema (version {int} NOT NULL PRIMARY KEY, applied_at {int} NOT NULL){opts}",
        int = d.int(),
        opts = d.table_options()
    )
}

/// The control database: the tenants of this installation.
fn control_steps(d: Dialect, version: i64) -> Vec<String> {
    match version {
        1 => vec![format!(
            "CREATE TABLE IF NOT EXISTS tenants (id {key} NOT NULL PRIMARY KEY, name {text} NOT NULL, \
             status {key} NOT NULL, db_name {key} NOT NULL, schema_version {int} NOT NULL, \
             created_at {int} NOT NULL){opts}",
            key = d.key(),
            text = d.text(),
            int = d.int(),
            opts = d.table_options()
        )],
        _ => Vec::new(),
    }
}

/// A tenant database: documents, the operation journal, ownership epochs, scheduler
/// leases, jobs, and the registry of physical tables.
fn tenant_steps(d: Dialect, version: i64) -> Vec<String> {
    let (key, text, int, opts) = (d.key(), d.text(), d.int(), d.table_options());
    match version {
        1 => {
            let mut steps = vec![
                format!(
                    "CREATE TABLE IF NOT EXISTS _documents (kind {key} NOT NULL, id {key} NOT NULL, \
                     version {int} NOT NULL, body {text} NOT NULL, updated_at {int} NOT NULL, \
                     PRIMARY KEY (kind, id)){opts}"
                ),
                format!(
                    "CREATE TABLE IF NOT EXISTS _journal (base {key} NOT NULL, seq {int} NOT NULL, \
                     client {int} NOT NULL, client_seq {int} NOT NULL, at {int} NOT NULL, \
                     actor {key} NOT NULL, body {text} NOT NULL, PRIMARY KEY (base, seq), \
                     UNIQUE (base, client, client_seq)){opts}"
                ),
                format!(
                    "CREATE TABLE IF NOT EXISTS _epochs (resource {key} NOT NULL PRIMARY KEY, \
                     epoch {int} NOT NULL, owner_id {int}, owner_incarnation {int}, owner_addr {key}){opts}"
                ),
                format!(
                    "CREATE TABLE IF NOT EXISTS _leases (shard {int} NOT NULL PRIMARY KEY, term {int} NOT NULL, \
                     holder_id {int}, holder_incarnation {int}, holder_addr {key}, \
                     expires_at {int} NOT NULL, cursor_at {int} NOT NULL){opts}"
                ),
                format!(
                    "CREATE TABLE IF NOT EXISTS _jobs (id {key} NOT NULL PRIMARY KEY, queue {key} NOT NULL, \
                     payload {text} NOT NULL, state {key} NOT NULL, attempts {int} NOT NULL, \
                     max_attempts {int} NOT NULL, run_at {int} NOT NULL, lease_owner {key}, \
                     lease_until {int}, token {int} NOT NULL, last_error {text}, created_at {int} NOT NULL, \
                     finished_at {int}, trace {key}{idx}){opts}",
                    idx = inline_index(d, "_jobs_due", "queue, state, run_at")
                ),
                format!(
                    "CREATE TABLE IF NOT EXISTS _tables (id {key} NOT NULL PRIMARY KEY, base {key} NOT NULL){opts}"
                ),
                format!(
                    "CREATE TABLE IF NOT EXISTS _fields (tbl {key} NOT NULL, id {key} NOT NULL, \
                     PRIMARY KEY (tbl, id)){opts}"
                ),
            ];
            steps.push(index(d, "_jobs_due", "_jobs", "queue, state, run_at"));
            steps.retain(|s| !s.is_empty());
            steps
        }
        // Refused batches, so every later owner of a base answers a batch sent again the same
        // way (found by the whole-system simulation, `mt-sim`).
        2 => vec![format!(
            "CREATE TABLE IF NOT EXISTS _refusals (base {key} NOT NULL, client {int} NOT NULL, \
             client_seq {int} NOT NULL, at {int} NOT NULL, body {text} NOT NULL, \
             PRIMARY KEY (base, client, client_seq)){opts}"
        )],
        _ => Vec::new(),
    }
}

/// Which schema to migrate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The control database.
    Control,
    /// A tenant database.
    Tenant,
}

/// Brings a database to the latest version of its schema; returns the version.
///
/// # Errors
/// The database's.
pub async fn migrate(pool: &AnyPool, d: Dialect, kind: Kind, now_ms: i64) -> StoreResult<i64> {
    sqlx::query(&schema_table(d)).execute(pool).await?;
    let current: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM _schema")
        .fetch_one(pool)
        .await?;
    let latest = match kind {
        Kind::Control => CONTROL_VERSION,
        Kind::Tenant => TENANT_VERSION,
    };
    let mut version = current.unwrap_or(0);
    while version < latest {
        let next = version.saturating_add(1);
        let steps = match kind {
            Kind::Control => control_steps(d, next),
            Kind::Tenant => tenant_steps(d, next),
        };
        for step in steps {
            sqlx::query(&step).execute(pool).await?;
        }
        sqlx::query(&format!(
            "INSERT INTO _schema (version, applied_at) VALUES ({})",
            d.params(1, 2)
        ))
        .bind(next)
        .bind(now_ms)
        .execute(pool)
        .await?;
        version = next;
    }
    Ok(version)
}

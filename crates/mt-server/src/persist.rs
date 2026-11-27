//! Where the workspace is kept: nowhere (tests), a JSON file (the first builds), or a
//! database through `mt-store` — SQLite by default, MySQL or PostgreSQL by URL.
//!
//! In a database, records live in physical tables and every change is an `mt-ops` batch:
//! each save compares the workspace's records with what each base has stored and turns the
//! differences into creates, sets and deletes, which get the base's next sequence numbers
//! and journal entries. The configuration (bases, tables, fields, people, access,
//! processes, settings, the audit chain) is one JSON document. Both are written in one
//! transaction, behind each base's fencing token: if another server takes the database
//! over, this one's writes are refused.

use crate::error::{ApiError, ApiResult};
use crate::model::{Data, Record, Workspace};
use mt_cluster::{FencingToken, NodeId};
use mt_core::decimal::Decimal;
use mt_core::id::{BaseId, FieldId, RecordId, TableId, TenantId};
use mt_core::time::Timestamp;
use mt_core::value::Value;
use mt_ops::{Accepted, BaseState, Batch, ClientId, ClientOpId, Op, Outcome, RecordState, MAX_OPS};
use mt_store::{DocumentWrite, Store, StoreConfig, StoreError, Tenant, WriteSet};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use tokio::sync::Mutex;

/// The one tenant of a single-workspace installation.
pub const TENANT: TenantId = TenantId::from_u128(1);
/// The document holding the configuration.
const DOCUMENT: (&str, &str) = ("workspace", "main");
/// The server's own client number for the batches it writes.
const SERVER: ClientId = ClientId(0);

/// Record metadata travels as reserved cells (never real field ids).
const CREATED_BY: FieldId = FieldId::from_u128(u128::MAX - 1);
const CREATED_AT: FieldId = FieldId::from_u128(u128::MAX - 2);
const MODIFIED_BY: FieldId = FieldId::from_u128(u128::MAX - 3);
const MODIFIED_AT: FieldId = FieldId::from_u128(u128::MAX - 4);
const VERSION: FieldId = FieldId::from_u128(u128::MAX - 5);

fn store_error(error: StoreError) -> ApiError {
    match error {
        StoreError::Fenced { .. } => ApiError::unavailable(
            "another server has taken this workspace over: this one no longer saves changes",
        ),
        StoreError::Db(_)
        | StoreError::Invalid(_)
        | StoreError::NotFound(_)
        | StoreError::Exists(_)
        | StoreError::Conflict(_)
        | StoreError::LeaseLost(_)
        | StoreError::Corrupt(_) => ApiError::internal(format!("storage: {error}")),
    }
}

fn cells_of(record: &Record) -> BTreeMap<FieldId, Value> {
    let mut cells: BTreeMap<FieldId, Value> = record
        .cells
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(f, v)| (*f, v.clone()))
        .collect();
    cells.insert(CREATED_BY, Value::Principal(vec![record.created_by]));
    cells.insert(CREATED_AT, Value::DateTime(record.created_at));
    cells.insert(MODIFIED_BY, Value::Principal(vec![record.modified_by]));
    cells.insert(MODIFIED_AT, Value::DateTime(record.modified_at));
    cells.insert(VERSION, Value::Number(Decimal::from(record.version)));
    cells
}

fn record_of(id: RecordId, state: &RecordState) -> ApiResult<Record> {
    let corrupt = |what: &str| ApiError::internal(format!("storage: record {id} has no {what}"));
    let person = |field: FieldId, what: &str| match state.cells.get(&field).map(|c| &c.value) {
        Some(Value::Principal(people)) => people.first().copied().ok_or_else(|| corrupt(what)),
        Some(_) | None => Err(corrupt(what)),
    };
    let instant = |field: FieldId, what: &str| match state.cells.get(&field).map(|c| &c.value) {
        Some(Value::DateTime(at)) => Ok(*at),
        Some(_) | None => Err(corrupt(what)),
    };
    let version = match state.cells.get(&VERSION).map(|c| &c.value) {
        Some(Value::Number(n)) => u64::try_from(*n).map_err(|_| corrupt("version"))?,
        Some(_) | None => return Err(corrupt("version")),
    };
    Ok(Record {
        id,
        cells: state
            .cells
            .iter()
            .filter(|(f, _)| !is_meta(**f))
            .map(|(f, c)| (*f, c.value.clone()))
            .collect(),
        errors: BTreeMap::new(),
        created_by: person(CREATED_BY, "creator")?,
        created_at: instant(CREATED_AT, "creation time")?,
        modified_by: person(MODIFIED_BY, "last editor")?,
        modified_at: instant(MODIFIED_AT, "change time")?,
        version,
    })
}

fn is_meta(field: FieldId) -> bool {
    [CREATED_BY, CREATED_AT, MODIFIED_BY, MODIFIED_AT, VERSION].contains(&field)
}

type Current = BTreeMap<(TableId, RecordId), BTreeMap<FieldId, Value>>;

/// The operations that turn what a base has stored into what the workspace holds.
fn diff(state: &BaseState, current: &Current) -> Vec<Op> {
    let mut ops = Vec::new();
    for ((table, record), cells) in current {
        let (table, record) = (*table, *record);
        let Some(stored) = state.record(table, record) else {
            ops.push(Op::Create {
                table,
                record,
                cells: cells.clone(),
            });
            continue;
        };
        if stored.deleted {
            ops.push(Op::Restore { table, record });
        }
        let fields: BTreeSet<FieldId> = stored.cells.keys().chain(cells.keys()).copied().collect();
        for field in fields {
            let now = cells.get(&field).cloned().unwrap_or(Value::Null);
            if now != stored.value(field) {
                ops.push(Op::Set {
                    table,
                    record,
                    field,
                    value: now,
                    expected: stored.cell_version(field),
                });
            }
        }
    }
    for ((table, record), stored) in state.records() {
        if !stored.deleted && !current.contains_key(&(*table, *record)) {
            ops.push(Op::Delete {
                table: *table,
                record: *record,
                expected: stored.version,
            });
        }
    }
    ops
}

/// What a database-backed workspace remembers between saves.
#[derive(Debug, Default)]
struct Kept {
    document: Option<(u64, String)>,
    bases: BTreeMap<BaseId, BaseState>,
    fences: BTreeMap<BaseId, FencingToken>,
    fenced: bool,
}

/// A workspace kept in a database.
#[derive(Debug)]
pub struct Stored {
    store: Store,
    tenant: Tenant,
    node: NodeId,
    kept: Mutex<Kept>,
}

/// Where the workspace is kept.
#[derive(Debug)]
pub enum Persist {
    /// Not kept (tests, demos).
    Memory,
    /// A JSON file.
    File(PathBuf),
    /// A database.
    Store(Box<Stored>),
}

fn document_of(data: &Data) -> ApiResult<String> {
    let mut value = serde_json::to_value(data).map_err(|e| ApiError::internal(e.to_string()))?;
    if let Some(tables) = value
        .get_mut("tables")
        .and_then(serde_json::Value::as_array_mut)
    {
        for table in tables {
            if let Some(map) = table.as_object_mut() {
                map.remove("records");
            }
        }
    }
    serde_json::to_string(&value).map_err(|e| ApiError::internal(e.to_string()))
}

fn millis(at: Timestamp) -> i64 {
    at.millis()
}

impl Stored {
    async fn fence(&self, kept: &mut Kept, base: BaseId) -> ApiResult<()> {
        if kept.fences.contains_key(&base) {
            return Ok(());
        }
        let token = self
            .tenant
            .acquire(format!("base:{base}").as_bytes(), &self.node, None)
            .await
            .map_err(store_error)?;
        kept.fences.insert(base, token);
        Ok(())
    }

    /// Writes what changed since the last save, in one transaction.
    async fn save(&self, ws: &Workspace, now: Timestamp) -> ApiResult<()> {
        let mut kept = self.kept.lock().await;
        if kept.fenced {
            return Err(store_error(StoreError::Fenced { current: 0 }));
        }
        let mut current: BTreeMap<BaseId, Current> = BTreeMap::new();
        for table in &ws.data.tables {
            let base = current.entry(table.base).or_default();
            for record in &table.records {
                base.insert((table.id, record.id), cells_of(record));
            }
        }
        let bases: BTreeSet<BaseId> = kept.bases.keys().chain(current.keys()).copied().collect();
        let empty = Current::new();
        let mut next: BTreeMap<BaseId, BaseState> = BTreeMap::new();
        let mut accepted: Vec<(BaseId, Accepted)> = Vec::new();
        for base in bases {
            self.fence(&mut kept, base).await?;
            let mut state = kept
                .bases
                .get(&base)
                .cloned()
                .unwrap_or_else(|| BaseState::new(base));
            let ops = diff(&state, current.get(&base).unwrap_or(&empty));
            if ops.is_empty() {
                continue;
            }
            for chunk in ops.chunks(MAX_OPS) {
                let batch = Batch {
                    id: ClientOpId {
                        client: SERVER,
                        seq: state.seq().saturating_add(1),
                    },
                    base,
                    ops: chunk.to_vec(),
                };
                match state.step(&batch).outcome {
                    Outcome::Accepted(a) => accepted.push((base, a)),
                    Outcome::Rejected(r) => {
                        return Err(ApiError::internal(format!(
                            "storage: the stored records of base {base} disagree with the workspace ({})",
                            r.reason.text()
                        )))
                    }
                }
            }
            next.insert(base, state);
        }
        let text = document_of(&ws.data)?;
        let (version, changed) = match &kept.document {
            Some((version, old)) => (*version, *old != text),
            None => (0, true),
        };
        if accepted.is_empty() && !changed {
            return Ok(());
        }
        for (base, batch) in &accepted {
            self.tenant
                .prepare(*base, batch)
                .await
                .map_err(store_error)?;
        }
        let set = WriteSet {
            batches: accepted
                .iter()
                .map(|(base, a)| mt_store::Batch {
                    base: *base,
                    accepted: a,
                    fence: kept.fences.get(base),
                })
                .collect(),
            document: changed.then_some(DocumentWrite {
                kind: DOCUMENT.0,
                id: DOCUMENT.1,
                body: &text,
                expected: version,
            }),
        };
        match self
            .tenant
            .write(&set, crate::settings::SYSTEM, millis(now))
            .await
        {
            Ok(written) => {
                if let Some(version) = written {
                    kept.document = Some((version, text));
                }
                kept.bases.extend(next);
                Ok(())
            }
            Err(error) => {
                if matches!(error, StoreError::Fenced { .. }) {
                    kept.fenced = true;
                }
                Err(store_error(error))
            }
        }
    }
}

impl Persist {
    /// Saves the workspace after a change.
    ///
    /// # Errors
    /// When it cannot be written; 503 when another server has taken a database over.
    pub async fn save(&self, ws: &Workspace, now: Timestamp) -> ApiResult<()> {
        match self {
            Persist::Memory => Ok(()),
            Persist::File(path) => {
                let text = serde_json::to_vec_pretty(&ws.data)
                    .map_err(|e| ApiError::internal(e.to_string()))?;
                let temporary = path.with_extension("json.tmp");
                std::fs::write(&temporary, text).map_err(|e| ApiError::internal(e.to_string()))?;
                std::fs::rename(&temporary, path).map_err(|e| ApiError::internal(e.to_string()))
            }
            Persist::Store(stored) => stored.save(ws, now).await,
        }
    }

    /// The database's engine, when kept in one.
    #[must_use]
    pub fn engine(&self) -> Option<&'static str> {
        match self {
            Persist::Store(stored) => Some(stored.store.dialect().name()),
            Persist::Memory | Persist::File(_) => None,
        }
    }

    /// Closes the database connections.
    pub async fn close(&self) {
        if let Persist::Store(stored) = self {
            stored.store.close().await;
        }
    }
}

/// How to open a database-backed workspace.
#[derive(Debug, Clone)]
pub struct Opening {
    /// The database.
    pub config: StoreConfig,
    /// Start over: the stored workspace is deleted first.
    pub fresh: bool,
    /// A workspace saved as JSON, to put into a new database.
    pub import: Option<Data>,
    /// Use `import` only for a new database (carrying over the data file of an earlier
    /// version); otherwise an import into a database that has a workspace is refused.
    pub import_if_new: bool,
}

/// Opens (or creates) the workspace in a database. A new one starts as `fresh` says, or from
/// the imported data; an existing one is loaded — its records from the physical tables —
/// and this server becomes the owner of each base.
///
/// # Errors
/// When the database cannot be opened or read, or an import meets an existing workspace.
pub async fn open(
    opening: Opening,
    now: Timestamp,
    new: impl FnOnce() -> ApiResult<Workspace>,
) -> ApiResult<(Workspace, Persist)> {
    let at = millis(now);
    let store = Store::open(&opening.config, at)
        .await
        .map_err(store_error)?;
    if opening.fresh && store.info(TENANT).await.map_err(store_error)?.is_some() {
        store.drop_tenant(TENANT).await.map_err(store_error)?;
    }
    let tenant = if store.info(TENANT).await.map_err(store_error)?.is_some() {
        store.tenant(TENANT, at).await.map_err(store_error)?
    } else {
        store
            .create_tenant(TENANT, "Workspace", at)
            .await
            .map_err(store_error)?
    };
    let node = NodeId::new(1, u64::try_from(at).unwrap_or(0), "local");
    let document = tenant
        .document(DOCUMENT.0, DOCUMENT.1)
        .await
        .map_err(store_error)?;
    let stored = Stored {
        store,
        tenant,
        node,
        kept: Mutex::new(Kept::default()),
    };
    let ws = match document {
        None => match opening.import {
            Some(data) => {
                let mut ws = Workspace::new(data)?;
                ws.recompute(now);
                ws
            }
            None => new()?,
        },
        Some(doc) => {
            if opening.import.is_some() && !opening.import_if_new {
                return Err(ApiError::conflict(
                    "the database holds a workspace already: start with --fresh to replace it",
                ));
            }
            let mut data: Data = serde_json::from_str(&doc.body)
                .map_err(|e| ApiError::internal(format!("storage: the workspace: {e}")))?;
            let bases: BTreeSet<BaseId> = data
                .bases
                .iter()
                .map(|b| b.id)
                .chain(data.tables.iter().map(|t| t.base))
                .collect();
            let mut kept = stored.kept.lock().await;
            for base in bases {
                let (seq, records) = stored.tenant.load_base(base).await.map_err(store_error)?;
                for table in data.tables.iter_mut().filter(|t| t.base == base) {
                    table.records = records
                        .iter()
                        .filter(|((t, _), r)| *t == table.id && !r.deleted)
                        .map(|((_, id), r)| record_of(*id, r))
                        .collect::<ApiResult<_>>()?;
                }
                kept.bases
                    .insert(base, BaseState::from_records(base, seq, records));
            }
            kept.document = Some((doc.version, doc.body));
            drop(kept);
            let mut ws = Workspace::new(data)?;
            ws.recompute(now);
            ws
        }
    };
    let persist = Persist::Store(Box::new(stored));
    persist.save(&ws, now).await?;
    Ok((ws, persist))
}

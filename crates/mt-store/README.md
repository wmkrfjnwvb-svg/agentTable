# mt-store

Storage on **SQLite, MySQL 8 and PostgreSQL** behind one code path: a control database for the
installation, **one database per tenant**, the operation journal, physical tables for the
user's tables, ownership epochs (fencing), scheduler leases and a **job queue with leases**.

## Public API

| Item | What it is |
|---|---|
| `Store::open(&StoreConfig, now)` | Opens (creating and migrating) the control database. URL: `sqlite://<dir>`, `mysql://…`, `postgres://…`; `prefix` names the databases (`mt_control`, `mt_t_<tenant>`) |
| `Store::create_tenant` / `tenant` / `tenants` / `info` / `drop_tenant` / `destroy` / `close` | Tenants and their databases. A tenant opened on an older schema is migrated first |
| `Tenant::document` / `documents` / `put_document` / `delete_document` | Configuration documents (JSON) with an optimistic version |
| `Tenant::ensure_table` / `ensure_field` / `prepare` | Physical tables `t_<table>` and columns `f_` (value as canonical JSON), `sk_` (sort key), `cv_` (cell version) per field — names from ids only |
| `Tenant::commit(base, accepted, actor, at, fence)` | One accepted `mt-ops` batch in one transaction: fence check, sequence check, journal row, row changes |
| `Tenant::load_base` / `last_seq` / `journal` / `journal_tail` | Reading a base back (to rebuild `mt_ops::BaseState`), catch-up and repeat recognition after a restart |
| `Tenant::ids_by(table, field)` | Record order by the stored sort keys |
| `Tenant::epoch` / `compare_and_set_epoch` / `acquire` / `release` | Ownership epochs (as `mt-cluster`'s `EpochStore` / `Fencer`) |
| `Tenant::lease` / `compare_and_set_lease` | Scheduler shard leases (as `mt-cluster`'s `LeaseStore`) |
| `Tenant::enqueue` / `claim` / `heartbeat` / `complete` / `fail` / `revive` / `job` / `jobs` | The job queue; `jobs::backoff` |
| `Dialect` | Quoting, placeholders, column types per engine |

## Invariants (each tested on all three engines)

* **Isolation.** Each tenant has its own database (file); dropping a tenant drops it (erasure).
* **Names from ids only.** Every identifier is checked (`[a-z][a-z0-9_]*`, ≤ 63 bytes) and
  quoted per dialect; values always go through bind parameters.
* **One transaction per batch.** A commit writes the journal row and every row change, or
  nothing: a batch whose effects do not fit the rows is rolled back whole.
* **No gaps, no repeats.** A batch must be the next in its base's sequence; the same client
  batch cannot be journaled twice (a unique key), even when committed from many tasks at once.
* **Fencing.** With a token, a commit first checks — and locks, on MySQL and PostgreSQL — the
  base's epoch row in the same transaction, so an owner that was superseded cannot commit.
  Racing nodes each get a distinct epoch.
* **Same answers as the reference.** Epoch and lease compare-and-set give the same results and
  records as `mt-cluster`'s in-memory stores (the simulator's reference).
* **Same order everywhere.** Records sort by the stored `mt-core` sort keys, so the order is
  byte-identical on SQLite, MySQL and PostgreSQL (and equals `compare_values`).
* **Jobs reach a terminal state.** A claim is a lease with a token; a stalled worker can
  neither finish, fail nor extend a job another worker took over; a failure waits 1 s, 2 s,
  4 s, … up to an hour; after the last attempt (or when its worker stops answering) a job is
  `dead`, and can be revived. Claims skip rows locked by other workers on MySQL and
  PostgreSQL; SQLite has a single writer (one connection per file).
* **Damaged data is reported, not trusted** (`Corrupt`): half-recorded owners, negative
  numbers, unknown states, unreadable ids.

## Decisions

* **`sqlx` Any driver + a small dialect layer instead of `sea-query`** (doc 20 named both). The
  store runs a fixed, small set of statement shapes; one pool type for three engines keeps a
  single code path, and every statement is exercised on every engine by the tests.
* MySQL sends long text with a BLOB wire type; text is read as a string or UTF-8 bytes.
* DDL (new tables and columns) runs before the write transaction: MySQL commits implicitly on
  DDL. `ensure_field` completes a column set added half-way.
* A pool per tenant database (MySQL too, rather than one shared pool with qualified names);
  pool limits and LRU closing come with many-tenant hosting.

## Tests

Every test runs on each engine in databases of its own (prefix per run), removed afterwards.
SQLite always runs; MySQL and PostgreSQL run when `MT_TEST_MYSQL_URL` / `MT_TEST_POSTGRES_URL`
are set (`scripts/dev-db.sh` starts both locally); `MT_TEST_REQUIRE_ALL_DBS=1` (CI) makes a
missing one fail.

| File | Category | What |
|---|---|---|
| `tests/store.rs` | standard, invalid | Tenants and databases, reopening, bad URLs and names |
| `tests/fencing.rs` | standard, model-based | Documents, epochs, leases; 120 random compare-and-set steps against `mt-cluster`'s reference |
| `tests/jobs.rs` | standard, edge, invalid, model-based, concurrency | Lifecycle, backoff, stalled workers, dead jobs and revival; 150 random steps against a reference; 6 workers claiming 60 jobs at once — each runs once |
| `tests/records.rs` | standard, edge, invalid, concurrency | Every value type loads back exactly, journal and catch-up, restart with repeat recognition, fencing, sequence conflicts and rollback, one batch committed from 6 tasks lands once, sort order equal to the engine's |
| `tests/maintenance.rs` | anomalous, concurrency | Damaged rows, an older schema migrated on open, unreachable servers time out, 8 nodes racing for ownership |

Coverage: 97.5 % of lines (the rest are defensive error branches).

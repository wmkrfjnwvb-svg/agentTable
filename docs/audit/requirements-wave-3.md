# Requirements audit — wave 3 (`mt-ops`, `mt-store`, job queue with leases, `mt-sim`, the product on a database)

Audit date: 2026-11-27. Audited against: doc 21 (development plan: wave 3 scope "operation
pipeline on real databases; simulated multi-node, multi-client runs", §4 simulation strategy,
§5 concurrency and security checklist, §6 definition of done), doc 20 §4 (crates, runtime model),
doc 04 §3 (database per tenant, physical-table mapping) and §6 (three-database harness), doc 03
§4.2 (key flows: edit a cell, conflicts), and the wave-1 deferral R1-10 (job queue with leases).

Method as before: **met** only when a public type or function implements it *and* a test
exercises it — for storage, on SQLite, MySQL and PostgreSQL.

## 1. Result

| Crate | Tests | Line coverage | Lint | WebAssembly | README | Verdict |
|---|---|---|---|---|---|---|
| `mt-ops` | 23 | 99.3% | clean | builds | yes | Meets wave-3 scope |
| `mt-store` | 23 (each on 3 engines) | 97.6% (below the 98% target: defensive error branches) | clean | n/a (native) | yes | Meets wave-3 scope with the deviations in §3 |
| `mt-sim` | 5 (150 seeds each for the main runs) | 94.8% (the rest are violation reports) | clean | builds | yes | Meets wave-3 scope |
| `mt-server` + `apps/web` | 52 API (3 on 3 engines) + 47 browser steps | ≥ 90% (gate) | clean | n/a | yes | The product keeps its workspace in a database |

All Rust tests pass locally with MySQL 8.0 and PostgreSQL 16, and in CI with MySQL 8.4 and
PostgreSQL 16 service containers (`MT_TEST_REQUIRE_ALL_DBS=1`: a missing engine fails).

## 2. Traceability

| Requirement | Where | Status |
|---|---|---|
| Operation model, client op ids, apply/rebase (doc 20 §4.1) | `mt-ops`: `Op`, `Batch`, `ClientOpId`, `BaseState::step`, `Replica` | Met |
| Binary codec (postcard) | `mt-ops::codec` (values as canonical JSON inside postcard) | Met |
| Edit a cell: stale `expected_cell_version` commits, sender told what it overwrote; transitions reject on stale (doc 03 §4.2) | `Effect::Set { overwritten }`, `Op::Transition`, `Op::Delete` | Met |
| Base actor assigns the per-base sequence (doc 20 §4.2) | `BaseState` (sequence without gaps), server per-base states | Met |
| Database per tenant; SQLite file / MySQL & PostgreSQL databases (doc 04 §3.1) | `mt-store::Store` | Met |
| Migrations per tenant, lazily on access when behind | `schema::migrate`, `Store::tenant` | Met (tested: schema 1 → 2) |
| Physical tables from ids only; sort keys computed in Rust (doc 04 §3.2) | `t_<table>`, `f_`, `sk_`, `cv_` | **Partly**: values are stored as canonical JSON text, not one typed column per field type; no `mv_`/`ln_` junction tables yet (R3-01) |
| Byte-identical order on every engine | `ids_by` against `compare_values` | Met |
| Fencing epoch per base checked in the write transaction (doc 21 §3) | `Tenant::acquire`, `commit`, `write` | Met |
| Job queue with leases, `SKIP LOCKED`, terminal states (doc 20 §4.2; R1-10) | `mt-store::jobs` | Met — **R1-10 closed** |
| Scheduler leases (doc 21 §3) | `Tenant::lease`, `compare_and_set_lease` (as `mt-cluster`'s `LeaseStore`) | Met |
| Deterministic simulator with crashes, partitions, loss, duplication (doc 21 §4.2) | `mt-sim` | Met |
| Invariants: one writer per epoch; no op lost or applied twice; replicas converge; terminal states (doc 21 §4.3) | `mt-sim` checks 1–5 | Met (audit chain and permissions: checked in the product tests, not in the simulator — R3-04) |
| Every DB test on SQLite, MySQL, PostgreSQL (doc 04 §6.1; AGENTS.md) | `mt-store` and `mt-server/tests/database.rs` harnesses | Met on Linux; Windows/macOS database matrix not yet (R3-05) |
| The product's operations on real databases | `mt-server::persist` | Met (single node) |
| `sea-query` for dynamic SQL (doc 20 §4.1) | — | **Deviation**: sqlx Any driver + own dialect layer (README "Decisions") |
| Column cache (doc 20 §4.1) | — | Not built (R3-02) |

## 3. Findings

| ID | Sev. | Finding | Resolution |
|---|---|---|---|
| R3-01 | — | Values are stored as JSON text per field rather than typed columns, and multi-values/links have no junction tables, so the SQL query compiler (R2-10) cannot yet filter on them in SQL | **Deferred to the query compiler** (with R2-10): typed columns and `mv_`/`ln_` tables come with SQL filtering |
| R3-02 | — | No column cache; the server keeps the whole workspace in memory and loads every record at start | **Deferred to wave 4** (server roles, large tenants) |
| R3-03 | — | The server is a single process: bases are fenced, but requests are not yet routed to the owning node of a cluster | **Deferred to wave 4** (cluster roles, `mt-cluster` membership in the server) |
| R3-04 | — | The simulator does not model the audit chain or permissions | **Deferred**: both are pure and tested on their own; the simulator gains them with the real-time protocol |
| R3-05 | — | Databases are tested on Linux only | **Deferred to wave 8** (OS × DB matrix) |
| R3-06 | Low | `mt-store` and `mt-sim` are below the 98% line target (97.6%, 94.8%) | **Accepted**: the uncovered lines are defensive error branches and violation reports; both numbers are stated in the READMEs |
| R3-07 | — | The server saves by comparing the whole workspace with what is stored (O(records) per save), and rewrites the configuration document (including the audit chain) on configuration changes | **Deferred to wave 4**: client batches arrive as `mt-ops` operations through the real-time API; the audit chain gets its own table |

## 4. Definition of done (doc 21 §6)

| | `mt-ops` | `mt-store` | `mt-sim` |
|---|---|---|---|
| Native and wasm32 (pure crates) | yes / yes | yes / n/a | yes / yes |
| Clippy clean under workspace lints | yes | yes | yes |
| Six test categories | standard, edge, invalid, property, model, concurrency | standard, edge, invalid, anomalous, model, concurrency (3 engines) | standard, edge, anomalous, concurrency |
| Model-based or simulation test for state machines | model test + seeded replica simulation | job-queue and epoch/lease models | is the simulation |
| README with the public API | yes | yes | yes |
| Both audits | this document; logic audit wave 3 | | |

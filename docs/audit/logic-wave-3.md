# Logic audit — wave 3 (operations, storage, fencing, jobs, the whole-system simulation)

Audit date: 2026-11-27. Question asked of every module: *is the behaviour right, and does it stay
right under concurrency and failure* — two owners at once, crashes between steps, messages
lost, duplicated or reordered, a stalled worker, a database that differs by engine.

Method: `mt-ops` read line by line and pinned by a model-based test against a naive reference and
a seeded multi-client simulation; `mt-store` run on SQLite, MySQL and PostgreSQL with reference
models for epochs, leases and the job queue, and concurrency tests (many tasks at once);
`mt-sim` runs the composed system (150 seeds per configuration) and checks five invariants; the
product runs its API and browser tests on a database.

## 1. Findings

| ID | Sev. | Where | Finding | Resolution |
|---|---|---|---|---|
| L3-01 | High | operations across owners | Found by `mt-sim`: refusals lived only in an owner's memory. A batch refused by one owner (and the client told so) could be applied by the next owner when a resend or duplicate crossed the answer before an ownership change | **Fixed**: refusals are journaled (`mt-store` `_refusals`, tenant schema 2; `Tenant::refuse` behind the fence) and recalled by owners (`BaseState::recall`). The simulator reproduces the anomaly without journaled refusals and never with them |
| L3-02 | Medium | product: lifecycles | A transition changed a record without recomputing: "changed by / at" and formulas reading the state stayed stale until the next unrelated write (found when a database copy disagreed with the in-memory workspace) | **Fixed**: transitions recompute at once |
| L3-03 | Medium | `mt-store` on SQLite | SQLite treats a double-quoted name that is no column as a string literal, so a "does this column exist" probe always succeeded and new columns were never added | **Fixed**: columns are read from the engine's catalogue (`pragma_table_info`, `information_schema`); tested on all engines |
| L3-04 | Medium | `mt-store` tests | The rollback test reused a client op id, so the journal's unique key refused the batch before any row was touched: the rollback path was never exercised | **Fixed**: the test asserts *why* the batch was refused ("does not fit") and uses its own id. Lesson applied: refusal tests check the reason, not only the kind |
| L3-05 | Medium | `mt-store` on MySQL | Long text comes with a BLOB wire type, which the portable driver does not decode as text | **Fixed**: text is read as a string or UTF-8 bytes (`row::text`) |
| L3-06 | Low | dev databases | `scripts/dev-db.sh` handed MySQL's data directory to the PostgreSQL user on every re-run and started mysqld attached to the shell | **Fixed** |
| L3-07 | Info | `mt-ops` | A client's batches reordered in flight can make an earlier batch "too old" once a later one was decided. Clients must send over one ordered connection and resend in order (as `Replica` keeps them) | Documented; terminal either way |
| L3-08 | Info | jobs | A lease that runs out while the worker is still working loses the job to another worker; the late completion is refused (`LeaseLost`) and the job runs again. Jobs must be idempotent | Documented (doc 20: every job has an idempotency key) |

## 2. Reasoning checks that passed

**One writer per base, even when two nodes believe they own it.** Ownership is an epoch raised by
compare-and-set; every write checks — and on MySQL and PostgreSQL locks — the epoch row inside
the write's own transaction. In 150 simulated runs there were about 2,500 moments with two
self-declared owners and 2,100 writes by superseded owners; all were refused, and journals were
written by non-decreasing epochs. With the check switched off, the simulator fails runs.

**No change applied twice, none lost silently.** Client op ids are remembered per base (the last
4,096 decisions in memory, every accepted batch and refusal in the journal), so a batch arriving
again — after a reconnect, a duplicate, a crash or a change of owner — gets its first answer. In
the simulation: ~1,900 repeats recognised, no batch journaled twice, no client left with an
unanswered batch.

**All or nothing.** A batch applies completely with the next sequence number or not at all — in
memory (`mt-ops` atomicity tests) and in storage (a batch whose second effect does not fit rolls
back the first, on all engines). A save writes several bases' batches and the configuration in
one transaction; a stale configuration version or a lost fence writes nothing.

**Everyone ends with the same data.** Replicas show the confirmed state plus their pending batches,
rebased on every change; when all batches are answered every replica equals the database (300
seeded multi-client runs in `mt-ops`, every `mt-sim` run). Clients that miss changes catch up from
the journal.

**Same results on every engine.** Ordering uses Rust-computed sort keys, so SQLite, MySQL and
PostgreSQL return records in the same order as `compare_values`; epochs and leases give the same
compare-and-set answers as `mt-cluster`'s reference stores; the job queue follows its reference
model step by step.

**Every job ends.** Claims are leases with tokens; a stalled worker cannot complete, fail or extend
a job another took over; failures back off up to an hour; the last attempt ends `dead`. In the
simulation every job completed exactly once despite ~12,000 leases taken over.

## 3. Items handed to later waves

| Item | Wave | Check to write there |
|---|---|---|
| Real-time protocol (WebSocket, `mt-ops` frames) with client batches reaching the owner | 4 / 7 | The simulator's invariants against the real server, clients through the API |
| Requests routed to the owning node of a cluster (R3-03) | 4 | Two server processes on one database: writes forwarded, never fenced in steady state |
| Typed columns, junction tables and the SQL query compiler (R3-01, R2-10) | 4 | Query results identical on three engines and in `mt-query::run` |
| Audit chain in its own table; per-operation saves (R3-07) | 4 | Save cost independent of workspace size |
| SLA timers and the scheduler on leases | 5 | A timer fires once after a crash between cursor move and fire (L1-05) |

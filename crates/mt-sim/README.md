# mt-sim

Deterministic simulation of the whole system (doc 21 §4): nodes that own bases, clients that
edit through any node, a shared database, and job workers — on a virtual clock, over a network
with delay, loss, duplication and reordering, with partitions, node crashes and restarts. Every
run is a function of its seed.

The parts under test are the real ones: `mt-cluster`'s rendezvous placement and `Fencer` (with
its in-memory epoch store as the database's epochs), `mt-ops`' `BaseState` as each owner's actor
and `Replica` as each client. The simulated database follows `mt-store`'s rules: a commit checks
the writer's token and the sequence atomically, decisions are journaled, jobs are leased with a
token. Membership views lag behind the truth, so two nodes can believe they own a base at once.

## Public API

| Item | What it is |
|---|---|
| `run(seed, &Config) -> Result<Report, Violation>` | One seeded run with every invariant checked at the end |
| `Config` | Nodes, clients, bases, duration, network (delays, loss, duplication), faults, detection delay, edit and resend pace, job lease; `persist_refusals` and `check_fences` (off only to show what goes wrong without them) |
| `Report` | What happened: commits, refusals, recognised repeats, fenced writes, split-brain moments, crashes, partitions, lost messages, resends, catch-ups, job claims and completions, lost leases |
| `Violation` | The broken invariant, its seed and virtual time |
| `World`, `db::Db` | The world and the simulated database |

## Invariants (checked on every run)

1. Each base's journal is numbered 1, 2, 3, … without gaps and written by non-decreasing epochs.
2. No batch is applied twice — across ownership changes, crashes, resends and duplicates.
3. No batch is applied after its client was told it was refused.
4. Every client ends with no unanswered batch and exactly the database's records.
5. Every job ends done, completed exactly once, despite crashes and leases taken over.

## What the runs exercise (150 seeds, default configuration)

About 24,000 commits, 16,000 refusals, 1,900 recognised repeats, 2,100 writes refused because
the writer had been superseded, 2,500 split-brain moments, 450 crashes, 460 partitions,
5,700 lost messages, 11,000 resends, 13,700 catch-ups, 37,000 job claims with 12,000 leases taken
over — and no violation. The test asserts that each of these cases occurs.

## Findings

* **Refusals must be journaled** (logic audit L3-01). Without them, an owner that never saw a
  refusal can apply a batch whose client was already told it was refused (a resend or duplicate
  crossing the answer, then an ownership change). `anomalous_unjournaled_refusals_…`
  reproduces it with `persist_refusals: false`; `mt-store` journals refusals (`_refusals`,
  tenant schema 2) and owners recall them.
* **The simulator catches a store without fencing**: with `check_fences: false`, superseded
  owners' writes land and runs fail.

## Tests

`tests/sim.rs`: many seeds (`MT_SIM_SEEDS`, default 150) keep every invariant and exercise every
hard case; a run is a function of its seed; a calm network and a hostile one (5 nodes, 6
clients, 15 % loss and duplication, faults every 150 ms); the two anomalies above.

Cross-platform: builds for `wasm32-unknown-unknown`. Coverage: 94.8 % of lines — the rest are
the violation reports, which run only when something is broken.

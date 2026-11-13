# Development Plan (v1.0)

> Implements docs 17–20. This plan adds the engineering method: clustering, concurrency, security,
> simulation testing, and the agent workflow (builders + two audit agents).

## 1. Method

| Rule | Detail |
|---|---|
| Waves | Work is done in dependency-ordered waves. Each wave ends with: all tests green, both audits run, findings fixed or logged, commit |
| Builder agents | One agent per crate (or small crate group). Each owns only its crate directory |
| **Requirements audit agent** | After each wave: checks the code against docs 17–20 (features, limits, acceptance criteria) and reports gaps as `docs/audit/requirements-wave-N.md` |
| **Logic audit agent** | After each wave: reviews business logic and reasoning (invariants, edge cases, money/time/permission correctness, concurrency hazards) and reports as `docs/audit/logic-wave-N.md` |
| Test categories (every crate) | standard · anomalous · edge · invalid · concurrency · cross-platform (pure crates also build for `wasm32-unknown-unknown`) |
| Test techniques | unit + property tests (`proptest`), golden vectors, model-based tests for state machines, deterministic simulation (§4), DB tests on SQLite + MySQL + PostgreSQL |
| Lints | Workspace lints: `unsafe_code = forbid`; no wildcard enum match arms; no `unwrap`/`expect`/`panic` in non-test code; checked arithmetic |
| Coverage | `cargo llvm-cov`; business crates 100% functions/lines target, gate ≥ 98% |

## 2. Waves

| Wave | Crates | Outcome |
|---|---|---|
| **W0** | workspace, lints, `xtask`, dev DB scripts | Scaffold |
| **W1** | `mt-core`, `mt-cluster`, `mt-audit` (+ trace types) | Values, field types, validation library, sort keys, provenance; cluster membership/placement/fencing with simulation; hash-chained audit |
| **W2** | `mt-refdata`, `mt-query`, `mt-formula`, `mt-perm`, `mt-lifecycle` | Pure business engines (native + WASM) |
| **W3** | `mt-ops`, `mt-store` (3 dialects, DB per tenant), `mt-sim` (whole-system simulator) | Operation pipeline on real databases; simulated multi-node, multi-client runs |
| **W4** | `mt-identity`, `mt-api` (Public + Admin, OpenAPI), `mt-server` (single binary, cluster roles) | Running server |
| **W5** | `mt-rules`, `mt-workflow`, `mt-notify`, `mt-integrate` | Process automation and connections |
| **W6** | `mt-metrics`, `mt-templates`, `mt-ai`, `mt-ai-eval`, `mt-mcp` | Analytics, templates, AI |
| **W7** | `mt-wasm`, `mt-grid`, `apps/web` (React) | Web app |
| **W8** | Templates S2/S3/S4/S6/S7, Docker image, GitHub Actions, end-to-end simulation of every template loop | R1 candidate |

## 3. Cluster design (single binary, N-to-N)

Industry references: SWIM gossip membership (HashiCorp memberlist/Serf; Rust `foca`), Quickwit's `chitchat`
(scuttlebutt + phi-accrual failure detector), rendezvous hashing for actor placement (Orleans/Akka-style
virtual actors), fencing tokens for single-writer safety (Kleppmann), FoundationDB/TigerBeetle-style
deterministic simulation.

| Concern | Design |
|---|---|
| Topology | Every node runs the same binary and can serve any request. No leader. Nodes talk to each other directly (full mesh over TCP, framed binary messages, mutual authentication with a cluster secret + TLS) |
| Membership | SWIM-style gossip implemented as a **sans-IO state machine** (probe, indirect probe, suspect, dead, incarnation numbers, piggybacked updates). Seeds from config/DNS |
| Placement | Each active base has exactly one **owner node** (base actor). Owner = highest rendezvous hash score among alive members. Membership changes move only the affected bases |
| Single-writer safety | **Fencing epoch** per base stored in the tenant database. A node must win a compare-and-set on the epoch to become owner; every write transaction checks the epoch, so a partitioned old owner cannot commit |
| Routing | Any node accepts client connections. Operations for a base it doesn't own are forwarded to the owner; committed changes are fanned out to nodes holding subscribers |
| Jobs | DB-backed queue with leases (visibility timeout + fencing), so any node can run jobs and crashed work is retried |
| Scheduling (timers, SLA, cron) | Sharded by rendezvous hash over tenants; lease-based, idempotent firing |
| SQLite mode | Single node only (clustering requires MySQL or PostgreSQL) |
| Rolling upgrade | Protocol version in handshake; nodes refuse incompatible peers; drain = give up ownership then leave |

## 4. Simulation strategy

1. **Sans-IO cores.** Cluster membership, placement, fencing, the base actor, the approval/lifecycle engines
   and the workflow engine are pure state machines: `step(state, input) -> (state, outputs)`. No clocks,
   sockets or randomness inside.
2. **Deterministic simulator (`mt-sim`).** A seeded scheduler drives many nodes and clients with a virtual
   clock and a simulated network (delay, loss, duplication, reordering, partitions) and simulated crashes
   and restarts. Every run is reproducible from its seed.
3. **Invariants checked continuously:** at most one writer per base epoch; no committed operation lost or
   applied twice; all replicas of a subscription converge; every operation/job/delivery reaches a terminal
   state; audit chain valid; permissions never widened.
4. **Workflow simulations.** Each business process (doc 17 §4, P1–P15) and each template loop has a scripted
   scenario plus randomised variants (model-based), run on all three databases.
5. **CI:** fixed seeds on every PR; thousands of random seeds nightly; failing seeds are saved as regression tests.

## 5. Concurrency & security checklist (per module)

- Concurrency: single-writer per base; optimistic versions on records; idempotency keys; lock-free reads;
  `loom` where shared-memory primitives are hand-written; no blocking in async code.
- Security: capability check on every entry point; tenant routing only from the authenticated principal;
  input size limits; parameterised SQL only; secrets never logged; constant-time comparisons for tokens;
  SSRF guard on outbound calls; fuzzing for every parser/decoder.

## 6. Definition of done (per crate)
Compiles on native (and wasm32 for pure crates) · clippy clean under workspace lints · all six test
categories present · simulation or model-based test where the crate has state machines · README with the
public API · both audits passed or findings logged.

# Requirements audit — wave 1 (`mt-core`, `mt-cluster`, `mt-audit`)

Audit date: 2026-11-14. Audited against: doc 17 (product design: §3.1, §3.2 D3/D4/D7/D15, §5 C1/C2/C10,
§6), doc 20 (architecture), doc 21 (development plan: §3 cluster design, §4 simulation, §5 checklist,
§6 definition of done).

Method: every spec item that wave 1 is responsible for was looked up in the code. An item counts as
**met** only when a public type or function implements it *and* a test exercises it. Gaps are either
fixed in this wave or logged with the wave that owns them.

## 1. Result

| Crate | Tests | Line coverage | Lint (`-D warnings`) | WebAssembly build | README | Verdict |
|---|---|---|---|---|---|---|
| `mt-core` | 209 | 99.65% | clean | builds | yes | Meets wave-1 scope after fixes R1-01…R1-06 |
| `mt-cluster` | 118 | 99.68% | clean | builds | yes | Meets wave-1 scope |
| `mt-audit` | 73 | 99.61% | clean | builds | yes | Meets wave-1 scope |

400 tests in total; all pass. Seven requirement gaps were found: six fixed in this wave, one group
deferred to wave 2 with an owner (R1-07…R1-10 below are deferrals, not defects of this wave's scope).

## 2. Traceability

### 2.1 `mt-core` against doc 17

| Spec item | Where | Status |
|---|---|---|
| §3.2 types: prefixed stable IDs | `id` — 33 typed IDs, `<prefix>_<26 chars>`, time-ordered | Met |
| §3.2 `ts` UTC timestamp; `money` exact decimal + currency | `time`, `decimal`, `money` (ISO 4217 minor units, no floats; float arithmetic is a denied lint) | Met |
| D3 Field: id, data_type, widget | `field::{FieldDef, DataType (15), Widget (31)}`; `Widget::supports` | Met |
| D3 Field: name (i18n) | `FieldDef::names`, `display_name(locale)` with region → language → default fallback | **Fixed (R1-01)** |
| D3 Field: options_source inline / refset + subtree + level / table / parameter | `OptionsSource::{Inline, RefSet{subtree, level, leaves_only, cascade_from}, Table, Parameter}` | **Fixed (R1-02)** |
| C2 "options from reference sets with cascades" | `OptionsSource::RefSet::cascade_from`, `validate::validate_cascade`, `IssueCode::CascadeMismatch` | **Fixed (R1-02)** |
| D3 Field: format (prefix, suffix, decimals, mask, numbering pattern) | `DisplayFormat`, `Mask`, `NumberingPattern` | Met |
| D3 Field: validation required, regex, range, unique scope, format id, cross-field | `ValidationSpec`, `TextPattern` (regex-free by design: linear time, no ReDoS), `UniqueScope`, `FormatId`, `CrossFieldRule` | Met |
| D3 Field: validation *conditional* | — | Deferred (R1-07) |
| D3 Field: role, classification, status, group, library_item, help_text | `FieldRole` (17), `Classification` (5 levels, ordered), `FieldStatus`, ids, text | Met |
| D3 Field: ai_skill, version | `FieldDef::ai_skill`, `FieldDef::version` | **Fixed (R1-03)** |
| D3 FieldGroup: id, name, fields[] | `FieldGroup` | Met |
| D3 FieldGroup: visibility_condition, capability | — | Deferred (R1-07, R1-08) |
| D3 FieldLibraryItem, FieldType registry; C2 "8 picker groups, ~35 field types" | ids only | Deferred (R1-09) |
| D3 Sequence: pattern, scope by period, next_value, gap-free | `NumberingPattern`, `period_key`, `allocate` (pure allocation; storage supplies the counter in the record's transaction) | **Fixed (R1-04)** |
| D4 Cell provenance: source_kind, principal, initiator, op_id, trace_id, ai {skill_version, model, confidence, evidence[], review_state}, at | `provenance::{CellMeta, Source, Actor, AiProvenance, Confidence, EvidenceRef, ReviewState}` | Met |
| §3.1 classification per field (5 levels) | `Classification` | Met |
| D7 Lifecycle state **category** | `state::StateCategory` (8 categories) | Met |
| C1 limits (1M rows, 400 fields, 200 views, 5 group levels, 100 filter conditions, 200 editors, 2 GB attachments) | `limits` | **Fixed (R1-05)** |
| C1 "identical sort results on SQLite/MySQL/PostgreSQL" | `sortkey::sort_key` (byte order ≡ `compare_values`; property-tested) — the mechanism; the three-database proof is wave 3 | Met for wave 1 |
| C1 "field type changes" | `convert::{convertibility, convert, preview_conversion}` (pure part; the async job is wave 3) | Met for wave 1 |
| C2 validation library: email, phone, postal codes, tax IDs JP/KR/DE/IN/US with checksums, IBAN | `format::FormatId` — 22 formats | Met |
| C2 acceptance "every validator has positive and negative test vectors" | `format` tests: each `FormatId` has accepted and rejected vectors | Met |
| C2 acceptance "disabling a field never deletes data" | `FieldStatus::accepts_writes`, `validate::check_writable` (writes refused, nothing removed); data retention itself is wave 3 | Met for wave 1 |
| §6 i18n: locale formats from day one | Parsing is locale-neutral on purpose (`coerce`); display formatting per locale belongs to the UI layer (wave 7) | Met for wave 1 |
| Doc 18 principles: provenance on every AI value, reviewable, AI cannot approve itself | `CellMeta::check`: AI needs an initiator; only AI values can be suggestions; AI cannot review its own suggestion | Met |

### 2.2 `mt-cluster` against doc 21 §3–§5

| Spec item | Where | Status |
|---|---|---|
| Single binary, no leader, nodes talk directly | Symmetric state machines; no leader election anywhere in the crate | Met (TCP/TLS wiring is `mt-server`, wave 4) |
| Mutual authentication with a cluster secret | `auth::{ClusterSecret, Handshake}` — three messages, nonces, HMAC-SHA256, constant-time verify | Met |
| Membership: SWIM as a sans-IO state machine (probe, indirect probe, suspect, dead, incarnations, piggybacked updates); seeds | `membership::Membership` — `handle(now_ms, Input) -> Vec<Output>`; `Input::Join { seeds }` | Met |
| Placement: owner = highest rendezvous score among alive members; changes move only affected bases | `placement::Placement`; property tests for minimal movement | Met |
| Single-writer safety: fencing epoch, compare-and-set, checked by every write | `fencing::{Fencer, EpochStore, check_token}` | Met (SQL implementation of `EpochStore` is wave 3) |
| Routing: any node accepts; forward to owner; fan out to nodes with subscribers | `routing::RoutingTable::{route, fan_out}` | Met |
| Scheduling: sharded by rendezvous hash, lease-based, idempotent firing | `scheduler::{claim, renew, release, advance, ShardScheduler}` | Met |
| Jobs: DB-backed queue with leases | — | Deferred (R1-10) |
| SQLite mode: single node only | — | Wave 4 (`mt-server` configuration check) |
| Rolling upgrade: protocol version in handshake, refuse incompatible peers, drain | `ProtocolVersion::is_compatible`, `ClusterError::IncompatibleProtocol`, `Fencer::begin_drain`, scheduler drain | Met |
| §4.1 sans-IO cores | No clock, socket, thread or random source in the crate (seeds and time are inputs) | Met |
| §4.2 deterministic simulator: delay, loss, duplication, reordering, partitions, crashes, restarts; reproducible by seed | `sim::{SimNet, SimCluster}`; `same_seed_replays_identically`; `MT_SIM_SEED` | Met for this crate (whole-system `mt-sim` is wave 3) |
| §4.3 invariant "at most one writer per base epoch" | Simulation, model test, 8-thread race test | Met |
| §5 security: input size limits, constant-time comparisons, secrets never logged, fuzzing for every decoder | `codec` limits on both sides; `verify_slice`; redacted `Debug`; `fuzz_*` tests | Met |

### 2.3 `mt-audit` against doc 17

| Spec item | Where | Status |
|---|---|---|
| D15 AuditEntry (seq, prev_hash, hash, actor, initiator, action, object, details, at) | `chain::AuditEntry` — exactly these fields plus `trace_id` | Met |
| D15 Checkpoint (seq, hash, signature) | `checkpoint::Checkpoint` (+ algorithm, key id, time) | Met |
| C10 hash-chained audit with signed checkpoints | `chain`, `checkpoint` | Met |
| C10 acceptance "tampering is detected" | `verify` — nine tamper kinds located; property tests flip any byte | Met |
| C10 completeness verifier (process contracts) | `process::{ProcessContract, CompletenessVerifier}` | Met |
| C10 acceptance "every operation reaches a terminal state" | `delivery` state machine; `DeliveryLedger::pending` lists operations that have not | Met (enforcement across the system is wave 3) |
| C10 trace explorer | `trace::TraceContext` (W3C) — the data it needs; the explorer UI is wave 7 | Met for wave 1 |
| D7 Exception | `exception::{ExceptionDescriptor, ExceptionKind (11), ExceptionStatus}` | Met |
| §3.1 privacy: classification-aware audit | `privacy::Privacy::HashClassified` | Met |
| C10 journal, cell history, restore points, retention, data requests, activity feed | — | Waves 3–5 (need storage) |

## 3. Findings

| ID | Severity | Finding | Resolution |
|---|---|---|---|
| R1-01 | Medium | Field names had one language only; the spec requires localised names | Fixed: `FieldDef::names` + `display_name`; locale tags validated |
| R1-02 | Medium | Option sources lacked the parameter source, the reference-set level and cascades | Fixed: `OptionsSource::Parameter`, `level`, `cascade_from`; `validate_cascade`; resolver extended |
| R1-03 | Low | Fields had no AI-skill binding and no definition version | Fixed: `ai_skill`, `version` (old stored definitions still load) |
| R1-04 | Medium | A numbering pattern existed, but no way to allocate numbers per period | Fixed: `NumberingPattern::allocate` |
| R1-05 | Low | Product limits were not defined anywhere in code | Fixed: `limits` module |
| R1-06 | Low | `mt-cluster` and `mt-audit` had no README (definition of done) | Fixed |
| R1-07 | — | Conditional validation and group visibility conditions need a condition language | **Deferred to wave 2**: the condition type goes into `mt-core::condition` (data) and is evaluated and compiled to SQL by `mt-query`; first work item of wave 2 |
| R1-08 | — | Field-group capability (view / edit) needs capability keys | **Deferred to wave 2** (`mt-perm` defines them) |
| R1-09 | — | Computed fields (formula, lookup, rollup), the built-in field-type registry (8 picker groups) and library items | **Deferred to wave 2** (with `mt-formula`; the registry needs the computed facet) |
| R1-10 | — | Job queue with leases is in the cluster design table but in no wave-1 crate | **Deferred to wave 3** — **closed in wave 3**: `mt-store::jobs` (claims with leases and tokens, back-off, dead jobs), tested on three engines and in `mt-sim` |

## 4. Definition of done (doc 21 §6)

| Criterion | `mt-core` | `mt-cluster` | `mt-audit` |
|---|---|---|---|
| Compiles native and wasm32 | yes | yes | yes |
| Clippy clean under workspace lints | yes | yes | yes |
| Standard / anomalous / edge / invalid tests | 124 / 5 / 26 / 37 | 52 / 12 / 23 / 10 | 19 / 7 / 5 / 7 |
| Concurrency tests | 3 | 1 thread race + simulations | 2 |
| Cross-platform | wasm build | wasm build | wasm build |
| Property / fuzz / model tests | 14 properties | 7 properties, 2 fuzz, 2 model | 11 properties, 2 model |
| Simulation for state machines | n/a (no state machine) | 6 simulations + 3 harness tests | simulated concurrent appends; delivery model test |
| README with public API and invariants | yes | yes | yes |

Not yet in place, by plan: tests on Windows, macOS and real browsers (GitHub Actions, wave 8); mutation
testing (wave 8); thousands of nightly simulation seeds (wave 8). Locally the simulations run 6
scenarios over the default seed set in about 55 seconds.

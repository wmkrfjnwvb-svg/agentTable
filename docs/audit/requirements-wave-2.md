# Requirements audit — wave 2 (`mt-core` completion, `mt-refdata`, `mt-query`, `mt-formula`, `mt-perm`, `mt-lifecycle`, and the product build)

Audit date: 2026-11-26. Audited against: doc 17 (product design: §3.2 D3/D5/D6/D7/D12, §4 P2/P5/P6/P9,
§5 C1–C6), doc 16 (taxonomy and field picker), doc 21 (development plan: wave 2 scope, §6 definition
of done), the wave-1 audit's deferrals R1-07…R1-09, and the user's direction of 2026-11-26: *the
deliverable is the table people configure themselves from the components, and the end-to-end tests
build such a configuration*.

Method as in wave 1: an item counts as **met** only when a public type or function implements it *and*
a test exercises it. Items the engines meet but the product does not yet offer on screen are listed as
**engine** with the wave that brings them to the product.

## 1. Result

| Crate | Tests | Line coverage | Lint | WebAssembly | README | Verdict |
|---|---|---|---|---|---|---|
| `mt-core` | 255 | 99.41% | clean | builds | yes | Meets wave-2 scope (R1-07…R1-09 closed) |
| `mt-refdata` | 57 | 100.00% | clean | builds | yes | Meets wave-2 scope |
| `mt-query` | 43 | 100.00% | clean | builds | yes | Meets wave-2 scope (SQL compiler: wave 3) |
| `mt-formula` | 87 | 99.80% | clean | builds | yes | Meets wave-2 scope |
| `mt-perm` | 42 | 98.99% | clean | builds | **fixed (R2-01)** | Meets wave-2 scope after R2-01, R2-02 |
| `mt-lifecycle` | 85 | 98.69% | clean | builds | yes | Meets wave-2 scope |
| `mt-server` + `apps/web` | 40 API + 40 browser steps | 91.8% (gate 90%) | clean | n/a (server) | yes | The product build: see §3 |

All 800 Rust tests and both browser walks pass locally and in CI. Engine crates together: 99.5% of
lines (gate 98%).

## 2. Traceability

### 2.1 Wave-1 deferrals

| Item | Where | Status |
|---|---|---|
| R1-07 one condition type for validation, visibility, filters, rules | `mt_core::condition::Condition` (all / any / not / test; 18 operators; periods; `me`); `ValidationSpec::required_when`; `FieldGroup::visible_when`; filters (`mt-query`), grants (`mt-perm`), guards and branches (`mt-lifecycle`) all take the same type | Met |
| R1-08 field-group capabilities | `FieldGroup::{view_capability, edit_capability}`; enforced by `mt-perm::field_permissions` | Met |
| R1-09 computed fields, field-type registry, field library | `FieldDef::computed` (formula / lookup / roll-up); `BuiltinType` (47 types in 8 picker groups, synonyms, `suggest`); `FieldLibraryItem::{check, instantiate, differences}` | Met |

### 2.2 C1 tables, records and views (query part) — `mt-query`

| Spec item | Where | Status |
|---|---|---|
| Filter: nested AND/OR, relative dates, “me” | `Query::filter: Condition`; `Operand::{Period, Me}` | Met |
| Multi-sort, empties first/last | `SortKey`; total order (property-tested) | Met |
| Group up to 5 levels with subtotals; date buckets | `GroupKey` (day … year); groups partition the result (property) | Met |
| Search across fields, labels not ids | `Search` | Met |
| Footer aggregates, selection summary | 13 functions; `summarize` | Met |
| Views: grid, list, kanban, calendar, timeline, gallery; personal / locked / protected / shared | `View`, `ViewKind`, access modes | Met (engine); saved views on screen: wave 7 grid |
| Acceptance: identical sort/filter results on SQLite/MySQL/PostgreSQL | `run` is the reference evaluator; the SQL compiler is compared with it | Wave 3 (R2-10) |
| Acceptance: 1M-row grid at 55 fps, first paint < 1 s | — | Wave 7 |

### 2.3 C2 field framework and D3 schema — `mt-core`

| Spec item | Where | Status |
|---|---|---|
| 8 picker groups, ~35 types, synonym search, suggestion from a sample | `PickerGroup`, `BuiltinType`, `search`, `suggest`; the product's field picker | Met |
| Options from reference sets with cascades | `OptionsSource::RefSet` (wave 1) | Met |
| Formats, validation library | wave 1 | Met |
| Field library; “updates propagate or stay pinned” | `FieldLibraryItem::differences` detects drift | Engine; propagation as an operation: wave 3 |
| Status enabled / disabled / deprecated; “disabling never deletes data” | wave 1; select choices removed in the product are disabled, values kept (`builder::update_field`) | Met |
| Conditional validation (`required_when`) | `validate_required_when` | Met: the field editor sets “required only when …”; saving and moving forward in a process are refused while the condition holds and the field is empty (R2-12) |

### 2.4 C3 reference data and C4 settings — `mt-refdata`

| Spec item | Where | Status |
|---|---|---|
| Flat and tree sets; standard and custom attributes | `RefSet::{structure, item_schema}` | Met |
| Tenant overlay (extend, relabel, disable) | `Overlay`, `State::apply` | Met |
| Crosswalk aliases per external system | `RefItem::aliases`, `crosswalk`, `alias` | Met |
| Impact analysis | `impact` (removal blocked while used) | Met |
| Versions and approvals (four eyes) | `Ledger` (`propose` / `approve` / `reject`, `as_of`, `restore`); `Change::SetPolicy` | Met: every code-list change in the product goes through the ledger; a list can need a second person; history and “as of” on screen (R2-06) |
| Built-in ISO sets | `builtin` (countries, currencies, languages, sex, fiscal periods) | Met |
| Acceptance: only enabled items valid on the effective date | `RefData::resolver(day, …)` drives `mt-core` validation | Met |
| Settings: typed, scoped (system → tenant → base → user), secrets, flags, `PARAM()` | `params::Registry` with `resolve` / `explain`; `PARAM` in `mt-formula` | Engine; the product's Settings page keeps tenant-level values only (R2-05) |
| Acceptance: parameter resolution explains its source | `Registry::explain` | Met (engine) |

### 2.5 Formulas — `mt-formula`

| Spec item | Where | Status |
|---|---|---|
| Formula language, functions by group, errors pointing at the character | `parse`, `functions/*`, `FormulaError` with position | Met |
| Type checker; formulas, lookups and roll-ups as fields | `check`, `check_field`, `field::attach` | Met |
| Bounded evaluator; `#REF!` for deleted fields | `evaluate*`, limits | Met |
| Dependency graph; cycles refused; recompute across tables | `deps::Graph` | Met; incremental recompute: wave 3 (`mt-ops`) |

### 2.6 C5 access — `mt-perm`

| Spec item | Where | Status |
|---|---|---|
| Built-in and custom capabilities | `Action` (25), `Capability::Custom` | Met |
| Grants with scope (own, team, reference subtree, record set, condition, all), conditions (amount limits), validity | `Grant`, `Scope`, `condition`, `valid_from/to` | Met |
| Capability sets, guests, share links, SoD rules | `CapabilitySet`, `PrincipalKind::Guest`, `ShareLink`, `SodRule` | Met |
| Explain (the grant chain) and preview as user | `Decision::{chain, refusals}`; deciding for another actor | Met |
| Access reviews | `AccessReview` (now model-tested, R2-02) | Met |
| Rules: one engine for every path; agents are principals | dual identity (`Actor::on_behalf_of`); the product asks it for screens, lists, fields, approvers and transitions | Met |
| Acceptance: a permission-matrix suite per template | The purchasing template has a matrix of seven people in `tests/api.rs`; the expense configuration a matrix of four in `tests/builder.rs` | Met for the two configurations; a generic per-template runner: wave 6 (templates) |
| Acceptance: no AI or API path can read hidden fields | API: hidden fields are never returned, filtering or sorting by them is refused (`tests/api.rs`); AI path: wave 6 | Met for the API |
| On screen | Grants (who, what, where, which records, which fields, limit, from / until, why), revocation, sets, SoD rules, explain, access reviews, share links | Met (R2-09) |

### 2.7 C6 process — `mt-lifecycle`

| Spec item | Where | Status |
|---|---|---|
| Lifecycles with state categories; transitions with guards, required fields, capability | `Lifecycle`, `StateDef`, `Transition`, `Blocker` | Met, and designed on screen (`design.rs`, the process designer) |
| The closed loop: every state reachable, every state can finish | `Lifecycle::check` (property: accepts exactly the closed lifecycles) | Met; the designer shows the refusal by state name |
| SLA clocks with business calendars and pause | `calendar`, `sla` (reference-model test) | Engine; on records: wave 5 with timers (R2-07) |
| Approval modes sequential / all / any / quorum; priority branches with default; parallel; fill-in; CC; versions; simulation | `approval` | Met (engine). On screen: any / all / one after another, “only when” branches, notifications, versions, “who would be asked”; quorum, parallel blocks, fill-in, timeouts: R2-08 |
| Assignment, acknowledgement, escalation | escalation of overdue tasks in `approval`; assignment and acknowledgement: wave 5 | Partly |
| Actions, waits, sub-flows | — | Wave 5 (`mt-workflow`) |
| Tasks and checklists, exceptions inbox, collection rounds, booking, scorecards, inspections | — | Waves 5–6 |
| Acceptance: model-based tests cover every approval mode and action sequence up to depth N | `tests/approval_model.rs`: reference model per mode (3,000 seeds), 1,500 random definitions under 60 random events with invariants, replay and persistence (depth 60) | Met |
| Acceptance: each transition is atomic | the server applies a transition, its approval start and its notices under one lock; a refused transition changes nothing (`tests/process.rs`) | Met (single node); across nodes with the operation pipeline: wave 3 |
| Acceptance: running approvals keep their version | engine `Version` error; designer publishes a new version only on change; `std_running_approvals_keep_their_version` | Met |

## 3. The product build (user direction: build your own)

| Need | Where | Status |
|---|---|---|
| Start empty; bases; tables | `builder::{create_base, create_table, …}`; Home, base page | Met |
| Every picker type that has its editor; required, help, roles, currency, single and two-way links | field picker; `api::add_field`, `builder::add_link` | Met; attachments, signatures, maps, AI fields: waves 3, 6, 7 |
| Edit and delete fields safely | `builder::{update_field, delete_field, usages}` | Met |
| Code lists, settings, people and groups | Code lists, Settings, People & groups pages | Met |
| Design processes and approvals | process designer, `design::publish` | Met (see 2.7) |
| Give access | Access page, `access` | Met (see 2.6) |
| Templates | gallery; the purchasing sample is a template and publishes through the designer | Met |
| End-to-end tests build a configuration | `e2e/build.spec.ts`: 19 steps from an empty workspace to a paid claim, run by four people | Met |

## 4. Findings

| ID | Severity | Finding | Resolution |
|---|---|---|---|
| R2-01 | Low | `mt-perm` had no README (definition of done) | Fixed |
| R2-02 | Medium | `AccessReview` is a state machine but had no model-based test (rules require one) | Fixed: 512 random sequences against a reference; refused steps change nothing |
| R2-03 | Medium | The product's process code assumed the demo (table, field names, guard text) | Fixed: the designer generates every definition; descriptions come from the definitions |
| R2-04 | Medium | “Records they own” ignored the creator in tables without a person field | Fixed (see L2-08) |
| R2-05 | — | Settings in the product are tenant-wide values only; the engine's scopes, history and explain are not used yet | **Deferred to wave 3** with storage (settings per base and person need it) |
| R2-06 | — | Code-list changes in the product bypass the versioned ledger (no history, no four eyes) | **Fixed**: changes are committed or proposed through `Ledger`; the history is stored next to the list and must replay to it on load; switching four eyes off needs approval (`Change::SetPolicy`, model-tested). Tests `std_four_eyes_is_switched_on_at_once_and_off_only_with_a_second_person`, `std_code_lists_keep_a_history_and_can_need_a_second_person`; browser step build-32 |
| R2-07 | — | SLA clocks and calendars are not attached to lifecycle states yet | **Deferred to wave 5** (timers need the job queue with leases, wave 3) |
| R2-08 | — | The designer does not offer quorum, parallel blocks, fill-in steps or timeouts, which the engine has | **Fixed**: all four in the designer; test `std_quorum_parallel_fill_in_and_timeouts`; browser steps build-21 … build-25 |
| R2-09 | — | The Access page does not offer field access, validity, share links or reviews, which the engine has | **Fixed**: grants on a table take “only” / “all but” fields and a period; reviews (open on a place, keep / revoke with a note, complete revokes, cancel) and read-only share links (a record or a table, fields left out, expiry, withdrawal, an anonymous page). Tests `std_grants_limited_to_fields_and_to_a_period`, `std_access_reviews_keep_or_revoke_with_a_note`, `std_share_links_show_one_table_and_never_more_than_their_maker_sees`, `edge_share_links_expire`; browser steps build-29 … build-31 |
| R2-10 | — | Query results on the three databases | **Wave 3** (SQL compiler compared with `mt-query::run`) |
| R2-11 | — | Full recompute after each change | **Wave 3** (`mt-ops`, incremental from the dependency graph) |
| R2-13 | Medium | The CI coverage step piped `cargo llvm-cov` into `tee` without `pipefail`, so a failing test did not fail the step; the 98 % gate then judged a partial run (it failed, but for the wrong reason) | **Fixed**: the step runs with `shell: bash` (pipefail); the failing test was a real counterexample (L2-14) |
| R2-12 | — | `required_when` is not enforced by the product and has no editor | **Fixed**: editor, enforcement on save and on forward transitions (withdraw / cancel are never blocked); test `std_fields_required_only_when_a_condition_holds`; browser step build-28 |

## 5. Definition of done (doc 21 §6)

| Criterion | `mt-refdata` | `mt-query` | `mt-formula` | `mt-perm` | `mt-lifecycle` |
|---|---|---|---|---|---|
| Compiles native and wasm32 | yes | yes | yes | yes | yes |
| Clippy clean under workspace lints | yes | yes | yes | yes | yes |
| Standard / edge / invalid / anomalous tests | yes | yes | yes | yes | yes |
| Concurrency | thread-safety, racing writers | thread-safety | thread-safety | one policy, many threads | order independence across threads |
| Property / model / simulation | 11 properties, ledger model | 9 properties | 9 properties | 4 properties, review model | 7 properties, SLA model, approval model and simulations |
| README with public API and invariants | yes | yes | yes | yes (R2-01) | yes |
| Line coverage ≥ 98% | 100.00% | 100.00% | 99.80% | 98.99% | 98.69% |

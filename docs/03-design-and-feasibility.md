# Design & Feasibility Review (pre-implementation)

> Status: design v0.1 for discussion · Inputs: `01-research-requirements.md`, `02-research-deep-dive.md`.
> **Partly superseded by `04-decisions-and-research-round3.md`** (license, UI framework, Canvas/DOM gate,
> database-per-tenant storage replacing Postgres RLS/JSONB, 3-DB support, full feature scope, coverage & lint policy).
> Nothing here is implemented yet. Sections marked **DECISION** need stakeholder sign-off.

---

## 1. Goals and non-goals

**Goals (v1)**
- A browser-first, open-source multidimensional table whose **business processes close their
  loops**, with every value attributable and every process provable.
- Rust for all business logic; one core shared by browser (WASM) and server; desktop later.
- Smooth grid at 1M rows; small WASM; secure multi-tenant SaaS **and** single-tenant self-host.
- 98% coverage per crate, plus property/fuzz/mutation/concurrency testing.

**Non-goals (v1)** — explicitly deferred so the loop-closing core ships first:
native mobile apps, offline editing, Gantt/pivot/map views, plugin marketplace, scripting,
AI agents, sync connectors to external DBs, 10M-row tables.

---

## 2. Product design

### 2.1 Concepts

Familiar concepts (Base, Table, Field, Record, View, Form, Dashboard, Workflow, Role) plus
five **loop-closing primitives** that most table products lack:

| Primitive | What it is | Loop it closes |
|---|---|---|
| **Lifecycle** | A state machine bound to a table's Status field: states, allowed transitions, who may transition, required fields per state, entry/exit actions, terminal states | "Status jumped to Done without the data / approval" |
| **Assignment** | Owner (person/group) per record per state, with `assigned → acknowledged → done` and escalation | "Nobody owned it / nobody saw it" |
| **SLA** | Timer per state (e.g. *Triage ≤ 4 business hours*), business calendar per region | "It sat there for two weeks" |
| **Exception** | A system-generated item when a loop breaks (unowned, SLA breached, run failed, validation failed, unacknowledged, stuck) — always has an owner | "It failed silently" |
| **Process instance** | The traced journey of one record through its lifecycle + all workflow runs it triggered | "Can we prove what happened?" |

Plus **asset metadata** on every table (owner, steward, classification, retention, description)
and **provenance** on every cell change (user / API / import / workflow / AI, with trace ID).

### 2.2 End-to-end walkthrough — Scenario 1: Request intake → resolution

Actors: Requester (employee or customer), Triage agent, Resolver, Team lead, Compliance auditor.

| Step | What happens in the product | Data asset effect | Loop checkpoint |
|---|---|---|---|
| 1 Define | Lead creates base from template "Requests": tables *Requests*, *Teams*, *Requesters*; lifecycle `New → Triaged → In progress → Resolved → Closed` (+ `Rejected`); SLA Triage ≤ 4h, Resolve ≤ 3 business days; owner rules | Schema, dictionary, owners, classification (requester email = PII) | Every state has an owner rule and an SLA, or the editor warns |
| 2 Capture | Requester submits a public/internal form; consent notice shown (DPDP/GDPR) | Record created, provenance = form, trace ID | Validation (required, email format, dedupe hint) before accept |
| 3 Route | Assignment rule (by category → team round-robin) sets owner; owner notified (in-app + email) | Assignment record | If no rule matches → **Exception "Unassigned"** to team lead |
| 4 Acknowledge | Owner opens and clicks *Acknowledge* (or edits it) | Ack timestamp | No ack in 1h → escalation to lead |
| 5 Act | Owner moves to *In progress*, comments, @mentions; resolves with required field "Resolution" | Cell history, comments | Transition blocked without required fields; illegal transitions impossible |
| 6 Notify | Workflow: on *Resolved* → email requester with resolution + satisfaction link | Workflow run ledger | Email delivery failure → **Exception** with retry |
| 7 Verify | Requester rates; auto-close after 5 days; reopen if unhappy | Rating field | Process instance complete only when terminal state reached |
| 8 Measure | Dashboard: volume, SLA hit rate, backlog by owner, CSAT; **every chart drills into records** | Metrics derived from records | Red KPI → click → filtered view → act |
| 9 Audit | Auditor opens trace of request #1042: every step, actor, timestamp, hash-chain verified | Tamper-evident log | Completeness verifier: 0 stuck/orphaned instances, or listed |
| 10 Improve | Lead changes routing rule / SLA; change itself is audited | Versioned config | Loop back to step 1 |

**Feasibility:** all steps rely on MVP features (forms, lifecycle, assignment, SLA, notifications
via in-app + SMTP, workflow-lite, dashboards w/ drill-down, trace & audit). External email
requires SMTP configuration in self-host — acceptable. **Verdict: feasible for MVP and delivers
clear value** (replaces spreadsheet + email chains; measurable SLA; audit evidence).

### 2.3 Scenario 2: Purchase request → approval → PO → receipt → payment match

Needs: lifecycle with **approval steps** (multi-level by amount thresholds), **field locking
after approval**, linked tables (Request → PO → Receipts → Invoices), rollups (received qty,
invoiced amount), three-way-match check as a formula + lifecycle guard ("cannot move to *Paid*
unless matched"), audit evidence for SOX/J-SOX.
**Verdict:** feasible on the same primitives; approvals with thresholds + field locking are
v1 (P1) items. No ERP integration in v1 — export/API only. Value is high for SMBs without an ERP
module; for large enterprises it's a front-end to ERP → integration matters (later).

### 2.4 Scenario 3: Asset register

Needs: lifecycle (`Ordered → In stock → Assigned → In repair → Retired`), assignment to people,
recurring audits (scheduled workflow creates "audit tasks"), renewal reminders (SLA relative to
date field), barcode field (P1), quality score (missing serials, unassigned assets).
**Verdict:** feasible; strong fit with "data assets" message; barcode scanning on mobile web
via camera API is P1.

### 2.5 Value assessment by persona

| Persona | Pain today | Value delivered | Proof metric |
|---|---|---|---|
| Operator (agent/resolver) | Work arrives by email/chat, context scattered | One queue, clear ownership, required data prompts | Time-to-acknowledge, rework rate |
| Team lead / manager | No visibility; status meetings | Live SLA dashboard with drill-down, exceptions inbox | SLA attainment, backlog age |
| Data owner / steward | Spreadsheet sprawl, conflicting definitions | Owned assets, dictionary, quality score, lineage | Quality score trend, duplicate rate |
| Admin / security | Shadow IT, unknown access | Roles, row/field rules, view-as, access review, SSO | Access review completion |
| Compliance / auditor | Evidence assembled manually | Tamper-evident trace per process instance, exports | Time to produce audit evidence |
| Developer / integrator | Fragile spreadsheet integrations | Stable-ID REST API, signed webhooks with ledger, MCP later | Integration incidents |

**Risk to value:** if lifecycle/assignment/SLA are hard to configure, users fall back to a
plain single-select and the loop opens again. → Templates per scenario + a "loop health"
checklist in the table settings (warns: state without owner rule, no SLA, no terminal state,
dashboard without drill-down target).

---

## 3. Scope

### 3.1 MVP (M1–M4) — "one loop, closed, proven"

- Auth (email+password argon2id, passkeys, OIDC SSO), tenants, workspaces, members, basic roles.
- Bases/tables/fields/records/views CRUD; field types: text, long text, number, currency,
  percent, checkbox, date/datetime, single/multi select, user, attachment, email, phone, URL,
  link (two-way), lookup, rollup, formula, auto number, created/modified time/by.
- **Canvas grid view** (virtualised, 1M rows), kanban view, form view; filter/sort/group;
  column footers.
- Real-time collaboration (op log, presence), comments with @mentions, cell history.
- **Lifecycle, assignment + ack, SLA timers, exceptions inbox** (loop primitives).
- Workflow-lite: triggers (record created/updated/state changed/scheduled/form submitted),
  actions (update/create record, notify in-app/email, webhook), run ledger with retries.
- Dashboards: number, bar, line, pie, table — each with drill-down to filtered records.
- Trace system (3 layers) + tamper-evident audit log + trace viewer per record.
- REST API v1 (tokens, scopes) + signed webhooks with delivery ledger; OpenAPI spec generated.
- CSV/XLSX import (dry run, upsert by key, validation report) and export (policy-aware).
- English UI; locale-aware formatting (en-US, en-IN, de, ja, ko formats) from day one.

### 3.2 v1 (after MVP)
Advanced permissions (custom roles, row/field rules, view-as), approvals with thresholds and
field locking, calendar/gallery views, asset metadata & lineage graph UI, quality score,
DSAR tools, region pinning, AI fields with provenance + review state, MCP server, i18n UI (de/ja/ko/hi).

### 3.3 Later
Desktop (Tauri wrapper reusing WASM grid), offline, Gantt/pivot, AI agents, connectors, scripting.

---

## 4. System architecture

```
Browser (WASM)                                          Server (Rust, native)
┌──────────────────────────────────────┐   HTTPS/JSON   ┌───────────────────────────────────────┐
│ app-web (Leptos CSR shell)           │ ─────────────► │ api (axum): REST v1, auth, OpenAPI     │
│  ├─ grid (Canvas2D renderer, input,  │   WebSocket    │ realtime: base actors (1 writer/base)  │
│  │        DOM editor overlay, ARIA)  │ ◄────────────► │   validate(op) with core ──► commit    │
│  ├─ client-sync (outbox, windows,    │   postcard     │   seq, broadcast (perm-filtered)       │
│  │        seq gap detection)         │                │ query: filter/sort/group via sort_keys │
│  ├─ core (shared)                    │                │ workflow: durable runs, SLA timers     │
│  └─ trace-client (traceparent, spans)│                │ notify: in-app, email(SMTP), webhooks  │
└──────────────────────────────────────┘                │ trace/audit: journal, hash chain,      │
            ▲ same crates compiled to wasm32             │   completeness verifier, exceptions    │
            │                                            │ core (shared)                          │
            │                                            └───────────────┬───────────────────────┘
            │                                                            │ sqlx
            │                                            ┌───────────────▼───────────────────────┐
            └──────── shared Rust crates ──────────────  │ PostgreSQL (RLS, partitions)          │
                                                         │ Object storage (S3-compatible / FS)    │
                                                         └───────────────────────────────────────┘
```

Self-host: **one server binary** + Postgres (+ optional S3). SaaS: same binary, horizontally
scaled; base actors placed by consistent hashing on `base_id`; Postgres per region.

### 4.1 Workspace crates

| Crate | Target | Responsibility |
|---|---|---|
| `mt-core` | native + wasm | IDs, field types, cell values (exact decimal, UTC instant), validation, coercion, sort-key encoding |
| `mt-formula` | native + wasm | Lexer/parser, type check, evaluator (bounded), dependency graph, incremental recompute |
| `mt-query` | native + wasm | Filter/sort/group/aggregate AST; in-memory evaluator; SQL compiler (server feature) |
| `mt-perm` | native + wasm | Roles, row/field rules, decision function `can(actor, action, resource)` |
| `mt-lifecycle` | native + wasm | State machine definition, transition guard, SLA calculation, business calendars |
| `mt-ops` | native + wasm | Operation types, wire codec (postcard), apply/rebase, `client_op_id` |
| `mt-trace` | native + wasm | Trace/span IDs, traceparent, delivery states, journal entry types, hash-chain verify |
| `mt-store` | native | Postgres repositories (sqlx), migrations, RLS session handling |
| `mt-server` | native | axum API, auth, realtime actors, workflow engine, notifications, audit |
| `mt-grid` | wasm | Canvas renderer, hit testing, selection, keyboard, editor overlay, ARIA mirror |
| `mt-web` | wasm | Leptos app shell, views, dialogs, client sync |
| `xtask` | native | `cargo xtask ci`, size budget, coverage gates, release |

Rules: shared crates have **no** tokio/sqlx/web-sys dependencies; `#![forbid(unsafe_code)]`
everywhere; server-only features behind cargo features.

### 4.2 Key flows

**Open a view**
1. Client: `GET /views/{id}/open` → schema, view config, `snapshot_seq`, total rows, first window (postcard).
2. Grid paints immediately; on scroll requests `window(snapshot_seq, offset, len)`.
3. WebSocket subscription delivers ops with `base_seq > snapshot_seq` → patched into windows;
   ops that change membership/order of the view trigger a window refresh.

**Edit a cell**
1. Grid → editor overlay → `mt-core` validates locally → optimistic render.
2. Op `{client_op_id, trace_id, base_id, record_id, field_id, new_value, expected_cell_version}` → outbox → WS.
3. Base actor: authenticate → `mt-perm` → `mt-core` validate → `mt-lifecycle` guard →
   apply in a Postgres transaction (record JSONB, sort_keys, history, journal) → assign `base_seq`
   → commit → ack to sender → broadcast permission-filtered change to subscribers →
   enqueue dependent recompute (formula/lookup/rollup) and workflow triggers.
4. Conflict (`expected_cell_version` stale): op still commits (last-writer-wins) but sender is
   notified with the overwritten value; history keeps both. (Lifecycle transitions instead
   **reject** on stale version — state changes must be deliberate.)

**Workflow run / SLA**
Trigger events are journal entries; the workflow engine consumes them durably (Postgres-backed
queue, `SKIP LOCKED`), runs steps with idempotency keys, records each step in the run ledger,
retries with backoff, and on final failure raises an Exception. SLA timers are rows with due
times scanned by a scheduler.

### 4.3 Data model (Postgres, abbreviated)

```
tenants(id, region, plan, settings)
users(id, email, ...), memberships(tenant_id, user_id, role)
workspaces(tenant_id, id, name)
bases(tenant_id, id, workspace_id, name, head_seq)
tables(tenant_id, id, base_id, name, asset_meta jsonb)           -- owner, steward, classification, retention
fields(tenant_id, id, table_id, type, config jsonb, classification, position)
records(tenant_id, id, table_id, cells jsonb, version, created_*, modified_*, deleted_at)  -- hash-partitioned by tenant
sort_keys(tenant_id, table_id, field_id, key bytea, record_id)   -- order-preserving keys from mt-core
cell_history(tenant_id, record_id, field_id, base_seq, old, new, actor, source, trace_id, at)
views(tenant_id, id, table_id, kind, config jsonb)
lifecycles(tenant_id, id, table_id, definition jsonb, version)
assignments(tenant_id, id, record_id, state, assignee, status, ack_at, due_at)
journal(tenant_id, base_id, base_seq, kind, payload, trace_id, client_op_id unique, at)
workflow_runs(tenant_id, id, workflow_id, trigger_seq, status, attempts, ...), run_steps(...)
exceptions(tenant_id, id, kind, subject_ref, owner, status, opened_at, resolved_at)
audit_log(tenant_id, seq, entry, prev_hash, hash, at), audit_checkpoints(tenant_id, seq, hash, signature)
webhook_deliveries(...), api_tokens(...), attachments(...)
```
All tables: RLS `FORCE`, policies `USING/WITH CHECK (tenant_id = current_setting('app.tenant')::uuid)`.

### 4.4 Trace-audit design

- IDs: `trace_id` (per user action), `client_op_id` (per op), `base_seq` (per committed op),
  `run_id/step_id` (workflows), `process_instance_id` (= record_id + lifecycle version).
- **Delivery ledger** (in `journal` + client outbox): each op ends in `committed` or `rejected`;
  periodic reconciliation: client reports last applied `base_seq`; server detects stuck clients.
- **Process contracts** derived from lifecycle + workflow definitions: required terminal state,
  SLA per state, required steps after given events (e.g. *Resolved ⇒ requester notified*).
- **Completeness verifier** (scheduled + on demand): evaluates contracts over the journal →
  `complete | in_progress | late | broken(reason)`; broken/late → Exceptions.
- **Trace viewer**: timeline per record/run (who, what, when, source, trace links), with
  hash-chain verification badge; export as JSON/PDF evidence.
- Privacy: audit entries store field IDs and value hashes for fields classified PII where the
  tenant chooses minimisation; full values stay in `cell_history` subject to retention/DSAR.

---

## 5. Quality strategy

### 5.1 Test matrix (categories × domains — examples)

| Domain | Standard | Anomalous | Edge | Invalid | Concurrency | Cross-platform |
|---|---|---|---|---|---|---|
| Field values | each type round-trips | NaN-like strings in number import, mixed-type columns | `Decimal` max/min, 0 decimals (JPY), leap day, DST gaps (US/DE), year 9999, empty vs null, 64 KB text, emoji/ZWJ, RTL marks, Devanagari | wrong type, overflow, invalid email/phone/URL, unknown option id | two users edit same cell | WASM vs native identical results (same test vectors run on both) |
| Formula | arithmetic/text/date | div by zero → error value, circular refs | deep nesting limit, 10k-item rollups | syntax errors, unknown field refs | recompute while schema changes | identical outputs native/wasm, all OS |
| Filter/sort | per operator | nulls, mixed types | collation (ä/ae, kana/kanji, Hangul), equal keys stable order | malformed AST | sort while records change | — |
| Lifecycle/SLA | allowed transitions | transition with missing required fields | SLA across weekends/holidays/time zones (IN/JP/KR no DST) | undefined state | concurrent transitions → exactly one wins | — |
| Permissions | role matrix | user removed mid-session | row rule on null owner | forged ids from other tenant | permission change during subscription | — |
| Ops/sync | edit → commit → broadcast | dropped/duplicated/reordered WS messages | 1M-row table, 500-op batch | malformed postcard bytes (fuzz) | 100 simulated clients, network partitions (deterministic sim) | Chrome/Firefox/Safari |
| Multi-tenancy | normal isolation | pooled connection reuse | tenant with 0 rows / max quotas | cross-tenant IDs in API | parallel tenants under load | — |
| API/webhooks | CRUD per resource | retries, 5xx from receiver | pagination boundaries, 500-record batch | bad scopes, bad signatures, oversized bodies | idempotency key replays | — |
| Grid UI | render, scroll, edit | slow network (skeletons) | 1M rows, 200 cols, frozen cols, HiDPI | paste of malformed TSV | remote edits while editing a cell | Playwright on Chromium/Firefox/WebKit × Win/macOS/Linux; IME composition tests |
| Trace/audit | full chain recorded | missing step detected | hash chain over 10M entries | tampered entry detected | concurrent appends keep chain valid | — |

Tooling: `cargo test` (unit/integration), `proptest`, `cargo-fuzz` (nightly CI job), `loom`,
deterministic network simulation, `cargo-mutants` (weekly), `wasm-bindgen-test`, Playwright,
`criterion` benchmarks with regression thresholds, `cargo-llvm-cov` (98% gate per crate).

**Shared test vectors**: JSON files of `input → expected` for core/formula/query/lifecycle,
executed natively on every OS **and** in browsers via wasm-bindgen-test — this is how we prove
"robust and accurate business logic" is identical everywhere.

### 5.2 Local gate and CI/CD

Local (`cargo xtask ci`, wired to a git pre-push hook):
`fmt --check` → `clippy -D warnings` → unit+integration tests (local Postgres) → coverage gate
→ wasm build + size budget → `cargo-deny`.

GitHub Actions workflows:

| Workflow | Trigger | Jobs |
|---|---|---|
| `ci.yml` | PR, push | lint; test matrix {ubuntu, windows, macos} × stable (+ MSRV on ubuntu) with Postgres; coverage (ubuntu, gate 98%); wasm build + size budget; wasm-bindgen-test {chrome, firefox} on ubuntu & windows, {safari} on macos; Playwright E2E {chromium, firefox, webkit} × {ubuntu, windows, macos}; cargo-deny/audit |
| `nightly.yml` | schedule | fuzzing (time-boxed), deterministic sim with many seeds, WASM coverage (nightly toolchain), benchmarks |
| `weekly.yml` | schedule | cargo-mutants on core crates, dependency updates report |
| `release.yml` | tag | build server binaries (linux x64/arm64, windows, macos), web bundle, container image, SBOM, signatures, provenance |

---

## 6. Feasibility assessment

| Requirement | Feasibility | Main risk | Mitigation |
|---|---|---|---|
| Rust core in browser, small binary | High | Feature creep bloats WASM | Hard CI budget; heavy data (collation, tzdb, regex) server-side; twiggy reviews |
| Grid: fast, no lag, 1M rows | High (proven pattern) but **largest engineering item** | Text measuring cost, IME, a11y | Canvas2D + DOM editor overlay + ARIA mirror; spike S1 first |
| Accurate business logic | High | Float/time-zone/collation bugs | Exact decimals, UTC, sort-key encoding, shared test vectors |
| High security & multi-tenancy | High | RLS misconfig, cross-tenant leaks via cache/WS fan-out | FORCE RLS + app filters + per-tenant cache keys + isolation test suite |
| Collaboration | High | Permission-filtered fan-out cost at scale | Actor per base; pre-computed subscriber permission sets; load test |
| 98% coverage | **Medium** | WASM/UI glue coverage tooling experimental; cost of tests ~1–1.5× code | Thin glue, logic in native-testable crates; per-crate gates; reviewed exclusions; accept WASM coverage via nightly + E2E |
| Trace-audit completeness | High | Performance of journal & verifier | Append-only partitioned journal; verifier incremental by seq |
| Cross-platform CI | High | Postgres on Windows/macOS runners; flaky canvas screenshots; CI minutes | Native Postgres install; per-OS baselines; run full browser×OS matrix on main + nightly, reduced on PRs |
| Product value / closed loop | High if loop primitives are usable | Users fall back to plain selects | Templates, loop-health checklist, exceptions inbox front and centre |
| Overall scope | **Risky** | MVP is still large for a small team | Strict MVP cut, spikes first, monthly re-plan |

**Overall verdict:** technically feasible with the chosen architecture. The biggest risks are
(1) grid engineering effort, (2) the 98% coverage cost on UI glue, (3) scope. The product is
valuable **only if** lifecycle/assignment/SLA/exceptions ship in the MVP — without them we'd be
building "another Airtable clone" whose loops stay open.

---

## 7. Plan

### 7.1 Spikes before committing (≈2–3 weeks total)

| Spike | Question | Exit criteria |
|---|---|---|
| S1 Grid | Can Rust/Canvas2D render 1M × 50 smoothly with JA/KO/HI text and IME editing? | ≥ 55 fps scroll on mid-range laptop in Chrome/Firefox/Safari; IME works; frame < 8 ms |
| S2 Size | Leptos vs Dioxus shell + grid + core WASM size | ≤ 600 KB brotli total; pick framework |
| S3 Storage | JSONB+sort_keys vs physical tables at 1M rows | filter+sort first window p95 < 300 ms; pick model |
| S4 Realtime | Base actor with 100 clients, permission-filtered broadcast | commit p95 < 100 ms server-side; no lost ops in fault injection |
| S5 Coverage & CI | cargo-llvm-cov native + WASM nightly; Postgres on win/mac runners | Green matrix; merged coverage report |
| S6 Trace | Outbox + journal + verifier prototype | Injected faults always end as committed/rejected/exception |

### 7.2 Milestones (indicative)
- **M0** repo, CI skeleton, xtask, hooks, spikes S1–S6.
- **M1** core/formula/query/perm crates (+ test vectors), storage, auth, tenancy.
- **M2** grid view, real-time ops, history, comments, API v1.
- **M3** lifecycle, assignment, SLA, exceptions, workflow-lite, notifications, trace-audit.
- **M4** forms, kanban, dashboards with drill-down, import/export, scenario-1 template, beta.

---

## 8. Decisions needed

1. **License**: Apache-2.0 / AGPL-3.0 / split (Apache core + AGPL server)?
2. **"PC" in the CI requirement**: we interpret as **Linux**; confirm (plus Windows/macOS/browsers).
3. **UI framework**: Leptos (recommended) vs Dioxus — final after spike S2.
4. **Storage model**: JSONB + sort_keys (recommended) vs physical tables — final after S3.
5. **Hosting**: will we run an official SaaS (needs regions EU/US/JP/KR/IN + SOC 2 path) or
   self-host-only at first?
6. **First validation scenario** for MVP: Request intake (recommended), Purchasing, or Assets?
7. **98% coverage definition**: line coverage per crate with a reviewed exclusion list (≤1%),
   WASM glue covered by E2E where tooling can't measure — acceptable?
8. **Team & timeline** to size milestones.

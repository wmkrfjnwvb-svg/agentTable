# Decisions & Research — Round 3

> Status: design v0.2 · Supersedes the parts of `03-design-and-feasibility.md` listed in §0.
> Claims marked **[verify]** must be confirmed by a spike or measurement before we rely on them.

---

## 0. Decision log

| # | Decision (stakeholder) | Consequence for the design | Replaces in doc 03 |
|---|---|---|---|
| D1 | License **Apache-2.0** | All crates Apache-2.0; `cargo-deny` allowlist of compatible licenses (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode-3.0, MPL-2.0 file-level only) | §8 Q1 |
| D2 | **No virtual-DOM UI framework** | **Leptos (CSR)**, fine-grained signals. Sycamore is the fallback. Dioxus and Yew are excluded | §4.1 / B2 |
| D3 | Canvas vs DOM needs evidence | Research in §1. The decision is made by a measured head-to-head in spike S1 | B1 |
| D4 | **One database per tenant**, physical tables inside it | No Postgres RLS. A control-plane DB plus tenant DBs, one physical SQL table per user table (§3) | §4.3, B6, B7 |
| D5 | Open-source Docker version first, managed cloud later; **5-minute deployment** | Single image, SQLite by default, first-run wizard; compose files for MySQL/PG (§5) | §3.3 |
| D6 | Support **SQLite, MySQL, PostgreSQL**; **every test runs on all three** | Dialect layer and a parameterised test harness; CI DB matrix (§6) | §5 |
| D7 | MVP = **the full multidimensional-table feature set** (scenarios added later) | Feature inventory and build waves (§4) | §3.1 |
| D8 | Comprehensive **OpenAPI**, **user dashboard**, **admin dashboard** | Defined in §4.3–4.5 | — |
| D9 | **Full coverage** of functions and business logic; **exhaustive `match`** enforced | Coverage gates and lint policy (§7) | §5.1 |

---

## 1. Why do Lark Base and Airtable feel slow? (input to D3)

### 1.1 Evidence

| Product | Finding | Source |
|---|---|---|
| Lark/Feishu Base | Feishu's official *formula calculation optimisation guide* addresses lag when **filtering, grouping, sorting, scrolling** and slow formulas. It states that the new Base version **switched from browser-side calculation to server-side calculation**. So the older product computed formulas and lookups in the browser | [Feishu help: 多维表格公式计算优化指南](https://www.feishu.cn/hc/zh-CN/articles/022441339437) |
| Lark/Feishu Base | Customer cases with hundreds or thousands of complex formulas over hundreds of thousands of rows; vendor claims of 1M rows and faster loading in the new database | [CSDN/Feishu launch article](https://blog.csdn.net/csdnnews/article/details/142208915) |
| Lark/Feishu Base | Third-party developers report clear slowdowns above ~50k rows and sync to MySQL as a workaround (anecdotal) | [CSDN sync guide](https://bbs.csdn.net/weixin_32667955/article/details/100503428) |
| Lark/Feishu Base | Base **already renders with Canvas**: a Flutter-like widget layout engine, dirty-rectangle redraw, and server-side output of render-ready structures (third-party analysis, author unknown) | [Tencent Cloud: 浅谈 Canvas 渲染引擎](https://cloud.tencent.com/developer/article/2240907), [Zhihu mirror](https://zhuanlan.zhihu.com/p/608415829) |
| Lark/Feishu desktop | The desktop client lags with less than 4 GB RAM; Feishu suggests using the web version | [Feishu help: memory & lag](https://www.feishu.cn/hc/zh-CN/articles/360039160114) |
| Airtable | Community and developer reports say **all of a table's data must be loaded into the browser** for apps. Airtable staff acknowledged slowdowns as bases grow | [Airtable community thread](https://community.airtable.com/development-apis-11/should-my-airtable-apps-slow-down-as-my-base-gets-bigger-6816) |
| Airtable | Official docs describe server work per base as a **request queue processed one request at a time**; syncs compete with users | [Airtable support: base performance](https://support.airtable.com/docs/troubleshooting-airtable-base-performance) |
| NocoDB | Rewrote its grid to **Canvas** for speed (0.262/0.263). Side effects reported by users: **browser Ctrl+F no longer finds cell text** and **blurry text when zooming** | [NocoDB PR #10520](https://github.com/nocodb/nocodb/pull/10520), [issue #13569](https://github.com/nocodb/nocodb/issues/13569), [issue #12047](https://github.com/nocodb/nocodb/issues/12047), [release notes](https://winterflow.io/catalog/nocodb/releases/0.263.0/) |
| Teable | Started on Glide Data Grid (Canvas) and then built its own canvas grid; claims millions of rows (vendor claim) | [Show HN: Teable](https://news.ycombinator.com/item?id=39666865), [GitHub](https://github.com/teableio/teable) |

### 1.2 Conclusion

The lag users feel in Lark and Airtable comes mainly from the **data and compute model**, not
from Canvas:

1. **Loading whole tables into the client.** First load grows with row count, and memory pressure
   grows with it.
2. **Computing formulas, lookups and rollups in the browser.** Lark itself moved this to the server.
3. **Serialised per-base server queues.** Reads and syncs wait behind writes.
4. **Formula dependency cascades** with no incremental recompute.

Canvas is what fast grids converge on (Lark, NocoDB, Teable, Google Sheets). Its known costs are
browser find, zoom sharpness, accessibility and text selection. All of these can be mitigated.

### 1.3 What we do differently

| Root cause | Our design |
|---|---|
| Whole-table load | **Windowed loading**: schema plus first window plus count. Only the visible window and prefetch are held in memory, so client memory stays flat at 10k or 1M rows |
| Client-side computation | **Server-side computation** with an incremental dependency graph in `mt-formula`. The client computes only optimistic previews for the edited record |
| Serialised queue | The **write path** is serialised per base actor, which is needed for ordering and audit. The **read path** (window queries) bypasses the actor and reads committed snapshots directly, so reads never wait for writes |
| Recompute cascades | Topologically ordered, batched, coalesced recompute. Recompute of a large table streams in pages and does not block edits. Stale cells show a "computing" marker |

### 1.4 Canvas vs DOM: decision gate (spike S1, expanded)

We build **both** prototypes on the same data source and the same Leptos shell:
- **A. Canvas2D grid** in Rust (`web-sys`): viewport-only drawing, text measurement cache, DOM editor overlay.
- **B. DOM-virtualised grid** in Leptos: fine-grained signals, recycled row and cell nodes, absolute positioning.

Measured in Chrome, Firefox and Safari, on Windows, macOS and Linux:

| Metric | Pass threshold |
|---|---|
| Scroll frame time p95 at 1M rows × 50 cols, mixed field types | ≤ 16 ms (no dropped frames under normal scroll) |
| Long tasks (> 50 ms) during 30 s of fling scrolling | 0 |
| Keystroke-to-paint latency while editing | ≤ 16 ms p95 |
| Memory after scrolling the full table | ≤ 150 MB and flat |
| Time to first paint after data arrives | ≤ 100 ms |
| Japanese/Korean IME, Devanagari rendering, HiDPI and zoom sharpness, Ctrl+F, screen-reader navigation | All work |
| WASM size added by the grid | ≤ 150 KB brotli |

**Rule:** choose DOM (B) if it passes everything, because it is simpler, more accessible, and has
native find and selection. Otherwise choose Canvas (A) with the mitigations below. Either way, the
non-grid UI (menus, dialogs, expanded record, forms, kanban/gallery cards, dashboards) uses
Leptos DOM.

Canvas mitigations if A is chosen:
- Intercept Ctrl/Cmd+F and open our own in-grid search, which searches the full dataset on the server.
- Re-render on `devicePixelRatio` changes (zoom) to keep text crisp.
- An ARIA grid mirror of the visible window for screen readers.
- Copy, paste and select-all through a hidden textarea.
- An optional "accessible table mode" fallback that uses DOM rendering.

---

## 2. UI framework (D2)

| Framework | Rendering model | Status |
|---|---|---|
| **Leptos (CSR)** | Fine-grained signals, direct DOM updates, no VDOM | **Chosen** |
| Sycamore | Fine-grained reactivity, no VDOM | Fallback |
| Dioxus, Yew | Virtual DOM | Excluded by D2 |

The grid crate (`mt-grid`) depends only on `web-sys` and exposes a small imperative API
(`set_window`, `apply_patch`, `on_edit` …), so the framework choice cannot leak into grid
performance.

---

## 3. Database-per-tenant architecture (D4, D6)

### 3.1 Layout

```
Control-plane DB ("system")                 Tenant DB  (one per tenant)
├─ instance_settings                         ├─ _meta_bases, _meta_tables, _meta_fields, _meta_views
├─ tenants (id, name, status, db_locator,    ├─ _meta_lifecycles, _meta_workflows, _meta_roles ...
│           schema_version, region, quota)   ├─ t_<tableId>            ← one physical table per user table
├─ users, identities (password/passkey/OIDC)  │    columns: id, f_<fieldId>..., sk_<fieldId> (sort keys),
├─ memberships (user ↔ tenant ↔ role)         │             _version, _created_at/_by, _modified_at/_by, _deleted_at
├─ sessions, api_tokens (hashed)             ├─ mv_<fieldId>(record_id, value)   ← multi-value index (multi-select/user)
├─ instance audit log                        ├─ ln_<fieldId>(src_id, dst_id, pos) ← link field junction
└─ jobs (provisioning, migrations)           ├─ _history, _journal, _audit (hash chain), _comments
                                             ├─ _assignments, _sla_timers, _exceptions, _workflow_runs ...
                                             └─ _attachments (metadata; blobs in FS/S3)
```

| Engine | "Database per tenant" implementation | Connection strategy |
|---|---|---|
| **SQLite** (default in Docker) | One file per tenant: `/data/tenants/<tenantId>.db`, WAL mode | Small pool per open tenant, LRU-closed when idle; a single writer per DB fits the base-actor model |
| **MySQL 8.0+ / 8.4 LTS** | `CREATE DATABASE mt_t_<tenantId>` | **One shared pool**; queries use qualified names `mt_t_x.t_tbl` (MySQL databases are schemas) |
| **PostgreSQL 14+** | `CREATE DATABASE mt_t_<tenantId>` | Connections are bound to a database → **pool per active tenant** with LRU eviction and a global connection cap. Optional **schema-per-tenant mode** (`search_path`) for hosts with many tenants **[verify limits in S3]** |

Benefits, which also count as data-asset features:
- Hard isolation. A query cannot reach another tenant's data unless it opens that tenant's DB.
- **Per-tenant backup, restore, export and deletion.** GDPR/DPDP erasure means dropping the DB or deleting the file.
- Noisy neighbours stay contained, and a tenant's DB can later be placed in a specific region for the cloud version.

Costs and how we handle them:
- **Schema migrations fan out across N databases.** A migration orchestrator tracks
  `schema_version` per tenant in the control plane. Migrations run idempotently and resumably,
  with batches and a progress UI in the admin dashboard. A tenant is upgraded lazily on first
  access if it has fallen behind.
- **Provisioning needs `CREATE DATABASE` privileges** on MySQL/PG. This is documented in the
  Docker compose files. A pre-created-database mode covers locked-down DBAs.
- **Connection counts** (PG). Pools are LRU-capped, and schema mode is available.
- **Defense in depth.** Tenant routing comes only from the authenticated session or token. Cross-tenant
  tests run in every suite. Per-tenant DB credentials are a later option.

### 3.2 Physical-table mapping (identical semantics across three dialects)

Physical names come **only from IDs** (`t_<id>`, `f_<id>`), never from user-typed names. This gives
no SQL injection through names, and **renaming is metadata-only** (no DDL).

| Field type | Value column (portable) | Sort / filter support |
|---|---|---|
| Text, long text, email, URL, phone | `TEXT` (not VARCHAR, to avoid MySQL's 65,535-byte row-size limit) | `sk_` BLOB collation key computed by `mt-core` |
| Number, currency, percent | **Canonical decimal string** `TEXT` (SQLite has no exact DECIMAL) | `sk_` BLOB order-preserving decimal key |
| Checkbox | `INTEGER` 0/1 | Direct |
| Date/datetime | `INTEGER` epoch ms UTC (+ field timezone in metadata) | Direct |
| Single select | `TEXT` option id | `sk_` = option position |
| Multi select, user/group (multi) | `TEXT` JSON array (display) | `mv_<fieldId>` junction table → portable "has any/all of" |
| Link | — | `ln_<fieldId>` junction table with position |
| Lookup, rollup, formula | Materialised `TEXT`/`INTEGER` + `sk_` | Computed by `mt-formula` on the server; never by SQL |
| Attachment | `TEXT` JSON of attachment ids | `mv_` for "has file" filters |
| Auto number | `INTEGER` | Direct |

Why this matters for accuracy and for the "all tests on all three DBs" requirement:
- **Collations differ.** SQLite defaults to BINARY, MySQL `utf8mb4_0900_ai_ci`, and PG uses libc or ICU,
  so "ORDER BY name" would give three different results. Every sort uses `sk_` BLOB keys computed in
  Rust, so ordering is **byte-identical on all three engines and in WASM**.
- **SQL floating point differs**, and SQLite has no DECIMAL. Aggregates (sum, avg, rollups) are
  computed in Rust with exact decimals by streaming rows (cached and incremental), not by SQL `SUM`.
- **Field type change** uses the same steps on all engines: add a new column, backfill through
  `mt-core` coercion with a preview, swap the metadata pointer, then drop the old column later.
  SQLite supports `DROP COLUMN` from 3.35.

Limits: SQLite allows 2000 columns by default, InnoDB 1017 per table, and PG 1600 **[verify]**.
With `f_` + `sk_` columns per field, we cap the product at **400 fields per table**, which is well
above typical use (Airtable allows 500).

Query layer: **`sea-query`** builds dialect-correct SQL at runtime, which we need because schemas
are dynamic. It executes through **`sqlx`** drivers (SQLite, MySQL, PostgreSQL) behind an internal
`Dialect` trait. Static tables (control plane, `_meta_*`) use migrations per dialect.
([sqlx](https://github.com/launchbadge/sqlx), [sea-query-sqlx](https://docs.rs/crate/sea-query-sqlx/0.8.1/source/Cargo.toml))
Note: sqlx's SQLite driver links `libsqlite3-sys` (unsafe C). Our own crates stay
`forbid(unsafe_code)`, and this dependency is reviewed with `cargo-vet`.

---

## 4. Full feature scope (D7, D8)

### 4.1 Feature inventory: "full multidimensional table" for v1.0

The inventory includes everything in doc 01 §3 and doc 03 §2.1, delivered in **build waves**.
Each wave ends shippable and fully tested on all three DBs.

| Wave | Contents |
|---|---|
| W1 Foundations | Control plane, tenant provisioning (3 DBs), auth (password/argon2id, passkeys, OIDC), sessions, API tokens, workspaces/bases/tables/fields/records CRUD, the 20 core field types, history, journal, trace layer 1–2 |
| W2 Grid | Grid view (S1 winner): virtual scroll, frozen columns, resize/reorder, row height, selection, keyboard, copy/paste, fill handle, undo/redo, expanded record, filter (nested AND/OR), multi-sort, multi-level group, column footers, search, color rules |
| W3 Computation | Formula engine (full function library), link/lookup/rollup, incremental dependency graph, field type conversion with preview |
| W4 Collaboration | Real-time ops, presence, comments and @mentions, notifications (in-app + SMTP), sharing links, record subscriptions |
| W5 Views & forms | Kanban, calendar, gallery, Gantt/timeline, hierarchy, form view (conditions, prefill, public/internal), personal/locked views |
| W6 Permissions | Roles + custom roles, table/row/field rules, view-as preview, share policies, access review |
| W7 Process (loop primitives) | Lifecycle state machine, assignment + acknowledge + escalation, SLA + business calendars, approvals with field locking, exceptions inbox, workflow engine (triggers/actions/branches/loops/HTTP/webhooks/run ledger), buttons, trace layer 3 + completeness verifier + hash-chain audit |
| W8 Analytics | Dashboards (KPI, bar/line/area/pie/scatter/funnel, table, pivot) with drill-down; export CSV/XLSX/PDF |
| W9 Data assets | Asset metadata (owner/steward/classification/retention), lineage graph and impact analysis, quality rules and score, DSAR export/erase, import (CSV/XLSX/JSON, upsert, dry run), snapshots, recycle bin |
| W10 Platform | Full OpenAPI v1 + generated SDKs, webhooks with ledger, MCP server, user dashboard, admin dashboard, extension SDK |
| W11 AI | AI fields (provenance + review state), AI building assistant, chat-with-base (permission-aware), pluggable providers incl. self-hosted |

> **Feasibility note:** this is a large product. The waves keep it shippable, and W1 to W4 form the
> first usable release. AI (W11) is last on purpose: it needs the permission engine, provenance
> and audit layers to be trustworthy. **Please confirm the wave order.**

### 4.2 OpenAPI (comprehensive)
- **OpenAPI 3.1** spec generated from Rust types (`utoipa`); every endpoint in doc 01 §4.3 plus
  lifecycles, assignments, exceptions, SLA, traces/audit (verify chain, export evidence),
  asset metadata, lineage, quality, admin, and SCIM 2.0.
- Contract tests: the spec is validated in CI, every documented endpoint has tests on all three DBs,
  and a **spec-diff check** blocks breaking changes in v1.
- An API explorer is served by the instance, and SDKs (TS, Python, Go, Rust) are generated in release.

### 4.3 User dashboard (end-user home)
- My work: **assigned to me** (with acknowledge), due soon, SLA at risk, **my exceptions**, approvals waiting.
- My bases (recent, starred), notifications and mentions, recent activity.
- Personal settings: profile, locale/timezone, notification preferences, **sessions and devices**,
  passkeys/MFA, **personal API tokens** (scopes, expiry, last used), my data export (DSAR self-service).

### 4.4 Admin dashboard (instance admin + tenant admin)
- **Instance (self-host operator)**: health (DB/storage/queue), version and **migration status per
  tenant**, tenants (create/suspend/delete/quota), SMTP/storage/SSO settings, background jobs,
  backups/restore, instance audit log, license notices.
- **Tenant admin**: members, groups, roles, invitations, SSO/SCIM, security policies (sharing,
  export, copy, IP allowlist, session length), usage and quotas, **audit log + trace explorer**
  (hash-chain verification, evidence export), webhook delivery ledger, API apps, data
  classification overview, access review, DSAR requests, retention policies.

---

## 5. Five-minute deployment (D5)

```bash
docker run -d -p 8080:8080 -v mtdata:/data ghcr.io/<org>/<app>:latest
# open http://localhost:8080 → first-run wizard (admin account, instance URL, SMTP optional)
```
- **One static binary** (musl) with the web assets embedded, in a distroless image (target < 40 MB **[verify]**).
- SQLite by default and zero configuration. `DATABASE_URL=mysql://…` or `postgres://…` switches engine;
  `docker-compose.mysql.yml` and `docker-compose.postgres.yml` are provided.
- Automatic migrations on start, `/healthz` and `/readyz` endpoints, a `backup` subcommand, and
  `ADMIN_EMAIL/ADMIN_PASSWORD` env vars for headless setup.
- **Tested in CI:** an E2E job runs `docker run` → wizard → create base → add record, and **fails if
  it takes over 5 minutes** including image pull. Docker E2E runs on Linux runners only.

---

## 6. Test strategy updates (D6, D9)

### 6.1 Three-database harness
- Every storage, integration, API and E2E test is **parameterised over
  `Dialect::{Sqlite, MySql, Postgres}`**. A test macro expands to three test cases, each in a fresh
  tenant DB.
- Shared golden vectors (core/formula/query/lifecycle) also run against all three DBs to prove that
  sort, filter and aggregate results are **byte-identical** across engines and in WASM.
- Version matrix: SQLite (bundled, plus min 3.35), MySQL 8.0 and 8.4, PostgreSQL 14 and 17.

### 6.2 CI matrix

| Runner | SQLite | PostgreSQL | MySQL | Browsers |
|---|---|---|---|---|
| ubuntu-latest | ✓ | service container | service container | Chrome, Firefox (wasm-bindgen-test + Playwright Chromium/Firefox/WebKit) |
| windows-latest | ✓ | native install via `ikalnytskyi/action-setup-postgres` (documents Linux/macOS/Windows support) | `shogo82148/actions-setup-mysql` (**win/mac support [verify] in S5**) | Chrome, Firefox, Edge |
| macos-latest | ✓ | same action | same action **[verify]** | Safari (safaridriver), Chrome, Firefox |

Sources: [action-setup-postgres](https://dev.to/ikalnytskyi/setup-postgresql-for-linux-windows-and-macos-using-github-actions-3fhd),
[actions-setup-mysql](https://github.com/marketplace/actions/actions-setup-mysql).
CI cost rises about 3× from the DB matrix. PRs run the full DB matrix on Linux, and the full
OS × DB × browser matrix runs on `main` and nightly. **Every test still runs on every DB before
merge to main.**

---

## 7. Coverage and exhaustive-match policy (D9)

### 7.1 Coverage gates

| Crate class | Function coverage | Line coverage | Branch coverage |
|---|---|---|---|
| Business-logic crates (`mt-core`, `mt-formula`, `mt-query`, `mt-perm`, `mt-lifecycle`, `mt-ops`, `mt-trace`, `mt-store` dialect layer) | **100%** | **100%** (reviewed exclusions only) | **100% target**, CI gate ≥ 98% |
| Service/glue crates (`mt-server`, `mt-grid`, `mt-web`) | **100%** | ≥ 98% | Reported |

- Tooling: `cargo-llvm-cov`. **Branch coverage needs nightly** (`-Zcoverage-options=branch`), so the
  coverage job uses pinned nightly while the build and tests use stable. **MC/DC is not
  available**: rustc removed its unstable MC/DC support in Aug 2025
  ([rust-lang/rust commit log](https://git.dreamy.place/mirrors/rust/log/tests/ui/instrument-coverage?showmsg=1)).
  Combinational conditions are therefore covered with property tests and mutation testing.
- **Mutation testing** (`cargo-mutants`) is the second gate: business crates need a **≥ 90%**
  mutation kill rate **[to tune]**. A coverage line is only "covered" in a meaningful way if the
  tests would fail when the line changes.
- Exclusions must use an attribute with a written reason and are reviewed in PRs. A CI report lists all exclusions.
- WASM-only glue: wasm-bindgen-test coverage is nightly and experimental
  ([wasm-bindgen guide](https://wasm-bindgen.github.io/wasm-bindgen/wasm-bindgen-test/coverage.html)),
  so we keep it **thin**, and every exported function is covered by wasm-bindgen-test or Playwright.

### 7.2 Exhaustive `match` policy

Rust already **rejects non-exhaustive matches at compile time** (E0004). The real risk is a
**wildcard arm `_`** that silently absorbs a new enum variant, such as a new field type or op kind.
Policy, in `[workspace.lints]` in the root `Cargo.toml`:

```toml
[workspace.lints.rust]
unsafe_code = "forbid"
unreachable_patterns = "deny"
unused_must_use = "deny"

[workspace.lints.clippy]
wildcard_enum_match_arm = "deny"            # no `_ =>` on enums: every variant handled explicitly
match_wildcard_for_single_variants = "deny"
wildcard_in_or_patterns = "deny"
rest_pat_in_fully_bound_structs = "deny"
unwrap_used = "deny"                        # no hidden panics in business logic
expect_used = "deny"
panic = "deny"
todo = "deny"
unimplemented = "deny"
indexing_slicing = "deny"
integer_division = "deny"                   # forces explicit decimal / checked math
float_arithmetic = "deny"                   # in business crates; money/number use exact decimals
arithmetic_side_effects = "deny"            # checked/saturating arithmetic only
```

- External `#[non_exhaustive]` enums such as `std::io::ErrorKind` or driver errors force a `_` arm.
  The rustc lint that would flag omitted variants (`non_exhaustive_omitted_patterns`) is **still
  unstable** ([tracking discussion](https://internals.rust-lang.org/t/stabilizing-non-exhaustive-omitted-patterns-or-a-better-alternative/21679)).
  Rule: convert external enums **at the boundary** into our own exhaustive error and kind enums, with
  a single `#[allow]` per conversion that is documented and tested.
- Our own public enums are **not** `#[non_exhaustive]` inside the workspace, so adding a variant
  breaks compilation everywhere it must be handled. That is intentional.
- Tests and test-support crates may relax `unwrap_used`, `expect_used`, `panic` and `indexing_slicing`
  via `#[cfg_attr(test, allow(...))]`.

---

## 8. Updated risks and feasibility

| Risk | Level | Mitigation |
|---|---|---|
| Full feature scope (W1–W11) for a small team | **High** | Waves, each shippable; W1–W4 is the first release; re-plan after each wave |
| Three-dialect physical tables (DDL differences, type change, limits) | Medium | IDs as names, portable column types, Rust-side sort keys and aggregates, `sea-query`, the full suite on all DBs |
| PG database-per-tenant connection count at scale | Medium (cloud only) | LRU pools, global cap, schema mode |
| Migrations across many tenant DBs | Medium | Orchestrator with per-tenant versions, resumable, lazy upgrade, admin UI |
| Canvas vs DOM | Medium | Measured decision gate (S1) |
| 100% function/line + branch coverage on nightly + mutation gate | Medium–High (effort) | Logic in pure crates, golden vectors, property tests; budget ~1–1.5× code in tests |
| CI time and cost (3 OS × 3 DB × browsers) | Medium | Linux full DB matrix on PRs; full matrix on main/nightly; caching (`sccache`, `Swatinem/rust-cache`) |
| MySQL setup on Windows/macOS runners | Low–Medium | Verify in S5; fallback: MySQL in WSL on Windows, Homebrew on macOS |

**Verdict:** still feasible. Database-per-tenant actually **simplifies** security reasoning and
per-tenant data operations. The two big cost drivers are the **full feature scope** and the
**3-DB × 3-OS × full-coverage** quality bar. Both are manageable with the wave plan, but they set
the timeline.

---

## 9. Updated spikes (before W1 coding)

| Spike | Question | Exit criteria |
|---|---|---|
| S1 Grid | Canvas vs Leptos-DOM head-to-head (§1.4) | Thresholds met; choose the renderer |
| S2 Size | Leptos shell + grid + core WASM size | ≤ 600 KB brotli total |
| S3 Storage | Physical tables on SQLite/MySQL/PG at 1M rows; type-change procedure; PG pool-per-tenant limits | First window with filter+sort p95 < 300 ms on all three; type change on 1M rows without blocking reads |
| S4 Realtime | Base actor + 100 clients, permission-filtered broadcast, all three DBs | Commit p95 < 100 ms; no lost ops under fault injection |
| S5 CI | 3 OS × 3 DB × browsers matrix; nightly branch coverage; Docker 5-minute E2E | Green matrix; merged coverage report; deploy < 5 min |
| S6 Trace | Outbox + journal + verifier | All injected faults end committed/rejected/exception |

## 10. Open questions
1. Confirm the **wave order** (AI last) and that W1–W4 is the first public release.
2. Cloud later: is **PostgreSQL schema-per-tenant** acceptable as a scale mode, given that self-host uses database-per-tenant?
3. Minimum DB versions: **SQLite 3.35+, MySQL 8.0+, PostgreSQL 14+**? Is MariaDB needed (it differs from MySQL)?
4. Repository: GitHub org/repo name, so I can create the workspace, CI and hooks and commit these docs.

# Technical Architecture & Solution (v1.1)

> **Status: authoritative technical design for development.** Implements docs 17 (product), 18 (AI) and
> 19 (parity & delivery).
> **Stack decisions:**
> - **All backend in Rust.** No Spring Boot or other JVM service.
> - **Two OpenAPI-documented Rust APIs:** the **Public API** for business data and the **Admin API** for administration and integration. Together with SDKs and MCP, they are the official way to integrate and extend.
> - **Web UI in React + TypeScript**, reusing Rust logic through **WebAssembly**.
> - **Future client app: Tauri 2 + React.** The same React UI, with the Rust core running natively inside the client.
>
> v1.1 replaces v1.0 (which included an optional Spring Boot platform) and doc 04 D2 (Leptos).
> The performance intent, *no virtual-DOM work on the hot path*, is kept: React renders the shell, and the
> grid and data engine run outside React's render cycle (§5.3).

---

## 1. Architecture principles

1. **One backend language.** Rust owns everything on the server: records, operations, formulas, queries, permissions, process, rules and workflows, notifications, integration, AI runtime, admin and audit. Business logic exists in exactly one implementation.
2. **Write once, run everywhere:**
   - pure Rust core crates compile **natively** for the server and the future Tauri client;
   - they compile to **WebAssembly** for the browser;
   - TypeScript never re-implements validation, formulas or permission rules.
3. **React is a thin, fast shell:** layout, dialogs, forms, designers, navigation. Hot paths (grid rendering, data windows, decoding, local evaluation) run in Rust/WASM.
4. **APIs are the extension model.** Customers don't fork or write server code. They integrate through the Public API, the Admin API, SDKs, webhooks and the change feed, and through MCP for AI agents. **Sandboxed WebAssembly plugins** are a later option in the same Rust runtime.
5. **Contracts first:**
   - both HTTP APIs are described in OpenAPI 3.1 generated from Rust code;
   - SDKs are generated from those specs;
   - the browser ↔ server real-time protocol is a shared Rust crate used natively and in WASM.
6. **One deployable.** A single Rust binary (with optional `--role worker` for scale-out) in a single Docker image serves the API, real-time and the React app. A 5-minute self-hosted install is possible.
7. **Inbound-only data sources, explicit outbound destinations** (doc 09), enforced at the API boundary.
8. **Quality gates:**
   - Rust: 100% function/line coverage in business crates, ≥ 98% branch;
   - TypeScript: ≥ 98% lines (Vitest + Playwright);
   - every DB-touching test runs on SQLite, MySQL and PostgreSQL;
   - cross-OS and cross-browser CI.

---

## 2. System context

```
        ┌──────────── People ─────────────┐      ┌──── Later: client app ────┐
        │ Browser: React + WASM           │      │ Tauri 2 + React           │
        │ (desktop & mobile web)          │      │ Rust core native, offline │
        └───────────────┬─────────────────┘      └─────────────┬─────────────┘
                        │ HTTPS REST + WebSocket (binary ops)  │ same protocol
                        ▼                                       ▼
   ┌────────────────────────────────────────────────────────────────────────────┐
   │                         PLATFORM SERVER (Rust, single binary)                │
   │  Public API · Admin API · real-time · data plane · process · rules &        │
   │  workflows · notifications · sources & destinations · AI runtime · MCP ·    │
   │  identity (local, passkeys, OIDC, SCIM) · audit & trace · static web app    │
   └───────┬───────────────────────┬──────────────────────┬─────────────────────┘
           │                       │                      │
  External systems           AI providers          Notification providers
  (data sources in /         (Claude, OpenAI-      (SMTP, SMS, chat)
   destinations out,         compatible, local,
   integration platforms)    document AI)
   Storage: control-plane DB · one DB per tenant (SQLite / MySQL / PostgreSQL) · object storage (FS / S3)
```

---

## 3. Deployable units

| Unit | Tech | Responsibilities | Owns data |
|---|---|---|---|
| **server** | Rust (tokio, axum) | Public API, Admin API, WebSocket real-time, identity, tenancy routing, operation pipeline, queries, formulas, permissions, process, rules & workflows, notifications, sources/destinations, import/export, AI runtime & gateway, MCP server, audit & trace, static hosting of the web app | Control-plane DB, tenant DBs, object storage |
| **worker** (same binary, `--role worker`) | Rust | Long jobs: imports/exports, batch AI, type changes, recompute, sync runs, scheduled reports | — (uses tenant DBs) |
| **web app** | React 19 + TypeScript, Vite, WASM | App shell, views, grid, forms, dashboards, designers, assistant UI, **admin console UI** | Browser cache (IndexedDB) |
| **client app** (later) | Tauri 2 + the same React UI | Desktop (Windows/macOS/Linux), later mobile; optional offline cache with local SQLite and the native Rust core | Local cache DB |

---

## 4. Rust backend design

### 4.1 Workspace crates

| Crate | Targets | Contents |
|---|---|---|
| `mt-core` | native · wasm | IDs, field types, values (exact decimal, UTC instants), validation & format library, coercion, sort keys, provenance types |
| `mt-ops` | native · wasm | Operation model, binary codec (postcard), apply/rebase, client op ids |
| `mt-query` | native · wasm | Filter/sort/group/aggregate AST, in-memory evaluator, SQL compiler (server feature) |
| `mt-formula` | native · wasm | Parser, type checker, evaluator, dependency graph, incremental recompute |
| `mt-perm` | native · wasm | Capabilities, grants, scopes, conditions, separation of duties, explain |
| `mt-lifecycle` | native · wasm | Lifecycles, state categories, transitions, SLA clocks, business calendars, approval state machine |
| `mt-refdata` | native · wasm | Reference sets, trees, effective sets, crosswalks |
| `mt-rules` | native | Rule model, trigger matching, policies (throttle, digest, quiet hours, escalation) |
| `mt-workflow` | native | Durable workflow engine (DB-backed, idempotent steps, retries, waits) |
| `mt-notify` | native | Channels, templates, deliveries, provider adapters, market messaging rules |
| `mt-integrate` | native | Inbox, DB/REST/file sources, mapping engine, xref, quarantine, reconciliation, destinations, change feed, import/export |
| `mt-identity` | native | Local accounts (argon2id), passkeys (WebAuthn), OIDC client, SCIM 2.0 server, LDAP sync (optional, `ldap3`), sessions, API tokens |
| `mt-metrics` | native | Certified metrics, aggregations from the column cache, dashboard data |
| `mt-ai` | native (+ wasm for typed schemas) | Gateway, model registry, routing, skills runtime, agents runtime, assistant intent router, bounded planner, build pipeline, budgets & metering |
| `mt-ai-eval` | native | Evaluation harness, recorded providers, benchmarks |
| `mt-templates` | native | Components K1–K23, template packages, overlays, change sets, 3-way upgrade |
| `mt-trace` / `mt-audit` | native | Trace ids, journal, hash-chained audit, completeness verifier |
| `mt-store` | native | Dialect layer (sqlx + sea-query) for SQLite/MySQL/PostgreSQL, DB-per-tenant routing, migration orchestrator, column cache |
| `mt-api` | native | Public API and Admin API handlers (axum), OpenAPI generation (`utoipa`), versioning, rate limits |
| `mt-server` | native | Process wiring: HTTP/WebSocket server, base actors, job workers, static assets, configuration |
| `mt-mcp` | native | MCP server exposing capability-checked tools |
| `mt-wasm` | wasm | wasm-bindgen facade over shared crates for the web app |
| `mt-grid` | wasm | Canvas grid engine (rendering, hit testing, selection, keyboard, editor overlay geometry, ARIA mirror data) |
| `mt-plugins` (later) | native | Sandboxed WebAssembly plugin host (wasmtime), capability-scoped |
| `xtask` | native | Local CI gate, size budgets, coverage gates, template validation, SDK generation |

Workspace lints: `unsafe_code = forbid` everywhere; exhaustive-match lints (doc 04 §7.2).

### 4.2 Runtime model
- **tokio** runtime. One **base actor** per active base serialises writes and assigns the per-base sequence, which gives ordering, audit and broadcast. Reads bypass the actor.
- **Operation pipeline:** decode → authenticate (dual principal for AI) → `mt-perm` → `mt-core` validate → `mt-lifecycle` guards → apply in a DB transaction (records, sort keys, history, journal) → commit → ack → permission-filtered broadcast → enqueue dependent work.
- **Jobs:** a DB-backed queue per tenant (`SKIP LOCKED` on PostgreSQL/MySQL 8, single writer on SQLite). Every job has a trace id and reaches a terminal state.
- **Storage:** one database per tenant, physical table per user table, Rust-computed sort keys, junction tables (doc 04 §3). Object storage via OpenDAL (filesystem or S3-compatible).
- **Column cache:** per tenant, loaded on demand, with LRU and budget (doc 06 §5).
- **Search:** Tantivy per tenant.
- **Vectors (opt-in):** int8 vectors in tenant DB BLOBs + pure-Rust HNSW (spike S7).

### 4.3 APIs

| API | Purpose | Consumers | Format |
|---|---|---|---|
| **Public API v1** | Business data and objects: bases, tables, fields, records, views, comments, attachments, reference data, lifecycles, approvals, tasks, rules, workflows, dashboards/metrics, AI skills/agents (use) | Integrators, SDKs, the web app, the client app | REST/JSON, OpenAPI 3.1 |
| **Admin API v1** | Administration and integration: tenants (provision, suspend, delete, region, quotas), members & groups, SSO/OIDC/SCIM settings, security policies, capability sets & access reviews, data sources & destinations, inboxes, credentials (write-only secrets), templates (install, upgrade, overlays, change sets), AI providers, models, budgets & usage, notification channels, audit & trace queries, retention & data requests, jobs, backups, health, metrics, migration status | Admin console UI, operators, enterprise IT automation, integration platforms | REST/JSON, separate OpenAPI 3.1 document |
| Real-time | Live editing and presence | Web app, client app | WebSocket, binary frames (`mt-ops`), decoded in WASM/native |
| Inbox | Data sources pushing data | External systems | `POST /in/{inbox}` JSON/NDJSON/CSV/XLSX; sync ≤ 500 records, async jobs above |
| Change feed & events | Outbound integration | Integrators | `GET /changes?since=` (cursor); signed webhooks in **CloudEvents 1.0** JSON |
| MCP | AI agents and tools | External AI clients | MCP (stdio/HTTP) |

**Admin API rules:**
- **Scope and authentication:** separate scopes (`admin:*`) and token types. Instance-level operations are available only to instance admins; tenant-level ones to tenant admins. It supports service accounts for automation (client credentials) and audit-logs every call.
- **Idempotency:** every mutating endpoint accepts an `Idempotency-Key`. Long operations (tenant provisioning, template upgrade, backups) return a job resource.
- **Coverage:** the admin console uses **only** the Admin API, so everything the console can do, automation can do too.
- **Stability:** a spec-diff check blocks breaking changes within a major version, and SDKs (TypeScript, Python, Go, Java) are generated from both specs. A Java SDK is still useful for customers integrating from Java, with no Java on our side.

### 4.4 AI runtime (implementation of doc 18)
- **Gateway:** async provider adapters (reqwest + rustls); model registry and routing; structured outputs (JSON Schema via `schemars` from field types and reference sets); retries, fallback and circuit breaker; metering and budgets.
- **Skills and agents:**
  - skills produce **suggestions or operations** that go through the pipeline;
  - agents are principals whose loop runs on the durable workflow engine;
  - every tool call is capability-checked against the action policy;
  - knowledge retrieval is permission-filtered.
- **Assistant:** intent router → bounded planner or build pipeline (plan → change set → verify → publish).
- **Optional local inference** (`candle`) behind a cargo feature.

### 4.5 Identity & security
- **Sign-in:**
  - **built in:** local accounts (argon2id), passkeys, OIDC (Okta, Entra ID, Google, Keycloak…), SCIM 2.0 provisioning, optional LDAP sync;
  - **SAML** customers use their identity provider's OIDC option or a broker that converts SAML to OIDC (e.g., Keycloak). Native SAML is a later spike (S13), not an R1 promise.
- **Sessions:** HttpOnly SameSite cookies for the browser; OAuth2/PKCE and scoped tokens for APIs; client credentials for service accounts.
- **Isolation and authorisation:** tenant isolation through DB-per-tenant with routing only from the authenticated principal; `mt-perm` enforces everywhere (browser WASM copies are UI hints only).
- **Protection:**
  - secrets encrypted at rest (envelope encryption, KMS or key file);
  - outbound HTTP protected against SSRF, with an intranet allowlist for self-host;
  - strict CSP (`script-src 'self' 'wasm-unsafe-eval'`) and Trusted Types.
- **Supply chain:** cargo-deny/audit/vet, npm audit + lockfile policy, CycloneDX SBOMs, signed images.

---

## 5. Web frontend (React + WASM)

### 5.1 Stack
| Concern | Choice |
|---|---|
| Framework | React 19 + TypeScript (strict) |
| Build | Vite + `vite-plugin-wasm` + `vite-plugin-top-level-await`; `wasm-pack --target web` |
| Routing | TanStack Router (typed, URL-addressable tabs and records) |
| Server state | TanStack Query (REST) + a real-time store fed by the WASM worker |
| Client state | Zustand + `useSyncExternalStore` bindings to WASM stores |
| UI primitives | Radix UI + design tokens (CSS variables, light/dark) |
| Forms | Schema-driven renderer; validation via `mt-core` in WASM |
| Charts | ECharts or Vega-Lite behind a chart adapter (decided in spike) |
| Designers | React Flow (workflow, approval canvases); custom form/page designers |
| API clients | Generated from the Public and Admin OpenAPI specs (`openapi-typescript`) |
| i18n | FormatJS (ICU messages); numbers/dates via `Intl` |
| Testing | Vitest + Testing Library, Playwright (Chromium/Firefox/WebKit), axe accessibility checks |

### 5.2 WASM integration
- `mt-wasm` exposes typed functions for validation and formatting, formula previews, filter evaluation, sort keys, op apply/rebase, allowed lifecycle transitions, permission hints, reference-data resolution and the binary codec. TypeScript types are generated (`tsify`).
- **Worker module (data engine):** owns the WebSocket, decodes frames, maintains windows and the op outbox, runs local filter/sort and formula previews, and passes transferable typed arrays to the main thread.
- **Main-thread module (`mt-grid`):** draws the grid to Canvas2D from typed-array snapshots.
- **Budgets (CI-enforced):** initial JS ≤ 250 KB gzip · main WASM ≤ 200 KB brotli · worker WASM ≤ 500 KB brotli · designers and dashboards code-split.

### 5.3 Grid performance path
- `<Grid/>` mounts a canvas and an editor overlay once, and updates flow imperatively from the worker to `mt-grid`.
- **DOM layers:** a native `<input>` editor overlay (IME for JA/KO), a hidden ARIA grid for screen readers, and React portals for menus and pickers.
- **Fallback:** Glide Data Grid with our worker data engine, if the Rust renderer misses targets in spike S1.
- **Targets:** ≥ 55 fps at 1M rows · keystroke-to-paint ≤ 16 ms · heap ≤ 64 MB.

### 5.4 App structure
```
apps/web/src
  app/            shell, routing, tabs, command bar (assistant)
  features/       grid · views · record-page · forms · dashboards · process · rules-workflows ·
                  access · connections · ai · templates · home · admin (Admin API only)
  platform/       runtime adapter: browser (WASM worker) | tauri (native commands)   ← enables client app reuse
  wasm/           worker entry, RPC, generated bindings
  api/            generated Public/Admin API clients
  design-system/  tokens, Radix-based components, themes
  i18n/  test/
```

---

## 6. Future client app (Tauri 2 + React)

- **Same UI:** the React app is built once. A small **runtime adapter** (`platform/`) switches between the browser runtime (WASM worker + HTTP/WebSocket) and the Tauri runtime (native Rust commands + the same protocol).
- **Native core:**
  - the client embeds the shared Rust crates natively (`mt-core`, `mt-ops`, `mt-query`, `mt-formula`, `mt-perm`, `mt-lifecycle`, `mt-refdata`), so heavy local work runs at native speed;
  - **offline cache:** a local SQLite replica of permitted data, with an op outbox that the server re-validates on reconnect (doc 02 B5);
  - **OS integration:** notifications, deep links, file drag-and-drop, document scanning on mobile.
- **Targets:** Windows, macOS and Linux first; iOS and Android later (Tauri 2 supports mobile targets **[verify per feature in a spike]**).
- **What this requires now:**
  - keep the `platform/` adapter boundary in the web app;
  - keep shared crates free of server-only dependencies;
  - make the binary protocol transport-agnostic.

No other work is scheduled before the client app is approved.

---

## 7. Contracts & code generation

| Contract | Source of truth | Generated for |
|---|---|---|
| Public API | Rust handlers + `utoipa` → `openapi-public.json` | TS client (web), SDKs (TS, Python, Go, Java), docs site |
| Admin API | Rust handlers + `utoipa` → `openapi-admin.json` | TS client (admin console), SDKs, docs site |
| Real-time protocol | `mt-ops` types | Rust native + WASM; TS types via `tsify` |
| Events | CloudEvents envelope + JSON Schemas from Rust types | All consumers |
| Template packages | JSON Schema from `mt-templates` | Validator, designer UI |
| AI output schemas | JSON Schema from field types / reference sets | Gateway, suggestion UI |

CI checks: both OpenAPI specs are diffed for breaking changes; SDKs are regenerated and compiled; template packages are validated.

---

## 8. Data architecture & ownership

| Store | Contents |
|---|---|
| Control-plane DB | Tenants, accounts, sessions, API tokens, routing, instance settings, migration state |
| Tenant DB (one per tenant) | All tenant metadata and business data, journal, audit chain, jobs |
| Object storage | Attachments, exports, thumbnails, extracted text, backups |
| Search index (per tenant) | Tantivy segments (rebuildable) |
| Browser IndexedDB / client SQLite (later) | Schema cache, recent windows, offline replica (client app) |

**Backups:**
- per-tenant DB dump or SQLite file copy + object storage snapshot;
- point-in-time restore through the journal;
- restore drills in CI on all three engines;
- backups and restores are available through the Admin API.

---

## 9. Cross-cutting concerns

| Concern | Solution |
|---|---|
| Observability | OpenTelemetry: `traceparent` created in the browser per action → server (`tracing` + OTLP) → outbound calls; business trace linked by trace id; metrics exposed for Prometheus |
| Configuration | Environment variables + config file; business settings in the parameter registry; feature flags |
| Rate limiting | Per tenant, token, IP and endpoint; separate quotas for Admin API |
| Time & money | UTC instants; exact decimals (`rust_decimal`; strings in JSON) |
| i18n | Browser formatting via `Intl`; server-side formatting for exports/notifications via ICU4X |
| Accessibility | WCAG 2.2 AA; ARIA grid mirror; keyboard maps; axe checks |

---

## 10. Deployment topologies

| Topology | Components | Target |
|---|---|---|
| **Starter** | 1 container (server + web app) + SQLite + local files | ≤ 5-minute install |
| **Standard** | server + worker role + PostgreSQL or MySQL + S3-compatible storage | Teams and companies (compose profiles) |
| **Enterprise** | Standard + OIDC/SCIM with the corporate identity provider, HA (2+ server replicas with base-actor placement), external backups, monitoring | Larger self-hosted deployments |
| **Cloud** (later) | Kubernetes (Helm): server replicas, worker pool, regional PostgreSQL (schema-per-tenant mode for scale) | Managed service per region |

The image is distroless with a static musl binary + web assets; its size is tracked in CI.

---

## 11. Code reuse map

| Logic | Server | Browser (WASM) | Client app (native, later) |
|---|---|---|---|
| Field types, validation, formats, tax-ID checks | ✓ authority | ✓ instant feedback | ✓ |
| Reference data resolution | ✓ | ✓ | ✓ |
| Sort keys, filter evaluation | ✓ | ✓ (loaded windows) | ✓ (offline replica) |
| Formula evaluation | ✓ authority | ✓ previews | ✓ offline previews |
| Operations codec, apply/rebase | ✓ | ✓ | ✓ |
| Permission evaluation | ✓ enforcement | ✓ hints | ✓ hints/offline gating |
| Lifecycle transitions, SLA calendars | ✓ | ✓ | ✓ |
| React UI | — | ✓ | ✓ (same build) |

---

## 12. Repository, build and CI

### 12.1 Monorepo layout
```
/crates/         Rust workspace (§4.1)
/apps/web/       React + TypeScript (Vite)
/apps/desktop/   Tauri 2 shell (later; reuses apps/web)
/sdks/           Generated SDKs from Public and Admin OpenAPI specs
/templates/      Solution packages and component definitions
/deploy/         Dockerfile, compose profiles, Helm (later)
/docs/           Product and technical design
/xtask/          Local CI gate, generators, budgets
```

### 12.2 Local gate (pre-push hook, `cargo xtask ci`)
- **Rust:** fmt, clippy (`-D warnings` + exhaustive-match lints), tests on SQLite (+ MySQL/PostgreSQL when available), coverage gate, WASM build + size budgets, cargo-deny.
- **Web:** typecheck, eslint, Vitest with coverage.
- **Contracts:** OpenAPI spec diffs (Public, Admin), SDK generation compiles, template validation.

### 12.3 GitHub Actions
| Workflow | Content |
|---|---|
| `ci.yml` (PR) | Rust on {ubuntu, windows, macos}; DB matrix {SQLite, MySQL, PostgreSQL} on Linux; coverage; web Vitest + Playwright {Chromium, Firefox, WebKit}; contracts & budgets |
| `main.yml` | Full OS × DB × browser matrix; Docker image; 5-minute install E2E |
| `nightly.yml` | Branch coverage (nightly Rust), fuzzing, mutation testing (cargo-mutants, Stryker for TS), AI evals with recorded providers, performance benchmarks |
| `release.yml` | Signed image, SBOMs, SDKs, changelog |

---

## 13. Decisions, trade-offs, risks

| Decision | Trade-off | Mitigation |
|---|---|---|
| All-Rust backend (no JVM platform) | No Java extension path for customers; fewer ready-made enterprise connectors | Public + Admin APIs, SDKs (incl. Java), webhooks, change feed, MCP; integration platforms push to inbox; Rust connectors added on demand; WASM plugins later |
| OIDC instead of native SAML in R1 | SAML-only customers need a broker | Document Keycloak/IdP bridging; spike S13 for native SAML |
| React for the UI | Virtual DOM; larger JS | Hot paths outside React; budgets; code splitting |
| Own canvas grid in Rust | Engineering effort | Glide Data Grid fallback (S1) |
| DB per tenant on three engines | Migration fan-out, CI cost | Migration orchestrator; Linux DB matrix on PRs, full matrix on main |
| Tauri client later | Must keep UI/runtime separation from day one | `platform/` adapter boundary; shared crates without server dependencies |

### Spikes before R1 build
| Spike | Scope |
|---|---|
| S1 | Grid: Rust canvas in React vs Glide |
| S2 | Bundle & WASM size |
| S3 | Storage on three DBs at 1M rows |
| S4 | Real-time base actor |
| S5 | CI matrix |
| S6 | Trace & outbox |
| S7 | Vectors |
| S8 | Planner benchmark |
| S9 | Capacity model |
| S10 | Approval model checking |
| S11 | Integration ease |
| **S12** | Admin API coverage: the admin console built only on the Admin API |
| **S13** | Native SAML feasibility in Rust (optional) |
| **S14** | Tauri 2 runtime adapter proof (later, before the client app) |

### Engineering milestones
| Milestone | Scope |
|---|---|
| **M0** | Repository scaffold, CI, OpenAPI pipelines, spikes S1–S12 |
| **M1** | Data plane (D1–D6) + React shell + grid |
| **M2** | Process, rules/workflows, notifications |
| **M3** | Connections, analytics, access, Admin API complete |
| **M4** | AI R1 + templates (HR & Admin, E-commerce after-sales, Customer service) |
| **R1** | Release |

---

## Sources
- React + Rust/WASM: [Vite + Rust WASM example](https://dev.to/jambochen/using-rust-webassembly-in-vite-react-a-modern-game-of-life-example-hde), [Integrating Rust & JS workflows 2026](https://dasroot.net/posts/2026/02/integrating-rust-javascript-workflows/), [WASM with React guide](https://reintech.io/blog/how-to-use-webassembly-with-react-integration-guide), [wasm-bindgen-rayon](https://docs.rs/wasm-bindgen-rayon), [wasmworker](https://docs.rs/crate/wasmworker/0.4.0)
- Earlier technical research: docs 02, 04, 06 (grid, storage, CRDT, multi-tenancy, coverage, CI).

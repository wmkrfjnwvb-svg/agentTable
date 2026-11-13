# Technical Architecture & Solution (v1.0)

> **Status: authoritative technical design for development.** Implements the product specification in
> docs 17 (product), 18 (AI) and 19 (parity & delivery).
> **Stack decision (stakeholder, this round):** core in **Rust**; frontend in **React + TypeScript** reusing
> Rust through **WebAssembly**; **Spring Boot** for the enterprise **management & integration platform**.
> This supersedes doc 04 D2 (Leptos). The earlier performance intent, *no virtual-DOM work on the hot path*,
> is kept: React renders the shell, and the grid and data engine run outside React's render cycle (§5.3).

---

## 1. Architecture principles

1. **Rust owns the data plane and all business logic:** records, operations, formulas, queries, permissions, process, rules/workflows, AI runtime, audit. Business logic exists in exactly one implementation.
2. **Write once, run in three places.** Pure Rust core crates compile to:
   - native, for the server;
   - **WASM**, for the browser (React);
   - a **C ABI library**, optionally for Java via the JDK Foreign Function & Memory API (finalised in JDK 22).

   Java and TypeScript never re-implement validation, formulas or permission rules.
3. **React is a thin, fast shell:** layout, dialogs, forms, navigation. Hot paths (grid rendering, data windows, decoding, local evaluation) run in Rust/WASM, partly in a Web Worker.
4. **Spring Boot is the enterprise edge, not a second core.** It hosts management, enterprise integration adapters and customer-specific extensions. It talks to the core only through public/internal APIs and **never connects to tenant databases**.
5. **Contracts first:**
   - Protobuf (buf) for internal service APIs;
   - OpenAPI 3.1 (generated from Rust) for the public REST API;
   - a binary wire codec for the browser ↔ core real-time channel, shared by Rust native and WASM.
6. **The Core runs standalone.** The Rust core + React app ships as **one Docker image** (5-minute self-host). The Spring platform is an **optional** image for enterprise deployments.
7. **Inbound-only data sources, explicit outbound destinations** (doc 09), enforced at service boundaries.
8. **Quality gates per language:**
   - Rust 100% function/line (business crates), ≥ 98% branch;
   - TypeScript ≥ 98% lines (vitest + Playwright);
   - Java ≥ 98% lines (JaCoCo);
   - every DB-touching test on SQLite/MySQL/PostgreSQL.

---

## 2. System context

```
                    ┌──────────────── Users ────────────────┐
                    │ Browser (React + WASM) · Mobile web   │
                    └───────────────┬───────────────────────┘
                                    │ HTTPS (REST/JSON) + WebSocket (binary ops)
      External systems              ▼
  (ERP, HR, shops, DBs,   ┌─────────────────────────────────────────────┐      AI providers
   files, chat, email) ──►│              CORE (Rust)                     │────► (Claude, OpenAI-compatible,
   via data sources       │  API gateway · realtime · data plane ·      │       local models, document AI)
                          │  process · rules/workflows · AI runtime ·   │
  Developers / agents ───►│  integration (sources/destinations) · audit │────► Notification providers
   (REST, SDKs, MCP)      └───────────┬───────────────────┬─────────────┘       (SMTP, SMS, chat)
                                      │ gRPC (internal)   │ events / change feed
                          ┌───────────▼───────────────────▼─────────────┐
                          │   MANAGEMENT & INTEGRATION PLATFORM          │──► Enterprise systems
                          │   (Spring Boot, optional)                    │    (SAP, Workday, Salesforce,
                          │   admin console API · integration hub ·     │     LDAP/AD, JMS/Kafka, SOAP)
                          │   identity federation · extension SDK/BFF   │
                          └─────────────────────────────────────────────┘
  Storage: control-plane DB · one DB per tenant (SQLite/MySQL/PostgreSQL) · object storage (FS/S3) · platform DB
```

---

## 3. Containers & responsibilities

| Container | Tech | Responsibilities | Owns data |
|---|---|---|---|
| **core-server** | Rust (tokio, axum, tonic) | REST API, WebSocket real-time, auth (local, OIDC), tenancy routing, operation pipeline, queries, formulas, permissions, process, rules & workflows, notifications, sources/destinations, import/export jobs, AI runtime & gateway, MCP server, audit & trace, static hosting of the React app | Control-plane DB; tenant DBs; object storage |
| **web-app** | React 19 + TypeScript, Vite, WASM core | App shell, views, grid (WASM canvas engine), forms, dashboards, designers (approval, workflow, form, page), assistant UI, admin UI for core settings | Browser cache only (IndexedDB) |
| **platform** (optional) | Spring Boot 4.1, Java 21/25, Spring Modulith | Instance & tenant management console API, licensing/usage (cloud), enterprise integration hub (Spring Integration/Camel adapters), identity federation (SAML 2.0, LDAP/AD via Spring Security), customer extension SDK and BFFs, scheduled enterprise reports to external systems | Platform DB (management metadata, integration configs, audit of platform actions) |
| **worker pool** (scale-out) | Same Rust binary, `--role worker` | Long jobs: imports/exports, batch AI, type changes, recompute, sync runs | — (uses tenant DBs) |

A single deployment can run `core-server` alone (SQLite, one container). Enterprise deployments add PostgreSQL/MySQL, `platform` and workers.

---

## 4. Rust core design

### 4.1 Workspace crates

| Crate | Targets | Contents |
|---|---|---|
| `mt-core` | native · wasm · ffi | IDs, field types, values (exact decimal, UTC instants), validation & format library, coercion, sort-key encoding, provenance types |
| `mt-ops` | native · wasm | Operation model, binary codec (postcard), apply/rebase, client op ids |
| `mt-query` | native · wasm | Filter/sort/group/aggregate AST; in-memory evaluator; SQL compiler (server feature) |
| `mt-formula` | native · wasm | Parser, type checker, evaluator, dependency graph, incremental recompute |
| `mt-perm` | native · wasm · ffi | Capability model, grant/scope/condition evaluation, SoD, explain |
| `mt-lifecycle` | native · wasm | Lifecycles, state categories, transitions, SLA clocks & business calendars, approval state machine |
| `mt-refdata` | native · wasm · ffi | Reference sets, trees, effective sets, crosswalk lookups |
| `mt-rules` | native | Rule model, trigger matching, policies (throttle, digest, quiet hours) |
| `mt-workflow` | native | Durable workflow engine (DB-backed, idempotent steps, retries, waits) |
| `mt-notify` | native | Channels, templates, deliveries, provider adapters, market rules |
| `mt-integrate` | native | Inbox, DB/REST/file sources, mapping engine, xref, quarantine, reconciliation, destinations, change feed, import/export |
| `mt-metrics` | native | Certified metrics, aggregations from column cache, dashboards data API |
| `mt-ai` | native (+ wasm for typed schemas) | Gateway, model registry, routing, skills runtime, agents runtime, assistant intent router, bounded planner, build pipeline, budgets & metering |
| `mt-ai-eval` | native | Evaluation harness, recorded providers, benchmarks |
| `mt-templates` | native | Components (K1–K23), template packages, overlays, change sets, 3-way upgrade |
| `mt-trace` / `mt-audit` | native | Trace ids, journal, hash-chained audit, completeness verifier |
| `mt-store` | native | Dialect layer (sqlx + sea-query) for SQLite/MySQL/PostgreSQL, DB-per-tenant routing, migrations orchestrator, column cache |
| `mt-server` | native | axum REST + WebSocket, tonic gRPC, auth, tenancy, base actors, static assets |
| `mt-mcp` | native | MCP server exposing capability-checked tools |
| `mt-wasm` | wasm | wasm-bindgen facade over shared crates for the React app |
| `mt-grid` | wasm | Canvas grid engine (rendering, hit testing, selection, keyboard, editor overlay positions, ARIA mirror data) |
| `mt-ffi` | cdylib | Stable C ABI over selected pure crates for Java FFM (optional) |
| `xtask` | native | Local CI gate, size budgets, coverage gates, template validation, code generation |

Workspace lints: `unsafe_code = forbid` (except `mt-ffi`, which is reviewed); exhaustive-match lints (doc 04 §7.2).

### 4.2 Runtime model
- **tokio** multi-threaded runtime. One **base actor** per active base serialises writes and assigns the per-base sequence, which gives ordering, audit and broadcast. Reads bypass the actor and query committed state.
- **Operation pipeline:** decode → authenticate (dual principal for AI) → `mt-perm` check → `mt-core` validate → `mt-lifecycle` guards → apply in a DB transaction (record table, sort keys, history, journal) → commit → ack → broadcast (permission-filtered) → enqueue dependent work (recompute, rules).
- **Jobs:** a DB-backed queue per tenant (`SKIP LOCKED` on PG/MySQL 8, single writer on SQLite). Workers claim jobs. Every job has a trace id and reaches a terminal state.
- **Storage:** one database per tenant (SQLite file / MySQL database / PostgreSQL database or schema mode). One physical table per user table, columns named by field id, Rust-computed sort keys, junction tables for links and multi-values (doc 04 §3). Object storage via OpenDAL (filesystem default, S3-compatible).
- **Column cache:** per-tenant, columnar, loaded on demand (doc 06 §5), with LRU and budget. It is used for aggregates, formula fan-out and dashboards.
- **Search:** Tantivy index per tenant (text fields, attachments' extracted text), updated from the journal.
- **Vectors (opt-in):** int8 vectors in tenant DB BLOBs + an in-process pure-Rust HNSW index per tenant (spike S7).

### 4.3 APIs exposed by the core
| API | Consumers | Format |
|---|---|---|
| Public REST v1 | Integrators, SDKs, platform | OpenAPI 3.1 generated from Rust (`utoipa`); JSON |
| Real-time | Browser | WebSocket, binary frames (postcard via `mt-ops`); decoded in WASM |
| Change feed & events | Integrators, platform | `GET /changes?since=` (cursor), signed webhooks; **CloudEvents 1.0** JSON envelope |
| Internal gRPC | Platform | Protobuf via buf: tenant provisioning, admin queries, integration push, usage export, identity sync |
| MCP | AI agents/tools | MCP over stdio/HTTP |
| Inbox | Data sources | `POST /in/{inbox}` JSON/NDJSON/CSV/XLSX; sync ≤ 500 records, async jobs above |

### 4.4 AI runtime (implementation of doc 18)
- **Model gateway:** async HTTP provider adapters (reqwest + rustls), a model registry and routing policy, structured-output enforcement (JSON Schema generated with `schemars` from field types/reference sets), retries/fallback/circuit breaker, metering.
- **Skills:** stateless executors. Inputs come from the operation context. Outputs become **suggestion or operation** objects sent back through the pipeline. Cache key = hash(inputs, version, model).
- **Agents:** run as principals.
  - **Execution:** the agent loop is a workflow-engine state machine (durable, step-limited); tool calls are capability-checked; the action policy (auto/confirm/forbidden) is enforced in the runtime.
  - **Inputs and state:** knowledge retrieval is permission-filtered; memory is stored per user.
- **Assistant:** intent router → planner (bounded templates) or build pipeline (plan → change set → verify).
- **Local inference (optional feature):** embeddings/OCR using pure-Rust ML (`candle`) behind a cargo feature, so default binaries stay small.

### 4.5 Security architecture (core)
- **Authentication:**
  - local accounts (argon2id), passkeys (WebAuthn), OIDC;
  - SAML via the platform or an external IdP broker;
  - session cookies (HttpOnly, SameSite) for the browser;
  - OAuth2/PKCE and scoped tokens for the API;
  - mTLS or signed service tokens between core and platform.
- **Authorization:** `mt-perm` everywhere. The browser copy (WASM) is used only for UI hints and never for enforcement.
- **Isolation and secrets:** tenant isolation through DB-per-tenant + routing from the authenticated principal only; secrets encrypted at rest (envelope encryption, key from KMS or a file).
- **Outbound:** SSRF guard for outbound HTTP, with an intranet allowlist for self-host.
- **Browser hardening:** strict CSP (`script-src 'self' 'wasm-unsafe-eval'`), Trusted Types, no user HTML injection.
- **Supply chain:** cargo-deny/audit/vet, npm audit + lockfile policy, OWASP Dependency-Check for Java; SBOM (CycloneDX) for all three ecosystems; signed images.

---

## 5. Frontend architecture (React + WASM)

### 5.1 Stack
| Concern | Choice | Reason |
|---|---|---|
| Framework | React 19 + TypeScript (strict) | Ecosystem, hiring, component libraries |
| Build | Vite with `vite-plugin-wasm` + `vite-plugin-top-level-await`; `wasm-pack --target web` ([Vite + Rust WASM guide](https://dev.to/jambochen/using-rust-webassembly-in-vite-react-a-modern-game-of-life-example-hde), [dasroot 2026](https://dasroot.net/posts/2026/02/integrating-rust-javascript-workflows/)) | Fast builds, first-class WASM loading |
| Routing | TanStack Router (typed routes, URL-addressable tabs and records) | Deep links, app-level tabs |
| Server state | TanStack Query (REST) + a custom real-time store fed by the WASM worker | Caching and invalidation; the real-time path bypasses JSON |
| Client state | Zustand (UI state) + `useSyncExternalStore` bindings to WASM stores | Minimal re-renders |
| UI primitives | Radix UI primitives + our design tokens (CSS variables, light/dark) | Accessible, unstyled, themable |
| Forms | Schema-driven form renderer; validation calls `mt-core` in WASM (same rules as server) | One validation implementation |
| Charts | ECharts or Vega-Lite (decide in spike) behind a chart adapter; data from `mt-metrics` | Rich chart types (radar, funnel, word cloud) |
| Designers | React Flow for workflow and approval canvases; custom form/page designers | Mature canvas editing |
| i18n | FormatJS (ICU messages); number/date via `Intl` | en first; ja/ko/de/hi ready |
| Testing | Vitest (+ coverage), Testing Library, Playwright (Chromium/Firefox/WebKit) | Coverage gates and cross-browser |

### 5.2 WASM integration
- Package `mt-wasm` (wasm-bindgen) exposes typed functions:
  - **validation:** `validateCell`, `formatValue`, `parseInput`;
  - **evaluation:** `evalFormulaPreview`, `evalFilter`, `sortKey`;
  - **operations:** `applyOps`/`rebase`;
  - **process and access:** `lifecycleAllowedTransitions`, `permHints`;
  - **reference data:** `refsetResolve`;
  - **binary codec:** `decodeWindow`/`encodeOps`.
- Generated TypeScript types (`tsify`/`serde-wasm-bindgen`) keep TS and Rust in sync.
- **Two WASM instances:**
  1. **Worker (data engine):** owns the WebSocket, decodes frames, maintains windows and the op outbox, runs local filter/sort on loaded windows and formula previews. Messages to the main thread are transferable typed arrays (Comlink-style RPC).
  2. **Main thread (grid engine):** `mt-grid` draws to Canvas2D from typed-array snapshots (no JSON, no React state per cell).
- Main-thread WASM stays small. Heavy code (formula engine, codec) lives in the worker module, and both modules are lazy-loaded per route.
- **Size budgets (CI-enforced):**
  - initial JS ≤ 250 KB gzip;
  - main WASM ≤ 200 KB brotli;
  - worker WASM ≤ 500 KB brotli;
  - designers and dashboards are code-split.

### 5.3 Grid integration with React (performance path)
- `<Grid/>` is a React component that mounts a `<canvas>` and an absolutely-positioned editor overlay. React renders it once; updates flow imperatively from the worker → `mt-grid`, not through React reconciliation.
- **DOM layers:**
  - **editor overlay:** a real `<input>`/`<textarea>`, so IME works for JA/KO;
  - **ARIA mirror:** a hidden `role="grid"` for the visible window;
  - **context menus and pickers:** React portals.
- **Fallback option evaluated in spike S1:** Glide Data Grid (React canvas grid, MIT) with our worker data engine, if the Rust canvas renderer doesn't meet the thresholds.
- **Targets:** ≥ 55 fps at 1M rows; keystroke-to-paint ≤ 16 ms; heap ≤ 64 MB.

### 5.4 App structure
```
apps/web/src
  app/            shell, routing, tabs, command bar (assistant)
  features/
    grid/ views/ record-page/ forms/ dashboards/ process/ rules-workflows/
    access/ connections/ ai/ templates/ admin-core/ home/
  wasm/           worker entry, RPC, typed bindings (generated)
  design-system/  tokens, components (Radix-based), themes
  i18n/  test/
```

---

## 6. Management & integration platform (Spring Boot)

### 6.1 Why Spring Boot here
- **Enterprise integration ecosystem:** SAP/RFC, SOAP, JMS, Kafka, LDAP/AD, SAML, Salesforce and Workday connectors via Spring Integration/Apache Camel.
- **Enterprise customers' Java teams** can build **custom extensions and portals** without touching the Rust core.
- **Tooling:** Spring Boot 4.1 (Java 17 baseline; Java 21/25 recommended) has gRPC auto-configuration backed by Spring gRPC 1.1 and OpenTelemetry improvements ([InfoQ Spring Boot 4.1](https://infoq.com/news/2026/06/spring-boot-4-1), [Spring gRPC 1.1.0](https://spring.io/blog/2026/06/10/spring-grpc-1-1-0-available-now/), [Spring gRPC 1.0 GA](https://spring.io/blog/2025/12/04/spring-grpc-1/), [Spring Boot 4 overview](https://www.danvega.dev/blog/2025/11/24/spring-boot-4-is-here)).

### 6.2 Modules (Spring Modulith, one deployable)
| Module | Responsibilities | Talks to core via |
|---|---|---|
| `platform-admin` | Instance and tenant lifecycle orchestration (create/suspend/delete/region/quotas), plan & licence (cloud), usage reporting, release & migration status console, support tooling | gRPC (TenantAdmin, Usage, Migrations) |
| `integration-hub` | Enterprise adapters (SAP, Workday, SuccessFactors, Salesforce, Dynamics, LDAP, SFTP, JMS/Kafka, SOAP) running as **data sources** (push into the core inbox) and **destinations** (consume change feed/events → call enterprise APIs); retries, dead letters, mapping previews | REST inbox, change feed, gRPC IntegrationPush |
| `identity-federation` | SAML 2.0 SP/IdP brokering, LDAP/AD sync → SCIM into the core, MFA policies for enterprises | OIDC tokens issued for the core; SCIM endpoints on the core |
| `extension-sdk` | Java SDK (generated from OpenAPI/proto), BFF starter for customer-specific portals and back-office screens, webhook verification helpers | Public REST/gRPC |
| `reporting-export` | Scheduled enterprise exports (data warehouse, BI), formatted reports to file shares | Change feed, report API |

### 6.3 Boundary rules (anti-duplication)
1. Platform modules **never connect to tenant databases** and never write business data except through the core's APIs (inbox, operations API).
2. Business rules (validation, permissions, formulas, lifecycles) are **not re-implemented in Java**. If Java needs them locally (e.g., pre-validating a batch in an adapter before push), it calls `mt-ffi` through the **FFM API** with jextract-generated bindings, or the core's validate endpoint.
3. The platform has its **own database** (PostgreSQL/MySQL) for management metadata only.
4. Everything the platform does is audited in both the platform log and the core audit (via the principal it uses).

### 6.4 Tech choices
- Spring Boot 4.1, Spring Modulith, Spring Security (OAuth2 resource server, SAML2), Spring gRPC, Spring Integration and/or Apache Camel.
- Flyway; jOOQ or Spring Data JDBC (management DB); Micrometer + OpenTelemetry.
- Testcontainers (Linux), JUnit 5, JaCoCo, ArchUnit (enforces the boundary rules).
- Gradle (Kotlin DSL) build.

---

## 7. Contracts & code generation

| Contract | Source of truth | Generated for |
|---|---|---|
| Internal service API | `proto/` (buf, lint + breaking-change check) | Rust (prost/tonic), Java (grpc-java via Spring gRPC), TypeScript (protobuf-es, for admin UI if needed) |
| Public REST API | Rust handlers + `utoipa` → `openapi.json` | TS SDK (openapi-typescript), Java SDK, Python SDK; spec-diff check in CI |
| Browser real-time protocol | `mt-ops` types | Rust native + WASM (same crate); TS types via `tsify` |
| Events | CloudEvents envelope + JSON Schemas generated from Rust types | All consumers |
| Templates | Template package schema (JSON Schema from `mt-templates`) | Validator in CI; designer UI |

---

## 8. Data architecture & ownership

| Store | Owner | Contents |
|---|---|---|
| Control-plane DB | core | Tenants, accounts, sessions, API tokens, routing, instance settings, migration states |
| Tenant DB (one per tenant) | core | All tenant metadata (D2–D15) and business data; journal; audit chain; jobs |
| Object storage | core | Attachments, exports, thumbnails, extracted text, backups |
| Search index (per tenant) | core | Tantivy segments (rebuildable) |
| Platform DB | platform | Management metadata, integration configs (secrets encrypted), platform audit |
| Browser IndexedDB | web-app | Schema cache, recent windows (validated by sequence on reconnect) |

Backups:
- per-tenant DB dump or SQLite file copy + object storage snapshot;
- point-in-time restore through the journal;
- restore drills in CI on all three DB engines.

---

## 9. Cross-cutting concerns

| Concern | Solution |
|---|---|
| Observability | OpenTelemetry end to end: W3C `traceparent` created in the browser per user action → core (`tracing` + OTLP) → platform (Micrometer tracing) → outbound calls; logs and metrics with tenant/base labels; business trace (doc 03 §4.4) linked by trace id |
| Configuration | 12-factor env + config files; parameter registry (C4) for business settings; feature flags |
| Rate limiting | Per tenant, token, IP and endpoint (core); per adapter (platform) |
| Multi-tenancy routing | Principal → tenant → DB locator (cached); no cross-tenant query path exists |
| Time & money | UTC instants; exact decimals (`rust_decimal`, `BigDecimal` in Java, string transport in JSON) |
| i18n | Locale formats in the browser via `Intl`; server-side formatting for exports/notifications via ICU4X (server only) |
| Accessibility | WCAG 2.2 AA; ARIA grid mirror; keyboard maps; automated axe checks in Playwright |

---

## 10. Deployment topologies

| Topology | Components | Target |
|---|---|---|
| **Starter (self-host)** | 1 container: core-server (serves React) + SQLite + local file storage | ≤ 5-minute install, laptop or small VM |
| **Standard (self-host)** | core-server (+ worker role) + PostgreSQL or MySQL + S3-compatible storage | Teams/companies; docker-compose profiles |
| **Enterprise (self-host)** | Standard + platform (Spring Boot) + platform DB; SSO/LDAP; integration hub | Enterprises with SAP/HR/ERP integration needs |
| **Cloud (later)** | Kubernetes (Helm): core replicas with base-actor placement by consistent hashing, worker pool, regional PostgreSQL clusters (schema-per-tenant mode for scale), platform for multi-tenant administration | Managed service per region (US, EU, JP, KR, IN) |

Images: `core` (distroless, static musl binary + web assets; size budget tracked), `platform` (JRE 21/25 distroless or GraalVM native image evaluated).

---

## 11. Code reuse map

| Logic | Server (Rust) | Browser (WASM) | Java (FFM, optional) |
|---|---|---|---|
| Field types, validation, formats, tax-ID checks | ✓ | ✓ (instant feedback) | ✓ (adapter pre-validation) |
| Reference data resolution | ✓ | ✓ | ✓ |
| Sort keys, filter evaluation | ✓ | ✓ (loaded windows) | — |
| Formula evaluation | ✓ (authority) | ✓ (previews) | — |
| Operations codec, apply/rebase | ✓ | ✓ | — |
| Permission evaluation | ✓ (enforcement) | ✓ (UI hints only) | ✓ (explain in admin tools) |
| Lifecycle transitions, SLA calendars | ✓ | ✓ (allowed buttons, due previews) | — |
| AI output schemas | ✓ | ✓ (typed suggestion UI) | — |

---

## 12. Repository, build and CI

### 12.1 Monorepo layout
```
/crates/            Rust workspace (§4.1)
/apps/web/          React + TypeScript (Vite)
/platform/          Spring Boot (Gradle, Spring Modulith)
/proto/             buf workspace (internal APIs)
/sdks/              generated TS/Java/Python SDKs
/templates/         solution packages (S1–S11) + component definitions
/deploy/            Dockerfiles, compose profiles, Helm (later)
/docs/              product & technical design (docs 01–20)
/tools/ (xtask)     local CI gate, generators, budgets
```

### 12.2 Local gate (pre-push hook, `cargo xtask ci`)
- **Rust:** fmt, clippy (`-D warnings` + exhaustive-match lints), tests on SQLite (+ MySQL/PG if available locally), coverage gate, WASM build + size budget, cargo-deny.
- **TypeScript:** typecheck, eslint, vitest with coverage.
- **Java:** Gradle check (tests, JaCoCo gate, ArchUnit boundary rules).
- **Contracts:** buf lint/breaking, OpenAPI spec diff, template validation.

### 12.3 GitHub Actions
| Workflow | Matrix / content |
|---|---|
| `ci.yml` (PR) | Rust: {ubuntu, windows, macos} × tests; DB matrix {SQLite, MySQL, PostgreSQL} on Linux; coverage. Web: vitest + Playwright {Chromium, Firefox, WebKit}. Java: JDK {21, 25} tests + JaCoCo. Contracts & budgets |
| `main.yml` | Full OS × DB × browser matrix; Docker images; 5-minute install E2E |
| `nightly.yml` | Branch coverage (nightly Rust), fuzzing, mutation testing (cargo-mutants, PIT for Java, Stryker for TS), AI evals with recorded providers, performance benchmarks |
| `release.yml` | Signed images, SBOMs, SDK publishing, changelog |

---

## 13. Decisions, trade-offs, risks

| Decision | Trade-off | Mitigation |
|---|---|---|
| React instead of Leptos | Virtual DOM; larger JS | Hot paths outside React (§5.3); strict bundle budgets; code splitting |
| Spring Boot platform | Second runtime, JVM footprint, dual-language team | Optional image; strict boundaries (ArchUnit); generated SDKs; no business logic in Java |
| WASM in worker + main thread | Two modules, messaging complexity | Generated typed RPC; transferable buffers; spike S1/S2 measure |
| Java FFM for shared logic | Native library packaging per OS/arch | Optional; default path is calling the core API |
| DB per tenant + three engines | Migration fan-out, CI cost | Migration orchestrator; Linux DB matrix on PRs; full matrix on main |
| Own canvas grid in Rust | Engineering effort | Glide Data Grid fallback evaluated in S1 |

### Updated spike list (before R1 build)
S1 Grid (Rust canvas in React vs Glide) · S2 bundle & WASM size · S3 storage on three DBs · S4 real-time base actor ·
S5 CI matrix (Rust + TS + Java) · S6 trace & outbox · S7 vectors · S8 planner benchmark · S9 capacity model ·
S10 approval model checking · S11 integration ease · **S12 Java FFM + `mt-ffi` packaging (Linux/Windows/macOS, x64/arm64)** ·
**S13 Spring Boot ↔ core gRPC + identity federation (SAML → core OIDC)**.

### Engineering milestones
- **M0:** repository scaffold, CI, contracts, spikes S1–S13.
- **M1:** core data plane (D1–D6) + React shell + grid.
- **M2:** process, rules/workflows, notifications.
- **M3:** connections, analytics, access.
- **M4:** AI R1 + templates (HR & Admin, E-commerce after-sales) + platform (admin, integration hub, SAML/LDAP).
- **R1 release.**

---

## Sources
- Spring: [Spring Boot 4.1 (InfoQ)](https://infoq.com/news/2026/06/spring-boot-4-1), [Spring Boot 4 overview (Dan Vega)](https://www.danvega.dev/blog/2025/11/24/spring-boot-4-is-here), [Spring Boot 4 / Framework 7 (InfoQ)](https://www.infoq.com/news/2025/11/spring-7-spring-boot-4), [Spring version compatibility](https://stevenpg.com/references/spring-version-compatibility/), [Spring gRPC 1.0 GA](https://spring.io/blog/2025/12/04/spring-grpc-1/), [Spring gRPC 1.1.0](https://spring.io/blog/2026/06/10/spring-grpc-1-1-0-available-now/)
- React + Rust/WASM: [Vite + Rust WASM example](https://dev.to/jambochen/using-rust-webassembly-in-vite-react-a-modern-game-of-life-example-hde), [Integrating Rust & JS workflows 2026](https://dasroot.net/posts/2026/02/integrating-rust-javascript-workflows/), [WASM with React guide](https://reintech.io/blog/how-to-use-webassembly-with-react-integration-guide), [wasm-bindgen-rayon](https://docs.rs/wasm-bindgen-rayon), [wasmworker](https://docs.rs/crate/wasmworker/0.4.0)
- Earlier technical research: docs 02, 04, 06 (grid, storage, CRDT, multi-tenancy, coverage, CI).

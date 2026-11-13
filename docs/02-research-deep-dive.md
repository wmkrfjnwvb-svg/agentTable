# Research Deep Dive — Product, Technology, Compliance, Quality

> Status: research v0.2 · Builds on `01-research-requirements.md`.
> Constraints given by stakeholders (2026-11):
> 1. Markets: US, Japan, South Korea, Germany, India. Primary language: English.
> 2. Open source. Core logic in Rust. Browser first; desktop later. Grid rendering is critical.
> 3. Fast loading / no UI lag, small Rust binary, accurate business logic, high security,
>    multi-tenancy, multi-user collaboration, **98% test coverage**.
> 4. A trace system (IM-style delivery tracking) that later becomes a trace-audit system
>    proving workflow completeness.
> 5. Product-first: "data assets" as the core dimension; every business process must close its loop.
> 6. Local tests before push; GitHub Actions cross-platform (Windows, macOS, Linux, browsers).
> 7. Tests: standard, anomalous, edge, invalid, concurrency, cross-platform.
> 8. Design + feasibility review before implementation (see `03-design-and-feasibility.md`).
>
> Claims about third-party projects are taken from their own docs/blogs unless noted;
> numbers marked **[verify]** must be measured in our spikes before we rely on them.

---

## Part A — Product research

### A1. Why most "table products" fail to close the loop

A multidimensional table makes it easy to **store** business data. Enterprises don't pay for
storage; they pay for **outcomes**: a request gets handled, an order gets paid, an asset gets
returned. Reviewing Lark Base, Airtable and the open-source clones (Teable, NocoDB, Baserow,
APITable, Grist), the feature lists are broad, but the same gaps recur — each is a place where
the business loop silently breaks:

| Feature that *looks* complete | Where the loop breaks | What closes it |
|---|---|---|
| Form collects requests | Submission lands in a table; nobody owns it; no SLA | Auto-assignment rule + owner field + SLA timer + "unassigned" exception |
| Single-select "Status" column | Anyone can jump from *New* to *Done*; required data missing at *Done* | **Lifecycle (state machine)**: allowed transitions, required fields per state, who may transition |
| Automation sends a notification | Fire-and-forget: no proof it was read or acted on | Assignment with **acknowledge** state; escalation if not acknowledged |
| Workflow run | Fails silently (HTTP 500, quota) — record stays half-processed | Durable runs, retries, **run ledger**, failed run → exception with owner |
| Dashboard shows a red KPI | Viewer can't get from the number to the records to the action | Every chart drills down to a filtered view; each row offers its next action |
| AI field fills values | Values look authoritative; nobody knows the model/prompt; wrong values propagate | Provenance (model, prompt version, time), "needs review" state, human accept |
| Import from spreadsheet | Duplicates, wrong types, partial imports | Upsert by key, validation report, dry run, rollback |
| Approvals | Approved in chat; record not updated, or edited after approval | Approval writes back status + **locks** approved fields; edits require re-approval |
| Advanced permissions | Misconfigured rules expose rows; admins can't see what a user sees | "View as user" preview, periodic access review, permission change audit |
| @mention in comment | Becomes a to-do nowhere | Mention can be converted into an assignment with resolve state |
| Webhook to external system | No delivery log; lost events | Signed delivery, retries, delivery ledger, replay |
| Delete a record/field | Breaks links, lookups, formulas, workflows downstream | Impact analysis (lineage) before delete; soft delete + restore |

**Product principle P-1:** *A feature is "done" only when its loop is closed: every input has an
owner and a next step, every step records an outcome, every outcome is verifiable, and results
are measurable and feed back into the process.*

### A2. Data assets as the core dimension

We treat each table as a **data asset** with explicit metadata, not just a grid:

| Asset attribute | Product feature | Why it matters to enterprises |
|---|---|---|
| Identity | Stable IDs for base/table/field/record | References never break; API stability |
| Schema & meaning | Typed fields + descriptions + allowed values = data dictionary | Shared definitions across teams |
| Ownership | Asset owner + data steward per table | Someone answers for quality and access |
| Classification | Field tags: Public / Internal / Confidential / **PII** / Sensitive PII | Drives masking, export policy, AI access, DSAR |
| Quality | Constraints (required, unique, range, regex), quality score, exception list | Trustworthy data |
| Lifecycle | Record lifecycle state machine; retention policy; archival | Process control + compliance |
| Lineage | Graph of links, lookups, rollups, formulas, syncs, workflows, AI fields | Impact analysis, trust |
| Access | Roles, row/field rules, access review, "view as" | Least privilege, audits |
| Provenance | Every value knows its source: user / API / import / workflow / AI | Audit, debugging, AI accountability |
| Consumption | Views, dashboards, API clients, webhooks registered per asset | Know who depends on the data |
| History | Cell history, snapshots, tamper-evident audit log | Recovery, legal evidence |

**Product principle P-2:** *Every cell value is attributable (who/what/when/why), and every asset
has an owner.*

### A3. The closed loop we design for

```
  ┌──────────── Define ─────────────┐
  │ schema, lifecycle, owners, SLAs │
  └───────────────┬─────────────────┘
                  ▼
Capture ──► Validate ──► Route/Assign ──► Act ──► Verify ──► Measure ──► Improve
(form/API/   (types,      (owner, SLA,     (state   (trace:    (dashboards  (change rules,
 import/AI)   constraints, ack)            changes,  complete?  w/ drill-     schema, SLAs)
              dedupe)                      approvals) exceptions) down)           │
   ▲                                                                              │
   └──────────────────────────────────────────────────────────────────────────────┘
          every break in the chain becomes an EXCEPTION with an owner
```

The **Exceptions inbox** (records with no owner, SLA breaches, failed runs, validation failures,
unacknowledged assignments, stuck processes) is the product mechanism that guarantees the loop
can't break silently. This is the user-facing face of the trace-audit system (Part C).

### A4. Reference scenarios for validating value (used in the feasibility review)

1. **Request intake → resolution** (IT/HR/facilities/customer requests): form → triage → assign
   → work → resolve → requester notified → satisfaction → dashboard. Universal across markets.
2. **Purchase request → approval → PO → receipt → payment match**: money-related; strong audit
   needs (SOX in the US, J-SOX in Japan, K-SOX in Korea).
3. **Asset register** (IT equipment, contracts, licenses): procure → assign → maintain → renew →
   retire; recurring audits; pure data-asset scenario.

### A5. Market-specific product requirements (English first, others prepared)

| Topic | US | Germany | Japan | South Korea | India |
|---|---|---|---|---|---|
| UI language (later) | en-US | de-DE | ja-JP | ko-KR | en-IN, hi-IN |
| Date format | MM/DD/YYYY | DD.MM.YYYY | YYYY/MM/DD | YYYY. MM. DD. | DD/MM/YYYY |
| Week starts | Sunday | Monday (ISO weeks) | Sunday | Sunday | Monday/Sunday |
| Number grouping | 1,234,567.89 | 1.234.567,89 | 1,234,567 | 1,234,567 | **12,34,567** (lakh/crore) |
| Currency minor units | USD 2 | EUR 2 | **JPY 0** | **KRW 0** | INR 2 |
| Fiscal year (for date grouping) | varies (often Oct/Jan) | Jan | **Apr–Mar** common | Jan | **Apr–Mar** |
| Name order | given family | given family | **family given** | **family given** | given family |
| Text input | — | — | **IME** (kana→kanji) | **IME** (Hangul) | **IME / complex script** (Devanagari shaping) |
| Business norms | SSO (Okta/Entra), SOC 2 report expected | **Data residency in EU**, works council: employee monitoring is sensitive → audit log of user activity must be configurable | Hanko/approval chains, **J-SOX** evidence | Strict PII rules, national IDs must not be collected | Mobile-heavy users, price-sensitive, **DPDP** consent |

Product implications:
- All formatting is locale-driven; storage is locale-neutral (UTC instants, exact decimals, ISO codes).
- Currency field stores ISO 4217 code + exact decimal; precision follows the currency.
- Fiscal-year setting per base; date grouping "by fiscal quarter".
- Person-name field with configurable order (P2).
- Grid editor must be **IME-safe** (composition events) — a hard technical requirement for JA/KO.
- Canvas text must handle **complex scripts** (Devanagari, Hangul, CJK) — see B2.
- Activity auditing must be **configurable per tenant** (German works councils; GDPR minimisation).

---

## Part B — Technology research

### B1. Rendering the grid in the browser (critical path)

Options evaluated:

| Approach | How | Max practical size | Text & i18n | IME / a11y | Binary cost | Verdict |
|---|---|---|---|---|---|---|
| DOM virtualized (rows/cells as elements) | Leptos components, render visible window | ~100k cells comfortably; cost grows with columns | Browser native, perfect | Native | Small | Good for forms/sidebars, **not** for the main grid at 1M × 50 |
| **Canvas 2D, rendered from Rust/WASM** | `web-sys` CanvasRenderingContext2d; draw visible cells only | Millions of rows (only visible cells drawn) | Uses browser's `fillText` → **browser shapes CJK/Hangul/Devanagari, uses system fonts, zero font bytes in our binary** | Needs DOM overlay editor + ARIA mirror (see below) | Small | **Chosen** |
| WebGL/WebGPU (wgpu) + Rust text shaping (cosmic-text/swash) | GPU renderer | Millions | We must ship fonts + shaper. CJK fonts are 5–20 MB each | Same issues as canvas | **Large** (MBs) | Rejected: violates "small binary"; i18n risk |
| egui (immediate mode on WebGL) | Full Rust UI | Good | Own text rendering → fonts must be bundled; IME support limited in browser | Weak a11y | Medium–large | Rejected for browser product |

Evidence: Canvas grids are the standard answer for "Google Sheets-scale" grids; DOM virtualization
tops out around 100k cells ([DEV: rendering a million rows](https://dev.to/ishaaan/rendering-a-million-rows-the-ultimate-frontend-performance-guide-virtualization-streaming-ssr-ef4),
[DEV: canvas for data grids](https://dev.to/riturathin/understanding-canvas-for-high-performance-data-grids-why-virtualization-isnt-the-end-of-the-story-4co7)).
Existing Rust/WASM canvas grid engines claim 1M rows at 60 fps with Canvas2D
([rs-grid](https://rs-grid.com/docs-overview)) **[verify]** — they show it's feasible, but we
will write our own renderer (we need tight coupling with our data window, permissions,
collaboration cursors, and license control). A worker+WASM approach for sorting/filtering is
also documented ([askturret-grid](https://github.com/askturret/askturret-grid)).

Grid design consequences:
- **Draw only the viewport** (+1 screen overscan); target < 4 ms per frame for draw calls.
- **Text measurement cache** (per font + string) because `measureText` is the hot spot.
- **Editing via a DOM overlay** (`<textarea>`/`<input>` positioned over the active cell) →
  native IME composition for Japanese/Korean, native spellcheck, clipboard, mobile keyboards.
- **Accessibility**: a hidden ARIA `role="grid"` mirror of the visible window with
  `aria-rowcount/aria-rowindex/aria-colindex`, keyboard navigation handled in Rust, focus moves
  announced. Needed for US (Section 508/ADA) and EU (European Accessibility Act).
- **Skeleton cells** when data for a scrolled region hasn't arrived — scrolling is never blocked
  by the network.
- HiDPI: scale canvas by `devicePixelRatio`; pixel-snap grid lines.
- Main thread does only rendering + input; heavy work (decoding, local sort of loaded window,
  formula preview) can go to a **Web Worker** running the same WASM core.

### B2. Rust UI framework for the non-grid UI (toolbars, dialogs, forms, settings)

| Framework | Model | Reported WASM size | Notes |
|---|---|---|---|
| **Leptos** (CSR) | Fine-grained signals, no VDOM | Most often reported smallest | Large community, good docs incl. size optimisation guide |
| Dioxus | VDOM | Mixed reports | Strong desktop/mobile story (useful later) |
| Yew | VDOM | Generally larger | Mature but slower-moving |

Size reports conflict and depend on app code ([reintech](https://reintech.io/blog/leptos-vs-yew-vs-dioxus-rust-frontend-framework-comparison-2026),
[pistack](https://www.pistack.xyz/posts/2026-08-29-dioxus-vs-yew-vs-leptos-rust-wasm-frameworks-comparison/),
[Leptos binary size guide](https://book.leptos.dev/deployment/binary_size.html)) **[verify]**.

**Recommendation: Leptos (CSR) for app chrome + our own Canvas grid crate.** The grid crate
depends only on `web-sys`, so it is framework-agnostic and can later be hosted in a desktop
webview (Tauri) unchanged. Spike S2 measures Leptos vs Dioxus with our real shell.

### B3. Binary size strategy ("small Rust binary")

Budget (initial, to be enforced in CI): **app WASM ≤ 600 KB brotli** for first load,
grid+core ≤ 350 KB brotli; lazy-load rarely used modules (formula editor help, import wizard)
as separate WASM modules if needed. **[verify in S2]**

Techniques:
- Release profile: `opt-level = "z"` (compare with `"s"`), `lto = "fat"`, `codegen-units = 1`,
  `panic = "abort"`, `strip = true`; post-process with `wasm-opt -Oz`; serve brotli.
- **Keep heavy, locale/time-zone data out of the browser:**
  - Collation: ICU4X collation data is estimated at MBs ([oxitext-icu notes](https://docs.rs/crate/oxitext-icu)) **[verify]**
    → sort full datasets **on the server**; the client receives ordered windows.
  - Time zones: Jiff bundles the IANA DB on wasm by default; its `js` feature uses
    `Intl` for the system zone ([Jiff platform docs](https://docs.rs/crate/jiff/0.2.19/source/PLATFORM.md))
    → disable bundling in the client; store UTC instants; display via browser `Intl.DateTimeFormat`;
    time-zone-dependent formulas evaluated on the server.
  - Regex: the `regex` crate is large → regex-based formulas/validations evaluated server-side;
    client shows result after round-trip (still optimistic for everything else).
  - Number/date display formatting via browser `Intl` (via `js-sys`), not Rust locale data.
- Wire format: **postcard** (serde, compact, tiny code) instead of JSON on the hot path;
  JSON for the public REST API.
- Avoid `format!`-heavy code and panic messages in the client crate; audit with `twiggy`.
- CI gate fails the build when the budget is exceeded.

### B4. Fast data loading

- **Schema first, then a window**: open view → schema + view config + first ~200 rows +
  total count in one binary response; render immediately.
- **Server-side query** (filter/sort/group/permissions) over the whole table; client requests
  windows by row index (`offset..offset+n`) against a **view snapshot version**; prefetch ahead
  of scroll direction; LRU of windows in memory.
- **Columnar chunk encoding** (per-column arrays, dictionary-encoded selects/users) → smaller
  payloads and faster decode in WASM.
- Incremental updates via WebSocket: `{base_seq, op}` deltas patched into loaded windows.
- Cache schema + recent windows in IndexedDB for instant reopen (validated by seq on connect).
- Targets: first paint of any view < 1 s on a typical connection; scroll at 60 fps regardless
  of table size; cell edit visible locally < 16 ms, committed < 300 ms (p95).

### B5. Collaboration model: server-authoritative op log (not a CRDT for table data)

| Option | Fit for tables with permissions & validation |
|---|---|
| CRDT (Yrs, Loro, Automerge) | Excellent for offline text/doc merge. But: clients merge each other's ops → **the server can't reject an op that violates permissions, constraints, lifecycle rules** without breaking convergence; CRDT metadata grows; WASM cost ~180 KB (Loro) to ~320 KB (Automerge) per one 2026 guide [verify] ([pkgpulse](https://www.pkgpulse.com/guides/yjs-vs-automerge-vs-loro-crdt-libraries-2026), [Loro doc-size table](https://cn.loro.dev/docs/performance/docsize)). |
| **Server-authoritative ordered op log per base** | Server validates every op with the shared Rust core (types, constraints, lifecycle, permissions), assigns a total order (`base_seq`), broadcasts. Clients apply optimistically and reconcile. This is the model commonly described for Airtable-style products ([DesignGurus summary](https://www.designgurus.io/answers/detail/what-to-expect-in-the-airtable-system-design-interview), weak evidence) and for authoritative real-time hosts ([DEV](https://dev.to/beefedai/scaling-real-time-collaboration-architecture-best-practices-bfk)). |

**Decision: server-authoritative op log**, one logical "base actor" per base (single writer →
no races inside a base, natural ordering for audit). Conflict unit = cell; concurrent edits on
the same cell → last committed wins, the losing client is told ("Alice changed this a moment
ago") and the overwritten value stays in history. Long-text collaborative editing (character
merge) can use Yrs inside a single cell later (P2). Desktop offline later: queued ops are
re-validated by the server on reconnect; rejected ops become user-visible conflicts.

### B6. Storage model (Postgres)

| Option | Pros | Cons |
|---|---|---|
| Physical SQL table per user table, column per field (Teable/NocoDB style; Teable docs describe field name as "column name in backend database" [teable docs](https://help.teable.ai/en/api-reference/table/create-table.md)) | Native types & indexes; SQL access for BI | DDL on every schema change (locks, rewrites on type change); catalog bloat with many tenants × many tables; harder RLS; migrations per tenant |
| **Single `records` table, `cells JSONB` keyed by field_id, + `sort_keys` side table** | No DDL; schema change = metadata change; uniform RLS; partition by tenant | Need careful indexing for big tables |

**Decision (to confirm with spike S3 at 1M rows):** JSONB records, hash-partitioned by tenant,
plus a **`sort_keys(table_id, field_id, sort_key BYTEA, record_id)`** index table where the
shared Rust core encodes an **order-preserving byte key** per cell (numbers → sign-flipped big-
endian, text → collation key, dates → epoch). Postgres then sorts/filters by plain `bytea`
comparisons that are *guaranteed* to match the core's semantics — correctness and speed.
Expression/GIN indexes added automatically for fields used in large-table filters.
Analytical access is provided by the API, CSV/Parquet export, and later a read-only SQL view.

### B7. Multi-tenancy

- `tenant_id` on every row; Postgres **Row-Level Security with `FORCE ROW LEVEL SECURITY`**,
  policies with both `USING` and `WITH CHECK`; app role without `BYPASSRLS`; tenant context set
  with **`SET LOCAL` inside each transaction** (never session `SET` — leaks across pooled
  connections) ([Bytebase RLS footguns](https://www.bytebase.com/blog/postgres-row-level-security-footguns/),
  [mvpfactory](https://dev.to/software_mvp-factory/postgresql-row-level-security-for-multi-tenant-saas-1lgp),
  [ClickHouse](https://clickhouse.com/resources/engineering/multi-tenant-saas-postgres-architecture)).
- RLS is the **safety net**; the application also filters by tenant explicitly (defense in depth).
- `tenant_id` leads all composite indexes.
- Noisy-neighbour controls: per-tenant rate limits, job quotas, row/attachment quotas.
- Region pinning per tenant (EU, US, JP, KR, IN deployments) — required for residency (Part D).
- Self-hosted single-tenant mode is the same code with one tenant.

### B8. Business-logic accuracy

- **Exact decimals** (`rust_decimal`) for Number/Currency/Percent; floats never used for money.
  Define rounding mode per field (banker's vs half-up; JPY/KRW 0 decimals).
- **Time**: store instants in UTC + field-level display zone; `jiff` on server for TZ math,
  DST edge cases tested (US/DE DST transitions; JP/KR/IN have no DST — tests per zone).
- **Formula engine**: pure, deterministic, bounded (step limit, recursion limit, max output
  size), no I/O; errors are values. Property-tested and fuzzed.
- **Validation in one place**: the same core crate validates in the browser (instant feedback)
  and on the server (authority). Server never trusts the client.
- **Invariants checked continuously**: e.g. two-way link symmetry, rollup = aggregate of links,
  lifecycle state ∈ allowed set — invariant checker runs in tests and as a background audit.

### B9. Security baseline

- Threat model (STRIDE) per component; OWASP ASVS L2 as the checklist.
- AuthN: passkeys (WebAuthn) + OIDC/SAML SSO + password (argon2id) + MFA; sessions in
  `HttpOnly; Secure; SameSite=Lax` cookies; CSRF tokens for state-changing requests.
- AuthZ: one permission engine (role → table → row rule → field rule) evaluated server-side for
  UI, API, export, search, notifications, webhooks, AI.
- Browser: strict CSP (`script-src 'self' 'wasm-unsafe-eval'`), Trusted Types, no `innerHTML`
  of user content; markdown sanitised server-side (`ammonia`).
- Outbound requests (webhooks, HTTP node): SSRF protection (deny private ranges, DNS rebinding
  checks), timeouts, size limits, egress allowlist option.
- Formulas & scripts: pure evaluator; scripting (later) only in a sandbox (WASM component, fuel limits).
- Supply chain: `#![forbid(unsafe_code)]` in our crates; `cargo-deny` (licenses, advisories,
  bans), `cargo-audit`, `cargo-vet` for new deps; SBOM (CycloneDX); signed releases & provenance.
- Data: TLS 1.2+; encryption at rest; per-tenant attachment keys (P1); secrets in a KMS.
- Abuse: rate limits per user/token/IP; public form CAPTCHA option; upload scanning hook.

### B10. Trace & trace-audit (IM-style delivery tracking)

Three layers, linked by IDs:

1. **Technical tracing** — Rust `tracing` + OpenTelemetry on the server. The browser creates a
   W3C `traceparent` per user action ([W3C Trace Context](https://www.w3.org/TR/2020/REC-trace-context-1-20200206/))
   and attaches it to HTTP and WebSocket messages; client spans are batched to our server (not
   directly to an OTLP collector — CORS/credential issues noted for browser clients
   ([Blazor/OTel note](https://oneuptime.com/blog/post/2026-02-06-opentelemetry-tracing-blazor-webassembly/markdown))).
   Rust OTel in browser WASM is not well documented → we keep the client tracer tiny and custom.
2. **Delivery tracking (like IM message states)** — every client op has `client_op_id`
   (UUIDv7) and passes through states:
   `created → sent → received → validated → committed(base_seq) | rejected(reason)` and
   `broadcast → applied_by_client(n)`. The client outbox keeps an op until it sees
   `committed/rejected`; idempotency by `client_op_id`; clients detect gaps in `base_seq` and
   resync. Nothing is "lost": every op ends in a terminal state.
3. **Business process trace** — causal chain: `event (record changed) → workflow run → steps →
   resulting ops → notifications → acknowledgements`. Each **process instance** (record
   lifecycle or workflow run) has an expected contract (terminal states, SLA). A **completeness
   verifier** finds instances that are stuck, missing steps, orphaned, or late → Exceptions inbox.

Audit log: append-only, per-tenant **hash chain** (each entry includes the hash of the previous
entry) with periodic signed checkpoints → tamper-evident; exportable as evidence (SOX/J-SOX
audits, GDPR accountability).

### B11. Testing to 98% coverage — feasibility

- Line/region coverage via **`cargo-llvm-cov`** (native targets, all OSes).
- WASM coverage: `wasm-bindgen-test` + `cargo llvm-cov` is documented but **experimental,
  nightly-only** ([wasm-bindgen coverage guide](https://wasm-bindgen.github.io/wasm-bindgen/wasm-bindgen-test/coverage.html));
  `wasmcov` is an alternative.
  → Architecture rule: **all logic lives in target-independent crates tested natively**; the
  `web-sys` glue layer is thin and tested with wasm-bindgen-test + Playwright; we attempt
  nightly WASM coverage and merge reports, with fallback to E2E-verified glue.
- 98% is enforced **per crate** with `--fail-under-lines 98` in CI, with a reviewed exclusion
  list (generated code, `main` bootstrap) kept under 1% of lines.
- Coverage ≠ correctness, so we add: **property tests** (`proptest`), **fuzzing**
  (`cargo-fuzz`: formula parser, wire decoder, filter parser, import parsers), **mutation testing**
  (`cargo-mutants`, weekly, score threshold), **concurrency model checking** (`loom` for the
  outbox/actor internals), **deterministic simulation** of many clients + network faults
  (`turmoil`-style seeded scheduler), DB integration tests on real Postgres.

### B12. CI on GitHub Actions — feasibility notes

- OS matrix: `ubuntu-latest`, `windows-latest`, `macos-latest` (Apple silicon).
- Browsers: Chrome & Firefox headless (Linux/Windows), Safari via `safaridriver` (macOS);
  Playwright E2E on Chromium/Firefox/WebKit.
- Postgres on all OSes: Linux service container; Windows/macOS cannot run Linux service
  containers → install Postgres natively in the job (setup action or package manager) **[verify]**.
- Canvas screenshot tests need **per-OS/per-browser baselines** (font rasterisation differs).
- Local gate before push: `cargo xtask ci` (fmt, clippy `-D warnings`, tests, coverage, wasm
  size budget, cargo-deny) run by a git **pre-push hook**.

---

## Part D — Compliance research by market (product implications)

| Market | Law / expectation | Product & ops implications |
|---|---|---|
| US | State privacy laws (CCPA/CPRA etc.); enterprise buyers expect **SOC 2 Type II**; ADA/Section 508 accessibility | DSAR tooling (export/delete a person's data), audit logs, SSO/SCIM, accessibility of the grid |
| Germany / EU | **GDPR** + BDSG; DPA (AVV) with customers; strong preference for **EU data residency**; EU–US transfers need a mechanism; **European Accessibility Act** (since June 2025); works-council sensitivity to employee monitoring; BSI C5 for public sector | EU region; data minimisation; configurable activity auditing; deletion/retention; subprocessor list |
| Japan | **APPI** (third-country transfer rules); ISMAP for government cloud; J-SOX for listed companies | Approval evidence & audit trail; JP region option; JPY 0-decimal currency |
| South Korea | **PIPA** (amended 2023, in force 2024): overseas transfer bases incl. disclosure for outsourcing/storage; **domestic representative** for qualifying foreign operators (effective date reported as 2024 or Oct 2025 — [verify]); PIPC guidance for foreign operators ([Baker McKenzie](https://connectontech.bakermckenzie.com/south-korea-issues-guidelines-on-applying-the-personal-information-protection-act-to-foreign-business-operators/), [Ius Laboris](https://iuslaboris.com/insights/south-korea-passes-major-overhaul-of-data-protection-law/)); ISMS-P certification common | Privacy policy disclosures; KR region option; never collect resident registration numbers (PII detector blocks it) |
| India | **DPDP Act 2023 + DPDP Rules 2025** (notified Nov 2025, phased: main obligations by ~13 May 2027) ([Legal500](https://www.legal500.com/firms/237811-saga-legal/c-india/news-and-developments/the-privacy-timer-starts-now-analysing-the-dpdp-rules-and-its-18-month-countdown), [Hunton](https://www.hunton.com/privacy-and-cybersecurity-law-blog/india-enacts-data-protection-rules-introducing-new-privacy-regime)); cross-border allowed except to blacklisted countries | Consent notices on public forms; breach-notification support from audit system; erasure workflows |

Cross-market takeaways: **self-hosting (open source) is a major compliance advantage**;
for our hosted cloud we need region pinning, DPA templates, DSAR tools, PII classification,
retention, tamper-evident audit — all of which are also *data-asset features*, not overhead.
(This is product research, not legal advice; counsel must review before launch.)

---

## Part E — Open-source landscape & positioning

| Project | License (per sources) | Stack | Note |
|---|---|---|---|
| Teable | AGPL-3.0 | TS/NestJS + Postgres | Closest modern OSS competitor |
| NocoDB | Source-available (Sustainable Use License) since Jan 2026 | TS | No longer OSI open source |
| Baserow | MIT core | Python/Django | Plugin system |
| Grist | Apache-2.0 | Python/TS | Python formulas |
| APITable | AGPL | TS/Java | Maintenance reportedly slowed |

Sources: [NocoBase overview](https://www.nocobase.com/en/blog/open-source-airtable-alternatives),
[Teable comparison](https://teable.ai/blog/best-nocodb-alternatives), [Toolradar](https://toolradar.com/compare/nocodb-vs-teable),
[openalternative](https://openalternative.co/alternatives/apitable).

Our differentiation (to validate):
1. **Closed-loop processes built in** (lifecycles, assignments with ack, SLAs, exceptions inbox).
2. **Provable processes**: trace-audit with tamper-evident log — audit-ready (SOX/J-SOX/GDPR).
3. **Data-asset governance** (ownership, classification, lineage, quality) out of the box.
4. **One Rust core** → identical logic in browser/server/desktop; small, fast, single binary self-host.

License decision needed: **Apache-2.0** (max adoption, patent grant, enterprise-friendly) vs
**AGPL-3.0** (prevents closed SaaS forks) vs split (Apache core crates + AGPL server).

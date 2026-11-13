# Lark Capability Parity & Delivery Plan (v1.0)

> Companion to doc 17 (product design) and doc 18 (AI design). Lark/Feishu Base capabilities are taken from
> the reviewed help-centre archive and earlier research (docs 05, 15, 16).
> **Status legend:** **=** parity · **+** we go further · **≠** deliberately different · **L** later release · **–** not planned.

---

## 1. Parity matrix by category

### 1.1 Data model, records, views
| Lark capability | Status | Our design / note | Release |
|---|---|---|---|
| Bases, tables, folders, doc pages, spaces | = | Workspaces, bases, folders, doc pages (K17) | R1 |
| ~30 field types (regular / business / advanced) | + | ~35 types in 8 groups, incl. reference, line items, lifecycle state, tax IDs, AI skill fields | R1 |
| Field groups (grid) | + | One definition for grid header, record page and forms; group permissions | R1 |
| Option linkage via referenced table | ≠ | Governed reference sets with trees and cascades | R1 |
| One-/two-way links, lookup | = | + rollups, line items | R1 |
| Process field | ≠ | Lifecycle with state categories, guards, required fields, capabilities | R1 |
| Button field | = | Button = rule trigger | R1 |
| Auto number | + | Sequences with patterns, per entity/year, gap-free option | R1 |
| Phone, email, location, barcode, progress, currency, rating, signature | = | Plus market validators | R1 / location+barcode R2 |
| Grid/kanban/calendar/Gantt/gallery/form views | = | Plus list view; form moves to Apps & pages | R1 (Gantt R2) |
| Protected / independent shared views | = | Locked & shared views | R1 |
| Custom record detail page | = | Record page layouts (tabs, sections, related lists) | R1 |
| Follow / share / summarise single record | = | Record subscription, share link, AI summary | R1 |
| Time Machine (history & restore) | = | Journal-based history and restore points; retention by setting | R1 |
| Archive table (1M read-only) | = | Archive tier, reportable | R2 |
| 2M rows per table | ≠ | 1M active rows (v1 target, spike S3) + archive tier; raise after measurement | R1 |
| 300 fields, 200 views, 3 group levels, 50 filters | + | 400 fields, 200 views, 5 group levels, 100 filters | R1 |
| Field type change ≤ 20,000 rows, serialised on > 100k rows | + | Async type-change jobs with no row ceiling | R1 |
| Cell fill colour, row height, time zone, shortcuts | = | | R1 |
| Work activity feed | = | Home activity feed | R1 |

### 1.2 Formulas
| Lark | Status | Note | R |
|---|---|---|---|
| Function library (IF/IFS, FILTER, SUMIF, COUNTIF, DATEDIF, TEXT, UNIQUE, LIST, MAP, LET, NETWORKDAYS…) | = | Same breadth; exact decimals; business calendars from reference data | R1 (LET/MAP/LIST R2) |
| CurrentValue in statistics | = | Row-context functions over linked records | R1 |
| AI generate formula | = | Skill A3 with explanation | R1 |
| Optimisation guide, overrun errors | + | Fan-out warnings at design time, compute-on-read, capacity estimator | R1 |

### 1.3 Automation & workflow
| Lark | Status | Note | R |
|---|---|---|---|
| Automation (simple) **and** workflow (canvas) | ≠ | One engine; simple editor for one-step flows | R1 |
| Triggers: new record, modified, meets condition, date reached, schedule, button, webhook, chat message | + | Same plus comment, metric threshold, inactivity, source failure; all as rules | R1 |
| Branches, multi-branch (2–10), loops | = | Plus waits, sub-flows, error paths | R1 (loops R2) |
| Actions: records, messages, email, HTTP, calendar/tasks/groups | = | Records, notifications (multi-channel), email, HTTP actions, document output; calendar via destinations | R1 |
| Send dashboard snapshots | = | Scheduled report delivery | R1 |
| Approval sync from Lark Approval | + | Native approval designer (doc 11 §4) | R1 |
| Run quota shared between automation and workflow | ≠ | One quota per tenant; run logs per rule | R1 |
| Sandbox | = | Change sets (staging) for structure, rules, workflows and access | R1 |

### 1.4 Dashboards
| Lark | Status | Note | R |
|---|---|---|---|
| KPI card, bar/line/area/combo, pie, funnel, word cloud, radar, NPS, countdown, progress, ranking, text, button, view component | = | Full component list in doc 17 C9 | R1 core / R2 rest |
| Pivot, slicers, Top N, row/column switch, cross-filtering, multiple sources | = | | R1 Top N & slicers; R2 pivot |
| Data follows viewer permission | + | Default behaviour (permission-scoped aggregates) | R1 |
| Subscriptions, themes | = | | R1 |
| Certified metrics, period close, per-recipient scheduled reports | + | Governance not in Lark | R1/R2 |

### 1.5 Access
| Lark | Status | Note | R |
|---|---|---|---|
| Advanced permissions (roles; table/record/field/view/dashboard rights) | + | Capabilities with scopes, limits, validity, SoD, guests | R1 |
| Preview permission results | = | "Preview as user" + explain | R1 |
| Space roles, app roles, agent sharing, bot permissions (stacked layers) | ≠ | One capability model across all objects | R1 |
| AI configure permissions (with alternatives) | = | Through change sets; alternatives in R2 | R1/R2 |

### 1.6 Apps, pages, forms
| Lark | Status | Note | R |
|---|---|---|---|
| App mode (multi-base, navigation, components, roles) | = | App shell with one layout system | R2 |
| Forms: prefill, detail table, Lark-doc embed | + | Line items, multi-table submit, record pickers, document/paste/voice capture | R1 (voice R2) |
| Query page | = | Portal/lookup pages | R1 |
| AI custom pages | ≠ | Generated layouts from our page system; marketing pages out of v1 scope | R2 |

### 1.7 Integration
| Lark | Status | Note | R |
|---|---|---|---|
| OpenAPI (rate-limited per endpoint), events (per-file subscription; formulas don't fire) | + | REST + change feed + signed events incl. computed fields; inbox push in client's own format | R1 |
| Connectors, plugins | = | Connector catalogue (database, REST, files); plugin SDK | R1 / plugins L |
| Cross-base sync, publish as shared data source | = | Base-to-base inbound sources | R2 |
| Sync from Lark Sheets/approvals/tasks | ≠ | Generic sources; Lark/Airtable importer | R2 |
| HTTP request action (public URLs only) | + | Intranet destinations allowed via allowlist in self-host | R1 |
| MCP server (beta), CLI | = | Built-in MCP server and CLI | R1 |

### 1.8 AI
| Lark | Status | Note | R |
|---|---|---|---|
| AI sidebar (modes) | = | Assistant with intents: build, configure, ask, find, explain | R1 |
| AI build system (standard / expert mode), app, table | = | Build pipeline with change sets and verification; expert-mode design document | R1 / expert R2 |
| AI configure permissions (1–3 options) | = | | R1 / options R2 |
| AI data Q&A & analysis (retrieval, stats, attribution, forecast, reports) | ≠ | Bounded planner: deterministic numbers, statistical forecasts only, visible plan | R1 / drivers & forecast R2 |
| Smart toolbar (fill, polish, translate, summarise, extract, tags, ask) | = | Cell toolbar using skills | R1 |
| AI field shortcuts (classify, translate, tags, extract, summary, custom; multimodal; image/video gen; OCR; PDF) | = / L | Skills T1–T9, D1–D3 in R1; image understanding, speech R2; image generation R3; video – | R1–R3 |
| AI paste entry, voice entry | = | Capture assistant | R1 / voice R2 |
| Agents (instructions, schedules, triggers, tools, knowledge, memory, entry points) | + | Agents as principals with grants, budgets, action policies; triggers via rules | R1 (MCP, memory R2) |
| Agent in field / workflow node / dashboard / app | ≠ | Same entry points; explicit initiator instead of last modifier / creator | R1–R2 |
| Workflow AI Agent node (separate concept) | ≠ | Merged into Agents (one concept) | — |
| AI classify node, generate text node, node shortcuts | = | Workflow AI steps calling skills | R1 |
| Dashboard AI (chart analysis, summary, build) | = | | R2 (build R1) |
| AI quota (points per feature, per cell, regeneration charged) | ≠ | Credits from metered usage, five budget levels, cost preview, cache avoids re-charge | R1 |
| Model choice (Auto + listed models) | = | Gateway routing with Auto, BYO keys, self-hosted models | R1 |

### 1.9 Not planned (or out of scope v1)
- Video generation, image style transfer.
- Lark-specific office integrations (Lark Docs embedding, Lark calendar/tasks as native objects). These are covered generically by destinations, with chat via adapters.
- Business mini-programs.

---

## 2. Release plan

| Release | Product scope | Templates |
|---|---|---|
| **R1** | Core C1–C10, C12–C14 per doc 17 at R1 level; AI R1 per doc 18 §12; Docker self-host; SQLite/MySQL/PostgreSQL | **HR & Admin** (S2 employee lifecycle & requests, S3 expenses, S4 assets) · **E-commerce after-sales** (S6) · S7 Customer service desk (shares S6 components) |
| **R2** | App mode, archive tier, pivot, Gantt, voice capture, expert-mode builder, MCP for agents, drivers/forecast, base-to-base sources, importer from Lark/Airtable, more chat channels | **New Media** (S8) · **Recruitment** (S1) · **Procurement** (S5) |
| **R3** | Agent templates gallery, image generation (optional), SLA risk prediction, plugin SDK, cloud service readiness | **Marketing** (S9) · **R&D** (S10) · **Projects** (S11) |

**Release gate (every release):**
- Template-only builds pass (no template code).
- Every test runs on 3 databases × 3 OS × browsers.
- Coverage gates are met.
- AI ship gates are met.
- Docker install-to-first-record takes < 5 min.

---

## 3. Engineering hand-off

### 3.1 Spikes before R1 implementation (from docs 03, 04, 06)
| Spike | Goal |
|---|---|
| S1 Grid | Canvas vs Leptos-DOM decision at 1M rows (fps, input latency, IME, accessibility) |
| S2 Size | WASM ≤ 600 KB brotli |
| S3 Storage | Physical tables on 3 databases at 1M rows; type-change jobs; Postgres pool per tenant |
| S4 Realtime | Base actor, 100 clients, permission-filtered broadcast |
| S5 CI | 3 OS × 3 DB × browsers; nightly branch coverage; Docker 5-minute E2E |
| S6 Trace | Outbox, journal, completeness verifier |
| S7 AI retrieval | Pure-Rust vector index + local embeddings memory/latency |
| S8 Planner | 300-question benchmark |
| S9 Capacity | Calibrate the capacity model |
| S10 Approvals | Model-based state exploration |
| S11 Integration | Time-to-first-sync scenarios |

### 3.2 Domain → crate mapping (initial)
| Domain (doc 17 §2) | Crate(s) |
|---|---|
| D3 Schema, D4 Records (core types, validation, provenance) | `mt-core`, `mt-ops` |
| D5 Query & Views | `mt-query` |
| D6 Computation | `mt-formula` |
| D7 Process | `mt-lifecycle` (lifecycle, SLA, approvals state machine) |
| D8 Automation | `mt-rules`, `mt-workflow` |
| D9 Messaging | `mt-notify` |
| D10 Integration | `mt-integrate` (sources, mapping, xref, destinations, jobs), `mt-mcp` |
| D11 Analytics | `mt-metrics` |
| D12 Access | `mt-perm` |
| D13 AI | `mt-ai` (gateway, skills, agents runtime, planner), `mt-ai-eval` |
| D14 Templates | `mt-templates` |
| D15 Trust | `mt-trace`, `mt-audit` |
| D1–D2 Identity, Workspace, server | `mt-server`, `mt-store` (3 dialects, database per tenant) |
| Client | `mt-grid` (renderer), `mt-web` (Leptos app) |
| Tooling | `xtask` (local CI gate, size budget, coverage, template validation) |

### 3.3 Definition of done (every feature)
1. Spec section in docs 17/18 referenced in the PR.
2. Works through the operation pipeline with provenance; capability-checked.
3. API + MCP + rule action equivalents exist where applicable (doc 16 §4).
4. Tests: standard, anomalous, edge, invalid, concurrency, cross-platform; on all three databases.
5. Coverage gates met; exhaustive-match lints clean.
6. Documentation page in the taxonomy (doc 16) with limits from the limits registry.
7. Used by at least one template, or explicitly marked as platform-only.

### 3.4 Open decisions before coding
1. Product name.
2. GitHub organisation/repository for code and docs.
3. First chat channel adapter for R1 (Slack, Teams or Lark).
4. First document-AI provider for R1.
5. Default AI provider for the R1 demo (Claude or OpenAI-compatible/local).

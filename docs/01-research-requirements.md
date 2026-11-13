# Multidimensional Table App (Rust) — Research & Requirements Baseline

> Status: research draft v0.1 · Purpose: define the full business-process and feature scope
> (core, OpenAPI, AI) before architecture and MVP planning.
> Reference products: Lark/Feishu Base (飞书多维表格), Airtable, Teable, NocoDB, Baserow, APITable, Grist.
> Numbers/limits of reference products should be re-verified against their current docs.

---

## 0. Product definition

A **multidimensional table** is a collaborative, typed, relational data store with a
spreadsheet-like UI, multiple views over the same data, fine-grained permissions,
no-code automation, an open API, and AI built into fields and conversations.

Positioning for our build:

- **System of record for business teams** (operational data + processes), not a data warehouse.
- **Cross-platform / multi-device**: Web, Windows, macOS, Linux, iOS, Android — same data,
  real-time sync, offline-capable on native clients.
- **Data-asset first**: every record has an ID, schema, owner, permissions, history and lineage.

---

## 1. Domain model (the "nouns")

```
Organization (tenant)
 ├─ Member / Department / Group / Role
 ├─ Workspace (space)
 │   └─ Base (app)
 │       ├─ Table
 │       │   ├─ Field (typed column, has field_id, type, property, description)
 │       │   ├─ Record (record_id, cells, created/modified meta, comments, history)
 │       │   └─ View (grid/kanban/gallery/gantt/calendar/form/hierarchy/timeline…)
 │       ├─ Dashboard (blocks/charts)
 │       ├─ Workflow (automation: trigger → nodes)
 │       ├─ Permission config (roles, table/row/field rules)
 │       ├─ Extensions/plugins, Scripts
 │       └─ AI assets (AI fields config, agents, knowledge)
 ├─ Templates
 ├─ API apps / tokens / webhooks
 └─ Audit log, quotas, billing
```

Stable identifiers everywhere (`baseId`, `tblId`, `fldId`, `recId`, `viwId`, `wfId`) — the API
and formulas must reference **IDs**, not names, so renames never break anything.

---

## 2. End-to-end business processes (the "verbs")

### 2.1 Tenant & identity
1. Org sign-up → verify domain → invite members (email/link/SSO auto-provision via SCIM).
2. Org structure: departments, user groups, admins; sync from an external directory (LDAP/OIDC/SCIM).
3. Login: password, OAuth2/OIDC (Google, Microsoft, Lark/Feishu, DingTalk, WeCom), SAML SSO, MFA.
4. Device management: session list per device, remote sign-out.
5. Admin console: member lifecycle, offboarding with ownership transfer, quotas, security policy
   (external sharing on/off, download/copy/print restrictions, watermark, IP allowlist), audit logs.

### 2.2 Build a Base (modeling)
1. Create base: blank / from template / import (CSV, XLSX, Lark Sheets, Airtable export) / AI-generate
   ("build me a CRM") → AI proposes tables, fields, relations, views, sample data.
2. Define tables and fields; set field descriptions (data dictionary), defaults, required, validation.
3. Define relations (link fields, one-way/two-way), lookups, rollups, formulas.
4. Create views per role/scenario; set filter/sort/group/hidden fields/row height/colour rules.
5. Configure permissions (roles, table/row/field level).
6. Configure forms for data collection; publish internal/public.
7. Configure automations/workflows; dashboards; publish as template.

### 2.3 Data entry & collection
- Inline grid editing, expanded record (detail) page, bulk paste from clipboard, fill-handle drag.
- Forms (public link, login required, one submission per user, prefill via URL, conditional
  questions, form-only fields, submit-to-notify).
- Mobile capture: camera/photo attachments, barcode/QR scan into field, GPS location field.
- Import (append / upsert by key field / create new table), field-type auto-detection.
- External ingestion: API, webhooks in, data sync connectors (DB, other bases, SaaS).

### 2.4 Collaboration
- Real-time co-editing with presence (cursors, who's viewing which record).
- Record comments, @mentions (people, records), threads, resolve; reactions.
- Notifications: in-app, push (mobile/desktop), email, IM bots (Lark/Slack/Teams/DingTalk/WeCom).
- Record subscription ("watch this record"), assignee change alerts, due date reminders.
- Sharing: base/table/view/form/dashboard links with scopes (view/comment/edit), external sharing.

### 2.5 Process execution (typical flows built on the product)
- Status-driven pipelines (Kanban): lead → qualified → won/lost; ticket open → in progress → closed.
- Approvals: submit record → approver(s) → approve/reject → write back status.
- Scheduling: due dates, reminders, recurring tasks, SLA timers.
- Cross-table flows: order created → create invoice record → notify finance.
- Buttons on records ("Send quote", "Generate contract") that trigger workflows.

### 2.6 Analysis & reporting
- View-level aggregates (sum/avg/count/min/max/filled/empty/unique/% per column footer).
- Group-by with subtotals; pivot view.
- Dashboards: metric cards, bar/line/pie/scatter/funnel/heatmap, tables, filters, drill-down,
  permission-aware (viewer sees only data they're allowed to).
- Export CSV/XLSX/PDF, scheduled report push.
- AI Q&A / natural-language analysis.

### 2.7 Governance & lifecycle
- Record history (field-level diffs), base snapshots / version restore, recycle bin (records,
  fields, tables, bases), retention policies.
- Audit log (who read/exported/changed what), data classification of fields (PII, sensitive).
- Templates, base duplication, cross-base sync for shared master data.
- Archival, large-table tiering.

---

## 3. Feature catalog (with priority)

Priority: **P0** = MVP, **P1** = v1 release, **P2** = later / differentiators.

### 3.1 Field types

| Category | Field types | Prio |
|---|---|---|
| Basic | Single-line text, Long text (rich text/markdown), Number (precision, thousands sep), Currency, Percent, Checkbox, Date/DateTime (timezone, format), Single select, Multi select (colored options) | P0 |
| Contact | Email, Phone, URL | P0 |
| People | User (single/multi, notify on assign), Group/Department | P0/P1 |
| Files | Attachment (images/docs/video, preview, thumbnails) | P0 |
| Relational | Link to record (one-way/two-way, single/multiple, cross-table, cross-base P2), Lookup, Rollup (sum/count/concat… over linked) | P0 |
| Computed | Formula (spreadsheet-like function library), Auto number (prefix/date pattern), Created time/by, Modified time/by (optionally scoped to specific fields) | P0 |
| UX | Rating, Progress, Duration, Button (triggers workflow / opens URL) | P1 |
| Special | Location (GPS + map), Barcode/QR, Signature, Hierarchy parent (self-link) | P1/P2 |
| AI | AI field ("field shortcut"): summarize, extract, classify, translate, generate, tag, image/OCR understanding — configured prompt referencing other fields | P1 |

Cross-cutting field features: description, default value, required, unique (constraint — an
improvement over Lark), validation rules (regex/range), field-level permissions, field type
conversion with preview & lossless rollback, primary (display) field per table.

### 3.2 Views

| View | Key features | Prio |
|---|---|---|
| Grid | Virtualized 1M-row scrolling, freeze columns, resize, row height, colour rules, group (multi-level), sort (multi), filter (AND/OR nested groups), hide fields, column footers, search & highlight, fill handle, copy/paste | P0 |
| Kanban | Group by single-select/user, drag between columns, card cover/fields, swimlanes (P2) | P0 |
| Form | See 2.3 | P0 |
| Calendar | By date field(s), drag to reschedule, month/week/day | P1 |
| Gallery | Card layout with cover image | P1 |
| Gantt / Timeline | Start/end fields, dependencies, drag resize, grouping, milestones | P1 |
| Hierarchy / Tree | Parent-child records | P2 |
| Pivot | Rows × columns × aggregation | P2 |
| Map | Location field | P2 |

View settings: personal vs shared views, locked views, view-level permissions, share a view
link, per-view "filter by current user" (`Me`), saved view snapshots.

### 3.3 Formula engine
- Parser + type checker + evaluator; references by field ID; functions: math, text, logical,
  date/time (timezone-aware, WORKDAY, DATEDIF), array/list ops over linked records, regex,
  lookup-ish (FILTER over linked records), conversion, `RECORD_ID()`, `NOW()/TODAY()` (scheduled
  recompute), `CURRENT_USER()` (view-time only).
- Dependency DAG across fields/tables (formula → lookup → link → rollup); cycle detection;
  incremental recompute on change; batch recompute on schema change; error values (`#ERROR`,
  `#REF`) instead of failures.
- **Must be one shared implementation** usable on server, desktop, mobile, and web (WASM) to
  guarantee identical results everywhere — a strong reason to write it in Rust.

### 3.4 Filtering, sorting, search
- Typed operators per field type (is/isNot/contains/isEmpty/isAnyOf/isWithin(date range)/</>…).
- Nested condition groups (AND/OR), relative dates (today, past N days, this week).
- Full-text search across base; per-table quick search; search by record ID.
- Same filter AST used by: views, API `search`, permission row rules, workflow conditions,
  dashboard filters, AI query tools → **one filter language, many consumers**.

### 3.5 Permissions (advanced)
- Base roles: Owner, Admin(Manager), Editor, Commenter, Viewer, + **custom roles**.
- Per custom role: table access (none/read/edit/add-only), **row rules** (filter AST, incl.
  `owner = Me`, `department = MyDept`), **field rules** (hidden/read/edit per field),
  view/dashboard visibility, action permissions (export, copy, print, share, manage workflows).
- Record-level actions: can add, can delete, can edit only own records.
- Permission enforcement point: **server-side, in the query layer**, applied identically to UI,
  API, export, dashboards, search, notifications, and AI (AI must never leak hidden rows/fields).
- Sharing policy inheritance from org security settings.

### 3.6 Automation / Workflow
Triggers:
- Record created / updated (specific fields, matching condition) / deleted; record matches condition.
- Scheduled (cron, relative to a date field: "3 days before due date").
- Button clicked; form submitted.
- Webhook received; IM message received (bot); approval result.

Nodes (actions & logic):
- Records: create, update, find, delete, upsert; batch.
- Messaging: notify user/group, send IM card, email, push notification.
- Integration: HTTP request (auth, retries), call another workflow, run script (sandboxed).
- Logic: IF/ELSE branches, loop over list, wait/delay, variables, error handling path.
- AI: AI generate/extract/classify step; **AI Agent node** (model + memory + tools incl. MCP).
- Approval: start approval, wait for result.

Runtime: durable execution (survive restarts), idempotency keys, retry with backoff,
per-run log with step inputs/outputs, run history, rate/quota limits, version & draft/publish,
loop/recursion protection (workflow updating the record that triggered it).

### 3.7 Dashboards
Blocks: metric/KPI, bar/line/area/pie/donut/scatter/funnel/heatmap, combo, table, text, filter
controls; data source = table + filter + group + aggregation; cross-table via links;
permission-aware rendering; auto-refresh; full-screen/TV mode; share link; AI "explain this chart".

### 3.8 Data ingress / egress
- Import: CSV, XLSX, JSON, Lark Sheets, Airtable/Notion export; mapping & type detection; upsert.
- Export: CSV/XLSX (respect permissions & export policy), PDF for record/dashboard.
- Sync connectors (P1/P2): Postgres/MySQL (read-only mirror), other bases (master data sync),
  Google Sheets, REST pull with schedule. Synced tables are read-only with source tracking.
- Attachments: object storage (S3 compatible), image thumbnails, virus scan hook, quotas.

### 3.9 History, recovery, audit
- Cell-level change log (old → new, actor, time, source: UI/API/workflow/AI).
- Base snapshots (auto daily + manual), restore whole base or a table.
- Recycle bin with retention (e.g. 30 days).
- Org audit log: login, permission changes, export, share, API token use.

### 3.10 Extensibility
- Front-end extensions (sidebar/view plugins) via sandboxed iframe/webview + JS SDK.
- Scripting (P2): sandboxed JS (e.g. via QuickJS/Deno) or WASM components.
- Custom automation triggers/actions provided by extensions.
- Template marketplace.

### 3.11 Multi-device / cross-platform specifics
- Platforms: Web (desktop & mobile browsers), Windows/macOS/Linux desktop, iOS, Android.
- Responsive mobile UI: record list/cards instead of wide grid, quick-add, form-first entry.
- Offline (native P1): local replica of permitted subset, queue edits, conflict resolution
  (CRDT/LWW per cell), sync status UI.
- Push notifications (APNs/FCM), deep links to records, share-sheet import, camera/scanner.
- Real-time sync latency target < 300 ms on LAN/cloud.

---

## 4. OpenAPI (developer platform)

### 4.1 Principles
- REST + JSON, versioned (`/api/v1`), **OpenAPI 3.1 spec generated from code** (Rust: `utoipa`),
  published SDKs (TS, Python, Go, Java, Rust) generated from the spec.
- IDs in paths; field references accept ID or name (`fieldKeyType=id|name`).
- Consistent pagination (`page_token` + `page_size`, max 500), batch endpoints (≤ 500 records/req,
  similar to Lark), partial success reporting, idempotency key header.
- Typed cell value formats per field type documented; `typecast` option to coerce strings.
- Errors: stable codes + message + request_id.
- Rate limits per app/tenant/base (e.g. QPS + burst), `429` with `Retry-After`.
- Permissions: API calls run as a **user** (user access token) or as an **app/bot** (tenant
  token with granted scopes and base-level grants). Same permission engine as the UI.

### 4.2 Authentication
- OAuth 2.0 authorization code + PKCE (user token), client credentials (app/tenant token),
  personal access tokens with scopes and expiry, base-scoped tokens.
- Scopes example: `base:read`, `base:write`, `record:read`, `record:write`, `field:manage`,
  `view:manage`, `workflow:manage`, `webhook:manage`, `ai:use`, `admin:*`.

### 4.3 Resource endpoints (draft)

| Resource | Endpoints |
|---|---|
| Workspaces | `GET/POST /workspaces`, `GET/PATCH/DELETE /workspaces/{id}` |
| Bases | `POST /bases` (blank/template/copy), `GET/PATCH/DELETE /bases/{baseId}`, `POST /bases/{baseId}/copy`, `GET /bases/{baseId}/schema` (full schema dump) |
| Tables | `GET/POST /bases/{b}/tables`, `POST .../tables:batchCreate`, `PATCH/DELETE /tables/{t}` |
| Fields | `GET/POST /tables/{t}/fields`, `PATCH/DELETE /fields/{f}` (type conversion with options) |
| Records | `GET /tables/{t}/records` (list with view/filter/sort/fields), `POST /tables/{t}/records:search` (filter AST body), `GET /records/{r}`, `POST /records` , `POST /records:batchCreate`, `PATCH /records:batchUpdate`, `POST /records:batchUpsert` (by key fields), `POST /records:batchDelete`, `POST /records:batchGet` |
| Views | `GET/POST /tables/{t}/views`, `PATCH/DELETE /views/{v}` (filter/sort/group/hidden fields config) |
| Comments | `GET/POST /records/{r}/comments`, `PATCH/DELETE /comments/{c}` |
| History | `GET /records/{r}/history`, `GET /bases/{b}/snapshots`, `POST /snapshots/{s}:restore` |
| Attachments | `POST /attachments:upload` (multipart / resumable), `GET /attachments/{a}` (signed URL) |
| Permissions | `GET/POST /bases/{b}/roles`, `PATCH/DELETE /roles/{role}`, `POST /roles/{role}/members`, collaborators CRUD |
| Forms | `GET /views/{formView}/form`, `POST /forms/{id}:submit` (public) |
| Dashboards | `GET /bases/{b}/dashboards`, `GET /dashboards/{d}/blocks/{blk}/data` |
| Workflows | `GET/POST /bases/{b}/workflows`, `PATCH/DELETE`, `POST /workflows/{w}:enable|disable|run`, `GET /workflows/{w}/runs` |
| Webhooks | `POST /bases/{b}/webhooks` (filter by table/event/fields), `GET/DELETE`, `POST /webhooks/{id}:refresh` |
| Import/Export | `POST /imports` (async job), `GET /jobs/{id}`, `POST /exports` |
| AI | `POST /ai/fields/{f}:generate` (re-run AI field for records), `POST /bases/{b}/ai/ask` (NL question → answer + citations), `POST /bases/{b}/ai/schema:generate` |
| Admin | users, departments, groups (SCIM 2.0), audit log query, tenant settings |

### 4.4 Events
- Event types: `record.created|updated|deleted`, `field.*`, `table.*`, `view.*`,
  `comment.created`, `base.permission.changed`, `workflow.run.completed`, `form.submitted`.
- Delivery: signed webhooks (HMAC-SHA256, timestamp, retries with backoff, dead-letter, replay),
  plus WebSocket/SSE streaming subscription for integrations; event payload includes changed
  fields with before/after (permission-filtered).
- Ordering: per-record ordering guaranteed; at-least-once delivery + event IDs for dedup.

### 4.5 Other developer surfaces
- **MCP server** (Model Context Protocol) exposing base tools (list schema, search records,
  create/update records, run workflow) so external AI agents (Claude, etc.) can operate the
  product under user permissions. Rust: official `rmcp` SDK.
- CLI (`mt base ...`) built on the Rust SDK — mirrors Lark's direction of `lark-cli base +...`.
- Front-end extension JS SDK (table/field/record/view/selection/UI bridge).
- Sandbox/test tenants, API explorer, request logs per app.

---

## 5. AI capabilities

### 5.1 AI in the data layer
| Capability | Description | Prio |
|---|---|---|
| AI field | Prompt template referencing other fields (`{{fld_xxx}}`), output type (text / single-select / multi-select / number / date / JSON→multiple fields), model choice, auto-run on change or manual, batch run with progress, cost estimation | P1 |
| Prebuilt AI shortcuts | Summarize, translate, extract entities (name/phone/amount), classify/tag, sentiment, OCR & image description from attachments, document (PDF) extraction, web lookup (enrich company info) | P1 |
| Smart fill | Learn from filled examples to fill empty cells | P2 |
| Data cleansing | Detect duplicates, normalize formats, suggest validation rules | P2 |

### 5.2 AI for building
- Generate base/table/field schema from a description or from an uploaded spreadsheet.
- Generate formulas from natural language; explain formula errors.
- Generate filters/views ("show overdue tasks owned by my team").
- Generate workflows from description ("when status becomes Won, notify #sales and create invoice").
- Generate dashboards/charts from a question.

### 5.3 AI for consuming
- **Chat with your base** (NL → query plan over filter AST + aggregations, not free SQL), answers
  with citations to records, chart output.
- Record-level summary ("summarize this customer's history incl. comments and linked records").
- Dashboard insight / anomaly explanation.
- Semantic search over text & attachments (embeddings; pgvector).

### 5.4 AI agents
- Base Agent / Team Agent: configured persona + knowledge (selected tables/views/docs) + tools
  (record CRUD, search, workflow trigger, HTTP, MCP) + memory; deployable as an IM bot, in-app
  chat, or a workflow node.
- Human-in-the-loop: agent proposes record changes → user approves (diff preview).

### 5.5 AI platform requirements (non-negotiable)
- **Permission-aware retrieval**: every AI tool call goes through the same permission engine;
  embeddings indexed with ACL metadata and filtered at query time.
- Model gateway: pluggable providers (Anthropic Claude, OpenAI-compatible, Azure, domestic
  Chinese models, self-hosted via vLLM/Ollama), per-tenant keys, model routing, fallback.
- Quotas & billing per tenant/user, token accounting, rate limits, cost preview for batch AI fields.
- Data residency & privacy: tenant opt-out from AI, PII masking option, no training on customer data.
- Observability: prompt/response logs (redacted), eval sets for NL→query accuracy.
- Determinism controls: AI field outputs stored as normal cell values + provenance metadata
  (model, prompt version, timestamp) so data assets stay auditable.

---

## 6. Non-functional requirements

| Area | Target (initial) |
|---|---|
| Scale | 1M records/table (P1), 100+ fields/table, 10k+ tables per tenant; larger via "data table" tier (P2) |
| Performance | Open view of 100k rows < 1 s (first screen), filter/sort on 1M rows < 2 s server-side, cell edit round-trip < 300 ms |
| Concurrency | 100+ concurrent editors per base with presence |
| Availability | 99.9% SaaS; private deployment option (single binary + Postgres) |
| Security | TLS, encryption at rest, tenant isolation, audit, SSO/MFA, OWASP ASVS L2 |
| Deployment | SaaS multi-tenant + on-prem/private cloud (Docker/K8s), air-gapped option with local LLM |
| i18n | zh-CN, en-US first; timezone- and locale-aware formats |
| Accessibility | Keyboard-first grid, screen-reader labels, WCAG 2.2 AA for forms |
| Compliance | China (PIPL, 等保 2.0) and/or GDPR depending on market — **to decide** |

---

## 7. Implications for a Rust implementation (preliminary — to be decided in architecture phase)

**Core idea: one Rust "core" crate shared by server and every client**, so schema rules, field
type coercion, formula evaluation, filter/sort, permission evaluation, and sync/merge logic
behave identically everywhere.

```
crates/
  mt-core        domain model, field types, cell values, validation
  mt-formula     parser, type checker, evaluator, dependency graph
  mt-query       filter/sort/group/aggregate AST + evaluator (in-memory) + SQL compiler
  mt-perm        permission model & evaluator (row/field rules)
  mt-sync        CRDT / op-log, conflict resolution, offline queue
  mt-server      HTTP API (axum), WebSocket realtime, auth, OpenAPI (utoipa)
  mt-storage     Postgres (sqlx) server-side; SQLite for local/offline clients
  mt-workflow    durable workflow engine, triggers, scheduler
  mt-ai          model gateway, AI fields, NL→query, embeddings, agent runtime
  mt-mcp         MCP server (rmcp)
  mt-sdk / mt-cli  Rust SDK + CLI
apps/
  client         cross-platform UI (see options below)
```

Candidate libraries: `tokio`, `axum`, `sqlx` (Postgres, SQLite), `utoipa` (OpenAPI), `serde`,
`jsonwebtoken`/`openidconnect`, `opendal` (S3/object storage), `tantivy` (full-text search),
`pgvector` (embeddings), `loro` / `yrs` / `automerge` (CRDT), `petgraph` (formula DAG),
`chumsky`/`logos` (formula parser), `apalis` or Postgres-backed queue (jobs), `cron` (scheduler),
`rmcp` (MCP), `tracing` + OpenTelemetry.

Cross-platform client options (key decision):

| Option | Targets | Pros | Cons |
|---|---|---|---|
| **A. Tauri 2 + web UI (TS/React) + Rust core via WASM/native commands** | Win/macOS/Linux/iOS/Android + Web | Mature grid/chart ecosystem (canvas grids), one UI codebase for web & native, Rust core reused | UI layer not Rust |
| B. Dioxus (Rust UI) | Web/desktop/mobile | All-Rust, shared types | Mobile & large-grid components less mature; more custom work |
| C. Slint / egui (Rust native UI) + separate web app | Desktop/mobile/embedded | Very performant native | Web needs a second UI; fewer business-UI components |

Preliminary recommendation: **Option A** — Rust for server + shared core (compiled natively for
Tauri and to WASM for the browser), TypeScript for the view layer. This maximizes reuse and
delivery speed while keeping all business logic in Rust. Confirm before architecture work.

---

## 8. Suggested release scope

- **MVP (P0)**: auth + workspace/base/table/field/record/view CRUD; core field types incl.
  link/lookup/rollup/formula; grid/kanban/form views; filter/sort/group; real-time collaboration;
  comments; record history; basic roles (owner/editor/viewer); REST API + tokens + webhooks;
  CSV/XLSX import/export; web + desktop clients.
- **v1 (P1)**: advanced permissions (custom roles, row/field rules); workflows; dashboards;
  calendar/gallery/gantt; mobile apps + offline; AI fields & AI building assistant; MCP server;
  snapshots/recycle bin; SSO.
- **v2 (P2)**: AI agents, chat-with-base, semantic search; pivot/map/hierarchy; sync connectors;
  extensions & scripting; 10M-row data tables; template marketplace; private deployment w/ local LLM.

---

## 9. Open questions for stakeholders

1. Target market & compliance: China (Feishu-like integrations, PIPL/等保) or global (GDPR), or both?
2. Deployment: SaaS multi-tenant, private deployment, or both from day one?
3. Client tech: accept TypeScript UI layer (Option A) or require all-Rust UI?
4. Which IM/ecosystem integrations matter (Lark/Feishu, DingTalk, WeCom, Slack, Teams)?
5. Offline requirement level: read-only offline, or full offline editing with merge?
6. AI models: which providers are allowed (data residency), and is on-prem LLM needed?
7. First business scenarios to validate (e.g. CRM, project mgmt, asset register)?
8. Team size & timeline for MVP.

---

## Sources
- Lark Open Platform – Base (bitable v1) docs: https://open.larksuite.com/document/server-docs/docs/bitable-v1/notification.md
- Lark Base JS SDK (table API): https://lark-base-team.github.io/js-sdk-docs/en/api/table
- Lark CLI Base skill (CLI direction for tables/fields/records/views/dashboards/workflows): https://skills.sh/larksuite/cli/lark-base
- Feishu – Base workflow tutorial (branches, loops, webhook, HTTP request): https://www.feishu.cn/content/article/7595146589556788441
- Feishu – workflow tools & AI agent node: https://www.feishu.cn/content/workflow-tools-and-systems
- Feishu – Base Agent / Team Agent: https://www.feishu.cn/content/article/7657782356450741223
- Feishu – AI record summary & workflow messages: https://www.feishu.cn/content/article/7597359403980852443
- Feishu – five Base modules (database, dashboard, advanced permissions, workflow, AI): https://www.feishu.cn/content/five-new-multidimensional-table-modules-feishu
- NocoBase – open-source Airtable alternatives overview: https://www.nocobase.com/en/blog/open-source-airtable-alternatives
- Teable – alternatives comparison: https://teable.ai/blog/best-nocodb-alternatives
- Toolradar – NocoDB vs Teable: https://toolradar.com/compare/nocodb-vs-teable

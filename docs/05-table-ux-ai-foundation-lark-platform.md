# Round 4 — Table UX catalogue, AI as the foundation, Lark Open Platform

> Status: research + design v0.3 · Builds on docs 01–04.
> New stakeholder requirements (R1–R8):
> R1 table colours & styles, light/dark · R2 open a row in a pop-up · R3 multiple tabs ·
> R4 task presets (task list, Kanban) · R5 embedded assets (images, attachments) ·
> R6 the full range of standard table features · R7 **AI as the foundational layer, not an add-on** ·
> R8 thorough research of the Lark OpenAPI & Developer Center.
>
> Note: the official doc hosts (open.feishu.cn / open.larksuite.com) are **blocked by this
> environment's network policy**. Lark facts below come from search results quoting the official
> pages (linked) and from prior knowledge. Items not seen in this pass are marked **[verify]**.

---

## Part 1 — Complete table feature catalogue (R1–R6)

Priority: **C** = core v1.0 (the "full table" scope), **L** = later.

### 1.1 Appearance & theming (R1)

| Feature | Detail | Prio |
|---|---|---|
| Light / dark / system theme | Follows `prefers-color-scheme`, with a per-user override. The grid renderer reads the same design tokens (DOM or canvas) | C |
| Design tokens | One token set (surface, text, border, accent, selection, status colours) shared by the Leptos UI and the grid; WCAG AA contrast checked automatically in tests for both themes | C |
| Base/table colour & icon | Per-base and per-table accent colour + icon (tabs, sidebar, kanban headers) | C |
| Table style presets | Density (compact/default/comfortable), zebra rows, grid lines on/off, header style (plain/filled/accent), font size | C |
| Option colours | Single/multi-select options use a **semantic palette** that remaps for dark mode, so colours stay readable in both themes | C |
| Conditional formatting | Row colour / cell colour / text style by rules (same filter AST); multiple rules with priority | C |
| Field header style | Field-type icon, description tooltip, required marker, lock marker | C |
| Custom themes | Org-level brand theme (logo, accent, fonts) | L |
| Print styles | Print/PDF of a view with the theme in light mode | L |

### 1.2 Record detail — pop-up and beyond (R2)

| Feature | Detail | Prio |
|---|---|---|
| Record pop-up (modal) | Opens from the row expand icon, the Space key, or a double-click on the primary field; shows all fields in a configurable layout | C |
| Side panel mode | Same component docked right; the grid stays interactive | C |
| Full page / new browser tab | `/{base}/{table}/r/{recordId}` deep link; "open in new window" | C |
| Prev/next navigation | ↑/↓ in the pop-up follows the current view order | C |
| Detail layout | Sections, field order, hidden fields per view; long text and attachments get large editors | C |
| Activity pane | Comments, cell history, process trace (lifecycle, assignment, SLA), linked records | C |
| Linked-record drill-in | Stacked pop-ups with breadcrumbs (record → linked record → …) | C |
| Lifecycle actions | Allowed transitions shown as buttons with required-field prompts | C |
| Presence | Who else has this record open; live field updates | C |
| Pop-out windows (desktop later) | Separate OS window in the future desktop client | L |

### 1.3 Multiple tabs (R3)

Three levels. All tab state is part of the URL and restored per user:

| Level | Detail | Prio |
|---|---|---|
| Table tabs inside a base | Like sheet tabs: reorder, rename, colour, hide, duplicate, overflow menu, keyboard switch | C |
| View tabs inside a table | Grid/Kanban/Calendar/… views as tabs; personal vs shared views grouped | C |
| App-level tabs | Several bases, records or dashboards open at once in our app shell (browser-like tab bar), with state kept per tab (scroll, selection, open pop-up). Plus normal browser tabs via deep links | C |
| Split view | Two views side by side (e.g., Orders grid + Customer kanban) | L |

### 1.4 Task presets (R4)

Shipped as **templates built only from generic primitives** (no special-case code), so every
preset feature also works in custom tables:

| Element | Implementation |
|---|---|
| Fields | Title, Status (**lifecycle**: To do → In progress → Review → Done, + Blocked), Assignee (assignment w/ acknowledge), Due date (SLA/reminder), Priority (select with colours), Tags, Estimate, **Parent task** (hierarchy self-link), **Subtasks** (reverse link + progress rollup), **Checklist** (new field type: ordered items with done state), **Dependencies** (blocked-by link) |
| Views | **Task list** (grouped by status/due, inline quick-add, checkbox to complete), **Kanban** (columns = lifecycle states, WIP limits, swimlanes by assignee), My tasks, Calendar, Timeline/Gantt |
| Loop closure | Unassigned → exception; overdue → SLA breach; blocked > N days → escalation; Done requires required fields (e.g., resolution) |
| Kanban behaviour | Drag between columns = lifecycle transition (rejected with a reason if not allowed); card cover image; card fields config; column collapse; quick-add per column |

Since you already have these presets, we will **match your existing field/view definitions** in
the template, so please share them (screenshots or a field list).

### 1.5 Embedded assets (R5)

| Feature | Detail | Prio |
|---|---|---|
| Attachment field | Multiple files, drag-drop, paste from clipboard, camera (mobile web), reorder | C |
| In-cell thumbnails | Image thumbnails in grid cells (lazy, cached), file-type icons otherwise | C |
| Preview / lightbox | Images (zoom/rotate), PDF, video, audio, text/code, office docs (rendered to PDF server-side, L) | C/L |
| Inline images in rich text | Long-text (markdown) with embedded images and links | C |
| Gallery / kanban covers | First image or chosen field as cover | C |
| Upload pipeline | Resumable chunked upload, size/type limits, content-type sniffing, **EXIF stripping** (privacy), thumbnail generation, virus-scan hook, quotas | C |
| Storage backends | Local filesystem (Docker default), S3-compatible (OpenDAL) | C |
| Access | Short-lived signed URLs, checked against record permissions; no public bucket | C |
| Derived content for AI | OCR text, captions and extracted PDF text stored as **derived values** (see Part 2) | C |
| Versioning | Replace-with-history for an attachment | L |

### 1.6 Standard grid functions (R6): full checklist

**Layout & navigation:** sticky header row, frozen columns (drag divider), row numbers,
column resize/auto-fit, column reorder (drag), hide/show fields panel, row height presets, text
wrap per field, zoom, horizontal/vertical virtual scroll, scroll to row/record, sticky group
headers, collapse/expand groups.

**Selection & editing:** cell/row/column/range selection, multi-range (Ctrl), select all;
inline edit with type-aware editors (date picker, select picker, user picker, link picker with
search, number with locale input, checkbox toggle); Enter/Tab/Shift navigation (Excel-like);
**copy/paste with Excel/Google Sheets interop** (TSV + HTML clipboard, auto type-cast, paste
creates rows); fill handle (drag down, series for numbers/dates); bulk edit selected cells;
**undo/redo** (per user, op-based); insert row above/below, duplicate row, delete with confirm;
manual row order (drag rows) when no sort is active; quick-add row at bottom.

**Query:** filter (nested AND/OR, per-type operators, relative dates, "me"), multi-level sort,
multi-level group (with group summaries), search with highlight & next/prev, **find & replace**,
quick filters (chips), saved views, personal views, view locking.

**Summaries:** column footer aggregates (sum, avg, min, max, count, filled, empty, unique, %
checked, date range), **selection summary bar** (sum/avg/count of the selected cells, like
Excel), group subtotals.

**Information:** field descriptions, cell validation errors inline, comment indicators, cell
history popover, presence (coloured cursors/selections of collaborators), "computing" indicator
for pending formula/AI values, lock indicators (approved/locked fields).

**Keyboard & accessibility:** full shortcut map (with a help overlay); screen-reader support;
focus management; high-contrast mode via tokens.

**Data in/out:** import/export from the view, print/PDF (L), share view link, embed view (L).

**Mobile web:** card list instead of the wide grid on small screens, swipe actions, form-first entry.

---

## Part 2 — AI as the foundational layer (R7)

### 2.1 What "foundational" means (and doesn't)

The market is moving this way. Airtable re-launched as "AI-native" (June 2025) with **Omni** (a
conversational builder) and **Field Agents** living inside bases
([Airtable CEO letter](https://www.airtable.com/newsroom/introducing-the-ai-native-airtable),
[Airtable community](https://community.airtable.com/announcements-6/introducing-the-ai-native-airtable-45645)).
Its fall release added **agent audit logs and agent permissions**
([fall release](https://community.airtable.com/announcements-6/fall-release-omni-2-0-airtable-canvas-a-bigger-mcp-agent-audit-logs-and-upcoming-agent-permissions-48850)).
Lark has the AI agent node with MCP tools and the Base Agent (doc 02).

The common weakness is that AI is still **bolted on**: it calls the same public API as a plugin,
its outputs look like normal cells with no provenance, and the permission, audit and loop logic
don't know an AI did something.

**Our definition:** AI is a **first-class part of the kernel**. The data model, value model,
operation log, permission engine, computation graph and loop engine all treat AI as a native
concept. However, **the deterministic Rust core stays the source of truth.** Models propose,
explain, extract and act through typed operations. They never bypass validation, permissions
or audit, and **the product still works fully with AI disabled** (self-host, privacy, Germany).
Research on LLMs and spreadsheets supports this split: LLMs read raw 2-D grids poorly and do
better with structural/semantic abstractions, while the formula/data engine remains the
reliable substrate ([Splinde](https://www.splinde.io/blog/llm-vs-spreadsheet)). AI-native
databases likewise put vector search and model inference inside the engine
([SynapCores](https://synapcores.com/blog/ai-native-database-guide), [SurrealDB](https://surrealdb.com/blog/context-layers-semantic-layers-and-knowledge-graphs-the-modern-data-architecture-for-ai)).

### 2.2 The seven AI-native kernel layers

```
┌────────────────────────────────────────────────────────────────────────────┐
│ 7  Experiences: Omni-style builder, chat-with-base, AI triage, AI steward    │
├────────────────────────────────────────────────────────────────────────────┤
│ 6  Agents as principals: identity, roles, budgets, approvals, audit         │
│ 5  Intent compiler: NL → typed plans (filter AST, op batch, schema ops,     │
│    workflow defs) → validate → preview diff → commit                        │
│ 4  Context engine: permission-filtered retrieval (structured + semantic +   │
│    lineage), token budgeting, prompt-injection fencing                      │
│ 3  AI computation: AI functions & AI fields as nodes in the dependency      │
│    graph; materialised, cached by input hash, async, cost-aware             │
│ 2  Semantic data layer: semantic schema, record cards, embeddings, derived  │
│    content (OCR/captions/PDF text), all updated from the op log             │
│ 1  Value model with provenance: every cell knows human/AI/formula/import/   │
│    workflow source, model, prompt version, confidence, evidence, review     │
├────────────────────────────────────────────────────────────────────────────┤
│ 0  Deterministic Rust core: types, validation, ops, permissions, lifecycle  │
│    (unchanged authority; AI enters only through layers 1–6)                 │
└────────────────────────────────────────────────────────────────────────────┘
          Model gateway (providers, local models, BYO key, metering) — pluggable
```

**Layer 1: value model with provenance (in `mt-core`).**
```rust
pub struct CellValue { pub value: Value, pub provenance: Provenance }
pub enum Source { Human { user: UserId }, Formula, Import { job: JobId },
                  Workflow { run: RunId }, Ai { agent: AgentId, model: ModelRef,
                  prompt_version: u32, confidence: Option<Decimal>,
                  evidence: Vec<EvidenceRef> } }
pub enum ReviewState { NotRequired, Suggested, Accepted { by: UserId }, Rejected { by: UserId } }
```
- AI output can be **committed** (for low-risk fields) or **suggested** (a pending op shown as a
  ghost value; one click accepts or rejects). Per-field policy, with a default of *suggest* for
  fields classified Confidential/PII.
- Filters can use provenance: "show AI-filled values not yet reviewed". Dashboards can show "% AI-generated".
- Every AI value links to its **evidence** (source cells, attachment pages, records used), so it
  can be audited.

**Layer 2: semantic data layer.**
- **Semantic schema:** each table/field carries machine-readable meaning: description, semantic
  type (person name, company, amount+currency, country, SKU…), units, examples, synonyms,
  glossary term, relationship meaning ("Order.customer → the buyer"). AI drafts it and a human
  approves it. This metadata is also the **data dictionary**, so data assets and AI share one source.
- **Record cards:** a canonical, compact text serialisation of each record (primary field +
  semantically important fields + linked titles) generated by `mt-core`. They are deterministic,
  so the same card comes out on every DB and in WASM.
- **Embeddings:** opt-in per table/field, computed from record cards and derived content and
  updated incrementally from the op log. Storage is portable across the three DBs: vectors are
  stored as int8-quantised BLOBs in the tenant DB, and an in-process **pure-Rust ANN index**
  (HNSW) is kept per tenant and rebuilt from the DB on demand. This avoids depending on
  pgvector, sqlite-vec or MySQL's vector type, which behave differently. **[verify crate choice
  + memory in spike S7]**
- **Derived content:** OCR, image captions, PDF/Office text extraction, and audio transcription
  (L) produce hidden derived values linked to the attachment. They are searchable and usable as
  AI evidence.

**Layer 3: AI computation in the dependency graph.**
- The formula language gets **AI functions**: `AI_CLASSIFY(text, options)`, `AI_EXTRACT(text, schema)`,
  `AI_SUMMARIZE(text, max_words)`, `AI_TRANSLATE(text, lang)`, `AI_SENTIMENT(text)`,
  `AI_MATCH(record, table)` (dedupe/entity resolution), `AI_ASK(prompt_template)`.
- The **AI field** type is a stored field whose producer is an AI function. Output types are typed
  (select, multi-select, number, date, JSON → several fields) and validated by the core.
- Non-determinism is handled explicitly. Results are **materialised and cached by
  hash(inputs, prompt_version, model)**. They are recomputed only when inputs change *and* the
  field's policy allows it, never on a silent model change. A model upgrade is a deliberate
  **"re-run with preview/diff"** operation.
- Async nodes: downstream formulas see `Pending` until the value arrives, and the grid shows the
  computing indicator. Batches are scheduled with per-tenant cost and rate budgets, and there is
  a cost preview before bulk runs.

**Layer 4: context engine.**
- Retrieval = structured filters (query AST) + semantic search (embeddings) + graph expansion
  (links/lineage). **Permission filtering happens before anything reaches a model**: row rules
  and field rules apply, and hidden fields never enter a prompt.
- **Field classification gates which provider may see what** (e.g., PII only to the self-hosted model).
- Cell content is untrusted. It is fenced as data in prompts, tools are scoped, and destructive
  plans always need human confirmation (prompt-injection defence).

**Layer 5: intent compiler (the core idea that makes AI native).**
The model never writes SQL or calls raw endpoints. It emits **typed plans in the kernel's own
languages**, using structured outputs or tool calling with JSON Schemas generated from the Rust
types (`schemars`):

| Intent | Typed target | Then |
|---|---|---|
| "Show overdue tasks of my team" | `FilterAst` + `SortSpec` | Validated, executed as a temporary view |
| "Mark these 30 as duplicates and merge" | `OpBatch` | Diff preview → user confirms → normal op pipeline |
| "Add a priority field and fill it from the description" | `SchemaOp` + AI field def | Preview → commit |
| "When a ticket is resolved, email the requester" | `WorkflowDef` | Lint (loop-closure checks) → enable |
| "Build me an asset register" | `BaseTemplate` (tables, fields, lifecycles, views) | Preview → create |
| "Why is SLA attainment down?" | `AnalysisPlan` (aggregations over query AST) | Executed deterministically; the model only narrates results with citations |

Every plan passes the same validation, permission, lifecycle and audit path as a human action.
That is what makes AI native rather than an add-on.

**Layer 6: agents as principals.**
- An agent has an identity (`AgentId`), an owner, a role (row/field rules like a user), budgets
  (tokens, money, ops/hour), allowed tools, an approval policy, and a schedule or triggers.
- Agent actions are ops with `Source::Ai`, appear in history, the trace and the hash-chained
  audit log, and can be reverted by batch.
- Agents can be assignees in lifecycles (e.g., the "AI triage" state owner), so the **closed-loop
  engine treats AI work like human work**: acknowledgement, SLA, exceptions on failure.

**Layer 7: experiences built on the layers.**
- **Builder** (Omni-like): describe → base/tables/lifecycles/views/workflows preview.
- **Chat with base**: answers come from deterministic analysis plans with record citations.
- **AI triage** in loops: classify + route + draft the reply, with a human approving.
- **AI data steward**: proposes dedupes, type fixes, missing owners, quality rules, schema
  descriptions; flags anomalies as exceptions.
- **Inline AI**: on a selection, "fill", "fix", "translate", "categorise", and so on (each one a typed op plan).

### 2.3 Model gateway (open-source friendly)
- Provider trait with adapters: Anthropic Claude, OpenAI-compatible endpoints (covers many
  vendors and **local Ollama/vLLM**), Azure. BYO key per tenant, configured in the admin dashboard.
- Capabilities negotiated per model: structured output, tool use, vision, context size, cost.
- **Local embedding/OCR option in pure Rust** (e.g., the `candle` ML framework) behind a cargo
  feature, so air-gapped installs keep the semantic features. Model weights are downloaded
  separately, so the base image stays small. **[verify performance in S7]**
- Metering per tenant/user/agent, response caching, retries, fallbacks, kill switch.
- Evals: golden sets for intent compilation (NL → AST accuracy), extraction accuracy, and
  injection tests, run in nightly CI against a recorded or mock provider; live-model evals are optional.

### 2.4 Testing AI under the 98–100% coverage rule
- All AI logic except the model call itself is deterministic Rust: plan validation, caching,
  provenance, context assembly, permission filtering and budgeting. It is fully covered like any other crate.
- The model is behind a trait. Tests use a **deterministic mock provider** and recorded fixtures
  (cassettes). Property tests check that *any* plan output by a model is either valid or rejected.
  It can never partially apply or bypass permissions.
- All six test categories apply: standard (correct plans), anomalous (malformed JSON, timeouts),
  edge (huge context, empty tables), invalid (plans referencing hidden fields → rejected),
  concurrency (AI batch vs human edits on the same cells), cross-platform (record cards
  byte-identical on all DBs/OS/WASM).

### 2.5 Feasibility & costs

| Topic | Assessment |
|---|---|
| Kernel changes | Provenance in the value model, AI nodes in the graph and agents as principals must be designed **in W1**. Retrofitting later is what makes AI an add-on elsewhere. Effort moderate, high payoff |
| Embedding storage | 1M records × 768 dims × int8 ≈ 0.77 GB per table. Hence opt-in per table/field, plus quantisation |
| Cost control | Budgets, previews, caching by input hash; suggested-mode by default for bulk |
| Privacy (EU/KR/IN/JP) | Classification-gated providers, local model option, no training on customer data, AI fully optional |
| Quality | Typed plans + validation make failures safe (rejected, not corrupting); evals track accuracy |

**Recommendation:** adopt the 7-layer design. Layers 0–3 are part of the core data model from
W1. Layers 4–7 ship with W11 but on foundations that already exist. Update the wave plan so that
**W1 includes provenance, semantic schema, record cards and agent principals**, and W3 includes
AI-function nodes in the dependency graph (with a mock provider).

---

## Part 3 — Lark Open Platform & Developer Center (R8)

### 3.1 Platform structure

| Area | What Lark offers | Source |
|---|---|---|
| App types | **Custom apps (自建应用)** for one tenant; **Store apps (商店应用)** for ISVs, distributed to many tenants | [Feishu: access tokens](https://open.feishu.cn/document/server-docs/api-call-guide/calling-process/get-access-token?lang=zh-CN) |
| App capabilities (developer console) | Bot, web app, gadget/mini-program, workplace widgets, message cards **[verify]** | prior knowledge |
| Credentials | `app_access_token`; **`tenant_access_token`** (acts as the app; third-party sources say it expires after 2 h **[verify]**); **`user_access_token`** (OAuth on behalf of a user; limited to that user's own permissions); store apps first obtain the app token via **`app_ticket`** | [Feishu: access tokens](https://open.feishu.cn/document/server-docs/api-call-guide/calling-process/get-access-token?lang=zh-CN), [user token](https://open.feishu.cn/document/server-docs/authentication-management/access-token/create-2?lang=zh-CN), [Go SDK](https://beta.pkg.go.dev/github.com/larksuite/oapi-sdk-go) |
| Scopes | Separate **app-identity** and **user-identity** scopes; app identity needs a separately configured **data range**; developers can debug with their own user token before publishing | [Feishu: API scopes](https://open.feishu.cn/document/server-docs/application-scope/introduction?lang=zh-CN) |
| Events | Two modes: **HTTP push to the developer server** (Encrypt Key → AES-256-CBC + Verification Token + signature check) or **long connection (WebSocket via SDK)**, which needs no public IP but is **custom apps only**; handlers must finish within **3 s** or Lark redelivers; the v2.0 schema is recommended to avoid duplicates | [long connection](https://open.feishu.cn/document/server-docs/event-subscription-guide/event-subscription-configure-/request-url-configuration-case?lang=zh-CN), [encryption](https://open.feishu.cn/document/server-docs/event-subscription-guide/event-subscription-configure-/encrypt-key-encryption-configuration-case?lang=zh-CN), [add events](https://open.feishu.cn/document/server-docs/event-subscription-guide/event-subscription-configure-/subscription-event-case?lang=zh-CN) |
| Base extensibility | Two open capabilities: **OpenAPI** and **Base extensions** (embed your product inside Base, distributed via a marketplace) | [Base extension intro](https://open.larksuite.com/document/uAjLw4CM/uYjL24iN/base-extensions/base-extension-introduction?from=home) |
| Base JS SDK | Front-end SDK for extensions (tables, fields, records, views, selection) | [JS SDK docs](https://lark-base-team.github.io/js-sdk-docs/en/api/guide) |
| Field shortcuts / automation extensions | Developers publish field shortcuts and **automation triggers/actions** (triggers push events via webhook) **[verify details]** | [Feishu workflow tools](https://www.feishu.cn/content/workflow-tools-and-systems) |
| AI developer tools | Official **MCP server** `@larksuiteoapi/lark-mcp` (Beta; stdio/SSE/StreamableHTTP; Base toolsets) and **Lark CLI** (`lark-cli base +…` for tables, fields, records, views, dashboards, workflows) | [MCP overview](https://open.larksuite.com/document/uAjLw4CM/ukTMukTMukTM/mcp_integration/mcp_introduction), [npm](https://www.npmjs.com/package/@larksuiteoapi/lark-mcp), [CLI skill](https://skills.sh/larksuite/cli/lark-base) |
| SDKs | Official server SDKs (Go, Java, Python, Node.js…) | [Go SDK](https://beta.pkg.go.dev/github.com/larksuite/oapi-sdk-go) |
| Doc index for LLMs | `open.feishu.cn/llms.txt` (blocked here; fetch in a verification pass) | [llms.txt](https://open.feishu.cn/llms.txt) |

### 3.2 Base (bitable v1) API: resources, limits, behaviour

Base path: `/open-apis/bitable/v1/apps/:app_token/…` Accepts `tenant_access_token` or `user_access_token`.

| Resource | Operations | Limits / notes | Source |
|---|---|---|---|
| App | get (`GET /apps/:app_token`), create, copy, update **[verify ops]** | get: **20 req/s** | [Get app](https://open.larksuite.com/document/uAjLw4CM/ukTMukTMukTM/reference/bitable-v1/app/get) |
| Table | list (paged), create, batch create, delete, batch delete, patch (rename) | list: **20/s** | [List tables](https://open.larksuite.com/document/uAjLw4CM/ukTMukTMukTM/reference/bitable-v1/app-table/list) |
| Field | list (optional `view_id`), create, update, delete | list: **20/s** | [List fields](https://open.larksuite.com/document/server-docs/docs/bitable-v1/app-table-field/list) |
| View | list, get, create, patch, delete | create: **10/s**; **max 200 views per table** (public + locked + personal) | [Add view](https://open.larksuite.com/document/server-docs/docs/bitable-v1/app-table-view/create) |
| Record | create, update (PUT), delete, **batch create/update/delete (≤ 500 rows)**, **search (POST, filter + sort body)**, batch get; *list (GET) is legacy, search is recommended* | create/update/batch: **50/s**; search/list: **20/s** | [Create](https://open.larksuite.com/document/server-docs/docs/bitable-v1/app-table-record/create), [Batch update](https://open.larksuite.com/document/server-docs/docs/bitable-v1/app-table-record/batch_update), [Search](https://open.larksuite.com/document/uAjLw4CM/ukTMukTMukTM/reference/bitable-v1/app-table-record/search), [List (legacy)](https://open.larksuite.com/document/uAjLw4CM/ukTMukTMukTM/reference/bitable-v1/app-table-record/list) |
| Form | get form, list/patch form fields **[verify]** | — | prior knowledge |
| Advanced permissions | roles CRUD, role members (batch add/remove) **[verify]** | — | prior knowledge |
| Dashboard | list, copy **[verify]** | — | prior knowledge |
| Workflow (automation) | list, enable/disable **[verify]** | — | prior knowledge |
| Attachments | uploaded via Drive media upload, referenced by file token **[verify]** | — | prior knowledge |
| Events | **`drive.file.bitable_record_changed_v1`** (add/edit/delete; **formula value changes do NOT trigger and are not in the payload**); **`drive.file.bitable_field_changed_v1`**; you must first call *subscribe* on each file; with a tenant token you need **both app- and user-identity** permissions; the app must be published | [Record changed](https://open.larksuite.com/document/server-docs/docs/drive-v1/event/list/bitable-record-changed), [Field changed](https://open.feishu.cn/document/server-docs/docs/drive-v1/event/list/bitable_field_changed), [Event list](https://open.larksuite.com/document/ukTMukTMukTM/uYDNxYjL2QTM24iN0EjN/event-list) |

### 3.3 What we adopt, and where we deliberately do better

| Lark pattern | Adopt? | Our version |
|---|---|---|
| Resource hierarchy app → table → field/record/view | ✓ | Same shape (familiar to Lark developers); stable prefixed IDs |
| Dual identity (app token vs user token), split scopes | ✓ | Same concept: **app/bot tokens** + **user OAuth (PKCE)** + personal tokens; scopes per identity; **agents are a third identity type** |
| Batch ≤ 500, search with filter body | ✓ | Same limits, plus an **atomic batch** option (all-or-nothing) and idempotency keys |
| Low per-endpoint QPS (20–50/s per app) | Improve | Documented, configurable limits for self-host; **streaming bulk export** (NDJSON/Parquet) so syncs don't need thousands of paged calls |
| Formula changes don't emit events | **Fix** | Opt-in **computed-field change events** (formula/lookup/rollup/AI). Closed loops need them |
| Per-file subscribe + publish before events work | Improve | Webhook subscriptions per base/table/field filter, created via API, active immediately; signed (HMAC), with delivery ledger & replay |
| Long connection only for custom apps | Improve | **WebSocket/SSE event stream for any token**, plus webhooks |
| 3 s handler timeout & redelivery | ✓ (spirit) | At-least-once with event IDs, configurable timeout, exponential backoff, dead-letter |
| Extensions + marketplace | ✓ (later) | Extension SDK (sandboxed iframe) using the same typed API; marketplace later |
| Field shortcuts (FaaS) | ✓ via AI functions | **AI/compute functions are kernel nodes** (Part 2 L3); custom functions later as WASM components |
| Official MCP (beta) & CLI | ✓ | **MCP server built in** (same permissions/audit; agents as principals) + Rust CLI generated from the OpenAPI spec |
| Machine-readable spec | Improve | **OpenAPI 3.1 published by every instance**, spec-diff CI gate, generated SDKs |

**Adoption idea (high value):** a **"Migrate from Lark Base / Airtable" importer** that reads
schemas, records, attachments and views through their APIs (user-supplied tokens) and maps field
types to ours. This lowers switching cost for customers in our markets, since Lark is sold
internationally as Lark Suite. **[decide]**

---

## Part 4 — Changes to the plan

| Wave | Additions from this round |
|---|---|
| W1 | Design tokens + light/dark theme; **provenance in value model**; **semantic schema metadata**; **record cards**; **agent principals** in identity/permissions; attachment storage pipeline |
| W2 | Full grid checklist (§1.6), record pop-up/side panel/full page, table & view tabs, app-level tabs, in-cell thumbnails, preview lightbox, conditional formatting |
| W3 | AI-function nodes in the dependency graph (mock provider), checklist field type |
| W5 | Task presets (task list, Kanban with WIP/swimlanes), matched to your existing presets |
| W10 | Built-in MCP server, CLI, Lark/Airtable importer (if approved) |
| W11 | Model gateway, embeddings + ANN, derived content (OCR/captions), intent compiler, agents, builder, chat-with-base, AI steward |
| New spike **S7** | Pure-Rust ANN index + local embedding (candle) memory/latency at 1M records; record-card determinism across DBs |

## Open questions
1. Please share your **existing task list / Kanban preset definitions** (fields, views, statuses).
2. **AI default policy:** should AI outputs default to *suggested* (needs human accept) or *committed* for non-sensitive fields?
3. Which **model providers** should be first-class (Claude, OpenAI-compatible/local, others)? Is local embedding (air-gapped) required for v1?
4. Approve the **Lark/Airtable migration importer**?
5. Allow network access to **open.feishu.cn / open.larksuite.com** for a verification pass on the [verify] items?

# Revision v0.5 — Field framework, reference data, parameters, integration-first, business capabilities, grounded AI

> Status: research + design · Refines doc 06. Where they conflict, this document wins.
> New stakeholder points (U1–U7):
> U1 custom fields (enable/disable, permissions, input types, prefixes, standard validation, field groups) ·
> U2 data dictionaries (hierarchical, custom fields) · U3 KV parameter configuration incl. field-type
> definitions · U4 reference data (ID/code/name/value, permissions, tenant validation, table + tree views) ·
> U5 make no assumptions about target users; **core value = integrating enterprise business data,
> with ease of use** (API, MCP, direct DB: SQLite/MySQL/PostgreSQL) · U6 no dependence on job titles:
> abstract **business capabilities** · U7 AI still "feels off", so research how Lark and others really apply AI.
>
> Access note: `https://base.feishu.cn/` and `www.feishu.cn` are **blocked by this environment's
> network proxy (HTTP 403)**, so the Lark "data table" page could not be viewed directly. Findings
> come from search results quoting Feishu/Airtable pages. Please share screenshots or a description
> if the concept you saw differs from §4.2.

---

## 0. What changed and why

| # | Change | Reason |
|---|---|---|
| U1 | A **field framework** that separates *data type*, *input widget*, *validation*, *display format* and *permissions*, plus **field groups** and a **field library** | Enterprises configure fields; they don't want fixed field types |
| U2+U4 | One **Reference Data** module: dictionaries are flat or tree-shaped sets with standard + custom attributes, tenant overlays, permissions, and table and tree views | Dictionaries and reference data are the same thing at different complexity levels. One module avoids two half-features |
| U3 | **Typed parameter registry** (scoped KV) and a **field-type registry** (custom derived types) | Configuration without code; reused by formulas, workflows and validation |
| U5 | Doc 06 assumed HRIS/IdP sources. **Corrected:** users and the org are optional and pluggable. The product's centre is an **integration hub** with push, pull, DB connections, API and MCP, designed for a *10-minute first integration* | Your stated core value; Lark integration is powerful but cumbersome |
| U6 | Permissions and approvals are now built on **business capabilities**, not roles, titles or a mandatory org chart | Customers model their own operations |
| U7 | AI reframed around **eight patterns that enterprises actually use** (Lark, Airtable, Microsoft, Salesforce, ServiceNow). AI's main job is to be the **adapter between messy enterprise reality and governed tables** | The earlier design started from architecture, not from proven usage |

---

## 1. Field framework (U1, U3)

### 1.1 A field = six independent facets

| Facet | Options |
|---|---|
| **Data type** (storage & semantics) | text, long text, number (exact decimal), currency, percent, boolean, date, datetime, duration, **reference** (to reference data set, §2), link (to table), person/principal, attachment, JSON, computed (formula/lookup/rollup/AI) |
| **Input widget** | text box, text area/rich text, number stepper, **dropdown (select)**, **multi-select**, **radio buttons**, **checkbox group**, segmented control, toggle, **cascader** (tree path: country → state → city), **tree picker**, searchable picker (large sets), tag input, date/time picker, slider, rating, signature, file uploader, barcode/QR scan, location |
| **Options source** | Inline list · **reference data set** (whole set, subtree, or one level, e.g. "leaves only") · query on another table · parameter (§3) |
| **Display format** | Prefix/suffix (`¥`, `kg`, `PO-`), number/date formats per locale, masking (`***-1234`), colour by option, **auto-number templates** `PO-{YYYY}{MM}-{seq:5}` (sequence per tenant, per year or per prefix) |
| **Validation** | Required / conditionally required; length/range/precision; regex; **standard format library** (§1.3); uniqueness with scope (table / within parent record / tenant); cross-field rules ("end ≥ start"); allowed states (editable only in lifecycle states X, Y) |
| **Governance** | **Status: enabled / disabled / deprecated**; field-level permissions (hidden / read / edit / masked per capability, §5); classification (PII…); owner; description & help text; i18n labels; change history (who changed the definition, when, why) |

**Status semantics** (so disabling never destroys data):
- **Enabled:** normal.
- **Disabled:** hidden from forms, new entry and default views; existing data is kept, visible in
  history and exports on request, and API writes are rejected.
- **Deprecated:** read-only everywhere and flagged in the UI and API, ready for removal. Removal is a
  separate, audited, reversible-until-purge action that runs an impact analysis first (views,
  formulas, workflows and integrations using the field).

### 1.2 Field groups and field library
- **Field group:** an ordered, named, collapsible set of fields with optional conditional visibility
  ("show *Bank details* when *Payment method = Transfer*") and group-level permission (e.g., the whole
  *Compensation* group is visible only with capability `employee.view_compensation`). Groups render as:
  - sections in the record pop-up and forms;
  - a **two-level header** in the grid;
  - nested objects in API payloads (optional).
- **Field library:** reusable, versioned field definitions shared across tables and bases ("Gender"
  bound to the `sex` reference set, "Employee ID" with format `^E\d{6}$`). Tables using a library field
  get its updates (or pin a version), so the same concept is encoded **the same way everywhere**. This
  is a data-asset feature: it prevents drift.

### 1.3 Standard format validation library (built in, for target markets)

| Category | Formats (with checksum where the standard defines one) |
|---|---|
| Contact | Email (RFC 5321 practical), URL, phone (E.164 + national formats US/JP/KR/DE/IN) |
| Postal codes | US ZIP / ZIP+4, JP `〒123-4567`, KR 5-digit, DE 5-digit, IN 6-digit PIN |
| Business/tax IDs | US EIN, DE USt-IdNr (VAT ID), JP Corporate Number (13 digits, check digit) and Qualified Invoice registration `T`+13, KR business registration number (10 digits, checksum), IN GSTIN (15 chars, checksum) and PAN |
| Banking | IBAN (mod-97), SWIFT/BIC, US ABA routing (checksum) |
| Codes | ISO 3166 country, ISO 4217 currency, ISO 639 language, UUID, custom regex |

Each validator is a pure Rust function in `mt-core` with test vectors (valid, invalid, edge cases),
shared by browser and server. **[verify each spec when implementing; national-ID numbers that are
sensitive (e.g., KR resident registration number) are deliberately *not* offered as collectable formats]**.

### 1.4 Field-type registry (custom derived types)
Tenants define **named field types** = base data type + widget + validation + format + default
permissions/classification. Examples:
- "Employee ID" = text · `^E\d{6}$` · prefix display none · classification Internal.
- "Amount (JPY)" = currency JPY · 0 decimals · ≥ 0.
- "Region" = reference(`region` set) · cascader · leaves only.

They appear in the field-type picker next to built-in types and are stored in the registry (§3) with versions.

---

## 2. Reference data & data dictionaries (U2, U4)

Following DAMA practice, **reference data** is the governed set of permitted values that classify
other data (code lists, lookup tables, hierarchies, crosswalks). It is relatively static and needs
its own governance, versioning and data-dictionary entry
([CASRAI](https://casrai.org/data-governance/reference-data), [TopQuadrant](https://topquadrant.com/blog/reference-data-management-master-data-management),
[Informatica R360](https://www.informatica.com/content/dam/informatica-cxp/techtuesdays-slides-pdf/Best%20Practices%20for%20Managing%20Reference%20Data%20at%20the%20Enterprise%20Level%20with%20R360.pdf),
[TIBCO](https://www.tibco.com/glossary/what-is-reference-data-management)).
**Master data** (customers, suppliers, products) are normal tables in our system. A "data dictionary"
is the simple, flat case of a reference set.

### 2.1 Model

**Reference set** (e.g., `gender`, `region`, `product_category`, `cost_center`, `ticket_priority`):

| Attribute | Notes |
|---|---|
| `id`, `code` (immutable, unique per tenant), `name` (i18n), description | `code` is what integrations and formulas use |
| `structure` | `flat` or `tree` (max depth optional; "leaf-only selectable" option) |
| `scope` | `system` (built-in, e.g., ISO sets) · `tenant` · `app/base` |
| `item_schema` | Standard attributes + **custom attributes** defined with the field framework (§1), e.g., *Region* items carry `timezone`, `tax_rate`, `manager_capability` |
| Owner / steward, status, version, change-approval policy | Governance |
| `crosswalks` | Aliases per external system (doc 06 §2.5): `ats_b: "F"`, `hris_a: "2"` |

**Reference item**: `id` (stable), `code`, `name` (i18n), `value` (typed, optional, e.g., numeric
weight), `parent_id` (tree), `sort_order`, `status` (enabled/disabled), `valid_from/valid_to`,
custom attribute values, crosswalk aliases.

### 2.2 Tenant-level validation (effective set)
- **Effective set for a tenant** = system items ∪ tenant extensions − items the tenant disabled,
  with tenant label overrides. Governance rule: tenants may **extend and relabel** but not change
  system codes, which prevents conflicting code sets for the same concept
  ([Informatica R360 on different businesses keeping different values](https://success.informatica.com/velocity/solutions/360-engagement/reference-360.html),
  [EWSolutions](https://www.ewsolutions.com/?p=1110)).
- **Write validation** (UI, API, import, integration, AI): the value must be an **enabled** item of
  the tenant's effective set, valid on the record's effective date, and allowed by the field's
  subtree/level restriction. Anything else is rejected, or quarantined for integrations.
- **Disabling an item:** existing records keep it (shown as "Name (disabled)"), new entries can't
  select it, and reports still group it correctly.
- **Permissions:** per set, *use* (select values), *view* (browse set), *manage items*, *manage
  schema*, *approve changes*. Optionally per subtree (a regional admin manages only their branch).

### 2.3 Views & operations
- **Table view:** all items with attributes, filter, sort, bulk edit, import/export CSV/XLSX
  (`parent_code` column for trees).
- **Tree view:** expand/collapse, drag to reparent (validated: no cycles, depth limit), path display
  `Asia / Japan / Kanto / Tokyo`, search showing paths, counts of records using each item.
- **Usage & impact:** before disabling or moving an item, see how many records, filters, workflows and
  integrations reference it.
- **Versioning:** every change is versioned and audited; changes can require approval; "as of"
  browsing.
- **Storage/performance:** reference sets live in the tenant DB and are cached in memory per
  tenant, with a closure table for trees. "Is under Kanto" filters are bitset or closure lookups.
- **API:** `GET /refdata/{set}/items?view=tree|flat&as_of=`, `GET /refdata/{set}/items/by-code/{code}`,
  `POST /refdata/{set}:validate` (bulk), `POST /refdata/{set}:import`, crosswalk lookup
  `GET /refdata/{set}/crosswalk/{system}/{external_value}`.

### 2.4 Built-in system sets (read-only, extendable)
ISO 3166-1/-2 (countries, subdivisions), ISO 4217, ISO 639, ISO/IEC 5218 (sex; separate from
gender identity), time zones (IANA), units of measure, fiscal period types. Locales:
en/ja/ko/de/hi labels where standard names exist.

---

## 3. Parameter registry (KV configuration) (U3)

| Aspect | Design |
|---|---|
| Key | Namespaced, immutable: `approval.default_timeout_hours`, `import.max_rows`, `ai.provider.default` |
| Type | string, int, decimal, bool, duration, date, cron, **enum (bound to a reference set)**, JSON (validated by a JSON Schema), **secret** (encrypted at rest, write-only, never returned by the API), **field-type definition** (§1.4) |
| Validation | Range/regex/schema; custom validator per key |
| **Scope hierarchy** | system default → tenant → workspace/base → (user, for user-preference keys only). **Effective value resolution** with an *explain* ("value 48 comes from tenant override set by X on date Y") |
| Governance | Description, owner, sensitivity, change approval for critical keys, full version history, rollback |
| Consumption | Typed access in Rust (`params.get::<Duration>(key, ctx)`), `PARAM("key")` in formulas, workflow conditions, validation rules; changes publish events (hot reload, caches invalidated) |
| Feature flags | A boolean/enum parameter with rollout scope (tenant, base) |
| UI | Admin dashboard: grouped by namespace, search, diff between scopes, import/export (secrets excluded) |

---

## 4. Integration-first: the product's centre (U5)

### 4.1 Corrected assumptions
- **No dependency on the client's user database or HR system.** Accounts can be local (email/password,
  passkeys) or federated (OIDC/SAML; SCIM optional). Org units are **optional**: if a client has
  none, scopes use any reference tree (regions, cost centres) or any table (projects, stores).
- The value we sell is **getting the client's business data in and out reliably and easily**, then
  running governed processes on it.

### 4.2 What competitors offer, and where it hurts

| Product | Mechanism | Limits / friction |
|---|---|---|
| Lark Base | OpenAPI (token types, per-endpoint QPS 10–50/s, batch ≤ 500), events needing per-file subscription; formula changes emit no events (doc 05); "data connector" import for analysis ([InfoQ](https://www.infoq.cn/article/ix1zmftlurti6iksiyzy)); 2024 "Base database" with 1M rows per table, help centre says max 2M rows per table ([Feishu launch](https://www.feishu.cn/content/wechat_post_35), [row expansion](https://www.feishu.cn/hc/zh-CN/articles/389918352455)) | Teams write sync scripts themselves: token handling, paging, type conversion, idempotent upserts. Third-party "DB sync assistant" sidebar plugins and ETL tools fill the gap; community advice prefers scheduled pulls over webhooks due to trigger limits ([Juejin DB sync](https://juejin.cn/post/7670475896516329523), [Juejin MySQL realtime](https://juejin.cn/post/7668704642684420136), [CSDN API sync](https://bbs.csdn.net/weixin_33404102/article/details/100502130), [DataY](https://juejin.cn/post/7680836559906553896)) |
| Airtable | **Sync API: POST raw CSV to a synced table** (≤ 10k rows per run, ~2 MB per request, 20 requests / 5 min / base, **one-way**, synced records can't be edited) ([Airtable Sync API](https://support.airtable.com/docs/airtable-sync-integration-sync-api), [API ref](https://airtable.com/developers/web/api/post-sync-api-endpoint)); **HyperDB** (Enterprise; CSV or Databricks/Salesforce/Snowflake; up to 100M rows; one-way; published subsets) ([HyperDB](https://support.airtable.com/docs/hyperdb-in-airtable)) | Push is simple but CSV-only, one-way, low throughput; large-data path is Enterprise-only |

**Lesson:** the simplest experiences are **push raw data, map it in the UI**, and **connect a source,
pick a table, schedule**. Both competitors make these one-way and limited. We make them first-class,
two-way capable, governed and testable.

### 4.3 Four integration modes, one governed pipeline

```
 PUSH (client → us)      PULL (us → client)         API / SDK / MCP        OUTBOUND (us → client)
 ┌──────────────────┐   ┌───────────────────────┐   ┌─────────────────┐    ┌──────────────────────┐
 │ Table inbox URL  │   │ DB connector:         │   │ REST (OpenAPI)  │    │ Webhooks (incl.      │
 │ JSON/NDJSON/CSV/ │   │  SQLite file, MySQL,  │   │ SDKs, CLI       │    │  computed changes)   │
 │ XLSX, any shape  │   │  PostgreSQL (read-    │   │ MCP server for  │    │ HTTP push to their   │
 │ sync or async    │   │  only user, table/    │   │ AI agents       │    │  API, DB write-back  │
 │                  │   │  view/SQL, increment.)│   │                 │    │  (guarded), files    │
 │                  │   │ REST pull, files      │   │                 │    │                      │
 └────────┬─────────┘   └──────────┬────────────┘   └───────┬─────────┘    └──────────▲───────────┘
          └──────────────► MAPPING SPEC (fields, reference-data value maps, transforms, tests)
                          ► XREF / upsert keys ► VALIDATION (field framework) ► QUARANTINE
                          ► COMMIT via op pipeline (provenance = integration) ► RUN LEDGER + TRACE
                          ► RECONCILIATION ► EXCEPTIONS (owner) ─────────────────────────┘
```

**A. Push: "table inbox"** (the biggest ease-of-use win)
- Every table can enable an **inbox endpoint** with its own scoped token: `POST /in/{inboxId}`.
- **Clients send data in their own shape** (their JSON field names, codes and formats). There is no
  need to learn our field IDs. Mapping happens on our side in the UI, AI-assisted (§6 pattern 1).
- Body formats: JSON object or array, NDJSON, CSV, XLSX.
- **Sync vs async is decided for the client, with explicit rules:**
  - ≤ 500 records and no slow transforms → **synchronous**: `200` with per-record results
    (created/updated/unchanged/quarantined + reasons).
  - Larger, or `Prefer: respond-async` → **asynchronous**: `202` with `job_id` + status URL; optional
    callback URL; results downloadable.
  - Every response carries `trace_id`, and `Idempotency-Key` makes retries safe.
- Upsert by the client's own key via xref. Per-key ordering is guaranteed. Deletes are explicit
  (`_op: "delete"`) or via a "full snapshot" mode, in which missing keys are retired after confirmation.
- Inbox "**first contact**" flow: the first payloads are captured as samples, the user maps them in the
  UI (AI suggests), runs a dry run on the samples, then enables processing. The samples are replayed.

**B. Pull: direct database & REST connectors**
- **DB connections: SQLite (file/upload), MySQL, PostgreSQL** (the engines we already support). Pick a
  table or view, or write a SQL query (read-only, with a timeout and row cap). Incremental sync by
  `updated_at` / monotonic PK / full compare. Schedule (cron) or manual; CDC later.
- Security: credentials encrypted (secret parameters), read-only DB user recommended and checked,
  network egress allowlist, SSRF protection, connection limits.
- REST pull: paginated JSON with JSONPath field selection; OAuth2/API-key auth; rate-limit aware.
- Files: SFTP/S3 folder or email-in attachments, using **import templates** (doc 06 §2.9).

**C. API / SDK / MCP**
- REST API (OpenAPI 3.1), generated SDKs, CLI.
- **MCP server**: tools for schema discovery, record search/upsert, reference-data lookup and
  validation, inbox push, job status. It runs under the agent's capabilities (§5) with a full audit.

**D. Outbound**
- Webhooks for record and computed-field changes, lifecycle and approval events; HTTP actions in
  workflows; **guarded DB write-back** (opt-in per field, with source-of-truth rules).

### 4.4 Integration-backed tables (our version of a "data table")
A table whose **source of truth is external** (pushed or pulled) has:
- **Source fields:** read-only locally and updated by the integration, with the raw payload retained.
- **Local enrichment fields:** editable here, e.g., status/lifecycle, owner, comments, AI fields, approvals.

So a company can run **closed-loop processes on top of data that stays mastered in their ERP/HR/CRM
system**. For example, invoices synced from the ERP get an approval lifecycle here, and the result is
written back through an outbound action. Airtable's synced tables and HyperDB are one-way and
read-only, so this combination is a differentiator.

### 4.5 Ease-of-use targets (tested, spike S11)

| Scenario | Target time to first successful sync |
|---|---|
| Developer pushes JSON from a script to a table inbox | **≤ 10 min** (copy URL+token, send, map in UI) |
| Admin connects a MySQL/PostgreSQL table, incremental schedule | **≤ 10 min** |
| Business user uploads a recurring XLSX via import template | **≤ 5 min** |
| AI agent (MCP) reads schema and upserts records | **≤ 5 min** setup |
Measured with usability tests and scripted E2E in CI (timing of the automated parts).

---

## 5. Business capabilities instead of roles/titles (U6)

### 5.1 Model
- **Business object:** any table, or a group of tables, representing something the business manages
  (purchase request, store report, supplier, contract).
- **Capability** = *action on a business object*, defined by the customer from their own operations:
  `purchase_request.submit`, `purchase_request.approve`, `purchase_request.view_amount`,
  `supplier.onboard`, `store_report.submit`, `store_report.consolidate`. Built-in generic actions
  (create, read, update, delete, export, comment, run workflow, manage schema) exist for every object.
  **Custom capabilities** map to lifecycle transitions, buttons, field groups and API operations.
- **Grant** = principal + capability + **scope** + **conditions** + validity:
  - Principals: user, group, org unit (if any), **external guest**, **AI agent**, API app.
  - Scope: a data-scope expression over *any* dimension: own records, a reference-data subtree
    (`region under Kanto`), a set of records in another table (`stores I manage`), org subtree if an
    org exists, or all.
  - Conditions: e.g., `amount ≤ 1,000,000 JPY`, `entity = JP01`, `lifecycle state in (...)`.
  - Validity: from/to, for temporary delegation.
- **Capability sets:** named bundles for convenience ("Purchasing operations"). They are optional and
  not tied to job titles.
- **Separation of duties** is expressed over capabilities: `submit` and `approve` on the same record
  by the same principal is forbidden.
- **Explain:** "Why can X do Y on record Z?" returns the grant chain.

### 5.2 Effects on other modules
- **Approvals** resolve approvers by capability, not title: "principals holding
  `purchase_request.approve` whose scope contains this record and whose condition limit ≥ amount; if
  several, mode = any-one". The DoA matrix (doc 06) becomes a set of capability grants with amount
  conditions. That is more general and needs no org chart.
- **Field permissions** become capabilities on field groups (`employee.view_compensation`).
- **AI agents** receive capability grants like users, with budgets.
- **Org model** (doc 06 §2.1) becomes an **optional module** that adds org-based scopes and manager
  chains when the client has or wants one.

### 5.3 Capability discovery from existing operations
Typical onboarding:
1. Import or connect the client's data (§4).
2. List the business objects (tables).
3. **AI proposes a capability map** from the client's process documents, existing spreadsheets'
   columns/status values, and API operations (e.g., "your `status` column has *Submitted → Approved →
   Paid*, so proposed capabilities are submit, approve, pay").
4. The admin reviews and confirms.
5. Grants are assigned to groups or users.

This is a concrete, bounded AI job (§6 pattern 5).

---

## 6. AI, grounded in what enterprises actually use (U7)

### 6.1 Research: AI capabilities shipping in enterprise tools

| Platform | Shipped capabilities (vendor sources) |
|---|---|
| **Lark Base** | **Field shortcuts**: extract, classify, summarise, translate, custom AI autofill, plus third-party shortcuts (contract extraction, OCR, AI search); **AI formula generation**; **AI build assistant**: drafts a base from a description, batch-creates fields, matches types, generates options and dashboards; **AI sidebar**: data retrieval, knowledge Q&A, analysis, related-record recommendations; **AI Agent node in workflows** with model, memory and tools (incl. MCP) ([Feishu AI in Base](https://www.feishu.cn/content/article/7579168498044161238), [field shortcuts](https://www.feishu.cn/content/ai-powered-multidimensional-tables-easy-shortcuts-for-beginners), [AI inspection case](https://www.feishu.cn/content/article/7593293322865724607), [workflow agent node](https://www.feishu.cn/content/workflow-tools-and-systems)) |
| **Lark Aily** (enterprise agents) | Group-chat Q&A assistant answering from a knowledge base, **handing over to on-duty staff when it can't answer, logging questions into Base and producing a question-analysis report**; reimbursement assistant (policy lookup, **invoice recognition, registration and compliance pre-check**); customer follow-up assistant; a manufacturer's dealer Q&A bot (vendor claim: onboarding from 2 months to 1 week); event listening on email, doc comments and chats → summary + draft reply or follow-up action ([Aily overview](https://www.feishu.cn/content/article/7621764358124604369), [Aily assistant upgrade](https://www.feishu.cn/content/article/7597739180457806780), [Aily agent customisation](https://www.feishu.cn/content/article/7631864469689240764), [Aily update May 2026](https://aily.feishu.cn/hc/8dlnbe0s_hv4ubub7)) |
| **Airtable** | **Omni**: builds data model, imports data, configures automations and AI fields from prompts/files. **Field Agents**: cell-level retrieve/analyse/generate (documents, web search, images, translation, transcripts), triggered by record changes, connectors to read/write external tools (Gmail, HubSpot…), credit-metered; Owners/Creators configure, Editors run; admin controls incl. field permissions and audit logs ([field agents](https://support.airtable.com/docs/using-airtable-ai-in-fields), [connectors](https://community.airtable.com/announcements-6/connect-field-agents-to-your-favorite-tools-47950), [admin controls](https://www.createwith.com/tool/airtable/updates/airtable-agent-admin-controls-add-field-permissions-and-hipaa-support), [fall release](https://www.airtable.com/articles/fall-release)) |
| **Microsoft** | **Excel Agent Mode** (GA): multi-step building/editing of workbooks using tables, charts, PivotTables, formulas; works with Business Central data in Excel; Copilot Studio agents use business records and approval flows ([Excel blog](https://techcommunity.microsoft.com/blog/excelblog/agent-mode-in-excel-is-now-generally-available-on-desktop/4457408), [Business Central](https://dynamicscommunities.com/ug/dynamics-business-central-nav-ug/copilot-agent-mode-a-new-way-to-work-with-business-central-data-in-excel/), [platform overview](https://agentmarketcap.ai/blog/2026/04/08/salesforce-agentforce-servicenow-microsoft-copilot-studio-crm-itsm-battle)) |
| **Salesforce / ServiceNow** | Agents embedded in the **system of record**, using existing records, workflows and permissions; governance of agent identities, permissions and runtime behaviour (ServiceNow AI Control Tower) ([ValueAdd VC](https://valueaddvc.com/blog/ai-in-enterprise-software-2026-salesforce-sap-servicenow-whats-actually-shipping), [Agentforce vs ServiceNow vs Copilot](https://agentmarketcap.ai/blog/2026/04/08/salesforce-agentforce-servicenow-microsoft-copilot-studio-crm-itsm-battle)) |
| **Adoption reality** | Only ~23% of enterprises are scaling agents and ~39% are stuck experimenting (secondary source) ([Unite.AI](https://www.unite.ai/best-ai-agents-for-business-automation/)); integration and data quality are the top blockers (doc 06 §1.2) |

### 6.2 Eight proven patterns (what AI actually does at work)

| # | Pattern | Seen in |
|---|---|---|
| P1 | **Messy input → structured record** (documents, images, chats, emails, external payloads) | Lark contract/invoice extraction, Aily CRM capture from chat screenshots, Airtable field agents |
| P2 | **Cell-level enrichment** (classify, tag, summarise, translate, score) | Lark field shortcuts, Airtable field agents |
| P3 | **Knowledge Q&A with escalation and logging** | Aily group assistant → on-duty staff → Base log → report |
| P4 | **In-process pre-check & drafting** (compliance checks, reply drafts) | Aily reimbursement pre-check, event-driven drafts |
| P5 | **Builder assistance** (schema, options, formulas, dashboards, automations) | Lark AI build, Airtable Omni, Excel Agent Mode |
| P6 | **Data Q&A / analysis** | Lark AI sidebar, Excel Agent Mode |
| P7 | **Event-driven agents** (listen → summarise → draft → act after approval) | Aily event listening, Lark workflow agent node |
| P8 | **Agents inside the system of record with governance** (identity, permissions, audit) | Salesforce, ServiceNow, Airtable admin controls |

### 6.3 Why the earlier design felt off, and the correction
- Docs 05/06 started from **kernel architecture** (7 layers, planner) and over-weighted **NL data
  querying (P6)**. Data querying is the pattern with the weakest enterprise evidence and the hardest
  reliability problem.
- The proven value is in **P1, P2, P3, P4, P5**: turning messy reality into governed records and
  helping people inside processes. That is also **exactly your core value**: integrating enterprise
  business data with ease.
- **Corrected positioning:** *AI is the adapter between messy enterprise reality and governed
  tables, and the assistant inside processes. The deterministic core (field framework, reference
  data, capabilities, lifecycles, audit) decides what is valid.* The kernel layers from doc 05
  (provenance, AI as a principal, permission-filtered context) stay as infrastructure. They are
  plumbing, not the product story.

### 6.4 AI feature set v1 (prioritised by value × feasibility)

| Priority | Feature | Pattern | Grounding in our model |
|---|---|---|---|
| **1** | **AI mapping for integrations & imports**: map any pushed payload, DB table, CSV/XLSX to fields; map values to **reference data** (`"F"`, `"女性"`, `"weiblich"` → `sex:2`); propose transforms; generate the mapping's example tests | P1 | Output = mapping spec (deterministic execution afterwards); unmatched → quarantine |
| **2** | **AI capture**: document/image/email/chat → record (with evidence spans) via inbox, email-in, upload, IM bot (later) | P1 | Field framework validates; reference data constrains outputs; suggested mode |
| **3** | **AI fields (shortcuts)**: extract, classify, summarise, translate, score, custom prompt; outputs typed and bound to reference sets | P2 | Cached by input hash; provenance; per-field review policy |
| **4** | **AI builder**: from a description, **a sample spreadsheet, a DB schema or an API payload** → tables, field library entries, reference sets (with tree), field groups, lifecycles, forms, capability map | P5 | Preview → confirm → normal schema ops |
| **5** | **Process assistant**: pre-checks at approval/lifecycle steps (findings, never decisions); reply/notification drafting; **Q&A bot with escalation to an owner and logging of unanswered questions into a table** | P3, P4 | Uses capabilities for escalation targets; every unanswered question becomes a task |
| **6** | **Data steward**: duplicates, unmapped/odd values, missing owners, reference-data candidates ("these 14 free-text values look like a new category") | P2 | Creates exceptions/tasks |
| 7 | Data Q&A via **bounded planner** (doc 06 §4), scoped to certified metrics and find-records | P6 | Kept but **lower priority** |
| 8 | Event-driven **agents** with capability grants, budgets, audit | P7, P8 | After 1–6 are solid |

**Model access:** pluggable providers incl. self-hosted (doc 05 §2.3). The product works fully without AI.

---

## 7. Plan changes

| Wave | Change |
|---|---|
| W1 Foundations | **Field framework (facets, widgets, status, groups, library, validation library, field-type registry), reference data module (flat/tree, custom attributes, tenant overlay, table+tree views), parameter registry**; org module becomes optional |
| W2 Grid | Two-level grouped headers, widget-aware editors, reference pickers/cascaders |
| **W3 Integration core** (moved up from W8) | **Table inbox (push, sync/async), DB connectors (SQLite/MySQL/PG), import templates, mapping specs with reference-data value maps, xref/upsert, quarantine, run ledger, integration-backed tables**; MCP read/upsert tools |
| W4 Computation | Formulas (incl. `PARAM()`, reference attributes), links/lookups/rollups |
| W5 Collaboration & views | as before |
| W6 Permissions | **Capability model** (grants, scopes over any dimension, conditions, SoD, explain), optional org scopes |
| W7 Process | Lifecycles, approvals with **capability-based approver resolution**, SLA, exceptions, trace-audit |
| W8 Integration advanced | REST pull, outbound write-back, bidirectional rules, drift detection, reconciliation reports, more connectors |
| W9 Reports & data exchange | as doc 06 |
| W10 Platform | OpenAPI/SDKs/CLI, full MCP, admin & user dashboards |
| W11 AI | Features 1–6 first (1 and 4 can start right after W3); 7–8 later |

New spike **S11**: integration ease. Time-to-first-sync for the four scenarios in §4.5, plus
sync/async correctness under load (500 vs 100k-record pushes, retries with idempotency keys).

## 8. Open questions
1. **Lark "data table":** could you share a screenshot or short description of what you saw at
   base.feishu.cn? The site is blocked from this environment, and I want §4.4 to match or improve on it.
2. Field widgets: is anything missing from §1.1 that your current product already has (for example
   address composite fields or Lark's "group" field)?
3. Parameter scopes: do you need **user-level** values beyond preferences, or should values stop at
   tenant/base?
4. Can you give **2–3 real capability examples** from your customers' operations (object + actions +
   conditions)? I'll use them as reference templates and test fixtures.
5. Direct DB connections: should **write-back** to client databases be in v1, or read-only first?

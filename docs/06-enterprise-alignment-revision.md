# Revision v0.4 — Enterprise Alignment: business operations, AI that does real work, capacity model

> Status: research + revised design · Supersedes doc 05 Part 2.2 Layer 5 ("intent compiler") and
> refines docs 03–05 wherever they conflict.
> Questions answered:
> Q1 Does the AI design integrate with real business operations?
> Q2 Is the "intent compiler" feasible?
> Q3 Refine the memory/performance estimation method.
> Q4 Align the table foundations with real enterprise use: permissions, multi-tenancy, complex
> approvals, API integration with inconsistent fields, large and dynamic org structures, task tracking.
>
> Most enterprise case material is vendor-published; such claims are marked as such.
> Estimates marked **[model]** are calculated and must be calibrated by spike S9.

---

## 0. Short answers

| Q | Honest answer | What changed |
|---|---|---|
| Q1 | Doc 05's AI design was **architecturally sound but not grounded in business**. It described layers, not the jobs AI does in real processes, and it was missing the enterprise prerequisites AI depends on: an org model, code sets and mappings, approval semantics, certified metrics | §3 maps AI to **10 concrete business jobs**, each with inputs, outputs, deterministic guardrails, a human checkpoint and a metric. §2 adds the missing enterprise foundations |
| Q2 | Open-ended "natural language → any plan" is **not reliable enough** for enterprise use. A **bounded** version is feasible: the model picks from governed, enumerated options and code compiles and checks the result. Recent research supports exactly this split | Renamed to **bounded planner**; design, evidence, acceptance tests and spike in §4 |
| Q3 | Kept and extended into a **per-field-type capacity model** for server cache, database storage, client window, wire payload, formula recompute, org structure and embeddings | §5, plus a product feature: a schema-time capacity estimator |
| Q4 | The table foundations were generic. Enterprise reality needs **effective-dated org structures, data-scope permissions, a real approval engine, a mapping layer with code sets and cross-references, data-collection campaigns, period-close reports** | §2, with revised waves in §6 |

---

## 1. Research: how enterprises really use spreadsheet-like tables

### 1.1 Observed usage patterns

| Pattern | Real examples (mostly vendor-published) | What it needs underneath |
|---|---|---|
| **Operational data entry / periodic reporting (填报)** | A department store chain consolidates operating data from **25 stores**: each region maintains only its own view, rolling up into a master table and dashboard. A ticketing company has all staff enter performance data online, with live dashboards ([Feishu cases](https://www.feishu.cn/content/feishu-multidimensional-spreadsheet-tools), [case library](https://base.feishu.cn/academy/user-cases)) | Assigned collection tasks per org unit and period, row scope per unit, deadline tracking, consolidation, period lock |
| **Lightweight business systems** | Order/member/sales-guide system with 5 modules built in 3 days; marketing management system; 50+ customers co-built "app mode" systems ([Feishu app mode](https://www.feishu.cn/content/article/7578812241772924111)) | Relational model, forms, role-based UIs, lifecycles |
| **Company-wide management on tables** | One retailer runs **300 tables and 13 dashboards** (sales, return rate, supplier spend) ([Feishu](https://www.feishu.cn/content/feishu-multidimensional-spreadsheet-tools)) | Governance: ownership, certified metrics, lineage, at scale |
| **Process data synchronisation** | A global commodity trader syncs data across **815 processes** from procurement and transport to warehousing and shipping ([Feishu](https://www.feishu.cn/content/feishu-multidimensional-spreadsheet-tools)) | Integration with source systems, cross-references, reconciliation |
| **Approval data in tables** | A hospital R&D department syncs approval data into tables ([Feishu](https://www.feishu.cn/content/feishu-multidimensional-spreadsheet-tools)); Airtable customers run capital-call approval and payment workflows ([Airtable community](https://community.airtable.com/announcements-6/change-makers-streamlining-capital-calls-processing-with-sonia-alonso-47104)) and PTO approval routing ([Airtable](https://www.airtable.com/use-cases/pto-absence-approval-routing-business-services)) | An approval engine bound to records, with write-back and locking |
| **HR system as source of truth** | ~1,500-employee company runs workforce planning on Airtable with a **scheduled Workday sync** feeding employee records ([Airtable use case](https://www.airtable.com/use-cases/workforce-planning-headcount-construction-software)) | Connectors, mapping, effective dating, source-of-truth rules |
| **AI on business data** | Contract key-term extraction from PDF/Word/images ([Feishu](https://www.feishu.cn/content/article/7594396362599353538)); a fashion retailer's after-sales hub pulls group-chat messages into a table, alerts on-duty staff if unanswered, and AI-classifies replies as valid or not; AI scoring of sales visit notes; resume screening listed as a use case ([Feishu AI shortcuts](https://www.feishu.cn/content/article/7592534632910867658), [Feishu AI overview](https://www.feishu.cn/content/article/7579168498044161238)) | Extraction with evidence, classification with routing, review loops. Accuracy data is not published, so customers must test on their own samples |

### 1.2 What goes wrong in enterprises (evidence)

- **Spreadsheet (end-user computing) risk:** regulators accept spreadsheets for business-critical
  work only with controls. A recurring problem is that it's unclear **who changed, reviewed or
  approved a formula** ([arXiv: spreadsheets & the financial collapse](https://arxiv.org/pdf/0908.4420),
  [GARP](https://www.garp.org/risk-intelligence/technology/spreadsheets-firms-risk-031023?hs_amp=true),
  [Mitratech on OCC scrutiny](https://mitratech.com/resource-hub/blog/occ-steps-up-scrutiny-of-eucs-at-financial-institutions/)).
  → Our provenance, history and hash-chained audit cover the data. We must also **version and
  approve formulas and metric definitions** (§2.7).
- **AI pilots fail on integration and data, not models:** the MIT report finds that most GenAI
  pilots show little P&L impact and blames integration and the learning gap. Surveys name
  **data quality and integration** as the top blockers, and **"inability to integrate pilots into
  systems and workflows"** as the #1 scaling barrier at 52%
  ([CO/AI on MIT](https://getcoai.com/news/mit-study-reveals-95-of-ai-pilots-fail-to-deliver-business-results/),
  [Techstrong](https://techstrong.ai/aiops/why-your-ai-pilots-are-failing-its-not-the-models/),
  [Alteryx survey](https://smartling.alteryx.com/?p=156286), [ITPro/Informatica](https://www.itpro.com/technology/artificial-intelligence/only-a-handful-of-generative-ai-projects-make-it-into-production-heres-why)).
  → **AI-native for us means: AI sits on the integration, master-data, process and permission
  layers.** Without these layers there is nothing reliable for the AI to work on. That is why §2
  comes before §3.
- **HR integrations break on org structure and effective dates:** Workday and SuccessFactors
  model workers, positions and organisations as **effective-dated** objects. Supervisory orgs
  don't map 1:1 between systems, mapping tables are costly to maintain, and ignoring effective
  dating causes duplicate or missing records
  ([Workday API guide](https://unified.to/blog/workday_api_integration_what_to_know_before_you_build),
  [SF→Workday guide](https://clonepartner.com/blog/successfactors-to-workday-migration-the-ctos-technical-guide/),
  [Workday–SAP playbook](https://knowledgelib.io/business/erp-integration/workday-sap-playbook/2026),
  [CloudApper](https://www.cloudapper.ai/?p=173320)).
- **Normalisation has limits:** unified-API vendors keep the **raw source enum** next to the
  normalised value (Merge's `show_enum_origins`) and offer **passthrough** for fields outside
  their model. Finch openly limits what it normalises
  ([Merge enum origins](https://help.merge.dev/en/articles/8950958-show_enum_origins),
  [Finch data modeling](https://www.tryfinch.com/blog/under-the-hood-opinionated-data-modeling-case-study),
  [Finch request forwarding](https://developer.tryfinch.com/developer-resources/Request-Forwarding.md)).

### 1.3 Gap analysis of our design (before this revision)

| Enterprise requirement | Real-world complexity | Doc 03–05 coverage | Gap → fixed in |
|---|---|---|---|
| Org structure | 10k–100k people, frequent reorgs, matrix and dotted lines, future-dated changes, synced from HRIS | "User/Group field", departments mentioned | **Missing model** → §2.1 |
| Permissions | Data scope by org ("my dept + sub-depts"), separation of duties, delegated admins, temporary access, "why can X see this?" | Roles, row/field rules | **Partial** → §2.2 |
| Multi-tenancy | Tenant = company, plus subsidiaries/legal entities inside it, external partners | DB-per-tenant | Entities and guests missing → §2.3 |
| Approvals | Sequential, countersign-all, any-one, quorum, conditional by amount/category, DoA matrix, add-approver, return-to-step, delegation, withdrawal, timeouts, approver leaves mid-flight | "Approvals with thresholds + field locking" | **Under-specified** → §2.4 |
| API integration | Same concept, different encodings ("gender" = `1/2/0/9`, `M/F/U`, `true/false`, `男/女`, `männlich/weiblich/divers`), IDs, effective dating, schema drift, bidirectional loops | REST API + webhooks + importer | **Missing mapping layer** → §2.5 |
| Forms / data entry | Periodic collection by org unit, deadlines, missing-submission chasing, prefill, period lock | Form view | **Campaigns missing** → §2.6 |
| Reports | Period close, as-of numbers, certified metrics, Excel output, scheduled delivery | Dashboards w/ drill-down | **Governance missing** → §2.7 |
| Task tracking | Tasks across bases, workload/capacity, recurring tasks, handoffs, tasks from mentions/approvals/AI | Presets + loop primitives | Cross-base & workload → §2.8 |
| Import / export | Monthly vendor files, legacy Excel migration, locale formats/encodings (Shift_JIS, EUC-KR), upsert, error files, rollback; formatted Excel out, PII-controlled exports, full data portability | CSV/XLSX import (dry run, upsert) + export | Templates, rollback, governance, scheduled exports → §2.9 |
| Reports | Pivot, period comparison, per-recipient scheduled delivery, alerts → actions, certified numbers | Dashboards w/ drill-down | Full report feature set → §2.10 |

---

## 2. Revised enterprise foundations

### 2.1 Organisation model (large, dynamic, effective-dated)

Org data lives in the **tenant DB as system tables**, synced from the HRIS/IdP when present.
It is exposed to users as normal read-only tables, so it can be linked, looked up, filtered and charted.

| Entity | Key attributes |
|---|---|
| **Person** | Stable ID, display name (with name order per locale), status (active/leave/terminated), external IDs (via xref) |
| **Account** | Login identity (password/passkey/SSO), linked to one Person (agents have no Person) |
| **Org unit** | Type (legal entity / business unit / department / team / cost centre / location), parent, head (position or person), **valid_from / valid_to** |
| **Position** (optional) | Job, org unit, incumbent(s), reports-to position. Supports position-managed HR systems |
| **Assignment** | Person ↔ org unit ↔ position, primary/secondary, **solid-line manager**, **dotted-line manager(s)**, valid_from / valid_to |
| **Group** | Static (members listed) or **dynamic** (rule: "all people in Sales with title contains Manager") |
| **Role** | Permission bundle (§2.2), granted to persons, groups, org units or agents |

Design rules:
1. **Effective dating everywhere.** Every org fact has a validity interval. Queries are
   "as of" a date, defaulting to now. **Future-dated changes** (a reorg on the 1st of next month)
   are stored ahead and take effect automatically, with an impact preview (§3 job 8).
2. **Fast hierarchy queries at scale** use a **closure table** `org_closure(ancestor, descendant, depth)`
   maintained incrementally on change. "Department and all sub-departments" is a single indexed
   lookup. Size is in §5.6.
3. **History is never rewritten by reorgs.** Business records store the **org reference (stable ID)**
   *and*, where it matters, a **snapshot** at event time. For example, the requester's department
   is captured when a request is submitted, so approvals and reports stay correct after a reorg.
   Each field chooses: *live* (follows current org) or *as-of-event* (snapshot).
4. **Source of truth is per attribute.** When synced from an HRIS, synced attributes are read-only
   locally. Local-only attributes (e.g., project roles) remain editable. Conflicts become exceptions.
5. **Org-change ripple handling** (closes the loop):
   - In-flight approvals whose resolved approver changed or left are re-resolved by rule
     (manager of departed approver → admin), and the change is logged.
   - **Orphaned ownership** (records, assignments, tasks, assets of a departed person) creates a
     transfer task to the person's manager.
   - Permission data scopes are re-evaluated (cached scope sets invalidated).
   - Dynamic groups are recomputed.
6. **Sync sources:** SCIM 2.0 (Okta, Entra ID) for accounts and groups; HRIS connectors via the
   mapping layer (§2.5) for persons, units, positions and assignments.

### 2.2 Permissions (enterprise grade)

Model = **RBAC + data scope + field rules + action rules + constraints**, evaluated by `mt-perm`
for UI, API, export, search, notifications, AI and agents alike.

| Element | Detail |
|---|---|
| Role grants | To person / group / org unit / agent; optional **expiry** (temporary access), with reason and approver |
| **Data scope** (rows) | `self`, `my direct reports`, `my reporting line (all levels)`, `my unit`, `my unit + descendants`, `units I head`, `cost centres I own`, `legal entity X`, `all`, or a custom filter; all evaluated through the closure table and assignments, as-of now |
| Field rules | Hidden / read / edit per field per role; fields classified PII are hidden by default for roles without the PII grant |
| Action rules | Create, delete, export, share, bulk edit, run workflow, manage schema, use AI on table |
| **Separation of duties (SoD)** | Declarative constraints, e.g. *requester ≠ approver*, *creator ≠ verifier*, *the same person cannot approve two consecutive levels*, *maker-checker for payments*. Violations are blocked at plan time and at action time |
| Delegated administration | Department admins manage roles only within their subtree |
| **Explain** | `explain(principal, action, resource)` returns the grant chain ("Alice → role Regional Manager via group APAC Sales → scope unit Japan + descendants → record unit = Osaka") and powers "view as", access reviews and support |
| Access reviews | Periodic campaigns: managers confirm or revoke their reports' grants, with an audited result |

### 2.3 Multi-tenancy, refined

- **Tenant = customer company** (database per tenant, doc 04). Inside a tenant, **legal entities and
  subsidiaries are org units**. Isolation between subsidiaries comes from data scopes, so group-level
  roles can still consolidate.
- **External collaborators** (suppliers, agencies, candidates via forms) are **guest principals**
  with record-level scope only. They never see the org directory.
- Per-tenant settings: region, fiscal calendar, business calendars (holidays per country: US/DE/JP/KR/IN),
  default locale, code-set overrides, AI provider policy.

### 2.4 Approval engine (complex, real-world)

**Definition** = a versioned graph bound to a table (or to a lifecycle transition):

| Node type | Semantics |
|---|---|
| Approval step | Approver set + **mode**: *sequential* (依次, in order), *countersign* (会签, all must approve), *any-one* (或签, first decision wins), *quorum* (N of M or %), *vote* (majority with tie rule) |
| Condition branch | On record fields (amount bands, category), requester attributes (unit, grade, entity), computed values |
| Parallel split / join | Independent reviews (legal + finance), join = all / any / custom; a named **resolver** decides conflicting outcomes |
| Automatic step | System action (update fields, call webhook) or **AI pre-check** (§3 job 4), which produces findings, never decisions |
| CC / notify | Informational recipients |
| Sub-process | Reusable fragments (e.g., "Finance review") |

**Approver resolution** (deterministic, as-of submission time):
- Specific person or role; **requester's manager chain**, N levels up or *until the authority
  covers the amount*; **head of the record's unit** (not the requester's); position; cost-centre
  owner; person field on the record.
- **Delegation-of-Authority (DoA) matrix**: a normal table (*category × entity × amount band →
  approver role/level*). It is versioned and itself approved. The matrix is a data asset that
  finance owns, not configuration buried in a workflow.

**Runtime actions:** approve, reject, **return** (to submitter or a specific earlier step),
**add approver** (before/after current), transfer, **delegate** (planned absence with a date range;
applies automatically), withdraw (by submitter, before the final decision), urge/remind, comment,
attach evidence.

**Policies:**
- Same approver appearing consecutively → auto-skip (configurable).
- Self-approval blocked (SoD).
- Timeout → remind, escalate, or auto-action.
- **Approver left** → re-resolve and log.
- Requester left → route to their manager.
- **Locking:** fields in the approval scope are frozen while pending and after approval. Editing them
  restarts approval, depending on configuration.
- **Versioning:** in-flight instances keep their definition version. A change to the definition
  never alters running instances.

**Evidence:** each action goes into the hash-chained audit with the org snapshot (who, in which
role, and why they were the approver). The approval instance is a process instance in the trace system.

**Testing:** the engine is a pure state machine in `mt-lifecycle`. **Model-based testing** generates
all action sequences up to depth N for each node mode and checks invariants: exactly one terminal
outcome, no approval by an ineligible principal, SoD never violated, locks consistent.

**External approval systems** (Lark Approval, SAP, Workday business processes): connector-based
status sync. Our record mirrors the external instance ID and state, so loops close across systems.

### 2.5 Integration & mapping layer (the "gender" problem)

The same business concept arrives in many encodings. Example: **sex/gender**.

| Source | Encoding |
|---|---|
| HRIS A | ISO/IEC 5218 style `0 / 1 / 2 / 9` (not known / male / female / not applicable). The standard covers **sex**, not gender, and the numbers carry no order or meaning ([ISO/IEC 5218](https://en.wikipedia.org/wiki/ISO/IEC_5218)) |
| ATS B | `"M" / "F" / "U"` |
| System C | Boolean `isMale = true/false` (cannot express other values, so mapping is lossy) |
| JP/KR systems | Labels `男性 / 女性`, `남 / 여` |
| German systems | `männlich / weiblich / divers` (Germany recognises a third option) |
| Indian HR forms | Often include a transgender/other category |
| Unified APIs | A normalised enum *plus* the raw origin value |

**Design: five building blocks**

1. **Code sets (first-class data assets).** A code set is a governed table of canonical values,
   with stable codes, locale labels, validity dates (deprecations) and **aliases per external
   system**. Select fields bind to a code set instead of free options. Standard code sets ship
   built in: ISO 3166 countries, ISO 4217 currencies, ISO 639 languages, ISO/IEC 5218 sex,
   E.164 phone parsing, employment status, and others. Tenants can extend them. **Sex** and
   **gender identity** are separate code sets, because they are different concepts.
2. **Mapping spec (declarative, versioned, tested).** One spec per connector and object maps
   external fields to our fields, with transforms (type coercion, date formats and time zones,
   locale numbers, delimiter splitting, unit conversion), **value maps referencing code sets**,
   defaults, and **embedded example tests** that run when the spec is saved and in CI:

   ```yaml
   mapping: ats_b.candidate -> Recruiting.Candidates   # version 7, owner: hr-integration
   key: { external: id, xref_system: ats_b }
   fields:
     full_name:      { from: [first_name, last_name], transform: person_name(locale_from: country) }
     email:          { from: email, transform: lowercase_trim, validate: email }
     applied_at:     { from: created_at, transform: datetime(format: rfc3339, tz: UTC) }
     sex:            { from: gender, codeset: iso5218_sex,
                       values: { "M": "1", "F": "2", "U": "0", null: "0" },
                       on_unmapped: quarantine }          # never silently drop or guess
     source_raw:     { from: "*", store: raw_payload }   # keep the original for audit/debug
   examples:
     - in:  { id: "c-9", first_name: "Aiko", last_name: "Sato", gender: "F", country: "JP" }
       out: { full_name: "Sato Aiko", sex: "2" }
   ```
3. **Cross-reference (xref) & identity resolution.** `xref(system, object, external_id → record_id,
   valid_from/to)` gives idempotent upserts, de-duplication and resolution of foreign keys (e.g., an
   ATS `job_id` resolves to our Job record). Matching rules (exact keys, then fuzzy candidates for
   human review) handle records that lack shared IDs.
4. **Sync runtime with closed-loop guarantees:**
   - Connectors handle auth, pagination, rate limits, and incremental cursors or webhooks.
   - Each run is a process instance in the trace system with a **run ledger**: fetched / created /
     updated / unchanged / quarantined / failed counts.
   - **Reconciliation** compares counts and hashes against the source, and any mismatch raises an
     exception.
   - **Quarantine queue:** unmapped values, validation failures and ambiguous matches land there. Each
     one is an exception with an owner (the integration steward), a one-click "add mapping", and a replay.
   - **Schema drift detection:** new, removed or retyped external fields raise an exception before data breaks.
   - **Bidirectional sync:** **field-level source of truth** plus echo suppression (change origin
     tags) prevents ping-pong loops. Conflicts follow a policy (source wins / latest wins / manual).
   - Effective-dated objects (org, assignments, compensation) are synced as intervals, never as overwrites.
5. **Raw retention & passthrough.** The raw payload is kept (subject to retention and PII rules) next
   to the normalised values, as unified-API vendors do. Unmapped source fields stay reachable as
   **passthrough attributes**, so nothing is lost while mappings evolve.

Connectors for v1: generic REST/JSON (configurable), CSV/SFTP drops, webhooks inbound, SCIM 2.0,
and our own API. Named HRIS/ATS connectors come later via the same framework. A unified-API
provider (Merge, Finch and similar) can also be used as a source.

### 2.6 Forms & operational data entry (collection campaigns)

The "25 stores report weekly" pattern becomes a first-class feature:

| Element | Behaviour |
|---|---|
| **Collection campaign** | Form + target table + **assignees per org unit or person** + **period schedule** (weekly/monthly/fiscal) + deadline + reminder policy |
| Prefill | Last period's values, unit attributes, the assignee's own data |
| Validation | Field rules, cross-field rules (e.g., returns ≤ sales), anomaly hints vs. last period (§3 job 6) |
| Drafts & resubmission | Draft save; resubmission after the deadline needs approval |
| **Tracking** | Submission matrix (units × periods: submitted / late / missing / returned), auto-reminders, **missing submission → exception** for the unit head |
| **Period lock** | After close, entries are frozen; corrections go through an adjustment approval |
| Scope | Each unit sees and edits only its own rows (data scope). HQ sees everything, consolidated |
| Channels | Web, mobile web, email link, and later IM bots; photo capture with OCR prefill (§3 job 5) |

### 2.7 Reports (operational & governed)

- **Metric definitions (semantic layer):** named, owned, versioned **metrics** (e.g., *Return rate =
  returns / gross sales, by fiscal month*) with dimensions and filters. They are approved like
  formulas. Dashboards, scheduled reports and **AI answers all use the same certified metrics**.
  This is also what makes AI Q&A accurate (§4).
- **Period close & as-of reporting:** reports can be pinned to a closed period. Numbers are
  reproduced from history and the journal (as-of queries), so a report from last quarter shows the
  same numbers next year.
- Report types: lists, grouped summaries, **pivot**, period-over-period comparison, fiscal calendars,
  top-N, and exception reports (SLA breaches, missing submissions, quarantined integration records).
- Delivery: scheduled email/IM delivery, **Excel export with formatting**, PDF. Every chart drills
  down to records.
- **Certification badge:** governed reports vs. ad-hoc views, so users know which numbers are official.

### 2.8 Task tracking (enterprise)

- Tasks can attach to **any record** (ticket, approval, project, asset, integration exception).
  "My tasks" aggregates across all bases in the user dashboard.
- Sources: manual, from @mention, from approval steps, from lifecycle assignments, from collection
  campaigns, from integration exceptions, and from AI suggestions (which need confirmation).
- Recurring tasks (cron or fiscal calendar), checklists, dependencies (blocked-by), handoffs between
  teams with acknowledgement, SLA and escalation along the org chain.
- **Workload view:** open tasks and estimates per person or team vs. capacity, with leave
  (from the HRIS) taken into account.
- Kanban / task list presets (doc 05 §1.4) are views over this model.

### 2.9 Import & export (enterprise grade)

Import and export are where spreadsheet data enters and leaves the governed system, so they get the
same mapping, validation and audit machinery as integrations (§2.5), not a separate one.

**Import**

| Capability | Detail |
|---|---|
| Formats | CSV/TSV (encoding detection: UTF-8/UTF-16/**Shift_JIS**/**EUC-KR**/Windows-1252; delimiter detection), **XLSX** (multiple sheets, merged headers, header row detection, date serials, formulas imported as values with an option to keep the formula text), JSON/NDJSON, Lark Base / Airtable (via API, W10), Google Sheets (L) |
| Modes | Create new table; **append**; **upsert by key** (one or more fields, or xref); **update only**; **replace** (with snapshot first) |
| Mapping | Column → field mapping with saved, reusable **import templates** (the monthly vendor file mapped the same way every time); value maps to code sets; person/org resolution by email/employee ID/name; link resolution by primary field or key; unit/locale-aware parsing (`1.234,56` in DE, `12,34,567` in IN, `2026/11/12` in JP) |
| **Dry run** | Preview: rows to create / update / unchanged / rejected, per-field type coercion results, and a **validation report** (row, column, value, reason) before anything is written |
| Partial failure policy | All-or-nothing (atomic) or import valid rows + downloadable **error file** (original rows + error column) to fix and re-import |
| Scale | Streaming parse (constant memory), batched writes, progress, cancel; target 1M rows × 30 cols in minutes **[model, S9]** |
| Safety | Respects permissions (field rules, data scope), lifecycle guards, uniqueness; formula injection protection (`=`, `+`, `-`, `@` prefixes treated as text unless the mapping says formula) |
| Audit | Each import is a job and a process instance: who, file hash, template version, counts; every created/changed cell carries `Source::Import{job}`; **one-click rollback** of an import job (via journal) |
| AI assist | §3 job 3 applied to files: proposes column mappings and value maps from the file sample |

**Export**

| Capability | Detail |
|---|---|
| Formats | CSV (UTF-8 with optional BOM for Excel; locale delimiter), **XLSX with formatting** (number/date formats, column widths, frozen header, select colours, multiple sheets for linked tables), JSON/NDJSON, **Parquet** (analytics, L), PDF (record/report) |
| Scope | Current view (filters, sort, hidden fields respected), selection, whole table, whole base (ZIP incl. attachments, schema JSON), **full tenant export** (data portability, DSAR, migration off our product — required by GDPR portability and good open-source practice) |
| Fidelity options | Display values vs raw values vs both; code-set codes vs labels (per locale); person as name/email/ID; links as primary values or IDs |
| Governance | Export is an **action permission**; fields classified PII are excluded or masked unless the role holds the PII grant; tenant policy can require approval for exports above N rows or containing PII; optional watermark (user/time) on XLSX/PDF |
| Audit | Every export logged (who, scope, row count, fields, file hash) in the hash-chained audit; download links are short-lived and single-user |
| Scheduled export | To email, S3/SFTP, or webhook (feeds data warehouses) using the same job framework |
| Scale | Streaming generation (constant memory); large exports run as background jobs with notification |

### 2.10 Reports — complete feature set (extends §2.7)

| Area | Features |
|---|---|
| Report types | Tabular list, grouped summary with subtotals, **pivot / cross-tab**, period-over-period (MoM, YoY, vs. budget/target), ranking/top-N, distribution, **exception reports** (SLA breaches, missing submissions, quarantined integration rows, pending approvals by age) |
| Charts | KPI cards with target and trend, bar/line/area/combo, pie/donut, funnel, scatter, heatmap, gauge, table; colour by series following the theme (light/dark) |
| Data sources | One table or **multiple tables via links** (e.g., orders × customers × regions); certified metrics (§2.7); org dimensions (unit hierarchy roll-up via closure table); fiscal calendars |
| Interactivity | Global filters (period, unit, category), **drill-down to records** (always), drill-through to another report, cross-filtering between charts |
| Layout | Report pages composed of blocks (charts, tables, text, filters); print/PDF layout; mobile layout |
| Governance | Owner, **certified** badge, metric versions shown, "numbers as of" timestamp, **period close** pinning, data-scope aware (each viewer sees only permitted rows; consolidated roll-ups respect scope) |
| Distribution | Scheduled delivery (email/IM) as PDF/XLSX/link, **per-recipient filtered** (each store manager gets their store), subscriptions, alert rules ("notify me when return rate > 8%") that create **exceptions/tasks**, closing the loop from number to action |
| Performance | Aggregates from the columnar cache (§5.5), cached per (report, filter, scope) with invalidation by journal sequence; target first render < 1 s at 1M rows **[model]** |
| AI | Narrative summary and governed Q&A via the bounded planner on certified metrics (§3 job 7, §4) |

---

## 3. AI in business operations: ten concrete jobs

Every job follows the same contract: **AI proposes → deterministic core validates → a human (or a
policy) confirms → the result is committed with provenance → a metric is tracked.**

| # | Job | Process | Input → AI output | Deterministic guardrails | Human checkpoint | Metric |
|---|---|---|---|---|---|---|
| 1 | **Intake classification & routing** | Tickets, customer-service chats, requests | Message/form → category, priority, language, sentiment + confidence | Output restricted to code-set values; routing rule maps category → team; low confidence → human triage queue | Triage owner confirms when confidence is below the threshold | Routing accuracy, time-to-assign |
| 2 | **Document extraction** | Contracts, invoices, receipts, IDs, resumes | Attachment → typed fields + **evidence spans** (page/coordinates) | Type validation, code-set validation, cross-checks (invoice total = sum of lines; PO match) | Reviewer accepts per field (suggested mode) | Field-level accuracy, review time |
| 3 | **Integration mapping assistant** | Any new connector | Sample payloads + target schema → proposed field mappings and **value maps** (e.g., `"F"→2`) with confidence | Mapping spec is compiled, and its example tests must pass | Integration steward approves the spec version | Time to integrate, quarantine rate |
| 4 | **Approval pre-check** | Purchase, expense, contract, hiring approvals | Record + attachments + policy text → findings ("amount exceeds budget line", "missing quote #2", "vendor not on approved list") | Policy checks that *can* be deterministic (DoA, budget) run as rules; AI covers document/policy reading | Approver sees findings; **AI never approves** | Rework rate, approval cycle time |
| 5 | **Assisted data entry** | Collection campaigns, field reports, receipts | Photo/voice/free text → prefilled form | Same validation as manual entry | Submitter confirms | Entry time, error rate |
| 6 | **Data quality steward** | All governed tables | Duplicates, anomalies (vs. last period), missing owners, suspicious values → exceptions | Exceptions use deterministic rules where possible; AI adds fuzzy matches | Data steward resolves | Quality score trend |
| 7 | **Governed Q&A & report narrative** | Management reporting | Question → **bounded plan over certified metrics** (§4) → numbers computed by the engine → narrative citing records/metrics | Model never computes numbers; it only selects metric, dimension, period and filters | Reader sees the plan and its sources | Answer acceptance rate |
| 8 | **Org-change impact analysis** | Reorgs, departures | Planned org change → affected approvals, permissions, assignments, campaigns, owned assets (computed deterministically) + AI summary and suggested reassignments | Impact is computed by the engine | HR/admin approves the reassignment batch | Orphaned items after the change = 0 |
| 9 | **Follow-up drafting** | Lifecycle steps (resolved → notify requester), overdue chasers | Record context → draft message in the recipient's locale | Template constraints, PII policy | Sender edits/approves (or a policy auto-sends for low-risk templates) | Response time |
| 10 | **Recruiting support (restricted)** | Hiring pipeline | Resume → extracted facts (skills, experience, contact) and summary | **No automated ranking or rejection by default.** Employment AI is **high-risk under the EU AI Act** (Annex III; obligations phasing in from Aug 2026, timing under EU review **[verify]**), and NYC Local Law 144 requires **bias audits** for automated employment decision tools | Recruiter makes and records every decision | Time-to-screen; audit completeness |

**AI prerequisites, built into the foundations:** code sets (job 1, 3), org model (8), approval
engine (4), certified metrics (7), provenance & review state (all), data scopes (all; AI only
sees what the acting principal may see), field classification (PII never goes to a disallowed
provider). These are exactly the layers whose absence makes enterprise AI pilots fail (§1.2).

---

## 4. From "intent compiler" to a **bounded planner** (Q2)

### 4.1 Evidence

- Adding a **semantic layer** (governed business definitions) raises LLM accuracy substantially. In
  Cube's public benchmark, three frontier models rose from ~46–51% to ~68% with a 4 KB
  semantic-layer document, and **context mattered more than which model was used**
  ([cube benchmark](https://github.com/cubedevinc/semantic-layer-benchmark)). Vendor reports show
  larger gains (dbt, AtScale, Snowflake) ([summary](https://medium.com/@kalingwu/text-to-sql-accuracy-semantic-layer-research-f39f62b952ba),
  [AtScale](https://www.atscale.com/blog/semantic-layer-text-to-sql-accuracy/)).
- Academic benchmarks overstate real-world accuracy: one agent scored **91.2% on an academic set
  but 21.3% on an enterprise-realistic set** (Spider 2.0) ([same summary](https://medium.com/@kalingwu/text-to-sql-accuracy-semantic-layer-research-f39f62b952ba)).
- **Semantic Path Compilation** (Aug 2026) has the LLM **ground phrases and choose among
  question-specific governed options**, while traversal, role predicates, SQL construction and
  checks run **in code**. The authors caution it is a single-domain systems result
  ([arXiv 2608.16663](https://arxiv.org/pdf/2608.16663), [SemPlan benchmark](https://arxiv.org/pdf/2608.13612)).
  A related result: a **rule-gated 7B agent beat a direct-prompted 32B baseline** on business
  correctness ([arXiv 2608.09254](https://arxiv.org/pdf/2608.09254)).

**Conclusion:** an open "NL → arbitrary plan" compiler is unreliable. **A bounded planner, where
the LLM selects and code compiles, is feasible**, and our kernel already contains the needed parts
(typed ASTs, permission engine, certified metrics, code sets).

### 4.2 Design: four steps, three of them deterministic

```
User text ──► 1 GROUND (deterministic + retrieval)  ──► candidate catalogue
              names/synonyms/code-set labels/org units/people/metrics/fields,
              filtered by the user's permissions
          ──► 2 SELECT (LLM, constrained)            ──► filled plan template
              choose ONE of ~12 plan templates; fill slots ONLY with candidate IDs
              (JSON schema built per request with enums of allowed IDs → invalid refs impossible)
          ──► 3 COMPILE & CHECK (Rust)                ──► FilterAst / OpBatch / MetricQuery
              type check, permission check, SoD, cardinality ("affects 12,480 rows"),
              lifecycle guards, dry-run diff
          ──► 4 CONFIRM & EXECUTE                      ──► normal op pipeline, provenance=AI-assisted
Out of scope / ambiguous → ask a clarifying question with the candidates; never guess.
```

**Plan templates (v1 = 6; extended by evals):**

| Template | Slots (all enumerated IDs) | Compiles to |
|---|---|---|
| `find_records` | table, conditions[(field, op, value∈code set/org/person/date range)], sort | `FilterAst` view |
| `metric_query` | certified metric, dimensions[], period (fiscal-aware), filters[] | `MetricQuery` |
| `bulk_update` | table, selection (FilterAst), field, new value | `OpBatch` (+ approval if over threshold) |
| `create_record` | table, field values | `Op::Create` |
| `explain_trace` | record | Trace query |
| `assign_or_route` | selection, assignee (person/unit/role) | Assignment ops |

Example: *"How many purchase requests from Osaka over ¥500,000 are waiting for approval this month?"*
- **Ground:** `Osaka` → org unit `ou_812` (+ descendants); `purchase requests` → table `tbl_pr`;
  `over ¥500,000` → field `fld_amount`, currency JPY; `waiting for approval` → lifecycle state
  `st_pending_approval`; `this month` → current fiscal month of the tenant's calendar.
- **Select:** `{template: "find_records", table: "tbl_pr", conditions: [["fld_unit","in_subtree","ou_812"], ["fld_amount",">","500000 JPY"], ["state","=","st_pending_approval"], ["fld_submitted","within","fiscal_month:current"]], aggregate: "count"}`
- **Compile & check:** valid IDs, user's data scope applied, result = count.
- **Answer:** "37 requests (view)", with a link to the generated filtered view.

### 4.3 Acceptance & spike (S8)
- Build an evaluation set of **300 questions** over three realistic sample bases (requests,
  purchasing, HR onboarding) in English, with JA/KO/DE phrasings later. Every question has a
  gold plan.
- **Ship criteria per template:** ≥ 95% *safe* (no wrong executed action: either correct or asks or
  refuses) and ≥ 85% *correct-first-time* **[targets to confirm]**. Below that, the template stays in
  suggest-only mode.
- Property tests: for **any** model output, steps 3–4 either produce a valid, permitted plan or
  reject it. This holds by construction, because slot values come from enumerations.

---

## 5. Capacity & performance estimation model (Q3)

The method: **bytes per row = Σ over fields of bytes(field type, average cardinality, average length)
+ fixed system overhead**, computed separately for each tier. All constants are **[model]** and are
calibrated by spike S9 on SQLite/MySQL/PG and in WASM.

### 5.1 Server in-memory column cache (Rust, columnar, loaded per column on demand)

| Field type | In-memory representation | Bytes per row |
|---|---|---|
| Record ID + row index | `u64` + hash map entry | ~32 |
| System (version, created/modified at/by) | u64 + i64 ×2 + u32 ×2 | ~32 |
| Checkbox | bitmap | 0.125 |
| Null bitmap (every column) | bitmap | 0.125 per column |
| Number / currency / percent | **scaled `i64`** with a per-field scale (exact); overflow falls back to `Decimal` (16 B) | 8 |
| Date / datetime | `i64` epoch ms | 8 |
| Single select / code set | `u16` dictionary index | 2 |
| Person / org unit ref | `u32` interned ID | 4 |
| Multi select (avg k values) | `u32` offset + k × `u16` | 4 + 2k |
| Link (avg k targets) | CSR: `u32` offset + k × `u32` (reverse index on the target side, same size) | 4 + 4k |
| Short text (avg L bytes) | `u32` offset + UTF-8 arena | 4 + L |
| Sort rank for a sorted text field | `u32` rank | 4 |
| Long text / attachments | **not cached** (fetched per window) | 0 |

**Worked example:** 1,000,000 rows; fields: 6 short text (L=24), 2 long text, 6 number, 4 date,
4 single select, 2 multi select (k=2), 3 person, 2 link (k=3), 1 checkbox.
- All cacheable columns: 32 + 32 + 6×28 + 6×8 + 4×8 + 4×2 + 2×8 + 3×4 + 2×16 + ~4 (bitmaps) ≈ **384 B/row → ~384 MB**.
- **Typical working set** (only columns used by the open views' filters/sorts/groups, formulas
  and dashboards, e.g., 2 text, 3 number, 2 date, 3 select, 1 person, 1 link):
  32 + 32 + 56 + 24 + 16 + 6 + 4 + 16 + ~2 ≈ **188 B/row → ~188 MB**.
- Policy: per-tenant cache budget (default 512 MB in Docker, configurable), LRU by column, and the
  DB is used directly when a column isn't cached. Small tables (< 50k rows) are cached entirely.

### 5.2 Database storage (per row, before compression)

Same example, **[model]**:
- Short text: 6 × (24 value + 28 sort key) ≈ 312
- Long text: 2 × 400 ≈ 800
- Numbers: 6 × (8 text + 9 sort key) ≈ 102
- Dates: 4 × 6 ≈ 24
- Selects: 4 × 14 ≈ 56
- Multi-select JSON + `mv_` rows: 60 + 120
- Persons: 36
- Links (junction + 2 indexes): 240
- System + row overhead: ~80

Total ≈ **1.8 KB/row → ~1.8 GB per 1M rows**, plus ~250 B/row for 10 indexed sort keys
→ **~2.1 GB**. History grows ~150 B per edited cell; plan retention per tenant.

### 5.3 Client (browser) memory: flat, independent of table size
- Window = visible rows + 3 screens of prefetch ≈ 300 rows × 30 columns × ~40 B decoded ≈ **0.4 MB**.
- Text-measure cache capped at 10k entries ≈ 0.6 MB. Schema, views and theme < 1 MB.
- **Total grid data < 10 MB at 1k or 10M rows**. WASM heap is budgeted at ≤ 64 MB with headroom.

### 5.4 Wire payload per window
200 rows × 30 cols × ~12 B (columnar postcard, dictionary-encoded selects) ≈ 72 KB → **~25 KB brotli**.
At 10 Mbit/s that is ~20 ms of transfer, so the window fetch is dominated by query time (target p95 < 300 ms).

### 5.5 Formula / recompute cost model
- **Dependency graph memory is per field, not per cell** (≤ 400 nodes per table + cross-table edges),
  so it is negligible. Per-record fan-out comes from the link CSR that is already cached.
- **Cost of one edit:** `T ≈ Σ_dependent_fields N_affected × (t_eval + t_write / batch)`.
  - `t_eval` ≈ 0.2–2 µs per simple formula in Rust **[model]**.
  - Batched DB write ≈ 5–20 µs/row (SQLite/MySQL/PG differ; S9 measures).
  - Example: editing a "settings" record that 1M records look up costs ~1 s of eval and 5–20 s of
    writes. Hence:
    - **Fan-out warning at schema time** ("this lookup affects up to 1,000,000 records on every
      change of the source; consider computing on read").
    - **Compute-on-read** for high-fan-out lookups (served from the cache, not materialised).
    - **Synchronous** recompute only when N_affected ≤ 1,000 (< 50 ms); otherwise async with a
      progress indicator and "computing" cells.
- **Incremental aggregates:** sum, count and avg are updated in O(1) by delta. Min/max are
  recomputed only when the extreme value changes. Dashboard aggregates over cached columns: 1M ×
  8 B scan ≈ 8 MB → **a few ms**. Group-by on a `u16` dictionary uses a hash aggregate, ≈ 10 ms **[model]**.
- **Exactness:** scaled `i64` sums accumulate in `i128`, with no floating point anywhere.

### 5.6 Org structure & permissions
- 100k persons, 10k units, average depth 7 → closure table ≈ 70k rows (~3 MB); assignments
  ≈ 120k rows (~6 MB).
- **Per-user visible-unit set** as a bitset of 10k bits = 1.25 KB. Cached for 100k users that is
  ~125 MB worst case, so only active users are cached (LRU). Row-scope checks are bitset lookups (ns).

### 5.7 AI / semantic layer
- Embeddings (opt-in): 768 dims int8 ≈ 768 B/record + HNSW graph (M=16) ≈ 128 B → **~0.9 GB per 1M embedded records**.
- Record cards: ~300 B/record if materialised (they can be generated on the fly instead).

### 5.8 Product feature: capacity estimator
The same model ships in the product. While designing a table, admins see estimated cache, disk and
recompute cost at 10k / 100k / 1M rows. Warnings appear for fan-out lookups, wide tables, and
embedding costs. The constants are calibrated per DB engine by S9 and refined from real telemetry
on self-hosted instances (opt-in).

---

## 6. Revised plan

| Wave | Content (changes in **bold**) |
|---|---|
| W1 Foundations | Control plane, tenants (3 DBs), auth, core field types, provenance, **org model (effective-dated, closure table), code sets (built-in ISO sets)**, design tokens/themes, attachments |
| W2 Grid | Full grid catalogue, record pop-up, tabs, thumbnails |
| W3 Computation | Formulas, links/lookups/rollups, incremental graph, **fan-out analysis, compute-on-read, capacity estimator** |
| W4 Collaboration | Real-time, comments, notifications |
| W5 Views & forms | Kanban/calendar/gallery/Gantt, task presets, **collection campaigns** |
| W6 Permissions | **Data scopes via org, SoD constraints, delegated admin, temporary grants, explain, access reviews** |
| W7 Process | Lifecycles, assignments, SLA, exceptions, **full approval engine (modes, DoA matrix, return/add/delegate/withdraw, versioning, locking)**, workflows, trace-audit |
| **W8 Integration** (moved earlier) | **Connector framework, mapping specs with tests, value maps, xref & identity resolution, quarantine, reconciliation, drift detection, bidirectional sync, SCIM, generic REST/CSV/webhook connectors** |
| W9 Reports & data exchange | **Certified metrics, period close/as-of, pivot, multi-table reports, per-recipient scheduled delivery, alert rules → exceptions**, dashboards; **import (encodings, templates, upsert, dry run, error file, rollback) and export (formatted XLSX, PII governance, scheduled, full-tenant portability)** — basic CSV/XLSX import/export already in W2 |
| W10 Data assets & platform | Lineage, quality, DSAR, OpenAPI/SDKs, MCP, admin & user dashboards |
| W11 AI jobs | Model gateway; **jobs 1–9 from §3** (job 10 restricted mode); **bounded planner with 6 templates**; AI steward |

New spikes: **S8** bounded planner eval (300 questions); **S9** capacity model calibration (per-type
bytes in Rust/WASM, DB size and write throughput on SQLite/MySQL/PG); **S10** approval engine
model-based tests (state-space exploration).

## 7. Open questions
1. Which **HRIS/ATS/IdP** systems do your target customers use most (Workday, SAP SuccessFactors,
   BambooHR, HiBob, SmartHR (JP), Darwinbox (IN), Personio (DE), Greenhouse, Lever, Okta, Entra ID)?
   This decides which named connectors come first.
2. Do your existing customers need **position management**, or is person-to-unit assignment enough for v1?
3. Approval modes: are there any Lark/DingTalk-style behaviours your users rely on beyond §2.4?
4. Bounded planner targets (≥ 95% safe / ≥ 85% correct-first-time): acceptable?
5. Can you provide **one or two real (anonymised) business scenarios** with sample data? They would
   become the eval bases for S8 and the reference templates.

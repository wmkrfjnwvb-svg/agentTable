# Final Product Design (v1.0) — Domains, Processes, Data Assets, Core Capabilities

> **Status: authoritative product specification for development.** It consolidates docs 01–16. Where an
> earlier document conflicts with this one, this one wins. AI design is in **doc 18**; Lark capability
> parity, release scope and the engineering hand-off are in **doc 19**.
> Working name in this document: **"the platform"**.

---

## 0. Product summary

| Item | Decision |
|---|---|
| What it is | An open-source, AI-native multidimensional table platform for running business processes out of the box |
| Who it is for | Business teams and IT in companies of 20–5,000 people; developers and partners who build templates |
| Markets | US, Japan, South Korea, Germany, India; English first (ja/ko/de/hi prepared) |
| Delivery | Self-hosted Docker first (5-minute start); managed cloud later |
| Licence | Apache-2.0 |
| Core value | Bring scattered business data in (sources, forms, documents), run closed-loop processes on it with clear owners, approvals and AI help, and send results out as actions, reports and evidence |
| Design rules | (1) Scenario-anchored, built only from generic components (doc 13). (2) Data sources are authoritative and inbound-only; outbound is explicit and one-way (doc 09). (3) AI is in the kernel: two AI objects (Skill, Agent), explicit dual identity, provenance, reversible changes (docs 15, 18). (4) Every loop closes: owner, deadline, terminal state, exception on failure. (5) Business capabilities instead of job titles |
| Tech constraints (for reference) | Rust core shared by browser (WASM) and server; Leptos UI (no virtual DOM); SQLite, MySQL and PostgreSQL; one database per tenant; 98–100% coverage; exhaustive `match` (docs 04, 06, 07) |

---

## 1. Product architecture (business view)

```
INPUT                                PLATFORM (layers)                                   OUTPUT
Existing systems  ─┐   ┌──────────────────────────────────────────────────────────────┐   ┌─ Decisions & finished work
People (forms,     │   │ L6 Solutions      templates by role / industry (S1–S11)       │   ├─ Actions in other systems
 rounds, grid)     ├──►│ L5 AI             Skills · Agents · Assistant · Builder       │──►├─ Notifications (in-app, push,
Documents &        │   │ L4 Process        lifecycles · approvals · SLA · tasks ·      │   │   email, SMS, chat)
 messages          │   │                   rules · workflows · exceptions              │   ├─ Reports & dashboards
Rules & settings  ─┘   │ L3 Data           tables · fields · reference data ·         │   ├─ Data feeds & exports
                       │                   settings · records · views · formulas      │   └─ Evidence (audit, trace)
                       │ L2 Connections    sources (in) · destinations (out) · API/MCP │
                       │ L1 Trust          access · history · audit · privacy · admin  │
                       └──────────────────────────────────────────────────────────────┘
```

---

## 2. Domains (bounded contexts for development)

| # | Domain | Responsibility | Key objects | Depends on |
|---|---|---|---|---|
| D1 | **Identity & Tenancy** | Tenants, members, accounts, authentication, guests, API apps, agent principals | Tenant, Member, Account, Session, ApiToken, Guest, Principal | — |
| D2 | **Workspace** | Organising content | Workspace, Base, Folder, Table, App, Page, DocPage | D1 |
| D3 | **Schema** | Field definitions, field groups, field library, field-type registry, reference data, settings, numbering | Field, FieldGroup, FieldLibraryItem, FieldType, RefSet, RefItem, Parameter, Sequence | D2 |
| D4 | **Records** | Records, cells with provenance, line items, links, attachments, comments, journal/history, archive | Record, Cell, LineItem, Link, Attachment, Comment, JournalEntry | D3 |
| D5 | **Query & Views** | Views, filters/sorts/groups, search, saved and personal views | View, FilterAst, SearchIndex | D4 |
| D6 | **Computation** | Formulas, lookups, rollups, dependency graph, aggregates, certified metrics | FormulaDef, DepGraph, Metric | D3, D4 |
| D7 | **Process** | Lifecycles, assignments, SLA clocks, approvals, tasks/checklists, exceptions, collection rounds | Lifecycle, State, Transition, Assignment, SlaClock, ApprovalDef, ApprovalInstance, Task, Exception, CollectionRound | D4, D12 |
| D8 | **Automation** | Rules (When/If/Then), workflows, runs, schedules | Rule, Workflow, Run, StepRun, Schedule | D4, D7 |
| D9 | **Messaging** | Notification channels, templates, deliveries, preferences, threads, consent | Channel, Template, Delivery, Preference, Thread, Message, Consent | D1, D4 |
| D10 | **Integration** | Data sources, inbox, mappings, cross-references, quarantine, destinations, event subscriptions, change feed, import/export jobs, API, MCP | DataSource, SyncRun, Mapping, Xref, QuarantineItem, Destination, Subscription, Job | D3, D4 |
| D11 | **Analytics** | Dashboards, components, reports, schedules, subscriptions, alert rules | Dashboard, Component, Report, ReportSchedule | D5, D6 |
| D12 | **Access** | Capabilities, grants, scopes, capability sets, sharing, separation of duties, explain | Capability, Grant, Scope, CapabilitySet, ShareLink, SodRule | D1, D3 |
| D13 | **AI** | Providers, models, skills, agents, knowledge, runs, suggestions, budgets, evaluations | Provider, Model, Skill, SkillVersion, Agent, Knowledge, Tool, AiRun, Suggestion, Budget, EvalSet | all |
| D14 | **Templates** | Components, templates, overlays, change sets (staging), install/upgrade | Component, Template, TemplateVersion, Overlay, ChangeSet | D2–D13 |
| D15 | **Trust & Admin** | Audit (hash chain), trace, retention, data requests, security policies, usage, instance ops | AuditEntry, Trace, RetentionPolicy, DataRequest, SecurityPolicy, UsageRecord | all |

**Domain rules:**
- D4 Records is the only writer of business data. Every change, whether from a person, rule, workflow, integration or AI, enters as an **operation** through D4's pipeline: validate → capability check → apply → journal → events.
- D8 Automation owns all triggers (rules); other domains emit events and expose actions.
- D13 AI never writes directly. It produces operations or suggestions that D4 processes.

---

## 3. Data assets

### 3.1 Data asset principles
Every table is a **data asset** with:
- owner and steward;
- classification per field (Public / Internal / Confidential / Personal / Sensitive personal);
- retention policy;
- lineage (sources, links, formulas, AI skills that produce it, destinations that consume it);
- quality rules and score;
- consumers (views, dashboards, API apps, destinations).

Every **cell value** carries provenance: source (person, import, data source, rule/workflow, AI skill/agent), time,
operation ID and trace ID. For AI values it also records model, skill version, confidence, evidence and review state.

### 3.2 Platform metadata model (system objects and key fields)

> Types: `id` (prefixed stable ID), `ref<X>`, `json`, `enum`, `ts` (UTC timestamp), `money` (exact decimal + currency).
> Each tenant database holds D2–D15 objects; the control-plane database holds tenants, accounts and instance settings.

**D1 Identity**
| Object | Key fields |
|---|---|
| Tenant | id, name, region, locale_default, timezone_default, fiscal_year_start, status, plan_limits json, db_locator |
| Account | id, email, name_parts, locale, timezone, auth_methods (password/passkey/oidc/saml), mfa, status |
| Member | id, tenant, account, kind (employee/guest/service), status, joined_at |
| Principal | id, kind (member/guest/agent/api_app/rule_owner), display_name, owner (for agent/api_app) |
| ApiToken | id, principal, scopes[], expires_at, last_used_at, hash |

**D2 Workspace**
| Object | Key fields |
|---|---|
| Workspace | id, name, visibility, admins[] |
| Base | id, workspace, name, icon, colour, template_ref, template_version, overlay_id, timezone, status |
| Table | id, base, name, description, primary_field, **asset_meta** (owner, steward, classification_default, retention_policy, quality_rules), source (native / data source id), archive_enabled |
| App | id, name, navigation json, theme, pages[], capability_set_defaults |
| Page | id, app/base, kind (record/list/board/calendar/form/portal/dashboard/doc), layout json |

**D3 Schema**
| Object | Key fields |
|---|---|
| Field | id, table, name (i18n), **data_type**, **widget**, options_source (inline / refset+subtree+level / table query / parameter), format (prefix, suffix, decimals, mask, numbering pattern), validation (required, conditional, regex, range, unique scope, format id, cross-field), **role** (field role, doc 13 §3), classification, status (enabled/disabled/deprecated), group, library_item, help_text, ai_skill (if AI field), version |
| FieldGroup | id, table, name, order, fields[], visibility_condition, capability (view/edit) |
| FieldLibraryItem | id, name, definition json, version, used_by[] |
| FieldType (registry) | id, name, base_type, widget, validation, format, classification_default |
| RefSet | id, code, name, structure (flat/tree), scope (system/tenant/base), item_schema (custom attributes), owner, change_policy, version |
| RefItem | id, set, code, name (i18n), value, parent, sort, status, valid_from, valid_to, attributes json, aliases {system: value} |
| Parameter | key, type, schema, default, values by scope (tenant/base/user), sensitivity, owner, version |
| Sequence | id, pattern (`PO-{YYYY}-{seq:5}`), scope (table/entity/year), next_value, gap_free |

**D4 Records**
| Object | Key fields |
|---|---|
| Record | id, table, cells (per field), **state** (if lifecycle), version, created_at/by, modified_at/by, source_ref (data source + external id), archived, deleted_at |
| Cell provenance | record, field, source_kind, principal, initiator, op_id, trace_id, ai {skill_version, model, confidence, evidence[], review_state}, at |
| LineItem | id, parent_record, line_table, order, cells, totals contribution |
| Link | field, from_record, to_record, order |
| Attachment | id, record, field, name, mime, size, hash, storage_key, thumbnails, extracted_text_ref, exif_stripped |
| Comment | id, record, author, body, mentions[], thread, resolved, visibility (internal/external) |
| JournalEntry | seq, op_id, client_op_id, trace_id, principal, initiator, kind, payload, at |

**D7 Process**
| Object | Key fields |
|---|---|
| Lifecycle | id, table, states[] (name, **category**, colour, required_fields[], editable_fields[], owner_rule, sla_policy), transitions[] (from, to, capability, guard, approval_def, button_label) |
| Assignment | id, record, state, assignee (principal), assigned_at, acknowledged_at, due_at, status |
| SlaClock | id, record, policy, kind (response/resolution/custom), started_at, paused_ms, due_at, breached_at |
| ApprovalDef | id, name, version, graph json (nodes: approval/branch/parallel/fill-in/notify/action/wait/subflow/end), status |
| ApprovalInstance | id, def_version, record, requester, status, current_nodes[], started_at, ended_at, outcome |
| ApprovalTask | id, instance, node, assignee, delegated_from, action (approve/reject/return/add/transfer), comment, acted_at |
| Task | id, title, attached_to (any record), assignee, due, state, checklist[], parent, dependencies[], source (manual/template/rule/AI) |
| Exception | id, kind (unassigned/overdue/failed_action/unmapped_value/missing_submission/quality/ai_quality…), subject_ref, owner, status, opened_at, resolved_at |
| CollectionRound | id, form, target_table, assignees (units/people), period, deadline, submissions matrix |

**D8 Automation**
| Object | Key fields |
|---|---|
| Rule | id, kind (validation/reaction/alert), trigger json, conditions (FilterAst), actions[], policies (throttle, digest, quiet_hours, escalation), owner, status, version |
| Workflow | id, graph json (steps incl. AI steps), trigger (rule), run_as policy, version, status |
| Run | id, rule/workflow, trigger_event, initiator, status, started_at, ended_at, trace_id |
| StepRun | id, run, step, input, output, status, attempts, error |

**D9 Messaging**
| Object | Key fields |
|---|---|
| Channel | id, kind (in-app/web push/email/SMS/chat/webhook), provider config (secrets), market rules |
| Template | id, channel, locale, subject, body, variables, registration (e.g., DLT template id), category (transactional/marketing) |
| Delivery | id, template, recipient, channel, status (queued/sent/delivered/read/failed), attempts, cost, record_ref, trace_id |
| Preference | principal, category, channels[], quiet_hours |
| Consent | party, purpose, channel, text_version, given_at, withdrawn_at |

**D10 Integration**
| Object | Key fields |
|---|---|
| DataSource | id, kind (inbox/db/rest/file), target_table, schedule, credentials (secret), incremental_key, status, health |
| Mapping | id, source, version, field_maps[], value_maps (→ RefSet), transforms, examples[], key spec |
| Xref | system, object, external_id, record, valid_from/to |
| SyncRun | id, source, started/ended, counts (fetched, created, updated, unchanged, quarantined, failed), reconciliation result |
| QuarantineItem | id, run, raw_payload, reason, owner, status |
| Destination | id, kind (event subscription/HTTP action/feed/change feed), config, secret, status |
| Job | id, kind (import/export/batch AI/type change/recompute), status, progress, result files, initiator |

**D11 Analytics**: Dashboard (id, layout, filters, theme, permission mode), Component (kind, data sources, metric refs, settings),
Metric (id, name, definition, dimensions, owner, certified, version), Report (definition, schedule, recipients, per-recipient filter).

**D12 Access**: Capability (id, object, action, kind built-in/custom), Grant (principal, capability, scope expression, conditions,
valid_from/to, granted_by, reason), CapabilitySet (name, capabilities[]), SodRule (capability pairs, scope), ShareLink (target, mode, expiry).

**D14 Templates**: Template (id, version, components, variation points, markets, profiles, tests), Overlay (tenant changes as patch),
ChangeSet (id, items[], dependencies, status draft/testing/published/rolled_back, author incl. AI).

**D15 Trust**: AuditEntry (seq, prev_hash, hash, actor, initiator, action, object, details, at), Checkpoint (seq, hash, signature),
RetentionPolicy, DataRequest (party, kind access/erase/export, status), SecurityPolicy, UsageRecord.

### 3.3 Business data assets (from the templates, doc 14)
Shared business objects that templates create and other templates reuse:
- **Party** (customers, suppliers, candidates, employees, creators)
- **Catalogue items** (products, asset models, job profiles)
- **Business documents** (claims, PR/PO/receipts/invoices, returns, offers)
- **Cases**
- **Pipelines** (applications, leads, collaborations)
- **Agreements**
- **Assets**
- **Budgets**
- **Metrics snapshots**
- **Knowledge articles**

They are governed like any data asset (§3.1). Cross-template reuse (e.g., Recruitment → Employee lifecycle → Assets)
happens through shared components, not copies.

---

## 4. Product processes (end-to-end flows)

Each flow lists steps, the system behaviour and the closing condition.

| # | Process | Steps | Closing condition / evidence |
|---|---|---|---|
| P1 | **Install & first value** | `docker run` → setup wizard (admin, URL, optional SMTP & AI provider) → choose templates → sample data → guided checklist walks one record through the loop | First loop completed; checklist done |
| P2 | **Build a base** | From template / blank / import / **AI builder** (doc 18 §5) → fields (picker groups, doc 16 §3) → views → lifecycle → access → publish (via change set if the base is live) | Loop-health check passes (owners, SLA, terminal states, drill-downs) |
| P3 | **Connect a data source** | Choose kind (push inbox / database / REST / file) → sample → **AI mapping suggestion** → value maps to code lists → dry run → enable schedule | Sync run ledger; reconciliation OK; quarantine empty or owned |
| P4 | **Capture data** | Grid edit, form (prefill, record pickers, line items, attachments), collection round, inbox push, **AI capture** (paste, voice, document, email, chat) | Record saved with provenance; validation passed or flagged |
| P5 | **Work a record** | Rule assigns owner → owner acknowledges → works (comments, tasks, AI help) → state transitions (guards, required fields) → done/failed/cancelled | Terminal state; SLA met or breach logged |
| P6 | **Approve** | Transition requests approval → approver resolution (capability + scope + limit) → AI pre-check findings → approve / reject / return / add / delegate → fields locked → follow-up actions | Instance outcome; audit trail |
| P7 | **Automate & notify** | Rule fires → workflow steps (actions, AI steps, waits) → notifications via channels → delivery tracking | Run log complete; failures become exceptions |
| P8 | **Measure & act** | Certified metrics → dashboards → alert rules / anomaly detection → tasks/exceptions → drill-down to records | Alert resolved; metric back in range |
| P9 | **Control access** | Define capabilities (AI proposes from statuses/actions) → capability sets → grants with scope/limits → preview as user → periodic access review | Review completed; explain available for any access |
| P10 | **Change safely** | Changes to a live base go to a **change set** (staging) → test (sample data, simulation, template tests) → publish → rollback if needed | Published version; changelog |
| P11 | **Exchange data** | Import (templates, encodings, dry run, error file, rollback); export (formatted, PII rules); feeds; change feed | Job result; audit entry |
| P12 | **Install / upgrade templates** | Install template → adapt (overlay) → upgrade: three-way merge, conflicts shown, tests re-run | Upgrade report |
| P13 | **AI skill lifecycle** | Create (from catalogue or describe) → bind inputs/outputs → test on samples → publish → monitor accept rate → improve | Quality above threshold, or exception to owner |
| P14 | **Agent lifecycle** | Create → instructions, knowledge, tools, entry points, rules/schedules → capability grants & budget → test in staging → publish → monitor runs | Run logs; budget within limit |
| P15 | **Privacy & retention** | Consent capture → retention timers → deletion/anonymisation tasks → data requests (access/export/erase) | Request closed with evidence |

---

## 5. Core capabilities (functional specification)

Format: **features** · **rules** · **limits (v1 targets)** · **acceptance criteria**. Lark reference values are noted where useful (see doc 19 for the full parity matrix).

### C1 Tables, records & views
- **Features:**
  - **Bases and structure:** bases, tables, folders, doc pages.
  - **Grid:** frozen header/columns, resize/reorder/hide, row height, wrap, colour rules, fill colour, multi-level group with subtotals, filter (nested AND/OR, relative dates, "me"), multi-sort, search & find/replace, footer aggregates, selection summary, Excel-compatible copy/paste, fill handle, undo/redo, keyboard map.
  - **Other views:** kanban, calendar, timeline/Gantt, gallery, list. Personal, locked, protected and shared views.
  - **Records:** record page (custom layout: tabs, sections, related lists, buttons), follow/share record, comments with mentions, line items, attachments with previews, history & restore (Time Machine equivalent), archive tier (read-only, reportable), tabs (table/view/app-level), light/dark themes.
- **Rules:** views never store data. Every edit is an operation with provenance. Field type changes run as async jobs without a row ceiling.
- **Limits (targets):** 1M active rows per table + archive tier (Lark: 2M rows; 1M archive); 400 fields per table (Lark: 300); 200 views per table; 5 group levels (Lark: 3); 100 filter conditions; 200 concurrent editors per base; attachments 2 GB each.
- **Acceptance:**
  - 1M-row grid scrolls at ≥ 55 fps.
  - First paint < 1 s.
  - Edit commit p95 < 300 ms.
  - Identical sort/filter results on SQLite/MySQL/PostgreSQL.

### C2 Field framework
- **Features:** 8 picker groups (doc 16 §3) · ~35 field types · widget per type · options from reference sets with cascades · formats (prefix/suffix/mask/numbering) · validation library (email, phone, postal codes, tax IDs JP/KR/DE/IN/US with checksums, IBAN) · field groups (grid header, record page, forms) · field library · custom field types registry · status enabled/disabled/deprecated · field-level classification and permissions.
- **Acceptance:**
  - Disabling a field never deletes data.
  - Library updates propagate, or the version stays pinned.
  - Every validator has positive and negative test vectors.

### C3 Reference data & C4 Settings
- **Features:**
  - **Reference sets:** flat/tree; standard + custom attributes; tenant overlay (extend, relabel, disable); crosswalk aliases per external system; table and tree views; impact analysis; versions and approvals; built-in ISO sets.
  - **Settings:** typed scoped parameters (system → tenant → base → user), secrets, feature flags, `PARAM()`.
- **Acceptance:** writes to reference fields accept only enabled items valid on the effective date; parameter resolution explains its source.

### C5 Access
- **Features:** capabilities (built-in per object + custom from lifecycle transitions, field groups, buttons, API operations) · grants with scope (own, team, reference subtree, record set, org subtree if org module, all) · conditions (amount limits, entity) · validity · capability sets · guests & portals · share links · separation-of-duties rules · **explain** and **preview as user** · access reviews · effect on forms and dashboards (numbers follow viewer permission).
- **Rules:** one engine for UI, API, export, search, notifications, dashboards and AI. Agents are principals.
- **Acceptance:**
  - A permission matrix test suite runs per template.
  - No AI or API path can read hidden fields.
  - "Explain" returns the grant chain.

### C6 Process
- **Features:** lifecycles with state categories · transitions with guards, required fields and capability · assignment + acknowledge + escalation · SLA clocks with business calendars and pause · **approval designer** (doc 11 §4: approval modes sequential/all/any/quorum; priority branches with default; parallel; fill-in; CC; actions; waits; sub-flows; simulation; versions) · tasks & checklist templates (relative dates, conditions) · exceptions inbox · collection rounds · booking with conflict check · scorecards (blind until submitted) · inspections.
- **Acceptance:**
  - Model-based tests cover every approval mode and action sequence up to depth N.
  - Each transition is atomic.
  - Running approvals keep their version.

### C7 Forms & capture
- **Features:** form designer (sections, pages, conditional logic, calculated fields, line items, multi-table submit) · prefill (user, URL, last submission) · record pickers scoped by permission · attachments (camera, compression, EXIF strip) · signature · location · barcode · **AI capture**: paste, voice, document (doc 18 §4.3) · public/internal/guest modes · consent notices · status lookup portal (query page) · QR/embed/email distribution.
- **Acceptance:**
  - Forms validate with the same rules as the grid.
  - AI-filled values are marked as suggestions until confirmed.

### C8 Connections
- **Features:**
  - **Sources (inbound only):** push inbox (sync ≤ 500 records / async jobs; any JSON/CSV/XLSX shape), database (SQLite/MySQL/PostgreSQL), REST, files (SFTP/S3/email); mapping with value maps to reference sets, xref, quarantine, reconciliation, drift detection.
  - **Destinations (outbound only):** signed event subscriptions (incl. computed-field changes), workflow HTTP actions, scheduled feeds, change feed API.
  - **Exchange:** import/export (encodings incl. Shift_JIS/EUC-KR, templates, dry run, error file, rollback; formatted XLSX).
  - **Developer access:** REST API (OpenAPI 3.1), SDKs, CLI, MCP server.
  - **Base to base:** publish a base as a shared data source for other bases.
- **Acceptance:**
  - Time to first sync is ≤ 10 min for push/database.
  - No silent data loss: every record lands as created, updated, unchanged, quarantined or failed.

### C9 Dashboards & reports
- **Features:**
  - **Components:** KPI card, bar/line/area/combo, pie/donut, funnel, scatter, radar, heatmap, word cloud, NPS, progress, countdown, ranking/Top N, pivot, table/view, text, button, slicer, AI component.
  - **Analysis:** chart cross-filtering, multiple sources per component, certified metrics, period comparison, fiscal calendars, drill-down to records.
  - **Distribution:** themes; scheduled delivery per recipient; subscriptions; alert rules → tasks; permission-scoped aggregates.
- **Acceptance:**
  - Every chart drills down to records.
  - Numbers are identical across engines.
  - Viewers only see aggregates of permitted rows.

### C10 Trust & audit
- **Features:** journal and cell history · restore points · hash-chained audit with signed checkpoints · trace explorer (record, run, approval, sync) · completeness verifier (process contracts) · retention policies · data requests · activity feed.
- **Acceptance:**
  - Tampering is detected.
  - Every operation reaches a terminal state (committed/rejected/exception).

### C12 Platform & admin
- **Features:**
  - **Tenancy:** multi-tenant (database per tenant), SSO (OIDC/SAML), passkeys, SCIM, security policies (sharing, export, IP allowlist, session length).
  - **Templates and change:** template install/adapt/upgrade, **change sets (staging/sandbox)**.
  - **Dashboards for people:** user dashboard (my work, approvals, exceptions, follows, activity) and admin dashboard (health, tenants, migrations, jobs, usage, audit, AI usage).
  - **Self-host:** single Docker image, backups.
- **Acceptance:**
  - Fresh install to first record takes < 5 min.
  - Upgrades are tested on all three databases.

### C13 Notifications & C14 Rules
- **Notifications:**
  - **Channels:** in-app, browser push, email, SMS (provider adapters; US 10DLC/TCPA quiet hours, India DLT templates), chat (Slack, Teams, Lark; LINE, KakaoTalk, WhatsApp via providers [verify]), webhook.
  - **Delivery:** templates per locale, recipients by field/capability/group/external contact, preferences, fallback chains, actionable messages, delivery tracking, consent registry.
- **Rules:** validation / reaction / alert kinds; triggers (record events, field change, state change, date reached, schedule, metric threshold, inactivity, source failure, comment, chat message, button, external event); policies (throttle, digest, quiet hours, escalation, loop protection); simulator; run log.
- **Acceptance:**
  - Every notification has a delivery status.
  - The simulator matches live behaviour on the same data.

### C11 AI → see doc 18 (emphasis).

---

## 6. Non-functional requirements (summary)

| Area | Requirement |
|---|---|
| Performance | Grid 1M rows ≥ 55 fps; first paint < 1 s; edit p95 < 300 ms; window query p95 < 300 ms; browser memory < 64 MB |
| Size | WASM ≤ 600 KB brotli (shell + grid + core) |
| Reliability | Every operation, run, sync and delivery reaches a terminal state; retries with idempotency |
| Security | OWASP ASVS L2; database per tenant; capability engine everywhere; SSRF protection; secrets encrypted; strict CSP |
| Privacy | Field classification; consent; retention; data requests; AI provider gating by classification |
| Accessibility | WCAG 2.2 AA (grid ARIA mirror, keyboard map) |
| i18n | en first; locale formats from day one (en-US, en-IN, de, ja, ko) |
| Quality | Function/line coverage 100% for business crates, branch ≥ 98%; mutation testing; every test on SQLite/MySQL/PostgreSQL and on Windows/macOS/Linux + browsers |

---

## 7. Glossary
- **Base:** a container of related tables, views, dashboards, workflows and pages.
- **Lifecycle:** states with categories and transitions bound to a table.
- **Capability:** an allowed action on a business object.
- **Grant:** capability + scope + conditions given to a principal.
- **Rule:** When / If / Then.
- **Workflow:** multi-step automation.
- **Data source:** an authoritative inbound system.
- **Destination:** an outbound target.
- **AI Skill:** a typed, stateless AI capability bound to fields.
- **Agent:** an AI principal with instructions, knowledge, tools, memory and budget.
- **Change set:** staged changes to a live base.
- **Template:** a solution package built from components.

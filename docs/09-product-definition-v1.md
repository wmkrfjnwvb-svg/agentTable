# Product Definition v1.0 — Scenario-anchored, core-capability platform

> Status: consolidated definition (supersedes conflicting parts of docs 01–08; see §8 document map).
> Stakeholder decisions this round:
> 1. **Anchor the product on the solution scenarios** (doc 08), supported by a **small set of broad core
>    capabilities**. Scenario details follow earlier rounds. Goal: **ready for immediate enterprise use**.
> 2. **No two-way synchronisation.** Use a **data-source model**: external systems are authoritative
>    sources whose data flows *in*. Data going *out* is a separate, explicit, one-way mechanism.
> 3. Clarify how Lark pushes data to external destinations (§3.1).

---

## 1. Product statement

**An open-source, AI-native multidimensional table for running business scenarios out of the box.**
Install it, choose a scenario package (HR & Admin, E-commerce after-sales, New Media, Recruitment,
Marketing, R&D), connect your data sources, invite people, and go. Every scenario is built from
the same core capabilities, so customers can adapt any package or build their own without code.

---

## 2. Scenario anchors

| Release | Packages (doc 08) | Why this order |
|---|---|---|
| **R1** | **HR & Admin** (requests, assets, onboarding/offboarding, expenses) · **E-commerce after-sales & reviews** | Together they exercise nearly every core capability; high value; clear loops |
| R2 | **New Media** content ops · **Recruitment** (with compliance guardrails) | Strongest AI showcase; high-value but compliance-sensitive |
| R3 | **Marketing** campaigns · **R&D** tracker | Broad reuse, fewer new primitives |

Each package ships with sample data, dashboards, AI configuration, tests, and a 5-minute
"first value" walkthrough.

---

## 3. Data-source model (replaces all two-way sync designs)

### 3.1 How Lark does it today (research)

| Direction | Lark mechanism | Notes |
|---|---|---|
| **Out** (Base → external) | Automation/workflow action **"Send HTTP request"**, triggered by record created/edited/conditions, buttons, schedules | Developer-oriented. **Internal (intranet) URLs are not supported** for security reasons, so the receiver must be publicly reachable. Auth is placed in headers ([Feishu help: send HTTP request](https://www.feishu.cn/hc/zh-CN/articles/410063847664), [automation guide](https://www.feishu.cn/content/article/7581775868524104882), [webhook order example](https://ae.feishu.cn/hc/zh-CN/articles/515976987951), [third-party guide](https://topppai.cn/blog/feishu-bitable-ai-integration/)) |
| Out | Open-platform **events** (`bitable_record_changed_v1`, `bitable_field_changed_v1`) | Subscription per file; formula changes don't fire; the app needs both identities' permissions (doc 05) |
| Out | **Pull via OpenAPI** by external tools (ETL/integration platforms, e.g. Alibaba Dataphin reads Base as a source) | ([Dataphin Base source](https://help.aliyun.com/zh/dataphin/fullmanaged/user-guide/create-a-multi-dimensional-table-data-source-for-flying-books)) |
| **In** (external → Base) | Workflow trigger **"When a webhook is received"**; OpenAPI writes; integration platforms or scripts scheduled to write data (attendance, MES orders via SQL) | ([integration platform case](https://www.feishu.cn/content/7312656872084258819), [Feishu workflow tools](https://www.feishu.cn/content/workflow-tools-and-systems), [MES script example](https://damodev.csdn.net/6aa7929361de6467390f1c25.html)) |

Takeaway: Lark gives building blocks (an HTTP node, a webhook trigger, events, an API), but the
**source/destination semantics are left to the customer**. Customers must decide who owns the
data, how to upsert, how to avoid loops, and what happens when something fails. Customers end up
writing scripts or buying integration platforms.

### 3.2 Our model

```
   DATA SOURCES (authoritative, inbound only)            DESTINATIONS (outbound only, explicit)
   ┌────────────────────────────────────┐               ┌──────────────────────────────────────┐
   │ Push inbox (JSON/NDJSON/CSV/XLSX)  │               │ Event subscriptions (signed webhooks) │
   │ Pull: SQLite / MySQL / PostgreSQL, │──► SOURCE ──► │ Workflow HTTP actions                 │
   │       REST, files (SFTP/S3/email)  │    TABLES     │ Scheduled feeds (CSV/XLSX/NDJSON/     │
   │ Manual import (templates)          │               │   Parquet → S3/SFTP/email/webhook)    │
   └────────────────────────────────────┘               │ Change feed API (GET /changes?since=) │
                                                        │ REST/MCP read access                  │
                                                        └──────────────────────────────────────┘
```

**Data source** = a named connection to an external system that is **authoritative** for a set of
records (orders from the shop platform, employees from a file, tickets from the help desk).

| Rule | Behaviour |
|---|---|
| **One direction in** | Data flows from source to source table only. **There is no write-back to the source.** No conflict resolution is needed, and there are no ping-pong loops |
| Source table | **Source fields** are read-only locally, updated only by the source (mapping spec, reference-data value maps, xref keys, raw payload kept). **Local fields** (status/lifecycle, owner, comments, approvals, AI fields, tags) are owned here and editable |
| Deletes at source | The record is marked **"retired at source"** (kept for history and links; hidden by default). Purge follows retention policy |
| Quality | Validation by the field framework; unmapped/invalid values go to **quarantine** with an owner; the run ledger and reconciliation (counts/hashes) appear in the source health page |
| Modes | Push (sync ≤ 500 records / async jobs above), scheduled pull (incremental by timestamp or PK), manual/templated import. All three end in the same pipeline |
| A table may be "native" | Tables without a source are fully editable (forms, manual entry, campaigns), as before |

**Changing source data from a business process** (the loop closes *through* the source):
> Example: an after-sales case is approved for refund → a workflow **HTTP action calls the shop
> platform's refund API** (destination) → the platform processes it → the next source sync brings the
> updated order status in → the case lifecycle closes automatically when the source status shows
> "refunded" (or raises an exception after the SLA).

The action's request/response is logged in the trace, and the *response* may be stored in local fields
(e.g., `refund_request_id`). The source field itself is changed **only by the source**.

### 3.3 Destinations (outbound, one-way)

| Mechanism | Use | Guarantees |
|---|---|---|
| **Event subscriptions** | Notify external systems of record/lifecycle/approval/**computed-field** changes (filter by table, fields, conditions) | HMAC-signed, at-least-once, event IDs for dedup, per-record ordering, retries with backoff, dead-letter, **replay**, delivery ledger |
| **Workflow HTTP action** | Call an external API as a business step (refund, create task, post message) | Request templating from fields, auth from **secret parameters**, timeouts, response mapping into *local* fields, failure → exception with owner |
| **Scheduled feeds** | Data warehouse/BI/partner deliveries | Snapshot or incremental (by journal sequence), formats CSV/XLSX/NDJSON/Parquet, PII export policy, audit |
| **Change feed API** | Consumers pull changes at their own pace: `GET /tables/{id}/changes?since={seq}` | Cursor-based, no lost events, easier and more robust than webhooks for many integrators |
| REST / MCP read | Ad-hoc reads, AI agents | Capability-scoped |

**Improvement over Lark:** in self-hosted deployments, admins can allow **internal-network
destinations** via an explicit allowlist (many ERP/MES systems live on the intranet). The SaaS
version keeps SSRF protection and public-only defaults.

### 3.4 Removed from earlier docs
- Doc 06 §2.5 "bidirectional sync, field-level source of truth for two-way, echo suppression": **removed**.
- Doc 07 §4.3 D "DB write-back" and §4.4 "write back through an outbound action" phrasing: **replaced**
  by §3.2 (actions call APIs; sources stay authoritative).
- Org/HRIS sync remains possible **as a data source** (inbound only).

---

## 4. Core capabilities (the minimal broad set)

Twelve capabilities cover all six scenarios. Details live in the referenced docs.

| # | Core capability | Contents | Detail in |
|---|---|---|---|
| C1 | **Table engine & grid** | Typed records, links/lookups/rollups, formulas (incremental), views (grid, kanban, calendar, gallery, timeline), record pop-up, tabs, themes (light/dark), attachments, 1M-row performance, capacity model | 04, 05 §1, 06 §5 |
| C2 | **Field framework** | Data type × widget × options source × format × validation × governance; field groups; field library; field-type registry; standard format library | 07 §1 |
| C3 | **Reference data** | Flat/tree sets, standard + custom attributes, tenant overlay, crosswalks, table & tree views, built-in ISO sets | 07 §2 |
| C4 | **Parameters** | Typed scoped KV, secrets, feature flags, `PARAM()` | 07 §3 |
| C5 | **Business capabilities & access** | Capabilities on business objects, grants with scope/conditions/validity, SoD, explain, field-group permissions, guests, agents; optional org module | 07 §5, 06 §2.2 |
| C6 | **Process engine** | Lifecycles, assignment + acknowledge, SLA & business calendars, **approval engine** (sequential/countersign/any-one/quorum, capability-based approvers, return/add/delegate/withdraw, locking, versioning), exceptions inbox, tasks | 03 §2, 06 §2.4, §2.8 |
| C7 | **Forms & collection campaigns** | Public/internal forms, conditional fields, prefill, campaigns by unit/period, submission tracking, period lock | 06 §2.6 |
| C8 | **Data sources & destinations** | §3 of this doc: push inbox, DB/REST/file pull, import templates, mapping + value maps, xref, quarantine, reconciliation; events, HTTP actions, feeds, change feed | this doc, 06 §2.5, §2.9 |
| C9 | **Reports & dashboards** | Certified metrics, pivots, period comparison/close, drill-down, scheduled per-recipient delivery, alerts → tasks, export | 06 §2.7, §2.10 |
| C10 | **Trace & audit** | Op delivery states, process instances, completeness verifier, hash-chained audit, evidence export | 02 B10, 03 §4.4 |
| C11 | **AI (five points)** | ① systems builder ② table AI (fields, capture, mapping, steward) ③ workflow AI (route, pre-check, draft, Q&A with escalation) ④ dashboard AI (generate, narrate, anomaly → task) ⑤ analytics (bounded planner on certified metrics); provenance, suggested mode, provider gateway, works without AI | 08 §4, 07 §6, 06 §4 |
| C12 | **Platform & operations** | Multi-tenant (DB per tenant; SQLite/MySQL/PG), auth (local/OIDC/SAML, passkeys), OpenAPI + SDKs + CLI + MCP, user & admin dashboards, solution packages (install/upgrade), 5-minute Docker deploy | 04, 05 §3 |

### 4.1 Scenario × capability coverage

| Capability | HR & Admin | E-com after-sales | New Media | Recruitment | Marketing | R&D |
|---|---|---|---|---|---|---|
| C1 Table/grid | ● | ● | ● | ● | ● | ● |
| C2 Fields | ● | ● | ● | ● | ● | ● |
| C3 Reference data | expense/asset trees | issue/return reasons | topic tree | job family/location | channel tree | component tree |
| C4 Parameters | policy limits | SLA hours | refresh interval | retention months | budget thresholds | SLA by severity |
| C5 Capabilities | request/approve/pay | handle/refund-approve | produce/publish | screen/offer-approve | budget/brand review | triage/release |
| C6 Process | approvals, onboarding | case lifecycle, SLA | production pipeline | pipeline, offers | campaign approval | bug/work lifecycle |
| C7 Forms/campaigns | requests, check-ins | — | — | careers form | lead forms | intake |
| C8 Sources/destinations | HR file, accounting feed | shop orders/reviews in; refund API out | platform analytics in | job boards/email in | ad spend in; CRM out | Git/support in |
| C9 Reports | SLA, assets | return rate, SLA | performance board | funnel | spend/ROAS | delivery/quality |
| C10 Trace/audit | ● | ● | ○ | ● | ○ | ○ |
| C11 AI | receipts, policy bot | classify/route/draft | variants, comments | CV extraction (restricted) | copy, pre-check | triage, notes |
| C12 Platform | ● | ● | ● | ● | ● | ● |

● central ○ used

---

## 5. "Ready for immediate use": first-run experience

1. `docker run …` → setup wizard (admin, instance URL, optional SMTP and AI provider). **≤ 5 minutes.**
2. **Choose scenario packages** (multi-select) → installed with sample data, dashboards and AI config.
3. **Connect data sources** with a wizard: push URL, DB, or file. AI suggests the mapping; a dry run
   shows the results; then enable. **≤ 10 minutes** per source.
4. Invite people → grant **capability sets** from the package (e.g., "Expense approver") → done.
5. A **guided checklist** in the user dashboard walks through the package's loop once (e.g., submit an
   expense → pre-check → approve → payment feed) so the customer sees the full loop working on day one.

---

## 6. Release plan (scenario-driven)

| Release | Core capabilities (minimum to complete the packages) | Packages |
|---|---|---|
| **R1** | C1, C2, C3, C4, C5 (no optional org module), C6 (lifecycles, assignments, SLA, approvals: sequential/countersign/any-one, exceptions, tasks), C7 (forms; campaigns basic), C8 (push inbox, DB pull, file import templates, events, HTTP action, feeds, change feed), C9 (dashboards, certified metrics basic, drill-down, scheduled delivery), C10, C11 (①, ② fields+capture+mapping, ③ route/pre-check/draft/Q&A bot, ④ narrative), C12 | HR & Admin; E-commerce after-sales |
| R2 | C6 quorum/advanced approvals, C7 full campaigns, C8 REST pull + more sources, C9 pivots/period close, C11 ⑤ analytics + ④ anomaly, optional org module | New Media; Recruitment |
| R3 | C11 agents (event-driven), extension SDK, more views (Gantt, hierarchy) | Marketing; R&D |

Engineering waves from docs 03–07 are reordered to serve these releases. The spikes (S1–S11) remain
gates before R1 implementation starts.

---

## 7. Decisions recorded
- Product anchored on scenarios + 12 core capabilities.
- **Data-source model; no two-way sync; no DB write-back.** Outbound is limited to events, HTTP actions,
  feeds, the change feed and API/MCP reads.
- Self-host may allow intranet destinations via allowlist.

## 8. Document map (current vs superseded)

| Doc | Status |
|---|---|
| 01 Research baseline | Background |
| 02 Deep dive (markets, tech, compliance) | Valid, except collaboration/storage details superseded by 04 |
| 03 Design & feasibility | Partly superseded (by 04, 06, 07, 09) |
| 04 Decisions round 3 (license, Leptos, DB-per-tenant, 3 DBs, coverage, lints) | **Valid** |
| 05 UX catalogue, AI kernel, Lark platform | UX catalogue & Lark research **valid**; AI kernel = infrastructure only (product AI per 07/08) |
| 06 Enterprise alignment | Valid except: org model now optional (07); bidirectional sync removed (09) |
| 07 Fields, reference data, params, integration, capabilities, AI | **Valid**, except write-back (09) |
| 08 Solution matrix | **Valid**, anchors the product |
| **09 Product definition v1.0** | **Authoritative summary** |

## 9. To confirm
1. R1 = HR & Admin + E-commerce after-sales: confirm.
2. Should the change feed API (pull) be the **recommended** outbound integration, with webhooks as
   the push alternative? (It is simpler and more robust for most integrators.)
3. Self-host: allow intranet HTTP destinations via admin allowlist (default off)?
4. Once confirmed, the next step is **implementation setup**: repository, workspace, CI, hooks,
   then the spikes. Please share the GitHub repository to use.

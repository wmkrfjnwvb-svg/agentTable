# Business Component Layer — how templates are assembled without custom code

> Status: product architecture v0.8 · Answers the stakeholder requirement: *no custom builds per client;
> the platform must let us (and customers) assemble ready-to-deploy templates from properly abstracted
> components.* Scenario templates with field-level detail are in doc 14.

---

## 1. Four levels, one rule

```
LEVEL 4  Tenant adaptation      a customer's own changes (fields, code values, approvers, limits, rules)
         ───────────────────    stored as an overlay, so template upgrades still apply
LEVEL 3  Templates              ready-to-deploy solutions (Recruitment, Expenses, Procurement, After-sales…)
         ───────────────────    = business components + configuration + content (code lists, rules, dashboards, sample data)
LEVEL 2  Business components    reusable business patterns: Party, Document with lines, Case, Pipeline,
         ───────────────────    Agreement, Asset custody, Scorecard, Booking, Ledger … (this document)
LEVEL 1  Platform primitives    tables, fields, code lists, settings, capabilities, process & approvals,
                                forms, rules, notifications, sources, reports, trace, AI skills (doc 09/11: C1–C14)
```

**Rules**
- **Templates contain no code.** If a template needs behaviour that no component offers, we build or extend a component that every template can use.
- **Components are configured, not modified.** Every difference between customers or markets goes into a declared **variation point**: optional fields, code lists, lifecycle states, approval policies, rules, market overlays.
- **Generic features work on any template.** SLA reports, budget tracking, scorecards, AI skills and dashboards work wherever a component's **field roles** are bound (§3). They don't depend on a table being named "Tickets" or "Expenses".

### 1.1 Research basis
Enterprise data modelling has long-established **universal patterns**:
- **Party / Party Role:** one person or organisation can hold several roles, such as customer, supplier, candidate or employee.
- **Agreement** and **Contextual Role:** parties hold roles *within* an order, shipment or project.
- Role assignments must be **tracked over time**; otherwise a changed project lead simply overwrites the old one.
- Fowler's **Accountability** links a commissioner and a responsible party, with a type and a time period.

([TDAN universal person/organisation model](https://tdan.com/?p=5014), [TDAN data model patterns](https://tdan.com/?p=22775),
[Data Model Resource Book Vol. 3](https://www.oreilly.com/library/view/the-data-model/9780470178454/chapter-47.html),
[Fowler, Analysis Patterns ch. 2](https://www.martinfowler.com/apsupp/apchap2.pdf)).
The components below apply these patterns to the scenarios. They add process behaviour (lifecycles,
SLAs, approvals) that classic data patterns leave out.

---

## 2. Component catalogue

Each component lists: **purpose**, **standard field set** (with field roles), **behaviour**,
**variation points**, and the **scenarios** that use it (doc 14 numbering: S1 Recruitment, S2 Employee
lifecycle & HR requests, S3 Expenses, S4 Assets, S5 Procurement, S6 After-sales & returns, S7 Customer
service desk, S8 Content & creators, S9 Marketing & leads, S10 R&D, S11 Projects).

### K1 Party & Party Role
- **Purpose:** every person or organisation the business deals with, inside or outside.
- **Fields:** `party_type` (person/organisation) · `display_name` · `legal_name` · `name_parts` (family/given, order per locale) · `contact_points[]` (email/phone/address/chat handle, with purpose and consent) · `identifiers[]` (tax ID, company number, employee ID; validated by format library) · `language` · `time_zone` · `country`.
- **Party Role** (child): `role_type` (customer, candidate, employee, supplier, creator, partner…) · `valid_from/to` · `status` · role-specific field group.
- **Behaviour:**
  - duplicate detection and merge (with history);
  - one party can hold many roles, e.g., a candidate becomes an employee and keeps the same party;
  - consent and retention apply per role (K19).
- **Variation points:** which roles are enabled; identifier types per market; required contact points.
- **Used by:** all scenarios.

### K2 Catalogue item & category-driven attributes
- **Purpose:** things that are offered, bought, assigned or produced: products/SKUs, asset models, job profiles, services, content formats.
- **Fields:** `code` · `name` · `category` (tree) · `status` · `unit` · `price/cost` (currency) · `attributes` (see below) · `media[]`.
- **Key abstraction: attribute sets by category.** Each category in the tree defines extra fields that appear only for items in that category (laptop → CPU, RAM; chair → material). Child categories inherit their parent's attributes.
- **Variation points:** category tree; attribute sets; pricing on/off; variants (size/colour) on/off.
- **Used by:** S4, S5, S6, S8, S9, S1 (job profiles).

### K3 Business document with lines
- **Purpose:** any header-plus-lines document: expense claim, purchase request/order, goods receipt, invoice, return request, quote, budget, offer.
- **Header fields:** `number` (K21) · `document_date` · `party` (K1 role) · `owner` · `currency` · `fx_rate` (snapshot) · `totals` (net, tax, gross; computed) · `state` (K-lifecycle) · `reference_documents[]` (e.g., PO for an invoice).
- **Line fields:** `item` (K2 or free text) · `description` · `quantity` · `unit` · `unit_price` · `tax_code` · `tax_amount` · `line_total` · `allocation[]` (K9 splits) · `attachments[]`.
- **Behaviour:**
  - totals are recalculated exactly (decimal arithmetic);
  - multi-currency with the exchange rate frozen at the document date;
  - **copy/convert to another document** (PR → PO, order → return), with links per line;
  - quantities are tracked across linked documents (ordered vs received vs invoiced);
  - generated PDF output (K22).
- **Variation points:** line fields; tax handling per market (e.g., JP qualified invoice registration number `T`+13 digits required for input-tax credit since Oct 2023 ([invoice guide](https://invoicedataextraction.com/blog/japan-qualified-invoice-system-qis), [EU-Japan centre](https://www.eu-japan.eu/qualified-invoice-system)); IN GSTIN; DE USt-IdNr; KR business registration number); conversion paths; tolerance settings.
- **Used by:** S3, S5, S6, S9 (budgets), S1 (offers).

### K4 Case (request / ticket)
- **Purpose:** something someone asks for that must be handled: HR request, IT request, customer complaint, return request, policy question, bug report intake.
- **Fields (roles):** `requester` (K1) · `channel` (form/email/chat/API/phone) · `category` (tree) · `priority` · `impact`/`urgency` (optional matrix → priority) · `owner` · `team` · `state` · `opened_at` · `first_response_at` · `resolved_at` · `closed_at` · `resolution_code` · `resolution_notes` · `satisfaction` · `related_records[]`.
- **Behaviour:**
  - **SLA clocks:** first response, resolution and custom clocks, with business calendars and **pause when the state category is "waiting on requester/third party"**;
  - assignment rules (round-robin, skills, load);
  - merge duplicates; split; parent/child cases;
  - reopen within N days;
  - satisfaction survey after resolve;
  - communication thread (K16).
- **Variation points:** category tree; SLA policies per category/priority/customer tier; states (each mapped to a state category, §4.1); assignment strategy; survey on/off.
- **Used by:** S2, S4, S6, S7, S10, S3 (questions).

### K5 Pipeline (staged relationship)
- **Purpose:** a relationship moving through stages toward an outcome: candidate × opening (application), lead × campaign, deal, creator × collaboration, supplier onboarding.
- **Fields:** `subject` (e.g., candidate) · `target` (e.g., opening) · `stage` · `stage_entered_at` · `owner` · `source` · `outcome` (won/hired/lost/rejected/withdrawn) · `outcome_reason` (code list) · `value` (optional).
- **Behaviour:**
  - full **stage history** (time in stage, who moved it);
  - stage gates (requirements before moving: e.g., scorecard submitted);
  - conversion and velocity reports;
  - bulk moves;
  - one subject can be in several pipelines;
  - re-entry rules (e.g., candidates can re-apply after 6 months).
- **Variation points:** stage list per pipeline (per opening type/market); gates; outcome reasons; probability per stage.
- **Used by:** S1, S8, S9, S5 (supplier onboarding), S11 (optional).

### K6 Work item, checklist & plan
- **Purpose:** units of work: tasks, checklist items, production steps, stories, project activities.
- **Fields:** `title` · `assignee` (person/group/capability) · `due` · `start` · `estimate` · `actual` · `state` · `parent` (K20) · `dependencies[]` · `checklist[]` · `attached_to` (**any record**).
- **Behaviour:**
  - **generate tasks from a checklist template** with conditions (e.g., "laptop setup only if role needs a laptop") and relative dates ("2 days before start date");
  - recurrence;
  - dependency scheduling;
  - "my tasks" across all templates;
  - workload views.
- **Variation points:** checklist templates per role/location/type; relative date anchors; dependency on/off.
- **Used by:** S2, S8, S10, S11, S4, S1.

### K7 Asset custody
- **Purpose:** individual things with identity that someone holds: laptops, phones, badges, vehicles, sample products, equipment.
- **Fields:** `asset_tag` · `serial_number` · `model` (K2) · `status` (in stock/assigned/in repair/lost/retired) · `location` (tree) · `custodian` (K1) · `purchase_date` · `cost` · `warranty_until` · `condition`.
- **Assignment** (child, effective-dated): `custodian` · `from` · `to` · `issued_by` · `signed_receipt` · `return_condition`.
- **Behaviour:**
  - check-out/check-in with signed receipts;
  - custody history;
  - **expected-return tasks** on exit or role change;
  - audits via collection rounds (K14);
  - depreciation (optional, straight-line);
  - maintenance schedule (K6 recurrence).
- **Variation points:** asset categories (K2 attributes); signature required; audit frequency.
- **Used by:** S4, S2, S8 (equipment, samples).

### K8 Agreement (contract) with obligations and rights
- **Purpose:** terms between parties: employment contract, offer, supplier contract, creator collaboration, NDA, service agreement.
- **Fields:** `parties[]` (K1 with contextual role) · `agreement_type` · `start` · `end` · `auto_renew` · `notice_period` · `value` (currency) · `payment_terms` · `document` (file/K22) · `signature_status` · `version`.
- **Obligations** (child): `description` · `owner_party` · `due` · `status` · `evidence` (e.g., deliverables).
- **Rights** (child): `right_type` (usage right, exclusivity, licence) · `scope` (channels, regions) · `valid_until`.
- **Behaviour:**
  - renewal and expiry alerts;
  - **obligation tracking** (deliverables vs contract);
  - rights expiry tasks;
  - amendment versions;
  - e-signature hand-off to a provider (destination).
- **Variation points:** agreement types; obligation templates; clause library (later).
- **Used by:** S1, S5, S8, S9, S2.

### K9 Budget, commitment & actuals (ledger-lite)
- **Purpose:** keep money under control across documents: budget lines, commitments (approved but not spent), actuals.
- **Fields:** `budget_line` (dimensions: cost centre, project, category, period) · `amount_budgeted` · `amount_committed` (computed) · `amount_actual` (computed) · `remaining` · `currency`.
- **Allocation splits** on documents: `dimension values` · `percent or amount`.
- **Behaviour:**
  - documents post **commitments** on approval and **actuals** on payment or receipt;
  - over-budget warnings or blocks (validation rule);
  - period roll-up;
  - exact decimals and currency conversion at document rate.
- **Variation points:** dimensions in use; block vs warn; periods (fiscal calendar).
- **Used by:** S3, S5, S8, S9, S11, S1 (headcount budget).

### K10 Approval binding
- **Purpose:** attach an approval process (doc 11 §4) to any record or document state change.
- **Field roles needed:** `amount` (for thresholds) · `requester` · `scope dimensions` (cost centre, entity, region) · `risk` (optional).
- **Behaviour:** approver resolution by capability + scope + limit; locks approved fields; approval history on the record.
- **Variation points:** chosen approval template; thresholds as settings; delegation rules.
- **Used by:** all.

### K11 Evaluation (scorecard / rubric)
- **Purpose:** structured judgement by one or more evaluators: interview scorecards, supplier evaluation, content review, performance check-ins.
- **Fields:** `subject` · `evaluator` · `rubric` (criteria with scale and weight) · `scores[]` · `recommendation` (code list) · `comments` · `submitted_at`.
- **Behaviour:**
  - several evaluators per subject;
  - **evaluators can't see each other's scores until they submit their own**, to avoid anchoring bias (optional);
  - aggregation (average, weighted, min);
  - deadlines and reminders;
  - calibration view.
- **Variation points:** rubric templates; blind until submitted; aggregation method.
- **Used by:** S1, S5, S8, S10 (reviews).

### K12 Inspection (observation against a specification)
- **Purpose:** recorded checks of something physical or digital: returned goods inspection, goods receipt quality check, asset audit, pre-publish compliance check.
- **Fields:** `subject` · `inspector` · `checklist` (items with expected value/spec) · `results[]` (value, pass/fail, photo) · `overall_result` · `disposition` (code list).
- **Behaviour:** fail triggers rules (e.g., create supplier claim, block refund); photo evidence required per item (configurable).
- **Variation points:** checklists per category; disposition codes; sampling rules.
- **Used by:** S6, S5, S4, S8.

### K13 Schedule & booking
- **Purpose:** time-bound events involving people or resources: interviews, shoots, publication slots, meeting rooms, deliveries.
- **Fields:** `start` · `end` · `time_zone` · `participants[]` · `resources[]` · `location`/`link` · `status` · `recurrence`.
- **Behaviour:**
  - conflict detection for people and resources;
  - availability-based slot proposals;
  - calendar invitations via destinations (calendar systems);
  - reminders.
- **Variation points:** bookable resource types; buffer times; who can book.
- **Used by:** S1, S8, S11, S2.

### K14 Collection round
Periodic submissions from units or people (doc 06 §2.6): store reports, asset audits, budget inputs, check-ins.
Used by S2, S4, S9, S11.

### K15 Metric snapshot (time series)
- **Purpose:** periodic values per entity: post views per day, ad spend per day, stock level, NPS per month.
- **Fields:** `subject` · `metric` (code list) · `period_start` · `granularity` · `value` · `source`.
- **Behaviour:** scheduled collection from sources; latest-value fields on the subject; trend and comparison reports; anomaly alerts (statistical).
- **Used by:** S8, S9, S6, S11.

### K16 Communication thread
- **Purpose:** messages in and out about any record: email, chat, SMS, portal comments, internal notes.
- **Fields:** `record` (any) · `direction` · `channel` · `from/to` (K1) · `subject` · `body` · `attachments` · `sent_at` · `delivery_status` · `visibility` (internal/external).
- **Behaviour:** inbound email/chat creates or updates records; templates (doc 11 §3); AI drafts; first-response SLA timestamps for K4.
- **Used by:** S1, S6, S7, S8, S9.

### K17 Knowledge article
- **Purpose:** versioned policy or help content.
- **Fields:** `title` · `body` · `category` · `audience` · `version` · `owner` · `review_due`.
- **Behaviour:** used for Q&A with handover (E2); review reminders; links to cases.
- **Used by:** S2, S3, S7.

### K18 Exception & SLA (platform-level)
- Exceptions inbox and SLA engine (docs 03, 06). Components emit standard exception types: unassigned, overdue, failed action, unmapped value, missing submission, unreturned asset.

### K19 Consent & retention
- **Purpose:** personal-data duties per role and purpose.
- **Fields:** `party` · `purpose` · `consent_text_version` · `given_at` · `withdrawn_at` · `retention_rule` · `delete_after`.
- **Behaviour:** retention timers → deletion or anonymisation tasks; subject access and export (DSAR); blocks processing after withdrawal.
- **Variation points:** retention periods per market and role.
- **Used by:** S1, S6, S7, S9, S2.

### K20 Hierarchy & dependency
- **Purpose:** parent/child trees of records (epic → story → task; project → phase → activity; category trees) and dependency graphs.
- **Behaviour:** roll-ups (progress, effort, cost); tree and outline views; cycle prevention.
- **Used by:** S10, S11, S8, K2 categories.

### K21 Identifier & numbering
- **Purpose:** human-readable numbers with patterns `PO-{YYYY}-{seq:5}`, per entity/legal entity/year; external IDs via cross-references (doc 06 §2.5).
- **Behaviour:** gap-free sequences where legally required (invoices in some markets) **[verify per market]**.
- **Used by:** all.

### K22 Document output & signature hand-off
- **Purpose:** generate PDFs/DOCX from templates (offer letter, PO, return label sheet, contract) with record fields; send to an e-signature provider and record signature status when the provider reports back.
- **Used by:** S1, S5, S8, S2.

### K23 Quantity ledger (stock movements)
- **Purpose:** quantities that go in and out: consumables stock, spare devices, returned goods to restock, samples.
- **Fields:** `item` (K2) · `location` · `movement_type` (receive/issue/transfer/adjust/return) · `quantity` · `reference_document` · `at`.
- **Behaviour:** balance per item/location (computed); reorder level alerts; reconciliation with counts (K14).
- **Used by:** S4, S5, S6.

---

## 3. Field roles: the contract that makes generic features work

Each component declares **roles** that template fields bind to. Platform features use roles, not table names.

| Role | Meaning | Features that use it |
|---|---|---|
| `party.requester`, `party.counterparty` | Who asked / who the other side is | Portals, notifications to externals, consent |
| `person.owner`, `person.assignee` | Who is responsible | My work, workload, escalation, exceptions |
| `state` (+ state categories) | Where it is in its lifecycle | SLA pause, dashboards, loop health |
| `time.opened`, `time.due`, `time.resolved` | Key timestamps | SLA, cycle time, ageing reports |
| `money.amount`, `money.currency` | Value | Approval thresholds, budgets, totals |
| `dimension.*` (cost centre, region, entity, project, brand) | Business scope | Permissions scopes, budgets, report slicing |
| `classification.category`, `classification.reason` | Code-list classification | Routing, root cause, AI classification target |
| `evidence.attachments` | Proof | Pre-checks, audits, AI extraction source |
| `quantity`, `item` | Goods | Ledgers, document matching |

Examples:
- The **"SLA & ageing" dashboard** works on HR requests, IT requests, returns, bugs and content stages alike, because each binds `state`, `time.opened`, `time.due` and `person.owner`.
- The **AI routing skill** works on any K4 case through `classification.category`.

---

## 4. Variation mechanisms (how one template fits many companies)

### 4.1 Lifecycles with state categories
Customers rename, add or remove states freely, but every state belongs to one **category**:
`new` · `in_progress` · `waiting_internal` · `waiting_external` (pauses SLA) · `pending_approval` ·
`done_success` · `done_failure` · `cancelled`. Components, SLA clocks, dashboards and AI understand
categories, so a customer's 12-state recruitment pipeline and another's 5-state one both work with the
same reports.

### 4.2 Variation points declared per template

| Variation point | Example |
|---|---|
| Optional field groups | "Compensation expectations" on/off; "Usage rights" on/off |
| Category attribute sets | Laptop vs furniture attributes |
| Code lists (tenant overlay) | Rejection reasons, return reasons, expense categories |
| Lifecycle states | Add "Background check" stage to recruitment |
| Approval policy | Single manager approval (small team) ↔ multi-level by amount (enterprise) |
| Thresholds & settings | Per-diem limits, SLA hours, refund auto-approve limit |
| Rule packs | Enable/disable reminder sets, escalation timings |
| Channels | Which notification channels and data sources |
| AI skills | Which skills are on; confidence thresholds |
| **Market overlays** | US, JP, KR, DE, IN packs adding fields, validations, code lists and rules |
| **Size profiles** | "Starter" (few states, one approver) vs "Standard" vs "Enterprise" (multi-level, budgets, audits) |

### 4.3 Market overlay examples

| Overlay | Adds |
|---|---|
| JP | Qualified invoice registration number (`T`+13 digits) on supplier/receipt documents; new-graduate recruiting cohort fields (graduation year, internship participation); name order family-given; fiscal year April–March default; stealth-marketing ad disclosure flag required on sponsored content since Oct 2023 ([Japan Times](https://www.japantimes.co.jp/news/2023/03/28/national/stealth-marketing-rules/), [Baker McKenzie](https://insightplus.bakermckenzie.com/bm/consumer-goods-retail_1/japan-consumer-affairs-agency-issues-its-first-stealth-marketing-related-order-for-action_1)) |
| DE | USt-IdNr; **14-day withdrawal period** setting for consumer returns (distance/off-premises contracts) ([German administration portal](https://verwaltung.bund.de/leistungsverzeichnis/en/rechte-und-pflichten/102837988)); **works-council approval step** option in hiring and transfers (co-determination under BetrVG, typically for establishments above 20 employees **[verify]**); "divers" gender value; configurable activity logging |
| IN | GSTIN and PAN validation; notice period field on candidates/employees; background verification stage; fiscal year April–March; DPDP consent notices; SMS template IDs (DLT) |
| KR | Business registration number; resident registration numbers blocked from collection; KakaoTalk notification channel; PIPA consent texts |
| US | EIN; voluntary EEO self-identification group (restricted); employment-eligibility (I-9) checklist tasks **[verify]**; state-specific retention; TCPA consent for SMS |

---

## 5. Template definition format (what a template file contains)

Templates are declarative packages. A shortened example:

```yaml
template: procurement.standard            # S5
version: 1.3.0
extends: procurement.starter               # inherit and add
profile: standard                          # starter | standard | enterprise
components:
  supplier:      { component: K1.party_role, role_type: supplier,
                   field_groups: [contact, tax_ids, bank_details?, evaluation?] }
  item:          { component: K2.catalogue, categories: refdata/purchase_category }
  purchase_request: { component: K3.document, lines: true, converts_to: purchase_order,
                      approval: approvals/pr_by_amount, budget: K9 { dimensions: [cost_center, project?] } }
  purchase_order: { component: K3.document, lines: true, converts_to: [goods_receipt, supplier_invoice],
                    output: K22 { template: docs/po.docx }, numbering: "PO-{YYYY}-{seq:5}" }
  goods_receipt: { component: K3.document, inspection: K12 { checklist: checklists/receipt_qc }, stock: K23 }
  supplier_invoice: { component: K3.document, match: three_way { qty_tolerance: PARAM(p2p.qty_tol),
                      price_tolerance: PARAM(p2p.price_tol) } }
  contract:      { component: K8.agreement, types: [framework, one_off], renewal_alert_days: 60 }
variation_points:
  - approvals/pr_by_amount.thresholds
  - fields.purchase_request.project (optional)
  - refdata/purchase_category (tenant overlay)
  - rules/overdue_delivery (on/off, days)
markets: [jp, de, in, kr, us]             # overlays available
rules: [rules/budget_warning, rules/overdue_delivery, rules/invoice_mismatch]
dashboards: [dash/spend_by_category, dash/po_cycle_time, dash/supplier_performance]
ai_skills: [ai/invoice_extraction, ai/duplicate_invoice, ai/pr_precheck]
sample_data: data/procurement_sample.ndjson
tests: [tests/three_way_match.yaml, tests/approval_paths.yaml]   # run on every DB and every upgrade
```

**Lifecycle of templates**
- **Install:** creates tables, fields, code lists, rules, dashboards and AI skills; loads sample data (optional).
- **Adapt:** tenant changes are recorded as an **overlay** (never edits to the template itself).
- **Upgrade:** a new template version is merged three ways (template old → template new, plus the tenant overlay). Conflicts are shown for decision, and template tests are re-run.
- **Compose:** templates can link, e.g., Recruitment's accepted offer → Employee lifecycle onboarding case through shared components (K1 party, K8 agreement).
- **Author:** templates are built in the product (by us, partners or customers), exported as packages, and validated by the loop-health checklist and their tests.

---

## 6. New platform requirements discovered from scenarios

| Need | Component(s) | Added to platform capability |
|---|---|---|
| Child line items with exact totals, conversions between documents, quantity tracking across documents | K3 | C1 Table engine (line tables, roll-ups, document links) |
| Multi-currency with frozen exchange rates; allocation splits | K3, K9 | C1 + C4 (rate source as data source) |
| Category-driven attribute sets | K2 | C2 Field framework |
| State categories; SLA pause on waiting states | K4, §4.1 | C6 Process |
| Polymorphic links: tasks, comments, communications and attachments on any record | K6, K16 | C1 |
| Effective-dated child records (assignments, roles) | K1, K7 | C1 |
| Booking with conflict detection | K13 | New in C6 |
| Rubric scoring with blind-until-submitted | K11 | New in C6 |
| Quantity ledger balances | K23 | C1 roll-ups + C14 rules |
| Document generation & e-signature hand-off | K22 | C8 destinations |
| Time-series snapshots with latest-value fields | K15 | C1 + C8 scheduled sources |
| Field roles on components | §3 | C2 + C9 reports + C11 AI |
| Template packages: extends, overlays, profiles, market packs, 3-way upgrade, tests | §4–5 | C12 Platform |

Each of these is generic: it serves several templates (see the "used by" lists) and passes the
"no single-client feature" test.

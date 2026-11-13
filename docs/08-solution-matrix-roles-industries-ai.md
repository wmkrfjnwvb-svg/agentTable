# Solution Matrix — Roles × Industries × AI points

> Status: design v0.6 · Builds on docs 06–07 (field framework, reference data, capabilities,
> integration-first, grounded AI).
> Stakeholder input:
> **Roles**: Recruitment, R&D, Marketing, HR & Administration, Industry →
> **Industries**: E-commerce, New Media ·
> **AI points**: AI-powered systems, AI for data tables, AI-driven workflows, AI-powered dashboards, AI analytics.
> This mirrors how Lark packages Base: template groups by role (HR & Admin, R&D, Recruitment…) and
> by industry (E-commerce & retail, New Media…) ([HR & Admin group](https://www.feishu.cn/template/human-resources-administration-template-group),
> [R&D group](https://www.feishu.cn/template/product-development-template-group),
> [Internet group](https://www.feishu.cn/template/internet-template-group),
> [template centre](https://www.feishu.cn/template/group/base), [recruitment system](https://www.feishu.cn/template/easy-recruitment-system)).

---

## 0. How the three axes fit together

```
            SOLUTION PACKAGE  =  role or industry scenario, built only from platform primitives
            ┌───────────────────────────────────────────────────────────────────────────┐
            │ tables + field library + reference sets + capabilities + lifecycles +     │
            │ approvals + forms/campaigns + views + dashboards + workflows + connectors │
            │ + AI configuration (5 AI points) + sample data + tests                    │
            └───────────────────────────────────────────────────────────────────────────┘
AI points cut across every package:
  ① AI-powered systems  → AI builder: generate/adapt the package from a description, a spreadsheet, a DB or an API payload
  ② AI for data tables  → AI fields, AI capture (docs/images/chats → records), AI mapping of imports/integrations, data steward
  ③ AI-driven workflows → AI steps in workflows/approvals: classify & route, pre-check, draft, Q&A with escalation, agents
  ④ AI-powered dashboards → generate dashboards from a goal; narrative summaries; anomaly alerts → tasks
  ⑤ AI analytics        → governed Q&A on certified metrics (bounded planner), drivers ("why did X change?"), period comparisons
```

**Rule:** solutions contain **no special-case code**. If a package needs something, it becomes a
platform primitive (as task presets did in doc 05). This keeps "AI-built systems" (①) honest: the
AI can only build what the primitives can express, and everything it builds is governed.

---

## 1. Matrix overview (the key AI use per cell)

| Solution | ① AI systems | ② AI tables | ③ AI workflows | ④ AI dashboards | ⑤ AI analytics |
|---|---|---|---|---|---|
| **Recruitment** | Generate an ATS from your hiring process | Resume → candidate record (extraction, dedupe); JD drafting | Interview feedback summary; offer pre-check; candidate message drafts | Funnel & time-to-hire with narrative | "Which sources produce hires fastest for Engineering?" |
| **R&D** | Generate backlog/sprint/bug system | Bug triage fields (component, severity, duplicate) | Release-note drafting; stale-item chasers | Delivery & quality dashboard | "Why did lead time rise this sprint?" |
| **Marketing** | Generate campaign + content + budget system | Asset tagging; copy variants per locale; lead enrichment | Content approval pre-check (brand/claims); campaign launch checklist | Spend vs. results by channel | "Which channels beat target CPA this quarter?" |
| **HR & Admin** | Generate onboarding, leave, assets, expenses systems | Receipt/contract/ID-document capture; policy-doc extraction | Policy Q&A bot with escalation; expense pre-check; onboarding task generation | Headcount, requests SLA, asset utilisation | "Which teams have the most overdue onboarding tasks?" |
| **E-commerce** | Generate product, order, after-sales, live-commerce system | Product copy & translation; review/return-reason classification | Negative-review handling (classify → route → reply draft); return pre-check | GMV, return rate, live-session board | "Which SKUs drive returns, and why?" |
| **New Media** | Generate content-production + publishing + creator system | Topic library tagging; title/script variants; comment classification | Production pipeline chasers; comment reply drafts; creator brief drafting | Cross-platform performance board (auto-refresh) | "Which topics outperform on our account vs. benchmark?" |

---

## 2. Solution packages

Each package lists the **closed loop**, the **data assets** it creates, **capabilities**,
**integrations** and **AI points**. Platform names in integrations are *candidates* to evaluate;
API availability and terms must be verified per platform **[verify]**.

### 2.1 Recruitment

| Aspect | Design |
|---|---|
| Closed loop | Headcount request → **approval** → job opening → posting → candidates (inbox from job boards/careers form) → screening → interviews (scheduling, scorecards) → offer (**approval**) → accept → **handoff to HR onboarding** → close opening → source/funnel metrics feed next plans |
| Tables | Requisitions, Openings, Candidates, Applications (candidate × opening), Interviews, Scorecards, Offers, Sources |
| Reference sets | Job family (tree), level, location (tree), source channel, pipeline stage, rejection reason, employment type |
| Field groups | Candidate: Contact · Experience · Compensation expectations (restricted) · **EEO/diversity self-ID (restricted, separate, US)** |
| Capabilities | `requisition.submit/approve`, `application.move_stage`, `interview.submit_scorecard`, `offer.approve`, `candidate.view_compensation` |
| Integrations | Careers-site form (inbox), email-in for CVs, job boards / ATS by market, e.g. LinkedIn, Indeed (US/global), doda/BizReach (JP), Saramin/JobKorea (KR), StepStone/XING (DE), Naukri (IN) **[verify APIs]**; calendar for scheduling |
| AI points | ② CV parsing with evidence, duplicate candidates, JD drafting from requisition ③ scorecard summary, offer pre-check (band vs. requisition budget), candidate messages in their language ④ funnel narrative ⑤ source effectiveness |
| Compliance (built in) | **No AI ranking, scoring or rejection of candidates by default** (EU AI Act high-risk; NYC LL144 bias audits); candidate data **retention policy** with consent tracking (GDPR, DPDP, PIPA, APPI); EEO data separated and access-restricted |
| KPIs | Time-to-fill, time-to-hire, offer acceptance rate, source-to-hire, interviewer feedback SLA |

### 2.2 R&D

| Aspect | Design |
|---|---|
| Closed loop | Request/idea → triage → backlog → planned in iteration → in progress → review/QA → released → feedback/bugs → back to backlog; bug SLA by severity |
| Tables | Requirements, Epics, Iterations, Work items, Bugs, Test cases, Releases |
| Reference sets | Component/module (tree), severity, priority, platform/OS, environment |
| Capabilities | `bug.triage`, `work_item.estimate`, `release.approve`, `requirement.prioritise` |
| Integrations | GitHub/GitLab (webhooks: PRs, commits linked by issue key), CI status, Jira import (migration), support desk → bugs (inbox) |
| AI points | ① generate tracker from team process ② bug triage fields (component/severity suggestion, **duplicate detection**), reproduction-step extraction from support tickets ③ release notes draft from merged items; stale-item chasers ④ delivery/quality board narrative ⑤ lead-time drivers |
| KPIs | Lead/cycle time, throughput, escaped defects, bug SLA attainment, iteration predictability |

### 2.3 Marketing

| Aspect | Design |
|---|---|
| Closed loop | Plan (goals, budget) → campaign brief (**approval**) → assets/content production → brand/legal review → launch → spend & results collected → leads to sales (handoff, ack) → review → next plan |
| Tables | Campaigns, Budget lines, Content items, Assets, Channels, Spend (daily), Leads, Events |
| Reference sets | Channel (tree: paid/owned/earned → platform), region, product line, funnel stage, asset type |
| Capabilities | `campaign.approve_budget`, `content.brand_review`, `lead.accept` |
| Integrations | Ad platforms spend/results pull (Google Ads, Meta, TikTok; LINE (JP), Naver (KR) **[verify]**), CRM (HubSpot/Salesforce) lead handoff, forms → leads (inbox) |
| AI points | ② asset auto-tagging, copy variants per locale (en/ja/ko/de/hi), lead enrichment ③ **content pre-check** (brand terms, required disclaimers, prohibited claims list = reference set), launch checklist generation ④ spend vs. outcome narrative, anomaly alerts (CPA spike → task) ⑤ channel efficiency |
| KPIs | Spend vs. budget, CPA/ROAS, MQL→SQL conversion, content cycle time |

### 2.4 HR & Administration

| Aspect | Design |
|---|---|
| Scenarios (sub-packages) | Employee records (core HR-lite, optional), **onboarding/offboarding**, leave & attendance requests, **IT/office assets**, expense reimbursement, contracts & certificate expiry, training, office supplies purchasing, meeting rooms & visitors |
| Closed loops | Onboarding: offer accepted (from Recruitment) → checklist tasks to IT/admin/manager → equipment assigned (asset register) → day-1 confirmation → 30/60/90 check-ins. Offboarding: assets returned, access revoked, **orphaned-ownership transfer** (doc 06). Expense: submit → **pre-check** → approval → payment export → reconciliation |
| Tables | People (or synced), Onboarding cases, Tasks, Assets, Asset assignments, Requests (leave/expense/purchase), Contracts, Trainings |
| Reference sets | Request type, expense category (tree), asset category (tree), location (tree), leave type, document type |
| Capabilities | `request.submit/approve`, `asset.assign/return`, `expense.pay`, `person.view_personal_data` |
| Integrations | Optional HRIS/IdP (SCIM), accounting export (CSV/API), e-signature (later), calendar |
| AI points | ① generate HR/admin systems from policy docs ② **receipt/invoice capture** (amount, date, tax ID validated by format library), contract term extraction, ID-document fields (restricted) ③ **policy Q&A bot with escalation & logging** (Aily pattern), expense pre-check (policy limits, duplicates), onboarding task generation per role/location ④ requests SLA, headcount ⑤ "which teams have overdue onboarding tasks?" |
| Compliance | PII classification on person fields; DE works-council-sensitive activity logging configurable; retention per document type |
| KPIs | Onboarding completion on time, request SLA, asset utilisation/loss, expense cycle time |

### 2.5 E-commerce

| Aspect | Design |
|---|---|
| Scenarios | Product & SKU master, inventory snapshot, orders (synced), **after-sales & negative-review handling** (Lark has a template for this), returns, promotions calendar, **live-commerce operations** (product selection → scripts → session → post-session review), suppliers & purchasing |
| Closed loops | Review/after-sales: review or ticket arrives → classified → routed to owner (ack, SLA) → resolution + reply → customer outcome → root-cause tags feed product/QA. Live commerce: selection (approval) → script → session data collected → review within 24h → selection adjustments |
| Tables | Products, SKUs, Orders (integration-backed), After-sales cases, Reviews, Returns, Promotions, Live sessions, Session items, Suppliers, POs |
| Reference sets | Product category (tree), attributes, return reason, issue type, channel/store, region |
| Integrations | Store platforms via push inbox or pull, e.g. Shopify, Amazon, Rakuten (JP), Coupang (KR), Flipkart (IN), Otto (DE) **[verify APIs]**; ERP/WMS via DB connectors |
| AI points | ① generate store-ops system from the shop's export files ② product copy & **translation per market**, review/return-reason classification, attribute extraction from supplier sheets ③ **negative-review workflow** (classify → route → draft reply → human approve), return pre-check ④ GMV/return dashboards with anomaly alerts ⑤ "which SKUs drive returns and why" (aggregation by reason codes, not free-form guessing) |
| KPIs | GMV, return rate, after-sales SLA, review response time, live-session conversion |

### 2.6 New Media

| Aspect | Design |
|---|---|
| Scenarios | Topic library (incl. benchmark/viral references), content production pipeline (topic → script → shoot → edit → review → publish), multi-platform publishing calendar, **performance data collection** (Lark cases refresh platform data every 15 min for live dashboards), creator/KOL collaborations (brief → contract → delivery → payment), comment & community handling |
| Closed loops | Topic → production with owners & deadlines ("one-click chase" button, as Lark customers use) → published → data collected → **compare vs. reference/benchmark** → topic scores update → next topics |
| Tables | Topics, Content items, Production tasks, Publications (content × platform), Metrics snapshots, Creators, Collaborations, Comments |
| Reference sets | Platform, content format, topic category (tree), audience segment, collaboration stage |
| Integrations | Platform analytics pulls, e.g. YouTube, TikTok, Instagram, X, LINE (JP), Naver (KR) **[verify APIs/terms]**; scheduled every N minutes |
| AI points | ① generate content-ops system ② topic tagging, title/script variants, transcript summary ③ chasers, comment classification + reply drafts (approve before posting), creator brief drafting ④ cross-platform board with narrative ⑤ topic/format performance vs. benchmark |
| KPIs | Output per week, on-time publishing, views/engagement per topic category, collaboration ROI |
| Evidence | Lark New Media cases: production chasers, 15-minute data refresh, viral-topic comparison ([content studio case](https://www.feishu.cn/content/article/7595522152070335693), [video studio case](https://www.feishu.cn/content/article/7584000469945552085), [content factory](https://www.feishu.cn/content/article/7584012383060757728)); e-commerce live operations ([live commerce](https://www.feishu.cn/content/article/7582157021714156772), [marketing loop](https://www.feishu.cn/content/article/7582904528077917408)) |

---

## 3. Shared building blocks (proof that packages need no special code)

| Primitive (docs 05–07) | Rec | R&D | Mkt | HR&A | E-com | NM |
|---|---|---|---|---|---|---|
| Field framework + field library | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Reference data (trees) | job family, location | component | channel | expense/asset cats | product category | topic category |
| Capabilities + scopes | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Lifecycle + assignment + SLA | pipeline | bug/work item | content | requests | after-sales | production |
| Approval engine | requisition, offer | release | budget, content | leave, expense | selection, refunds | collaborations |
| Collection campaigns | — | — | — | onboarding check-ins | store reports | — |
| Integration inbox / connectors | job boards, email | Git, support | ads, CRM | HRIS, accounting | store platforms | platform analytics |
| Exceptions inbox | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Dashboards + certified metrics | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Cross-package links | → HR onboarding | ← support | → sales | ← recruitment | ← marketing | ↔ marketing |

---

## 4. The five AI points: v1 specification

| AI point | v1 scope | Inputs → outputs | Guardrails | Maps to doc 07 |
|---|---|---|---|---|
| **① AI-powered systems** | Generate a package (or adapt one) from: description, uploaded spreadsheet, DB schema, sample API payload, policy/process document | → preview of tables, field library entries, reference sets (with trees), capabilities, lifecycles, approvals, forms, views, dashboards, workflows | Only platform primitives; preview & diff; nothing created until confirmed; generated package passes the loop-health checklist | Feature 4 (builder) + capability discovery |
| **② AI for data tables** | AI fields (extract/classify/summarise/translate/score/custom), AI capture (doc/image/email/chat → record), AI mapping (imports/integrations), data steward | Cell/record values with provenance & evidence | Outputs bound to reference sets/field types; suggested mode for sensitive fields; cached by input hash | Features 1, 2, 3, 6 |
| **③ AI-driven workflows** | Workflow/approval steps: classify & route, **pre-check (findings only)**, draft message/reply, Q&A bot with escalation + logging, generate tasks | Step outputs into records/tasks; never final approvals | Capability-checked actions; human approval for outbound messages by default; run ledger & trace | Feature 5 (+ agents later) |
| **④ AI-powered dashboards** | Generate dashboard from a goal ("track after-sales SLA by region"); narrative summary per refresh; **anomaly detection → exception/task** | Dashboard definition; text summary citing metrics; alerts | Built on certified metrics; narrative states numbers computed by engine only | New (uses W9 reports) |
| **⑤ AI analytics** | Bounded planner: `metric_query`, `find_records`, driver breakdown ("why did X change": decomposition by dimensions), period comparison | Plan → deterministic results → explanation with links | Certified metrics only; asks when ambiguous; permission-scoped | Feature 7 (bounded planner) |

---

## 5. Prioritisation (which packages first)

Scoring 1–5 (5 = best): business value, loop strength, integration effort (5 = easy),
AI value, compliance risk (5 = low), reuse of primitives.

| Package | Value | Loop | Integration | AI value | Compliance | Reuse | **Total** |
|---|---|---|---|---|---|---|---|
| HR & Admin (onboarding, requests, assets, expenses) | 5 | 5 | 4 | 4 | 3 | 5 | **26** |
| E-commerce after-sales & reviews | 5 | 5 | 3 | 5 | 4 | 4 | **26** |
| New Media content ops | 4 | 4 | 3 | 5 | 5 | 4 | **25** |
| Recruitment | 5 | 5 | 3 | 4 | 2 | 4 | **23** |
| Marketing campaigns | 4 | 4 | 3 | 4 | 4 | 4 | **23** |
| R&D tracker | 3 | 4 | 4 | 3 | 5 | 4 | **23** |

**Recommendation:**
- **First validation packages: HR & Admin (requests + assets + onboarding) and E-commerce
  after-sales.** They exercise approvals, capabilities, reference trees, capture AI, workflow AI and
  the integration inbox, which covers almost every primitive.
- **Next:** New Media (strongest AI showcase, low compliance risk), then Recruitment (high value,
  but needs the compliance guardrails finished), Marketing and R&D.

---

## 6. Packaging, testing, delivery

- **Solution package format:** versioned, installable, upgradable bundle (schema + reference
  data + capabilities + lifecycles + approvals + views + dashboards + workflows + AI configs +
  connectors config + sample data + **tests**). Tenant changes are preserved on upgrade (3-way merge
  with conflicts surfaced).
- **Every package is also a test fixture:**
  - E2E tests run on all 3 DBs.
  - Loop-health checklist (every state has an owner rule, SLA and terminal state; every dashboard
    drills down).
  - AI evals (S8 bounded planner questions per package; extraction accuracy sets for capture).
  - Performance runs at 1M rows using the capacity model.
- **Localization:** en first; package labels & reference sets prepared for ja/ko/de/hi;
  market-specific reference data (e.g., JP/KR/DE/IN tax-ID formats, holiday calendars).
- **Plan:** packages are built after W7 (process) and W3 (integration core) land. HR & Admin + E-commerce
  after-sales packages become the acceptance gate for the first public release; others follow per wave.

## 7. Questions
1. Do the **scenarios per role** in §2 match what you have in mind? Any must-have scenarios missing
   (e.g., Recruitment: referral programme; HR: performance reviews; E-commerce: inventory replenishment)?
2. Which **platforms per market** matter most for E-commerce and New Media integrations?
3. Agree with the **first two packages** (HR & Admin, E-commerce after-sales)?
4. "Industry" as a role: should we add more industries later (e.g., manufacturing, restaurants), as
   Lark's template centre does?

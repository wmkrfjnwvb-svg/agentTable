# Core capability detail — Data collection, Rules, Notifications, Workflow designer, AI catalogue

> Status: product design v0.7 · Extends doc 09 (core capabilities) and doc 10 (proposal).
> Answers stakeholder questions 1–3, 7 and 8 of this round. Scenario detail (question 6) is in doc 12.
> Vendor figures (prices, accuracy) come from secondary sources and must be verified before use **[verify]**.

---

## 0. Are these capabilities included? (questions 1–3)

| Capability | Status before this round | What changes now |
|---|---|---|
| **1. Data collection (forms)** | **Included** at concept level (doc 06 §2.6 collection campaigns, doc 07 §1 field widgets, doc 09 C7) | Full form-builder spec in §1: record pickers from data tables, image/file upload, repeating line items, AI document reading, public and internal modes |
| **2. Output: alerts, push, email, SMS, IM** | **Partly included**: in-app notifications, email (SMTP) and webhooks were specified. **Browser push, mobile push, SMS, chat apps and delivery tracking were missing** | New core capability **C13 Notification Center** (§3): channels, templates, recipients, preferences, quiet hours, consent, delivery tracking, per-country SMS rules |
| **3. Data rules (conditions → outputs)** | **Partly included**: workflow triggers, report alert rules and lifecycle guards existed separately | New core capability **C14 Rules** (§2): one "When / If / Then" rule model for validation, reactions and alerts, with throttling, quiet hours, escalation, a simulator and a run log |

Core capabilities are now **14** (doc 09 C1–C12 + C13 Notification Center + C14 Rules). The visual
workflow/approval designer (§4) is part of C6 Process.

### 0.1 Synchronous vs asynchronous: one product rule

| Operation | Mode | User experience |
|---|---|---|
| Saving a record or form, validation rules, permission checks | **Synchronous** | Instant result; errors shown on the field |
| Lifecycle transition, approval action | **Synchronous** decision + **asynchronous** follow-ups | "Approved" shows immediately; notifications and actions follow |
| Notifications (all channels) | **Asynchronous** (queued, retried) | Delivery status visible on the record |
| Reaction rules, workflow steps, HTTP actions to other systems | **Asynchronous** | Progress and result in the record's activity, failures become exceptions |
| AI on a single short text (classify, short draft) | Asynchronous, usually returns within seconds | Shown as a suggestion when ready; saving never waits for AI |
| AI on documents, batch AI, imports, exports, source syncs | **Asynchronous jobs** | Progress bar, notification on completion, downloadable result/error file |
| Push into a table (inbox) | Sync up to 500 records, async above (doc 07 §4.3) | Per-record results or job status |

**Principle:** a person's action is never blocked by anything slow or external. Slow work becomes a
visible job, and every failed job becomes an exception with an owner.

---

## 1. Data collection: form builder (C7, expanded)

### 1.1 Building forms
- **Visual designer:** drag fields from the target table (or create new fields inline), arrange them into sections and pages, and preview them on desktop and phone.
- **Field widgets** from the field framework (doc 07 §1): text, number, date, dropdown, multi-select, radio, checkbox group, cascader, tree picker, toggle, rating, signature, location, barcode/QR scan, file and image upload.
- **Conditional logic:** show or hide fields, sections or pages; make fields conditionally required. Rules are written in the same "If" builder as C14.
- **Calculated fields** (e.g., line total = qty × price) and read-only display fields.
- **Repeating sections (line items):** e.g., expense lines or returned items. Each line becomes a **child record** linked to the parent submission.
- **Multi-table submission:** one form can create a header record plus child records in other tables.
- **Translations** of labels and help texts (en first; ja/ko/de/hi ready).

### 1.2 Integrating existing data into forms (example requested)
| Feature | Example |
|---|---|
| **Prefill from the signed-in user** | Name, email, department, cost centre taken from the user/people table |
| **Pick a record from a data table** | "Asset" picker shows only assets in the user's location; "Order" picker shows the customer's orders. It searches, shows key fields and respects business permissions |
| **Look up and verify** | Public return form: customer enters order number + email; the form checks it against the Orders source table before showing items |
| **Prefill from URL / previous submission** | Links that carry a store ID; weekly report pre-filled with last week's values |
| **Reference data pickers** | Category trees, reasons, regions from shared code lists, so values are consistent |

### 1.3 Images and files
- Multiple files per field, drag-and-drop, paste, **phone camera capture**, reordering.
- Type and size limits per field; **automatic compression** of large photos; **location metadata (EXIF) removed** by default for privacy.
- Previews for images and PDFs; virus-scan hook; storage quota per tenant.
- Large uploads continue in the background and resume after network interruptions.

### 1.4 AI document reading in forms (third-party AI APIs)
Flow: user uploads a receipt, invoice, contract, CV or ID document → **"Reading your document…"**
(asynchronous) → fields are filled as **suggestions with highlighted source areas** → user confirms or
corrects → submit. If AI is off, slow or fails, the user simply types the values.

| Provider type | Examples | Good for |
|---|---|---|
| Cloud document AI with prebuilt models | Azure AI Document Intelligence, Google Document AI, AWS Textract (incl. expense analysis), Mindee | Receipts, invoices, IDs, tables. Prices reported at roughly $1.50 per 1,000 pages for plain OCR and $10–50 per 1,000 pages for structured extraction **[verify]** ([comparison](https://imagetotable.ai/blog/google-vs-aws-vs-azure-ocr-2026), [overview](https://invoicedataextraction.com/blog/aws-textract-vs-google-document-ai-vs-azure-document-intelligence)) |
| Vision-capable LLMs with structured output | Claude, GPT, Gemini (JSON-schema outputs) | Unusual documents, custom fields, multilingual documents |
| Self-hosted OCR | Tesseract, PaddleOCR (+ local LLM) | Air-gapped/self-hosted customers; Azure also offers disconnected containers **[verify]** ([Nutrient](https://www.nutrient.io/blog/azure-document-intelligence-alternatives/)) |

Product rules:
- The admin picks a provider **per document type** (receipts → provider A; contracts → provider B).
- All providers return one **standard extraction result** (field, value, confidence, source location).
- Values are checked by field rules and code lists.
- **Fields below the confidence threshold are highlighted for review.**
- Extraction APIs return data only, so the review, validation and audit layer is ours, as the comparisons note ([Nutrient](https://www.nutrient.io/blog/google-document-ai-alternatives/)).

### 1.5 Access, distribution, protection
- **Modes:** internal (signed in), public link, invited people only, guests (suppliers/candidates see only their own submissions).
- **Options:** one response per person, edit after submit until a deadline, draft save, multi-step with progress.
- **Distribution:** link, QR code, embed on a website, email invitation, collection campaign (doc 06 §2.6).
- **Protection:** rate limits, optional CAPTCHA on public forms, file-type checks, **consent notices** (GDPR/DPDP/PIPA/APPI) with stored consent text version.
- **After submit:** confirmation page/email with a copy, record created, **rules run** (§2), status link for the submitter to track progress.

---

## 2. Rules: "When / If / Then" (new core capability C14)

### 2.1 One rule model

```
WHEN  (trigger)      record created · field changed · status changed · form submitted ·
                     date reached (e.g. 3 days before due) · schedule · metric crosses threshold ·
                     nothing happened for N days · source sync failed · external event received
IF    (conditions)   field comparisons, AND/OR groups, "changed from → to", in code list / subtree,
                     compared with a setting (PARAM), time windows, counts/aggregates
THEN  (actions)      notify (any channel) · create task / exception · update fields · start approval ·
                     call another system · run an AI skill · block save with message (validation rules)
WITH  (policies)     throttle / once per record · digest · quiet hours · escalation chain ·
                     priority · active period · owner
```

### 2.2 Three rule kinds

| Kind | Runs | Example |
|---|---|---|
| **Validation rule** | Synchronously, before save; can **block** or **warn** | "Expense amount above the per-diem limit (setting) needs a justification" |
| **Reaction rule** | Asynchronously, after save | "When a 1-star review arrives, notify the after-sales lead in the store's region and create a case" |
| **Alert rule** (on aggregates) | On schedule or on change of the underlying data | "If the 7-day return rate of any product > 8%, alert the category manager and open a task" |

### 2.3 Policies that make rules usable in real operations
- **Throttle/deduplicate:** once per record per N hours, so the same alert doesn't fire repeatedly.
- **Digest:** collect into a daily or hourly summary instead of single messages.
- **Quiet hours** in the recipient's time zone (US SMS rules require them, §3.3), with an urgent override.
- **Escalation:** if not acknowledged in X minutes, notify the next person or group.
- **Loop protection:** a rule cannot retrigger itself endlessly (depth and rate limits).
- **Ownership & versioning:** every rule has an owner; changes are versioned; critical rules can require approval to change.

### 2.4 Building and trusting rules
- **Visual rule builder** that reads as a sentence: *When [Review] is created, If [Rating] ≤ 2 and [Channel] in [Marketplace], Then notify [Region lead] via [Chat, Email].*
- **Simulator:** "test this rule against the last 100 records" shows which records would fire and what would be sent, before it is switched on.
- **Run log:** every evaluation that fired, with actions and delivery results, linked to the record trace.
- Rule templates come with each scenario package.

---

## 3. Notification Center (new core capability C13)

### 3.1 Channels

| Channel | Use | Notes |
|---|---|---|
| **In-app inbox** | All notifications; badges; mark done | Always on; central history |
| **Browser push** | Urgent items while not on the page | Opt-in per browser |
| **Mobile push** | Later, with the mobile app | Until then: email/chat |
| **Email** | Notifications, digests, external recipients (customers, candidates) | Own SMTP or providers; templates; bounce handling |
| **SMS** | Urgent internal alerts; external transactional messages (e.g., appointment, return status) | Through provider adapters (e.g., Twilio, Vonage, Infobip) **[verify]**; country rules §3.3; cost controls |
| **Chat apps** | Team alerts and approvals in the tools people already use | Slack, Microsoft Teams, Lark/Feishu; market-relevant channels: LINE (JP), KakaoTalk business messages (KR), WhatsApp Business (IN) via providers **[verify each]** |
| **Webhook** | Machine recipients | Signed, retried (doc 09 §3.3) |

### 3.2 Features
- **Templates** with fields from the record, per channel and per language; preview with a sample record.
- **Recipients:** a person field, the record owner, holders of a business capability within scope ("refund approvers for Kanto"), groups, external contact fields (customer email/phone), fixed addresses.
- **User preferences** per category and channel. Some categories are mandatory (e.g., approval requests). There is also a do-not-disturb setting.
- **Fallback chain:** e.g., push → chat → email → SMS if not delivered or read within N minutes.
- **Delivery tracking:** queued → sent → delivered → read (where the channel supports it) → failed, shown on the record and in reports. Failures after retries become exceptions.
- **Actionable notifications:** approve/reject or acknowledge directly from email or chat. Each link is signed, single-use and permission-checked.
- **Cost and volume controls** for SMS: per-tenant budget, rate limits, monthly report.

### 3.3 Per-market messaging rules (built into SMS/external messaging)

| Market | Rule highlights | Product behaviour |
|---|---|---|
| US | TCPA: prior express written consent for marketing texts; penalties per message; **A2P 10DLC brand/campaign registration**, otherwise carriers block traffic; quiet hours commonly 8 am–9 pm recipient time; stricter state laws ([Infobip TCPA guide](https://cdn-web.infobip.com/uploads/markdown-exports/www.infobip.com/blog/tcpa-compliance-sms.md), [MessageCentral US](https://www.messagecentral.com/sms-guideline/usa)) | Consent registry per external contact; STOP/opt-out handling; quiet hours enforced; registration guidance in admin |
| India | TRAI TCCCPR (amended Feb 2025): **DLT registration of sender headers and message templates** ([Infobip global guide](https://cdn-web.infobip.com/uploads/markdown-exports/www.infobip.com/blog/a-guide-to-global-sms-compliance-laws.md), [D7 India](https://d7networks.com/sms/india/sms-regulations/)) | Template IDs stored with SMS templates; only registered templates can be sent |
| Japan, Korea | Messaging commonly happens through LINE (JP) and KakaoTalk business notification messages (KR), which have their own template approval rules **[verify]** | Channel adapters with template registration fields |
| Germany/EU | GDPR consent and purpose limitation; email marketing typically needs double opt-in **[verify]** | Consent records with purpose; transactional vs marketing categories |

(Product research, not legal advice; counsel must review.)

---

## 4. Visual workflow & approval designer (part of C6)

Research: approval designers converge on a **simplified BPMN-like canvas**. Business users struggle
with raw BPMN ([Latenode thread](https://community.latenode.com/t/what-s-the-best-way-to-model-multi-branch-approval-flows-with-deadlines-and-manual-steps-in-a-no-code-builder/48829)).
DingTalk's designer gives **conditional branches a priority** (only the highest-priority matching
branch runs, with a fixed "other cases" fallback), while **parallel branches** run all matching paths
([DingTalk Yida](https://help.dingtalk.io/yida/process/yv8dnv)). Power Automate and SAP route by amount
and category with If/Else stages ([Microsoft](https://learn.microsoft.com/en-us/power-automate/guidance/business-approvals-templates/configure-preset-approvals),
[SAP](https://learning.sap.com/learning-journeys/create-processes-and-automations-with-sap-build-process-automation/using-conditions-to-control-the-process-flow_df45b353-142d-42a3-91e4-4c8d13a1aace)).
Creatio separates exclusive and parallel gateways ([Creatio](https://academy.creatio.com/docs/8.x/no-code-customization/bpm-tools/business-process-setup/process-designer-basics)).

### 4.1 Canvas building blocks

| Block | Settings |
|---|---|
| Start | Trigger: form submitted, status changed, button, rule, schedule |
| **Approval** | Approvers (person, field, **capability holders in scope**, manager chain if org module used); mode: one after another / all must approve / any one / at least N or %; timeout & escalation; allowed actions (approve, reject, send back, add approver, delegate, comment); fields editable or locked at this step |
| **Condition branches** | Ordered by priority; conditions with the "If" builder; mandatory **"Other cases"** default |
| **Parallel branches** | All run; join when all / any / N finish |
| Fill-in / task step | A person completes fields (e.g., finance enters payment date) |
| CC / notify | Recipients and channel (C13) |
| Automatic action | Update fields, create records, call another system, run AI skill (findings only) |
| Wait / timer | Until date, duration, or until a source status arrives (e.g., refund confirmed by shop) |
| Sub-process | Reusable fragment ("Finance review") |
| End | Outcome: approved / rejected / cancelled, with field updates |

### 4.2 Designer qualities
- **Validation while drawing:** unreachable blocks, missing default branch, approvers that resolve to nobody, editable fields that conflict with locks.
- **Simulation:** pick a sample record and step through it, seeing who would approve at each step.
- **Versions & publishing:** draft → test → publish. **Running requests keep their version.** A changelog and a diff between versions are available.
- **Templates** per scenario (expense, purchase, refund, offer, content, release).
- **Monitoring:** live view of running requests per step, average time per step, bottlenecks, and overdue steps (feeds dashboards and exceptions).

---

## 5. AI capability catalogue (question 8)

### 5.1 Maturity scale used below
- **Mature:** widely shipped as standard features; quality is predictable with modest tuning.
- **Established:** works well in production with per-customer configuration and review.
- **Emerging:** promising but needs strong guardrails; not for unattended decisions.

**Format note:** modern APIs can **guarantee the output format** (JSON-schema structured outputs via
constrained decoding) but **not correctness**. A valid category can still be wrong, so each AI skill
needs its own accuracy check ([constrained decoding overview](https://tianpan.co/blog/2026-04-12-structured-outputs-constrained-decoding-eliminating-parsing-failures-production-llm),
[structured outputs 2026](https://alexcloudstar.com/blog/structured-outputs-llm-developer-guide-2026/)).

### 5.2 Catalogue

| ID | Capability | Maturity | Input → output | Mode | Guardrails & how quality is measured | Used in |
|---|---|---|---|---|---|---|
| **A. Documents & images** |||||||
| A1 | Text recognition (OCR) | Mature | Image/PDF → text | Async | Confidence per word; manual fallback | All capture |
| A2 | Receipt / invoice extraction | Mature (prebuilt models) | Document → amount, date, vendor, tax ID, lines | Async | Field rules + format library (tax IDs, currencies); low-confidence fields flagged; accept rate per field | HR expenses, E-com supplier invoices |
| A3 | Custom document extraction (contracts, CVs, forms) | Established | Document → fields defined by the table schema | Async | Output bound to field types/code lists; source highlight; reviewer accepts | Recruitment, HR, Marketing |
| A4 | ID document reading | Established, **restricted** | ID → name, number, expiry | Async | Admin opt-in; restricted fields; never for sensitive national IDs unless legally required | HR (optional) |
| A5 | Image tagging / classification | Established | Photo → tags from a code list (e.g., damage type, product category) | Async | Restricted to code-list values; reviewer check | E-com returns, New Media assets |
| **B. Text understanding** |||||||
| B1 | Classification into a code list (routing) | Established | Text → category, priority, language | Async (seconds) | Confidence threshold → auto vs human triage; accuracy from accept/override rate | E-com cases, R&D bugs, HR questions |
| B2 | Information extraction from messages | Established | Email/chat → contact, order no., request type | Async | Field validation; suggestion mode | E-com, Marketing leads |
| B3 | Sentiment & urgency | Mature | Text → sentiment, urgency | Async | Used for priority only, never for decisions about people | E-com reviews, New Media comments |
| B4 | Summaries | Mature | Record history, thread, feedback → short summary | Async | Shows sources; labelled "AI summary" | All |
| B5 | Translation | Mature | Text → target language | Async | Glossary from code lists; human check for external content | E-com replies, Marketing copy |
| B6 | Duplicate / similar detection | Established | Record → likely duplicates with scores | Async | **Human confirms merge**; research reports accuracy from the high 70s to low 90s % on bug datasets, and simple search baselines are competitive ([arXiv 2504.14797](https://arxiv.org/html/2504.14797v1), [How far are we](https://ar5iv.labs.arxiv.org/html/2212.00548)) | Recruitment candidates, R&D bugs, HR assets |
| B7 | Personal-data detection & masking | Established | Text/attachments → PII flags | Async | Flags only; admin policy decides masking | All (privacy) |
| **C. Generation** |||||||
| C1 | Reply & message drafts | Mature | Record + template + tone → draft | Async | **Human sends** (default) or auto-send for approved low-risk templates | E-com, Recruitment, HR |
| C2 | Content variants (titles, copy, scripts) | Mature | Brief → variants per platform/locale | Async | Brand rules check (B1 against prohibited-claims list) | New Media, Marketing |
| C3 | Structured drafting (job descriptions, briefs, release notes) | Mature | Record fields → document | Async | Human edit/approve | Recruitment, Marketing, R&D |
| **D. Data & integration** |||||||
| D1 | Field mapping suggestions | Established | Sample payload/file → field mapping | Interactive | Mapping tested on samples before enabling | All data sources/imports |
| D2 | Value mapping to code lists | Established | "F", "女性", "weiblich" → code | Interactive | Unmatched → review queue | All |
| D3 | Data quality anomalies | Mature (statistical) | Records → outliers, missing owners, odd values | Scheduled | **Statistical methods, not LLM**; creates exceptions | All |
| D4 | System builder (schema from description/spreadsheet) | Established | Description/file → tables, fields, code lists, flows (preview) | Interactive | Preview & confirm; only platform building blocks | AI point ① |
| **E. Process assistance** |||||||
| E1 | Pre-check before approval | Established | Request + attachments + policy → findings list | Async | **Findings only, never approve**; deterministic policy rules run first | HR expenses, E-com refunds, Marketing content |
| E2 | Knowledge Q&A with handover | Established | Question + policy documents → answer with sources, or handover to owner; unanswered questions logged as tasks | Interactive | Answers cite sources; "I don't know" → human | HR policy desk, E-com support |
| E3 | Task and checklist generation | Established | Case/onboarding context → tasks | Async | Human confirms | HR onboarding, R&D |
| **F. Analytics & dashboards** |||||||
| F1 | Questions on official numbers (bounded planner) | Established when bounded (doc 06 §4) | Question → plan over certified metrics → numbers | Interactive | Asks when unclear; shows the plan | AI point ⑤ |
| F2 | Metric anomaly alerts | Mature (statistical) | Metric series → anomaly + alert rule | Scheduled | Seasonality-aware statistics; feeds C14 alerts | AI point ④ |
| F3 | Dashboard narrative | Established | Dashboard data → short summary | On refresh | Numbers come from the engine; text only explains them | AI point ④ |
| F4 | Dashboard from a goal | Established | "Track refund SLA by region" → dashboard draft | Interactive | Preview & confirm | AI point ④ |
| **G. Agents** |||||||
| G1 | Event-driven agents (listen → summarise → draft → act after approval) | **Emerging** | Events → proposed actions | Async | Capability grants, budgets, human approval for actions; later release | Later |

### 5.3 What we deliberately do not offer (not reliable or not acceptable today)
- AI that **approves, rejects or ranks people** (candidates, employees); employment AI is high-risk under the EU AI Act, and NYC requires bias audits.
- Free-form questions over any table without certified metrics, or answers whose numbers come from the model.
- Auto-sending external messages without a human, except for approved low-risk templates.
- Unattended agents changing data, before the agent release with full governance.

### 5.4 "AI skill": the product object customers configure
Every AI use is an **AI skill**, configured without code and reusable across scenarios:

| Setting | Example (E-commerce "Review triage") |
|---|---|
| Capability | B1 classification + B3 urgency |
| Input | Review text, rating, product |
| Output | Field *Issue type* (code list "after-sales issue"), field *Urgency* |
| When it runs | On new review (rule) / on button / on schedule |
| Provider & model | Chosen from those allowed for this data's privacy class |
| Instructions & examples | Short instructions + 10 labelled examples (versioned) |
| Confidence threshold | ≥ 0.8 → fill directly; below → suggestion for a person |
| Budget | Max cost per month; cost preview for batch runs |
| Quality | Test set score at publish; live accept/override rate; alert if it drops |

**AI's own loop is closed:** people's accept/override decisions feed the quality dashboard, and a falling
rate raises an exception for the skill owner.

### 5.5 Mapping to the five AI points (doc 08)

| AI point | Catalogue items |
|---|---|
| ① AI-powered systems | D4 (+ D1/D2 when built from files or databases) |
| ② AI for data tables | A1–A5, B1–B7, C2–C3, D1–D3 |
| ③ AI-driven workflows | B1 routing, C1 drafts, E1 pre-check, E2 Q&A with handover, E3 tasks, (G1 later) |
| ④ AI-powered dashboards | F2 anomalies, F3 narrative, F4 dashboard from goal |
| ⑤ AI analytics | F1 bounded questions, F3 explanations |

### 5.6 Privacy, cost, compliance (applies to all skills)
- Each field's privacy class decides which providers may receive it. Self-hosted models are possible for sensitive data.
- No customer data is used for model training (provider terms checked).
- AI-generated values are labelled, and their source and history are kept.
- A per-tenant monthly budget applies, with a cost preview before batch runs.
- The product works completely with AI switched off.

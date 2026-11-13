# Final AI Design (v1.0) — AI at the foundation

> **Status: authoritative AI specification for development.** Consolidates docs 05, 06 §4, 07 §6, 08 §4,
> 11 §5 and 15 §4, and aligns with Lark Base's AI capability set (doc 15 inventory; parity in doc 19).
> Companion to doc 17 (product design).

---

## 1. Positioning and principles

**AI's role in the product:** turn messy enterprise input into governed records, help people at every step of
a process, build and adapt systems from descriptions, and answer business questions from trustworthy
numbers. The deterministic core decides what is valid.

**Ten principles (binding for development)**
1. **Two AI objects only.**
   - **AI Skill:** stateless, typed inputs and outputs.
   - **Agent:** an AI principal with instructions, knowledge, tools, memory and budget.

   Every surface invokes one of them.
2. **AI never writes directly.** Outputs become **operations** or **suggestions** in the record pipeline (validate → capability check → journal).
3. **Dual identity.** Each run records the **AI principal** (skill/agent) and the **initiator** (user, rule owner or schedule owner). Effective rights are the intersection of both, and loss of either blocks the run with an exception.
4. **Provenance on every AI value:** model, skill/agent version, confidence, evidence, review state, trace.
5. **Reversible by design:**
   - Data changes can be undone in the same place where they were made.
   - Structural changes (schema, workflows, access) are always made in a **change set** (staging) before publishing.
   - No irreversible overwrites.
6. **Typed outputs.** Outputs are bound to field types and reference sets. Structured output formats are enforced, and content accuracy is measured separately.
7. **Permission-first context.** Models receive only the data the dual identity may read, and field classification decides which providers may receive which data.
8. **People decide on people and money.** AI never approves, rejects or ranks people. It produces findings, drafts and suggestions.
9. **Measured quality.** Every skill and agent has an evaluation set, a live accept/override rate, and alerts when quality drops.
10. **Optional by design.** Every capability works with AI turned off. AI can be enabled per tenant, base, table, field, skill and user.

---

## 2. Architecture

```
SURFACES  cell toolbar · AI field · form capture (paste/voice/document) · import & source mapping · workflow step ·
          approval pre-check · record page · dashboard · chat channels · comments · command bar (Assistant)
                │                                  │                                │
                ▼                                  ▼                                ▼
          ┌──────────────┐                 ┌──────────────┐                ┌───────────────────┐
          │  AI Skill     │◄──────uses─────│    Agent     │◄────uses───────│ Assistant & Builder│
          │ typed I/O     │                │ principal    │                │ (intent router,    │
          └──────┬───────┘                 └──────┬───────┘                │ bounded planner,   │
                 │                                │                        │ build pipeline)    │
                 ▼                                ▼                        └─────────┬─────────┘
        ┌──────────────────────────── AI RUNTIME ─────────────────────────────────────┴──────┐
        │ context assembly (permission-filtered, classification-gated) · prompt/version registry │
        │ structured output enforcement · tool execution (capability-checked) · budgets & metering│
        │ caching (input hash) · run log & trace · evaluation hooks                              │
        └───────────────────────────────┬────────────────────────────────────────────────────────┘
                                        ▼
        ┌──────────────────────────── MODEL GATEWAY ─────────────────────────────────────────────┐
        │ provider adapters · model registry · routing (task, privacy, region, cost) · fallback  │
        │ rate limits · kill switch                                                              │
        └────────────────────────────────────────────────────────────────────────────────────────┘
KERNEL (unchanged authority): Rules · Workflow engine · Record operation pipeline · Field framework ·
Reference data · Capabilities · Lifecycle · Change sets · Journal & audit
```

**Domain D13 objects:** Provider, Model, Skill, SkillVersion, Agent, AgentVersion, Knowledge, Tool, AiRun,
Suggestion, Budget, UsageRecord, EvalSet, EvalResult, Conversation (Assistant).

---

## 3. Model gateway

| Topic | Design |
|---|---|
| Provider adapters | LLMs: Anthropic Claude; OpenAI-compatible endpoints (covers many vendors and **self-hosted vLLM/Ollama**); Azure OpenAI; Google Gemini. Document AI: Azure Document Intelligence, Google Document AI, AWS Textract, Mindee. OCR fallback: self-hosted (Tesseract/PaddleOCR). Embeddings: provider or local (pure-Rust inference option). Speech-to-text: provider (R2) |
| Model registry | Per model: capabilities (structured output, tool use, vision, audio, max context), regions, privacy approval (allowed classifications), unit prices, latency class |
| Routing | Policy chooses the model per **task class** (classify, extract, generate, reason, vision, embed), **privacy class** of the input, **tenant region**, **cost tier**. Admin can pin models per skill/agent. An "Auto" default works like Lark's Auto model selection |
| Structured output | JSON-schema outputs generated from field types/reference sets; constrained decoding where the provider supports it; validation and one repair retry otherwise |
| Reliability | Timeouts per task class, retries with backoff, provider fallback list, circuit breaker, **kill switch** per tenant/provider |
| Caching | Cache key = hash(inputs, skill version, model, parameters) → identical calls reuse results (cost and consistency) |
| Metering | Tokens, pages, images, audio seconds → **credits** (configurable price table); self-host with own keys uses provider prices |
| BYO keys | Tenant-level provider credentials (secrets); instance default optional |

---

## 4. AI Skills

### 4.1 Skill definition (configurable object)
| Field | Description |
|---|---|
| id, name, description, owner, status, version | Governance |
| capability | One catalogue capability (§4.2) or "custom" |
| inputs | Fields (incl. attachments), constants, parameters (`PARAM()`), related-record fields via links |
| outputs | Target field(s) with types; select/reference outputs bound to a **reference set** (values restricted); JSON → several fields |
| instructions | Short task description (versioned), output rules, tone, glossary (from reference data) |
| examples | Labelled input → output pairs (min 3 recommended; used as few-shot and as an eval set seed) |
| model policy | Auto / pinned model; allowed providers by privacy class |
| trigger | Manual (button, toolbar) · on create · on change of selected input fields (rule) · schedule · batch job · workflow step |
| write policy | `suggest` (default for Confidential/Personal fields and for bulk) · `commit_if_confidence ≥ threshold` · `commit`; **never overwrite human-entered values without confirmation** |
| scope | Whole table / view / selection / first N rows |
| budget | Per run limit, monthly cap; cost preview for batches |
| quality | Eval set, ship threshold, live accept rate, alert threshold |

### 4.2 Skill catalogue (v1–v3)
Maturity: **M** mature · **E** established · **X** emerging. Lark equivalent in brackets.

| ID | Skill | Input → Output | Mat. | Default write | Release |
|---|---|---|---|---|---|
| **Text** ||||||
| T1 | Classify into code list (classification shortcut / AI classify node) | text → ref value(s) + confidence | E | commit ≥ threshold | R1 |
| T2 | Tag (smart tags) | text → multi ref values | E | suggest | R1 |
| T3 | Extract fields (information extraction) | text → typed fields | E | suggest | R1 |
| T4 | Summarise (summary, record summary) | record/thread/text → summary | M | commit (AI-labelled field) | R1 |
| T5 | Translate (translate; 12+ languages incl. ja/ko/de/hi) | text → text in target locale | M | commit (separate field) | R1 |
| T6 | Polish / rewrite (improve, grammar, expand, shorten) | text → text | M | suggest | R1 |
| T7 | Sentiment & urgency | text → ref values | M | commit ≥ threshold | R1 |
| T8 | Personal-data detection | text/attachment → PII flags | E | flag only | R1 |
| T9 | Custom autofill (custom AI autofill) | any inputs + instructions → typed output | E | suggest | R1 |
| T10 | Similar / duplicate detection | record → candidates + scores | E | suggest merge | R2 |
| T11 | Draft message/reply | record + template → draft | M | suggest (human sends) | R1 |
| T12 | Content variants (titles, copy) | brief → N variants | M | suggest | R2 |
| T13 | Structured drafting (JD, brief, release notes) | fields → document | M | suggest | R2 |
| **Documents & media** ||||||
| D1 | OCR (general OCR) | image/PDF → text | M | commit (derived text) | R1 |
| D2 | Receipt & invoice extraction | doc → amounts, dates, vendor, tax IDs, lines | M | suggest | R1 |
| D3 | Custom document extraction (contracts, CVs) (PDF parsing, entity recognition) | doc → table-defined fields with source highlight | E | suggest | R1 |
| D4 | Image understanding & tagging (image understanding) | image → tags/description | E | suggest | R2 |
| D5 | Speech to text (speech recognition) | audio → text | M | commit (derived) | R2 |
| D6 | Image generation (optional, admin-enabled) (image generation) | prompt → image attachment | E | suggest | R3 |
| **Data** ||||||
| A1 | Field mapping suggestion | sample payload/file → mapping | E | suggest (spec) | R1 |
| A2 | Value mapping to reference data | external values → ref items | E | suggest; unmatched → quarantine | R1 |
| A3 | Formula from description (AI generate formula) | text → formula + explanation | E | suggest | R1 |
| A4 | Option/code-list generation (AI generate options) | context → candidate items | E | suggest | R1 |
| A5 | Quality anomalies (statistical) | table → anomalies | M | exceptions | R2 |

### 4.3 Surfaces that invoke skills
| Surface | Behaviour | Lark reference |
|---|---|---|
| **Cell toolbar** | On a selected cell: Fill, Polish (improve/grammar/expand/shorten), Translate, Summarise, Extract, Tag, Ask. Result → replace / insert / copy / regenerate / feedback | Smart toolbar |
| **AI field** | Field whose producer is a skill; shows provenance, confidence, review state; batch generation as an async job with progress and cost preview | AI field shortcuts |
| **Form capture** | **Paste** (unstructured text → fields), **voice** (speech → fields), **document** (upload → fields with highlights); user confirms | AI paste entry, voice entry |
| **Import & data sources** | Mapping and value-mapping suggestions; quarantine explanations | (none comparable) |
| **Workflow step** | Classify (2–10 branches), extract, generate text, any skill; typed outputs as variables | AI classify / generate text / node shortcuts |
| **Approval & lifecycle** | Pre-check findings shown to approvers; triage routing | (none comparable) |
| **Record page** | Record summary, "what changed", related-record suggestions | Record summary |
| **Dashboard** | Chart insight, dashboard summary | Smart chart analysis, dashboard summary |

### 4.4 Review & provenance UX
- AI values show an **AI badge**. Hovering shows the model, skill version, confidence, evidence links and time.
- Suggested values appear as **ghost values** with Accept / Edit / Reject. Bulk review queues show items "to review" per field.
- Filters: "AI-filled", "AI not yet reviewed", "low confidence".
- Accept/override actions feed the skill's quality metrics.

---

## 5. Assistant & Builder (one entry point)

### 5.1 Assistant (command bar + sidebar)
- **Intents:**
  - **Build:** system, table, form, page/app, dashboard, workflow, rules, approvals, access.
  - **Configure:** change existing objects.
  - **Ask:** data questions.
  - **Find:** templates, connectors, help.
  - **Explain:** a record's history and trace, a permission, a rule run, a formula, a number.
- **Inputs:** text, @mentions (tables, fields, people, groups), file upload (spreadsheets, documents), voice.
- **Context:** current base/table/view/selection; permission-filtered.
- **Sessions:** per user, private, last 50 kept (Lark parity); new session; rename/delete.
- **Every action shows its plan, a progress log, stop/continue, and undo** (data) or **discard change set** (structure).

### 5.2 Build pipeline (applies to every Build/Configure intent)

```
Describe ─► Clarify (questions if ambiguous) ─► Plan
   │                                             │  Standard mode: concise plan
   │                                             │  Expert mode:   system design document (business goals, data assets,
   │                                             │                 processes, lifecycles, access, metrics) for review
   ▼                                             ▼
Alternatives (1–3 options for access/process design) ─► Apply into a CHANGE SET (staging) ─► Verify
   ─► Verify:  schema valid · loop-health checklist (owners, SLA, terminal states, drill-downs) ·
               access preview per capability set · sample data walk-through · template tests if any
   ─► Review diff ─► Publish ─► (Rollback available)
```

- **Order of building** (as Lark): data model → reference data → lifecycles & approvals → rules/workflows → dashboards → forms/pages → access.
- **Builds only from platform components** (K1–K23 and field types). It never produces custom code.
- **Sources for building:** description, uploaded spreadsheet, database schema (data source), sample API payload, policy/process document, existing template (adapt).
- **Expert-mode design document** is stored as the base's system documentation and updated on later AI changes.
- **Destructive operations** (delete table/field, remove capability) always require explicit confirmation inside the change set.

### 5.3 Ask: governed data Q&A (bounded planner)
- **Pipeline** (doc 06 §4):
  1. **Ground:** phrases → tables, fields, reference values, people, metrics, periods.
  2. **Select:** the LLM fills one plan template using enumerated IDs only.
  3. **Compile & check:** in Rust, including permissions.
  4. **Execute deterministically.**
  5. **Narrate** with citations.
- **Plan templates (R1 → R2):**

| Template | Example | R |
|---|---|---|
| find_records | "Open cases in Osaka older than 3 days" → filtered view | R1 |
| aggregate | "Total refunds this fiscal month by reason" | R1 |
| metric_query | Certified metric × dimensions × period | R1 |
| compare_periods | MoM / YoY / vs target | R1 |
| top_n / ranking | "Top 5 SKUs by return rate" | R1 |
| breakdown / drivers | "Why did sales drop 15%?" → decomposition by dimensions, ranked contributions | R2 |
| forecast | Statistical forecast (labelled method, confidence band) on a metric series; **never model-invented numbers** | R2 |
| report | Daily/weekly report from a report template (metrics + narrative) | R2 |
| bulk_update / create_records | Through the same confirmation and change rules | R1 |

- **Outputs:** text, table, chart; "open as view/dashboard"; the plan is always visible.
- **Ambiguity** → asks a question with candidates. **Out of scope** → says so.

### 5.4 Capture assistant (data entry)
- **Paste:** text (email, chat, notes) → record fields; multiple records from a list.
- **Voice:** speech → form answers (R2).
- **Document:** upload → fields with highlights (skills D1–D3).
- **Email-in / chat capture:** messages to a table address or bot → records (via data source inbox + skills).

---

## 6. Agents

### 6.1 Agent definition
| Field | Description |
|---|---|
| id, name, avatar, owner, status, version | Governance; agent is a **principal** (D1) |
| instructions | Markdown role, abilities, rules, tone; @references to knowledge, tools, people, groups |
| knowledge | Bases/tables/views (permission-filtered), knowledge articles (K17), uploaded files (PDF, Word, Excel, PPT, CSV, TXT, MD, images); optional embeddings index per knowledge source |
| tools | Built-in tools (§6.2) + **MCP tools** (external servers, admin-approved) + skills |
| memory | Session context; optional long-term memory per user (visible, deletable by the user; off by default) |
| triggers & schedules | **Defined as rules (C14)**: schedule, record created/first meets condition/changed/deleted, comment received, chat message received, button. One rule system; agent is the action |
| entry points | Assistant sidebar, app sidebar, record page, chat channels (bot in direct and group chats via C13 adapters), comments, workflow step, AI field, dashboard component |
| capability grants | What the agent may read and do (scopes, limits). Effective rights = agent grants ∩ initiator rights |
| action policy | Per tool: `auto` / `confirm` (user approves in chat or task) / `forbidden`. Defaults: reads auto; single-record writes confirm; bulk/destructive/external messages confirm; access changes forbidden |
| budget | Per run (tokens/steps/time 1–30 min), monthly cap |
| sharing | Use / edit / manage, granted to people or groups |
| quality | Eval conversations; feedback; alert thresholds |

### 6.2 Built-in tool catalogue
- **Search & read:** search records (find_records), read record, query metric (metric_query), read knowledge.
- **Write:** create/update records (operations, policy-gated), add comment, create task/exception, start approval, run workflow.
- **Communicate & produce:** send notification (template), generate document (K22).
- **Use skills:** call skill.
- **Optional, admin-enabled:** web fetch.

### 6.3 Run model
1. Trigger (rule) or conversation starts a run.
2. Context is assembled under the dual identity.
3. Plan → tool calls (each one checked against capabilities and the action policy) → confirmations where required.
4. Result (message, records, tasks).

- Each run is **isolated**. Workflow and field runs don't write into long-term memory (Lark behaviour adopted).
- Step and time limits apply. Every step is logged in the trace.
- **Failure handling:** permission loss, budget exhaustion, tool error or timeout → run marked failed → exception to the agent owner, with the reason shown to the initiator.
- **Field-bound agents** run as the agent principal with **the field's configured owner as initiator**. Lark instead uses the *last modifier*; we avoid that implicit rule.
- **Limits (configurable defaults):** 200 agents per tenant workspace, 30 schedules per agent (Lark: 200 per user, 30 schedules), 10,000 records per batch run.

### 6.4 Typical agents shipped with templates
- **Policy desk** (HR): answers from policy articles; hands over to HR; logs unanswered questions as tasks.
- **After-sales triage** (E-commerce): classify → route → draft reply → wait for agent approval.
- **Sales/ops weekly reporter:** a schedule that builds the weekly report from certified metrics and posts it to a chat channel.
- **Data steward:** nightly duplicates, unmapped values, missing owners → exceptions.
- **Content assistant** (New Media): variants, disclosure checks, comment replies (drafts).

---

## 7. AI across core capabilities (matrix)

| Capability | AI functions | Objects used |
|---|---|---|
| C1 Tables & records | Cell toolbar, record summary, related-record suggestions, AI fields | Skills |
| C2 Fields | Type suggestion from sample, option/code list generation, formula from description | Skills A3, A4 |
| C3 Reference data | Candidate items from free text values, alias suggestions for crosswalks | Skills A2, A4 |
| C5 Access | Capability map proposal from statuses/actions; permission plans (1–3 alternatives); explain access | Assistant (configure/explain) |
| C6 Process | Triage & routing, pre-check findings, task/checklist generation, SLA risk prediction (statistical, R3) | Skills, agents |
| C7 Forms & capture | Paste, voice, document capture; form generation from description | Skills, Assistant |
| C8 Connections | Field and value mapping, drift explanation, quarantine resolution suggestions | Skills A1, A2 |
| C9 Dashboards | Dashboard from goal, chart insight, dashboard summary, anomaly alerts → tasks, agent component | Assistant, skills, agents |
| C13/C14 Rules & notifications | Rule from description (via change set), message drafts, digest summaries | Assistant, skill T11 |
| C14 Workflows | AI steps (classify with branches, extract, generate, call skill, call agent) | Skills, agents |
| Templates | Builder adapts or creates templates; expert-mode design docs | Assistant |
| Admin | AI usage analytics, quality dashboards, anomaly in usage | — |

---

## 8. Governance, privacy and compliance

| Area | Design |
|---|---|
| Enablement | AI on/off at tenant, base, table, field, skill, agent; per-user opt-out of the cell toolbar (Lark parity) |
| Privacy classes | Field classification → allowed providers/models; Personal/Sensitive → self-hosted or approved providers only; redaction option for prompts |
| Data residency | Provider endpoints per tenant region; requests never leave the configured region unless allowed |
| Prompt/response retention | Configurable (off / 30 days / 1 year), redacted, accessible to admins for audit |
| Transparency | AI badges; AI-generated content labelled; external messages from AI drafts carry no hidden automation unless policy allows |
| Restricted uses | No AI ranking, scoring or rejection of people (EU AI Act high-risk employment; NYC LL144); recruitment template locks those skills |
| Security | Prompt-injection defences: data fenced as untrusted, tool allow-lists, capability checks on every tool call, confirmation for writes, no secrets in prompts, outbound web fetch off by default |
| Audit | Every AI run in the hash-chained audit with both principals, inputs references, outputs, tools called, cost |
| Admin console | Providers & keys, model routing, budgets, usage by base/skill/agent/user, quality dashboards, incidents, kill switch |

---

## 9. Cost, quotas and attribution

- **Unit:** credits derived from metered tokens/pages/images/audio. The price table is set by the admin (self-host) or by the plan (cloud).
- **Budgets at five levels:** tenant → base → skill/agent → user (optional) → per run. Warnings at 80%; the hard-stop policy is configurable (stop or downgrade the model).
- **Attribution:** cost is charged to the **budget of the skill/agent's scope**, with the **initiator recorded** for reporting. This is one consistent rule, unlike Lark's split between the user (agents) and the base owner (everything else).
- **Batch runs** show a cost estimate first. Re-running identical inputs uses the cache (no extra cost). Lark charges for regeneration.

---

## 10. Quality & evaluation
- **Skills:**
  - The eval set comes from examples plus labelled production samples.
  - **Ship gate** per skill (e.g., classification accuracy ≥ 90% on the eval set **[target per template]**).
  - The live accept rate is tracked; a drop of 10 points or more → exception to the owner.
- **Planner:** a 300-question benchmark over template bases; ≥ 95% *safe* and ≥ 85% *correct first time* to enable a template (doc 06 §4).
- **Builder:** generated systems must pass schema validation, the loop-health checklist and template tests in the change set.
- **Agents:** scripted conversation tests per shipped agent; tool-policy tests (no forbidden tool ever called); budget tests.
- **Nightly runs:** recorded/mock providers for deterministic CI; optional live-model evals.
- **Coverage rule:** all AI logic except the model call is deterministic Rust and falls under the coverage gates (doc 04).

---

## 11. Performance targets (AI)
| Operation | Target |
|---|---|
| Classification / short generation (interactive) | p50 < 2 s, p95 < 5 s |
| Document extraction | p95 < 30 s per document (async, progress shown) |
| Batch AI field | ≥ 20 records/s per tenant (provider-limited), resumable job |
| Assistant plan (build) | First plan < 15 s; build progress streamed |
| Agent run | Default timeout 10 min (range 1–30 min) |
| Saving a record | Never waits for AI |

---

## 12. Release scope (AI)
| Release | AI scope |
|---|---|
| **R1** | Gateway (Claude, OpenAI-compatible incl. local, one document-AI provider); skills T1–T9, T11, D1–D3, A1–A4; cell toolbar; AI fields; form paste & document capture; workflow AI steps (classify with branches, extract, generate); approval pre-check; Assistant: build (tables, fields, reference data, lifecycle, rules, dashboards, forms), configure, find, explain; bounded planner R1 templates; agents with knowledge, built-in tools, rule triggers, chat channel (one adapter), budgets; governance console; evaluation framework |
| **R2** | T10, T12, T13, D4, D5, A5; voice capture; expert-mode design documents; permission plans with alternatives; planner drivers/forecast/report; MCP tools for agents; long-term memory; dashboard insight & summary; more chat channels |
| **R3** | D6 image generation (optional); SLA risk prediction; agent marketplace/templates; cross-base agents |

## 13. AI acceptance criteria (definition of done)
1. No AI path writes outside the operation pipeline (verified by architecture tests).
2. Every AI value has complete provenance and can be undone where it was made.
3. Hidden fields and rows never appear in prompts (permission test suite with canary values).
4. Classification-gated routing is enforced (Personal data never reaches a disallowed provider).
5. Budgets stop runs at the configured limits; cost previews match actual usage within ±10%.
6. Disabling AI at any level leaves all features working without errors.
7. Ship gates per skill/template are met before release.

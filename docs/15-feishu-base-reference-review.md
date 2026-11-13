# Reference Review — Feishu/Lark Base help documentation (157 pages)

> Status: analysis v0.9 · Source: stakeholder-provided archive of 157 saved Feishu help-centre pages
> (last updated 2025-07 to 2026-11), covering fields, views, forms, formulas, dashboards, automation,
> workflows, AI features, agents, spaces, sandbox, limits.
> Purpose: **use as reference, do not copy.** Diagnose where their design is incoherent, extract what is
> worth learning, and show how our component layer (doc 13) with AI at the foundation handles each point.
> Quotes are paraphrased translations of the source pages.

---

## 1. What Feishu Base offers today (inventory)

### 1.1 Core
- **Structure:**
  - spaces → bases → data tables / dashboards / workflows / doc pages / folders (up to 300 objects per base);
  - views per table (grid, kanban, calendar, gantt, gallery, form, query page, independent shared view, protected view);
  - an "application mode" that composes several bases into one system with pages and roles.
- **Fields:**
  - ~30 types in three groups (regular, business, advanced): text, select, person, group, date, attachment, number, checkbox, link, formula, lookup;
  - **process field** (stages with "who can complete" and required fields per stage), button, auto number, phone, email, location, barcode, progress, currency, rating, signature;
  - created/modified by/at;
  - field shortcuts (AI and formula).
- **Formulas:** a large function library, including LET, MAP, LIST, FILTER, NETWORKDAYS and CurrentValue.
- **Automation:** "automation" (2–4 simple steps) **and** "workflow" (canvas, branches, loops, AI nodes). They share triggers, actions and the monthly run quota.
- **Data protection:** advanced permissions (roles with table/record/field/view/dashboard rights), Time Machine (retention by plan: 1 week to 1 year), **sandbox** (isolated copy, change items with dependencies, one-click update), archive tables (1M read-only rows).
- **Scale:** 2M rows per table, 1,000 concurrent editors (vendor claim), 300 fields per table, 100 formula + lookup fields per table, 200 views, 3 group levels, 50 filters, 20,000 attachments per table, import ≤ 20 MB.

### 1.2 AI surfaces (counted from the documents)

| # | Surface | What it does |
|---|---|---|
| 1 | **AI sidebar** ("Ask AI") | Modes: build table, build dashboard, configure permissions, build workflow, data analysis, plugin/connector search, tutorial Q&A |
| 2 | **AI build business system** | Standard mode (executes directly) or **expert mode (writes a system design document first, user confirms, then builds)**; order: tables → dashboards → workflows → permissions; auto-verification; undo |
| 3 | AI build application | Navigation, pages, components, roles across bases |
| 4 | AI build data table | Schema, views, sample data, links; asks before editing or deleting |
| 5 | AI custom page | Generated landing pages / catalogues bound to tables (≤ 20 tables) |
| 6 | AI configure advanced permissions | Creates roles and rights; **can propose 1–3 alternative plans**; cannot delete or rename roles; must be saved manually; saved permissions can't be undone |
| 7 | AI data Q&A & analysis | Retrieval, aggregates, attribution, **forecasting**, reports; results as text, table or chart |
| 8 | Smart Q&A (separate icon) | Q&A in a dialog, with its own settings |
| 9 | **Smart toolbar** on cells | Fill, polish, translate (12 languages), summarise, extract, tag, quick ask |
| 10 | AI generate formula | Natural language → formula |
| 11 | **AI field shortcuts** | Classify, translate, tags, extract, summarise, custom autofill; third-party: multimodal models, image/video generation, OCR, PDF parsing, speech; model choice; examples; auto-update; whole column or N rows |
| 12 | AI paste entry / AI voice entry | Unstructured paste → new record; voice → questionnaire |
| 13 | **Agents (智能体)** | Persistent assistants: instructions (Markdown), ≤ 30 schedules, triggers (comment received, record first meets condition, record changed, record deleted), tools/skills, knowledge (docs, uploads), long-term memory, entry points (bot, group chat, sidebar, app sidebar); ≤ 200 per user |
| 14 | Agent as field shortcut | Agent runs per record; **runs as the field's last modifier**; up to 10,000 rows per batch; writes extra read-only result fields |
| 15 | Agent node in workflow | Calls a persistent agent; **runs as the workflow creator**; isolated context; timeout 1–30 min |
| 16 | **AI Agent node** in workflow (different concept) | Inline LLM + memory + tools + **custom MCP tools**; prompt ≤ 10,000 chars; JSON output schema |
| 17 | AI classify node, AI generate text node, AI node shortcuts | Workflow/automation steps |
| 18 | Dashboard: smart chart analysis, dashboard summary, AI build dashboard, agent component | |
| 19 | AI quota | Points per call, model and cell (e.g., field shortcut cost × rows; re-generation costs again; image 5 points, video 40–50, PDF 0.5/page); **agent usage billed to the user, everything else to the base owner** |

**Count:** about 19 AI surfaces, **two different things called "agent"**, at least **five trigger systems**,
and **three run-identity rules**.

---

## 2. Diagnosis: why the design feels chaotic

| # | Problem (evidence from the docs) | Consequence for customers |
|---|---|---|
| D1 | **Two automation products with the same triggers and actions**: "automation" vs "workflow", with a dedicated page explaining how to choose; they share one quota | Users must learn two tools; migration between them is manual |
| D2 | **Five or more trigger mechanisms**: automation/workflow triggers; agent triggers (comment, first meets condition, changed, deleted); agent schedules; field-shortcut auto-update; button field. Each has its own semantics (e.g., "first time meets condition" vs "record meets condition"), filters and failure modes | Hard to know why something ran or didn't; the FAQ lists six reasons an agent may not trigger |
| D3 | **Two "agents"**: the persistent *智能体* and the workflow *AI Agent node* (inline model + tools + MCP). Plus default agents auto-created per user for field shortcuts and workflows | Duplicate concepts, sprawl of auto-created agents, unclear ownership |
| D4 | **Inconsistent run identity**: agent field shortcut runs as the *last person who edited the field configuration*; workflow agent node as the *workflow creator*; triggers fail when the *creator* loses permission or leaves | Permission leaks or silent failures; audit can't answer "on whose authority did AI change this?" |
| D5 | **Inconsistent safety**: AI field shortcut "overwrites existing column content and it cannot be recovered"; AI table edits must be undone through history/Time Machine, not where they were made; AI-built workflows and permissions need a manual save; saved AI permissions can't be undone | Users can't predict whether an AI action is reversible |
| D6 | **AI surfaces overlap**: sidebar data analysis vs "smart Q&A" dialog vs agent chat vs dashboard summary; smart toolbar vs field shortcuts vs AI paste entry | Same job, several entry points with different capabilities and costs |
| D7 | **Several page/UI concepts**: views, forms, query pages, independent shared views, protected views, custom record detail pages, dashboards, doc pages, app mode pages, AI custom pages | Permissions and layouts are configured separately for each |
| D8 | **Permission layers stack up**: space roles → file collaborator rights → advanced permissions (old/new) → page and app roles → agent sharing (use/edit/manage) → admin-console bot permissions | Effective access is hard to explain |
| D9 | **Process logic split across field, workflow and permissions**: the "process field" (stages, who can complete, required fields) is separate from status selects, workflow approvals and record permissions | Status logic lives in three places |
| D10 | **Reuse via workarounds**: shared option lists are done by referencing another table's select column; cascade selects rely on that source table's layout | Fragile reference data, no governance |
| D11 | **Low structural ceilings and side effects**: field type change converts only 20,000 rows (more → a new field); on tables over 100k rows, field changes are serialised and slow down others; 100 formula + lookup fields per table | Large customers hit limits mid-project |
| D12 | **Cost model is per surface and per cell**: different point prices per model and feature; re-generation is charged again; billing attribution differs between agents and other features | Hard to forecast or govern AI spend |

**Root cause:** AI and automation were **added feature by feature on top of a spreadsheet-like core**.
Each feature brought its own trigger, identity, undo, permission and billing semantics. There is no
shared kernel for *events*, *principals*, *operations* and *provenance*. This is the "AI as add-on"
pattern we decided to avoid (docs 05, 07).

---

## 3. What is worth learning (reference value)

| Their idea | Why it's good | Our adoption |
|---|---|---|
| **Expert mode for AI building:** design document → confirm → build → auto-verify → undo | Makes large AI changes reviewable | Builder (AI point ①) always follows **plan → preview diff → apply in a staging branch → verify (loop-health checklist + template tests) → publish**; the design document is kept as system documentation |
| **AI proposes 1–3 alternative plans** (permissions) | Users choose between trade-offs instead of accepting one answer | Bounded planner may return up to 3 candidate plans for build and config intents |
| **Sandbox** with change items, dependencies, one-click update, rollback via Time Machine | Safe change management for live systems | **Staging branch for tenant overlays** (doc 13 §5): change set with dependencies, test run, publish, rollback; AI structural changes always land in staging first |
| Agent configuration: instructions, schedules, triggers, tools, knowledge, memory, entry points | Complete agent anatomy | Our **Agent principal** gets the same anatomy, but triggers come from the shared rule engine and identity/budget are explicit |
| Structured output (JSON schema) for AI nodes | Reliable downstream use | Already in our AI skill spec; all AI outputs are typed and bound to fields and code lists |
| Field shortcut with examples + custom requirements + classification options from a referenced list | Practical prompt configuration | AI skill "instructions & examples" with outputs bound to **reference data sets** (not ad-hoc option copies) |
| **AI paste entry** (paste unstructured text → record) and **voice entry** in forms | High-value data capture | Added to capture (A/B catalogue): paste & voice capture as input channels of AI capture |
| Comment-triggered agent; agent in group chat | Meets users where they talk | Agent entry points: comments, chat channels (via C13 adapters), sidebar |
| **Query page** (look up and update own records without table access) | Strong pattern for external and frontline users | Portal pattern: form + status lookup + permitted edits, built from views + capability scopes |
| **Form detail table** (line items in a form) | Header-lines capture | Already K3 lines in forms (doc 11 §1.1) |
| Archive table (read-only, still referenceable) | Data lifecycle at scale | Archive tier in data lifecycle: read-only partition, still queryable by reports, retention-governed |
| Time Machine retention by plan | Recovery + audit | Our journal/history + snapshots; retention is a setting, not a plan gimmick |
| Dashboard: slicers, Top N, pivot, row/column switch, "follow viewer permission" statistics | Analysis depth; permission-aware numbers | C9 reports: slicers, Top N, pivot (planned), and permission-scoped aggregates by default |
| Work activity feed across bases, follow a single record | Awareness | User dashboard activity feed + record subscriptions (C13 preferences) |
| Field change constraints on large tables | Honest about cost | We make **all type changes asynchronous jobs** (new column, backfill, swap; doc 04 §3.2) with progress, without a row ceiling |
| Per-feature AI cost table | Transparency | Budget per skill/agent/tenant with cost preview; unified token-based metering |

---

## 4. Our design: one kernel instead of nineteen surfaces

### 4.1 Five unifications

| Unification | Replaces (theirs) | Ours |
|---|---|---|
| **U1 One event & rule system** | Automation triggers, workflow triggers, agent triggers, agent schedules, field-shortcut auto-update, buttons | C14 Rules: *When / If / Then*. Every trigger (record events, schedules, dates, comments, chat messages, buttons, sources, metrics) is a rule. Workflows, AI skills, agents and notifications are **actions** of rules. One run log explains every execution |
| **U2 One execution engine** | Automation vs workflow | One workflow engine; "simple automation" is just a one-step workflow shown in a compact editor. Same nodes, same quota, upgrade in place |
| **U3 Two AI concepts only** | Field shortcuts, toolbar, AI nodes, AI Agent node, agent node, agent field, default agents, dashboard AI | **AI Skill** (stateless, typed inputs/outputs bound to fields and code lists; doc 11 §5.4) and **Agent** (a principal with instructions, knowledge, tools incl. MCP, memory, schedules via rules, budget). Every UI surface (cell toolbar, field, workflow step, form, dashboard, chat) **invokes a skill or an agent**; it never defines its own AI logic |
| **U4 One identity & authority rule** | Last modifier / workflow creator / trigger creator | Every AI execution records **two principals**: the **agent/skill identity** (with its own capability grants and budget) and the **initiator** (user, rule owner, or schedule owner). Effective permission = intersection of both. If either loses access, the run is blocked with an exception, not silently skipped |
| **U5 One safety & provenance contract** | Irreversible overwrite, undo in a different place, manual saves | All AI writes go through the **op pipeline**: provenance per cell (doc 05 L1), *suggested* vs *committed* per field policy, **undo from the same place** (journal-based), structural changes via **staging branch** (§3). Nothing AI does is irreversible before publish |

### 4.2 Same principle for pages, process and permissions
- **Pages:** one layout system (record page, list/grid, board, calendar, form, portal/lookup, dashboard, app shell), all built from the same components and scoped by capabilities. AI page generation produces these layouts; it never creates a separate page type.
- **Process:** lifecycle with state categories (doc 13 §4.1) is the single home for stages, who may move a record, required fields, SLA and approvals. There is no separate "process field".
- **Permissions:** capabilities with scopes (doc 07 §5). Spaces, apps and pages inherit them; agents and skills are principals; "explain" answers who can do what and why.
- **Reference data:** governed code lists and trees (doc 07 §2) replace option-copy workarounds; cascades come from tree structure.

### 4.3 Where AI sits in our architecture (foundation, not add-on)

```
            Surfaces: cell toolbar · field · form (paste/voice/document) · workflow step · dashboard · chat · sidebar
                                    │  (all invoke the same two objects)
                      ┌─────────────┴─────────────┐
                      │   AI Skill        Agent   │   ← typed I/O · principal · budget · provenance
                      └─────────────┬─────────────┘
   Kernel:  Rules (events) ─► Workflow engine ─► Op pipeline (validate · capability check · provenance · journal)
            Field framework · Reference data · Components (K1–K23) · Lifecycle · Capabilities · Trace & audit
```

AI is foundational because the kernel already knows about AI: provenance on every value, AI principals
in the capability model, AI skills as rule actions and workflow steps, typed outputs bound to field
roles and code lists, and staging and verification for AI-built structures. Surfaces stay thin.

---

## 5. Gap check: features in their docs vs our design

| Feature (theirs) | In our design? | Where / action |
|---|---|---|
| Process field (stages, who completes, required fields) | Yes, unified | Lifecycle + capabilities (C6, doc 13 §4.1) |
| Button field | Yes | Rule trigger "button pressed" (C14) |
| Query page | Yes, generalised | Portal pattern (forms + lookup + capability-scoped edit) |
| Form detail table | Yes | K3 lines in forms |
| Option linkage / cascade | Yes, governed | Reference data trees + cascader widget |
| Field grouping (grid) + custom record detail layout | Yes, unified | Field groups render in grid header, record page and forms from one definition (doc 07 §1.2) |
| Sandbox | **Add** | Staging branch for tenant overlays + AI changes (C12) |
| Archive table | **Add** | Archive tier (data lifecycle, retention) in C1/K19 |
| Time Machine | Yes | Journal, history, snapshots (C10) |
| Work activity feed, follow record | Partly | Add activity feed to user dashboard; record subscription in C13 |
| Doc pages inside a base, folders, navigation | **Add (light)** | Knowledge/doc pages (K17) and folder structure in workspace |
| Dashboard pivot, slicers, Top N, combo, funnel, word cloud, button component | Mostly | Add slicers, Top N, word cloud and button component to C9 |
| Formula: LET, MAP, LIST, FILTER, NETWORKDAYS, CurrentValue | Partly | Formula library spec: add LET/MAP/LIST and business-calendar functions |
| AI paste entry, voice entry | **Add** | AI capture input channels (doc 11 catalogue A/B) |
| AI forecasting in analysis | Restricted | Only as clearly labelled statistical forecasts on certified metrics (F-group), never model-invented numbers |
| AI custom landing pages | Partly | Generated layouts from our page system; marketing pages are out of scope for v1 |
| Plugin/connector recommendation in sidebar | **Add** | Sidebar intent: "find connector/template" over our catalogue |
| Agents in group chats, comment triggers | Yes, unified | Agent entry points via C13 channels; comment events in C14 |

---

## 6. Decisions proposed
1. Adopt the five unifications (§4.1) as **architecture rules**; any new feature must fit them.
2. Add to the platform: staging branch (sandbox), archive tier, activity feed, slicers/Top N/word cloud, paste & voice capture, extended formula library, connector/template recommendation.
3. Keep our restrictions where theirs are looser: no irreversible AI overwrites, no model-invented numbers, no AI-only permission changes without staging, explicit dual-principal identity.

*Source handling: the archive was downloaded from the shared link into an isolated scratch folder,
converted to plain text for analysis, and not redistributed.*

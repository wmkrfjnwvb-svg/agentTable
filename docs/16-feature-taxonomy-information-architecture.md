# Feature Taxonomy & Information Architecture

> Status: product design v0.9 · Inspired by the Feishu Base help-centre categorisation (navigation tree of
> the reviewed archive, doc 15). We keep the parts of their categorisation that work and adapt the structure
> to our kernel (docs 09, 11, 13, 15). The taxonomy is used in three places: **in-product navigation &
> pickers**, the **feature catalogue** (docs, marketing, pricing), and the **documentation site**.

---

## 1. Their categorisation (summary) and what makes it work

**Feishu's top-level categories:** Quick start (basics, settings, AI) · Fields (basics, regular, business,
advanced) · Views · Records & rows · Formulas & functions (basics, function reference, practices) ·
Automation · Workflow · Dashboards · Advanced permissions · App mode · Sync, plugins & apps ·
Practices (function, scenario) · Customer cases.

What works and is worth adopting:
1. **Categories follow the objects users touch:** field, view, record, formula, dashboard, permission. Users look things up by the object in front of them.
2. **Fields grouped by intent:** *regular* (text, select, person, date, number, attachment…), *business* (process, button, auto number, phone, email, location, barcode, progress, currency, rating, signature) and *advanced* (links, system fields). This makes the field picker understandable.
3. **Same internal pattern in every category:** basics → individual features → practices → FAQ & limits.
4. **Practices sit next to features**, e.g., "calculate month-over-month growth" sits under formulas and "ticket group-chat system" under automation. Users learn by scenario.
5. **"Records & rows" as its own category** (follow, share, summarise a record, archive, capacity, record page): the record is a first-class object.
6. **A separate integration category** (connectors, plugins, cross-base sync, publish as shared data source).

What we deliberately change (from doc 15's diagnosis):
- **Automation and Workflow are one category** (one engine; U2), and **Rules** become the entry point for every trigger (U1).
- **AI is two things:** an **AI category** for the two AI objects (Skills, Agents) and their governance, *plus* an "AI" sub-item inside every other category showing how that object uses skills and agents. AI is not scattered as unrelated features.
- **Process (lifecycle, approvals, SLA, exceptions) gets its own category**; at Feishu it is split between the process field, workflows and permissions.
- **Reference data, settings and templates/solutions** become top-level categories, because our platform is built on them (docs 07, 13).
- **Permissions become "Access"** (capabilities, scopes, roles, sharing, explain), covering apps, pages, agents and API alike.

---

## 2. Our taxonomy (12 categories)

Order follows the user's journey: **build data → work with records → compute → run processes → see results →
control access → present → connect → AI → templates → administer**.

| # | Category | What users find there | Platform capability / components |
|---|---|---|---|
| 1 | **Get started** | Concepts, first base, templates gallery, what's new, limits, keyboard shortcuts | — |
| 2 | **Data model** | Workspaces & bases, tables, **fields** (§3), field groups, field library, field types registry, links & lookups | C1, C2, K1–K3 |
| 3 | **Records** | Add/import records, record page (layout, tabs, related data), comments & mentions, follow/share a record, history & restore, archive tier, capacity | C1, C10, K16 |
| 4 | **Views** | Grid, kanban, calendar, timeline/Gantt, gallery, list; filter/sort/group, colour rules, personal/locked/protected views, shared views | C1 |
| 5 | **Reference data & settings** | Code lists (flat/tree), crosswalks, market sets (ISO), settings/parameters, business calendars, number sequences | C3, C4, K21 |
| 6 | **Formulas & functions** | Formula basics, function reference (by group), statistics on related records, optimisation, errors, practices | C1 (mt-formula) |
| 7 | **Process** | Lifecycles (states & categories), assignments & acknowledgement, SLA & calendars, **approvals (visual designer)**, tasks & checklists, exceptions inbox, collection rounds | C6, C7, K4–K6, K10, K14 |
| 8 | **Rules & workflows** | Rules (When / If / Then), workflows (canvas: branches, loops, waits, sub-flows), actions catalogue, notifications & channels, run logs, practices | C13, C14 |
| 9 | **Dashboards & reports** | Dashboards (components by type), certified metrics, pivots, slicers, Top N, period comparison, scheduled delivery, subscriptions, alerts → tasks | C9, K15 |
| 10 | **Access** | Capabilities & scopes, roles/capability sets, record/field/view rights, guests & portals, sharing, separation of duties, **explain / preview as user**, access reviews | C5 |
| 11 | **Apps & pages** | App shell (navigation, pages), forms (incl. line items, prefill, paste/voice/document capture), portals & lookup pages, record pages, page components | C7, C1 |
| 12 | **Connections** | Data sources (push inbox, database, REST, files), destinations (events, HTTP actions, feeds, change feed), import/export, API & SDKs, MCP, connectors catalogue | C8, C12 |
| 13 | **AI** | **AI Skills** (catalogue, configuration, quality), **Agents** (instructions, knowledge, tools/MCP, memory, schedules via rules), AI builder & assistant, budgets & usage, providers & privacy, evaluation | C11 |
| 14 | **Templates & solutions** | Solution packages by role and industry (doc 14), components (K1–K23), install/adapt/upgrade, market overlays, size profiles, authoring templates | C12, doc 13 |
| 15 | **Administration & trust** | Tenants & members, SSO/SCIM, security policies, audit & trace explorer, staging & publishing (sandbox), retention & data requests, usage & quotas, self-host operations | C10, C12 |
| — | **Practices & cases** (cross-cutting) | Scenario walkthroughs per role/industry; practices attached to each category | — |

Categories 1–15 appear in the documentation site; in-product navigation uses the subset in §5.

---

## 3. Field palette categorisation (adopting Feishu's intent grouping, extended)

| Group | Field types | Note |
|---|---|---|
| **Basic** | Text, long text (rich), number, currency, percent, date & time, duration, checkbox, single select, multiple select, attachment, URL | Everyday data |
| **People & organisation** | Person, group/team, party (customer/supplier/candidate… via K1 roles), org unit (optional) | Feishu mixes these into "regular"; we separate them because they carry permissions and notifications |
| **Business** | Phone, email, address, location, barcode/QR, rating, progress, signature, auto number (sequence), **tax/business ID** (market formats), **amount with currency** | Validated formats from the format library |
| **Reference** | Code-list value (flat/tree, cascade), **catalogue item** (K2 with category attributes) | Governed values, not ad-hoc options |
| **Relational & computed** | Link (one-/two-way), lookup, rollup, formula, **line items** (child table with totals) | Feishu's "advanced" group plus our K3 lines |
| **Process** | **Lifecycle state** (replaces Feishu's process field + status select), assignment, due/SLA, approval status, button (trigger) | All backed by C6/C14; one place for process logic |
| **AI** | **AI skill field** (classify, extract, summarise, translate, generate, score: output typed and bound to a code list or field type), agent result field | Shows provenance and review state; never an untyped "AI text" column |
| **System** | Created/modified by/at, record ID, version, source (data source), trace link | Read-only |

**Picker behaviour:**
- Groups are shown in this order.
- A search box matches synonyms ("status" → Lifecycle state; "dropdown" → Single select).
- Each type shows its widget options (doc 07 §1).
- AI suggests the type from a sample value.

---

## 4. One documentation pattern for every category

Adopted from Feishu and made explicit as a template every page set follows:

1. **Basics:** the concept in one page, with a diagram.
2. **Features:** one page per feature: what it does → how to use it → options → limits → permissions → AI support → API/MCP equivalent.
3. **Practices:** short scenario recipes tied to solution templates (e.g., "three-way match tolerances" under Process; "month-over-month growth" under Formulas).
4. **FAQ & limits:** generated from the product's limits registry, so limits are never out of date.
5. **Related:** links to the components (K) and templates (S) that use the feature.

Each feature page also states the **API, rule action, and MCP tool** that does the same thing, so
people, automations and AI agents learn one vocabulary.

---

## 5. In-product navigation (derived from the taxonomy)

```
Workspace sidebar                         Inside a base                          Base settings menu
├─ Home (my work, activity, follows)      ├─ Tables ▸ views                      ├─ Fields & field library
├─ Bases                                  ├─ Process (lifecycles, approvals)     ├─ Reference data & settings
├─ Apps                                   ├─ Rules & workflows                   ├─ Access
├─ Templates & solutions                  ├─ Dashboards                          ├─ Connections
├─ Agents                                 ├─ Pages & forms                       ├─ AI skills & budgets
└─ Admin (if permitted)                   └─ Docs (knowledge pages)              ├─ Staging & publish
                                                                                 └─ History & audit
Global: ⌘K command bar = search + AI assistant (build, configure, ask) — one entry point for AI
```

- **One AI entry point** (command bar/sidebar) with intents: *build*, *configure*, *ask*, *find template or connector*, *explain*. Other surfaces (cell, field, workflow step, form, dashboard) show AI where the object is, always using a skill or agent.
- **Home** combines the user dashboard: my tasks, approvals, exceptions, followed records and an activity feed (adopted from Feishu's work activity).

---

## 6. Mapping: Feishu category → our category

| Feishu | Ours | Change |
|---|---|---|
| Quick start: basics / settings | Get started; Records; Views; Administration | Settings split by object |
| Quick start: Base AI | AI | AI objects plus per-category AI sub-items |
| Fields: regular / business / advanced | Data model → field groups (§3) | + People, Reference, Process, AI, System groups |
| Views | Views; Apps & pages (forms) | Forms move to Apps & pages with capture features |
| Records & rows | Records | Same, plus history and archive tier |
| Formulas & functions | Formulas & functions | Same structure (basics, reference, practices) |
| Automation + Workflow | Rules & workflows | **Merged** (one engine) |
| (process field, approval sync) | **Process** | New category |
| Dashboards | Dashboards & reports | + certified metrics, scheduled reports |
| Advanced permissions | Access | Capability-based; covers apps, agents, API |
| App mode | Apps & pages | Same idea, one layout system |
| Sync, plugins & apps | Connections | Inbound sources and outbound destinations explicit; API/MCP here |
| — | **Reference data & settings** | New |
| — | **Templates & solutions** | New (component-based) |
| Sandbox, Time Machine, spaces (scattered) | Administration & trust | Grouped |
| Practices / customer cases | Practices & cases | Attached to categories and templates |

## 7. Additions noticed in their navigation (for the backlog)
- Dashboard components: radar, NPS chart, countdown, progress chart, layout groups, multiple source tables per component, chart cross-filtering, **subscribe to dashboard updates**, scheduled dashboard snapshots via automation → C9 backlog.
- Permissions: **preview permission results** and the effect of permissions on form submissions → C5 "explain/preview".
- Integration: **publish a base as a shared data source** for other bases, cross-base sync → C8 (internal source between bases, inbound-only model kept).
- Printing/layout for records (auto-layout printing practice) → K22 document output.
- Intelligent document processing app → A-group skills (doc 11 §5).

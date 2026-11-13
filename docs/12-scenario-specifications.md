# Scenario Specifications — six solution packages in product detail

> Status: product design v0.7 · Builds on doc 08 (solution matrix), doc 09 (product definition),
> doc 11 (collection, rules, notifications, workflow designer, AI catalogue).
> Each scenario is specified with the same structure:
> **A** users & goal · **B** core functions · **C** industry challenges → our answer · **D** data model ·
> **E** data in (collection & sources, sync/async) · **F** rules & notifications · **G** approvals (visual designer
> template) · **H** permissions · **I** import/export · **J** AI skills · **K** reports & KPIs · **L** release scope.
> Industry research is mostly vendor-published; claims are labelled where it matters.

---

## Shared conventions (apply to all six)
- **Sync/async:** follows doc 11 §0.1. Saves and approval decisions are instant; notifications, external actions, AI and imports are jobs with visible status.
- **Permissions:** business capabilities with scopes and limits (doc 07 §5). Field groups carry sensitive-data permissions. External people (candidates, customers, creators, suppliers) use guest access or forms and see only their own records.
- **Approvals:** every scenario ships approval templates built in the visual designer (doc 11 §4). Customers change approvers, limits, steps and branches without code. Running requests keep their version.
- **Import/export:** every table supports templated import (dry run, error file, rollback) and export (formatted Excel/CSV, privacy rules) (doc 06 §2.9). Scenario-specific templates are listed under **I**.
- **Data sources:** inbound only; external systems stay authoritative (doc 09 §3).

---

## 1. HR & Administration (Release 1)

**A. Users & goal.** HR/admin staff, employees, managers, IT, finance. Goal: run employee requests,
equipment and onboarding/offboarding with clear owners, so nothing falls through between teams.

**B. Core functions**
1. **Employee directory (light)**: people with department, location, manager, start date. Imported or synced; optional.
2. **Requests desk**: leave, expense claims, purchase requests, certificates, general HR/admin requests, all through forms with status tracking.
3. **Expense claims**: line items, receipts, policy limits, approval by amount, payment export to accounting.
4. **Equipment & assets**: catalogue, stock, assignment to people (with signed receipt), returns, repairs, disposal.
5. **Onboarding & offboarding**: checklists generated per role/location, tasks for HR/IT/manager/facilities, day-1 readiness, asset return and access removal on exit.
6. **Contracts & documents**: employment and vendor contracts, certificates, expiry tracking.
7. **Policy desk**: questions answered from policy documents, with handover to HR.

**C. Industry challenges → our answer**

| Challenge (research) | Our answer |
|---|---|
| Offboarding is the weak link: returns "often have no owner, no deadline, no follow-up"; a cited survey found 71% of HR professionals had at least one departing employee who failed to return equipment ([Rocket](https://www.rocket.new/blog/how-to-manage-company-equipment-assignments-and-returns), [Zones](https://blog.zones.com/8-it-asset-retrieval-challenges-and-how-to-fix-them-in-2026)) | Offboarding case **auto-created** from the exit date; each asset return is a task with owner and deadline; escalation sequence (reminders → manager → final notice); exceptions for unreturned items |
| Device tasks depend on someone remembering; should be triggered by HR/identity events ([Rocket](https://www.rocket.new/blog/how-to-manage-company-equipment-assignments-and-returns)) | Rules on start date / exit date / role change create tasks automatically |
| "Ghost" asset records inflate inventory and skew audits ([AirPinpoint](https://airpinpoint.com/asset-tracking/it-asset-return-management)) | Assignment requires serial + signed receipt; periodic **asset audit campaign** (collection round per location); unconfirmed assets → exception |
| Role changes need device swaps, not just hires/exits ([HAM guide](https://hardwareassetmanagement-com.pages.dev/guides/onboarding-offboarding-hardware.html)) | "Role change" trigger compares standard kit per role and creates swap tasks |
| Expense tools: buyers value policy enforcement, card/invoice connection, accounting/HR integration ([BILL](https://bill.com/blog/best-expense-management-software), [Spendesk](https://www.spendesk.com/blog/best-expense-management-software/)) | Policy limits as settings; validation rules; AI pre-check; payment export feed to accounting; card statement import (CSV) with matching |
| Manual spreadsheet processes and extra steps frustrate users ([Software Advice](https://www.softwareadvice.com/ca/hr/applicant-tracking-software-comparison/)) | One "My requests" page; receipts by phone photo; approvals from chat/email |

**D. Data model**
- Tables: People · Requests (type, status, requester, amounts) · Expense lines · Assets · Asset assignments · Onboarding cases · Tasks · Contracts · Policy articles · Questions log
- Code lists: request type · expense category (tree) · asset category (tree) · location (tree) · leave type · document type · kit per role
- Field groups: Person → *Contact* · *Employment* · *Personal (restricted)* · *Bank details (restricted)*

**E. Data in**
| Input | How | Mode |
|---|---|---|
| People list | Import template or HR system source (file/DB/push) | Scheduled/async |
| Requests | Forms with prefill from People; record pickers (my assets, my cost centre) | Sync save |
| Receipts/invoices | Phone photo in form → A2 extraction suggestions | Async (seconds) |
| Card statements | CSV import template → matched to expense lines | Async job |
| Asset stock | Import or purchase records | Async |

**F. Rules & notifications (examples)**
- Validation: expense line over the per-diem setting needs a justification; receipt required above amount X.
- Reaction: exit date set → create offboarding case and asset-return tasks, notify IT + manager (chat + email).
- Alert: asset not returned 3 days after exit → reminder to person (email/SMS), day 7 → manager, day 14 → HR exception.
- Digest: weekly "open requests by age" to team leads.

**G. Approval templates (visual designer)**
- *Expense*: Start → [Condition: amount ≤ limit A → Manager (any one)] / [A < amount ≤ B → Manager → Finance (one after another)] / [Other → Manager → Finance → Director] → Payment step (finance fills payment date) → End.
- *Purchase*: adds a parallel IT review branch for IT categories.
- *Leave*: Manager; if > N days, plus HR. Delegation during the manager's own leave.

**H. Permissions**
- `request.submit` (all employees, own records).
- `expense.approve` (scope: cost centres I own; limit by amount).
- `expense.pay` (finance).
- `asset.assign/return` (IT, scope: location).
- `person.view_personal` (HR only).
- Employees see only their own requests and assets.

**I. Import/export:** people template; asset register template; card statement template.
Payment feed (CSV/XLSX in accounting format) and asset audit export.

**J. AI skills:** A2 receipts → expense lines · E1 expense pre-check (duplicates, policy, missing receipt) ·
E2 policy desk with handover + unanswered-question log · E3 onboarding checklist from role/location ·
B6 duplicate asset/receipt detection · D4 build from policy documents.

**K. Reports & KPIs:** request SLA by type · expense cycle time · policy exceptions · onboarding tasks on time ·
assets unreturned after exit · asset utilisation.

**L. Release 1 scope:** requests, expenses, assets, onboarding/offboarding, policy desk. Later: training, contracts.

---

## 2. E-commerce after-sales & reviews (Release 1)

**A. Users & goal.** Customer-service agents, after-sales leads, warehouse/returns staff, category
managers, finance. Goal: every complaint, return and negative review is handled on time, refunds are
controlled, and root causes reach product teams.

**B. Core functions**
1. **Case inbox**: messages, return requests and reviews from shop platforms, email and forms become cases.
2. **Returns (RMA)**: eligibility check, return authorisation, label/instructions, receipt and inspection, disposition (restock/refurbish/scrap), refund/exchange.
3. **Refund control**: approval by amount/risk, refund action in the shop platform, confirmation from the source.
4. **Review management**: negative review alerts, reply drafting, follow-up, review response time.
5. **Root-cause analysis**: reason codes per SKU/supplier, feedback to product/QA.
6. **Customer communication**: status updates by email/SMS/chat, with templates per language.

**C. Industry challenges → our answer**

| Challenge (research) | Our answer |
|---|---|
| Returns run in phases (initiation/eligibility → RMA → labels/communication → inspection/grading → disposition) and are often treated identically; high- and low-value returns need different SLAs ([Saara](https://saara.io/blog/10-best-practices-to-manage-rma-returns-in-ecommerce), [Ringly](https://www.ringly.io/nl/blog/product-returns-management)) | Lifecycle per phase; **SLA by value band and risk** (settings); condition branches in the return flow |
| RMA-to-refund cycle time is the key metric (top performers under 5 days, per a cited source) ([Ringly](https://www.ringly.io/nl/blog/ecommerce-returns-management)) | KPI built in; timers per phase; exceptions on breach |
| Refunding before inspection on high-risk SKUs; return fraud ([Ringly](https://www.ringly.io/nl/blog/ecommerce-returns-best-practices), [ShippyPro](https://www.shippypro.com/blog/en/how-to-easily-manage-ecommerce-returns)) | "High-risk SKU" code list → flow requires inspection before refund; photo evidence required; repeat-returner rule flags cases |
| Customer-facing status must reflect verified warehouse and financial events ([ClickPost](https://www.clickpost.ai/en-us/blog/streamlining-ecommerce-returns-process)) | Status comes **from the source** (shop/warehouse) after actions; the case closes only when the source confirms refund (doc 09 §3.2) |
| Average return rates around 24.5% (one cited estimate) ([BigCommerce](https://www.bigcommerce.com/articles/ecommerce/returns)) | Return-rate dashboards by SKU/reason with anomaly alerts |

**D. Data model**
- Tables: Orders (source) · Order items (source) · Products/SKUs (source or native) · Cases · Returns · Return items · Inspections · Refunds · Reviews (source) · Replies · Customers (source)
- Code lists: issue type (tree) · return reason (tree) · disposition · risk band · channel/store · region · reply tone
- Local fields on source tables: owner, case status, tags, AI fields

**E. Data in**
| Input | How | Mode |
|---|---|---|
| Orders, items, customers, reviews | Shop platform sources (push inbox or scheduled pull), e.g. Shopify/Amazon/Rakuten/Coupang/Flipkart/Otto **[verify APIs]** | Async, incremental |
| Return requests | **Public return form**: order no. + email verified against Orders; item picker; photos; reason code | Sync save, AI async |
| Messages | Email-in / chat channel → case (B2 extraction: order no., request type) | Async |
| Inspection results | Warehouse staff form on phone (photos, grade) | Sync |

**F. Rules & notifications**
- Reaction: 1–2★ review → case + notify region lead (chat), reply draft prepared.
- Alert: case SLA at 80% → owner; breach → lead; return rate of a SKU > threshold (7 days) → category manager task.
- Customer updates: return authorised / received / refunded → email or SMS (consent and country rules, doc 11 §3.3).
- Throttle: one customer message per case per status.

**G. Approval templates**
- *Refund*: [Condition: amount ≤ A and risk low → auto-approve] / [≤ B → Team lead] / [Other or high risk → Lead → Finance] → **Action: call shop refund API** → **Wait** for source status "refunded" (timeout → exception) → End.
- *Goodwill compensation*: lead approval with monthly budget check (setting).

**H. Permissions**
- `case.handle` (scope: my store/region).
- `refund.approve` (limit by amount).
- `refund.execute` (system/finance).
- `review.reply.publish`.
- `customer.view_contact` (masked phone/email for most agents).

**I. Import/export:** historical cases/reviews import; SKU master import with category mapping.
Exports: refund report for finance, root-cause report for suppliers (per supplier, filtered).

**J. AI skills:** B1 issue classification + B3 urgency · B2 extraction from messages · A5 damage photo tagging ·
C1 reply drafts in the customer's language + B5 translation · E1 refund pre-check (policy window, previous
refunds, inspection status) · B6 duplicate cases · F3 narrative on return reasons.

**K. Reports & KPIs:** RMA-to-refund cycle time · case SLA attainment · review response time ·
return rate by SKU/reason/supplier · refund amount by approver/band · repeat returners.

**L. Release 1 scope:** cases, returns, refunds, reviews, customer messaging, dashboards. Later: supplier claims, live-commerce module.

---

## 3. New Media content operations (Release 2)

**A. Users & goal.** Content planners, creators/editors, reviewers, channel operators, creator-partnership
managers. Goal: a predictable content pipeline across platforms, with performance data feeding the next topics.

**B. Core functions**
1. **Topic pool**: ideas, references and benchmark content, tags, priority, owner.
2. **Production pipeline**: script → shoot → edit → review → publish, with owners and deadlines per stage.
3. **Review & compliance**: brand/legal checks and disclosure requirements before publishing.
4. **Publishing calendar**: per platform and account; one item can have many publications.
5. **Performance collection**: scheduled pulls of views, engagement and followers per publication.
6. **Creator partnerships**: briefs, contracts (deliverables, usage rights, exclusivity), delivery check, payments.
7. **Comments & community**: comment intake, classification, replies.

**C. Industry challenges → our answer**

| Challenge (research) | Our answer |
|---|---|
| Only 32% of teams can update content across channels the same day; manual handoffs don't scale; only 24% consider their stack optimised ([Canto 2026](https://www.canto.com/blog/state-of-digital-content/)) | One pipeline with stage owners; publications per platform linked to one content item; status changes trigger the next stage |
| Social teams operate in silos, causing drift from core messaging ([AirOps](https://www.airops.com/blog/the-broken-content-team)) | Shared topic pool and calendar; brand rules as code lists checked before publish |
| Unclear ownership and missing processes ([Easycontent](https://easycontent.io/resources/what-content-teams-need-to-fix-in-2026-and-ignore/)) | Every stage has an owner rule and SLA; overdue stages chased automatically |
| Influencer contracts: confirm approvals, rights and disclosure before going live; verify delivery against contract and usage rights ([Ebiquity](https://ebiquity.com/?p=40624)); the hard part is operating contracts over time (payouts, rights) ([NetInfluencer](https://www.netinfluencer.com/?p=42853)); usage-right duration is a key term ([InfluenceFlow](https://influenceflow.io/resources/understanding-usage-rights-in-influencer-contracts-your-2026-guide-to-ip-content-licensing/)) | Collaboration lifecycle: brief → contract approval → delivery review (checklist vs contract) → payment approval; **usage-rights expiry rule** alerts before content must be removed |
| 75% of marketers now use AI in content processes, which speeds production without fixing workflow ([Contentoo](https://www.contentoo.com/blog/content-operations-for-beginners)) | AI skills sit inside the pipeline (variants, checks), not beside it |

**D. Data model**
- Tables: Topics · Content items · Production tasks · Publications (content × platform/account) · Metric snapshots · Creators · Collaborations · Deliverables · Payments · Comments
- Code lists: platform/account · format · topic category (tree) · stage · audience · disclosure type · usage-right scope

**E. Data in**
| Input | How | Mode |
|---|---|---|
| Platform metrics | Scheduled pull per account (interval set by a setting), e.g. YouTube/TikTok/Instagram/X/LINE/Naver **[verify APIs/terms]** | Async, scheduled |
| Topics & references | Form, browser link capture, import | Sync |
| Creator applications | Public form with portfolio links/files | Sync, AI async |
| Comments | Platform source or export import | Async |

**F. Rules & notifications**
- Stage overdue → chase owner (chat) → lead after 24h.
- Published → schedule metric pulls at +1h, +24h, +7d.
- Usage rights expire in 14 days → task to remove or renew.
- Content scoring below benchmark after 7 days → topic review task.

**G. Approval templates**
- *Content review*: Editor → [Condition: sponsored → Legal/brand (parallel with Editor-in-chief)] → Publish.
- *Collaboration*: Budget ≤ A → Lead; above → Lead → Finance; contract terms locked after approval.
- *Creator payment*: delivery verified → finance.

**H. Permissions**
- `content.edit` (my team's items).
- `content.approve_publish`.
- `collaboration.approve_budget` (limit).
- `creator.view_payment_details` (finance only).
- Creators as guests see only their briefs and deliverables.

**I. Import/export:** historical publications/metrics import; creator roster import. Exports: performance
report per platform, creator payment file.

**J. AI skills:** C2 title/script variants per platform and language · B1 topic tagging · B4 transcript/summary ·
B1+B3 comment classification + C1 reply drafts · E1 pre-publish check (disclosure present, prohibited claims) ·
F3 performance narrative · F1 "which topics outperform our benchmark?".

**K. Reports & KPIs:** output per week · on-time publishing · performance by topic/format/platform vs benchmark ·
collaboration ROI · rights expiring.

**L. Release 2 scope:** all of the above. Later: live-stream operations (shared with e-commerce).

---

## 4. Recruitment (Release 2)

**A. Users & goal.** Recruiters, hiring managers, interviewers, HR, finance (headcount), candidates
(guests). Goal: fill approved roles quickly and fairly, with a defensible record of every decision.

**B. Core functions**
1. **Headcount requests (requisitions)** with budget approval.
2. **Openings & job descriptions**, posting links per channel.
3. **Candidate intake**: careers form, email-in, job board sources, referrals; **duplicate handling**.
4. **Pipeline**: stages per opening (Kanban), owners, SLAs, rejection reasons.
5. **Interviews**: scheduling, interview kits, **scorecards**, feedback deadlines.
6. **Offers**: offer details, approval, letter generation, acceptance tracking.
7. **Handoff**: accepted offer → HR onboarding case (scenario 1).
8. **Candidate privacy**: consent, retention, deletion, data requests.

**C. Industry challenges → our answer**

| Challenge (research) | Our answer |
|---|---|
| Modern ATS acts as a hub that must keep the recruiting stack in sync, not a filing cabinet ([Lever](https://www.lever.co/blog/modern-applicant-tracking-systems-what-to-look-for-in-2026)) | Sources for job boards/email; outbound events to HR and calendar; one candidate record across openings |
| Consistent evaluation through interview kits and customisable scorecards ([Gem](https://app.gem.com/blog/best-applicant-tracking-system)) | Scorecard templates per opening; feedback required before stage move (validation rule) |
| Midsize companies struggle to connect ATS with HR/payroll; extra manual steps ([Software Advice](https://www.softwareadvice.com/ca/hr/applicant-tracking-software-comparison/)) | Built-in handoff to HR onboarding; export/feeds for payroll |
| Data security and compliance should be a top priority ([iCIMS](https://www.icims.com/blog/what-to-look-for-in-an-applicant-tracking-system-complete-buyers-guide/)) | Retention rules per market, consent records, restricted field groups, deletion workflow |
| Employment AI is high-risk (EU AI Act); NYC requires bias audits for automated decision tools | **No AI ranking/rejection**; AI only extracts and summarises; decisions recorded with reasons |

**D. Data model**
- Tables: Requisitions · Openings · Candidates · Applications (candidate × opening) · Interviews · Scorecards · Offers · Sources · Consents
- Code lists: job family (tree) · level · location (tree) · stage · rejection reason · source channel · employment type
- Field groups: Candidate → *Contact* · *Experience* · *Compensation expectations (restricted)* · *Self-identification data (US, restricted, separate)*

**E. Data in:** careers form (CV upload → A3 extraction suggestions, consent checkbox with retention notice) ·
email-in for CVs · job board sources **[verify]** · referral form · bulk CV import (async job).

**F. Rules & notifications**
- Feedback not submitted 24h after interview → interviewer reminder → recruiter.
- Application idle in a stage > N days → recruiter task.
- Retention period reached → deletion task (or automatic deletion after review).
- Candidate status emails from templates (human-sent by default).

**G. Approval templates**
- *Requisition*: Hiring manager → [Condition: new headcount → Finance] → HR → End.
- *Offer*: [Condition: within band → HR] / [Above band → HR → Finance → Department head] → Letter → Candidate acceptance (wait) → Handoff.

**H. Permissions**
- `application.move_stage` (recruiters for their openings).
- `scorecard.submit` (assigned interviewers see only their kits).
- `offer.approve` (limit).
- `candidate.view_compensation` (recruiter/HR).
- Self-identification data is visible only in aggregate reports.

**I. Import/export:** migration import from another ATS (CSV); export for audits and candidate data requests.

**J. AI skills:** A3 CV extraction · B6 duplicate candidates (human merges) · C3 job description draft ·
B4 scorecard summary · C1 candidate message drafts · E1 offer pre-check (band vs requisition budget).
**Not offered:** ranking, scoring or rejecting candidates by AI.

**K. Reports & KPIs:** time-to-fill · time-to-hire · funnel conversion per stage · source-to-hire ·
feedback SLA · offer acceptance rate · retention compliance.

**L. Release 2 scope:** all core functions. Later: interview calendar sync depth, referral programme rewards.

---

## 5. Marketing campaigns (Release 3)

**A. Users & goal.** Marketing managers, channel specialists, designers, brand/legal reviewers,
sales (lead recipients), finance. Goal: plan and launch campaigns on budget, with brand-safe content and
measurable results.

**B. Core functions:** annual/quarterly plan and budget lines · campaign briefs · content and asset
production · brand/legal review · launch checklist · spend & results collection · lead capture and handoff ·
event management · post-campaign review.

**C. Industry challenges → our answer**

| Challenge | Our answer |
|---|---|
| Intake, approval routing, asset management and calendars are the core components of content workflow ([CI HUB](https://ci-hub.com/blog/content-workflow-management-chaos-to-structured-operations)) | Brief form → approval → production pipeline (shared with New Media) |
| Multi-brand/market teams need a consistent operating model ([WP Engine](https://wpengine.com/blog/scaling-content-operations/)) | Brand and market as code lists; templates per market; capability scopes per brand |
| Spend data scattered across ad platforms | Scheduled spend/result sources per channel **[verify APIs]**; budget vs actual by line |
| Leads lost between marketing and sales | Lead handoff with acknowledgement and SLA; outbound event to CRM |

**D. Data model:** Plans · Budget lines · Campaigns · Briefs · Content items · Assets · Channels · Spend (daily) ·
Leads · Events. Code lists: channel (tree), region, product line, funnel stage, asset type, required disclaimers.

**E. Data in:** ad platform spend pulls (async, daily) · lead forms (public) · event registration forms · asset uploads.

**F. Rules & notifications:** budget line ≥ 90% spent → owner alert · CPA above target 3 days → task ·
new lead → sales owner (ack within N hours, else escalate).

**G. Approval templates:** *Budget/campaign* (by amount) · *Content* (brand → legal if claims) · *Event spend*.

**H. Permissions:** `campaign.approve_budget` (limit) · `content.brand_review` · `lead.accept` (sales, scope: region).

**I. Import/export:** budget plan import; lead export/feed to CRM; campaign report export.

**J. AI skills:** C2 copy variants per locale · E1 brand/claims pre-check · A5 asset tagging · B2 lead
extraction from emails · F2 spend anomalies · F3 campaign narrative.

**K. KPIs:** spend vs budget · CPA/ROAS · lead → opportunity conversion · lead acceptance SLA · content cycle time.

**L. Release 3.**

---

## 6. R&D tracker (Release 3)

**A. Users & goal.** Product managers, engineers, QA, support, release managers. Goal: requests and bugs
flow into planned work and releases, with quality and delivery visible.

**B. Core functions:** request intake (incl. from support cases) · backlog & prioritisation · iterations ·
bugs with severity SLAs · test cases & runs · releases with approval · release notes · links to code changes.

**C. Industry challenges → our answer**

| Challenge (research) | Our answer |
|---|---|
| Duplicates inflate triage load ([Augment](https://www.augmentcode.com/guides/ai-bug-triage)); duplicate detection research shows 70s–90s % accuracy, and simple search baselines are competitive ([arXiv](https://arxiv.org/html/2504.14797v1), [How far are we](https://ar5iv.labs.arxiv.org/html/2212.00548)) | B6 suggests likely duplicates at creation; **human confirms**; merged items link to a master |
| Severity conventions are project-specific ([Augment](https://www.augmentcode.com/guides/ai-bug-triage)) | Severity definitions as a code list per product; AI suggestions use the product's own examples |
| Embed duplicate checks at creation, triage and backlog review, with audit logs ([Supportbench](https://www.supportbench.com/?p=25640)) | Rules at each stage; trace log |

**D. Data model:** Requests · Epics · Work items · Bugs · Iterations · Test cases · Test runs · Releases ·
Code links. Code lists: component (tree), severity, priority, platform, environment.

**E. Data in:** support cases → requests/bugs (from E-commerce or help desk sources) · code host events
(push inbox: PRs/commits linked by key) **[verify]** · bug report form · Jira/CSV migration import.

**F. Rules & notifications:** severity-1 bug → on-call notify (chat + SMS) · bug SLA by severity · item idle
in review > N days → chase · release date approaching with open blockers → alert.

**G. Approval templates:** *Release*: QA sign-off → [Condition: includes high-risk components → Security review (parallel)] → Release manager.

**H. Permissions:** `bug.triage` · `work_item.edit` (team scope) · `release.approve` · support sees status of linked items only.

**I. Import/export:** Jira/CSV import; release report export; changelog feed.

**J. AI skills:** B1 component/severity suggestion · B6 duplicates · B2 reproduction steps from support text ·
C3 release notes · F3 delivery narrative.

**K. KPIs:** lead/cycle time · escaped defects · bug SLA attainment · iteration predictability.

**L. Release 3.**

---

## 7. Cross-scenario links (why one platform beats six tools)

| From | To | Link |
|---|---|---|
| Recruitment offer accepted | HR onboarding case | Automatic handoff with candidate data (minimised) |
| HR offboarding | Assets | Return tasks per assigned asset |
| E-commerce cases | R&D | Product defects become requests/bugs with case count |
| Marketing campaigns | New Media | Shared content pipeline and calendar |
| New Media creators | Finance (HR&A expenses/payments) | Payment approvals and feeds |

## 8. Open points
1. Market priorities for named platform sources (shop platforms, job boards, social platforms) so we verify APIs in that order.
2. Whether the HR employee directory should be an optional module or always present in the HR package.
3. Which chat/SMS channels to support first (e.g., Slack/Teams/Lark + email first; LINE/Kakao/WhatsApp next).

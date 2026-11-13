# Scenario Templates — field-level specification built from business components

> Status: product design v0.8 · Replaces the scenario detail of doc 12 (doc 12 remains valid for industry
> research and challenge tables). Component codes K1–K23, field roles and variation mechanisms are
> defined in doc 13.
> Eleven templates: the six requested scenarios (some split where real processes differ) plus three
> additions that share the same components (Procurement, Customer service desk, Projects).
>
> | # | Template | Origin |
> |---|---|---|
> | S1 | Recruitment (ATS) | Recruitment |
> | S2 | Employee lifecycle & HR requests | HR & Admin |
> | S3 | Expenses & reimbursement | HR & Admin |
> | S4 | Assets & equipment | HR & Admin |
> | S5 | Procurement (request → order → receipt → invoice) | **added** |
> | S6 | E-commerce after-sales & returns | E-commerce |
> | S7 | Customer service desk | **added** (generalises S6 for any industry) |
> | S8 | Content production & creator collaborations | New Media |
> | S9 | Marketing campaigns & leads | Marketing |
> | S10 | R&D: requirements, bugs, releases | R&D |
> | S11 | Projects & tasks | **added** |
>
> Every template follows the same structure:
> **1 Real process** (main flow, variants, edge cases) · **2 Entities & fields** (type · component role) ·
> **3 Variation points** · **4 Approvals** · **5 Rules & notifications** · **6 Data in/out** (sync/async) ·
> **7 Permissions** · **8 AI skills** · **9 Reports**.
>
> Field type shorthand: `text`, `longtext`, `num`, `money` (amount + currency), `date`, `datetime`,
> `bool`, `ref:<code list>`, `link:<table>`, `party:<role>`, `person`, `files`, `lines`, `calc`,
> `seq` (numbering). `?` = optional field group (variation point). Market overlay fields are tagged
> `[JP]`, `[DE]`, `[IN]`, `[KR]`, `[US]`.

---

## S1 Recruitment (ATS)
**Components:** K1 (candidate, agency, employee roles) · K2 (job profiles) · K5 (application pipeline) ·
K8 (offer, agency agreement) · K9 (headcount budget) · K10 · K11 (scorecards) · K13 (interviews) ·
K16 · K19 · K21 · K22 (offer letter) · K6 (pre-boarding tasks)

### 1. Real process
- **Main:**
  1. Requisition (headcount request) is raised.
  2. Approval (budgeted or new headcount).
  3. One or more openings are created; job description is published to channels.
  4. Applications arrive (careers form, email, job boards, agencies, referrals).
  5. Screening, then interview rounds with scorecards, then debrief.
  6. Offer (approval) → letter → acceptance.
  7. Pre-boarding; hand-off to S2.
- **Variants:**
  - *Evergreen/high-volume roles*: one opening, many hires, rolling pipeline.
  - *Internal transfer*: an employee applies; manager-notification rules.
  - *Agency candidates*: candidate ownership period; fee agreement.
  - *Referrals*: bonus eligibility after probation.
  - *Campus/new graduate* `[JP]` 新卒一括採用: cohorts by graduation year, information sessions, informal offers (内定) months before start.
  - *Works-council step* `[DE]` before hiring or transfer.
  - *Background verification* `[IN]`/`[US]`; notice-period tracking `[IN]` (often 30–90 days).
- **Edge cases:**
  - Same person applies to several openings, or reapplies (duplicate party, history kept).
  - Agency submits a candidate who already applied directly (**ownership conflict** → first-touch rule).
  - Offer revised (versioned agreement) or rescinded.
  - Candidate accepts, then withdraws before start (status *reneged*, reopen opening).
  - Opening frozen mid-process (bulk move with communication).
  - Interviewer leaves the company (reassign scorecards).
  - Data-deletion request during process.

### 2. Entities & fields

**Requisition** (K3 header + K10 + K9)
| Field | Type | Role / note |
|---|---|---|
| req_no | seq `REQ-{YYYY}-{seq:4}` | K21 |
| job_profile | link:Job profiles | K2 item |
| department / cost_center | ref:org or ref:cost_center | dimension.* (scope, budget) |
| location | ref:location (tree) | dimension.region |
| headcount | num | quantity |
| employment_type | ref:employment_type | full-time, part-time, contract, intern |
| reason | ref:req_reason | new, replacement (link replaced person), backfill |
| budgeted | bool | true → lighter approval branch |
| salary_band_min/max | money | money.amount (threshold), restricted group |
| target_start | date | time.due |
| hiring_manager / recruiter | person | person.owner |
| state | lifecycle | draft → pending_approval → approved → open → filled / cancelled / on_hold |

**Opening** · `opening_no` · `requisition` link · `title` (localised) · `description` longtext · `channels[]` ref ·
`posted_at` · `pipeline_template` ref · `scorecard_template` ref · `positions_total` / `positions_filled` calc · `state`.

**Candidate** (K1 party with role *candidate*)
| Field | Type | Role / note |
|---|---|---|
| name parts, display_name | text | locale name order |
| emails[], phones[] | contact points | consent per channel (K19) |
| current_company, current_title | text | |
| location, willing_to_relocate | ref, bool | |
| cv / portfolio | files | evidence.attachments (AI extraction source) |
| skills[] | ref:skills (tree) | AI-extracted, human-confirmed |
| experience_years | num | |
| education[] | lines (school, degree, year) | `[JP]` graduation_year for cohorts |
| source / referrer / agency | ref, person, party:agency | ownership rule input |
| expected_salary? / notice_period `[IN]` | money?, num days | restricted group |
| eeo_self_id? `[US]` | separate restricted group | aggregate reports only |
| consent, retention_until | K19 | deletion task |

**Application** (K5 pipeline: candidate × opening) · `stage` (state categories) · `stage_entered_at` · `owner` ·
`source` · `outcome` (hired/rejected/withdrawn/reneged) · `rejection_reason` ref · `rejected_by_party` (company/candidate) ·
`agency_ownership_until` date · `duplicate_of` link.

**Interview** (K13) · `application` · `round` ref · `start/end/time_zone` · `interviewers[]` · `format` (onsite/video/phone) ·
`location_or_link` · `status` · `feedback_due` calc.

**Scorecard** (K11) · `interview` · `evaluator` · `criteria_scores[]` (rubric) · `recommendation` ref (strong yes…strong no) ·
`notes` · `submitted_at` · blind-until-submitted = on.

**Offer** (K8 agreement + K22) · `application` · `version` · `base_salary` money · `bonus/equity?` · `start_date` ·
`probation_months` · `benefits` · `expires_at` · `letter` (generated) · `signature_status` · `state`
(draft → pending_approval → approved → sent → accepted/declined/expired/rescinded).

### 3. Variation points
Pipeline stages per opening type · scorecard rubrics · channels · agency module on/off · referral bonus on/off ·
size profile (starter: no requisitions, single approver) · markets: JP cohort fields & informal-offer stage,
DE works-council step, IN notice period & background check, US EEO group & eligibility checklist.

### 4. Approvals
- *Requisition*:
  - budgeted → hiring manager's head;
  - not budgeted → head → finance → HR.
- *Offer*:
  - within band → HR;
  - above band or with sign-on bonus → HR → finance → department head;
  - revised offer → re-approval of changed fields only.
- `[DE]` *Works council*: an external party step with a wait for the recorded decision (an approval node of type "external decision").

### 5. Rules & notifications
- Scorecard not submitted 24 h after interview → reminder (chat); 48 h → recruiter.
- Application idle in a stage over N days → recruiter task.
- Offer expires in 2 days → recruiter alert.
- Accepted → create S2 onboarding case (minimised data) and close opening if positions filled.
- Retention date reached → deletion task (K19).
- Candidate emails come from templates; a human sends them by default.

### 6. Data in/out
- In:
  - careers form: sync save; CV extraction runs async;
  - email-in;
  - job-board sources (async, scheduled) **[verify per board]**;
  - agency portal (guest);
  - bulk CV import (job).
- Out:
  - offer letter to e-signature (destination);
  - hire event → S2;
  - calendar invites (destination);
  - audit export.

### 7. Permissions
- `requisition.submit/approve` (scope: department, limit by band).
- `application.manage` (recruiters: assigned openings).
- `scorecard.submit` (assigned interviewers only; they see the candidate summary, not compensation).
- `offer.approve` (limit).
- `candidate.view_restricted` (HR).
- Agencies see only their candidates' status.

### 8. AI skills
CV extraction (A3) · duplicate candidate detection (B6, human merges) · JD drafting (C3) · scorecard summary (B4) ·
candidate message drafts (C1) · offer pre-check vs band (E1). **No AI ranking or rejection.**

### 9. Reports
Time-to-fill/hire · stage conversion & time-in-stage · source and agency effectiveness (incl. cost per hire from K9) ·
scorecard timeliness · offer acceptance and decline reasons · diversity (aggregate, restricted).

---

## S2 Employee lifecycle & HR requests
**Components:** K1 (employee role) · K4 (HR cases) · K6 (checklists) · K7 (assets, via S4) · K8 (employment contract) ·
K13 · K14 (check-ins) · K17 (policies) · K19 · K21 · K22

### 1. Real process
- **Main:**
  - **Onboarding:**
    1. Hire event (from S1 or manual).
    2. Onboarding case; **tasks generated by role, location and employment type**: IT account, equipment, workspace, payroll data, contract signature, training, buddy.
    3. Day-1 readiness check, then 30/60/90-day check-ins.
  - **Requests:** certificates, data changes (address, bank), leave, letters, and general requests, as cases with SLAs.
  - **Offboarding:** resignation or termination → exit date → tasks (asset return, access removal, final pay data, exit interview, knowledge hand-over) → ownership transfer of records the person owned.
- **Variants:** internal transfer/role change (re-run of the delta checklist, device swap); rehire (same party, new employee role); contractors (shorter checklist, end date); remote vs office onboarding.
- **Edge cases:**
  - Start date moves (relative task dates shift automatically).
  - Hire cancelled before start (cancel open tasks, retrieve shipped equipment).
  - Exit date moved earlier (urgent re-plan).
  - Manager leaves at the same time as their report (escalation path).
  - Personal-data change needs verification (bank data change requires a second check).

### 2. Entities & fields

**Employee** (K1 role *employee*; source may be an HR system)
| Field | Type | Role / note |
|---|---|---|
| employee_id | text (format per tenant) | identifier, xref to HR system |
| party (name, contacts) | K1 | |
| employment_type, job_title, grade | ref | |
| unit / cost_center / location | ref | dimension.* |
| manager | person | approver resolution (optional org module) |
| start_date / end_date | date | anchors for checklists |
| work_mode | ref (office/hybrid/remote) | checklist conditions |
| personal_data? | restricted group | birth date, address, emergency contact |
| bank_details? | restricted group | finance-only |
| `[IN]` PAN, notice_period; `[JP]` my-number handling **excluded by default** | | sensitive identifiers off unless legally needed |

**Lifecycle case** (K4 + K6): `type` (onboarding/offboarding/transfer) · `employee` · `anchor_date` · `checklist_template` ·
`tasks[]` (K6) · `progress` calc · `ready_for_day1` calc · `state`.

**HR request** (K4): `request_type` ref (tree) · `requester` · `subject_employee` (can differ) · `details` (field group per type) ·
`attachments` · `owner_team` · SLA fields · `resolution`.

**Checklist template** (K6): `items[]` (title, owner role/capability, relative due e.g. `start_date - 5d`, condition e.g.
`work_mode != remote`, required evidence).

**Check-in** (K14/K11): `employee` · `milestone` (30/60/90) · `questions` · `manager_notes` · `status`.

### 3. Variation points
Checklist templates per role/location/type · request types & their field groups · SLA per type ·
check-in milestones · market overlays (statutory documents per country as checklist items) · size profile.

### 4. Approvals
Data change requests (bank details: employee → HR verification, a second person by SoD) · leave (manager; long leave + HR) ·
letters (HR only) · transfer (old manager → new manager → HR; `[DE]` works council).

### 5. Rules & notifications
- Task due tomorrow → assignee.
- Day-1 readiness < 100% three days before start → HR lead.
- Exit date set → offboarding case.
- Asset return overdue → escalation chain (S4).
- Access removal not confirmed on exit day → security exception.

### 6. Data in/out
- In:
  - employees from an HR system/file (source, async);
  - hire events from S1;
  - forms for requests (sync).
- Out:
  - provisioning requests to IT tools via HTTP actions (async, result recorded);
  - payroll change feed (scheduled export).

### 7. Permissions
- `hr_case.handle` (scope: entity/location).
- `employee.view_personal` (HR).
- `employee.view_bank` (payroll).
- Managers see their team's cases and check-ins.
- Employees see their own.

### 8. AI skills
Checklist generation for unusual roles (E3, confirm) · policy Q&A with handover (E2) · request classification (B1) ·
summary of an employee's open items for managers (B4).

### 9. Reports
Onboarding readiness on day 1 · tasks overdue by owner team · request SLA by type · offboarding completeness (assets, access).

---

## S3 Expenses & reimbursement
**Components:** K3 (claim with lines) · K9 (budgets, allocations) · K10 · K12-lite (audit sample) · K21 · K1 (employee, merchant) ·
K16 · K17 (policy)

### 1. Real process
- **Main:**
  1. The employee captures receipts (photo or email-in) and the claim lines are built from them.
  2. Policy validation and pre-check.
  3. Manager approval, then finance review.
  4. Payment run (export to accounting/bank).
  5. Posted actuals to budgets.
- **Variants:**
  - *Corporate card*: card transactions are imported, matched to receipts and lines; personal spend is marked for repayment.
  - *Cash advance*: advance request → payment → later claim **settles against the advance** (balance owed either way).
  - *Mileage* (distance × rate per vehicle type, from settings).
  - *Per diem* (country/city rates, partial days, meals provided deductions).
  - *Travel trip* grouping several claims.
  - *Multi-currency* receipts with the exchange rate at the transaction date.
  - *Split allocation* across cost centres/projects.
- **Edge cases:**
  - Duplicate receipt (same receipt submitted twice or by two people).
  - Missing receipt (declaration with limit).
  - Partial approval (reject one line).
  - Receipt in a foreign language.
  - Tax reclaim requirements:
    - `[JP]` qualified invoice number check; simplified receipts below thresholds **[verify]**;
    - `[DE]` VAT ID for reclaim on larger amounts **[verify]**;
    - `[IN]` GSTIN for input credit.
  - Approver is the claimant's own subordinate (SoD block).
  - Claim spans two fiscal periods.

### 2. Entities & fields

**Expense claim** (K3 header)
| Field | Type | Role / note |
|---|---|---|
| claim_no | seq `EXP-{YYYY}-{seq:5}` | K21 |
| claimant | party:employee | party.requester |
| purpose / trip | text / link:Trips | |
| period | date range | |
| currency (reimbursement) | ref:currency | money.currency |
| total_claimed / total_approved / total_reimbursable | calc money | money.amount (approval threshold) |
| advance_applied | link:Advances, money | balance logic |
| cost_center / project | ref (default per employee) | dimension.* |
| state | lifecycle | draft → submitted → pending_approval → finance_review → approved → paid / rejected / returned |

**Expense line** (K3 line)
| Field | Type | Role / note |
|---|---|---|
| date | date | |
| category | ref:expense_category (tree) | classification.category, drives policy & attributes |
| merchant | text / party:merchant | |
| amount_original / currency_original | money | |
| fx_rate / amount_converted | num / calc | frozen at transaction date |
| tax_amount / tax_code | money / ref | market overlays |
| `[JP]` invoice_registration_no | text `T`+13 digits, validated | qualified invoice |
| `[IN]` supplier_gstin | text, checksum | |
| attendees? | persons/parties | meals & entertainment |
| distance_km? / vehicle_type? | num / ref | mileage category attribute set |
| per_diem_days? / meals_provided? | num / bool | per-diem category attribute set |
| receipt | files | evidence (AI A2 source) |
| missing_receipt_declaration? | bool + text | below limit only |
| allocation[] | K9 splits | |
| card_transaction | link:Card transactions | match status |
| policy_flags | calc (list) | rule results |
| line_status | ref | approved/rejected/adjusted |

**Card transaction** (source table, inbound only): `card_last4` · `holder` · `posted_at` · `merchant` · `amount` · `currency` ·
`match_status` (unmatched/matched/personal) · `matched_line`.

**Advance**: `requester` · `amount` · `paid_at` · `settled_amount` calc · `balance` calc · `state`.

### 3. Variation points
Expense category tree & attribute sets · policy limits (settings by category/country/grade) · per-diem and mileage rate tables
(reference data with validity dates) · card module on/off · advances on/off · approval policy · audit sampling rate ·
market overlays (tax fields).

### 4. Approvals
- Manager approval, with line-level approve/reject.
- Amount bands → second level.
- Policy exceptions → finance controller.
- Finance review as a "fill-in" step (payment date, GL mapping).
- **SoD**: the approver can't be the claimant, and can't be the claimant's subordinate.

### 5. Rules & notifications
- Validation:
  - receipt required above the threshold;
  - per-category limit (warn or block per setting);
  - duplicate receipt suspicion (hash + AI B6) → flag.
- Reaction: unmatched card transactions older than 15 days → holder reminder.
- Alert: budget line > 90% → owner.
- Payment executed → claimant notified.

### 6. Data in/out
- In:
  - receipts (photo/email-in; async extraction);
  - card statements (file/API source; async);
  - FX rates (source, daily);
  - rate tables (import).
- Out:
  - **payment batch export** (format per bank/accounting system, scheduled);
  - GL journal feed;
  - audit export with receipts.

### 7. Permissions
- `expense.submit` (own).
- `expense.approve` (scope: cost centres I own; limit).
- `expense.finance_review`.
- `expense.pay`.
- Auditors get read-only access to sampled claims.

### 8. AI skills
Receipt extraction (A2) incl. tax IDs validated by the format library · category suggestion (B1) · duplicate detection (B6) ·
pre-check findings (E1: policy, missing attendees for meals, weekend spend) · policy Q&A (E2).

### 9. Reports
Spend by category/cost centre/project · policy-violation rate · claim cycle time (submit → paid) · card match rate ·
advances outstanding · top merchants.

---

## S4 Assets & equipment
**Components:** K2 (asset models, category attributes) · K7 (custody) · K23 (consumables stock) · K14 (audits) · K12 (condition checks) ·
K6 (maintenance) · K4 (asset requests/incidents) · K21 · K22 (handover receipt)

### 1. Real process
- **Main:**
  1. Purchase or receipt (from S5).
  2. Tagging.
  3. In stock.
  4. Request or onboarding task → assignment with signed receipt.
  5. In use (maintenance, repairs).
  6. Return or swap → condition check → back to stock, repair, or retirement (disposal/data wipe certificate).
- **Variants:**
  - shared/pool assets (booked via K13);
  - consumables (quantity ledger, reorder level);
  - leased assets (lease end date);
  - remote employees (shipping legs with tracking numbers);
  - loaners during repair.
- **Edge cases:**
  - Lost/stolen (incident case, deactivate, insurance claim).
  - Asset found in audit that isn't in records (create from audit).
  - Record with no physical asset ("ghost" → audit exception).
  - Departing employee unreachable (escalation to manager/HR, cost recovery decision).
  - Assets moved between locations without a record (audit variance).

### 2. Entities & fields

**Asset model** (K2): `model_code` · `name` · `category` (tree: laptop, phone, monitor, furniture, vehicle…) ·
`manufacturer` · category attributes (e.g., CPU/RAM for laptops) · `standard_kit_roles[]` · `useful_life_months`.

**Asset** (K7)
| Field | Type | Role / note |
|---|---|---|
| asset_tag | seq `AST-{seq:6}` / barcode | K21; scan widget |
| serial_number | text, unique | |
| model | link:Asset models | item |
| status | lifecycle | in_stock, assigned, in_transit, in_repair, lost, retired |
| location | ref:location (tree) | dimension.region |
| custodian | party:employee | person.owner |
| purchase_doc / cost / currency | link:S5 PO, money | |
| purchase_date / warranty_until / lease_end? | date | rule anchors |
| condition | ref:condition | |
| depreciation_method? / book_value? | ref / calc | optional |
| data_wipe_certificate? | files | required to retire IT assets |

**Assignment** (K7 child, effective-dated): `asset` · `custodian` · `from` · `to` · `issued_by` · `signed_receipt` (K22/signature) ·
`shipping_tracking?` · `return_condition` · `return_photos`.

**Consumable movement** (K23): `item` · `location` · `movement_type` · `quantity` · `reference` · `by` · `at`.

**Audit round** (K14): `location` · `period` · `expected_assets` (calc) · `scanned[]` · `variance` calc · `status`.

### 3. Variation points
Category tree & attributes · signature required · depreciation on/off · audit frequency/sampling · consumables on/off ·
shipping legs on/off.

### 4. Approvals
Asset request above value or outside standard kit → manager → IT · disposal → IT lead (+ finance above book value) ·
lost asset write-off → finance.

### 5. Rules & notifications
- Exit date → return tasks for each assigned asset (S2).
- Return overdue: day 3 person → day 7 manager → day 14 HR exception.
- Warranty ends in 30 days → owner.
- Stock below reorder level → purchase request draft (S5).
- Audit variance → exception per missing asset.

### 6. Data in/out
- In: receipts from S5 · bulk asset import (template) · scanner app/form for audits (sync) · MDM/inventory tool as source **[verify]**.
- Out: asset register export · disposal certificates · feed to accounting (book values).

### 7. Permissions
- `asset.assign/return` (IT, scope: location).
- `asset.dispose` (IT lead).
- Employees see their own assets and confirm receipt.
- Auditors scan within their location.

### 8. AI skills
Serial/model extraction from photos of labels or invoices (A1/A2) · damage tagging on return photos (A5) ·
anomaly on loss rates by location (F2).

### 9. Reports
Assets by status/location · unreturned after exit · utilisation (assigned/in stock) · loss rate · warranty expiries · audit accuracy.

---

## S5 Procurement (request → order → receipt → invoice) — added
**Components:** K1 (supplier) · K2 (purchase items) · K3 ×4 (PR, PO, goods receipt, supplier invoice) · K8 (supplier contracts) ·
K9 (budgets, commitments) · K10 · K11 (supplier evaluation) · K12 (receipt QC) · K21 · K22 (PO PDF) · K23 (stock)

### 1. Real process
- **Main:**
  1. Purchase request (PR) with lines and allocation.
  2. Budget check (commitment).
  3. Approval by amount and category.
  4. Sourcing (quotes for amounts above a threshold).
  5. Purchase order (PO) to supplier (PDF/e-mail/portal).
  6. Supplier confirmation.
  7. Goods/service receipt (partial possible) with quality check.
  8. Supplier invoice capture.
  9. **Three-way match** (PO × receipt × invoice within tolerances).
  10. Approval for payment.
  11. Payment feed.
- **Variants:**
  - catalogue purchases (pre-approved items/prices);
  - framework contracts (call-offs against a contract value);
  - services (receipt = service acceptance by requester);
  - recurring subscriptions;
  - two-way match for services;
  - emergency purchases (post-approval);
  - supplier onboarding (pipeline K5: request → due diligence → approval → active).
- **Edge cases:**
  - Partial deliveries and multiple invoices per PO.
  - Price change at invoice (tolerance exceeded → exception).
  - Quantity received less than invoiced.
  - Duplicate invoice.
  - PO change after approval (re-approval of the delta).
  - Currency difference between PO and invoice.
  - Supplier without a valid tax registration `[JP]` (qualified invoice; affects input tax credit).

### 2. Entities & fields

**Supplier** (K1 role *supplier*)
| Field | Type | Role / note |
|---|---|---|
| supplier_code | seq | |
| legal_name / display_name | text | |
| tax_ids | identifiers | `[JP]` T-number, `[DE]` USt-IdNr, `[IN]` GSTIN+PAN, `[KR]` BRN, `[US]` EIN |
| addresses, contacts | contact points | |
| bank_details? | restricted group | change requires second-person verification |
| categories_supplied | ref:purchase_category (multi) | |
| status | lifecycle | prospect → onboarding → active → blocked → inactive |
| risk_rating / evaluation_score | ref / calc (K11) | |
| payment_terms / currency | ref | defaults to PO |

**Purchase request** (K3) — header: `pr_no` · `requester` · `needed_by` · `justification` · `cost_center/project` ·
`total` calc · `budget_status` calc · `state`. Lines: `item` or `description` · `quantity` · `unit` · `estimated_price` ·
`suggested_supplier` · `category` · `allocation[]`.

**Purchase order** (K3 converted from PR lines)
| Field | Type | Role / note |
|---|---|---|
| po_no | seq `PO-{YYYY}-{seq:5}` | K21, per legal entity |
| supplier | party:supplier | party.counterparty |
| contract | link:Agreements? | framework call-off |
| currency / fx_rate | ref / num | frozen |
| lines | lines (item, qty, unit_price, tax, delivery_date, pr_line link) | quantities ordered/received/invoiced calc |
| delivery_address | address | |
| terms | text/ref | |
| document | K22 PDF | |
| state | lifecycle | draft → pending_approval → approved → sent → confirmed → partially_received → received → closed / cancelled |

**Goods receipt** (K3 + K12 + K23): `gr_no` · `po` · `received_by` · `received_at` · lines (`po_line`, `qty_received`,
`qty_accepted`, `qty_rejected`, `reason`) · `inspection` (K12) · stock movements (K23).

**Supplier invoice** (K3): `invoice_no` (supplier's) · `supplier` · `invoice_date` · `due_date` · `currency` · lines ·
`tax` · `total` · `po_refs[]` · `match_status` calc (matched / price variance / quantity variance / no PO) ·
`variance_amount` calc · `document` (AI A2 source) · `state` (captured → matching → exception → approved → scheduled → paid).

### 3. Variation points
Quote requirement thresholds · match tolerances (qty %, price %, absolute) as settings · two-way vs three-way per category ·
catalogue module · supplier onboarding pipeline · budget block vs warn · approval policy by amount × category × entity.

### 4. Approvals
- *PR*:
  - ≤ A → cost-centre owner;
  - ≤ B → + department head;
  - > B → + finance;
  - IT categories → parallel IT review.
- *PO change*: delta above tolerance → re-approval.
- *Invoice exception*: buyer + finance.
- *Supplier onboarding*: procurement → compliance (+ finance for bank details).

### 5. Rules & notifications
- Delivery date passed without receipt → buyer + supplier reminder.
- Invoice variance beyond tolerance → exception to buyer.
- Contract value 80% consumed → contract owner.
- Contract ends in 60 days → renewal task.
- Supplier bank detail change → verification task (fraud control).

### 6. Data in/out
- In:
  - supplier invoices (email-in, upload; async extraction);
  - supplier master from ERP (source, inbound only) or native;
  - catalogue import;
  - FX rates.
- Out:
  - PO PDF/email to the supplier;
  - approved invoices → payment feed to accounting/ERP (scheduled);
  - commitments/actuals export.

### 7. Permissions
- `pr.submit` (all).
- `pr.approve` (scope: cost centre; limit).
- `po.issue` (buyers; category scope).
- `gr.record` (warehouse/requester for services).
- `invoice.process` (AP).
- `supplier.manage_bank` (finance; SoD with `invoice.process`).

### 8. AI skills
Invoice extraction (A2) · duplicate invoice detection (B6) · PR pre-check (E1: missing quotes, wrong category, budget) ·
category suggestion for free-text lines (B1) · spend anomaly alerts (F2).

### 9. Reports
Spend by category/supplier/cost centre · PR-to-PO and PO-to-receipt cycle times · match exception rate ·
on-time delivery by supplier · contract consumption · maverick spend (no PO).

---

## S6 E-commerce after-sales & returns
**Components:** K1 (customer) · K2 (products/SKUs) · K3 (return request with lines, refund) · K4 (case) · K10 · K12 (inspection) ·
K16 · K19 · K23 (restock) · K15 (return-rate metrics)

### 1. Real process
- **Main:**
  1. Customer request (form/marketplace/email/chat).
  2. Eligibility check (order found, within return window, item returnable).
  3. Return authorisation (RMA) with instructions/label.
  4. Item in transit (tracking).
  5. Received.
  6. Inspection/grading.
  7. Disposition (restock A-grade, refurbish, return to supplier, scrap).
  8. Resolution (refund to original payment / store credit / exchange / repair).
  9. Shop platform confirms.
  10. Customer notified.
  11. Root-cause tags.
- **Variants:**
  - *Refund without return* (low-value items).
  - *Exchange* (new outbound order, price difference).
  - *Partial return* of multi-item orders.
  - *Warranty claim* (repair, supplier claim).
  - *Marketplace-managed returns* (platform decides; we mirror status).
  - *Damaged/wrong item on delivery* (carrier claim).
  - *Chargeback/dispute* from the payment provider (evidence pack).
- **Edge cases:**
  - Return after the window (goodwill approval).
  - **Legal withdrawal periods** vary by market (`[DE]` 14-day withdrawal right for distance purchases; others by policy).
  - Item received doesn't match the RMA (wrong item/serial).
  - Customer returns more than authorised.
  - Repeat returner.
  - Bundled promotions (refund value allocation).
  - Refund fails at the payment provider.
  - Order not yet synced from the source (retry lookup).

### 2. Entities & fields
**Order / Order line / Customer** (source tables, inbound only): `order_no` · `channel/store` · `ordered_at` · `delivered_at` ·
`customer` · `currency` · lines (`sku`, `qty`, `unit_price`, `discount_allocated`, `serial?`) · `payment_method` · `status`.

**Return request** (K3 + K4)
| Field | Type | Role / note |
|---|---|---|
| rma_no | seq `RMA-{YYMM}-{seq:5}` | K21 |
| order | link:Orders (source) | verified on intake |
| customer | party:customer | party.requester |
| channel | ref | form/marketplace/email/chat |
| lines | lines (order_line, qty_requested, reason, customer_comment, photos) | classification.reason per line |
| request_type | ref | refund, exchange, repair, refund-without-return |
| within_window | calc | from delivered_at + window setting (market overlay) |
| risk_band | calc | value, SKU risk list, customer history |
| return_method / label / tracking_no | ref / files / text | |
| received_at | datetime | from warehouse form or source |
| state | lifecycle | requested → authorised → in_transit → received → inspected → resolved → closed (rejected / cancelled) |
| SLA fields | K4 | per value band |

**Inspection** (K12) per returned line: `grade` (A/B/C/scrap) · `checklist results` · `photos` · `serial_match` bool · `disposition` ref.

**Resolution / Refund** (K3): `type` (refund/store credit/exchange/repair) · `amount` calc (incl. restocking fee setting, shipping) ·
`method` · `approval` · `external_refund_id` (response from shop/payment API) · `confirmed_by_source_at` · `state`.

**Review** (source + local fields): `platform` · `rating` · `text` · `sku` · `posted_at` · local: `issue_type`, `case`, `reply_status`, `reply_text`.

### 3. Variation points
Return windows and reasons per market/category · refund-without-return threshold · restocking fee rules · risk SKU list ·
inspection checklists per category · disposition codes · exchange on/off · warranty module on/off · marketplace mirror mode.

### 4. Approvals
Auto-approve below value A with low risk · team lead ≤ B · lead + finance above B or high risk · goodwill outside window → lead ·
supplier claim → purchasing (S5 link).

### 5. Rules & notifications
- 1–2★ review → case + alert.
- Return authorised/received/refunded → customer message (template, consent, quiet hours; SMS/chat per market).
- Refund not confirmed by source in 24 h → exception.
- SKU return rate above threshold (7-day window) → category manager task.
- Repeat returner (N returns in 90 days) → flag on next request.

### 6. Data in/out
- In:
  - orders, customers, reviews from shop platforms (push/pull, async);
  - public return form with order verification (sync);
  - carrier tracking (pull);
  - warehouse inspection form (sync, photos).
- Out:
  - **refund action** to shop/payment API (async; status returns via source);
  - customer messages;
  - restock movements to the WMS feed;
  - supplier claim export.

### 7. Permissions
- `return.handle` (scope: store/region).
- `refund.approve` (limit).
- `refund.execute` (system/finance).
- `customer.view_contact` (masked for most).
- Warehouse role sees inspection tasks only.

### 8. AI skills
Reason classification from customer text (B1) · damage photo grading suggestion (A5) · reply drafts in the customer's language (C1+B5) ·
refund pre-check (E1: window, history, inspection) · review issue tagging (B1/B3) · return-reason narrative (F3).

### 9. Reports
Return rate by SKU/reason/supplier · RMA-to-refund cycle time · refund value by band/approver · grade/disposition mix ·
review response time · repeat returners.

---

## S7 Customer service desk (any industry) — added
Same components as S6 without returns: **K4 case** + K16 threads + K17 knowledge + K11 satisfaction survey + K1 customers/accounts.

- **Real process:**
  1. Contact via email/chat/form/phone log.
  2. Case created or merged with an existing one.
  3. Classified and routed (team, skills, language).
  4. First response within SLA.
  5. Work, with internal notes and escalation to tier 2 or other teams.
  6. Waiting on customer (**SLA paused**).
  7. Resolved, then survey.
  8. Reopened within N days if needed.
  9. Closed.
- **Variants:** VIP/contract SLAs per customer tier · B2B accounts with several contacts · problem cases linking many incidents · field visits (K13).
- **Fields (case additions):** `account` · `contact` · `product/service` ref · `language` · `tier` · `sla_policy` calc ·
  `first_response_due` · `resolution_due` · `paused_minutes` calc · `escalation_level` · `problem` link · `csat` · `knowledge_used[]`.
- **AI:** routing (B1) · reply drafts with knowledge citations (C1+E2) · summary on hand-over (B4) · duplicate/related cases (B6).
- **Reports:** SLA attainment by tier/team · backlog ageing · first-contact resolution · CSAT · top contact reasons.

---

## S8 Content production & creator collaborations (New Media)
**Components:** K2 (formats) · K5 (creator collaboration pipeline) · K6 (production tasks) · K8 (collaboration agreement with deliverables & usage rights) ·
K9 (budget) · K11 (content review) · K12 (pre-publish compliance check) · K13 (shoots, publishing slots) · K15 (metrics) · K16 · K20 (series/episodes)

### 1. Real process
- **Main (own content):**
  1. Topic idea (pool) with benchmark references.
  2. Prioritised.
  3. Brief.
  4. Script.
  5. Shoot (booking crew, location, equipment from S4).
  6. Edit (versions).
  7. Review (brand, legal, disclosure).
  8. Publish to one or more platforms/accounts (adapted cuts per platform).
  9. Metric snapshots at +1 h/+24 h/+7 d/+30 d.
  10. Retrospective; topic score updates.
- **Main (creator collaboration):**
  1. Creator discovery (pool).
  2. Shortlist and approval.
  3. Brief and offer.
  4. Contract (deliverables, usage rights, exclusivity, disclosure) and signature.
  5. Draft submitted.
  6. Brand review (rounds).
  7. Published (link captured).
  8. Delivery verified against the contract.
  9. Payment approval.
  10. Performance and usage-rights tracking.
- **Variants:** series/episodes (hierarchy) · repurposing one master into many cuts · live streams · paid boosting of posts (link to S9 spend) · agency-managed creators.
- **Edge cases:**
  - Publication removed or edited by the platform.
  - Creator posts without the required ad disclosure (`[JP]` stealth-marketing rules since Oct 2023; FTC disclosure in the US; similar rules elsewhere) → compliance exception.
  - Usage rights expire while content is in paid ads.
  - Metrics unavailable due to API limits (snapshot gaps marked).
  - Draft rounds exceed the contract maximum (change request).

### 2. Entities & fields

**Content item** (K6 + K20)
| Field | Type | Role / note |
|---|---|---|
| content_no | seq | |
| title / working_title | text | |
| topic | link:Topics | |
| series / episode_no | link / num | K20 |
| format | ref:format (short video, long video, article, live, image post) | category attributes (duration, aspect ratio) |
| owner / editor / reviewer | person | |
| stage | lifecycle | idea → brief → script → shoot → edit → review → scheduled → published → retrospective |
| due_per_stage | dates (relative to publish date) | K6 generation |
| assets | files (versions) | |
| sponsored | bool | triggers disclosure check |
| disclosure_text | text | `[JP]` required when sponsored |

**Publication** (content × account): `platform/account` ref · `planned_at` · `published_at` · `url` · `platform_post_id` (xref) ·
`cut/version` · `paid_boost` link:S9 · latest metrics (views, likes, comments, shares, watch time, followers gained) calc from K15.

**Metric snapshot** (K15): `publication` · `metric` · `captured_at` · `value` · `source_run`.

**Creator** (K1 role *creator*): `handles[]` (platform, handle, followers, audience country/age mix) · `categories` · `rate_card` money ·
`agency` · `past_collaborations` calc · `rating` (K11) · `payment_details?` restricted.

**Collaboration** (K5 + K8)
| Field | Type | Role / note |
|---|---|---|
| creator | party:creator | |
| campaign | link:S9 Campaigns? | |
| deliverables | obligations (type, platform, count, due, max_revision_rounds) | K8 obligations |
| fee / currency / payment_schedule | money / lines | K9 commitment |
| usage_rights | rights (scope, channels, regions, valid_until) | expiry rules |
| exclusivity | text + dates | |
| disclosure_required | bool + required wording | compliance check |
| contract | K22 + signature status | |
| stage | lifecycle | shortlisted → approved → contracted → in_production → delivered → verified → paid → closed |

### 3. Variation points
Stages per format · review rounds and reviewers per sponsored/non-sponsored · metric capture schedule · platforms/accounts ·
collaboration module on/off · budget on/off · market disclosure rules.

### 4. Approvals
- Topic priority (lead).
- *Content review*:
  - editor;
  - sponsored → brand + legal **in parallel**;
  - final → editor-in-chief.
- Collaboration budget by amount.
- Creator payment after verification (finance).

### 5. Rules & notifications
- Stage overdue → owner, then lead.
- Publish date in 24 h and review not approved → alert.
- Usage rights expire in 14 days → remove/renew task.
- Snapshot jobs scheduled on publish.
- Content below benchmark after 7 days → retrospective task.
- Sponsored and disclosure missing → block publish (validation).

### 6. Data in/out
- In:
  - platform metrics (scheduled pulls per account, async) **[verify each platform]**;
  - creator applications (public form);
  - assets upload.
- Out:
  - publishing calendar feed;
  - creator payment file;
  - performance report.

### 7. Permissions
- `content.edit` (team scope).
- `content.approve`.
- `collaboration.approve_budget` (limit).
- `creator.view_payment` (finance).
- Creators (guests) see their briefs, drafts and payment status.

### 8. AI skills
Title/script variants per platform and language (C2) · transcript summaries (B4) · topic tagging (B1) · disclosure and claims pre-check (E1) ·
comment classification and reply drafts (B1/C1) · performance narrative and "topics vs benchmark" questions (F3/F1).

### 9. Reports
Output and on-time rate by stage · performance by topic/format/platform/account vs benchmark · creator ROI (fee vs results) ·
rights expiring · review cycle time.

---

## S9 Marketing campaigns & leads
**Components:** K3 (budget plan lines) · K9 · K5 (lead pipeline) · K1 (leads/contacts/accounts) · K6 · K11 (content review) · K15 (daily spend/results) ·
K13 (events) · K16 · K19 (marketing consent)

### 1. Real process
- **Main:**
  1. Annual/quarterly plan with budget lines by channel/region/product.
  2. Campaign brief.
  3. Approval.
  4. Asset and content production (S8 pipeline).
  5. Brand/legal review.
  6. Launch checklist.
  7. Daily spend and results collection.
  8. Leads captured (forms, events, imports).
  9. Qualification (MQL).
  10. Hand-off to sales with acknowledgement (SQL).
  11. Outcome feedback.
  12. Post-campaign review.
- **Variants:** events (registration, attendance, follow-up) · partner/co-marketing (shared budgets) · always-on campaigns · regional campaigns under a global plan.
- **Edge cases:**
  - Lead without marketing consent (no nurturing, sales contact only if legitimate interest **[verify per market]**).
  - Duplicate leads across campaigns (attribution rule: first/last touch setting).
  - Spend data arrives late or is revised by the platform (snapshot corrections).
  - Budget reallocation mid-quarter (approval).

### 2. Entities & fields
**Budget line** (K9): `period` · `channel` (tree) · `region` · `product_line` · `budgeted` · `committed` calc · `actual` calc · `owner`.

**Campaign**: `campaign_no` · `name` · `objective` ref · `channels[]` · `regions[]` · `start/end` · `budget_lines[]` ·
`target KPIs` (leads, CPA, ROAS) · `owner` · `brief` longtext · `state` (draft → approved → in_production → live → completed → reviewed).

**Spend & results snapshot** (K15): `campaign` · `channel/account` · `date` · `spend` money · `impressions` · `clicks` · `conversions` · `revenue?`.

**Lead** (K1 + K5)
| Field | Type | Role / note |
|---|---|---|
| name, email, phone, company, title | K1 | |
| country / language | ref | routing |
| source_campaign / first_touch / last_touch | links | attribution setting |
| consent_marketing / consent_text_version | K19 | gating for nurture |
| score | num (rules) | rule-based scoring (transparent) |
| stage | lifecycle | new → MQL → handed_off → accepted (SQL) / rejected → won/lost (from CRM) |
| sales_owner / accepted_at | person / datetime | SLA for acceptance |
| rejection_reason | ref | feedback loop |

### 3. Variation points
Channel tree · attribution model · lead scoring rules · hand-off SLA · events module · consent rules per market.

### 4. Approvals
Plan & budget (finance) · campaign brief by amount · content (brand → legal if claims) · budget reallocation above % (head of marketing).

### 5. Rules & notifications
Budget line ≥ 90% → owner · CPA above target 3 consecutive days (F2) → task · new MQL → sales owner (acknowledge in N hours, else escalate) ·
lead rejected → marketing owner digest.

### 6. Data in/out
- In: ad platforms spend/results (scheduled pulls) **[verify]** · lead forms (public) · event registrations · list imports with consent mapping.
- Out: leads to CRM (event/HTTP action; CRM stays the source for opportunities, which flow back inbound) · campaign reports.

### 7. Permissions
`campaign.manage` (scope: region/brand) · `budget.approve` (limit) · `lead.accept` (sales, region) · `lead.view_contact` (masked for agencies).

### 8. AI skills
Copy variants per locale (C2) · brand/claims pre-check (E1) · lead extraction from emails (B2) · spend anomaly alerts (F2) · campaign narrative (F3).

### 9. Reports
Budget vs actual · CPA/ROAS by channel/region · lead funnel and acceptance SLA · campaign retrospectives.

---

## S10 R&D: requirements, bugs, releases
**Components:** K4 (bug intake, support escalations) · K6 + K20 (epics/stories/tasks) · K13 (iterations as time boxes) · K11 (reviews) ·
K12 (test runs) · K10 (release approval) · K21

### 1. Real process
- **Main:**
  1. Requests (from customers, support S7, internal) are triaged.
  2. Accepted into the backlog (epic/story).
  3. Prioritised.
  4. Planned into an iteration.
  5. In progress → review → test.
  6. Done.
  7. Released (release notes) → feedback.
  8. Bugs follow a severity SLA path with reproduction, fix and verification.
- **Variants:** Kanban flow without iterations · hotfix release path · multiple products and versions (affected/fixed versions) · regulated change approval.
- **Edge cases:**
  - Duplicate bugs (merge, keep reporter links).
  - A bug affects several versions (backport tasks).
  - Release blocked by an open sev-1.
  - Reopened after release.
  - A story spans iterations (carry-over with history).

### 2. Entities & fields
**Work item** (K6 + K20): `key` seq (`PRJ-123`) · `type` (epic/story/task/bug) · `title` · `description` · `parent` · `status` · `assignee` ·
`priority` · `estimate` (points/hours) · `iteration` · `component` (tree) · `labels` · `links` (blocks/relates/duplicates) · `code_links[]` (inbound events).

**Bug additions**: `severity` ref (definitions per product) · `environment` · `affected_versions[]` · `fixed_version` · `steps_to_reproduce` ·
`reporter` (party; may be a customer via S7) · `customer_count` calc (linked cases) · SLA by severity.

**Iteration**: `name` · `start/end` · `capacity` · `committed` calc · `completed` calc.

**Release**: `version` · `product` · `planned_date` · `items[]` calc · `open_blockers` calc · `qa_signoff` · `notes` (AI draft) · `state`.

**Test case / test run** (K12): `steps` · `expected` · `result` · `build` · `evidence`.

### 3. Variation points
Work item types and states · severity definitions · iterations on/off · estimation unit · release approval policy · component tree.

### 4. Approvals
Release: QA sign-off → (high-risk components → security review, parallel) → release manager · hotfix: lighter path with post-review task.

### 5. Rules & notifications
Sev-1 created → on-call (chat + SMS) · bug SLA breach → lead · item idle in review > N days → chase · release date in 3 days with blockers → alert ·
release published → linked S7 cases notified.

### 6. Data in/out
- In: code host events (push inbox: commits/PRs linked by key) **[verify]** · support escalations from S7 · import from other trackers.
- Out: release notes · status events to S7 · changelog feed.

### 7. Permissions
`work_item.edit` (team scope) · `bug.triage` · `release.approve` · support sees status of linked items only.

### 8. AI skills
Severity/component suggestion (B1, per-product examples) · duplicate detection (B6) · reproduction steps from support text (B2) · release notes (C3) · delivery narrative (F3).

### 9. Reports
Lead/cycle time · throughput · escaped defects · bug SLA by severity · iteration predictability · release scope changes.

---

## S11 Projects & tasks — added
**Components:** K6 + K20 (phases, tasks, dependencies) · K13 (milestones, bookings) · K9 (project budget) · K14 (status reports) ·
K8 (client/vendor agreements) · K11 (stage-gate reviews)

- **Real process:**
  1. Project request → approval.
  2. Plan (phases, tasks, owners, dependencies, milestones).
  3. Execution with weekly status reports (collection round by project).
  4. Risks and issues logs.
  5. Change requests (scope/budget/date, approval).
  6. Stage-gate reviews.
  7. Closure with lessons learned.
- **Fields (project):** `project_no` · `type` · `sponsor` · `manager` · `start/end` (planned/forecast/actual) · `budget` (K9) ·
  `health` (rag, rule-computed + manual override with reason) · `phase` · `progress` calc (rolled-up) · `risks[]` (probability, impact, owner, mitigation) ·
  `issues[]` · `change_requests[]` (K10).
- **Variants:** client projects (billable hours, invoices from S5-like documents) · internal programmes with sub-projects (K20) · templates per project type.
- **Reports:** portfolio health · milestone slippage · budget burn · resource load.

---

## Reuse matrix (why templates assemble quickly)

| Component | S1 | S2 | S3 | S4 | S5 | S6 | S7 | S8 | S9 | S10 | S11 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| K1 Party & roles | ● | ● | ● | ● | ● | ● | ● | ● | ● | ○ | ○ |
| K2 Catalogue & category attributes | ○ |  |  | ● | ● | ● | ○ | ○ |  |  |  |
| K3 Document with lines | ● |  | ● | ○ | ● | ● |  |  | ● |  | ○ |
| K4 Case |  | ● | ○ | ○ |  | ● | ● |  |  | ● |  |
| K5 Pipeline | ● |  |  |  | ○ |  |  | ● | ● |  | ○ |
| K6 Work items & checklists | ○ | ● |  | ○ |  |  |  | ● | ○ | ● | ● |
| K7 Asset custody |  | ○ |  | ● |  |  |  | ○ |  |  |  |
| K8 Agreement | ● | ○ |  |  | ● |  |  | ● |  |  | ○ |
| K9 Budget & actuals | ○ |  | ● |  | ● |  |  | ● | ● |  | ● |
| K10 Approval binding | ● | ● | ● | ● | ● | ● | ○ | ● | ● | ● | ● |
| K11 Evaluation | ● | ○ |  |  | ● |  | ● | ● | ○ | ○ | ○ |
| K12 Inspection |  |  | ○ | ● | ● | ● |  | ● |  | ● |  |
| K13 Schedule & booking | ● | ○ |  | ○ |  |  | ○ | ● | ○ | ○ | ● |
| K14 Collection round |  | ● |  | ● |  |  |  |  | ○ |  | ● |
| K15 Metric snapshots |  |  |  |  |  | ● |  | ● | ● |  | ○ |
| K16 Communication thread | ● | ○ |  |  | ○ | ● | ● | ● | ● | ○ |  |
| K17 Knowledge | | ● | ● |  |  |  | ● |  |  |  |  |
| K19 Consent & retention | ● | ● |  |  |  | ● | ● | ○ | ● |  |  |
| K20 Hierarchy |  |  | ○ | ○ |  |  |  | ● |  | ● | ● |
| K21 Numbering | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |
| K22 Document output & signature | ● | ● |  | ○ | ● |  |  | ● |  |  |  |
| K23 Quantity ledger |  |  |  | ● | ● | ● |  |  |  |  |  |

● core ○ used

**Assembly test:** a new template, such as *facility & visitor management* (K4 requests + K13 room
booking + K1 visitor role + K19 consent + K16 notifications) or *quality audits* (K14 rounds + K12
inspections + K4 non-conformance cases + K6 corrective actions), can be assembled **only from existing
components**. This is the acceptance criterion for the component layer: a new industry-standard
scenario should need configuration and content, not new code.

---

## Validation plan
1. **Template-only build:** each of S1–S11 is built using only components and configuration, with zero template code. A CI check rejects templates that reference anything outside the component catalogue.
2. **Variant tests:** for each template, run the main flow, every listed variant and every listed edge case as automated scenario tests on SQLite/MySQL/PostgreSQL.
3. **Market overlay tests:** JP/DE/IN/KR/US overlays applied to each template. Validators get test vectors (tax IDs, windows, required steps).
4. **Size profile tests:** starter, standard and enterprise profiles install and upgrade cleanly.
5. **New-scenario drill:** before the first release, assemble two scenarios not listed here (e.g., facility management, quality audits) to prove the abstraction. Every gap found becomes a component change, not a template hack.

## Sources
- Universal data patterns: [TDAN person/organisation model](https://tdan.com/?p=5014), [TDAN patterns for agile projects](https://tdan.com/?p=22775), [Data Model Resource Book Vol. 3](https://www.oreilly.com/library/view/the-data-model/9780470178454/chapter-47.html), [Fowler, Analysis Patterns ch. 2](https://www.martinfowler.com/apsupp/apchap2.pdf)
- Japan qualified invoices: [Invoice Data Extraction guide](https://invoicedataextraction.com/blog/japan-qualified-invoice-system-qis), [EU-Japan Centre](https://www.eu-japan.eu/qualified-invoice-system)
- Germany withdrawal right: [German administration portal](https://verwaltung.bund.de/leistungsverzeichnis/en/rechte-und-pflichten/102837988)
- Japan stealth marketing rules: [Japan Times](https://www.japantimes.co.jp/news/2023/03/28/national/stealth-marketing-rules/), [Baker McKenzie](https://insightplus.bakermckenzie.com/bm/consumer-goods-retail_1/japan-consumer-affairs-agency-issues-its-first-stealth-marketing-related-order-for-action_1)
- Industry challenges per scenario: see doc 12 source lists.

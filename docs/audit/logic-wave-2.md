# Logic audit — wave 2 (business engines and the product build)

Audit date: 2026-11-26. Question asked of every module: *is the behaviour right for the business and
sound in its reasoning* — independent of whether it matches the specification text. Focus this wave:
computation (formulas, totals), access decisions and their explanations, process control (who may
decide, what is locked, what can never get stuck), and the integrity of configurations people build
themselves.

Method:

- `mt-lifecycle` and `mt-perm` read line by line; every approval transition traced by hand; then
  pinned by reference-model tests and seeded simulations (a failure names its seed).
- `mt-formula`, `mt-query`, `mt-refdata` reviewed while written; conclusions pinned by properties
  (type soundness, total order, partitions, overlay rules, history replay).
- The product: each control (locks, approval authority, ownership, deletion) attacked through the API
  as another person, an agent, or with a stale view; then the browser walks.

## 1. Findings

Severity as in wave 1: **High** = wrong business result possible; **Medium** = wrong or surprising
result in a realistic situation; **Low** = hardening; **Info** = right, but must be known.

| ID | Sev. | Where | Finding | Resolution |
|---|---|---|---|---|
| L2-01 | High | `mt-lifecycle::approval` | When several tasks of one step became overdue in the same tick, each escalation target was computed before any was applied, so all went to the same person — one person could then decide a step meant for several | **Fixed** (found by the random-event simulation): escalations apply one at a time, each excluding everyone already asked. Regression test `tasks_overdue_together_are_not_all_escalated_to_one_person` |
| L2-02 | High | `mt-formula` checker | `IF` with alternatives of different types was typed as the first branch: a formula could pass the check and produce a text where a number field expected a number | **Fixed**: mixed alternatives type as `Any`; a computed field checks every possible outcome |
| L2-03 | Medium | `mt-formula` evaluator | Arithmetic on an empty date produced a number (days since the epoch) instead of nothing | **Fixed**: empty date operands give an empty result |
| L2-04 | High | product: approvals | The lines of a request under approval could still be edited, so the amount people approved was not the amount ordered | **Fixed**: states lock their fields and the records composed into them (lines), including moving lines in or out and deleting them |
| L2-05 | Medium | product: links | A draft could take a line from a submitted request by linking it (single-record links move the line), changing the submitted total | **Fixed**: a change that takes children from another parent needs that parent to allow it |
| L2-06 | High | product: approvals | An approver chosen because they held a capability kept their open task after the capability was revoked, and could still approve | **Fixed**: approve / reject / send back re-check a capability-named approver's right at decision time (delegated and escalated tasks excepted); test `std_a_withdrawn_right_no_longer_decides_open_tasks` |
| L2-07 | Medium | `mt-lifecycle::approval` | Delegation or an added approver could go to someone already asked at the step, or still waiting in a sequential queue — one person deciding twice | **Fixed**: only people not yet asked (queue included) |
| L2-08 | Medium | product: access | “Records they own” ignored the creator in tables without a person field, so an employee could not read the lines she had just entered | **Fixed**: the creator is an owner for the policy in every table |
| L2-09 | Low | product: designer | Refusals named states by internal key (`on_hold`) | **Fixed**: by the names people typed |
| L2-10 | Low | product: views | Explanations named “the Purchasing base” for every base; link titles containing commas showed as several records; a new record lost its “Saved.” confirmation | **Fixed** |
| L2-12 | High | `mt-refdata::ledger` | Found while bringing the ledger into the product: the engine had no recorded way to change a list's policy, so it could only be changed outside the history — one person could have switched four eyes off and then changed the list unchecked | **Fixed** before use: the policy is a recorded change (`Change::SetPolicy`); under approval, switching it off is proposed and needs a second person. The ledger model test now checks that every version made while approval was required carries an approver |
| L2-11 | Medium | `mt-lifecycle::calendar` | The module's own example said 8 working hours from Friday 16:00 end Monday 15:00; the code (correctly) says 16:00 | **Fixed** (documentation); the example is now a test |

## 2. Reasoning checks that passed

**Nobody approves their own request — three times over.** The approval engine never resolves the
requester as an approver; a separation-of-duties rule (`record.create` vs the approval capability on
the same record) refuses it in the policy; and capability-named approvers are resolved by asking the
policy for this very record, which applies both. Each layer is tested on its own: the engine's
requester rule (`the_requester_never_approves_their_own_request`), the policy's same-record rule
(`tests/policy.rs`), and the product's routing (Carol's own requests skip her, `tests/process.rs`).

**What is approved is what is ordered.** Entering a waiting state locks the record's fields and its
lines; the state field changes only through transitions; limits are conditions on the record's total,
which is locked while the approval runs. So the limit checked when the approver was chosen still holds
when they decide (L2-06 covers the other half: the approver's right itself).

**No record can get stuck by design.** The designer publishes only lifecycles where every state is
reachable and every state can reach a final state (property: the check accepts exactly such
lifecycles). At run time, an approval whose step resolves to nobody — an amount above every limit —
becomes *stuck*, not silently approved or rejected: administrators are told, the requester can
withdraw, and no task stays open (simulation invariant).

**Approvals are deterministic and survive restarts.** The same events give the same approval (500
seeds); a copy saved as JSON at a random point continues identically (in 1,500 random runs). Running approvals keep
their definition version; the designer creates a new version only when the steps change.

**Order of decisions does not matter where it should not.** In “all” and quorum steps, every order in
which people decide gives the same outcome (all 24 orders of four approvers, checked on several
threads). In “any” the first decision decides, by design.

**Access is additive and explainable.** There are no deny rules; every refusal lists, for each grant
that came close, why it did not apply. Lists use a row filter that agrees with the per-record decision
for every record (property). An agent acting for a person never exceeds either (property).

**Money.** Totals refuse to add different currencies; averages of money round half-even to the
currency's minor units; empty is never zero; limits compare money with money in the same currency
(a limit in another currency does not apply rather than converting silently).

**A share link never outlives its maker's rights.** A link is not a stored copy: each time it is
opened it is evaluated as its maker's current access (row filter and field permissions), narrowed by
the link itself (fields left out, no classified or protected fields, no links into other tables). A
maker who loses `share.manage` breaks the link; a maker who loses sight of a field or record takes it
out of the link (test `std_share_links_show_one_table_and_never_more_than_their_maker_sees`).

**Configurations cannot be broken by editing them.** A field used by a formula, lifecycle, approval or
access rule cannot be deleted (the users are named); a table others link to cannot be deleted; a
design that removes a state still holding records is refused; removed select choices are disabled,
never deleted, so records keep their values. Every building step is in the audit chain.

## 3. Items handed to later waves

| Item | Wave | Check to write there |
|---|---|---|
| One lock per workspace today; per-base single writer with fencing (wave 1) and the operation pipeline | 3 | Concurrent transitions on one record from two nodes: exactly one applies |
| Re-resolve capability-named approvers when grants change (today: the old approver is refused, an administrator reassigns) | 5 | Rule-driven reassignment on grant change |
| SLA clocks on states; overdue approvals ticked by the scheduler | 5 | Timer fires once after a crash between cursor move and fire (L1-05) |
| Settings scopes in the product (R2-05) | 3 | Base/person overrides resolve and explain |
| Limits in another currency than the record | 6 (FX rates) | Convert with the rate of the record's date, or refuse with the reason |

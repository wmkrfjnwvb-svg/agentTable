# mt-lifecycle

Process engines of the product (design doc 17: domain D7, capability C6, processes P5 and P6):
record lifecycles, business calendars, SLA clocks and the approval engine.

Pure and sans-IO: no clock, no I/O, no randomness. Time, the record's values and "who holds
this capability" are inputs; the state machines return what happened. The crate builds for
`wasm32-unknown-unknown`, so the browser can run the same rules (for example the designer's
"simulate" button).

## Modules

### `lifecycle` — states and transitions

* `Lifecycle { states, initial, transitions }`. Every `StateDef` has a `StateCategory` (new,
  in progress, waiting internal/external, pending approval, done success/failure, cancelled), so
  SLA pauses, "my open work" and dashboards work on any lifecycle without knowing its names.
  A state can list the fields `required` to enter it and the only fields `editable` in it.
* `Transition { from, to, capability, guard, approval }`. An empty `from` means "from any open
  state" (a generic *Cancel*).
* `check()` enforces the **closed-loop rule**: valid keys, a non-final initial state, at least
  one final state, no self-loops, every state reachable from the initial one, and from every
  state some final state reachable — no record can get stuck by design.
  `check_in_table(type_of)` checks field references and guard conditions against the table.
* `offers(current, value_of, ctx, may)` lists every way out of the current state with what
  blocks it (`Blocker::WrongState | Capability | Guard | Required`); `take(...)` returns the
  `Move` to apply (including the approval to start) or *all* blockers at once.
  `may_edit(state, field)` and `tally(states)` serve the record form and boards.

### `calendar` — business calendars

* `BusinessCalendar { week, holidays, utc_offset_minutes }`; `office(open, close, offset)` and
  `always()` build common ones.
* `add(start, duration)` is the deadline in working time, `between(a, b)` the working time in
  `[a, b)`, `next_working(at)` the first working instant. Working days may cross UTC midnight.
* The offset is fixed: where daylight saving applies the caller keeps one calendar per period.
  Walks are bounded to `MAX_SPAN_DAYS` (about ten years) and return `None` beyond.

### `sla` — SLA clocks

* `SlaPolicy { name, target, warn_percent, calendar }`, `SlaClock::start(policy, at)`.
* `step(SlaInput::{Pause, Resume, Stop, Tick})` returns `SlaEvent::{Warned, Breached, Met,
  Missed}`. The warning and the breach are each reported once, with the *exact* instant they
  happened (a late tick still reports when the target was exceeded). A breach is strictly after
  the due instant. Time going backwards and inputs that do not fit the state are errors and
  change nothing.

### `approval` — the approval engine

* `ApprovalDef { key, name, version, steps }` with `Node`s:
  * `Approval { approvers, mode, allow_return, timeout, escalate_to }` with
    `Mode::{Sequential, All, Any, Quorum { required }}` and approvers that are a person, the
    people in a field of the record, or the holders of a capability;
  * `Branch { cases, otherwise }` — the first case whose condition holds, by priority;
  * `Parallel { branches, join: Join::{All, Any, AtLeast { count }} }`;
  * `FillIn { assignee, fields }` — someone completes fields;
  * `Notify { to }` — people are told, nobody waits.
* `Approval::start(def, requester, env)` and `Approval::step(def, Event, env)` with
  `Event::{Act { task, actor, act }, Tick, Cancel}` and
  `Act::{Approve, Reject, Return, Submit, Delegate, AddApprover, Comment}`. Outputs:
  `Opened`, `Closed`, `Notified`, `Reassigned`, `Overdue`, `Stuck`, `Finished`.
* `simulate(def, requester, env)` walks the definition as if everyone approved and lists who
  would be asked at each step.

## Invariants (enforced and tested)

* The requester never approves their own request: they are never resolved as an approver, and
  approval tasks cannot be delegated or added to them. (They can be told, and they can fill in
  fields.)
* Only the assignee acts on an open task; a rejection or a send-back needs a reason; a refused
  event changes nothing.
* Nobody holds two open tasks of one step: delegation, added approvers and escalation only go to
  people not yet asked at that step (including those still waiting their turn).
* In a parallel block, the first rejection or send-back ends the whole approval.
* A step that resolves to nobody (or to fewer people than its quorum) makes the approval
  `Stuck`; it then has no open task and waits for an administrator to cancel it.
* `Running` ⇒ at least one open task; ended or stuck ⇒ none. `Finished` is reported once.
* A running approval keeps the definition version it started with (`LifecycleError::Version`).
* Every approval opens at most `MAX_TASKS` tasks; definitions have at most `MAX_NODES` steps,
  nested at most `MAX_NESTING` deep.
* Approvals and clocks serialise to JSON and resume exactly where they were.

## Tests

| File | Category | Tests |
|---|---|---|
| `tests/calendar.rs` | standard, time zones, edge, invalid, 3 properties | 19 |
| `tests/sla.rs` | standard, invalid, anomalous, model-based (reference model, 512 cases) | 11 |
| `tests/lifecycle.rs` | standard, invalid, 2 properties (the check accepts exactly the closed lifecycles; random walks always can finish) | 15 |
| `tests/approval.rs` | every mode, routes, parallel joins, fill-in, notify, acts, escalation, cancel, versions, persistence, simulation, definitions | 36 |
| `tests/approval_model.rs` | reference model of one step (3,000 seeds), order independence on several threads, 1,500 random definitions under random events with invariants and a persisted shadow copy, replay determinism (500 seeds) | 4 |

Simulations are seeded (SplitMix64); a failure names its seed. The random-event simulation
found one real bug while this crate was written: two tasks of one step that became overdue in
the same tick were both escalated to the same person. Escalations are now applied one at a
time; `tasks_overdue_together_are_not_all_escalated_to_one_person` guards it.

```sh
export CARGO_TARGET_DIR=/var/tmp/mt-target/mt-lifecycle
cargo test -p mt-lifecycle
cargo llvm-cov -p mt-lifecycle --summary-only
cargo build -p mt-lifecycle --target wasm32-unknown-unknown
```

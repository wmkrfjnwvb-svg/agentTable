# mt-server — the `agenttable` binary

One program that serves the API and the web app (embedded). It is the **end-to-end build of the
product as it stands**: every finished engine is wired in behind a demo purchasing workspace,
and the parts that do not exist yet have clearly marked stand-ins. As each wave lands, its
stand-in is replaced and the same binary, web app and end-to-end tests grow with it.

| Area | What runs here | Stand-in until |
|---|---|---|
| Field types, validation, auto-numbering | `mt-core` (email, phone, IBAN, ratings, `PR-{YYYY}-{seq:4}`) | — |
| Reference data | `mt-refdata` (cost-centre tree, ISO countries and currencies) | change screens with four-eyes approval: next |
| Grid: search, filter, sort, group, totals | `mt-query` | — |
| Formulas, lookups, roll-ups across tables | `mt-formula` (full recompute after each change) | incremental recompute: `mt-ops` (wave 3) |
| Who sees and changes what, and why | `mt-perm` (scopes, limits, field protection, masks, separation of duties, agents acting for people, explain) | — |
| Lifecycles and approvals | `mt-lifecycle`: requests go draft → submitted → approved → ordered (or rejected, cancelled); submitting starts the purchase approval, routed by the access rules (whoever may approve the amount, never the requester; the CFO above 25,000); my tasks, approve / reject / send back / delegate, an in-app inbox; requests and their lines are locked while approved | SLA clocks on requests, approval designer: wave 5 |
| Audit | `mt-audit` hash chain, verification, tamper simulation | — |
| Storage | one JSON file, written atomically after each change | `mt-store`: SQLite/MySQL/PostgreSQL (wave 3) |
| Identity | a user switcher (`x-user`, `x-on-behalf-of` headers) | `mt-identity` (wave 4) |
| API | `/api/*`, JSON | versioned Public/Admin APIs with OpenAPI: `mt-api` (wave 4) |
| Building it yourself | bases, tables, the field picker, code lists, settings, people and groups (`builder`); the process designer (`design`); the access builder (`access`) | views saved per person, forms, templates of your own: waves 6–7 |
| Automations, email/chat notifications, dashboards, AI | placeholders in the web app | waves 5, 6 |

## Run

```bash
(cd apps/web && npm ci && npm run build)       # the web app (embedded in release builds)
cargo run --release -p mt-server -- --open      # http://127.0.0.1:8080
```

Options: `--port`, `--host`, `--data <file>` (default `agenttable-data.json`), `--fresh` (start
over), `--demo` (a new workspace starts with the purchasing sample), `--open` (open the
browser). A new workspace is otherwise empty: one administrator, the built-in ISO code lists,
and the template gallery. Debug builds read `apps/web/dist` from disk.

## Modules

| Module | Purpose |
|---|---|
| `model` | The workspace: tables, records, reference sets, people, policy, audit; create/update/delete and add-field operations, each checked, validated, recomputed and audited |
| `seed` | The demo: suppliers, purchase requests, request lines; seven people with different access; the request lifecycle and purchase approval; records created, submitted and approved through the real operations |
| `builder` | Building a configuration: bases, tables, field editing and deletion (refused while formulas, lifecycles, approvals or access rules use the field), two-way links, code lists and items, settings for `PARAM`, people and groups, templates |
| `design` | The process designer's format (states, buttons, approval steps with approvers and “only when” conditions, outcomes) and `publish`: compiles it into `mt-lifecycle` definitions, checks them (closed loop, field references), creates or reuses the state field, keeps approval versions |
| `access` | The access builder: grants (who, what, where, which records, limit, why), revocation, capability sets, separation-of-duties rules, the capability catalogue |
| `process` | Lifecycles bound to tables, transitions, approvals (start, act, finish → state), approvers from the access policy, locks on frozen records and their lines, notices, and the views of tasks, approvals and the process design |
| `formula` | How formulas see tables, records, linked records and the environment |
| `view` | Values as people read them; field descriptions; decisions explained |
| `api` | The HTTP routes |
| `web` | The embedded web app with client-side routing fallback |

## Rules the API enforces (each is tested)

* Every read goes through the row filter and field permissions of `mt-perm`; hidden values are
  never sent, masked values are sent masked only, and nobody filters, sorts or groups by a field
  they cannot see.
* Every write is checked (`record.create` / `record.update` / `record.delete`, field
  editability), validated (`mt-core`), applied, recomputed and appended to the audit chain;
  refused writes leave no trace in data or log.
* An agent acting for a person gets the intersection of both: rows, fields and actions.
* Mirrored link fields (a request's lines, a line's request) stay in step.
* A field “required only when …” must be filled in whenever its condition holds: saving is
  refused (422, naming the field), and so is every transition that moves the record forward;
  withdrawing, rejecting and cancelling are never blocked by it.

## Tests

`tests/api.rs` — 19 in-process API tests (every endpoint, as different people).
`tests/builder.rs` — 15 tests that build the expense configuration of doc 14 (S3) from an empty
workspace (base, tree code list, two tables, a two-way link, a roll-up, a formula reading a
setting, people and groups), edit and delete safely, install the template, survive a restart,
design the claim lifecycle and approval, give access (sets, scopes, a limit, separation of
duties, revocation), run the process as the employee, the manager and finance, keep running
approvals on their version, run quorum steps, parallel branches, fill-in steps and timeouts,
enforce fields required only when a condition holds, and refuse every malformed or unauthorised step.
`tests/process.rs` — 9 tests of lifecycles and approvals through the API: the seeded requests'
history, submit → approve → order, the CFO step and sending back, rejection, revision,
withdrawal, cancellation and deletion rules, every refused transition with its reasons, acts
that are not allowed (someone else's task, an agent acting for the approver, unknown tasks,
missing reasons), a request nobody may approve (stuck, admins told), locks on lines of frozen
requests, and approvals surviving a restart.
The browser walk-throughs in `apps/web/e2e` (Playwright, with screenshots) run against the
release package in CI: `build.spec.ts` builds the expense configuration from an empty workspace
(19 steps: data model, process design, access, and the process run by three people);
`product.spec.ts` works with the purchasing sample (21 steps).
Line coverage of this crate: see the coverage job (gate: 90 % from the API tests alone).

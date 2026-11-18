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
| Audit | `mt-audit` hash chain, verification, tamper simulation | — |
| Storage | one JSON file, written atomically after each change | `mt-store`: SQLite/MySQL/PostgreSQL (wave 3) |
| Identity | a user switcher (`x-user`, `x-on-behalf-of` headers) | `mt-identity` (wave 4) |
| API | `/api/*`, JSON | versioned Public/Admin APIs with OpenAPI: `mt-api` (wave 4) |
| Approvals, automations, notifications, dashboards, AI | placeholders in the web app | waves 2, 5, 6 |

## Run

```bash
(cd apps/web && npm ci && npm run build)       # the web app (embedded in release builds)
cargo run --release -p mt-server -- --open      # http://127.0.0.1:8080
```

Options: `--port`, `--host`, `--data <file>` (default `agenttable-data.json`), `--fresh` (start
over with the demo), `--open` (open the browser). Debug builds read `apps/web/dist` from disk.

## Modules

| Module | Purpose |
|---|---|
| `model` | The workspace: tables, records, reference sets, people, policy, audit; create/update/delete and add-field operations, each checked, validated, recomputed and audited |
| `seed` | The demo: suppliers, purchase requests, request lines; six people with different access; created through the real operations |
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

## Tests

`tests/api.rs` — 19 in-process API tests (every endpoint, as different people). The browser
walk-through in `apps/web/e2e` (Playwright, 14 steps with screenshots) runs against the release
package in CI. Line coverage of this crate: 91 % from the API tests alone.

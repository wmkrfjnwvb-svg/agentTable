# apps/web — the web app

React + TypeScript (Vite). The app is a thin client: every business rule — validation,
formulas, permissions, explanations — runs in Rust on the server (`crates/mt-server`), and the
app shows what the server answers. Release builds of the server embed `dist/`, so the product
ships as one executable.

| File | What |
|---|---|
| `src/App.tsx` | Shell: navigation (tables, platform pages, coming-next placeholders) and the demo user switcher |
| `src/TablePage.tsx` | Grid: search, filters, sort, group with subtotals, totals, hidden-field notices |
| `src/RecordPanel.tsx` | Record editor with per-field permissions, validation messages and access decisions (change, delete, approve) |
| `src/FieldPicker.tsx` | The field picker (8 groups, synonym search) and the live formula editor |
| `src/pages.tsx` | Product status, reference data, access explain, audit log, placeholders |
| `src/Process.tsx` | Lifecycle state and buttons in the record panel, the approval timeline and decision box, Approvals (my tasks, all approvals, process design), Inbox |
| `e2e/product.spec.ts` | The browser walk-through (Playwright), one screenshot per step: tables, formulas, access, approvals (submit, my tasks, reject, CFO, order, inbox, process design), audit |

## Develop

```bash
npm ci
cargo run -p mt-server -- --port 8080 --fresh   # in another terminal, from the repository root
npm run dev                                       # http://localhost:5173, API proxied to :8080
```

## Build and test

```bash
npm run build                                                   # type-check and build dist/
AGENTTABLE_BIN=../../target/release/agenttable npx playwright test   # starts the binary on a fresh data file
```

Screenshots land in `e2e/screenshots/`, the HTML report in `e2e/report/`. CI runs the
walk-through against the Linux release package and publishes both.

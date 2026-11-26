# Multidimensional Table Platform (working name)

[![CI](https://github.com/wmkrfjnwvb-svg/agentTable/actions/workflows/ci.yml/badge.svg)](https://github.com/wmkrfjnwvb-svg/agentTable/actions/workflows/ci.yml)

Open-source, AI-native multidimensional table platform for running business processes out of the box.
All-Rust backend with OpenAPI Public and Admin APIs, web app in React + TypeScript (reusing Rust via
WebAssembly); a Tauri 2 + React client app is planned. Licence: Apache-2.0.

## Status
Engineering follows the waves of [doc 21](docs/21-development-plan.md). Every crate is pure Rust,
builds natively and for WebAssembly, and is tested in six categories (standard, edge, invalid,
anomalous, concurrency, cross-platform) plus property, model and simulation tests.

| Wave | Crate | Purpose | State |
|---|---|---|---|
| 1 | `mt-core` | Values, money, time, IDs, field definitions and field types, validation library, conditions, sort keys, provenance | done |
| 1 | `mt-cluster` | Single-binary N-to-N cluster: gossip membership, placement, fencing, leases (simulated) | done |
| 1 | `mt-audit` | Hash-chained audit log, signed checkpoints, tamper localisation | done |
| 2 | `mt-refdata` | Reference data, versioned changes with four-eyes approval, parameters | done |
| 2 | `mt-query` | Filter, search, sort, group, totals, views | done |
| 2 | `mt-formula` | Formula language, type checker, bounded evaluator, dependency graph | done |
| 2 | `mt-perm` | Capabilities, grants, separation of duties, field permissions, share links, access reviews, explain | done |
| 2 | `mt-lifecycle` | Lifecycles with the closed-loop check, business calendars, SLA clocks, the approval engine | done |
| — | `mt-server` + `apps/web` | End-to-end build: one binary with the web app; placeholders for what is not built yet | live, grows every wave |
| 3–8 | storage, API, identity, automation, AI, real-time grid | see doc 21 | planned |

Audits per wave: [`docs/audit/`](docs/audit/).

## Try it
Every push to `main` publishes installable packages for Windows, macOS and Linux under
[Releases → nightly](https://github.com/wmkrfjnwvb-svg/agentTable/releases/tag/nightly), together
with screenshots of every end-to-end test step. Unpack, start `start.bat` / `start.command` /
`start.sh`, and the browser opens the product: an empty workspace in which you build your own
bases from the components (tables, fields of every kind, code lists, settings, formulas, people,
lifecycles and approvals in the process designer, access rules) — the end-to-end test builds an
expense-claim process this way and runs it as three people, screenshots `build-*` — or
install the purchasing sample from the template gallery (screenshots `01`–`29`). What is still
to come is marked as a placeholder. Switch between people at the top right to see access
control at work. (The same files are attached to each CI run as artifacts.)

## Build and test
```bash
scripts/ci-toolchain.sh                      # the pinned toolchain (rust-toolchain.toml)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --target wasm32-unknown-unknown
cargo llvm-cov --workspace --summary-only    # target: 98 % of lines
(cd apps/web && npm ci && npm run build)     # the web app
cargo run --release -p mt-server -- --open   # the product at http://127.0.0.1:8080
(cd apps/web && AGENTTABLE_BIN=../../target/release/agenttable npx playwright test)  # browser E2E
```

## Documents (read in this order)
| Doc | Purpose |
|---|---|
| [17 Final product design](docs/17-final-product-design.md) | Domains, processes, data assets, core capabilities (authoritative) |
| [18 Final AI design](docs/18-final-ai-design.md) | AI architecture, skills, agents, assistant, governance (authoritative) |
| [19 Lark parity & delivery plan](docs/19-lark-parity-and-delivery-plan.md) | Capability parity, releases, engineering hand-off |
| [20 Technical architecture](docs/20-technical-architecture.md) | All-Rust backend (Public API + Admin API, OpenAPI), React + WASM web app, future Tauri 2 + React client |
| [13 Component layer](docs/13-business-component-layer.md) · [14 Scenario templates](docs/14-scenario-templates-field-level.md) · [16 Taxonomy](docs/16-feature-taxonomy-information-architecture.md) | Template components, field-level scenarios, information architecture |
| Docs 01–12, 15 | Research history and decisions (superseded where 17–20 differ) |

## Planned repository layout
```
crates/      Rust workspace (core, formula, query, perm, lifecycle, identity, ai, store, api, server, wasm, grid …)
apps/web/    React + TypeScript (Vite) with WASM worker and canvas grid
apps/desktop/ Tauri 2 shell reusing apps/web (later)
sdks/        SDKs generated from the Public and Admin OpenAPI specs (TS, Python, Go, Java)
templates/   Solution packages and component definitions
deploy/      Dockerfiles, compose profiles, Helm (later)
docs/        Product and technical design
```

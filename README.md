# Multidimensional Table Platform (working name)

Open-source, AI-native multidimensional table platform for running business processes out of the box.
Core in Rust, web app in React + TypeScript (reusing Rust via WebAssembly), optional enterprise
management & integration platform in Spring Boot. Licence: Apache-2.0.

## Status
Design phase complete; engineering starts with milestone M0 (scaffold, CI, spikes).

## Documents (read in this order)
| Doc | Purpose |
|---|---|
| [17 Final product design](docs/17-final-product-design.md) | Domains, processes, data assets, core capabilities (authoritative) |
| [18 Final AI design](docs/18-final-ai-design.md) | AI architecture, skills, agents, assistant, governance (authoritative) |
| [19 Lark parity & delivery plan](docs/19-lark-parity-and-delivery-plan.md) | Capability parity, releases, engineering hand-off |
| [20 Technical architecture](docs/20-technical-architecture.md) | Rust core, React + WASM, Spring Boot platform, contracts, deployment, CI |
| [13 Component layer](docs/13-business-component-layer.md) · [14 Scenario templates](docs/14-scenario-templates-field-level.md) · [16 Taxonomy](docs/16-feature-taxonomy-information-architecture.md) | Template components, field-level scenarios, information architecture |
| Docs 01–12, 15 | Research history and decisions (superseded where 17–20 differ) |

## Planned repository layout
```
crates/      Rust workspace (core, formula, query, perm, lifecycle, ai, store, server, wasm, grid, ffi …)
apps/web/    React + TypeScript (Vite) with WASM worker and canvas grid
platform/    Spring Boot management & integration platform (optional)
proto/       Internal service contracts (buf)
sdks/        Generated SDKs (TS, Java, Python)
templates/   Solution packages and component definitions
deploy/      Dockerfiles, compose profiles, Helm (later)
docs/        Product and technical design
```

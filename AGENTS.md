# Engineering rules for every builder (humans and agents)

Read first: `docs/21-development-plan.md`, then the spec sections named in your task
(`docs/17-final-product-design.md`, `docs/18-final-ai-design.md`, `docs/20-technical-architecture.md`,
and `docs/13`/`docs/14` for components and templates).

## Ownership
- You own **only** your crate directory `crates/<name>/`. Do not edit the root `Cargo.toml`, other crates,
  or docs outside `crates/<name>/README.md`. If you need a change elsewhere, write it in your final report.
- Declare dependencies in your own `Cargo.toml`. Use `{ workspace = true }` for deps listed in the root
  `[workspace.dependencies]`; otherwise pin a version in your crate. Prefer few, well-known crates.
- Do **not** run `git commit`; the lead commits after verification.
- Build with your own target dir to avoid lock contention: `export CARGO_TARGET_DIR=/var/tmp/mt-target/<name>`.

## Crate conventions
- `Cargo.toml`: `version/edition/license/rust-version = { workspace = true }`, and `[lints] workspace = true`.
- `#![forbid(unsafe_code)]`. No `unwrap`/`expect`/`panic!`/indexing/unchecked arithmetic in non-test code
  (workspace clippy lints enforce this). No `_ =>` arms on enums: match every variant.
- Errors: one `thiserror` enum per crate; errors are values, never panics.
- Money and numbers: exact decimals (`rust_decimal`), never floats. Time: UTC, integer milliseconds since
  epoch in core types (no wall-clock calls inside pure logic — time is an input).
- Pure crates (no I/O) must also compile for WASM: `cargo build -p <name> --target wasm32-unknown-unknown`.
- State machines are **sans-IO**: `step(state, input) -> outputs`. No clocks, sockets, threads or
  randomness inside; these are injected so the deterministic simulator can drive them.
- Public items documented. `crates/<name>/README.md` explains purpose, public API and invariants.

## Tests (all required)
- Categories: standard, anomalous, edge, invalid, concurrency (where state is shared or ordered), and
  cross-platform (wasm build for pure crates).
- Property tests with `proptest` for parsers, encoders, ordering, and invariants.
- Model-based or simulation tests for every state machine (seeded, reproducible; print the seed on failure).
- Databases (only for crates that touch SQL): run every DB test on SQLite, MySQL and PostgreSQL.
  Start servers with `scripts/dev-db.sh`; URLs come from `MT_TEST_POSTGRES_URL` and `MT_TEST_MYSQL_URL`.
- Must pass before you report: `cargo fmt -p <name> --check`, `cargo clippy -p <name> --all-targets -- -D warnings`,
  `cargo test -p <name>`, the wasm build if pure, and `cargo llvm-cov -p <name> --summary-only`
  (target ≥ 98% lines; report the number honestly).

## Final report (your last message)
State: what was built (public API summary), test counts by category, coverage numbers, wasm status,
anything in the spec you did **not** implement, assumptions you made, and changes you need outside your crate.
Never claim something passes unless you ran it and saw it pass.

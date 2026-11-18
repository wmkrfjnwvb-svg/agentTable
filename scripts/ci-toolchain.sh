#!/usr/bin/env bash
# Installs the toolchain pinned in rust-toolchain.toml with its components and targets, so CI
# and local builds use exactly the same compiler.
set -euo pipefail
cd "$(dirname "$0")/.."
channel="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)"
rustup toolchain install "$channel" --profile minimal \
  --component rustfmt,clippy,llvm-tools-preview \
  --target wasm32-unknown-unknown
rustup override set "$channel"
rustc --version
cargo --version

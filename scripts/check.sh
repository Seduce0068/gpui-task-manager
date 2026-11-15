#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Runs the standard verification loop for this repository:
#   fmt -> clippy -> tests -> wasm compile checks
#
# Usage: bash scripts/check.sh
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

echo '==> cargo fmt --all --check'
cargo fmt --all --check

echo '==> cargo clippy --workspace --all-targets -- -D warnings'
cargo clippy --workspace --all-targets -- -D warnings

echo '==> cargo test --workspace'
cargo test --workspace

echo '==> cargo check --target wasm32-unknown-unknown (tm-core, tm-viewmodel)'
cargo check --target wasm32-unknown-unknown -p tm-core -p tm-viewmodel

if [ -f web/Cargo.toml ]; then
  echo '==> cargo check --target wasm32-unknown-unknown (web shell)'
  cargo check --target wasm32-unknown-unknown --manifest-path web/Cargo.toml

  echo '==> cargo test --manifest-path web/Cargo.toml --lib'
  cargo test --manifest-path web/Cargo.toml --lib
fi

if command -v cargo-deny >/dev/null 2>&1; then
  echo '==> cargo deny check (root / web)'
  cargo deny check
  if [ -f web/Cargo.toml ]; then
    cargo deny --manifest-path web/Cargo.toml check
  fi
else
  echo 'cargo-deny is not installed; skipping dependency policy check.'
fi

if command -v cargo-vet >/dev/null 2>&1; then
  echo '==> cargo vet --locked (root / web)'
  cargo vet --locked
  if [ -f web/Cargo.toml ]; then
    (cd web && cargo vet --locked)
  fi
else
  echo 'cargo-vet is not installed; skipping supply-chain audit check.'
fi

echo 'All checks passed.'

#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Runs the standard verification loop for this repository:
#   fmt -> clippy -> tests -> wasm -> deny -> vet -> machete
#
# Usage: bash scripts/check.sh
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

echo '==> cargo fmt --all --check'
cargo fmt --all --check

echo '==> cargo clippy --workspace --all-targets -- -D warnings -D clippy::undocumented_unsafe_blocks'
cargo clippy --workspace --all-targets -- -D warnings -D clippy::undocumented_unsafe_blocks

echo '==> cargo test --workspace'
cargo test --workspace

echo '==> cargo check --target wasm32-unknown-unknown (tm-core, tm-viewmodel)'
cargo check --target wasm32-unknown-unknown -p tm-core -p tm-viewmodel

if [ -f web/Cargo.toml ]; then
  echo '==> cargo clippy --manifest-path web/Cargo.toml --all-targets (same lints)'
  cargo clippy --manifest-path web/Cargo.toml --all-targets -- \
    -D warnings -D clippy::undocumented_unsafe_blocks

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

if command -v cargo-machete >/dev/null 2>&1; then
  echo '==> cargo machete (root / web): unused dependencies'
  cargo machete
  if [ -f web/Cargo.toml ]; then
    (cd web && cargo machete)
  fi
else
  echo 'cargo-machete is not installed; skipping unused dependency check.'
fi

echo '==> RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps'
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
if [ -f web/Cargo.toml ]; then
  echo '==> RUSTDOCFLAGS=-D warnings cargo doc (web workspace)'
  RUSTDOCFLAGS="-D warnings" cargo doc --manifest-path web/Cargo.toml --no-deps
fi

echo 'All checks passed.'

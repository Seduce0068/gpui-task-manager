#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Runs the standard verification loop for this repository:
#   fmt -> clippy -> tests -> wasm -> web gates -> deny -> vet -> machete -> rustdoc -> markdownlint
#
# Gate order and commands are kept intact. Skipped gates are now reported
# explicitly: a gate skipped because its tool is missing is NOT reported as a
# clean success (exit 3). --fast marks its skips as intentional and keeps the
# success exit code. Gate command failures (including network errors) are NOT
# converted to skips: `set -e` aborts with the tool's own non-zero status.
#
# Exit codes:
#   0  all gates passed (or only --fast-intentional skips)
#   2  usage error
#   3  one or more gates were skipped unexpectedly (tool not installed)
#
# Usage:
#   bash scripts/check.sh            # full loop
#   bash scripts/check.sh --fast     # intentionally skip the optional tool gates
#   bash scripts/check.sh --app      # scope alias (unified loop; see argument parsing)
#   bash scripts/check.sh --poc      # scope alias (unified loop; see argument parsing)
#   bash scripts/check.sh --help
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

usage() {
  cat <<'EOF'
Runs the standard verification loop for this repository.

  fmt -> clippy -> tests -> wasm -> web gates -> deny -> vet -> machete -> rustdoc -> markdownlint

Usage:
  bash scripts/check.sh            # full loop
  bash scripts/check.sh --fast     # intentionally skip optional tool gates (still exit 0)
  bash scripts/check.sh --app      # scope alias; this repo runs one unified loop
  bash scripts/check.sh --poc      # scope alias; this repo runs one unified loop
  bash scripts/check.sh --help

A gate skipped because its tool is missing is printed as "SKIP" and makes the
script exit 3, so a partial run is never mistaken for a full success.
EOF
}

fast_mode=false
scope_flag=''

for argument in "$@"; do
  case "${argument}" in
  --fast)
    fast_mode=true
    ;;
  --app | --poc)
    # Sibling projects use these scope flags. This repository has one
    # workspace-wide loop, so they are accepted for CLI compatibility without
    # removing or reordering any gate.
    if [ -n "${scope_flag}" ] && [ "${scope_flag}" != "${argument}" ]; then
      echo "error: --app and --poc are mutually exclusive" >&2
      exit 2
    fi
    scope_flag="${argument}"
    ;;
  -h | --help)
    usage
    exit 0
    ;;
  *)
    echo "error: unknown argument: ${argument}" >&2
    exit 2
    ;;
  esac
done

if [ -n "${scope_flag}" ]; then
  echo "note: ${scope_flag} accepted; this repository runs a single unified gate loop."
fi

intentional_skips=0
unexpected_skips=0
declare -a skipped_intentional=()
declare -a skipped_unexpected=()

record_skip() {
  local gate="$1"
  local reason="$2"
  if [ "${fast_mode}" = true ]; then
    printf '==> SKIP: %s - intentional (--fast)\n' "${gate}"
    intentional_skips=$((intentional_skips + 1))
    skipped_intentional+=("${gate}: intentional (--fast)")
  else
    printf '==> SKIP: %s - %s\n' "${gate}" "${reason}"
    unexpected_skips=$((unexpected_skips + 1))
    skipped_unexpected+=("${gate}: ${reason}")
  fi
}

optional_gate_enabled() {
  local tool="$1"
  if [ "${fast_mode}" = true ]; then
    return 1
  fi
  if ! command -v "${tool}" >/dev/null 2>&1; then
    return 1
  fi
  return 0
}

echo '==> cargo fmt --all --check'
cargo fmt --all --check

echo '==> cargo fmt --manifest-path web/Cargo.toml --all --check'
cargo fmt --manifest-path web/Cargo.toml --all --check

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

# Advisory-ignore policy (always on, no external tool required). Mirrors the
# "Validate advisory ignore policy" step in the deny job of
# .github/workflows/ci.yml: enforce reason/expiry/3-month-cap locally too.
# Kept outside `optional_gate_enabled cargo-deny` so --fast or a missing
# cargo-deny cannot silently skip a policy check that CI always runs.
echo '==> check-advisory-ignores policy'
bash "${repo_root}/scripts/check-advisory-ignores.sh"

if optional_gate_enabled cargo-deny; then
  # Flags are kept identical to the `deny` job in .github/workflows/ci.yml.
  # -D advisory-ignore-expired: turn an expired ignore into a hard failure
  #   (check-advisory-ignores.sh above only validates reason/expiry/cap).
  # advisory-not-detected is deliberately NOT allowed: cargo-deny's default is
  # a warning (unused-ignored-advisory = "warn"), so suppressing it would hide
  # stale ignores. CI used to pass -A; R4-CI01 removed it to match this gate.
  echo '==> cargo deny check -D advisory-ignore-expired (root / web)'
  cargo deny check -D advisory-ignore-expired
  if [ -f web/Cargo.toml ]; then
    cargo deny --manifest-path web/Cargo.toml check -D advisory-ignore-expired
  fi
  # A clean scan is only meaningful once the advisory DB is confirmed loaded.
  bash "${repo_root}/scripts/check-advisory-db.sh"
else
  record_skip 'cargo deny check' 'cargo-deny is not installed'
fi

if optional_gate_enabled cargo-vet; then
  echo '==> cargo vet --locked (root / web)'
  cargo vet --locked
  if [ -f web/Cargo.toml ]; then
    (cd web && cargo vet --locked)
  fi
else
  record_skip 'cargo vet --locked' 'cargo-vet is not installed'
fi

if optional_gate_enabled cargo-machete; then
  echo '==> cargo machete (root / web): unused dependencies'
  cargo machete
  if [ -f web/Cargo.toml ]; then
    (cd web && cargo machete)
  fi
else
  record_skip 'cargo machete' 'cargo-machete is not installed'
fi

echo '==> RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps'
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
if [ -f web/Cargo.toml ]; then
  echo '==> RUSTDOCFLAGS=-D warnings cargo doc (web workspace)'
  RUSTDOCFLAGS="-D warnings" cargo doc --manifest-path web/Cargo.toml --no-deps
fi

if optional_gate_enabled gitleaks; then
  # Secret scanning (working tree + git history). Matches the CI extra gate.
  echo '==> gitleaks detect --redact'
  gitleaks detect --source "${repo_root}" --redact --exit-code 1
else
  record_skip 'gitleaks' 'gitleaks is not installed'
fi

if optional_gate_enabled markdownlint; then
  # Markdown lint gate. Keep this scope in sync with extra_gates in
  # .github/workflows/ci.yml.
  # - MD060/MD055: new table-style rules that conflict with the compact GFM
  #   tables used throughout the existing documents.
  # - log.md: append-only historical record (duplicate headings / bare URLs).
  # - SECURITY.md, notes/{verification,cargo-vet-adoption,ime}.md: pre-existing
  #   MD012/MD024/MD034/MD047 violations that are out of scope for this task.
  # - packaging/linux/README.deb.md ships in the DEB and is required by
  #   check-public-docs.sh; lint scope follows that public-document surface.
  echo '==> markdownlint (documentation)'
  markdownlint '*.md' 'notes/*.md' 'packaging/linux/README.md' 'packaging/linux/README.deb.md' 'poc-wasm/README.md' 'web/README.md' \
    --ignore log.md \
    --ignore SECURITY.md \
    --ignore notes/verification.md \
    --ignore notes/cargo-vet-adoption.md \
    --ignore notes/ime.md \
    --disable MD060 MD055
else
  record_skip 'markdownlint' 'markdownlint is not installed'
fi

# Public-document disclosure gate (always on, no external tool required).
# Guards against reintroducing development paths / account identifiers into
# published docs after the Twitter-publication review.
echo '==> public-doc disclosure scan'
bash "${repo_root}/scripts/check-public-docs.sh"

if [ "${unexpected_skips}" -gt 0 ]; then
  printf '\nSKIPPED GATES (%d unexpected):\n' "${unexpected_skips}"
  for detail in "${skipped_unexpected[@]}"; do
    printf '  - %s\n' "${detail}"
  done
  if [ "${intentional_skips}" -gt 0 ]; then
    printf 'SKIPPED GATES (%d intentional via --fast):\n' "${intentional_skips}"
    for detail in "${skipped_intentional[@]}"; do
      printf '  - %s\n' "${detail}"
    done
  fi
  echo 'Some gates were skipped; this is NOT a full success.'
  exit 3
fi

if [ "${intentional_skips}" -gt 0 ]; then
  printf 'All checks passed (%d optional gate(s) intentionally skipped via --fast).\n' \
    "${intentional_skips}"
else
  echo 'All checks passed.'
fi

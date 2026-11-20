#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Fail closed when cargo-deny did not actually load the RustSec advisory DB.
# `cargo deny check` reports "0 vulnerabilities" both for a clean graph and for
# an empty/unfetched database; the advisory gate is only trustworthy once the
# database itself is verified to be present and populated (R4-CI01).
#
# Called by both .github/workflows/ci.yml (deny job, after the two check steps)
# and scripts/check.sh (after its cargo-deny checks) so the two gates cannot
# drift apart.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

# cargo-deny 0.21 clones the advisory DB into $CARGO_HOME/advisory-dbs/<name>-<hash>/.
# deny.toml in this repo does not override db-path, so the default applies.
# ADVISORY_DB_PATH allows a custom db-path to be asserted without editing this
# script; it never changes the cargo-deny invocation itself.
cargo_home="${CARGO_HOME:-${HOME}/.cargo}"
db_root="${ADVISORY_DB_PATH:-${cargo_home}/advisory-dbs}"

if [ ! -d "${db_root}" ]; then
  echo "advisory-db: database directory is missing: ${db_root}" >&2
  echo 'advisory-db: run cargo deny check (it fetches the DB) before this assertion.' >&2
  exit 1
fi

# Advisory files are RUSTSEC-<year>-<seq>.md under <db>/crates/<crate>/.
# Count them recursively rather than matching a specific hash-suffixed dir.
advisory_count="$(find "${db_root}" -type f -name 'RUSTSEC-*.md' 2>/dev/null | wc -l)"
advisory_count="${advisory_count//[[:space:]]/}"
# The live DB carries >1000 entries; a floor of 100 still catches an empty,
# partial or stale-checkout database without pinning today's exact count.
minimum=100
if [ "${advisory_count}" -lt "${minimum}" ]; then
  echo "advisory-db: only ${advisory_count} advisory file(s) under ${db_root} (expected >= ${minimum})" >&2
  echo 'advisory-db: the advisory gate is NOT trustworthy; aborting.' >&2
  exit 1
fi

echo "advisory-db: OK (${advisory_count} advisory file(s) loaded from ${db_root})"

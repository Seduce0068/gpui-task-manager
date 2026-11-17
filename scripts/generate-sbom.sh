#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Generate CycloneDX SBOMs for the native and web Cargo workspaces.
# Requires: cargo-cyclonedx (`cargo install cargo-cyclonedx`).
#
# Usage: bash scripts/generate-sbom.sh [output-dir]
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

out_dir="${1:-${repo_root}/target/sbom}"
mkdir -p "${out_dir}"

if ! command -v cargo-cyclonedx >/dev/null 2>&1; then
  echo 'cargo-cyclonedx is not installed; skipping SBOM generation.'
  echo 'Install with: cargo install cargo-cyclonedx'
  exit 0
fi

echo "==> generating SBOMs in ${out_dir}"

# Native workspace (root Cargo.toml)
cargo cyclonedx --format json --all --override-filename sbom-root

# Web workspace (separate Cargo.toml)
if [ -f web/Cargo.toml ]; then
  (
    cd web
    cargo cyclonedx --format json --all --override-filename sbom-web
  )
fi

# Collect outputs in one place for CI artifacts. cargo-cyclonedx writes one
# `<override-filename>.json` per workspace member next to its Cargo.toml, so
# keep the member directory in the copy target to avoid basename collisions.
find . -maxdepth 3 -type f \( -name 'sbom-root*.json' -o -name 'sbom-web*.json' -o -name 'bom.json' \) \
  -not -path "${out_dir}/*" -print0 | while IFS= read -r -d '' file; do
  rel="${file#./}"
  cp "${file}" "${out_dir}/${rel//\//__}"
done

# Remove the per-member originals (script-specific names only) so the working
# tree stays clean; generic names like bom.json are left untouched.
find . -maxdepth 3 -type f \( -name 'sbom-root*.json' -o -name 'sbom-web*.json' \) \
  -not -path "${out_dir}/*" -delete

echo "SBOM generation complete."
ls -lh "${out_dir}" 2>/dev/null || true

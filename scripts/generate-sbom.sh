#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Generate CycloneDX SBOMs for the native and web Cargo workspaces.
# Requires: cargo-cyclonedx (`cargo install cargo-cyclonedx`).
#
# Usage: bash scripts/generate-sbom.sh [output-dir]
#
# Idempotency: stale SBOMs from earlier runs are removed before collection,
# so repeated runs yield the same file set instead of accumulating copies.
# Only filenames owned by this script are removed; unrelated files in the
# output directory are preserved.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "${repo_root}"

out_arg="${1:-${repo_root}/target/sbom}"
mkdir -p -- "${out_arg}"
out_dir="$(cd -- "${out_arg}" && pwd -P)"

# `find .` emits ./relative paths. Normalise the output directory to the same
# form so it can be pruned whether the caller passed a relative path, an
# absolute path, or a path containing `..`. An output directory outside the
# repository needs no pruning because `find` never descends into it.
case "${out_dir}" in
"${repo_root}"/*) out_rel="${out_dir#"${repo_root}/"}" ;;
*) out_rel="" ;;
esac
if [ -n "${out_rel}" ]; then
  prune=(-path "./${out_rel}" -prune -o)
else
  prune=()
fi

# Drop stale artefacts before generating: per-member originals possibly left
# by an interrupted run, and this script's top-level outputs in out_dir.
# Use -exec rm instead of -delete: -delete implies -depth, which would disable
# the -prune guard protecting out_dir.
find . "${prune[@]}" -type f \( -name 'sbom-root*.json' -o -name 'sbom-web*.json' \) \
  -exec rm -f -- {} +
find "${out_dir}" -maxdepth 1 -type f \
  \( -name 'sbom-root*.json' -o -name '*__sbom-root*.json' \
  -o -name 'sbom-web*.json' -o -name '*__sbom-web*.json' \
  -o -name 'bom.json' -o -name '*__bom.json' \) -exec rm -f -- {} +

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

# cargo-cyclonedx writes one `<override-filename>.json` per workspace member
# next to its Cargo.toml. Collect them into out_dir, keeping the member path
# in the name to avoid basename collisions. Sorting makes the order stable.
declare -a sbom_files=()
while IFS= read -r -d '' file; do
  sbom_files+=("${file}")
done < <(
  find . "${prune[@]}" -type f \
    \( -name 'sbom-root*.json' -o -name 'sbom-web*.json' -o -name 'bom.json' \) \
    -print0 | sort -z
)

if [ "${#sbom_files[@]}" -eq 0 ]; then
  echo 'No SBOM files were generated.' >&2
  exit 1
fi

for file in "${sbom_files[@]}"; do
  rel="${file#./}"
  name="${rel//\//__}"
  dest="${out_dir}/${name}"
  if [ -e "${dest}" ]; then
    echo "SBOM name collision: ${dest}" >&2
    exit 1
  fi
  cp -- "${file}" "${dest}"
done

# Remove the per-member originals (script-specific names only) so the working
# tree stays clean; generic names like bom.json are left untouched.
find . "${prune[@]}" -type f \( -name 'sbom-root*.json' -o -name 'sbom-web*.json' \) \
  -exec rm -f -- {} +

echo "SBOM generation complete: ${#sbom_files[@]} file(s)."
ls -lh "${out_dir}"

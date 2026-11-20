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
# output directory are preserved. SOURCE_DATE_EPOCH (see below) additionally
# makes the file *contents* byte-stable for the CI SHA256 gate.
set -euo pipefail

# Pass the target explicitly to cargo-cyclonedx (0.5.10 supports --target;
# confirmed with `cargo cyclonedx --help`). Without it the tool defaults to the
# build host, so the web BOM would list Linux-only crates and omit the wasm-only
# ones even though the shipped artifact is a wasm32 binary. Native = Linux host,
# web = wasm32 browser build.
readonly ROOT_TARGET='x86_64-unknown-linux-gnu'
readonly WEB_TARGET='wasm32-unknown-unknown'

# cargo-cyclonedx otherwise stamps a random `serialNumber` and a wall-clock
# `timestamp` into each BOM. SOURCE_DATE_EPOCH is honored by the tool: the
# serial is omitted and the timestamp is fixed, giving byte-for-byte identical
# output across runs. Override the variable to change the recorded date.
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-1700000000}"

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
cargo cyclonedx --format json --all --override-filename sbom-root \
  --target "${ROOT_TARGET}"

# Web workspace (separate Cargo.toml). Track whether it was generated so the
# matching dependency-set check below only runs when a web BOM exists.
web_generated=0
if [ -f web/Cargo.toml ]; then
  (
    cd web
    cargo cyclonedx --format json --all --override-filename sbom-web \
      --target "${WEB_TARGET}"
  )
  web_generated=1
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

# Cross-check the generated BOMs against `cargo metadata` for the same target.
# cargo-cyclonedx emits one BOM per workspace member, and a member's own
# package sits in `metadata.component` rather than `components`; both are
# unioned by (name, version) before comparing with the metadata package set.
# With a matching target the sets are expected to be equal. Minor deltas can
# still come from build-dependency / target-specific resolution differences
# between the two tools; they are printed with their package names and
# tolerated up to 5 entries, while a larger divergence (e.g. a host-target BOM
# checked against the wasm closure) fails the run.
verify_sbom_target() {
  python3 - "$1" "$2" "$3" "${out_dir}" "$4" <<'PY'
import glob
import json
import os
import subprocess
import sys

label, manifest, target, out_dir, pattern = sys.argv[1:6]

cmd = ["cargo", "metadata", "--locked", "--format-version", "1",
       "--filter-platform", target]
if manifest:
    cmd += ["--manifest-path", manifest]
metadata = json.loads(subprocess.check_output(cmd))
meta_set = {(p["name"], p["version"]) for p in metadata["packages"]}

sbom_set = set()
paths = sorted(glob.glob(os.path.join(out_dir, pattern)))
for path in paths:
    with open(path, encoding="utf-8") as handle:
        bom = json.load(handle)
    for component in bom.get("components", []):
        sbom_set.add((component.get("name"), component.get("version")))
    own = bom.get("metadata", {}).get("component")
    if own:
        sbom_set.add((own.get("name"), own.get("version")))

print(f"==> {label} [{target}]: metadata packages={len(meta_set)}, "
      f"SBOM components={len(sbom_set)} ({len(paths)} file(s))")
only_meta = sorted(meta_set - sbom_set)
only_sbom = sorted(sbom_set - meta_set)
if not only_meta and not only_sbom:
    print("    dependency sets match exactly.")
else:
    print(f"    only in metadata ({len(only_meta)}): {only_meta[:8]}")
    print(f"    only in SBOM ({len(only_sbom)}): {only_sbom[:8]}")
    # Small resolution differences are expected across tool versions; a large
    # gap means the wrong target was used and must fail generation.
    if len(only_meta) + len(only_sbom) > 5:
        sys.exit(f"    ERROR: {label} SBOM diverges from {target} metadata "
                 "beyond tolerance.")
PY
}

verify_sbom_target 'native' '' "${ROOT_TARGET}" '*sbom-root*.json'
if [ "${web_generated}" -eq 1 ]; then
  verify_sbom_target 'web' 'web/Cargo.toml' "${WEB_TARGET}" '*sbom-web*.json'
fi

# Remove the per-member originals (script-specific names only) so the working
# tree stays clean; generic names like bom.json are left untouched.
find . "${prune[@]}" -type f \( -name 'sbom-root*.json' -o -name 'sbom-web*.json' \) \
  -exec rm -f -- {} +

echo "SBOM generation complete: ${#sbom_files[@]} file(s)."
ls -lh "${out_dir}"

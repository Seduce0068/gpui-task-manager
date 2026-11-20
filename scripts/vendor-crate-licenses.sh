#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Vendor the full-text license files of a resolved Cargo dependency graph into
# <licenses_root>/crates/, and record crates with no local text in
# <licenses_root>/NOT-VENDORED.txt.
#
# Extracted from packaging/linux/build-deb.sh so the DEB build and the
# web-dist CI job produce the same licenses/crates/** tree (R4-CI01). The DEB
# job uses the native host graph; web-dist uses web/Cargo.toml filtered to
# wasm32 so only the crates actually shipped in the browser build are copied.
#
# Existing-solution survey (AGENTS.md §2, 2026-11-20; preserved on extraction):
#   - cargo-about is purpose-built for license reports, but is not installed
#     here: adopting it means `cargo install` (a new build-time dependency,
#     forbidden for this task). It also renders a consolidated report instead
#     of populating a per-crate directory of upstream license full texts.
#   - cargo-deny 0.21.0 is installed, but its license support is policy checking
#     (`cargo deny check licenses` / `cargo deny list`) against deny.toml. It
#     has no command that copies those full texts into a distribution tree, so
#     it stays in place as the license-policy gate, not as the vendoring tool.
#   - cargo metadata + the local registry source cache needs no new dependency:
#     cargo and jq are already hard requirements of the DEB build, --offline
#     keeps the step network-free, and each source directory is derived from
#     the package's manifest_path (no hardcoded registry hash or home path,
#     so the same code works in CI).
# Decision: use cargo metadata + jq; keep cargo-deny unchanged as policy gate.
#
# Usage:
#   bash scripts/vendor-crate-licenses.sh <licenses_root> [manifest_path] [target_platform]
#     licenses_root   directory that will receive crates/ and NOT-VENDORED.txt
#                     (absolute, or relative to the current working directory)
#     manifest_path   Cargo.toml to resolve (absolute, or relative to the
#                     current working directory; default: Cargo.toml)
#     target_platform rustc target triple passed to --filter-platform
#                     (default: the host triple reported by `rustc -vV`)
set -euo pipefail

licenses_root="${1:?licenses_root is required}"
manifest_path="${2:-Cargo.toml}"
target_platform="${3:-}"

if [ -z "${target_platform}" ]; then
  target_platform="$(rustc -vV | sed -n 's/^host: //p')"
fi
if [ -z "${target_platform}" ]; then
  echo "error: could not determine target platform from 'rustc -vV'" >&2
  exit 1
fi
if [ ! -f "${manifest_path}" ]; then
  echo "error: Cargo manifest not found: ${manifest_path}" >&2
  exit 1
fi

vendor_crate_license_texts() {
  local licenses_root="$1"
  local metadata name version manifest source_dir file base
  local count=0
  local -a license_files=()
  local -a no_text_crates=()

  # Idempotent: rebuilding replaces the tree so a removed dependency cannot
  # leave a stale text behind. The `:?` guard prevents an empty root from
  # ever turning into `rm -rf /crates`.
  rm -rf "${licenses_root:?}/crates"
  mkdir -p "${licenses_root}/crates"

  # Filter to the target actually being shipped. The unfiltered graph also
  # resolves platform-specific crates (e.g. accesskit_macos) that are not in
  # the local registry during a single-target CI build, which made --offline
  # metadata fail. Must run after a build populated the local cache.
  if ! metadata="$(cargo metadata --offline --format-version 1 \
    --manifest-path "${manifest_path}" \
    --filter-platform "${target_platform}")"; then
    echo "error: cargo metadata --offline failed for ${manifest_path} (${target_platform}); build once to cache dependencies." >&2
    return 1
  fi

  while IFS=$'\t' read -r name version manifest; do
    [ -n "${name}" ] || continue
    source_dir="$(dirname -- "${manifest}")"
    [ -d "${source_dir}" ] || continue

    # Match upstream names case-insensitively: some crates ship `license-mit`
    # / `license-apache-2.0` instead of uppercase `LICENSE*`.
    license_files=()
    for file in "${source_dir}"/*; do
      [ -f "${file}" ] || continue
      base="$(basename -- "${file}")"
      case "${base,,}" in
      license* | licence* | copying*) license_files+=("${file}") ;;
      esac
    done

    # `source == null` marks workspace-local crates, covered by the top-level
    # LICENSE / NOTICE.md shipped alongside, not by per-crate texts.
    if ((${#license_files[@]} == 0)); then
      no_text_crates+=("${name}-${version}")
      continue
    fi

    mkdir -p "${licenses_root}/crates/${name}-${version}"
    for file in "${license_files[@]}"; do
      install -m 0644 "${file}" \
        "${licenses_root}/crates/${name}-${version}/$(basename -- "${file}")"
      count=$((count + 1))
    done
  done < <(printf '%s\n' "${metadata}" |
    jq -r '.packages[] | select(.source != null) | [.name, .version, .manifest_path] | @tsv')

  # Complete audit trail for crates whose published source has no text. The
  # shipped README points here: the gap is "not vendored yet", not a claim
  # that the upstream license is incompatible or cannot be shipped.
  : >"${licenses_root}/NOT-VENDORED.txt"
  if ((${#no_text_crates[@]} > 0)); then
    printf '%s\n' "${no_text_crates[@]}" | LC_ALL=C sort >>"${licenses_root}/NOT-VENDORED.txt"
  fi
  echo "==> vendored ${count} crate license text(s) from ${manifest_path} (${target_platform}); ${#no_text_crates[@]} crate(s) have no local text"
}

vendor_crate_license_texts "${licenses_root}"

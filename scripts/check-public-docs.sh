#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Public-document disclosure gate.
#
# Fails when development-host paths, personal account identifiers, or internal
# network terms leak into files that are published on GitHub or shown in
# screenshots. This is the regression guard for the Twitter-publication review
# (swarm-project-review R6): once a string is removed it must not come back.
#
# Scope: human-facing docs only. Source code may legitimately contain
# redaction patterns such as `.ts.net` or fixture paths such as `/home/alice`.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

files=(
  README.md
  SECURITY.md
  DESIGN.md
  NOTICE.md
  RESEARCH.md
  todo.md
  packaging/linux/README.deb.md
  log.md
  notes/verification.md
)
for path in notes/*.md; do
  [[ -f "${path}" ]] || continue
  already=false
  for current in "${files[@]}"; do
    if [[ "${current}" == "${path}" ]]; then
      already=true
      break
    fi
  done
  if [[ "${already}" == false ]]; then
    files+=("${path}")
  fi
done

missing=()
for path in "${files[@]}"; do
  [[ -f "${path}" ]] || missing+=("${path}")
done
if (( ${#missing[@]} > 0 )); then
  echo "public-docs: expected files are missing: ${missing[*]}" >&2
  exit 1
fi

# Extended-regex patterns. Each one matched a real leak in the R6 review.
patterns=(
  'work/gpui-'
  'onoderakeita'
  '/home/(koppe|onoderakeita)/'
  '[0-9]{6,}\+[A-Za-z0-9_-]+@users\.noreply\.github\.com'
  '\.ts\.net'
  'archive/master'
  'openclaw-gateway'
  'payments failed'
  'spending limit'
  '18789'
)

found=0
for pattern in "${patterns[@]}"; do
  matches="$(grep -En -- "${pattern}" "${files[@]}" || true)"
  if [[ -n "${matches}" ]]; then
    echo "public-docs: internal pattern found: /${pattern}/" >&2
    echo "${matches}" >&2
    found=1
  fi
done

if (( found > 0 )); then
  echo 'public-docs: FAILED (replace with generic wording before publishing)' >&2
  exit 1
fi

echo 'public-docs: OK (no internal paths / account identifiers in published docs)'

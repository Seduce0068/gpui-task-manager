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
# Scope: every Markdown file in the repository, enumerated dynamically with
# `git ls-files` (tracked plus non-ignored untracked files), so a newly added
# public document cannot silently escape the gate. Source code may legitimately
# contain redaction patterns such as `.ts.net` or fixture paths such as
# `/home/alice`.
#
# Findings are reported as file name + line number + pattern name only; the
# matching line is never echoed (same policy as `gitleaks --redact`).
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

# Dynamic scan set: every *.md tracked by git, plus untracked non-ignored *.md
# so no unlisted document can bypass the disclosure gate.
md_list="$(git ls-files --cached --others --exclude-standard -- '*.md')"
if [[ -z "${md_list}" ]]; then
  echo 'public-docs: no Markdown files found to scan' >&2
  exit 1
fi
mapfile -t files <<< "${md_list}"

# Public-facing documents that must always be part of the scan set. A missing
# file means the publication surface changed and needs an explicit review.
required=(
  README.md
  SECURITY.md
  DESIGN.md
  NOTICE.md
  RESEARCH.md
  todo.md
  THIRD_PARTY_NOTICES.md
  packaging/linux/README.md
  packaging/linux/README.deb.md
  web/README.md
  log.md
  notes/verification.md
)
missing=()
for path in "${required[@]}"; do
  listed=false
  for current in "${files[@]}"; do
    if [[ "${current}" == "${path}" ]]; then
      listed=true
      break
    fi
  done
  if [[ "${listed}" == false || ! -f "${path}" ]]; then
    missing+=("${path}")
  fi
done
if (( ${#missing[@]} > 0 )); then
  echo "public-docs: expected files are missing from the scan set: ${missing[*]}" >&2
  exit 1
fi

# Pattern name -> extended regex. Only the name is logged; the matched text is
# never printed, mirroring `gitleaks --redact`.
declare -A patterns=(
  [dev-work-path]='work/gpui-'
  [account-identifier]='onoderakeita'
  [home-directory]='/home/(koppe|onoderakeita)/'
  [noreply-email]='[0-9]{6,}\+[A-Za-z0-9_-]+@users\.noreply\.github\.com'
  [tailscale-domain]='\.ts\.net'
  [archive-master]='archive/master'
  [openclaw-gateway]='openclaw-gateway'
  [payments-failed]='payments failed'
  [spending-limit]='spending limit'
  [port-18789]='18789'
)

found=0
for name in "${!patterns[@]}"; do
  pattern="${patterns[${name}]}"
  if grep -En -- "${pattern}" "${files[@]}" >/dev/null 2>&1; then
    echo "public-docs: pattern '${name}' matched in published docs:" >&2
    # Redacted report: file name and line number only, never the matching line.
    grep -En -- "${pattern}" "${files[@]}" | cut -d: -f1,2 >&2
    found=1
  fi
done

if (( found > 0 )); then
  echo 'public-docs: FAILED (replace with generic wording before publishing)' >&2
  exit 1
fi

echo 'public-docs: OK (no internal paths / account identifiers in published docs)'

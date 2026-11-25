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
# Scope: every Markdown file in the repository plus the distributed license and
# notice text (LICENSE*, COPYING*, NOTICE*, THIRD_PARTY_NOTICES*,
# licenses/crates/**), enumerated dynamically with `git ls-files` (tracked plus
# non-ignored untracked files), so a newly added public document cannot silently
# escape the gate. Generated artifacts (packaging/linux/build/, web/dist/) are
# gitignored and covered by the separate CI artifact gate. Source code may
# legitimately contain redaction patterns such as `.ts.net` or fixture paths
# such as `/home/alice`.
#
# Findings are reported as file name + line number + pattern name only; the
# matching line is never echoed (same policy as `gitleaks --redact`).
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

# Dynamic scan set: Markdown, configuration (*.toml), scripts (*.py), plus
# distributed license/notice text, tracked by git or untracked-but-not-ignored,
# so no unlisted public file can bypass the disclosure gate. `sort -u` collapses
# files matching more than one pathspec.
scan_list="$(
  git ls-files --cached --others --exclude-standard -- \
    '*.md' \
    'LICENSE*' \
    'COPYING*' \
    'NOTICE*' \
    'THIRD_PARTY_NOTICES*' \
    'licenses/crates/**' \
    '*.log' \
    '*.txt' \
    '*.json' \
    '*.toml' \
    '*.py' | sort -u
)"
if [[ -z "${scan_list}" ]]; then
  echo 'public-docs: no public documents found to scan' >&2
  exit 1
fi
mapfile -t files <<<"${scan_list}"

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
if ((${#missing[@]} > 0)); then
  echo "public-docs: expected files are missing from the scan set: ${missing[*]}" >&2
  exit 1
fi

# Pattern name -> extended regex. Only the name is logged; the matched text is
# never printed, mirroring `gitleaks --redact`.
declare -A patterns=(
  ['dev-work-path']='work/gpui-'
  ['account-identifier']='onoderakeita'
  ['home-directory']='/home/(koppe|onoderakeita)/'
  ['noreply-email']='[0-9]{6,}\+[A-Za-z0-9_-]+@users\.noreply\.github\.com'
  ['tailscale-domain']='\.ts\.net'
  ['archive-master']='archive/master'
  ['openclaw-gateway']='openclaw-gateway'
  ['payments-failed']='payments failed'
  ['spending-limit']='spending limit'
  ['port-18789']='18789'
  ['hex64-token']='[0-9a-fA-F]{64}'
)

# 64-hex values that are public by design (reproducibility SHA256 checksums and
# container IDs already printed in the published notes). The list is exact: any
# other 64-hex token is a finding. Keys are allowlist data, never report output.
declare -A benign_hex64=(
  [1f1ad0723fe1a01e297849abf262e3a5d4545640ffbe82b2cd76147478705265]=1
  [2363ec980e8503da19476f7f564f4dfb790b14efa2c0a7779d6abe6d95dd8da1]=1
  [4552193caac72682c0937f5b6358775a71f865532d9c8604bf2b0e604e96bc65]=1
  [530f50b2009a67ea73b07074c2a91cfa7e4f1fee8c30132890924c90ba8eeb29]=1
  [761a3a4d11561be86a2ef894e937b2130447f2da5b826e3061384ab03d6459d0]=1
  [c3408f5bdb41df4a3859ce73cdf763ffb26b152cdf872a36b2d5aae708f28c30]=1
  [c86e0c08b907c91f17e8574f7b840b5fae1a5148d3ebf85a71f2ebee5aae4a8e]=1
  [c9c452aed8386d79f5949016c193c338bcf43f5fe9f514bb94d55d4be33fcabe]=1
  [cb3f2761da4750b3a80f43bd4d3030f1ec4d5a2930a9c86e11235f1bba9f02a4]=1
  [e0f257737dcb05e2fdbabfb52356a43a551f373807ab0dfe5092902255a260f0]=1
)

found=0
for name in "${!patterns[@]}"; do
  pattern="${patterns[${name}]}"
  matches="$(grep -En -- "${pattern}" "${files[@]}" 2>/dev/null || true)"
  if [[ "${name}" == 'hex64-token' && -n "${matches}" ]]; then
    # Drop hits whose every 64-hex token is on the public-by-design allowlist.
    kept=''
    while IFS= read -r hit; do
      if [[ -z "${hit}" ]]; then
        continue
      fi
      suspect=false
      while IFS= read -r token; do
        if [[ -z "${token}" ]]; then
          continue
        fi
        if [[ -z "${benign_hex64[${token}]+x}" ]]; then
          suspect=true
          break
        fi
      done < <(grep -oE '[0-9a-fA-F]{64}' <<<"${hit}")
      if [[ "${suspect}" == true ]]; then
        kept+="${hit}"$'\n'
      fi
    done <<<"${matches}"
    matches="${kept%$'\n'}"
  fi
  if [[ -n "${matches}" ]]; then
    echo "public-docs: pattern '${name}' matched in published docs:" >&2
    # Redacted report: file name and line number only, never the matching line.
    printf '%s\n' "${matches}" | cut -d: -f1,2 >&2
    found=1
  fi
done

if ((found > 0)); then
  echo 'public-docs: FAILED (replace with generic wording before publishing)' >&2
  exit 1
fi

echo 'public-docs: OK (no internal paths / account identifiers in published docs)'

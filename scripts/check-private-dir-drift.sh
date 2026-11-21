#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Drift guard for the intentionally duplicated private-directory logic in
# agent/src/private_dir.rs and app/src/private_dir.rs.
#
# Why a normalising comparison instead of a byte-for-byte diff:
#   - the two files deliberately carry different module-level comments
#     ("agent-side" framing vs "Mirrors agent/src/private_dir.rs" framing);
#   - their #[cfg(test)] modules use different env-var names, temp prefixes
#     and test-function names for the same checks.
# A byte-for-byte diff would therefore be permanently red on the intended
# documentation/test drift, so comments and the whole test module are excluded
# and only the executable logic is compared. A comment-only edit does not fail
# the gate; a one-sided logic fix does.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

agent_file="agent/src/private_dir.rs"
app_file="app/src/private_dir.rs"

for file in "${agent_file}" "${app_file}"; do
  if [ ! -f "${file}" ]; then
    echo "private-dir-drift: missing ${file}" >&2
    exit 1
  fi
done

# Extract the logic body: drop comment-only lines (the //! module docs, ///
# item docs and // inline explanations) and everything from the test module
# onwards, then drop blank lines. Statement whitespace is kept so a real logic
# edit is detected. Comments are the intended per-crate difference and are out
# of scope by design.
extract_logic() {
  awk '
    /^[[:space:]]*#\[cfg\(all\(test/ { exit }
    /^[[:space:]]*mod tests([[:space:]]|\{|$)/ { exit }
    /^[[:space:]]*\/\// { next }
    /^[[:space:]]*$/ { next }
    { print }
  ' "$1"
}

agent_logic="$(extract_logic "${agent_file}")"
app_logic="$(extract_logic "${app_file}")"

# A pure equality check would also pass if both files were emptied, so assert
# the two key functions are present in each normalised body. This doubles as
# the lightweight signature/existence fallback if the extraction ever stops
# matching a refactored file.
for file in "${agent_file}" "${app_file}"; do
  logic="$(extract_logic "${file}")"
  for symbol in 'fn ensure_private_dir_chain' 'fn chmod_private_dir'; do
    if [[ "${logic}" != *"${symbol}"* ]]; then
      echo "private-dir-drift: ${symbol} missing from ${file}" >&2
      exit 1
    fi
  done
done

if [ "${agent_logic}" != "${app_logic}" ]; then
  echo 'private-dir-drift: logic bodies diverged (comments/tests excluded)' >&2
  diff -u --label 'agent/src/private_dir.rs (logic)' \
    --label 'app/src/private_dir.rs (logic)' \
    <(printf '%s\n' "${agent_logic}") <(printf '%s\n' "${app_logic}") || true
  exit 1
fi

echo 'private-dir-drift: OK (agent/app private-dir logic bodies are identical)'

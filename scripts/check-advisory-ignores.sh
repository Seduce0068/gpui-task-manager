#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Enforce the advisory-ignore policy declared in deny.toml:
#   - every ignore must be a table with a non-empty `reason` and an absolute
#     `expiry` date (YYYY-MM-DD);
#   - the expiry must still be in the future (cargo-deny's native
#     `advisory-ignore-expired` lint is the primary gate);
#   - no suppression may be granted for more than 3 calendar months, so a
#     stale rationale cannot be silently carried forward.
#
# This is deliberately a policy check on top of cargo-deny: cargo-deny honours
# whatever absolute expiry is written, but does not itself cap the length.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
deny_file="${1:-${repo_root}/deny.toml}"

python3 - "${deny_file}" <<'PY'
import calendar
import datetime
import pathlib
import sys
import tomllib


def add_months(day: datetime.date, months: int) -> datetime.date:
    month_index = day.month - 1 + months
    year = day.year + month_index // 12
    month = month_index % 12 + 1
    return datetime.date(year, month, min(day.day, calendar.monthrange(year, month)[1]))


deny_path = pathlib.Path(sys.argv[1])
with deny_path.open("rb") as stream:
    config = tomllib.load(stream)

ignores = config.get("advisories", {}).get("ignore", [])
if not isinstance(ignores, list):
    print("advisories.ignore must be an array", file=sys.stderr)
    raise SystemExit(1)

today = datetime.date.today()
max_expiry = add_months(today, 3)
errors = []
checked = 0

for entry in ignores:
    if isinstance(entry, str):
        errors.append(f"{entry}: bare string ignore lacks reason and expiry")
        continue
    if not isinstance(entry, dict):
        errors.append(f"{entry!r}: ignore entry is not a table")
        continue

    identifier = entry.get("id") or entry.get("crate") or "<unknown>"
    reason = entry.get("reason")
    if not isinstance(reason, str) or not reason.strip():
        errors.append(f"{identifier}: missing or empty reason")

    expiry = entry.get("expiry")
    if not isinstance(expiry, str):
        errors.append(f"{identifier}: missing expiry (absolute YYYY-MM-DD)")
        continue
    try:
        expiry_date = datetime.date.fromisoformat(expiry)
    except ValueError:
        errors.append(f"{identifier}: invalid expiry {expiry!r} (expected YYYY-MM-DD)")
        continue

    if expiry_date <= today:
        errors.append(f"{identifier}: expiry {expiry} is not in the future")
    if expiry_date > max_expiry:
        errors.append(
            f"{identifier}: expiry {expiry} exceeds the 3-month maximum ({max_expiry.isoformat()})"
        )
    checked += 1

if errors:
    print("advisory ignore policy violations:", file=sys.stderr)
    for error in errors:
        print(f"  - {error}", file=sys.stderr)
    raise SystemExit(1)

print(f"advisory ignores: {checked} entry/entries valid (max expiry {max_expiry.isoformat()})")
PY

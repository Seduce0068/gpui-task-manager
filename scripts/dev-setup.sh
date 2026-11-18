#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Creates the local libxkbcommon-x11 linker shim for hosts that have the
# runtime library but not the -dev package (Kubuntu 2026.04 is one).
#
# GPUI's X11 feature links with -lxkbcommon-x11; without libxkbcommon-x11-dev
# the unversioned .so symlink is missing and the final link fails. This script
# creates app/native-libs/libxkbcommon-x11.so -> runtime library. It does not
# touch system files and does not need root.
#
# Generation / removal conditions (kept in sync with README.md):
#   * create or refresh the shim only when pkg-config cannot find
#     xkbcommon-x11 (no -dev package) and a known runtime library exists;
#   * remove the shim when -dev files become available, when the runtime
#     library disappears, or when the shim points to a stale target.
# app/build.rs adds native-libs to the linker search path whenever the
# directory exists, so a leftover shim could mis-link after a system update.
# This script is idempotent and safe to re-run at any time.
#
# Preferred system-wide fix: sudo apt install libxkbcommon-x11-dev
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"
native_libs="${repo_root}/app/native-libs"
shim="${native_libs}/libxkbcommon-x11.so"

remove_shim() {
  rm -f "${shim}"
  rmdir "${native_libs}" 2>/dev/null || true
}

if pkg-config --exists xkbcommon-x11 2>/dev/null; then
  echo "xkbcommon-x11 development files found; removing any local shim."
  remove_shim
  exit 0
fi

runtime_lib=""
for candidate in \
  /usr/lib/x86_64-linux-gnu/libxkbcommon-x11.so.0 \
  /usr/lib/libxkbcommon-x11.so.0 \
  /usr/lib64/libxkbcommon-x11.so.0 \
  /usr/lib/aarch64-linux-gnu/libxkbcommon-x11.so.0; do
  if [ -e "${candidate}" ]; then
    runtime_lib="${candidate}"
    break
  fi
done

if [ -z "${runtime_lib}" ]; then
  echo "error: libxkbcommon-x11.so.0 not found." >&2
  echo "Install it system-wide instead: sudo apt install libxkbcommon-x11-dev" >&2
  echo "Removing any stale shim so it cannot mis-link at build time." >&2
  remove_shim
  exit 1
fi

mkdir -p "${native_libs}"
current_target="$(readlink -f "${shim}" 2>/dev/null || true)"
wanted_target="$(readlink -f "${runtime_lib}")"
if [ ! -L "${shim}" ] || [ "${current_target}" != "${wanted_target}" ]; then
  ln -sfn "${runtime_lib}" "${shim}"
  echo "Created/refreshed ${shim} -> ${runtime_lib}"
else
  echo "${shim} already points to ${runtime_lib}; no change."
fi

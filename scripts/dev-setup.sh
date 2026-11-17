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
# Preferred system-wide fix: sudo apt install libxkbcommon-x11-dev
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"
native_libs="${repo_root}/app/native-libs"

if pkg-config --exists xkbcommon-x11 2>/dev/null; then
  echo "xkbcommon-x11 development files found; no local shim needed."
  rm -f "${native_libs}/libxkbcommon-x11.so"
  rmdir "${native_libs}" 2>/dev/null || true
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
  exit 1
fi

mkdir -p "${native_libs}"
ln -sfn "${runtime_lib}" "${native_libs}/libxkbcommon-x11.so"
echo "Created ${native_libs}/libxkbcommon-x11.so -> ${runtime_lib}"

#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Builds and installs the native GPUI task manager for the current user.
# No root and no .deb are needed: the binary, .desktop entry and icons all live
# under $XDG_DATA_HOME / ~/.local, so this is safe to re-run during development.
#
# Usage:
#   bash scripts/install-local.sh              # build + install + refresh menu
#   bash scripts/install-local.sh --skip-build # reuse target/release binary
#
# What it installs:
#   $HOME/.local/bin/gpui-task-manager
#   $HOME/.local/share/applications/gpui-task-manager.desktop
#   $HOME/.local/share/icons/hicolor/256x256/apps/gpui-task-manager.png
#   $HOME/.local/share/icons/hicolor/scalable/apps/gpui-task-manager.svg
#
# The .deb build (packaging/linux/build-deb.sh) remains the distribution path;
# this script is the fast development loop. Both install the same launcher.
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"

skip_build=false
case "${1:-}" in
"") ;;
--skip-build) skip_build=true ;;
-h | --help)
	sed -n '3,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
	exit 0
	;;
*)
	echo "error: unknown argument: ${1}" >&2
	exit 2
	;;
esac

if [ "${skip_build}" = false ]; then
	echo '==> cargo build --release'
	cargo build --release --manifest-path "${repo_root}/Cargo.toml" -p gpui-task-manager
fi

binary="${repo_root}/target/release/gpui-task-manager"
if [ ! -f "${binary}" ]; then
	echo "error: binary not found: ${binary} (build first)" >&2
	exit 1
fi

data_home="${XDG_DATA_HOME:-${HOME}/.local/share}"
bin_dir="${HOME}/.local/bin"
desktop_dir="${data_home}/applications"
icons_dir="${data_home}/icons/hicolor"

mkdir -p "${bin_dir}" "${desktop_dir}" \
	"${icons_dir}/256x256/apps" "${icons_dir}/scalable/apps"

# Install atomically via a temporary file, then replace. A failed copy never
# leaves a half-written executable or launcher entry behind.
install_binary() {
	local tmp="${bin_dir}/gpui-task-manager.new.$$"
	cp "${binary}" "${tmp}"
	chmod 755 "${tmp}"
	mv "${tmp}" "${bin_dir}/gpui-task-manager"
	echo "installed: ${bin_dir}/gpui-task-manager"
}

install_desktop() {
	local desktop="${desktop_dir}/gpui-task-manager.desktop"
	local tmp="${desktop}.new.$$"
	cat >"${tmp}" <<EOF
[Desktop Entry]
Type=Application
Name=GPUI Task Manager
Name[ja]=GPUI タスクマネージャー
Comment=System task manager (processes, CPU, memory, network) built with GPUI
Comment[ja]=プロセス・CPU・メモリ・ネットワークを監視する GPUI 製タスクマネージャー
Exec=${bin_dir}/gpui-task-manager
Icon=gpui-task-manager
Terminal=false
Categories=System;Monitor;
Keywords=task;process;cpu;memory;network;system;
StartupWMClass=gpui-task-manager
EOF
	chmod 644 "${tmp}"
	mv "${tmp}" "${desktop}"
	echo "installed: ${desktop}"
}

install_icons() {
	cp "${repo_root}/packaging/linux/gpui-task-manager.png" \
		"${icons_dir}/256x256/apps/gpui-task-manager.png"
	cp "${repo_root}/packaging/linux/gpui-task-manager.svg" \
		"${icons_dir}/scalable/apps/gpui-task-manager.svg"
	echo "installed: icons under ${icons_dir}"
}

install_binary
install_desktop
install_icons

if command -v update-desktop-database >/dev/null 2>&1; then
	update-desktop-database "${desktop_dir}" >/dev/null 2>&1 || true
fi
if command -v kbuildsycoca6 >/dev/null 2>&1; then
	kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
	echo 'menu cache refreshed'
fi

echo 'done. Launch "GPUI タスクマネージャー" from the application menu.'

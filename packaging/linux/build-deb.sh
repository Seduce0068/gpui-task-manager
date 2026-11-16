#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Builds a Debian package for the native GPUI task manager without root.
#
# Usage:
#   bash packaging/linux/build-deb.sh              # build release binary + .deb
#   bash packaging/linux/build-deb.sh --skip-build # reuse target/release binary
#
# Output:
#   packaging/linux/build/gpui-task-manager_<version>-1_<arch>.deb
#
# The package installs:
#   /usr/bin/gpui-task-manager
#   /usr/share/applications/gpui-task-manager.desktop
#   /usr/share/icons/hicolor/{scalable,256x256}/apps/gpui-task-manager.*
#   /usr/share/doc/gpui-task-manager/{README.md,NOTICE.md,copyright}
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/../.." && pwd)"
package="gpui-task-manager"
release="1"

skip_build=false
for argument in "$@"; do
  case "${argument}" in
  --skip-build) skip_build=true ;;
  -h | --help)
    sed -n '3,17p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
  *)
    echo "error: unknown argument: ${argument}" >&2
    exit 2
    ;;
  esac
done

cd "${repo_root}"

if ! command -v dpkg-deb >/dev/null 2>&1; then
  echo "error: dpkg-deb is required (Debian/Ubuntu)." >&2
  exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
  echo "error: jq is required to read the workspace version." >&2
  exit 1
fi

version="$(cargo metadata --no-deps --format-version 1 |
  jq -r '.packages[] | select(.name == "gpui-task-manager") | .version')"
if [ -z "${version}" ] || [ "${version}" = "null" ]; then
  echo "error: could not determine the package version." >&2
  exit 1
fi
arch="$(dpkg --print-architecture)"
binary="${repo_root}/target/release/${package}"

if [ "${skip_build}" = false ]; then
  echo "==> cargo build --release -p ${package}"
  cargo build --release -p "${package}"
fi
if [ ! -x "${binary}" ]; then
  echo "error: ${binary} is missing; run without --skip-build first." >&2
  exit 1
fi

staging="${script_dir}/build/${package}_${version}-${release}_${arch}"
output="${script_dir}/build/${package}_${version}-${release}_${arch}.deb"
rm -rf "${staging}" "${output}"
mkdir -p \
  "${staging}/DEBIAN" \
  "${staging}/usr/bin" \
  "${staging}/usr/share/applications" \
  "${staging}/usr/share/icons/hicolor/scalable/apps" \
  "${staging}/usr/share/icons/hicolor/256x256/apps" \
  "${staging}/usr/share/doc/${package}"

install -m 0755 "${binary}" "${staging}/usr/bin/${package}"
install -m 0644 "${script_dir}/${package}.desktop" "${staging}/usr/share/applications/${package}.desktop"
install -m 0644 "${script_dir}/${package}.svg" "${staging}/usr/share/icons/hicolor/scalable/apps/${package}.svg"
install -m 0644 "${script_dir}/${package}.png" "${staging}/usr/share/icons/hicolor/256x256/apps/${package}.png"
install -m 0644 "${repo_root}/README.md" "${staging}/usr/share/doc/${package}/README.md"
install -m 0644 "${repo_root}/NOTICE.md" "${staging}/usr/share/doc/${package}/NOTICE.md"
install -m 0644 "${script_dir}/copyright" "${staging}/usr/share/doc/${package}/copyright"

# Compute runtime dependencies from ldd + dpkg -S. The Vulkan loader is
# dlopen()ed by wgpu/Blade, so it does not appear in ldd and is added explicitly.
declare -A dependency_set=()
while IFS= read -r library; do
  [ -n "${library}" ] || continue
  resolved="$(readlink -f "${library}" 2>/dev/null || true)"
  [ -n "${resolved}" ] || continue
  owner="$(dpkg -S "${resolved}" 2>/dev/null | head -n 1 | cut -d: -f1)"
  if [ -n "${owner}" ]; then
    dependency_set["${owner}"]=1
  fi
done < <(ldd "${binary}" 2>/dev/null | awk '/=>/ {print $3}' | sort -u)
dependency_set["libvulkan1"]=1
depends="$(printf '%s\n' "${!dependency_set[@]}" | sort | paste -sd, -)"
if [ -z "${depends}" ]; then
  echo "error: could not compute runtime dependencies." >&2
  exit 1
fi

cat >"${staging}/DEBIAN/control" <<EOF
Package: ${package}
Version: ${version}-${release}
Section: utils
Priority: optional
Architecture: ${arch}
Maintainer: Seduce0068 <32996406+Seduce0068@users.noreply.github.com>
Installed-Size: $(($(du -sk "${staging}" | cut -f1)))
Depends: ${depends}
Description: GPUI-based system task manager
 Cross-platform system task manager for Ubuntu-family desktops.
 Shows processes, CPU, memory, network and disk usage and allows
 SIGTERM/SIGKILL/SIGSTOP/SIGCONT after an explicit confirmation.
EOF

dpkg-deb --build --root-owner-group "${staging}" "${output}" >/dev/null
echo "==> ${output} ($(du -h "${output}" | cut -f1))"

extract_dir="$(mktemp -d)"
trap 'rm -rf "${extract_dir}"' EXIT
dpkg-deb -x "${output}" "${extract_dir}"
version_output="$("${extract_dir}/usr/bin/${package}" --version)"
echo "==> extracted binary: ${version_output}"
dpkg-deb -I "${output}" | sed -n '1,20p'

#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Builds a Debian package for the native GPUI task manager without root.
#
# Usage:
#   bash packaging/linux/build-deb.sh              # build release binary + .deb
#   bash packaging/linux/build-deb.sh --skip-build # reuse target/release binary
#   DEB_MAINTAINER='Name <real-email>' bash packaging/linux/build-deb.sh
#     # override the neutral default Maintainer for distribution builds
#
# Output:
#   packaging/linux/build/gpui-task-manager_<version>-1_<arch>.deb
#
# The package installs:
#   /usr/bin/gpui-task-manager
#   /usr/share/applications/gpui-task-manager.desktop
#   /usr/share/icons/hicolor/{scalable,256x256}/apps/gpui-task-manager.*
#   /usr/share/doc/gpui-task-manager/{README.md,NOTICE.md,THIRD_PARTY_NOTICES.md,LICENSE,copyright}
#   /usr/share/doc/gpui-task-manager/licenses/ (vendored third-party license texts)
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/../.." && pwd)"
package="gpui-task-manager"
release="1"
# SECURITY.md routes reports through GitHub Security Advisory only, so the
# default carries no (fabricated) mail address. Distribution builds may set
# DEB_MAINTAINER to a real maintainer and contact address.
deb_maintainer="${DEB_MAINTAINER:-gpui-task-manager contributors (security reports via GitHub Security Advisory)}"

skip_build=false
for argument in "$@"; do
  case "${argument}" in
  --skip-build) skip_build=true ;;
  -h | --help)
    sed -n '3,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
  *)
    echo "error: unknown argument: ${argument}" >&2
    exit 2
    ;;
  esac
done

cd "${repo_root}"

# Drop stale packages before building. Release workflows upload the
# packaging/linux/build/*.deb wildcard, so an old artifact must not survive.
if [ -d "${script_dir}/build" ]; then
  find "${script_dir}/build" -maxdepth 1 -type f -name '*.deb' -delete
fi

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
  "${staging}/usr/share/doc/${package}" \
  "${staging}/usr/share/doc/${package}/licenses"

install -m 0755 "${binary}" "${staging}/usr/bin/${package}"
install -m 0644 "${script_dir}/${package}.desktop" "${staging}/usr/share/applications/${package}.desktop"
install -m 0644 "${script_dir}/${package}.svg" "${staging}/usr/share/icons/hicolor/scalable/apps/${package}.svg"
install -m 0644 "${script_dir}/${package}.png" "${staging}/usr/share/icons/hicolor/256x256/apps/${package}.png"
install -m 0644 "${script_dir}/README.deb.md" "${staging}/usr/share/doc/${package}/README.md"
install -m 0644 "${repo_root}/NOTICE.md" "${staging}/usr/share/doc/${package}/NOTICE.md"
install -m 0644 "${repo_root}/THIRD_PARTY_NOTICES.md" \
  "${staging}/usr/share/doc/${package}/THIRD_PARTY_NOTICES.md"
install -m 0644 "${repo_root}/LICENSE" "${staging}/usr/share/doc/${package}/LICENSE"
install -m 0644 "${script_dir}/copyright" "${staging}/usr/share/doc/${package}/copyright"

# Bundle the third-party license full texts that are actually vendored in this
# repository. A missing source is a hard error so the .deb never silently ships
# without a text that is known to exist. Texts that are not vendored are listed
# in README.deb.md with authoritative places to verify them; never invent one.
licenses_dir="${staging}/usr/share/doc/${package}/licenses"
vendored_licenses=("web/assets/fonts/LICENSE-IBM-Plex.txt")
for source_file in "${vendored_licenses[@]}"; do
  if [ ! -f "${repo_root}/${source_file}" ]; then
    echo "error: vendored license text is missing: ${source_file}" >&2
    exit 1
  fi
  install -m 0644 "${repo_root}/${source_file}" \
    "${licenses_dir}/$(basename -- "${source_file}")"
done

# Compute runtime dependencies from ldd + dpkg -S. The Vulkan loader is
# dlopen()ed by wgpu/Blade, so it does not appear in ldd and is added explicitly.
# `ldd` exits 0 even when a library is unresolved ("=> not found"), so check the
# output explicitly: otherwise awk would turn it into "not" and dpkg -S would
# silently skip it, producing a .deb with a missing dependency (R1S-OPS-05).
if ! ldd_output="$(LC_ALL=C ldd "${binary}" 2>&1)"; then
  echo "error: ldd failed for ${binary}:" >&2
  echo "${ldd_output}" >&2
  exit 1
fi
if missing="$(grep -E '=>[[:space:]]+not found' <<<"${ldd_output}")"; then
  echo "error: unresolved shared libraries (ldd 'not found'):" >&2
  echo "${missing}" >&2
  exit 1
fi

declare -A dependency_set=()
while IFS= read -r library; do
  [ -n "${library}" ] || continue
  resolved="$(readlink -f "${library}" 2>/dev/null || true)"
  [ -n "${resolved}" ] || continue
  # `sed -n '1p'` reads the whole stream, unlike `head -n 1` which closes the
  # pipe early and can abort this script via SIGPIPE under `set -o pipefail`.
  owner="$(dpkg -S "${resolved}" 2>/dev/null | sed -n '1p' | cut -d: -f1)"
  if [ -n "${owner}" ]; then
    dependency_set["${owner}"]=1
  fi
done < <(printf '%s\n' "${ldd_output}" | awk '/=>/ {print $3}' | sort -u)
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
Maintainer: ${deb_maintainer}
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

# A stale target/release binary (the --skip-build path) must not be packaged
# under a different version than the one recorded in DEBIAN/control. The native
# binary prints "<binary> <version>" for --version, while the control Version
# carries an extra Debian revision ("<version>-<release>").
control_version="$(
  awk -F': ' '$1 == "Version" { print $2; exit }' "${staging}/DEBIAN/control"
)"
binary_version="${version_output##* }"
if [ "${version_output}" != "${package} ${binary_version}" ] ||
  [ "${binary_version}" != "${control_version%-"${release}"}" ]; then
  # Do not leave a mismatched artifact for the release wildcard to upload.
  rm -f "${output}"
  echo "error: version mismatch: extracted binary reports '${version_output}'," >&2
  echo "       DEBIAN/control Version is '${control_version}'." >&2
  exit 1
fi

dpkg-deb -I "${output}" | sed -n '1,20p'

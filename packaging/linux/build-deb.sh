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
#   /usr/share/doc/gpui-task-manager/licenses/ (vendored + per-crate license texts)
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/../.." && pwd)"
package="gpui-task-manager"
release="1"

# Reproducible .deb: dpkg-deb (and the tar/gzip layers it drives) embed
# timestamps, so pin SOURCE_DATE_EPOCH to make identical inputs byte-identical.
# Precedence: an explicitly provided value always wins; otherwise derive the
# last commit time when git is usable; otherwise use a fixed epoch so source
# tarballs and git-less CI still build reproducibly.
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
  source_date_epoch=""
  if command -v git >/dev/null 2>&1; then
    source_date_epoch="$(git -C "${repo_root}" log -1 --format=%ct 2>/dev/null || true)"
  fi
  case "${source_date_epoch}" in
  '' | *[!0-9]*) source_date_epoch="1700000000" ;;
  esac
  SOURCE_DATE_EPOCH="${source_date_epoch}"
fi
export SOURCE_DATE_EPOCH

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

# Collect the license full texts of the resolved dependency graph from the
# local Cargo registry. This runs after `cargo build` (or, with --skip-build,
# after a previous build populated the cache), so `cargo metadata --offline`
# sees the same sources that produced the binary.
#
# Existing-solution survey (AGENTS.md §2, 2026-11-20):
#   - cargo-about is purpose-built for license reports, but is not installed
#     here: adopting it means `cargo install` (a new build-time dependency,
#     forbidden for this task). It also renders a consolidated report instead
#     of populating a per-crate directory of upstream license full texts.
#   - cargo-deny 0.21.0 is installed, but its license support is policy checking
#     (`cargo deny check licenses` / `cargo deny list`) against deny.toml. It
#     has no command that copies those full texts into a Debian package, so it
#     stays in place as the license-policy gate, not as the vendoring tool.
#   - cargo metadata + the local registry source cache needs no new dependency:
#     cargo and jq are already hard requirements of this script, --offline
#     keeps the step network-free, and each source directory is derived from
#     the package's manifest_path (no hardcoded registry hash or home path,
#     so the same code works in CI).
# Decision: use cargo metadata + jq; keep cargo-deny unchanged as policy gate.
vendor_crate_license_texts() {
  local licenses_root="$1"
  local metadata name version manifest source_dir file base
  local count=0
  local -a license_files=()
  local -a no_text_crates=()

  # Idempotent: rebuilding replaces the tree so a removed dependency cannot
  # leave a stale text behind. The `:?` guard prevents an empty root from
  # ever turning into `rm -rf /crates`.
  rm -rf "${licenses_root:?}/crates"
  mkdir -p "${licenses_root}/crates"

  # Filter to the Linux target: the unfiltered graph also resolves
  # macOS/Windows-only crates (e.g. accesskit_macos) that are not in the local
  # registry during a Linux-only CI build, which made --offline metadata fail.
  local target_platform
  target_platform="$(rustc -vV | sed -n 's/^host: //p')"
  if ! metadata="$(cargo metadata --offline --format-version 1 \
    --filter-platform "${target_platform}")"; then
    echo "error: cargo metadata --offline failed; build once to cache dependencies." >&2
    return 1
  fi

  while IFS=$'\t' read -r name version manifest; do
    [ -n "${name}" ] || continue
    source_dir="$(dirname -- "${manifest}")"
    [ -d "${source_dir}" ] || continue

    # Match upstream names case-insensitively: some crates ship `license-mit`
    # / `license-apache-2.0` instead of uppercase `LICENSE*`.
    license_files=()
    for file in "${source_dir}"/*; do
      [ -f "${file}" ] || continue
      base="$(basename -- "${file}")"
      case "${base,,}" in
      license* | licence* | copying*) license_files+=("${file}") ;;
      esac
    done

    # `source == null` marks this workspace's own crates, covered by the
    # top-level LICENSE / NOTICE.md installed above, not by per-crate texts.
    if ((${#license_files[@]} == 0)); then
      no_text_crates+=("${name}-${version}")
      continue
    fi

    mkdir -p "${licenses_root}/crates/${name}-${version}"
    for file in "${license_files[@]}"; do
      install -m 0644 "${file}" \
        "${licenses_root}/crates/${name}-${version}/$(basename -- "${file}")"
      count=$((count + 1))
    done
  done < <(printf '%s\n' "${metadata}" |
    jq -r '.packages[] | select(.source != null) | [.name, .version, .manifest_path] | @tsv')

  # Complete audit trail for crates whose published source has no text. The
  # installed README.deb.md points here: the gap is "not vendored yet", not a
  # claim that the upstream license is incompatible or cannot be shipped.
  : >"${licenses_root}/NOT-VENDORED.txt"
  if ((${#no_text_crates[@]} > 0)); then
    printf '%s\n' "${no_text_crates[@]}" | LC_ALL=C sort >>"${licenses_root}/NOT-VENDORED.txt"
  fi
  echo "==> vendored ${count} crate license text(s); ${#no_text_crates[@]} crate(s) have no local text"
}

# Bundle the third-party license full texts that are actually vendored in this
# repository. A missing source is a hard error so the .deb never silently ships
# without a text that is known to exist. Crates whose registry source has no
# text at all are recorded in NOT-VENDORED.txt (see README.deb.md for the
# authoritative verification links); never invent a license text.
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

# Every other license text ships inside the crate source archives themselves.
# This includes gpui-kit-assets/LICENSE-LUCIDE (ISC + Feather MIT attributions),
# which cargo metadata resolves from the local registry just like any crate.
vendor_crate_license_texts "${licenses_dir}"

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

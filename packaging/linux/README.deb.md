# GPUI Task Manager

A system task manager for Ubuntu-family desktops.
It shows process, CPU, memory, network and disk usage.

## Features

- Process list with name, command, CPU, memory, PID, user, state and thread
  count, plus a tree-view toggle
- Overview cards for CPU (total, per-core and history), memory / swap,
  network receive / transmit rates, host / uptime / disk usage and temperature
  sensors when available
- Column-click sorting, filtering by name / command / user / PID and a details
  panel
- **Filter presets** (all / high CPU / high memory / large virtual memory /
  running) applied in one click; **refresh-interval presets** (0.5s / 1s / 2s /
  5s) can be switched and saved (native and browser)
- **Power- and window-aware sampling**: 1s on AC, 2.5s on battery, 5s at low
  battery (20% or less) or when the window is hidden. Applied as a lower bound
  on the configured interval, so deliberately slower settings are preserved
  (native and browser)
- **Read-only cgroup view**: normalized group labels, member counts and the
  group count in the status bar; no process control or limit changes
- Signal actions (SIGTERM / SIGKILL / SIGSTOP / SIGCONT) behind a confirmation
  dialog, with PID 1 / self protection and PID-reuse detection
- Keyboard shortcuts: Up/Down, PgUp/PgDn, Home/End, Space, Ctrl+F, Ctrl+K, F2,
  F5, F1
- **Japanese IME (composition) support** in both the browser and native builds:
  uncommitted text stays in the input field and only committed text is applied
  to the filter
- **CSV / JSON export**: `gpui-task-manager --export csv|json PATH` writes a
  redacted snapshot with mode 0600 and exits; the browser toolbar has matching
  CSV / JSON export buttons (both share `tm_core::export`)
- **Settings persistence** (native and browser): refresh interval, sort column
  and direction, details panel and tree view are restored on next launch.
  Native uses XDG config (`~/.config/gpui-task-manager/settings.json`, with
  platform-specific paths on Windows / macOS, atomic writes, mode 0600 and a
  schema version); browser uses the `localStorage` key
  `gpui-task-manager.settings`
- **Corrupted-settings eviction**: a corrupt settings file is moved aside with
  a `.corrupt-<timestamp>` suffix and the app starts with defaults; only the
  five most recent backups are kept and manual restore is documented in the
  repository README
- **Light theme (Catppuccin Latte)**: a header Light/Dark button in both builds
  toggles the theme and persists the choice in settings
- Deterministic `--demo` data for screenshots and regression checks

## Install

```bash
sudo apt install ./gpui-task-manager_<version>-1_<arch>.deb
```

## Run

Launch "GPUI Task Manager" from the applications menu, or run:

```bash
gpui-task-manager
```

## License

AGPL-3.0-or-later. See the LICENSE, NOTICE.md and THIRD_PARTY_NOTICES.md
files included in this package. Third-party components and their licenses are
listed in `NOTICE.md`; package authorship and the Debian copyright format are in
`copyright`.

## Third-party license texts

License full texts are installed under
`/usr/share/doc/gpui-task-manager/licenses/`. The package build combines two
sources there:

- texts vendored directly in this source tree, listed in the table below;
- every `LICENSE*` / `LICENCE*` / `COPYING*` file found by `build-deb.sh` in the
  source directory of a resolved dependency, installed as
  `licenses/crates/<name>-<version>/<file>`. Files are copied verbatim, so the
  original copyright notice and attribution remain with the text. The
  reference build covers 724 of the 804 registry packages this way, including
  `gpui-kit-assets/LICENSE-LUCIDE` for the Lucide icons.

| File | License | Component |
| --- | --- | --- |
| `LICENSE-IBM-Plex.txt` | SIL OFL 1.1 (Reserved Font Name "Plex") | IBM Plex Sans / Mono fonts |
| `crates/gpui-kit-assets-<version>/LICENSE-LUCIDE` | ISC, with Feather-derived icons under MIT | Lucide icons bundled by `gpui-kit-assets` |
| `crates/<name>-<version>/LICENSE*` | per-crate declaration (MIT, Apache-2.0, ISC, BSD, Zlib, ...) | Rust dependencies from `Cargo.lock` |
| `NOT-VENDORED.txt` | — | complete list of crates with no local license text |

`LICENSE-IBM-Plex.txt` also contains the ISC text for the font's own Debian
packaging files and a pointer to CC0-1.0; that ISC text is not the one used by
Lucide. The Apache-2.0 full text is installed as `THIRD_PARTY_NOTICES.md` in
`/usr/share/doc/gpui-task-manager/` (Debian systems also provide
`/usr/share/common-licenses/Apache-2.0`).

### Crates without a vendored text (not vendored yet)

`licenses/NOT-VENDORED.txt`, installed with this package, lists every resolved
registry package whose published source has no license text file. The
reference build lists 80 such crates; most target Windows or macOS and are not
linked into the Linux binary. This is a packaging gap, **not** a license
incompatibility: their SPDX expressions are still declared in `NOTICE.md` and
allowed by `deny.toml`. Confirm the authoritative text for any such crate
before relying on it; upstream links for concrete crates are in `NOTICE.md`.

| License | Where to confirm |
| --- | --- |
| MIT / Apache-2.0 (crates with no text in their registry source) | `licenses/NOT-VENDORED.txt`; upstream repositories linked in `NOTICE.md`; <https://spdx.org/licenses/MIT.html>; <https://spdx.org/licenses/Apache-2.0.html> |
| Unicode-3.0 (allowed by `deny.toml`) | <https://www.unicode.org/license.txt>; <https://spdx.org/licenses/Unicode-3.0.html> |
| BSL-1.0 (allowed by `deny.toml`) | <https://www.boost.org/LICENSE_1_0.txt>; <https://spdx.org/licenses/BSL-1.0.html> |

## Software bill of materials (SBOM)

CycloneDX SBOMs are generated as per-target snapshots of the two Cargo
workspaces: the native workspace is resolved for `x86_64-unknown-linux-gnu`
and the browser workspace (`web/`) for `wasm32-unknown-unknown`. Each BOM
contains only the default features and the dependency closure of its own
target. It therefore does **not** cover every target audited by `deny.toml`
(Windows, macOS, aarch64, ...) or optional features. To inspect a different
target, regenerate the SBOM for that target or consult `cargo deny check` /
`cargo metadata --locked --filter-platform <target>`.

## Source

<https://github.com/Seduce0068/gpui-task-manager>

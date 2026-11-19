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

License full texts that are vendored in this source tree are installed under
`/usr/share/doc/gpui-task-manager/licenses/`:

| File | License | Component |
| --- | --- | --- |
| `LICENSE-IBM-Plex.txt` | SIL OFL 1.1 (Reserved Font Name "Plex") | IBM Plex Sans / Mono fonts |

`LICENSE-IBM-Plex.txt` also contains the ISC text for the font's own Debian
packaging files and a pointer to CC0-1.0; that ISC text is not the one used by
Lucide. The Apache-2.0 full text is installed as `THIRD_PARTY_NOTICES.md` in
`/usr/share/doc/gpui-task-manager/` (Debian systems also provide
`/usr/share/common-licenses/Apache-2.0`).

The following license full texts are **not** vendored in this repository, so
this package cannot ship them. Check the authoritative sources below; upstream
links for the concrete crates and components are in `NOTICE.md`.

| License | Where to confirm |
| --- | --- |
| MIT (sysinfo, serde / serde_json, wasm-bindgen, web-sys / js-sys, console_error_panic_hook, anyhow / futures / log) | Upstream repositories linked in `NOTICE.md`; <https://spdx.org/licenses/MIT.html> |
| ISC / MIT (Lucide icons) | <https://github.com/lucide-icons/lucide/blob/main/LICENSE>; <https://spdx.org/licenses/ISC.html> |
| Unicode-3.0 (allowed by `deny.toml`) | <https://www.unicode.org/license.txt>; <https://spdx.org/licenses/Unicode-3.0.html> |
| BSL-1.0 (allowed by `deny.toml`) | <https://www.boost.org/LICENSE_1_0.txt>; <https://spdx.org/licenses/BSL-1.0.html> |

## Source

<https://github.com/Seduce0068/gpui-task-manager>

# GPUI Task Manager

A system task manager for Ubuntu-family desktops.
It shows process, CPU, memory, network and disk usage.

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

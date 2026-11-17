# Linux packaging (DEB)

`build-deb.sh` builds a Debian package without root and without installing
anything on the host. The first build compiles the release binary; subsequent
builds can reuse it with `--skip-build`.

```bash
# From the repository root
bash packaging/linux/build-deb.sh

# Re-package an existing target/release binary
bash packaging/linux/build-deb.sh --skip-build
```

Output: `packaging/linux/build/gpui-task-manager_<version>-1_<arch>.deb`

## Contents

| Path | Purpose |
|---|---|
| `/usr/bin/gpui-task-manager` | Native GPUI binary |
| `/usr/share/applications/gpui-task-manager.desktop` | Desktop entry (`StartupWMClass=gpui-task-manager`) |
| `/usr/share/icons/hicolor/scalable/apps/gpui-task-manager.svg` | Scalable icon |
| `/usr/share/icons/hicolor/256x256/apps/gpui-task-manager.png` | Raster icon |
| `/usr/share/doc/gpui-task-manager/{README.md,NOTICE.md,copyright}` | Documentation and attribution |

## Notes

- Runtime dependencies are derived from `ldd` + `dpkg -S`; `libvulkan1` is added
  explicitly because Vulkan is loaded at runtime with `dlopen`. Unresolved
  libraries (`ldd` "not found") fail the build rather than being silently
  omitted from `Depends`.
- No maintainer scripts or conffiles are installed: `/usr` files are removed by
  `dpkg -R`, and user settings under `~/.config/gpui-task-manager/` are kept
  intentionally. With no `postrm`/conffiles, `apt remove` effectively purges
  the package; no `/var` state or logs are created.
- The package is unsigned. Signing and repository hosting are out of scope for
  the prototype.
- Installation is left to the user (`sudo apt install ./...deb`); the script
  itself never invokes `sudo` and never installs to the host.

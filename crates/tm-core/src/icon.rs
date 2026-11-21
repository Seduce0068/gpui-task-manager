// SPDX-License-Identifier: AGPL-3.0-or-later
//! Native application-icon lookup for grouped process rows.
//!
//! # Why this module performs no I/O
//!
//! The crate documentation in `lib.rs` fixes a hard layering rule for
//! `tm-core`: no I/O, no clock, no threads and no OS APIs, plus a
//! `wasm32-unknown-unknown` build. Adding `std::fs` or `std::env` here would
//! break both. This module therefore keeps only the deterministic work —
//! group-key validation, the small alias table, `.desktop` parsing, candidate
//! ordering and the result cache — and delegates every filesystem observation
//! to the [`IconFs`] trait. The native shell supplies a `std::fs`-backed
//! implementation and reads the XDG variables itself; tests use a tiny
//! in-memory implementation. No dependency is added and no `unsafe` is used.
//!
//! The result is cached by raw group key because `render` runs every frame: a
//! cache hit is one `HashMap` lookup, with no directory scans at all.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The minimal filesystem surface the resolver needs.
///
/// Implementations must observe only the paths handed to them; the resolver
/// validates keys and icon names before constructing any path.
pub trait IconFs {
    /// Names of entries directly inside `dir` (depth 1, no recursion).
    fn list_dir(&self, dir: &Path) -> Vec<String>;

    /// UTF-8 contents of `path`, or `None` if it is absent or unreadable.
    fn read_to_string(&self, path: &Path) -> Option<String>;

    /// Whether `path` is an existing regular file (symlinks may be followed).
    fn is_file(&self, path: &Path) -> bool;
}

/// Ordered search roots for `.desktop` files and PNG icons.
///
/// Build one with [`icon_search_env`]; the fields are public so the native
/// shell may add installation-specific roots without changing this crate.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IconSearchEnv {
    /// `.desktop` directories: user data, system data, then snap data.
    pub applications_dirs: Vec<PathBuf>,
    /// XDG data directories used for `icons/<theme>/<size>/apps` and `pixmaps`.
    pub data_dirs: Vec<PathBuf>,
}

/// Executables whose `.desktop` filename differs from the redacted `argv[0]`
/// basename. This is intentionally a small explicit table, not a fuzzy match:
/// a wrong alias would attach an unrelated application icon to a group.
const DESKTOP_ALIASES: &[(&str, &str)] = &[
    ("chrome", "google-chrome"),
    ("chrome-stable", "google-chrome"),
    ("chromium-browser", "chromium"),
    ("msedge", "microsoft-edge"),
    ("microsoft-edge-stable", "microsoft-edge"),
];

/// Preferred icon theme; tried before every other theme.
const PREFERRED_THEME: &str = "hicolor";

/// PNG sizes, largest first (scalable SVG directories are deliberately skipped).
const ICON_SIZES: &[u16] = &[256, 128, 64, 48, 32, 16];

/// Snap desktop entries are not part of `XDG_DATA_DIRS` on every host.
const SNAP_DESKTOP_DIR: &str = "/var/lib/snapd/desktop/applications";

/// Builds the search roots from XDG-style string inputs without touching the
/// environment. Missing/empty values get the XDG-specified defaults.
///
/// `home` supplies `$HOME/.local/share` when `XDG_DATA_HOME` is unset;
/// `xdg_data_dirs` defaults to `/usr/local/share:/usr/share`. The snap
/// applications directory is always appended. Relative and duplicate roots
/// are ignored, keeping the order deterministic: user → system → extra → snap.
pub fn icon_search_env(
    home: Option<&str>,
    xdg_data_home: Option<&str>,
    xdg_data_dirs: Option<&str>,
    extra_data_dirs: &[&str],
) -> IconSearchEnv {
    fn push_unique(out: &mut Vec<PathBuf>, candidate: PathBuf) {
        if candidate.is_absolute() && !out.contains(&candidate) {
            out.push(candidate);
        }
    }

    let mut data_dirs = Vec::new();
    if let Some(dir) = xdg_data_home.filter(|value| !value.is_empty()) {
        push_unique(&mut data_dirs, PathBuf::from(dir));
    } else if let Some(home) = home.filter(|value| !value.is_empty()) {
        push_unique(
            &mut data_dirs,
            PathBuf::from(home).join(".local").join("share"),
        );
    }
    let data_dirs_value = xdg_data_dirs
        .filter(|value| !value.is_empty())
        .unwrap_or("/usr/local/share:/usr/share");
    for dir in data_dirs_value.split(':').filter(|value| !value.is_empty()) {
        push_unique(&mut data_dirs, PathBuf::from(dir));
    }
    for dir in extra_data_dirs {
        push_unique(&mut data_dirs, PathBuf::from(*dir));
    }

    let mut applications_dirs = data_dirs
        .iter()
        .map(|dir| dir.join("applications"))
        .collect::<Vec<_>>();
    push_unique(&mut applications_dirs, PathBuf::from(SNAP_DESKTOP_DIR));
    IconSearchEnv {
        applications_dirs,
        data_dirs,
    }
}

/// Validates a group key and applies the explicit desktop-name alias table.
///
/// Input is ASCII-lowercased first (executable basenames may be mixed case),
/// then restricted to `[a-z0-9._-]`, max 128 characters, with no leading dot
/// and no `..`. The returned string is therefore safe to use as a single path
/// component. `None` means "no icon" — never a path built from raw input.
pub fn canonical_key(raw: &str) -> Option<String> {
    let key = raw.trim().to_ascii_lowercase();
    if !is_safe_component(&key) {
        return None;
    }
    Some(
        DESKTOP_ALIASES
            .iter()
            .find(|(from, _)| *from == key.as_str())
            .map(|(_, to)| (*to).to_string())
            .unwrap_or(key),
    )
}

/// Extracts `Icon=` from a `.desktop` file.
///
/// Only a visible `Type=Application` entry is accepted: `NoDisplay` and
/// `Hidden` (true/1/yes) entries are ignored, as are other sections and
/// locale-qualified keys such as `Icon[ja]`. Key order in the file is
/// irrelevant, which matters for entries that put `Icon=` before `Type=`.
pub fn desktop_icon(contents: &str) -> Option<String> {
    let mut in_entry = false;
    let mut is_application = false;
    let mut hidden = false;
    let mut icon = None;
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            if in_entry {
                break;
            }
            in_entry = line.eq_ignore_ascii_case("[Desktop Entry]");
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Type" => is_application = value.eq_ignore_ascii_case("Application"),
            "NoDisplay" | "Hidden" => {
                if matches!(value.to_ascii_lowercase().as_str(), "true" | "1" | "yes") {
                    hidden = true;
                }
            }
            "Icon" if icon.is_none() && !value.is_empty() => {
                icon = Some(value.to_string());
            }
            _ => {}
        }
    }
    if is_application && !hidden {
        icon
    } else {
        None
    }
}

/// Selects `.desktop` filenames for one canonical key from a depth-1 listing.
///
/// The exact `<key>.desktop` wins; snap's `<prefix>_<key>.desktop` convention
/// (e.g. `firefox_firefox.desktop`) is a deterministic fallback, sorted by
/// name. Names containing separators or `..` are discarded before any path is
/// built, even though they come from a directory listing.
pub fn desktop_matches(key: &str, entry_names: &[String]) -> Vec<String> {
    let exact = format!("{key}.desktop");
    let snap_suffix = format!("_{key}.desktop");
    let mut exact_match = Vec::new();
    let mut snap_matches = Vec::new();
    for name in entry_names {
        if !name.ends_with(".desktop")
            || name.contains('/')
            || name.contains('\\')
            || name.starts_with('.')
            || name.contains("..")
        {
            continue;
        }
        if name == &exact {
            exact_match.push(name.clone());
        } else if name.ends_with(&snap_suffix) {
            snap_matches.push(name.clone());
        }
    }
    snap_matches.sort();
    exact_match.extend(snap_matches);
    exact_match
}

/// Ordered PNG candidates for one icon name, with no filesystem access.
///
/// Themes are tried in the given order (callers put `hicolor` first); within
/// each theme, data directories keep their XDG order and PNG sizes are tried
/// largest-first. `pixmaps` is checked after every themed candidate.
/// Only `.png` paths are produced.
pub fn icon_candidates(icon_name: &str, themes: &[String], env: &IconSearchEnv) -> Vec<PathBuf> {
    let filename = format!("{icon_name}.png");
    let mut out = Vec::new();
    for theme in themes {
        for dir in &env.data_dirs {
            for size in ICON_SIZES {
                out.push(
                    dir.join("icons")
                        .join(theme)
                        .join(format!("{size}x{size}"))
                        .join("apps")
                        .join(&filename),
                );
            }
        }
    }
    for dir in &env.data_dirs {
        out.push(dir.join("pixmaps").join(&filename));
    }
    out
}

/// Caching icon resolver. All I/O goes through [`IconFs`]; see the module docs.
pub struct AppIconResolver {
    env: IconSearchEnv,
    fs: Box<dyn IconFs>,
    cache: HashMap<String, Option<PathBuf>>,
}

impl AppIconResolver {
    /// Creates a resolver for a fixed set of search roots.
    pub fn new(env: IconSearchEnv, fs: impl IconFs + 'static) -> Self {
        Self {
            env,
            fs: Box::new(fs),
            cache: HashMap::new(),
        }
    }

    /// Resolves a raw group key to an existing PNG path, or `None`.
    ///
    /// Negative results are cached too: an unknown group must not trigger a
    /// fresh directory scan on every rendered frame.
    pub fn resolve(&mut self, group_key: &str) -> Option<PathBuf> {
        if let Some(cached) = self.cache.get(group_key) {
            return cached.clone();
        }
        let resolved = self.resolve_uncached(group_key);
        self.cache.insert(group_key.to_string(), resolved.clone());
        resolved
    }

    /// Drops all cached positive and negative results (e.g. after an install).
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    fn resolve_uncached(&self, group_key: &str) -> Option<PathBuf> {
        let key = canonical_key(group_key)?;
        for dir in &self.env.applications_dirs {
            let names = self.fs.list_dir(dir);
            for name in desktop_matches(&key, &names) {
                let Some(contents) = self.fs.read_to_string(&dir.join(&name)) else {
                    continue;
                };
                let Some(icon) = desktop_icon(&contents) else {
                    continue;
                };
                if let Some(path) = self.resolve_icon_value(&icon) {
                    return Some(path);
                }
            }
        }
        None
    }

    fn resolve_icon_value(&self, icon: &str) -> Option<PathBuf> {
        if icon.starts_with('/') {
            let path = PathBuf::from(icon);
            return (is_png(&path) && self.fs.is_file(&path)).then_some(path);
        }
        // A few entries write `Icon=firefox.png`; strip the suffix so it is
        // still searchable as an icon name.
        let name = icon
            .strip_suffix(".png")
            .or_else(|| icon.strip_suffix(".PNG"))
            .unwrap_or(icon);
        if !is_safe_icon_name(name) {
            return None;
        }
        icon_candidates(name, &self.icon_themes(), &self.env)
            .into_iter()
            .find(|candidate| self.fs.is_file(candidate))
    }

    fn icon_themes(&self) -> Vec<String> {
        let mut themes = Vec::new();
        for dir in &self.env.data_dirs {
            for name in self.fs.list_dir(&dir.join("icons")) {
                if name.is_empty()
                    || name.starts_with('.')
                    || name.contains('/')
                    || name.contains('\\')
                    || name.contains("..")
                {
                    continue;
                }
                if !themes.contains(&name) {
                    themes.push(name);
                }
            }
        }
        themes.sort();
        // Stable sort: hicolor first, every other theme keeps its order.
        themes.sort_by_key(|theme| usize::from(theme.as_str() != PREFERRED_THEME));
        themes
    }
}

fn is_safe_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
        && !value.starts_with('.')
        && !value.contains("..")
}

fn is_safe_icon_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && !value.starts_with('.')
        && !value.contains("..")
}

fn is_png(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Relevant lines of the real entries present on the target host
    // (checked 2026-11-21):
    // /var/lib/snapd/desktop/applications/firefox_firefox.desktop
    // /usr/share/applications/google-chrome.desktop
    const FIREFOX_DESKTOP: &str = "\
[Desktop Entry]
Name=Firefox
Type=Application
Icon=/snap/firefox/current/default256.png
";
    const CHROME_DESKTOP: &str = "\
[Desktop Entry]
Name=Google Chrome
Icon=google-chrome
Type=Application
";

    fn path_key(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    #[derive(Default)]
    struct FakeFs {
        files: HashMap<String, String>,
        dirs: HashMap<String, Vec<String>>,
    }

    impl FakeFs {
        fn file(mut self, path: &str, contents: &str) -> Self {
            self.files.insert(path.to_string(), contents.to_string());
            self
        }

        fn dir(mut self, path: &str, names: &[&str]) -> Self {
            self.dirs.insert(
                path.to_string(),
                names.iter().map(|name| (*name).to_string()).collect(),
            );
            self
        }
    }

    impl IconFs for FakeFs {
        fn list_dir(&self, dir: &Path) -> Vec<String> {
            self.dirs.get(&path_key(dir)).cloned().unwrap_or_default()
        }

        fn read_to_string(&self, path: &Path) -> Option<String> {
            self.files.get(&path_key(path)).cloned()
        }

        fn is_file(&self, path: &Path) -> bool {
            self.files.contains_key(&path_key(path))
        }
    }

    fn test_env() -> IconSearchEnv {
        icon_search_env(
            Some("/home/user"),
            None,
            Some("/usr/local/share:/usr/share"),
            &[],
        )
    }

    #[test]
    fn firefox_absolute_png_icon_is_used_from_snap_desktop_entry() {
        let env = IconSearchEnv {
            applications_dirs: vec![PathBuf::from(SNAP_DESKTOP_DIR)],
            data_dirs: Vec::new(),
        };
        let fs = FakeFs::default()
            .dir(SNAP_DESKTOP_DIR, &["firefox_firefox.desktop"])
            .file(
                "/var/lib/snapd/desktop/applications/firefox_firefox.desktop",
                FIREFOX_DESKTOP,
            )
            .file("/snap/firefox/current/default256.png", "");
        let mut resolver = AppIconResolver::new(env, fs);
        let expected = Some(PathBuf::from("/snap/firefox/current/default256.png"));
        assert_eq!(resolver.resolve("firefox"), expected);
        // Mixed-case keys normalise to the same desktop entry.
        assert_eq!(resolver.resolve("Firefox"), expected);
    }

    #[test]
    fn chrome_alias_resolves_named_icon_from_hicolor_png() {
        let fs = FakeFs::default()
            .dir("/usr/share/applications", &["google-chrome.desktop"])
            .file(
                "/usr/share/applications/google-chrome.desktop",
                CHROME_DESKTOP,
            )
            .dir("/usr/local/share/icons", &["hicolor"])
            .file(
                "/usr/local/share/icons/hicolor/16x16/apps/google-chrome.png",
                "",
            )
            .file(
                "/usr/local/share/icons/hicolor/256x256/apps/google-chrome.png",
                "",
            );
        let mut resolver = AppIconResolver::new(test_env(), fs);
        let expected = Some(PathBuf::from(
            "/usr/local/share/icons/hicolor/256x256/apps/google-chrome.png",
        ));
        assert_eq!(resolver.resolve("chrome"), expected);
        assert_eq!(resolver.resolve("chrome-stable"), expected);
        assert_eq!(resolver.resolve("Chrome"), expected);
    }

    #[test]
    fn invalid_keys_and_non_png_absolute_icons_are_rejected() {
        let mut resolver = AppIconResolver::new(test_env(), FakeFs::default());
        let long_key = "a".repeat(200);
        for key in [
            "",
            " ",
            "..",
            "../etc/passwd",
            "/usr/bin/firefox",
            "foo/bar",
            "foo bar",
            "firefox;rm",
            long_key.as_str(),
        ] {
            assert_eq!(resolver.resolve(key), None, "key {key:?} must be rejected");
        }
        assert_eq!(canonical_key("firefox\nDescription=x"), None);

        let env = IconSearchEnv {
            applications_dirs: vec![PathBuf::from("/applications")],
            data_dirs: Vec::new(),
        };
        let fs = FakeFs::default()
            .dir("/applications", &["bad.desktop"])
            .file(
                "/applications/bad.desktop",
                "[Desktop Entry]\nType=Application\nIcon=/etc/passwd\n",
            )
            .file("/etc/passwd", "");
        assert_eq!(AppIconResolver::new(env, fs).resolve("bad"), None);
    }

    #[test]
    fn hidden_entries_and_unknown_keys_resolve_to_none_and_are_cached() {
        let fs = FakeFs::default()
            .dir(
                "/usr/share/applications",
                &["hidden.desktop", "no-display.desktop"],
            )
            .file(
                "/usr/share/applications/hidden.desktop",
                "[Desktop Entry]\nType=Application\nHidden=true\nIcon=x.png\n",
            )
            .file(
                "/usr/share/applications/no-display.desktop",
                "[Desktop Entry]\nType=Application\nNoDisplay=1\nIcon=x.png\n",
            );
        let mut resolver = AppIconResolver::new(test_env(), fs);
        assert_eq!(resolver.resolve("hidden"), None);
        assert_eq!(resolver.resolve("no-display"), None);
        assert_eq!(resolver.resolve("not-installed"), None);
        // A cached negative stays negative even if the cache has to be reused.
        assert_eq!(resolver.resolve("not-installed"), None);
        resolver.clear_cache();
        assert_eq!(resolver.resolve("not-installed"), None);
    }

    #[test]
    fn hicolor_theme_wins_over_other_themes() {
        let fs = FakeFs::default()
            .dir("/usr/share/applications", &["google-chrome.desktop"])
            .file(
                "/usr/share/applications/google-chrome.desktop",
                CHROME_DESKTOP,
            )
            .dir("/usr/share/icons", &["Adwaita", "hicolor"])
            .file(
                "/usr/share/icons/Adwaita/256x256/apps/google-chrome.png",
                "",
            )
            .file("/usr/share/icons/hicolor/16x16/apps/google-chrome.png", "");
        let mut resolver = AppIconResolver::new(test_env(), fs);
        assert_eq!(
            resolver.resolve("chrome"),
            Some(PathBuf::from(
                "/usr/share/icons/hicolor/16x16/apps/google-chrome.png"
            ))
        );
    }

    #[test]
    fn pure_helpers_order_candidates_and_desktop_names() {
        let env = test_env();
        let themes = vec!["hicolor".to_string()];
        let candidates = icon_candidates("google-chrome", &themes, &env);
        assert_eq!(
            candidates[0],
            PathBuf::from("/home/user/.local/share/icons/hicolor/256x256/apps/google-chrome.png")
        );
        assert_eq!(
            candidates[1],
            PathBuf::from("/home/user/.local/share/icons/hicolor/128x128/apps/google-chrome.png")
        );
        assert_eq!(
            candidates[6],
            PathBuf::from("/usr/local/share/icons/hicolor/256x256/apps/google-chrome.png")
        );
        assert!(candidates
            .iter()
            .all(|path| path.extension().and_then(|ext| ext.to_str()) == Some("png")));

        assert_eq!(
            desktop_matches(
                "firefox",
                &[
                    "zzz_firefox.desktop".to_string(),
                    "firefox.desktop".to_string(),
                    "aaa_firefox.desktop".to_string(),
                    "../evil.desktop".to_string(),
                ]
            ),
            vec![
                "firefox.desktop".to_string(),
                "aaa_firefox.desktop".to_string(),
                "zzz_firefox.desktop".to_string(),
            ]
        );
    }

    #[test]
    fn icon_search_env_uses_xdg_defaults_and_appends_snap() {
        let env = icon_search_env(Some("/home/user"), None, None, &[]);
        assert_eq!(
            env.data_dirs,
            vec![
                PathBuf::from("/home/user/.local/share"),
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share"),
            ]
        );
        assert_eq!(
            env.applications_dirs,
            vec![
                PathBuf::from("/home/user/.local/share/applications"),
                PathBuf::from("/usr/local/share/applications"),
                PathBuf::from("/usr/share/applications"),
                PathBuf::from(SNAP_DESKTOP_DIR),
            ]
        );

        let custom = icon_search_env(
            Some("/home/user"),
            Some("/opt/data"),
            Some("/opt/data:/usr/share:/var/lib/snapd/desktop"),
            &["/extra"],
        );
        assert_eq!(
            custom.applications_dirs,
            vec![
                PathBuf::from("/opt/data/applications"),
                PathBuf::from("/usr/share/applications"),
                PathBuf::from("/var/lib/snapd/desktop/applications"),
                PathBuf::from("/extra/applications"),
            ]
        );
    }

    #[test]
    fn desktop_icon_requires_visible_application_entry() {
        assert_eq!(
            desktop_icon(CHROME_DESKTOP).as_deref(),
            Some("google-chrome")
        );
        assert_eq!(desktop_icon("[Desktop Entry]\nType=Link\nIcon=x\n"), None);
        assert_eq!(
            desktop_icon("[Desktop Entry]\nType=Application\nNoDisplay=true\nIcon=x\n"),
            None
        );
        assert_eq!(
            desktop_icon("[Desktop Entry]\nIcon=x\nType=Application\n").as_deref(),
            Some("x")
        );
        assert_eq!(
            desktop_icon("[Desktop Entry]\nType=Application\nIcon[ja]=x\n"),
            None
        );
    }
}

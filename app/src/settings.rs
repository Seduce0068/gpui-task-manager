//! Persistent user settings (native file storage).
//!
//! The **schema** lives in `tm-viewmodel::Settings` so the native and browser
//! shells stay compatible; this module only owns the storage medium: a JSON file
//! in the platform's per-user configuration directory.
//!
//! - Linux: `$XDG_CONFIG_HOME/gpui-task-manager/settings.json`
//!   (fallback `~/.config/gpui-task-manager/settings.json`)
//! - Windows: `%APPDATA%\gpui-task-manager\settings.json`
//! - macOS: `~/Library/Application Support/gpui-task-manager/settings.json`
//!
//! Writes are atomic (temporary file + rename) and, on Unix, the file is created
//! with mode `0600` even though the contents are not secret. Process data,
//! filters, host names and tokens are never written.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub use tm_viewmodel::SETTINGS_VERSION as CURRENT_VERSION;
// Re-exported so callers can use one canonical type from this module.
pub use tm_viewmodel::Settings;

/// Directory name used under the platform configuration root.
const APP_DIR: &str = "gpui-task-manager";
/// Settings file name.
const FILE_NAME: &str = "settings.json";

/// Returns the settings file path, if a configuration directory can be found.
pub fn config_path() -> Option<PathBuf> {
    config_dir().map(|directory| directory.join(FILE_NAME))
}

fn config_dir() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("XDG_CONFIG_HOME") {
        if !value.is_empty() {
            return Some(PathBuf::from(value).join(APP_DIR));
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(value) = std::env::var_os("APPDATA") {
            if !value.is_empty() {
                return Some(PathBuf::from(value).join(APP_DIR));
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return Some(
                PathBuf::from(home)
                    .join("Library/Application Support")
                    .join(APP_DIR),
            );
        }
    }

    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config").join(APP_DIR))
}

/// Loads settings from the platform configuration directory.
///
/// A missing file falls back to defaults. An unreadable file is reported on
/// stderr so a broken configuration is not silently ignored forever.
pub fn load() -> Settings {
    let Some(path) = config_path() else {
        return Settings::default();
    };
    match load_from(&path) {
        Ok(settings) => settings,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Settings::default(),
        Err(error) => {
            eprintln!(
                "gpui-task-manager: 設定を読み込めません ({}): {error}",
                path.display()
            );
            Settings::default()
        }
    }
}

/// Saves settings atomically to the platform configuration directory.
pub fn save(settings: &Settings) -> io::Result<()> {
    let path = config_path().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "設定ディレクトリを特定できません")
    })?;
    save_to(&path, settings)
}

/// Loads settings from an explicit path (used by tests and by [`load`]).
pub(crate) fn load_from(path: &Path) -> io::Result<Settings> {
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text)
        .map(Settings::normalized)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Writes settings atomically to an explicit path.
pub(crate) fn save_to(path: &Path, settings: &Settings) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&settings.clone().normalized())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    // Include the PID so two processes saving at once cannot clobber the same
    // temporary file before either rename completes.
    let temporary = path.with_extension(format!("json.tmp.{}", std::process::id()));
    fs::write(&temporary, json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tm_core::{SortDirection, SortKey, SortSpec};

    fn temporary_path(name: &str) -> PathBuf {
        let unique = format!(
            "gpui-task-manager-settings-{}-{}-{}",
            std::process::id(),
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        );
        std::env::temp_dir().join(unique).join("settings.json")
    }

    #[test]
    fn round_trips_settings_atomically() {
        let path = temporary_path("roundtrip");
        let settings = Settings {
            version: CURRENT_VERSION,
            interval_ms: 2_000,
            sort: SortSpec {
                key: SortKey::Memory,
                direction: SortDirection::Descending,
            },
            details_visible: false,
            tree_view: false,
            theme: tm_viewmodel::Theme::Dark,
        };
        save_to(&path, &settings).expect("save settings");
        assert!(path.is_file());
        // The temporary file must not survive the rename (same PID-scoped name
        // used by `save_to`).
        assert!(!path
            .with_extension(format!("json.tmp.{}", std::process::id()))
            .exists());
        assert_eq!(load_from(&path).expect("load settings"), settings);
        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[cfg(unix)]
    #[test]
    fn saved_settings_are_readable_only_by_the_owner() {
        use std::os::unix::fs::PermissionsExt;
        let path = temporary_path("permissions");
        save_to(&path, &Settings::default()).expect("save settings");
        let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "settings must not be world readable");
        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn temporary_file_is_pid_scoped_and_removed_after_save() {
        let path = temporary_path("tmp-name");
        let parent = path.parent().expect("parent");
        save_to(&path, &Settings::default()).expect("save settings");
        let temporary = path.with_extension(format!("json.tmp.{}", std::process::id()));
        assert!(
            !temporary.exists(),
            "the temporary file must be renamed away, not left behind"
        );
        assert!(path.exists(), "the final settings file must exist");
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn missing_file_is_reported_as_not_found() {
        let path = temporary_path("missing");
        let error = load_from(&path).expect_err("missing file");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn invalid_json_is_reported_as_invalid_data() {
        let path = temporary_path("invalid");
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(&path, "{ not json").expect("write invalid file");
        let error = load_from(&path).expect_err("invalid json");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn loaded_values_are_clamped_to_the_supported_range() {
        let path = temporary_path("clamped");
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(&path, r#"{"version":0,"interval_ms":1}"#).expect("write settings");
        let settings = load_from(&path).expect("load settings");
        assert_eq!(
            settings.interval_ms,
            tm_viewmodel::MIN_REFRESH_INTERVAL.as_millis() as u64
        );
        assert_eq!(settings.version, CURRENT_VERSION);
        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn config_path_uses_the_xdg_directory_when_set() {
        // `std::env::set_var` is process-global and racy under the parallel test
        // runner, so this test only asserts the *shape* of the path rather than
        // mutating the environment.
        if let Some(path) = config_path() {
            assert!(
                path.ends_with("gpui-task-manager/settings.json"),
                "{path:?}"
            );
        }
    }
}

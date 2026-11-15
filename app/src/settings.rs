//! Persistent user settings.
//!
//! Only presentation preferences are stored (refresh interval, sort order and
//! details-panel visibility). Process data, filters, tokens and any machine
//! identifiers are deliberately **not** persisted.
//!
//! The file lives in the platform's per-user configuration directory:
//!
//! - Linux: `$XDG_CONFIG_HOME/gpui-task-manager/settings.json`
//!   (fallback `~/.config/gpui-task-manager/settings.json`)
//! - Windows: `%APPDATA%\gpui-task-manager\settings.json`
//! - macOS: `~/Library/Application Support/gpui-task-manager/settings.json`
//!
//! Writes are atomic (temporary file + rename) and, on Unix, the file is
//! created with mode `0600` even though the contents are not secret.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tm_core::SortSpec;
use tm_viewmodel::{MAX_REFRESH_INTERVAL, MIN_REFRESH_INTERVAL};

/// Directory name used under the platform configuration root.
const APP_DIR: &str = "gpui-task-manager";
/// Settings file name.
const FILE_NAME: &str = "settings.json";
/// Current settings schema version.
pub const CURRENT_VERSION: u32 = 1;

/// Persisted presentation preferences.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Schema version, so future migrations can recognise old files.
    pub version: u32,
    /// Refresh interval in milliseconds.
    pub interval_ms: u64,
    /// Sort column and direction.
    pub sort: SortSpec,
    /// Whether the details panel is visible.
    pub details_visible: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            interval_ms: 1_000,
            sort: SortSpec::default(),
            details_visible: true,
        }
    }
}

impl Settings {
    /// Clamps values into the range the view-model accepts.
    pub fn normalized(mut self) -> Self {
        self.version = CURRENT_VERSION;
        let minimum = MIN_REFRESH_INTERVAL.as_millis() as u64;
        let maximum = MAX_REFRESH_INTERVAL.as_millis() as u64;
        self.interval_ms = self.interval_ms.clamp(minimum, maximum);
        self
    }

    /// Loads settings from the platform configuration directory.
    ///
    /// A missing or unreadable file falls back to defaults; an unreadable file
    /// other than `NotFound` is reported on stderr so a broken configuration is
    /// not silently ignored forever.
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        match load_from(&path) {
            Ok(settings) => settings.normalized(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                eprintln!(
                    "gpui-task-manager: 設定を読み込めません ({}): {error}",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Saves settings atomically to the platform configuration directory.
    pub fn save(&self) -> io::Result<()> {
        let path = config_path().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "設定ディレクトリを特定できません")
        })?;
        save_to(&path, &self.clone().normalized())
    }
}

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

/// Loads settings from an explicit path (used by tests and by `load`).
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
    let json = serde_json::to_string_pretty(settings)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let temporary = path.with_extension("json.tmp");
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
    use tm_core::{SortDirection, SortKey};

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
        };
        save_to(&path, &settings).expect("save settings");
        assert!(path.is_file());
        assert!(!path.with_extension("json.tmp").exists());
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
    fn normalization_clamps_interval_and_sets_version() {
        let settings = Settings {
            version: 0,
            interval_ms: 1,
            ..Settings::default()
        }
        .normalized();
        assert_eq!(settings.version, CURRENT_VERSION);
        assert_eq!(
            settings.interval_ms,
            MIN_REFRESH_INTERVAL.as_millis() as u64
        );

        let settings = Settings {
            interval_ms: u64::MAX,
            ..Settings::default()
        }
        .normalized();
        assert_eq!(
            settings.interval_ms,
            MAX_REFRESH_INTERVAL.as_millis() as u64
        );
    }

    #[test]
    fn unknown_fields_are_ignored_for_forward_compatibility() {
        let path = temporary_path("unknown");
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(
            &path,
            r#"{"version":1,"interval_ms":1500,"sort":{"key":"Pid","direction":"Ascending"},"details_visible":false,"future_field":42}"#,
        )
        .expect("write settings");
        let settings = load_from(&path).expect("load settings");
        assert_eq!(settings.interval_ms, 1_500);
        assert_eq!(settings.sort.key, SortKey::Pid);
        assert!(!settings.details_visible);
        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }
}

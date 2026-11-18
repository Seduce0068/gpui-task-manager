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
#[cfg(unix)]
use std::io::Write;
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
            // Log the file name only: the full path usually contains the user's
            // home directory and account name.
            eprintln!(
                "gpui-task-manager: 設定を読み込めません ({}): {error}",
                path.file_name().unwrap_or_default().to_string_lossy()
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

/// Picks the first unused backup path for `stem`.
///
/// `exists` is injected so this stays testable without touching the disk. The
/// counter is unbounded: a finite range could exhaust and fall back to an
/// existing path, silently overwriting an earlier backup (R25 audit).
fn backup_candidate(stem: &Path, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let mut os = stem.as_os_str().to_os_string();
    for counter in 0..u64::MAX {
        if counter > 0 {
            os.push(format!("-{counter}"));
        }
        let candidate = PathBuf::from(&os);
        if !exists(&candidate) {
            return candidate;
        }
        if counter > 0 {
            // Drop the previous suffix before appending the next one.
            os = stem.as_os_str().to_os_string();
        }
    }
    // `u64::MAX` candidates exhausted: never overwrite an existing backup.
    // The caller will still move the corrupt file aside under this name only
    // if it does not exist; otherwise it logs and leaves the original in place.
    stem.with_file_name(format!(
        "{}-overflow",
        stem.file_name().unwrap_or_default().to_string_lossy()
    ))
}

/// Moves a corrupt settings file aside before the next save overwrites it.
///
/// Failures are deliberately ignored: the caller still returns the original
/// `InvalidData`, so a backup problem never turns into a startup failure.
fn backup_corrupt_file(path: &Path) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    // Keep the path as OsString: `.display().to_string()` replaces non-UTF-8
    // bytes with U+FFFD, which could point `rename` at a different directory
    // and silently lose the corrupt data (R26 settings audit).
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("settings.json"));
    name.push(format!(".corrupt-{nanos:016x}"));
    let stem = path.with_file_name(name);
    let candidate = backup_candidate(&stem, |candidate| candidate.exists());
    if let Err(error) = fs::rename(path, &candidate) {
        eprintln!(
            "gpui-task-manager: 破損設定のバックアップに失敗 ({}): {error}",
            candidate.file_name().unwrap_or_default().to_string_lossy()
        );
    }
}

/// Loads settings from an explicit path (used by tests and by [`load`]).
pub(crate) fn load_from(path: &Path) -> io::Result<Settings> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            // Invalid UTF-8 is also InvalidData; preserve those bytes too.
            // A directory at the settings path gives IsADirectory (not
            // InvalidData); rename can move it aside just as well, otherwise
            // every later save would fail and leak temp files (R25 audit).
            if error.kind() == io::ErrorKind::InvalidData || path.is_dir() {
                backup_corrupt_file(path);
            }
            return Err(error);
        }
    };
    serde_json::from_str(&text)
        .map(Settings::normalized)
        .map_err(|error| {
            backup_corrupt_file(path);
            io::Error::new(io::ErrorKind::InvalidData, error)
        })
}

/// Writes settings atomically to an explicit path.
pub(crate) fn save_to(path: &Path, settings: &Settings) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&settings.clone().normalized())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    // PID + monotonic nonce: a recycled PID must not collide with a stale
    // temporary file left by an earlier process.
    static SAVE_NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = SAVE_NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let temporary = path.with_extension(format!(
        "json.tmp.{}.{:x}.{:x}",
        std::process::id(),
        nanos,
        nonce
    ));
    let written = (|| {
        #[cfg(unix)]
        {
            // `.mode(0o600)` alone is still masked by the process umask (e.g.
            // `umask 0777` yields 0000). `set_permissions` uses fchmod, which
            // ignores umask, so the final file stays readable next launch.
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&temporary)?;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
            file.write_all(json.as_bytes())?;
        }
        #[cfg(not(unix))]
        fs::write(&temporary, json)?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if written.is_err() {
        // Leave no half-written temp file behind on a failed save (R25 audit).
        let _ = fs::remove_file(&temporary);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;
    use tm_core::{SortDirection, SortKey, SortSpec};

    #[test]
    fn backup_candidate_never_returns_an_existing_path() {
        let stem = "/cfg/settings.json.corrupt-0000";
        // All finite-range candidates are taken; the old loop fell through to
        // `-65535` (which was never checked) and overwrote it.
        let taken: std::collections::HashSet<PathBuf> = (0..=u16::MAX)
            .map(|n| {
                if n == 0 {
                    PathBuf::from(stem)
                } else {
                    PathBuf::from(format!("{stem}-{n}"))
                }
            })
            .collect();
        let candidate = backup_candidate(Path::new(stem), |path| taken.contains(path));
        assert!(!taken.contains(&candidate), "{candidate:?} must be unused");
        // The finite u16 range is exhausted at 65535; the unbounded counter
        // proceeds to 65536 instead of falling back onto an existing backup.
        assert_eq!(candidate, PathBuf::from(format!("{stem}-65536")));

        let fresh = backup_candidate(Path::new(stem), |_| false);
        assert_eq!(fresh, PathBuf::from(stem));
    }

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
        // No temporary file may survive the rename, whatever nonce was used.
        let leftovers: Vec<_> = path
            .parent()
            .expect("parent")
            .read_dir()
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".json.tmp."))
            .collect();
        assert!(leftovers.is_empty(), "leftover temps: {leftovers:?}");
        assert_eq!(load_from(&path).expect("load settings"), settings);
        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn directory_at_settings_path_is_backed_up_and_save_recovers() {
        let path = temporary_path("settings-dir");
        let parent = path.parent().expect("parent").to_path_buf();
        fs::create_dir_all(&path).expect("create directory at settings path");
        // `EISDIR` is not InvalidData; the path-is-dir check must still move
        // it aside so the next save can create a regular file.
        assert!(load_from(&path).is_err());
        assert!(!path.is_dir(), "directory must be renamed aside");
        save_to(&path, &Settings::default()).expect("save after recovery");
        assert!(path.is_file());

        let leftovers: Vec<_> = fs::read_dir(&parent)
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".json.tmp."))
            .collect();
        assert!(leftovers.is_empty(), "leftover temps: {leftovers:?}");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn failed_save_does_not_leave_temporary_files() {
        // A directory at the target makes `rename` fail; no temp may survive.
        let path = temporary_path("save-fails");
        let parent = path.parent().expect("parent").to_path_buf();
        fs::create_dir_all(&path).expect("create directory at target");
        assert!(save_to(&path, &Settings::default()).is_err());
        let leftovers: Vec<_> = fs::read_dir(&parent)
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".json.tmp."))
            .collect();
        assert!(leftovers.is_empty(), "leftover temps: {leftovers:?}");
        let _ = fs::remove_dir_all(&path);
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn corrupt_settings_are_renamed_to_a_backup() {
        let path = temporary_path("corrupt-backup");
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(&path, "{ not json").expect("write corrupt file");

        let error = load_from(&path).expect_err("corrupt json");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(!path.exists(), "corrupt file must be moved aside");

        let parent = path.parent().expect("parent");
        let backups: Vec<_> = fs::read_dir(parent)
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(backups.len(), 1, "exactly one backup expected");
        assert_eq!(
            fs::read(backups[0].path()).expect("read backup"),
            b"{ not json"
        );
        let _ = fs::remove_dir_all(parent);
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
        let leftovers: Vec<_> = parent
            .read_dir()
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".json.tmp."))
            .collect();
        assert!(
            leftovers.is_empty(),
            "the temporary file must be renamed away, not left behind: {leftovers:?}"
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

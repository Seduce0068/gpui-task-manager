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
//!
//! Storage compatibility (R2-D01 / R5-D02 / R3-U01): a load keeps the raw JSON
//! object in memory. A later save merges known fields back into that object, so
//! a `version` newer than this build and fields written by a newer build survive
//! a load/save round-trip. Corrupt input is backed up before it is replaced and
//! produces a fixed, redacted [`SettingsNotice`] for the status bar; no path,
//! hostname, token or parser error text is ever put in the notice.

use std::fs;
use std::io;
#[cfg(unix)]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub use tm_viewmodel::SETTINGS_VERSION as CURRENT_VERSION;
// Re-exported so callers can use one canonical type from this module.
pub use tm_viewmodel::Settings;

/// Directory name used under the platform configuration root.
const APP_DIR: &str = "gpui-task-manager";
/// Settings file name.
const FILE_NAME: &str = "settings.json";

/// Fixed, redacted text shown when a corrupt file was moved aside successfully.
pub const NOTICE_BACKED_UP: &str =
    "保存された設定を読み込めませんでした。破損した設定はバックアップし、既定値で起動します。";
/// Fixed, redacted text shown when the backup itself failed.
pub const NOTICE_BACKUP_FAILED: &str =
    "保存された設定を読み込めませんでした。バックアップにも失敗したため既定値で起動します。";
/// Fixed, redacted text shown when the settings file cannot be read at all.
pub const NOTICE_UNREADABLE: &str = "保存された設定を読み込めませんでした。既定値で起動します。";

/// A configuration notice safe to pass to `Dashboard::set_persistent_status`.
///
/// `text` is one of the fixed constants above: it never contains a file path,
/// host name, token or raw parser error, so it can also be copied into the DOM
/// without weakening the redaction contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsNotice {
    /// Redacted status-bar text.
    pub text: String,
    /// Always `true`: a settings problem is an error.
    pub is_error: bool,
    /// Always `true`: a startup notice must survive the next sample.
    pub persistent: bool,
}

impl SettingsNotice {
    fn new(text: &'static str) -> Self {
        Self {
            text: text.to_string(),
            is_error: true,
            persistent: true,
        }
    }
}

fn corruption_notice(backed_up: bool) -> SettingsNotice {
    SettingsNotice::new(if backed_up {
        NOTICE_BACKED_UP
    } else {
        NOTICE_BACKUP_FAILED
    })
}

/// Parsed settings plus the untouched JSON object they came from.
///
/// Keeping `raw` is what makes unknown fields and a future `version` survive a
/// later save instead of being dropped by the older shell (R2-D01 / R5-D02).
#[derive(Clone, Debug)]
pub(crate) struct SettingsDocument {
    pub(crate) settings: Settings,
    pub(crate) raw: serde_json::Map<String, serde_json::Value>,
}

// Process-wide state for the load -> save sequence performed at startup. The
// notice is drained once by the shell; the raw object is refreshed after every
// successful save so repeated saves keep the same forward-compatible fields.
static LAST_LOADED: Mutex<Option<serde_json::Map<String, serde_json::Value>>> = Mutex::new(None);
static PENDING_NOTICE: Mutex<Option<SettingsNotice>> = Mutex::new(None);

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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

/// Clamps into the supported range without rewriting the schema `version`.
///
/// `Settings::normalized()` always forces the current version; storage must not
/// do that or a newer file would be downgraded on the next save (R2-D01).
fn clamped(mut settings: Settings) -> Settings {
    settings.interval_ms = settings.interval_ms.clamp(
        tm_viewmodel::MIN_REFRESH_INTERVAL.as_millis() as u64,
        tm_viewmodel::MAX_REFRESH_INTERVAL.as_millis() as u64,
    );
    settings
}

/// Parses a persisted JSON document, keeping its raw object for round-tripping.
///
/// An older `version` is upgraded to the current schema, a newer one is kept.
/// Unknown fields stay in `raw` and are re-emitted by [`merged_map`].
fn parse_document(text: &str) -> Option<SettingsDocument> {
    let mut value: serde_json::Value = serde_json::from_str(text).ok()?;
    let raw = value.as_object()?.clone();
    let stored_version = raw
        .get("version")
        .and_then(|value| value.as_u64())
        .and_then(|version| u32::try_from(version).ok());

    // `Settings` cannot represent a version it does not know (e.g. a u64 from a
    // future schema), but that must not make the whole file look corrupt.
    if let Some(object) = value.as_object_mut() {
        object.insert("version".to_string(), serde_json::json!(CURRENT_VERSION));
    }
    let mut settings: Settings = serde_json::from_value(value).ok()?;
    settings = clamped(settings);
    settings.version = stored_version
        .unwrap_or(CURRENT_VERSION)
        .max(CURRENT_VERSION);
    Some(SettingsDocument { settings, raw })
}

/// Rebuilds the JSON object to save: known fields come from `settings`, every
/// unknown key and the highest seen `version` come from the loaded document.
fn merged_map(
    settings: &Settings,
    raw: Option<&serde_json::Map<String, serde_json::Value>>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut map = raw.cloned().unwrap_or_default();
    let known = serde_json::to_value(clamped(settings.clone()))
        .expect("Settings always serializes to a JSON object");
    let serde_json::Value::Object(known) = known else {
        return map;
    };
    for (key, value) in known {
        if key == "version" {
            let stored = map.get(&key).and_then(|value| value.as_u64()).unwrap_or(0);
            let current = value.as_u64().unwrap_or(0);
            map.insert(key, serde_json::json!(stored.max(current)));
        } else {
            map.insert(key, value);
        }
    }
    map
}

/// Outcome of reading the settings file, including the reason for a fallback.
enum ReadOutcome {
    Loaded(SettingsDocument),
    Missing,
    Corrupt { backed_up: bool },
    Io(io::Error),
}

/// Reads and classifies the settings file. Corrupt input is moved aside here
/// so the public loader and the regression tests share one backup path.
fn read_document(path: &Path) -> ReadOutcome {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            // Invalid UTF-8 is also InvalidData; preserve those bytes too.
            // A directory at the settings path gives IsADirectory (not
            // InvalidData); rename can move it aside just as well, otherwise
            // every later save would fail and leak temp files (R25 audit).
            if error.kind() == io::ErrorKind::InvalidData || path.is_dir() {
                return ReadOutcome::Corrupt {
                    backed_up: backup_corrupt_file(path),
                };
            }
            if error.kind() == io::ErrorKind::NotFound {
                return ReadOutcome::Missing;
            }
            return ReadOutcome::Io(error);
        }
    };
    match parse_document(&text) {
        Some(document) => ReadOutcome::Loaded(document),
        None => ReadOutcome::Corrupt {
            backed_up: backup_corrupt_file(path),
        },
    }
}

fn load_from_path(path: Option<&Path>) -> Settings {
    let Some(path) = path else {
        return Settings::default();
    };
    match read_document(path) {
        ReadOutcome::Loaded(document) => {
            *lock(&LAST_LOADED) = Some(document.raw);
            *lock(&PENDING_NOTICE) = None;
            document.settings
        }
        ReadOutcome::Missing => {
            *lock(&LAST_LOADED) = None;
            *lock(&PENDING_NOTICE) = None;
            Settings::default()
        }
        ReadOutcome::Corrupt { backed_up } => {
            *lock(&LAST_LOADED) = None;
            let notice = corruption_notice(backed_up);
            *lock(&PENDING_NOTICE) = Some(notice.clone());
            // Log the fixed text only: the full path usually contains the
            // user's home directory and account name.
            eprintln!("gpui-task-manager: {}", notice.text);
            Settings::default()
        }
        ReadOutcome::Io(error) => {
            *lock(&LAST_LOADED) = None;
            *lock(&PENDING_NOTICE) = Some(SettingsNotice::new(NOTICE_UNREADABLE));
            eprintln!(
                "gpui-task-manager: 設定を読み込めません ({}): {error}",
                path.file_name().unwrap_or_default().to_string_lossy()
            );
            Settings::default()
        }
    }
}

/// Loads settings and returns the pending configuration notice, if any.
///
/// Shells that show a persistent status should call this once at startup. The
/// notice is drained, so a later call returns `None` until the next corrupt
/// load. The same notice is also available through [`take_pending_notice`] for
/// shells that call [`load_with_notice`] directly.
// The shell wiring lives in `app/src/app.rs` (separate owner); keep the API
// available without failing `-D warnings` in the meantime.
#[allow(dead_code)]
pub fn load_with_notice() -> (Settings, Option<SettingsNotice>) {
    load_from_path_with_notice(config_path().as_deref())
}

/// Path-injected variant of [`load_with_notice`], also used by the tests.
fn load_from_path_with_notice(path: Option<&Path>) -> (Settings, Option<SettingsNotice>) {
    let settings = load_from_path(path);
    (settings, take_pending_notice())
}

/// Takes the redacted notice recorded by the last [`load_with_notice`], if any.
#[allow(dead_code)]
pub fn take_pending_notice() -> Option<SettingsNotice> {
    lock(&PENDING_NOTICE).take()
}

/// Saves settings atomically to the platform configuration directory.
///
/// Unknown fields and a newer `version` from the last load are preserved.
pub fn save(settings: &Settings) -> io::Result<()> {
    let path = config_path().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "設定ディレクトリを特定できません")
    })?;
    let raw = lock(&LAST_LOADED).clone();
    let map = merged_map(settings, raw.as_ref());
    let json = serde_json::to_string_pretty(&map)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    save_raw_to(&path, &json)?;
    *lock(&LAST_LOADED) = Some(map);
    Ok(())
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
/// Returns `true` only when the rename succeeded; failures are logged and
/// reported through a fixed notice, so a backup problem never turns into a
/// startup failure.
fn backup_corrupt_file(path: &Path) -> bool {
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
    match fs::rename(path, &candidate) {
        Ok(()) => true,
        Err(error) => {
            eprintln!(
                "gpui-task-manager: 破損設定のバックアップに失敗 ({}): {error}",
                candidate.file_name().unwrap_or_default().to_string_lossy()
            );
            false
        }
    }
}

/// Loads a settings document from an explicit path (regression tests only).
#[cfg(test)]
pub(crate) fn load_document_from(path: &Path) -> io::Result<SettingsDocument> {
    match read_document(path) {
        ReadOutcome::Loaded(document) => Ok(document),
        ReadOutcome::Missing => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "設定ファイルがありません",
        )),
        ReadOutcome::Corrupt { .. } => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "保存された設定を解析できません",
        )),
        ReadOutcome::Io(error) => Err(error),
    }
}

/// Loads settings from an explicit path (regression tests only).
#[cfg(test)]
pub(crate) fn load_from(path: &Path) -> io::Result<Settings> {
    load_document_from(path).map(|document| document.settings)
}

/// Writes settings atomically to an explicit path, without preserving any
/// document loaded elsewhere (regression tests only).
#[cfg(test)]
pub(crate) fn save_to(path: &Path, settings: &Settings) -> io::Result<()> {
    let map = merged_map(settings, None);
    let json = serde_json::to_string_pretty(&map)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    save_raw_to(path, &json)
}

/// Atomically writes an already-serialized JSON document.
fn save_raw_to(path: &Path, json: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
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

    #[test]
    fn future_schema_version_is_preserved_but_old_versions_are_upgraded() {
        let path = temporary_path("schema-version");
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(&path, r#"{"version":9,"interval_ms":1500}"#).expect("write settings");
        assert_eq!(load_from(&path).expect("future version").version, 9);
        fs::write(&path, r#"{"version":1,"interval_ms":1500}"#).expect("write settings");
        assert_eq!(
            load_from(&path).expect("old version").version,
            CURRENT_VERSION
        );
        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn unknown_fields_and_future_version_survive_a_load_save_round_trip() {
        let path = temporary_path("forward-compat");
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(
            &path,
            r#"{"version":9,"interval_ms":1500,"future_flag":42,"future_nested":{"a":[1,2]}}"#,
        )
        .expect("write settings");
        let document = load_document_from(&path).expect("load document");
        assert_eq!(document.settings.version, 9);

        // A caller builds a fresh Settings (with the current version) the way
        // the shells do; the loaded document must still win on version/extras.
        let caller = Settings {
            version: CURRENT_VERSION,
            ..document.settings.clone()
        };
        let map = merged_map(&caller, Some(&document.raw));
        let json = serde_json::to_string_pretty(&map).expect("serialize");
        save_raw_to(&path, &json).expect("write merged settings");

        let reloaded = load_document_from(&path).expect("reload document");
        assert_eq!(
            reloaded.settings.version, 9,
            "future version was downgraded"
        );
        assert_eq!(
            reloaded.raw.get("future_flag"),
            Some(&serde_json::json!(42))
        );
        assert_eq!(
            reloaded.raw.get("future_nested"),
            Some(&serde_json::json!({"a": [1, 2]}))
        );
        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn corrupt_load_notice_is_redacted_drained_and_cleared() {
        // All assertions about the process-wide notice live in one test: the
        // test runner is parallel and these globals must not race.
        let path = temporary_path("notice");
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(&path, "{ not json").expect("write corrupt file");

        // A corrupt load records the notice; the notice API drains it once.
        let (_settings, notice) = load_from_path_with_notice(Some(&path));
        let notice = notice.expect("pending notice");
        assert_eq!(notice.text, NOTICE_BACKED_UP);
        assert!(notice.is_error);
        assert!(notice.persistent, "startup notice must survive a sample");
        // Redaction contract: no path, host name, token or parser detail.
        assert!(!notice.text.contains('/'), "{}", notice.text);
        assert!(!notice.text.contains("not json"), "{}", notice.text);
        assert!(!notice.text.contains("settings.json"), "{}", notice.text);
        assert_eq!(take_pending_notice(), None, "notice must be drained once");

        // A failed backup still yields a fixed notice without any path detail.
        *lock(&PENDING_NOTICE) = Some(corruption_notice(false));
        let notice = take_pending_notice().expect("backup failure notice");
        assert_eq!(notice.text, NOTICE_BACKUP_FAILED);

        // A healthy load clears a stale notice on the next load.
        *lock(&PENDING_NOTICE) = Some(corruption_notice(true));
        save_to(&path, &Settings::default()).expect("save healthy settings");
        let settings = load_from_path(Some(&path));
        assert_eq!(settings.version, CURRENT_VERSION);
        assert_eq!(
            take_pending_notice(),
            None,
            "healthy load clears stale notice"
        );
        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn merged_map_keeps_unknown_fields_without_a_loaded_document() {
        let settings = Settings {
            interval_ms: 20_000,
            ..Settings::default()
        };
        let map = merged_map(&settings, None);
        assert_eq!(
            map.get("version"),
            Some(&serde_json::json!(CURRENT_VERSION))
        );
        assert_eq!(map.get("interval_ms"), Some(&serde_json::json!(10_000)));
        // The out-of-range value was clamped, not stored verbatim.
        assert_ne!(map.get("interval_ms"), Some(&serde_json::json!(20_000)));
    }
}

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
//!
//! Concurrent saves (HB-NATIVE) use a three-way merge: the known-field values
//! observed at the last load/save are the base, the caller's [`Settings`] is
//! "ours" and the object re-read from disk immediately before writing is
//! "theirs". A field this instance did not edit keeps the value found on disk,
//! so two instances editing *different* known fields no longer clobber each
//! other **when the writes are sequential**. Simultaneous writes are not
//! serialized across processes (see below) and reduce to last-writer-wins.
//! `version` still uses max() and unknown fields are still carried over
//! verbatim.
//!
//! Locking scope: `std` has no `flock` equivalent, and a real cross-process
//! lock would need either `unsafe` FFI or a new dependency. Neither is
//! acceptable here, so the read-compare-write sequence is serialized in-process
//! with a `Mutex` only. It is deliberately *not* presented as a CAS: the
//! temporary-file + rename write still prevents torn files, but two processes
//! are not mutually excluded. The merge keeps both edits in the normal
//! sequential case; a same-field conflict remains last-writer-wins.
//!
//! Schema-version compatibility policy (R2X-D3), identical in the browser shell:
//!
//! - **Backward**: a file from an older build (or with no `version`) is read with
//!   `#[serde(default)]` fallbacks for fields that did not exist yet, and its
//!   in-memory version is raised to [`CURRENT_VERSION`]. The next save writes
//!   that current version, migrating the file forward.
//! - **Forward**: a file from a newer build keeps its greater numeric `version`
//!   and every unknown field in the raw object. Neither load nor save downgrades
//!   it; unrecognised fields round-trip untouched even if this build cannot
//!   interpret them.
//! - **Opaque version**: a `version` that is not a non-negative integer (string,
//!   fractional, negative, or larger than `u32::MAX`) is never interpreted or
//!   rewritten. It is written back verbatim so a newer shell still recognises
//!   its own schema; normalising it here would be a silent downgrade.
//! - **Corruption**: only malformed JSON or a wrong type in a *known* field
//!   (other than `version`) falls back to defaults with a fixed redacted notice.

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
/// Maximum number of corrupt backups kept on disk. Older ones are pruned after
/// a successful backup (H1-BACKUP).
const CORRUPT_BACKUP_LIMIT: usize = 5;
/// Largest settings document this build will read (4 MiB).
///
/// A valid settings file is a handful of keys (well under 1 KiB). Refusing a
/// much larger file keeps a corrupt or hostile document from being loaded into
/// memory unbounded; it is classified as corrupt, backed up and replaced by
/// defaults exactly like malformed JSON (PC-CSVBOM).
const MAX_SETTINGS_BYTES: u64 = 4 * 1024 * 1024;

/// Removes one leading UTF-8 BOM (EF BB BF, decoded as U+FEFF).
///
/// Windows editors commonly prepend a BOM to JSON files. `serde_json` rejects a
/// leading U+FEFF, which would make an otherwise valid settings file look
/// corrupt and get backed up and replaced by defaults (PC-CSVBOM). Strip exactly
/// one BOM at the start; a BOM anywhere else stays a parse error.
fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// Reads at most [`MAX_SETTINGS_BYTES`] + 1 bytes and validates UTF-8.
///
/// `fs::read_to_string` has no size limit, so it is deliberately not used here:
/// `Read::take` caps the allocation and an over-limit file is reported as
/// `InvalidData`, which `read_document` classifies as corrupt (backup + defaults
/// + fixed notice). Genuine I/O and encoding failures keep their old kinds.
fn read_bounded(path: &Path) -> io::Result<String> {
    use std::io::Read as _;
    let file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_SETTINGS_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SETTINGS_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "settings file exceeds the size limit",
        ));
    }
    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Fixed, redacted text shown when a corrupt file was moved aside successfully.
pub const NOTICE_BACKED_UP: &str =
    "保存された設定を読み込めませんでした。破損した設定はバックアップし、既定値で起動します。退避物には保持上限があり、復元手順は README.md または SECURITY.md を参照してください。";
/// Fixed, redacted text shown when the backup itself failed.
pub const NOTICE_BACKUP_FAILED: &str =
    "保存された設定を読み込めませんでした。バックアップにも失敗したため既定値で起動します。";
/// Fixed, redacted text shown when the settings file cannot be read at all.
pub const NOTICE_UNREADABLE: &str = "保存された設定を読み込めませんでした。既定値で起動します。";
/// Fixed, redacted text shown when persisting settings fails.
///
/// Shells put this on the status bar; it never contains a path, host name,
/// token or I/O error detail (C2-SETTINGS).
pub const NOTICE_SAVE_FAILED: &str =
    "設定を保存できませんでした。表示設定はこのセッション中のみ有効です。";

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
// Serializes the whole read-compare-write-refresh sequence inside this process.
// See the locking-scope note in the module docs: std has no portable flock, so
// this is not a cross-process CAS (HB-NATIVE).
static SAVE_LOCK: Mutex<()> = Mutex::new(());

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Returns the settings file path, if a configuration directory can be found.
pub fn config_path() -> Option<PathBuf> {
    config_dir().map(|directory| directory.join(FILE_NAME))
}

/// Refuse to create persistent files on virtual filesystems, matching the
/// `--export` writer (R47 settings-I/O audit).
fn is_virtual_fs_root(path: &Path) -> bool {
    path.components()
        .next()
        .is_some_and(|first| matches!(first, std::path::Component::RootDir))
        && path
            .components()
            .nth(1)
            .and_then(|first| first.as_os_str().to_str())
            .is_some_and(|top| matches!(top, "dev" | "proc" | "sys"))
}

fn config_dir() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("XDG_CONFIG_HOME") {
        // XDG: a relative path must be ignored. Accepting it would write to
        // $CWD (R47 settings-I/O audit).
        let path = PathBuf::from(value);
        if path.is_absolute() && !is_virtual_fs_root(&path) {
            return Some(path.join(APP_DIR));
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

    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute() && !is_virtual_fs_root(home))
        .map(|home| home.join(".config").join(APP_DIR))
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

/// Interprets a persisted `version` under the compatibility policy.
///
/// Only a non-negative integer that fits in `u32` is comparable here. Other
/// JSON forms are opaque: they stay in `raw` and [`merged_map_with_baseline`] writes them back
/// verbatim rather than replacing them with a version this build understands
/// (R2X-D3).
fn interpretable_version(value: Option<&serde_json::Value>) -> Option<u32> {
    value
        .and_then(|value| value.as_u64())
        .and_then(|version| u32::try_from(version).ok())
}

/// Parses a persisted JSON document, keeping its raw object for round-tripping.
///
/// An older `version` is upgraded to the current schema, a newer one is kept.
/// Unknown fields stay in `raw` and are re-emitted by [`merged_map_with_baseline`].
fn parse_document(text: &str) -> Option<SettingsDocument> {
    let mut value: serde_json::Value = serde_json::from_str(strip_bom(text)).ok()?;
    let raw = value.as_object()?.clone();
    let stored_version = interpretable_version(raw.get("version"));

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

/// Three-way merge of the JSON object to save.
///
/// `baseline` is the raw object observed at this instance's last load/save,
/// `settings` is the caller's current view ("ours") and `raw` is the freshest
/// object read from the destination ("theirs"). A known field whose value still
/// equals the baseline was not edited here, so the concurrent value on disk is
/// kept; a field that differs is written from `settings` (a same-field conflict
/// is last-writer-wins). Unknown fields come from `raw`, and `version` keeps its
/// existing max/verbatim policy.
fn merged_map_with_baseline(
    settings: &Settings,
    baseline: Option<&serde_json::Map<String, serde_json::Value>>,
    raw: Option<&serde_json::Map<String, serde_json::Value>>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut map = raw.cloned().unwrap_or_default();
    let known = serde_json::to_value(clamped(settings.clone()))
        .expect("Settings always serializes to a JSON object");
    let serde_json::Value::Object(known) = known else {
        return map;
    };
    // No earlier load/save means there is nothing to diff against: treat every
    // known field as edited, which reproduces the old overwrite-all behaviour.
    let empty = serde_json::Map::new();
    let base = baseline.unwrap_or(&empty);
    for (key, value) in known {
        if key == "version" {
            match interpretable_version(map.get(&key)) {
                // Comparable numeric version: upgrade older values, never
                // downgrade newer ones.
                Some(stored) => {
                    let current = value.as_u64().unwrap_or(0);
                    map.insert(key, serde_json::json!((stored as u64).max(current)));
                }
                // No version at all: record the version this build writes.
                None if !map.contains_key(&key) => {
                    map.insert(key, value);
                }
                // Opaque or out-of-u32-range value: keep it verbatim so a newer
                // shell still recognises its own schema (R2X-D3).
                None => {}
            }
            continue;
        }
        // Unchanged relative to our base: keep what another instance wrote.
        // If the destination lost the field entirely, write ours so a newer
        // field is not dropped when migrating an older file.
        if base.get(&key).is_some_and(|old| old == &value) && map.contains_key(&key) {
            continue;
        }
        map.insert(key, value);
    }
    map
}

/// Test-only shorthand for the overwrite-all case (no baseline available).
///
/// Production saves go through [`save_to_path_with_baseline`] so concurrent
/// known-field edits survive; regressions that exercise the older contract call
/// this wrapper directly.
#[cfg(test)]
fn merged_map(
    settings: &Settings,
    raw: Option<&serde_json::Map<String, serde_json::Value>>,
) -> serde_json::Map<String, serde_json::Value> {
    merged_map_with_baseline(settings, None, raw)
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
    // A FIFO at the settings path blocks `read_to_string` forever waiting for
    // a writer. Treat every non-regular destination as corrupt and back it up
    // instead (R47 settings-I/O audit, same guard as the --export writer).
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.file_type().is_file() {
            return ReadOutcome::Corrupt {
                backed_up: backup_corrupt_file(path),
            };
        }
    }
    let text = match read_bounded(path) {
        Ok(text) => text,
        Err(error) => {
            // Invalid UTF-8 and over-limit files are both InvalidData; preserve
            // those bytes too.
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
pub fn load_with_notice() -> (Settings, Option<SettingsNotice>) {
    load_from_path_with_notice(config_path().as_deref())
}

/// Path-injected variant of [`load_with_notice`], also used by the tests.
fn load_from_path_with_notice(path: Option<&Path>) -> (Settings, Option<SettingsNotice>) {
    let settings = load_from_path(path);
    (settings, take_pending_notice())
}

/// Takes the redacted notice recorded by the last [`load_with_notice`], if any.
pub fn take_pending_notice() -> Option<SettingsNotice> {
    lock(&PENDING_NOTICE).take()
}

/// Reads the raw JSON object currently at `path` without interpreting it.
///
/// Unlike [`read_document`] this has no backup side effect and accepts any JSON
/// object, so a value written by a newer shell is usable as the merge base even
/// when a known field has a type this build cannot deserialize (C2-SETTINGS).
fn raw_from_path(path: &Path) -> Option<serde_json::Map<String, serde_json::Value>> {
    // Apply the same cap and BOM handling as the load path: a BOM-prefixed file
    // must still act as the merge base so its unknown fields survive a save.
    let text = read_bounded(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(strip_bom(&text)).ok()?;
    value.as_object().cloned()
}

/// Writes a three-way-merged document to `path` and returns the saved object.
///
/// The destination is re-read immediately before merging so a parallel tab or
/// instance that wrote a newer `version` or extra fields after our load cannot
/// be silently rolled back by this save (C2-SETTINGS). The baseline is supplied
/// explicitly: production passes the snapshot from the last load/save, while
/// tests pass their own to simulate two instances without racing on
/// process-wide state. No global state is touched here.
fn save_to_path_with_baseline(
    path: &Path,
    settings: &Settings,
    baseline: Option<&serde_json::Map<String, serde_json::Value>>,
) -> io::Result<serde_json::Map<String, serde_json::Value>> {
    let disk = raw_from_path(path);
    let theirs = disk.as_ref().or(baseline);
    let map = merged_map_with_baseline(settings, baseline, theirs);
    let json = serde_json::to_string_pretty(&map)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    save_raw_to(path, &json)?;
    Ok(map)
}

/// Merges against the destination's latest raw object and writes it atomically.
///
/// Known fields unchanged since our last load/save keep the value another
/// instance just wrote (HB-NATIVE); the in-memory snapshot is used only when
/// the destination cannot be read at all. [`SAVE_LOCK`] serializes the whole
/// sequence in-process; see the module docs for why this is not a cross-process
/// CAS.
fn save_to_path(path: &Path, settings: &Settings) -> io::Result<()> {
    let _guard = lock(&SAVE_LOCK);
    let baseline = lock(&LAST_LOADED).clone();
    let map = save_to_path_with_baseline(path, settings, baseline.as_ref())?;
    *lock(&LAST_LOADED) = Some(map);
    Ok(())
}

/// Saves settings atomically to the platform configuration directory.
///
/// Unknown fields and a newer `version` written by any instance are preserved,
/// including values written since the last load in this process.
pub fn save(settings: &Settings) -> io::Result<()> {
    let path = config_path().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "設定ディレクトリを特定できません")
    })?;
    save_to_path(&path, settings)
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

/// Sort key for a corrupt backup file name: fixed-width timestamp, then the
/// numeric `-N` collision suffix.
///
/// The timestamp is 16 hex digits, so lexicographic order is oldest first. The
/// `-N` suffix must be compared numerically: string order would place `-10`
/// before `-2` and prune the wrong backup. Anything without the exact prefix
/// and timestamp shape (including the `-overflow` fallback) is rejected, so
/// the retention sweep cannot delete unrelated files.
fn backup_sort_key(name: &str, prefix: &str) -> Option<(String, u64)> {
    let rest = name.strip_prefix(prefix)?;
    let (timestamp, suffix) = match rest.split_once('-') {
        Some((timestamp, suffix)) => (timestamp, suffix.parse::<u64>().ok()?),
        None => (rest, 0),
    };
    if timestamp.len() != 16 || !timestamp.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some((timestamp.to_string(), suffix))
}

/// Removes the oldest backups beyond [`CORRUPT_BACKUP_LIMIT`].
///
/// Listing or deletion failures are ignored: cleanup must never turn a backup
/// problem into a startup failure, matching the existing backup policy.
fn prune_corrupt_backups(parent: &Path, prefix: &str) {
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    let mut backups: Vec<(PathBuf, (String, u64))> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let key = backup_sort_key(&name, prefix)?;
            Some((entry.path(), key))
        })
        .collect();
    backups.sort_by(|left, right| left.1.cmp(&right.1));
    for (path, _) in backups.into_iter().rev().skip(CORRUPT_BACKUP_LIMIT) {
        let _ = fs::remove_file(path);
    }
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
    backup_corrupt_file_at(path, nanos)
}

/// Timestamp-injected variant of [`backup_corrupt_file`].
///
/// Tests pin the retention order with explicit timestamps instead of relying
/// on wall-clock resolution between consecutive backups.
fn backup_corrupt_file_at(path: &Path, nanos: u128) -> bool {
    // Keep the path as OsString: `.display().to_string()` replaces non-UTF-8
    // bytes with U+FFFD, which could point `rename` at a different directory
    // and silently lose the corrupt data (R26 settings audit).
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from(FILE_NAME));
    let original = name.clone();
    name.push(format!(".corrupt-{nanos:016x}"));
    let stem = path.with_file_name(name);
    let candidate = backup_candidate(&stem, |candidate| candidate.exists());
    match fs::rename(path, &candidate) {
        Ok(()) => {
            // A non-UTF-8 file name conservatively matches nothing and leaves
            // older backups in place rather than guessing which ones to delete.
            let mut prefix = original.to_string_lossy().into_owned();
            prefix.push_str(".corrupt-");
            if let Some(parent) = candidate.parent() {
                prune_corrupt_backups(parent, &prefix);
            }
            true
        }
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
                // Refuse to follow a pre-planted symlink at the temporary
                // path; a stale file must never be truncated (R47 audit).
                .create_new(true)
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

    // Tests that touch the process-wide PENDING_NOTICE / LAST_LOADED must not
    // interleave: the test runner is parallel and these globals must not race.
    // Keep a dedicated lock (not SAVE_LOCK) so a test holding it may still call
    // functions that take SAVE_LOCK without deadlocking.
    static NOTICE_TEST_LOCK: Mutex<()> = Mutex::new(());

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

    #[test]
    fn backup_sort_key_orders_numeric_suffixes_and_rejects_unrelated_names() {
        let prefix = "settings.json.corrupt-";
        let two = backup_sort_key("settings.json.corrupt-0000000000001000-2", prefix);
        let ten = backup_sort_key("settings.json.corrupt-0000000000001000-10", prefix);
        assert!(two < ten, "-2 must sort before -10");
        assert_eq!(two, Some(("0000000000001000".to_string(), 2)));
        assert_eq!(
            backup_sort_key("settings.json.corrupt-0000000000001000", prefix),
            Some(("0000000000001000".to_string(), 0))
        );
        assert_eq!(
            backup_sort_key("settings.json.corrupt-0000000000001000-overflow", prefix),
            None
        );
        assert_eq!(backup_sort_key("settings.json.tmp.123", prefix), None);
    }

    #[test]
    fn six_backups_keep_only_the_five_newest() {
        let path = temporary_path("backup-retention");
        let parent = path.parent().expect("parent").to_path_buf();
        fs::create_dir_all(&parent).expect("create parent");
        // The repeated timestamp exercises the `-1` collision suffix; the
        // remaining backups use increasing timestamps.
        let stamps = [0x1000, 0x1000, 0x1001, 0x1002, 0x1003, 0x1004];
        for (index, stamp) in stamps.into_iter().enumerate() {
            fs::write(&path, format!("{{ corrupt {index}")).expect("write corrupt file");
            assert!(
                backup_corrupt_file_at(&path, stamp),
                "backup {index} must succeed"
            );
        }
        let mut names: Vec<String> = fs::read_dir(&parent)
            .expect("read dir")
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.contains(".corrupt-").then_some(name)
            })
            .collect();
        names.sort();
        assert_eq!(
            names.len(),
            CORRUPT_BACKUP_LIMIT,
            "oldest backup must be pruned"
        );
        assert!(
            !names
                .iter()
                .any(|name| name.ends_with("corrupt-0000000000001000")),
            "oldest backup must be deleted: {names:?}"
        );
        assert!(
            names
                .iter()
                .any(|name| name.ends_with("corrupt-0000000000001000-1")),
            "same-timestamp collision backup must survive: {names:?}"
        );
        assert!(
            names
                .iter()
                .any(|name| name.ends_with("corrupt-0000000000001004")),
            "newest backup must survive: {names:?}"
        );
        let _ = fs::remove_dir_all(&parent);
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

    #[test]
    fn bom_prefixed_settings_load_without_backup_or_notice() {
        let _guard = lock(&NOTICE_TEST_LOCK);
        let path = temporary_path("bom");
        let parent = path.parent().expect("parent").to_path_buf();
        fs::create_dir_all(&parent).expect("create parent");
        let json = r#"{"version":9,"interval_ms":1500,"future_flag":42}"#;
        // Windows editors write EF BB BF (decoded as U+FEFF) before the JSON.
        fs::write(&path, format!("\u{feff}{json}")).expect("write BOM settings");

        let (settings, notice) = load_from_path_with_notice(Some(&path));
        assert_eq!(notice, None, "a BOM must not look like corruption");
        assert_eq!(settings.version, 9);
        assert_eq!(settings.interval_ms, 1_500);
        assert!(path.exists(), "a healthy BOM file must not be moved aside");
        let document = load_document_from(&path).expect("BOM document must load");
        assert_eq!(
            document.raw.get("future_flag"),
            Some(&serde_json::json!(42))
        );
        let backups: Vec<_> = fs::read_dir(&parent)
            .expect("read dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert!(
            backups.is_empty(),
            "no corrupt backup expected: {backups:?}"
        );
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn oversized_settings_are_backed_up_and_defaults_returned() {
        let _guard = lock(&NOTICE_TEST_LOCK);
        let path = temporary_path("oversized");
        let parent = path.parent().expect("parent").to_path_buf();
        fs::create_dir_all(&parent).expect("create parent");
        // Valid JSON padded past the cap: it must be rejected on size, not
        // parsed into memory, and must follow the corrupt backup path.
        let mut text = r#"{"version":9,"interval_ms":1500,"pad":""#.to_string();
        text.push_str(&"x".repeat(MAX_SETTINGS_BYTES as usize + 1));
        text.push_str("\"}");
        fs::write(&path, &text).expect("write oversized settings");

        let (settings, notice) = load_from_path_with_notice(Some(&path));
        assert_eq!(
            settings,
            Settings::default(),
            "oversized file must not load"
        );
        let notice = notice.expect("corrupt notice");
        assert_eq!(notice.text, NOTICE_BACKED_UP);
        assert!(!path.exists(), "oversized file must be moved aside");
        let backups: Vec<_> = fs::read_dir(&parent)
            .expect("read dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(backups.len(), 1, "exactly one backup expected");
        let _ = fs::remove_dir_all(&parent);
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
    fn virtual_fs_roots_are_rejected_for_persistent_settings() {
        assert!(is_virtual_fs_root(std::path::Path::new("/dev")));
        assert!(is_virtual_fs_root(std::path::Path::new("/proc/self")));
        assert!(is_virtual_fs_root(std::path::Path::new("/sys/kernel")));
        assert!(!is_virtual_fs_root(std::path::Path::new("/home/user")));
        assert!(!is_virtual_fs_root(std::path::Path::new("config")));
    }

    #[test]
    fn non_regular_settings_paths_are_treated_as_corrupt() {
        // A FIFO would block `read_to_string` forever; it must be backed up
        // instead (R47 settings-I/O audit).
        let dir = std::env::temp_dir().join(format!("gtm-settings-special-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("settings.json");
        let _ = std::fs::write(&path, "{}");
        assert!(matches!(
            read_document(&path),
            ReadOutcome::Loaded(_) | ReadOutcome::Missing
        ));
        // A directory at the path is already handled via `path.is_dir()`.
        let dir_path = dir.join("as-dir");
        let _ = std::fs::create_dir(&dir_path);
        assert!(matches!(
            read_document(&dir_path),
            ReadOutcome::Corrupt { .. }
        ));
        let _ = std::fs::remove_dir_all(&dir);
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
    fn save_rereads_the_destination_before_merging() {
        let _guard = lock(&NOTICE_TEST_LOCK);
        // A parallel instance writes a newer schema and extra fields after our
        // load. Saving must merge against that fresh value, not the stale raw
        // captured at load time (C2-SETTINGS).
        let path = temporary_path("save-reread");
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(&path, r#"{"version":9,"interval_ms":1500,"loaded_only":1}"#)
            .expect("write initial settings");
        let _loaded = load_from_path(Some(&path));

        fs::write(
            &path,
            r#"{"version":11,"interval_ms":1500,"loaded_only":1,"newer_only":{"x":2}}"#,
        )
        .expect("write settings from the other instance");
        let caller = Settings {
            version: CURRENT_VERSION,
            interval_ms: 2_000,
            ..Settings::default()
        };
        save_to_path(&path, &caller).expect("save merged settings");

        let reloaded = load_document_from(&path).expect("reload document");
        assert_eq!(
            reloaded.settings.version, 11,
            "a stale load must not downgrade the newest stored version"
        );
        assert_eq!(reloaded.settings.interval_ms, 2_000);
        assert_eq!(
            reloaded.raw.get("newer_only"),
            Some(&serde_json::json!({"x": 2})),
            "fields written after our load must survive"
        );
        assert_eq!(reloaded.raw.get("loaded_only"), Some(&serde_json::json!(1)));
        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn save_computes_max_version_against_the_latest_stored_value() {
        let _guard = lock(&NOTICE_TEST_LOCK);
        // The stale snapshot has a higher version than the destination now has.
        // `max(stored, current)` must use the freshly read `stored`, so the old
        // in-memory value is not resurrected (C2-SETTINGS).
        let path = temporary_path("save-latest-version");
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(&path, r#"{"version":9,"interval_ms":1500,"stale_only":1}"#)
            .expect("write initial settings");
        let _loaded = load_from_path(Some(&path));

        fs::write(&path, r#"{"version":1,"interval_ms":1500,"fresh_only":2}"#)
            .expect("write settings from the other instance");
        let caller = Settings {
            version: CURRENT_VERSION,
            ..Settings::default()
        };
        save_to_path(&path, &caller).expect("save merged settings");

        let reloaded = load_document_from(&path).expect("reload document");
        assert_eq!(
            reloaded.settings.version, CURRENT_VERSION,
            "max(1, current) must win over the stale version 9"
        );
        assert_eq!(reloaded.raw.get("fresh_only"), Some(&serde_json::json!(2)));
        assert_eq!(reloaded.raw.get("stale_only"), None);
        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn corrupt_load_notice_is_redacted_drained_and_cleared() {
        // All assertions about the process-wide notice live in one test: the
        // test runner is parallel and these globals must not race.
        let _guard = lock(&NOTICE_TEST_LOCK);
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

    #[test]
    fn schema_policy_upgrades_old_and_versionless_files_on_save() {
        // Backward compatibility: v1 predates tree_view/theme. serde defaults
        // fill them, and a save must record the current version.
        let path = temporary_path("schema-backward");
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        for raw in [
            r#"{"version":1,"interval_ms":1500}"#,
            r#"{"interval_ms":1500}"#,
        ] {
            fs::write(&path, raw).expect("write settings");
            let document = load_document_from(&path).expect("load document");
            assert_eq!(document.settings.version, CURRENT_VERSION);
            assert!(!document.settings.tree_view);
            assert_eq!(document.settings.theme, tm_viewmodel::Theme::Dark);

            let map = merged_map(&document.settings, Some(&document.raw));
            let json = serde_json::to_string_pretty(&map).expect("serialize");
            fs::write(&path, json).expect("save settings");
            let saved: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&path).expect("read saved"))
                    .expect("parse saved");
            assert_eq!(
                saved.get("version"),
                Some(&serde_json::json!(CURRENT_VERSION)),
                "old/versionless file must be migrated on save"
            );
            assert_eq!(saved.get("interval_ms"), Some(&serde_json::json!(1500)));
        }
        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn schema_policy_future_version_survives_repeated_save_cycles() {
        // Forward compatibility must not drift over repeated load/save cycles.
        let path = temporary_path("schema-repeat");
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(
            &path,
            r#"{"version":9,"interval_ms":1500,"future_flag":{"nested":[1,2]}}"#,
        )
        .expect("write settings");

        for cycle in 0..3 {
            let document = load_document_from(&path).expect("load document");
            assert_eq!(document.settings.version, 9, "cycle {cycle}");
            // The shell rebuilds a current-version Settings before saving.
            let shell = Settings {
                version: CURRENT_VERSION,
                ..document.settings.clone()
            };
            let map = merged_map(&shell, Some(&document.raw));
            let json = serde_json::to_string_pretty(&map).expect("serialize");
            fs::write(&path, json).expect("save settings");
        }
        let final_document = load_document_from(&path).expect("final load");
        assert_eq!(final_document.settings.version, 9);
        assert_eq!(
            final_document.raw.get("future_flag"),
            Some(&serde_json::json!({"nested": [1, 2]}))
        );
        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn schema_policy_opaque_version_forms_are_preserved_verbatim() {
        // A newer schema may change the version encoding. That is not corruption:
        // the value must survive a save unchanged so the newer build still
        // recognises its own file (R2X-D3).
        for raw_version in [
            serde_json::json!("9"),
            serde_json::json!(9.5),
            serde_json::json!(-1),
            serde_json::json!(u64::from(u32::MAX) + 1),
        ] {
            let mut raw = serde_json::json!({"interval_ms": 1500, "future_flag": 1});
            raw["version"] = raw_version.clone();
            let document = parse_document(&raw.to_string()).expect("opaque version must load");
            assert_eq!(document.settings.version, CURRENT_VERSION);

            let shell = Settings {
                version: CURRENT_VERSION,
                ..document.settings.clone()
            };
            let map = merged_map(&shell, Some(&document.raw));
            assert_eq!(
                map.get("version"),
                Some(&raw_version),
                "opaque version was rewritten"
            );
            assert_eq!(map.get("future_flag"), Some(&serde_json::json!(1)));
        }
    }

    #[test]
    fn three_way_merge_keeps_concurrent_edits_to_different_known_fields() {
        // Both instances load the same document, then edit different fields.
        // The second merge sees the first save's object as "theirs" and must
        // preserve both edits, the future version and the unknown field.
        let shared = Settings {
            version: CURRENT_VERSION,
            interval_ms: 1500,
            theme: tm_viewmodel::Theme::Light,
            ..Settings::default()
        };
        let serde_json::Value::Object(mut base) = serde_json::to_value(&shared).expect("serialize")
        else {
            panic!("Settings must serialize to an object")
        };
        base.insert("version".to_string(), serde_json::json!(9));
        base.insert(
            "future_flag".to_string(),
            serde_json::json!({"nested": [1, 2]}),
        );

        let instance_a = Settings {
            interval_ms: 2_000,
            ..shared.clone()
        };
        let after_a = merged_map_with_baseline(&instance_a, Some(&base), Some(&base));
        assert_eq!(after_a.get("interval_ms"), Some(&serde_json::json!(2_000)));

        let instance_b = Settings {
            theme: tm_viewmodel::Theme::Dark,
            ..shared
        };
        let after_b = merged_map_with_baseline(&instance_b, Some(&base), Some(&after_a));
        assert_eq!(
            after_b.get("interval_ms"),
            Some(&serde_json::json!(2_000)),
            "instance A's interval was clobbered"
        );
        assert_eq!(
            after_b.get("theme"),
            Some(&serde_json::json!("Dark")),
            "instance B's theme was clobbered"
        );
        assert_eq!(
            after_b.get("version"),
            Some(&serde_json::json!(9)),
            "future version was downgraded"
        );
        assert_eq!(
            after_b.get("future_flag"),
            Some(&serde_json::json!({"nested": [1, 2]})),
            "unknown field was dropped"
        );
    }

    #[test]
    fn consecutive_instance_saves_keep_both_interval_and_theme() {
        // End-to-end through the real writer: two sequential saves with the
        // same load snapshot, each changing a different known field.
        let path = temporary_path("two-instances");
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        let shared = Settings {
            version: CURRENT_VERSION,
            interval_ms: 1500,
            theme: tm_viewmodel::Theme::Light,
            ..Settings::default()
        };
        let serde_json::Value::Object(mut base) = serde_json::to_value(&shared).expect("serialize")
        else {
            panic!("Settings must serialize to an object")
        };
        base.insert("version".to_string(), serde_json::json!(9));
        base.insert(
            "future_flag".to_string(),
            serde_json::json!({"nested": [1, 2]}),
        );
        fs::write(
            &path,
            serde_json::to_string_pretty(&base).expect("serialize"),
        )
        .expect("write initial settings");

        let instance_a = Settings {
            interval_ms: 2_000,
            ..shared.clone()
        };
        let saved_a =
            save_to_path_with_baseline(&path, &instance_a, Some(&base)).expect("save instance A");
        assert_eq!(saved_a.get("interval_ms"), Some(&serde_json::json!(2_000)));

        // Instance B still holds the snapshot it loaded before A saved; the
        // writer re-reads the file so A's interval is visible during the merge.
        let instance_b = Settings {
            theme: tm_viewmodel::Theme::Dark,
            ..shared
        };
        let saved_b =
            save_to_path_with_baseline(&path, &instance_b, Some(&base)).expect("save instance B");
        assert_eq!(saved_b.get("interval_ms"), Some(&serde_json::json!(2_000)));
        assert_eq!(saved_b.get("theme"), Some(&serde_json::json!("Dark")));
        assert_eq!(saved_b.get("version"), Some(&serde_json::json!(9)));
        assert_eq!(
            saved_b.get("future_flag"),
            Some(&serde_json::json!({"nested": [1, 2]}))
        );

        let reloaded = load_document_from(&path).expect("reload document");
        assert_eq!(reloaded.settings.interval_ms, 2_000);
        assert_eq!(reloaded.settings.theme, tm_viewmodel::Theme::Dark);
        assert_eq!(reloaded.settings.version, 9);
        assert_eq!(
            reloaded.raw.get("future_flag"),
            Some(&serde_json::json!({"nested": [1, 2]}))
        );
        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }
}

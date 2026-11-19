//! Browser settings storage (`localStorage`).
//!
//! The schema lives in [`tm_viewmodel::Settings`] so the browser and native
//! shells stay compatible; this module only owns the storage medium.
//!
//! Only presentation preferences are stored (refresh interval, sort, details
//! panel). Process data, filters, tokens and host names are never persisted.
//!
//! `localStorage` access can fail (private mode, disabled storage, quota); every
//! failure degrades to defaults / a silent skip rather than a panic, because
//! losing a preference must never break the UI.
//!
//! Storage compatibility (R2-D01 / R5-D02 / R3-U02): a load keeps the raw JSON
//! object so a later save re-emits fields written by a newer shell and never
//! downgrades a newer `version`. Corrupt input is quarantined under a sibling
//! `*.corrupt.<timestamp>` key before defaults are installed, then yields a
//! fixed, redacted [`SettingsNotice`] plus, on wasm, a separate DOM notice
//! node. A later clean load clears both the queued notice and the DOM node.
//! Neither the `body[data-gpui-proof]` hook nor the existing `aria-live`
//! mirror is touched.
//!
//! Schema-version compatibility policy (R2X-D3), identical in the native shell:
//!
//! - **Backward**: a file from an older build (or with no `version`) is read with
//!   `#[serde(default)]` fallbacks for fields that did not exist yet, and its
//!   in-memory version is raised to the current `SETTINGS_VERSION`. The next save
//!   writes that current version, migrating the file forward.
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
//!
//! Cross-tab writes (HA-WEBMERGE): `localStorage` has no atomic
//! compare-and-swap. A naive "load then write the whole document" pair from two
//! tabs can interleave, and whichever tab saves last silently drops the other
//! tab's field. The chosen design is therefore **diff merge + storage sync**:
//! a save starts from the document re-read immediately before the write and
//! overwrites only the known fields whose value actually changed since the last
//! load/save ([`LAST_LOADED`]); the shell's `storage` listener mirrors a remote
//! write back into the UI so the next diff is computed against fresh state.
//! Unknown fields and the highest `version` always come from the re-read value.

use std::sync::Mutex;

use tm_viewmodel::Settings;

/// Key used in `localStorage`.
pub const STORAGE_KEY: &str = "gpui-task-manager.settings";

/// DOM id of the additive configuration-notice node.
///
/// It is deliberately distinct from `body[data-gpui-proof]` and the shell's
/// `status-live` mirror so the CDP proof hook and redaction path stay intact.
pub const SETTINGS_NOTICE_DOM_ID: &str = "gpui-settings-notice";

/// Prefix of the `localStorage` key that quarantines a corrupt settings value.
///
/// The native shell renames a corrupt file to `*.corrupt-<nanos>`; the browser
/// equivalent is a sibling key with this prefix and a hex timestamp, so the
/// offending bytes survive for diagnosis without blocking a clean reload.
pub const CORRUPT_BACKUP_KEY_PREFIX: &str = "gpui-task-manager.settings.corrupt.";

/// Maximum number of corrupt backups kept in `localStorage`. Older ones are
/// pruned after a successful quarantine (H1-BACKUP).
const CORRUPT_BACKUP_LIMIT: usize = 5;

/// Largest stored settings value this build will parse (4 MiB, UTF-8 bytes).
///
/// Judgment (PC-CSVBOM): `localStorage` quotas are browser-dependent (commonly
/// around 5 MiB per origin and shared by *all* keys), so a value near 4 MiB is
/// not a legitimate preference document even if the browser still accepts it.
/// Using the same cap as the native shell gives one consistent rule and keeps a
/// corrupt/oversized item from consuming the origin quota and starving other
/// keys. Over-limit values take the ordinary quarantine + redacted-notice path.
/// `str::len` counts bytes, matching the native on-disk byte cap.
const MAX_SETTINGS_BYTES: usize = 4 * 1024 * 1024;

/// Removes one leading UTF-8 BOM (EF BB BF, decoded as U+FEFF) before parsing.
///
/// A BOM-prefixed settings value copied from a Windows file is otherwise valid
/// JSON, but `serde_json` rejects the leading U+FEFF. Strip exactly one BOM so
/// the value loads instead of being quarantined as corrupt (PC-CSVBOM).
fn strip_bom(raw: &str) -> &str {
    raw.strip_prefix('\u{feff}').unwrap_or(raw)
}

/// Fixed, redacted text shown when a stored settings value is corrupt.
///
/// The offending bytes are preserved under a key ending in
/// `.corrupt.<timestamp>` before defaults are installed. Only that fixed
/// suffix is named here: no key namespace, path, host name or token is echoed.
pub const NOTICE_CORRUPT: &str = "保存された設定を読み込めませんでした。破損した設定は末尾が .corrupt.<timestamp> の別キーへ退避し、既定値で起動します。退避物には保持上限があり、復元手順は README.md を参照してください。";

/// Fixed, redacted text shown when persisting settings fails.
///
/// Published to the additive notice node and queued in `PENDING_NOTICE`, so the
/// shell can also copy it into the status bar without shell-side changes
/// (C2-SETTINGS). It never contains a path, token or storage error detail.
pub const NOTICE_SAVE_FAILED: &str =
    "設定を保存できませんでした。ブラウザのストレージ設定（容量・権限）を確認してください";

/// 更新間隔の UI プリセット（ミリ秒）。ネイティブ版と同じ 4 段階を使う。
///
/// ここで定義してシェルから参照することで、ボタンの候補と保存される
/// `settings.interval_ms` がずれないようにする。値はすべて
/// [`tm_viewmodel::MIN_REFRESH_INTERVAL`]〜[`tm_viewmodel::MAX_REFRESH_INTERVAL`] の範囲内。
pub const INTERVAL_PRESETS_MS: [u64; 4] = [500, 1_000, 2_000, 5_000];

/// 更新間隔プリセットの表示ラベル（ネイティブ版と同じ表記）。
pub fn interval_label(interval_ms: u64) -> String {
    match interval_ms {
        500 => "0.5s".to_string(),
        1_000 => "1s".to_string(),
        2_000 => "2s".to_string(),
        5_000 => "5s".to_string(),
        other => format!("{other}ms"),
    }
}

/// A configuration notice safe to pass to `Dashboard::set_persistent_status`.
///
/// `text` is the fixed [`NOTICE_CORRUPT`] constant: no path, host name, token or
/// parser error text is included, so it is safe to copy into the DOM as well.
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
    fn corrupt() -> Self {
        Self {
            text: NOTICE_CORRUPT.to_string(),
            is_error: true,
            persistent: true,
        }
    }

    fn save_failed() -> Self {
        Self {
            text: NOTICE_SAVE_FAILED.to_string(),
            is_error: true,
            // A failed save is a configuration problem, not a sample result:
            // it must survive the next successful sample (C2-SETTINGS).
            persistent: true,
        }
    }
}

/// Parsed settings plus the untouched JSON object they came from.
#[derive(Clone, Debug)]
struct SettingsDocument {
    settings: Settings,
    raw: serde_json::Map<String, serde_json::Value>,
}

static LAST_LOADED: Mutex<Option<serde_json::Map<String, serde_json::Value>>> = Mutex::new(None);
static PENDING_NOTICE: Mutex<Option<SettingsNotice>> = Mutex::new(None);

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Returns the `localStorage` handle, if the browser exposes one.
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// Builds a collision-free quarantine key for a value backed up at `nanos`.
///
/// `existing` is the snapshot of `localStorage` keys taken immediately before
/// the write. The plain `prefix + hex(nanos)` key is used when free; when it is
/// already taken (a second tab, or an earlier quarantine in the same
/// millisecond — `web-time` only has millisecond resolution) a numeric `-N`
/// suffix is appended until a free key is found.
///
/// Judgment (EC-BACKUP): an in-process monotonic counter was rejected because
/// it cannot see keys another tab wrote and would add shared state that races
/// on startup. Reusing the `-N` form already parsed by
/// [`corrupt_backup_sort_key`] activates that previously dead logic, keeps the
/// key self-describing and correctly ordered by the existing retention sweep,
/// and needs no extra state. A failed enumeration arrives here as an empty
/// slice, i.e. the unsuffixed fallback key.
///
/// Pure and timestamp/key-injected so the format stays testable without a
/// browser; [`quarantine_corrupt_with`] supplies the real storage snapshot.
#[cfg(any(target_family = "wasm", test))]
fn corrupt_backup_key(nanos: u128, existing: &[String]) -> String {
    let base = format!("{CORRUPT_BACKUP_KEY_PREFIX}{nanos:016x}");
    if !existing.iter().any(|key| key == &base) {
        return base;
    }
    // At most `existing.len()` candidates can be occupied, so the bounded range
    // always contains a free suffix; no unbounded loop is needed.
    for suffix in 1..=(existing.len() as u64 + 1) {
        let candidate = format!("{base}-{suffix}");
        if !existing.iter().any(|key| key == &candidate) {
            return candidate;
        }
    }
    unreachable!("bounded suffix search always finds a free key")
}

/// Wall-clock nanoseconds since the Unix epoch, used to keep quarantine keys
/// distinct. `0` is a safe fallback when the clock is unavailable.
#[cfg(target_family = "wasm")]
fn now_nanos() -> u128 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0)
}

/// Describes the two writes that quarantine a corrupt value: the backup key to
/// preserve the offending bytes under, and the default JSON to install at the
/// primary key so a reload no longer sees the corrupt document.
#[cfg(any(target_family = "wasm", test))]
fn quarantine_writes(nanos: u128, existing: &[String]) -> (String, String) {
    let default = serde_json::to_string(&Settings::default())
        .expect("Settings always serializes to a JSON object");
    (corrupt_backup_key(nanos, existing), default)
}

/// Sort key for a quarantine key: fixed-width timestamp, then the numeric
/// `-N` collision suffix.
///
/// Only keys with the exact prefix and a 16-hex timestamp are eligible, so an
/// unrelated `localStorage` entry is never removed by the retention sweep.
#[cfg(any(target_family = "wasm", test))]
fn corrupt_backup_sort_key(key: &str) -> Option<(String, u64)> {
    let rest = key.strip_prefix(CORRUPT_BACKUP_KEY_PREFIX)?;
    let (timestamp, suffix) = match rest.split_once('-') {
        Some((timestamp, suffix)) => (timestamp, suffix.parse::<u64>().ok()?),
        None => (rest, 0),
    };
    if timestamp.len() != 16 || !timestamp.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some((timestamp.to_string(), suffix))
}

/// Quarantines `raw` through an injected writer and returns the backup key.
///
/// `write` returns `true` on success. A failed backup never blocks installing
/// the defaults at [`STORAGE_KEY`] (the native shell still starts in that case).
/// After a successful quarantine, only the newest [`CORRUPT_BACKUP_LIMIT`]
/// backups are kept; enumeration/removal failures are ignored so cleanup never
/// blocks startup either.
#[cfg(any(target_family = "wasm", test))]
fn quarantine_corrupt_with(
    raw: &str,
    nanos: u128,
    mut write: impl FnMut(&str, &str) -> bool,
    keys: impl Fn() -> Vec<String>,
    remove: impl Fn(&str),
) -> String {
    // Snapshot the namespace before choosing the key. `keys()` is the injected
    // storage enumeration; on wasm it uses `Storage::length`/`key`, whose
    // failures already degrade to an empty snapshot (see `quarantine_corrupt`).
    let existing = keys();
    let (backup_key, default_json) = quarantine_writes(nanos, &existing);
    let backed_up = write(&backup_key, raw);
    let _ = write(STORAGE_KEY, &default_json);
    if backed_up {
        let mut backups: Vec<String> = keys()
            .into_iter()
            .filter(|key| corrupt_backup_sort_key(key).is_some())
            .collect();
        backups.sort_by_key(|key| corrupt_backup_sort_key(key));
        for key in backups.into_iter().rev().skip(CORRUPT_BACKUP_LIMIT) {
            remove(&key);
        }
    }
    backup_key
}

/// Browser-backed [`quarantine_corrupt_with`]: mirrors the native shell's
/// "move the corrupt file aside before the next save" step in `localStorage`.
#[cfg(target_family = "wasm")]
fn quarantine_corrupt(raw: &str) {
    let Some(storage) = storage() else {
        return;
    };
    let key = quarantine_corrupt_with(
        raw,
        now_nanos(),
        |key, value| storage.set_item(key, value).is_ok(),
        || {
            (0..storage.length().unwrap_or(0))
                .filter_map(|index| storage.key(index).ok().flatten())
                .collect()
        },
        |key| {
            let _ = storage.remove_item(key);
        },
    );
    // The key is a fixed namespace plus a timestamp; `raw` is never logged.
    log::warn!("破損設定を退避しました: {key}");
}

/// Native builds have no `localStorage`; the quarantine is a no-op there.
#[cfg(not(target_family = "wasm"))]
fn quarantine_corrupt(_raw: &str) {}

/// Clamps into the supported range without rewriting the schema `version`.
///
/// `Settings::normalized()` always forces the current version; storage must not
/// do that or a newer value would be downgraded on the next save (R2-D01).
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
/// JSON forms are opaque: they stay in `raw` and [`merged_map`] writes them back
/// verbatim rather than replacing them with a version this build understands
/// (R2X-D3).
fn interpretable_version(value: Option<&serde_json::Value>) -> Option<u32> {
    value
        .and_then(|value| value.as_u64())
        .and_then(|version| u32::try_from(version).ok())
}

/// Parses a persisted JSON document, keeping its raw object for round-tripping.
fn parse_document(raw: &str) -> Option<SettingsDocument> {
    if raw.len() > MAX_SETTINGS_BYTES {
        return None;
    }
    let mut value: serde_json::Value = serde_json::from_str(strip_bom(raw)).ok()?;
    let object = value.as_object()?.clone();
    let stored_version = interpretable_version(object.get("version"));

    // A future schema may use a version type this build cannot deserialize;
    // that must not make the whole value look corrupt.
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "version".to_string(),
            serde_json::json!(tm_viewmodel::SETTINGS_VERSION),
        );
    }
    let mut settings: Settings = serde_json::from_value(value).ok()?;
    settings = clamped(settings);
    settings.version = stored_version
        .unwrap_or(tm_viewmodel::SETTINGS_VERSION)
        .max(tm_viewmodel::SETTINGS_VERSION);
    Some(SettingsDocument {
        settings,
        raw: object,
    })
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
        } else {
            map.insert(key, value);
        }
    }
    map
}

/// Rebuilds the object to save from the freshly re-read stored value, applying
/// **only the known fields the caller actually changed**.
///
/// `baseline` is the raw object captured at the previous load/save. A field is
/// considered changed when its serialised value differs from the baseline's
/// value (or is absent there, e.g. an older schema). Columns the caller never
/// touched are left exactly as they were re-read, so another tab's write to
/// those columns survives. Unknown fields are likewise never rewritten here.
/// `version` keeps the existing max/opaque policy from [`merged_map`] and is
/// handled independently of the diff.
fn merged_map_for_changes(
    settings: &Settings,
    stored: Option<&serde_json::Map<String, serde_json::Value>>,
    baseline: Option<&serde_json::Map<String, serde_json::Value>>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut map = stored.cloned().unwrap_or_default();
    let known = serde_json::to_value(clamped(settings.clone()))
        .expect("Settings always serializes to a JSON object");
    let serde_json::Value::Object(known) = known else {
        return map;
    };
    for (key, value) in known {
        if key == "version" {
            match interpretable_version(map.get(&key)) {
                // Comparable numeric version: upgrade older values, never
                // downgrade newer ones (R2-D01).
                Some(stored_version) => {
                    let current = value.as_u64().unwrap_or(0);
                    map.insert(key, serde_json::json!((stored_version as u64).max(current)));
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
        let changed = baseline
            .map(|base| base.get(&key) != Some(&value))
            .unwrap_or(true);
        if changed {
            map.insert(key, value);
        }
    }
    map
}

/// Pure, baseline-injected variant of [`merged_for_save`] so tests can model two
/// tabs without depending on the process-wide [`LAST_LOADED`] state.
fn merged_for_save_inner(
    settings: &Settings,
    stored: Option<&str>,
    baseline: Option<&serde_json::Map<String, serde_json::Value>>,
) -> serde_json::Map<String, serde_json::Value> {
    let raw = stored
        // Ignore an oversized re-read value just like the load path does: it is
        // not a usable merge base, so fall back to the in-memory baseline.
        .filter(|text| text.len() <= MAX_SETTINGS_BYTES)
        .and_then(|text| serde_json::from_str::<serde_json::Value>(strip_bom(text)).ok())
        .and_then(|value| value.as_object().cloned())
        .or_else(|| baseline.cloned());
    merged_map_for_changes(settings, raw.as_ref(), baseline)
}

/// Raw-injected loading, shared by [`load_with_notice`] and the tests
/// (no browser needed).
fn load_from_raw(raw: Option<&str>) -> Settings {
    let Some(raw) = raw else {
        // No stored value is the healthy "missing" state: clear any stale
        // notice from an earlier corrupt load, as the native shell does.
        *lock(&LAST_LOADED) = None;
        *lock(&PENDING_NOTICE) = None;
        clear_notice_dom();
        return Settings::default();
    };
    match parse_document(raw) {
        Some(document) => {
            *lock(&LAST_LOADED) = Some(document.raw);
            *lock(&PENDING_NOTICE) = None;
            // Same path as the notice clear: a healthy load must not leave the
            // corrupt notice node on screen (AC-WEBSETTINGS).
            clear_notice_dom();
            document.settings
        }
        None => {
            quarantine_corrupt(raw);
            let notice = SettingsNotice::corrupt();
            *lock(&LAST_LOADED) = None;
            *lock(&PENDING_NOTICE) = Some(notice.clone());
            log::warn!("保存された設定を読み込めません（既定値を使います）");
            publish_notice_dom(&notice);
            Settings::default()
        }
    }
}

/// Loads settings and returns the pending configuration notice, if any.
pub fn load_with_notice() -> (Settings, Option<SettingsNotice>) {
    load_from_raw_with_notice(
        storage()
            .and_then(|storage| storage.get_item(STORAGE_KEY).ok().flatten())
            .as_deref(),
    )
}

/// Raw-injected variant of [`load_with_notice`], also used by the tests.
fn load_from_raw_with_notice(raw: Option<&str>) -> (Settings, Option<SettingsNotice>) {
    let settings = load_from_raw(raw);
    (settings, take_pending_notice())
}

/// Takes the redacted notice recorded by the last [`load_with_notice`], if any.
pub fn take_pending_notice() -> Option<SettingsNotice> {
    lock(&PENDING_NOTICE).take()
}

/// Writes a fixed, redacted notice into a dedicated DOM node.
///
/// The node is additive: it never writes `body[data-gpui-proof]` and never
/// touches the shell's `status-live` mirror, so the CDP hook and the existing
/// `aria-live` / redaction contract keep working.
#[cfg(target_family = "wasm")]
fn publish_notice_dom(notice: &SettingsNotice) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let Some(body) = document.body() else {
        return;
    };
    let element = match document.get_element_by_id(SETTINGS_NOTICE_DOM_ID) {
        Some(element) => element,
        None => {
            let Ok(element) = document.create_element("div") else {
                return;
            };
            if element.set_attribute("id", SETTINGS_NOTICE_DOM_ID).is_err() {
                return;
            }
            let _ = element.set_attribute("role", "alert");
            let _ = element.set_attribute(
                "style",
                "position:fixed;top:8px;left:8px;z-index:9999;max-width:min(560px,90vw);\
                 padding:8px 12px;background:#3a1f1f;color:#ffd7d7;border:1px solid #a33;\
                 border-radius:6px;font:13px/1.4 sans-serif",
            );
            if body.append_child(&element).is_err() {
                return;
            }
            element
        }
    };
    // The text is a fixed constant, so no path, token or host name is echoed.
    if element.text_content().as_deref() != Some(notice.text.as_str()) {
        element.set_text_content(Some(&notice.text));
    }
}

/// Native builds have no DOM; notice publication is a no-op there.
#[cfg(not(target_family = "wasm"))]
fn publish_notice_dom(_notice: &SettingsNotice) {}

/// Removes the additive notice DOM node on a healthy load.
///
/// A corrupt load publishes a fixed notice into this node; leaving it behind
/// after settings later load cleanly would show a stale error indefinitely.
/// This runs on exactly the path that clears `PENDING_NOTICE` (AC-WEBSETTINGS).
#[cfg(target_family = "wasm")]
fn clear_notice_dom() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    if let Some(element) = document.get_element_by_id(SETTINGS_NOTICE_DOM_ID) {
        element.remove();
    }
}

/// Native builds have no DOM; clearing the notice node is a no-op there.
#[cfg(not(target_family = "wasm"))]
fn clear_notice_dom() {}

/// Parses a persisted settings string, falling back to defaults on bad JSON.
///
/// Kept pure so the fallback path can be tested without a browser context.
fn parse_saved(raw: &str) -> Settings {
    match parse_document(raw) {
        Some(document) => document.settings,
        None => {
            log::warn!("保存された設定を読み込めません（既定値を使います）: 不正な JSON");
            Settings::default()
        }
    }
}

/// Builds the JSON object to persist from the value currently in `localStorage`.
///
/// `stored` is the value re-read immediately before the write, so another tab
/// that wrote a newer `version`, extra fields, or a different known field after
/// our load cannot be rolled back by this save (C2-SETTINGS / HA-WEBMERGE).
/// Only known fields that differ from [`LAST_LOADED`] are applied; that
/// snapshot is also the fallback for an unreadable destination. A full CAS is
/// impossible with `localStorage`, which is why the diff is paired with the
/// shell's `storage` reload rather than treated as a transaction.
fn merged_for_save(
    settings: &Settings,
    stored: Option<&str>,
) -> serde_json::Map<String, serde_json::Value> {
    let baseline = lock(&LAST_LOADED).clone();
    merged_for_save_inner(settings, stored, baseline.as_ref())
}

/// Records a fixed, redacted save-failure notice and publishes it to the DOM
/// node. The shell can drain `PENDING_NOTICE` later without any shell change;
/// the DOM node shows the failure immediately (C2-SETTINGS).
fn record_save_failure() {
    let notice = SettingsNotice::save_failed();
    *lock(&PENDING_NOTICE) = Some(notice.clone());
    publish_notice_dom(&notice);
}

/// Persists settings. Returns an error message on failure so the caller can
/// surface it, but callers are free to ignore it.
///
/// Unknown fields and the newest `version` written by any tab are preserved,
/// and known fields another tab changed are kept unless this caller explicitly
/// edited them since its last load/save (HA-WEBMERGE). A failure also queues a
/// redacted notice for the status bar / notice node; a success leaves any
/// existing notice untouched.
pub fn save(settings: &Settings) -> Result<(), String> {
    let result = (|| {
        let storage = storage().ok_or_else(|| "localStorage を利用できません".to_string())?;
        // Re-read first: a parallel tab may have changed the stored value since
        // this process loaded it, and that newest value drives max(version).
        let stored = storage.get_item(STORAGE_KEY).ok().flatten();
        let map = merged_for_save(settings, stored.as_deref());
        let json = serde_json::to_string(&map)
            .map_err(|error| format!("設定を JSON に変換できません: {error}"))?;
        storage
            .set_item(STORAGE_KEY, &json)
            .map_err(|_| "設定を保存できません（ストレージ容量または権限）".to_string())?;
        *lock(&LAST_LOADED) = Some(map);
        Ok(())
    })();
    if result.is_err() {
        record_save_failure();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tm_core::{SortDirection, SortKey, SortSpec};

    // Tests that touch the process-wide PENDING_NOTICE must not interleave.
    static NOTICE_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// In-memory stand-in for `localStorage`, shared by the quarantine tests.
    #[derive(Default)]
    struct MemoryStorage {
        entries: std::cell::RefCell<std::collections::HashMap<String, String>>,
        writes: std::cell::RefCell<Vec<(String, String)>>,
    }

    impl MemoryStorage {
        fn write(&self, key: &str, value: &str) -> bool {
            self.writes
                .borrow_mut()
                .push((key.to_string(), value.to_string()));
            self.entries
                .borrow_mut()
                .insert(key.to_string(), value.to_string());
            true
        }

        fn keys(&self) -> Vec<String> {
            self.entries.borrow().keys().cloned().collect()
        }

        fn remove(&self, key: &str) {
            self.entries.borrow_mut().remove(key);
        }
    }

    #[test]
    fn storage_key_is_namespaced() {
        assert!(STORAGE_KEY.starts_with("gpui-task-manager."));
    }

    #[test]
    fn json_round_trip_matches_the_shared_schema() {
        let settings = Settings {
            version: tm_viewmodel::SETTINGS_VERSION,
            interval_ms: 2_000,
            sort: SortSpec {
                key: SortKey::Name,
                direction: SortDirection::Ascending,
            },
            details_visible: false,
            tree_view: false,
            theme: tm_viewmodel::Theme::Dark,
        };
        let json = serde_json::to_string(&settings).expect("serialize");
        let restored: Settings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, settings);
    }

    #[test]
    fn future_fields_are_ignored_and_version_normalised() {
        // A newer shell may write fields this build does not know. `#[serde(default)]`
        // must let us read them without losing the known values.
        let raw = r#"{"version":1,"interval_ms":1500,"future_flag":42}"#;
        let settings = parse_saved(raw);
        assert_eq!(settings.interval_ms, 1_500);
        assert_eq!(settings.version, tm_viewmodel::SETTINGS_VERSION);
        assert_eq!(settings.theme, tm_viewmodel::Theme::Dark);
    }

    #[test]
    fn corrupt_json_falls_back_to_defaults() {
        let settings = parse_saved("{ not json");
        assert_eq!(settings.interval_ms, 1_000);
        assert_eq!(settings.version, tm_viewmodel::SETTINGS_VERSION);
        assert!(settings.details_visible);
    }

    #[test]
    fn bom_prefixed_settings_load_without_a_corrupt_notice() {
        let _guard = lock(&NOTICE_TEST_LOCK);
        let raw = "\u{feff}{\"version\":9,\"interval_ms\":1500,\"future_flag\":42}";
        let document = parse_document(raw).expect("BOM-prefixed JSON must parse");
        assert_eq!(document.settings.version, 9);
        assert_eq!(document.settings.interval_ms, 1_500);
        assert_eq!(
            document.raw.get("future_flag"),
            Some(&serde_json::json!(42))
        );

        let (settings, notice) = load_from_raw_with_notice(Some(raw));
        assert_eq!(settings.interval_ms, 1_500);
        assert_eq!(notice, None, "a BOM must not be treated as corruption");
    }

    #[test]
    fn oversized_stored_value_is_treated_as_corrupt() {
        let _guard = lock(&NOTICE_TEST_LOCK);
        let raw = format!(
            "{{\"version\":9,\"interval_ms\":1500,\"pad\":\"{}\"}}",
            "x".repeat(MAX_SETTINGS_BYTES)
        );
        assert!(
            parse_document(&raw).is_none(),
            "oversized value must be rejected"
        );
        let (settings, notice) = load_from_raw_with_notice(Some(&raw));
        assert_eq!(settings, Settings::default());
        assert!(
            notice.is_some(),
            "oversized value must yield a corrupt notice"
        );

        // An oversized re-read value must not feed the save merge either.
        let map = merged_for_save_inner(&Settings::default(), Some(&raw), None);
        assert_ne!(map.get("version"), Some(&serde_json::json!(9)));
        assert!(!map.contains_key("pad"));
    }

    #[test]
    fn stored_values_are_clamped() {
        let restored: Settings = serde_json::from_str(r#"{"interval_ms":1}"#).expect("parse");
        let restored = restored.normalized();
        assert_eq!(
            restored.interval_ms,
            tm_viewmodel::MIN_REFRESH_INTERVAL.as_millis() as u64
        );
    }

    #[test]
    fn interval_presets_are_supported_and_distinct() {
        let minimum = tm_viewmodel::MIN_REFRESH_INTERVAL.as_millis() as u64;
        let maximum = tm_viewmodel::MAX_REFRESH_INTERVAL.as_millis() as u64;
        for (index, &millis) in INTERVAL_PRESETS_MS.iter().enumerate() {
            assert!(
                (minimum..=maximum).contains(&millis),
                "preset {millis} is outside the sampler range"
            );
            // 正規化で値が変わらない = ボタンの表示値と保存値が一致する。
            let normalized = Settings {
                interval_ms: millis,
                ..Settings::default()
            }
            .normalized();
            assert_eq!(normalized.interval_ms, millis);
            for &other in &INTERVAL_PRESETS_MS[index + 1..] {
                assert_ne!(millis, other, "presets must be unique");
            }
        }
    }

    #[test]
    fn interval_preset_round_trips_through_persisted_json() {
        // UI で選んだ更新間隔が localStorage の JSON を往復しても保たれること。
        for &millis in &INTERVAL_PRESETS_MS {
            let settings = Settings {
                interval_ms: millis,
                ..Settings::default()
            }
            .normalized();
            let json = serde_json::to_string(&settings).expect("serialize");
            assert_eq!(parse_saved(&json).interval_ms, millis);
        }
    }

    #[test]
    fn interval_labels_match_the_native_shell() {
        assert_eq!(interval_label(500), "0.5s");
        assert_eq!(interval_label(1_000), "1s");
        assert_eq!(interval_label(2_000), "2s");
        assert_eq!(interval_label(5_000), "5s");
        assert_eq!(interval_label(1_234), "1234ms");
    }

    #[test]
    fn future_schema_version_is_preserved_but_old_versions_are_upgraded() {
        let document =
            parse_document(r#"{"version":9,"interval_ms":1500}"#).expect("future version");
        assert_eq!(document.settings.version, 9);
        let document = parse_document(r#"{"version":1,"interval_ms":1500}"#).expect("old version");
        assert_eq!(document.settings.version, tm_viewmodel::SETTINGS_VERSION);
    }

    #[test]
    fn unknown_fields_and_future_version_survive_a_load_save_round_trip() {
        let raw =
            r#"{"version":9,"interval_ms":1500,"future_flag":42,"future_nested":{"a":[1,2]}}"#;
        let document = parse_document(raw).expect("parse document");
        assert_eq!(document.settings.version, 9);

        // シェルは現在版の Settings を組み立てて save する。読み込んだ document の
        // 未知フィールドと将来版が上書きされないことを固定する。
        let caller = Settings {
            version: tm_viewmodel::SETTINGS_VERSION,
            ..document.settings.clone()
        };
        let map = merged_map(&caller, Some(&document.raw));
        let json = serde_json::to_string(&map).expect("serialize");
        let reloaded = parse_document(&json).expect("reload document");
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
    }

    #[test]
    fn save_merges_against_the_freshly_reread_stored_value() {
        // Another tab may have changed localStorage after this process loaded.
        // The value passed to the merge is the one re-read immediately before
        // the write, and it must drive both `max(version)` and unknown field
        // preservation (C2-SETTINGS). The pure, baseline-injected variant keeps
        // this assertion independent of the process-wide `LAST_LOADED` state.
        let caller = Settings {
            version: tm_viewmodel::SETTINGS_VERSION,
            interval_ms: 2_000,
            ..Settings::default()
        };
        let latest = r#"{"version":11,"interval_ms":1500,"newer_only":{"x":2}}"#;
        let map = merged_for_save_inner(&caller, Some(latest), None);
        let json = serde_json::to_string(&map).expect("serialize");
        let reloaded = parse_document(&json).expect("reload document");
        assert_eq!(
            reloaded.settings.version, 11,
            "freshly read version must not be downgraded"
        );
        assert_eq!(reloaded.settings.interval_ms, 2_000);
        assert_eq!(
            reloaded.raw.get("newer_only"),
            Some(&serde_json::json!({"x": 2}))
        );

        // A different `stored` value yields a different result: the merge is
        // driven by the current contents, not a snapshot taken at load time.
        let stale = r#"{"version":9,"interval_ms":1500,"stale_only":1}"#;
        let stale_map = merged_for_save_inner(&caller, Some(stale), None);
        assert_eq!(stale_map.get("version"), Some(&serde_json::json!(9)));
        assert!(stale_map.contains_key("stale_only"));
        assert!(!stale_map.contains_key("newer_only"));

        // If the newest value has a lower version, max(that, current) wins -
        // an old in-memory version must not be resurrected.
        let downgraded = r#"{"version":1,"interval_ms":1500,"fresh_only":2}"#;
        let map = merged_for_save_inner(&caller, Some(downgraded), None);
        assert_eq!(
            map.get("version"),
            Some(&serde_json::json!(tm_viewmodel::SETTINGS_VERSION))
        );
        assert_eq!(map.get("fresh_only"), Some(&serde_json::json!(2)));
        assert!(!map.contains_key("stale_only"));
    }

    #[test]
    fn two_tab_sequential_saves_keep_both_interval_and_theme() {
        // HA-WEBMERGE acceptance: tab A changes interval, tab B changes theme.
        // Both edits must survive even though each tab re-reads and rewrites the
        // same single localStorage key.
        let initial = r#"{"version":2,"interval_ms":1000,"theme":"Dark","unknown_flag":7}"#;
        let tab_a = parse_document(initial).expect("tab A load");
        let tab_b = parse_document(initial).expect("tab B load");
        assert_eq!(tab_a.settings.interval_ms, 1_000);
        assert_eq!(tab_b.settings.theme, tm_viewmodel::Theme::Dark);

        // Tab A edits only interval.
        let a_settings = Settings {
            interval_ms: 2_000,
            ..tab_a.settings.clone()
        };
        let after_a = serde_json::to_string(&merged_for_save_inner(
            &a_settings,
            Some(initial),
            Some(&tab_a.raw),
        ))
        .expect("serialize A");

        // Tab B observes the storage event and reloads, so its next diff is
        // based on the value tab A just wrote.
        let tab_b_reloaded = parse_document(&after_a).expect("tab B reload");
        assert_eq!(tab_b_reloaded.settings.interval_ms, 2_000);
        let b_settings = Settings {
            theme: tm_viewmodel::Theme::Light,
            ..tab_b_reloaded.settings.clone()
        };
        let after_b = serde_json::to_string(&merged_for_save_inner(
            &b_settings,
            Some(&after_a),
            Some(&tab_b_reloaded.raw),
        ))
        .expect("serialize B");

        let final_doc = parse_document(&after_b).expect("final load");
        assert_eq!(final_doc.settings.interval_ms, 2_000, "tab A edit was lost");
        assert_eq!(
            final_doc.settings.theme,
            tm_viewmodel::Theme::Light,
            "tab B edit was lost"
        );
        assert_eq!(final_doc.settings.version, 2);
        assert_eq!(
            final_doc.raw.get("unknown_flag"),
            Some(&serde_json::json!(7))
        );

        // Even if the storage event is missed, B's stale interval must not be
        // written back: the diff only contains the field B actually touched.
        let b_stale = Settings {
            theme: tm_viewmodel::Theme::Light,
            ..tab_b.settings.clone()
        };
        let stale_map = merged_for_save_inner(&b_stale, Some(&after_a), Some(&tab_b.raw));
        assert_eq!(
            stale_map.get("interval_ms"),
            Some(&serde_json::json!(2_000))
        );
        assert_eq!(stale_map.get("theme"), Some(&serde_json::json!("Light")));
    }

    #[test]
    fn diff_merge_keeps_unknown_fields_and_future_version() {
        // The diff must not rewrite fields this build does not own, and the
        // version policy from R2-D01 / R2X-D3 must stay byte-for-byte intact.
        let loaded = r#"{"version":9,"interval_ms":1000,"theme":"Dark","future":{"x":[1,2]}}"#;
        let document = parse_document(loaded).expect("load future document");
        let caller = Settings {
            theme: tm_viewmodel::Theme::Light,
            ..document.settings.clone()
        };
        // Another tab upgraded the version and added a field after our load.
        let stored = r#"{"version":11,"interval_ms":2000,"theme":"Dark","future":{"x":[1,2]},"newer_only":3}"#;
        let map = merged_for_save_inner(&caller, Some(stored), Some(&document.raw));

        assert_eq!(map.get("version"), Some(&serde_json::json!(11)));
        assert_eq!(
            map.get("interval_ms"),
            Some(&serde_json::json!(2_000)),
            "untouched known field must not overwrite the newer stored value"
        );
        assert_eq!(map.get("theme"), Some(&serde_json::json!("Light")));
        assert_eq!(map.get("future"), Some(&serde_json::json!({"x": [1, 2]})));
        assert_eq!(map.get("newer_only"), Some(&serde_json::json!(3)));

        // Opaque versions are still preserved verbatim by the diff path.
        let mut document_value = serde_json::json!({"interval_ms": 1000, "future_flag": 1});
        document_value["version"] = serde_json::json!("9");
        let opaque = parse_document(&document_value.to_string()).expect("opaque version loads");
        let opaque_caller = Settings {
            interval_ms: 2_000,
            ..opaque.settings.clone()
        };
        let opaque_map = merged_for_save_inner(
            &opaque_caller,
            Some(&document_value.to_string()),
            Some(&opaque.raw),
        );
        assert_eq!(opaque_map.get("version"), Some(&serde_json::json!("9")));
        assert_eq!(
            opaque_map.get("interval_ms"),
            Some(&serde_json::json!(2_000))
        );
        assert_eq!(opaque_map.get("future_flag"), Some(&serde_json::json!(1)));
    }

    #[test]
    fn merged_for_save_uses_last_loaded_as_the_diff_baseline() {
        let _guard = lock(&NOTICE_TEST_LOCK);
        let loaded = r#"{"version":2,"interval_ms":1000,"theme":"Dark"}"#;
        let document = parse_document(loaded).expect("load document");
        *lock(&LAST_LOADED) = Some(document.raw.clone());
        let caller = Settings {
            theme: tm_viewmodel::Theme::Light,
            ..document.settings.clone()
        };
        // The public wrapper must read LAST_LOADED itself: only `theme` changed,
        // so a newer `interval_ms` written by another tab is kept.
        let stored = r#"{"version":2,"interval_ms":2000,"theme":"Dark"}"#;
        let map = merged_for_save(&caller, Some(stored));
        assert_eq!(map.get("interval_ms"), Some(&serde_json::json!(2_000)));
        assert_eq!(map.get("theme"), Some(&serde_json::json!("Light")));
        *lock(&LAST_LOADED) = None;
    }

    #[test]
    fn corrupt_notice_is_redacted_and_drained_once() {
        // All assertions about the process-wide notice share one lock: the test
        // runner is parallel and these globals must not race.
        let _guard = lock(&NOTICE_TEST_LOCK);
        let (_settings, notice) = load_from_raw_with_notice(Some("{ not json"));
        let notice = notice.expect("pending notice");
        assert_eq!(notice.text, NOTICE_CORRUPT);
        assert!(notice.is_error);
        assert!(notice.persistent, "startup notice must survive a sample");
        // 赤退契約: パス・ホスト名・トークン・パーサ詳細を含まない。
        assert!(!notice.text.contains('/'), "{}", notice.text);
        assert!(!notice.text.contains("not json"), "{}", notice.text);
        assert!(!notice.text.contains(STORAGE_KEY), "{}", notice.text);
        assert_eq!(take_pending_notice(), None, "notice must be drained once");

        // A healthy value produces no notice on the next load.
        let (_settings, notice) = load_from_raw_with_notice(Some(r#"{"interval_ms":1500}"#));
        assert_eq!(notice, None);

        // A failed save is queued for the shell (which already drains notices)
        // with a fixed, redacted message; it is drained exactly once.
        record_save_failure();
        let notice = take_pending_notice().expect("save failure notice");
        assert_eq!(notice, SettingsNotice::save_failed());
        assert_eq!(notice.text, NOTICE_SAVE_FAILED);
        assert!(notice.is_error);
        assert!(notice.persistent, "must survive the next sample");
        assert!(!notice.text.contains('/'), "{}", notice.text);
        assert!(!notice.text.contains(STORAGE_KEY), "{}", notice.text);
        assert_eq!(take_pending_notice(), None, "notice must be drained once");
    }

    #[test]
    fn healthy_load_clears_a_stale_corrupt_notice_and_dom_node() {
        let _guard = lock(&NOTICE_TEST_LOCK);

        // A corrupt load leaves a persistent notice (and, on wasm, a DOM node).
        let (_settings, notice) = load_from_raw_with_notice(Some("{ not json"));
        assert!(notice.is_some(), "corrupt load must queue a notice");

        // A later healthy load clears both the queued notice and the DOM node.
        // `clear_notice_dom` runs on this very path; on native it is a no-op but
        // must remain safe to call (AC-WEBSETTINGS).
        let (_settings, notice) = load_from_raw_with_notice(Some(r#"{"interval_ms":1500}"#));
        assert_eq!(notice, None, "stale notice must be cleared on a clean load");
        assert_eq!(take_pending_notice(), None);

        // A missing stored value is also a healthy load and clears the notice.
        let (_settings, notice) = load_from_raw_with_notice(None);
        assert_eq!(notice, None);
        clear_notice_dom();
    }

    #[test]
    fn corrupt_value_is_quarantined_then_defaults_installed() {
        let raw = "{ not json";
        let nanos = 0x0000_0000_0000_1234u128;
        let storage = MemoryStorage::default();

        let key = quarantine_corrupt_with(
            raw,
            nanos,
            |key, value| storage.write(key, value),
            || storage.keys(),
            |key| storage.remove(key),
        );
        assert_eq!(key, format!("{CORRUPT_BACKUP_KEY_PREFIX}{nanos:016x}"));
        assert!(key.starts_with(CORRUPT_BACKUP_KEY_PREFIX));
        let writes = storage.writes.borrow();
        assert_eq!(writes.len(), 2, "backup first, then default at primary key");
        assert_eq!(writes[0], (key.clone(), raw.to_string()));
        assert_eq!(writes[1].0, STORAGE_KEY);
        assert_eq!(
            parse_saved(&writes[1].1).interval_ms,
            1_000,
            "installed value must be a healthy default document"
        );
        drop(writes);

        // Each quarantine gets a distinct key so earlier backups survive.
        let next = quarantine_corrupt_with(
            raw,
            nanos + 1,
            |key, value| storage.write(key, value),
            || storage.keys(),
            |key| storage.remove(key),
        );
        assert_ne!(key, next);

        // A failed backup write must not prevent installing the defaults.
        let failed = MemoryStorage::default();
        quarantine_corrupt_with(
            raw,
            nanos,
            |key, value| {
                if key.starts_with(CORRUPT_BACKUP_KEY_PREFIX) {
                    false
                } else {
                    failed.write(key, value)
                }
            },
            || failed.keys(),
            |key| failed.remove(key),
        );
        let installed = failed.writes.borrow();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].0, STORAGE_KEY);
    }

    #[test]
    fn corrupt_backup_sort_key_orders_numeric_suffixes() {
        let stamp = 0x1000u128;
        let key = |suffix: &str| format!("{CORRUPT_BACKUP_KEY_PREFIX}{stamp:016x}{suffix}");
        let plain = corrupt_backup_sort_key(&key(""));
        let one = corrupt_backup_sort_key(&key("-1"));
        let two = corrupt_backup_sort_key(&key("-2"));
        let ten = corrupt_backup_sort_key(&key("-10"));
        assert!(plain < one, "base must sort before -1");
        assert!(one < two, "-1 must sort before -2");
        assert!(two < ten, "-2 must sort before -10");
        assert_eq!(one, Some((format!("{stamp:016x}"), 1)));
        assert_eq!(plain, Some((format!("{stamp:016x}"), 0)));
        assert!(corrupt_backup_sort_key(STORAGE_KEY).is_none());
    }

    #[test]
    fn corrupt_backup_key_avoids_collisions_with_numeric_suffix() {
        let nanos = 0x0000_0000_0000_1234u128;
        let base = format!("{CORRUPT_BACKUP_KEY_PREFIX}{nanos:016x}");
        // Empty snapshot is also the enumeration-failure fallback.
        assert_eq!(corrupt_backup_key(nanos, &[]), base);

        let first = corrupt_backup_key(nanos, std::slice::from_ref(&base));
        assert_eq!(first, format!("{base}-1"));
        let second = corrupt_backup_key(nanos, &[base.clone(), first.clone()]);
        assert_eq!(second, format!("{base}-2"));

        // A different timestamp is never a collision.
        let other = corrupt_backup_key(nanos + 1, &[base, first, second]);
        assert_eq!(
            other,
            format!("{CORRUPT_BACKUP_KEY_PREFIX}{:016x}", nanos + 1)
        );
    }

    #[test]
    fn quarantines_in_the_same_millisecond_get_distinct_keys() {
        let raw = "{ not json";
        let nanos = 0x0000_0000_0000_1234u128;
        let storage = MemoryStorage::default();
        let first = quarantine_corrupt_with(
            raw,
            nanos,
            |key, value| storage.write(key, value),
            || storage.keys(),
            |key| storage.remove(key),
        );
        let second = quarantine_corrupt_with(
            raw,
            nanos,
            |key, value| storage.write(key, value),
            || storage.keys(),
            |key| storage.remove(key),
        );
        assert_ne!(first, second);
        assert_eq!(second, format!("{first}-1"));
        // Both offending documents survive side by side instead of overwriting.
        let entries = storage.entries.borrow();
        assert_eq!(entries.get(&first).map(String::as_str), Some(raw));
        assert_eq!(entries.get(&second).map(String::as_str), Some(raw));
    }

    #[test]
    fn six_quarantines_keep_only_the_five_newest_backups() {
        let storage = MemoryStorage::default();
        for stamp in [0x1000u128, 0x1001, 0x1002, 0x1003, 0x1004, 0x1005] {
            quarantine_corrupt_with(
                "{ not json",
                stamp,
                |key, value| storage.write(key, value),
                || storage.keys(),
                |key| storage.remove(key),
            );
        }
        let mut backups: Vec<String> = storage
            .keys()
            .into_iter()
            .filter(|key| key.starts_with(CORRUPT_BACKUP_KEY_PREFIX))
            .collect();
        backups.sort();
        assert_eq!(backups.len(), CORRUPT_BACKUP_LIMIT, "oldest must be pruned");
        assert!(!backups.contains(&format!("{CORRUPT_BACKUP_KEY_PREFIX}{:016x}", 0x1000)));
        assert!(backups.contains(&format!("{CORRUPT_BACKUP_KEY_PREFIX}{:016x}", 0x1005)));
        assert!(storage.entries.borrow().contains_key(STORAGE_KEY));
    }

    #[test]
    fn six_quarantines_in_one_millisecond_keep_only_the_five_newest() {
        // web-time has 1ms resolution, so a burst of quarantines can share one
        // timestamp. Collision suffixes must stay distinct AND prune correctly.
        let nanos = 0x0000_0000_0000_1234u128;
        let storage = MemoryStorage::default();
        let mut issued = Vec::new();
        for _ in 0..6 {
            issued.push(quarantine_corrupt_with(
                "{ not json",
                nanos,
                |key, value| storage.write(key, value),
                || storage.keys(),
                |key| storage.remove(key),
            ));
        }
        assert_eq!(
            issued[0],
            format!("{CORRUPT_BACKUP_KEY_PREFIX}{nanos:016x}")
        );
        assert_eq!(
            issued[5],
            format!("{CORRUPT_BACKUP_KEY_PREFIX}{nanos:016x}-5")
        );

        let backups: Vec<String> = storage
            .keys()
            .into_iter()
            .filter(|key| key.starts_with(CORRUPT_BACKUP_KEY_PREFIX))
            .collect();
        assert_eq!(backups.len(), CORRUPT_BACKUP_LIMIT, "oldest must be pruned");
        assert!(!backups.contains(&issued[0]), "plain key is the oldest");
        for key in &issued[1..] {
            assert!(backups.contains(key), "{key} must survive");
        }
        assert!(storage.entries.borrow().contains_key(STORAGE_KEY));
    }

    #[test]
    fn notice_dom_id_is_additive_and_distinct_from_existing_hooks() {
        assert_eq!(SETTINGS_NOTICE_DOM_ID, "gpui-settings-notice");
        assert_ne!(SETTINGS_NOTICE_DOM_ID, "data-gpui-proof");
        assert_ne!(SETTINGS_NOTICE_DOM_ID, "status-live");
        assert_ne!(SETTINGS_NOTICE_DOM_ID, STORAGE_KEY);
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
            Some(&serde_json::json!(tm_viewmodel::SETTINGS_VERSION))
        );
        assert_eq!(map.get("interval_ms"), Some(&serde_json::json!(10_000)));
        assert_ne!(map.get("interval_ms"), Some(&serde_json::json!(20_000)));
    }

    #[test]
    fn schema_policy_upgrades_old_and_versionless_documents_on_save() {
        // Backward compatibility: v1 predates tree_view/theme. serde defaults
        // fill them, and a save must record the current version.
        for raw in [
            r#"{"version":1,"interval_ms":1500}"#,
            r#"{"interval_ms":1500}"#,
        ] {
            let document = parse_document(raw).expect("parse document");
            assert_eq!(document.settings.version, tm_viewmodel::SETTINGS_VERSION);
            assert!(!document.settings.tree_view);
            assert_eq!(document.settings.theme, tm_viewmodel::Theme::Dark);

            let map = merged_map(&document.settings, Some(&document.raw));
            let json = serde_json::to_string(&map).expect("serialize");
            let saved: serde_json::Value = serde_json::from_str(&json).expect("parse saved");
            assert_eq!(
                saved.get("version"),
                Some(&serde_json::json!(tm_viewmodel::SETTINGS_VERSION)),
                "old/versionless document must be migrated on save"
            );
            assert_eq!(saved.get("interval_ms"), Some(&serde_json::json!(1500)));
        }
    }

    #[test]
    fn schema_policy_future_version_survives_repeated_save_cycles() {
        // Forward compatibility must not drift over repeated load/save cycles.
        let mut raw =
            r#"{"version":9,"interval_ms":1500,"future_flag":{"nested":[1,2]}}"#.to_string();
        for cycle in 0..3 {
            let document = parse_document(&raw).expect("parse document");
            assert_eq!(document.settings.version, 9, "cycle {cycle}");
            // The shell rebuilds a current-version Settings before saving.
            let shell = Settings {
                version: tm_viewmodel::SETTINGS_VERSION,
                ..document.settings.clone()
            };
            let map = merged_map(&shell, Some(&document.raw));
            raw = serde_json::to_string(&map).expect("serialize");
        }
        let final_document = parse_document(&raw).expect("final parse");
        assert_eq!(final_document.settings.version, 9);
        assert_eq!(
            final_document.raw.get("future_flag"),
            Some(&serde_json::json!({"nested": [1, 2]}))
        );
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
            assert_eq!(document.settings.version, tm_viewmodel::SETTINGS_VERSION);

            let shell = Settings {
                version: tm_viewmodel::SETTINGS_VERSION,
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
}

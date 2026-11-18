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
//! downgrades a newer `version`. Corrupt input yields a fixed, redacted
//! [`SettingsNotice`] and, on wasm, a separate DOM notice node. Neither the
//! `body[data-gpui-proof]` hook nor the existing `aria-live` mirror is touched.
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

use std::sync::Mutex;

use tm_viewmodel::Settings;

/// Key used in `localStorage`.
pub const STORAGE_KEY: &str = "gpui-task-manager.settings";

/// DOM id of the additive configuration-notice node.
///
/// It is deliberately distinct from `body[data-gpui-proof]` and the shell's
/// `status-live` mirror so the CDP proof hook and redaction path stay intact.
pub const SETTINGS_NOTICE_DOM_ID: &str = "gpui-settings-notice";

/// Fixed, redacted text shown when a stored settings value is corrupt.
pub const NOTICE_CORRUPT: &str =
    "保存された設定を読み込めませんでした。既定値で起動します（破損した設定は保存されません）。";

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
    let mut value: serde_json::Value = serde_json::from_str(raw).ok()?;
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

/// Raw-injected loading, shared by [`load_with_notice`] and the tests
/// (no browser needed).
fn load_from_raw(raw: Option<&str>) -> Settings {
    let Some(raw) = raw else {
        return Settings::default();
    };
    match parse_document(raw) {
        Some(document) => {
            *lock(&LAST_LOADED) = Some(document.raw);
            *lock(&PENDING_NOTICE) = None;
            document.settings
        }
        None => {
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
        let _ = element.set_text_content(Some(&notice.text));
    }
}

/// Native builds have no DOM; notice publication is a no-op there.
#[cfg(not(target_family = "wasm"))]
fn publish_notice_dom(_notice: &SettingsNotice) {}

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

/// Persists settings. Returns an error message on failure so the caller can
/// surface it, but callers are free to ignore it.
///
/// Unknown fields and a newer `version` from the last load are preserved.
pub fn save(settings: &Settings) -> Result<(), String> {
    let storage = storage().ok_or_else(|| "localStorage を利用できません".to_string())?;
    let raw = lock(&LAST_LOADED).clone();
    let map = merged_map(settings, raw.as_ref());
    let json = serde_json::to_string(&map)
        .map_err(|error| format!("設定を JSON に変換できません: {error}"))?;
    storage
        .set_item(STORAGE_KEY, &json)
        .map_err(|_| "設定を保存できません（ストレージ容量または権限）".to_string())?;
    *lock(&LAST_LOADED) = Some(map);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tm_core::{SortDirection, SortKey, SortSpec};

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
    fn corrupt_notice_is_redacted_and_drained_once() {
        // All assertions about the process-wide notice live in one test: the
        // test runner is parallel and these globals must not race.
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

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

use tm_viewmodel::Settings;

/// Key used in `localStorage`.
pub const STORAGE_KEY: &str = "gpui-task-manager.settings";

/// 更新間隔の UI プリセット（ミリ秒）。ネイティブ版と同じ 4 段階を使う。
///
/// ここで定義してシェルから参照することで、ボタンの候補と保存される
/// `settings.interval_ms` がずれないようにする。値はすべて
/// [`tm_viewmodel::MIN_REFRESH_INTERVAL`]〜[`MAX_REFRESH_INTERVAL`] の範囲内。
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

/// Returns the `localStorage` handle, if the browser exposes one.
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// Loads settings, falling back to defaults when storage is unavailable or the
/// stored value is not valid JSON.
pub fn load() -> Settings {
    let Some(storage) = storage() else {
        return Settings::default();
    };
    let Ok(Some(raw)) = storage.get_item(STORAGE_KEY) else {
        return Settings::default();
    };
    parse_saved(&raw)
}

/// Parses a persisted settings string, falling back to defaults on bad JSON.
///
/// Kept pure so the fallback path can be tested without a browser context.
fn parse_saved(raw: &str) -> Settings {
    match serde_json::from_str::<Settings>(raw) {
        Ok(settings) => settings.normalized(),
        Err(error) => {
            log::warn!("保存された設定を読み込めません（既定値を使います）: {error}");
            Settings::default()
        }
    }
}

/// Persists settings. Returns an error message on failure so the caller can
/// surface it, but callers are free to ignore it.
pub fn save(settings: &Settings) -> Result<(), String> {
    let Some(storage) = storage() else {
        return Err("localStorage を利用できません".to_string());
    };
    let json = serde_json::to_string(&settings.clone().normalized())
        .map_err(|error| format!("設定を JSON に変換できません: {error}"))?;
    storage
        .set_item(STORAGE_KEY, &json)
        .map_err(|_| "設定を保存できません（ストレージ容量または権限）".to_string())
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
}

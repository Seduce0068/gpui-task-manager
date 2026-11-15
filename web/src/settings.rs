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
    match serde_json::from_str::<Settings>(&raw) {
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
        };
        let json = serde_json::to_string(&settings).expect("serialize");
        let restored: Settings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, settings);
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
}

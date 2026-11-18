//! Persisted presentation preferences, shared by the native and browser shells.
//!
//! Only presentation state is stored: refresh interval, sort key/direction and
//! details-panel visibility. Process data, filters, host names and tokens are
//! deliberately **not** persisted.
//!
//! The shells own the storage medium (a JSON file on native, `localStorage` in
//! the browser); this module owns the schema so both stay compatible.

use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize};
use tm_core::SortSpec;

use crate::{MAX_REFRESH_INTERVAL, MIN_REFRESH_INTERVAL};

/// Deserialises a known field tolerantly: a newer build may write an unknown
/// enum variant or a partial nested struct. Falling back to the default keeps
/// the rest of the document (including a future `version` and unknown fields)
/// intact instead of marking the whole settings file corrupt (R34 audit).
fn lenient<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// Available colour themes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Theme {
    /// Dark theme (Catppuccin Mocha).
    #[default]
    Dark,
    /// Light theme (Catppuccin Latte).
    Light,
}

/// Current settings schema version.
pub const SETTINGS_VERSION: u32 = 2;

/// Persisted presentation preferences.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Schema version, so future migrations can recognise old files.
    pub version: u32,
    /// Refresh interval in milliseconds.
    pub interval_ms: u64,
    /// Sort column and direction.
    #[serde(deserialize_with = "lenient")]
    pub sort: SortSpec,
    /// Whether the details panel is visible.
    pub details_visible: bool,
    /// Whether the process table is shown as a process tree.
    pub tree_view: bool,
    /// Colour theme.
    #[serde(deserialize_with = "lenient")]
    pub theme: Theme,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            interval_ms: 1_000,
            sort: SortSpec::default(),
            details_visible: true,
            tree_view: false,
            theme: Theme::Dark,
        }
    }
}

impl Settings {
    /// Clamps values into the range the view-model accepts.
    pub fn normalized(mut self) -> Self {
        self.version = SETTINGS_VERSION;
        let minimum = MIN_REFRESH_INTERVAL.as_millis() as u64;
        let maximum = MAX_REFRESH_INTERVAL.as_millis() as u64;
        self.interval_ms = self.interval_ms.clamp(minimum, maximum);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tm_core::{SortDirection, SortKey};

    #[test]
    fn unknown_enum_values_do_not_invalidate_the_document() {
        // A future build may write enum values this build does not know. The
        // whole document must still parse so a future `version` and unknown
        // top-level fields survive the next save (R34 audit).
        let restored: Settings = serde_json::from_str(
            r#"{"version":9,"interval_ms":1500,"sort":{"key":"FutureKey","direction":"Ascending"},"theme":"FutureTheme","future":42}"#,
        )
        .expect("future enum values must not reject the settings");
        assert_eq!(restored.version, 9);
        assert_eq!(restored.interval_ms, 1500);
        assert_eq!(restored.sort, SortSpec::default());
        assert_eq!(restored.theme, Theme::default());

        // A partial nested struct (missing `direction`) is tolerated too.
        let partial: Settings =
            serde_json::from_str(r#"{"sort":{"key":"Pid"}}"#).expect("partial sort");
        assert_eq!(partial.sort, SortSpec::default());
    }

    #[test]
    fn defaults_match_the_documented_values() {
        let settings = Settings::default();
        assert_eq!(settings.version, SETTINGS_VERSION);
        assert_eq!(settings.interval_ms, 1_000);
        assert_eq!(settings.sort.key, SortKey::Cpu);
        assert!(settings.details_visible);
        assert!(!settings.tree_view);
        assert_eq!(settings.theme, Theme::Dark);
    }

    #[test]
    fn normalization_clamps_the_interval_and_sets_the_version() {
        let settings = Settings {
            version: 0,
            interval_ms: 1,
            ..Settings::default()
        }
        .normalized();
        assert_eq!(settings.version, SETTINGS_VERSION);
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
    fn json_round_trip_keeps_the_schema_stable() {
        let settings = Settings {
            version: SETTINGS_VERSION,
            interval_ms: 2_000,
            sort: SortSpec {
                key: SortKey::Memory,
                direction: SortDirection::Descending,
            },
            details_visible: false,
            tree_view: true,
            theme: Theme::Light,
        };
        let json = serde_json::to_string(&settings).expect("serialize");
        assert!(json.contains("\"interval_ms\":2000"), "{json}");
        assert!(json.contains("\"tree_view\":true"), "{json}");
        let restored: Settings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, settings);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let restored: Settings = serde_json::from_str("{\"interval_ms\":3000}").expect("partial");
        assert_eq!(restored.interval_ms, 3_000);
        assert_eq!(restored.sort, SortSpec::default());
        assert!(restored.details_visible);
        // Fields added in schema v2 must fall back too (v1 files have neither).
        assert!(!restored.tree_view);
        assert_eq!(restored.theme, Theme::Dark);
    }

    #[test]
    fn unknown_fields_are_ignored_for_forward_compatibility() {
        let restored: Settings = serde_json::from_str(
            "{\"version\":1,\"interval_ms\":1500,\"sort\":{\"key\":\"Pid\",\"direction\":\"Ascending\"},\"details_visible\":false,\"future\":42}",
        )
        .expect("forward compatible");
        assert_eq!(restored.interval_ms, 1_500);
        assert!(!restored.details_visible);
    }
}

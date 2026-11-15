//! Persisted presentation preferences, shared by the native and browser shells.
//!
//! Only presentation state is stored: refresh interval, sort key/direction and
//! details-panel visibility. Process data, filters, host names and tokens are
//! deliberately **not** persisted.
//!
//! The shells own the storage medium (a JSON file on native, `localStorage` in
//! the browser); this module owns the schema so both stay compatible.

use serde::{Deserialize, Serialize};
use tm_core::SortSpec;

use crate::{MAX_REFRESH_INTERVAL, MIN_REFRESH_INTERVAL};

/// Current settings schema version.
pub const SETTINGS_VERSION: u32 = 1;

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
    /// Whether the process table is shown as a process tree.
    pub tree_view: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            interval_ms: 1_000,
            sort: SortSpec::default(),
            details_visible: true,
            tree_view: false,
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
    fn defaults_match_the_documented_values() {
        let settings = Settings::default();
        assert_eq!(settings.version, SETTINGS_VERSION);
        assert_eq!(settings.interval_ms, 1_000);
        assert_eq!(settings.sort.key, SortKey::Cpu);
        assert!(settings.details_visible);
        assert!(!settings.tree_view);
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
            tree_view: false,
        };
        let json = serde_json::to_string(&settings).expect("serialize");
        assert!(json.contains("\"interval_ms\":2000"), "{json}");
        let restored: Settings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, settings);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let restored: Settings = serde_json::from_str("{\"interval_ms\":3000}").expect("partial");
        assert_eq!(restored.interval_ms, 3_000);
        assert_eq!(restored.sort, SortSpec::default());
        assert!(restored.details_visible);
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

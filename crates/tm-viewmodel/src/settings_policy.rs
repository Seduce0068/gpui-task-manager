//! Pure settings-storage policy shared by the native and browser shells.
//!
//! The shells own the storage medium (a JSON file on native, `localStorage` in
//! the browser), the byte-size gate and the fixed, redacted notice text. This
//! module owns the policy that must not drift between them: BOM handling,
//! interval clamping, version interpretation, document parsing, the three-way
//! merge that keeps unknown fields and future versions intact, and the shape of
//! a configuration notice (R2-D01 / R2X-D3 / HB-NATIVE / HA-WEBMERGE /
//! C2-SETTINGSSHARE).
//!
//! Keeping this logic here means a compatibility fix (for example restoring a
//! known field that disappeared from the destination) needs one edit and one
//! golden vector instead of two divergent shell implementations.

use serde_json::{Map, Value};

use crate::settings::Settings;
use crate::settings::SETTINGS_VERSION;
use crate::{MAX_REFRESH_INTERVAL, MIN_REFRESH_INTERVAL};

/// Largest settings document either shell will accept, in UTF-8 bytes (4 MiB).
///
/// A valid document is a handful of keys (well under 1 KiB). Refusing a much
/// larger value keeps a corrupt or hostile document from being loaded into
/// memory unbounded. The native shell applies this while reading bytes, the
/// browser shell to the stored string length; both use this one constant.
pub const MAX_SETTINGS_BYTES: u64 = 4 * 1024 * 1024;

/// Maximum number of corrupt backups a shell keeps before pruning the oldest.
///
/// The native shell prunes renamed files, the browser shell prunes quarantined
/// keys; the retention count is shared policy (H1-BACKUP).
pub const CORRUPT_BACKUP_LIMIT: usize = 5;

/// Removes exactly one leading UTF-8 BOM (EF BB BF, decoded as `U+FEFF`).
///
/// Windows editors commonly prepend a BOM to JSON files. `serde_json` rejects a
/// leading `U+FEFF`, which would make an otherwise valid document look corrupt.
/// A BOM anywhere else stays a parse error (PC-CSVBOM).
pub fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// Clamps `interval_ms` into the supported range without rewriting `version`.
///
/// [`Settings::normalized`] always forces the current version; storage must not
/// do that or a newer document would be downgraded on the next save (R2-D01).
pub(crate) fn clamp_interval_only(mut settings: Settings) -> Settings {
    // A document from a future schema may legitimately use a wider interval
    // range. Clamping it to this build's bounds and then saving would persist a
    // version:N file whose value no longer means what N expects (R1-FWDCOMPAT).
    // Leave future-version values untouched; the current build treats them as
    // opaque for this field only.
    if settings.version <= SETTINGS_VERSION {
        settings.interval_ms = settings.interval_ms.clamp(
            MIN_REFRESH_INTERVAL.as_millis() as u64,
            MAX_REFRESH_INTERVAL.as_millis() as u64,
        );
    }
    settings
}

/// Interprets a persisted `version` under the compatibility policy.
///
/// Only a non-negative integer that fits in `u32` is comparable here. Other JSON
/// forms (string, fractional, negative, larger than `u32::MAX`) are opaque: they
/// stay in the raw object and are written back verbatim instead of being
/// replaced with a version this build understands (R2X-D3).
pub(crate) fn interpretable_version(value: Option<&Value>) -> Option<u32> {
    value
        .and_then(|value| value.as_u64())
        .and_then(|version| u32::try_from(version).ok())
}

/// Merges the `version` field of `value` into `map` under the shared policy.
///
/// A comparable stored version is upgraded but never downgraded; a missing
/// version is filled in with this build's version; an opaque or out-of-range
/// value is left untouched so a newer shell still recognises its own document.
pub(crate) fn merge_version(map: &mut Map<String, Value>, value: Value) {
    match interpretable_version(map.get("version")) {
        Some(stored) => {
            let current = value.as_u64().unwrap_or(0);
            map.insert(
                "version".to_string(),
                Value::from((stored as u64).max(current)),
            );
        }
        None if !map.contains_key("version") => {
            map.insert("version".to_string(), value);
        }
        None => {}
    }
}

/// Parsed settings plus the untouched JSON object they came from.
///
/// Keeping `raw` is what makes unknown fields and a future `version` survive a
/// later save instead of being dropped by an older shell (R2-D01 / R5-D02).
#[derive(Clone, Debug)]
pub struct SettingsDocument {
    /// Parsed, clamped settings carrying the effective (never downgraded) version.
    pub settings: Settings,
    /// The original JSON object, including unknown fields and the stored version.
    pub raw: Map<String, Value>,
}

/// Parses a persisted JSON document, keeping its raw object for round-tripping.
///
/// An older `version` is upgraded to [`SETTINGS_VERSION`], a newer one is kept.
/// A version this build cannot represent (e.g. a `u64` from a future schema) is
/// treated as opaque and does not make the whole document look corrupt.
///
/// # Size-limit boundary
///
/// The caller must have applied [`MAX_SETTINGS_BYTES`] to the input first: the
/// native shell caps the byte read before decoding UTF-8, and the browser shell
/// checks the stored string length. This function therefore only strips a BOM
/// and parses; it never allocates for an over-limit document.
pub fn parse_document(text: &str) -> Option<SettingsDocument> {
    let mut value: Value = serde_json::from_str(strip_bom(text)).ok()?;
    let raw = value.as_object()?.clone();
    let stored_version = interpretable_version(raw.get("version"));

    // `Settings` cannot represent a version it does not know, but that must not
    // make the whole file look corrupt: substitute this build's version before
    // deserialising and restore the effective version afterwards.
    if let Some(object) = value.as_object_mut() {
        object.insert("version".to_string(), Value::from(SETTINGS_VERSION));
    }
    let mut settings: Settings = serde_json::from_value(value).ok()?;
    settings = clamp_interval_only(settings);
    settings.version = stored_version
        .unwrap_or(SETTINGS_VERSION)
        .max(SETTINGS_VERSION);
    Some(SettingsDocument { settings, raw })
}

/// Three-way merge of the JSON object to save.
///
/// `baseline` is the raw object observed at this instance's last load/save,
/// `settings` is the caller's current view ("ours") and `raw` is the freshest
/// object read from the destination ("theirs"). A known field whose value still
/// equals the baseline was not edited here, so the concurrent value on disk is
/// kept; a field that differs (or that the destination dropped entirely) is
/// written from `settings`. Dropping a known field is repaired from our side so
/// a newer field is not lost while migrating an older document. Unknown fields
/// come from `raw`, and `version` keeps its max/verbatim policy.
///
/// Passing `None` as `baseline` reproduces the overwrite-all behaviour: with no
/// earlier snapshot every known field is treated as edited.
pub fn merged_map_with_baseline(
    settings: &Settings,
    baseline: Option<&Map<String, Value>>,
    raw: Option<&Map<String, Value>>,
) -> Map<String, Value> {
    let mut map = raw.cloned().unwrap_or_default();
    let known = serde_json::to_value(clamp_interval_only(settings.clone()))
        .expect("Settings always serializes to a JSON object");
    let Value::Object(known) = known else {
        return map;
    };
    let empty = Map::new();
    let base = baseline.unwrap_or(&empty);
    for (key, value) in known {
        if key == "version" {
            merge_version(&mut map, value);
            continue;
        }
        // Unchanged relative to our base AND still present on disk: keep what
        // another instance wrote. A destination that lost the key gets ours
        // written back (native behaviour, also adopted by the browser shell).
        if base.get(&key).is_some_and(|old| old == &value) && map.contains_key(&key) {
            continue;
        }
        map.insert(key, value);
    }
    map
}

/// A redacted configuration notice safe to pass to `Dashboard::set_persistent_status`.
///
/// # Redaction contract
///
/// `text` must be one of the shell's fixed constants; it never contains a file
/// path, host name, token or raw parser/I/O error. [`Self::new`] only accepts a
/// `&'static str`, so a dynamically formatted error cannot enter the notice by
/// construction. Both settings problems (corrupt load, failed save) are
/// persistent errors, so the flags need no per-shell decision. The shells keep
/// their own wording and, for web, its save-failure announcement guard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsNotice {
    /// Fixed, redacted status text.
    pub text: String,
    /// Always `true`: a settings problem is an error.
    pub is_error: bool,
    /// Always `true`: a startup notice must survive the next sample.
    pub persistent: bool,
}

impl SettingsNotice {
    /// Builds a notice from fixed, redacted text.
    pub fn new(text: &'static str) -> Self {
        Self {
            text: text.to_string(),
            is_error: true,
            persistent: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tm_core::{SortDirection, SortKey};

    const CURRENT: u32 = SETTINGS_VERSION;

    fn settings_json(interval_ms: u64, theme: &str) -> Value {
        serde_json::json!({
            "version": CURRENT,
            "interval_ms": interval_ms,
            "sort": {"key": "Cpu", "direction": "Descending"},
            "details_visible": true,
            "tree_view": false,
            "theme": theme,
        })
    }

    fn as_map(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            other => panic!("expected an object, got {other}"),
        }
    }

    #[test]
    fn golden_version_upgrade_and_versionless_parse() {
        // Backward: an older schema and a versionless document both load with
        // defaults for the missing fields and the current version.
        for raw in [
            r#"{"version":1,"interval_ms":1500,"details_visible":false}"#,
            r#"{"interval_ms":1500,"details_visible":false}"#,
        ] {
            let document = parse_document(raw).expect("old/versionless document must load");
            assert_eq!(document.settings.version, CURRENT);
            assert_eq!(document.settings.interval_ms, 1_500);
            assert!(!document.settings.details_visible);
            assert!(!document.settings.tree_view);
            assert_eq!(document.settings.theme, crate::Theme::Dark);
            // A save records the migrated version.
            let map = merged_map_with_baseline(&document.settings, None, Some(&document.raw));
            assert_eq!(map.get("version"), Some(&Value::from(CURRENT)));
        }
    }

    #[test]
    fn golden_unknown_fields_and_future_version_round_trip() {
        let raw = r#"{"version":9,"interval_ms":1500,"sort":{"key":"Pid","direction":"Ascending"},"details_visible":false,"tree_view":true,"theme":"Light","future_flag":42,"future_nested":{"a":[1,2]}}"#;
        let mut text = raw.to_string();
        for cycle in 0..3 {
            let document = parse_document(&text).expect("future document must load");
            assert_eq!(document.settings.version, 9, "cycle {cycle}");
            assert_eq!(document.settings.interval_ms, 1_500);
            assert_eq!(document.settings.sort.key, SortKey::Pid);
            assert_eq!(document.settings.sort.direction, SortDirection::Ascending);
            assert_eq!(document.settings.theme, crate::Theme::Light);
            let caller = Settings {
                version: CURRENT,
                ..document.settings.clone()
            };
            let map = merged_map_with_baseline(&caller, Some(&document.raw), Some(&document.raw));
            text = serde_json::to_string(&map).expect("serialize");
        }
        let final_document = parse_document(&text).expect("final parse");
        assert_eq!(
            final_document.settings.version, 9,
            "future version downgraded"
        );
        assert_eq!(
            final_document.raw.get("future_flag"),
            Some(&serde_json::json!(42))
        );
        assert_eq!(
            final_document.raw.get("future_nested"),
            Some(&serde_json::json!({"a": [1, 2]}))
        );
    }

    #[test]
    fn golden_schema_policy_future_version_is_never_downgraded() {
        let document = parse_document(r#"{"version":9,"interval_ms":1500}"#)
            .expect("future version must load");
        assert_eq!(document.settings.version, 9);
        assert_eq!(interpretable_version(document.raw.get("version")), Some(9));

        // A caller holding the current version cannot lower the stored one,
        // including when the destination is re-read with a newer value.
        let caller = Settings {
            version: CURRENT,
            ..document.settings.clone()
        };
        let map = merged_map_with_baseline(&caller, None, Some(&document.raw));
        assert_eq!(map.get("version"), Some(&Value::from(9u64)));
    }

    #[test]
    fn golden_schema_policy_opaque_version_forms_are_preserved_verbatim() {
        for raw_version in [
            serde_json::json!("9"),
            serde_json::json!(9.5),
            serde_json::json!(-1),
            serde_json::json!(u64::from(u32::MAX) + 1),
        ] {
            let mut value = serde_json::json!({"interval_ms": 1500, "future_flag": 1});
            value["version"] = raw_version.clone();
            let document = parse_document(&value.to_string()).expect("opaque version must load");
            assert_eq!(document.settings.version, CURRENT);
            assert_eq!(interpretable_version(document.raw.get("version")), None);

            let caller = Settings {
                version: CURRENT,
                ..document.settings.clone()
            };
            let map = merged_map_with_baseline(&caller, Some(&document.raw), Some(&document.raw));
            assert_eq!(map.get("version"), Some(&raw_version), "rewritten");
            assert_eq!(map.get("future_flag"), Some(&serde_json::json!(1)));
        }
    }

    #[test]
    fn golden_merged_map_without_a_baseline_overwrites_all_known_fields() {
        let settings = Settings {
            interval_ms: 20_000,
            ..Settings::default()
        };
        let map = merged_map_with_baseline(&settings, None, None);
        assert_eq!(map.get("version"), Some(&Value::from(CURRENT)));
        // Out-of-range values are clamped, not stored verbatim.
        assert_eq!(map.get("interval_ms"), Some(&serde_json::json!(10_000)));
        assert_ne!(map.get("interval_ms"), Some(&serde_json::json!(20_000)));
    }

    #[test]
    fn golden_three_way_merge_keeps_concurrent_edits_to_different_fields() {
        let shared = Settings {
            version: CURRENT,
            interval_ms: 1_500,
            theme: crate::Theme::Light,
            ..Settings::default()
        };
        let mut base = as_map(serde_json::to_value(&shared).expect("serialize"));
        base.insert("version".to_string(), Value::from(9u64));
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
            theme: crate::Theme::Dark,
            ..shared
        };
        let after_b = merged_map_with_baseline(&instance_b, Some(&base), Some(&after_a));
        assert_eq!(after_b.get("interval_ms"), Some(&serde_json::json!(2_000)));
        assert_eq!(after_b.get("theme"), Some(&serde_json::json!("Dark")));
        assert_eq!(after_b.get("version"), Some(&Value::from(9u64)));
        assert_eq!(
            after_b.get("future_flag"),
            Some(&serde_json::json!({"nested": [1, 2]}))
        );
    }

    #[test]
    fn golden_missing_known_field_is_restored_from_ours() {
        let settings = Settings {
            version: CURRENT,
            interval_ms: 1_500,
            theme: crate::Theme::Light,
            ..Settings::default()
        };
        let base = as_map(serde_json::to_value(&settings).expect("serialize"));

        // base == ours and the destination still has the key: keep theirs.
        let mut theirs = base.clone();
        theirs.insert("theme".to_string(), Value::from("Dark"));
        let kept = merged_map_with_baseline(&settings, Some(&base), Some(&theirs));
        assert_eq!(kept.get("theme"), Some(&serde_json::json!("Dark")));

        // base == ours but the destination dropped the key: write ours back.
        let mut missing = base.clone();
        missing.remove("theme");
        let restored = merged_map_with_baseline(&settings, Some(&base), Some(&missing));
        assert_eq!(restored.get("theme"), Some(&serde_json::json!("Light")));
        assert_eq!(restored.get("interval_ms"), Some(&serde_json::json!(1_500)));

        // Several fields dropped at once are all restored.
        let mut stripped = base.clone();
        stripped.remove("interval_ms");
        stripped.remove("details_visible");
        stripped.remove("tree_view");
        let restored = merged_map_with_baseline(&settings, Some(&base), Some(&stripped));
        assert_eq!(restored.get("interval_ms"), Some(&serde_json::json!(1_500)));
        assert_eq!(restored.get("details_visible"), Some(&Value::Bool(true)));
        assert_eq!(restored.get("tree_view"), Some(&Value::Bool(false)));
    }

    #[test]
    fn golden_bom_is_stripped_once_and_elsewhere_is_a_parse_error() {
        let json = settings_json(1_500, "Light").to_string();
        assert_eq!(strip_bom(&format!("\u{feff}{json}")), json);
        assert_eq!(strip_bom(&json), json);

        let document = parse_document(&format!("\u{feff}{json}")).expect("BOM must parse");
        assert_eq!(document.settings.interval_ms, 1_500);
        assert_eq!(document.settings.theme, crate::Theme::Light);
        assert!(parse_document(&format!("{json}\u{feff}")).is_none());
        assert!(parse_document(&format!("\u{feff}\u{feff}{json}")).is_none());
    }

    #[test]
    fn golden_clamp_interval_only_preserves_the_stored_version() {
        // Current-schema values are clamped into the supported range.
        let low = Settings {
            version: SETTINGS_VERSION,
            interval_ms: 1,
            ..Settings::default()
        };
        let clamped = clamp_interval_only(low);
        assert_eq!(clamped.version, SETTINGS_VERSION);
        assert_eq!(clamped.interval_ms, MIN_REFRESH_INTERVAL.as_millis() as u64);

        let high = Settings {
            version: SETTINGS_VERSION,
            interval_ms: u64::MAX,
            ..Settings::default()
        };
        let clamped = clamp_interval_only(high);
        assert_eq!(clamped.version, SETTINGS_VERSION);
        assert_eq!(clamped.interval_ms, MAX_REFRESH_INTERVAL.as_millis() as u64);

        // A future schema may define a wider range: leave its value verbatim
        // so saving does not silently reinterpret it under v2 bounds.
        let future = Settings {
            version: 9,
            interval_ms: 20_000,
            ..Settings::default()
        };
        let kept = clamp_interval_only(future);
        assert_eq!(kept.version, 9);
        assert_eq!(kept.interval_ms, 20_000);
    }

    #[test]
    fn golden_notice_shape_is_fixed_and_redacted() {
        let notice = SettingsNotice::new("固定された文本");
        assert_eq!(notice.text, "固定された文本");
        assert!(notice.is_error);
        assert!(notice.persistent);
        assert_eq!(MAX_SETTINGS_BYTES, 4 * 1024 * 1024);
        assert_eq!(CORRUPT_BACKUP_LIMIT, 5);
    }

    #[test]
    fn malformed_and_non_object_documents_are_rejected() {
        assert!(parse_document("{ not json").is_none());
        assert!(parse_document("[1,2,3]").is_none());
        assert!(parse_document("null").is_none());
        // A wrong type in a known field is corruption; an unknown key is not.
        assert!(parse_document(r#"{"interval_ms":"oops"}"#).is_none());
        assert!(parse_document(r#"{"future":1}"#).is_some());
    }
}

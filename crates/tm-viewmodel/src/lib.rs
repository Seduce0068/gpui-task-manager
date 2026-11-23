//! UI-independent presentation state machine.
//!
//! The native GPUI shell and the browser shell both drive this same type. It
//! owns the "what is on screen" state (sorting, filtering, selection, dialog,
//! refresh cadence) and emits [`Effect`] values that the shell executes
//! (`sample now`, `send a signal`). Because there is no I/O here, every
//! interaction is a deterministic unit test.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::collections::{BTreeSet, HashSet};
use std::time::Duration;

use tm_core::format;
// `app_group_key` lives in `tm_core::model`; importing through the module keeps
// this change inside the assigned files (no core `lib.rs` re-export needed).
use tm_core::model::app_group_key;
use tm_core::{
    filter_and_sort, flatten_process_tree, group_by_app, normalise_percent, Filter, MetricHistory,
    NetworkCounters, PlatformInfo, ProcessGroup, ProcessKey, ProcessSnapshot, ProcessStatus,
    ProcessTreeRow, RingBuffer, Signal, SortKey, SortSpec, SystemSnapshot, Timestamp,
};

pub mod filter_input;
pub mod settings;
pub mod settings_policy;

pub use filter_input::{EditOutcome, FilterInput};
pub use settings::{Settings, Theme, SETTINGS_VERSION};
pub use settings_policy::{
    merged_map_with_baseline, parse_document, strip_bom, SettingsDocument, SettingsNotice,
    CORRUPT_BACKUP_LIMIT, MAX_SETTINGS_BYTES,
};

/// Lowest refresh interval the UI accepts (250 ms).
pub const MIN_REFRESH_INTERVAL: Duration = Duration::from_millis(250);
/// Highest refresh interval the UI accepts (10 s).
pub const MAX_REFRESH_INTERVAL: Duration = Duration::from_secs(10);
/// Default refresh interval (1 s).
pub const DEFAULT_REFRESH_INTERVAL: Duration = Duration::from_millis(1_000);
/// Number of chart samples kept (two minutes at the default cadence).
pub const HISTORY_CAPACITY: usize = 120;

/// Reserved expansion key for the collapsed "other applications" row (C1-COLLAPSE).
///
/// A canonical application key from [`app_group_key`] can never start with a
/// control character: the executable-name validator rejects them, and a
/// process `comm` is NUL-terminated. The leading NUL therefore keeps this bucket
/// key disjoint from every real application key, including keys that reach
/// [`app_group_key`] through its `ProcessSnapshot::name` fallback. Shells must
/// pass this value back unchanged to [`Command::ToggleAppGroup`]; it is an
/// identity token, not display text, so it must never be written to the DOM.
pub const OTHERS_APP_GROUP_KEY: &str = "\0others";

/// Fixed, already-redacted label for the "others" bucket row (C1-COLLAPSE).
///
/// Shells render their own localised text (e.g. `その他 N アプリ`) from
/// [`AppGroupRow::process_count`]; this value is only a stable, non-sensitive
/// fallback for consumers that need a label string.
pub const OTHERS_APP_GROUP_LABEL: &str = "others";

/// Fewest single-process application groups needed to form an "others" bucket.
///
/// Two or more one-process apps are replaced by one summary row, but only when
/// at least one multi-process group remains visible: folding every app would
/// leave a lone "others" row and erase the ordinary list (C1-COLLAPSE).
pub const OTHERS_MIN_GROUP_COUNT: usize = 2;

/// Reserved identity key for the "アプリ" section heading row (C1-VMCLASSIFY).
///
/// The leading NUL keeps it disjoint from every canonical [`app_group_key`] and
/// from [`OTHERS_APP_GROUP_KEY`]. Like the others key it is an identity token,
/// never display text, and shells must not feed it back into a command.
pub const APPS_SECTION_KEY: &str = "\0section-apps";
/// Reserved identity key for the "バックグラウンド プロセス" section heading row (C1-VMCLASSIFY).
///
/// Disjoint from every real application key and from [`APPS_SECTION_KEY`] by the
/// same leading-NUL convention. Display-only; never sent to a shell command.
pub const BACKGROUND_SECTION_KEY: &str = "\0section-background";
/// Japanese label of the application section heading (C1-VMCLASSIFY).
pub const APPS_SECTION_LABEL: &str = "アプリ";
/// Japanese label of the background-process section heading (C1-VMCLASSIFY).
pub const BACKGROUND_SECTION_LABEL: &str = "バックグラウンド プロセス";

/// Text marker prepended to informational status messages (R3-U04).
///
/// The status bar is colour-coded in both shells; the marker keeps the
/// severity readable in monochrome and for colour-vision-deficient users.
pub const STATUS_PREFIX_INFO: &str = "[情報]";
/// Text marker prepended to error status messages (R3-U04).
pub const STATUS_PREFIX_ERROR: &str = "[エラー]";

/// Dead-band for percentage trends: changes of at most 0.05 points are noise
/// from sampling jitter rather than a real change in utilisation (R5-I05).
const PERCENT_TREND_EPSILON: f64 = 0.05;

/// Direction of a metric trend between the oldest and newest history samples
/// (R5-I05).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrendDirection {
    /// Fewer than two samples, or a non-finite value: no judgement possible.
    Unknown,
    /// The change is within the noise threshold.
    Flat,
    /// The newest value is higher than the oldest.
    Up,
    /// The newest value is lower than the oldest.
    Down,
}

impl TrendDirection {
    /// Monochrome-safe cue shown in the visible badge text.
    pub fn cue(self) -> &'static str {
        match self {
            Self::Unknown => "—",
            Self::Flat => "→",
            Self::Up => "↑",
            Self::Down => "↓",
        }
    }

    /// Japanese state label shared by the visible badge and its accessible name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "不明",
            Self::Flat => "横ばい",
            Self::Up => "上昇",
            Self::Down => "下降",
        }
    }
}

/// Unit used when rendering a trend delta's amount (R5-I05).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrendUnit {
    /// Percentage points (CPU, memory).
    PercentPoints,
    /// Bytes per second (network rates).
    BytesPerSecond,
}

/// A trend delta between the oldest and newest samples of one metric series.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrendDelta {
    /// Direction of the change.
    pub direction: TrendDirection,
    /// Signed amount with its unit, e.g. `+1.2pt` or `-320 B/s`.
    pub amount: String,
}

impl TrendDelta {
    /// Visible badge text: cue, signed amount and Japanese state label.
    ///
    /// The label is part of the visible text so the state never depends on
    /// colour perception; the cue keeps it scannable in a narrow badge.
    pub fn badge_text(&self) -> String {
        match self.direction {
            TrendDirection::Unknown => "— 不明".to_string(),
            direction => format!("{} {} {}", direction.cue(), self.amount, direction.label()),
        }
    }

    /// Accessible name (`aria-label` equivalent) for the trend badge.
    pub fn a11y_label(&self, metric: &str) -> String {
        match self.direction {
            TrendDirection::Unknown => format!("{metric} の推移 不明"),
            direction => format!("{metric} の推移 {} {}", direction.label(), self.amount),
        }
    }
}

/// Computes a trend delta from two samples of one history series (R5-I05).
///
/// `oldest` and `latest` are the values returned by the corresponding ring
/// buffer's `earliest()` / `latest()`. Missing or non-finite values produce
/// [`TrendDirection::Unknown`]; `epsilon` is the dead-band that turns tiny
/// floating-point drift into [`TrendDirection::Flat`] (pass `0.0` for integer
/// byte rates). `unit` only affects the rendered [`TrendDelta::amount`], never
/// the direction decision.
pub fn trend_delta(
    oldest: Option<f64>,
    latest: Option<f64>,
    epsilon: f64,
    unit: TrendUnit,
) -> TrendDelta {
    let (Some(oldest), Some(latest)) = (oldest, latest) else {
        return unknown_trend();
    };
    if !oldest.is_finite() || !latest.is_finite() {
        return unknown_trend();
    }
    let epsilon = if epsilon.is_finite() && epsilon >= 0.0 {
        epsilon
    } else {
        0.0
    };
    let delta = latest - oldest;
    let direction = if delta.abs() <= epsilon {
        TrendDirection::Flat
    } else if delta > 0.0 {
        TrendDirection::Up
    } else {
        TrendDirection::Down
    };
    // A flat trend must not display a phantom "+0.0" from floating point
    // jitter: dead-band noise is rendered as a neutral zero for every unit.
    let amount = if direction == TrendDirection::Flat {
        match unit {
            TrendUnit::PercentPoints => "±0.0pt".to_string(),
            TrendUnit::BytesPerSecond => format!("±{}", format::format_rate(0)),
        }
    } else {
        match unit {
            TrendUnit::PercentPoints => format!("{delta:+.1}pt"),
            TrendUnit::BytesPerSecond => {
                let magnitude = format::format_rate(delta.abs() as u64);
                if delta > 0.0 {
                    format!("+{magnitude}")
                } else {
                    format!("-{magnitude}")
                }
            }
        }
    };
    TrendDelta { direction, amount }
}

// ---------------------------------------------------------- filter presets (R1-I06)
//
// ADR-012 deliberately keeps filters out of the persisted `Settings`: a filter
// describes "the work right now", not a display preference, and restoring one
// at startup would hide processes without explanation. These presets are therefore
// built-in, in-memory values only. They are never written to disk and never
// become part of the settings schema.

/// CPU threshold used by the "高 CPU" preset, in percent.
pub const PRESET_HIGH_CPU_PERCENT: f32 = 50.0;
/// Resident-memory threshold used by the "高メモリ" preset (256 MiB).
pub const PRESET_HIGH_MEMORY_BYTES: u64 = 256 * 1024 * 1024;
/// Virtual-memory threshold used by the "大きな仮想メモリ" preset (1 GiB).
pub const PRESET_LARGE_VIRTUAL_MEMORY_BYTES: u64 = 1024 * 1024 * 1024;

/// Stable identity of a built-in filter preset (R1-I06).
///
/// A small `Copy` enum (rather than the [`Filter`] itself) keeps
/// [`Command::ApplyFilterPreset`] cheap, `Eq` and safe to compare in tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum FilterPresetId {
    /// No filtering at all.
    All,
    /// Processes using at least [`PRESET_HIGH_CPU_PERCENT`] CPU.
    HighCpu,
    /// Processes using at least [`PRESET_HIGH_MEMORY_BYTES`] resident memory.
    HighMemory,
    /// Processes with at least [`PRESET_LARGE_VIRTUAL_MEMORY_BYTES`] of address space.
    LargeVirtualMemory,
    /// Processes currently in the running state.
    Running,
}

impl FilterPresetId {
    /// Japanese label shown on the preset chip.
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "すべて",
            Self::HighCpu => "高 CPU",
            Self::HighMemory => "高メモリ",
            Self::LargeVirtualMemory => "大きな仮想メモリ",
            Self::Running => "実行中",
        }
    }
}

/// A named, immutable filter shortcut shared by both shells (R1-I06).
#[derive(Clone, Debug, PartialEq)]
pub struct FilterPreset {
    /// Stable identity of this preset.
    pub id: FilterPresetId,
    /// Japanese label shown to the user.
    pub label: &'static str,
    /// Filter applied when the preset is selected.
    pub filter: Filter,
}

/// Built-in filter presets in display order (R1-I06).
///
/// The `All` entry is first so a UI can offer it as the "reset" choice. The
/// thresholds are static on purpose: presets are in-memory shortcuts, not
/// persisted or user-editable settings (ADR-012).
pub static FILTER_PRESETS: &[FilterPreset] = &[
    FilterPreset {
        id: FilterPresetId::All,
        label: FilterPresetId::All.label(),
        filter: Filter {
            text: String::new(),
            user: None,
            status: None,
            min_cpu_percent: None,
            min_memory_bytes: None,
            min_virtual_memory_bytes: None,
        },
    },
    FilterPreset {
        id: FilterPresetId::HighCpu,
        label: FilterPresetId::HighCpu.label(),
        filter: Filter {
            text: String::new(),
            user: None,
            status: None,
            min_cpu_percent: Some(PRESET_HIGH_CPU_PERCENT),
            min_memory_bytes: None,
            min_virtual_memory_bytes: None,
        },
    },
    FilterPreset {
        id: FilterPresetId::HighMemory,
        label: FilterPresetId::HighMemory.label(),
        filter: Filter {
            text: String::new(),
            user: None,
            status: None,
            min_cpu_percent: None,
            min_memory_bytes: Some(PRESET_HIGH_MEMORY_BYTES),
            min_virtual_memory_bytes: None,
        },
    },
    FilterPreset {
        id: FilterPresetId::LargeVirtualMemory,
        label: FilterPresetId::LargeVirtualMemory.label(),
        filter: Filter {
            text: String::new(),
            user: None,
            status: None,
            min_cpu_percent: None,
            min_memory_bytes: None,
            min_virtual_memory_bytes: Some(PRESET_LARGE_VIRTUAL_MEMORY_BYTES),
        },
    },
    FilterPreset {
        id: FilterPresetId::Running,
        label: FilterPresetId::Running.label(),
        filter: Filter {
            text: String::new(),
            user: None,
            status: Some(ProcessStatus::Running),
            min_cpu_percent: None,
            min_memory_bytes: None,
            min_virtual_memory_bytes: None,
        },
    },
];

/// Looks up a built-in preset by identity.
///
/// Panics only for a value not listed in [`FILTER_PRESETS`], which is a
/// programming error in the caller rather than a runtime condition.
pub fn filter_preset(id: FilterPresetId) -> &'static FilterPreset {
    FILTER_PRESETS
        .iter()
        .find(|preset| preset.id == id)
        .expect("filter preset id must be listed in FILTER_PRESETS")
}

/// A user intention produced by the shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// The refresh timer fired.
    Tick,
    /// Select a process (or clear the selection with `None`).
    Select(Option<ProcessKey>),
    /// Move the selection by `delta` rows in the visible table.
    SelectRelative(i32),
    /// Sort by a column; clicking the active column flips the direction.
    SetSortKey(SortKey),
    /// Set an exact sort spec (used when restoring persisted settings).
    SetSortSpec(SortSpec),
    /// Apply a built-in named filter preset (R1-I06).
    ApplyFilterPreset(FilterPresetId),
    /// Replace the free-text filter.
    SetFilterText(String),
    /// Clear every filter field.
    ClearFilter,
    /// Set the minimum virtual memory in bytes (`None` clears the threshold).
    SetMinVirtualMemoryBytes(Option<u64>),
    /// Change the refresh interval (clamped to the supported range).
    SetInterval(Duration),
    /// Pause or resume sampling.
    SetPaused(bool),
    /// Show or hide the details panel.
    ToggleDetails,
    /// Set the details panel visibility explicitly (used when restoring settings).
    SetDetailsVisible(bool),
    /// Show or hide the process tree view.
    ///
    /// Turning the tree on leaves application grouping (R1-VISIBLE), so the two
    /// presentation modes are mutually exclusive without a settings field.
    ToggleTreeView,
    /// Set the process tree view explicitly (used when restoring settings).
    SetTreeView(bool),
    /// Toggle collapse/expand state for a process node in tree view.
    ToggleTreeNode(ProcessKey),
    /// Switch the process table between application-grouped rows and the flat
    /// (or tree) process rows (R1-VISIBLE).
    ///
    /// Grouping is on by default; turning it on leaves the tree view. This is
    /// transient dashboard state and is deliberately absent from `Settings`.
    ToggleAppGrouping,
    /// Set application grouping explicitly (R1-VISIBLE).
    SetAppGrouping(bool),
    /// Select an application group row by its redacted app key (R1-GROUP).
    ///
    /// Resolves to the first member (smallest PID) and reuses the ordinary
    /// selection path, so details, signals and PID-reuse protection are intact.
    SelectAppGroup(String),
    /// Expand or collapse an application group row by its redacted app key (R1-GROUP).
    ToggleAppGroup(String),
    /// Open the confirmation dialog for a signal.
    RequestSignal {
        /// Target process.
        key: ProcessKey,
        /// Signal to send after confirmation.
        signal: Signal,
    },
    /// Confirm the pending signal.
    ConfirmKill,
    /// Dismiss the pending signal.
    CancelKill,
}

impl Command {
    /// Whether applying this command can never produce an [`Effect`] (R2-C01/C02).
    ///
    /// An effect-free command only mutates presentation state, so a caller that
    /// restores or applies it may rely on [`Dashboard::apply`] returning an empty
    /// vector. This is a `const`, allocation-free classification of the command
    /// itself: it holds in every dashboard state. The settings-restore commands
    /// are all effect-free. The remaining commands ([`Command::Tick`],
    /// [`Command::SetPaused`] on resume, and [`Command::ConfirmKill`]) emit an
    /// effect only in certain states, so their result must always be executed
    /// by the shell.
    pub const fn is_effect_free(&self) -> bool {
        matches!(
            self,
            Self::Select(_)
                | Self::SelectRelative(_)
                | Self::SetSortKey(_)
                | Self::SetSortSpec(_)
                | Self::ApplyFilterPreset(_)
                | Self::SetFilterText(_)
                | Self::ClearFilter
                | Self::SetMinVirtualMemoryBytes(_)
                | Self::SetInterval(_)
                | Self::ToggleDetails
                | Self::SetDetailsVisible(_)
                | Self::ToggleTreeView
                | Self::SetTreeView(_)
                | Self::ToggleTreeNode(_)
                | Self::ToggleAppGrouping
                | Self::SetAppGrouping(_)
                | Self::SelectAppGroup(_)
                | Self::ToggleAppGroup(_)
                | Self::RequestSignal { .. }
                | Self::CancelKill
        )
    }
}

/// Side effect the shell must perform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Ask the probe for a fresh snapshot.
    SampleNow,
    /// Send a signal to a process after the user confirmed it.
    Signal {
        /// Process identity, including the start time used to detect PID reuse.
        key: ProcessKey,
        /// Signal to send.
        signal: Signal,
    },
}

/// Signal waiting for user confirmation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingKill {
    /// Target process.
    pub key: ProcessKey,
    /// Display name captured when the dialog opened.
    pub name: String,
    /// Command line captured when the dialog opened (already sanitised by the
    /// platform adapter). Shown in the confirmation dialog so the user can
    /// verify the target before sending a signal.
    pub command: String,
    /// Owning user, when known.
    pub user: Option<String>,
    /// Signal that will be sent on confirmation.
    pub signal: Signal,
    /// Signal-specific explanation shown while the dialog is open (R3-U03).
    ///
    /// Stop / Continue are reversible; Kill is not. The native shell should
    /// render this instead of a single "cannot be undone" line so the dialog
    /// never misleads the user about the chosen signal.
    pub confirmation_note: &'static str,
}

/// Transient message shown in the status bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusMessage {
    /// Message text.
    pub text: String,
    /// `true` for errors and rejections.
    pub is_error: bool,
    /// `true` for startup/configuration notices that must survive the next
    /// successful sample (R3-U05).
    pub persistent: bool,
}

impl StatusMessage {
    fn info(text: impl Into<String>) -> Self {
        Self {
            text: format!("{STATUS_PREFIX_INFO} {}", text.into()),
            is_error: false,
            persistent: false,
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            text: format!("{STATUS_PREFIX_ERROR} {}", text.into()),
            is_error: true,
            persistent: false,
        }
    }

    fn persistent_info(text: impl Into<String>) -> Self {
        Self {
            text: format!("{STATUS_PREFIX_INFO} {}", text.into()),
            is_error: false,
            persistent: true,
        }
    }

    fn persistent_error(text: impl Into<String>) -> Self {
        Self {
            text: format!("{STATUS_PREFIX_ERROR} {}", text.into()),
            is_error: true,
            persistent: true,
        }
    }
}

/// Which kind of row an [`AppGroupRow`] represents (R1-GROUP).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppRowKind {
    /// An application group summarising one or more processes.
    Group,
    /// One individual process revealed by expanding a group.
    Process,
    /// A single row summarising several one-process applications (C1-COLLAPSE).
    ///
    /// The row carries the aggregate CPU/memory of those applications and the
    /// count in [`AppGroupRow::process_count`]. Expanding it reveals the original
    /// one-process group rows as `Group` rows at `depth = 1`.
    Others,
    /// A display-only section heading inserted by [`Dashboard::app_group_display_rows`]
    /// (C1-VMCLASSIFY).
    ///
    /// It points at no real process: shells render [`AppGroupRow::label`] as a
    /// heading and never pass its `key`/`process_key` to a command. It is never
    /// returned by [`Dashboard::app_group_rows`] and never enters `visible`.
    SectionHeader,
}

/// One row of the application-grouped process table (R1-GROUP).
///
/// With no groups expanded, [`Dashboard::app_group_rows`] returns group rows
/// only; expanding a group inserts its members as `depth = 1` process rows
/// immediately after it. Several one-process applications are compacted into a
/// single [`AppRowKind::Others`] row; expanding that row reveals the original
/// one-process group rows at `depth = 1` (C1-COLLAPSE). This is an *additive*
/// view model: existing [`ProcessTreeRow`] consumers keep the flat/tree process
/// list unchanged, so the export and wire contracts are untouched.
///
/// [`Dashboard::app_group_display_rows`] additionally wraps this list with
/// [`AppRowKind::SectionHeader`] rows once an application-key classification has
/// been injected; those heading rows are display-only and never participate in
/// selection, export or the wire format (C1-VMCLASSIFY).
#[derive(Clone, Debug, PartialEq)]
pub struct AppGroupRow {
    /// Whether this row is a group summary or an expanded process.
    pub kind: AppRowKind,
    /// Stable identity used for expansion state; the group's redacted app key
    /// derived from the executable ([`app_group_key`]).
    ///
    /// Empty for individual process rows; [`OTHERS_APP_GROUP_KEY`] for the
    /// compacted "others" row (C1-COLLAPSE).
    pub key: String,
    /// Display text: the app name for a group row (identical to `key`), or the
    /// process name for an expanded member row (e.g. `Isolated Web Co`).
    pub label: String,
    /// Index into `snapshot.processes` of the represented process.
    ///
    /// For a group row this is the first member (smallest [`ProcessKey`]).
    pub index: usize,
    /// Process identity a shell should pass to `Command::Select` when this row
    /// is activated. For a group row it is the first member's identity, so the
    /// existing details panel, signal confirmation and PID-reuse guard all keep
    /// working on a real process.
    pub process_key: ProcessKey,
    /// Every member key of a group row; empty for individual process rows.
    ///
    /// Lets a shell highlight the group when one of its hidden members is the
    /// current selection.
    pub member_keys: Vec<ProcessKey>,
    /// `0` for group and "others" rows, `1` for expanded process rows and for
    /// the one-process group rows revealed by expanding "others".
    pub depth: usize,
    /// Whether activating this row can reveal children.
    ///
    /// Always `false` for a single-member group and for process rows; always
    /// `true` for the "others" row while at least two candidates exist.
    pub has_children: bool,
    /// Whether this group is currently expanded (`false` for process rows).
    pub expanded: bool,
    /// Group CPU sum, or the individual process CPU reading.
    pub cpu_percent: f32,
    /// Group memory sum, or the individual process memory reading.
    pub memory_bytes: u64,
    /// Saturating sum of the members' virtual memory in bytes for a group row,
    /// or the individual process virtual memory reading for a process row.
    ///
    /// Computed from `snapshot.processes` inside this additive view; the wire
    /// and export contracts keep reading the snapshot directly and are unchanged.
    pub virtual_memory_bytes: u64,
    /// Saturating sum of the members' swap bytes in bytes for a group row, or
    /// the individual process swap reading for a process row (C2-NATIVESWAP).
    ///
    /// Mirrors [`Self::virtual_memory_bytes`]: derived from the filtered member
    /// snapshots inside this additive view, so the wire/export contracts keep
    /// reading `snapshot.processes` and stay unchanged. A process with no
    /// swap reading contributes zero.
    pub swap_bytes: u64,
    /// Number of grouped processes (`1` for individual process rows), or the
    /// number of compacted applications for the "others" row.
    pub process_count: usize,
}

/// Saturating sum of a group's member virtual memory bytes (C3-VMCOLUMN).
///
/// `group_by_app` stores `members` as indices into the filtered snapshot it was
/// built from, so the aggregation can be derived here without adding a field to
/// the core `ProcessGroup` or changing the wire/export contracts. It mirrors the
/// existing `memory_bytes` accumulation: non-finite/negative values never enter
/// this path (the field is `u64`), and overflow saturates.
fn group_virtual_memory_bytes(group: &ProcessGroup, filtered: &[ProcessSnapshot]) -> u64 {
    group.members.iter().fold(0_u64, |sum, &index| {
        sum.saturating_add(filtered[index].virtual_memory_bytes)
    })
}

/// Saturating sum of a group's member swap bytes (C2-NATIVESWAP).
///
/// Uses the same filtered-index accumulation as
/// [`group_virtual_memory_bytes`]: `group_by_app` stores members as indices into
/// the filtered snapshot, so no field needs to be added to the core
/// `ProcessGroup` and neither the wire nor the export contract changes. Swap is
/// a `u64` byte count, so there are no non-finite or negative values to guard.
fn group_swap_bytes(group: &ProcessGroup, filtered: &[ProcessSnapshot]) -> u64 {
    group.members.iter().fold(0_u64, |sum, &index| {
        sum.saturating_add(filtered[index].swap_bytes)
    })
}

/// Orders application groups with the injected classification as the primary
/// key and the chosen sort as the secondary key (C1-VMCLASSIFY).
///
/// Groups whose key is in `app_keys` sort before every other group, regardless
/// of `sort.direction`; the caller's [`SortSpec`] is applied only inside one
/// class. An empty `app_keys` classifies nothing, so the historical order (the
/// flat-table comparator alone) is preserved while a web classification is
/// still loading. Shared with the "others" representative path so both resolve
/// the same first row for a given filter and sort spec (C1-COLLAPSE).
fn sort_app_groups(
    groups: &mut [ProcessGroup],
    sort: SortSpec,
    filtered: &[ProcessSnapshot],
    app_keys: &BTreeSet<String>,
) {
    groups.sort_by(|left, right| {
        // `false < true`, so comparing (right is app, left is app) puts app
        // groups first. When both are equally classified this is `Equal` and
        // the caller's sort decides, keeping the explicit sort as key two.
        let classification = app_keys
            .contains(&right.key)
            .cmp(&app_keys.contains(&left.key));
        if classification != std::cmp::Ordering::Equal {
            return classification;
        }
        let ordering = match sort.key {
            SortKey::Cpu => left.cpu_percent.total_cmp(&right.cpu_percent),
            SortKey::Memory => left.memory_bytes.cmp(&right.memory_bytes),
            SortKey::Pid => filtered[left.members[0]]
                .key
                .pid
                .cmp(&filtered[right.members[0]].key.pid),
            SortKey::VirtualMemory => group_virtual_memory_bytes(left, filtered)
                .cmp(&group_virtual_memory_bytes(right, filtered)),
            SortKey::Swap => {
                group_swap_bytes(left, filtered).cmp(&group_swap_bytes(right, filtered))
            }
            // The remaining columns have no aggregate equivalent; keep the
            // stable BTreeMap key order for them.
            SortKey::Name
            | SortKey::User
            | SortKey::Status
            | SortKey::Threads
            | SortKey::StartTime => left.key.cmp(&right.key),
        };
        let ordering = match sort.direction {
            tm_core::SortDirection::Ascending => ordering,
            tm_core::SortDirection::Descending => ordering.reverse(),
        };
        ordering.then_with(|| left.key.cmp(&right.key))
    });
}

/// Builds the one summary row that replaces several one-process applications.
///
/// The representative is the first group in display order; its process identity
/// is the row's `process_key`/`index`, so selection, signal confirmation and the
/// PID-reuse guard keep operating on a real process (C1-COLLAPSE).
fn others_row(
    singles: &[ProcessGroup],
    filtered: &[ProcessSnapshot],
    base: &[usize],
    expanded: bool,
) -> AppGroupRow {
    let representative = &singles[0];
    let process = &filtered[representative.members[0]];
    // Each input is already a display-normalised group total; sum in f64 and
    // narrow once, mirroring `group_by_app`'s accumulation rules.
    let cpu_total = singles.iter().fold(0.0_f64, |sum, group| {
        sum + f64::from(group.cpu_percent.max(0.0))
    });
    let cpu_percent = if cpu_total.is_finite() {
        cpu_total as f32
    } else {
        0.0
    };
    let memory_bytes = singles
        .iter()
        .fold(0_u64, |sum, group| sum.saturating_add(group.memory_bytes));
    let virtual_memory_bytes = singles.iter().fold(0_u64, |sum, group| {
        sum.saturating_add(group_virtual_memory_bytes(group, filtered))
    });
    let swap_bytes = singles.iter().fold(0_u64, |sum, group| {
        sum.saturating_add(group_swap_bytes(group, filtered))
    });
    AppGroupRow {
        kind: AppRowKind::Others,
        key: OTHERS_APP_GROUP_KEY.to_string(),
        label: OTHERS_APP_GROUP_LABEL.to_string(),
        index: base[representative.members[0]],
        process_key: process.key,
        member_keys: singles
            .iter()
            .map(|group| filtered[group.members[0]].key)
            .collect(),
        depth: 0,
        has_children: true,
        expanded,
        cpu_percent,
        memory_bytes,
        virtual_memory_bytes,
        swap_bytes,
        process_count: singles.len(),
    }
}

/// Builds a display-only section heading row (C1-VMCLASSIFY).
///
/// `index` reuses the first classified row's in-range snapshot index so the
/// row stays structurally valid, but `process_key` is an unreachable sentinel
/// and `member_keys` is empty: a shell that accidentally activates the heading
/// cannot select or signal a real process.
fn section_header(key: &'static str, label: &'static str, index: usize) -> AppGroupRow {
    AppGroupRow {
        kind: AppRowKind::SectionHeader,
        key: key.to_string(),
        label: label.to_string(),
        index,
        process_key: ProcessKey::new(u32::MAX, u64::MAX),
        member_keys: Vec::new(),
        depth: 0,
        has_children: false,
        expanded: false,
        cpu_percent: 0.0,
        memory_bytes: 0,
        virtual_memory_bytes: 0,
        swap_bytes: 0,
        process_count: 0,
    }
}

/// One original one-process group revealed by expanding the "others" row.
///
/// It keeps `kind = Group` and `process_count = 1`; only `depth = 1` marks it as
/// a child of the summary row (C1-COLLAPSE).
fn single_group_row(
    group: &ProcessGroup,
    filtered: &[ProcessSnapshot],
    base: &[usize],
    depth: usize,
) -> AppGroupRow {
    let process = &filtered[group.members[0]];
    AppGroupRow {
        kind: AppRowKind::Group,
        key: group.key.clone(),
        label: group.label.clone(),
        index: base[group.members[0]],
        process_key: process.key,
        member_keys: vec![process.key],
        depth,
        has_children: false,
        expanded: false,
        cpu_percent: group.cpu_percent,
        memory_bytes: group.memory_bytes,
        virtual_memory_bytes: process.virtual_memory_bytes,
        swap_bytes: process.swap_bytes,
        process_count: 1,
    }
}

/// Presentation state for one dashboard window.
pub struct Dashboard {
    platform: PlatformInfo,
    snapshot: Option<SystemSnapshot>,
    visible: Vec<usize>,
    visible_depths: Vec<usize>,
    visible_has_children: Vec<bool>,
    tree_view: bool,
    /// Whether the table shows one row per application instead of one row per
    /// process (R1-VISIBLE). On by default so grouping needs no settings field
    /// or shell change; mutually exclusive with `tree_view` at the setter.
    app_group_view: bool,
    /// Redacted app keys classified as "アプリ" (C1-VMCLASSIFY).
    ///
    /// Injected by the shell because classification is platform-specific and
    /// may arrive asynchronously; the viewmodel stays I/O-free and wasm-safe.
    /// Empty means "no classification yet": sorting and display rows keep the
    /// legacy, unclassified behaviour. Presentation-only — never persisted and
    /// never part of `Filter`, export or the wire format.
    app_keys: BTreeSet<String>,
    collapsed: HashSet<ProcessKey>,
    /// Redacted app keys whose group rows are expanded (R1-GROUP).
    ///
    /// A `HashSet` keyed by `String` keeps the tree-view `ProcessKey` collapse
    /// set untouched; the two views are independent and both remain available.
    expanded_app_groups: HashSet<String>,
    history: MetricHistory,
    sort: SortSpec,
    filter: Filter,
    active_preset: Option<FilterPresetId>,
    selected: Option<ProcessKey>,
    selection_hint: usize,
    /// CPU history of the currently selected process (R1-I08).
    ///
    /// Only one series is retained: it is bounded by [`HISTORY_CAPACITY`], reset
    /// whenever the selection changes, and never persisted. Swapping to another
    /// process starts a fresh series rather than attributing old samples to it.
    selected_cpu_history: RingBuffer<f32>,
    /// Process identity the retained series belongs to.
    selected_cpu_history_key: Option<ProcessKey>,
    details_visible: bool,
    paused: bool,
    interval: Duration,
    pending_kill: Option<PendingKill>,
    status: Option<StatusMessage>,
    /// Standing notice retained while a transient message is on screen.
    ///
    /// A transient write (e.g. the export confirmation) may cover the demo-mode
    /// guidance on screen; clearing it restores this slot instead of leaving the
    /// live region empty (SC-STATUSRESTORE). Only explicit persistent setters
    /// populate it, so live mode never gains a demo notice through this path.
    persistent_status: Option<StatusMessage>,
    last_network: Option<(u64, u64)>,
    /// Physical interface names whose counters contributed to `last_network`.
    ///
    /// Cumulative totals are only comparable while the set of contributors is
    /// unchanged: a hot-plugged interface brings its own boot-time counter and
    /// would otherwise look like a one-frame traffic spike (WC-IFHOT).
    last_network_interfaces: HashSet<String>,
    /// High-water mark of ingested sample times in the current clock era. A
    /// snapshot older than this is late unless its network counters show it is
    /// newer than the baseline, in which case the mark is rebased onto the new
    /// clock era (WB-CLOCK). Only an accepted snapshot may clear transient
    /// status or become the network baseline.
    last_ingested_at: Option<Timestamp>,
    network_rates: (u64, u64),
}

impl Dashboard {
    /// Creates a dashboard for a backend.
    pub fn new(platform: PlatformInfo, interval: Duration) -> Self {
        Self {
            platform,
            snapshot: None,
            visible: Vec::new(),
            visible_depths: Vec::new(),
            visible_has_children: Vec::new(),
            tree_view: false,
            app_group_view: true,
            app_keys: BTreeSet::new(),
            collapsed: HashSet::new(),
            expanded_app_groups: HashSet::new(),
            history: MetricHistory::new(HISTORY_CAPACITY),
            sort: SortSpec::default(),
            filter: Filter::default(),
            active_preset: None,
            selected: None,
            selection_hint: 0,
            selected_cpu_history: RingBuffer::new(HISTORY_CAPACITY),
            selected_cpu_history_key: None,
            details_visible: true,
            paused: false,
            interval: clamp_interval(interval),
            pending_kill: None,
            status: None,
            persistent_status: None,
            last_network: None,
            last_network_interfaces: HashSet::new(),
            last_ingested_at: None,
            network_rates: (0, 0),
        }
    }

    /// Backend description.
    pub fn platform_info(&self) -> &PlatformInfo {
        &self.platform
    }

    /// Latest snapshot, if one has been ingested.
    pub fn snapshot(&self) -> Option<&SystemSnapshot> {
        self.snapshot.as_ref()
    }

    /// Chart history.
    pub fn history(&self) -> &MetricHistory {
        &self.history
    }

    /// Active sort spec.
    pub fn sort_spec(&self) -> SortSpec {
        self.sort
    }

    /// Active filter.
    pub fn filter(&self) -> &Filter {
        &self.filter
    }

    /// Free-text filter as typed by the user.
    pub fn filter_text(&self) -> &str {
        &self.filter.text
    }

    /// Preset currently driving the filter, if any (R1-I06).
    ///
    /// Any direct filter edit clears this marker: the table must not claim a
    /// preset is active after the user has changed one of its fields.
    pub fn active_filter_preset(&self) -> Option<FilterPresetId> {
        self.active_preset
    }

    /// Current user-configured refresh interval.
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Interval the sampler should actually wait: the configured value raised
    /// to the snapshot's adaptive floor, when one is available (R1-I11).
    ///
    /// Before the first snapshot (or with a legacy wire snapshot whose
    /// `sampling` field defaults to zero) this is exactly `interval()`, so the
    /// existing startup cadence is unchanged.
    pub fn effective_interval(&self) -> Duration {
        self.snapshot
            .as_ref()
            .map(|snapshot| snapshot.sampling.effective_interval(self.interval))
            .unwrap_or(self.interval)
    }

    /// Power state of the latest snapshot, if one has been ingested.
    pub fn power(&self) -> Option<&tm_core::PowerSnapshot> {
        self.snapshot.as_ref().map(|snapshot| &snapshot.power)
    }

    /// Adaptive sampling plan of the latest snapshot, if one has been ingested.
    pub fn sampling_plan(&self) -> Option<tm_core::SamplingPlan> {
        self.snapshot.as_ref().map(|snapshot| snapshot.sampling)
    }

    /// Read-only cgroup groups of the latest snapshot (R1-I12).
    pub fn cgroup_groups(&self) -> &[tm_core::CgroupGroup] {
        self.snapshot
            .as_ref()
            .map(|snapshot| snapshot.cgroups.as_slice())
            .unwrap_or_default()
    }

    /// Whether sampling is paused.
    pub fn paused(&self) -> bool {
        self.paused
    }

    /// Whether the details panel is visible.
    pub fn details_visible(&self) -> bool {
        self.details_visible
    }

    /// Whether the process table is shown as a tree.
    pub fn tree_view(&self) -> bool {
        self.tree_view
    }

    /// Whether the visible rows are application groups (R1-VISIBLE).
    ///
    /// `true` from construction; transient and intentionally not persisted.
    pub fn app_group_view(&self) -> bool {
        self.app_group_view
    }

    /// Enable grouping and leave the tree view (R1-VISIBLE).
    fn show_app_groups(&mut self) {
        self.app_group_view = true;
        self.tree_view = false;
    }

    /// Enable the tree view and leave grouping (R1-VISIBLE).
    fn show_tree_view(&mut self) {
        self.tree_view = true;
        self.app_group_view = false;
    }

    /// Selected process identity.
    pub fn selected_key(&self) -> Option<ProcessKey> {
        self.selected
    }

    /// Selected process row, resolved against the latest snapshot.
    pub fn selected_process(&self) -> Option<&ProcessSnapshot> {
        let snapshot = self.snapshot.as_ref()?;
        snapshot.process(self.selected?)
    }

    /// CPU history of the selected process, oldest sample first (R1-I08).
    ///
    /// Values are normalised (non-finite and negative readings become zero) but
    /// are not capped at 100: a multicore process may legitimately exceed it,
    /// and the widget scales the sparkline to the observed maximum. The buffer
    /// is empty until a process is selected and the first snapshot is ingested.
    pub fn selected_cpu_history(&self) -> &RingBuffer<f32> {
        &self.selected_cpu_history
    }

    /// Copies the selected process's CPU history into a vector for rendering.
    pub fn selected_cpu_values(&self) -> Vec<f32> {
        self.selected_cpu_history.to_vec()
    }

    /// Indices into `snapshot.processes` for the rows the table shows, in display order.
    ///
    /// In the default grouped view these are the group rows from
    /// [`Self::app_group_rows`]: a group maps to its first (smallest
    /// [`ProcessKey`]) member, and an expanded member maps to itself. The
    /// existing native and web renderers therefore show grouped rows without
    /// any drawing-side change (R1-VISIBLE).
    pub fn visible_indices(&self) -> &[usize] {
        &self.visible
    }

    /// Nesting depth for each visible row (0 for flat view, tree roots and
    /// group rows; 1 for processes revealed by expanding a group).
    pub fn visible_depths(&self) -> &[usize] {
        &self.visible_depths
    }

    /// Returns visible child markers aligned with `visible_indices`.
    ///
    /// A group with more than one member exposes `true` here, which is the
    /// existing expand affordance the renderers already understand.
    pub fn visible_has_children(&self) -> &[bool] {
        &self.visible_has_children
    }

    /// Returns whether a process node is collapsed in tree view.
    pub fn is_collapsed(&self, key: ProcessKey) -> bool {
        self.collapsed.contains(&key)
    }

    /// Visible rows with process indices, tree depth, and child marker.
    pub fn visible_rows(&self) -> impl Iterator<Item = ProcessTreeRow> + '_ {
        self.visible
            .iter()
            .enumerate()
            .map(|(pos, &index)| ProcessTreeRow {
                index,
                depth: self.visible_depths.get(pos).copied().unwrap_or(0),
                has_children: self.visible_has_children.get(pos).copied().unwrap_or(false),
            })
    }

    /// Indices of snapshot processes eligible for an individual display row.
    ///
    /// This is the filter/sort pipeline over the whole snapshot: a `0.0%` CPU
    /// reading keeps its row, so a background process never appears or
    /// disappears between samples solely because its CPU usage is zero.
    /// Export/wire paths read `snapshot.processes` directly and never come
    /// through here.
    fn display_base(&self, snapshot: &SystemSnapshot) -> Vec<usize> {
        filter_and_sort(&snapshot.processes, &self.filter, self.sort)
    }

    /// Whether an application group is currently expanded (R1-GROUP).
    pub fn is_app_group_expanded(&self, key: &str) -> bool {
        self.expanded_app_groups.contains(key)
    }

    /// Rows of the application-grouped process table, in display order (R1-GROUP).
    ///
    /// By default this returns one row per redacted app key (see
    /// [`app_group_key`]), with the members' CPU/memory totals already computed
    /// by [`group_by_app`]. One-process applications are compacted into a single
    /// [`AppRowKind::Others`] row when at least [`OTHERS_MIN_GROUP_COUNT`] of
    /// them exist and a multi-process group remains visible; the summary row is
    /// always the last one (only its own expanded children follow it), and
    /// expanding that reserved key reveals the original rows at `depth = 1`.
    /// A multi-process group added to the expansion set through
    /// [`Command::ToggleAppGroup`] is likewise followed by its individual
    /// processes at `depth = 1`; collapsing it removes them again. The current
    /// filter is applied first and the active sort spec orders the group rows
    /// (`Cpu`, `Memory`, `VirtualMemory`, `Swap` and `Pid` use their totals,
    /// other columns fall back to the deterministic group key).
    ///
    /// A `0.0%` CPU process is never suppressed: it keeps its group row (or its
    /// place inside the "others" bucket) across samples, so rows stay stable
    /// when a background process goes idle.
    ///
    /// No snapshot yields an empty vector. Export and wire code keep receiving
    /// `snapshot.processes` directly and never see these rows.
    pub fn app_group_rows(&self) -> Vec<AppGroupRow> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        // Build the filtered subset so group totals describe exactly the rows
        // the user can see; `base` maps filtered positions back to the snapshot.
        let base = self.display_base(snapshot);
        let filtered: Vec<ProcessSnapshot> = base
            .iter()
            .map(|&index| snapshot.processes[index].clone())
            .collect();
        let mut groups = group_by_app(&filtered);
        sort_app_groups(&mut groups, self.sort, &filtered, &self.app_keys);

        // Snapshot the one-process groups before consuming `groups`: they are
        // shown either as a single "others" row or, when expanded, as a run of
        // one-process `Group` rows directly beneath it (C1-COLLAPSE).
        let singles: Vec<ProcessGroup> = groups
            .iter()
            .filter(|group| group.process_count == 1)
            .cloned()
            .collect();
        // Keep at least one real group row beside the summary: when every app
        // has a single process, folding them all would leave a lone "others"
        // row with nothing to contrast against and destroy the existing list
        // (e.g. the demo data). The compacted view is only useful when a
        // multi-process app remains individually visible (C1-COLLAPSE).
        let has_multi_process_group = groups.iter().any(|group| group.process_count > 1);
        let fold_others = singles.len() >= OTHERS_MIN_GROUP_COUNT && has_multi_process_group;
        let others_expanded =
            fold_others && self.expanded_app_groups.contains(OTHERS_APP_GROUP_KEY);

        let mut rows = Vec::new();
        for group in groups {
            // Folded singles are appended once after the loop, so the summary
            // row always closes the list instead of interrupting the ordinary
            // groups at the first single-process position (C4-VIEWMODEL).
            if fold_others && group.process_count == 1 {
                continue;
            }
            let member_keys: Vec<ProcessKey> = group
                .members
                .iter()
                .map(|&member| filtered[member].key)
                .collect();
            let first_key = member_keys[0]; // groups always have one member
            let has_children = group.members.len() > 1;
            let expanded = has_children && self.expanded_app_groups.contains(&group.key);
            let virtual_memory_bytes = group_virtual_memory_bytes(&group, &filtered);
            let swap_bytes = group_swap_bytes(&group, &filtered);
            rows.push(AppGroupRow {
                kind: AppRowKind::Group,
                key: group.key.clone(),
                label: group.label,
                index: base[group.members[0]],
                process_key: first_key,
                member_keys,
                depth: 0,
                has_children,
                expanded,
                cpu_percent: group.cpu_percent,
                memory_bytes: group.memory_bytes,
                virtual_memory_bytes,
                swap_bytes,
                process_count: group.process_count,
            });
            if expanded {
                for &member in &group.members {
                    let process = &filtered[member];
                    rows.push(AppGroupRow {
                        kind: AppRowKind::Process,
                        key: group.key.clone(),
                        label: process.name.clone(),
                        index: base[member],
                        process_key: process.key,
                        member_keys: Vec::new(),
                        depth: 1,
                        has_children: false,
                        expanded: false,
                        cpu_percent: process.cpu_percent,
                        memory_bytes: process.memory_bytes,
                        virtual_memory_bytes: process.virtual_memory_bytes,
                        swap_bytes: process.swap_bytes,
                        process_count: 1,
                    });
                }
            }
        }
        if fold_others {
            // Trailing summary: only its own revealed one-process rows may
            // follow it, so expanding "others" never pushes the bucket away
            // from the end of the list (C4-VIEWMODEL).
            rows.push(others_row(&singles, &filtered, &base, others_expanded));
            if others_expanded {
                rows.extend(
                    singles
                        .iter()
                        .map(|single| single_group_row(single, &filtered, &base, 1)),
                );
            }
        }
        rows
    }

    /// Injects the redacted application keys classified as "アプリ" (C1-VMCLASSIFY).
    ///
    /// The shell owns classification (platform-specific and possibly async), so
    /// the viewmodel only stores the resulting key set and stays I/O-free and
    /// wasm-compatible. An empty set means "not classified yet": ordering and
    /// display headings fall back to the legacy behaviour. The visible arrays
    /// are recomputed so [`Self::app_group_rows`], `visible_indices` and the
    /// headings always reflect the same order.
    pub fn set_app_keys(&mut self, keys: BTreeSet<String>) {
        self.app_keys = keys;
        self.recompute_visible();
    }

    /// Currently injected application key set (C1-VMCLASSIFY).
    pub fn app_keys(&self) -> &BTreeSet<String> {
        &self.app_keys
    }

    /// Rows for rendering the grouped table with section headings (C1-VMCLASSIFY).
    ///
    /// Wraps [`Self::app_group_rows`] with two display-only
    /// [`AppRowKind::SectionHeader`] rows: 「アプリ」 before the first top-level
    /// group whose key is in the injected set, and 「バックグラウンド プロセス」
    /// before the first one whose key is not. The trailing "others" bucket,
    /// along with its revealed children, stays unclassified and keeps closing
    /// the list. With no injected keys the input is returned
    /// unchanged (legacy order, no headings), so an async/classification-loading
    /// shell sees no flicker.
    ///
    /// Purely additive: [`Self::app_group_rows`], `visible_indices` and every
    /// export/wire path keep their existing shape and contracts. Heading rows
    /// carry no selectable process identity and must never reach a command.
    pub fn app_group_display_rows(&self) -> Vec<AppGroupRow> {
        let rows = self.app_group_rows();
        if self.app_keys.is_empty() {
            return rows;
        }
        let mut display = Vec::with_capacity(rows.len() + 2);
        let mut apps_heading = false;
        let mut background_heading = false;
        for row in rows {
            // Only top-level group rows carry a section. Expanded process rows
            // and children revealed from "others" inherit their parent's place;
            // the "others" row itself is deliberately unclassified so it keeps
            // closing the list without a heading of its own.
            let class = match row.kind {
                AppRowKind::Group if row.depth == 0 => Some(self.app_keys.contains(&row.key)),
                _ => None,
            };
            match class {
                Some(true) if !apps_heading => {
                    display.push(section_header(
                        APPS_SECTION_KEY,
                        APPS_SECTION_LABEL,
                        row.index,
                    ));
                    apps_heading = true;
                }
                Some(false) if !background_heading => {
                    display.push(section_header(
                        BACKGROUND_SECTION_KEY,
                        BACKGROUND_SECTION_LABEL,
                        row.index,
                    ));
                    background_heading = true;
                }
                _ => {}
            }
            display.push(row);
        }
        display
    }

    /// Position of the selected process inside [`Self::app_group_rows`].
    ///
    /// A hidden member of a collapsed group highlights the group row, so the
    /// selection never disappears from the application view.
    pub fn selected_app_group_row_index(&self) -> Option<usize> {
        let selected = self.selected?;
        let rows = self.app_group_rows();
        // An expanded member has its own row; the group row is only a fallback
        // for a selection hidden inside a collapsed group.
        rows.iter()
            .position(|row| row.process_key == selected)
            .or_else(|| {
                rows.iter().position(|row| {
                    matches!(row.kind, AppRowKind::Group | AppRowKind::Others)
                        && row.member_keys.contains(&selected)
                })
            })
    }

    /// First member (smallest [`ProcessKey`]) of a keyed, visible group.
    ///
    /// Uses [`Self::display_base`], the same filter/sort pipeline that feeds the
    /// visible rows, so the representative always belongs to the displayed set.
    fn first_app_group_key(&self, key: &str) -> Option<ProcessKey> {
        self.snapshot.as_ref().and_then(|snapshot| {
            let base = self.display_base(snapshot);
            base.iter()
                .filter(|&&index| app_group_key(&snapshot.processes[index]) == key)
                .map(|&index| snapshot.processes[index].key)
                .min()
        })
    }

    fn is_member_of_app_group(&self, member: ProcessKey, key: &str) -> bool {
        self.snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.process(member))
            .is_some_and(|process| app_group_key(process) == key)
    }

    /// Group-row representative for a selection that has no row of its own.
    ///
    /// In the default grouped view a collapsed group shows only its first
    /// member. A selection on another (hidden) member is remapped to that first
    /// member, keeping the existing `ProcessKey`-based highlight, signal target
    /// and PID-reuse guard intact (R1-VISIBLE). An expanded member already has
    /// its own row and is left unchanged.
    fn grouped_representative(&self, key: ProcessKey) -> Option<ProcessKey> {
        if !self.app_group_view {
            return None;
        }
        let snapshot = self.snapshot.as_ref()?;
        let app_key = app_group_key(snapshot.process(key)?);
        let first = self.first_app_group_key(app_key)?;
        if first != key {
            // An expanded member has its own row; only a hidden member of a
            // collapsed multi-process group needs the representative.
            return (!self.expanded_app_groups.contains(app_key)).then_some(first);
        }
        // A one-process app folded into "others" has no row of its own while
        // the bucket is collapsed. Keep its selection on the bucket's
        // representative, mirroring the hidden-member rule above (C1-COLLAPSE).
        if self.expanded_app_groups.contains(OTHERS_APP_GROUP_KEY) {
            return None;
        }
        self.app_group_rows()
            .into_iter()
            .find(|row| row.kind == AppRowKind::Others)
            .and_then(|row| {
                (row.member_keys.contains(&key) && row.process_key != key)
                    .then_some(row.process_key)
            })
    }

    /// Drops expansion keys whose application no longer exists in a new sample.
    ///
    /// The reserved "others" key has no real application behind it, so it is
    /// retained only while an "others" row would still be shown; otherwise a
    /// stale expansion entry would survive forever (C1-COLLAPSE).
    fn prune_expanded_app_groups(&mut self) {
        let Some(snapshot) = &self.snapshot else {
            self.expanded_app_groups.clear();
            return;
        };
        let rows = self.app_group_rows();
        let others_available = rows.iter().any(|row| row.kind == AppRowKind::Others);
        self.expanded_app_groups.retain(|key| {
            if key == OTHERS_APP_GROUP_KEY {
                return others_available;
            }
            snapshot
                .processes
                .iter()
                .any(|process| app_group_key(process) == key.as_str())
        });
    }

    /// Number of rows currently displayed (group rows in the default view).
    ///
    /// DOM/a11y row counts keep this "visible rows" meaning; the unfiltered
    /// process count remains available from `snapshot.processes` for exports.
    pub fn visible_count(&self) -> usize {
        self.visible.len()
    }

    /// Signal waiting for confirmation.
    pub fn pending_kill(&self) -> Option<&PendingKill> {
        self.pending_kill.as_ref()
    }

    /// Current status-bar message.
    pub fn status(&self) -> Option<&StatusMessage> {
        self.status.as_ref()
    }

    /// Latest network rates in bytes per second.
    pub fn network_rates(&self) -> (u64, u64) {
        self.network_rates
    }

    /// CPU trend from the oldest retained sample to the newest (R5-I05).
    pub fn cpu_trend(&self) -> TrendDelta {
        percent_trend(&self.history.cpu_percent)
    }

    /// Memory trend from the oldest retained sample to the newest (R5-I05).
    pub fn memory_trend(&self) -> TrendDelta {
        percent_trend(&self.history.memory_percent)
    }

    /// Combined network trend (RX + TX) over the retained history (R5-I05).
    ///
    /// RX and TX are summed at each end of the window so the badge describes
    /// total traffic, matching the single network overview card.
    pub fn network_trend(&self) -> TrendDelta {
        if self.history.len() < 2 {
            return unknown_trend();
        }
        let total = |rx: Option<&u64>, tx: Option<&u64>| -> Option<f64> {
            Some(rx?.saturating_add(*tx?) as f64)
        };
        trend_delta(
            total(
                self.history.network_rx_rate.earliest(),
                self.history.network_tx_rate.earliest(),
            ),
            total(
                self.history.network_rx_rate.latest(),
                self.history.network_tx_rate.latest(),
            ),
            0.0,
            TrendUnit::BytesPerSecond,
        )
    }

    /// Position of the selected row inside the visible table.
    pub fn selected_visible_index(&self) -> Option<usize> {
        let selected = self.selected?;
        let snapshot = self.snapshot.as_ref()?;
        self.visible
            .iter()
            .position(|&index| snapshot.processes[index].key == selected)
    }

    /// Returns `true` for processes the UI must never signal.
    pub fn is_protected(&self, key: ProcessKey) -> bool {
        if key.pid <= 1 {
            return true;
        }
        self.snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.self_pid)
            .is_some_and(|self_pid| self_pid == key.pid)
    }

    /// Applies a command to the presentation state and returns the effects the
    /// shell must execute (R2-C01/C02).
    ///
    /// # Effect contract
    ///
    /// This method performs no I/O: every returned [`Effect`] is a request that
    /// the calling shell MUST execute exactly once ([`Effect::SampleNow`]
    /// requests a fresh probe sample, [`Effect::Signal`] sends a confirmed
    /// signal). Discarding the vector silently loses the user's intent, so the
    /// shells iterate over and execute the whole result rather than inspecting
    /// a single case.
    ///
    /// Commands for which [`Command::is_effect_free`] is `true` only change
    /// display state and are guaranteed to return an empty vector in every
    /// state — this is the "empty" precondition the settings-restore path relies
    /// on. The three stateful commands ([`Command::Tick`], [`Command::SetPaused`]
    /// on resume, [`Command::ConfirmKill`]) emit an effect only in some states,
    /// so their result must never be dropped. Tests in this module pin both
    /// halves of the contract.
    pub fn apply(&mut self, command: Command) -> Vec<Effect> {
        match command {
            Command::Tick => {
                if self.paused {
                    Vec::new()
                } else {
                    vec![Effect::SampleNow]
                }
            }
            Command::Select(key) => {
                let key = key.filter(|key| {
                    self.snapshot
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.process(*key).is_some())
                });
                // A member hidden inside a collapsed group has no row of its
                // own; keep the ProcessKey-based selection on the group row's
                // first member so the highlight and arrow keys stay in sync.
                self.selected = key.map(|key| self.grouped_representative(key).unwrap_or(key));
                self.sync_selection_hint();
                self.sync_selected_cpu_history();
                Vec::new()
            }
            Command::SelectRelative(delta) => {
                self.select_relative(delta);
                Vec::new()
            }
            Command::SetSortKey(key) => {
                self.sort.toggle(key);
                self.recompute_visible();
                Vec::new()
            }
            Command::SetSortSpec(spec) => {
                self.sort = spec;
                self.recompute_visible();
                Vec::new()
            }
            Command::ApplyFilterPreset(id) => {
                // Clone the static preset: a preset is shared immutable data,
                // the dashboard owns its own mutable filter.
                self.filter = filter_preset(id).filter.clone();
                self.active_preset = Some(id);
                self.recompute_visible();
                Vec::new()
            }
            Command::SetFilterText(text) => {
                // A preset replaces the entire filter, including the structural
                // fields. As soon as the user starts a free-text search the
                // preset no longer applies, so drop those structural conditions
                // too; otherwise the list stays invisibly narrowed by e.g. the
                // High CPU threshold while only the text appears to be active.
                if self.active_preset.is_some() {
                    self.filter = Filter::default();
                }
                self.filter.text = text;
                self.active_preset = None;
                self.recompute_visible();
                Vec::new()
            }
            Command::ClearFilter => {
                self.filter = Filter::default();
                self.active_preset = None;
                self.recompute_visible();
                Vec::new()
            }
            Command::SetMinVirtualMemoryBytes(bytes) => {
                self.filter.min_virtual_memory_bytes = bytes;
                self.active_preset = None;
                self.recompute_visible();
                Vec::new()
            }
            Command::SetInterval(interval) => {
                self.interval = clamp_interval(interval);
                Vec::new()
            }
            Command::SetPaused(paused) => {
                self.paused = paused;
                // A persistent startup/configuration notice must survive a
                // pause/resume cycle (R24 history review).
                let notice_persistent = self.status.as_ref().is_some_and(|s| s.persistent);
                if paused {
                    if !notice_persistent {
                        self.status = Some(StatusMessage::info("更新を一時停止しました"));
                    }
                    Vec::new()
                } else {
                    // The bytes received while paused are stale for an instant
                    // rate; start a fresh baseline on the next sample.
                    self.last_network = None;
                    if !notice_persistent {
                        self.status = Some(StatusMessage::info("更新を再開しました"));
                    }
                    vec![Effect::SampleNow]
                }
            }
            Command::ToggleDetails => {
                self.details_visible = !self.details_visible;
                Vec::new()
            }
            Command::SetDetailsVisible(visible) => {
                self.details_visible = visible;
                Vec::new()
            }
            Command::ToggleTreeView => {
                if self.tree_view {
                    self.tree_view = false;
                } else {
                    self.show_tree_view();
                }
                self.recompute_visible();
                Vec::new()
            }
            Command::SetTreeView(tree_view) => {
                if tree_view {
                    self.show_tree_view();
                } else {
                    self.tree_view = false;
                }
                self.recompute_visible();
                Vec::new()
            }
            Command::ToggleAppGrouping => {
                if self.app_group_view {
                    self.app_group_view = false;
                } else {
                    self.show_app_groups();
                }
                self.recompute_visible();
                Vec::new()
            }
            Command::SetAppGrouping(enabled) => {
                if enabled {
                    self.show_app_groups();
                } else {
                    self.app_group_view = false;
                }
                self.recompute_visible();
                Vec::new()
            }
            Command::ToggleTreeNode(key) => {
                if self.tree_view {
                    if self.collapsed.contains(&key) {
                        self.collapsed.remove(&key);
                    } else {
                        // A selected row hidden by this collapse would make
                        // `ensure_selection_visible` fall back to the first
                        // visible row (an unrelated process). Move the
                        // selection to the collapsed node itself instead
                        // (R42 audit).
                        if self.selected.is_some_and(|selected| {
                            selected != key && self.is_descendant(selected, key)
                        }) {
                            self.selected = Some(key);
                        }
                        self.collapsed.insert(key);
                    }
                    self.recompute_visible();
                }
                Vec::new()
            }
            Command::SelectAppGroup(key) => {
                // A group row has no identity of its own: selecting it selects
                // its first member, which keeps the existing detail panel,
                // signal confirmation and PID-reuse protection unchanged.
                // The "others" bucket resolves to its representative the same
                // way (C1-COLLAPSE).
                let first = if key == OTHERS_APP_GROUP_KEY {
                    self.app_group_rows()
                        .into_iter()
                        .find(|row| row.kind == AppRowKind::Others)
                        .map(|row| row.process_key)
                } else {
                    self.first_app_group_key(&key)
                };
                if let Some(first) = first {
                    self.selected = Some(first);
                    self.sync_selection_hint();
                    self.sync_selected_cpu_history();
                }
                Vec::new()
            }
            Command::ToggleAppGroup(key) => {
                // In flat/tree mode no visible row advertises group children.
                if !self.app_group_view {
                    return Vec::new();
                }
                // Only a row whose `has_children` marker is set can be toggled:
                // single-member groups and expanded process rows have no
                // children to reveal (R1-VISIBLE). This also ignores stale keys
                // so the set cannot grow from clicks on removed groups.
                let expandable = self.app_group_rows().iter().any(|row| {
                    matches!(row.kind, AppRowKind::Group | AppRowKind::Others)
                        && row.key == key
                        && row.has_children
                });
                if !expandable {
                    if self.expanded_app_groups.remove(&key) {
                        self.recompute_visible();
                    }
                    return Vec::new();
                }
                if self.expanded_app_groups.remove(&key) {
                    // Collapsing a row must not leave a hidden member as the
                    // selection: move it to that row's representative instead.
                    if key == OTHERS_APP_GROUP_KEY {
                        // The bucket holds processes from several apps, so it
                        // needs its own member lookup.
                        let summary = self
                            .app_group_rows()
                            .into_iter()
                            .find(|row| row.kind == AppRowKind::Others);
                        if let Some(summary) = summary {
                            if self.selected.is_some_and(|selected| {
                                summary.member_keys.contains(&selected)
                                    && summary.process_key != selected
                            }) {
                                self.selected = Some(summary.process_key);
                                self.sync_selected_cpu_history();
                            }
                        }
                    } else if self.selected.is_some_and(|selected| {
                        self.is_member_of_app_group(selected, &key)
                            && self.first_app_group_key(&key) != self.selected
                    }) {
                        self.selected = self.first_app_group_key(&key);
                        self.sync_selected_cpu_history();
                    }
                } else {
                    self.expanded_app_groups.insert(key);
                }
                // The visible arrays mirror app_group_rows(), so expansion must
                // refresh them (c1-group only rebuilt the rows on demand).
                self.recompute_visible();
                Vec::new()
            }
            Command::RequestSignal { key, signal } => {
                self.request_signal(key, signal);
                Vec::new()
            }
            Command::ConfirmKill => self.confirm_kill(),
            Command::CancelKill => {
                self.pending_kill = None;
                Vec::new()
            }
        }
    }

    /// Ingests a new snapshot and updates history, rates and selection.
    ///
    /// A paused dashboard stays frozen: an in-flight native sample (or the web
    /// sampler's next tick) can complete after the user pressed pause. Keep
    /// `Tick`, `ingest` and `ConfirmKill` under the same pause invariant.
    ///
    /// A snapshot whose `sampled_at` predates the newest sample already seen
    /// is normally a late arrival. It still refreshes the display, but it
    /// leaves the transient status, the network rate baseline, `network_rates`
    /// and the chart `history` untouched so a delayed sample cannot erase the
    /// latest error, skew the next fresh rate or write a spurious 0 spike
    /// (R2-BV01). CPU/memory percentages shown in the overview come from the
    /// refreshed `snapshot`, not from `history`, so they still update.
    ///
    /// The exception is a wall-clock step back: when a regressed `sampled_at`
    /// arrives with network counters that have strictly advanced, the sample
    /// itself is newer than the baseline and only the clock is stale. The
    /// high-water mark and rate baseline are rebased onto it, so a *sustained*
    /// regression cannot keep accumulating rates from an obsolete baseline or
    /// pin a transient error forever (WB-CLOCK). Equal counters are not
    /// progress: an idle delayed sample stays late and cannot clear the newest
    /// transient error or roll the baseline back (SB-CLOCKFIX).
    pub fn ingest(&mut self, snapshot: SystemSnapshot) {
        if self.paused {
            // Keep the "paused" notice: an in-flight sample finishing now must
            // not clear it (R23 regression from R21/R22 interaction).
            return;
        }
        // An asynchronous sampler can deliver an older sample after a newer
        // one. Compare against the high-water mark rather than the last stored
        // `snapshot`: a repeated late arrival must not be able to walk the mark
        // backwards and eventually become the baseline itself.
        //
        // Cumulative network counters cannot be higher in an older sample. If
        // a "late" snapshot nevertheless carries counters that strictly
        // advanced past the current baseline, the sample is newer than the
        // baseline and only the wall clock stepped back (NTP/VM resume). Rebase
        // the mark and the baseline onto it so a *sustained* regression cannot
        // accumulate rates or pin a transient error forever (WB-CLOCK).
        //
        // Equality is NOT progress: on an idle network a delayed sample carries
        // the very same totals, so it stays late and must not clear the newest
        // transient error, rewind `last_ingested_at` or overwrite the baseline
        // and its physical-IF set (SB-CLOCKFIX).
        let wall_clock_regressed = self
            .last_ingested_at
            .is_some_and(|last| snapshot.sampled_at < last);
        let (total_rx, total_tx) = snapshot.network_totals();
        let counters_advanced = self.last_network.is_none_or(|(previous_rx, previous_tx)| {
            total_rx > previous_rx || total_tx > previous_tx
        });
        let is_late = wall_clock_regressed && !counters_advanced;
        if !is_late {
            self.last_ingested_at = Some(snapshot.sampled_at);
            // Only a current snapshot may clear a transient notice. Doing this
            // here covers every shell (native, live web, demo). Persistent
            // notices are never touched (SECURITY §5).
            self.clear_transient_status();
        }
        // `network_totals()` sums the cumulative counters of the physical
        // interfaces, so those totals are only comparable while the contributor
        // set is unchanged. A hot-plugged interface brings its own boot-time
        // counter: without a guard the aggregate delta looks like a one-frame
        // multi-GB/s spike (WC-IFHOT). Treat a changed set like the first sample
        // after startup — no rate for this frame, then re-baseline.
        let physical_interfaces: HashSet<String> = snapshot
            .network
            .interfaces
            .iter()
            .filter(|iface| !tm_core::model::is_virtual_interface(&iface.name))
            .map(|iface| iface.name.clone())
            .collect();
        // A late arrival must not overwrite the most recently accepted rate or
        // append to the chart history: its counters are stale (or its physical
        // interface set differs), so the computed rate can be a spurious 0 that
        // would replace `network_rates` and write a zero spike into `history`
        // (R2-BV01). Only current snapshots feed those series — the display
        // (snapshot/selection) is still refreshed below for every sample, so
        // CPU/memory percentages keep updating as before.
        if !is_late {
            let (rx_rate, tx_rate) = match self.last_network {
                Some((previous_rx, previous_tx))
                    if self.last_network_interfaces == physical_interfaces =>
                {
                    // Use the probe's monotonic elapsed time, not a wall-clock
                    // delta: NTP/VM resume can move `sampled_at` backwards and
                    // produce a 1000x rate spike (R49 audit).
                    let elapsed = snapshot.sampled_elapsed;
                    NetworkCounters::new(snapshot.network_totals().0, snapshot.network_totals().1)
                        .rate_since(NetworkCounters::new(previous_rx, previous_tx), elapsed)
                }
                _ => (0, 0),
            };
            self.network_rates = (rx_rate, tx_rate);
            let (total_rx, total_tx) = snapshot.network_totals();
            self.last_network = Some((total_rx, total_tx));
            self.last_network_interfaces = physical_interfaces;
            self.history.push(
                snapshot.cpu.total_percent,
                snapshot.memory.used_percent(),
                rx_rate,
                tx_rate,
            );
        }

        // Distinguish "no snapshot yet", "explicitly cleared" and "selected
        // process disappeared": only the first two cases may auto-select, and
        // `Select(None)` must stay cleared across recompute/ingest (R29 audit).
        let had_snapshot = self.snapshot.is_some();
        let had_selection = self.selected.is_some();
        let selected_still_exists = self
            .selected
            .is_some_and(|key| snapshot.process(key).is_some());
        // `recompute_visible()` re-syncs the hint against the *new* selection,
        // so preserve the previous row position for the disappearance fallback.
        let previous_hint = self.selection_hint;
        self.snapshot = Some(snapshot);
        // Expansion state is keyed by the redacted app name; a new sample can
        // remove a group entirely, so drop keys that no longer resolve.
        self.prune_expanded_app_groups();
        self.recompute_visible();
        if !had_snapshot || (had_selection && !selected_still_exists) {
            let fallback_index = self
                .visible
                .get(previous_hint.min(self.visible.len().saturating_sub(1)))
                .copied();
            let fallback_key = fallback_index.and_then(|index| {
                self.snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.processes[index].key)
            });
            self.selected = fallback_key;
        }
        self.ensure_selection_visible();
        self.sync_selection_hint();
        // A disappeared selection may have fallen back to a different process;
        // drop samples that belong to the previous identity before appending.
        self.sync_selected_cpu_history();
        if let Some(key) = self.selected {
            let cpu = self
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.process(key))
                .map(|process| normalise_percent(process.cpu_percent));
            if let Some(cpu) = cpu {
                self.selected_cpu_history.push(cpu);
            }
        }
    }

    /// Sets a transient status message (cleared on the next successful sample).
    ///
    /// Persistent notices survive automatic transitions (ingest / pause-resume)
    /// but an explicit transient message may replace them on screen, as
    /// documented in SECURITY.md §5. The standing notice is retained in the
    /// persistent slot and restored by [`Self::clear_transient_status`], so the
    /// demo guidance never degrades into "ステータス: 変更なし" after an export
    /// confirmation (SC-STATUSRESTORE). All transient writes go through this
    /// method so a future policy change needs one edit here.
    pub fn set_status(&mut self, text: impl Into<String>, is_error: bool) {
        self.status = Some(if is_error {
            StatusMessage::error(text)
        } else {
            StatusMessage::info(text)
        });
    }

    /// Sets a persistent status message (startup/configuration notices).
    ///
    /// Unlike transient messages, it is not cleared by a successful sample.
    /// Existing callers keep the error tone; informational notices should use
    /// [`Self::set_persistent_info`] or [`Self::set_persistent_status_with_tone`].
    pub fn set_persistent_status(&mut self, text: impl Into<String>) {
        self.set_persistent_status_with_tone(text, true);
    }

    /// Sets a persistent informational status (`is_error = false`).
    ///
    /// Used for neutral guidance such as the browser demo-mode hint. It keeps
    /// the `[情報]` prefix and the persistent flag, so a successful sample does
    /// not clear it while assistive technology never announces it as an error.
    pub fn set_persistent_info(&mut self, text: impl Into<String>) {
        let status = StatusMessage::persistent_info(text);
        self.persistent_status = Some(status.clone());
        self.status = Some(status);
    }

    /// Sets a persistent status with an explicit severity (V2-HELP).
    ///
    /// `is_error = true` is the legacy `set_persistent_status` behaviour;
    /// `false` produces the same persistent message with the info prefix.
    pub fn set_persistent_status_with_tone(&mut self, text: impl Into<String>, is_error: bool) {
        let status = if is_error {
            StatusMessage::persistent_error(text)
        } else {
            StatusMessage::persistent_info(text)
        };
        self.persistent_status = Some(status.clone());
        self.status = Some(status);
    }

    /// Clears the current status only when it is transient (R3-U05).
    ///
    /// When a standing notice was temporarily covered by a transient message,
    /// it is restored from the persistent slot instead of leaving the live
    /// region empty (SC-STATUSRESTORE). Live mode never installs a demo notice,
    /// so this cannot surface demo guidance there.
    pub fn clear_transient_status(&mut self) {
        if self
            .status
            .as_ref()
            .is_some_and(|status| !status.persistent)
        {
            self.status = self.persistent_status.clone();
        }
    }

    /// Clears the status message and its persistent restore slot.
    ///
    /// The native shell's probe-retry recovery uses this to dismiss a matching
    /// startup failure for good; restoring it on a later transient clear would
    /// resurrect a failure the user already saw recovered.
    pub fn clear_status(&mut self) {
        self.status = None;
        self.persistent_status = None;
    }

    /// Formats the overview CPU label, shared by both shells.
    pub fn cpu_label(&self) -> String {
        match &self.snapshot {
            Some(snapshot) => format::format_percent(snapshot.cpu.total_percent),
            None => "—".to_string(),
        }
    }

    fn request_signal(&mut self, key: ProcessKey, signal: Signal) {
        if !self.platform.supports_process_control {
            self.set_status("この環境ではプロセス操作は無効です（読み取り専用）", true);
            return;
        }
        // UI buttons already hide unsupported signals; reject them here too so
        // a direct Command cannot bypass the per-signal capability gate.
        if !self.platform.supports_signal(signal) {
            self.set_status("この環境ではこのシグナルは使用できません", true);
            return;
        }
        let Some(process) = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.process(key))
        else {
            self.set_status("対象のプロセスは既に終了しています", true);
            return;
        };
        if self.is_protected(key) {
            self.set_status(
                "PID 1 と自分自身は操作できません（安全のため保護されています）",
                true,
            );
            return;
        }
        // Do not open a confirmation dialog from a stale snapshot while paused:
        // `confirm_kill` would reject it anyway, so the dialog could never be
        // completed (R34 audit). Capability/protection messages take priority.
        if self.paused {
            self.set_status(
                "更新が一時停止中のため操作できません。再開してからやり直してください",
                true,
            );
            return;
        }
        // Never silently replace an unanswered confirmation with a different
        // target/signal: the dialog the user is looking at would change under
        // them (R2-F02).
        if self.pending_kill.is_some() {
            self.set_status(
                "確認待ちの操作があります。先に確定またはキャンセルしてください",
                false,
            );
            return;
        }
        let confirmation_note = signal_confirmation_note(signal);
        self.pending_kill = Some(PendingKill {
            key,
            name: process.name.clone(),
            command: process.command.clone(),
            user: process.user.clone(),
            signal,
            confirmation_note,
        });
        // Surface the signal-specific warning in the status bar too. It stays
        // transient, so the next successfully ingested snapshot clears it just
        // like any other notice (R3-U05).
        self.set_status(confirmation_note, false);
    }

    fn confirm_kill(&mut self) -> Vec<Effect> {
        // A paused dashboard shows stale data: the process may have exited and
        // its PID been reused without our knowledge. Never signal from a stale
        // confirmation (R2-F01).
        if self.paused {
            self.pending_kill = None;
            self.set_status(
                "更新が一時停止中のため操作できません。再開してからやり直してください",
                true,
            );
            return Vec::new();
        }
        let Some(pending) = self.pending_kill.take() else {
            return Vec::new();
        };
        let still_running = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.process(pending.key))
            .is_some();
        if !still_running {
            self.set_status(
                "対象のプロセスは既に終了しています（PID 再利用を検出）",
                true,
            );
            return Vec::new();
        }
        vec![Effect::Signal {
            key: pending.key,
            signal: pending.signal,
        }]
    }

    fn select_relative(&mut self, delta: i32) {
        if self.visible.is_empty() {
            self.selected = None;
            return;
        }
        let current = self.selected_visible_index().unwrap_or(0) as i64;
        let max = self.visible.len() as i64 - 1;
        let target = (current + delta as i64).clamp(0, max) as usize;
        if let Some(snapshot) = &self.snapshot {
            self.selected = Some(snapshot.processes[self.visible[target]].key);
            self.selection_hint = target;
        }
        self.sync_selected_cpu_history();
    }

    fn recompute_visible(&mut self) {
        let Some(snapshot) = &self.snapshot else {
            self.visible.clear();
            self.visible_depths.clear();
            self.visible_has_children.clear();
            return;
        };
        let base = self.display_base(snapshot);
        if self.app_group_view {
            // Grouping drives the *existing* visible arrays so the native and
            // web renderers need no changes (R1-VISIBLE). `app_group_rows()`
            // already applied the filter, sort and expansion state; a group's
            // index is its first member, its depth is 0 and a group with more
            // than one member advertises children.
            let rows = self.app_group_rows();
            self.visible = rows.iter().map(|row| row.index).collect();
            self.visible_depths = rows.iter().map(|row| row.depth).collect();
            self.visible_has_children = rows.iter().map(|row| row.has_children).collect();
        } else if self.tree_view {
            let rows = flatten_process_tree(&snapshot.processes, &base, &self.collapsed);
            self.visible = rows.iter().map(|row| row.index).collect();
            self.visible_depths = rows.iter().map(|row| row.depth).collect();
            self.visible_has_children = rows.iter().map(|row| row.has_children).collect();
        } else {
            self.visible = base;
            self.visible_depths = vec![0; self.visible.len()];
            self.visible_has_children = vec![false; self.visible.len()];
        }
        self.ensure_selection_visible();
        self.sync_selection_hint();
        self.sync_selected_cpu_history();
    }

    /// Whether `candidate` is a descendant of `ancestor` via `parent_pid`.
    /// Guards against cycles and missing parents so a corrupt snapshot cannot
    /// make this loop forever.
    fn is_descendant(&self, candidate: ProcessKey, ancestor: ProcessKey) -> bool {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return false;
        };
        let mut current = candidate;
        let mut seen = std::collections::HashSet::new();
        while seen.insert(current) {
            let Some(process) = snapshot.process(current) else {
                return false;
            };
            let Some(parent) = process.parent_pid else {
                return false;
            };
            if parent == ancestor.pid {
                return true;
            }
            let Some(parent_process) = snapshot
                .processes
                .iter()
                .find(|candidate| candidate.key.pid == parent)
            else {
                return false;
            };
            current = parent_process.key;
        }
        false
    }

    fn ensure_selection_visible(&mut self) {
        // An explicitly cleared selection (`Select(None)`) must stay cleared;
        // recomputing the visible rows must not auto-select the first one.
        if self.selected.is_none() {
            return;
        }
        let Some(snapshot) = &self.snapshot else {
            self.selected = None;
            return;
        };
        if let Some(selected) = self.selected {
            if self
                .visible
                .iter()
                .any(|&index| snapshot.processes[index].key == selected)
            {
                return;
            }
            // A collapsed group hides all but its first member. Represent the
            // hidden selection with that group row instead of jumping to an
            // unrelated first row (R1-VISIBLE).
            if let Some(representative) = self.grouped_representative(selected) {
                self.selected = Some(representative);
                return;
            }
        }
        self.selected = self
            .visible
            .first()
            .map(|&index| snapshot.processes[index].key);
    }

    fn sync_selection_hint(&mut self) {
        if let Some(position) = self.selected_visible_index() {
            self.selection_hint = position;
        }
    }

    /// Drops the retained CPU series whenever the selection identity changes.
    ///
    /// Called from every path that can move the selection (explicit select,
    /// relative move, filter/sort recompute and snapshot fallback) so samples
    /// from process A are never shown as process B's history (R1-I08).
    fn sync_selected_cpu_history(&mut self) {
        if self.selected != self.selected_cpu_history_key {
            self.selected_cpu_history.clear();
            self.selected_cpu_history_key = self.selected;
        }
    }
}

/// Explains what the confirmed signal actually does (R3-U03).
///
/// Kept next to the state machine, not in a shell, so the native and browser
/// dialogs show the same wording. Only `Kill` is irreversible; `Stop` and
/// `Continue` can be undone, and `Terminate` gives the process a chance to
/// clean up. Returning `&'static str` keeps this allocation-free and usable
/// from `const` contexts.
pub const fn signal_confirmation_note(signal: Signal) -> &'static str {
    match signal {
        Signal::Terminate => {
            "終了 (SIGTERM) はプロセスに終了を通知します。保存していない作業は失われる可能性があります。"
        }
        Signal::Kill => "強制終了 (SIGKILL) はプロセスを即座に終了させます。この操作は取り消せません。",
        Signal::Stop => "一時停止 (SIGSTOP) はプロセスを停止します。再開 (SIGCONT) で元に戻せます。",
        Signal::Continue => "再開 (SIGCONT) は停止中のプロセスを再開します。",
    }
}

/// A trend that cannot be judged yet (no second sample to compare against).
fn unknown_trend() -> TrendDelta {
    TrendDelta {
        direction: TrendDirection::Unknown,
        amount: "—".to_string(),
    }
}

/// Shared helper for the percentage series (CPU / memory).
fn percent_trend(history: &RingBuffer<f32>) -> TrendDelta {
    if history.len() < 2 {
        return unknown_trend();
    }
    trend_delta(
        history.earliest().copied().map(f64::from),
        history.latest().copied().map(f64::from),
        PERCENT_TREND_EPSILON,
        TrendUnit::PercentPoints,
    )
}

fn clamp_interval(interval: Duration) -> Duration {
    interval.clamp(MIN_REFRESH_INTERVAL, MAX_REFRESH_INTERVAL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tm_core::demo::{demo_snapshot, DemoProbe};
    use tm_core::{ProcessSnapshot, SortDirection, SystemProbe};

    fn dashboard() -> Dashboard {
        Dashboard::new(PlatformInfo::native("Linux"), DEFAULT_REFRESH_INTERVAL)
    }

    /// Dashboard pinned to the legacy flat rows for non-regression tests.
    ///
    /// The default is application grouping (R1-VISIBLE), so tests that check
    /// one-row-per-process behaviour must opt out explicitly.
    fn flat_dashboard() -> Dashboard {
        let mut dashboard = dashboard();
        assert!(dashboard.app_group_view());
        dashboard.apply(Command::SetAppGrouping(false));
        dashboard
    }

    fn ingest_ticks(dashboard: &mut Dashboard, ticks: u64) {
        for tick in 0..ticks {
            dashboard.ingest(demo_snapshot(tick));
        }
    }

    /// Demo snapshot with an extra physical interface carrying boot-time totals.
    fn snapshot_with_usb0(tick: u64, rx: u64, tx: u64) -> tm_core::SystemSnapshot {
        let mut snapshot = demo_snapshot(tick);
        snapshot
            .network
            .interfaces
            .push(tm_core::NetworkInterfaceSnapshot {
                name: "usb0".to_string(),
                received_bytes: rx,
                transmitted_bytes: tx,
            });
        snapshot
    }

    fn firefox_key() -> ProcessKey {
        tm_core::demo::demo_process_key(800).expect("demo firefox process")
    }

    #[test]
    fn persistent_notice_survives_pause_resume_and_rate_starts_fresh() {
        let mut dashboard = dashboard();
        dashboard.set_persistent_status("デモデータで起動しました");
        dashboard.apply(Command::SetPaused(true));
        dashboard.apply(Command::SetPaused(false));
        let status = dashboard.status().expect("persistent notice survives");
        assert!(status.persistent);
        assert_eq!(
            status.text,
            format!("{STATUS_PREFIX_ERROR} デモデータで起動しました")
        );

        // A sample after resume starts a fresh network baseline, so bytes
        // received while paused are not reported as an instant-rate spike.
        dashboard.ingest(demo_snapshot(0));
        assert_eq!(dashboard.network_rates(), (0, 0));
        dashboard.ingest(demo_snapshot(1));
        assert!(dashboard.network_rates().0 > 0 || dashboard.network_rates().1 > 0);
    }

    #[test]
    fn pause_notice_survives_in_flight_ingest() {
        let mut dashboard = dashboard();
        dashboard.ingest(demo_snapshot(0));
        dashboard.apply(Command::SetPaused(true));
        assert_eq!(
            dashboard.status().map(|status| status.text.as_str()),
            Some("[情報] 更新を一時停止しました")
        );
        // A sample that was in flight when pause was pressed must not wipe the
        // notice: `ingest` is a no-op while paused.
        dashboard.ingest(demo_snapshot(1));
        assert_eq!(
            dashboard.status().map(|status| status.text.as_str()),
            Some("[情報] 更新を一時停止しました")
        );

        dashboard.apply(Command::SetPaused(false));
        dashboard.ingest(demo_snapshot(2));
        assert!(dashboard.status().is_none());
    }

    #[test]
    fn transient_status_is_cleared_but_persistent_status_survives() {
        let mut dashboard = dashboard();
        dashboard.set_status("一時的なお知らせ", false);
        dashboard.clear_transient_status();
        assert!(dashboard.status().is_none());

        dashboard.set_persistent_status("起動時の警告");
        dashboard.clear_transient_status();
        let status = dashboard.status().expect("persistent status survives");
        assert!(status.persistent);
        assert_eq!(status.text, format!("{STATUS_PREFIX_ERROR} 起動時の警告"));
    }

    #[test]
    fn demo_persistent_info_is_restored_after_a_transient_notice_is_cleared() {
        let mut dashboard = dashboard();
        // Demo mode installs a standing info notice at startup.
        dashboard.set_persistent_info("デモ表示中。実データ接続はヘルプ F1 を参照");
        let standing = dashboard.status().cloned().expect("demo guidance");

        // An export confirmation is transient and briefly covers the guidance.
        dashboard.set_status("CSV をエクスポートしました", false);
        let transient = dashboard.status().expect("export notice");
        assert!(!transient.persistent);
        assert_ne!(transient.text, standing.text);

        // The next accepted sample clears the transient notice; the standing
        // demo guidance must come back instead of an empty live region.
        dashboard.ingest(demo_snapshot(0));
        assert_eq!(dashboard.status(), Some(&standing));

        // A second cycle must behave identically (no one-shot restore).
        dashboard.set_status("JSON をエクスポートしました", false);
        dashboard.clear_transient_status();
        assert_eq!(dashboard.status(), Some(&standing));
    }

    #[test]
    fn live_mode_without_a_persistent_notice_stays_empty_after_transient_clear() {
        let mut dashboard = dashboard();
        // Live mode only sets a transient "connecting" notice; no demo guidance
        // is ever installed, so clearing it must not surface demo text there.
        dashboard.set_status("エージェントに接続しています…", false);
        dashboard.ingest(demo_snapshot(0));
        assert!(
            dashboard.status().is_none(),
            "live mode must not show the demo notice"
        );
    }

    #[test]
    fn persistent_error_notices_are_restored_and_explicit_clear_dismisses_them() {
        for text in ["設定ファイルが破損しています", "設定を保存できませんでした"]
        {
            let mut dashboard = dashboard();
            dashboard.set_persistent_status(text);
            let standing = dashboard.status().cloned().expect("persistent error");
            assert!(standing.persistent && standing.is_error);

            // A transient notice may cover it on screen but must not destroy
            // the corrupt / save-failure notice (SECURITY.md §5, non-regression).
            dashboard.set_status("一時的なお知らせ", false);
            dashboard.clear_transient_status();
            assert_eq!(dashboard.status(), Some(&standing), "{text}");

            // An explicit clear (native probe-retry recovery) dismisses both,
            // so a recovered startup failure cannot resurrect later.
            dashboard.clear_status();
            dashboard.set_status("一時的なお知らせ", false);
            dashboard.clear_transient_status();
            assert!(dashboard.status().is_none(), "{text} must not resurrect");
        }
    }

    #[test]
    fn status_text_identifies_severity_without_colour() {
        let mut dashboard = dashboard();
        dashboard.set_status("情報のみ", false);
        let info = dashboard.status().expect("info status");
        assert!(!info.is_error);
        assert!(!info.persistent);
        assert!(info.text.starts_with(STATUS_PREFIX_INFO), "{}", info.text);
        assert!(info.text.ends_with("情報のみ"), "{}", info.text);

        dashboard.set_status("失敗しました", true);
        let error = dashboard.status().expect("error status");
        assert!(error.is_error);
        assert!(!error.persistent);
        assert!(
            error.text.starts_with(STATUS_PREFIX_ERROR),
            "{}",
            error.text
        );
        assert!(error.text.ends_with("失敗しました"), "{}", error.text);

        dashboard.set_persistent_status("起動時の警告");
        let persistent = dashboard.status().expect("persistent status");
        assert!(persistent.is_error);
        assert!(persistent.persistent);
        assert!(
            persistent.text.starts_with(STATUS_PREFIX_ERROR),
            "{}",
            persistent.text
        );
    }

    #[test]
    fn persistent_info_is_info_toned_and_survives_ingest() {
        let mut dashboard = dashboard();
        dashboard.set_persistent_info("デモ表示中。実データ接続はヘルプ F1 を参照");
        let expected = dashboard.status().cloned().expect("persistent info");
        assert!(!expected.is_error);
        assert!(expected.persistent);
        assert!(
            expected.text.starts_with(STATUS_PREFIX_INFO),
            "{}",
            expected.text
        );
        dashboard.ingest(demo_snapshot(0));
        assert_eq!(dashboard.status(), Some(&expected));

        // The tone-taking entry point must agree with the dedicated helper and
        // leave the legacy error-tone callers untouched.
        dashboard.set_persistent_status_with_tone("無線の案内", false);
        assert_eq!(
            dashboard.status().map(|status| status.is_error),
            Some(false)
        );
        dashboard.set_persistent_status("起動時の警告");
        assert_eq!(dashboard.status().map(|status| status.is_error), Some(true));
        assert_eq!(
            dashboard.status().map(|status| status.persistent),
            Some(true)
        );
    }

    #[test]
    fn prefixed_transient_status_is_cleared_by_ingest_but_persistent_is_not() {
        let mut dashboard = dashboard();
        dashboard.set_status("一時的なお知らせ", true);
        assert!(dashboard
            .status()
            .expect("transient")
            .text
            .starts_with(STATUS_PREFIX_ERROR));
        dashboard.ingest(demo_snapshot(0));
        assert!(
            dashboard.status().is_none(),
            "an ingested snapshot clears a transient notice even when its text has a marker"
        );

        dashboard.set_persistent_status("デモデータで起動しました");
        dashboard.ingest(demo_snapshot(1));
        let status = dashboard.status().expect("persistent survives ingest");
        assert!(status.persistent);
        assert!(
            status.text.starts_with(STATUS_PREFIX_ERROR),
            "{}",
            status.text
        );
    }

    #[test]
    fn ingest_is_ignored_while_paused() {
        let mut dashboard = dashboard();
        dashboard.ingest(demo_snapshot(0));
        let before_at = dashboard.snapshot().unwrap().sampled_at;
        let before_len = dashboard.history().len();

        dashboard.apply(Command::SetPaused(true));
        // An in-flight sample completing after pause must not move the display.
        dashboard.ingest(demo_snapshot(1));
        assert_eq!(dashboard.history().len(), before_len);
        assert_eq!(dashboard.snapshot().unwrap().sampled_at, before_at);

        dashboard.apply(Command::SetPaused(false));
        dashboard.ingest(demo_snapshot(2));
        assert_eq!(dashboard.history().len(), before_len + 1);
        assert_ne!(dashboard.snapshot().unwrap().sampled_at, before_at);
    }

    #[test]
    fn tick_samples_only_when_running() {
        let mut dashboard = dashboard();
        assert_eq!(dashboard.apply(Command::Tick), vec![Effect::SampleNow]);
        dashboard.apply(Command::SetPaused(true));
        assert!(dashboard.apply(Command::Tick).is_empty());
        assert_eq!(
            dashboard.apply(Command::SetPaused(false)),
            vec![Effect::SampleNow]
        );
    }

    #[test]
    fn interval_is_clamped() {
        let mut dashboard = dashboard();
        dashboard.apply(Command::SetInterval(Duration::from_millis(1)));
        assert_eq!(dashboard.interval(), MIN_REFRESH_INTERVAL);
        dashboard.apply(Command::SetInterval(Duration::from_secs(3_600)));
        assert_eq!(dashboard.interval(), MAX_REFRESH_INTERVAL);
    }

    #[test]
    fn effective_interval_follows_the_snapshot_plan_without_changing_startup() {
        use tm_core::{
            BatterySnapshot, BatteryState, PowerSnapshot, PowerSource, SamplingPlan,
            SamplingProfile, BATTERY_INTERVAL_MS, LOW_BATTERY_PERCENT,
        };

        let mut dashboard = dashboard();
        // No snapshot yet: exactly the configured cadence.
        assert_eq!(dashboard.effective_interval(), DEFAULT_REFRESH_INTERVAL);
        assert!(dashboard.power().is_none());
        assert!(dashboard.sampling_plan().is_none());
        assert!(dashboard.cgroup_groups().is_empty());

        // The demo snapshot is on AC and keeps the interactive 1 s cadence.
        dashboard.ingest(demo_snapshot(0));
        assert_eq!(dashboard.effective_interval(), DEFAULT_REFRESH_INTERVAL);
        assert_eq!(dashboard.cgroup_groups().len(), 2);

        // A battery plan raises a too-fast configured interval as a floor.
        let mut battery = demo_snapshot(1);
        battery.power = PowerSnapshot {
            source: PowerSource::Battery,
            batteries: vec![BatterySnapshot {
                name: "BAT0".into(),
                capacity_percent: Some(LOW_BATTERY_PERCENT + 10),
                state: BatteryState::Discharging,
            }],
        };
        battery.sampling =
            tm_core::recommended_sampling_plan(&battery.power, tm_core::WindowVisibility::Visible);
        dashboard.ingest(battery);
        assert_eq!(
            dashboard.sampling_plan(),
            Some(SamplingPlan {
                interval_ms: BATTERY_INTERVAL_MS,
                profile: SamplingProfile::Battery,
            })
        );
        assert_eq!(
            dashboard.effective_interval(),
            Duration::from_millis(BATTERY_INTERVAL_MS)
        );
        // A deliberately slower user setting is always respected.
        dashboard.apply(Command::SetInterval(Duration::from_secs(10)));
        assert_eq!(dashboard.effective_interval(), MAX_REFRESH_INTERVAL);
    }

    #[test]
    fn ingest_fills_history_and_rates() {
        let mut dashboard = dashboard();
        ingest_ticks(&mut dashboard, 2);
        assert_eq!(dashboard.history().len(), 2);
        assert!(dashboard.snapshot().is_some());
        // wlan0 rx +1_234_567 over one second. The demo snapshot also carries a
        // `lo` interface (+4_096), which the overview must ignore: loopback
        // traffic is not network usage and would otherwise be double-counted.
        assert_eq!(dashboard.network_rates(), (1_234_567, 321_456));
    }

    #[test]
    fn current_snapshot_updates_rates_and_history() {
        let mut dashboard = dashboard();
        // First sample establishes the baseline: no rate yet, one history row.
        dashboard.ingest(demo_snapshot(0));
        assert_eq!(dashboard.network_rates(), (0, 0));
        assert_eq!(dashboard.history().len(), 1);
        assert_eq!(dashboard.history().network_rx_rate.latest(), Some(&0));

        // A fresh sample with advanced counters reports one tick of traffic and
        // appends a matching history row (R2-BV01 happy path).
        dashboard.ingest(demo_snapshot(1));
        assert_eq!(dashboard.network_rates(), (1_234_567, 321_456));
        assert_eq!(dashboard.history().len(), 2);
        assert_eq!(
            dashboard.history().network_rx_rate.latest(),
            Some(&1_234_567)
        );
        assert_eq!(dashboard.history().network_tx_rate.latest(), Some(&321_456));
    }

    #[test]
    fn network_rates_ignore_wall_clock_regressions() {
        let mut dashboard = dashboard();
        let first = demo_snapshot(0);
        dashboard.ingest(first);
        let mut second = demo_snapshot(1);
        // Simulate an NTP step back: wall clock moves backwards while the
        // probe still reports a full one-second monotonic interval (R49).
        second.sampled_at = tm_core::Timestamp::from_millis(1);
        dashboard.ingest(second);
        assert_eq!(dashboard.network_rates(), (1_234_567, 321_456));
    }

    #[test]
    fn sustained_clock_regression_does_not_accumulate_network_rates() {
        let mut regressing = dashboard();
        let mut dashboard = dashboard();
        dashboard.ingest(demo_snapshot(0));
        assert_eq!(dashboard.network_rates(), (0, 0));
        let expected = (1_234_567, 321_456);

        // The wall clock steps back once and stays there while the probe keeps
        // producing one second of new traffic per tick (WB-CLOCK).
        for tick in 1..=4 {
            let mut snapshot = demo_snapshot(tick);
            snapshot.sampled_at = tm_core::Timestamp::from_millis(1);
            dashboard.ingest(snapshot);
            assert_eq!(
                dashboard.network_rates(),
                expected,
                "tick {tick} must not accumulate an obsolete baseline"
            );
        }

        // The same holds when the clock keeps drifting further backwards.
        regressing.ingest(demo_snapshot(0));
        let base = demo_snapshot(0).sampled_at.unix_millis;
        for tick in 1..=4 {
            let mut snapshot = demo_snapshot(tick);
            snapshot.sampled_at = tm_core::Timestamp::from_millis(base - tick * 1_000);
            regressing.ingest(snapshot);
            assert_eq!(
                regressing.network_rates(),
                expected,
                "progressively regressing tick {tick} must not accumulate an obsolete baseline"
            );
        }
    }

    #[test]
    fn sustained_clock_regression_clears_transient_status() {
        let mut dashboard = dashboard();
        dashboard.ingest(demo_snapshot(0));
        dashboard.set_status("直近の取得失敗", true);
        assert!(dashboard.status().is_some_and(|status| status.is_error));

        for tick in 1..=4 {
            let mut snapshot = demo_snapshot(tick);
            snapshot.sampled_at = tm_core::Timestamp::from_millis(1);
            dashboard.ingest(snapshot);
            assert!(
                dashboard.status().is_none(),
                "a newer sample in a stepped-back clock era must clear the transient error (tick {tick})"
            );
        }

        // A genuinely older sample (lower counters *and* older clock) still
        // preserves the error: the late-arrival contract is unchanged.
        dashboard.set_status("直近の取得失敗", true);
        let mut older = demo_snapshot(0);
        older.sampled_at = tm_core::Timestamp::from_millis(0);
        dashboard.ingest(older);
        assert!(dashboard.status().is_some_and(|status| status.is_error));
    }

    #[test]
    fn forward_progress_after_clock_rebase_reports_normal_rates() {
        let mut dashboard = dashboard();
        let mut first = demo_snapshot(0);
        first.sampled_at = tm_core::Timestamp::from_millis(1);
        dashboard.ingest(first);
        assert_eq!(dashboard.network_rates(), (0, 0));

        let mut next = demo_snapshot(1);
        next.sampled_at = tm_core::Timestamp::from_millis(1_001);
        dashboard.ingest(next);
        assert_eq!(dashboard.network_rates(), (1_234_567, 321_456));

        let mut later = demo_snapshot(2);
        later.sampled_at = tm_core::Timestamp::from_millis(2_001);
        dashboard.ingest(later);
        assert_eq!(dashboard.network_rates(), (1_234_567, 321_456));
    }

    #[test]
    fn new_interface_does_not_spike_the_network_rate() {
        let mut dashboard = dashboard();
        ingest_ticks(&mut dashboard, 3); // baseline: tick 2, steady traffic
        assert_eq!(dashboard.network_rates(), (1_234_567, 321_456));

        // A physical interface appears with large boot-time counters. The
        // aggregate jumps by 50 GB/20 GB even though no traffic moved.
        dashboard.ingest(snapshot_with_usb0(3, 50_000_000_000, 20_000_000_000));
        assert_eq!(
            dashboard.network_rates(),
            (0, 0),
            "the hot-plug frame must re-baseline instead of reporting the new IF's boot counter"
        );

        // With the interface set stable again, only real traffic is reported.
        dashboard.ingest(snapshot_with_usb0(4, 50_000_000_000, 20_000_000_000));
        assert_eq!(dashboard.network_rates(), (1_234_567, 321_456));
    }

    #[test]
    fn removed_interface_rebaselines_without_a_negative_rate() {
        let mut dashboard = dashboard();
        dashboard.ingest(snapshot_with_usb0(0, 50_000_000_000, 20_000_000_000));
        assert_eq!(dashboard.network_rates(), (0, 0));
        dashboard.ingest(snapshot_with_usb0(1, 50_000_000_000, 20_000_000_000));
        assert_eq!(dashboard.network_rates(), (1_234_567, 321_456));

        // usb0 disappears: totals fall below the old baseline. The saturated
        // subtraction already yielded (0, 0) here; the set-change guard must
        // preserve that result and move the baseline to the reduced totals.
        dashboard.ingest(demo_snapshot(2));
        assert_eq!(dashboard.network_rates(), (0, 0));
        dashboard.ingest(demo_snapshot(3));
        assert_eq!(dashboard.network_rates(), (1_234_567, 321_456));
    }

    #[test]
    fn late_snapshot_preserves_the_latest_transient_error() {
        let mut dashboard = dashboard();
        dashboard.ingest(demo_snapshot(2));
        dashboard.set_status("直近の取得失敗", true);
        let before = dashboard.status().expect("transient error").clone();
        assert!(before.is_error);
        assert!(!before.persistent);

        // A sample taken before the newest one arrives out of order.
        let late = demo_snapshot(1);
        assert!(late.sampled_at < dashboard.snapshot().unwrap().sampled_at);
        dashboard.ingest(late);

        let after = dashboard
            .status()
            .expect("a late arrival must not clear the transient error");
        assert_eq!(*after, before);

        // A persistent notice is likewise untouched by a late arrival
        // (SECURITY §5): only an explicit set/clear may replace it.
        dashboard.set_persistent_status("起動時の警告");
        dashboard.ingest(demo_snapshot(0));
        let persistent = dashboard.status().expect("persistent survives");
        assert!(persistent.persistent);
        assert_eq!(
            persistent.text,
            format!("{STATUS_PREFIX_ERROR} 起動時の警告")
        );
    }

    #[test]
    fn late_snapshot_does_not_move_the_network_baseline() {
        let mut dashboard = dashboard();
        dashboard.ingest(demo_snapshot(1)); // baseline becomes tick 1 counters
        assert_eq!(dashboard.network_rates(), (0, 0));
        dashboard.ingest(demo_snapshot(2)); // one tick of traffic since tick 1
        let expected = (1_234_567, 321_456);
        assert_eq!(dashboard.network_rates(), expected);

        // An out-of-order sample carrying older counters must not become the
        // new baseline, otherwise the next fresh sample would measure two
        // ticks of traffic from tick 0 and report twice the real rate.
        dashboard.ingest(demo_snapshot(0));

        dashboard.ingest(demo_snapshot(3));
        assert_eq!(
            dashboard.network_rates(),
            expected,
            "rate must still be measured from the newest baseline"
        );
    }

    #[test]
    fn late_snapshot_with_equal_counters_keeps_rates_and_history() {
        let mut dashboard = dashboard();
        dashboard.ingest(demo_snapshot(1)); // baseline tick 1
        dashboard.ingest(demo_snapshot(2)); // one tick of traffic
        let expected_rate = (1_234_567, 321_456);
        assert_eq!(dashboard.network_rates(), expected_rate);
        let history_rx = dashboard.history().network_rx_rate.to_vec();
        let history_tx = dashboard.history().network_tx_rate.to_vec();
        let history_len = dashboard.history().len();

        dashboard.set_status("直近の取得失敗", true);

        // Identical counters to the newest accepted baseline, but an older
        // clock: equal counters are not progress, so this is a late arrival
        // (not a wall-clock rebase). Its rate would compute to a spurious 0,
        // which must neither overwrite `network_rates` nor be appended to
        // `history` (R2-BV01). The transient error must also survive (the
        // existing late_snapshot_* contract).
        let mut late = demo_snapshot(2);
        late.sampled_at = tm_core::Timestamp::from_millis(late.sampled_at.unix_millis - 2_000);
        assert_eq!(late.network_totals(), demo_snapshot(2).network_totals());
        assert!(late.sampled_at < dashboard.snapshot().unwrap().sampled_at);
        dashboard.ingest(late);

        assert_eq!(dashboard.network_rates(), expected_rate);
        assert_eq!(dashboard.history().len(), history_len);
        assert_eq!(dashboard.history().network_rx_rate.to_vec(), history_rx);
        assert_eq!(dashboard.history().network_tx_rate.to_vec(), history_tx);
        assert!(
            history_rx.last().is_some_and(|&rx| rx > 0),
            "a late arrival must not write a 0 spike into the rate history"
        );
        assert!(
            dashboard.status().is_some_and(|status| status.is_error),
            "a late arrival must not clear the transient error"
        );
    }

    #[test]
    fn late_snapshot_keeps_an_explicitly_cleared_selection() {
        let mut dashboard = dashboard();
        dashboard.ingest(demo_snapshot(2));
        dashboard.apply(Command::Select(None));
        assert_eq!(dashboard.selected_key(), None);

        // Re-syncing against the late snapshot must not auto-select a row.
        dashboard.ingest(demo_snapshot(1));
        assert_eq!(dashboard.selected_key(), None);
    }

    #[test]
    fn idle_snapshot_with_older_timestamp_is_late_and_keeps_baseline() {
        let mut dashboard = dashboard();
        let current = demo_snapshot(3);
        let current_at = current.sampled_at;
        let current_totals = current.network_totals();
        dashboard.ingest(current);
        assert_eq!(dashboard.network_rates(), (0, 0));
        let baseline_interfaces = dashboard.last_network_interfaces.clone();

        dashboard.set_status("直近の取得失敗", true);
        assert!(dashboard.status().is_some_and(|status| status.is_error));

        // An idle delayed sample: `usb0` carries zero bytes so the cumulative
        // totals are unchanged, but sampled_at is older and the physical-IF set
        // differs. Equal counters are not progress, so this stays late instead
        // of being mistaken for a clock step back and rebased (SB-CLOCKFIX).
        let mut stale = snapshot_with_usb0(3, 0, 0);
        stale.sampled_at = tm_core::Timestamp::from_millis(current_at.unix_millis - 2_000);
        assert_eq!(stale.network_totals(), current_totals);
        assert!(stale.sampled_at < current_at);
        dashboard.ingest(stale);

        // The transient error, high-water mark, network baseline and IF set are
        // all preserved: none may be rolled back by the stale sample.
        assert!(
            dashboard.status().is_some_and(|status| status.is_error),
            "an idle late sample must not clear the newest transient error"
        );
        assert_eq!(dashboard.last_ingested_at, Some(current_at));
        assert_eq!(dashboard.last_network, Some(current_totals));
        assert_eq!(dashboard.last_network_interfaces, baseline_interfaces);

        // The next fresh sample still reports one tick of traffic from the
        // preserved baseline. Had the stale IF set been accepted, this frame
        // would look like a set change and report (0, 0) instead (WC-IFHOT).
        dashboard.ingest(demo_snapshot(4));
        assert_eq!(dashboard.network_rates(), (1_234_567, 321_456));
    }

    #[test]
    fn advancing_counters_still_rebase_a_regressed_timestamp() {
        let mut dashboard = dashboard();
        dashboard.ingest(demo_snapshot(1));
        dashboard.set_status("直近の取得失敗", true);

        // The clock steps back while the network stays active: both counters
        // advance, so this really is a new sample in a stepped-back clock era.
        // It must keep the 6c035fc contract and be rebased (WB-CLOCK).
        let mut regressed = demo_snapshot(2);
        regressed.sampled_at = tm_core::Timestamp::from_millis(1);
        let regressed_at = regressed.sampled_at;
        let regressed_totals = regressed.network_totals();
        assert!(regressed_at < dashboard.last_ingested_at.expect("high-water mark"));
        assert!(regressed_totals.0 > dashboard.last_network.expect("baseline").0);
        dashboard.ingest(regressed);

        assert!(
            dashboard.status().is_none(),
            "a genuinely newer sample must still clear the transient error"
        );
        assert_eq!(dashboard.last_ingested_at, Some(regressed_at));
        assert_eq!(dashboard.last_network, Some(regressed_totals));
        assert_eq!(dashboard.network_rates(), (1_234_567, 321_456));
    }

    #[test]
    fn rebased_clock_regression_still_updates_rates_and_history() {
        let mut dashboard = dashboard();
        dashboard.ingest(demo_snapshot(1)); // baseline tick 1: rate (0, 0)
        let history_before = dashboard.history().len();

        // The clock steps back, but the counters strictly advance: this is a
        // genuine new sample in the stepped-back era, not a late arrival.
        // WB-CLOCK rebases the mark/baseline, and the sample still feeds rates
        // and history just like any current sample.
        let mut regressed = demo_snapshot(2);
        regressed.sampled_at = tm_core::Timestamp::from_millis(1);
        dashboard.ingest(regressed);

        assert_eq!(dashboard.network_rates(), (1_234_567, 321_456));
        assert_eq!(dashboard.history().len(), history_before + 1);
        assert_eq!(
            dashboard.history().network_rx_rate.latest(),
            Some(&1_234_567)
        );
        assert_eq!(
            dashboard.last_ingested_at,
            Some(tm_core::Timestamp::from_millis(1))
        );
    }

    #[test]
    fn forward_progress_after_an_idle_streak_reports_normal_rates() {
        let mut dashboard = dashboard();
        let base_at = demo_snapshot(0).sampled_at;
        dashboard.ingest(demo_snapshot(0));
        assert_eq!(dashboard.network_rates(), (0, 0));

        // Idle ticks keep the counters equal while the wall clock moves
        // forward. They are current samples, never late arrivals, and must not
        // disturb the baseline.
        for tick in 1..=3 {
            let mut idle = demo_snapshot(0);
            idle.sampled_at = tm_core::Timestamp::from_millis(base_at.unix_millis + tick * 1_000);
            dashboard.ingest(idle);
            assert_eq!(dashboard.network_rates(), (0, 0));
        }

        // Traffic resumes: the rate is measured from the unchanged baseline.
        let mut moving = demo_snapshot(1);
        moving.sampled_at = tm_core::Timestamp::from_millis(base_at.unix_millis + 4_000);
        dashboard.ingest(moving);
        assert_eq!(dashboard.network_rates(), (1_234_567, 321_456));
    }

    #[test]
    fn default_sort_is_cpu_descending() {
        let mut dashboard = flat_dashboard();
        ingest_ticks(&mut dashboard, 1);
        let snapshot = dashboard.snapshot().expect("snapshot");
        let cpus: Vec<f32> = dashboard
            .visible_indices()
            .iter()
            .map(|&index| snapshot.processes[index].cpu_percent)
            .collect();
        assert!(cpus.windows(2).all(|pair| pair[0] >= pair[1]));
    }

    #[test]
    fn sorting_by_pid_is_ascending() {
        let mut dashboard = flat_dashboard();
        ingest_ticks(&mut dashboard, 1);
        dashboard.apply(Command::SetSortKey(SortKey::Pid));
        let snapshot = dashboard.snapshot().expect("snapshot");
        let pids: Vec<u32> = dashboard
            .visible_indices()
            .iter()
            .map(|&index| snapshot.processes[index].key.pid)
            .collect();
        assert!(pids.windows(2).all(|pair| pair[0] <= pair[1]));
    }

    #[test]
    fn sorting_by_virtual_memory_defaults_to_descending() {
        let mut dashboard = flat_dashboard();
        ingest_ticks(&mut dashboard, 1);
        dashboard.apply(Command::SetSortKey(SortKey::VirtualMemory));
        let spec = dashboard.sort_spec();
        assert_eq!(spec.key, SortKey::VirtualMemory);
        assert_eq!(spec.direction, SortDirection::Descending);
        // IBM Plex に無い ▲/▼ の tofu 回避で ↑/↓ に統一済み（R4-VISUAL）。
        assert_eq!(spec.direction.arrow(), "↓");
        assert_eq!(spec.key.label(), "仮想メモリ");
        let snapshot = dashboard.snapshot().expect("snapshot");
        let values: Vec<u64> = dashboard
            .visible_indices()
            .iter()
            .map(|&index| snapshot.processes[index].virtual_memory_bytes)
            .collect();
        assert!(values.windows(2).all(|pair| pair[0] >= pair[1]));
    }

    #[test]
    fn filter_reduces_visible_rows() {
        let mut dashboard = flat_dashboard();
        ingest_ticks(&mut dashboard, 1);
        dashboard.apply(Command::SetFilterText("firefox".to_string()));
        assert_eq!(dashboard.visible_count(), 1);
        assert_eq!(
            dashboard.visible_indices().len(),
            1,
            "filter should keep one row"
        );
        dashboard.apply(Command::ClearFilter);
        assert!(dashboard.visible_count() > 1);
    }

    #[test]
    fn virtual_memory_filter_limits_visible_rows_and_clears() {
        let mut dashboard = flat_dashboard();
        ingest_ticks(&mut dashboard, 1);
        let snapshot = dashboard.snapshot().expect("snapshot").clone();
        let max_vm = snapshot
            .processes
            .iter()
            .map(|process| process.virtual_memory_bytes)
            .max()
            .expect("processes");
        let min_vm = snapshot
            .processes
            .iter()
            .map(|process| process.virtual_memory_bytes)
            .min()
            .expect("processes");

        dashboard.apply(Command::SetMinVirtualMemoryBytes(Some(max_vm)));
        assert_eq!(dashboard.filter().min_virtual_memory_bytes, Some(max_vm));
        assert!(dashboard.filter().is_active());
        assert_eq!(
            dashboard.visible_count(),
            snapshot
                .processes
                .iter()
                .filter(|process| process.virtual_memory_bytes >= max_vm)
                .count()
        );

        dashboard.apply(Command::SetMinVirtualMemoryBytes(None));
        assert_eq!(dashboard.filter().min_virtual_memory_bytes, None);
        assert!(!dashboard.filter().is_active());
        // No threshold left: every process keeps its row, including idle ones.
        assert_eq!(dashboard.visible_count(), snapshot.processes.len());

        // Both the dedicated clear and ClearFilter must reset the threshold.
        dashboard.apply(Command::SetMinVirtualMemoryBytes(Some(min_vm)));
        dashboard.apply(Command::ClearFilter);
        assert_eq!(dashboard.filter().min_virtual_memory_bytes, None);
        assert_eq!(dashboard.visible_count(), snapshot.processes.len());
    }

    #[test]
    fn selection_survives_snapshots_and_moves_relative() {
        let mut dashboard = dashboard();
        ingest_ticks(&mut dashboard, 1);
        dashboard.apply(Command::SelectRelative(1));
        let first = dashboard.selected_key().expect("selected");
        ingest_ticks(&mut dashboard, 1);
        assert_eq!(dashboard.selected_key(), Some(first));
        dashboard.apply(Command::SelectRelative(1));
        assert_ne!(dashboard.selected_key(), Some(first));
    }

    #[test]
    fn clearing_selection_stays_cleared_across_recompute_and_ingest() {
        let mut dashboard = dashboard();
        dashboard.ingest(demo_snapshot(0));
        assert!(dashboard.selected_key().is_some());

        dashboard.apply(Command::Select(None));
        assert_eq!(dashboard.selected_key(), None);
        // Both re-sync paths must respect the explicit clear (R29 audit).
        dashboard.apply(Command::SetSortKey(SortKey::Pid));
        assert_eq!(dashboard.selected_key(), None);
        dashboard.ingest(demo_snapshot(1));
        assert_eq!(dashboard.selected_key(), None);
        dashboard.apply(Command::SetFilterText("abc".into()));
        assert_eq!(dashboard.selected_key(), None);
    }

    #[test]
    fn selection_falls_back_when_process_disappears() {
        let mut dashboard = dashboard();
        let mut snapshot = demo_snapshot(0);
        snapshot.processes.truncate(3);
        dashboard.ingest(snapshot);
        let selected = dashboard.selected_key().expect("selection");
        let mut next = demo_snapshot(1);
        next.processes.retain(|process| process.key != selected);
        dashboard.ingest(next);
        assert!(dashboard.selected_key().is_some());
        assert!(dashboard.selected_process().is_some());
    }

    #[test]
    fn selection_falls_back_to_the_same_row_position() {
        let mut dashboard = flat_dashboard();
        dashboard.ingest(demo_snapshot(0));
        dashboard.apply(Command::SelectRelative(2));
        let visible = dashboard.visible_indices().to_vec();
        assert!(visible.len() > 3, "need at least four rows for this test");

        let old_key = dashboard.selected_key().expect("selection");
        let position = dashboard
            .selected_visible_index()
            .expect("selected row position");
        assert_eq!(position, 2);
        // The process that will take the vacated slot at the same row index.
        let expected = dashboard.snapshot.as_ref().unwrap().processes[visible[3]].key;

        let mut next = demo_snapshot(1);
        next.processes.retain(|process| process.key != old_key);
        dashboard.ingest(next);

        // The vanished selection must fall back to the row that now occupies
        // the same position, not jump to the top of the list.
        assert_eq!(dashboard.selected_key(), Some(expected));
    }

    #[test]
    fn kill_flow_requires_confirmation_and_is_protected() {
        let mut dashboard = dashboard();
        ingest_ticks(&mut dashboard, 1);
        assert!(dashboard
            .apply(Command::RequestSignal {
                key: firefox_key(),
                signal: Signal::Terminate,
            })
            .is_empty());
        assert_eq!(
            dashboard.pending_kill().expect("pending").key,
            firefox_key()
        );
        assert_eq!(
            dashboard.apply(Command::ConfirmKill),
            vec![Effect::Signal {
                key: firefox_key(),
                signal: Signal::Terminate
            }]
        );
        assert!(dashboard.pending_kill().is_none());

        // PID 1 is protected.
        dashboard.apply(Command::RequestSignal {
            key: ProcessKey::new(1, 0),
            signal: Signal::Terminate,
        });
        assert!(dashboard.pending_kill().is_none());
        assert!(dashboard.status().expect("status").is_error);

        // The app itself is protected.
        let self_key = dashboard
            .snapshot()
            .expect("snapshot")
            .processes
            .iter()
            .find(|process| process.key.pid == 4_242)
            .expect("demo self process")
            .key;
        dashboard.apply(Command::RequestSignal {
            key: self_key,
            signal: Signal::Terminate,
        });
        assert!(dashboard.pending_kill().is_none());
    }

    #[test]
    fn pending_kill_is_not_overwritten_or_confirmed_while_paused() {
        let mut dashboard = dashboard();
        ingest_ticks(&mut dashboard, 1);
        let code_key = dashboard
            .snapshot
            .as_ref()
            .unwrap()
            .processes
            .iter()
            .find(|process| process.name == "code")
            .unwrap()
            .key;

        dashboard.apply(Command::RequestSignal {
            key: firefox_key(),
            signal: Signal::Terminate,
        });
        assert_eq!(
            dashboard.pending_kill().expect("first pending").key,
            firefox_key()
        );
        // A second request must not swap the target under the open dialog.
        dashboard.apply(Command::RequestSignal {
            key: code_key,
            signal: Signal::Kill,
        });
        assert_eq!(
            dashboard.pending_kill().expect("still first").key,
            firefox_key()
        );

        // Pausing then confirming must not emit a signal from stale data.
        dashboard.apply(Command::SetPaused(true));
        assert!(dashboard.apply(Command::ConfirmKill).is_empty());
        assert!(dashboard.pending_kill().is_none());
        assert!(dashboard.status().expect("status").is_error);
    }

    #[test]
    fn request_signal_is_rejected_while_paused() {
        let mut dashboard = dashboard();
        ingest_ticks(&mut dashboard, 1);
        dashboard.apply(Command::SetPaused(true));

        dashboard.apply(Command::RequestSignal {
            key: firefox_key(),
            signal: Signal::Terminate,
        });
        assert!(dashboard.pending_kill().is_none());
        assert!(dashboard
            .status()
            .expect("status")
            .text
            .contains("一時停止"));

        // After resuming, the dialog opens normally again.
        dashboard.apply(Command::SetPaused(false));
        dashboard.apply(Command::RequestSignal {
            key: firefox_key(),
            signal: Signal::Terminate,
        });
        assert!(dashboard.pending_kill().is_some());
    }

    #[test]
    fn read_only_backend_rejects_kill() {
        let mut dashboard = Dashboard::new(PlatformInfo::demo(), DEFAULT_REFRESH_INTERVAL);
        ingest_ticks(&mut dashboard, 1);
        dashboard.apply(Command::RequestSignal {
            key: firefox_key(),
            signal: Signal::Terminate,
        });
        assert!(dashboard.pending_kill().is_none());
        assert!(dashboard.status().expect("status").is_error);
    }

    #[test]
    fn windows_platform_rejects_unsupported_signals_without_pending_kill() {
        let mut dashboard = Dashboard::new(
            PlatformInfo::native("Windows 11 Pro"),
            DEFAULT_REFRESH_INTERVAL,
        );
        ingest_ticks(&mut dashboard, 1);

        // Process control is enabled and a valid target exists, so a rejection
        // below can only come from the per-signal capability gate (`!supports_signal`).
        dashboard.apply(Command::RequestSignal {
            key: firefox_key(),
            signal: Signal::Terminate,
        });
        assert!(
            dashboard.pending_kill().is_some(),
            "Windows must still support Terminate"
        );
        dashboard.apply(Command::CancelKill);

        for signal in [Signal::Stop, Signal::Continue] {
            dashboard.apply(Command::RequestSignal {
                key: firefox_key(),
                signal,
            });
            assert!(
                dashboard.pending_kill().is_none(),
                "{signal:?} must not open a confirmation dialog"
            );
            let status = dashboard.status().expect("status");
            assert!(status.is_error, "{signal:?} must be reported as an error");
            assert_eq!(
                status.text,
                format!("{STATUS_PREFIX_ERROR} この環境ではこのシグナルは使用できません"),
                "{signal:?} must report the per-signal capability message"
            );
        }
    }

    #[test]
    fn confirm_after_disappearance_emits_nothing() {
        let mut dashboard = dashboard();
        ingest_ticks(&mut dashboard, 1);
        dashboard.apply(Command::RequestSignal {
            key: firefox_key(),
            signal: Signal::Terminate,
        });
        let mut next = demo_snapshot(1);
        next.processes
            .retain(|process| process.key != firefox_key());
        dashboard.ingest(next);
        assert!(dashboard.apply(Command::ConfirmKill).is_empty());
        assert!(dashboard.status().expect("status").is_error);
    }

    #[test]
    fn demo_probe_drives_dashboard() {
        let mut probe = DemoProbe::new();
        let mut dashboard = Dashboard::new(probe.platform_info(), DEFAULT_REFRESH_INTERVAL);
        for _ in 0..3 {
            let snapshot = probe.sample().expect("demo sample");
            dashboard.ingest(snapshot);
        }
        assert_eq!(dashboard.history().len(), 3);
        assert!(dashboard.cpu_label().ends_with('%'));
    }

    #[test]
    fn settings_restore_commands_emit_no_effects() {
        // The native shell checks this contract when restoring settings; keep
        // it pinned here so a future effect cannot be silently dropped (R2-C01).
        let mut dashboard = dashboard();
        for command in [
            Command::SetInterval(Duration::from_millis(1_500)),
            Command::SetSortSpec(SortSpec {
                key: SortKey::Name,
                direction: SortDirection::Ascending,
            }),
            Command::SetDetailsVisible(false),
            Command::SetTreeView(true),
        ] {
            assert!(
                command.is_effect_free(),
                "{command:?} is a settings-restore command and must be effect-free"
            );
            assert!(
                dashboard.apply(command).is_empty(),
                "settings restore must not emit effects (R2-C01)"
            );
        }
    }

    #[test]
    fn effect_free_commands_return_no_effects_in_every_state() {
        // Effect-free commands only mutate presentation state. Their empty
        // return is the "empty" precondition every display-only caller relies
        // on; pin it on a fresh dashboard, with a snapshot, and while paused.
        let commands = [
            Command::Select(None),
            Command::SelectRelative(1),
            Command::SetSortKey(SortKey::Pid),
            Command::SetSortSpec(SortSpec::default()),
            Command::ApplyFilterPreset(FilterPresetId::All),
            Command::SetFilterText("firefox".to_string()),
            Command::ClearFilter,
            Command::SetMinVirtualMemoryBytes(Some(1)),
            Command::SetInterval(DEFAULT_REFRESH_INTERVAL),
            Command::ToggleDetails,
            Command::SetDetailsVisible(true),
            Command::ToggleTreeView,
            Command::SetTreeView(false),
            Command::ToggleTreeNode(firefox_key()),
            Command::ToggleAppGrouping,
            Command::SetAppGrouping(true),
            Command::RequestSignal {
                key: firefox_key(),
                signal: Signal::Terminate,
            },
            Command::CancelKill,
        ];

        for command in commands {
            assert!(
                command.is_effect_free(),
                "{command:?} must be classified effect-free"
            );

            let mut fresh = dashboard();
            assert!(
                fresh.apply(command.clone()).is_empty(),
                "{command:?} on an empty dashboard"
            );

            let mut loaded = dashboard();
            ingest_ticks(&mut loaded, 1);
            assert!(
                loaded.apply(command.clone()).is_empty(),
                "{command:?} with a snapshot"
            );

            loaded.apply(Command::SetPaused(true));
            assert!(
                loaded.apply(command.clone()).is_empty(),
                "{command:?} while paused"
            );
        }
    }

    #[test]
    fn stateful_commands_are_not_effect_free_and_do_emit() {
        // The three commands whose effects depend on state must never be
        // classified as effect-free, otherwise a caller could drop their result.
        for command in [
            Command::Tick,
            Command::SetPaused(false),
            Command::ConfirmKill,
        ] {
            assert!(!command.is_effect_free(), "{command:?} may emit an effect");
        }

        let mut dashboard = dashboard();
        assert_eq!(dashboard.apply(Command::Tick), vec![Effect::SampleNow]);
        dashboard.apply(Command::SetPaused(true));
        assert_eq!(
            dashboard.apply(Command::SetPaused(false)),
            vec![Effect::SampleNow]
        );

        ingest_ticks(&mut dashboard, 1);
        dashboard.apply(Command::RequestSignal {
            key: firefox_key(),
            signal: Signal::Kill,
        });
        assert_eq!(
            dashboard.apply(Command::ConfirmKill),
            vec![Effect::Signal {
                key: firefox_key(),
                signal: Signal::Kill
            }]
        );
    }

    #[test]
    fn sort_spec_and_details_can_be_restored_exactly() {
        let mut dashboard = dashboard();
        ingest_ticks(&mut dashboard, 1);
        let spec = SortSpec {
            key: SortKey::Name,
            direction: SortDirection::Ascending,
        };
        dashboard.apply(Command::SetSortSpec(spec));
        assert_eq!(dashboard.sort_spec(), spec);
        dashboard.apply(Command::SetDetailsVisible(false));
        assert!(!dashboard.details_visible());
        dashboard.apply(Command::SetDetailsVisible(true));
        assert!(dashboard.details_visible());
    }

    #[test]
    fn pending_kill_captures_the_process_identity() {
        let mut dashboard = dashboard();
        ingest_ticks(&mut dashboard, 1);
        dashboard.apply(Command::RequestSignal {
            key: firefox_key(),
            signal: Signal::Terminate,
        });
        let pending = dashboard.pending_kill().expect("pending signal");
        assert_eq!(pending.name, "firefox");
        assert!(pending.command.contains("firefox"));
        assert_eq!(pending.user.as_deref(), Some("user"));
    }

    #[test]
    fn signal_confirmation_notes_are_distinct_and_state_reversibility() {
        let terminate = signal_confirmation_note(Signal::Terminate);
        let kill = signal_confirmation_note(Signal::Kill);
        let stop = signal_confirmation_note(Signal::Stop);
        let resume = signal_confirmation_note(Signal::Continue);

        assert!(terminate.contains("SIGTERM"));
        assert!(kill.contains("SIGKILL"));
        assert!(stop.contains("SIGSTOP"));
        assert!(resume.contains("SIGCONT"));
        // Only the destructive signal must claim the action cannot be undone.
        assert!(kill.contains("取り消せません"));
        for note in [terminate, stop, resume] {
            assert!(!note.contains("取り消せません"), "{note}");
        }
        assert!(stop.contains("元に戻せます"));

        let notes = [terminate, kill, stop, resume];
        for (index, note) in notes.iter().enumerate() {
            assert!(!note.is_empty());
            assert!(notes[index + 1..].iter().all(|other| other != note));
        }
    }

    #[test]
    fn pending_kill_carries_the_signal_specific_note() {
        let mut dashboard = dashboard();
        ingest_ticks(&mut dashboard, 1);
        dashboard.apply(Command::RequestSignal {
            key: firefox_key(),
            signal: Signal::Stop,
        });

        let pending = dashboard.pending_kill().expect("pending signal");
        assert_eq!(pending.signal, Signal::Stop);
        assert_eq!(
            pending.confirmation_note,
            signal_confirmation_note(Signal::Stop)
        );
        assert!(pending.confirmation_note.contains("元に戻せます"));

        // The same note is visible in the status bar as a transient message,
        // and the usual ingest contract still clears it on the next sample.
        let status = dashboard.status().expect("status");
        assert!(!status.is_error);
        assert!(
            status.text.ends_with(pending.confirmation_note),
            "{}",
            status.text
        );
        dashboard.ingest(demo_snapshot(1));
        assert!(dashboard.status().is_none());
    }

    #[test]
    fn confirmation_preserves_the_requested_signal() {
        let mut dashboard = dashboard();
        ingest_ticks(&mut dashboard, 1);
        dashboard.apply(Command::RequestSignal {
            key: firefox_key(),
            signal: Signal::Kill,
        });
        assert_eq!(
            dashboard.apply(Command::ConfirmKill),
            vec![Effect::Signal {
                key: firefox_key(),
                signal: Signal::Kill
            }]
        );
    }

    #[test]
    fn custom_snapshot_keeps_selection_valid() {
        let mut dashboard = dashboard();
        let mut snapshot = demo_snapshot(0);
        snapshot.processes = vec![ProcessSnapshot {
            key: ProcessKey::new(50, 10),
            name: "only".to_string(),
            cpu_percent: 1.0,
            ..ProcessSnapshot::default()
        }];
        dashboard.ingest(snapshot);
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(50, 10)));
        assert_eq!(dashboard.visible_count(), 1);
    }

    #[test]
    fn tree_view_flattens_process_hierarchy() {
        let mut dashboard = dashboard();
        let mut snapshot = demo_snapshot(0);
        let parent = ProcessKey::new(100, 10);
        let child = ProcessKey::new(101, 11);
        let grandchild = ProcessKey::new(102, 12);
        snapshot.processes = vec![
            ProcessSnapshot {
                key: parent,
                parent_pid: None,
                name: "parent".to_string(),
                command: "parent".to_string(),
                cpu_percent: 1.0,
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: child,
                parent_pid: Some(parent.pid),
                name: "child".to_string(),
                command: "child".to_string(),
                cpu_percent: 2.0,
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: grandchild,
                parent_pid: Some(child.pid),
                name: "grandchild".to_string(),
                command: "grandchild".to_string(),
                cpu_percent: 3.0,
                ..ProcessSnapshot::default()
            },
        ];
        dashboard.ingest(snapshot);
        dashboard.apply(Command::SetSortSpec(SortSpec {
            key: SortKey::Pid,
            direction: SortDirection::Ascending,
        }));
        dashboard.apply(Command::SetTreeView(true));
        assert!(dashboard.tree_view());
        assert_eq!(dashboard.visible_indices(), &[0, 1, 2]);
        assert_eq!(dashboard.visible_depths(), &[0, 1, 2]);
        let rows: Vec<_> = dashboard.visible_rows().collect();
        assert_eq!(rows[0].index, 0);
        assert!(rows[0].has_children);
        assert_eq!(rows[1].index, 1);
        assert!(rows[1].has_children);
        assert_eq!(rows[2].index, 2);
        assert!(!rows[2].has_children);

        dashboard.apply(Command::ToggleTreeView);
        assert!(!dashboard.tree_view());
        assert_eq!(dashboard.visible_depths(), &[0, 0, 0]);
        assert!(dashboard.visible_rows().all(|row| !row.has_children));
    }

    #[test]
    fn set_tree_view_recomputes_visible_rows() {
        let mut dashboard = dashboard();
        let mut snapshot = demo_snapshot(0);
        snapshot.processes = vec![
            ProcessSnapshot {
                key: ProcessKey::new(10, 1),
                parent_pid: None,
                name: "root".to_string(),
                command: "root".to_string(),
                cpu_percent: 1.0,
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: ProcessKey::new(11, 2),
                parent_pid: Some(10),
                name: "leaf".to_string(),
                command: "leaf".to_string(),
                cpu_percent: 1.0,
                ..ProcessSnapshot::default()
            },
        ];
        dashboard.ingest(snapshot);
        dashboard.apply(Command::SetSortSpec(SortSpec {
            key: SortKey::Pid,
            direction: SortDirection::Ascending,
        }));
        assert_eq!(dashboard.visible_depths(), &[0, 0]);
        dashboard.apply(Command::SetTreeView(true));
        assert_eq!(dashboard.visible_depths(), &[0, 1]);
    }

    #[test]
    fn toggle_tree_node_collapses_children() {
        let mut dashboard = dashboard();
        let mut snapshot = demo_snapshot(0);
        let parent = ProcessKey::new(100, 10);
        let child = ProcessKey::new(101, 11);
        snapshot.processes = vec![
            ProcessSnapshot {
                key: parent,
                parent_pid: None,
                name: "parent".to_string(),
                command: "parent".to_string(),
                cpu_percent: 1.0,
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: child,
                parent_pid: Some(parent.pid),
                name: "child".to_string(),
                command: "child".to_string(),
                cpu_percent: 1.0,
                ..ProcessSnapshot::default()
            },
        ];
        dashboard.ingest(snapshot);
        dashboard.apply(Command::SetSortSpec(SortSpec {
            key: SortKey::Pid,
            direction: SortDirection::Ascending,
        }));
        dashboard.apply(Command::SetTreeView(true));
        assert_eq!(dashboard.visible_indices(), &[0, 1]);
        assert!(dashboard.visible_rows().next().unwrap().has_children);

        // Selecting the child and collapsing its parent must move the
        // selection to the parent, not to an unrelated first row (R42 audit).
        dashboard.apply(Command::Select(Some(child)));
        assert_eq!(dashboard.selected_key(), Some(child));
        dashboard.apply(Command::ToggleTreeNode(parent));
        assert!(dashboard.is_collapsed(parent));
        assert_eq!(dashboard.visible_indices(), &[0]);
        assert_eq!(dashboard.selected_key(), Some(parent));
        let row = dashboard.visible_rows().next().unwrap();
        assert_eq!(row.index, 0);
        assert!(row.has_children);

        dashboard.apply(Command::ToggleTreeNode(parent));
        assert!(!dashboard.is_collapsed(parent));
        assert_eq!(dashboard.visible_indices(), &[0, 1]);
    }

    // --- R1-GROUP: application-grouped rows ----------------------------------

    fn app_group_snapshot() -> tm_core::SystemSnapshot {
        let mut snapshot = demo_snapshot(0);
        snapshot.processes = vec![
            ProcessSnapshot {
                key: ProcessKey::new(30, 3),
                name: "firefox".into(),
                command: "/usr/bin/firefox --pid=30".into(),
                cpu_percent: 0.25,
                memory_bytes: 300,
                virtual_memory_bytes: 3_000,
                swap_bytes: 3_000,
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: ProcessKey::new(10, 1),
                name: "firefox".into(),
                command: "/usr/bin/firefox --pid=10".into(),
                cpu_percent: 1.5,
                memory_bytes: 100,
                virtual_memory_bytes: 1_000,
                swap_bytes: 1_000,
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: ProcessKey::new(20, 2),
                name: "firefox".into(),
                command: "/usr/bin/firefox --pid=20".into(),
                cpu_percent: 2.25,
                memory_bytes: 200,
                virtual_memory_bytes: 2_000,
                swap_bytes: 2_000,
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: ProcessKey::new(40, 4),
                name: String::new(),
                command: "kernel-task".into(),
                cpu_percent: 9.0,
                memory_bytes: 999,
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: ProcessKey::new(50, 5),
                name: "solo".into(),
                command: "solo".into(),
                cpu_percent: 3.0,
                memory_bytes: 42,
                virtual_memory_bytes: 42_000,
                swap_bytes: 42,
                ..ProcessSnapshot::default()
            },
        ];
        snapshot
    }

    #[test]
    fn app_group_rows_are_empty_without_a_snapshot() {
        let dashboard = dashboard();
        assert!(dashboard.app_group_rows().is_empty());
        assert_eq!(dashboard.selected_app_group_row_index(), None);
    }

    #[test]
    fn app_group_rows_sum_totals_and_skip_empty_names() {
        let mut dashboard = dashboard();
        dashboard.ingest(app_group_snapshot());
        let rows = dashboard.app_group_rows();
        assert_eq!(rows.len(), 2, "empty process names must not create a group");
        let firefox = &rows[0];
        assert_eq!(firefox.kind, AppRowKind::Group);
        assert_eq!(firefox.depth, 0);
        assert_eq!(firefox.key, "firefox");
        assert_eq!(firefox.label, "firefox");
        assert_eq!(firefox.process_count, 3);
        assert_eq!(firefox.cpu_percent, 4.0);
        assert_eq!(firefox.memory_bytes, 600);
        assert_eq!(firefox.virtual_memory_bytes, 6_000);
        assert_eq!(
            firefox.swap_bytes, 6_000,
            "member swap bytes saturating sum"
        );
        assert!(firefox.has_children);
        assert!(!firefox.expanded);
        assert_eq!(firefox.process_key, ProcessKey::new(10, 1));
        assert_eq!(
            firefox.member_keys,
            vec![
                ProcessKey::new(10, 1),
                ProcessKey::new(20, 2),
                ProcessKey::new(30, 3)
            ]
        );
        let solo = &rows[1];
        assert_eq!(solo.key, "solo");
        assert_eq!(solo.cpu_percent, 3.0);
        assert_eq!(solo.memory_bytes, 42);
        assert_eq!(solo.virtual_memory_bytes, 42_000);
        assert_eq!(solo.swap_bytes, 42);
        assert!(!solo.has_children, "a single member is not expandable");
        assert!(rows.iter().all(|row| !row.key.is_empty()));
    }

    #[test]
    fn app_group_rows_sort_by_virtual_memory_totals_and_keep_member_values() {
        let mut dashboard = dashboard();
        dashboard.ingest(app_group_snapshot());

        // Descending: the single-process `solo` (42_000) outranks the three-member
        // firefox total (6_000), proving the comparator uses the aggregate and
        // not the key order or the process count.
        dashboard.apply(Command::SetSortSpec(SortSpec {
            key: SortKey::VirtualMemory,
            direction: SortDirection::Descending,
        }));
        let rows = dashboard.app_group_rows();
        assert_eq!(rows[0].key, "solo");
        assert_eq!(rows[1].key, "firefox");
        assert_eq!(rows[0].virtual_memory_bytes, 42_000);
        assert_eq!(rows[1].virtual_memory_bytes, 6_000);

        // Ascending flips the order using the same totals.
        dashboard.apply(Command::SetSortSpec(SortSpec {
            key: SortKey::VirtualMemory,
            direction: SortDirection::Ascending,
        }));
        let rows = dashboard.app_group_rows();
        assert_eq!(rows[0].key, "firefox");
        assert_eq!(rows[1].key, "solo");

        // Expanded members carry their own reading, never the group sum.
        dashboard.apply(Command::ToggleAppGroup("firefox".into()));
        let expanded = dashboard.app_group_rows();
        let mut totals = 0_u64;
        for member in expanded
            .iter()
            .filter(|row| row.kind == AppRowKind::Process)
        {
            assert_ne!(member.virtual_memory_bytes, 6_000);
            totals = totals.saturating_add(member.virtual_memory_bytes);
        }
        assert_eq!(totals, 6_000);
    }

    #[test]
    fn app_group_rows_sort_by_swap_totals_independently_of_virtual_memory() {
        let mut dashboard = dashboard();
        dashboard.ingest(app_group_snapshot());

        // Descending swap puts the three-member firefox total (6_000) ahead of
        // solo (42) — the opposite of the descending virtual-memory order, so
        // the comparator is proven to read the swap aggregate (C2-NATIVESWAP).
        dashboard.apply(Command::SetSortSpec(SortSpec {
            key: SortKey::Swap,
            direction: SortDirection::Descending,
        }));
        let rows = dashboard.app_group_rows();
        assert_eq!(rows[0].key, "firefox");
        assert_eq!(rows[1].key, "solo");
        assert_eq!(rows[0].swap_bytes, 6_000);
        assert_eq!(rows[1].swap_bytes, 42);

        // Ascending flips the same swap totals.
        dashboard.apply(Command::SetSortSpec(SortSpec {
            key: SortKey::Swap,
            direction: SortDirection::Ascending,
        }));
        let rows = dashboard.app_group_rows();
        assert_eq!(rows[0].key, "solo");
        assert_eq!(rows[1].key, "firefox");

        // Expanded members carry their own swap reading, never the group sum.
        dashboard.apply(Command::ToggleAppGroup("firefox".into()));
        let expanded = dashboard.app_group_rows();
        let mut totals = 0_u64;
        for member in expanded
            .iter()
            .filter(|row| row.kind == AppRowKind::Process)
        {
            assert_ne!(member.swap_bytes, 6_000);
            totals = totals.saturating_add(member.swap_bytes);
        }
        assert_eq!(totals, 6_000);
    }

    #[test]
    fn toggle_app_group_expands_depth_one_processes_then_collapses() {
        let mut dashboard = dashboard();
        dashboard.ingest(app_group_snapshot());
        assert!(dashboard
            .app_group_rows()
            .iter()
            .all(|row| row.kind == AppRowKind::Group));
        assert!(!dashboard.is_app_group_expanded("firefox"));

        dashboard.apply(Command::ToggleAppGroup("firefox".into()));
        assert!(dashboard.is_app_group_expanded("firefox"));
        let rows = dashboard.app_group_rows();
        assert_eq!(rows.len(), 5); // 2 groups + 3 expanded firefox members
        assert!(rows[0].expanded);
        for row in &rows[1..4] {
            assert_eq!(row.kind, AppRowKind::Process);
            assert_eq!(row.depth, 1);
            assert_eq!(row.key, "firefox");
            assert!(!row.has_children);
            assert!(!row.expanded);
            assert_eq!(row.process_count, 1);
        }
        assert_eq!(rows[4].kind, AppRowKind::Group);
        assert_eq!(rows[4].key, "solo");

        dashboard.apply(Command::ToggleAppGroup("firefox".into()));
        assert!(!dashboard.is_app_group_expanded("firefox"));
        assert_eq!(dashboard.app_group_rows().len(), 2);
    }

    #[test]
    fn select_app_group_selects_smallest_pid_and_keeps_signal_identity() {
        let mut dashboard = dashboard();
        dashboard.ingest(app_group_snapshot());
        dashboard.apply(Command::SelectAppGroup("firefox".into()));
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(10, 1)));
        assert_eq!(
            dashboard
                .selected_process()
                .map(|process| process.name.as_str()),
            Some("firefox")
        );
        assert_eq!(dashboard.selected_app_group_row_index(), Some(0));

        dashboard.apply(Command::RequestSignal {
            key: dashboard.selected_key().expect("selected member"),
            signal: Signal::Terminate,
        });
        let pending = dashboard.pending_kill().expect("pending signal");
        assert_eq!(pending.key, ProcessKey::new(10, 1));
        assert_eq!(pending.name, "firefox");
    }

    #[test]
    fn collapsing_app_group_moves_a_hidden_member_selection_to_first_member() {
        let mut dashboard = dashboard();
        dashboard.ingest(app_group_snapshot());
        dashboard.apply(Command::ToggleAppGroup("firefox".into()));
        dashboard.apply(Command::Select(Some(ProcessKey::new(20, 2))));
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(20, 2)));
        assert_eq!(dashboard.selected_app_group_row_index(), Some(2));

        dashboard.apply(Command::ToggleAppGroup("firefox".into()));
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(10, 1)));
        assert_eq!(dashboard.selected_app_group_row_index(), Some(0));
    }

    #[test]
    fn app_group_rows_follow_filter_and_sort() {
        let mut dashboard = dashboard();
        dashboard.ingest(app_group_snapshot());
        dashboard.apply(Command::SetFilterText("solo".into()));
        let rows = dashboard.app_group_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "solo");
        assert_eq!(rows[0].memory_bytes, 42);

        dashboard.apply(Command::ClearFilter);
        dashboard.apply(Command::SetSortSpec(SortSpec {
            key: SortKey::Name,
            direction: SortDirection::Ascending,
        }));
        let keys: Vec<_> = dashboard
            .app_group_rows()
            .iter()
            .map(|row| row.key.clone())
            .collect();
        assert_eq!(keys, ["firefox", "solo"]);
        dashboard.apply(Command::SetSortSpec(SortSpec {
            key: SortKey::Cpu,
            direction: SortDirection::Ascending,
        }));
        let keys: Vec<_> = dashboard
            .app_group_rows()
            .iter()
            .map(|row| row.key.clone())
            .collect();
        assert_eq!(keys, ["solo", "firefox"]);
    }

    #[test]
    fn app_group_commands_are_effect_free_in_every_state() {
        for command in [
            Command::SelectAppGroup("firefox".into()),
            Command::ToggleAppGroup("firefox".into()),
            Command::SelectAppGroup(OTHERS_APP_GROUP_KEY.into()),
            Command::ToggleAppGroup(OTHERS_APP_GROUP_KEY.into()),
        ] {
            assert!(command.is_effect_free(), "{command:?}");
            let mut fresh = dashboard();
            assert!(fresh.apply(command.clone()).is_empty());
            let mut loaded = dashboard();
            loaded.ingest(app_group_snapshot());
            assert!(loaded.apply(command.clone()).is_empty());
            loaded.apply(Command::SetPaused(true));
            assert!(loaded.apply(command).is_empty());
        }
    }

    #[test]
    fn app_group_expansion_is_pruned_when_the_group_disappears() {
        let mut dashboard = dashboard();
        dashboard.ingest(app_group_snapshot());
        dashboard.apply(Command::ToggleAppGroup("firefox".into()));
        assert!(dashboard.is_app_group_expanded("firefox"));

        let mut next = app_group_snapshot();
        next.processes.retain(|process| process.name != "firefox");
        dashboard.ingest(next);
        assert!(!dashboard.is_app_group_expanded("firefox"));
        assert!(dashboard
            .app_group_rows()
            .iter()
            .all(|row| row.key != "firefox"));
    }

    #[test]
    fn app_group_rows_follow_executable_keys_not_comm() {
        // Firefox parent + two children with generic comm values. Only argv[0]
        // identifies the application, so the group label must be the app name.
        let mut snapshot = app_group_snapshot();
        snapshot.processes = vec![
            ProcessSnapshot {
                key: ProcessKey::new(60, 6),
                name: "firefox-bin".into(),
                command: "/usr/lib/firefox/firefox-bin".into(),
                cpu_percent: 1.0,
                memory_bytes: 100,
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: ProcessKey::new(70, 7),
                name: "Isolated Web Co".into(),
                command: "/usr/lib/firefox/firefox --type=renderer --token=***".into(),
                cpu_percent: 2.0,
                memory_bytes: 200,
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: ProcessKey::new(80, 8),
                name: "solo".into(),
                command: "solo".into(),
                cpu_percent: 1.0,
                ..ProcessSnapshot::default()
            },
        ];
        let mut dashboard = dashboard();
        dashboard.ingest(snapshot);
        let rows = dashboard.app_group_rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].key, "firefox");
        assert_eq!(rows[0].label, "firefox");
        assert_eq!(rows[0].process_count, 2);

        // Expansion state and selection resolve by app key, not by comm.
        dashboard.apply(Command::ToggleAppGroup("firefox".into()));
        assert!(dashboard.is_app_group_expanded("firefox"));
        let rows = dashboard.app_group_rows();
        // One expanded group row + its two members + the unrelated `solo` group.
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1].label, "firefox-bin");
        assert_eq!(rows[2].label, "Isolated Web Co");
        assert_eq!(rows[2].key, "firefox");

        dashboard.apply(Command::ToggleAppGroup("firefox".into()));
        dashboard.apply(Command::Select(Some(ProcessKey::new(70, 7))));
        assert_eq!(
            dashboard.selected_key(),
            Some(ProcessKey::new(60, 6)),
            "a hidden child must map to the group's first member"
        );
        dashboard.apply(Command::SelectAppGroup("firefox".into()));
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(60, 6)));
    }

    // --- R1-VISIBLE: grouped rows through the existing visible arrays --------

    #[test]
    fn grouping_is_default_and_visible_arrays_mirror_group_rows() {
        let mut dashboard = dashboard();
        assert!(dashboard.app_group_view(), "grouping must be on by default");
        assert!(!dashboard.tree_view());
        dashboard.ingest(app_group_snapshot());

        let rows = dashboard.app_group_rows();
        assert_eq!(dashboard.visible_count(), rows.len());
        assert_eq!(dashboard.visible_count(), 2);
        assert_eq!(dashboard.visible_indices(), &[1, 4]);
        assert_eq!(dashboard.visible_depths(), &[0, 0]);
        assert_eq!(dashboard.visible_has_children(), &[true, false]);
        assert_eq!(
            dashboard.selected_key(),
            Some(ProcessKey::new(10, 1)),
            "a group row selects its first member"
        );
    }

    #[test]
    fn toggling_app_grouping_switches_between_grouped_and_flat_rows() {
        let mut dashboard = dashboard();
        dashboard.ingest(app_group_snapshot());
        assert_eq!(dashboard.visible_count(), 2);

        dashboard.apply(Command::ToggleAppGrouping);
        assert!(!dashboard.app_group_view());
        assert_eq!(
            dashboard.visible_count(),
            5,
            "flat mode keeps every process"
        );
        assert_eq!(dashboard.visible_depths(), &[0; 5]);
        assert!(dashboard.visible_rows().all(|row| !row.has_children));

        dashboard.apply(Command::SetAppGrouping(true));
        assert!(dashboard.app_group_view());
        assert_eq!(dashboard.visible_count(), 2);
        assert_eq!(dashboard.visible_has_children(), &[true, false]);
    }

    #[test]
    fn grouping_and_tree_view_are_mutually_exclusive() {
        let mut dashboard = dashboard();
        dashboard.apply(Command::SetTreeView(true));
        assert!(dashboard.tree_view());
        assert!(!dashboard.app_group_view());

        dashboard.apply(Command::ToggleAppGrouping);
        assert!(dashboard.app_group_view());
        assert!(!dashboard.tree_view());

        dashboard.apply(Command::ToggleTreeView);
        assert!(dashboard.tree_view());
        assert!(!dashboard.app_group_view());
    }

    #[test]
    fn expanding_a_group_rebuilds_visible_rows_and_navigation_targets_members() {
        let mut dashboard = dashboard();
        dashboard.ingest(app_group_snapshot());
        dashboard.apply(Command::ToggleAppGroup("firefox".into()));

        assert_eq!(dashboard.visible_count(), 5);
        assert_eq!(dashboard.visible_indices(), &[1, 1, 2, 0, 4]);
        assert_eq!(dashboard.visible_depths(), &[0, 1, 1, 1, 0]);
        assert_eq!(
            dashboard.visible_has_children(),
            &[true, false, false, false, false]
        );

        // Down / PageDown / End / Home all reduce to SelectRelative.
        dashboard.apply(Command::SelectRelative(2));
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(20, 2)));
        dashboard.apply(Command::SelectRelative(1_000_000));
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(50, 5)));
        dashboard.apply(Command::SelectRelative(-1_000_000));
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(10, 1)));
        dashboard.apply(Command::SelectRelative(-10));
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(10, 1)));

        // Collapsing a group with a member selected returns to the first member.
        dashboard.apply(Command::SelectRelative(2));
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(20, 2)));
        dashboard.apply(Command::ToggleAppGroup("firefox".into()));
        assert_eq!(dashboard.visible_count(), 2);
        assert_eq!(dashboard.visible_depths(), &[0, 0]);
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(10, 1)));
    }

    #[test]
    fn toggle_app_group_ignores_rows_without_children() {
        let mut dashboard = dashboard();
        dashboard.ingest(app_group_snapshot());
        // "solo" is a single-member group: has_children is false.
        dashboard.apply(Command::ToggleAppGroup("solo".into()));
        assert!(!dashboard.is_app_group_expanded("solo"));
        assert_eq!(dashboard.visible_count(), 2);
        // Unknown keys must not grow the expansion set either.
        dashboard.apply(Command::ToggleAppGroup("missing".into()));
        assert_eq!(dashboard.visible_count(), 2);
    }

    #[test]
    fn grouped_selection_maps_hidden_members_to_the_group_row() {
        let mut dashboard = dashboard();
        dashboard.ingest(app_group_snapshot());
        dashboard.apply(Command::Select(Some(ProcessKey::new(20, 2))));
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(10, 1)));
        assert_eq!(dashboard.selected_visible_index(), Some(0));
        assert_eq!(dashboard.selected_app_group_row_index(), Some(0));

        dashboard.apply(Command::ToggleAppGroup("firefox".into()));
        dashboard.apply(Command::Select(Some(ProcessKey::new(20, 2))));
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(20, 2)));
        assert_eq!(dashboard.selected_visible_index(), Some(2));
    }

    #[test]
    fn grouped_visible_rows_follow_filter_and_sort() {
        let mut dashboard = dashboard();
        dashboard.ingest(app_group_snapshot());
        dashboard.apply(Command::SetFilterText("solo".into()));
        assert_eq!(dashboard.visible_indices(), &[4]);
        assert_eq!(dashboard.visible_depths(), &[0]);
        assert_eq!(dashboard.visible_has_children(), &[false]);

        dashboard.apply(Command::ClearFilter);
        dashboard.apply(Command::SetSortSpec(SortSpec {
            key: SortKey::Name,
            direction: SortDirection::Descending,
        }));
        assert_eq!(dashboard.visible_indices(), &[4, 1]);
        assert_eq!(
            dashboard
                .app_group_rows()
                .iter()
                .map(|row| row.key.as_str())
                .collect::<Vec<_>>(),
            ["solo", "firefox"]
        );
    }

    #[test]
    fn grouping_does_not_change_snapshot_or_export_rows() {
        use tm_core::export::snapshot_to_csv;
        use tm_core::redact::RedactionIdentifiers;

        let mut dashboard = dashboard();
        dashboard.ingest(app_group_snapshot());
        let snapshot = dashboard.snapshot().expect("snapshot").clone();
        let ids = RedactionIdentifiers {
            home: "",
            user: "",
            hostname: "",
        };
        let grouped_csv = snapshot_to_csv(&snapshot, &ids);
        assert_eq!(grouped_csv.lines().count(), snapshot.processes.len() + 1);
        assert_eq!(dashboard.visible_count(), 2);

        dashboard.apply(Command::SetAppGrouping(false));
        assert_eq!(dashboard.snapshot().expect("snapshot").processes.len(), 5);
        let flat_csv = snapshot_to_csv(dashboard.snapshot().expect("snapshot"), &ids);
        assert_eq!(
            grouped_csv, flat_csv,
            "exports read snapshot.processes only"
        );
    }

    // --- C1-COLLAPSE: one row for single-process applications ----------------

    fn others_snapshot() -> tm_core::SystemSnapshot {
        let mut snapshot = demo_snapshot(0);
        snapshot.processes = vec![
            ProcessSnapshot {
                key: ProcessKey::new(11, 1),
                name: "alpha".into(),
                command: "/usr/bin/alpha".into(),
                cpu_percent: 1.0,
                memory_bytes: 100,
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: ProcessKey::new(22, 2),
                name: "beta".into(),
                command: "/usr/bin/beta".into(),
                cpu_percent: 2.0,
                memory_bytes: 200,
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: ProcessKey::new(33, 3),
                name: "gamma".into(),
                command: "/usr/bin/gamma".into(),
                cpu_percent: 3.0,
                memory_bytes: 300,
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: ProcessKey::new(44, 4),
                name: "firefox".into(),
                command: "/usr/bin/firefox".into(),
                cpu_percent: 10.0,
                memory_bytes: 1_000,
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: ProcessKey::new(55, 5),
                name: "firefox".into(),
                command: "/usr/bin/firefox --type=renderer".into(),
                cpu_percent: 5.0,
                memory_bytes: 500,
                ..ProcessSnapshot::default()
            },
        ];
        snapshot
    }

    #[test]
    fn others_row_compacts_single_process_apps_and_keeps_multi_process_group() {
        let mut dashboard = dashboard();
        dashboard.ingest(others_snapshot());
        let rows = dashboard.app_group_rows();
        assert_eq!(rows.len(), 2, "three single apps collapse into one row");
        assert_eq!(rows[0].kind, AppRowKind::Group);
        assert_eq!(rows[0].key, "firefox");
        assert_eq!(rows[0].process_count, 2);
        assert!(rows[0].has_children);
        assert_eq!(rows[0].depth, 0);

        let others = &rows[1];
        assert_eq!(others.kind, AppRowKind::Others);
        assert_eq!(others.key, OTHERS_APP_GROUP_KEY);
        assert_eq!(others.label, OTHERS_APP_GROUP_LABEL);
        assert_eq!(others.depth, 0);
        assert!(others.has_children);
        assert!(!others.expanded);
        assert_eq!(others.process_count, OTHERS_MIN_GROUP_COUNT + 1);
        assert_eq!(others.cpu_percent, 6.0);
        assert_eq!(others.memory_bytes, 600);
        assert_eq!(
            others.index, 2,
            "representative is the first sorted single app"
        );
        assert_eq!(others.process_key, ProcessKey::new(33, 3));
        assert_eq!(
            others.member_keys,
            vec![
                ProcessKey::new(33, 3),
                ProcessKey::new(22, 2),
                ProcessKey::new(11, 1),
            ]
        );
        assert_eq!(dashboard.visible_count(), 2);
        assert_eq!(dashboard.visible_depths(), &[0, 0]);
        assert_eq!(dashboard.visible_has_children(), &[true, true]);
    }

    #[test]
    fn expanding_others_reveals_one_process_group_rows_at_depth_one() {
        let mut dashboard = dashboard();
        dashboard.ingest(others_snapshot());
        dashboard.apply(Command::ToggleAppGroup(OTHERS_APP_GROUP_KEY.into()));
        assert!(dashboard.is_app_group_expanded(OTHERS_APP_GROUP_KEY));

        let rows = dashboard.app_group_rows();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[1].kind, AppRowKind::Others);
        assert!(rows[1].expanded);
        assert_eq!(
            rows[2..5]
                .iter()
                .map(|row| row.label.as_str())
                .collect::<Vec<_>>(),
            ["gamma", "beta", "alpha"]
        );
        assert!(rows[2..5].iter().all(|row| {
            row.kind == AppRowKind::Group
                && row.depth == 1
                && row.process_count == 1
                && !row.has_children
        }));
        assert_eq!(
            rows[2..5]
                .iter()
                .map(|row| row.process_key)
                .collect::<Vec<_>>(),
            vec![
                ProcessKey::new(33, 3),
                ProcessKey::new(22, 2),
                ProcessKey::new(11, 1),
            ]
        );
        assert_eq!(dashboard.visible_depths(), &[0, 0, 1, 1, 1]);
    }

    #[test]
    fn collapsing_others_moves_a_non_first_selection_to_the_representative() {
        let mut dashboard = dashboard();
        dashboard.ingest(others_snapshot());
        dashboard.apply(Command::ToggleAppGroup(OTHERS_APP_GROUP_KEY.into()));
        dashboard.apply(Command::Select(Some(ProcessKey::new(22, 2))));
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(22, 2)));
        assert_eq!(dashboard.selected_app_group_row_index(), Some(3));

        dashboard.apply(Command::ToggleAppGroup(OTHERS_APP_GROUP_KEY.into()));
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(33, 3)));
        assert_eq!(dashboard.selected_app_group_row_index(), Some(1));
        assert_eq!(dashboard.app_group_rows().len(), 2);
    }

    #[test]
    fn selecting_a_folded_app_maps_to_the_others_representative() {
        let mut dashboard = dashboard();
        dashboard.ingest(others_snapshot());
        dashboard.apply(Command::Select(Some(ProcessKey::new(22, 2))));
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(33, 3)));
        assert_eq!(dashboard.selected_visible_index(), Some(1));
        assert_eq!(dashboard.selected_app_group_row_index(), Some(1));
    }

    #[test]
    fn others_follows_filter_and_sort() {
        let mut dashboard = dashboard();
        dashboard.ingest(others_snapshot());

        // One surviving single app keeps its own row instead of a bucket.
        dashboard.apply(Command::SetFilterText("alpha".into()));
        let rows = dashboard.app_group_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, AppRowKind::Group);
        assert_eq!(rows[0].key, "alpha");
        assert!(rows.iter().all(|row| row.kind != AppRowKind::Others));

        dashboard.apply(Command::ClearFilter);
        dashboard.apply(Command::SetSortSpec(SortSpec {
            key: SortKey::Name,
            direction: SortDirection::Ascending,
        }));
        let rows = dashboard.app_group_rows();
        // The multi-process `firefox` group comes first; the bucket closes the
        // list. Name ascending still makes `alpha` the representative and keeps
        // the single apps in their sorted order inside the bucket.
        assert_eq!(rows[0].kind, AppRowKind::Group);
        assert_eq!(rows[0].key, "firefox");
        assert_eq!(rows[1].kind, AppRowKind::Others);
        assert_eq!(rows[1].process_key, ProcessKey::new(11, 1));
        assert_eq!(
            rows[1].member_keys,
            vec![
                ProcessKey::new(11, 1),
                ProcessKey::new(22, 2),
                ProcessKey::new(33, 3),
            ]
        );

        // CPU descending (the default) puts `gamma` first instead.
        dashboard.apply(Command::SetSortSpec(SortSpec {
            key: SortKey::Cpu,
            direction: SortDirection::Descending,
        }));
        let rows = dashboard.app_group_rows();
        assert_eq!(rows[1].kind, AppRowKind::Others);
        assert_eq!(rows[1].process_key, ProcessKey::new(33, 3));

        // Hiding the only multi-process app also removes the bucket: the
        // remaining single apps keep their own rows rather than becoming a
        // lone "others" entry.
        dashboard.apply(Command::SetFilterText("a".into()));
        let rows = dashboard.app_group_rows();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|row| row.kind == AppRowKind::Group));
        assert!(rows.iter().all(|row| row.key != OTHERS_APP_GROUP_KEY));
    }

    /// Two multi-process apps plus three single-process apps: the bucket has to
    /// clear both real group rows before it can be last (C4-VIEWMODEL).
    fn others_multi_group_snapshot() -> tm_core::SystemSnapshot {
        let mut snapshot = demo_snapshot(0);
        let process = |pid: u32, name: &str, cpu_percent| ProcessSnapshot {
            key: ProcessKey::new(pid, pid as u64),
            name: name.into(),
            command: format!("/usr/bin/{name} --pid={pid}"),
            cpu_percent,
            memory_bytes: 100,
            ..ProcessSnapshot::default()
        };
        snapshot.processes = vec![
            process(11, "alpha", 3.0),
            process(22, "beta", 2.0),
            process(33, "gamma", 1.0),
            process(44, "browser", 10.0),
            process(45, "browser", 5.0),
            process(66, "editor", 8.0),
            process(67, "editor", 4.0),
        ];
        snapshot
    }

    #[test]
    fn others_row_is_last_with_multiple_multi_process_groups() {
        let mut dashboard = dashboard();
        dashboard.ingest(others_multi_group_snapshot());
        let rows = dashboard.app_group_rows();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].key, "browser");
        assert_eq!(rows[1].key, "editor");
        let others = rows.last().expect("others row");
        assert_eq!(others.kind, AppRowKind::Others);
        assert_eq!(others.key, OTHERS_APP_GROUP_KEY);
        assert_eq!(others.process_count, 3);
        assert_eq!(dashboard.visible_count(), rows.len());
        assert_eq!(dashboard.visible_depths(), &[0, 0, 0]);
    }

    #[test]
    fn others_row_stays_last_when_expanded_and_selection_falls_back_to_it() {
        let mut dashboard = dashboard();
        dashboard.ingest(others_multi_group_snapshot());

        // A folded single resolves to the trailing bucket's representative.
        dashboard.apply(Command::Select(Some(ProcessKey::new(22, 22))));
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(11, 11)));
        assert_eq!(dashboard.selected_app_group_row_index(), Some(2));

        dashboard.apply(Command::ToggleAppGroup(OTHERS_APP_GROUP_KEY.into()));
        let rows = dashboard.app_group_rows();
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[2].kind, AppRowKind::Others);
        assert!(rows[2].expanded);
        assert_eq!(
            rows[3..]
                .iter()
                .map(|row| row.label.as_str())
                .collect::<Vec<_>>(),
            ["alpha", "beta", "gamma"]
        );
        assert_eq!(dashboard.selected_app_group_row_index(), Some(2));

        // A hidden member of a collapsed real group still maps to that group,
        // unaffected by the bucket now sitting behind it.
        dashboard.apply(Command::ToggleAppGroup(OTHERS_APP_GROUP_KEY.into()));
        dashboard.apply(Command::Select(Some(ProcessKey::new(67, 67))));
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(66, 66)));
        assert_eq!(dashboard.selected_app_group_row_index(), Some(1));
    }

    #[test]
    fn zero_cpu_rows_are_always_visible_and_stable_across_samples() {
        // Flat mode has one row per process: a 0% CPU reading must never hide a
        // background process, and the row set must not churn between samples.
        let mut flat = flat_dashboard();
        let mut previous_keys: Option<Vec<ProcessKey>> = None;
        for tick in 0..3 {
            flat.ingest(demo_snapshot(tick));
            let snapshot = flat.snapshot().expect("snapshot");
            let idle_keys: Vec<ProcessKey> = snapshot
                .processes
                .iter()
                .filter(|process| normalise_percent(process.cpu_percent) == 0.0)
                .map(|process| process.key)
                .collect();
            assert!(!idle_keys.is_empty(), "demo must contain 0% CPU rows");
            let mut indices = flat.visible_indices().to_vec();
            indices.sort_unstable();
            assert_eq!(
                indices,
                (0..snapshot.processes.len()).collect::<Vec<_>>(),
                "tick {tick}: every process keeps its visible row"
            );
            for key in &idle_keys {
                assert!(
                    indices
                        .iter()
                        .any(|&index| snapshot.processes[index].key == *key),
                    "tick {tick}: idle process {key:?} must stay visible"
                );
            }
            let keys: Vec<ProcessKey> = indices
                .iter()
                .map(|&index| snapshot.processes[index].key)
                .collect();
            if let Some(previous) = &previous_keys {
                assert_eq!(&keys, previous, "row set must not change between samples");
            }
            previous_keys = Some(keys);
        }
        assert!(!flat.filter().is_active());
    }

    #[test]
    fn grouped_view_keeps_zero_cpu_processes() {
        let mut dashboard = dashboard();
        dashboard.ingest(demo_snapshot(0));
        let snapshot = dashboard.snapshot().expect("snapshot").clone();
        assert!(snapshot
            .processes
            .iter()
            .any(|process| normalise_percent(process.cpu_percent) == 0.0));

        let rows = dashboard.app_group_rows();
        // Every process keeps a representation in the grouped view (a group row
        // or a folded "others" bucket member); none is dropped for being idle.
        let mut represented: HashSet<ProcessKey> = HashSet::new();
        for row in &rows {
            represented.insert(row.process_key);
            represented.extend(row.member_keys.iter().copied());
        }
        for process in &snapshot.processes {
            assert!(
                represented.contains(&process.key),
                "process {:?} must be represented in the grouped view",
                process.key
            );
        }
        // "Others", when present, still closes the list.
        if let Some(others) = rows.iter().position(|row| row.kind == AppRowKind::Others) {
            assert_eq!(others + 1, rows.len());
        }
        // ClearFilter / the All preset no longer change which rows exist.
        let before = rows.len();
        dashboard.apply(Command::ClearFilter);
        assert_eq!(dashboard.app_group_rows().len(), before);
        dashboard.apply(Command::ApplyFilterPreset(FilterPresetId::All));
        assert_eq!(dashboard.app_group_rows().len(), before);
    }

    #[test]
    fn idle_group_member_is_a_normal_representative_and_selectable() {
        let mut snapshot = demo_snapshot(0);
        snapshot.processes = vec![
            ProcessSnapshot {
                key: ProcessKey::new(10, 10),
                name: "worker".into(),
                command: "/usr/bin/worker".into(),
                cpu_percent: 0.0,
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: ProcessKey::new(20, 20),
                name: "worker".into(),
                command: "/usr/bin/worker --type=child".into(),
                cpu_percent: 4.0,
                ..ProcessSnapshot::default()
            },
        ];
        let mut dashboard = dashboard();
        dashboard.ingest(snapshot);
        let rows = dashboard.app_group_rows();
        assert_eq!(rows.len(), 1);
        let worker = &rows[0];
        assert_eq!(worker.kind, AppRowKind::Group);
        assert_eq!(worker.process_count, 2);
        assert_eq!(worker.process_key, ProcessKey::new(10, 10));
        assert!(worker.member_keys.contains(&ProcessKey::new(20, 20)));

        // The group row resolves to its smallest member even when that member
        // is idle; no summary row intercepts the selection.
        dashboard.apply(Command::Select(Some(ProcessKey::new(20, 20))));
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(10, 10)));
        // Expanding the group gives both members their own selectable rows,
        // including the idle one.
        dashboard.apply(Command::ToggleAppGroup("worker".into()));
        let expanded = dashboard.app_group_rows();
        assert_eq!(expanded.len(), 3);
        assert_eq!(expanded[1].kind, AppRowKind::Process);
        assert_eq!(expanded[1].process_key, ProcessKey::new(10, 10));
        dashboard.apply(Command::Select(Some(ProcessKey::new(20, 20))));
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(20, 20)));
        assert_eq!(dashboard.selected_app_group_row_index(), Some(2));
    }

    #[test]
    fn keeping_idle_rows_does_not_change_snapshot_or_export_input() {
        let mut dashboard = flat_dashboard();
        ingest_ticks(&mut dashboard, 1);
        let snapshot = dashboard.snapshot().expect("snapshot").clone();
        assert_eq!(snapshot.processes.len(), demo_snapshot(0).processes.len());
        assert!(snapshot
            .processes
            .iter()
            .any(|process| normalise_percent(process.cpu_percent) == 0.0));

        // All snapshot indices stay visible; export/wire keep reading the
        // unmodified `snapshot.processes` (no aggregate, no dropped idle row).
        let mut indices = dashboard.visible_indices().to_vec();
        indices.sort_unstable();
        assert_eq!(indices, (0..snapshot.processes.len()).collect::<Vec<_>>());
    }

    #[test]
    fn idle_processes_do_not_break_the_trailing_others_row() {
        const GIB: u64 = 1024 * 1024 * 1024;
        let mut snapshot = demo_snapshot(0);
        snapshot.processes = vec![
            ProcessSnapshot {
                key: ProcessKey::new(10, 10),
                name: "worker".into(),
                command: "/usr/bin/worker --one".into(),
                cpu_percent: 2.0,
                memory_bytes: 4 * GIB,
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: ProcessKey::new(20, 20),
                name: "worker".into(),
                command: "/usr/bin/worker --two".into(),
                cpu_percent: 3.0,
                memory_bytes: 5 * GIB,
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: ProcessKey::new(30, 30),
                name: "alpha".into(),
                command: "/usr/bin/alpha".into(),
                cpu_percent: 1.0,
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: ProcessKey::new(40, 40),
                name: "beta".into(),
                command: "/usr/bin/beta".into(),
                cpu_percent: 1.0,
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: ProcessKey::new(50, 50),
                name: "idle-daemon".into(),
                command: "/usr/bin/idle-daemon".into(),
                cpu_percent: 0.0,
                memory_bytes: 5 * GIB,
                ..ProcessSnapshot::default()
            },
        ];
        let mut dashboard = dashboard();
        dashboard.ingest(snapshot);
        let rows = dashboard.app_group_rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].key, "worker");
        assert_eq!(rows[0].memory_bytes, 9 * GIB);
        // The 0% single-process app folds into the trailing bucket like any
        // other single-process app; it is not dropped or moved into a summary.
        assert_eq!(rows[1].kind, AppRowKind::Others);
        assert_eq!(rows[1].process_count, 3);
        assert!(rows[1].member_keys.contains(&ProcessKey::new(50, 50)));
        assert_eq!(rows[1].memory_bytes, 5 * GIB);
        let total_memory: u64 = rows.iter().map(|row| row.memory_bytes).sum();
        assert_eq!(total_memory, 14 * GIB);

        // Expanding the bucket reveals its single-process groups (including the
        // idle daemon) directly beneath it; no idle summary participates.
        dashboard.apply(Command::ToggleAppGroup(OTHERS_APP_GROUP_KEY.into()));
        let expanded = dashboard.app_group_rows();
        let others = expanded
            .iter()
            .position(|row| row.kind == AppRowKind::Others)
            .expect("others");
        assert_eq!(others, 1);
        assert!(expanded[others].expanded);
        assert_eq!(
            expanded[others + 1..]
                .iter()
                .map(|row| row.label.as_str())
                .collect::<Vec<_>>(),
            ["alpha", "beta", "idle-daemon"]
        );
        assert!(expanded
            .iter()
            .any(|row| { row.kind == AppRowKind::Group && row.key == "idle-daemon" }));
    }

    #[test]
    fn others_is_not_created_without_a_multi_process_group() {
        let mut dashboard = dashboard();
        dashboard.ingest(demo_snapshot(0));
        let rows = dashboard.app_group_rows();
        assert!(rows.len() > 1, "demo still shows the ordinary app list");
        assert!(
            rows.iter().all(|row| row.kind != AppRowKind::Others),
            "an all-single-process snapshot must not gain an Others row"
        );
        assert_eq!(dashboard.visible_count(), rows.len());
    }

    #[test]
    fn others_expansion_is_pruned_when_the_bucket_disappears() {
        let mut dashboard = dashboard();
        dashboard.ingest(others_snapshot());
        dashboard.apply(Command::ToggleAppGroup(OTHERS_APP_GROUP_KEY.into()));
        assert!(dashboard.is_app_group_expanded(OTHERS_APP_GROUP_KEY));

        let mut next = others_snapshot();
        next.processes
            .retain(|process| process.name != "alpha" && process.name != "beta");
        dashboard.ingest(next);
        assert!(!dashboard.is_app_group_expanded(OTHERS_APP_GROUP_KEY));
        assert!(dashboard
            .app_group_rows()
            .iter()
            .all(|row| row.kind != AppRowKind::Others));
    }

    #[test]
    fn others_reserved_key_cannot_match_a_canonical_app_key() {
        assert!(OTHERS_APP_GROUP_KEY.starts_with('\0'));
        let mut snapshot = demo_snapshot(0);
        snapshot.processes = vec![ProcessSnapshot {
            key: ProcessKey::new(99, 9),
            name: "_others".into(),
            command: "_others --token=***".into(),
            ..ProcessSnapshot::default()
        }];
        // Even the tempting `_others` spelling is a legal canonical key, which
        // is why the reserved value carries a leading NUL instead.
        assert_eq!(app_group_key(&snapshot.processes[0]), "_others");
        assert_ne!(app_group_key(&snapshot.processes[0]), OTHERS_APP_GROUP_KEY);
    }

    #[test]
    fn trend_delta_reports_direction_amount_and_textual_cue() {
        let up = trend_delta(
            Some(40.0),
            Some(42.5),
            PERCENT_TREND_EPSILON,
            TrendUnit::PercentPoints,
        );
        assert_eq!(up.direction, TrendDirection::Up);
        assert_eq!(up.amount, "+2.5pt");
        assert_eq!(up.badge_text(), "↑ +2.5pt 上昇");
        assert_eq!(up.a11y_label("CPU"), "CPU の推移 上昇 +2.5pt");

        let down = trend_delta(
            Some(42.5),
            Some(40.0),
            PERCENT_TREND_EPSILON,
            TrendUnit::PercentPoints,
        );
        assert_eq!(down.direction, TrendDirection::Down);
        assert_eq!(down.amount, "-2.5pt");
        assert_eq!(down.badge_text(), "↓ -2.5pt 下降");
        assert_eq!(down.a11y_label("メモリ"), "メモリ の推移 下降 -2.5pt");
    }

    #[test]
    fn trend_delta_dead_band_and_rate_units_are_stable() {
        let flat = trend_delta(
            Some(40.0),
            Some(40.01),
            PERCENT_TREND_EPSILON,
            TrendUnit::PercentPoints,
        );
        assert_eq!(flat.direction, TrendDirection::Flat);
        assert_eq!(flat.badge_text(), "→ ±0.0pt 横ばい");

        let rate = trend_delta(Some(0.0), Some(1_556_023.0), 0.0, TrendUnit::BytesPerSecond);
        assert_eq!(rate.direction, TrendDirection::Up);
        assert!(rate.amount.starts_with('+'), "{}", rate.amount);
        assert!(rate.amount.ends_with("/s"), "{}", rate.amount);
        assert!(rate.badge_text().contains("上昇"), "{}", rate.badge_text());

        let unchanged = trend_delta(Some(10.0), Some(10.0), 0.0, TrendUnit::BytesPerSecond);
        assert_eq!(unchanged.direction, TrendDirection::Flat);
        assert_eq!(unchanged.amount, "±0 B/s");
    }

    #[test]
    fn trend_delta_bytes_per_second_down_uses_negative_magnitude() {
        let down = trend_delta(Some(1_556_023.0), Some(0.0), 0.0, TrendUnit::BytesPerSecond);
        assert_eq!(down.direction, TrendDirection::Down);
        // The Down branch must prefix the magnitude with a minus, mirroring
        // the Up branch's leading plus (R5-I05).
        assert!(down.amount.starts_with('-'), "{}", down.amount);
        assert!(down.amount.ends_with("/s"), "{}", down.amount);
        assert_eq!(down.amount, format!("-{}", format::format_rate(1_556_023)));

        // The Japanese badge/a11y contract stays intact for a falling rate.
        assert_eq!(down.badge_text(), format!("↓ {} 下降", down.amount));
        assert_eq!(
            down.a11y_label("ネットワーク"),
            format!("ネットワーク の推移 下降 {}", down.amount)
        );
    }

    #[test]
    fn trend_delta_requires_two_finite_samples() {
        let samples = [
            (None, Some(1.0)),
            (Some(1.0), None),
            (None, None),
            (Some(f64::NAN), Some(1.0)),
            (Some(1.0), Some(f64::INFINITY)),
        ];
        for (oldest, latest) in samples {
            let delta = trend_delta(oldest, latest, 0.0, TrendUnit::PercentPoints);
            assert_eq!(delta.direction, TrendDirection::Unknown);
            assert_eq!(delta.badge_text(), "— 不明");
            assert_eq!(delta.a11y_label("メモリ"), "メモリ の推移 不明");
        }

        // An invalid dead-band must not suppress a real movement.
        for epsilon in [-1.0, f64::NAN] {
            let moved = trend_delta(Some(0.0), Some(1.0), epsilon, TrendUnit::PercentPoints);
            assert_eq!(moved.direction, TrendDirection::Up);
        }
    }

    #[test]
    fn dashboard_trends_read_the_oldest_and_newest_history_samples() {
        let mut dashboard = dashboard();
        dashboard.ingest(demo_snapshot(0));
        assert_eq!(dashboard.cpu_trend().direction, TrendDirection::Unknown);
        assert_eq!(dashboard.memory_trend().direction, TrendDirection::Unknown);
        assert_eq!(dashboard.network_trend().direction, TrendDirection::Unknown);

        dashboard.ingest(demo_snapshot(1));
        let history = dashboard.history();
        let expected_cpu = trend_delta(
            history.cpu_percent.earliest().copied().map(f64::from),
            history.cpu_percent.latest().copied().map(f64::from),
            PERCENT_TREND_EPSILON,
            TrendUnit::PercentPoints,
        );
        assert_eq!(dashboard.cpu_trend(), expected_cpu);
        // (0 B/s, 0 B/s) → (+1_234_567 B/s, +321_456 B/s) on the demo interface.
        assert_eq!(dashboard.network_trend().direction, TrendDirection::Up);
        assert!(
            dashboard.network_trend().amount.ends_with("MiB/s"),
            "{}",
            dashboard.network_trend().amount
        );
    }

    #[test]
    fn trend_window_follows_the_ring_buffer_window() {
        let mut dashboard = dashboard();
        let final_tick = HISTORY_CAPACITY as u64 + 2;
        for tick in 0..=final_tick {
            dashboard.ingest(demo_snapshot(tick));
        }
        assert_eq!(dashboard.history().len(), HISTORY_CAPACITY);

        let history = dashboard.history();
        let expected = trend_delta(
            history.cpu_percent.earliest().copied().map(f64::from),
            history.cpu_percent.latest().copied().map(f64::from),
            PERCENT_TREND_EPSILON,
            TrendUnit::PercentPoints,
        );
        assert_eq!(dashboard.cpu_trend(), expected);
        let expected_network = trend_delta(
            history
                .network_rx_rate
                .earliest()
                .zip(history.network_tx_rate.earliest())
                .map(|(rx, tx)| rx.saturating_add(*tx) as f64),
            history
                .network_rx_rate
                .latest()
                .zip(history.network_tx_rate.latest())
                .map(|(rx, tx)| rx.saturating_add(*tx) as f64),
            0.0,
            TrendUnit::BytesPerSecond,
        );
        assert_eq!(dashboard.network_trend(), expected_network);
    }

    // ------------------------------------------------------ filter presets (R1-I06)

    #[test]
    fn filter_presets_are_built_in_unique_and_labelled() {
        assert_eq!(
            FILTER_PRESETS.first().expect("presets").id,
            FilterPresetId::All,
            "the reset choice must come first"
        );
        let mut ids = HashSet::new();
        for preset in FILTER_PRESETS {
            assert!(
                ids.insert(preset.id),
                "duplicate preset id: {:?}",
                preset.id
            );
            assert_eq!(preset.label, preset.id.label());
            assert_eq!(filter_preset(preset.id), preset);
        }
        for id in [
            FilterPresetId::All,
            FilterPresetId::HighCpu,
            FilterPresetId::HighMemory,
            FilterPresetId::LargeVirtualMemory,
            FilterPresetId::Running,
        ] {
            assert!(ids.contains(&id), "missing preset: {id:?}");
        }
        assert!(!filter_preset(FilterPresetId::All).filter.is_active());
        assert_eq!(
            filter_preset(FilterPresetId::HighCpu)
                .filter
                .min_cpu_percent,
            Some(PRESET_HIGH_CPU_PERCENT)
        );
        assert_eq!(
            filter_preset(FilterPresetId::HighMemory)
                .filter
                .min_memory_bytes,
            Some(PRESET_HIGH_MEMORY_BYTES)
        );
        assert_eq!(
            filter_preset(FilterPresetId::LargeVirtualMemory)
                .filter
                .min_virtual_memory_bytes,
            Some(PRESET_LARGE_VIRTUAL_MEMORY_BYTES)
        );
        assert_eq!(
            filter_preset(FilterPresetId::Running).filter.status,
            Some(ProcessStatus::Running)
        );
    }

    #[test]
    fn applying_a_preset_filters_the_table_and_marks_it_active() {
        let mut dashboard = flat_dashboard();
        ingest_ticks(&mut dashboard, 1);
        // Every process keeps its row by default; the presets only filter by
        // their structural conditions.
        let default_count = dashboard.visible_count();
        let total_count = dashboard.snapshot().expect("snapshot").processes.len();
        assert_eq!(default_count, total_count);
        assert_eq!(dashboard.active_filter_preset(), None);

        assert!(dashboard
            .apply(Command::ApplyFilterPreset(FilterPresetId::HighCpu))
            .is_empty());
        assert_eq!(
            dashboard.active_filter_preset(),
            Some(FilterPresetId::HighCpu)
        );
        let expected = dashboard
            .snapshot()
            .expect("snapshot")
            .processes
            .iter()
            .filter(|process| normalise_percent(process.cpu_percent) >= PRESET_HIGH_CPU_PERCENT)
            .count();
        assert_eq!(dashboard.visible_count(), expected);
        assert!(
            expected < default_count,
            "demo data must contain low-CPU rows"
        );

        // Switching presets is a single effect-free command; "All" resets the
        // filter while keeping the marker so the UI can highlight that chip.
        dashboard.apply(Command::ApplyFilterPreset(FilterPresetId::All));
        assert_eq!(dashboard.visible_count(), total_count);
        assert_eq!(dashboard.active_filter_preset(), Some(FilterPresetId::All));
    }

    #[test]
    fn direct_filter_edits_clear_the_active_preset_marker() {
        let commands = [
            Command::SetFilterText("firefox".to_string()),
            Command::ClearFilter,
            Command::SetMinVirtualMemoryBytes(Some(1)),
        ];
        for command in commands {
            let mut dashboard = dashboard();
            ingest_ticks(&mut dashboard, 1);
            dashboard.apply(Command::ApplyFilterPreset(FilterPresetId::HighCpu));
            assert_eq!(
                dashboard.active_filter_preset(),
                Some(FilterPresetId::HighCpu)
            );
            dashboard.apply(command.clone());
            assert_eq!(
                dashboard.active_filter_preset(),
                None,
                "{command:?} changed the filter directly"
            );
        }
    }

    #[test]
    fn typing_after_a_preset_clears_its_structural_conditions() {
        let mut dashboard = flat_dashboard();
        ingest_ticks(&mut dashboard, 1);
        let (total, firefox_index) = {
            let snapshot = dashboard.snapshot().expect("snapshot");
            let index = snapshot
                .processes
                .iter()
                .position(|process| process.name == "firefox")
                .expect("demo firefox process");
            assert!(
                normalise_percent(snapshot.processes[index].cpu_percent) < PRESET_HIGH_CPU_PERCENT,
                "firefox must be a low-CPU process for this regression test"
            );
            (snapshot.processes.len(), index)
        };

        // The High CPU preset hides the low-CPU firefox row via a structural
        // condition that is not shown in the text field.
        dashboard.apply(Command::ApplyFilterPreset(FilterPresetId::HighCpu));
        assert_eq!(
            dashboard.filter().min_cpu_percent,
            Some(PRESET_HIGH_CPU_PERCENT)
        );
        assert!(dashboard.visible_count() < total);
        assert!(!dashboard.visible_indices().contains(&firefox_index));

        // Typing a text query must drop the preset's CPU threshold; otherwise
        // "firefox" stays hidden by a condition the UI no longer advertises.
        dashboard.apply(Command::SetFilterText("firefox".to_string()));
        assert_eq!(dashboard.active_filter_preset(), None);
        assert_eq!(dashboard.filter().min_cpu_percent, None);
        assert_eq!(dashboard.filter().text, "firefox");
        assert_eq!(dashboard.visible_count(), 1);
        assert!(dashboard.visible_indices().contains(&firefox_index));
    }

    // -------------------------------------------- selected process series (R1-I08)

    #[test]
    fn selected_cpu_history_tracks_only_the_selected_process() {
        let mut dashboard = dashboard();
        assert!(dashboard.selected_cpu_history().is_empty());
        assert_eq!(
            dashboard.selected_cpu_history().capacity(),
            HISTORY_CAPACITY
        );

        dashboard.ingest(demo_snapshot(0));
        let key = firefox_key();
        dashboard.apply(Command::Select(Some(key)));
        assert!(
            dashboard.selected_cpu_history().is_empty(),
            "a selection change starts a new series"
        );

        for tick in 1..=3 {
            dashboard.ingest(demo_snapshot(tick));
        }
        let values = dashboard.selected_cpu_values();
        assert_eq!(values.len(), 3);
        for (offset, &value) in values.iter().enumerate() {
            let expected = demo_snapshot(offset as u64 + 1)
                .process(key)
                .expect("firefox")
                .cpu_percent;
            assert_eq!(value, expected, "sample {offset}");
        }
        assert_eq!(
            dashboard.selected_cpu_history().earliest().copied(),
            Some(values[0])
        );
        assert_eq!(
            dashboard.selected_cpu_history().latest().copied(),
            Some(values[2])
        );

        let other = dashboard
            .snapshot()
            .expect("snapshot")
            .processes
            .iter()
            .find(|process| process.key != key)
            .expect("another process")
            .key;
        dashboard.apply(Command::Select(Some(other)));
        assert!(
            dashboard.selected_cpu_history().is_empty(),
            "the previous process's samples must not be attributed to the new one"
        );
        dashboard.ingest(demo_snapshot(4));
        assert_eq!(dashboard.selected_cpu_values().len(), 1);
        let expected = demo_snapshot(4)
            .process(other)
            .expect("selected process")
            .cpu_percent;
        assert_eq!(dashboard.selected_cpu_values()[0], expected);
    }

    #[test]
    fn cleared_and_replaced_selections_reset_the_cpu_series() {
        let mut dashboard = dashboard();
        ingest_ticks(&mut dashboard, 3);
        assert_eq!(dashboard.selected_cpu_values().len(), 3);

        dashboard.apply(Command::Select(None));
        assert!(dashboard.selected_cpu_history().is_empty());
        dashboard.ingest(demo_snapshot(3));
        assert!(
            dashboard.selected_cpu_history().is_empty(),
            "no series while nothing is selected"
        );

        let key = firefox_key();
        dashboard.apply(Command::Select(Some(key)));
        dashboard.ingest(demo_snapshot(4));
        assert_eq!(dashboard.selected_cpu_values().len(), 1);

        // A disappearing process triggers the row-position fallback; the new
        // identity must not inherit the old process's samples.
        let old_key = dashboard.selected_key().expect("selection");
        let mut next = demo_snapshot(5);
        next.processes.retain(|process| process.key != old_key);
        dashboard.ingest(next);
        assert_ne!(dashboard.selected_key(), Some(old_key));
        assert_eq!(dashboard.selected_cpu_values().len(), 1);
    }

    #[test]
    fn selected_cpu_history_is_bounded_and_paused_ingests_add_nothing() {
        let mut dashboard = dashboard();
        for tick in 0..(HISTORY_CAPACITY as u64 + 5) {
            dashboard.ingest(demo_snapshot(tick));
        }
        assert_eq!(
            dashboard.selected_cpu_history().len(),
            HISTORY_CAPACITY,
            "the series must not grow without bound"
        );

        dashboard.apply(Command::SetPaused(true));
        dashboard.ingest(demo_snapshot(HISTORY_CAPACITY as u64 + 6));
        assert_eq!(dashboard.selected_cpu_history().len(), HISTORY_CAPACITY);
    }

    // --- C1-VMCLASSIFY: injected app/background classification ---------------

    /// Two app groups, two background groups and two foldable singles.
    ///
    /// The background groups win on the default CPU sort, so injecting the app
    /// keys produces a visible order change instead of a no-op.
    fn classification_snapshot() -> tm_core::SystemSnapshot {
        let mut snapshot = demo_snapshot(0);
        let processes = {
            let mut processes = Vec::new();
            let mut push = |pid: u32, name: &str, cpu_percent: f32| {
                processes.push(ProcessSnapshot {
                    key: ProcessKey::new(pid, 1),
                    name: name.into(),
                    command: format!("/usr/bin/{name}"),
                    cpu_percent,
                    memory_bytes: 100,
                    ..ProcessSnapshot::default()
                });
            };
            push(10, "zebra", 4.0);
            push(11, "zebra", 3.0);
            push(20, "apple", 1.0);
            push(21, "apple", 0.5);
            push(30, "yard", 6.0);
            push(31, "yard", 2.0);
            push(40, "beach", 0.9);
            push(41, "beach", 0.4);
            push(50, "alpha", 0.5);
            push(60, "beta", 0.25);
            processes
        };
        snapshot.processes = processes;
        snapshot
    }

    fn key_set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    fn group_keys(dashboard: &Dashboard) -> Vec<String> {
        dashboard
            .app_group_rows()
            .iter()
            .map(|row| row.key.clone())
            .collect()
    }

    #[test]
    fn empty_app_keys_keep_legacy_sort_and_add_no_headings() {
        let mut dashboard = dashboard();
        dashboard.ingest(classification_snapshot());
        assert!(dashboard.app_keys().is_empty());
        assert_eq!(
            group_keys(&dashboard),
            ["yard", "zebra", "apple", "beach", OTHERS_APP_GROUP_KEY]
        );
        // While classification is still loading the display API is a pure
        // wrapper: no headings, byte-for-byte the legacy rows.
        assert_eq!(
            dashboard.app_group_display_rows(),
            dashboard.app_group_rows()
        );

        // With no injected set the chosen sort is the only ordering key.
        dashboard.apply(Command::SetSortSpec(SortSpec {
            key: SortKey::Name,
            direction: SortDirection::Ascending,
        }));
        assert_eq!(
            group_keys(&dashboard),
            ["apple", "beach", "yard", "zebra", OTHERS_APP_GROUP_KEY]
        );
    }

    #[test]
    fn app_keys_injection_puts_apps_first_and_keeps_chosen_sort_second() {
        let mut dashboard = dashboard();
        dashboard.ingest(classification_snapshot());
        let keys = key_set(&["zebra", "apple"]);
        dashboard.set_app_keys(keys.clone());
        assert_eq!(dashboard.app_keys(), &keys);
        assert_eq!(
            group_keys(&dashboard),
            ["zebra", "apple", "yard", "beach", OTHERS_APP_GROUP_KEY]
        );
        // `visible` follows the new order and still holds real snapshot indices.
        assert_eq!(dashboard.visible_indices(), &[0, 2, 4, 6, 8]);

        // Classification stays the primary key; an explicit sort only orders
        // groups *inside* each class (SortSpec itself is untouched).
        dashboard.apply(Command::SetSortSpec(SortSpec {
            key: SortKey::Name,
            direction: SortDirection::Ascending,
        }));
        assert_eq!(dashboard.sort_spec().key, SortKey::Name);
        assert_eq!(
            group_keys(&dashboard),
            ["apple", "zebra", "beach", "yard", OTHERS_APP_GROUP_KEY]
        );
    }

    #[test]
    fn display_rows_insert_two_section_headings_and_keep_others_last() {
        let mut dashboard = dashboard();
        dashboard.ingest(classification_snapshot());
        dashboard.set_app_keys(key_set(&["zebra", "apple"]));

        let display = dashboard.app_group_display_rows();
        assert_eq!(display.len(), dashboard.app_group_rows().len() + 2);
        assert_eq!(
            display
                .iter()
                .filter(|row| row.kind == AppRowKind::SectionHeader)
                .count(),
            2
        );
        assert_eq!(display[0].key, APPS_SECTION_KEY);
        assert_eq!(display[0].label, APPS_SECTION_LABEL);
        assert_eq!(display[3].key, BACKGROUND_SECTION_KEY);
        assert_eq!(display[3].label, BACKGROUND_SECTION_LABEL);

        // Headings select nothing: no members, an unreachable identity and no
        // children. Their index is a borrowed in-range value, not a new row.
        let snapshot = dashboard.snapshot().expect("snapshot");
        for header in display
            .iter()
            .filter(|row| row.kind == AppRowKind::SectionHeader)
        {
            assert!(header.member_keys.is_empty());
            assert!(snapshot.process(header.process_key).is_none());
            assert!(!header.has_children);
            assert!(!header.expanded);
            assert!(header.index < snapshot.processes.len());
        }

        // The "others" bucket is unclassified, has no heading and still closes
        // the list (C1-COLLAPSE contract preserved).
        let last = display.last().expect("display rows");
        assert_eq!(last.kind, AppRowKind::Others);
        assert_eq!(last.key, OTHERS_APP_GROUP_KEY);
        assert!(display[..display.len() - 1]
            .iter()
            .all(|row| row.key != OTHERS_APP_GROUP_KEY));
    }

    #[test]
    fn classification_leaves_selection_visible_and_export_contracts_intact() {
        use tm_core::export::snapshot_to_csv;
        use tm_core::redact::RedactionIdentifiers;

        let ids = RedactionIdentifiers {
            home: "",
            user: "",
            hostname: "",
        };
        let mut dashboard = dashboard();
        dashboard.ingest(classification_snapshot());
        let before = snapshot_to_csv(dashboard.snapshot().expect("snapshot"), &ids);

        dashboard.set_app_keys(key_set(&["zebra"]));
        dashboard.apply(Command::SelectAppGroup("zebra".into()));
        assert_eq!(dashboard.selected_key(), Some(ProcessKey::new(10, 1)));
        assert_eq!(dashboard.selected_visible_index(), Some(0));
        assert_eq!(dashboard.selected_app_group_row_index(), Some(0));

        // Display headings never enter `visible`; exports still read the raw
        // snapshot and are identical before and after classification.
        assert_eq!(dashboard.visible_count(), dashboard.app_group_rows().len());
        let snapshot = dashboard.snapshot().expect("snapshot");
        let after = snapshot_to_csv(snapshot, &ids);
        assert_eq!(before, after);
        assert_eq!(after.lines().count(), snapshot.processes.len() + 1);

        // A heading's sentinel identity matches no process, so the ordinary
        // selection path clears the selection instead of latching a bogus
        // target that could later be signalled.
        dashboard.apply(Command::Select(Some(ProcessKey::new(u32::MAX, u64::MAX))));
        assert_eq!(dashboard.selected_key(), None);
    }
}

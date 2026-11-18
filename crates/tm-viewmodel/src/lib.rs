//! UI-independent presentation state machine.
//!
//! The native GPUI shell and the browser shell both drive this same type. It
//! owns the "what is on screen" state (sorting, filtering, selection, dialog,
//! refresh cadence) and emits [`Effect`] values that the shell executes
//! (`sample now`, `send a signal`). Because there is no I/O here, every
//! interaction is a deterministic unit test.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::collections::HashSet;
use std::time::Duration;

use tm_core::format;
use tm_core::{
    filter_and_sort, flatten_process_tree, Filter, MetricHistory, NetworkCounters, PlatformInfo,
    ProcessKey, ProcessSnapshot, ProcessTreeRow, RingBuffer, Signal, SortKey, SortSpec,
    SystemSnapshot, Timestamp,
};

pub mod filter_input;
pub mod settings;

pub use filter_input::{EditOutcome, FilterInput};
pub use settings::{Settings, Theme, SETTINGS_VERSION};

/// Lowest refresh interval the UI accepts (250 ms).
pub const MIN_REFRESH_INTERVAL: Duration = Duration::from_millis(250);
/// Highest refresh interval the UI accepts (10 s).
pub const MAX_REFRESH_INTERVAL: Duration = Duration::from_secs(10);
/// Default refresh interval (1 s).
pub const DEFAULT_REFRESH_INTERVAL: Duration = Duration::from_millis(1_000);
/// Number of chart samples kept (two minutes at the default cadence).
pub const HISTORY_CAPACITY: usize = 120;

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
    ToggleTreeView,
    /// Set the process tree view explicitly (used when restoring settings).
    SetTreeView(bool),
    /// Toggle collapse/expand state for a process node in tree view.
    ToggleTreeNode(ProcessKey),
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
                | Self::SetFilterText(_)
                | Self::ClearFilter
                | Self::SetMinVirtualMemoryBytes(_)
                | Self::SetInterval(_)
                | Self::ToggleDetails
                | Self::SetDetailsVisible(_)
                | Self::ToggleTreeView
                | Self::SetTreeView(_)
                | Self::ToggleTreeNode(_)
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

    fn persistent_error(text: impl Into<String>) -> Self {
        Self {
            text: format!("{STATUS_PREFIX_ERROR} {}", text.into()),
            is_error: true,
            persistent: true,
        }
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
    collapsed: HashSet<ProcessKey>,
    history: MetricHistory,
    sort: SortSpec,
    filter: Filter,
    selected: Option<ProcessKey>,
    selection_hint: usize,
    details_visible: bool,
    paused: bool,
    interval: Duration,
    pending_kill: Option<PendingKill>,
    status: Option<StatusMessage>,
    last_network: Option<(u64, u64, Timestamp)>,
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
            collapsed: HashSet::new(),
            history: MetricHistory::new(HISTORY_CAPACITY),
            sort: SortSpec::default(),
            filter: Filter::default(),
            selected: None,
            selection_hint: 0,
            details_visible: true,
            paused: false,
            interval: clamp_interval(interval),
            pending_kill: None,
            status: None,
            last_network: None,
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

    /// Current refresh interval.
    pub fn interval(&self) -> Duration {
        self.interval
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

    /// Selected process identity.
    pub fn selected_key(&self) -> Option<ProcessKey> {
        self.selected
    }

    /// Selected process row, resolved against the latest snapshot.
    pub fn selected_process(&self) -> Option<&ProcessSnapshot> {
        let snapshot = self.snapshot.as_ref()?;
        snapshot.process(self.selected?)
    }

    /// Indices into `snapshot.processes` for the rows the table shows, in display order.
    pub fn visible_indices(&self) -> &[usize] {
        &self.visible
    }

    /// Nesting depth for each visible row (0 for flat view and tree roots).
    pub fn visible_depths(&self) -> &[usize] {
        &self.visible_depths
    }

    /// Returns visible child markers aligned with `visible_indices`.
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

    /// Number of rows after filtering.
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
                self.selected = key.filter(|key| {
                    self.snapshot
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.process(*key).is_some())
                });
                self.sync_selection_hint();
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
            Command::SetFilterText(text) => {
                self.filter.text = text;
                self.recompute_visible();
                Vec::new()
            }
            Command::ClearFilter => {
                self.filter = Filter::default();
                self.recompute_visible();
                Vec::new()
            }
            Command::SetMinVirtualMemoryBytes(bytes) => {
                self.filter.min_virtual_memory_bytes = bytes;
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
                self.tree_view = !self.tree_view;
                self.recompute_visible();
                Vec::new()
            }
            Command::SetTreeView(tree_view) => {
                self.tree_view = tree_view;
                self.recompute_visible();
                Vec::new()
            }
            Command::ToggleTreeNode(key) => {
                if self.tree_view {
                    if self.collapsed.contains(&key) {
                        self.collapsed.remove(&key);
                    } else {
                        self.collapsed.insert(key);
                    }
                    self.recompute_visible();
                }
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
    pub fn ingest(&mut self, snapshot: SystemSnapshot) {
        if self.paused {
            // Keep the "paused" notice: an in-flight sample finishing now must
            // not clear it (R23 regression from R21/R22 interaction).
            return;
        }
        // Only a snapshot that was actually ingested may clear a transient
        // notice. Doing this here covers every shell (native, live web, demo).
        self.clear_transient_status();
        let (rx_rate, tx_rate) = match self.last_network {
            Some((previous_rx, previous_tx, previous_at)) => {
                let elapsed = Duration::from_millis(
                    snapshot
                        .sampled_at
                        .unix_millis
                        .saturating_sub(previous_at.unix_millis),
                );
                NetworkCounters::new(snapshot.network_totals().0, snapshot.network_totals().1)
                    .rate_since(NetworkCounters::new(previous_rx, previous_tx), elapsed)
            }
            None => (0, 0),
        };
        self.network_rates = (rx_rate, tx_rate);
        let (total_rx, total_tx) = snapshot.network_totals();
        self.last_network = Some((total_rx, total_tx, snapshot.sampled_at));

        self.history.push(
            snapshot.cpu.total_percent,
            snapshot.memory.used_percent(),
            rx_rate,
            tx_rate,
        );

        let selected_still_exists = self
            .selected
            .is_some_and(|key| snapshot.process(key).is_some());
        // `recompute_visible()` re-syncs the hint against the *new* selection,
        // so preserve the previous row position for the disappearance fallback.
        let previous_hint = self.selection_hint;
        self.snapshot = Some(snapshot);
        self.recompute_visible();
        if !selected_still_exists {
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
    }

    /// Sets a transient status message (cleared on the next successful sample).
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
    pub fn set_persistent_status(&mut self, text: impl Into<String>) {
        self.status = Some(StatusMessage::persistent_error(text));
    }

    /// Clears the current status only when it is transient (R3-U05).
    pub fn clear_transient_status(&mut self) {
        if self
            .status
            .as_ref()
            .is_some_and(|status| !status.persistent)
        {
            self.status = None;
        }
    }

    /// Clears the status message.
    pub fn clear_status(&mut self) {
        self.status = None;
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
            self.status = Some(StatusMessage::error(
                "この環境ではプロセス操作は無効です（読み取り専用）",
            ));
            return;
        }
        // UI buttons already hide unsupported signals; reject them here too so
        // a direct Command cannot bypass the per-signal capability gate.
        if !self.platform.supports_signal(signal) {
            self.status = Some(StatusMessage::error(
                "この環境ではこのシグナルは使用できません",
            ));
            return;
        }
        let Some(process) = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.process(key))
        else {
            self.status = Some(StatusMessage::error("対象のプロセスは既に終了しています"));
            return;
        };
        if self.is_protected(key) {
            self.status = Some(StatusMessage::error(
                "PID 1 と自分自身は操作できません（安全のため保護されています）",
            ));
            return;
        }
        // Never silently replace an unanswered confirmation with a different
        // target/signal: the dialog the user is looking at would change under
        // them (R2-F02).
        if self.pending_kill.is_some() {
            self.status = Some(StatusMessage::info(
                "確認待ちの操作があります。先に確定またはキャンセルしてください",
            ));
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
        self.status = Some(StatusMessage::info(confirmation_note));
    }

    fn confirm_kill(&mut self) -> Vec<Effect> {
        // A paused dashboard shows stale data: the process may have exited and
        // its PID been reused without our knowledge. Never signal from a stale
        // confirmation (R2-F01).
        if self.paused {
            self.pending_kill = None;
            self.status = Some(StatusMessage::error(
                "更新が一時停止中のため操作できません。再開してからやり直してください",
            ));
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
            self.status = Some(StatusMessage::error(
                "対象のプロセスは既に終了しています（PID 再利用を検出）",
            ));
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
    }

    fn recompute_visible(&mut self) {
        let Some(snapshot) = &self.snapshot else {
            self.visible.clear();
            self.visible_depths.clear();
            self.visible_has_children.clear();
            return;
        };
        let base = filter_and_sort(&snapshot.processes, &self.filter, self.sort);
        if self.tree_view {
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
    }

    fn ensure_selection_visible(&mut self) {
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

    fn ingest_ticks(dashboard: &mut Dashboard, ticks: u64) {
        for tick in 0..ticks {
            dashboard.ingest(demo_snapshot(tick));
        }
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
    fn default_sort_is_cpu_descending() {
        let mut dashboard = dashboard();
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
        let mut dashboard = dashboard();
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
        let mut dashboard = dashboard();
        ingest_ticks(&mut dashboard, 1);
        dashboard.apply(Command::SetSortKey(SortKey::VirtualMemory));
        let spec = dashboard.sort_spec();
        assert_eq!(spec.key, SortKey::VirtualMemory);
        assert_eq!(spec.direction, SortDirection::Descending);
        assert_eq!(spec.direction.arrow(), SortDirection::Descending.arrow());
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
        let mut dashboard = dashboard();
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
        let mut dashboard = dashboard();
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
        let mut dashboard = dashboard();
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
            Command::SetFilterText("firefox".to_string()),
            Command::ClearFilter,
            Command::SetMinVirtualMemoryBytes(Some(1)),
            Command::SetInterval(DEFAULT_REFRESH_INTERVAL),
            Command::ToggleDetails,
            Command::SetDetailsVisible(true),
            Command::ToggleTreeView,
            Command::SetTreeView(false),
            Command::ToggleTreeNode(firefox_key()),
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
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: ProcessKey::new(11, 2),
                parent_pid: Some(10),
                name: "leaf".to_string(),
                command: "leaf".to_string(),
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
                ..ProcessSnapshot::default()
            },
            ProcessSnapshot {
                key: child,
                parent_pid: Some(parent.pid),
                name: "child".to_string(),
                command: "child".to_string(),
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

        dashboard.apply(Command::ToggleTreeNode(parent));
        assert!(dashboard.is_collapsed(parent));
        assert_eq!(dashboard.visible_indices(), &[0]);
        let row = dashboard.visible_rows().next().unwrap();
        assert_eq!(row.index, 0);
        assert!(row.has_children);

        dashboard.apply(Command::ToggleTreeNode(parent));
        assert!(!dashboard.is_collapsed(parent));
        assert_eq!(dashboard.visible_indices(), &[0, 1]);
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
}

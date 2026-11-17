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
    ProcessKey, ProcessSnapshot, ProcessTreeRow, Signal, SortKey, SortSpec, SystemSnapshot,
    Timestamp,
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
}

/// Transient message shown in the status bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusMessage {
    /// Message text.
    pub text: String,
    /// `true` for errors and rejections.
    pub is_error: bool,
}

impl StatusMessage {
    fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: false,
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: true,
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

    /// Applies a command and returns the effects the shell must execute.
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
            Command::SetInterval(interval) => {
                self.interval = clamp_interval(interval);
                Vec::new()
            }
            Command::SetPaused(paused) => {
                self.paused = paused;
                if paused {
                    self.status = Some(StatusMessage::info("更新を一時停止しました"));
                    Vec::new()
                } else {
                    self.status = Some(StatusMessage::info("更新を再開しました"));
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
    pub fn ingest(&mut self, snapshot: SystemSnapshot) {
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

    /// Sets a transient status message (used by shells after executing effects).
    pub fn set_status(&mut self, text: impl Into<String>, is_error: bool) {
        self.status = Some(StatusMessage {
            text: text.into(),
            is_error,
        });
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
        self.pending_kill = Some(PendingKill {
            key,
            name: process.name.clone(),
            command: process.command.clone(),
            user: process.user.clone(),
            signal,
        });
        self.status = None;
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
        assert!(dashboard
            .apply(Command::SetInterval(Duration::from_millis(1_500)))
            .is_empty());
        assert!(dashboard
            .apply(Command::SetSortSpec(SortSpec {
                key: SortKey::Name,
                direction: SortDirection::Ascending,
            }))
            .is_empty());
        assert!(dashboard
            .apply(Command::SetDetailsVisible(false))
            .is_empty());
        assert!(dashboard.apply(Command::SetTreeView(true)).is_empty());
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
}

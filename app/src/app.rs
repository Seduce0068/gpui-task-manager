//! Native GPUI shell: window, overview cards, process table and details panel.
//!
//! The shell is intentionally thin. All presentation decisions that can be
//! tested live in `tm-viewmodel`; this file only renders [`Dashboard`] state and
//! forwards user intentions as [`Command`] values. The `SystemProbe` is kept
//! behind an `Arc<Mutex<...>>` and is only touched from the GPUI background
//! executor, so the UI thread never blocks on `/proc` reads.

use std::ops::Range;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::accesskit::{Live, Node, SortDirection as AccessKitSortDirection};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use tm_core::demo::DemoProbe;
use tm_core::format;
use tm_core::{
    PlatformInfo, ProcessKey, ProcessSnapshot, ProcessStatus, Signal, SortDirection, SortKey,
    SystemProbe, Timestamp,
};
use tm_viewmodel::{
    AppGroupRow, AppRowKind, Command, Dashboard, EditOutcome, Effect, FilterInput,
    DEFAULT_REFRESH_INTERVAL,
};

use crate::settings::Settings;
use crate::theme::*;
use crate::ui::{accessible_metric_bar, action_button, card, kv, pill, section_title, sparkline};

const COL_CPU: f32 = 76.0;
const COL_MEMORY: f32 = 96.0;
const COL_PID: f32 = 64.0;
const COL_USER: f32 = 104.0;
const COL_STATUS: f32 = 78.0;
const COL_THREADS: f32 = 64.0;
const DETAILS_WIDTH: f32 = 320.0;

/// Read-only power / sampling / cgroup summary for the status bar (R1-I10..I12).
///
/// All values come from the snapshot's fixed identifiers and labels, which were
/// sanitised at the probe boundary; no host name or user text is added here.
fn power_summary_text(snapshot: Option<&tm_core::SystemSnapshot>) -> String {
    match snapshot {
        Some(snapshot) => format!(
            "電源 {} / {} / cgroup {} グループ",
            snapshot.power.source.label(),
            snapshot.sampling.profile.label(),
            snapshot.cgroups.len()
        ),
        None => "電源 —".to_string(),
    }
}

/// Read-only cgroup grouping row: normalised label plus member count (R1-I12).
fn cgroup_group_text(label: &str, members: usize) -> String {
    format!("{label} — {members} 件")
}

/// Values rendered by one row of the process table (R1-APPDRAW).
///
/// A group row takes its name and its CPU/memory *totals* from the view model's
/// `AppGroupRow`. Columns that have no aggregate (command, PID, user, status,
/// threads) carry fixed text, so a group never shows one representative
/// process's identity as if it described the whole group.
#[derive(Clone, Debug, PartialEq)]
struct ProcessRowView {
    key: ProcessKey,
    /// Set only for a group row; the shell passes it to `ToggleAppGroup`.
    group_key: Option<String>,
    name: String,
    command: String,
    cpu_percent: f32,
    memory_bytes: u64,
    pid: String,
    user: String,
    status: ProcessStatus,
    status_text: String,
    threads: String,
    selected: bool,
    depth: usize,
    has_children: bool,
    is_group: bool,
    process_count: usize,
}

/// Resolves a visible row's display values without touching GPUI (R1-APPDRAW).
///
/// `group` is supplied only in the default grouped view and only for a row
/// whose view-model kind is [`AppRowKind::Group`]. Expanded member rows and all
/// flat/tree rows deliberately keep their original `ProcessSnapshot` values,
/// so their command/PID/user/status/threads rendering is unchanged.
fn process_row_values(
    group: Option<&AppGroupRow>,
    process: &ProcessSnapshot,
    selected: bool,
    depth: usize,
    has_children: bool,
) -> ProcessRowView {
    match group {
        Some(group) if group.kind == AppRowKind::Group => ProcessRowView {
            key: group.process_key,
            group_key: Some(group.key.clone()),
            name: group.label.clone(),
            command: format!("{} プロセス", group.process_count),
            cpu_percent: group.cpu_percent,
            memory_bytes: group.memory_bytes,
            pid: "—".to_string(),
            user: "—".to_string(),
            status: ProcessStatus::Unknown,
            status_text: "—".to_string(),
            threads: "—".to_string(),
            selected,
            depth,
            has_children,
            is_group: true,
            process_count: group.process_count,
        },
        _ => ProcessRowView {
            key: process.key,
            group_key: None,
            name: process.name.clone(),
            command: process.command.clone(),
            cpu_percent: process.cpu_percent,
            memory_bytes: process.memory_bytes,
            pid: process.key.pid.to_string(),
            user: process.user.clone().unwrap_or_else(|| "—".to_string()),
            status: process.status,
            status_text: process.status.label().to_string(),
            threads: process
                .threads
                .map(|threads| threads.to_string())
                .unwrap_or_else(|| "—".to_string()),
            selected,
            depth,
            has_children,
            is_group: false,
            process_count: 1,
        },
    }
}

type SharedProbe = Arc<Mutex<Option<Box<dyn SystemProbe>>>>;

/// Startup decision for the native shell (R3-S02B).
///
/// This is the window-side counterpart of the `main.rs` preflight. Both use
/// `tm_core::demo::choose_backend` and `tm_core::demo::probe_failure_message`,
/// so the policy and the wording stay identical across every entry point:
/// `--demo` is the *only* route to synthetic data, and a failed native probe is
/// an explicit error rather than an implicit switch to demo data (R3-S02).
///
/// A failed startup deliberately carries no probe: [`TaskManagerApp`] keeps an
/// empty `SharedProbe`, shows the shared failure message as a persistent status
/// and never displays the DEMO badge.
#[derive(Clone, Debug, PartialEq, Eq)]
enum BackendInit<P> {
    /// Synthetic data, requested explicitly with a validated `--demo`.
    Demo,
    /// Live data from a successfully created native probe.
    Native(P),
    /// The native probe failed; the message is shown as a persistent status.
    Failed(String),
}

impl<P> BackendInit<P> {
    /// Resolves the backend once, without ever creating a native probe for
    /// `--demo` (the same guarantee the GUI preflight gives in `main.rs`).
    fn resolve(demo: bool, native: impl FnOnce() -> Result<P, tm_core::ProbeError>) -> Self {
        match tm_core::demo::choose_backend(demo, native) {
            Ok(tm_core::demo::BackendChoice::Demo) => Self::Demo,
            Ok(tm_core::demo::BackendChoice::Native(probe)) => Self::Native(probe),
            Err(error) => Self::Failed(tm_core::demo::probe_failure_message(&error)),
        }
    }

    /// Persistent startup status to surface in the status bar.
    fn startup_status(&self) -> Option<&str> {
        match self {
            Self::Demo | Self::Native(_) => None,
            Self::Failed(message) => Some(message.as_str()),
        }
    }

    /// Whether the DEMO badge and synthetic data are justified.
    fn is_demo(&self) -> bool {
        matches!(self, Self::Demo)
    }
}

/// Pure outcome of re-resolving the backend after a failed native probe (R3-S02B).
///
/// This maps [`BackendInit::resolve`] onto the state update a retry performs:
/// [`ProbeRetry::Recovered`] installs a freshly built native probe, clears the
/// persistent startup failure and lets `request_sample` resume on the next tick;
/// [`ProbeRetry::StillFailed`] leaves the failure status visible and sampling
/// suppressed. No synthetic/demo fallback is ever produced here.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ProbeRetry<P> {
    /// A new native probe is available; install it and resume sampling.
    Recovered(P),
    /// The probe is still unavailable; keep the persistent failure status.
    StillFailed(String),
}

/// Re-runs the startup backend resolution for a retry.
///
/// It deliberately calls [`BackendInit::resolve`] with the same arguments as
/// startup, so the policy and the failure wording stay identical to `main.rs`.
fn resolve_probe_retry<P>(
    demo: bool,
    native: impl FnOnce() -> Result<P, tm_core::ProbeError>,
) -> ProbeRetry<P> {
    match BackendInit::resolve(demo, native) {
        BackendInit::Native(probe) => ProbeRetry::Recovered(probe),
        BackendInit::Failed(message) => ProbeRetry::StillFailed(message),
        // Retry is only reachable after a failed native probe, so `demo` is
        // false here. If that invariant is ever broken, stay conservative:
        // report "still unavailable" instead of installing synthetic data.
        BackendInit::Demo => ProbeRetry::StillFailed(tm_core::demo::probe_failure_message(
            &tm_core::ProbeError::Unsupported("demo requires no probe retry".to_string()),
        )),
    }
}

/// Applies the status half of a probe retry, with the persistent-notice guard.
///
/// A recovery only clears a status that is still the matching startup failure;
/// any unrelated notice (for example a settings warning) is preserved. A still
/// failing probe re-publishes the shared message as a persistent error. Kept
/// free of GPUI state so the guard is directly testable (R3-S02B).
fn apply_retry_status<P>(dashboard: &mut Dashboard, outcome: &ProbeRetry<P>, failure: &str) {
    match outcome {
        ProbeRetry::Recovered(_) => {
            if dashboard
                .status()
                .is_some_and(|status| status.persistent && status.text == failure)
            {
                dashboard.clear_status();
            }
        }
        ProbeRetry::StillFailed(message) => dashboard.set_persistent_status(message.clone()),
    }
}

/// Small snapshot of the values the overview and status bar render.
///
/// Cloning these values instead of the whole process list keeps `render` cheap:
/// process rows are built lazily by the virtual list from the live snapshot.
struct Summary {
    cpu_total: f32,
    cores: Vec<f32>,
    load: [f32; 3],
    uptime_secs: u64,
    frequency_mhz: Option<u64>,
    mem_used: u64,
    mem_total: u64,
    mem_percent: f32,
    swap_used: u64,
    swap_total: u64,
    swap_percent: f32,
    rx_rate: u64,
    tx_rate: u64,
    process_count: usize,
    visible_count: usize,
    hostname: String,
    kernel: String,
    sampled_at: Option<Timestamp>,
    paused: bool,
    interval_ms: u64,
    disks: Vec<DiskRow>,
    cpu_history: Vec<f32>,
    mem_history: Vec<f32>,
    rx_history: Vec<f32>,
    tx_history: Vec<f32>,
}

struct DiskRow {
    mount: String,
    percent: f32,
}

/// Native application state.
pub struct TaskManagerApp {
    dashboard: Dashboard,
    probe: SharedProbe,
    sampling_in_flight: bool,
    filter_editing: bool,
    /// 絞り込み欄の編集状態（IME の未確定文字列を含む）。
    ///
    /// 確定済み文字列だけを `dashboard` に反映する。IME からの UTF-16 編集は
    /// すべて [`EntityInputHandler`] 経由でここを通る。
    filter_input: FilterInput,
    show_help: bool,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    /// 列見出し（7 列）のフォーカスハンドル。Tab で辿り、Enter/Space で
    /// 既存の `on_click`（`Command::SetSortKey`）を発火させる（WCAG 2.1.1）。
    /// フレームをまたいで同じハンドルを使うため `render` 内では生成しない。
    sort_header_focus: [FocusHandle; 7],
    /// 表示行のフォーカスハンドル（visible index 対応）。Enter/Space で行の
    /// `on_click`（Group 行は `ToggleAppGroup`、他は `Select`）を発火させる。
    /// 行は仮想リストで再利用されるため index をキーにハンドルを保持する。
    row_focus: Vec<FocusHandle>,
    /// 確認ダイアログの安全側ボタン（キャンセル）のフォーカス先。
    dialog_cancel_focus: FocusHandle,
    /// 確認ダイアログの実行ボタンのフォーカス先。
    dialog_confirm_focus: FocusHandle,
    /// ヘルプの「閉じる」ボタンのフォーカス先。
    help_close_focus: FocusHandle,
    /// 前フレームのオーバーレイ状態。表示/非表示の遷移でフォーカスを移送・復帰する。
    last_overlay: OverlayKind,
    /// オーバーレイ表示前にフォーカスされていた要素（閉じたら復帰する）。
    focus_before_overlay: Option<FocusHandle>,
    demo_mode: bool,
    redact: bool,
    /// The native probe could not be created: no synthetic data is substituted,
    /// the persistent startup error stays visible and sampling is suppressed so
    /// a transient "probe is not available" message cannot replace it (R3-S02B).
    probe_unavailable: bool,
}

/// Surfaces a failed settings save on the status bar.
///
/// Success deliberately leaves the current status untouched: a startup or
/// action notice must not be erased by an unrelated save (C2-SETTINGS). The
/// message is a fixed, redacted constant, so no path or I/O detail leaks into
/// the UI. Factored out of the shell so the policy is directly testable.
///
/// The failure is persistent (not cleared by the next successful sample),
/// matching the web shell's `SettingsNotice::save_failed()` (EB-SAVE).
fn report_save_result(dashboard: &mut Dashboard, result: Result<(), std::io::Error>) {
    if let Err(error) = result {
        eprintln!("gpui-task-manager: 設定を保存できません: {error}");
        dashboard.set_persistent_status(crate::settings::NOTICE_SAVE_FAILED);
    }
}

impl TaskManagerApp {
    /// Creates the application with the requested backend.
    pub fn new(cx: &mut Context<Self>, demo: bool, redact: bool) -> Self {
        // Same policy as the `main.rs` preflight and `--export`: demo data is
        // only ever an explicit `--demo` opt-in. A failed native probe is an
        // error state with no probe and a persistent shared failure message;
        // it must never silently become a demo session (R3-S02 / R3-S02B).
        let backend = BackendInit::resolve(demo, tm_platform::NativeProbe::new);
        let demo_mode = backend.is_demo();
        let startup_status = backend.startup_status().map(str::to_owned);
        let platform = match &backend {
            BackendInit::Demo => PlatformInfo::demo(),
            BackendInit::Native(probe) => probe.platform_info(),
            // Read-only and non-demo: no data, no process control, no DEMO badge.
            BackendInit::Failed(_) => PlatformInfo::native_read_only("収集不可"),
        };
        let probe: Option<Box<dyn SystemProbe>> = match backend {
            BackendInit::Demo => Some(Box::new(DemoProbe::new())),
            BackendInit::Native(probe) => Some(Box::new(probe)),
            BackendInit::Failed(_) => None,
        };
        let mut dashboard = Dashboard::new(platform, DEFAULT_REFRESH_INTERVAL);
        let had_startup_error = startup_status.is_some();
        if let Some(message) = startup_status {
            // Startup failures must stay visible until the user acts.
            dashboard.set_persistent_status(message);
        }
        let (settings, notice) = crate::settings::load_with_notice();
        // A startup failure takes precedence; otherwise surface a settings
        // load/backup notice as a persistent status message.
        if !had_startup_error {
            if let Some(notice) = notice {
                dashboard.set_persistent_status(notice.text);
            }
        }
        for command in [
            Command::SetInterval(Duration::from_millis(settings.interval_ms)),
            Command::SetSortSpec(settings.sort),
            Command::SetDetailsVisible(settings.details_visible),
            Command::SetTreeView(settings.tree_view),
        ] {
            // Restoring settings only changes display state; these commands
            // return no effects today. Check explicitly so a future effect is
            // not silently dropped (R2-C01). The web shell does the same.
            for effect in dashboard.apply(command) {
                match effect {
                    Effect::SampleNow => {}
                    Effect::Signal { .. } => {}
                }
            }
        }
        // Apply persisted theme.
        crate::theme::set_theme(settings.theme);
        Self {
            dashboard,
            probe: Arc::new(Mutex::new(probe)),
            sampling_in_flight: false,
            filter_editing: false,
            filter_input: FilterInput::new(),
            show_help: false,
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            sort_header_focus: std::array::from_fn(|_| cx.focus_handle()),
            row_focus: Vec::new(),
            dialog_cancel_focus: cx.focus_handle(),
            dialog_confirm_focus: cx.focus_handle(),
            help_close_focus: cx.focus_handle(),
            last_overlay: OverlayKind::None,
            focus_before_overlay: None,
            demo_mode,
            redact,
            probe_unavailable: had_startup_error,
        }
    }

    /// Focus handle used by the window and by keyboard shortcuts.
    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    /// Tab / Shift+Tab cycling inside the confirmation dialog.
    ///
    /// This GPUI backend (gpui-pre 0.3) has no built-in modal focus scope and no
    /// default Tab keybinding, so a framework-level trap is not available. With
    /// exactly two buttons, toggling between these two handles keeps focus in
    /// the dialog. If more focusable controls are added to the dialog, the
    /// `DialogButton` order above must be extended as well.
    fn cycle_dialog_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = if window
            .focused(cx)
            .map(|handle| handle == self.dialog_confirm_focus)
            .unwrap_or(false)
        {
            DialogButton::Confirm
        } else {
            DialogButton::Cancel
        };
        let next = match current.next() {
            DialogButton::Cancel => &self.dialog_cancel_focus,
            DialogButton::Confirm => &self.dialog_confirm_focus,
        };
        next.focus(window, cx);
        cx.stop_propagation();
    }

    /// Starts the sampling loop and requests the first sample.
    pub fn start(&mut self, cx: &mut Context<Self>) {
        self.start_timer(cx);
        self.handle_command(Command::Tick, cx);
        self.spawn_ime_debug_hook(cx);
        self.debug_save_settings_once();
    }

    /// Leaf-only label for the opt-in settings debug hook.
    ///
    /// The absolute config path contains the home directory and account name;
    /// emitting it would bypass `--redact` when a user shares debug stderr.
    fn config_file_label(path: Option<std::path::PathBuf>) -> String {
        path.and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "<unknown>".to_string())
    }

    /// デバッグ専用: 設定の保存経路を GUI 操作なしで確認するフック。
    /// `TM_DEBUG_SETTINGS=1` のときだけ動き、通常起動では無効。
    fn debug_save_settings_once(&self) {
        if std::env::var("TM_DEBUG_SETTINGS").as_deref() != Ok("1") {
            return;
        }
        let settings = Settings {
            version: crate::settings::CURRENT_VERSION,
            interval_ms: self.dashboard.interval().as_millis() as u64,
            sort: self.dashboard.sort_spec(),
            details_visible: self.dashboard.details_visible(),
            tree_view: self.dashboard.tree_view(),
            theme: crate::theme::theme(),
        };
        match crate::settings::save(&settings) {
            Ok(()) => {
                // Opt-in debug hook: emit only the leaf file name, never the
                // absolute path (which would disclose the home/account name).
                let config_leaf = Self::config_file_label(crate::settings::config_path());
                eprintln!(
                    "TM_DEBUG_SETTINGS: saved interval_ms={} key={:?} direction={:?} details={} file={}",
                    settings.interval_ms,
                    settings.sort.key,
                    settings.sort.direction,
                    settings.details_visible,
                    config_leaf
                );
            }
            Err(error) => eprintln!("TM_DEBUG_SETTINGS: save failed: {error}"),
        }
    }

    fn start_timer(&self, cx: &mut Context<Self>) {
        let background = cx.background_executor().clone();
        cx.spawn(async move |this, cx| loop {
            // Adaptive sampling may raise the configured cadence on battery or
            // low power (R1-I11). Read it every cycle so the floor follows the
            // newest ingested snapshot without restarting the timer.
            let interval = match this.update(cx, |this, _| this.dashboard.effective_interval()) {
                Ok(interval) => interval,
                Err(_) => break,
            };
            background.timer(interval).await;
            if this
                .update(cx, |this, cx| this.handle_command(Command::Tick, cx))
                .is_err()
            {
                break;
            }
        })
        .detach();
    }

    fn request_sample(&mut self, cx: &mut Context<Self>) {
        // With no probe there is nothing to sample. Keep the persistent startup
        // error visible instead of replacing it with a transient
        // "probe is not available" status on every tick (R3-S02B).
        if self.probe_unavailable || self.sampling_in_flight || self.dashboard.paused() {
            return;
        }
        self.sampling_in_flight = true;
        let probe = self.probe.clone();
        let background = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let result = background
                .spawn(async move {
                    let mut guard = probe
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    match guard.as_mut() {
                        Some(probe) => probe.sample(),
                        None => Err(tm_core::ProbeError::Io(
                            "probe is not available".to_string(),
                        )),
                    }
                })
                .await;
            if this
                .update(cx, |this, cx| {
                    this.sampling_in_flight = false;
                    match result {
                        Ok(snapshot) => {
                            let before = this.dashboard.selected_key();
                            this.dashboard.ingest(snapshot);
                            // If ingest had to move the selection (e.g. the
                            // previously selected process disappeared), keep
                            // it on screen. Do not scroll on every sample.
                            if this.dashboard.selected_key() != before {
                                if let Some(position) = this.dashboard.selected_visible_index() {
                                    this.scroll_handle
                                        .scroll_to_item(position, ScrollStrategy::Nearest);
                                }
                            }
                        }
                        Err(error) => this.dashboard.set_status(error.to_string(), true),
                    }
                    cx.notify();
                })
                .is_err()
            {
                // The view was dropped while sampling was in flight; nothing
                // left to do in this terminal callback.
            }
        })
        .detach();
    }

    /// Re-attempts the native probe after a startup failure (R3-S02B).
    ///
    /// The retry reuses the startup resolution path through
    /// [`resolve_probe_retry`]. On success the recovered probe replaces the
    /// empty `SharedProbe`, the matching persistent failure status is cleared
    /// and sampling resumes on the next tick. A still-failing probe keeps the
    /// failure visible and sampling suppressed, so no synthetic data and no
    /// transient "probe is not available" message can leak into the UI.
    fn retry_probe(&mut self, cx: &mut Context<Self>) {
        if !self.probe_unavailable {
            return;
        }
        let Some(failure) = self.dashboard.status().map(|status| status.text.clone()) else {
            // No startup failure to recover from; nothing to retry.
            return;
        };
        let outcome = resolve_probe_retry(self.demo_mode, tm_platform::NativeProbe::new);
        apply_retry_status(&mut self.dashboard, &outcome, &failure);
        if let ProbeRetry::Recovered(probe) = outcome {
            *self
                .probe
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Box::new(probe));
            self.probe_unavailable = false;
            // `request_sample` still honours `paused`; the guard is unchanged.
            self.request_sample(cx);
        }
        cx.notify();
    }

    fn send_signal(&mut self, key: ProcessKey, signal: Signal, cx: &mut Context<Self>) {
        let probe = self.probe.clone();
        let background = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let result = background
                .spawn(async move {
                    let mut guard = probe
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    match guard.as_mut() {
                        Some(probe) => probe.control(key, signal),
                        None => Err(tm_core::ControlError::Unsupported),
                    }
                })
                .await;
            if this
                .update(cx, |this, cx| {
                    match result {
                        Ok(()) => this.dashboard.set_status(
                            format!("{} を送信しました（PID {}）", signal.short_label(), key.pid),
                            false,
                        ),
                        Err(error) => this.dashboard.set_status(error.to_string(), true),
                    }
                    this.request_sample(cx);
                    cx.notify();
                })
                .is_err()
            {
                // The view was dropped while the signal was in flight; nothing
                // left to do in this terminal callback.
            }
        })
        .detach();
    }

    fn execute_effects(&mut self, effects: Vec<Effect>, cx: &mut Context<Self>) {
        for effect in effects {
            match effect {
                Effect::SampleNow => self.request_sample(cx),
                Effect::Signal { key, signal } => self.send_signal(key, signal, cx),
            }
        }
    }

    fn handle_command(&mut self, command: Command, cx: &mut Context<Self>) {
        // Only presentation preferences are persisted; sampling ticks and
        // process actions must not touch the disk.
        let persist = matches!(
            &command,
            Command::SetInterval(_)
                | Command::SetSortKey(_)
                | Command::SetSortSpec(_)
                | Command::ToggleDetails
                | Command::SetDetailsVisible(_)
                | Command::ToggleTreeView
                | Command::SetTreeView(_)
        );
        let effects = self.dashboard.apply(command);
        self.execute_effects(effects, cx);
        if let Some(position) = self.dashboard.selected_visible_index() {
            self.scroll_handle
                .scroll_to_item(position, ScrollStrategy::Nearest);
        }
        if persist {
            self.save_settings();
        }
        cx.notify();
    }

    /// Persists the current presentation preferences.
    fn save_settings(&mut self) {
        let settings = Settings {
            version: crate::settings::CURRENT_VERSION,
            interval_ms: self.dashboard.interval().as_millis() as u64,
            sort: self.dashboard.sort_spec(),
            details_visible: self.dashboard.details_visible(),
            tree_view: self.dashboard.tree_view(),
            theme: crate::theme::theme(),
        };
        report_save_result(&mut self.dashboard, crate::settings::save(&settings));
    }

    /// 絞り込み編集を開始する。現在の確定文字列を入力欄へ引き継ぐ。
    fn begin_filter_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.filter_editing {
            self.filter_input.clear();
            let current = self.dashboard.filter_text().to_string();
            if !current.is_empty() {
                self.filter_input.replace_text(None, &current);
            }
            self.filter_editing = true;
        }
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// 絞り込み編集を終了する。確定文字列は Dashboard に残したままにする。
    fn end_filter_editing(&mut self, cx: &mut Context<Self>) {
        self.filter_editing = false;
        self.filter_input.unmark();
        cx.notify();
    }

    /// 絞り込み欄を空にして、一覧の絞り込みも解除する。
    fn clear_filter(&mut self, cx: &mut Context<Self>) {
        self.filter_input.clear();
        self.handle_command(Command::ClearFilter, cx);
    }

    /// [`EntityInputHandler`] からの編集結果を表示と Dashboard に反映する。
    ///
    /// IME 変換中（[`EditOutcome::Composing`]）は表示だけ更新し、確定文字列を
    /// フィルタへ渡さない。確定・削除・貼り付けのときだけ
    /// [`Command::SetFilterText`] / [`Command::ClearFilter`] を発行する。
    fn apply_input_outcome(&mut self, outcome: EditOutcome, cx: &mut Context<Self>) {
        if outcome == EditOutcome::Committed {
            let committed = self.filter_input.committed_text();
            let command = if committed.is_empty() {
                Command::ClearFilter
            } else {
                Command::SetFilterText(committed)
            };
            self.handle_command(command, cx);
        } else if outcome == EditOutcome::Composing {
            cx.notify();
        }
    }

    /// デバッグ専用: 実 IME を注入できない環境で composition の表示と確定反映を
    /// 確認するためのフック。`TM_DEBUG_IME=1` のときだけ動き、通常起動では無効。
    ///
    /// 実際のプラットフォーム IME 経路（Wayland text-input-v3 / X11 XIM）は
    /// 迂回し、[`FilterInput`] の状態遷移と Dashboard 反映だけを再現する。
    fn spawn_ime_debug_hook(&self, cx: &mut Context<Self>) {
        if std::env::var("TM_DEBUG_IME").as_deref() != Ok("1") {
            return;
        }
        let background = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            // (表示テキスト, 待ち時間 ms)。3 ステップ目で確定する。
            let steps = [("にほんご", 2_500u64), ("日本語", 4_000), ("", 4_000)];
            for (text, delay) in steps {
                background.timer(Duration::from_millis(delay)).await;
                let alive = this.update(cx, |this, cx| {
                    if text.is_empty() {
                        let outcome = this.filter_input.replace_text(None, "日本語");
                        this.apply_input_outcome(outcome, cx);
                        eprintln!(
                            "TM_DEBUG_IME: committed filter={:?} composing={} field={:?}",
                            this.dashboard.filter_text(),
                            this.filter_input.is_composing(),
                            this.filter_input.text()
                        );
                    } else {
                        if !this.filter_editing {
                            this.filter_editing = true;
                            this.filter_input.clear();
                        }
                        let outcome = this.filter_input.replace_and_mark(None, text, None);
                        this.apply_input_outcome(outcome, cx);
                        eprintln!(
                            "TM_DEBUG_IME: composing field={:?} marked={:?} filter={:?}",
                            this.filter_input.text(),
                            this.filter_input.marked_range(),
                            this.dashboard.filter_text()
                        );
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.to_lowercase();
        let modifiers = event.keystroke.modifiers;

        // Confirmation dialog first: its key contract is deliberately narrow and
        // safety-critical (a bare Enter always cancels). Keep this path ahead of
        // text editing so an Enter pressed with the dialog open never edits the
        // filter instead.
        if self.dashboard.pending_kill().is_some() {
            // Tab / Shift+Tab keep keyboard focus inside the modal by cycling
            // between キャンセル and 実行 (see `cycle_dialog_focus`).
            if key == "tab" {
                self.cycle_dialog_focus(window, cx);
                return;
            }
            if let Some(command) = dialog_command(&key, modifiers) {
                self.handle_command(command, cx);
            }
            return;
        }

        // WCAG 2.1.1: a focused column header owns Enter/Space for sorting.
        // GPUI's focusable Div fires the existing on_click (SetSortKey) on the
        // key-up, so this handler only keeps the key away from Space=pause and
        // from filter editing; propagation is stopped on key-down to suppress
        // platform defaults without cancelling that built-in activation.
        if is_sort_key_activation(&key, modifiers, self.filter_editing)
            && self
                .sort_header_focus
                .iter()
                .any(|handle| handle.is_focused(window))
        {
            cx.stop_propagation();
            return;
        }

        // Help overlay keeps its original priority over filter editing: a bare
        // Esc/F1 always closes it. Modified keys are ignored here as well, so
        // the overlay cannot be dismissed by an IME/shortcut gesture.
        if self.show_help {
            let unmodified = !modifiers.control
                && !modifiers.platform
                && !modifiers.alt
                && !modifiers.shift
                && !modifiers.function;
            // Tab / Shift+Tab re-focus the single "閉じる" control so keyboard
            // focus cannot leave the help overlay.
            if key == "tab" {
                self.help_close_focus.focus(window, cx);
                cx.stop_propagation();
                return;
            }
            if unmodified && (key == "escape" || key == "f1") {
                self.show_help = false;
                cx.notify();
            }
            return;
        }

        // A focused process row owns Enter/Space for its existing `on_click`
        // (ToggleAppGroup on a group row, Select otherwise), exactly like a
        // focused column header. Consume the key here so the global Space=pause
        // and filter-edit paths never fire underneath row activation.
        if is_row_activation(&key, modifiers, self.filter_editing)
            && self
                .row_focus
                .iter()
                .take(self.dashboard.visible_count())
                .any(|handle| handle.is_focused(window))
        {
            cx.stop_propagation();
            return;
        }

        // Single resolution point for every other keystroke. `key_action`
        // suppresses navigation and shell shortcuts while the filter is being
        // edited (IME candidate selection included), which is the native half
        // of the cross-shell keyboard contract.
        let Some(action) = key_action(&key, modifiers, self.filter_editing) else {
            // 文字入力・貼り付け・IME 変換は EntityInputHandler が受ける。
            // ここで文字を足すと二重挿入になるので何もしない
            // （stop_propagation するとプラットフォーム側の挿入も止まる）。
            return;
        };

        match action {
            // Esc: 絞り込みを解除して編集モードを抜ける。
            KeyAction::CancelFilter => {
                self.filter_editing = false;
                self.clear_filter(cx);
                cx.stop_propagation();
            }
            // Enter: IME の確定はプラットフォーム側が行う。編集モードを抜ける。
            KeyAction::CommitFilter => {
                self.end_filter_editing(cx);
                cx.stop_propagation();
            }
            // Backspace はネイティブではプラットフォームが処理しないため、
            // ここで入力欄の状態を更新する（Web はブラウザ既定動作が担当）。
            KeyAction::BackspaceFilter => {
                let outcome = self.filter_input.backspace();
                self.apply_input_outcome(outcome, cx);
                cx.stop_propagation();
            }
            KeyAction::BeginFilter => self.begin_filter_editing(window, cx),
            KeyAction::RequestTerminate => {
                // Protection (PID 1 / self) is decided by the view model, so the
                // shell must always forward the request: that way the user gets
                // the same "protected" message as the button path instead of a
                // silently ignored shortcut.
                if let Some(process) = self.dashboard.selected_process() {
                    self.handle_command(
                        Command::RequestSignal {
                            key: process.key,
                            signal: Signal::Terminate,
                        },
                        cx,
                    );
                }
            }
            KeyAction::SelectRelative(delta) => {
                // Arrow navigation keeps the selected row and the keyboard
                // focus in sync when a row was focused, so a following Enter or
                // Space activates the row the user just moved to.
                let row_focused = self
                    .row_focus
                    .iter()
                    .take(self.dashboard.visible_count())
                    .any(|handle| handle.is_focused(window));
                self.handle_command(Command::SelectRelative(delta), cx);
                if row_focused {
                    if let Some(index) = self.dashboard.selected_visible_index() {
                        if let Some(handle) = self.row_focus.get(index) {
                            handle.focus(window, cx);
                        }
                    }
                }
            }
            KeyAction::TogglePaused => {
                let paused = !self.dashboard.paused();
                self.handle_command(Command::SetPaused(paused), cx);
            }
            KeyAction::ToggleDetails => self.handle_command(Command::ToggleDetails, cx),
            KeyAction::Refresh => self.handle_command(Command::Tick, cx),
            KeyAction::ShowHelp => {
                self.show_help = true;
                cx.notify();
            }
            KeyAction::ClearFilter => {
                if self.dashboard.filter().is_active() {
                    self.handle_command(Command::ClearFilter, cx);
                }
            }
        }
    }

    /// Returns a display string with local identifiers redacted when
    /// `--redact` / `TM_REDACT=1` is active (used for public screenshots).
    fn display(&self, value: &str) -> String {
        if self.redact {
            let hostname = self
                .dashboard
                .snapshot()
                .and_then(|snapshot| snapshot.hostname.clone())
                .unwrap_or_default();
            redact_text(value, &hostname)
        } else {
            value.to_string()
        }
    }

    fn summary(&self) -> Summary {
        let snapshot = self.dashboard.snapshot();
        let cpu_history = self
            .dashboard
            .history()
            .cpu_percent
            .iter()
            .copied()
            .collect();
        let mem_history = self
            .dashboard
            .history()
            .memory_percent
            .iter()
            .copied()
            .collect();
        let rx_history = self
            .dashboard
            .history()
            .network_rx_rate
            .iter()
            .map(|&value| value as f32)
            .collect();
        let tx_history = self
            .dashboard
            .history()
            .network_tx_rate
            .iter()
            .map(|&value| value as f32)
            .collect();
        Summary {
            cpu_total: snapshot.map(|s| s.cpu.total_percent).unwrap_or(0.0),
            cores: snapshot
                .map(|s| s.cpu.per_core_percent.clone())
                .unwrap_or_default(),
            load: snapshot.map(|s| s.cpu.load_average).unwrap_or([0.0; 3]),
            uptime_secs: snapshot.map(|s| s.cpu.uptime_secs).unwrap_or(0),
            frequency_mhz: snapshot.and_then(|s| s.cpu.frequency_mhz),
            mem_used: snapshot.map(|s| s.memory.used_bytes).unwrap_or(0),
            mem_total: snapshot.map(|s| s.memory.total_bytes).unwrap_or(0),
            mem_percent: snapshot.map(|s| s.memory.used_percent()).unwrap_or(0.0),
            swap_used: snapshot.map(|s| s.memory.swap_used_bytes).unwrap_or(0),
            swap_total: snapshot.map(|s| s.memory.swap_total_bytes).unwrap_or(0),
            swap_percent: snapshot
                .map(|s| s.memory.swap_used_percent())
                .unwrap_or(0.0),
            rx_rate: self.dashboard.network_rates().0,
            tx_rate: self.dashboard.network_rates().1,
            process_count: snapshot.map(|s| s.process_count()).unwrap_or(0),
            visible_count: self.dashboard.visible_count(),
            hostname: snapshot
                .and_then(|s| s.hostname.clone())
                .map(|host| self.display(&host))
                .unwrap_or_else(|| "—".to_string()),
            kernel: snapshot
                .and_then(|s| s.kernel_version.clone())
                .map(|kernel| self.display(&kernel))
                .unwrap_or_else(|| "—".to_string()),
            sampled_at: snapshot.map(|s| s.sampled_at),
            paused: self.dashboard.paused(),
            interval_ms: self.dashboard.interval().as_millis() as u64,
            disks: snapshot
                .map(|s| {
                    s.disks
                        .iter()
                        .take(2)
                        .map(|disk| DiskRow {
                            mount: self.display(&disk.mount_point),
                            percent: disk.used_percent(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            cpu_history,
            mem_history,
            rx_history,
            tx_history,
        }
    }

    fn top_bar(&mut self, cx: &mut Context<Self>) -> Div {
        let summary = self.summary();
        let paused = summary.paused;
        let pause_label = if paused { "再開" } else { "一時停止" };
        let pause_color = if paused {
            palette().green
        } else {
            palette().blue
        };
        h_flex()
            .items_center()
            .justify_between()
            .px_4()
            .py_2()
            .bg(rgb(palette().mantle))
            .border_b_1()
            .border_color(rgb(palette().surface))
            .child(
                h_flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(palette().text))
                            .child("GPUI Task Manager"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette().muted))
                            .child(format!("{} · {}", summary.hostname, summary.kernel)),
                    )
                    .when(self.demo_mode, |this| {
                        this.child(pill("デモデータ", palette().base, palette().yellow))
                    }),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(action_button(
                        "pause-toggle",
                        pause_label,
                        pause_color,
                        true,
                        cx.listener(|this, _event, window, cx| {
                            this.focus_handle.focus(window, cx);
                            let paused = !this.dashboard.paused();
                            this.handle_command(Command::SetPaused(paused), cx);
                        }),
                    ))
                    .child(action_button(
                        "toggle-details",
                        "詳細",
                        palette().surface_hi,
                        true,
                        cx.listener(|this, _event, window, cx| {
                            this.focus_handle.focus(window, cx);
                            this.handle_command(Command::ToggleDetails, cx);
                        }),
                    ))
                    .child(action_button(
                        "show-help",
                        "ヘルプ",
                        palette().surface_hi,
                        true,
                        cx.listener(|this, _event, window, cx| {
                            this.focus_handle.focus(window, cx);
                            this.show_help = true;
                            cx.notify();
                        }),
                    ))
                    .child(action_button(
                        "toggle-theme",
                        if crate::theme::theme() == tm_viewmodel::Theme::Dark {
                            "ライト"
                        } else {
                            "ダーク"
                        },
                        palette().surface_hi,
                        true,
                        cx.listener(|this, _event, window, cx| {
                            this.focus_handle.focus(window, cx);
                            let new_theme = match crate::theme::theme() {
                                tm_viewmodel::Theme::Dark => tm_viewmodel::Theme::Light,
                                tm_viewmodel::Theme::Light => tm_viewmodel::Theme::Dark,
                            };
                            crate::theme::set_theme(new_theme);
                            this.save_settings();
                            cx.notify();
                        }),
                    )),
            )
    }

    fn overview(&self) -> Div {
        let summary = self.summary();
        let max_rate = summary
            .rx_history
            .iter()
            .chain(summary.tx_history.iter())
            .copied()
            .fold(1_024.0f32, f32::max);
        let frequency = summary
            .frequency_mhz
            .map(|mhz| format!(" / {:.2} GHz", mhz as f32 / 1_000.0))
            .unwrap_or_default();

        let cpu_card = card(
            "CPU",
            palette().blue,
            v_flex()
                .gap_2()
                .child(
                    h_flex()
                        .items_end()
                        .justify_between()
                        .child(
                            div()
                                .text_xl()
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(usage_color(summary.cpu_total)))
                                .child(format::format_percent(summary.cpu_total)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette().muted))
                                .child(format!("{} コア{}", summary.cores.len(), frequency)),
                        ),
                )
                .child(sparkline(&summary.cpu_history, 100.0, palette().blue, 34.0))
                .child(
                    h_flex().items_end().gap(px(2.0)).h(px(16.0)).children(
                        summary
                            .cores
                            .iter()
                            .map(|&percent| {
                                div()
                                    .flex_1()
                                    .h(px(((percent / 100.0) * 16.0).max(1.0)))
                                    .rounded_sm()
                                    .bg(rgb(usage_color(percent)))
                            })
                            .collect::<Vec<_>>(),
                    ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette().muted))
                        .child(format!(
                            "負荷 {}",
                            format::format_load_average(summary.load)
                        )),
                ),
        )
        .h(px(200.0))
        .flex_1()
        .min_w_0()
        // The big number is colour-coded; name the gauge in AccessKit so AT-SPI
        // exposes a reader-meaningful group with the same severity in text.
        .id("overview-cpu")
        .role(Role::Group)
        .aria_label(a11y_usage_label("CPU 使用率", summary.cpu_total));

        let memory_card = card(
            "メモリ",
            palette().mauve,
            v_flex()
                .gap_2()
                .child(
                    h_flex()
                        .items_end()
                        .justify_between()
                        .child(
                            div()
                                .text_xl()
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(usage_color(summary.mem_percent)))
                                .child(format::format_percent(summary.mem_percent)),
                        )
                        .child(
                            v_flex()
                                .items_end()
                                .child(div().text_xs().text_color(rgb(palette().muted)).child(
                                    format::format_used_of_total(
                                        summary.mem_used,
                                        summary.mem_total,
                                    ),
                                ))
                                // Synthetic values are fixed at 16 GiB; make
                                // that explicit next to the number so it is
                                // never mistaken for a 64 GB machine's RAM.
                                .when(self.demo_mode, |this| {
                                    this.child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(palette().yellow))
                                            .child("※ デモ用の固定値"),
                                    )
                                }),
                        ),
                )
                .child(accessible_metric_bar(
                    "memory-usage-bar",
                    summary.mem_percent,
                    usage_color(summary.mem_percent),
                    a11y_usage_label("メモリ使用率", summary.mem_percent),
                ))
                .child(
                    h_flex()
                        .justify_between()
                        .text_xs()
                        .text_color(rgb(palette().muted))
                        .child("スワップ")
                        .child(format::format_used_of_total(
                            summary.swap_used,
                            summary.swap_total,
                        )),
                )
                .child(accessible_metric_bar(
                    "swap-usage-bar",
                    summary.swap_percent,
                    palette().peach,
                    a11y_usage_label("スワップ使用率", summary.swap_percent),
                ))
                .child(sparkline(
                    &summary.mem_history,
                    100.0,
                    palette().mauve,
                    22.0,
                ))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette().muted))
                        .child(format!(
                            "空き {}",
                            format::format_bytes(
                                summary.mem_total.saturating_sub(summary.mem_used)
                            )
                        )),
                ),
        )
        .h(px(200.0))
        .flex_1()
        .min_w_0()
        .id("overview-memory")
        .role(Role::Group)
        .aria_label(a11y_usage_label("メモリ使用率", summary.mem_percent));

        let network_card = card(
            "ネットワーク",
            palette().teal,
            v_flex()
                .gap_2()
                .child(
                    h_flex()
                        .items_end()
                        .justify_between()
                        .child(
                            div()
                                .text_xl()
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(palette().teal))
                                .child(format::format_rate(summary.rx_rate)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette().muted))
                                .child("受信"),
                        ),
                )
                .child(sparkline(
                    &summary.rx_history,
                    max_rate,
                    palette().teal,
                    22.0,
                ))
                .child(
                    h_flex()
                        .items_end()
                        .justify_between()
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(palette().peach))
                                .child(format::format_rate(summary.tx_rate)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette().muted))
                                .child("送信"),
                        ),
                )
                .child(sparkline(
                    &summary.tx_history,
                    max_rate,
                    palette().peach,
                    14.0,
                )),
        )
        .h(px(200.0))
        .flex_1()
        .min_w_0()
        .id("overview-network")
        .role(Role::Group)
        .aria_label(format!(
            "ネットワーク、受信 {}、送信 {}",
            format::format_rate(summary.rx_rate),
            format::format_rate(summary.tx_rate)
        ));

        let disk_rows: Vec<Div> = summary
            .disks
            .iter()
            .enumerate()
            .map(|(index, disk)| {
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .justify_between()
                            .text_xs()
                            .text_color(rgb(palette().muted))
                            .child(disk.mount.clone())
                            .child(format!("{:.0}%", disk.percent)),
                    )
                    .child(accessible_metric_bar(
                        ("disk-usage-bar", index),
                        disk.percent,
                        usage_color(disk.percent),
                        a11y_usage_label(&format!("ディスク {} 使用率", disk.mount), disk.percent),
                    ))
            })
            .collect();

        let mut system_body = v_flex()
            .gap_2()
            .child(kv("ホスト", summary.hostname.clone()))
            .child(kv("稼働時間", format::format_duration(summary.uptime_secs)))
            .child(kv("プロセス", format::format_count(summary.process_count)))
            .children(disk_rows);
        let components = self
            .dashboard
            .snapshot()
            .map(|s| s.components.as_slice())
            .unwrap_or(&[]);
        if components.is_empty() {
            system_body = system_body.child(kv("温度", "センサーなし".to_string()));
        } else {
            for (index, component) in components.iter().take(3).enumerate() {
                let label = if component.label.is_empty() {
                    "センサー".to_string()
                } else {
                    self.display(&component.label)
                };
                let value = match (
                    component.temperature_c,
                    component.max_c,
                    component.critical_c,
                ) {
                    (Some(t), Some(max), Some(crit)) => {
                        format!("{t:.0}°C (上限 {max:.0}°C / 危険しきい値 {crit:.0}°C)")
                    }
                    (Some(t), Some(max), None) => format!("{t:.0}°C (max {max:.0}°C)"),
                    (Some(t), None, Some(crit)) => format!("{t:.0}°C (危険しきい値 {crit:.0}°C)"),
                    (Some(t), None, None) => format!("{t:.0}°C"),
                    _ => "—".to_string(),
                };
                // The temperature state is otherwise colour-only; expose the
                // value and 正常/注意/危険/不明 wording in the AccessKit name.
                let aria_label = a11y_temperature_label(
                    &label,
                    &value,
                    component.temperature_c,
                    component.max_c,
                    component.critical_c,
                );
                system_body = system_body.child(
                    kv(&label, value)
                        .id(("temperature-gauge", index))
                        .role(Role::Group)
                        .aria_label(aria_label),
                );
            }
        }

        let system_card = card("システム", palette().green, system_body)
            .h(px(200.0))
            .flex_1()
            .min_w_0()
            .id("overview-system")
            .role(Role::Group)
            .aria_label(format!(
                "システム概要、ホスト {}、プロセス {} 件",
                self.display(&summary.hostname),
                summary.process_count
            ));

        h_flex().gap_3().px_4().py_3().items_start().children(vec![
            cpu_card,
            memory_card,
            network_card,
            system_card,
        ])
    }

    fn toolbar(&mut self, cx: &mut Context<Self>) -> Div {
        let summary = self.summary();
        let editing = self.filter_editing;
        let sort = self.dashboard.sort_spec();
        let active_interval = self.dashboard.interval();

        // 表示は「確定前 + 変換中 + 確定後」の 3 分割。IME 変換中の文字列は
        // 青 + 下線で見せ、Dashboard の絞り込みにはまだ渡さない。
        // 編集中でなければ Dashboard の確定文字列をそのまま表示する。
        let (before, composing, after) = if editing {
            let (before, composing, after) = self.filter_input.display_parts();
            (before.to_string(), composing.to_string(), after.to_string())
        } else {
            (
                self.dashboard.filter_text().to_string(),
                String::new(),
                String::new(),
            )
        };
        let composing_active = !composing.is_empty();
        let field_is_empty = before.is_empty() && composing.is_empty() && after.is_empty();

        let mut field_children: Vec<Div> = Vec::new();
        if field_is_empty {
            field_children.push(div().child("名前・コマンド・ユーザー・PID"));
        } else {
            if !before.is_empty() {
                field_children.push(div().child(before));
            }
            if composing_active {
                field_children.push(
                    div()
                        .text_color(rgb(palette().blue))
                        .underline()
                        .child(composing),
                );
            }
            if !after.is_empty() {
                field_children.push(div().child(after));
            }
        }

        // IME ハンドラは絞り込み編集中だけ、フィルタボックスの bounds に対して登録する。
        // `Window::handle_input` は paint 中に呼ぶのが GPUI の作法。
        let input_view = cx.entity();
        let input_focus_handle = self.focus_handle.clone();

        // Mirror the browser shell's text-input semantics in AccessKit. The
        // `aria_value` is the *display* (possibly 伏せ字化済み) text, never the
        // raw filter input, so a screen reader cannot announce an identifier
        // that the redacted UI deliberately hides.
        let filter_aria_value = self.display(self.dashboard.filter_text());
        let filter_a11y_hint = a11y_filter_hint(composing_active, editing);
        let filter_box = div()
            .id("filter-box")
            .role(Role::TextInput)
            .aria_label(a11y_filter_label())
            .aria_placeholder("名前・コマンド・ユーザー・PID")
            .aria_value(filter_aria_value)
            .aria_description(filter_a11y_hint)
            .relative()
            .flex()
            .items_center()
            .gap_2()
            .w(px(300.0))
            .h(px(30.0))
            .px_3()
            .rounded_md()
            .bg(rgb(palette().base))
            .border_1()
            .border_color(rgb(if editing {
                palette().blue
            } else {
                palette().surface
            }))
            .cursor_pointer()
            .on_click(cx.listener(|this, _event, window, cx| {
                this.begin_filter_editing(window, cx);
            }))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette().muted))
                    .child("検索"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(rgb(if field_is_empty {
                        palette().dim
                    } else {
                        palette().text
                    }))
                    .truncate()
                    .children(field_children),
            )
            .when(editing, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette().blue))
                        .child(if composing_active { "変換中" } else { "▌" }),
                )
                .child(
                    canvas(
                        |bounds, _window, _cx| bounds,
                        move |bounds, _prepaint, window, cx| {
                            window.handle_input(
                                &input_focus_handle,
                                ElementInputHandler::new(bounds, input_view.clone()),
                                cx,
                            );
                        },
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
            });

        let clear_button = action_button(
            "filter-clear",
            "クリア",
            palette().surface_hi,
            self.dashboard.filter().is_active(),
            cx.listener(|this, _event, window, cx| {
                this.focus_handle.focus(window, cx);
                this.filter_editing = false;
                this.filter_input.clear();
                this.handle_command(Command::ClearFilter, cx);
            }),
        );

        let mut interval_buttons = Vec::new();
        for (millis, label) in [
            (500u64, "0.5s"),
            (1_000, "1s"),
            (2_000, "2s"),
            (5_000, "5s"),
        ] {
            let active = active_interval == Duration::from_millis(millis);
            interval_buttons.push(action_button(
                format!("interval-{millis}"),
                label,
                if active {
                    palette().blue
                } else {
                    palette().surface_hi
                },
                true,
                cx.listener(move |this, _event, window, cx| {
                    this.focus_handle.focus(window, cx);
                    this.handle_command(Command::SetInterval(Duration::from_millis(millis)), cx);
                }),
            ));
        }

        h_flex()
            .items_center()
            .gap_3()
            .px_4()
            .py_2()
            .bg(rgb(palette().mantle))
            .border_b_1()
            .border_color(rgb(palette().surface))
            .child(filter_box)
            .child(clear_button)
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette().muted))
                    .child(format!(
                        "並び: {} {}",
                        sort.key.label(),
                        sort.direction.arrow()
                    )),
            )
            .child(h_flex().items_center().gap_1().children(interval_buttons))
            .child(div().flex_1())
            .child(
                div()
                    .id("toggle-tree")
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_xs()
                    .cursor_pointer()
                    .border_1()
                    .border_color(rgb(palette().surface))
                    .bg(rgb(if self.dashboard.tree_view() {
                        palette().surface
                    } else {
                        palette().mantle
                    }))
                    .text_color(rgb(if self.dashboard.tree_view() {
                        palette().blue
                    } else {
                        palette().muted
                    }))
                    .hover(|style| {
                        style
                            .bg(rgb(palette().surface))
                            .text_color(rgb(palette().text))
                    })
                    .on_click(cx.listener(|this, _event, window, cx| {
                        this.focus_handle.focus(window, cx);
                        this.handle_command(Command::ToggleTreeView, cx);
                    }))
                    .child(if self.dashboard.tree_view() {
                        "ツリー: ON"
                    } else {
                        "ツリー"
                    }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette().muted))
                    .child(format!(
                        "表示 {} / 全 {} 件",
                        summary.visible_count, summary.process_count
                    )),
            )
    }

    fn table_header(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let sort = self.dashboard.sort_spec();
        let columns: [(&'static str, SortKey, Option<f32>); 7] = [
            ("名前", SortKey::Name, None),
            ("CPU", SortKey::Cpu, Some(COL_CPU)),
            ("メモリ", SortKey::Memory, Some(COL_MEMORY)),
            ("PID", SortKey::Pid, Some(COL_PID)),
            ("ユーザー", SortKey::User, Some(COL_USER)),
            ("状態", SortKey::Status, Some(COL_STATUS)),
            ("スレッド", SortKey::Threads, Some(COL_THREADS)),
        ];
        let mut headers = Vec::new();
        for (index, (label, key, width)) in columns.iter().enumerate() {
            let active = sort.key == *key;
            let text = sort_header_text(label, active, sort.direction);
            let key = *key;
            let header_focus = self.sort_header_focus[index].clone();
            // AccessKit role/name for the column header. The browser shell
            // exposes the same `ColumnHeader` role; a bare ↑/↓ arrow carries no
            // text, so the direction is spelled out in Japanese in aria_label.
            let mut cell = div()
                .id(("header", index))
                .role(Role::ColumnHeader)
                .aria_label(sort_header_label(label, active, sort.direction))
                // Mouse clicks keep the old path; track_focus adds Tab access
                // and lets GPUI activate the same on_click on Enter/Space.
                .track_focus(&header_focus)
                .aria_keyshortcuts("Enter, Space")
                // Native equivalent of the browser mirror's `aria-sort`.
                .a11y_synthetic_children(move |builder| {
                    match a11y_sort_accesskit(active, sort.direction) {
                        Some(direction) => builder.parent_node().set_sort_direction(direction),
                        None => builder.parent_node().clear_sort_direction(),
                    }
                })
                .flex()
                .items_center()
                .cursor_pointer()
                .text_xs()
                .text_color(rgb(if active {
                    palette().blue
                } else {
                    palette().muted
                }))
                .hover(|style| style.text_color(rgb(palette().text)))
                .on_click(cx.listener(move |this, _event, window, cx| {
                    // マウスでもキーボードでもソート後に見出しへフォーカスを
                    // 保ち、続けて Enter/Space を押してもソートできるようにする。
                    header_focus.focus(window, cx);
                    this.handle_command(Command::SetSortKey(key), cx);
                }))
                .child(text);
            cell = match width {
                Some(width) => cell.w(px(*width)),
                None => cell.flex_1().min_w(px(200.0)),
            };
            headers.push(cell);
        }
        h_flex()
            .id("table-header")
            .role(Role::RowGroup)
            .aria_label("プロセス一覧の列見出し")
            .w_full()
            .h(px(30.0))
            .px_3()
            .gap_2()
            .items_center()
            .bg(rgb(palette().mantle))
            .border_b_1()
            .border_color(rgb(palette().surface))
            .children(headers)
    }

    fn process_list(&mut self, cx: &mut Context<Self>) -> Div {
        let count = self.dashboard.visible_count();
        let list = uniform_list(
            "process-list",
            count,
            cx.processor(
                move |this: &mut Self, range: std::ops::Range<usize>, _window, cx| {
                    range
                        .filter_map(|index| this.process_row(index, cx))
                        .collect::<Vec<_>>()
                },
            ),
        )
        .h_full()
        .track_scroll(&self.scroll_handle);

        let overlay = if self.probe_unavailable {
            // Reuse the shared failure message from the status bar so the error
            // is explicit and the wording stays aligned with `main.rs`, and
            // offer an in-place retry that re-runs the startup probe path.
            let message = self
                .dashboard
                .status()
                .map(|status| self.display(&status.text));
            Some(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .children(
                        message
                            .map(|text| div().text_sm().text_color(rgb(palette().red)).child(text)),
                    )
                    .child(action_button(
                        "retry-probe",
                        "再試行",
                        palette().surface_hi,
                        true,
                        cx.listener(|this, _event, window, cx| {
                            this.focus_handle.focus(window, cx);
                            this.retry_probe(cx);
                        }),
                    )),
            )
        } else if self.dashboard.snapshot().is_none() {
            Some(overlay_text(
                "システム情報を取得しています…",
                palette().muted,
            ))
        } else if count == 0 {
            Some(overlay_text(
                "一致するプロセスがありません",
                palette().muted,
            ))
        } else {
            None
        };

        div()
            .flex_1()
            .min_h_0()
            .w_full()
            .relative()
            .bg(rgb(palette().base))
            .child(list)
            .children(overlay)
    }

    fn process_row(&mut self, index: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (view, expanded) = {
            let snapshot = self.dashboard.snapshot()?;
            let row_index = *self.dashboard.visible_indices().get(index)?;
            let depth = self
                .dashboard
                .visible_depths()
                .get(index)
                .copied()
                .unwrap_or(0);
            let has_children = self
                .dashboard
                .visible_has_children()
                .get(index)
                .copied()
                .unwrap_or(false);
            let process = snapshot.processes.get(row_index)?;
            // In the grouped view `app_group_rows()` is aligned with
            // `visible_indices()`. Only a Group row takes its values from the
            // aggregate; an expanded member and every flat/tree row keep the
            // original `snapshot.processes[row_index]` values (R1-APPDRAW).
            let group = if self.dashboard.app_group_view() {
                self.dashboard.app_group_rows().get(index).cloned()
            } else {
                None
            };
            let group = group.as_ref().filter(|row| row.kind == AppRowKind::Group);
            let selected = if group.is_some() {
                self.dashboard.selected_app_group_row_index() == Some(index)
            } else {
                self.dashboard.selected_key() == Some(process.key)
            };
            let expanded = group
                .map(|row| row.expanded)
                .unwrap_or_else(|| !self.dashboard.is_collapsed(process.key));
            (
                process_row_values(group, process, selected, depth, has_children),
                expanded,
            )
        };

        // Stable per-index focus handle: GPUI synthesises the `on_click` below
        // on Enter/Space while this row is focused (WCAG 2.1.1), the same path
        // already used by the sortable column headers.
        while self.row_focus.len() <= index {
            self.row_focus.push(cx.focus_handle());
        }
        let row_focus = self.row_focus[index].clone();

        let tree_view = self.dashboard.tree_view();
        let group_view = self.dashboard.app_group_view();
        let key = view.key;
        let group_key = view.group_key.clone();
        let has_children = view.has_children;
        let depth = view.depth;
        let selected = view.selected;
        let status = view.status;
        let status_text = view.status_text.clone();
        let threads = view.threads.clone();
        let cpu = view.cpu_percent;
        let memory = view.memory_bytes;
        let pid = view.pid.clone();
        let process_count = view.process_count;
        let is_group = view.is_group;
        let collapsed = !expanded;
        // All process users collapse to "user" under --redact, matching the
        // agent snapshot (not just the login account) — R41 audit. Group rows
        // already carry the fixed "—" and are left untouched.
        let user = if self.redact && view.user != "—" {
            "user".to_string()
        } else {
            self.display(&view.user)
        };
        let command = self.display(&view.command);
        let name = self.display(&view.name);
        let marker = if is_group && has_children {
            if expanded {
                "▾ "
            } else {
                "▸ "
            }
        } else if tree_view && has_children {
            if collapsed {
                "▸ "
            } else {
                "▾ "
            }
        } else if (tree_view || group_view) && depth > 0 {
            "  "
        } else {
            ""
        };
        let display_name = if depth == 0 {
            format!("{marker}{name}")
        } else {
            format!("{}{}{}", "  ".repeat(depth.saturating_sub(1)), marker, name)
        };
        let aria_label = if is_group {
            a11y_group_row_label(&name, cpu, memory, process_count, selected)
        } else {
            a11y_row_label(&name, status, cpu, memory, key.pid, selected)
        };

        Some(
            div()
                .id(("row", index))
                // Mirror the browser shell's row semantics: role/name plus the
                // selected state and 1-based row index, so a screen reader can
                // describe the row without relying on the colour-only status
                // dot. `name` is the already-displayed (possibly redacted) value.
                .role(Role::Row)
                .aria_label(aria_label)
                .aria_selected(selected)
                .aria_row_index(index + 1)
                // Group rows advertise the same expansion state as tree nodes;
                // both use `has_children` from the visible row arrays.
                .when(has_children, |this| this.aria_expanded(expanded))
                .track_focus(&row_focus)
                .aria_keyshortcuts("Enter, Space")
                .flex()
                .items_center()
                .w_full()
                .h(px(ROW_HEIGHT))
                .px_3()
                .gap_2()
                .cursor_pointer()
                .when(selected, |this| this.bg(rgb(palette().surface)))
                .when(!selected, |this| {
                    this.hover(|style| style.bg(rgb(palette().mantle)))
                })
                .on_click(cx.listener(move |this, _event, window, cx| {
                    row_focus.focus(window, cx);
                    if let Some(group_key) = group_key.clone() {
                        // A group has no process identity of its own: activation
                        // toggles the expansion set instead of selecting a member.
                        this.handle_command(Command::ToggleAppGroup(group_key), cx);
                    } else if tree_view && has_children {
                        this.handle_command(Command::Select(Some(key)), cx);
                        this.handle_command(Command::ToggleTreeNode(key), cx);
                    } else {
                        this.handle_command(Command::Select(Some(key)), cx);
                    }
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(200.0))
                        .flex()
                        .items_center()
                        .gap_2()
                        .overflow_hidden()
                        .child(
                            div()
                                .w(px(6.0))
                                .h(px(6.0))
                                .rounded_full()
                                .bg(rgb(status_color(status))),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(if selected {
                                    palette().text
                                } else {
                                    palette().subtext
                                }))
                                .max_w(px(200.0))
                                .truncate()
                                .child(display_name),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette().dim))
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(command),
                        ),
                )
                .child(
                    div()
                        .w(px(COL_CPU))
                        .text_sm()
                        // `f32::min(NaN, 100.0)` returns 100.0 (red) while the
                        // label shows 0.0%; usage_color already handles NaN.
                        .text_color(rgb(usage_color(cpu)))
                        .child(format::format_cpu_percent(cpu)),
                )
                .child(
                    div()
                        .w(px(COL_MEMORY))
                        .text_sm()
                        .text_color(rgb(palette().subtext))
                        .child(format::format_bytes(memory)),
                )
                .child(
                    div()
                        .w(px(COL_PID))
                        .text_xs()
                        .text_color(rgb(palette().muted))
                        .child(pid),
                )
                .child(
                    div()
                        .w(px(COL_USER))
                        .text_xs()
                        .text_color(rgb(palette().subtext))
                        .truncate()
                        .child(user),
                )
                .child(div().w(px(COL_STATUS)).child(pill(
                    status_text,
                    status_color(status),
                    palette().surface,
                )))
                .child(
                    div()
                        .w(px(COL_THREADS))
                        .text_xs()
                        .text_color(rgb(palette().muted))
                        .child(threads),
                )
                .into_any_element(),
        )
    }

    fn details_panel(&mut self, cx: &mut Context<Self>) -> Div {
        let selected = self.dashboard.selected_process().cloned();
        let platform = self.dashboard.platform_info();
        let supports_control = platform.supports_process_control;
        let mut content = v_flex().gap_2();

        match selected {
            Some(process) => {
                let key = process.key;
                let protected = self.dashboard.is_protected(key);
                let now = self
                    .dashboard
                    .snapshot()
                    .map(|snapshot| snapshot.sampled_at.as_secs())
                    .unwrap_or_else(now_unix_secs);
                content = content
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(self.display(&process.name)),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(pill(
                                process.status.label(),
                                status_color(process.status),
                                palette().surface,
                            ))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette().muted))
                                    .child(format!("PID {}", process.key.pid)),
                            ),
                    )
                    .child(kv(
                        "ユーザー",
                        // Match the agent's redact_snapshot: every process user
                        // becomes "user", not just the login account (R41).
                        if self.redact && process.user.is_some() {
                            "user".to_string()
                        } else {
                            process.user.clone().unwrap_or_else(|| "—".to_string())
                        },
                    ))
                    .child(kv(
                        "起動から",
                        format::format_process_age(now, process.key.start_time_secs),
                    ))
                    .child(kv(
                        "スレッド",
                        process
                            .threads
                            .map(|threads| threads.to_string())
                            .unwrap_or_else(|| "—".to_string()),
                    ))
                    .child(kv(
                        "メモリ",
                        format!(
                            "{} / {}",
                            format::format_bytes(process.memory_bytes),
                            format::format_bytes(process.virtual_memory_bytes)
                        ),
                    ))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette().muted))
                            .child("コマンド"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette().subtext))
                            .child(self.display(&process.command)),
                    )
                    .child(div().h(px(1.0)).w_full().bg(rgb(palette().surface)))
                    .child(section_title("シグナル"))
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(action_button(
                                        "signal-term",
                                        "終了",
                                        palette().blue,
                                        signal_enabled(platform, protected, Signal::Terminate),
                                        cx.listener(move |this, _event, window, cx| {
                                            this.focus_handle.focus(window, cx);
                                            this.handle_command(
                                                Command::RequestSignal {
                                                    key,
                                                    signal: Signal::Terminate,
                                                },
                                                cx,
                                            );
                                        }),
                                    ))
                                    .child(action_button(
                                        "signal-kill",
                                        "強制終了",
                                        palette().red,
                                        signal_enabled(platform, protected, Signal::Kill),
                                        cx.listener(move |this, _event, window, cx| {
                                            this.focus_handle.focus(window, cx);
                                            this.handle_command(
                                                Command::RequestSignal {
                                                    key,
                                                    signal: Signal::Kill,
                                                },
                                                cx,
                                            );
                                        }),
                                    )),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(action_button(
                                        "signal-stop",
                                        "一時停止",
                                        palette().yellow,
                                        signal_enabled(platform, protected, Signal::Stop),
                                        cx.listener(move |this, _event, window, cx| {
                                            this.focus_handle.focus(window, cx);
                                            this.handle_command(
                                                Command::RequestSignal {
                                                    key,
                                                    signal: Signal::Stop,
                                                },
                                                cx,
                                            );
                                        }),
                                    ))
                                    .child(action_button(
                                        "signal-cont",
                                        "再開",
                                        palette().green,
                                        signal_enabled(platform, protected, Signal::Continue),
                                        cx.listener(move |this, _event, window, cx| {
                                            this.focus_handle.focus(window, cx);
                                            this.handle_command(
                                                Command::RequestSignal {
                                                    key,
                                                    signal: Signal::Continue,
                                                },
                                                cx,
                                            );
                                        }),
                                    )),
                            ),
                    )
                    .when(
                        supports_control
                            && (!platform.supports_signal(Signal::Stop)
                                || !platform.supports_signal(Signal::Continue)),
                        |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette().yellow))
                                    .child("この OS では一時停止 / 再開は利用できません"),
                            )
                        },
                    )
                    .when(!supports_control, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette().yellow))
                                .child("この環境は読み取り専用です（デモ / ブラウザ）"),
                        )
                    })
                    .when(protected, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette().yellow))
                                .child("PID 1 と自分自身は保護されています"),
                        )
                    });
            }
            None => {
                content = content.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette().muted))
                        .child("行を選択すると詳細を表示します"),
                );
            }
        }
        // cgroup グルーピングは読み取り専用。既存の詳細セクションを消さず、
        // ラベルと件数だけを追加する（R1-I12）。
        content = content.child(section_title("cgroup グループ"));
        let groups = self.dashboard.cgroup_groups();
        if groups.is_empty() {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette().muted))
                    .child("cgroup 情報なし"),
            );
        } else {
            for group in groups {
                content = content.child(div().text_xs().text_color(rgb(palette().text)).child(
                    cgroup_group_text(&self.display(&group.label), group.members.len()),
                ));
            }
        }
        content = content.child(section_title("センサー / 温度"));
        let components = self
            .dashboard
            .snapshot()
            .map(|s| s.components.as_slice())
            .unwrap_or(&[]);
        if components.is_empty() {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette().muted))
                    .child("センサーなし"),
            );
        } else {
            for (index, component) in components.iter().enumerate() {
                let mut value = component
                    .temperature_c
                    .map(|t| format!("{t:.0}°C"))
                    .unwrap_or_else(|| "—".to_string());
                if let Some(max) = component.max_c {
                    value.push_str(&format!(" / max {max:.0}°C"));
                }
                if let Some(critical) = component.critical_c {
                    value.push_str(&format!(" / 危険しきい値 {critical:.0}°C"));
                }
                let label = if component.label.is_empty() {
                    "センサー".to_string()
                } else {
                    self.display(&component.label)
                };
                // Mirror the overview temperature gauges: the detail row also
                // carries the textual state so AT-SPI never has to infer it
                // from the colour of the number.
                let aria_label = a11y_temperature_label(
                    &label,
                    &value,
                    component.temperature_c,
                    component.max_c,
                    component.critical_c,
                );
                content = content.child(
                    kv(label, value)
                        .id(("details-temperature", index))
                        .role(Role::Group)
                        .aria_label(aria_label),
                );
            }
        }
        v_flex()
            .w(px(DETAILS_WIDTH))
            .h_full()
            .bg(rgb(palette().mantle))
            .border_1()
            .border_color(rgb(palette().surface))
            .p_4()
            .gap_3()
            .child(section_title("プロセス詳細"))
            .child(
                div()
                    .id("details-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(content),
            )
    }

    fn main_area(&mut self, cx: &mut Context<Self>) -> Div {
        let header = self.table_header(cx);
        let list = self.process_list(cx);
        let details = if self.dashboard.details_visible() {
            Some(self.details_panel(cx))
        } else {
            None
        };
        h_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .items_start()
            .child(
                // The table node must wrap both the header and the rows so the
                // `RowGroup` / `Row` children are exposed in the right order.
                // Native has 7 columns (no separate コマンド column like Web).
                v_flex()
                    .id("process-table")
                    .role(Role::Table)
                    .aria_label("プロセス一覧")
                    .aria_row_count(self.dashboard.visible_count())
                    .aria_column_count(7)
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .min_h_0()
                    .child(header)
                    .child(list),
            )
            .children(details)
    }

    fn status_bar(&self) -> Stateful<Div> {
        let summary = self.summary();
        let platform = self.dashboard.platform_info().label();
        let status = self.dashboard.status().cloned();
        // AccessKit exposes this text to assistive technology, so announce the
        // same redacted display string as the visible bar (Z1-A11Y). Only the
        // error tone is derived from the original StatusMessage flag.
        let status_text = status.as_ref().map(|status| self.display(&status.text));
        let live_text = a11y_live_text(
            status_text
                .as_deref()
                .zip(status.as_ref().map(|status| status.is_error)),
        );
        let announcement = live_text.clone();
        let left = h_flex()
            .items_center()
            .gap_2()
            .text_xs()
            .text_color(rgb(palette().muted))
            .child(platform)
            .child(div().text_color(rgb(palette().dim)).child("|"))
            .child(if summary.paused {
                "一時停止中".to_string()
            } else {
                format!("更新間隔 {} ms", summary.interval_ms)
            })
            .child(div().text_color(rgb(palette().dim)).child("|"))
            .child(match summary.sampled_at {
                Some(sampled_at) => format!("最終更新 {}", format_clock(sampled_at.as_secs())),
                None => "最終更新 —".to_string(),
            })
            .child(div().text_color(rgb(palette().dim)).child("|"))
            .child(power_summary_text(self.dashboard.snapshot()));
        let right = match status {
            Some(status) => div()
                .text_xs()
                .text_color(rgb(if status.is_error {
                    palette().red
                } else {
                    palette().green
                }))
                .truncate()
                // Status text may carry probe/control errors with local paths
                // or identifiers; redact it like every other display string.
                .child(self.display(&status.text))
                .into_any_element(),
            None if self.filter_editing => div()
                .text_xs()
                .text_color(rgb(palette().blue))
                .child("日本語 IME 入力可（変換中は青+下線）· Enter で終了 · Esc で解除")
                .into_any_element(),
            None => div()
                .text_xs()
                .text_color(rgb(palette().dim))
                .child("↑↓ 選択 · Space 表示の一時停止 · Ctrl+F 検索 · Ctrl+K 終了 · F1 ヘルプ")
                .into_any_element(),
        };
        h_flex()
            .id("status-bar")
            .role(Role::Status)
            .aria_label("ステータス")
            .aria_value(live_text)
            // The native window already exposes a real AccessKit tree (unlike
            // the browser canvas), so no DOM mirror is needed. The synthetic
            // child is only a polite live region for status changes. Native has
            // no live-agent snapshot cache, hence no `stale` state to announce;
            // a failed probe is a persistent StatusMessage instead.
            .a11y_synthetic_children(move |builder| {
                let mut node = Node::new(Role::Status);
                node.set_live(Live::Polite);
                node.set_label(announcement);
                builder.push_child(builder.synthetic_node_id("status-live"), node);
            })
            .items_center()
            .justify_between()
            .gap_3()
            .px_3()
            .h(px(26.0))
            .bg(rgb(palette().mantle))
            .border_t_1()
            .border_color(rgb(palette().surface))
            .child(left)
            .child(right)
    }

    fn kill_dialog(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let pending = self.dashboard.pending_kill().cloned()?;
        let age = self
            .dashboard
            .snapshot()
            .map(|snapshot| {
                format::format_process_age(
                    snapshot.sampled_at.as_secs(),
                    pending.key.start_time_secs,
                )
            })
            .unwrap_or_else(|| "—".to_string());
        // Match the table/detail paths: every process user becomes "user"
        // under --redact (not just the login account) — R41 follow-up.
        let user = if self.redact && pending.user.is_some() {
            "user".to_string()
        } else {
            self.display(pending.user.as_deref().unwrap_or("—"))
        };
        let command = self.display(&pending.command);
        let (color, label) = match pending.signal {
            Signal::Kill => (palette().red, "強制終了"),
            Signal::Terminate => (palette().red, "終了"),
            Signal::Stop => (palette().yellow, "一時停止"),
            Signal::Continue => (palette().green, "再開"),
        };
        // R2X-P4: keep the stable label for DOM hooks, but state what Windows
        // actually does (`taskkill /F`) before the user confirms.
        let terminate_is_forceful = pending.signal == Signal::Terminate
            && !Signal::terminate_is_graceful_on(&self.dashboard.platform_info().os);
        Some(
            div()
                .id("kill-overlay")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgba(0x000000A6))
                // Guarantee that clicks never reach the process table behind
                // the modal (review s10): `occlude` is GPUI's explicit
                // hitbox-blocking API, unlike the no-op listener alone.
                .occlude()
                .on_click(cx.listener(|_this, _event, _window, _cx| {}))
                .child(
                    v_flex()
                        .w(px(470.0))
                        .p_5()
                        .gap_3()
                        .rounded_lg()
                        .bg(rgb(palette().mantle))
                        .border_1()
                        .border_color(rgb(palette().surface))
                        .shadow_lg()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::BOLD)
                                .child("プロセスの操作確認"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(palette().subtext))
                                .child(format!(
                                    "「{}」（PID {} / ユーザー {} / 起動から {}）に {} を送信します。",
                                    self.display(&pending.name),
                                    pending.key.pid,
                                    user,
                                    age,
                                    pending.signal.label()
                                )),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette().muted))
                                .child("コマンド（対象確認用）"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette().subtext))
                                .max_h(px(80.0))
                                .overflow_hidden()
                                .child(command),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette().muted))
                                .child(
                                    format!(
                                        "{} Enter でキャンセル、Ctrl+Enter または「実行」ボタンで確定します。",
                                        pending.confirmation_note
                                    ),
                                ),
                        )
                        .when(terminate_is_forceful, |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette().yellow))
                                    .child("この OS では「終了」も強制終了 (taskkill /F) として実行されます。"),
                            )
                        })
                        .child(
                            h_flex()
                                .justify_end()
                                .gap_2()
                                .child(
                                    action_button(
                                        "kill-cancel",
                                        "キャンセル",
                                        palette().surface_hi,
                                        true,
                                        cx.listener(|this, _event, window, cx| {
                                            this.focus_handle.focus(window, cx);
                                            this.handle_command(Command::CancelKill, cx);
                                        }),
                                    )
                                    .track_focus(&self.dialog_cancel_focus),
                                )
                                .child(
                                    action_button(
                                        "kill-confirm",
                                        label,
                                        color,
                                        true,
                                        cx.listener(|this, _event, window, cx| {
                                            this.focus_handle.focus(window, cx);
                                            this.handle_command(Command::ConfirmKill, cx);
                                        }),
                                    )
                                    .track_focus(&self.dialog_confirm_focus),
                                ),
                        ),
                ),
        )
    }

    fn help_overlay(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        if !self.show_help {
            return None;
        }
        let shortcuts = help_shortcuts();
        let rows: Vec<Div> = shortcuts
            .iter()
            .map(|(keys, description)| {
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .text_sm()
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .bg(rgb(palette().surface))
                            .text_xs()
                            .text_color(rgb(palette().text))
                            .child(*keys),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette().subtext))
                            .child(*description),
                    )
            })
            .collect();
        Some(
            div()
                .id("help-overlay")
                // Without a role the modal is just a generic container in the
                // real AccessKit tree, so AT-SPI never sees a dialog. The
                // 「閉じる」 button inside already goes through `action_button`,
                // which now supplies Role::Button and its Japanese name.
                .role(Role::Dialog)
                .aria_label("キーボード操作")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgba(0x000000A6))
                .occlude()
                .on_click(cx.listener(|_this, _event, _window, _cx| {}))
                .child(
                    v_flex()
                        .w(px(520.0))
                        .p_5()
                        .gap_3()
                        .rounded_lg()
                        .bg(rgb(palette().mantle))
                        .border_1()
                        .border_color(rgb(palette().surface))
                        .shadow_lg()
                        .child(
                            h_flex()
                                .justify_between()
                                .items_center()
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(FontWeight::BOLD)
                                        .child("キーボード操作"),
                                )
                                .child(
                                    action_button(
                                        "help-close",
                                        "閉じる",
                                        palette().surface_hi,
                                        true,
                                        cx.listener(|this, _event, window, cx| {
                                            this.focus_handle.focus(window, cx);
                                            this.show_help = false;
                                            cx.notify();
                                        }),
                                    )
                                    .track_focus(&self.help_close_focus),
                                ),
                        )
                        .children(rows),
                ),
        )
    }
}

// ------------------------------------------------------- IME / text input

/// ネイティブの IME（Wayland text-input-v3 / X11 XIM）を絞り込み欄へ橋渡しする。
///
/// `gpui-pre-linux` は composition の変換中文字列を
/// [`Self::replace_and_mark_text_in_range`]、確定を
/// [`Self::replace_text_in_range`]、変換中止を [`Self::unmark_text`] として呼ぶ。
/// UTF-16 オフセットの解釈は共有の [`FilterInput`] が担当し、**確定済み文字列
/// だけ**を Dashboard に反映する。絞り込み編集中だけ
/// `accepts_text_input` が `true` になる。
impl EntityInputHandler for TaskManagerApp {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let (text, actual) = self.filter_input.text_for_range(range);
        *adjusted_range = Some(actual);
        Some(text)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.filter_input.selection(),
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.filter_input.marked_range()
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let outcome = self.filter_input.unmark();
        self.apply_input_outcome(outcome, cx);
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let outcome = self.filter_input.replace_text(range, text);
        self.apply_input_outcome(outcome, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let outcome = self
            .filter_input
            .replace_and_mark(range, new_text, new_selected_range);
        self.apply_input_outcome(outcome, cx);
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // 変換候補ウィンドウを絞り込み欄の位置に出す。フィルタ欄は 1 行なので、
        // 行内の厳密なキャレット位置ではなく欄の矩形を返す。
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }

    fn text_length_utf16(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.filter_input.utf16_len())
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.filter_input.set_selection(range_utf16);
        cx.notify();
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        self.filter_editing
    }

    fn text_input_configuration(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> TextInputConfiguration {
        TextInputConfiguration {
            autocorrect: false,
            suggestions: false,
            ..Default::default()
        }
    }
}

impl Render for TaskManagerApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Overlay focus is driven in one place so it applies regardless of how
        // the overlay opened (keyboard shortcut or button click). On open, the
        // safe default control receives focus; on close, the previously focused
        // element (the root handle used for row navigation) is restored.
        let overlay_now = if self.dashboard.pending_kill().is_some() {
            OverlayKind::Dialog
        } else if self.show_help {
            OverlayKind::Help
        } else {
            OverlayKind::None
        };
        if overlay_now != self.last_overlay {
            match overlay_now {
                OverlayKind::None => {
                    if let Some(previous) = self.focus_before_overlay.take() {
                        previous.focus(window, cx);
                    } else {
                        self.focus_handle.focus(window, cx);
                    }
                }
                OverlayKind::Dialog => {
                    self.focus_before_overlay = Some(
                        window
                            .focused(cx)
                            .unwrap_or_else(|| self.focus_handle.clone()),
                    );
                    // 安全側（キャンセル）を初期フォーカスにする。
                    self.dialog_cancel_focus.focus(window, cx);
                }
                OverlayKind::Help => {
                    self.focus_before_overlay = Some(
                        window
                            .focused(cx)
                            .unwrap_or_else(|| self.focus_handle.clone()),
                    );
                    self.help_close_focus.focus(window, cx);
                }
            }
            self.last_overlay = overlay_now;
        }

        let top = self.top_bar(cx);
        let overview = self.overview();
        let toolbar = self.toolbar(cx);
        let main = self.main_area(cx);
        let status = self.status_bar();
        let dialog = self.kill_dialog(cx);
        let help = self.help_overlay(cx);

        div()
            .id("task-manager-root")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(palette().base))
            .text_color(rgb(palette().text))
            .child(top)
            .child(overview)
            .child(toolbar)
            .child(main)
            .child(status)
            .children(dialog)
            .children(help)
    }
}

/// Whether a signal button must be interactive for the selected process.
///
/// Kept pure so the per-signal capability gate can be regression-tested without
/// a GPUI window. `protected` is the view model's PID 1 / self safety rule;
/// button ids (`signal-term`, `signal-kill`, `signal-stop`, `signal-cont`) stay
/// unchanged so existing DOM hooks keep working.
fn signal_enabled(platform: &PlatformInfo, protected: bool, signal: Signal) -> bool {
    !protected && platform.supports_signal(signal)
}

fn overlay_text(message: impl Into<String>, color: u32) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(div().text_sm().text_color(rgb(color)).child(message.into()))
}

fn status_color(status: ProcessStatus) -> u32 {
    match status {
        ProcessStatus::Running => palette().green,
        ProcessStatus::Sleeping => palette().subtext,
        ProcessStatus::Idle => palette().muted,
        ProcessStatus::Stopped => palette().yellow,
        ProcessStatus::Zombie => palette().red,
        ProcessStatus::Tracing => palette().mauve,
        ProcessStatus::Unknown => palette().dim,
    }
}

// ------------------------------------------------------------ accessibility
//
// gpui-pre 0.3 exposes AccessKit `role` / `aria_*` through
// `StatefulInteractiveElement` on both native and wasm, so the native shell
// can carry the same semantics as the browser shell. What it cannot do on its
// own is mirror the text into a DOM `aria-live` node: the native window has no
// DOM (and no DOM proof), only the real AccessKit tree. The synthetic polite
// child in `status_bar()` is the native equivalent of that announcement.

/// Japanese direction name for a sortable header (↑/↓ is a glyph, not text).
fn a11y_sort_direction(direction: SortDirection) -> &'static str {
    match direction {
        SortDirection::Ascending => "昇順",
        SortDirection::Descending => "降順",
    }
}

/// Accessible name of the filter box, matching the browser shell.
fn a11y_filter_label() -> &'static str {
    "プロセスの絞り込み"
}

/// Accessible description (hint) of the filter box, matching the browser
/// shell: announcement follows the composed / editing / idle state, and never
/// contains the filter text itself.
fn a11y_filter_hint(composing_active: bool, editing: bool) -> &'static str {
    if composing_active {
        "変換中"
    } else if editing {
        "入力可"
    } else {
        "クリックで入力"
    }
}

/// Visible text of a sortable column header.
///
/// The active header carries the direction glyph (↑ / ↓). It must change with
/// the sort direction so the two states differ on screen even before the row
/// order is compared; the arrow is drawn from `SortDirection::arrow()` in both
/// shells.
fn sort_header_text(label: &str, active: bool, direction: SortDirection) -> String {
    if active {
        format!("{label}{}", direction.arrow())
    } else {
        label.to_string()
    }
}

/// Accessible name of a sortable column header, matching the browser shell.
fn sort_header_label(label: &str, active: bool, direction: SortDirection) -> String {
    if active {
        format!(
            "{label} 列で並び替え、現在 {}",
            a11y_sort_direction(direction)
        )
    } else {
        format!("{label} 列で並び替え")
    }
}

/// AccessKit `aria-sort` counterpart of the browser DOM mirror's attribute.
///
/// The browser mirror publishes a direction only on the active column; native
/// headers clear the property so a stale sort state is never announced.
fn a11y_sort_accesskit(active: bool, direction: SortDirection) -> Option<AccessKitSortDirection> {
    if !active {
        return None;
    }
    Some(match direction {
        SortDirection::Ascending => AccessKitSortDirection::Ascending,
        SortDirection::Descending => AccessKitSortDirection::Descending,
    })
}

/// Enter/Space on a focused column header is reserved for sorting (WCAG 2.1.1).
///
/// GPUI's focusable `Div` activates its existing `on_click` (SetSortKey) on the
/// key-up; `on_key_down` must not treat that key as Space=pause or a filter key.
fn is_sort_key_activation(key: &str, modifiers: Modifiers, editing: bool) -> bool {
    !editing && !modifiers.modified() && matches!(key, "enter" | "space")
}

/// Enter/Space on a focused process row is reserved for that row's `on_click`
/// (WCAG 2.1.1). Same unmodified-key predicate as the sortable header, so a
/// focused row never triggers Space=pause or filter editing underneath it.
fn is_row_activation(key: &str, modifiers: Modifiers, editing: bool) -> bool {
    is_sort_key_activation(key, modifiers, editing)
}

/// Textual severity for a colour-coded value (no colour dependency).
///
/// Mirrors the browser shell and the 60/85 thresholds of `theme::usage_color`,
/// so an AccessKit name never contradicts the colour it describes.
fn usage_severity(percent: f32) -> &'static str {
    if percent >= 85.0 {
        "高"
    } else if percent >= 60.0 {
        "中"
    } else {
        "低"
    }
}

/// Accessible name of a colour-coded utilisation gauge, matching the browser
/// shell's `a11y_usage_label`: the displayed percentage plus the Japanese
/// severity word, so a colour-blind reader (and AT-SPI) hears the same state
/// the colour conveys.
fn a11y_usage_label(title: &str, percent: f32) -> String {
    let percent = percent.clamp(0.0, 100.0);
    format!(
        "{title} {percent:.0}%、{severity}",
        severity = usage_severity(percent)
    )
}

/// Japanese state of a temperature reading relative to its two thresholds.
///
/// Only finite readings and strictly positive thresholds are compared: a NaN
/// from a driver or a missing/broken threshold must never be reported as an
/// optimistic "正常". Mirrors the browser shell's `temperature_severity`.
fn temperature_severity_label(
    temperature_c: Option<f32>,
    max_c: Option<f32>,
    critical_c: Option<f32>,
) -> &'static str {
    let finite = |value: Option<f32>| value.filter(|value| value.is_finite());
    let positive = |value: Option<f32>| finite(value).filter(|value| *value > 0.0);
    let Some(temperature) = finite(temperature_c) else {
        return "不明";
    };
    let max = positive(max_c);
    let critical = positive(critical_c);
    if max.is_none() && critical.is_none() {
        return "不明";
    }
    if critical.is_some_and(|critical| temperature >= critical) {
        return "危険";
    }
    if max.is_some_and(|max| temperature >= max) {
        return "注意";
    }
    "正常"
}

/// Accessible name of a temperature gauge: sensor, already-displayed value
/// (including thresholds) and the textual state (正常/注意/危険/不明).
fn a11y_temperature_label(
    sensor: &str,
    value_text: &str,
    temperature_c: Option<f32>,
    max_c: Option<f32>,
    critical_c: Option<f32>,
) -> String {
    format!(
        "{sensor} 温度 {value_text}、{state}",
        state = temperature_severity_label(temperature_c, max_c, critical_c)
    )
}

/// Accessible name of one process row, matching the browser shell.
///
/// `name` is the already-displayed (possibly 伏せ字化済み) value, so a screen
/// reader never hears an identifier that the screenshot hides.
fn a11y_row_label(
    name: &str,
    status: ProcessStatus,
    cpu: f32,
    memory_bytes: u64,
    pid: u32,
    selected: bool,
) -> String {
    let suffix = if selected { "、選択中" } else { "" };
    format!(
        "プロセス {name}、状態 {status_label}、CPU {cpu_label}（{severity}）、メモリ {memory_label}、PID {pid}{suffix}",
        status_label = status.label(),
        cpu_label = format::format_cpu_percent(cpu),
        severity = usage_severity(cpu),
        memory_label = format::format_bytes(memory_bytes),
    )
}

/// Accessible name of an application-group row (R1-APPDRAW).
///
/// Announces the same totals and member count the visible row shows and no
/// representative process's PID/user/status, which are not meaningful for a
/// group. `name` is already redacted by the caller like `a11y_row_label`.
fn a11y_group_row_label(
    name: &str,
    cpu: f32,
    memory_bytes: u64,
    process_count: usize,
    selected: bool,
) -> String {
    let suffix = if selected { "、選択中" } else { "" };
    format!(
        "アプリ {name}、{count} プロセス、CPU 合計 {cpu_label}（{severity}）、メモリ合計 {memory_label}{suffix}",
        count = process_count,
        cpu_label = format::format_cpu_percent(cpu),
        severity = usage_severity(cpu),
        memory_label = format::format_bytes(memory_bytes),
    )
}

/// Collapses whitespace and truncates for a one-line live announcement.
fn truncate_label(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    let mut out: String = normalized
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect();
    out.push('…');
    out
}

/// Text announced by the native status bar's AccessKit live node.
///
/// The browser shell also has a `stale` flag (a cached snapshot after a failed
/// poll) and mirrors the announcement into a DOM region. Native has no live
/// agent polling and no cached snapshot, so there is no freshness state to
/// announce: a failed native probe is a persistent `StatusMessage` instead.
/// If a freshness flag is added later, its user-facing wording must be the
/// Japanese `古いデータ` — never the internal `stale` / `赤退` terms.
///
/// The `text` is the already-redacted display string (Z1-A11Y); only the
/// `is_error` flag comes from the original `StatusMessage`. Callers must never
/// pass raw `StatusMessage::text` here, or a probe/control error could leak a
/// local path or host name through the native accessibility tree.
fn a11y_live_text(status: Option<(&str, bool)>) -> String {
    let Some((text, is_error)) = status else {
        return "ステータス: 変更なし".to_string();
    };
    let prefix = if is_error && !text.contains("エラー") {
        "エラー: "
    } else {
        ""
    };
    format!("{prefix}{}", truncate_label(text, 160))
}

/// Replaces local identifiers with generic values for public screenshots.
///
/// The rules are shared with the agent's `--redact` snapshot endpoint so the
/// native UI and the JSON evidence hide the same set of identifiers.
fn redact_text(text: &str, hostname: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default();
    let ids = tm_core::redact::RedactionIdentifiers {
        home: &home,
        user: &user,
        hostname,
    };
    tm_core::redact::redact_text(text, &ids)
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Resolved intent for one keystroke in the native shell.
///
/// A pure resolver keeps the native/Web keyboard contract testable without a
/// GPUI window and guarantees that navigation cannot fire while text is being
/// edited: the dispatcher below only executes what this function returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyAction {
    /// Esc while editing the filter: clear it and leave edit mode.
    CancelFilter,
    /// Enter while editing the filter: leave edit mode (IME commits itself).
    CommitFilter,
    /// Backspace while editing the filter (native platform does not handle it).
    BackspaceFilter,
    /// Ctrl/Cmd+F: focus the filter entry.
    BeginFilter,
    /// Ctrl/Cmd+K: request termination of the selected process.
    RequestTerminate,
    /// Arrow / Page / Home / End: move the selection by this many rows.
    SelectRelative(i32),
    TogglePaused,
    ToggleDetails,
    Refresh,
    ShowHelp,
    ClearFilter,
}

/// Resolves a key press to a shell action without touching GPUI state.
///
/// `editing_text` mirrors `TaskManagerApp::filter_editing`, which is `true` for
/// the whole IME session (including uncommitted composition). While it is set,
/// only the three text-editing keys are resolved: arrows, Page keys, Home/End
/// and shell shortcuts must reach the input method / browser instead, matching
/// the Web shell's rule that navigation never steals text input.
fn key_action(key: &str, modifiers: Modifiers, editing_text: bool) -> Option<KeyAction> {
    if editing_text {
        // Let the platform handle word deletion and other modified editing keys.
        if modifiers.control || modifiers.platform || modifiers.alt || modifiers.function {
            return None;
        }
        return match key {
            "escape" => Some(KeyAction::CancelFilter),
            "enter" => Some(KeyAction::CommitFilter),
            "backspace" => Some(KeyAction::BackspaceFilter),
            _ => None,
        };
    }

    // Shift/Alt/Fn combinations can be IME candidate or text-selection gestures;
    // never treat them as shell shortcuts. Ctrl (Linux/Windows) and Cmd (macOS)
    // are equivalent, so the same shortcut works cross-shell (R3-A03).
    if modifiers.alt || modifiers.shift || modifiers.function {
        return None;
    }
    let primary = modifiers.control || modifiers.platform;
    match (primary, key) {
        (true, "f") => Some(KeyAction::BeginFilter),
        (true, "k") => Some(KeyAction::RequestTerminate),
        (false, "up") => Some(KeyAction::SelectRelative(-1)),
        (false, "down") => Some(KeyAction::SelectRelative(1)),
        (false, "pageup") => Some(KeyAction::SelectRelative(-10)),
        (false, "pagedown") => Some(KeyAction::SelectRelative(10)),
        (false, "home") => Some(KeyAction::SelectRelative(-1_000_000)),
        (false, "end") => Some(KeyAction::SelectRelative(1_000_000)),
        (false, "space") => Some(KeyAction::TogglePaused),
        (false, "f2") => Some(KeyAction::ToggleDetails),
        (false, "f5") => Some(KeyAction::Refresh),
        (false, "f1") => Some(KeyAction::ShowHelp),
        (false, "escape") => Some(KeyAction::ClearFilter),
        _ => None,
    }
}

/// Help overlay shortcuts. Kept as a pure table so the wording can be asserted
/// in unit tests (in particular that `Esc` is listed exactly once and that its
/// description matches the real `on_key_down` priority).
///
/// `Esc` is intentionally consolidated into a single row: the shell resolves it
/// with the priority dialog > help > filter, so while the help overlay is open a
/// bare `Esc` only closes the help (it never clears the filter). F1 is a plain
/// toggle and no longer shares a row with `Esc`.
fn help_shortcuts() -> [(&'static str, &'static str); 13] {
    [
        ("↑ / ↓", "行の選択"),
        ("PgUp / PgDn", "10 行移動"),
        ("Home / End", "先頭 / 末尾"),
        ("Space", "更新の一時停止・再開"),
        ("Ctrl+F / Cmd+F", "絞り込み入力"),
        ("Ctrl+K / Cmd+K", "選択プロセスの終了"),
        ("F2", "詳細パネルの表示切替"),
        ("F5", "今すぐ更新"),
        ("F1", "このヘルプを開く / 閉じる"),
        ("Enter", "確認ダイアログをキャンセル（安全側）"),
        (
            "Enter / Space",
            "行のグループ展開/折りたたみ（行フォーカス時）",
        ),
        ("Ctrl/Cmd+Enter", "確認ダイアログでシグナルを実行"),
        (
            "Esc",
            "ダイアログ/ヘルプを閉じる · 絞り込み解除（表示中は閉じる操作が優先）",
        ),
    ]
}

/// Which overlay was visible in the last rendered frame. Focusing on open and
/// restoring on close is driven by transitions of this value in `Render::render`,
/// so it works for both keyboard shortcuts and button clicks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OverlayKind {
    None,
    Dialog,
    Help,
}

/// The two focusable controls of the confirmation dialog. Both Tab and
/// Shift+Tab toggle between them, which is enough to keep focus in the modal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DialogButton {
    Cancel,
    Confirm,
}

impl DialogButton {
    fn next(self) -> Self {
        match self {
            Self::Cancel => Self::Confirm,
            Self::Confirm => Self::Cancel,
        }
    }
}

/// Maps a key press while the confirmation dialog is open to a safe action.
///
/// A single Enter always cancels; confirming a signal requires Ctrl/Cmd+Enter or
/// an explicit click on the confirm button. This is the safety-critical part of
/// the dialog: an accidental Enter must never terminate a process.
fn dialog_command(key: &str, modifiers: Modifiers) -> Option<Command> {
    match key {
        "escape" => Some(Command::CancelKill),
        "enter" if modifiers.control || modifiers.platform => Some(Command::ConfirmKill),
        "enter" => Some(Command::CancelKill),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    // Explicit imports: the crate-root `use gpui_kit::*` re-exports a `test`
    // attribute macro that would shadow the built-in `#[test]` if `super::*`
    // were imported here.
    use super::{
        a11y_filter_hint, a11y_filter_label, a11y_group_row_label, a11y_live_text, a11y_row_label,
        a11y_sort_accesskit, a11y_sort_direction, a11y_temperature_label, a11y_usage_label,
        apply_retry_status, cgroup_group_text, dialog_command, help_shortcuts, is_row_activation,
        is_sort_key_activation, key_action, power_summary_text, process_row_values,
        report_save_result, resolve_probe_retry, signal_enabled, sort_header_label,
        sort_header_text, temperature_severity_label, truncate_label, usage_severity,
        AccessKitSortDirection, BackendInit, DialogButton, KeyAction, ProbeRetry,
    };
    use gpui_kit::Modifiers;
    use std::time::Duration;
    use tm_core::{
        PlatformInfo, ProcessKey, ProcessSnapshot, ProcessStatus, Signal, SortDirection,
    };
    use tm_viewmodel::{
        AppGroupRow, AppRowKind, Command, Dashboard, StatusMessage, DEFAULT_REFRESH_INTERVAL,
    };

    fn plain() -> Modifiers {
        Modifiers::default()
    }

    fn ctrl() -> Modifiers {
        Modifiers {
            control: true,
            ..Modifiers::default()
        }
    }

    fn cmd() -> Modifiers {
        Modifiers {
            platform: true,
            ..Modifiers::default()
        }
    }

    #[test]
    fn enter_cancels_the_confirmation_dialog() {
        assert_eq!(dialog_command("enter", plain()), Some(Command::CancelKill));
    }

    #[test]
    fn ctrl_enter_confirms_the_confirmation_dialog() {
        assert_eq!(dialog_command("enter", ctrl()), Some(Command::ConfirmKill));
    }

    #[test]
    fn cmd_enter_confirms_the_confirmation_dialog() {
        assert_eq!(dialog_command("enter", cmd()), Some(Command::ConfirmKill));
    }

    #[test]
    fn escape_cancels_the_confirmation_dialog() {
        assert_eq!(dialog_command("escape", plain()), Some(Command::CancelKill));
    }

    #[test]
    fn other_keys_are_ignored_while_the_dialog_is_open() {
        assert_eq!(dialog_command("down", plain()), None);
        assert_eq!(dialog_command("space", plain()), None);
        // Tab is handled by the focus-cycling path, never as a dialog command.
        assert_eq!(dialog_command("tab", plain()), None);
        assert_eq!(dialog_command("enter", ctrl()), Some(Command::ConfirmKill));
        assert_eq!(dialog_command("enter", cmd()), Some(Command::ConfirmKill));
    }

    #[test]
    fn dialog_tab_cycles_between_the_two_buttons_in_both_directions() {
        // With two buttons, forward and backward Tab both toggle, which keeps
        // keyboard focus inside the modal (there is no built-in focus scope in
        // this GPUI backend).
        assert_eq!(DialogButton::Cancel.next(), DialogButton::Confirm);
        assert_eq!(DialogButton::Confirm.next(), DialogButton::Cancel);
        assert_eq!(
            DialogButton::Cancel.next().next(),
            DialogButton::Cancel,
            "tab must cycle back to the first button"
        );
    }

    #[test]
    fn help_lists_esc_exactly_once_with_priority_wording() {
        let shortcuts = help_shortcuts();
        let esc_rows: Vec<_> = shortcuts.iter().filter(|(key, _)| *key == "Esc").collect();
        assert_eq!(esc_rows.len(), 1, "Esc must appear in exactly one row");
        let (_, description) = esc_rows[0];
        assert!(
            description.contains("ダイアログ") && description.contains("ヘルプ"),
            "the Esc description must cover both overlays: {description}"
        );
        assert!(
            description.contains("閉じる") && description.contains("絞り込み解除"),
            "the Esc description must state both actions: {description}"
        );
        // F1 no longer shares a row with Esc, so the two keys are unambiguous.
        let f1_rows: Vec<_> = shortcuts.iter().filter(|(key, _)| *key == "F1").collect();
        assert_eq!(f1_rows.len(), 1);
        assert!(!f1_rows[0].1.contains("Esc"), "F1 row must not re-list Esc");
    }

    #[test]
    fn ctrl_and_cmd_start_filter_editing() {
        assert_eq!(key_action("f", ctrl(), false), Some(KeyAction::BeginFilter));
        assert_eq!(key_action("f", cmd(), false), Some(KeyAction::BeginFilter));
    }

    #[test]
    fn ctrl_and_cmd_request_termination() {
        assert_eq!(
            key_action("k", ctrl(), false),
            Some(KeyAction::RequestTerminate)
        );
        assert_eq!(
            key_action("k", cmd(), false),
            Some(KeyAction::RequestTerminate)
        );
    }

    #[test]
    fn plain_keys_resolve_to_navigation_and_shell_commands() {
        let expected = [
            ("up", KeyAction::SelectRelative(-1)),
            ("down", KeyAction::SelectRelative(1)),
            ("pageup", KeyAction::SelectRelative(-10)),
            ("pagedown", KeyAction::SelectRelative(10)),
            ("home", KeyAction::SelectRelative(-1_000_000)),
            ("end", KeyAction::SelectRelative(1_000_000)),
            ("space", KeyAction::TogglePaused),
            ("f2", KeyAction::ToggleDetails),
            ("f5", KeyAction::Refresh),
            ("f1", KeyAction::ShowHelp),
            ("escape", KeyAction::ClearFilter),
        ];
        for (key, action) in expected {
            assert_eq!(key_action(key, plain(), false), Some(action), "{key}");
        }
    }

    #[test]
    fn navigation_never_fires_while_filter_text_is_being_edited() {
        // R3-A02: arrows, page keys and Home/End are IME candidate / caret keys.
        for key in [
            "up", "down", "pageup", "pagedown", "home", "end", "space", "f1", "f2", "f5",
        ] {
            assert_eq!(
                key_action(key, plain(), true),
                None,
                "{key} must not navigate during text input"
            );
        }
    }

    #[test]
    fn shell_shortcuts_never_fire_while_filter_text_is_being_edited() {
        for modifiers in [ctrl(), cmd()] {
            for key in ["f", "k", "up", "f2", "f5"] {
                assert_eq!(
                    key_action(key, modifiers, true),
                    None,
                    "shortcut must not fire during text input: {key}"
                );
            }
        }
    }

    #[test]
    fn editing_keys_still_work_while_filter_text_is_being_edited() {
        assert_eq!(
            key_action("escape", plain(), true),
            Some(KeyAction::CancelFilter)
        );
        assert_eq!(
            key_action("enter", plain(), true),
            Some(KeyAction::CommitFilter)
        );
        assert_eq!(
            key_action("backspace", plain(), true),
            Some(KeyAction::BackspaceFilter)
        );
        // Text keystrokes stay with the platform IME / browser input mirror.
        assert_eq!(key_action("a", plain(), true), None);
        assert_eq!(key_action("あ", plain(), true), None);
    }

    #[test]
    fn modified_editing_keys_are_left_to_the_platform() {
        for modifiers in [ctrl(), cmd()] {
            assert_eq!(key_action("backspace", modifiers, true), None);
        }
    }

    #[test]
    fn shift_alt_and_function_keys_are_not_shortcuts() {
        for modifiers in [
            Modifiers {
                shift: true,
                ..Modifiers::default()
            },
            Modifiers {
                alt: true,
                ..Modifiers::default()
            },
            Modifiers {
                function: true,
                ..Modifiers::default()
            },
        ] {
            for key in ["f", "k", "up", "f2"] {
                assert_eq!(key_action(key, modifiers, false), None, "{key}");
            }
        }
    }

    #[test]
    fn windows_signal_buttons_disable_only_stop_and_continue() {
        // R2X-P2: the gate is per signal, not per section. Terminate/Kill keep
        // working on Windows while SIGSTOP/SIGCONT are disabled.
        let platform = PlatformInfo::native("Windows 11 Pro");
        assert!(signal_enabled(&platform, false, Signal::Terminate));
        assert!(signal_enabled(&platform, false, Signal::Kill));
        assert!(!signal_enabled(&platform, false, Signal::Stop));
        assert!(!signal_enabled(&platform, false, Signal::Continue));
    }

    #[test]
    fn linux_signal_buttons_all_enabled_when_unprotected() {
        let platform = PlatformInfo::native("Linux");
        for signal in [
            Signal::Terminate,
            Signal::Kill,
            Signal::Stop,
            Signal::Continue,
        ] {
            assert!(signal_enabled(&platform, false, signal), "{signal:?}");
        }
    }

    #[test]
    fn protected_process_disables_every_signal() {
        let platform = PlatformInfo::native("Linux");
        for signal in [
            Signal::Terminate,
            Signal::Kill,
            Signal::Stop,
            Signal::Continue,
        ] {
            assert!(!signal_enabled(&platform, true, signal), "{signal:?}");
        }
    }

    #[test]
    fn read_only_backend_disables_every_signal() {
        for platform in [
            PlatformInfo::native_read_only("Linux (エージェント経由)"),
            PlatformInfo::demo(),
        ] {
            for signal in [
                Signal::Terminate,
                Signal::Kill,
                Signal::Stop,
                Signal::Continue,
            ] {
                assert!(!signal_enabled(&platform, false, signal), "{signal:?}");
            }
        }
    }

    #[test]
    fn explicit_demo_stays_demo_without_creating_a_native_probe() {
        // R3-S02B: `--demo` remains the explicit opt-in and must not depend on
        // native probe availability, matching the `main.rs` preflight.
        let called = std::cell::Cell::new(false);
        let init = BackendInit::resolve(true, || {
            called.set(true);
            Ok::<_, tm_core::ProbeError>(())
        });
        assert_eq!(init, BackendInit::Demo);
        assert!(init.is_demo());
        assert_eq!(init.startup_status(), None);
        assert!(!called.get(), "demo mode must not create a native probe");
    }

    #[test]
    fn native_probe_success_starts_live_without_error_status() {
        let init = BackendInit::resolve(false, || Ok::<_, tm_core::ProbeError>(()));
        assert_eq!(init, BackendInit::Native(()));
        assert!(!init.is_demo());
        assert_eq!(init.startup_status(), None);
    }

    #[test]
    fn probe_failure_is_an_explicit_error_not_an_implicit_demo() {
        // R3-S02B: app::new() must not substitute demo data when the native
        // probe fails. The failure becomes a persistent status using the same
        // shared message as the GUI preflight and `--export`.
        let error = tm_core::ProbeError::Io("初期化失敗".to_string());
        let expected = tm_core::demo::probe_failure_message(&error);
        let init = BackendInit::resolve(false, || Err::<(), _>(error));
        assert_eq!(init, BackendInit::Failed(expected.clone()));
        assert!(
            !init.is_demo(),
            "a failed probe must show neither demo data nor a DEMO badge"
        );
        assert_eq!(init.startup_status(), Some(expected.as_str()));
        assert!(expected.contains("初期化失敗"), "{expected}");
    }

    #[test]
    fn probe_failure_status_uses_the_shared_preflight_wording() {
        // The status text must be byte-for-byte the message `main.rs` prints
        // on the preflight path, for every classified probe error.
        for error in [
            tm_core::ProbeError::Io("I/O 失敗".to_string()),
            tm_core::ProbeError::Permission("権限不足".to_string()),
            tm_core::ProbeError::Unsupported("未対応".to_string()),
        ] {
            let expected = tm_core::demo::probe_failure_message(&error);
            let init = BackendInit::resolve(false, || Err::<(), _>(error));
            assert_eq!(init.startup_status(), Some(expected.as_str()));
            assert!(!init.is_demo());
        }
    }

    #[test]
    fn sort_header_accessible_name_spells_out_the_direction_in_japanese() {
        assert_eq!(a11y_sort_direction(SortDirection::Ascending), "昇順");
        assert_eq!(a11y_sort_direction(SortDirection::Descending), "降順");
        assert_eq!(
            sort_header_label("CPU", true, SortDirection::Descending),
            "CPU 列で並び替え、現在 降順"
        );
        assert_eq!(
            sort_header_label("名前", false, SortDirection::Ascending),
            "名前 列で並び替え"
        );
    }

    #[test]
    fn sort_header_visible_text_changes_with_direction() {
        // Tofu regression: ▲/▼ used to be missing from IBM Plex, so both
        // directions rendered as the same box. The visible header must differ.
        let asc = sort_header_text("CPU", true, SortDirection::Ascending);
        let desc = sort_header_text("CPU", true, SortDirection::Descending);
        assert_eq!(asc, "CPU↑");
        assert_eq!(desc, "CPU↓");
        assert_ne!(asc, desc);
        assert_eq!(
            sort_header_text("CPU", false, SortDirection::Ascending),
            "CPU"
        );
    }

    #[test]
    fn focused_sort_header_owns_enter_space_for_accesskit() {
        assert_eq!(
            a11y_sort_accesskit(true, SortDirection::Ascending),
            Some(AccessKitSortDirection::Ascending)
        );
        assert_eq!(
            a11y_sort_accesskit(true, SortDirection::Descending),
            Some(AccessKitSortDirection::Descending)
        );
        assert_eq!(a11y_sort_accesskit(false, SortDirection::Ascending), None);
        assert_eq!(a11y_sort_accesskit(false, SortDirection::Descending), None);

        assert!(is_sort_key_activation("enter", plain(), false));
        assert!(is_sort_key_activation("space", plain(), false));
        assert!(!is_sort_key_activation("enter", plain(), true));
        assert!(!is_sort_key_activation(
            "space",
            Modifiers {
                shift: true,
                ..plain()
            },
            false
        ));
        // 既存の Space=一時停止 / F2 ショートカットは見出し以外で従来どおり。
        assert_eq!(
            key_action("space", plain(), false),
            Some(KeyAction::TogglePaused)
        );
        assert_eq!(
            key_action("f2", plain(), false),
            Some(KeyAction::ToggleDetails)
        );
    }

    #[test]
    fn power_and_cgroup_display_text_is_read_only_and_bounded() {
        assert_eq!(power_summary_text(None), "電源 —");
        let demo = tm_core::demo::demo_snapshot(0);
        let text = power_summary_text(Some(&demo));
        assert!(text.contains("AC 電源"), "{text}");
        assert!(text.contains("通常"), "{text}");
        assert!(text.contains("cgroup 2 グループ"), "{text}");
        assert!(!text.contains('\n'), "{text}");
        assert_eq!(
            cgroup_group_text("demo.slice/system.service", 3),
            "demo.slice/system.service — 3 件"
        );
    }

    #[test]
    fn filter_box_accessible_name_matches_browser_shell() {
        assert_eq!(a11y_filter_label(), "プロセスの絞り込み");
    }

    #[test]
    fn filter_box_hint_follows_composition_and_editing_state() {
        assert_eq!(a11y_filter_hint(true, true), "変換中");
        assert_eq!(a11y_filter_hint(true, false), "変換中");
        assert_eq!(a11y_filter_hint(false, true), "入力可");
        assert_eq!(a11y_filter_hint(false, false), "クリックで入力");
    }

    #[test]
    fn filter_box_wires_textinput_role_and_redacted_value() {
        // `Div`'s role / aria fields are private to gpui-pre and this repo has
        // no window-rendering test harness, so check the wiring at the source
        // level. Split needles keep the test from matching its own literals.
        let source = include_str!("app.rs");
        assert!(
            source.contains(concat!(".role(", "Role::TextInput)")),
            "filter box must expose the AccessKit text-input role"
        );
        assert!(
            source.contains(concat!(".aria_label(", "a11y_filter_label())")),
            "filter box must expose the Japanese accessible name"
        );
        assert!(
            source.contains(concat!(
                "let filter_aria_value = self.display(self.dashboard.",
                "filter_text());"
            )),
            "aria_value must be fed the redacted display text"
        );
        assert!(
            source.contains(concat!(".aria_value(", "filter_aria_value)")),
            "filter box must expose the display text as its value"
        );
        assert!(
            source.contains(concat!(".aria_description(", "filter_a11y_hint)")),
            "filter box must expose the state hint as its description"
        );
    }

    #[test]
    fn filter_aria_value_never_speaks_raw_local_identifier() {
        // `aria_value` is fed `self.display(filter_text)`, which delegates to
        // the shared redaction boundary under `--redact`. A local identifier
        // typed into the filter must not survive into the announced value.
        use tm_core::redact::{redact_text as core_redact_text, RedactionIdentifiers};

        let ids = RedactionIdentifiers {
            home: "/home/koppe",
            user: "koppe",
            hostname: "nuc",
        };
        let redacted = core_redact_text("secret /home/koppe/project user:koppe host:nuc", &ids);
        assert!(!redacted.contains("koppe"), "{redacted}");
        assert!(!redacted.contains("/home/koppe"), "{redacted}");
        assert!(!redacted.contains("host:nuc"), "{redacted}");
        assert!(redacted.contains("<home>"), "{redacted}");
    }

    #[test]
    fn row_accessible_name_uses_japanese_wording_and_marks_selection() {
        let label = a11y_row_label("emacs", ProcessStatus::Running, 90.0, 1_024, 42, true);
        assert!(label.starts_with("プロセス emacs、状態 "), "{label}");
        assert!(label.contains("CPU 90.0%（高）"), "{label}");
        assert!(label.contains("PID 42"), "{label}");
        assert!(label.ends_with("、選択中"), "{label}");
        assert!(
            !label.contains("redact") && !label.contains("stale"),
            "{label}"
        );

        let plain = a11y_row_label("emacs", ProcessStatus::Sleeping, 10.0, 0, 1, false);
        assert!(!plain.contains("選択中"), "{plain}");
        assert!(plain.contains("CPU 10.0%（低）"), "{plain}");
    }

    #[test]
    fn usage_severity_matches_the_colour_thresholds() {
        assert_eq!(usage_severity(59.9), "低");
        assert_eq!(usage_severity(60.0), "中");
        assert_eq!(usage_severity(84.9), "中");
        assert_eq!(usage_severity(85.0), "高");
    }

    #[test]
    fn action_button_exposes_button_role_and_name_in_shared_base() {
        // `Div`'s role / aria fields are private to gpui-pre and there is no
        // window-rendering harness here, so verify the source wiring. Split
        // needles keep the test from matching its own literals. The inspected
        // slice ends before the enabled/disabled branch: both must inherit the
        // same semantics from `base`.
        let source = include_str!("ui.rs");
        let base = source
            .split(concat!("pub fn action_", "button("))
            .nth(1)
            .and_then(|rest| rest.split("if enabled {").next())
            .expect("action_button base must exist");
        assert!(
            base.contains(concat!(".role(", "Role::Button)")),
            "every action button must expose the AccessKit push-button role"
        );
        assert!(
            base.contains(concat!(".aria_label(", "label)")),
            "the visible label must also be the accessible name"
        );
        assert!(
            !base.contains("if enabled"),
            "role/name must live on the shared base, not in one branch"
        );
    }

    #[test]
    fn accessible_metric_bar_exposes_progress_indicator_semantics() {
        let source = include_str!("ui.rs");
        let helper = source
            .split(concat!("pub fn accessible_", "metric_bar("))
            .nth(1)
            .and_then(|rest| rest.split("\n}").next())
            .expect("accessible_metric_bar helper must exist");
        assert!(
            helper.contains(concat!(".role(", "Role::ProgressIndicator)")),
            "gauges must be discoverable as progress indicators by AT-SPI"
        );
        assert!(helper.contains(concat!(".aria_numeric_value(", "percent as f64)")));
        assert!(helper.contains(".aria_value(format!"));
        assert!(
            helper.contains("metric_bar(percent, color)"),
            "the visual bar must remain the unchanged metric_bar"
        );
    }

    #[test]
    fn help_overlay_exposes_a_named_dialog_to_atspi() {
        let source = include_str!("app.rs");
        let overlay = source
            .split(concat!("fn help_", "overlay("))
            .nth(1)
            .expect("help_overlay must exist");
        let header = overlay
            .split(".absolute()")
            .next()
            .expect("help overlay header must exist");
        assert!(
            header.contains(concat!(".role(", "Role::Dialog)")),
            "the F1 modal must be a dialog node in the real AccessKit tree"
        );
        assert!(
            header.contains(concat!(".aria_label(", "\"キーボード操作\")")),
            "the dialog must carry its Japanese title as accessible name"
        );
        assert!(
            overlay.contains(concat!("\"help-", "close\"")),
            "the close control must use the shared action_button (Role::Button)"
        );
    }

    #[test]
    fn usage_gauge_aria_label_spells_out_value_and_severity() {
        assert_eq!(a11y_usage_label("CPU 使用率", 90.0), "CPU 使用率 90%、高");
        assert_eq!(
            a11y_usage_label("メモリ使用率", 70.0),
            "メモリ使用率 70%、中"
        );
        assert_eq!(
            a11y_usage_label("スワップ使用率", 10.0),
            "スワップ使用率 10%、低"
        );
        // Out-of-range probe data is clamped like the visible bar.
        assert_eq!(a11y_usage_label("CPU 使用率", 140.0), "CPU 使用率 100%、高");
    }

    #[test]
    fn temperature_aria_label_carries_state_and_rejects_invalid_data() {
        assert_eq!(
            temperature_severity_label(Some(101.0), Some(68.0), Some(100.0)),
            "危険"
        );
        assert_eq!(
            temperature_severity_label(Some(70.0), Some(68.0), Some(100.0)),
            "注意"
        );
        assert_eq!(
            temperature_severity_label(Some(30.0), Some(68.0), Some(100.0)),
            "正常"
        );
        assert_eq!(temperature_severity_label(None, None, None), "不明");
        // A NaN reading or a broken threshold must never become "正常".
        assert_eq!(
            temperature_severity_label(Some(f32::NAN), Some(68.0), Some(100.0)),
            "不明"
        );
        assert_eq!(
            temperature_severity_label(Some(30.0), Some(-1.0), Some(0.0)),
            "不明"
        );

        let label = a11y_temperature_label(
            "CPU",
            "101°C / crit 100°C",
            Some(101.0),
            Some(68.0),
            Some(100.0),
        );
        assert_eq!(label, "CPU 温度 101°C / crit 100°C、危険");
    }

    #[test]
    fn gauge_aria_wiring_reaches_memory_swap_and_disk_bars() {
        let source = include_str!("app.rs");
        assert!(source.contains(concat!("accessible_", "metric_bar(")));
        assert!(source.contains(concat!("\"memory-", "usage-bar\"")));
        assert!(source.contains(concat!("\"swap-", "usage-bar\"")));
        assert!(source.contains(concat!("(\"disk-", "usage-bar\", index)")));
        assert!(source.contains(concat!("a11y_temperature_", "label(")));
    }

    #[test]
    fn existing_table_row_and_status_accesskit_contract_is_preserved() {
        // The new dialog / button / gauge nodes must not replace the contracts
        // already consumed by the AT-SPI (ta-nativeax) discovery.
        let source = include_str!("app.rs");
        for role in [
            concat!("Role::", "Table"),
            concat!("Role::", "RowGroup"),
            concat!("Role::", "Row"),
            concat!("Role::", "ColumnHeader"),
            concat!("Role::", "Status"),
            concat!("Role::", "TextInput"),
        ] {
            assert!(source.contains(role), "missing AccessKit role {role}");
        }
    }

    #[test]
    fn status_live_text_avoids_internal_terms_and_marks_errors() {
        assert_eq!(a11y_live_text(None), "ステータス: 変更なし");
        let error = StatusMessage {
            text: "収集不可".to_string(),
            is_error: true,
            persistent: true,
        };
        assert_eq!(
            a11y_live_text(Some((&error.text, error.is_error))),
            "エラー: 収集不可"
        );
        let info = StatusMessage {
            text: "  更新  しました ".to_string(),
            is_error: false,
            persistent: false,
        };
        assert_eq!(
            a11y_live_text(Some((&info.text, info.is_error))),
            "更新 しました"
        );
    }

    #[test]
    fn status_a11y_announcement_never_speaks_raw_local_identifiers() {
        // Z1-A11Y: the AccessKit live region is fed the same redacted display
        // string as the visible status bar, never raw `StatusMessage::text`.
        let source = include_str!("app.rs");
        assert!(
            source.contains(concat!(
                "let status_text = status.as_ref().map(|status| self.",
                "display(&status.text));"
            )),
            "the announcement must be built from the redacted status text"
        );
        assert!(
            source.contains(concat!(".aria_value(", "live_text)")),
            "aria_value must expose the redacted announcement"
        );
        assert!(
            source.contains(concat!("node.set_label(", "announcement)")),
            "the synthetic live node must expose the redacted announcement"
        );

        // Behavioural proof: with --redact the shared boundary strips local
        // paths, user names and host names from the announced error text.
        use tm_core::redact::{redact_text as core_redact_text, RedactionIdentifiers};
        let ids = RedactionIdentifiers {
            home: "/home/koppe",
            user: "koppe",
            hostname: "nuc",
        };
        let raw = "probe failed under /home/koppe/proj on nuc for user koppe";
        let redacted = core_redact_text(raw, &ids);
        let announced = a11y_live_text(Some((&redacted, true)));
        assert!(!announced.contains("/home/koppe"), "{announced}");
        assert!(!announced.contains("koppe"), "{announced}");
        assert!(!announced.contains("nuc"), "{announced}");
        assert!(announced.starts_with("エラー: "), "{announced}");
    }

    #[test]
    fn cgroup_detail_row_never_shows_raw_local_identifiers() {
        // Z1-A11Y: cgroup labels derive from /proc paths and can embed local
        // identifiers; the detail row must use the sensor-row redaction path.
        let source = include_str!("app.rs");
        assert!(
            source.contains(concat!(
                "cgroup_group_text(&self.display(&group.label), ",
                "group.members.len())"
            )),
            "cgroup detail row must feed its label through self.display"
        );

        use tm_core::redact::{redact_text as core_redact_text, RedactionIdentifiers};
        let ids = RedactionIdentifiers {
            home: "/home/koppe",
            user: "koppe",
            hostname: "nuc",
        };
        let raw = "/home/koppe/nuc.slice";
        let redacted = core_redact_text(raw, &ids);
        let row = cgroup_group_text(&redacted, 3);
        assert!(!row.contains("/home/koppe"), "{row}");
        assert!(!row.contains("koppe"), "{row}");
        assert!(!row.contains("nuc"), "{row}");
        assert!(row.contains("<home>"), "{row}");
        assert!(row.ends_with("— 3 件"), "{row}");
    }

    #[test]
    fn failed_settings_save_is_visible_but_success_preserves_status() {
        use crate::settings::NOTICE_SAVE_FAILED;
        use std::io;

        let mut dashboard = Dashboard::new(PlatformInfo::demo(), DEFAULT_REFRESH_INTERVAL);
        dashboard.set_persistent_status("起動時の警告");
        report_save_result(
            &mut dashboard,
            Err(io::Error::other("secret /home/user/settings.json")),
        );
        let status = dashboard.status().expect("failed save must be visible");
        assert!(status.is_error);
        assert!(
            status.persistent,
            "save failure must persist like the web shell"
        );
        assert!(status.text.ends_with(NOTICE_SAVE_FAILED), "{}", status.text);
        assert!(
            !status.text.contains("/home"),
            "I/O detail must not leak into the status: {}",
            status.text
        );

        // A successful save must not erase an existing (startup) notice.
        let mut dashboard = Dashboard::new(PlatformInfo::demo(), DEFAULT_REFRESH_INTERVAL);
        dashboard.set_persistent_status("起動時の警告");
        report_save_result(&mut dashboard, Ok(()));
        let status = dashboard
            .status()
            .expect("success must not clear an existing status");
        assert!(status.text.ends_with("起動時の警告"), "{}", status.text);
    }

    #[test]
    fn truncate_label_boundaries_keep_a_visible_ellipsis() {
        // A zero limit still yields the ellipsis marker (a visibly truncated
        // announcement); no byte slicing is involved.
        assert_eq!(truncate_label("abc", 0), "…");
        assert_eq!(truncate_label("", 0), "");
        // A one-char limit keeps a single char verbatim but collapses a longer
        // input to the ellipsis (1 - 1 = 0 kept chars + ellipsis).
        assert_eq!(truncate_label("a", 1), "a");
        assert_eq!(truncate_label("abc", 1), "…");
    }

    #[test]
    fn truncate_label_counts_chars_not_bytes_and_preserves_utf8() {
        // Whitespace is collapsed first, then the char limit is applied.
        assert_eq!(truncate_label("  収集  不可  ", 10), "収集 不可");
        // Over the limit: exactly `max_chars` chars, the last one being '…'.
        let over = truncate_label("abcdefghij", 5);
        assert_eq!(over, "abcd…");
        assert_eq!(over.chars().count(), 5);

        // Multibyte text: every cut lands on a char boundary, so the result is
        // always valid UTF-8 and a prefix of the original string.
        let text = "日本語の長いステータスメッセージです";
        for limit in [0, 1, 2, 3, 5, 8] {
            let out = truncate_label(text, limit);
            assert_eq!(String::from_utf8(out.as_bytes().to_vec()).unwrap(), out);
            let kept = out.strip_suffix('…').unwrap_or(out.as_str());
            assert!(text.starts_with(kept), "limit={limit}: {out}");
            assert!(out.chars().count() <= limit.max(1), "limit={limit}: {out}");
        }
        // A limit at or above the length returns the whole normalized string.
        assert_eq!(truncate_label(text, 100), text);
    }

    #[test]
    fn probe_retry_rebuilds_the_native_probe_through_startup_resolution() {
        // Success is only a freshly built native probe, never a synthetic one.
        let calls = std::cell::Cell::new(0);
        let outcome = resolve_probe_retry(false, || {
            calls.set(calls.get() + 1);
            Ok::<_, tm_core::ProbeError>(())
        });
        assert_eq!(outcome, ProbeRetry::Recovered(()));
        assert_eq!(
            calls.get(),
            1,
            "retry must actually re-run the native factory"
        );
    }

    #[test]
    fn probe_retry_keeps_the_shared_failure_wording_when_still_unavailable() {
        let error = tm_core::ProbeError::Permission("再試行も失敗".to_string());
        let expected = tm_core::demo::probe_failure_message(&error);
        let outcome = resolve_probe_retry(false, || Err::<(), _>(error));
        assert_eq!(outcome, ProbeRetry::StillFailed(expected.clone()));
        // Byte-for-byte identical to the startup path's persistent status.
        let startup = BackendInit::resolve(false, || {
            Err::<(), _>(tm_core::ProbeError::Permission("再試行も失敗".to_string()))
        });
        assert_eq!(startup.startup_status(), Some(expected.as_str()));
    }

    #[test]
    fn probe_retry_never_installs_demo_data() {
        // Defensive: even if a caller passed demo = true, retry cannot install
        // synthetic data or touch the native factory.
        let called = std::cell::Cell::new(false);
        let outcome = resolve_probe_retry(true, || {
            called.set(true);
            Ok::<_, tm_core::ProbeError>(())
        });
        assert!(matches!(outcome, ProbeRetry::StillFailed(_)));
        assert!(
            !called.get(),
            "demo resolution must not create a native probe"
        );
    }

    #[test]
    fn retry_status_clears_only_the_matching_persistent_failure() {
        let recovered: ProbeRetry<()> = ProbeRetry::Recovered(());
        let original = "元の起動エラー";

        let mut dashboard = Dashboard::new(
            PlatformInfo::native_read_only("収集不可"),
            DEFAULT_REFRESH_INTERVAL,
        );
        dashboard.set_persistent_status(original);
        // The stored text carries the shared error prefix, which is exactly
        // what the retry path passes back from `dashboard.status()`.
        let stored = dashboard.status().unwrap().text.clone();
        apply_retry_status(&mut dashboard, &recovered, &stored);
        assert_eq!(dashboard.status(), None, "matching failure must be cleared");

        // An unrelated persistent notice is preserved by a recovery.
        let mut dashboard = Dashboard::new(
            PlatformInfo::native_read_only("収集不可"),
            DEFAULT_REFRESH_INTERVAL,
        );
        dashboard.set_persistent_status("設定の警告");
        apply_retry_status(&mut dashboard, &recovered, &stored);
        assert!(
            dashboard
                .status()
                .is_some_and(|status| status.text.ends_with("設定の警告")),
            "a retry must not erase an unrelated notice"
        );

        // A still-failing probe re-publishes the failure as a persistent error.
        let failed: ProbeRetry<()> = ProbeRetry::StillFailed("再試行失敗".to_string());
        apply_retry_status(&mut dashboard, &failed, original);
        let status = dashboard.status().expect("failure must stay visible");
        assert!(status.persistent && status.is_error);
        assert!(status.text.ends_with("再試行失敗"), "{}", status.text);
    }

    #[test]
    fn probe_unavailable_overlay_wires_a_retry_control() {
        // The failure overlay must expose a retry button bound to the recovery
        // path; split literals keep this test from matching its own source.
        let source = include_str!("app.rs");
        assert!(
            source.contains(concat!("\"retry-", "probe\"")),
            "failure overlay must expose a retry button"
        );
        assert!(
            source.contains(concat!("this.", "retry_probe(cx);")),
            "retry button must rebuild the probe through the startup path"
        );
    }

    #[test]
    fn native_timer_floor_follows_the_snapshot_power_plan() {
        let mut dashboard = Dashboard::new(PlatformInfo::demo(), DEFAULT_REFRESH_INTERVAL);
        // No snapshot yet: startup cadence is unchanged.
        assert_eq!(dashboard.effective_interval(), DEFAULT_REFRESH_INTERVAL);
        dashboard.ingest(tm_core::demo::demo_snapshot(0));
        assert_eq!(dashboard.effective_interval(), DEFAULT_REFRESH_INTERVAL);

        let mut battery = tm_core::demo::demo_snapshot(1);
        battery.power = tm_core::PowerSnapshot {
            source: tm_core::PowerSource::Battery,
            batteries: vec![tm_core::BatterySnapshot {
                name: "BAT0".into(),
                capacity_percent: Some(80),
                state: tm_core::BatteryState::Discharging,
            }],
        };
        battery.sampling =
            tm_core::recommended_sampling_plan(&battery.power, tm_core::WindowVisibility::Visible);
        dashboard.ingest(battery);
        assert_eq!(
            dashboard.effective_interval(),
            Duration::from_millis(tm_core::BATTERY_INTERVAL_MS)
        );
    }
    #[test]
    fn settings_debug_hook_never_emits_the_absolute_config_path() {
        let label = super::TaskManagerApp::config_file_label(Some(std::path::PathBuf::from(
            "/home/alice/.config/gpui-task-manager/settings.json",
        )));
        assert_eq!(label, "settings.json");
        assert!(!label.contains("home"));
        assert!(!label.contains("alice"));
        assert!(!label.contains('/'));

        let none = super::TaskManagerApp::config_file_label(None);
        assert_eq!(none, "<unknown>");
        assert!(!none.contains('/'));
    }

    fn sample_process(pid: u32, name: &str) -> ProcessSnapshot {
        ProcessSnapshot {
            key: ProcessKey::new(pid, 1_700_000_000),
            name: name.to_string(),
            command: format!("/usr/bin/{name} --flag"),
            user: Some("alice".to_string()),
            status: ProcessStatus::Sleeping,
            cpu_percent: 1.0,
            memory_bytes: 1_024,
            virtual_memory_bytes: 4_096,
            threads: Some(7),
            parent_pid: Some(1),
        }
    }

    fn sample_group() -> AppGroupRow {
        let first = ProcessKey::new(100, 1_700_000_000);
        AppGroupRow {
            kind: AppRowKind::Group,
            key: "firefox".to_string(),
            label: "firefox".to_string(),
            index: 0,
            process_key: first,
            member_keys: vec![first, ProcessKey::new(101, 1_700_000_000)],
            depth: 0,
            has_children: true,
            expanded: false,
            cpu_percent: 12.5,
            memory_bytes: 3 * 1_024 * 1_024,
            process_count: 3,
        }
    }

    #[test]
    fn grouped_row_shows_totals_and_fixed_non_aggregatable_columns() {
        let group = sample_group();
        let representative = sample_process(100, "firefox");
        let view = process_row_values(Some(&group), &representative, false, 0, true);

        assert!(view.is_group);
        assert_eq!(view.group_key.as_deref(), Some("firefox"));
        assert_eq!(view.name, "firefox");
        assert_eq!(view.command, "3 プロセス");
        assert_eq!(view.cpu_percent, group.cpu_percent);
        assert_eq!(view.memory_bytes, group.memory_bytes);
        // The representative's single-process numbers never stand in for the sum.
        assert_ne!(view.cpu_percent, representative.cpu_percent);
        assert_ne!(view.memory_bytes, representative.memory_bytes);
        // No single member's identity reaches a group column.
        assert_eq!(view.pid, "—");
        assert_eq!(view.user, "—");
        assert_eq!(view.status_text, "—");
        assert_eq!(view.threads, "—");
        assert!(!view.command.contains("--flag"));
        assert!(!view.user.contains("alice"));
        assert_eq!(view.status, ProcessStatus::Unknown);
    }

    #[test]
    fn flat_tree_and_expanded_member_rows_keep_snapshot_values() {
        let process = sample_process(42, "emacs");
        let flat = process_row_values(None, &process, true, 0, false);
        let tree = process_row_values(None, &process, false, 1, true);
        for view in [&flat, &tree] {
            assert!(!view.is_group);
            assert_eq!(view.group_key, None);
            assert_eq!(view.name, process.name);
            assert_eq!(view.command, process.command);
            assert_eq!(view.cpu_percent, process.cpu_percent);
            assert_eq!(view.memory_bytes, process.memory_bytes);
            assert_eq!(view.pid, "42");
            assert_eq!(view.user, "alice");
            assert_eq!(view.status_text, "待機");
            assert_eq!(view.threads, "7");
            assert_eq!(view.status, process.status);
        }
        assert!(flat.selected);
        assert_eq!(tree.depth, 1);
        assert!(tree.has_children);

        // An expanded member has `kind = Process`: even when a group row is
        // supplied it must fall back to the snapshot, never to group totals.
        let member = AppGroupRow {
            kind: AppRowKind::Process,
            key: "firefox".to_string(),
            label: process.name.clone(),
            index: 7,
            process_key: process.key,
            member_keys: Vec::new(),
            depth: 1,
            has_children: false,
            expanded: false,
            cpu_percent: 99.0,
            memory_bytes: 9_999,
            process_count: 1,
        };
        let view = process_row_values(Some(&member), &process, false, 1, false);
        assert!(!view.is_group);
        assert_eq!(view.group_key, None);
        assert_eq!(view.cpu_percent, process.cpu_percent);
        assert_eq!(view.memory_bytes, process.memory_bytes);
        assert_eq!(view.command, process.command);
        assert_eq!(view.pid, "42");
    }

    #[test]
    fn group_accessible_name_states_totals_without_a_member_identity() {
        let label = a11y_group_row_label("firefox", 12.5, 3 * 1_024 * 1_024, 3, true);
        assert!(
            label.starts_with("アプリ firefox、3 プロセス、CPU 合計 12.5%"),
            "{label}"
        );
        assert!(label.contains("メモリ合計 3.0 MiB"), "{label}");
        assert!(label.ends_with("、選択中"), "{label}");
        assert!(!label.contains("PID"), "{label}");
        assert!(!label.contains("--flag"), "{label}");
    }

    #[test]
    fn focused_row_owns_enter_space_without_breaking_global_key_contracts() {
        assert!(is_row_activation("enter", plain(), false));
        assert!(is_row_activation("space", plain(), false));
        assert!(!is_row_activation("enter", plain(), true));
        assert!(!is_row_activation("space", ctrl(), false));
        assert!(!is_row_activation("space", cmd(), false));
        // Outside a focused row the old keys still mean exactly what they did.
        assert_eq!(
            key_action("space", plain(), false),
            Some(KeyAction::TogglePaused)
        );
        assert_eq!(
            key_action("down", plain(), false),
            Some(KeyAction::SelectRelative(1))
        );
        assert_eq!(dialog_command("enter", plain()), Some(Command::CancelKill));
    }

    #[test]
    fn help_advertises_group_expansion_and_keeps_existing_shortcuts() {
        let shortcuts = help_shortcuts();
        assert!(
            shortcuts.iter().any(|(key, description)| {
                key.contains("Enter") && description.contains("グループ展開/折りたたみ")
            }),
            "help must document group expansion: {shortcuts:?}"
        );
        assert!(shortcuts
            .iter()
            .any(|(key, description)| *key == "Space" && description.contains("一時停止")));
        assert!(shortcuts
            .iter()
            .any(|(key, description)| *key == "Enter" && description.contains("キャンセル")));
        assert!(shortcuts
            .iter()
            .any(|(key, _description)| *key == "Ctrl/Cmd+Enter"));
    }

    #[test]
    fn process_row_wires_group_activation_and_accesskit_semantics() {
        let source = include_str!("app.rs");
        let row = source
            .split("fn process_row(")
            .nth(1)
            .expect("process_row must exist")
            .split("fn details_panel(")
            .next()
            .expect("process_row slice must end at details_panel");
        assert!(row.contains(concat!("Command::Toggle", "AppGroup(group_key)")));
        assert!(row.contains(concat!("app_group_", "rows().get(index)")));
        assert!(row.contains(concat!("AppRowKind::", "Group")));
        assert!(row.contains(concat!("process_row_values(", "group")));
        assert!(row.contains(concat!(".aria_label(", "aria_label)")));
        assert!(row.contains(concat!(".aria_expanded(", "expanded)")));
        assert!(row.contains(concat!(".track_focus(&", "row_focus)")));
        assert!(row.contains(concat!(".aria_keyshortcuts(", "\"Enter, Space\")")));
        // Tree nodes keep their original select + toggle behaviour.
        assert!(row.contains(concat!("Command::Toggle", "TreeNode(key)")));
    }

    #[test]
    fn toggle_app_group_command_expands_and_collapses_group_members() {
        let mut dashboard = Dashboard::new(PlatformInfo::demo(), DEFAULT_REFRESH_INTERVAL);
        let mut snapshot = tm_core::demo::demo_snapshot(0);
        assert!(
            snapshot.processes.len() >= 2,
            "demo snapshot needs two processes"
        );
        // Group keys now derive from the executable in `command`, so copy
        // both the display name and the command to form a real multi-member
        // app group (R1-GROUPKEY).
        let first = snapshot.processes[0].clone();
        snapshot.processes[1].name = first.name.clone();
        snapshot.processes[1].command = first.command.clone();
        dashboard.ingest(snapshot);

        let group = dashboard
            .app_group_rows()
            .into_iter()
            .find(|row| row.kind == AppRowKind::Group && row.has_children)
            .expect("a multi-member group in demo data");
        let key = group.key.clone();
        let collapsed_count = dashboard.app_group_rows().len();
        dashboard.apply(Command::ToggleAppGroup(key.clone()));
        assert!(dashboard.is_app_group_expanded(&key));
        assert!(
            dashboard.app_group_rows().len() > collapsed_count,
            "expanding must reveal member rows"
        );
        dashboard.apply(Command::ToggleAppGroup(key.clone()));
        assert!(!dashboard.is_app_group_expanded(&key));
        assert_eq!(dashboard.app_group_rows().len(), collapsed_count);
    }

    #[test]
    fn grouped_display_does_not_change_the_export_twelve_column_contract() {
        use tm_core::redact::RedactionIdentifiers;

        let mut snapshot = tm_core::SystemSnapshot::default();
        for pid in [10, 20, 30] {
            snapshot.processes.push(ProcessSnapshot {
                key: ProcessKey::new(pid, 1),
                name: "firefox".to_string(),
                ..ProcessSnapshot::default()
            });
        }
        let before = snapshot.processes.clone();
        let group = sample_group();
        // Building an aggregate display row only borrows the snapshot.
        let _ = process_row_values(Some(&group), &snapshot.processes[0], false, 0, true);
        assert_eq!(snapshot.processes, before);

        let ids = RedactionIdentifiers {
            home: "<home>",
            user: "<user>",
            hostname: "<host>",
        };
        let csv = tm_core::export::snapshot_to_csv(&snapshot, &ids);
        for line in csv.lines() {
            assert_eq!(line.matches(',').count(), 11, "12 columns: {line}");
        }
    }
}

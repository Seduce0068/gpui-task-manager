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

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use tm_core::demo::DemoProbe;
use tm_core::format;
use tm_core::{ProcessKey, ProcessStatus, Signal, SortKey, SystemProbe, Timestamp};
use tm_viewmodel::{
    Command, Dashboard, EditOutcome, Effect, FilterInput, DEFAULT_REFRESH_INTERVAL,
};

use crate::settings::Settings;
use crate::theme::*;
use crate::ui::{action_button, card, kv, metric_bar, pill, section_title, sparkline};

const COL_CPU: f32 = 76.0;
const COL_MEMORY: f32 = 96.0;
const COL_PID: f32 = 64.0;
const COL_USER: f32 = 104.0;
const COL_STATUS: f32 = 78.0;
const COL_THREADS: f32 = 64.0;
const DETAILS_WIDTH: f32 = 320.0;

type SharedProbe = Arc<Mutex<Option<Box<dyn SystemProbe>>>>;

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
    demo_mode: bool,
    redact: bool,
}

impl TaskManagerApp {
    /// Creates the application with the requested backend.
    pub fn new(cx: &mut Context<Self>, demo: bool, redact: bool) -> Self {
        // The factory is the only place that knows which backend is in use, so
        // adding a replay or remote probe later does not touch the UI code.
        let (probe, init_error): (Box<dyn SystemProbe>, Option<String>) = if demo {
            (Box::new(DemoProbe::new()), None)
        } else {
            match tm_platform::NativeProbe::new() {
                Ok(probe) => (Box::new(probe), None),
                Err(error) => (
                    Box::new(DemoProbe::new()),
                    Some(format!(
                        "ネイティブ収集を初期化できなかったためデモデータで起動しました: {error}"
                    )),
                ),
            }
        };
        let platform = probe.platform_info();
        let demo_mode = platform.is_demo;
        let mut dashboard = Dashboard::new(platform, DEFAULT_REFRESH_INTERVAL);
        let had_init_error = init_error.is_some();
        if let Some(error) = init_error {
            // Startup failures must stay visible until the user acts.
            dashboard.set_persistent_status(error);
        }
        let (settings, notice) = crate::settings::load_with_notice();
        // A startup failure takes precedence; otherwise surface a settings
        // load/backup notice as a persistent status message.
        if !had_init_error {
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
            probe: Arc::new(Mutex::new(Some(probe))),
            sampling_in_flight: false,
            filter_editing: false,
            filter_input: FilterInput::new(),
            show_help: false,
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            demo_mode,
            redact,
        }
    }

    /// Focus handle used by the window and by keyboard shortcuts.
    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    /// Starts the sampling loop and requests the first sample.
    pub fn start(&mut self, cx: &mut Context<Self>) {
        self.start_timer(cx);
        self.handle_command(Command::Tick, cx);
        self.spawn_ime_debug_hook(cx);
        self.debug_save_settings_once();
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
            Ok(()) => eprintln!(
                "TM_DEBUG_SETTINGS: saved interval_ms={} key={:?} direction={:?} details={} path={:?}",
                settings.interval_ms,
                settings.sort.key,
                settings.sort.direction,
                settings.details_visible,
                crate::settings::config_path()
            ),
            Err(error) => eprintln!("TM_DEBUG_SETTINGS: save failed: {error}"),
        }
    }

    fn start_timer(&self, cx: &mut Context<Self>) {
        let background = cx.background_executor().clone();
        cx.spawn(async move |this, cx| loop {
            let interval = match this.update(cx, |this, _| this.dashboard.interval()) {
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
        if self.sampling_in_flight || self.dashboard.paused() {
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
    fn save_settings(&self) {
        let settings = Settings {
            version: crate::settings::CURRENT_VERSION,
            interval_ms: self.dashboard.interval().as_millis() as u64,
            sort: self.dashboard.sort_spec(),
            details_visible: self.dashboard.details_visible(),
            tree_view: self.dashboard.tree_view(),
            theme: crate::theme::theme(),
        };
        if let Err(error) = crate::settings::save(&settings) {
            eprintln!("gpui-task-manager: 設定を保存できません: {error}");
        }
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
        let control = event.keystroke.modifiers.control;

        if self.dashboard.pending_kill().is_some() {
            if let Some(command) = dialog_command(&key, control) {
                self.handle_command(command, cx);
            }
            return;
        }
        if self.show_help {
            if key == "escape" || key == "f1" {
                self.show_help = false;
                cx.notify();
            }
            return;
        }
        if self.filter_editing {
            match key.as_str() {
                // Esc: 絞り込みを解除して編集モードを抜ける。
                "escape" => {
                    self.filter_editing = false;
                    self.clear_filter(cx);
                    cx.stop_propagation();
                }
                // Enter: IME の確定はプラットフォーム側が行う。編集モードを抜ける。
                "enter" => {
                    self.end_filter_editing(cx);
                    cx.stop_propagation();
                }
                // Backspace はネイティブではプラットフォームが処理しないため、
                // ここで入力欄の状態を更新する（Web はブラウザ既定動作が担当）。
                "backspace" => {
                    let outcome = self.filter_input.backspace();
                    self.apply_input_outcome(outcome, cx);
                    cx.stop_propagation();
                }
                // 文字入力・貼り付け・IME 変換は EntityInputHandler が受ける。
                // ここで文字を足すと二重挿入になるので何もしない
                // （stop_propagation するとプラットフォーム側の挿入も止まる）。
                _ => {}
            }
            return;
        }

        match (control, key.as_str()) {
            (true, "f") => self.begin_filter_editing(window, cx),
            (true, "k") => {
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
            (false, "up") => self.handle_command(Command::SelectRelative(-1), cx),
            (false, "down") => self.handle_command(Command::SelectRelative(1), cx),
            (false, "pageup") => self.handle_command(Command::SelectRelative(-10), cx),
            (false, "pagedown") => self.handle_command(Command::SelectRelative(10), cx),
            (false, "home") => self.handle_command(Command::SelectRelative(-1_000_000), cx),
            (false, "end") => self.handle_command(Command::SelectRelative(1_000_000), cx),
            (false, "space") => {
                let paused = !self.dashboard.paused();
                self.handle_command(Command::SetPaused(paused), cx);
            }
            (false, "f2") => self.handle_command(Command::ToggleDetails, cx),
            (false, "f5") => self.handle_command(Command::Tick, cx),
            (false, "f1") => {
                self.show_help = true;
                cx.notify();
            }
            (false, "escape") => {
                if self.dashboard.filter().is_active() {
                    self.handle_command(Command::ClearFilter, cx);
                }
            }
            _ => {
                let _ = window;
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
                        this.child(pill("DEMO", palette().base, palette().yellow))
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
                            "Load {}",
                            format::format_load_average(summary.load)
                        )),
                ),
        )
        .h(px(200.0))
        .flex_1()
        .min_w_0();

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
                        .child(div().text_xs().text_color(rgb(palette().muted)).child(
                            format::format_used_of_total(summary.mem_used, summary.mem_total),
                        )),
                )
                .child(metric_bar(
                    summary.mem_percent,
                    usage_color(summary.mem_percent),
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
                .child(metric_bar(summary.swap_percent, palette().peach))
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
        .min_w_0();

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
        .min_w_0();

        let disk_rows: Vec<Div> = summary
            .disks
            .iter()
            .map(|disk| {
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
                    .child(metric_bar(disk.percent, usage_color(disk.percent)))
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
            for component in components.iter().take(3) {
                let label = if component.label.is_empty() {
                    "センサー".to_string()
                } else {
                    component.label.clone()
                };
                let value = match (
                    component.temperature_c,
                    component.max_c,
                    component.critical_c,
                ) {
                    (Some(t), Some(max), Some(crit)) => {
                        format!("{t:.0}°C (max {max:.0}°C / crit {crit:.0}°C)")
                    }
                    (Some(t), Some(max), None) => format!("{t:.0}°C (max {max:.0}°C)"),
                    (Some(t), None, Some(crit)) => format!("{t:.0}°C (crit {crit:.0}°C)"),
                    (Some(t), None, None) => format!("{t:.0}°C"),
                    _ => "—".to_string(),
                };
                system_body = system_body.child(kv(&label, value));
            }
        }

        let system_card = card("システム", palette().green, system_body)
            .h(px(200.0))
            .flex_1()
            .min_w_0();

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

        let filter_box = div()
            .id("filter-box")
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

    fn table_header(&mut self, cx: &mut Context<Self>) -> Div {
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
            let text = if active {
                format!("{label}{}", sort.direction.arrow())
            } else {
                (*label).to_string()
            };
            let key = *key;
            let mut cell = div()
                .id(("header", index))
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
                    this.focus_handle.focus(window, cx);
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

        let overlay = if self.dashboard.snapshot().is_none() {
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
        let (
            key,
            name,
            command,
            cpu,
            memory,
            pid,
            user,
            status,
            threads,
            selected,
            depth,
            has_children,
        ) = {
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
            (
                process.key,
                self.display(&process.name),
                process.command.clone(),
                process.cpu_percent,
                process.memory_bytes,
                process.key.pid,
                process.user.clone().unwrap_or_else(|| "—".to_string()),
                process.status,
                process.threads,
                self.dashboard.selected_key() == Some(process.key),
                depth,
                has_children,
            )
        };

        let tree_view = self.dashboard.tree_view();
        let collapsed = self.dashboard.is_collapsed(key);
        let user = self.display(&user);
        let command = self.display(&command);
        let marker = if tree_view && has_children {
            if collapsed {
                "▸ "
            } else {
                "▾ "
            }
        } else if tree_view && depth > 0 {
            "  "
        } else {
            ""
        };
        let display_name = if depth == 0 {
            format!("{marker}{name}")
        } else {
            format!("{}{}{}", "  ".repeat(depth.saturating_sub(1)), marker, name)
        };

        Some(
            div()
                .id(("row", index))
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
                    this.focus_handle.focus(window, cx);
                    if tree_view && has_children {
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
                        .text_color(rgb(usage_color(cpu.min(100.0))))
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
                        .child(pid.to_string()),
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
                    status.label(),
                    status_color(status),
                    palette().surface,
                )))
                .child(
                    div()
                        .w(px(COL_THREADS))
                        .text_xs()
                        .text_color(rgb(palette().muted))
                        .child(
                            threads
                                .map(|threads| threads.to_string())
                                .unwrap_or_else(|| "—".to_string()),
                        ),
                )
                .into_any_element(),
        )
    }

    fn details_panel(&mut self, cx: &mut Context<Self>) -> Div {
        let selected = self.dashboard.selected_process().cloned();
        let supports_control = self.dashboard.platform_info().supports_process_control;
        let mut content = v_flex().gap_2();

        match selected {
            Some(process) => {
                let key = process.key;
                let protected = self.dashboard.is_protected(key);
                let enabled = supports_control && !protected;
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
                        self.display(&process.user.clone().unwrap_or_else(|| "—".to_string())),
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
                                        enabled,
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
                                        enabled,
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
                                        enabled,
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
                                        enabled,
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
            for component in components {
                let mut value = component
                    .temperature_c
                    .map(|t| format!("{t:.0}°C"))
                    .unwrap_or_else(|| "—".to_string());
                if let Some(max) = component.max_c {
                    value.push_str(&format!(" / max {max:.0}°C"));
                }
                if let Some(critical) = component.critical_c {
                    value.push_str(&format!(" / crit {critical:.0}°C"));
                }
                let label = if component.label.is_empty() {
                    "センサー".to_string()
                } else {
                    component.label.clone()
                };
                content = content.child(kv(label, value));
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
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .min_h_0()
                    .child(header)
                    .child(list),
            )
            .children(details)
    }

    fn status_bar(&self) -> Div {
        let summary = self.summary();
        let platform = self.dashboard.platform_info().label();
        let status = self.dashboard.status().cloned();
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
            });
        let right = match status {
            Some(status) => div()
                .text_xs()
                .text_color(rgb(if status.is_error {
                    palette().red
                } else {
                    palette().green
                }))
                .truncate()
                .child(status.text)
                .into_any_element(),
            None if self.filter_editing => div()
                .text_xs()
                .text_color(rgb(palette().blue))
                .child("日本語 IME 入力可（変換中は青+下線）· Enter で終了 · Esc で解除")
                .into_any_element(),
            None => div()
                .text_xs()
                .text_color(rgb(palette().dim))
                .child("↑↓ 選択 · Space 一時停止 · Ctrl+F 検索 · Ctrl+K 終了 · F1 ヘルプ")
                .into_any_element(),
        };
        h_flex()
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
        let user = self.display(pending.user.as_deref().unwrap_or("—"));
        let command = self.display(&pending.command);
        let (color, label) = match pending.signal {
            Signal::Kill => (palette().red, "強制終了"),
            Signal::Terminate => (palette().red, "終了"),
            Signal::Stop => (palette().yellow, "一時停止"),
            Signal::Continue => (palette().green, "再開"),
        };
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
                        .child(
                            h_flex()
                                .justify_end()
                                .gap_2()
                                .child(action_button(
                                    "kill-cancel",
                                    "キャンセル",
                                    palette().surface_hi,
                                    true,
                                    cx.listener(|this, _event, window, cx| {
                                        this.focus_handle.focus(window, cx);
                                        this.handle_command(Command::CancelKill, cx);
                                    }),
                                ))
                                .child(action_button(
                                    "kill-confirm",
                                    label,
                                    color,
                                    true,
                                    cx.listener(|this, _event, window, cx| {
                                        this.focus_handle.focus(window, cx);
                                        this.handle_command(Command::ConfirmKill, cx);
                                    }),
                                )),
                        ),
                ),
        )
    }

    fn help_overlay(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        if !self.show_help {
            return None;
        }
        let shortcuts = [
            ("↑ / ↓", "行の選択"),
            ("PgUp / PgDn", "10 行移動"),
            ("Home / End", "先頭 / 末尾"),
            ("Space", "更新の一時停止・再開"),
            ("Ctrl+F", "絞り込み入力"),
            ("Ctrl+K", "選択プロセスの終了"),
            ("F2", "詳細パネルの表示切替"),
            ("F5", "今すぐ更新"),
            ("F1", "このヘルプ"),
            ("Enter", "確認ダイアログをキャンセル（安全側）"),
            ("Ctrl+Enter", "確認ダイアログでシグナルを実行"),
            ("Esc", "絞り込み解除 / ダイアログを閉じる"),
        ];
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
                                .child(action_button(
                                    "help-close",
                                    "閉じる",
                                    palette().surface_hi,
                                    true,
                                    cx.listener(|this, _event, window, cx| {
                                        this.focus_handle.focus(window, cx);
                                        this.show_help = false;
                                        cx.notify();
                                    }),
                                )),
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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

fn overlay_text(message: &'static str, color: u32) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(div().text_sm().text_color(rgb(color)).child(message))
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

/// Maps a key press while the confirmation dialog is open to a safe action.
///
/// A single Enter always cancels; confirming a signal requires Ctrl+Enter or an
/// explicit click on the confirm button. This is the safety-critical part of the
/// dialog: an accidental Enter must never terminate a process.
fn dialog_command(key: &str, control: bool) -> Option<Command> {
    match key {
        "escape" => Some(Command::CancelKill),
        "enter" if control => Some(Command::ConfirmKill),
        "enter" => Some(Command::CancelKill),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    // Explicit imports: the crate-root `use gpui_kit::*` re-exports a `test`
    // attribute macro that would shadow the built-in `#[test]` if `super::*`
    // were imported here.
    use super::dialog_command;
    use tm_viewmodel::Command;

    #[test]
    fn enter_cancels_the_confirmation_dialog() {
        assert_eq!(dialog_command("enter", false), Some(Command::CancelKill));
    }

    #[test]
    fn ctrl_enter_confirms_the_confirmation_dialog() {
        assert_eq!(dialog_command("enter", true), Some(Command::ConfirmKill));
    }

    #[test]
    fn escape_cancels_the_confirmation_dialog() {
        assert_eq!(dialog_command("escape", false), Some(Command::CancelKill));
    }

    #[test]
    fn other_keys_are_ignored_while_the_dialog_is_open() {
        assert_eq!(dialog_command("down", false), None);
        assert_eq!(dialog_command("space", false), None);
        assert_eq!(dialog_command("enter", true), Some(Command::ConfirmKill));
    }
}

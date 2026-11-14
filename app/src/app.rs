//! Native GPUI shell: window, overview cards, process table and details panel.
//!
//! The shell is intentionally thin. All presentation decisions that can be
//! tested live in `tm-viewmodel`; this file only renders [`Dashboard`] state and
//! forwards user intentions as [`Command`] values. The `SystemProbe` is kept
//! behind an `Arc<Mutex<...>>` and is only touched from the GPUI background
//! executor, so the UI thread never blocks on `/proc` reads.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use tm_core::demo::DemoProbe;
use tm_core::format;
use tm_core::{ProcessKey, ProcessStatus, Signal, SortKey, SystemProbe, Timestamp};
use tm_viewmodel::{Command, Dashboard, Effect, DEFAULT_REFRESH_INTERVAL};

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
        if let Some(error) = init_error {
            dashboard.set_status(error, true);
        }
        Self {
            dashboard,
            probe: Arc::new(Mutex::new(Some(probe))),
            sampling_in_flight: false,
            filter_editing: false,
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
            let _ = this.update(cx, |this, cx| {
                this.sampling_in_flight = false;
                match result {
                    Ok(snapshot) => this.dashboard.ingest(snapshot),
                    Err(error) => this.dashboard.set_status(error.to_string(), true),
                }
                cx.notify();
            });
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
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.dashboard.set_status(
                        format!("{} を送信しました（PID {}）", signal.short_label(), key.pid),
                        false,
                    ),
                    Err(error) => this.dashboard.set_status(error.to_string(), true),
                }
                this.request_sample(cx);
                cx.notify();
            });
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
        let effects = self.dashboard.apply(command);
        self.execute_effects(effects, cx);
        if let Some(position) = self.dashboard.selected_visible_index() {
            self.scroll_handle
                .scroll_to_item(position, ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.to_lowercase();
        let control = event.keystroke.modifiers.control;
        let alt = event.keystroke.modifiers.alt;

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
                "escape" => {
                    self.filter_editing = false;
                    self.handle_command(Command::ClearFilter, cx);
                }
                "backspace" => {
                    let mut text = self.dashboard.filter_text().to_string();
                    text.pop();
                    self.handle_command(Command::SetFilterText(text), cx);
                }
                "enter" => {
                    self.filter_editing = false;
                    cx.notify();
                }
                _ => {
                    if !control && !alt {
                        if let Some(character) = event.keystroke.key_char.as_deref() {
                            if !character.is_empty() {
                                let mut text = self.dashboard.filter_text().to_string();
                                text.push_str(character);
                                self.handle_command(Command::SetFilterText(text), cx);
                            }
                        }
                    }
                }
            }
            return;
        }

        match (control, key.as_str()) {
            (true, "f") => {
                self.filter_editing = true;
                cx.notify();
            }
            (true, "k") => {
                if let Some(process) = self.dashboard.selected_process() {
                    let key = process.key;
                    if !self.dashboard.is_protected(key) {
                        self.handle_command(
                            Command::RequestSignal {
                                key,
                                signal: Signal::Terminate,
                            },
                            cx,
                        );
                    }
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
            redact_text(value)
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
                .map(|host| {
                    if self.redact {
                        "host".to_string()
                    } else {
                        host
                    }
                })
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
                            mount: disk.mount_point.clone(),
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
        let pause_color = if paused { C_GREEN } else { C_BLUE };
        h_flex()
            .items_center()
            .justify_between()
            .px_4()
            .py_2()
            .bg(rgb(C_MANTLE))
            .border_b_1()
            .border_color(rgb(C_SURFACE))
            .child(
                h_flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(C_TEXT))
                            .child("GPUI Task Manager"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(C_MUTED))
                            .child(format!("{} · {}", summary.hostname, summary.kernel)),
                    )
                    .when(self.demo_mode, |this| {
                        this.child(pill("DEMO", C_BASE, C_YELLOW))
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
                        C_SURFACE_HI,
                        true,
                        cx.listener(|this, _event, window, cx| {
                            this.focus_handle.focus(window, cx);
                            this.handle_command(Command::ToggleDetails, cx);
                        }),
                    ))
                    .child(action_button(
                        "show-help",
                        "ヘルプ",
                        C_SURFACE_HI,
                        true,
                        cx.listener(|this, _event, window, cx| {
                            this.focus_handle.focus(window, cx);
                            this.show_help = true;
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
            C_BLUE,
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
                        .child(div().text_xs().text_color(rgb(C_MUTED)).child(format!(
                            "{} コア{}",
                            summary.cores.len(),
                            frequency
                        ))),
                )
                .child(sparkline(&summary.cpu_history, 100.0, C_BLUE, 34.0))
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
                .child(div().text_xs().text_color(rgb(C_MUTED)).child(format!(
                    "Load {}",
                    format::format_load_average(summary.load)
                ))),
        )
        .h(px(200.0))
        .flex_1()
        .min_w_0();

        let memory_card = card(
            "メモリ",
            C_MAUVE,
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
                        .child(div().text_xs().text_color(rgb(C_MUTED)).child(
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
                        .text_color(rgb(C_MUTED))
                        .child("スワップ")
                        .child(format::format_used_of_total(
                            summary.swap_used,
                            summary.swap_total,
                        )),
                )
                .child(metric_bar(summary.swap_percent, C_PEACH))
                .child(sparkline(&summary.mem_history, 100.0, C_MAUVE, 22.0))
                .child(div().text_xs().text_color(rgb(C_MUTED)).child(format!(
                    "空き {}",
                    format::format_bytes(summary.mem_total.saturating_sub(summary.mem_used))
                ))),
        )
        .h(px(200.0))
        .flex_1()
        .min_w_0();

        let network_card = card(
            "ネットワーク",
            C_TEAL,
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
                                .text_color(rgb(C_TEAL))
                                .child(format::format_rate(summary.rx_rate)),
                        )
                        .child(div().text_xs().text_color(rgb(C_MUTED)).child("受信")),
                )
                .child(sparkline(&summary.rx_history, max_rate, C_TEAL, 22.0))
                .child(
                    h_flex()
                        .items_end()
                        .justify_between()
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(C_PEACH))
                                .child(format::format_rate(summary.tx_rate)),
                        )
                        .child(div().text_xs().text_color(rgb(C_MUTED)).child("送信")),
                )
                .child(sparkline(&summary.tx_history, max_rate, C_PEACH, 14.0)),
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
                            .text_color(rgb(C_MUTED))
                            .child(disk.mount.clone())
                            .child(format!("{:.0}%", disk.percent)),
                    )
                    .child(metric_bar(disk.percent, usage_color(disk.percent)))
            })
            .collect();

        let system_card = card(
            "システム",
            C_GREEN,
            v_flex()
                .gap_2()
                .child(kv("ホスト", summary.hostname.clone()))
                .child(kv("稼働時間", format::format_duration(summary.uptime_secs)))
                .child(kv("プロセス", format::format_count(summary.process_count)))
                .children(disk_rows),
        )
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
        let filter_text = self.dashboard.filter_text().to_string();
        let sort = self.dashboard.sort_spec();
        let active_interval = self.dashboard.interval();

        let filter_box = div()
            .id("filter-box")
            .flex()
            .items_center()
            .gap_2()
            .w(px(300.0))
            .h(px(30.0))
            .px_3()
            .rounded_md()
            .bg(rgb(C_BASE))
            .border_1()
            .border_color(rgb(if editing { C_BLUE } else { C_SURFACE }))
            .cursor_pointer()
            .on_click(cx.listener(|this, _event, window, cx| {
                this.filter_editing = true;
                this.focus_handle.focus(window, cx);
                cx.notify();
            }))
            .child(div().text_xs().text_color(rgb(C_MUTED)).child("検索"))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(if filter_text.is_empty() {
                        C_DIM
                    } else {
                        C_TEXT
                    }))
                    .child(if filter_text.is_empty() {
                        "名前・コマンド・ユーザー・PID".to_string()
                    } else {
                        filter_text
                    }),
            )
            .when(editing, |this| {
                this.child(div().text_color(rgb(C_BLUE)).child("▌"))
            });

        let clear_button = action_button(
            "filter-clear",
            "クリア",
            C_SURFACE_HI,
            self.dashboard.filter().is_active(),
            cx.listener(|this, _event, window, cx| {
                this.focus_handle.focus(window, cx);
                this.filter_editing = false;
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
                if active { C_BLUE } else { C_SURFACE_HI },
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
            .bg(rgb(C_MANTLE))
            .border_b_1()
            .border_color(rgb(C_SURFACE))
            .child(filter_box)
            .child(clear_button)
            .child(div().text_xs().text_color(rgb(C_MUTED)).child(format!(
                "並び: {} {}",
                sort.key.label(),
                sort.direction.arrow()
            )))
            .child(h_flex().items_center().gap_1().children(interval_buttons))
            .child(div().flex_1())
            .child(div().text_xs().text_color(rgb(C_MUTED)).child(format!(
                "表示 {} / 全 {} 件",
                summary.visible_count, summary.process_count
            )))
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
                .text_color(rgb(if active { C_BLUE } else { C_MUTED }))
                .hover(|style| style.text_color(rgb(C_TEXT)))
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
            .bg(rgb(C_MANTLE))
            .border_b_1()
            .border_color(rgb(C_SURFACE))
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
            Some(overlay_text("システム情報を取得しています…", C_MUTED))
        } else if count == 0 {
            Some(overlay_text("一致するプロセスがありません", C_MUTED))
        } else {
            None
        };

        div()
            .flex_1()
            .min_h_0()
            .w_full()
            .relative()
            .bg(rgb(C_BASE))
            .child(list)
            .children(overlay)
    }

    fn process_row(&mut self, index: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (key, name, command, cpu, memory, pid, user, status, threads, selected) = {
            let snapshot = self.dashboard.snapshot()?;
            let row_index = *self.dashboard.visible_indices().get(index)?;
            let process = snapshot.processes.get(row_index)?;
            (
                process.key,
                process.name.clone(),
                process.command.clone(),
                process.cpu_percent,
                process.memory_bytes,
                process.key.pid,
                process.user.clone().unwrap_or_else(|| "—".to_string()),
                process.status,
                process.threads,
                self.dashboard.selected_key() == Some(process.key),
            )
        };

        let user = self.display(&user);
        let command = self.display(&command);

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
                .when(selected, |this| this.bg(rgb(C_SURFACE)))
                .when(!selected, |this| {
                    this.hover(|style| style.bg(rgb(C_MANTLE)))
                })
                .on_click(cx.listener(move |this, _event, window, cx| {
                    this.focus_handle.focus(window, cx);
                    this.handle_command(Command::Select(Some(key)), cx);
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
                                .text_color(rgb(if selected { C_TEXT } else { C_SUBTEXT }))
                                .max_w(px(200.0))
                                .truncate()
                                .child(name),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(C_DIM))
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
                        .text_color(rgb(C_SUBTEXT))
                        .child(format::format_bytes(memory)),
                )
                .child(
                    div()
                        .w(px(COL_PID))
                        .text_xs()
                        .text_color(rgb(C_MUTED))
                        .child(pid.to_string()),
                )
                .child(
                    div()
                        .w(px(COL_USER))
                        .text_xs()
                        .text_color(rgb(C_SUBTEXT))
                        .truncate()
                        .child(user),
                )
                .child(div().w(px(COL_STATUS)).child(pill(
                    status.label(),
                    status_color(status),
                    C_SURFACE,
                )))
                .child(
                    div()
                        .w(px(COL_THREADS))
                        .text_xs()
                        .text_color(rgb(C_MUTED))
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
                            .child(process.name.clone()),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(pill(
                                process.status.label(),
                                status_color(process.status),
                                C_SURFACE,
                            ))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(C_MUTED))
                                    .child(format!("PID {}", process.key.pid)),
                            ),
                    )
                    .child(kv(
                        "ユーザー",
                        process.user.clone().unwrap_or_else(|| "—".to_string()),
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
                    .child(div().text_xs().text_color(rgb(C_MUTED)).child("コマンド"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(C_SUBTEXT))
                            .child(self.display(&process.command)),
                    )
                    .child(div().h(px(1.0)).w_full().bg(rgb(C_SURFACE)))
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
                                        C_BLUE,
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
                                        C_RED,
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
                                        C_YELLOW,
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
                                        C_GREEN,
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
                                .text_color(rgb(C_YELLOW))
                                .child("この環境は読み取り専用です（デモ / ブラウザ）"),
                        )
                    })
                    .when(protected, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(rgb(C_YELLOW))
                                .child("PID 1 と自分自身は保護されています"),
                        )
                    });
            }
            None => {
                content = content.child(
                    div()
                        .text_sm()
                        .text_color(rgb(C_MUTED))
                        .child("行を選択すると詳細を表示します"),
                );
            }
        }
        v_flex()
            .w(px(DETAILS_WIDTH))
            .h_full()
            .bg(rgb(C_MANTLE))
            .border_1()
            .border_color(rgb(C_SURFACE))
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
            .text_color(rgb(C_MUTED))
            .child(platform)
            .child(div().text_color(rgb(C_DIM)).child("|"))
            .child(if summary.paused {
                "一時停止中".to_string()
            } else {
                format!("更新間隔 {} ms", summary.interval_ms)
            })
            .child(div().text_color(rgb(C_DIM)).child("|"))
            .child(match summary.sampled_at {
                Some(sampled_at) => format!("最終更新 {}", format_clock(sampled_at.as_secs())),
                None => "最終更新 —".to_string(),
            });
        let right = match status {
            Some(status) => div()
                .text_xs()
                .text_color(rgb(if status.is_error { C_RED } else { C_GREEN }))
                .truncate()
                .child(status.text)
                .into_any_element(),
            None => div()
                .text_xs()
                .text_color(rgb(C_DIM))
                .child("↑↓ 選択 · Space 一時停止 · Ctrl+F 検索 · Ctrl+K 終了 · F1 ヘルプ")
                .into_any_element(),
        };
        h_flex()
            .items_center()
            .justify_between()
            .gap_3()
            .px_3()
            .h(px(26.0))
            .bg(rgb(C_MANTLE))
            .border_t_1()
            .border_color(rgb(C_SURFACE))
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
        let user = pending.user.as_deref().unwrap_or("—").to_string();
        let command = self.display(&pending.command);
        let (color, label) = match pending.signal {
            Signal::Kill => (C_RED, "強制終了"),
            Signal::Terminate => (C_RED, "終了"),
            Signal::Stop => (C_YELLOW, "一時停止"),
            Signal::Continue => (C_GREEN, "再開"),
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
                .on_click(cx.listener(|_this, _event, _window, _cx| {}))
                .child(
                    v_flex()
                        .w(px(470.0))
                        .p_5()
                        .gap_3()
                        .rounded_lg()
                        .bg(rgb(C_MANTLE))
                        .border_1()
                        .border_color(rgb(C_SURFACE))
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
                                .text_color(rgb(C_SUBTEXT))
                                .child(format!(
                                    "「{}」（PID {} / ユーザー {} / 起動から {}）に {} を送信します。",
                                    pending.name,
                                    pending.key.pid,
                                    user,
                                    age,
                                    pending.signal.label()
                                )),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(C_MUTED))
                                .child("コマンド（対象確認用）"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(C_SUBTEXT))
                                .max_h(px(80.0))
                                .overflow_hidden()
                                .child(command),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(C_MUTED))
                                .child(
                                    "この操作は取り消せません。Enter でキャンセル、Ctrl+Enter または「実行」ボタンで確定します。",
                                ),
                        )
                        .child(
                            h_flex()
                                .justify_end()
                                .gap_2()
                                .child(action_button(
                                    "kill-cancel",
                                    "キャンセル",
                                    C_SURFACE_HI,
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
                            .bg(rgb(C_SURFACE))
                            .text_xs()
                            .text_color(rgb(C_TEXT))
                            .child(*keys),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(C_SUBTEXT))
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
                .on_click(cx.listener(|_this, _event, _window, _cx| {}))
                .child(
                    v_flex()
                        .w(px(520.0))
                        .p_5()
                        .gap_3()
                        .rounded_lg()
                        .bg(rgb(C_MANTLE))
                        .border_1()
                        .border_color(rgb(C_SURFACE))
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
                                    C_SURFACE_HI,
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
            .bg(rgb(C_BASE))
            .text_color(rgb(C_TEXT))
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
        ProcessStatus::Running => C_GREEN,
        ProcessStatus::Sleeping => C_SUBTEXT,
        ProcessStatus::Idle => C_MUTED,
        ProcessStatus::Stopped => C_YELLOW,
        ProcessStatus::Zombie => C_RED,
        ProcessStatus::Tracing => C_MAUVE,
        ProcessStatus::Unknown => C_DIM,
    }
}

/// Replaces the current home directory and user name with generic values.
fn redact_text(text: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default();
    let mut out = text.to_string();
    if !home.is_empty() {
        out = out.replace(&home, "/home/user");
    }
    if !user.is_empty() {
        out = out.replace(&user, "user");
    }
    out
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

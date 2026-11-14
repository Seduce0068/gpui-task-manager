//! Read-only browser shell: header, overview cards, process table, details panel
//! and status bar.
//!
//! All presentation state lives in [`tm_viewmodel::Dashboard`]; this module only
//! renders it and forwards user intentions as [`Command`] values, exactly like
//! the native shell. The differences are deliberate:
//!
//! - the browser is read-only, so no signal buttons exist and the platform is
//!   always reported as `supports_process_control = false`,
//! - data comes from [`tm_core::demo::DemoProbe`] or from the loopback agent,
//!   polled once per second by a single `cx.spawn` task,
//! - a small DOM hook (`body[data-gpui-proof]`, `document.title`) exposes the
//!   state to the headless-Chrome verification script, because everything else
//!   is drawn on a `<canvas>`.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, Context, Div, FocusHandle, KeyDownEvent, Render, ScrollStrategy, Stateful,
    UniformListScrollHandle, Window, div, prelude::*, px, rgb, uniform_list,
};
use tm_core::demo::DemoProbe;
use tm_core::format;
use tm_core::{PlatformInfo, ProcessSnapshot, ProcessStatus, SortDirection, SortKey, SystemProbe};
use tm_viewmodel::{Command, DEFAULT_REFRESH_INTERVAL, Dashboard, Effect};

use crate::agent::{AgentClient, AgentConfig};
use crate::theme::*;
use crate::widgets::{card, h_flex, kv, metric_bar, pill, section_title, sparkline, v_flex};

const FONT_SANS: &str = "IBM Plex Sans";

/// Fixed layout constants (CSS px). The CDP verification script recomputes click
/// coordinates from these same numbers, so keep them in sync.
const HEADER_HEIGHT: f32 = 52.0;
const OVERVIEW_HEIGHT: f32 = 186.0;
const TOOLBAR_HEIGHT: f32 = 36.0;
const STATUS_HEIGHT: f32 = 26.0;
const GAP: f32 = 12.0;
const ROOT_PAD: f32 = 12.0;

const COL_NAME: f32 = 170.0;
const COL_CPU: f32 = 68.0;
const COL_MEMORY: f32 = 84.0;
const COL_PID: f32 = 60.0;
const COL_USER: f32 = 96.0;
const COL_STATUS: f32 = 74.0;
const COL_THREADS: f32 = 64.0;
const DETAILS_WIDTH: f32 = 320.0;

/// How the shell gets its data.
enum DataSource {
    /// Deterministic synthetic data generated in the browser (`tm-core`).
    Demo(DemoProbe),
    /// Loopback `tm-agent` over HTTP (read-only).
    Live(Arc<AgentClient>),
}

/// Browser application state.
pub struct TaskManagerApp {
    dashboard: Dashboard,
    source: DataSource,
    focus_handle: FocusHandle,
    scroll_handle: UniformListScrollHandle,
    /// Number of processes on the agent's machine before `?limit=` truncation.
    live_total: Option<usize>,
    /// Last live-mode error, shown in the status bar and the table overlay.
    live_error: Option<String>,
    /// Number of snapshots ingested (used by the DOM proof hook).
    ticks: u64,
}

impl TaskManagerApp {
    /// Creates the shell. Live mode is selected by `?agent=` in the page URL.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);

        let (dashboard, source) = match agent_config_from_url() {
            Some(config) => {
                let http = cx.http_client();
                let client = Arc::new(AgentClient::new(config, http));
                // The browser can never signal processes, even in live mode.
                let platform = PlatformInfo {
                    os: "Linux (エージェント経由)".to_string(),
                    supports_process_control: false,
                    supports_network: true,
                    supports_disks: true,
                    is_demo: false,
                };
                let mut dashboard = Dashboard::new(platform, DEFAULT_REFRESH_INTERVAL);
                dashboard.set_status("エージェントに接続しています…", false);
                (dashboard, DataSource::Live(client))
            }
            None => {
                let probe = DemoProbe::new();
                let platform = probe.platform_info();
                (
                    Dashboard::new(platform, DEFAULT_REFRESH_INTERVAL),
                    DataSource::Demo(probe),
                )
            }
        };

        let app = Self {
            dashboard,
            source,
            focus_handle,
            scroll_handle: UniformListScrollHandle::new(),
            live_total: None,
            live_error: None,
            ticks: 0,
        };
        app.spawn_sampler(cx);
        app.sync_proof_dom();
        app
    }

    /// Starts the 1 s sampling loop.
    ///
    /// One task handles both modes. It awaits the current sample before sleeping,
    /// so at most one fetch is in flight (no request pile-up if the agent stalls).
    fn spawn_sampler(&self, cx: &mut Context<Self>) {
        let live = match &self.source {
            DataSource::Live(client) => Some(client.clone()),
            DataSource::Demo(_) => None,
        };
        cx.spawn(async move |this, cx| {
            loop {
                match &live {
                    Some(client) => {
                        let result = client.fetch_snapshot().await;
                        let alive = this.update(cx, |app, cx| {
                            app.handle_live_result(result);
                            cx.notify();
                        });
                        if alive.is_err() {
                            break;
                        }
                    }
                    None => {
                        let alive = this.update(cx, |app, cx| {
                            app.ingest_demo();
                            cx.notify();
                        });
                        if alive.is_err() {
                            break;
                        }
                    }
                }
                cx.background_executor()
                    .timer(Duration::from_millis(
                        DEFAULT_REFRESH_INTERVAL.as_millis() as u64
                    ))
                    .await;
            }
        })
        .detach();
    }

    /// Takes one sample from the deterministic demo probe.
    fn ingest_demo(&mut self) {
        if let DataSource::Demo(probe) = &mut self.source {
            match probe.sample() {
                Ok(snapshot) => {
                    self.ticks = self.ticks.saturating_add(1);
                    self.live_error = None;
                    self.dashboard.ingest(snapshot);
                }
                Err(error) => {
                    self.dashboard
                        .set_status(format!("デモデータの生成に失敗しました: {error}"), true);
                }
            }
        }
        self.sync_proof_dom();
    }

    /// Ingests a live response, or surfaces the error without falling back to demo.
    fn handle_live_result(&mut self, result: Result<crate::agent::SnapshotResponse, String>) {
        match result {
            Ok(response) => {
                self.ticks = self.ticks.saturating_add(1);
                self.live_total = Some(response.total_process_count);
                self.live_error = None;
                self.dashboard.ingest(response.snapshot);
                self.dashboard.clear_status();
            }
            Err(error) => {
                self.live_error = Some(error.clone());
                self.dashboard
                    .set_status(format!("エージェントから取得できません: {error}"), true);
            }
        }
        self.sync_proof_dom();
    }

    /// Applies a user intention and executes the effects the shell supports.
    fn handle_command(&mut self, command: Command, cx: &mut Context<Self>) {
        for effect in self.dashboard.apply(command) {
            match effect {
                // The sampling loop owns the cadence; a manual sample request is a no-op.
                Effect::SampleNow => {}
                Effect::Signal { .. } => self.dashboard.set_status(
                    "ブラウザ版は読み取り専用です。プロセス操作（シグナル送信）は行いません。",
                    true,
                ),
            }
        }
        if let Some(position) = self.dashboard.selected_visible_index() {
            self.scroll_handle
                .scroll_to_item(position, ScrollStrategy::Nearest);
        }
        self.sync_proof_dom();
        cx.notify();
    }

    /// Keyboard filter + selection navigation (the browser shell has no modal).
    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.control || keystroke.modifiers.alt || keystroke.modifiers.platform {
            return;
        }
        match keystroke.key.to_lowercase().as_str() {
            "backspace" => {
                let mut text = self.dashboard.filter_text().to_string();
                text.pop();
                self.handle_command(Command::SetFilterText(text), cx);
            }
            "escape" => self.handle_command(Command::ClearFilter, cx),
            "up" => self.handle_command(Command::SelectRelative(-1), cx),
            "down" => self.handle_command(Command::SelectRelative(1), cx),
            "pageup" => self.handle_command(Command::SelectRelative(-10), cx),
            "pagedown" => self.handle_command(Command::SelectRelative(10), cx),
            "home" => self.handle_command(Command::SelectRelative(-1_000_000), cx),
            "end" => self.handle_command(Command::SelectRelative(1_000_000), cx),
            _ => {
                if let Some(text) = keystroke.key_char.as_deref()
                    && !text.is_empty()
                    && !keystroke.modifiers.function
                {
                    let mut filter = self.dashboard.filter_text().to_string();
                    filter.push_str(text);
                    self.handle_command(Command::SetFilterText(filter), cx);
                }
            }
        }
    }

    // ---------------------------------------------------------------- header

    fn header(&self) -> Div {
        let snapshot = self.dashboard.snapshot();
        let hostname = snapshot
            .and_then(|snapshot| snapshot.hostname.clone())
            .unwrap_or_else(|| "—".to_string());
        let os = snapshot
            .and_then(|snapshot| snapshot.os_name.clone())
            .unwrap_or_else(|| "—".to_string());
        let kernel = snapshot
            .and_then(|snapshot| snapshot.kernel_version.clone())
            .unwrap_or_else(|| "—".to_string());
        let (mode_label, mode_color) = match &self.source {
            DataSource::Demo(_) => ("デモ", C_YELLOW),
            DataSource::Live(_) => ("ライブ", C_TEAL),
        };

        h_flex()
            .justify_between()
            .h(px(HEADER_HEIGHT))
            .flex_shrink_0()
            .child(
                v_flex()
                    .items_start()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child("GPUI タスクマネージャー"),
                            )
                            .child(pill("ブラウザ版", C_BASE, C_BLUE)),
                    )
                    .child(div().text_xs().text_color(rgb(C_MUTED)).child(
                        "gpui-pre 0.3.12 / wasm32-unknown-unknown / wgpu (WebGPU→WebGL2 fallback)",
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        v_flex()
                            .items_end()
                            .gap_1()
                            .child(div().text_sm().text_color(rgb(C_TEXT)).child(hostname))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(C_MUTED))
                                    .child(format!("{os} / {kernel}")),
                            ),
                    )
                    .child(pill(mode_label, C_BASE, mode_color))
                    .child(pill("読み取り専用", C_BASE, C_RED)),
            )
    }

    // -------------------------------------------------------------- overview

    fn overview(&self) -> Div {
        let snapshot = self.dashboard.snapshot();
        let history = self.dashboard.history();

        let cpu_total = snapshot.map(|s| s.cpu.total_percent).unwrap_or(0.0);
        let cores: Vec<f32> = snapshot
            .map(|s| s.cpu.per_core_percent.clone())
            .unwrap_or_default();
        let load = snapshot.map(|s| s.cpu.load_average).unwrap_or([0.0; 3]);
        let frequency = snapshot.and_then(|s| s.cpu.frequency_mhz);
        let cpu_history = history.cpu_percent.to_vec();

        let memory = snapshot.map(|s| &s.memory);
        let mem_percent = memory.map(|m| m.used_percent()).unwrap_or(0.0);
        let mem_used = memory.map(|m| m.used_bytes).unwrap_or(0);
        let mem_total = memory.map(|m| m.total_bytes).unwrap_or(0);
        let mem_free = memory.map(|m| m.available_bytes).unwrap_or(0);
        let swap_percent = memory.map(|m| m.swap_used_percent()).unwrap_or(0.0);
        let swap_used = memory.map(|m| m.swap_used_bytes).unwrap_or(0);
        let swap_total = memory.map(|m| m.swap_total_bytes).unwrap_or(0);
        let mem_history = history.memory_percent.to_vec();

        let (rx_rate, tx_rate) = self.dashboard.network_rates();
        let rx_history: Vec<f32> = history
            .network_rx_rate
            .iter()
            .map(|&value| value as f32)
            .collect();
        let tx_history: Vec<f32> = history
            .network_tx_rate
            .iter()
            .map(|&value| value as f32)
            .collect();
        let max_rate = rx_history
            .iter()
            .chain(tx_history.iter())
            .copied()
            .fold(1_024.0_f32, f32::max);

        let hostname = snapshot
            .and_then(|s| s.hostname.clone())
            .unwrap_or_else(|| "—".to_string());
        let uptime = snapshot.map(|s| s.cpu.uptime_secs).unwrap_or(0);
        let process_count = snapshot.map(|s| s.process_count()).unwrap_or(0);
        let disks: Vec<(String, f32)> = snapshot
            .map(|s| {
                s.disks
                    .iter()
                    .take(2)
                    .map(|disk| (disk.mount_point.clone(), disk.used_percent()))
                    .collect()
            })
            .unwrap_or_default();

        let frequency_label = frequency
            .map(|mhz| format!(" / {:.2} GHz", mhz as f32 / 1_000.0))
            .unwrap_or_default();

        let cpu_card = card(
            "CPU",
            C_BLUE,
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .justify_between()
                        .items_end()
                        .child(
                            div()
                                .text_xl()
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(rgb(usage_color(cpu_total)))
                                .child(format::format_percent(cpu_total)),
                        )
                        .child(div().text_xs().text_color(rgb(C_MUTED)).child(format!(
                            "{} コア{}",
                            cores.len(),
                            frequency_label
                        ))),
                )
                .child(sparkline(&cpu_history, 100.0, C_BLUE, 28.0))
                .child(
                    h_flex().items_end().gap(px(2.0)).h(px(18.0)).children(
                        cores
                            .iter()
                            .map(|&percent| {
                                div()
                                    .flex_1()
                                    .h(px(((percent / 100.0) * 18.0).max(1.0)))
                                    .rounded_sm()
                                    .bg(rgb(usage_color(percent)))
                            })
                            .collect::<Vec<_>>(),
                    ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(C_MUTED))
                        .child(format!("Load {}", format::format_load_average(load))),
                ),
        )
        .h_full()
        .flex_1()
        .min_w_0();

        let memory_card = card(
            "メモリ",
            C_MAUVE,
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .justify_between()
                        .items_end()
                        .child(
                            div()
                                .text_xl()
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(rgb(usage_color(mem_percent)))
                                .child(format::format_percent(mem_percent)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(C_MUTED))
                                .child(format::format_used_of_total(mem_used, mem_total)),
                        ),
                )
                .child(metric_bar(mem_percent, usage_color(mem_percent)))
                .child(
                    h_flex()
                        .justify_between()
                        .text_xs()
                        .text_color(rgb(C_MUTED))
                        .child("スワップ")
                        .child(format::format_used_of_total(swap_used, swap_total)),
                )
                .child(metric_bar(swap_percent, C_PEACH))
                .child(sparkline(&mem_history, 100.0, C_MAUVE, 18.0))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(C_MUTED))
                        .child(format!("空き {}", format::format_bytes(mem_free))),
                ),
        )
        .h_full()
        .flex_1()
        .min_w_0();

        let network_card = card(
            "ネットワーク",
            C_TEAL,
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .justify_between()
                        .items_end()
                        .child(
                            div()
                                .text_xl()
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(rgb(C_TEAL))
                                .child(format::format_rate(rx_rate)),
                        )
                        .child(div().text_xs().text_color(rgb(C_MUTED)).child("受信")),
                )
                .child(sparkline(&rx_history, max_rate, C_TEAL, 20.0))
                .child(
                    h_flex()
                        .justify_between()
                        .items_end()
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(C_PEACH))
                                .child(format::format_rate(tx_rate)),
                        )
                        .child(div().text_xs().text_color(rgb(C_MUTED)).child("送信")),
                )
                .child(sparkline(&tx_history, max_rate, C_PEACH, 12.0)),
        )
        .h_full()
        .flex_1()
        .min_w_0();

        let mut system_body = v_flex()
            .gap_1()
            .child(kv("ホスト", hostname))
            .child(kv("稼働時間", format::format_duration(uptime)))
            .child(kv("プロセス", format::format_count(process_count)));
        for (mount, percent) in disks {
            system_body = system_body.child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .justify_between()
                            .text_xs()
                            .text_color(rgb(C_MUTED))
                            .child(mount)
                            .child(format!("{percent:.0}%")),
                    )
                    .child(metric_bar(percent, usage_color(percent))),
            );
        }
        let system_card = card("システム", C_GREEN, system_body)
            .h_full()
            .flex_1()
            .min_w_0();

        div()
            .flex()
            .gap_3()
            .h(px(OVERVIEW_HEIGHT))
            .flex_shrink_0()
            .child(cpu_card)
            .child(memory_card)
            .child(network_card)
            .child(system_card)
    }

    // --------------------------------------------------------------- toolbar

    fn toolbar(&self, focused: bool, cx: &mut Context<Self>) -> Div {
        let filter_text = self.dashboard.filter_text().to_string();
        let sort = self.dashboard.sort_spec();
        let filter_box = div()
            .id("filter-box")
            .flex()
            .items_center()
            .gap_2()
            .w(px(340.0))
            .h(px(30.0))
            .px_3()
            .rounded_md()
            .bg(rgb(C_MANTLE))
            .border_1()
            .border_color(rgb(if focused { C_BLUE } else { C_SURFACE }))
            .cursor_pointer()
            .on_click(cx.listener(|this, _event, window, cx| {
                this.focus_handle.focus(window, cx);
                cx.notify();
            }))
            .child(div().text_xs().text_color(rgb(C_MUTED)).child("検索"))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(rgb(if filter_text.is_empty() {
                        C_DIM
                    } else {
                        C_TEXT
                    }))
                    .truncate()
                    .child(if filter_text.is_empty() {
                        "名前・コマンド・ユーザー・PID".to_string()
                    } else {
                        filter_text
                    }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(if focused { C_BLUE } else { C_MUTED }))
                    .child(if focused {
                        "入力可"
                    } else {
                        "クリックで入力"
                    }),
            );

        h_flex()
            .justify_between()
            .h(px(TOOLBAR_HEIGHT))
            .flex_shrink_0()
            .child(filter_box)
            .child(
                h_flex()
                    .gap_3()
                    .child(div().text_xs().text_color(rgb(C_MUTED)).child(format!(
                        "ソート: {} {}",
                        sort.key.label(),
                        sort.direction.arrow()
                    )))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(C_MUTED))
                            .child(format!("表示 {} 件", self.dashboard.visible_count())),
                    ),
            )
    }

    // ------------------------------------------------------------- main area

    fn main_area(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .gap_3()
            .flex_1()
            .min_h(px(0.0))
            .child(self.table(cx))
            .child(self.details_panel())
    }

    fn table(&self, cx: &mut Context<Self>) -> Div {
        let count = self.dashboard.visible_count();
        let list = uniform_list(
            "process-list",
            count,
            cx.processor(move |this: &mut Self, range: Range<usize>, _window, cx| {
                range
                    .filter_map(|index| this.process_row(index, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .h_full()
        .track_scroll(&self.scroll_handle);

        let overlay = if self.dashboard.snapshot().is_none() {
            Some(overlay_text(self.empty_message(), C_MUTED))
        } else if count == 0 {
            Some(overlay_text(
                "一致するプロセスがありません".to_string(),
                C_MUTED,
            ))
        } else {
            None
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .rounded_md()
            .border_1()
            .border_color(rgb(C_SURFACE))
            .overflow_hidden()
            .bg(rgb(C_BASE))
            .child(self.table_header(cx))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .relative()
                    .child(list)
                    .children(overlay),
            )
    }

    fn empty_message(&self) -> String {
        if self.live_error.is_some() {
            "エージェントに接続できません（デモへはフォールバックしません）".to_string()
        } else {
            "データを取得しています…".to_string()
        }
    }

    fn table_header(&self, cx: &mut Context<Self>) -> Div {
        h_flex()
            .gap_2()
            .h(px(30.0))
            .px_3()
            .flex_shrink_0()
            .bg(rgb(C_MANTLE))
            .border_b_1()
            .border_color(rgb(C_SURFACE))
            .child(
                self.sort_cell("sort-name", "名前", SortKey::Name, cx)
                    .w(px(COL_NAME))
                    .flex_shrink_0(),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(rgb(C_MUTED))
                    .child("コマンド"),
            )
            .child(
                self.sort_cell("sort-cpu", "CPU", SortKey::Cpu, cx)
                    .w(px(COL_CPU))
                    .flex_shrink_0(),
            )
            .child(
                self.sort_cell("sort-memory", "メモリ", SortKey::Memory, cx)
                    .w(px(COL_MEMORY))
                    .flex_shrink_0(),
            )
            .child(
                self.sort_cell("sort-pid", "PID", SortKey::Pid, cx)
                    .w(px(COL_PID))
                    .flex_shrink_0(),
            )
            .child(
                self.sort_cell("sort-user", "ユーザー", SortKey::User, cx)
                    .w(px(COL_USER))
                    .flex_shrink_0(),
            )
            .child(
                self.sort_cell("sort-status", "状態", SortKey::Status, cx)
                    .w(px(COL_STATUS))
                    .flex_shrink_0(),
            )
            .child(
                self.sort_cell("sort-threads", "スレッド", SortKey::Threads, cx)
                    .w(px(COL_THREADS))
                    .flex_shrink_0(),
            )
    }

    fn sort_cell(
        &self,
        id: &'static str,
        label: &'static str,
        key: SortKey,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let sort = self.dashboard.sort_spec();
        let active = sort.key == key;
        let text = if active {
            format!("{label} {}", sort.direction.arrow())
        } else {
            label.to_string()
        };
        div()
            .id(id)
            .flex()
            .items_center()
            .cursor_pointer()
            .text_xs()
            .text_color(rgb(if active { C_BLUE } else { C_MUTED }))
            .hover(|style| style.text_color(rgb(C_TEXT)))
            .on_click(cx.listener(move |this, _event, _window, cx| {
                this.handle_command(Command::SetSortKey(key), cx);
            }))
            .child(text)
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
                process.threads.unwrap_or(0),
                self.dashboard.selected_key() == Some(process.key),
            )
        };

        Some(
            div()
                .id(("proc-row", index))
                .flex()
                .items_center()
                .w_full()
                .h(px(ROW_HEIGHT))
                .px_3()
                .gap_2()
                .cursor_pointer()
                .when(selected, |this| this.bg(rgb(C_SURFACE)))
                .hover(|style| style.bg(rgb(C_MANTLE)))
                .on_click(cx.listener(move |this, _event, window, cx| {
                    this.focus_handle.focus(window, cx);
                    this.handle_command(Command::Select(Some(key)), cx);
                }))
                .child(
                    div()
                        .w(px(COL_NAME))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap_2()
                        .overflow_hidden()
                        .child(
                            div()
                                .w(px(6.0))
                                .h(px(6.0))
                                .flex_shrink_0()
                                .rounded_full()
                                .bg(rgb(status_color(status))),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .text_sm()
                                .text_color(rgb(if selected { C_TEXT } else { C_SUBTEXT }))
                                .truncate()
                                .child(name),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_xs()
                        .text_color(rgb(C_DIM))
                        .truncate()
                        .child(command),
                )
                .child(
                    div()
                        .w(px(COL_CPU))
                        .flex_shrink_0()
                        .text_sm()
                        .text_color(rgb(usage_color(cpu.min(100.0))))
                        .child(format::format_cpu_percent(cpu)),
                )
                .child(
                    div()
                        .w(px(COL_MEMORY))
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(C_SUBTEXT))
                        .child(format::format_bytes(memory)),
                )
                .child(
                    div()
                        .w(px(COL_PID))
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(C_MUTED))
                        .child(pid.to_string()),
                )
                .child(
                    div()
                        .w(px(COL_USER))
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(C_SUBTEXT))
                        .truncate()
                        .child(user),
                )
                .child(div().w(px(COL_STATUS)).flex_shrink_0().child(pill(
                    status.label(),
                    status_color(status),
                    C_SURFACE,
                )))
                .child(
                    div()
                        .w(px(COL_THREADS))
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(C_MUTED))
                        .child(threads.to_string()),
                )
                .into_any_element(),
        )
    }

    fn details_panel(&self) -> Div {
        let selected = self.dashboard.selected_process().cloned();
        let now = self
            .dashboard
            .snapshot()
            .map(|snapshot| snapshot.sampled_at.as_secs())
            .unwrap_or(0);

        let content = match selected {
            Some(process) => self.details_content(&process, now),
            None => v_flex().gap_2().child(
                div()
                    .text_sm()
                    .text_color(rgb(C_MUTED))
                    .child("行をクリックすると詳細を表示します"),
            ),
        };

        v_flex()
            .w(px(DETAILS_WIDTH))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(C_MANTLE))
            .border_1()
            .border_color(rgb(C_SURFACE))
            .rounded_md()
            .p_4()
            .gap_3()
            .child(section_title("プロセス詳細"))
            .child(
                div()
                    .id("details-scroll")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .child(content),
            )
    }

    fn details_content(&self, process: &ProcessSnapshot, now: u64) -> Div {
        let user = process.user.clone().unwrap_or_else(|| "—".to_string());
        let threads = process
            .threads
            .map(|threads| threads.to_string())
            .unwrap_or_else(|| "—".to_string());
        let parent = process
            .parent_pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "—".to_string());

        v_flex()
            .gap_2()
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .truncate()
                    .child(process.name.clone()),
            )
            .child(
                h_flex()
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
            .child(kv("ユーザー", user))
            .child(kv("CPU", format::format_cpu_percent(process.cpu_percent)))
            .child(kv(
                "メモリ (RSS)",
                format::format_bytes(process.memory_bytes),
            ))
            .child(kv(
                "仮想メモリ",
                format::format_bytes(process.virtual_memory_bytes),
            ))
            .child(kv("スレッド", threads))
            .child(kv("親 PID", parent))
            .child(kv("起動時刻", format_clock(process.key.start_time_secs)))
            .child(kv(
                "起動から",
                format::format_process_age(now, process.key.start_time_secs),
            ))
            .child(div().h(px(1.0)).w_full().bg(rgb(C_SURFACE)))
            .child(section_title("コマンド"))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(C_SUBTEXT))
                    .child(process.command.clone()),
            )
            .child(div().h(px(1.0)).w_full().bg(rgb(C_SURFACE)))
            .child(div().text_xs().text_color(rgb(C_YELLOW)).child(
                "ブラウザ版は読み取り専用です。プロセス操作（シグナル送信）は提供しません。",
            ))
    }

    // ------------------------------------------------------------ status bar

    fn status_bar(&self) -> Div {
        let snapshot = self.dashboard.snapshot();
        let source_label = match &self.source {
            DataSource::Demo(_) => "データ源: デモ (tm-core DemoProbe)".to_string(),
            DataSource::Live(client) => format!(
                "データ源: エージェント {} (読み取り専用)",
                client.endpoint_label()
            ),
        };
        let last_update = snapshot
            .map(|snapshot| format!("最終更新 {}", format_clock(snapshot.sampled_at.as_secs())))
            .unwrap_or_else(|| "最終更新 —".to_string());
        let counts = match (snapshot, self.live_total) {
            (Some(snapshot), Some(total)) => format!(
                "表示 {} / 取得 {} / 全 {} 件",
                self.dashboard.visible_count(),
                snapshot.process_count(),
                total
            ),
            (Some(snapshot), None) => format!(
                "表示 {} / 全 {} 件",
                self.dashboard.visible_count(),
                snapshot.process_count()
            ),
            (None, _) => "表示 0 / 全 0 件".to_string(),
        };
        let (message, is_error) = match self.dashboard.status() {
            Some(status) => (truncate_label(&status.text, 160), status.is_error),
            None => (
                "文字入力で絞り込み · ↑↓ 選択 · Esc クリア · 列クリックでソート".to_string(),
                false,
            ),
        };

        h_flex()
            .justify_between()
            .gap_3()
            .px_3()
            .h(px(STATUS_HEIGHT))
            .flex_shrink_0()
            .rounded_md()
            .bg(rgb(C_MANTLE))
            .border_1()
            .border_color(rgb(C_SURFACE))
            .child(
                h_flex()
                    .gap_2()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(rgb(C_MUTED))
                    .child(source_label)
                    .child(div().text_color(rgb(C_DIM)).child("|"))
                    .child(last_update)
                    .child(div().text_color(rgb(C_DIM)).child("|"))
                    .child(counts),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(rgb(if is_error { C_RED } else { C_DIM }))
                    .truncate()
                    .text_right()
                    .child(message),
            )
    }

    // -------------------------------------------------------------- DOM proof

    /// Writes a compact state summary into `document.title` and
    /// `body[data-gpui-proof]` so the CDP verification script can assert on it.
    ///
    /// This is the only DOM interaction in the shell; everything else is drawn on
    /// the GPUI canvas.
    #[cfg(target_family = "wasm")]
    fn sync_proof_dom(&self) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let Some(document) = window.document() else {
            return;
        };
        let snapshot = self.dashboard.snapshot();
        let mode = match &self.source {
            DataSource::Demo(_) => "demo",
            DataSource::Live(_) => "live",
        };
        let (cpu, mem, total, sampled) = match snapshot {
            Some(snapshot) => (
                snapshot.cpu.total_percent,
                snapshot.memory.used_percent(),
                snapshot.process_count(),
                snapshot.sampled_at.as_secs(),
            ),
            None => (0.0, 0.0, 0, 0),
        };
        let sort = self.dashboard.sort_spec();
        let direction = match sort.direction {
            SortDirection::Ascending => "asc",
            SortDirection::Descending => "desc",
        };
        let selected = self
            .dashboard
            .selected_key()
            .map(|key| key.pid.to_string())
            .unwrap_or_else(|| "-".to_string());
        let filter = self.dashboard.filter_text().replace(';', ",");
        let viewport_width = window
            .inner_width()
            .ok()
            .and_then(|value| value.as_f64())
            .map(|value| value as u32)
            .unwrap_or(0);
        let viewport_height = window
            .inner_height()
            .ok()
            .and_then(|value| value.as_f64())
            .map(|value| value as u32)
            .unwrap_or(0);
        let rows = self.dashboard.visible_count();
        let proof = format!(
            "mode={mode};tick={};cpu={cpu:.1};mem={mem:.1};rows={rows};total={total};\
             filter={filter};selected_pid={selected};sort={};dir={direction};\
             sampled={sampled};vw={viewport_width};vh={viewport_height};live_error={};",
            self.ticks,
            sort.key.label(),
            if self.live_error.is_some() { 1 } else { 0 },
        );
        document.set_title(&format!(
            "GPUI タスクマネージャー (Web) | mode={mode} tick={} rows={rows} selected={selected} filter={filter}",
            self.ticks
        ));
        if let Some(body) = document.body() {
            let _ = body.set_attribute("data-gpui-proof", &proof);
        }
    }

    /// Native builds have no DOM; the hook is a no-op there.
    #[cfg(not(target_family = "wasm"))]
    fn sync_proof_dom(&self) {}
}

impl Render for TaskManagerApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);
        let header = self.header();
        let overview = self.overview();
        let toolbar = self.toolbar(focused, cx);
        let main_area = self.main_area(cx);
        let status_bar = self.status_bar();

        div()
            .id("tm-web-root")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_click(cx.listener(|this, _event, window, cx| {
                this.focus_handle.focus(window, cx);
            }))
            .size_full()
            .flex()
            .flex_col()
            .gap(px(GAP))
            .p(px(ROOT_PAD))
            .bg(rgb(C_BASE))
            .text_color(rgb(C_TEXT))
            .font_family(FONT_SANS)
            .child(header)
            .child(overview)
            .child(toolbar)
            .child(main_area)
            .child(status_bar)
    }
}

// ------------------------------------------------------------------ helpers

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

/// Truncates a status-bar label on a character boundary (the bar is one line).
fn truncate_label(text: &str, max_chars: usize) -> String {
    // ステータスバーは 1 行なので、改行や連続空白は 1 個の空白へ畳む。
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

fn overlay_text(message: String, color: u32) -> Div {
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

/// Reads `?agent=<url>#token=<token>` from the page URL.
///
/// The endpoint comes from the query string; the token comes from the URL
/// **fragment**, which browsers do not send to the server and do not include in
/// the `Referer` header. The token is only kept in memory: it is never written
/// to the DOM, the title, the console or the agent request URL.
fn agent_config_from_url() -> Option<AgentConfig> {
    #[cfg(target_family = "wasm")]
    {
        let location = web_sys::window()?.location();
        let search = location.search().ok()?;
        if search.is_empty() {
            return None;
        }
        let params = web_sys::UrlSearchParams::new_with_str(&search).ok()?;
        let endpoint = params.get("agent")?;
        let endpoint = endpoint.trim().to_string();
        if endpoint.is_empty() {
            return None;
        }
        let token = location
            .hash()
            .ok()
            .and_then(|hash| hash.strip_prefix('#').map(str::to_string))
            .and_then(|fragment| {
                web_sys::UrlSearchParams::new_with_str(&fragment)
                    .ok()
                    .and_then(|params| params.get("token"))
            });
        Some(AgentConfig::new(endpoint, token))
    }
    #[cfg(not(target_family = "wasm"))]
    {
        None
    }
}

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
    AnyElement, Bounds, Context, Div, ElementInputHandler, EntityInputHandler, FocusHandle,
    KeyDownEvent, Pixels, Render, ScrollStrategy, Stateful, TextInputConfiguration, UTF16Selection,
    UniformListScrollHandle, Window, canvas, div, prelude::*, px, rgb, uniform_list,
};
use tm_core::demo::DemoProbe;
use tm_core::format;
use tm_core::{PlatformInfo, ProcessSnapshot, ProcessStatus, SortDirection, SortKey, SystemProbe};
use tm_viewmodel::{
    Command, DEFAULT_REFRESH_INTERVAL, Dashboard, EditOutcome, Effect, FilterInput,
};

use crate::agent::{AgentClient, AgentConfig};
use crate::theme::*;
use crate::widgets::{card, h_flex, kv, metric_bar, pill, section_title, sparkline, v_flex};

const FONT_SANS: &str = "IBM Plex Sans";

/// Fixed layout constants (CSS px). The CDP verification script recomputes click
/// coordinates from these same numbers, so keep them in sync.
const HEADER_HEIGHT: f32 = 52.0;
/// 概要カードの高さ。システムカードは最大でホスト/稼働時間/プロセス + ディスク 2 件
/// （ラベル+バー）+ 温度 3 件を縦に並べるため、186 px では内容がはみ出して
/// ツールバーと重なる（Runtime Proof: 2026-11-16 のスクリーンショットで確認）。
const OVERVIEW_HEIGHT: f32 = 260.0;
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
    /// 絞り込み欄の編集状態（IME の未確定文字列を含む）。
    ///
    /// 確定済み文字列だけを `dashboard` に反映する。GPUI の
    /// [`EntityInputHandler`] から届く UTF-16 の編集はすべてここを通る。
    filter_input: FilterInput,
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
                let platform = PlatformInfo::native_read_only("Linux (エージェント経由)");
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

        let mut app = Self {
            dashboard,
            filter_input: FilterInput::new(),
            source,
            focus_handle,
            scroll_handle: UniformListScrollHandle::new(),
            live_total: None,
            live_error: None,
            ticks: 0,
        };
        app.restore_settings();
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

    /// 保存された表示設定（更新間隔・ソート・詳細パネル）を復元する。
    ///
    /// 保存対象は表示設定だけで、プロセス情報・フィルタ・トークンは含まない。
    fn restore_settings(&mut self) {
        let settings = crate::settings::load();
        // パレットは Dashboard の状態ではなくシェル側の表示状態なので、
        // 設定の復元時にここで直接切り替える（ネイティブ版と同じ手順）。
        set_theme(settings.theme);
        for command in [
            Command::SetInterval(Duration::from_millis(settings.interval_ms)),
            Command::SetSortSpec(settings.sort),
            Command::SetDetailsVisible(settings.details_visible),
            Command::SetTreeView(settings.tree_view),
        ] {
            for effect in self.dashboard.apply(command) {
                if let Effect::Signal { .. } = effect {
                    // Restoring settings never triggers process actions.
                }
            }
        }
    }

    /// 表示設定だけを `localStorage` に保存する。
    fn persist_settings(&self) {
        let settings = tm_viewmodel::Settings {
            version: tm_viewmodel::SETTINGS_VERSION,
            interval_ms: self.dashboard.interval().as_millis() as u64,
            sort: self.dashboard.sort_spec(),
            details_visible: self.dashboard.details_visible(),
            tree_view: self.dashboard.tree_view(),
            theme: theme(),
        };
        if let Err(message) = crate::settings::save(&settings) {
            log::warn!("設定を保存できません: {message}");
        }
    }

    /// Applies a user intention and executes the effects the shell supports.
    fn handle_command(&mut self, command: Command, cx: &mut Context<Self>) {
        // 表示設定の変更だけを永続化する（Tick や操作では書き込まない）。
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
        if persist {
            self.persist_settings();
        }
        self.sync_proof_dom();
        cx.notify();
    }

    /// Keyboard filter + selection navigation (the browser shell has no modal).
    ///
    /// 文字入力・Backspace・Delete・貼り付け・IME 変換は **ここでは扱わない**。
    /// ブラウザ側の隠し `<textarea>`（`gpui-pre-web` の IME ミラー）が DOM の
    /// `keydown` / `composition*` / `input` を受け取り、[`EntityInputHandler`]
    /// 経由で [`Self::replace_text_in_range`] などへ届く。ここで文字を足すと
    /// 二重挿入になるので、この関数はナビゲーションと Esc だけを担当する。
    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.control || keystroke.modifiers.alt || keystroke.modifiers.platform {
            return;
        }
        match keystroke.key.to_lowercase().as_str() {
            // Esc: 絞り込みを解除する（IME 変換中なら変換ごと破棄する）。
            "escape" => {
                self.clear_filter(cx);
                cx.stop_propagation();
            }
            // Enter: IME の確定はブラウザ側が行う。一覧を閉じるモーダルは無いので
            // ここでは改行の既定動作だけを止める。
            "enter" => cx.stop_propagation(),
            "up" => {
                self.handle_command(Command::SelectRelative(-1), cx);
                cx.stop_propagation();
            }
            "down" => {
                self.handle_command(Command::SelectRelative(1), cx);
                cx.stop_propagation();
            }
            "pageup" => {
                self.handle_command(Command::SelectRelative(-10), cx);
                cx.stop_propagation();
            }
            "pagedown" => {
                self.handle_command(Command::SelectRelative(10), cx);
                cx.stop_propagation();
            }
            "home" => {
                self.handle_command(Command::SelectRelative(-1_000_000), cx);
                cx.stop_propagation();
            }
            "end" => {
                self.handle_command(Command::SelectRelative(1_000_000), cx);
                cx.stop_propagation();
            }
            _ => {}
        }
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
    fn apply_input_outcome(
        &mut self,
        outcome: EditOutcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if outcome == EditOutcome::Committed {
            let committed = self.filter_input.committed_text();
            let command = if committed.is_empty() {
                Command::ClearFilter
            } else {
                Command::SetFilterText(committed)
            };
            self.handle_command(command, cx);
        } else if outcome == EditOutcome::Composing {
            self.sync_proof_dom();
            cx.notify();
        }
        // ネイティブテキストサービス（Web では IME ミラー）へ編集・選択の変化を通知する。
        window.notify_text_input_changed(&self.focus_handle);
        window.notify_text_selection_changed(&self.focus_handle);
    }

    // ---------------------------------------------------------------- header

    fn header(&self, cx: &mut Context<Self>) -> Div {
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
            DataSource::Demo(_) => ("デモ", palette().yellow),
            DataSource::Live(_) => ("ライブ", palette().teal),
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
                            .child(pill("ブラウザ版", palette().base, palette().blue)),
                    )
                    .child(div().text_xs().text_color(rgb(palette().muted)).child(
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
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(palette().text))
                                    .child(hostname),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette().muted))
                                    .child(format!("{os} / {kernel}")),
                            ),
                    )
                    .child(pill(mode_label, palette().base, mode_color))
                    .child(pill("読み取り専用", palette().base, palette().red))
                    .child(
                        // 固定幅にしてヘッダー右端からの座標を決め打ちできるようにする
                        // （CDP 検証スクリプトがこのボタンをクリックする）。
                        div()
                            .id("toggle-theme-web")
                            .w(px(56.0))
                            .py_1()
                            .rounded_md()
                            .text_xs()
                            .cursor_pointer()
                            .border_1()
                            .border_color(rgb(palette().surface))
                            .bg(rgb(palette().surface_hi))
                            .text_color(rgb(palette().text))
                            .hover(|style| style.bg(rgb(palette().surface)))
                            .on_click(cx.listener(|this, _event, window, cx| {
                                this.focus_handle.focus(window, cx);
                                let new_theme = match theme() {
                                    tm_viewmodel::Theme::Dark => tm_viewmodel::Theme::Light,
                                    tm_viewmodel::Theme::Light => tm_viewmodel::Theme::Dark,
                                };
                                set_theme(new_theme);
                                this.persist_settings();
                                this.sync_proof_dom();
                                cx.notify();
                            }))
                            .child(if theme() == tm_viewmodel::Theme::Dark {
                                "ライト"
                            } else {
                                "ダーク"
                            }),
                    ),
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
            palette().blue,
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
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette().muted))
                                .child(format!("{} コア{}", cores.len(), frequency_label)),
                        ),
                )
                .child(sparkline(&cpu_history, 100.0, palette().blue, 28.0))
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
                        .text_color(rgb(palette().muted))
                        .child(format!("Load {}", format::format_load_average(load))),
                ),
        )
        .h_full()
        .flex_1()
        .min_w_0();

        let memory_card = card(
            "メモリ",
            palette().mauve,
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
                                .text_color(rgb(palette().muted))
                                .child(format::format_used_of_total(mem_used, mem_total)),
                        ),
                )
                .child(metric_bar(mem_percent, usage_color(mem_percent)))
                .child(
                    h_flex()
                        .justify_between()
                        .text_xs()
                        .text_color(rgb(palette().muted))
                        .child("スワップ")
                        .child(format::format_used_of_total(swap_used, swap_total)),
                )
                .child(metric_bar(swap_percent, palette().peach))
                .child(sparkline(&mem_history, 100.0, palette().mauve, 18.0))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette().muted))
                        .child(format!("空き {}", format::format_bytes(mem_free))),
                ),
        )
        .h_full()
        .flex_1()
        .min_w_0();

        let network_card = card(
            "ネットワーク",
            palette().teal,
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
                                .text_color(rgb(palette().teal))
                                .child(format::format_rate(rx_rate)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette().muted))
                                .child("受信"),
                        ),
                )
                .child(sparkline(&rx_history, max_rate, palette().teal, 20.0))
                .child(
                    h_flex()
                        .justify_between()
                        .items_end()
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(palette().peach))
                                .child(format::format_rate(tx_rate)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette().muted))
                                .child("送信"),
                        ),
                )
                .child(sparkline(&tx_history, max_rate, palette().peach, 12.0)),
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
                            .text_color(rgb(palette().muted))
                            .child(mount)
                            .child(format!("{percent:.0}%")),
                    )
                    .child(metric_bar(percent, usage_color(percent))),
            );
        }
        let components = snapshot.map(|s| s.components.as_slice()).unwrap_or(&[]);
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
                system_body = system_body.child(kv(label, value));
            }
        }
        let system_card = card("システム", palette().green, system_body)
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
        // 表示は「確定前 + 変換中 + 確定後」の 3 分割。IME 変換中の文字列は
        // 青 + 下線で見せ、Dashboard の絞り込みにはまだ渡さない。
        let (before, composing, after) = self.filter_input.display_parts();
        let composing = composing.to_string();
        let composing_active = !composing.is_empty();
        let field_is_empty = self.filter_input.is_empty();

        let mut field_children: Vec<Div> = Vec::new();
        if field_is_empty {
            field_children.push(div().child("名前・コマンド・ユーザー・PID"));
        } else {
            if !before.is_empty() {
                field_children.push(div().child(before.to_string()));
            }
            if composing_active {
                field_children.push(
                    div()
                        .text_color(rgb(palette().blue))
                        .underline()
                        .child(composing.clone()),
                );
            }
            if !after.is_empty() {
                field_children.push(div().child(after.to_string()));
            }
        }

        let (hint, hint_color) = if composing_active {
            ("変換中", palette().blue)
        } else if focused {
            ("入力可", palette().blue)
        } else {
            ("クリックで入力", palette().muted)
        };

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
            .bg(rgb(palette().mantle))
            .border_1()
            .border_color(rgb(if focused {
                palette().blue
            } else {
                palette().surface
            }))
            .cursor_pointer()
            .on_click(cx.listener(|this, _event, window, cx| {
                this.focus_handle.focus(window, cx);
                cx.notify();
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
            .child(div().text_xs().text_color(rgb(hint_color)).child(hint));

        h_flex()
            .justify_between()
            .h(px(TOOLBAR_HEIGHT))
            .flex_shrink_0()
            .child(filter_box)
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette().muted))
                            .child(format!(
                                "ソート: {} {}",
                                sort.key.label(),
                                sort.direction.arrow()
                            )),
                    )
                    .child(
                        div()
                            .id("toggle-tree-web")
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
            Some(overlay_text(self.empty_message(), palette().muted))
        } else if count == 0 {
            Some(overlay_text(
                "一致するプロセスがありません".to_string(),
                palette().muted,
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
            .border_color(rgb(palette().surface))
            .overflow_hidden()
            .bg(rgb(palette().base))
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
            .bg(rgb(palette().mantle))
            .border_b_1()
            .border_color(rgb(palette().surface))
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
                    .text_color(rgb(palette().muted))
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
            .text_color(rgb(if active {
                palette().blue
            } else {
                palette().muted
            }))
            .hover(|style| style.text_color(rgb(palette().text)))
            .on_click(cx.listener(move |this, _event, _window, cx| {
                this.handle_command(Command::SetSortKey(key), cx);
            }))
            .child(text)
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
                process.name.clone(),
                process.command.clone(),
                process.cpu_percent,
                process.memory_bytes,
                process.key.pid,
                process.user.clone().unwrap_or_else(|| "—".to_string()),
                process.status,
                process.threads.unwrap_or(0),
                self.dashboard.selected_key() == Some(process.key),
                depth,
                has_children,
            )
        };
        let tree_view = self.dashboard.tree_view();
        let collapsed = self.dashboard.is_collapsed(key);
        let marker = if tree_view && has_children {
            if collapsed { "▸ " } else { "▾ " }
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
                .id(("proc-row", index))
                .flex()
                .items_center()
                .w_full()
                .h(px(ROW_HEIGHT))
                .px_3()
                .gap_2()
                .cursor_pointer()
                .when(selected, |this| this.bg(rgb(palette().surface)))
                .hover(|style| style.bg(rgb(palette().mantle)))
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
                                .text_color(rgb(if selected {
                                    palette().text
                                } else {
                                    palette().subtext
                                }))
                                .truncate()
                                .child(display_name),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_xs()
                        .text_color(rgb(palette().dim))
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
                        .text_color(rgb(palette().subtext))
                        .child(format::format_bytes(memory)),
                )
                .child(
                    div()
                        .w(px(COL_PID))
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(palette().muted))
                        .child(pid.to_string()),
                )
                .child(
                    div()
                        .w(px(COL_USER))
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(palette().subtext))
                        .truncate()
                        .child(user),
                )
                .child(div().w(px(COL_STATUS)).flex_shrink_0().child(pill(
                    status.label(),
                    status_color(status),
                    palette().surface,
                )))
                .child(
                    div()
                        .w(px(COL_THREADS))
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(palette().muted))
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

        let mut content = match selected {
            Some(process) => self.details_content(&process, now),
            None => v_flex().gap_2().child(
                div()
                    .text_sm()
                    .text_color(rgb(palette().muted))
                    .child("行をクリックすると詳細を表示します"),
            ),
        };
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
            .flex_shrink_0()
            .bg(rgb(palette().mantle))
            .border_1()
            .border_color(rgb(palette().surface))
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
                        palette().surface,
                    ))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette().muted))
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
            .child(div().h(px(1.0)).w_full().bg(rgb(palette().surface)))
            .child(section_title("コマンド"))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette().subtext))
                    .child(process.command.clone()),
            )
            .child(div().h(px(1.0)).w_full().bg(rgb(palette().surface)))
            .child(div().text_xs().text_color(rgb(palette().yellow)).child(
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
            .bg(rgb(palette().mantle))
            .border_1()
            .border_color(rgb(palette().surface))
            .child(
                h_flex()
                    .gap_2()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(rgb(palette().muted))
                    .child(source_label)
                    .child(div().text_color(rgb(palette().dim)).child("|"))
                    .child(last_update)
                    .child(div().text_color(rgb(palette().dim)).child("|"))
                    .child(counts),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(rgb(if is_error {
                        palette().red
                    } else {
                        palette().dim
                    }))
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
        // IME 検証用: 入力欄の表示テキスト（変換中を含む）と変換中フラグ。
        // composition 中は `filter` が変わらず `ime_composing=1` になることを
        // CDP スクリプトが確認する。
        let ime_field = self.filter_input.text().replace(';', ",");
        let ime_composing = if self.filter_input.is_composing() {
            1
        } else {
            0
        };
        let ime_marked = self
            .filter_input
            .marked_range()
            .map(|range| format!("{}:{}", range.start, range.end))
            .unwrap_or_else(|| "-".to_string());
        let ime_selection = self.filter_input.selection();
        let ime_selection = format!("{}:{}", ime_selection.start, ime_selection.end);
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
        let theme_label = match theme() {
            tm_viewmodel::Theme::Dark => "dark",
            tm_viewmodel::Theme::Light => "light",
        };
        let proof = format!(
            "mode={mode};tick={};cpu={cpu:.1};mem={mem:.1};rows={rows};total={total};\
             filter={filter};selected_pid={selected};sort={};dir={direction};\
             sampled={sampled};vw={viewport_width};vh={viewport_height};live_error={};\
             ime_composing={ime_composing};ime_field={ime_field};ime_marked={ime_marked};\
             ime_selection={ime_selection};theme={theme_label};",
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

// ------------------------------------------------------- IME / text input

/// ブラウザの IME を絞り込み欄へ橋渡しする。
///
/// `gpui-pre-web` はキャンバス上に隠し `<textarea>`（IME ミラー）を置き、
/// `compositionstart` / `compositionupdate` / `compositionend` と `input` を
/// 購読して、この trait のメソッドとして呼び出す。UTF-16 オフセットの解釈は
/// [`FilterInput`] が担当し、**確定済み文字列だけ**を Dashboard に反映する。
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

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let outcome = self.filter_input.unmark();
        self.apply_input_outcome(outcome, window, cx);
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let outcome = self.filter_input.replace_text(range, text);
        self.apply_input_outcome(outcome, window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let outcome = self
            .filter_input
            .replace_and_mark(range, new_text, new_selected_range);
        self.apply_input_outcome(outcome, window, cx);
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // 変換候補ウィンドウの位置指定はしない（隠し textarea は 1px で不可視）。
        // 候補ウィンドウはブラウザ既定の位置に出る。
        None
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
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
        // ブラウザの `selectionchange`（IME が変換開始前に選択を動かす等）を取り込む。
        self.filter_input.set_selection(range_utf16);
        self.sync_proof_dom();
        cx.notify();
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        // 絞り込み欄は常に編集可能（ブラウザ版にモーダルやモードは無い）。
        true
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
        let focused = self.focus_handle.is_focused(window);
        let header = self.header(cx);
        let overview = self.overview();
        let toolbar = self.toolbar(focused, cx);
        let main_area = self.main_area(cx);
        let status_bar = self.status_bar();

        // IME ミラー（隠し textarea）からの composition / input をこのビューへ
        // 届けるための登録。GPUI の作法では paint 中に `Window::handle_input` を
        // 呼ぶ。フレームごとに登録し直す（`ElementInputHandler` はフレーム単位）。
        let input_view = cx.entity();
        let input_focus_handle = self.focus_handle.clone();
        let input_registration = canvas(
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
        .size_full();

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
            .bg(rgb(palette().base))
            .text_color(rgb(palette().text))
            .font_family(FONT_SANS)
            .child(header)
            .child(overview)
            .child(toolbar)
            .child(main_area)
            .child(status_bar)
            .child(input_registration)
    }
}

// ------------------------------------------------------------------ helpers

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

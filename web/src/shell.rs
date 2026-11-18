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

use gpui::accesskit::{Live, Node, Role, Toggled};
use gpui::{
    AnyElement, Bounds, Context, Div, ElementInputHandler, EntityInputHandler, FocusHandle,
    KeyDownEvent, Pixels, Render, ScrollStrategy, Stateful, TextInputConfiguration, UTF16Selection,
    UniformListScrollHandle, Window, canvas, div, prelude::*, px, rgb, uniform_list,
};
use tm_core::demo::DemoProbe;
use tm_core::export::{snapshot_to_csv, snapshot_to_json};
use tm_core::format;
use tm_core::port::Section;
use tm_core::redact::RedactionIdentifiers;
use tm_core::{
    PlatformInfo, ProcessSnapshot, ProcessStatus, SortDirection, SortKey, SystemProbe,
    SystemSnapshot,
};
use tm_viewmodel::{
    Command, DEFAULT_REFRESH_INTERVAL, Dashboard, EditOutcome, Effect, FilterInput,
    MAX_REFRESH_INTERVAL, MIN_REFRESH_INTERVAL, StatusMessage,
};
#[cfg(target_family = "wasm")]
use wasm_bindgen::prelude::wasm_bindgen;

use crate::agent::{AgentClient, AgentConfig};
use crate::settings::{INTERVAL_PRESETS_MS, interval_label};
use crate::theme::*;
use crate::widgets::{
    accessible_metric_bar, accessible_pill, card, h_flex, kv, section_title, sparkline,
    temperature_gauge, temperature_severity, trend_badge, usage_severity, v_flex,
};

const FONT_SANS: &str = "IBM Plex Sans";

/// Fixed layout constants (CSS px). The CDP verification script recomputes click
/// coordinates from these same numbers, so keep them in sync.
const HEADER_HEIGHT: f32 = 52.0;
/// 概要カードの高さ。システムカードは最大でホスト/稼働時間/プロセス + ディスク 2 件
/// （ラベル+バー）+ 温度 3 件（しきい値ゲージ: テキスト行+バー）を縦に並べるため、
/// 186 px では内容がはみ出してツールバーと重なる（Runtime Proof: 2026-11-16 の
/// スクリーンショットで確認）。CDP 検証スクリプトの座標定数と同期するため変更しない。
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

/// Screen-reader-only status node mirrored into the DOM on wasm. The canvas
/// itself carries no semantics, so this is the browser-side `aria-live`
/// counterpart of the AccessKit status node in `status_bar()`.
const A11Y_LIVE_DOM_ID: &str = "tm-web-a11y-live";

/// 既存の読み取り専用マーカーに使われている固定文言。headless 検証と aria の
/// 回帰を避けるため、`supports_process_control` の判定を追加しても変更しない。
const READONLY_PILL_ARIA: &str = "ブラウザ版ではプロセスを操作できません";
const READONLY_DETAIL_TEXT: &str =
    "ブラウザ版は読み取り専用です。プロセス操作（シグナル送信）は提供しません。";

/// `?agent=` 未指定の初回起動でデモモードを明示するための固定通知（R3-S05）。
///
/// `Dashboard::set_persistent_status` へ渡すため、トークン・ホスト名・
/// エージェント応答を一切含まない。`;` / 改行が無いので、将来この文言が
/// DOM フックへ鏡像されても `key=value;` の区切りを壊さない。
const DEMO_MODE_NOTICE: &str = "デモデータを表示しています。実データを見るには URL に ?agent=http://127.0.0.1:18787#token=<トークン> を指定してください";

/// ページ URL から接続設定を再読込したことを知らせる固定文言（R3-S04）。
const RELOAD_NOTICE: &str = "接続設定を再読込しました。エージェントへ再接続しています…";

/// スナップショットのエクスポート形式。ファイル名・MIME・アクセシブル名はすべて
/// 固定値で、ユーザー入力もエージェントトークンも含まない（DOM フックへ安全に
/// 書き出せるようにするため）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExportFormat {
    Csv,
    Json,
}

impl ExportFormat {
    fn filename(self) -> &'static str {
        match self {
            Self::Csv => "gpui-task-manager-snapshot.csv",
            Self::Json => "gpui-task-manager-snapshot.json",
        }
    }

    fn mime(self) -> &'static str {
        match self {
            Self::Csv => "text/csv;charset=utf-8",
            Self::Json => "application/json;charset=utf-8",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Csv => "CSV",
            Self::Json => "JSON",
        }
    }

    fn proof_value(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }
}

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
    /// Whether the last live error was an auth rejection that needs the user to
    /// re-enter the URL / token. Mirrored into the DOM proof as `reconnect=`.
    reconnect_required: bool,
    /// Whether the currently displayed snapshot came from a fetch that failed
    /// afterwards. It is kept on screen (never replaced by demo data) but
    /// flagged as stale so the freshness boundary stays visible (R3-S01).
    stale: bool,
    /// Number of snapshots ingested (used by the DOM proof hook).
    ticks: u64,
    /// 直近のエクスポート形式。DOM フックの `export=` フィールドにのみ使い、
    /// スナップショット本文やトークンは保持しない。
    last_export: Option<ExportFormat>,
}

impl TaskManagerApp {
    /// Creates the shell. Live mode is selected by `?agent=` in the page URL.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);

        let (dashboard, source) = match agent_config_from_url() {
            Some(config) => {
                let http = cx.http_client();
                // The executor supplies the fetch deadline on wasm as well as
                // natively; see `AgentClient::fetch_snapshot`.
                let client = Arc::new(AgentClient::new(
                    config,
                    http,
                    cx.background_executor().clone(),
                ));
                // The browser can never signal processes, even in live mode.
                let platform = PlatformInfo::native_read_only("Linux (エージェント経由)");
                let mut dashboard = Dashboard::new(platform, DEFAULT_REFRESH_INTERVAL);
                dashboard.set_status("エージェントに接続しています…", false);
                (dashboard, DataSource::Live(client))
            }
            None => {
                let probe = DemoProbe::new();
                let platform = probe.platform_info();
                let mut dashboard = Dashboard::new(platform, DEFAULT_REFRESH_INTERVAL);
                // A missing `?agent=` used to drop straight into demo mode with
                // no explanation. Keep the notice persistent so the first demo
                // sample cannot clear it (R3-S05).
                dashboard.set_persistent_status(DEMO_MODE_NOTICE);
                (dashboard, DataSource::Demo(probe))
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
            reconnect_required: false,
            stale: false,
            ticks: 0,
            last_export: None,
        };
        app.restore_settings();
        app.spawn_sampler(cx);
        app.sync_proof_dom();
        app
    }

    /// Starts the sampling loop at the currently configured interval.
    ///
    /// One task handles both modes. It awaits the current sample before sleeping,
    /// so at most one fetch is in flight (no request pile-up if the agent stalls).
    /// The wait is re-read from the dashboard on every cycle: a restored
    /// `settings.interval_ms` and a UI preset change both reach the sampler
    /// without restarting the task.
    fn spawn_sampler(&self, cx: &mut Context<Self>) {
        let live = match &self.source {
            DataSource::Live(client) => Some(client.clone()),
            DataSource::Demo(_) => None,
        };
        cx.spawn(async move |this, cx| {
            loop {
                // `?agent=` / `#token=` can change without a page reload when
                // the loopback agent is restarted with a fresh token. Re-read
                // them every cycle so the *same* client (and its single
                // in-flight fetch) picks up the new settings (R3-S04).
                let alive = this.update(cx, |app, cx| {
                    if app.refresh_agent_config() {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
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
                // Read *after* the sample so the newest applied interval (preset
                // click or persisted settings) decides the next wait. Never
                // freeze the cadence at startup.
                let interval =
                    match this.update(cx, |app, _| sampler_wait(app.dashboard.interval())) {
                        Ok(interval) => interval,
                        Err(_) => break,
                    };
                cx.background_executor().timer(interval).await;
            }
        })
        .detach();
    }

    /// Re-reads the connection settings from the page URL and applies them to
    /// the live client when they changed (R3-S04).
    ///
    /// The agent generates a new token per process, so a restart invalidates the
    /// one captured in `TaskManagerApp::new`. Because only the URL fragment
    /// changes, the browser never reloads the page; polling `agent_config_from_url`
    /// here is what closes that gap. Returns `true` only on a real change so the
    /// sampler avoids notifying the UI on every poll.
    fn refresh_agent_config(&mut self) -> bool {
        let Some(config) = agent_config_from_url() else {
            return false;
        };
        let DataSource::Live(client) = &self.source else {
            return false;
        };
        if !client.set_config(config) {
            return false;
        }
        self.live_error = None;
        self.reconnect_required = false;
        // Transient: the next successful sample clears it, and the auth
        // guidance (if any) is replaced by the fresh connection attempt.
        self.dashboard.set_status(RELOAD_NOTICE, false);
        self.sync_proof_dom();
        true
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
        // Freshness is decided against the snapshot that is *already on screen*,
        // before the new one replaces it. A failure before the first successful
        // fetch has nothing to mark stale; `live_error` covers that case.
        let had_snapshot = self.dashboard.snapshot().is_some();
        self.stale = stale_after_poll(had_snapshot, result.is_ok());
        match result {
            Ok(response) => {
                self.ticks = self.ticks.saturating_add(1);
                self.live_total = Some(response.total_process_count);
                self.live_error = None;
                self.reconnect_required = false;
                self.dashboard.ingest(response.snapshot);
            }
            Err(error) => {
                // An auth rejection carries fixed re-entry guidance (the token
                // itself is never echoed). Track it so the DOM proof can expose
                // a boolean without mirroring any error text.
                // Keep the re-entry guidance sticky: a transient follow-up
                // failure (e.g. the restarted agent not responding yet) must
                // not erase the one actionable path for the user (R49 audit).
                // Cleared only on success or an actual config change.
                let newly_required = crate::agent::requires_reconnect(&error);
                self.reconnect_required = next_reconnect_required(self.reconnect_required, &error);
                if !newly_required || self.live_error.is_none() {
                    self.live_error = Some(error.clone());
                }
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
        let (settings, notice) = crate::settings::load_with_notice();
        if let Some(notice) = notice {
            self.dashboard.set_persistent_status(notice.text);
        }
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

    /// 現在のスナップショットを redact 済みの CSV / JSON としてダウンロードする。
    ///
    /// 変換そのものは [`tm_core::export`] の純粋関数へ委譲する。ブラウザは
    /// エージェント側の環境変数（HOME / USER / hostname）を読めないため識別子は
    /// 空で渡すが、Dashboard のスナップショットはエージェント／デモプローブが
    /// 既に redact したものだけである。エクスポート関数はさらにプライベートな
    /// clone へ識別子非依存の redact（tailnet ドメインなど）を適用する。
    /// エージェントのトークンは `AgentClient` 内に留まり、ここへは渡らない。
    fn export_snapshot(&mut self, format: ExportFormat, cx: &mut Context<Self>) {
        let (content, rows) = match self.dashboard.snapshot() {
            Some(snapshot) => (export_document(snapshot, format), snapshot.processes.len()),
            None => {
                self.dashboard
                    .set_status("エクスポートできるスナップショットがありません", true);
                self.sync_proof_dom();
                cx.notify();
                return;
            }
        };
        download_text(format.filename(), format.mime(), &content);
        self.last_export = Some(format);
        // 既存の aria-live 経路で完了を読み上げる（色だけに依存しない）。
        self.dashboard
            .set_status(export_status_message(format, rows, content.len()), false);
        self.sync_proof_dom();
        cx.notify();
    }

    /// エクスポートボタン（CSV / JSON）。既存ボタンと同じ DOM id / role / aria 規約。
    fn export_button(
        &self,
        format: ExportFormat,
        id: &'static str,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(format!(
                "現在のスナップショットを {} ファイルとしてダウンロードする（redact 済み）",
                format.label()
            ))
            .px_2()
            .py_1()
            .rounded_md()
            .text_xs()
            .cursor_pointer()
            .border_1()
            .border_color(rgb(palette().surface))
            .bg(rgb(palette().mantle))
            .text_color(rgb(palette().blue))
            .hover(|style| {
                style
                    .bg(rgb(palette().surface))
                    .text_color(rgb(palette().text))
            })
            .on_click(cx.listener(move |this, _event, window, cx| {
                this.focus_handle.focus(window, cx);
                this.export_snapshot(format, cx);
            }))
            .child(format.label())
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
        let live = matches!(&self.source, DataSource::Live(_));
        let (mode_label, mode_color) = match &self.source {
            DataSource::Demo(_) => ("デモ", palette().yellow),
            DataSource::Live(_) => ("ライブ", palette().teal),
        };
        let platform = self.dashboard.platform_info();
        let show_stale = is_stale(live, self.stale);
        let (theme_now, theme_next) = if theme() == tm_viewmodel::Theme::Dark {
            ("ダーク", "ライト")
        } else {
            ("ライト", "ダーク")
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
                                    .id("header-title")
                                    .role(Role::Heading)
                                    .aria_level(1)
                                    .aria_label("GPUI タスクマネージャー（Web 版）")
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child("GPUI タスクマネージャー"),
                            )
                            .child(accessible_pill(
                                "header-browser",
                                "ブラウザ版",
                                palette().base,
                                palette().blue,
                                "ブラウザ版（読み取り専用）",
                            )),
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
                    .child(accessible_pill(
                        "header-mode",
                        mode_label,
                        palette().base,
                        mode_color,
                        if live {
                            "データ源: ライブ（エージェント経由）"
                        } else {
                            "データ源: デモ"
                        },
                    ))
                    .children(show_stale.then(|| {
                        accessible_pill(
                            "header-stale",
                            "stale (旧データ)",
                            palette().base,
                            palette().yellow,
                            "表示中のデータは古いです",
                        )
                    }))
                    .children((!platform.supports(Section::ProcessControl)).then(|| {
                        accessible_pill(
                            "header-readonly",
                            "読み取り専用",
                            palette().base,
                            palette().red,
                            READONLY_PILL_ARIA,
                        )
                    }))
                    .child(
                        // 固定幅にしてヘッダー右端からの座標を決め打ちできるようにする
                        // （CDP 検証スクリプトがこのボタンをクリックする）。
                        div()
                            .id("toggle-theme-web")
                            .role(Role::Button)
                            .aria_label(format!(
                                "テーマを切り替える。現在 {theme_now}、押すと {theme_next} に変更"
                            ))
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
                            .child(theme_next),
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
                .child(trend_badge(
                    "trend-cpu-web",
                    "CPU",
                    &self.dashboard.cpu_trend(),
                ))
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
                .child(
                    sparkline(&cpu_history, 100.0, palette().blue, 28.0)
                        .id("cpu-sparkline")
                        .role(Role::Image)
                        .aria_label(format!(
                            "CPU 使用率の推移、現在 {label}",
                            label = a11y_usage_label("CPU", cpu_total)
                        )),
                )
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
        .min_w_0()
        .id("overview-cpu")
        .role(Role::Group)
        .aria_label(a11y_usage_label("CPU 使用率", cpu_total));

        let memory_card = card(
            "メモリ",
            palette().mauve,
            v_flex()
                .gap_1()
                .child(trend_badge(
                    "trend-memory-web",
                    "メモリ",
                    &self.dashboard.memory_trend(),
                ))
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
                .child(accessible_metric_bar(
                    "memory-usage-bar",
                    mem_percent,
                    usage_color(mem_percent),
                    a11y_usage_label("メモリ使用率", mem_percent),
                ))
                .child(
                    h_flex()
                        .justify_between()
                        .text_xs()
                        .text_color(rgb(palette().muted))
                        .child("スワップ")
                        .child(format::format_used_of_total(swap_used, swap_total)),
                )
                .child(accessible_metric_bar(
                    "swap-usage-bar",
                    swap_percent,
                    palette().peach,
                    a11y_usage_label("スワップ使用率", swap_percent),
                ))
                .child(
                    sparkline(&mem_history, 100.0, palette().mauve, 18.0)
                        .id("memory-sparkline")
                        .role(Role::Image)
                        .aria_label(format!(
                            "メモリ使用率の推移、現在 {label}",
                            label = a11y_usage_label("メモリ", mem_percent)
                        )),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette().muted))
                        .child(format!("空き {}", format::format_bytes(mem_free))),
                ),
        )
        .h_full()
        .flex_1()
        .min_w_0()
        .id("overview-memory")
        .role(Role::Group)
        .aria_label(a11y_usage_label("メモリ使用率", mem_percent));

        // 取得不可でもカード枠と既存の id / aria は残す（CDP の座標定数を壊さない）。
        // 中身を説明へ差し替えるだけで、4 枚のカード構成は常に不変。
        let network_body = if self.dashboard.platform_info().supports(Section::Network) {
            v_flex()
                .gap_1()
                .child(trend_badge(
                    "trend-network-web",
                    "ネットワーク",
                    &self.dashboard.network_trend(),
                ))
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
                .child(
                    sparkline(&rx_history, max_rate, palette().teal, 20.0)
                        .id("network-rx-sparkline")
                        .role(Role::Image)
                        .aria_label(format!(
                            "ネットワーク受信レートの推移、現在 {}",
                            format::format_rate(rx_rate)
                        )),
                )
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
                .child(
                    sparkline(&tx_history, max_rate, palette().peach, 12.0)
                        .id("network-tx-sparkline")
                        .role(Role::Image)
                        .aria_label(format!(
                            "ネットワーク送信レートの推移、現在 {}",
                            format::format_rate(tx_rate)
                        )),
                )
        } else {
            v_flex()
                .gap_1()
                .justify_center()
                .child(unavailable_node(Section::Network, "network-unavailable"))
        };
        let network_card = card("ネットワーク", palette().teal, network_body)
            .h_full()
            .flex_1()
            .min_w_0()
            .id("overview-network")
            .role(Role::Group)
            .aria_label(format!(
                "ネットワーク、受信 {rx}、送信 {tx}",
                rx = format::format_rate(rx_rate),
                tx = format::format_rate(tx_rate),
            ));

        let mut system_body = v_flex()
            .gap_1()
            .child(kv("ホスト", hostname.clone()))
            .child(kv("稼働時間", format::format_duration(uptime)))
            .child(kv("プロセス", format::format_count(process_count)));
        if self.dashboard.platform_info().supports(Section::Disks) {
            for (index, (mount, percent)) in disks.into_iter().enumerate() {
                system_body = system_body.child(
                    v_flex()
                        .gap_1()
                        .child(
                            h_flex()
                                .justify_between()
                                .text_xs()
                                .text_color(rgb(palette().muted))
                                .child(mount.clone())
                                .child(format!("{percent:.0}%")),
                        )
                        .child(accessible_metric_bar(
                            ("disk-usage-bar", index),
                            percent,
                            usage_color(percent),
                            a11y_usage_label(&format!("ディスク {mount} 使用率"), percent),
                        )),
                );
            }
        } else {
            system_body = system_body.child(unavailable_node(Section::Disks, "disks-unavailable"));
        }
        let components = snapshot.map(|s| s.components.as_slice()).unwrap_or(&[]);
        if components.is_empty() {
            system_body = system_body.child(kv("温度", "センサーなし".to_string()));
        } else {
            // 温度行をしきい値ゲージに置き換える。値・max/crit・状態をテキストで
            // 表示し、バーと AccessKit 名にも同じ状態を載せる（色だけに依存しない）。
            for (index, component) in components.iter().take(3).enumerate() {
                let label = if component.label.is_empty() {
                    "センサー".to_string()
                } else {
                    component.label.clone()
                };
                system_body = system_body.child(temperature_gauge(
                    ("temperature-gauge", index),
                    label,
                    component.temperature_c,
                    component.max_c,
                    component.critical_c,
                ));
            }
        }
        let system_card = card("システム", palette().green, system_body)
            .h_full()
            .flex_1()
            .min_w_0()
            .id("overview-system")
            .role(Role::Group)
            .aria_label(format!(
                "システム概要、ホスト {hostname}、プロセス {process_count} 件"
            ));

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
        let active_interval = self.dashboard.interval();
        // 更新間隔ボタン（ネイティブ版と同じプリセット）。クリックで
        // Command::SetInterval を発行し、Dashboard → localStorage → サンプラー
        // の順に同じ値が反映される。
        let interval_buttons: Vec<Stateful<Div>> = INTERVAL_PRESETS_MS
            .iter()
            .map(|&millis| {
                let label = interval_label(millis);
                let active = active_interval == Duration::from_millis(millis);
                div()
                    .id(("interval-web", millis))
                    .role(Role::Button)
                    .aria_label(format!(
                        "更新間隔 {label}（{}）",
                        if active { "選択中" } else { "未選択" }
                    ))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_xs()
                    .cursor_pointer()
                    .border_1()
                    .border_color(rgb(palette().surface))
                    .bg(rgb(if active {
                        palette().surface
                    } else {
                        palette().mantle
                    }))
                    .text_color(rgb(if active {
                        palette().blue
                    } else {
                        palette().muted
                    }))
                    .hover(|style| {
                        style
                            .bg(rgb(palette().surface))
                            .text_color(rgb(palette().text))
                    })
                    .on_click(cx.listener(move |this, _event, window, cx| {
                        this.focus_handle.focus(window, cx);
                        this.handle_command(
                            Command::SetInterval(Duration::from_millis(millis)),
                            cx,
                        );
                    }))
                    .child(label)
            })
            .collect();
        let filter_box = div()
            .id("filter-box")
            .role(Role::TextInput)
            .aria_label("プロセスの絞り込み")
            .aria_placeholder("名前・コマンド・ユーザー・PID")
            .aria_value(self.dashboard.filter_text())
            .aria_description(hint)
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
                            .id("sort-indicator")
                            .role(Role::Label)
                            .aria_label(format!(
                                "ソート: {} {}",
                                sort.key.label(),
                                a11y_sort_direction(sort.direction)
                            ))
                            .text_xs()
                            .text_color(rgb(palette().muted))
                            .child(format!(
                                "ソート: {} {}",
                                sort.key.label(),
                                sort.direction.arrow()
                            )),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette().dim))
                                    .child("更新間隔"),
                            )
                            .children(interval_buttons),
                    )
                    .child(
                        div()
                            .id("toggle-tree-web")
                            .role(Role::Button)
                            .aria_label("ツリービューを切り替える")
                            .aria_toggled(if self.dashboard.tree_view() {
                                Toggled::True
                            } else {
                                Toggled::False
                            })
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
                            .id("toggle-details-web")
                            .role(Role::Button)
                            .aria_label("詳細パネルを切り替える")
                            .aria_toggled(if self.dashboard.details_visible() {
                                Toggled::True
                            } else {
                                Toggled::False
                            })
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .text_xs()
                            .cursor_pointer()
                            .border_1()
                            .border_color(rgb(palette().surface))
                            .bg(rgb(if self.dashboard.details_visible() {
                                palette().surface
                            } else {
                                palette().mantle
                            }))
                            .text_color(rgb(if self.dashboard.details_visible() {
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
                                this.handle_command(Command::ToggleDetails, cx);
                            }))
                            .child("詳細"),
                    )
                    .child(
                        div()
                            .id("visible-count")
                            .role(Role::Label)
                            .aria_label(format!("表示 {} 件", self.dashboard.visible_count()))
                            .text_xs()
                            .text_color(rgb(palette().muted))
                            .child(format!("表示 {} 件", self.dashboard.visible_count())),
                    )
                    .child(self.export_button(ExportFormat::Csv, "export-csv-web", cx))
                    .child(self.export_button(ExportFormat::Json, "export-json-web", cx)),
            )
    }

    // ------------------------------------------------------------- main area

    fn main_area(&self, cx: &mut Context<Self>) -> Div {
        let details = self
            .dashboard
            .details_visible()
            .then(|| self.details_panel());
        div()
            .flex()
            .gap_3()
            .flex_1()
            .min_h(px(0.0))
            .child(self.table(cx))
            .children(details)
    }

    fn table(&self, cx: &mut Context<Self>) -> Stateful<Div> {
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
            .id("process-table")
            .role(Role::Table)
            .aria_label("プロセス一覧")
            .aria_row_count(count)
            .aria_column_count(8)
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

    fn table_header(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        h_flex()
            .gap_2()
            .h(px(30.0))
            .px_3()
            .flex_shrink_0()
            .bg(rgb(palette().mantle))
            .border_b_1()
            .border_color(rgb(palette().surface))
            .id("table-header")
            .role(Role::RowGroup)
            .aria_label("プロセス一覧の列見出し")
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
            .role(Role::ColumnHeader)
            .aria_label(if active {
                format!(
                    "{label} 列で並び替え、現在 {direction}",
                    direction = a11y_sort_direction(sort.direction)
                )
            } else {
                format!("{label} 列で並び替え")
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

        let row_a11y_label = a11y_row_label(&name, status, cpu, memory, pid, selected);

        Some(
            div()
                .id(("proc-row", index))
                .role(Role::Row)
                .aria_label(row_a11y_label)
                .aria_selected(selected)
                .aria_row_index(index + 1)
                .when(tree_view && has_children, |this| {
                    this.aria_expanded(!collapsed)
                })
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
                .child(
                    div()
                        .w(px(COL_STATUS))
                        .flex_shrink_0()
                        .child(accessible_pill(
                            ("row-status", index),
                            status.label(),
                            status_color(status),
                            palette().surface,
                            format!("状態: {}", status.label()),
                        )),
                )
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

    fn details_panel(&self) -> Stateful<Div> {
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
        content = content.child(
            section_title("センサー / 温度")
                .id("details-sensors-title")
                .role(Role::Heading)
                .aria_level(2),
        );
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
                // 詳細パネルにも色に依存しない状態テキストを添える（概要のゲージと同じ判定）。
                let severity = temperature_severity(
                    component.temperature_c,
                    component.max_c,
                    component.critical_c,
                );
                value.push_str(&format!(" ({})", severity.label()));
                let label = if component.label.is_empty() {
                    "センサー".to_string()
                } else {
                    component.label.clone()
                };
                content = content.child(kv(label, value));
            }
        }

        v_flex()
            .id("details-panel")
            .role(Role::Complementary)
            .aria_label("プロセス詳細パネル")
            .w(px(DETAILS_WIDTH))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(palette().mantle))
            .border_1()
            .border_color(rgb(palette().surface))
            .rounded_md()
            .p_4()
            .gap_3()
            .child(
                section_title("プロセス詳細")
                    .id("details-title")
                    .role(Role::Heading)
                    .aria_level(2),
            )
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
                    .child(accessible_pill(
                        "details-status",
                        process.status.label(),
                        status_color(process.status),
                        palette().surface,
                        format!("状態: {}", process.status.label()),
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
            .child(
                section_title("コマンド")
                    .id("details-command-title")
                    .role(Role::Heading)
                    .aria_level(2),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette().subtext))
                    .child(process.command.clone()),
            )
            .child(div().h(px(1.0)).w_full().bg(rgb(palette().surface)))
            .children(
                (!self
                    .dashboard
                    .platform_info()
                    .supports(Section::ProcessControl))
                .then(|| {
                    div()
                        .text_xs()
                        .text_color(rgb(palette().yellow))
                        .child(READONLY_DETAIL_TEXT)
                }),
            )
    }

    // ------------------------------------------------------------ status bar

    fn status_bar(&self) -> Stateful<Div> {
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
        let show_stale = is_stale(matches!(&self.source, DataSource::Live(_)), self.stale);
        let live_text = a11y_live_text(self.dashboard.status(), show_stale);
        let announcement = live_text.clone();

        h_flex()
            .id("status-bar")
            .role(Role::Status)
            .aria_label("ステータス")
            .aria_value(live_text)
            // AccessKit has no DOM on wasm, so expose an explicit polite live
            // node here; sync_proof_dom() mirrors the same text into a hidden
            // `role=status aria-live=polite` element for real browsers.
            .a11y_synthetic_children(move |builder| {
                let mut node = Node::new(Role::Status);
                node.set_live(Live::Polite);
                node.set_label(announcement);
                builder.push_child(builder.synthetic_node_id("status-live"), node);
            })
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
                    .child(counts)
                    .children(show_stale.then(|| {
                        div()
                            .text_color(rgb(palette().yellow))
                            .child("stale: 表示中のデータは古いです")
                    })),
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
                    .child(if is_error {
                        format!("エラー: {message}")
                    } else {
                        message
                    }),
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
        let live = matches!(&self.source, DataSource::Live(_));
        let mode = if live { "live" } else { "demo" };
        // Additive field: existing CDP assertions parse `key=value;` pairs and
        // keep working, while new checks can assert that a failed refresh
        // leaves the old snapshot flagged as stale.
        let stale = u8::from(is_stale(live, self.stale));
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
        // Same helper as the sampler uses, so the reported value is exactly the
        // cadence the next sample will wait for.
        let interval_ms = sampler_wait(self.dashboard.interval()).as_millis() as u64;
        let theme_label = match theme() {
            tm_viewmodel::Theme::Dark => "dark",
            tm_viewmodel::Theme::Light => "light",
        };
        let proof = format!(
            "mode={mode};tick={};cpu={cpu:.1};mem={mem:.1};rows={rows};total={total};\
             filter={filter};selected_pid={selected};sort={};dir={direction};\
             sampled={sampled};vw={viewport_width};vh={viewport_height};live_error={};\
             stale={stale};ime_composing={ime_composing};ime_field={ime_field};ime_marked={ime_marked};\
             ime_selection={ime_selection};theme={theme_label};",
            self.ticks,
            sort.key.label(),
            if self.live_error.is_some() { 1 } else { 0 },
        );
        let proof = proof_with_interval(&proof, interval_ms);
        let proof = proof_with_export(&proof, self.last_export);
        let proof = proof_with_reconnect(&proof, self.reconnect_required);
        document.set_title(&format!(
            "GPUI タスクマネージャー (Web) | mode={mode} tick={} rows={rows} selected={selected} filter={filter}",
            self.ticks
        ));
        if let Some(body) = document.body() {
            let _ = body.set_attribute("data-gpui-proof", &proof);
        }
        self.sync_a11y_dom(&document);
    }

    /// Browser-side `aria-live` mirror for the canvas-only UI.
    ///
    /// The canvas carries no DOM semantics, so a visually hidden
    /// `role="status" aria-live="polite"` node announces status changes. This
    /// is additive: `body[data-gpui-proof]` is written just before this call
    /// and keeps its exact key=value; format. Only already-redacted status
    /// text is copied here — never the agent token.
    #[cfg(target_family = "wasm")]
    fn sync_a11y_dom(&self, document: &web_sys::Document) {
        let Some(body) = document.body() else {
            return;
        };
        let element = match document.get_element_by_id(A11Y_LIVE_DOM_ID) {
            Some(element) => element,
            None => {
                let Ok(element) = document.create_element("div") else {
                    return;
                };
                if element.set_attribute("id", A11Y_LIVE_DOM_ID).is_err() {
                    return;
                }
                let _ = element.set_attribute("role", "status");
                let _ = element.set_attribute("aria-live", "polite");
                let _ = element.set_attribute("aria-atomic", "true");
                // Visually hidden but still in the accessibility tree.
                let _ = element.set_attribute(
                    "style",
                    "position:fixed;width:1px;height:1px;overflow:hidden;\
                     clip:rect(0 0 0 0);white-space:nowrap",
                );
                if body.append_child(&element).is_err() {
                    return;
                }
                element
            }
        };
        let text = a11y_live_text(
            self.dashboard.status(),
            is_stale(matches!(&self.source, DataSource::Live(_)), self.stale),
        );
        // Avoid re-announcing an unchanged message on every sample.
        if element.text_content().as_deref() != Some(text.as_str()) {
            let _ = element.set_text_content(Some(&text));
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

/// Accessible name for one process row.
///
/// The coloured status dot and usage colour are decorative; the name repeats
/// their meaning in text so it survives colour-blindness and screen readers.
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

/// Text announced by the status live region (AccessKit + DOM mirror).
///
/// Error and freshness states are spelled out instead of relying on the red /
/// yellow status-bar text colour. The text comes from `Dashboard::status()`,
/// which the agent client has already redacted before it reaches the UI.
fn a11y_live_text(status: Option<&StatusMessage>, stale: bool) -> String {
    let mut parts = Vec::new();
    if stale {
        parts.push("表示中のデータは古いです".to_string());
    }
    if let Some(status) = status {
        // StatusMessage already carries a non-colour "[エラー]" marker; only
        // add the legacy prefix for messages that predate that marker.
        let prefix = if status.is_error && !status.text.contains("エラー") {
            "エラー: "
        } else {
            ""
        };
        parts.push(format!("{prefix}{}", truncate_label(&status.text, 160)));
    }
    if parts.is_empty() {
        "ステータス: 変更なし".to_string()
    } else {
        parts.join("。")
    }
}

/// Accessible name for a colour-coded utilisation value.
fn a11y_usage_label(title: &str, percent: f32) -> String {
    let percent = percent.clamp(0.0, 100.0);
    format!(
        "{title} {percent:.0}%、{severity}",
        severity = usage_severity(percent)
    )
}

/// Japanese direction name for sort headers (the ▲/▼ glyph alone is not text).
fn a11y_sort_direction(direction: SortDirection) -> &'static str {
    match direction {
        SortDirection::Ascending => "昇順",
        SortDirection::Descending => "降順",
    }
}

/// Text shown in place of a capability-gated section (R5-I04).
///
/// Fixed strings only: no user input or agent data, and no `;` / newline that
/// could corrupt the `key=value;` DOM proof if a future check mirrors them.
/// Section ids and aria labels of the surrounding cards stay untouched; the
/// explanation is an additional child node, never an aria-label override.
fn unavailable_text(section: Section) -> &'static str {
    match section {
        Section::Network => "この環境ではネットワーク情報を取得できません",
        Section::Disks => "この環境ではディスク情報を取得できません",
        Section::ProcessControl => READONLY_PILL_ARIA,
    }
}

/// Renders the explanation for an unavailable section without changing the
/// surrounding card's geometry (CDP click coordinates stay valid).
fn unavailable_node(section: Section, id: &'static str) -> Stateful<Div> {
    let text = unavailable_text(section);
    div()
        .id(id)
        .role(Role::Label)
        .aria_label(text)
        .text_xs()
        .text_color(rgb(palette().muted))
        .child(text)
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

/// Resolves the sampler's timer wait from the interval currently applied to the
/// dashboard.
///
/// `Dashboard::interval()` is already clamped by `Command::SetInterval` and
/// `Settings::normalized`; clamping here too keeps the cadence inside the
/// supported range even if a future caller applies a raw value. Kept pure so
/// the `settings.interval_ms` → sampler mapping is testable on the native host.
fn sampler_wait(interval: Duration) -> Duration {
    interval.clamp(MIN_REFRESH_INTERVAL, MAX_REFRESH_INTERVAL)
}

/// Appends the refresh-interval field to the DOM proof string.
///
/// Additive only: existing `key=value;` pairs stay byte-for-byte identical, so
/// the headless-Chrome assertions keep parsing the hook. The value is a plain
/// millisecond count derived from the clamped sampler interval — it can never
/// carry user text or an agent token.
#[cfg(any(target_family = "wasm", test))]
fn proof_with_interval(proof: &str, interval_ms: u64) -> String {
    format!("{proof}interval={interval_ms};")
}

/// Appends the last export format to the DOM proof string.
///
/// Additive only: the fixed `csv` / `json` / `none` value can never carry user
/// text, a hostname or the agent token, and every pre-existing `key=value;`
/// pair stays byte-for-byte identical.
#[cfg(any(target_family = "wasm", test))]
fn proof_with_export(proof: &str, last_export: Option<ExportFormat>) -> String {
    let value = last_export.map(ExportFormat::proof_value).unwrap_or("none");
    format!("{proof}export={value};")
}

/// Appends the auth-failure re-entry marker to the DOM proof string.
///
/// Additive only: a plain `0` / `1` with no error text, token or endpoint, so
/// every pre-existing `key=value;` pair stays byte-for-byte identical and the
/// headless-Chrome parser keeps working.
#[cfg(any(target_family = "wasm", test))]
fn proof_with_reconnect(proof: &str, required: bool) -> String {
    format!("{proof}reconnect={};", u8::from(required))
}

/// The re-entry flag is sticky until success or an explicit config change: a
/// transient follow-up failure must not hide the auth guidance (R49 audit).
fn next_reconnect_required(current: bool, error: &str) -> bool {
    current || crate::agent::requires_reconnect(error)
}

/// Identifiers handed to the pure exporters.
///
/// The browser cannot read the agent host's `HOME` / `USER` / hostname, and the
/// dashboard only ever holds snapshots that were redacted before reaching it
/// (synthetic demo data, or the agent's `--redact`-style wire response). Passing
/// empty identifiers is therefore correct here; `tm_core::export` still applies
/// its identifier-independent redaction (tailnet domains, `host.<id>` suffixes
/// and bare `tail…` tokens) to a private clone of the snapshot.
fn export_identifiers() -> RedactionIdentifiers<'static> {
    RedactionIdentifiers {
        home: "",
        user: "",
        hostname: "",
    }
}

/// Runs the pure `tm_core` exporter for the requested format.
fn export_document(snapshot: &SystemSnapshot, format: ExportFormat) -> String {
    let ids = export_identifiers();
    match format {
        ExportFormat::Csv => snapshot_to_csv(snapshot, &ids),
        ExportFormat::Json => snapshot_to_json(snapshot, &ids),
    }
}

/// Status text announced through the existing polite `aria-live` path.
fn export_status_message(format: ExportFormat, rows: usize, bytes: usize) -> String {
    format!(
        "{} をエクスポートしました（{} 件 / {} バイト、redact 済み）",
        format.label(),
        rows,
        bytes
    )
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen(inline_js = r#"
export function tmDownloadText(filename, mime, text) {
  const blob = new Blob([text], { type: mime });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = filename;
  anchor.rel = 'noopener';
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  setTimeout(() => URL.revokeObjectURL(url), 0);
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = "tmDownloadText")]
    fn tm_download_text(filename: &str, mime: &str, text: &str);
}

/// Hands the exported text to the browser as a Blob download.
///
/// The wasm implementation is a tiny inline-JS helper because `web/Cargo.toml`
/// (out of scope for this task) does not enable the `Url` / `HtmlAnchorElement`
/// web-sys features. Native builds have no DOM; the same conversion path is
/// still exercised by the pure-function tests below.
#[cfg(target_family = "wasm")]
fn download_text(filename: &str, mime: &str, content: &str) {
    tm_download_text(filename, mime, content);
}

#[cfg(not(target_family = "wasm"))]
fn download_text(_filename: &str, _mime: &str, _content: &str) {}

/// Whether the snapshot left on screen after a poll must be shown as stale.
///
/// Only a failed refresh of an already-populated dashboard makes displayed data
/// old. A first-attempt failure has nothing stale to label; it is reported via
/// `live_error` and the empty-state overlay instead. Keeping this decision in a
/// pure function makes the freshness boundary testable on the native host.
fn stale_after_poll(had_snapshot: bool, succeeded: bool) -> bool {
    had_snapshot && !succeeded
}

/// Stale markers apply to live data only; demo mode has no freshness contract.
fn is_stale(is_live: bool, stale: bool) -> bool {
    is_live && stale
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_refresh_marks_the_existing_snapshot_stale() {
        assert!(stale_after_poll(true, false));
        assert!(is_stale(true, stale_after_poll(true, false)));
    }

    #[test]
    fn successful_refresh_clears_stale() {
        assert!(!stale_after_poll(true, true));
        assert_ne!(stale_after_poll(true, true), stale_after_poll(true, false));
    }

    #[test]
    fn first_failure_has_no_old_snapshot_to_mark_stale() {
        assert!(!stale_after_poll(false, false));
        assert!(!stale_after_poll(false, true));
    }

    #[test]
    fn stale_marker_is_live_only() {
        assert!(!is_stale(false, true));
        assert!(!is_stale(false, false));
        assert!(is_stale(true, true));
        assert!(!is_stale(true, false));
    }

    #[test]
    fn row_label_always_spells_out_status_and_usage_in_text() {
        let label = a11y_row_label("systemd", ProcessStatus::Zombie, 92.0, 4_096, 1, true);
        assert!(label.contains("systemd"), "{label}");
        assert!(label.contains("ゾンビ"), "{label}");
        assert!(label.contains("高"), "{label}");
        assert!(label.contains("選択中"), "{label}");
        let unselected = a11y_row_label("idle", ProcessStatus::Idle, 1.0, 0, 2, false);
        assert!(unselected.contains("アイドル"), "{unselected}");
        assert!(unselected.contains("低"), "{unselected}");
        assert!(!unselected.contains("選択中"), "{unselected}");
    }

    #[test]
    fn live_text_marks_errors_and_staleness_without_colour() {
        let error = StatusMessage {
            text: "接続に失敗しました".to_string(),
            is_error: true,
            persistent: false,
        };
        let text = a11y_live_text(Some(&error), true);
        assert!(text.contains("エラー:"), "{text}");
        assert!(text.contains("古い"), "{text}");
        assert!(text.contains("接続に失敗しました"), "{text}");
        assert_eq!(a11y_live_text(None, false), "ステータス: 変更なし");
    }

    #[test]
    fn usage_label_includes_threshold_word() {
        assert!(a11y_usage_label("CPU 使用率", 90.0).contains("高"));
        assert!(a11y_usage_label("CPU 使用率", 70.0).contains("中"));
        assert!(a11y_usage_label("CPU 使用率", 10.0).contains("低"));
    }

    #[test]
    fn sort_direction_has_a_textual_name() {
        assert_eq!(a11y_sort_direction(SortDirection::Ascending), "昇順");
        assert_eq!(a11y_sort_direction(SortDirection::Descending), "降順");
    }

    #[test]
    fn sampler_wait_tracks_the_applied_interval_and_clamps() {
        // UI / 復元後の interval_ms がサンプラーへそのまま渡ること。
        assert_eq!(
            sampler_wait(Duration::from_millis(2_000)),
            Duration::from_millis(2_000)
        );
        assert_eq!(sampler_wait(Duration::from_millis(1)), MIN_REFRESH_INTERVAL);
        assert_eq!(
            sampler_wait(Duration::from_secs(3_600)),
            MAX_REFRESH_INTERVAL
        );
    }

    #[test]
    fn interval_presets_produce_distinct_sampler_waits() {
        let waits: Vec<Duration> = INTERVAL_PRESETS_MS
            .iter()
            .map(|&millis| sampler_wait(Duration::from_millis(millis)))
            .collect();
        assert_eq!(waits.len(), INTERVAL_PRESETS_MS.len());
        assert!(waits.windows(2).all(|pair| pair[0] != pair[1]));
        for (&millis, wait) in INTERVAL_PRESETS_MS.iter().zip(waits) {
            assert_eq!(wait, Duration::from_millis(millis));
        }
    }

    #[test]
    fn temperature_gauge_state_is_textual_and_dom_proof_stays_additive() {
        use crate::widgets::{temperature_a11y_label, temperature_value_text};

        // ゲージが色で示す「危険」は、アクセシブル名にもテキストで現れる。
        let value = temperature_value_text(Some(101.0), Some(68.0), Some(100.0));
        let label = temperature_a11y_label("CPU Packages", &value);
        assert!(label.contains("危険"), "{label}");
        assert!(label.contains("max 68°C"), "{label}");
        assert!(label.contains("crit 100°C"), "{label}");

        // ゲージを追加しても既存 DOM hook の key=value; は壊さない（末尾追加のみ）。
        let legacy = "mode=demo;tick=7;rows=3;theme=dark;";
        let proof = proof_with_interval(legacy, 2_000);
        assert!(proof.starts_with(legacy), "{proof}");
    }

    #[test]
    fn dom_proof_interval_field_is_additive_and_token_free() {
        // 既存 DOM hook の key=value; は壊さず、末尾に interval を足すだけ。
        // エージェントのトークンは数値フィールドへ一切入らない。
        let token = "super-secret-agent-token";
        let legacy = "mode=demo;tick=7;rows=3;theme=dark;";
        let proof = proof_with_interval(legacy, 2_000);
        assert!(proof.starts_with(legacy), "{proof}");
        assert_eq!(proof, "mode=demo;tick=7;rows=3;theme=dark;interval=2000;");
        assert!(!proof.contains(token), "{proof}");
        for field in [
            "mode=demo",
            "tick=7",
            "rows=3",
            "theme=dark",
            "interval=2000",
        ] {
            assert!(
                proof.split(';').any(|part| part == field),
                "{field} missing from {proof}"
            );
        }
    }

    fn export_sample_snapshot() -> SystemSnapshot {
        SystemSnapshot {
            hostname: Some("host".into()),
            os_name: Some("Linux".into()),
            kernel_version: Some("6.11-generic".into()),
            processes: vec![ProcessSnapshot {
                key: tm_core::ProcessKey::new(4242, 1_700_000_000),
                name: "worker".into(),
                command: "worker --endpoint https://host.tail1234.ts.net:8443".into(),
                user: Some("user".into()),
                status: ProcessStatus::Running,
                cpu_percent: 1.5,
                memory_bytes: 1_024,
                virtual_memory_bytes: 2_048,
                threads: Some(4),
                parent_pid: None,
            }],
            ..SystemSnapshot::default()
        }
    }

    #[test]
    fn export_document_delegates_to_pure_tm_core_exporters_without_mutation() {
        let snapshot = export_sample_snapshot();
        let before = format!("{snapshot:?}");
        let ids = export_identifiers();
        for (format, expected) in [
            (
                ExportFormat::Csv,
                tm_core::export::snapshot_to_csv(&snapshot, &ids),
            ),
            (
                ExportFormat::Json,
                tm_core::export::snapshot_to_json(&snapshot, &ids),
            ),
        ] {
            assert_eq!(export_document(&snapshot, format), expected);
        }
        assert_eq!(
            format!("{snapshot:?}"),
            before,
            "export must not mutate the dashboard snapshot"
        );
    }

    #[test]
    fn export_output_applies_redaction_and_never_carries_the_agent_token() {
        let snapshot = export_sample_snapshot();
        let token = "super-secret-agent-token";
        for format in [ExportFormat::Csv, ExportFormat::Json] {
            let text = export_document(&snapshot, format);
            // 識別子非依存の redact がエクスポート時にも適用されること。
            for secret in ["tail1234", "ts.net"] {
                assert!(!text.contains(secret), "{format:?} leaked {secret}: {text}");
            }
            assert!(text.contains("host"), "{format:?}: {text}");
            // トークンは AgentClient に留まり、エクスポート関数へ渡さない。
            assert!(!text.contains(token), "{format:?} leaked the token");
        }
    }

    #[test]
    fn export_filenames_are_fixed_and_status_uses_the_existing_live_contract() {
        assert_eq!(
            ExportFormat::Csv.filename(),
            "gpui-task-manager-snapshot.csv"
        );
        assert_eq!(
            ExportFormat::Json.filename(),
            "gpui-task-manager-snapshot.json"
        );
        assert!(ExportFormat::Csv.mime().starts_with("text/csv"));
        assert!(ExportFormat::Json.mime().starts_with("application/json"));

        let status = export_status_message(ExportFormat::Csv, 28, 1_234);
        assert!(status.contains("CSV"), "{status}");
        assert!(status.contains("28 件"), "{status}");
        assert!(status.contains("1234 バイト"), "{status}");
        assert!(status.contains("redact 済み"), "{status}");
        // 既存の aria-live 契約: 成功ステータスは「エラー:」を付けず読み上げる。
        let announced = a11y_live_text(
            Some(&StatusMessage {
                text: status,
                is_error: false,
                persistent: false,
            }),
            false,
        );
        assert!(announced.contains("エクスポートしました"), "{announced}");
        assert!(!announced.starts_with("エラー:"), "{announced}");
    }

    #[test]
    fn dom_proof_export_field_is_additive_and_token_free() {
        let token = "super-secret-agent-token";
        let legacy = "mode=demo;tick=7;rows=3;theme=dark;interval=2000;";
        for (format, expected) in [
            (None, "none"),
            (Some(ExportFormat::Csv), "csv"),
            (Some(ExportFormat::Json), "json"),
        ] {
            let proof = proof_with_export(legacy, format);
            assert!(proof.starts_with(legacy), "{proof}");
            assert_eq!(proof, format!("{legacy}export={expected};"));
            assert!(!proof.contains(token), "{proof}");
        }
        for field in [
            "mode=demo",
            "tick=7",
            "rows=3",
            "theme=dark",
            "interval=2000",
        ] {
            let proof = proof_with_export(legacy, Some(ExportFormat::Csv));
            assert!(
                proof.split(';').any(|part| part == field),
                "{field} missing from {proof}"
            );
        }
    }

    #[test]
    fn demo_mode_notice_is_fixed_actionable_and_dom_safe() {
        // URL 未指定の初回起動では、何も起きずにデモへ落ちるのではなく
        // 「どの URL を指定すれば実データになるか」を必ず通知する（R3-S05）。
        assert!(DEMO_MODE_NOTICE.contains("デモ"));
        assert!(DEMO_MODE_NOTICE.contains("?agent="));
        assert!(DEMO_MODE_NOTICE.contains("#token="));
        // 固定文言なので `;` / 改行 / トークンは入らず、DOM hook と redact
        // 契約を壊さない。
        assert!(!DEMO_MODE_NOTICE.contains(';'));
        assert!(!DEMO_MODE_NOTICE.contains('\n'));
        assert!(!DEMO_MODE_NOTICE.contains("super-secret-agent-token"));
        // 永続通知は [エラー] 付きの a11y テキストでも全文が読み上げられる。
        let announced = a11y_live_text(
            Some(&StatusMessage {
                text: format!("[エラー] {DEMO_MODE_NOTICE}"),
                is_error: true,
                persistent: true,
            }),
            false,
        );
        assert!(announced.contains("?agent="), "{announced}");
        assert!(announced.contains("#token="), "{announced}");

        assert!(!RELOAD_NOTICE.contains(';'));
        assert!(!RELOAD_NOTICE.contains('\n'));
        assert!(RELOAD_NOTICE.contains("再読込"));
    }

    #[test]
    fn auth_failure_guidance_is_announced_and_token_free() {
        let guidance = crate::agent::RECONNECT_GUIDANCE;
        assert!(crate::agent::requires_reconnect(guidance));
        let status_text = format!("エージェントから取得できません: {guidance}");
        // ステータスバーの 160 字切り詰めでも再入力の導線が残ること。
        let displayed = truncate_label(&status_text, 160);
        assert!(displayed.contains("#token="), "{displayed}");
        assert!(displayed.contains("貼り付けて"), "{displayed}");

        let announced = a11y_live_text(
            Some(&StatusMessage {
                text: status_text,
                is_error: true,
                persistent: false,
            }),
            false,
        );
        assert!(announced.contains("エラー:"), "{announced}");
        assert!(announced.contains("#token="), "{announced}");
        assert!(
            !announced.contains("super-secret-agent-token"),
            "{announced}"
        );
    }

    #[test]
    fn reconnect_guidance_is_sticky_across_transient_failures() {
        use crate::agent::RECONNECT_GUIDANCE;
        assert!(next_reconnect_required(false, RECONNECT_GUIDANCE));
        // A generic follow-up failure must not clear the auth guidance.
        assert!(next_reconnect_required(true, "接続に失敗しました"));
        assert!(next_reconnect_required(
            true,
            "エージェントが HTTP 500 を返しました"
        ));
        // Without a prior auth error, a generic error stays non-reconnect.
        assert!(!next_reconnect_required(false, "接続に失敗しました"));
    }

    #[test]
    fn dom_proof_reconnect_field_is_additive_and_token_free() {
        let token = "super-secret-agent-token";
        let legacy = "mode=live;tick=5;rows=3;theme=dark;interval=2000;export=none;";
        for required in [false, true] {
            let proof = proof_with_reconnect(legacy, required);
            assert!(proof.starts_with(legacy), "{proof}");
            assert_eq!(proof, format!("{legacy}reconnect={};", u8::from(required)));
            assert!(!proof.contains(token), "{proof}");
            assert!(!proof.contains("エラー"), "{proof}");
            for field in ["mode=live", "tick=5", "interval=2000", "export=none"] {
                assert!(
                    proof.split(';').any(|part| part == field),
                    "{field} missing from {proof}"
                );
            }
        }
        // 既存フックは末尾追加だけなので不変（interval/export との連結も確認）。
        let proof = proof_with_reconnect(
            &proof_with_export(&proof_with_interval("mode=demo;tick=7;", 2_000), None),
            false,
        );
        assert_eq!(
            proof,
            "mode=demo;tick=7;interval=2000;export=none;reconnect=0;"
        );
    }

    #[test]
    fn capability_gating_preserves_existing_dom_aria_and_cdp_constants() {
        // CDP 検証スクリプトはこれらの数値からクリック座標を再計算する。
        // 取得不可セクションを追加してもカード枠を残すので、値は不変。
        assert_eq!(HEADER_HEIGHT, 52.0);
        assert_eq!(OVERVIEW_HEIGHT, 260.0);
        assert_eq!(TOOLBAR_HEIGHT, 36.0);
        assert_eq!(STATUS_HEIGHT, 26.0);
        assert_eq!(GAP, 12.0);
        assert_eq!(ROOT_PAD, 12.0);
        assert_eq!(DETAILS_WIDTH, 320.0);

        // 既存 DOM hook の key=value; はバイト単位で不変（末尾追加のみ）。
        let proof = proof_with_export(&proof_with_interval("mode=demo;tick=7;", 2_000), None);
        assert_eq!(proof, "mode=demo;tick=7;interval=2000;export=none;");
        assert_eq!(A11Y_LIVE_DOM_ID, "tm-web-a11y-live");
        assert_eq!(READONLY_PILL_ARIA, "ブラウザ版ではプロセスを操作できません");
        assert_eq!(
            READONLY_DETAIL_TEXT,
            "ブラウザ版は読み取り専用です。プロセス操作（シグナル送信）は提供しません。"
        );
    }

    #[test]
    fn unavailable_section_text_is_fixed_and_dom_safe() {
        for section in [Section::Network, Section::Disks, Section::ProcessControl] {
            let text = unavailable_text(section);
            assert!(!text.is_empty(), "{section:?}");
            assert!(!text.contains(';'), "{section:?}: {text}");
            assert!(!text.contains('\n'), "{section:?}: {text}");
        }
        assert!(unavailable_text(Section::Network).contains("ネットワーク"));
        assert!(unavailable_text(Section::Disks).contains("ディスク"));
        assert_eq!(
            unavailable_text(Section::ProcessControl),
            READONLY_PILL_ARIA
        );
    }

    #[test]
    fn web_shell_sections_follow_platform_capabilities() {
        // web の実データソース（デモ / エージェント経由）は network/disks を
        // 提供し、プロセス制御だけが無効。カード枠は残し中身だけ差し替える。
        for info in [
            PlatformInfo::demo(),
            PlatformInfo::native_read_only("Linux (エージェント経由)"),
        ] {
            assert!(info.supports(Section::Network));
            assert!(info.supports(Section::Disks));
            assert!(!info.supports(Section::ProcessControl));
        }
        let capable = PlatformInfo::native("Linux");
        assert!(capable.supports(Section::Network));
        assert!(capable.supports(Section::Disks));
        assert!(capable.supports(Section::ProcessControl));
    }
}

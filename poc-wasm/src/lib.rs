//! GPUI（gpui-pre 0.3.12）Web バックエンドのスパイク用 PoC。
//!
//! 目的: ブラウザ上で「テキスト・表・スクロール・検索入力・クリック・定期更新・HTTP fetch・CJK」
//! が実現できるかを Runtime Proof 付きで確認する。
//!
//! 設計メモ:
//! - `gpui_platform::application()` ではなく `gpui_web::WebPlatform` を直接構築している。
//!   理由は (1) CJK を Canvas フォールバックで描画する `CanvasFontFallback::EmojiAndCjk` を
//!   有効にするため、(2) `FetchHttpClient` をアプリの HTTP クライアントとして注入するため。
//! - 実装はすべて本スパイク用の自作コード。AGPL プロジェクトのソースはコピーしていない。
//! - 検証用フックとして `document.title` と `body[data-gpui-proof]` に状態を書き出す。
//!   これはヘッドレス Chrome（CDP）から DOM 経由で Runtime Proof を取るための計装。

#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use std::{borrow::Cow, time::Duration};

use futures::AsyncReadExt as _;
use gpui::http_client::{AsyncBody, HttpClient};
use gpui::{
    AnyElement, App, Context, FocusHandle, KeyDownEvent, Render, SharedString, Window,
    WindowOptions, div, prelude::*, px, rgb,
};

// Catppuccin Mocha 風の配色（UI は自作）。
const BG: u32 = 0x1e1e2e;
const PANEL: u32 = 0x252537;
const BORDER: u32 = 0x3b3b52;
const TEXT: u32 = 0xcdd6f4;
const MUTED: u32 = 0x8a8aa3;
const ACCENT: u32 = 0x89b4fa;
const GREEN: u32 = 0xa6e3a1;
const YELLOW: u32 = 0xf9e2af;
const RED: u32 = 0xf38ba8;

const FONT_SANS: &str = "IBM Plex Sans";
const FONT_MONO: &str = "IBM Plex Mono";

/// プロセス表の 1 行（ブラウザから OS の実プロセスは読めないためデモデータ）。
struct ProcessRow {
    pid: u32,
    name: SharedString,
    user: SharedString,
    cpu: f32,
    mem_mb: u32,
}

struct MonitorApp {
    focus_handle: FocusHandle,
    cpu_percent: f32,
    mem_used_mb: u32,
    mem_total_mb: u32,
    ticks: u64,
    query: String,
    clicks: u64,
    selected_pid: Option<u32>,
    processes: Vec<ProcessRow>,
    http_status: SharedString,
    http_body: SharedString,
    rng_state: u64,
}

impl MonitorApp {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        // 検索入力を最初からフォーカスしておく（キーボードイベント検証のため）。
        window.focus(&focus_handle, cx);

        let app = Self {
            focus_handle,
            cpu_percent: 37.5,
            mem_used_mb: 10_240,
            mem_total_mb: 16_384,
            ticks: 0,
            query: String::new(),
            clicks: 0,
            selected_pid: None,
            processes: demo_processes(),
            http_status: "fetch 待ち".into(),
            http_body: String::new().into(),
            rng_state: 0x9e37_79b9_7f4a_7c15,
        };
        app.spawn_ticker(cx);
        app.spawn_fetch(cx);
        app.sync_proof_dom();
        app
    }

    /// 1 秒ごとに疑似メトリクスを更新する定期タスク（cx.spawn + BackgroundExecutor::timer）。
    fn spawn_ticker(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let alive = this.update(cx, |app, cx| {
                    app.tick();
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// 同一オリジンへの HTTP fetch（gpui-pre-web の FetchHttpClient 経由）。
    fn spawn_fetch(&self, cx: &mut Context<Self>) {
        let client = cx.http_client();
        cx.spawn(async move |this, cx| {
            let (status, body) = fetch_stats(client).await;
            log::info!(
                "fetch /stats.json => status={}, body_len={}",
                status,
                body.len()
            );
            this.update(cx, |app, cx| {
                app.http_status = status.into();
                app.http_body = body.into();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn tick(&mut self) {
        self.ticks += 1;
        let n = self.next_rand();
        self.cpu_percent = 8.0 + (n % 7_800) as f32 / 100.0;
        let n = self.next_rand();
        self.mem_used_mb = 6_000 + (n % 8_000) as u32;
        let updates: Vec<(f32, u32)> = (0..self.processes.len())
            .map(|index| {
                let cpu = ((self.next_rand() >> (index % 8)) % 2_400) as f32 / 100.0;
                let mem_mb = 32 + (self.next_rand() % 2_400) as u32;
                (cpu, mem_mb)
            })
            .collect();
        for (process, (cpu, mem_mb)) in self.processes.iter_mut().zip(updates) {
            process.cpu = cpu;
            process.mem_mb = mem_mb;
        }
        self.sync_proof_dom();
    }

    fn next_rand(&mut self) -> u64 {
        self.rng_state = self
            .rng_state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.rng_state >> 33
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.control || keystroke.modifiers.alt || keystroke.modifiers.platform {
            return;
        }
        match keystroke.key.as_str() {
            "backspace" => {
                self.query.pop();
            }
            "escape" => self.query.clear(),
            "enter" => {
                self.clicks += 1;
            }
            _ => {
                if let Some(text) = keystroke.key_char.as_deref() {
                    if !text.is_empty() && !keystroke.modifiers.function {
                        self.query.push_str(text);
                    }
                }
            }
        }
        self.sync_proof_dom();
        cx.notify();
    }

    /// CDP（ヘッドレス Chrome）から観測できるよう、状態を DOM に書き出す計装。
    fn sync_proof_dom(&self) {
        #[cfg(target_family = "wasm")]
        {
            let Some(window) = web_sys::window() else {
                return;
            };
            let Some(document) = window.document() else {
                return;
            };
            document.set_title(&format!(
                "GPUI Web Spike | tick={} cpu={:.1}% mem={}MB clicks={} query={}",
                self.ticks, self.cpu_percent, self.mem_used_mb, self.clicks, self.query
            ));
            if let Some(body) = document.body() {
                let _ = body.set_attribute(
                    "data-gpui-proof",
                    &format!(
                        "tick={};cpu={:.1};mem_mb={};clicks={};query={};selected_pid={};http={}",
                        self.ticks,
                        self.cpu_percent,
                        self.mem_used_mb,
                        self.clicks,
                        self.query,
                        self.selected_pid
                            .map(|pid| pid.to_string())
                            .unwrap_or_else(|| "-".to_string()),
                        self.http_status
                    ),
                );
            }
        }
    }

    fn render_card(
        label: &'static str,
        value: String,
        detail: String,
        accent: u32,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .flex_1()
            .p_3()
            .rounded_md()
            .bg(rgb(PANEL))
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(label.to_string()),
            )
            .child(
                div()
                    .text_2xl()
                    .font_family(FONT_MONO)
                    .text_color(rgb(accent))
                    .child(value),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(detail),
            )
    }

    fn render_row(&self, process: &ProcessRow, cx: &mut Context<Self>) -> AnyElement {
        let pid = process.pid;
        let selected = self.selected_pid == Some(pid);
        div()
            .id(("proc-row", pid as usize))
            .flex()
            .items_center()
            .h(px(30.))
            .px_3()
            .gap_2()
            .bg(if selected {
                rgb(0x313244)
            } else {
                rgb(PANEL)
            })
            .hover(|style| style.bg(rgb(0x2b2b40)))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _event, _window, cx| {
                this.selected_pid = Some(pid);
                this.clicks += 1;
                this.sync_proof_dom();
                cx.notify();
            }))
            .child(
                div()
                    .w(px(72.))
                    .font_family(FONT_MONO)
                    .text_xs()
                    .text_color(rgb(if selected { ACCENT } else { MUTED }))
                    .child(process.pid.to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(rgb(TEXT))
                    .child(process.name.clone()),
            )
            .child(
                div()
                    .w(px(96.))
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(process.user.clone()),
            )
            .child(
                div()
                    .w(px(84.))
                    .font_family(FONT_MONO)
                    .text_xs()
                    .text_color(rgb(if process.cpu > 50.0 { RED } else { GREEN }))
                    .child(format!("{:5.1}%", process.cpu)),
            )
            .child(
                div()
                    .w(px(96.))
                    .font_family(FONT_MONO)
                    .text_xs()
                    .text_color(rgb(TEXT))
                    .child(format!("{} MB", process.mem_mb)),
            )
            .into_any_element()
    }
}

impl Render for MonitorApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);
        let query = self.query.to_lowercase();
        let rows: Vec<AnyElement> = self
            .processes
            .iter()
            .filter(|process| {
                query.is_empty()
                    || process.name.to_lowercase().contains(&query)
                    || process.pid.to_string().contains(&query)
                    || process.user.to_lowercase().contains(&query)
            })
            .map(|process| self.render_row(process, cx))
            .collect();
        let filtered_count = rows.len();
        let mem_percent = if self.mem_total_mb == 0 {
            0.0
        } else {
            self.mem_used_mb as f32 / self.mem_total_mb as f32 * 100.0
        };
        let http_preview: String = self.http_body.chars().take(110).collect();
        let selected = self
            .selected_pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "-".to_string());

        div()
            .id("root")
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_5()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .font_family(FONT_SANS)
            // ---- ヘッダ ----
            .child(
                div()
                    .flex()
                    .items_end()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_2xl()
                                    .text_color(rgb(TEXT))
                                    .child("GPUI Web システムモニタ"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child(
                                        "gpui-pre 0.3.12 / wasm32-unknown-unknown / wgpu (WebGPU→WebGL2 fallback)",
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_end()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child(format!("更新回数: {} 回（1 秒間隔）", self.ticks)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(if focused { GREEN } else { YELLOW }))
                                    .child(if focused {
                                        "検索入力: フォーカス中（キー入力可）"
                                    } else {
                                        "検索入力: 未フォーカス（クリックでフォーカス）"
                                    }),
                            ),
                    ),
            )
            // ---- メトリクスカード ----
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(Self::render_card(
                        "CPU 使用率",
                        format!("{:5.1}%", self.cpu_percent),
                        "疑似データ（1 秒ごとに更新）".to_string(),
                        ACCENT,
                    ))
                    .child(Self::render_card(
                        "メモリ",
                        format!("{} MB", self.mem_used_mb),
                        format!("{} MB 中 / {:.1}%", self.mem_total_mb, mem_percent),
                        GREEN,
                    ))
                    .child(Self::render_card(
                        "クリック回数",
                        format!("{}", self.clicks),
                        format!("選択中 PID: {}", selected),
                        YELLOW,
                    ))
                    .child(Self::render_card(
                        "HTTP fetch",
                        self.http_status.to_string(),
                        http_preview,
                        RED,
                    )),
            )
            // ---- 検索入力 ----
            .child(
                div()
                    .id("search")
                    .track_focus(&self.focus_handle)
                    .on_key_down(cx.listener(Self::on_key_down))
                    .on_click(cx.listener(|this, _event, window, cx| {
                        this.focus_handle.focus(window, cx);
                        cx.notify();
                    }))
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(38.))
                    .px_3()
                    .rounded_md()
                    .border_1()
                    .border_color(if focused { rgb(ACCENT) } else { rgb(BORDER) })
                    .bg(rgb(PANEL))
                    .cursor_pointer()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("検索 (プロセス名 / PID / ユーザー)"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(rgb(if self.query.is_empty() {
                                MUTED
                            } else {
                                TEXT
                            }))
                            .child(if self.query.is_empty() {
                                "クリックして入力（例: render / 128）".to_string()
                            } else {
                                self.query.clone()
                            }),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(if focused { ACCENT } else { PANEL }))
                            .child("▌"),
                    ),
            )
            // ---- プロセス表 ----
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.))
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .h(px(32.))
                            .px_3()
                            .gap_2()
                            .bg(rgb(0x1a1a28))
                            .child(
                                div()
                                    .w(px(72.))
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child("PID"),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child("プロセス名"),
                            )
                            .child(
                                div()
                                    .w(px(96.))
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child("ユーザー"),
                            )
                            .child(
                                div()
                                    .w(px(84.))
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child("CPU"),
                            )
                            .child(
                                div()
                                    .w(px(96.))
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child("メモリ"),
                            ),
                    )
                    .child(
                        div()
                            .id("proc-list")
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h(px(0.))
                            .overflow_y_scroll()
                            .children(rows),
                    ),
            )
            // ---- フッター ----
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(format!(
                                "表示中: {} 件 / 全 {} 件 ・ 行クリックで選択、ホイールでスクロール、\
                                 Backspace で検索文字を削除",
                                filtered_count,
                                self.processes.len()
                            )),
                    )
                    .child(
                        div()
                            .id("click-button")
                            .flex()
                            .items_center()
                            .justify_center()
                            .h(px(32.))
                            .px_4()
                            .rounded_md()
                            .bg(rgb(0x313244))
                            .hover(|style| style.bg(rgb(0x45475a)))
                            .cursor_pointer()
                            .text_sm()
                            .text_color(rgb(TEXT))
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.clicks += 1;
                                this.sync_proof_dom();
                                cx.notify();
                            }))
                            .child("クリックでカウント +1"),
                    ),
            )
    }
}

async fn fetch_stats(client: std::sync::Arc<dyn HttpClient>) -> (String, String) {
    match client
        .get("/stats.json", AsyncBody::empty(), true)
        .await
    {
        Ok(response) => {
            let status = format!("HTTP {}", response.status().as_u16());
            let mut body = response.into_body();
            let mut buffer = Vec::new();
            match body.read_to_end(&mut buffer).await {
                Ok(_) => (status, String::from_utf8_lossy(&buffer).into_owned()),
                Err(error) => (status, format!("body read error: {error:#}")),
            }
        }
        Err(error) => ("fetch 失敗".to_string(), format!("{error:#}")),
    }
}

fn demo_processes() -> Vec<ProcessRow> {
    let rows = [
        (128, "gpui-web-spike", "user", "WebAssembly レンダラ本体"),
        (256, "システムモニタ", "user", "CPU / メモリ表示"),
        (512, "バックグラウンド更新", "system", "1 秒タイマーで再描画"),
        (768, "検索インデックス", "user", "日本語テキストを含む行"),
        (1024, "HTTPクライアント", "system", "FetchHttpClient で stats.json 取得"),
        (1280, "日本語フォント描画テスト", "user", "CJK グリフの表示確認"),
        (1536, "スクロール検証用ダミー行 1", "system", "リストのスクロール対象"),
        (1792, "スクロール検証用ダミー行 2", "system", "リストのスクロール対象"),
        (2048, "スクロール検証用ダミー行 3", "system", "リストのスクロール対象"),
        (2304, "スクロール検証用ダミー行 4", "system", "リストのスクロール対象"),
        (2560, "スクロール検証用ダミー行 5", "system", "リストのスクロール対象"),
        (2816, "スクロール検証用ダミー行 6", "system", "リストのスクロール対象"),
        (3072, "スクロール検証用ダミー行 7", "system", "リストのスクロール対象"),
        (3328, "スクロール検証用ダミー行 8", "system", "リストのスクロール対象"),
        (3584, "スクロール検証用ダミー行 9", "system", "リストのスクロール対象"),
        (3840, "スクロール検証用ダミー行 10", "system", "リストのスクロール対象"),
        (4096, "スクロール検証用ダミー行 11", "system", "リストのスクロール対象"),
        (4352, "スクロール検証用ダミー行 12", "system", "リストのスクロール対象"),
    ];
    rows.iter()
        .map(|(pid, name, user, _detail)| ProcessRow {
            pid: *pid,
            name: (*name).into(),
            user: (*user).into(),
            cpu: 1.0,
            mem_mb: 64,
        })
        .collect()
}

fn load_fonts(cx: &App) -> anyhow::Result<()> {
    let fonts: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf").as_slice()),
    ];
    cx.text_system().add_fonts(fonts)
}

/// URL クエリで描画バックエンドを切り替える（検証用）。
/// 例: `?backend=webgpu` / `?backend=webgl` / 既定は Auto（WebGPU → WebGL2 fallback）。
fn backend_preference() -> gpui_web::WebBackendPreference {
    #[cfg(target_family = "wasm")]
    {
        let search = web_sys::window()
            .and_then(|window| window.location().search().ok())
            .unwrap_or_default();
        if search.contains("backend=webgpu") {
            return gpui_web::WebBackendPreference::WebGpu;
        }
        if search.contains("backend=webgl") {
            return gpui_web::WebBackendPreference::WebGl;
        }
    }
    gpui_web::WebBackendPreference::Auto
}

fn run() {
    let platform = std::rc::Rc::new(
        gpui_web::WebPlatform::new_with_backend_and_font_fallback(
            true,
            backend_preference(),
            gpui_web::CanvasFontFallback::EmojiAndCjk,
        ),
    );
    // ブラウザの Fetch API を使う HTTP クライアントを注入する。
    let http_client = std::sync::Arc::new(platform.fetch_http_client());
    let app = gpui::Application::with_platform(platform).with_http_client(http_client);

    app.run(|cx: &mut App| {
        if let Err(error) = load_fonts(cx) {
            log::error!("フォントのロードに失敗: {error:#}");
        } else {
            log::info!("アプリケーションフォントをロードしました (IBM Plex Sans/Mono)");
        }

        if let Err(error) = cx.open_window(WindowOptions::default(), |window, cx| {
            cx.new(|cx| MonitorApp::new(window, cx))
        }) {
            log::error!("ウィンドウ作成に失敗: {error:#}");
        }
    });
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    gpui_platform::web_init();
    log::info!("GPUI web spike: start()");
    run();
}

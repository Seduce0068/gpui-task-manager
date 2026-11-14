//! Browser (wasm) GPUI shell for the GPUI task manager.
//!
//! The shell is read-only by design: a browser cannot read `/proc`, so the data
//! comes either from the deterministic `tm_core::demo::DemoProbe` (default) or
//! from a loopback agent (`tm-agent`) via `GET /api/v1/snapshot`.
//!
//! Initialisation follows the spike in `../../poc-wasm/` (same project):
//! `gpui_web::WebPlatform` is built directly so that the CJK canvas fallback and
//! the browser `FetchHttpClient` can be installed; `gpui_platform::application()`
//! cannot configure either of them.

#![forbid(unsafe_code)]

#[cfg(target_family = "wasm")]
use std::borrow::Cow;

#[cfg(target_family = "wasm")]
use gpui::{App, Application, WindowOptions, prelude::*};

mod agent;
mod shell;
mod theme;
mod widgets;

#[cfg(target_family = "wasm")]
use shell::TaskManagerApp;

#[cfg(target_family = "wasm")]
fn load_fonts(cx: &App) -> anyhow::Result<()> {
    let fonts: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf").as_slice()),
    ];
    cx.text_system().add_fonts(fonts)
}

#[cfg(target_family = "wasm")]
fn run() {
    // WebPlatform を直接構築する理由:
    // 1. CJK を Canvas フォールバックで描画する `CanvasFontFallback::EmojiAndCjk` を有効にするため
    //    （既定の `Emoji` では日本語が描画されない）。
    // 2. ブラウザの Fetch API を使う HTTP クライアントを注入するため
    //    （`Application` の既定は NullHttpClient で fetch が失敗する）。
    let platform = std::rc::Rc::new(gpui_web::WebPlatform::new_with_backend_and_font_fallback(
        true,
        gpui_web::WebBackendPreference::Auto,
        gpui_web::CanvasFontFallback::EmojiAndCjk,
    ));
    let http_client = std::sync::Arc::new(platform.fetch_http_client());
    let app = Application::with_platform(platform).with_http_client(http_client);

    app.run(|cx: &mut App| {
        if let Err(error) = load_fonts(cx) {
            log::error!("フォントのロードに失敗: {error:#}");
        } else {
            log::info!("アプリケーションフォントをロードしました (IBM Plex Sans/Mono)");
        }

        if let Err(error) = cx.open_window(WindowOptions::default(), |window, cx| {
            cx.new(|cx| TaskManagerApp::new(window, cx))
        }) {
            log::error!("ウィンドウ作成に失敗: {error:#}");
        }
    });
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    gpui_platform::web_init();
    log::info!("tm-web: start()");
    run();
}

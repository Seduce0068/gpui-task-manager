//! GPUI Task Manager — native shell entry point.
//!
//! The binary crate forbids `unsafe` like every other crate in the workspace.
//!
//! Run with `cargo run -p gpui-task-manager` for live system data, or
//! `cargo run -p gpui-task-manager -- --demo` for deterministic demo data.

#![forbid(unsafe_code)]

mod app;
mod settings;
mod theme;
mod ui;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::prelude::*;
use gpui_kit::*;

fn main() {
    if handle_cli() {
        return;
    }
    // `args()` panics on non-UTF-8 arguments; use `args_os()` so a stray byte
    // in argv cannot abort the GUI before it starts (flags are plain ASCII).
    let arguments: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let flags = match parse_gui_flags(&arguments) {
        Ok(flags) => flags,
        Err(message) => {
            eprintln!("gpui-task-manager: {message}");
            std::process::exit(2);
        }
    };
    let demo = flags.demo;
    let redact = flags.redact || std::env::var("TM_REDACT").is_ok_and(|value| value == "1");

    let application = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    application.run(move |cx: &mut App| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);

        cx.on_window_closed(|cx, _window_id| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1280.0), px(820.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("GPUI Task Manager".into()),
                ..Default::default()
            }),
            focus: true,
            show: true,
            app_id: Some("gpui-task-manager".to_string()),
            ..Default::default()
        };

        gpui_kit::open_window(options, cx, move |window, cx| {
            let view = cx.new(|cx| app::TaskManagerApp::new(cx, demo, redact));
            view.update(cx, |this, cx| this.start(cx));
            let focus_handle = view.read(cx).focus_handle().clone();
            focus_handle.focus(window, cx);
            view
        })
        .expect("failed to open the task manager window");

        cx.activate(true);
    });
}

/// Parsed GUI-only flags (no GPUI dependency, so it is unit-testable).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct GuiFlags {
    demo: bool,
    redact: bool,
}

/// Parses `--demo` / `--redact` without panicking on non-UTF-8 argv.
///
/// A `--flag=value` form is rejected rather than silently ignored: someone
/// asking for `--demo` by mistake must not end up showing real process data.
fn parse_gui_flags(args: &[std::ffi::OsString]) -> Result<GuiFlags, String> {
    let mut flags = GuiFlags::default();
    for argument in args {
        let text = argument.to_string_lossy();
        for (flag, slot) in [("--demo", &mut flags.demo), ("--redact", &mut flags.redact)] {
            if text == flag {
                *slot = true;
            } else if text.starts_with(flag) && text[flag.len()..].starts_with('=') {
                // Do not echo `text`: a mistaken `--demo=<secret>` would put
                // the secret in stderr, so name only the flag.
                return Err(format!("{flag} に値は指定できません"));
            }
        }
    }
    Ok(flags)
}

/// Handles `--version` / `--help` before GPUI starts.
///
/// Returns `true` when the process should exit without opening a window.
fn handle_cli() -> bool {
    let arguments: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if arguments
        .iter()
        .any(|argument| argument == "--version" || argument == "-V")
    {
        println!("gpui-task-manager {}", env!("CARGO_PKG_VERSION"));
        return true;
    }
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        println!(
            "gpui-task-manager {}\n\n\
             使い方: gpui-task-manager [--demo] [--redact]\n\n\
             --demo    合成データで起動します（スクリーンショット・ブラウザ比較用）\n\
             --redact  ホスト名・ユーザー名・ホームパスを表示時に伏せます（公開用スクリーンショット）\n\n\
             主なキー: ↑↓ 選択 / Space 一時停止 / Ctrl+F 絞り込み /\n\
             Ctrl+K 選択プロセスの終了 / F2 詳細パネル / F5 更新 / F1 ヘルプ\n\n\
             ライセンス: Apache-2.0",
            env!("CARGO_PKG_VERSION")
        );
        return true;
    }
    false
}

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

/// Parsed GUI flags (no GPUI dependency, so it is unit-testable).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct GuiFlags {
    demo: bool,
    redact: bool,
    /// Accepted and validated for parity with `tm-agent`. This GUI has no
    /// structured logging to configure, so the value is recorded only.
    #[allow(dead_code)]
    log_level: LogLevel,
}

/// Severity accepted by `--log-level` (same accept set as `tm-agent`).
/// Matching is ASCII case-insensitive; the canonical spelling is lowercase.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

/// One-line identity printed by `--version` (`<binary> <version>`), the same
/// shape as `tm-agent --version`.
const VERSION_LINE: &str = concat!("gpui-task-manager ", env!("CARGO_PKG_VERSION"));

/// Help text. The `使い方 / オプション / 終了コード` layout mirrors the
/// agent's `USAGE` so both binaries present the same CLI contract.
fn help_text() -> String {
    format!(
        concat!(
            "gpui-task-manager {version} — ネイティブ GPUI シェル\n\n",
            "使い方:\n",
            "  gpui-task-manager [--demo] [--redact] [--log-level LEVEL] [-V, --version] [-h, --help]\n\n",
            "オプション:\n",
            "  --demo            合成データで起動します（スクリーンショット・ブラウザ比較用）\n",
            "  --redact          ホスト名・ユーザー名・ホームパスを表示時に伏せます（公開用スクリーンショット）\n",
            "  --log-level LEVEL ログレベル (error/warn/info/debug/trace、既定 info、受け付け・検証のみ)\n",
            "  -V, --version     バージョンを 1 行で表示して終了 (tm-agent と同一形式)\n",
            "  -h, --help        このヘルプを表示\n\n",
            "主なキー: ↑↓ 選択 / Space 一時停止 / Ctrl+F 絞り込み /\n",
            "Ctrl+K 選択プロセスの終了 / F2 詳細パネル / F5 更新 / F1 ヘルプ\n\n",
            "終了コード (agent と共通の分類。gpui-task-manager は 0 / 2 を返します):\n",
            "  0 正常 (--help / --version を含む) / 1 内部エラー / 2 引数・設定エラー / 3 ネットワークエラー\n",
            "  4 プローブエラー / 5 認証・トークンエラー\n\n",
            "ライセンス: Apache-2.0\n",
        ),
        version = env!("CARGO_PKG_VERSION")
    )
}

/// `--version` / `--help` short-circuit, like the agent's early exit.
///
/// Pure so the precedence can be regression-tested without touching argv.
/// Version wins over help, matching the previous implementation. Only exact
/// arguments count: `--version=1` is an unknown flag handled by the strict
/// parser below, not a version request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EarlyAction {
    Version,
    Help,
}

fn early_action(args: &[std::ffi::OsString]) -> Option<EarlyAction> {
    if args
        .iter()
        .any(|argument| argument == "--version" || argument == "-V")
    {
        Some(EarlyAction::Version)
    } else if args
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        Some(EarlyAction::Help)
    } else {
        None
    }
}

/// Handles `--version` / `--help` before GPUI starts.
///
/// Returns `true` when the process should exit without opening a window.
fn handle_cli() -> bool {
    let arguments: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    match early_action(&arguments) {
        Some(EarlyAction::Version) => {
            println!("{VERSION_LINE}");
            true
        }
        Some(EarlyAction::Help) => {
            print!("{}", help_text());
            true
        }
        None => false,
    }
}

/// Parses `--demo` / `--redact` / `--log-level` without panicking on non-UTF-8
/// argv.
///
/// A boolean `--flag=value` form is rejected rather than silently ignored:
/// someone asking for `--demo` by mistake must not end up showing real process
/// data. The raw argument is never echoed in an error, so a mistaken
/// `--demo=<secret>` cannot put the secret in stderr.
fn parse_gui_flags(args: &[std::ffi::OsString]) -> Result<GuiFlags, String> {
    let mut flags = GuiFlags::default();
    let mut args = args.iter();
    while let Some(argument) = args.next() {
        let text = argument.to_string_lossy();
        let (flag, inline_value) = match text.split_once('=') {
            Some((flag, value)) => (flag, Some(value)),
            None => (text.as_ref(), None),
        };
        match flag {
            "--demo" => {
                reject_inline_value("--demo", inline_value)?;
                flags.demo = true;
            }
            "--redact" => {
                reject_inline_value("--redact", inline_value)?;
                flags.redact = true;
            }
            "--log-level" => {
                let value = match inline_value {
                    Some(value) => value.to_string(),
                    None => args
                        .next()
                        .map(|argument| argument.to_string_lossy().into_owned())
                        .ok_or_else(|| "--log-level には値が必要です".to_string())?,
                };
                flags.log_level = parse_log_level(&value)?;
            }
            _ => {
                if flag.starts_with('-') {
                    // A typo such as `--redcat` must not silently disable
                    // redaction (SECURITY.md public-evidence hygiene). Echo
                    // only the fact that an unknown flag was seen, never its
                    // value.
                    return Err("不明なフラグがあります".to_string());
                }
                // Non-flag arguments were ignored before; keep that behaviour.
            }
        }
    }
    Ok(flags)
}

/// Rejects `--demo=...`-style values for boolean flags.
fn reject_inline_value(flag: &str, inline_value: Option<&str>) -> Result<(), String> {
    if inline_value.is_some() {
        Err(format!("{flag} に値は指定できません"))
    } else {
        Ok(())
    }
}

/// Parses `--log-level` with the same accept set, default and error text as
/// `tm-agent`. The raw value is never echoed in the error message.
fn parse_log_level(value: &str) -> Result<LogLevel, String> {
    let level = match value.trim().to_ascii_lowercase().as_str() {
        "error" => LogLevel::Error,
        "warn" => LogLevel::Warn,
        "info" => LogLevel::Info,
        "debug" => LogLevel::Debug,
        "trace" => LogLevel::Trace,
        _ => {
            return Err(
                "--log-level には error/warn/info/debug/trace のいずれかを指定してください"
                    .to_string(),
            )
        }
    };
    Ok(level)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<std::ffi::OsString> {
        values.iter().map(std::ffi::OsString::from).collect()
    }

    // `use gpui_kit::*` imports GPUI's `test` macro, so name Rust's explicitly.
    #[::core::prelude::v1::test]
    fn version_line_uses_the_shared_binary_space_version_shape() {
        assert_eq!(
            VERSION_LINE,
            format!("gpui-task-manager {}", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(VERSION_LINE.split_whitespace().count(), 2);
    }

    #[::core::prelude::v1::test]
    fn early_action_short_circuits_version_before_help() {
        assert_eq!(
            early_action(&args(&["--version"])),
            Some(EarlyAction::Version)
        );
        assert_eq!(early_action(&args(&["-V"])), Some(EarlyAction::Version));
        assert_eq!(early_action(&args(&["--help"])), Some(EarlyAction::Help));
        assert_eq!(early_action(&args(&["-h"])), Some(EarlyAction::Help));
        assert_eq!(
            early_action(&args(&["--help", "--version"])),
            Some(EarlyAction::Version)
        );
        assert_eq!(early_action(&args(&[])), None);
        // Inline forms are not the short-circuit flags; the strict parser
        // rejects them instead of printing a version banner.
        assert_eq!(early_action(&args(&["--version=1"])), None);
        assert_eq!(early_action(&args(&["-V=1"])), None);
    }

    #[::core::prelude::v1::test]
    fn help_text_documents_the_shared_cli_contract() {
        let help = help_text();
        assert!(help.starts_with("gpui-task-manager "), "{help}");
        for fragment in [
            "使い方:",
            "オプション:",
            "--demo",
            "--redact",
            "--log-level LEVEL",
            "error/warn/info/debug/trace",
            "--version",
            "--help",
            "終了コード",
            "0 正常",
            "2 引数・設定エラー",
        ] {
            assert!(help.contains(fragment), "missing {fragment:?}: {help}");
        }
    }

    #[::core::prelude::v1::test]
    fn gui_flags_default_to_live_data_and_info_level() {
        let flags = parse_gui_flags(&args(&[])).expect("empty argv");
        assert!(!flags.demo);
        assert!(!flags.redact);
        assert_eq!(flags.log_level, LogLevel::Info);
    }

    #[::core::prelude::v1::test]
    fn parses_all_gui_flags_and_log_level_forms() {
        let flags =
            parse_gui_flags(&args(&["--demo", "--redact", "--log-level", "trace"])).expect("flags");
        assert!(flags.demo);
        assert!(flags.redact);
        assert_eq!(flags.log_level, LogLevel::Trace);

        let inline = parse_gui_flags(&args(&["--log-level=WARN"])).expect("inline level");
        assert_eq!(inline.log_level, LogLevel::Warn);
        assert!(!inline.demo);

        for (value, expected) in [
            ("error", LogLevel::Error),
            ("warn", LogLevel::Warn),
            ("INFO", LogLevel::Info),
            ("Debug", LogLevel::Debug),
            (" trace ", LogLevel::Trace),
        ] {
            let flags = parse_gui_flags(&args(&["--log-level", value])).expect(value);
            assert_eq!(flags.log_level, expected, "{value}");
        }
    }

    #[::core::prelude::v1::test]
    fn rejects_invalid_log_levels_without_echoing_the_value() {
        for value in ["", "   ", "verbose", "info=1", "SECRET-TOKEN"] {
            let error = parse_gui_flags(&args(&["--log-level", value])).expect_err("invalid level");
            assert!(error.contains("error/warn/info/debug/trace"), "{error}");
            if !value.trim().is_empty() {
                assert!(!error.contains(value), "value must not be echoed: {error}");
            }
        }
    }

    #[::core::prelude::v1::test]
    fn status_markers_and_signal_notes_are_visible_to_the_native_shell() {
        // The status bar is rendered by `app::app`, but its contract is owned
        // by the viewmodel. Pin the colour-free severity markers and the
        // signal-specific note here so a viewmodel change cannot silently
        // regress the native shell (R3-U03 / R3-U04).
        use tm_viewmodel::{signal_confirmation_note, STATUS_PREFIX_ERROR, STATUS_PREFIX_INFO};
        assert_eq!(STATUS_PREFIX_INFO, "[情報]");
        assert_eq!(STATUS_PREFIX_ERROR, "[エラー]");
        assert!(signal_confirmation_note(tm_core::Signal::Kill).contains("取り消せません"));
        assert!(!signal_confirmation_note(tm_core::Signal::Stop).contains("取り消せません"));
    }

    #[::core::prelude::v1::test]
    fn status_and_dialog_changes_leave_the_cli_contract_unchanged() {
        // These are GUI-only changes: help text, version output and flag
        // parsing must keep the shared agent/app CLI shape.
        let help = help_text();
        assert!(help.contains("Ctrl+K 選択プロセスの終了"));
        assert!(help.contains("error/warn/info/debug/trace"));
        assert_eq!(
            VERSION_LINE,
            format!("gpui-task-manager {}", env!("CARGO_PKG_VERSION"))
        );
        assert!(parse_gui_flags(&args(&["--log-level", "trace"])).is_ok());
    }

    #[::core::prelude::v1::test]
    fn boolean_flags_still_reject_inline_values_and_typos() {
        assert!(parse_gui_flags(&args(&["--demo=yes"])).is_err());
        assert!(parse_gui_flags(&args(&["--redact=1"])).is_err());
        assert!(parse_gui_flags(&args(&["--redcat"])).is_err());
        assert!(parse_gui_flags(&args(&["--log-level"])).is_err());
        // Non-flag positional arguments remain ignored as before.
        assert!(parse_gui_flags(&args(&["some/path"])).is_ok());
    }
}

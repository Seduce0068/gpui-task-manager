//! GPUI Task Manager — native shell entry point.
//!
//! The binary crate forbids `unsafe` like every other crate in the workspace.
//!
//! Run with `cargo run -p gpui-task-manager` for live system data, or
//! `cargo run -p gpui-task-manager -- --demo` for deterministic demo data.
//!
//! `--export csv|json <path>` samples one snapshot, renders it with the pure
//! `tm_core::export` functions (redaction is always applied) and writes the
//! file with mode `0600`, then exits without opening a window. Failures are
//! reported through a classified exit status and a short stderr message.

#![forbid(unsafe_code)]

mod app;
mod settings;
mod theme;
mod ui;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::prelude::*;
use gpui_kit::*;

/// Classifies a probe error for stderr without echoing its free-form detail.
///
/// `ProbeError` payloads can embed internal paths or host names. Shell output
/// keeps only the variant, which is enough to triage a failure without leaking
/// identifiers. The detailed message stays on the UI status path, which
/// applies its own redaction (the same policy as `agent::probe_error_label`).
fn probe_error_label(error: &tm_core::ProbeError) -> &'static str {
    match error {
        tm_core::ProbeError::Unsupported(_) => "未対応",
        tm_core::ProbeError::Io(_) => "I/O エラー",
        tm_core::ProbeError::Permission(_) => "権限不足",
    }
}

/// Classified stderr line for a failed native probe initialisation.
fn probe_init_failure_message(error: &tm_core::ProbeError) -> String {
    format!(
        "スナップショット収集を初期化できません ({})",
        probe_error_label(error)
    )
}

/// Classified stderr line for a failed snapshot sample.
fn probe_sample_failure_message(error: &tm_core::ProbeError) -> String {
    format!("スナップショット取得に失敗 ({})", probe_error_label(error))
}

/// Writes `content` to `writer`, discarding every I/O error.
///
/// Rust ignores `SIGPIPE` on Unix, so a downstream that closes early — e.g.
/// `gpui-task-manager --version | head -c 1` — makes the write return
/// `BrokenPipe` instead of terminating the process. `println!` / `print!`
/// unwrap that error and panic ("failed printing to stdout", exit 101), which
/// is a false alarm for an ordinary shell pipeline. Restoring the default
/// `SIGPIPE` disposition would need `unsafe` FFI or a new `libc` dependency;
/// AGENTS.md §15 forbids `unsafe`, and a dependency is unnecessary. Ignoring
/// the `write_all` result gives the same quiet-exit behaviour in safe Rust.
fn write_ignoring_io_errors(writer: &mut impl std::io::Write, content: &str) {
    let _ = writer.write_all(content.as_bytes());
    let _ = writer.flush();
}

/// Terminal output for `--version` / `--help` (both exit before GPUI starts).
fn write_stdout(content: &str) {
    let stdout = std::io::stdout();
    let mut locked = stdout.lock();
    write_ignoring_io_errors(&mut locked, content);
}

fn main() {
    // `args()` panics on non-UTF-8 arguments; use `args_os()` so a stray byte
    // in argv cannot abort the GUI before it starts (flags are plain ASCII).
    let arguments: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    // Validate the *whole* argv before honoring `--version` / `--help`, exactly
    // like `tm-agent::run`: a bad flag next to `--version` is a usage error
    // (exit 2), never a version banner.
    let flags = match resolve_cli(&arguments) {
        CliAction::Version => {
            write_stdout(&format!("{VERSION_LINE}\n"));
            return;
        }
        CliAction::Help => {
            write_stdout(&help_text());
            return;
        }
        CliAction::UsageError(message) => {
            eprintln!("gpui-task-manager: {message}");
            std::process::exit(EXIT_USAGE);
        }
        CliAction::Run(flags) => flags,
    };
    // `--export` is a headless mode: no GPUI window, no event loop. The pure
    // exporter always redacts, so `--redact` / `TM_REDACT` need not be passed.
    if let Some(request) = flags.export.as_ref() {
        let status = run_export(request, flags.demo);
        if status != EXIT_OK {
            std::process::exit(status);
        }
        return;
    }
    // A failed native probe is a classified error for every shell entry point:
    // demo data is only ever an explicit, validated `--demo` opt-in (R3-S02/S03).
    // `NativeProbe::new` only refreshes its own process table, and the GUI's
    // viewmodel creates its own probe when the window opens, so the probe
    // created for this preflight is intentionally discarded.
    let demo = match tm_core::demo::choose_backend(flags.demo, || {
        tm_platform::NativeProbe::new().map(|_| ())
    }) {
        Ok(tm_core::demo::BackendChoice::Demo) => true,
        Ok(tm_core::demo::BackendChoice::Native(())) => false,
        Err(error) => {
            eprintln!("gpui-task-manager: {}", probe_init_failure_message(&error));
            std::process::exit(EXIT_PROBE);
        }
    };
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
                // Show the data source in the title bar too: task switchers
                // and screenshots must not hide that this is synthetic data.
                title: Some(if demo {
                    "GPUI Task Manager [デモデータ]".into()
                } else {
                    "GPUI Task Manager".into()
                }),
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
#[derive(Clone, Debug, PartialEq, Eq, Default)]
struct GuiFlags {
    demo: bool,
    redact: bool,
    /// `--version` / `-V`: validated like every other flag, then handled by
    /// `resolve_cli` before the GUI starts (same order as `tm-agent`).
    version: bool,
    /// `--help` / `-h`: validated like every other flag, then handled by
    /// `resolve_cli` when no version was requested.
    help: bool,
    /// Headless export request; `None` keeps the normal GUI startup path.
    export: Option<ExportRequest>,
    /// Accepted and validated for parity with `tm-agent`. This GUI has no
    /// structured logging to configure, so the value is recorded only.
    #[allow(dead_code)]
    log_level: LogLevel,
}

/// File format requested by `--export`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExportFormat {
    Csv,
    Json,
}

impl ExportFormat {
    fn label(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }
}

/// A validated `--export <format> <path>` request.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ExportRequest {
    format: ExportFormat,
    path: std::path::PathBuf,
}

/// Exit status categories shared by `tm-agent` and this binary (help text).
const EXIT_OK: i32 = 0;
const EXIT_INTERNAL: i32 = 1;
const EXIT_USAGE: i32 = 2;
const EXIT_PROBE: i32 = 4;

/// Discarded-sample delay for `--export` with a native probe. Matches the
/// agent's warm-up (must exceed sysinfo's ~200 ms minimum CPU interval).
fn export_warmup_interval() -> std::time::Duration {
    std::time::Duration::from_millis(250)
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
            "  gpui-task-manager [--demo] [--redact] [--log-level LEVEL]\n",
            "                    [--export csv|json PATH] [-V, --version] [-h, --help]\n\n",
            "オプション:\n",
            "  --demo            合成データで起動します（スクリーンショット・ブラウザ比較用）。\n",
            "                     probe 失敗時の暗黙切替は行いません（不正値は終了コード 2）\n",
            "  --redact          ホスト名・ユーザー名・ホームパスを表示時に伏せます（公開用スクリーンショット）\n",
            "                    環境変数 TM_REDACT=1 でも同じ伏せ字化が有効\n",
            "  --log-level LEVEL ログレベル (error/warn/info/debug/trace、既定 info、受け付け・検証のみ)\n",
            "  --export csv|json PATH  スナップショットを 1 回取得し、伏せ字化した CSV/JSON を\n",
            "                         PATH へ 0600 で書き出して終了（ウィンドウは開きません）\n",
            "  -V, --version     バージョンを 1 行で表示して終了 (tm-agent と同一形式)\n",
            "  -h, --help        このヘルプを表示\n\n",
            "主なキー: ↑↓ 選択 / Space 一時停止 / Ctrl+F 絞り込み /\n",
            "Ctrl+K 選択プロセスの終了 / F2 詳細パネル / F5 更新 / F1 ヘルプ\n\n",
            "終了コード (gpui-task-manager が返す値のみ):\n",
            "  0 正常 (--help / --version を含む) / 1 内部エラー / 2 引数・設定エラー\n",
            "  4 プローブエラー（3 ネットワーク / 5 認証は tm-agent 固有でこのバイナリは返しません）\n\n",
            "ライセンス: AGPL-3.0-or-later\n",
        ),
        version = env!("CARGO_PKG_VERSION")
    )
}

/// Outcome of validating argv before any GPUI window is created.
///
/// `tm-agent` parses every argument (including `--version` / `--help`) into its
/// config and only then checks `version` before `help`. The GUI mirrors that
/// order: an invalid argument anywhere in argv wins over either terminal flag.
#[derive(Clone, Debug, PartialEq, Eq)]
enum CliAction {
    /// All arguments were valid and `--version` / `-V` was requested.
    Version,
    /// All arguments were valid and `--help` / `-h` was requested.
    Help,
    /// Validation failed; the message goes to stderr with exit code 2.
    UsageError(String),
    /// Validation passed with no terminal flag: start the GUI.
    Run(GuiFlags),
}

fn resolve_cli(arguments: &[std::ffi::OsString]) -> CliAction {
    match parse_gui_flags(arguments) {
        Err(message) => CliAction::UsageError(message),
        Ok(flags) if flags.version => CliAction::Version,
        Ok(flags) if flags.help => CliAction::Help,
        Ok(flags) => CliAction::Run(flags),
    }
}

/// Parses all GUI flags (including `--version` / `--help`) without panicking on
/// non-UTF-8 argv.
///
/// A boolean `--flag=value` form is rejected rather than silently ignored:
/// someone asking for `--demo` by mistake must not end up showing real process
/// data. Unknown flags *and* bare positional arguments are both rejected, and
/// the raw argument is never echoed in an error, so a mistaken `--demo=<secret>`
/// or a misplaced token cannot put the secret in stderr.
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
            "--version" | "-V" => {
                reject_inline_value(flag, inline_value)?;
                flags.version = true;
            }
            "--help" | "-h" => {
                reject_inline_value(flag, inline_value)?;
                flags.help = true;
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
            "--export" => {
                let format_value = match inline_value {
                    Some(value) => std::borrow::Cow::Borrowed(value),
                    None => args
                        .next()
                        .map(|argument| argument.to_string_lossy())
                        .ok_or_else(|| "--export には csv または json が必要です".to_string())?,
                };
                // A flag-looking value means the format was omitted. Reject it
                // instead of consuming e.g. `--demo` as a file name.
                if format_value.starts_with('-') {
                    return Err("--export には csv または json が必要です".to_string());
                }
                let format = parse_export_format(&format_value)?;
                let path_value = args
                    .next()
                    .ok_or_else(|| "--export には出力先パスが必要です".to_string())?;
                if path_value.to_string_lossy().starts_with('-') {
                    return Err("--export には出力先パスが必要です".to_string());
                }
                flags.export = Some(ExportRequest {
                    format,
                    path: std::path::PathBuf::from(path_value.as_os_str()),
                });
            }
            _ => {
                // tm-agent rejects every unrecognised argument — flags and bare
                // positional values alike — with one fixed message. The raw
                // value is never echoed: a misplaced token or path must not
                // reach stderr (SECURITY.md public-evidence hygiene).
                return Err("不明な引数があります".to_string());
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
            );
        }
    };
    Ok(level)
}

/// Parses the `--export` format with a fixed accept set. As with
/// `--log-level`, the raw value is never echoed so a mistyped secret cannot
/// reach stderr.
fn parse_export_format(value: &str) -> Result<ExportFormat, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "csv" => Ok(ExportFormat::Csv),
        "json" => Ok(ExportFormat::Json),
        _ => Err("--export には csv または json を指定してください".to_string()),
    }
}

/// Renders one snapshot with the pure `tm_core::export` functions.
///
/// The identifiers used for masking are captured **before** any redaction:
/// the host name comes from the snapshot and home/user come from the
/// environment. `tm_core::export` re-applies redaction to a private clone, so
/// an unredacted snapshot cannot leak local identifiers even when the caller
/// did not pass `--redact`.
///
/// The native probe has no wire envelope, so there is no machine-wide total to
/// preserve: pass `None` and let the exporter publish the sampled row count, as
/// before. Only the web shell, which owns the agent's `SnapshotEnvelope`, can
/// supply an explicit total.
fn render_export(snapshot: &tm_core::SystemSnapshot, format: ExportFormat) -> String {
    let hostname = snapshot.hostname.clone().unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default();
    let ids = tm_core::redact::RedactionIdentifiers {
        home: &home,
        user: &user,
        hostname: &hostname,
    };
    match format {
        ExportFormat::Csv => tm_core::export::snapshot_to_csv_with_total(snapshot, &ids, None),
        ExportFormat::Json => tm_core::export::snapshot_to_json_with_total(snapshot, &ids, None),
    }
}

/// Returns only the leaf name of an export destination for operator messages.
///
/// The full path can contain the user's home directory or internal mount
/// points, so error and success logs must never echo it. The file name keeps
/// the message actionable without revealing the directory layout.
fn export_target_name(path: &std::path::Path) -> std::borrow::Cow<'_, str> {
    path.file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| std::borrow::Cow::Borrowed("(名前なし)"))
}

/// Writes an export atomically to `path` with mode `0600` on Unix.
///
/// A PID/nonce/timestamp temporary file in the target directory is created
/// with `0600` from the start and renamed into place, so an existing file
/// cannot keep a permissive mode and a failed write leaves no partial output.
/// A failure to open, write or rename is returned with the file name only,
/// never the full path or the file contents.
///
/// Known limitation (unsupported): resource-limit signals such as `SIGXFSZ`
/// (exceeding `RLIMIT_FSIZE`, e.g. under `ulimit -f`) are not caught. The
/// default signal action terminates the process before the `io::Error`
/// cleanup path below runs, so the 0600 temporary file can be left behind.
/// Ordinary I/O failures (permission denied, ENOSPC, ...) do surface as
/// `io::Error` and are cleaned up. Catching the signal would need `unsafe`
/// FFI or a new dependency; per AGENTS §15 (no `unsafe`, minimal dependencies)
/// signal handling is explicitly out of scope.
fn write_export(path: &std::path::Path, contents: &str) -> Result<(), String> {
    use std::io::Write as _;

    // Reject non-regular destinations before creating a temporary file.
    // `rename(2)` replaces a symlink/FIFO/device node *itself*, so exporting to
    // `/dev/stdout` as root would destroy the symlink (R45 export-I/O audit).
    match std::fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_file() => {
            return Err(format!(
                "エクスポート先は通常ファイルではありません: {}",
                export_target_name(path)
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "エクスポート先を確認できません ({}): {error}",
                export_target_name(path)
            ));
        }
    }
    // Even when the path does not exist yet, refuse to create files under
    // virtual filesystems where a rename could clobber a device node.
    if path
        .components()
        .next()
        .and_then(|first| match first {
            std::path::Component::RootDir => path.components().nth(1),
            _ => None,
        })
        .and_then(|first| first.as_os_str().to_str())
        .is_some_and(|top| matches!(top, "dev" | "proc" | "sys"))
    {
        return Err(format!(
            "エクスポート先が仮想ファイルシステム配下です: {}",
            export_target_name(path)
        ));
    }
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| "エクスポート先のファイル名が不正です".to_string())?;
    static EXPORT_NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = EXPORT_NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let temporary = parent.join(format!(
        ".{name}.export.tmp.{}.{:x}.{:x}",
        std::process::id(),
        nanos,
        nonce,
        name = name.to_string_lossy()
    ));
    let written = (|| -> std::io::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            // `create_new` + `mode(0600)` avoids a umask-dependent 0644 window
            // and refuses to reuse a stale/poisoned temporary file.
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        // `mode(0o600)` is still masked by the process umask (e.g. umask 0777
        // yields 0000). fchmod via set_permissions ignores umask, matching the
        // settings writer: the export must remain readable after rename.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(contents.as_bytes())?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written.map_err(|error| {
        format!(
            "エクスポート先に書き込めません ({}): {error}",
            export_target_name(path)
        )
    })
}

/// Samples one snapshot and writes the requested format.
///
/// Returns a classified exit status: `4` when the probe cannot be created or
/// sampled, `1` when the file cannot be written, `0` on success. As on the GUI
/// startup path, a failed native probe is an error here: silently exporting
/// demo data would produce misleading evidence (R3-S02).
fn run_export(request: &ExportRequest, demo: bool) -> i32 {
    // Same policy as the GUI startup: `--demo` is the only route to synthetic
    // data, and a native probe failure is an error rather than a silent switch.
    let mut probe: Box<dyn tm_core::SystemProbe> =
        match tm_core::demo::choose_backend(demo, tm_platform::NativeProbe::new) {
            Ok(tm_core::demo::BackendChoice::Demo) => Box::new(tm_core::demo::DemoProbe::new()),
            Ok(tm_core::demo::BackendChoice::Native(probe)) => Box::new(probe),
            Err(error) => {
                eprintln!("gpui-task-manager: {}", probe_init_failure_message(&error));
                return EXIT_PROBE;
            }
        };
    // The native probe needs two samples for per-process CPU: the first one
    // has no previous counters and reports 0 for every process (R44 export
    // audit). Discard it, matching the agent's HTTP/--once warm-up. Demo data
    // is already meaningful on the first tick, so skip the wait there.
    if !demo {
        if let Err(error) = probe.sample() {
            eprintln!(
                "gpui-task-manager: {}",
                probe_sample_failure_message(&error)
            );
            return EXIT_PROBE;
        }
        std::thread::sleep(crate::export_warmup_interval());
    }
    let snapshot = match probe.sample() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            eprintln!(
                "gpui-task-manager: {}",
                probe_sample_failure_message(&error)
            );
            return EXIT_PROBE;
        }
    };
    let contents = render_export(&snapshot, request.format);
    if let Err(error) = write_export(&request.path, &contents) {
        eprintln!("gpui-task-manager: {error}");
        return EXIT_INTERNAL;
    }
    eprintln!(
        "gpui-task-manager: {} を {} 形式でエクスポートしました",
        export_target_name(&request.path),
        request.format.label()
    );
    EXIT_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<std::ffi::OsString> {
        values.iter().map(std::ffi::OsString::from).collect()
    }

    /// A closed stdout (e.g. `| head -c 1`) fails every write with
    /// `BrokenPipe`; that used to make `println!` panic with exit 101.
    #[::core::prelude::v1::test]
    fn terminal_write_ignores_broken_pipe() {
        struct BrokenPipe;
        impl std::io::Write for BrokenPipe {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
            }
        }
        write_ignoring_io_errors(&mut BrokenPipe, VERSION_LINE);
        write_ignoring_io_errors(&mut BrokenPipe, &help_text());

        let mut sink = Vec::new();
        write_ignoring_io_errors(&mut sink, VERSION_LINE);
        assert_eq!(sink, VERSION_LINE.as_bytes());
    }

    /// Exit status a real process would use for this argv: `Some(0)` for the
    /// terminal flags, `Some(2)` for any validation failure, `None` when the
    /// GUI actually starts. Mirrors `main`'s `resolve_cli` dispatch exactly.
    fn exit_code_for(values: &[&str]) -> Option<i32> {
        match resolve_cli(&args(values)) {
            CliAction::Version | CliAction::Help => Some(EXIT_OK),
            CliAction::UsageError(_) => Some(EXIT_USAGE),
            CliAction::Run(_) => None,
        }
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
    fn version_wins_over_help_after_full_validation() {
        // Contract change: version/help are no longer an argv-scanning early
        // exit. They are parsed flags, so precedence only applies after every
        // argument has been validated (same order as tm-agent's `run`).
        assert_eq!(resolve_cli(&args(&["--version"])), CliAction::Version);
        assert_eq!(resolve_cli(&args(&["-V"])), CliAction::Version);
        assert_eq!(resolve_cli(&args(&["--help"])), CliAction::Help);
        assert_eq!(resolve_cli(&args(&["-h"])), CliAction::Help);
        assert_eq!(
            resolve_cli(&args(&["--help", "--version"])),
            CliAction::Version
        );
        assert_eq!(
            resolve_cli(&args(&["--version", "--help"])),
            CliAction::Version
        );
        assert!(matches!(resolve_cli(&args(&[])), CliAction::Run(_)));
        // Inline forms are not terminal flags; the strict parser rejects them
        // instead of printing a version banner or help text.
        for inline in ["--version=1", "-V=1", "--help=1", "-h=1"] {
            assert!(
                matches!(resolve_cli(&args(&[inline])), CliAction::UsageError(_)),
                "{inline} must be rejected"
            );
        }
    }

    #[::core::prelude::v1::test]
    fn cli_exit_status_matches_agent_validation_order() {
        // An unknown flag anywhere in argv wins over `--version`: tm-agent
        // validates the whole argument list before printing a version banner.
        assert_eq!(
            exit_code_for(&["--bogusflag", "--version"]),
            Some(EXIT_USAGE)
        );
        assert_eq!(
            exit_code_for(&["--version", "--bogusflag"]),
            Some(EXIT_USAGE)
        );
        assert_eq!(exit_code_for(&["--bogusflag", "--help"]), Some(EXIT_USAGE));
        // Positional arguments are rejected the same way (usage error, 2).
        assert_eq!(exit_code_for(&["some/path"]), Some(EXIT_USAGE));
        assert_eq!(exit_code_for(&["--demo", "file.csv"]), Some(EXIT_USAGE));
        // Plain terminal flags still succeed with exit 0.
        assert_eq!(exit_code_for(&["--version"]), Some(EXIT_OK));
        assert_eq!(exit_code_for(&["-V"]), Some(EXIT_OK));
        assert_eq!(exit_code_for(&["--help"]), Some(EXIT_OK));
        assert_eq!(exit_code_for(&["-h"]), Some(EXIT_OK));
        // Valid non-terminal argv proceeds to GUI startup (no exit status).
        assert_eq!(exit_code_for(&[]), None);
        assert_eq!(exit_code_for(&["--demo", "--redact"]), None);
        // The positional-looking `--export` path is consumed as a flag value,
        // not rejected as a stray positional argument.
        assert_eq!(exit_code_for(&["--export", "csv", "/tmp/out.csv"]), None);
    }

    #[::core::prelude::v1::test]
    fn unknown_flags_and_positional_args_use_the_fixed_message() {
        for argv in [
            vec!["--bogusflag"],
            vec!["--redcat"],
            vec!["some/path"],
            vec!["--demo", "SECRET-POSITIONAL"],
        ] {
            let error = parse_gui_flags(&args(&argv)).expect_err(&format!("{argv:?}"));
            assert!(error.contains("不明な引数があります"), "{error}");
            // The rejected value must never be echoed to stderr.
            assert!(!error.contains("SECRET-POSITIONAL"), "{error}");
            assert!(!error.contains("some/path"), "{error}");
        }
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
            "TM_REDACT=1",
            "--log-level LEVEL",
            "--export csv|json PATH",
            "0600",
            "error/warn/info/debug/trace",
            "--version",
            "--help",
            "終了コード",
            "0 正常",
            "2 引数・設定エラー",
            "このバイナリは返しません",
            "暗黙切替は行いません",
        ] {
            assert!(help.contains(fragment), "missing {fragment:?}: {help}");
        }
    }

    #[::core::prelude::v1::test]
    fn gui_flags_default_to_live_data_and_info_level() {
        let flags = parse_gui_flags(&args(&[])).expect("empty argv");
        assert!(!flags.demo);
        assert!(!flags.redact);
        assert!(!flags.version);
        assert!(!flags.help);
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
        // Positional arguments are no longer ignored: tm-agent rejects them.
        assert!(parse_gui_flags(&args(&["some/path"])).is_err());
    }

    #[::core::prelude::v1::test]
    fn invalid_demo_values_keep_the_exit_2_contract() {
        // R3-S03: an invalid `--demo` must never be silently ignored and fall
        // back to live data. The strict parser rejects every inline form so
        // `main` exits with EXIT_USAGE before GPUI starts.
        for value in [
            "--demo=yes",
            "--demo=1",
            "--demo=0",
            "--demo=false",
            "--demo=",
        ] {
            let error = parse_gui_flags(&args(&[value])).expect_err("invalid demo value");
            assert!(error.contains("--demo"), "{error}");
            // The raw value is never echoed: `--demo=<secret>` must not leak.
            if let Some((_, payload)) = value.split_once('=') {
                if !payload.is_empty() {
                    assert!(!error.contains(payload), "{error}");
                }
            }
        }
        assert!(parse_gui_flags(&args(&["--DEMO"])).is_err());
        assert!(parse_gui_flags(&args(&["--dem"])).is_err());
        // Valid forms still work, including when combined with other flags.
        assert!(parse_gui_flags(&args(&["--demo"])).expect("demo").demo);
        assert!(
            parse_gui_flags(&args(&["--demo", "--redact"]))
                .expect("demo + redact")
                .demo
        );
    }

    #[::core::prelude::v1::test]
    fn export_flags_are_parsed_in_separate_and_inline_forms() {
        let csv = parse_gui_flags(&args(&["--export", "csv", "/tmp/out.csv"])).expect("csv");
        assert_eq!(
            csv.export,
            Some(ExportRequest {
                format: ExportFormat::Csv,
                path: std::path::PathBuf::from("/tmp/out.csv"),
            })
        );
        // Format matching is case-insensitive, like `--log-level`.
        let inline =
            parse_gui_flags(&args(&["--export=JSON", "/tmp/out.json"])).expect("inline json");
        assert_eq!(
            inline.export,
            Some(ExportRequest {
                format: ExportFormat::Json,
                path: std::path::PathBuf::from("/tmp/out.json"),
            })
        );
        // Combining with the existing flags must not change the GUI contract.
        let combined = parse_gui_flags(&args(&[
            "--demo",
            "--redact",
            "--log-level",
            "trace",
            "--export",
            "csv",
            "/tmp/out.csv",
        ]))
        .expect("combined");
        assert!(combined.demo);
        assert!(combined.redact);
        assert_eq!(combined.log_level, LogLevel::Trace);
        assert_eq!(combined.export, csv.export);
    }

    #[::core::prelude::v1::test]
    fn export_rejects_missing_or_unknown_values_without_echoing_them() {
        for argv in [
            vec!["--export"],
            vec!["--export", "csv"],
            vec!["--export", "--demo", "/tmp/out.csv"],
            vec!["--export", "csv", "--demo"],
        ] {
            let error = parse_gui_flags(&args(&argv)).expect_err(&format!("{argv:?}"));
            assert!(error.contains("--export"), "{error}");
        }
        let error = parse_gui_flags(&args(&["--export", "SECRET-FORMAT", "/tmp/out"]))
            .expect_err("unknown format");
        assert!(error.contains("csv または json"), "{error}");
        assert!(
            !error.contains("SECRET-FORMAT"),
            "value must not be echoed: {error}"
        );
    }

    #[::core::prelude::v1::test]
    fn render_export_redacts_local_identifiers_by_default() {
        let home = std::env::var("HOME").unwrap_or_default();
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_default();
        let hostname = "secret-test-host";
        let mut snapshot = tm_core::demo::demo_snapshot(0);
        snapshot.hostname = Some(hostname.to_string());
        snapshot.processes[0].user = Some(user.clone());
        snapshot.processes[0].command =
            format!("{home}/private/app --issuer=https://{hostname}.tail1234ab.ts.net:8443");
        snapshot.disks[0].mount_point = format!("{home}/work/project");

        for format in [ExportFormat::Csv, ExportFormat::Json] {
            let output = render_export(&snapshot, format);
            assert!(
                !output.contains(hostname),
                "{format:?} leaked hostname: {output}"
            );
            assert!(
                !output.contains("tail1234ab"),
                "{format:?} leaked tailnet id: {output}"
            );
            if !home.is_empty() {
                assert!(!output.contains(&home), "{format:?} leaked home: {output}");
            }
            if !user.is_empty() {
                assert!(!output.contains(&user), "{format:?} leaked user: {output}");
            }
        }
    }

    #[::core::prelude::v1::test]
    fn render_export_publishes_sampled_rows_without_an_envelope_total() {
        // The CLI samples a local probe: there is no `?limit=` envelope, so
        // `total_process_count` must stay equal to the published row count.
        let snapshot = tm_core::demo::demo_snapshot(0);
        let rows = snapshot.processes.len();
        assert!(rows > 0, "demo probe must publish at least one row");

        let json = render_export(&snapshot, ExportFormat::Json);
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON export");
        assert_eq!(
            value["total_process_count"].as_u64(),
            Some(rows as u64),
            "{json}"
        );
        assert_eq!(
            value["snapshot"]["processes"]
                .as_array()
                .map(|processes| processes.len()),
            Some(rows)
        );

        let csv = render_export(&snapshot, ExportFormat::Csv);
        assert_eq!(csv.lines().count(), rows + 1, "{csv}");
    }

    #[::core::prelude::v1::test]
    fn export_messages_use_only_the_file_name() {
        let path = std::path::Path::new("/home/alice/private/project/out.json");
        assert_eq!(export_target_name(path), "out.json");
        assert!(!export_target_name(path).contains("alice"));
        assert_eq!(export_target_name(std::path::Path::new("/")), "(名前なし)");
    }

    #[::core::prelude::v1::test]
    fn export_write_is_atomic_0600_and_replaces_a_permissive_file() {
        let dir = std::env::temp_dir().join(format!("gtm-export-mode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("out.json");
        std::fs::write(&path, "old contents").expect("seed file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
                .expect("chmod 0644");
        }
        write_export(&path, "new contents").expect("write export");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read export"),
            "new contents"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "export must not be world readable");
        }
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains("export.tmp"))
            .collect();
        assert!(leftovers.is_empty(), "no half-written temp file may remain");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[::core::prelude::v1::test]
    fn export_write_reports_failure_without_leaving_output() {
        let dir = std::env::temp_dir().join(format!("gtm-export-fail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("missing-parent").join("out.csv");
        let error = write_export(&path, "payload").expect_err("missing parent must fail");
        assert!(error.contains("エクスポート先に書き込めません"), "{error}");
        assert!(error.contains("out.csv"), "{error}");
        assert!(
            !error.contains(dir.to_string_lossy().as_ref()),
            "the absolute path must not leak: {error}"
        );
        assert!(!path.exists(), "a failed export must not create output");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[::core::prelude::v1::test]
    fn export_warmup_exceeds_sysinfo_minimum_cpu_interval() {
        // A shorter delay would produce all-zero per-process CPU in `--export`
        // (the first sample has no previous counters) — R44 audit.
        assert_eq!(export_warmup_interval().as_millis(), 250);
    }

    #[::core::prelude::v1::test]
    fn run_export_demo_writes_a_redacted_file_and_returns_success() {
        let dir = std::env::temp_dir().join(format!("gtm-export-demo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("snapshot.json");
        let request = ExportRequest {
            format: ExportFormat::Json,
            path: path.clone(),
        };
        assert_eq!(run_export(&request, true), EXIT_OK);
        let text = std::fs::read_to_string(&path).expect("export file");
        assert!(text.contains("\"schema_version\": 1"), "{text}");
        assert!(text.contains("\"total_process_count\""), "{text}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "export must not be world readable");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[::core::prelude::v1::test]
    fn run_export_reports_write_failure_as_internal_error_status() {
        let dir = std::env::temp_dir().join(format!("gtm-export-status-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        // A directory cannot be replaced by a renamed file: exercise the
        // failure status promised by the CLI contract.
        let request = ExportRequest {
            format: ExportFormat::Csv,
            path: dir.clone(),
        };
        assert_eq!(run_export(&request, true), EXIT_INTERNAL);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[::core::prelude::v1::test]
    fn write_export_rejects_special_destinations() {
        // A rename would replace a symlink or device node instead of writing
        // through it; exporting to /dev/stdout as root breaks the system (R45).
        assert!(write_export(std::path::Path::new("/dev/stdout"), "x").is_err());
        assert!(write_export(std::path::Path::new("/proc/self/status"), "x").is_err());

        let dir = std::env::temp_dir().join(format!("gtm-export-special-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let link = dir.join("link.json");
        let target = dir.join("target.json");
        let _ = std::fs::write(&target, "original");
        #[cfg(unix)]
        {
            let _ = std::os::unix::fs::symlink(&target, &link);
            assert!(write_export(&link, "overwrite").is_err());
            assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[::core::prelude::v1::test]
    fn probe_failure_is_reported_instead_of_switching_to_demo() {
        // R3-S02: both the GUI preflight and `--export` use this policy.
        // Without `--demo`, a failed native probe is an error (exit 4); with
        // `--demo`, synthetic data is used even when the native probe fails.
        use tm_core::demo::{choose_backend, probe_failure_message, BackendChoice};
        let failure = || Err::<(), _>(tm_core::ProbeError::Io("初期化失敗".to_string()));
        assert!(matches!(
            choose_backend(false, failure),
            Err(tm_core::ProbeError::Io(_))
        ));
        assert_eq!(choose_backend(true, failure), Ok(BackendChoice::Demo));
        assert_eq!(
            EXIT_PROBE, 4,
            "the classified probe status is part of the CLI contract"
        );
        assert!(
            probe_failure_message(&tm_core::ProbeError::Io("初期化失敗".to_string()))
                .contains("初期化失敗")
        );
    }

    #[::core::prelude::v1::test]
    fn probe_error_label_is_fixed_per_variant() {
        let detail = "SECRET-PATH /home/victim HOST internal.example";
        let cases = [
            (
                tm_core::ProbeError::Unsupported(detail.to_string()),
                "未対応",
            ),
            (tm_core::ProbeError::Io(detail.to_string()), "I/O エラー"),
            (
                tm_core::ProbeError::Permission(detail.to_string()),
                "権限不足",
            ),
        ];
        for (error, label) in cases {
            assert_eq!(probe_error_label(&error), label);
        }
    }

    #[::core::prelude::v1::test]
    fn probe_stderr_messages_never_echo_the_error_detail() {
        // The GUI preflight and both `--export` probe paths must print only the
        // classified label: a native ProbeError detail can carry internal paths
        // or host names that would otherwise land in shell/CI logs.
        let detail = "SECRET-PATH /home/victim HOST internal.example";
        for error in [
            tm_core::ProbeError::Unsupported(detail.to_string()),
            tm_core::ProbeError::Io(detail.to_string()),
            tm_core::ProbeError::Permission(detail.to_string()),
        ] {
            for message in [
                probe_init_failure_message(&error),
                probe_sample_failure_message(&error),
            ] {
                for secret in ["SECRET-PATH", "/home/victim", "internal.example"] {
                    assert!(
                        !message.contains(secret),
                        "stderr must not echo the probe detail: {message}"
                    );
                }
                assert!(
                    message.contains(probe_error_label(&error)),
                    "stderr must keep the classified label: {message}"
                );
            }
        }
    }
}

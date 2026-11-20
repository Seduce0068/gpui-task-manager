//! Command-line configuration for the agent.
//!
//! Parsing is hand-written so the binary keeps a `std`-only dependency set. The
//! parser is strict on purpose: an unknown flag is an error, and a non-loopback
//! listen address is rejected before a socket is opened.

use std::net::SocketAddr;
use std::path::PathBuf;

/// Default listen address (loopback only).
pub const DEFAULT_LISTEN: &str = "127.0.0.1:8787";

/// Severity accepted by `--log-level` (same accept set in both binaries).
///
/// The canonical spelling is lowercase, but matching is ASCII case-insensitive
/// so `--log-level=INFO` behaves identically in the agent and in the app.
// Parsed here and applied by `main.rs` as the minimum structured-log level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

/// Help text shown for `--help`.
///
/// The first line and the `使い方 / オプション / 終了コード` sections mirror the
/// app's help text so both binaries present the same CLI contract. `concat!`
/// keeps the printed version tied to this crate's Cargo version.
pub const USAGE: &str = concat!(
    "tm-agent ",
    env!("CARGO_PKG_VERSION"),
    " — 読み取り専用のループバック・スナップショットエージェント\n\n",
    "使い方:\n",
    "  tm-agent [--listen ADDR] [--token TOKEN | --token-file PATH] [--demo] [--once] [--redact]\n",
    "           [--log-level LEVEL] [-V, --version] [-h, --help]\n\n",
    "オプション:\n",
    "  --listen ADDR   待ち受けアドレス (既定: 127.0.0.1:8787、ループバックのみ)\n",
    "  --token TOKEN   Bearer トークン (テスト用。/proc/<pid>/cmdline は既定で同一ホストの全ユーザーが読める)\n",
    "  --token-file PATH  トークンをファイルから読む (末尾の改行は除去、最も推奨)\n",
    "  TM_AGENT_TOKEN  環境変数でもトークンを渡せる (--token/--token-file より後に解決。\n",
    "                  /proc/<pid>/environ に残存し得るため --token-file より推奨度は低い)\n",
    "  --demo          実機ではなく決定的なデモデータを返す\n",
    "  --once          スナップショットを 1 回だけ stdout に JSON で出して終了（先頭 500 件、認証なし）。\n",
    "                  --token/--token-file/TM_AGENT_TOKEN はサーバーモードと同じ検証を行い、不正なら exit 5。\n",
    "                  自動生成トークンは作成しません（リスナーがなく、ファイル生成の副作用を避けるため）。\n",
    "  --redact        ホスト名・ユーザー名・ホームパスを伏せて返す（公開用スクリーンショット）\n",
    "  --log-level LEVEL  構造化ログの最低レベル (error/warn/info/debug/trace、既定 info、\n",
    "                     受け付け・検証のみ。debug/trace も info と同じ出力)\n",
    "  -V, --version   バージョンを 1 行で表示して終了 (gpui-task-manager と同一形式)\n",
    "  -h, --help      このヘルプを表示\n\n",
    "エンドポイント (GET のみ):\n",
    "  /healthz               {\"ok\":true,\"schema_version\":1}\n",
    "  /api/v1/snapshot       {\"schema_version\":1,\"total_process_count\":N,\"snapshot\":{...}}\n",
    "                         ?limit=N で返すプロセス数を指定 (既定 500、最大 2000)\n",
    "  /api/v1/stream         SSE (text/event-stream、1 秒間隔、同時 2 接続まで、Bearer 必須、\n",
    "                         認証は Authorization ヘッダのみ（クエリトークン不可）、Host/Origin は loopback のみ)\n",
    "  /metrics               Prometheus テキストのカウンタ/ゲージ (認証不要、機械データなし)\n\n",
    "認証:\n",
    "  Authorization: Bearer <token>（クエリ文字列のトークンは拒否）\n",
    "  トークン未指定時は起動時に生成し、0600 のファイルに保存して場所を stderr で案内します（値はログに出力しません）。\n",
    "  再起動時は同じファイルを再利用します。明示的に制御するには --token-file または TM_AGENT_TOKEN を使ってください。\n",
    "  推奨順: --token-file（argv/environ を避けられる）> TM_AGENT_TOKEN（/proc/<pid>/environ に残存し得る）\n",
    "          > --token（/proc/<pid>/cmdline が既定で同一ホストの全ユーザーから読める）。\n",
    "  解決順: --token または --token-file（同時指定はエラー）> TM_AGENT_TOKEN > 自動生成。\n\n",
    "HTTP エラー本文: {\"code\":\"...\",\"message\":\"...\",\"error\":\"...\"}\n",
    "  (error は message の後方互換の別名。 code で分岐してください)\n",
    "HTTP エラーコード:\n",
    "  bad_request / invalid_limit / unauthorized / forbidden / not_found / method_not_allowed\n",
    "  request_too_large / request_timeout / internal / busy\n\n",
    "終了コード (app と共通の分類。tm-agent は 0〜5 を返します):\n",
    "  0 正常 (--help / --version を含む) / 1 内部エラー / 2 引数・設定エラー / 3 ネットワークエラー\n",
    "  4 プローブエラー / 5 認証・トークンエラー\n\n",
    "構造化ログ:\n",
    "  stderr に JSON 1 行 (ts_ms, level, event, ...)。--once の stdout は従来どおり JSON のみ。\n",
);

/// Parsed command-line configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// Socket address to listen on; always a loopback address.
    pub listen: SocketAddr,
    /// Explicit bearer token, or `None` to generate one at start-up.
    pub token: Option<String>,
    /// File containing the bearer token (preferred over `--token`).
    pub token_file: Option<PathBuf>,
    /// Serve deterministic demo data instead of the real machine.
    pub demo: bool,
    /// Print one snapshot and exit instead of serving HTTP.
    pub once: bool,
    /// Redact host name, user names and home paths from the snapshot.
    pub redact: bool,
    /// Print usage and exit.
    pub help: bool,
    /// `--version` / `-V` was requested; `main.rs` prints the version and exits.
    pub version: bool,
    /// Level requested by `--log-level` (default: `info`), applied by `main.rs`.
    pub log_level: LogLevel,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: DEFAULT_LISTEN
                .parse()
                .expect("DEFAULT_LISTEN is a valid socket address"),
            token: None,
            token_file: None,
            demo: false,
            once: false,
            redact: false,
            help: false,
            version: false,
            log_level: LogLevel::Info,
        }
    }
}

/// Parses command-line arguments (without the program name).
///
/// # Errors
///
/// Returns a human-readable message for unknown flags, missing values, an empty
/// token, or a listen address that is not loopback.
pub fn parse<I>(args: I) -> Result<Config, String>
where
    I: IntoIterator<Item = String>,
{
    let mut config = Config::default();
    let mut args = args.into_iter();

    while let Some(argument) = args.next() {
        let (flag, inline_value) = match argument.split_once('=') {
            Some((flag, value)) => (flag, Some(value.to_string())),
            None => (argument.as_str(), None),
        };
        match flag {
            "--listen" => {
                let value = take_value(flag, inline_value, &mut args)?;
                config.listen = value
                    .parse()
                    // Do not echo the supplied value: a mistaken invocation may
                    // have put a token or other secret in this position.
                    .map_err(|_| "--listen に不正なアドレスが指定されました".to_string())?;
            }
            "--token" => {
                let value = take_value(flag, inline_value, &mut args)?;
                if value.trim().is_empty() {
                    return Err("--token に空文字は指定できません".to_string());
                }
                config.token = Some(value);
            }
            "--token-file" => {
                let value = take_value(flag, inline_value, &mut args)?;
                if value.trim().is_empty() {
                    return Err("--token-file に空文字は指定できません".to_string());
                }
                config.token_file = Some(PathBuf::from(value));
            }
            "--demo" => {
                reject_inline_value(flag, inline_value)?;
                config.demo = true;
            }
            "--once" => {
                reject_inline_value(flag, inline_value)?;
                config.once = true;
            }
            "--redact" => {
                reject_inline_value(flag, inline_value)?;
                config.redact = true;
            }
            "--version" | "-V" => {
                reject_inline_value(flag, inline_value)?;
                config.version = true;
            }
            "--log-level" => {
                let value = take_value(flag, inline_value, &mut args)?;
                config.log_level = parse_log_level(&value)?;
            }
            "-h" | "--help" => {
                reject_inline_value(flag, inline_value)?;
                config.help = true;
            }
            // Echo only the fact that an argument was unknown, not the value:
            // a token passed as a positional argument must not reach the log.
            _ => return Err(format!("不明な引数があります\n\n{USAGE}")),
        }
    }

    if config.token.is_some() && config.token_file.is_some() {
        return Err("--token と --token-file は同時に指定できません".to_string());
    }

    // Must match `http::is_allowed_host` exactly: accepting e.g. 127.0.0.2
    // here would let the agent start but reject every browser request whose
    // Host header carries that address.
    let ip = config.listen.ip();
    if ip != std::net::Ipv4Addr::LOCALHOST && ip != std::net::Ipv6Addr::LOCALHOST {
        // Never echo the supplied address: a mistaken invocation may have put
        // an internal IP (or another identifier) in this position.
        return Err("--listen は 127.0.0.1 または [::1] のみ指定できます".to_string());
    }
    Ok(config)
}

/// Returns the value of a flag, either inline (`--flag=value`) or from the next
/// argument (`--flag value`).
fn take_value<I>(flag: &str, inline_value: Option<String>, args: &mut I) -> Result<String, String>
where
    I: Iterator<Item = String>,
{
    match inline_value {
        Some(value) => Ok(value),
        None => args
            .next()
            .ok_or_else(|| format!("{flag} には値が必要です")),
    }
}

/// Rejects `--demo=...`-style values for boolean flags.
fn reject_inline_value(flag: &str, inline_value: Option<String>) -> Result<(), String> {
    match inline_value {
        Some(_) => Err(format!("{flag} は値を取りません")),
        None => Ok(()),
    }
}

/// Parses `--log-level` with the same accept set, default and error text as
/// `gpui-task-manager`. The raw value is never echoed: a mistyped invocation
/// might have put a token or a path in this position.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Config, String> {
        parse(args.iter().map(|argument| (*argument).to_string()))
    }

    #[test]
    fn defaults_to_loopback_and_native_mode() {
        let config = parse_args(&[]).expect("defaults");
        assert_eq!(config.listen, "127.0.0.1:8787".parse().expect("addr"));
        assert_eq!(config.token, None);
        assert_eq!(config.token_file, None);
        assert!(!config.demo);
        assert!(!config.once);
        assert!(!config.redact);
        assert!(!config.help);
        assert!(!config.version);
        assert_eq!(config.log_level, LogLevel::Info);
    }

    #[test]
    fn parses_all_flags() {
        let config = parse_args(&[
            "--listen",
            "127.0.0.1:18787",
            "--token",
            "secret",
            "--demo",
            "--once",
            "--redact",
            "--log-level",
            "debug",
            "--version",
        ])
        .expect("flags");
        assert_eq!(config.listen, "127.0.0.1:18787".parse().expect("addr"));
        assert_eq!(config.token.as_deref(), Some("secret"));
        assert!(config.demo);
        assert!(config.once);
        assert!(config.redact);
        assert_eq!(config.log_level, LogLevel::Debug);
        assert!(config.version);
    }

    #[test]
    fn parses_inline_values() {
        let config =
            parse_args(&["--listen=127.0.0.1:9000", "--token=abc"]).expect("inline values");
        assert_eq!(config.listen, "127.0.0.1:9000".parse().expect("addr"));
        assert_eq!(config.token.as_deref(), Some("abc"));
    }

    #[test]
    fn parses_a_token_file() {
        let config = parse_args(&["--token-file", "/tmp/tm-agent-token"]).expect("token file");
        assert_eq!(
            config.token_file.as_deref(),
            Some(std::path::Path::new("/tmp/tm-agent-token"))
        );
        assert_eq!(config.token, None);
    }

    #[test]
    fn rejects_token_and_token_file_together() {
        let error = parse_args(&["--token", "secret", "--token-file", "/tmp/x"])
            .expect_err("must reject two token sources");
        assert!(error.contains("同時に指定できません"), "{error}");
    }

    #[test]
    fn accepts_ipv6_loopback() {
        let config = parse_args(&["--listen", "[::1]:8787"]).expect("ipv6 loopback");
        assert!(config.listen.ip().is_loopback());
    }

    #[test]
    fn rejects_non_loopback_addresses() {
        // 127.0.0.2 is loopback but not in `is_allowed_host`'s allowlist, so
        // accepting it would make a running agent unreachable from the browser.
        for address in [
            "0.0.0.0:8787",
            "192.168.1.10:8787",
            "10.0.0.1:80",
            "127.0.0.2:8787",
        ] {
            let error = parse_args(&["--listen", address]).expect_err("must be rejected");
            assert!(
                error.contains("127.0.0.1"),
                "unexpected error for {address}: {error}"
            );
            assert!(
                !error.contains(address),
                "the rejected address must not be echoed: {error}"
            );
        }
        // A host name is not a socket address at all; it is rejected earlier.
        assert!(parse_args(&["--listen", "example.com:8787"]).is_err());
    }

    #[test]
    fn rejects_bad_arguments() {
        assert!(parse_args(&["--unknown"]).is_err());
        assert!(parse_args(&["--listen"]).is_err());
        assert!(parse_args(&["--token"]).is_err());
        assert!(parse_args(&["--token", "   "]).is_err());
        assert!(parse_args(&["--demo=yes"]).is_err());
        assert!(parse_args(&["--listen", "not-an-address"]).is_err());
    }

    #[test]
    fn help_is_recognised() {
        assert!(parse_args(&["--help"]).expect("help").help);
        assert!(parse_args(&["-h"]).expect("help").help);
    }

    #[test]
    fn version_is_recognised_with_the_app_flag_names() {
        assert!(parse_args(&["--version"]).expect("version").version);
        assert!(parse_args(&["-V"]).expect("version").version);
        let config = parse_args(&["--version", "--log-level", "info"]).expect("combined");
        assert!(config.version);
        assert_eq!(config.log_level, LogLevel::Info);
        assert!(!config.help);
    }

    #[test]
    fn log_level_accepts_the_same_values_as_the_app() {
        for (value, expected) in [
            ("error", LogLevel::Error),
            ("warn", LogLevel::Warn),
            ("INFO", LogLevel::Info),
            ("Debug", LogLevel::Debug),
            (" trace ", LogLevel::Trace),
        ] {
            let config = parse_args(&["--log-level", value]).expect(value);
            assert_eq!(config.log_level, expected, "{value}");
        }
        let inline = parse_args(&["--log-level=warn"]).expect("inline level");
        assert_eq!(inline.log_level, LogLevel::Warn);
    }

    #[test]
    fn invalid_log_levels_are_rejected_without_echoing_the_value() {
        for value in ["", "   ", "verbose", "info=1", "SECRET-TOKEN"] {
            let error = parse_args(&["--log-level", value])
                .expect_err("invalid log level must be rejected");
            assert!(error.contains("error/warn/info/debug/trace"), "{error}");
            if !value.trim().is_empty() {
                assert!(!error.contains(value), "value must not be echoed: {error}");
            }
        }
        assert!(parse_args(&["--log-level"]).is_err());
        assert!(parse_args(&["--version=yes"]).is_err());
        assert!(parse_args(&["-V=1"]).is_err());
    }

    #[test]
    fn usage_documents_the_shared_cli_contract() {
        // Both binaries print `<binary> <version>` as the first help line and
        // use the same section order and exit-code classification.
        assert!(USAGE.starts_with(concat!("tm-agent ", env!("CARGO_PKG_VERSION"))));
        // An auto-generated token must never be advertised as stderr output;
        // only the location of its 0600 file is announced there.
        assert!(USAGE.contains("値はログに出力しません"));
        assert!(USAGE.contains("0600 のファイルに保存して場所を stderr で案内します"));
        assert!(USAGE.contains("再起動時は同じファイルを再利用します"));
        assert!(!USAGE.contains("stderr に 1 回だけ表示"));
        assert!(!USAGE.contains("argv は他プロセスから見える"));
        for fragment in [
            "使い方:",
            "オプション:",
            "--log-level LEVEL",
            "error/warn/info/debug/trace",
            "受け付け・検証のみ。debug/trace も info と同じ出力",
            "/proc/<pid>/cmdline",
            "同一ホストの全ユーザー",
            "--token-file",
            "推奨順",
            "/proc/<pid>/environ",
            "/api/v1/stream",
            "text/event-stream",
            "1 秒間隔",
            "同時 2 接続まで",
            "認証は Authorization ヘッダのみ",
            "クエリトークン不可",
            "Host/Origin は loopback のみ",
            "HTTP エラー本文",
            "HTTP エラーコード",
            "後方互換の別名",
            "bad_request",
            "invalid_limit",
            "unauthorized",
            "forbidden",
            "not_found",
            "method_not_allowed",
            "request_too_large",
            "request_timeout",
            "internal",
            "busy",
            "-V, --version",
            "--redact",
            "--once",
            "終了コード",
            "0 正常",
            "2 引数・設定エラー",
            "3 ネットワークエラー",
            "4 プローブエラー",
            "5 認証・トークンエラー",
        ] {
            assert!(USAGE.contains(fragment), "missing {fragment:?}");
        }
    }
}

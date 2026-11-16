//! Command-line configuration for the agent.
//!
//! Parsing is hand-written so the binary keeps a `std`-only dependency set. The
//! parser is strict on purpose: an unknown flag is an error, and a non-loopback
//! listen address is rejected before a socket is opened.

use std::net::SocketAddr;
use std::path::PathBuf;

/// Default listen address (loopback only).
pub const DEFAULT_LISTEN: &str = "127.0.0.1:8787";

/// Help text shown for `--help`.
pub const USAGE: &str = "\
tm-agent — 読み取り専用のループバック・スナップショットエージェント

使い方:
  tm-agent [--listen ADDR] [--token TOKEN | --token-file PATH] [--demo] [--once] [--redact]

オプション:
  --listen ADDR   待ち受けアドレス (既定: 127.0.0.1:8787、ループバックのみ)
  --token TOKEN   Bearer トークン (テスト用。argv は他プロセスから見える)
  --token-file PATH  トークンをファイルから読む (末尾の改行は除去、推奨)
  TM_AGENT_TOKEN  環境変数でもトークンを渡せる (--token/--token-file より後に解決)
  --demo          実機ではなく決定的なデモデータを返す
  --once          スナップショットを 1 回だけ stdout に JSON で出して終了（先頭 500 件）
  --redact        ホスト名・ユーザー名・ホームパスを伏せて返す（公開用スクリーンショット）
  -h, --help      このヘルプを表示

エンドポイント (GET のみ):
  /healthz               {\"ok\":true,\"schema_version\":1}
  /api/v1/snapshot       {\"schema_version\":1,\"total_process_count\":N,\"snapshot\":{...}}
                         ?limit=N で返すプロセス数を指定 (既定 500、最大 2000)

認証:
  Authorization: Bearer <token>（クエリ文字列のトークンは拒否）
  トークン未指定時は起動時に生成して stderr に 1 回だけ表示する。
  解決順: --token または --token-file（同時指定はエラー）> TM_AGENT_TOKEN > 自動生成。
";

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
                    .map_err(|_| format!("--listen に不正なアドレスが指定されました: {value}"))?;
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
            "-h" | "--help" => {
                reject_inline_value(flag, inline_value)?;
                config.help = true;
            }
            other => return Err(format!("不明な引数です: {other}\n\n{USAGE}")),
        }
    }

    if config.token.is_some() && config.token_file.is_some() {
        return Err("--token と --token-file は同時に指定できません".to_string());
    }

    if !config.listen.ip().is_loopback() {
        return Err(format!(
            "--listen はループバックアドレスのみ指定できます (指定値: {})",
            config.listen
        ));
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
        ])
        .expect("flags");
        assert_eq!(config.listen, "127.0.0.1:18787".parse().expect("addr"));
        assert_eq!(config.token.as_deref(), Some("secret"));
        assert!(config.demo);
        assert!(config.once);
        assert!(config.redact);
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
        for address in ["0.0.0.0:8787", "192.168.1.10:8787", "10.0.0.1:80"] {
            let error = parse_args(&["--listen", address]).expect_err("must be rejected");
            assert!(
                error.contains("ループバック"),
                "unexpected error for {address}: {error}"
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
}

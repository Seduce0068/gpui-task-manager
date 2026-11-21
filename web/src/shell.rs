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
//! - a small DOM hook (`body[data-gpui-proof]`) exposes the state to the
//!   headless-Chrome verification script, because everything else is drawn on
//!   a `<canvas>`. `document.title` is human-facing only (a fixed label plus
//!   the data source) and never carries DOM proof fields or user input.

use std::ops::Range;
use std::sync::{Arc, OnceLock, PoisonError, RwLock};
use std::time::Duration;

use crate::redaction::{OwnedIdentifiers, identifiers_from_snapshot};
use gpui::accesskit::{Live, Node, Role, SortDirection as AccessKitSortDirection, Toggled};
use gpui::{
    AnyElement, Bounds, Context, Div, ElementInputHandler, EntityInputHandler, FocusHandle,
    KeyDownEvent, Modifiers, MouseButton, Pixels, Render, ScrollStrategy, Stateful, Task,
    TextInputConfiguration, UTF16Selection, UniformListScrollHandle, Window, canvas, div,
    prelude::*, px, rgb, uniform_list,
};
use tm_core::demo::DemoProbe;
use tm_core::export::{snapshot_to_csv_with_total, snapshot_to_json_with_total};
use tm_core::format;
use tm_core::port::Section;
use tm_core::{
    PlatformInfo, ProcessKey, ProcessSnapshot, ProcessStatus, SortDirection, SortKey, SortSpec,
    SystemProbe, SystemSnapshot,
};
use tm_viewmodel::{
    Command, DEFAULT_REFRESH_INTERVAL, Dashboard, EditOutcome, Effect, FILTER_PRESETS, FilterInput,
    FilterPresetId, MAX_REFRESH_INTERVAL, MIN_REFRESH_INTERVAL, StatusMessage,
};
#[cfg(target_family = "wasm")]
use wasm_bindgen::JsCast;
#[cfg(target_family = "wasm")]
use wasm_bindgen::prelude::wasm_bindgen;
use web_time::Instant;

use crate::agent::{
    AgentClient, AgentConfig, FetchError, PollSuspendGuard, SnapshotEnvelope, SnapshotResponse,
    onboarding_steps,
};
use crate::settings::{INTERVAL_PRESETS_MS, STORAGE_KEY, interval_label};
use crate::theme::*;
use crate::widgets::{
    accessible_metric_bar, accessible_pill, card, filter_preset_chip, h_flex, kv,
    process_sparkline, process_sparkline_value_text, section_title, sparkline, temperature_gauge,
    temperature_severity, temperature_value_text, trend_badge, usage_severity, usage_value_text,
    v_flex,
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
/// 絞り込み欄の固定幅。CDP 検証のフォールバック座標（ROOT_PAD + 170）が
/// この中に収まる前提なので、プリセット行を追加しても変更しない。
const FILTER_BOX_WIDTH: f32 = 340.0;

/// Screen-reader-only status node mirrored into the DOM on wasm. The canvas
/// itself carries no semantics, so this is the browser-side `aria-live`
/// counterpart of the AccessKit status node in `status_bar()`.
const A11Y_LIVE_DOM_ID: &str = "tm-web-a11y-live";

/// キャンバス内の AccessKit 構造をブラウザ AX へ橋渡しする非表示ミラーのコンテナ。
/// `aria-hidden` は付けない（付けると `getFullAXTree` から消える）。
const A11Y_MIRROR_DOM_ID: &str = "tm-web-a11y-mirror";
/// テーブル＋指標の領域。サンプルごとに差し替えてもヘルプの DOM は触らない。
const A11Y_MIRROR_DATA_DOM_ID: &str = "tm-web-a11y-data";
/// ヘルプダイアログの領域。開閉時だけ更新し、「閉じる」のフォーカスを保持する。
const A11Y_MIRROR_HELP_DOM_ID: &str = "tm-web-a11y-help";
/// ミラー内のヘルプ「閉じる」ボタン。DOM フォーカスの移動先。
const A11Y_HELP_CLOSE_DOM_ID: &str = "tm-web-a11y-help-close";
/// 視覚的に隠すが AX ツリーには残すスタイル。live リージョンとミラーで共用。
const A11Y_HIDDEN_STYLE: &str =
    "position:fixed;width:1px;height:1px;overflow:hidden;clip:rect(0 0 0 0);white-space:nowrap";

/// 詳細パネルの選択プロセス CPU スパークラインの ElementId。`process_sparkline`
/// はフレーム内で一意な id を要求するため、固定値をここで共有する。
const PROCESS_CPU_SPARKLINE_ID: &str = "process-cpu-sparkline";
/// ツールバー中央のプリセット行（AccessibilityKit の Group 名も固定）。
const FILTER_PRESETS_GROUP_ID: &str = "filter-presets";

/// 既存の読み取り専用マーカーに使われている固定文言。headless 検証と aria の
/// 回帰を避けるため、`supports_process_control` の判定を追加しても変更しない。
const READONLY_PILL_ARIA: &str = "ブラウザ版ではプロセスを操作できません";
const READONLY_DETAIL_TEXT: &str =
    "ブラウザ版は読み取り専用です。プロセス操作（シグナル送信）は提供しません。";
/// `Effect::Signal` と Ctrl+K で共用する読み取り専用通知（V3-KEYS）。
///
/// 既存の `handle_command` がシグナル要求に返す文言を定数化しただけなので、
/// 既存の headless 検証・aria 文言はバイト単位で不変。DOM proof の
/// `key=value;` 契約を壊さないよう `;` と改行は含まない。
const READONLY_SIGNAL_NOTICE: &str =
    "ブラウザ版は読み取り専用です。プロセス操作（シグナル送信）は行いません。";
/// F5（手動更新要求）を可視化する固定通知。`tick=` はサンプラーが増やすので、
/// この通知は「要求した」ことだけを表し、サンプル件数を偽装しない。
///
/// 判断（C2-UXKEY）: 選択肢 (A) の `Effect::SampleNow` で `fetch_snapshot` を
/// 1 回実行するならネイティブの即サンプリングと旅程が一致するが、Web の
/// サンプラーは SSE / ポーリング / backoff / stale 判定を single-flight で
/// 管理しており、別経路の one-shot fetch を足すと in-flight の二重取得と
/// 通知優先度（永続通知の上書き）の回帰リスクが生じる。受入条件は
/// 「実更新するか文言を要求のみへ統一する」なので、既存の sampler-owned
/// cadence 契約を壊さない (B) を採用し、文言を要求のみへ明示する。
const MANUAL_REFRESH_NOTICE: &str = "次回の自動更新を待ちます（要求のみ）";
/// Space（一時停止相当）。Web サンプラーは一時停止を持たないため、
/// 無反応にせず理由を status へ出す（V3-KEYS）。
const PAUSE_UNSUPPORTED_NOTICE: &str = "ブラウザ版では更新の一時停止はできません";

/// デモモードの詳細案内（R3-S05、V2-HELP でヘルプ側へ移動）。
///
/// 初回起動のステータスバーには短い [`DEMO_MODE_STATUS_NOTICE`] を出す。
/// 実データ接続の手順はこの全文をヘルプ（F1）に表示するので、8787 の案内は
/// 従来どおり残る。トークン・ホスト名・エージェント応答を一切含まない。
/// `;` / 改行が無いので、将来この文言が DOM フックへ鏡像されても
/// `key=value;` の区切りを壊さない。Web 側でも応答の redact は必須である。
const DEMO_MODE_NOTICE: &str = "デモデータを表示しています。実データを見るには tm-agent を --redact 付きで起動し、URL に ?agent=http://127.0.0.1:8787#token=<トークン> を指定してください";

/// ステータスバー用の短いデモ通知（V2-HELP）。
///
/// 詳細な接続手順はヘルプ（F1）側の [`DEMO_MODE_NOTICE`] に残し、常時見える
/// ステータスバーは info 扱いの永続通知だけにする。';' / 改行 / トークンを
/// 含まないので DOM proof の `key=value;` 契約も壊さない。
const DEMO_MODE_STATUS_NOTICE: &str = "デモ表示中。実データ接続はヘルプ F1 を参照";

/// ヘルプオーバーレイの見出し（ネイティブと同じ固定文言）。
const HELP_TITLE: &str = "キーボード操作";

/// ネイティブの `help_shortcuts()` と同等の一覧（V2-HELP）。
///
/// ブラウザ版は読み取り専用だが、ネイティブと同じ表を表示して操作方法の
/// 認知をそろえる。利用できない行はオーバーレイ内の但し書きで明示する。
const HELP_SHORTCUTS: &[(&str, &str)] = &[
    ("↑ / ↓", "行の選択"),
    ("PgUp / PgDn", "10 行移動"),
    ("Home / End", "先頭 / 末尾"),
    ("Enter / Space", "フォーカスした列見出しで並び替え"),
    ("Space", "更新の一時停止・再開"),
    ("Shift+F10 / ContextMenu", "選択行のメニューを開く"),
    ("Ctrl+F / Cmd+F", "絞り込み入力"),
    ("Ctrl+K / Cmd+K", "選択プロセスの終了"),
    ("F2", "詳細パネルの表示切替"),
    ("F5", "今すぐ更新"),
    ("F1", "このヘルプを開く / 閉じる"),
    ("Enter", "確認ダイアログをキャンセル（安全側）"),
    ("Ctrl/Cmd+Enter", "確認ダイアログでシグナルを実行"),
    (
        "Esc",
        "ダイアログ/ヘルプを閉じる · 絞り込み解除（表示中は閉じる操作が優先）",
    ),
];

/// ページ URL から接続設定を再読込したことを知らせる固定文言（R3-S04）。
const RELOAD_NOTICE: &str = "接続設定を再読込しました。エージェントへ再接続しています…";

/// Fixed notice when a snapshot stream ends cleanly and monitoring falls back
/// to the existing poll. No token, endpoint or agent text, and no `;` / newline
/// that could corrupt the `key=value;` DOM proof grammar.
const SSE_FALLBACK_NOTICE: &str = "ストリーム接続が終了したため、定期取得に切り替えました";

/// Cooldown before retrying a failed stream. Bounds request churn when the
/// agent does not offer SSE, while still recovering without a page reload.
const SSE_RECONNECT_DELAY: Duration = Duration::from_secs(5);

/// Base step of the *poll* backoff (WA-RETRY). One second matches the legacy
/// ~1 req/s cadence, so the first failure does not slow an already slow poll.
/// This is deliberately separate from [`SSE_RECONNECT_DELAY`]: SSE and polling
/// own their own retry state, so a stream error never doubles the poll delay
/// and a failed poll never delays a stream reconnect.
const POLL_BACKOFF_BASE: Duration = Duration::from_secs(1);

/// Ceiling for the poll backoff, including a server `Retry-After` hint.
///
/// Capped so a broken agent cannot park monitoring indefinitely; the ceiling
/// also stays above the sampler's `MAX_REFRESH_INTERVAL` (10 s), so a
/// configured cadence is never shortened by backoff.
const POLL_BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Fixed lower bound for the SSE idle watchdog.
///
/// The agent emits snapshot frames on its own `STREAM_INTERVAL` (1 s in
/// `tm-agent`), which is independent of the sampler's refresh interval.
/// Deriving the watchdog only from `effective_interval()` would shrink the grace
/// window to 750 ms at the sampler's 250 ms floor and misclassify healthy 1 s
/// frames as idle. This 3 s floor tolerates roughly three missed agent frames
/// without coupling the browser watchdog to the agent's cadence.
const MIN_SSE_IDLE_TIMEOUT: Duration = Duration::from_secs(3);

/// 行メニューを開いてからプロセスが入れ替わった（PID 再利用）ときの固定文言。
/// 数値 PID すら含めず、`;` / 改行も無いので DOM proof / aria の契約を壊さない。
const STALE_ROW_MENU_NOTICE: &str = "行のプロセスが更新されたため、PID をコピーしませんでした";

/// 設定保存失敗時に status へ出す固定文言（C2-WEB）。
///
/// `save()` の詳細メッセージは console にだけ残し、status / `aria-live` / DOM
/// にはこの固定文言だけを流す。トークン・ホスト名・パスを含まず、`;` / 改行も
/// ないので既存の DOM proof 契約を壊さない。
const SETTINGS_SAVE_ERROR_NOTICE: &str =
    "設定を保存できませんでした。ブラウザのストレージ設定（容量・権限）を確認してください";

/// Fixed, redacted notice when another tab's settings save was mirrored into
/// this view. No key, value, host name or agent token is included, and no `;` /
/// newline that could corrupt the `key=value;` DOM proof grammar (HA-WEBMERGE).
const SETTINGS_SYNC_NOTICE: &str = "別のタブで設定が変更されたため、表示を同期しました";

/// Maps a settings save result to the status text shown to the user.
///
/// Kept pure so the notification contract can be tested without a browser
/// context; `None` on success means "do not touch the current status".
fn settings_save_status(result: &Result<(), String>) -> Option<&'static str> {
    match result {
        Ok(()) => None,
        Err(_) => Some(SETTINGS_SAVE_ERROR_NOTICE),
    }
}

/// Pure re-entry guard for the cross-tab settings reload task.
///
/// Storage events can arrive in bursts (another tab saving two fields in a
/// row). A reload that is already in flight must not be started again from a
/// second event, or the repeated `restore_settings()` would race with the UI's
/// own `persist_settings()` and could re-trigger a save. Kept pure for tests.
fn begin_settings_sync(in_flight: &mut bool) -> bool {
    if *in_flight {
        false
    } else {
        *in_flight = true;
        true
    }
}

/// Releases the guard installed by [`begin_settings_sync`].
fn end_settings_sync(in_flight: &mut bool) {
    *in_flight = false;
}

/// Whether the informational cross-tab notice may replace the current status.
///
/// A transient write replaces persistent notices (SECURITY.md §5), so an
/// outstanding corrupt / save-failure notice must win over a sync message.
fn should_announce_settings_sync(status: Option<&StatusMessage>) -> bool {
    !status.is_some_and(|status| status.persistent)
}

/// Fixed status text for a clean SSE end / idle timeout, gated by the same
/// persistent guard as the cross-tab sync notice.
///
/// The fallback is purely informational, so it may replace no status or a
/// transient one but must never erase an outstanding persistent corrupt /
/// save-failure notice (SECURITY.md §5). Both fallback call sites go through
/// this helper so the policy lives in exactly one place.
fn sse_fallback_status(status: Option<&StatusMessage>) -> Option<&'static str> {
    should_announce_settings_sync(status).then_some(SSE_FALLBACK_NOTICE)
}

/// The complete, deliberately minimal effect an SSE endpoint failure may have.
///
/// A stream error says the *stream endpoint* broke, not that the snapshot poll
/// is failing. The poll therefore remains the sole owner of `live_error`
/// (wa-netfail / WB-SSEERR2): this type has no field for it, so a broken stream
/// cannot blink the DOM proof's `live_error=` marker while the poll keeps
/// ticking. The normal effect is an optional transient status notice, gated by
/// [`sse_fallback_status`] so an outstanding persistent corrupt / save-failure
/// notice keeps priority (SECURITY.md §5).
///
/// The one deliberate exception is `401`: an auth rejection carries the same
/// token re-entry guidance whichever transport observed it, so `reconnect=`
/// must be raised immediately instead of waiting for the next poll tick
/// (SA-SSE401). Only that flag and the guidance status cross the boundary;
/// `live_error`, `stale`, `ticks` and the snapshot stay poll-owned.
#[derive(Debug, PartialEq, Eq)]
struct SseErrorEffect {
    transient_status: Option<&'static str>,
    reconnect_required: bool,
}

/// Computes the SSE error effect without touching the poll-owned `live_error`.
fn sse_error_effect(status: Option<&StatusMessage>, error: &FetchError) -> SseErrorEffect {
    SseErrorEffect {
        transient_status: sse_fallback_status(status),
        reconnect_required: crate::agent::requires_reconnect(error),
    }
}

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

/// Fixed value written to the DOM proof's `menu=` field (R1-I19).
///
/// Never carries a process name, command, hostname or agent token, so adding
/// this field cannot break the redaction contract or the `key=value;` grammar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuProof {
    None,
    Open,
    Select,
    CopyPid,
}

impl MenuProof {
    fn proof_value(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Open => "open",
            Self::Select => "select",
            Self::CopyPid => "copy_pid",
        }
    }
}

/// State of the row context menu.
///
/// Only the numeric process identity is kept. Any text shown in the menu is
/// either fixed UI copy or the PID itself; agent-supplied strings are never
/// stored here or mirrored into the DOM proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RowMenu {
    key: ProcessKey,
    index: usize,
}

/// How the shell gets its data.
enum DataSource {
    /// Deterministic synthetic data generated in the browser (`tm-core`).
    Demo(DemoProbe),
    /// Loopback `tm-agent` over HTTP (read-only).
    Live(Arc<AgentClient>),
}

/// Which live transport currently keeps the view fresh.
///
/// Mirrored into the DOM proof as a trailing `sse=active|fallback` field. The
/// value is fixed ASCII and can never carry a token or agent-derived text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StreamMode {
    /// The `/api/v1/stream` subscription has delivered its first snapshot and
    /// is pushing frames. Polling is suspended only from this point on.
    Active,
    /// The stream has not delivered a frame yet, ended, failed or went idle;
    /// the existing fetch poll keeps the view fresh.
    Fallback,
}

impl StreamMode {
    #[cfg(any(target_family = "wasm", test))]
    fn proof_value(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Fallback => "fallback",
        }
    }
}

/// Browser application state.
pub struct TaskManagerApp {
    dashboard: Dashboard,
    /// 絞り込み欄の編集状態（IME の未確定文字列を含む）。
    ///
    /// 確定済み文字列だけを `dashboard` に反映する。GPUI の
    /// [`EntityInputHandler`] から届く UTF-16 の編集はすべてここを通る。
    filter_input: FilterInput,
    /// 絞り込み欄が編集中か（IME の未確定文字列を含むセッション全体）。
    ///
    /// ネイティブの `TaskManagerApp::filter_editing` と同じ契約。true の間は
    /// ナビゲーションキーとシェルショートカットを `on_key_down` で解決せず、
    /// 隠し textarea の既定動作（キャレット移動・変換候補選択）へ渡す。
    /// Ctrl+F で開始し、Enter / Esc で終了する。
    filter_editing: bool,
    source: DataSource,
    /// 証跡生成時に完全一致でマスクするトークン値（URL の `#token=` 由来）。
    ///
    /// `AgentClient` の設定は再読込で変わり得るため、証跡ビルダが参照する値も
    /// ここへ保持して設定変更と同じタイミングで更新する。平文を DOM・ログへ
    /// 書き出すことはなく、置換後の `***` だけが証跡に現れる。
    proof_token: Option<String>,
    focus_handle: FocusHandle,
    /// 列見出し（7 列）のフォーカスハンドル。Tab で辿り、Enter/Space で
    /// 既存の `on_click`（`Command::SetSortKey`）を発火させる（WCAG 2.1.1）。
    /// フレームをまたいで同じハンドルを使うため `render` 内では生成しない。
    sort_header_focus: [FocusHandle; 7],
    /// 行メニューをキーボードで開いたときに最初へ移す項目のフォーカスハンドル。
    row_menu_item_focus: FocusHandle,
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
    /// 行コンテキストメニューの状態（R1-I19）。PID と行番号だけを保持する。
    row_menu: Option<RowMenu>,
    /// 直近のメニュー操作。DOM フックの `menu=` にのみ使う固定値。
    menu_proof: MenuProof,
    /// Current live transport. Demo mode always stays in `Fallback`.
    stream_mode: StreamMode,
    /// Handle for the SSE task: dropping it cancels an idle old stream when the
    /// connection settings change (the generation check inside `agent.rs` is
    /// the second line of defence for a read already in flight).
    sse_task: Option<Task<()>>,
    /// Startup poll suspension held from `start_sse_stream` until the first
    /// frame (or stream end). While it is alive, `fetch_snapshot_guarded`
    /// issues no HTTP request, so a `Connecting(Fallback)` sampler cannot
    /// double-sample the SSE probe (R2-BV01). The first frame calls
    /// `confirm_sse()` before dropping it, which also invalidates a poll that
    /// was already in flight; the error paths simply drop it to resume polling.
    sse_poll_guard: Option<PollSuspendGuard>,
    /// Bumped on every subscription attempt so a stale task that unwinds late
    /// cannot overwrite the state of its replacement.
    sse_epoch: u64,
    /// Timestamp of the most recent frame from the *current* subscription.
    /// `None` until the first frame proves the HTTP stream is delivering; that
    /// first frame is also the moment `stream_mode` becomes `Active`. The idle
    /// watchdog measures from here, not from task spawn.
    sse_last_snapshot_at: Option<Instant>,
    /// When the current subscription started. While `sse_poll_guard` is held
    /// but no snapshot frame has arrived (e.g. the agent keeps sending only
    /// `event: error`), this is the watchdog baseline so polling cannot stay
    /// suspended forever. `None` once a real frame arrives.
    sse_connect_started_at: Option<Instant>,
    /// Earliest instant at which a failed stream may be retried.
    sse_retry_at: Option<Instant>,
    /// Consecutive fetch-poll failures and the last one-shot `Retry-After`.
    /// Independent of [`Self::sse_retry_at`] so the two retry paths cannot
    /// accelerate or delay each other (WA-RETRY).
    poll_backoff: PollBackoff,
    /// F1 / ヘッダーのヘルプボタンで開くキーボード操作オーバーレイの状態。
    show_help: bool,
    /// ヘルプの「閉じる」ボタン用フォーカスハンドル。キーボードだけで
    /// オーバーレイを開いて閉じられるようにする（V2-HELP）。
    help_close_focus: FocusHandle,
    /// 直前フレームのヘルプ表示状態。表示遷移のときだけフォーカスを移動する。
    last_help_shown: bool,
    /// ヘルプを開く前に DOM でフォーカスされていた要素。閉じたときに復帰する。
    /// GPUI 側の `help_close_focus` はキー入力の宛先だけを担い、DOM の
    /// activeElement 復帰はこの 1 箇所でだけ行う（二重管理を避ける）。
    #[cfg(target_family = "wasm")]
    dom_help_previous_focus: Option<web_sys::HtmlElement>,
}

impl TaskManagerApp {
    /// Creates the shell. Live mode is selected by `?agent=` in the page URL.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);

        // 証跡のトークンマスクは URL の実際の値に完全一致で依存する。推測用の
        // hex 長判定は使わず、以下で保持した値だけを置換に渡す。
        let mut proof_token = None;
        let (dashboard, source) = match agent_config_from_url() {
            Some(config) => {
                proof_token = config.token.clone();
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
                // no explanation. Keep a short *info* notice persistent so the
                // first demo sample cannot clear it (R3-S05, V2-HELP). The full
                // connection guide lives in the F1 help overlay.
                dashboard.set_persistent_info(DEMO_MODE_STATUS_NOTICE);
                (dashboard, DataSource::Demo(probe))
            }
        };

        let mut app = Self {
            dashboard,
            filter_input: FilterInput::new(),
            filter_editing: false,
            source,
            proof_token,
            focus_handle,
            sort_header_focus: std::array::from_fn(|_| cx.focus_handle()),
            row_menu_item_focus: cx.focus_handle(),
            scroll_handle: UniformListScrollHandle::new(),
            live_total: None,
            live_error: None,
            reconnect_required: false,
            stale: false,
            ticks: 0,
            last_export: None,
            row_menu: None,
            menu_proof: MenuProof::None,
            stream_mode: StreamMode::Fallback,
            sse_task: None,
            sse_poll_guard: None,
            sse_epoch: 0,
            sse_last_snapshot_at: None,
            sse_connect_started_at: None,
            sse_retry_at: None,
            poll_backoff: PollBackoff::default(),
            show_help: false,
            help_close_focus: cx.focus_handle(),
            last_help_shown: false,
            #[cfg(target_family = "wasm")]
            dom_help_previous_focus: None,
        };
        app.restore_settings();
        app.spawn_settings_storage_sync(cx);
        app.spawn_sampler(cx);
        // Live mode prefers SSE. Any failure flips back to the existing fetch
        // poll, so a monitor is never left without updates (R3-S01).
        app.start_sse_stream(cx);
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
                // in-flight fetch) picks up the new settings (R3-S04). A real
                // change bumps the generation, aborting the old stream, and
                // `refresh_agent_config` reconnects it immediately.
                let alive = this.update(cx, |app, cx| {
                    if app.refresh_agent_config(cx) {
                        cx.notify();
                    }
                    // A stream that stopped delivering frames must not keep
                    // polling suspended forever. Check before the retry so the
                    // cooldown scheduled by the watchdog is honoured on this
                    // same cycle instead of reconnecting immediately.
                    if app.sse_watchdog_expired() {
                        app.on_sse_idle_timeout();
                        cx.notify();
                    }
                    // A terminated stream is retried after its cooldown so a
                    // transient outage does not force a page reload.
                    if app.should_retry_sse() {
                        app.start_sse_stream(cx);
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
                // While SSE is active it pushes snapshots; polling in parallel
                // would duplicate work and consume one of the agent's two
                // long-lived stream slots for no benefit.
                let stream_active =
                    match this.update(cx, |app, _| app.stream_mode == StreamMode::Active) {
                        Ok(active) => active,
                        Err(_) => break,
                    };
                if !stream_active {
                    match &live {
                        Some(client) => {
                            let result = client.fetch_snapshot_guarded().await;
                            let alive = this.update(cx, |app, cx| {
                                // `Ok(None)` means polling is suspended while
                                // SSE is connecting, or the sample was
                                // superseded by SSE confirmation. Skip it
                                // silently: no `live_error`, no backoff, no
                                // tick, no DOM proof rewrite and no notify
                                // (R2-BV01).
                                if app.ingest_guarded_poll(result) {
                                    cx.notify();
                                }
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
                }
                // Read *after* the sample so the newest applied interval (preset
                // click or persisted settings) decides the next wait. Never
                // freeze the cadence at startup. `effective_interval` also
                // applies the adaptive power plan as a floor (R1-I11).
                // Consecutive poll failures stretch this wait (and a one-shot
                // `Retry-After` raises it); a success resets it to the plain
                // configured interval. No extra request is issued here — the
                // single-flight fetch above still gates every attempt (WA-RETRY).
                let interval = match this.update(cx, |app, _| {
                    app.poll_backoff
                        .take_wait(sampler_wait(app.dashboard.effective_interval()))
                }) {
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
    fn refresh_agent_config(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(config) = agent_config_from_url() else {
            return false;
        };
        let DataSource::Live(client) = &self.source else {
            return false;
        };
        // `set_config` へ move する前に値を退避し、実際に設定が変わったときだけ
        // 証跡用のマスク対象も差し替える（以前のトークンが残ると漏えいする）。
        let proof_token = config.token.clone();
        if !client.set_config(config) {
            return false;
        }
        self.proof_token = proof_token;
        self.live_error = None;
        self.reconnect_required = false;
        // A fresh connection starts with the configured cadence instead of
        // inheriting a backoff earned by the previous credentials/endpoint.
        self.poll_backoff.record_success();
        // Transient: the next successful sample clears it, and the auth
        // guidance (if any) is replaced by the fresh connection attempt.
        self.dashboard.set_status(RELOAD_NOTICE, false);
        // `set_config` bumps the generation (the in-flight read aborts); drop
        // the task as well so an *idle* stream cancels without waiting for the
        // agent's next event, then subscribe against the new settings.
        self.start_sse_stream(cx);
        self.sync_proof_dom();
        true
    }

    /// Opens (or reopens) the SSE subscription for live mode.
    ///
    /// Demo mode is a no-op. The previous task is dropped first, which cancels
    /// the browser-side `ReadableStream` even while idle; the generation check
    /// inside `agent.rs` independently stops a read already in flight. A stale
    /// task cannot overwrite the replacement's state because it carries the
    /// epoch captured when it was spawned.
    ///
    /// Subscribing alone does **not** mean the view is fresh: `stream_mode`
    /// stays `Fallback` until the first frame arrives in the callback below.
    /// A [`PollSuspendGuard`] nevertheless holds poll *requests* during that
    /// window so the sampler cannot issue a second probe against the same
    /// endpoint; it is confirmed and dropped on the first frame, and simply
    /// dropped on every failure path (setup timeout included), so a stream
    /// that never delivers cannot suspend polling forever (R2-BV01).
    fn start_sse_stream(&mut self, cx: &mut Context<Self>) {
        let DataSource::Live(client) = &self.source else {
            return;
        };
        let client = Arc::clone(client);
        self.sse_task = None;
        self.sse_epoch = self.sse_epoch.wrapping_add(1);
        let epoch = self.sse_epoch;
        // Connection established is only *proven* by the first frame, so keep
        // polling until then and reset the watchdog baseline for this attempt.
        self.stream_mode = stream_mode_while_connecting();
        self.sse_last_snapshot_at = None;
        self.sse_connect_started_at = Some(Instant::now());
        self.sse_retry_at = None;
        // Hold poll requests from the moment the subscription starts. Storing
        // the guard on the app also drops any guard left by a previous attempt
        // (config change / retry), keeping exactly one suspension per client.
        self.sse_poll_guard = Some(client.suspend_polls_while_connecting());

        let task = cx.spawn(async move |this, cx| {
            let outcome = {
                // Reborrow `cx` for the callback; the outer context is reused
                // below once this block (and the borrowed future) has ended.
                let stream_cx = &mut *cx;
                let this_for_snapshot = this.clone();
                match client.subscribe_snapshot_stream(move |envelope: SnapshotEnvelope| {
                    let _ = this_for_snapshot.update(stream_cx, |app, cx| {
                        // A frame from a superseded subscription is data from
                        // the past. Ingesting it would overwrite the newer
                        // state after a config change, retry or idle timeout.
                        if !sse_frame_is_current(epoch, app.sse_epoch) {
                            return;
                        }
                        // First frame received: the stream is genuinely
                        // delivering, so confirm it and release the startup
                        // suspension *before* flipping the mode. Confirming
                        // bumps the poll generation, so a sample already in
                        // flight is discarded instead of being ingested after
                        // SSE becomes authoritative (R2-BV01).
                        app.confirm_sse_and_resume_polls();
                        // `handle_live_result` rewrites the DOM proof with this
                        // mode before returning (sse=active).
                        app.stream_mode = StreamMode::Active;
                        app.sse_connect_started_at = None;
                        app.sse_last_snapshot_at = Some(Instant::now());
                        // Same ingest path as polling: redaction, ticks, stale
                        // and reconnect handling stay in exactly one place.
                        app.handle_live_result(Ok(envelope.into_response()));
                        cx.notify();
                    });
                }) {
                    Ok(stream) => stream.await,
                    Err(error) => Err(error),
                }
            };
            // A released entity simply ends the task; there is nothing left
            // to update and no stream to keep alive.
            let _ = this.update(cx, |app, cx| {
                if app.sse_epoch == epoch {
                    app.on_sse_ended(outcome);
                }
                cx.notify();
            });
        });
        self.sse_task = Some(task);
        self.sync_proof_dom();
    }

    /// Returns the view to the existing fetch poll after a subscription ends.
    ///
    /// A clean EOF or a typed error both get a fixed, DOM-safe transient notice
    /// via [`Self::handle_sse_error`] / [`sse_fallback_status`]. The stream does
    /// not write the poll-owned `live_error` field (WB-SSEERR2); the sole
    /// exception is a `401`, which raises `reconnect_required` immediately so
    /// the token guidance is visible without waiting for the next poll tick
    /// (SA-SSE401). A snapshot delivered by this stream is never re-labelled
    /// stale just because its endpoint failed afterwards (X2-STALE). Either way
    /// the user keeps receiving snapshots from the poll while the retry
    /// cooldown runs, and the `sse=` field is rewritten in the same callback.
    fn on_sse_ended(&mut self, outcome: Result<(), FetchError>) {
        // A stream that never produced a frame still owns the startup guard.
        // Release it unconditionally (also on a clean EOF) so the poll resumes
        // immediately instead of being suspended until a retry replaces it.
        self.sse_poll_guard = None;
        self.stream_mode = stream_mode_after_sse_end(&outcome);
        self.sse_last_snapshot_at = None;
        self.sse_connect_started_at = None;
        self.sse_retry_at = Some(Instant::now() + SSE_RECONNECT_DELAY);
        match outcome {
            Ok(()) => {
                if let Some(text) = sse_fallback_status(self.dashboard.status()) {
                    self.dashboard.set_status(text, false);
                }
            }
            Err(error) => self.handle_sse_error(error),
        }
        self.sync_proof_dom();
    }

    /// Announces an SSE endpoint error without touching the poll-owned error.
    ///
    /// A BodyRead / Parse / HTTP failure means the *stream endpoint* broke, not
    /// that the poll is failing. Writing `live_error` here used to blink the
    /// DOM proof's `live_error=` flag for a second or two until the next healthy
    /// poll cleared it (wa-netfail). Now only a transient status notice is
    /// emitted through [`sse_error_effect`], so `live_error`, `stale`, `ticks`,
    /// the snapshot and the row menu stay exactly as the poll left them
    /// (WB-SSEERR2 / X2-STALE). The retry cooldown is set by
    /// [`Self::on_sse_ended`].
    ///
    /// Exception (SA-SSE401): when [`crate::agent::requires_reconnect`] holds,
    /// the sticky `reconnect_required` flag and the token guidance status are
    /// published here as well. Waiting for the poll to observe the same `401`
    /// would leave `reconnect=0` for a whole tick and contradict the
    /// `web/README.md` contract. `live_error` is still not written, so the DOM
    /// proof changes only `reconnect=1`, never `live_error=1`.
    fn handle_sse_error(&mut self, error: FetchError) {
        let effect = sse_error_effect(self.dashboard.status(), &error);
        if effect.reconnect_required {
            self.reconnect_required = true;
            self.dashboard
                .set_status(format!("エージェントから取得できません: {error}"), true);
            return;
        }
        if let Some(text) = effect.transient_status {
            self.dashboard.set_status(text, false);
        }
    }

    /// Whether the active stream has gone quiet long enough to force a fallback.
    ///
    /// Only a stream that actually delivered a first frame can be watched:
    /// before that the shell stays in `Fallback` and the poll keeps running, so
    /// there is nothing to recover from.
    fn sse_watchdog_expired(&self) -> bool {
        let now = Instant::now();
        let interval = self.dashboard.effective_interval();
        // A subscription that only emits `event: error` never reaches Active
        // (no snapshot frame arrives to confirm it), yet it still holds the
        // poll suspension guard. Watch that state too, otherwise polling is
        // suspended forever while the user sees a stale snapshot with no fault
        // notification (R1-FRESH). The same bound covers a stream that sends
        // headers but then goes silent before its first snapshot.
        if connecting_watchdog_expired(
            self.sse_poll_guard.is_some(),
            self.sse_last_snapshot_at,
            self.sse_connect_started_at,
            now,
            interval,
        ) {
            return true;
        }
        sse_idle_expired(self.stream_mode, self.sse_last_snapshot_at, now, interval)
    }

    /// Retracts a silent SSE stream and hands freshness back to the fetch poll.
    ///
    /// Dropping the task cancels the browser-side read, and bumping the epoch
    /// makes any frame that still arrives from it a no-op in the callback. The
    /// retry cooldown mirrors [`Self::on_sse_ended`] so a broken agent is not
    /// reconnected on every sampler tick.
    fn on_sse_idle_timeout(&mut self) {
        self.sse_task = None;
        self.sse_epoch = self.sse_epoch.wrapping_add(1);
        // Defensive release: the first frame normally already dropped the
        // guard, but no error path may leave polling suspended after a reset.
        self.sse_poll_guard = None;
        self.stream_mode = StreamMode::Fallback;
        self.sse_last_snapshot_at = None;
        self.sse_connect_started_at = None;
        self.sse_retry_at = Some(Instant::now() + SSE_RECONNECT_DELAY);
        if let Some(text) = sse_fallback_status(self.dashboard.status()) {
            self.dashboard.set_status(text, false);
        }
        self.sync_proof_dom();
    }

    /// Whether a terminated stream's retry cooldown has elapsed.
    fn should_retry_sse(&self) -> bool {
        matches!(&self.source, DataSource::Live(_))
            && self.stream_mode == StreamMode::Fallback
            && self.sse_retry_at.is_some_and(|at| Instant::now() >= at)
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
        self.revalidate_row_menu();
        self.sync_proof_dom();
    }

    /// Ingests one guarded poll result, returning whether anything changed.
    ///
    /// `Ok(None)` is `fetch_snapshot_guarded`'s "nothing to ingest" signal: the
    /// poll was suspended for a connecting SSE stream, or an in-flight sample
    /// lost to SSE confirmation. It must be a complete no-op so the connecting
    /// window cannot advance `tick=`, write `live_error=`, stretch the backoff
    /// or rewrite the DOM proof (R2-BV01). Every real sample/error keeps the
    /// existing [`Self::handle_live_result`] path unchanged.
    fn ingest_guarded_poll(
        &mut self,
        result: Result<Option<SnapshotResponse>, FetchError>,
    ) -> bool {
        let Some(result) = result.transpose() else {
            return false;
        };
        self.handle_live_result(result);
        true
    }

    /// Confirms SSE authority and releases the startup poll suspension.
    ///
    /// Called by the first-frame callback immediately before `stream_mode`
    /// becomes `Active`. `confirm_sse()` bumps the poll generation first, so a
    /// poll already in flight cannot land after SSE takes over; dropping the
    /// guard then resumes polling for the next fallback. Idempotent: later
    /// frames find no guard and do nothing (R2-BV01).
    fn confirm_sse_and_resume_polls(&mut self) {
        if let Some(guard) = self.sse_poll_guard.take() {
            guard.confirm_sse();
            drop(guard);
        }
    }

    /// Ingests a live response, or surfaces the error without falling back to demo.
    fn handle_live_result(&mut self, result: Result<crate::agent::SnapshotResponse, FetchError>) {
        // Freshness is decided against the snapshot that is *already on screen*,
        // before the new one replaces it. A failure before the first successful
        // fetch has nothing to mark stale; `live_error` covers that case.
        let had_snapshot = self.dashboard.snapshot().is_some();
        self.stale = stale_after_poll(had_snapshot, result.is_ok());
        match result {
            Ok(response) => {
                // A healthy sample from either source proves the agent is
                // answering, so the poll returns to its configured cadence.
                // This shared ingest path is also used by SSE frames; that is
                // intentional — the *error* paths stay separate (WB-SSEERR2).
                self.poll_backoff.record_success();
                self.ticks = self.ticks.saturating_add(1);
                self.live_total = Some(response.total_process_count);
                self.live_error = None;
                self.reconnect_required = false;
                self.dashboard.ingest(response.snapshot);
            }
            Err(error) => {
                // Only poll failures reach here. SSE endpoint errors go through
                // `handle_sse_error`, so they cannot change poll spacing (WA-RETRY).
                self.poll_backoff
                    .record_failure(crate::agent::fetch_retry_after(&error));
                self.record_live_error(&error);
            }
        }
        self.revalidate_row_menu();
        self.sync_proof_dom();
    }

    /// Writes the error-only fields for a **poll** failure.
    ///
    /// This is the only caller of [`live_error_fields`] that writes the DOM
    /// proof's `live_error=` / `reconnect=` markers; the SSE endpoint path goes
    /// through [`Self::handle_sse_error`] instead and never reaches here
    /// (WB-SSEERR2). The `401` / `403` decision lives in [`live_error_fields`],
    /// while freshness stays the caller's responsibility: the poll sets it in
    /// [`Self::handle_live_result`].
    fn record_live_error(&mut self, error: &FetchError) {
        let (live_error, reconnect_required, status) =
            live_error_fields(self.live_error.as_deref(), self.reconnect_required, error);
        self.live_error = live_error;
        self.reconnect_required = reconnect_required;
        self.dashboard.set_status(status, true);
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

    /// Subscribes to `localStorage` writes made by other tabs.
    ///
    /// A `storage` event is only delivered to *other* documents (the document
    /// that wrote the value never receives one), and the JS bridge below also
    /// checks `event.source === window` as an explicit source guard, so our own
    /// `persist_settings()` cannot re-enter this path. The DOM callback sends a
    /// unit message into a channel; a spawned GPUI task receives it and runs
    /// [`Self::reload_settings_from_other_tab`] with a real `Context`, because a
    /// DOM listener itself has no access to GPUI's app context.
    ///
    /// Native builds have no `localStorage`, so the whole subscription is a
    /// no-op there while the pure guard helpers remain testable.
    #[cfg(target_family = "wasm")]
    fn spawn_settings_storage_sync(&self, cx: &mut Context<Self>) {
        use futures::StreamExt;

        let (tx, mut rx) = futures::channel::mpsc::unbounded::<()>();
        cx.spawn(async move |this, cx| {
            let mut in_flight = false;
            while rx.next().await.is_some() {
                if !begin_settings_sync(&mut in_flight) {
                    continue;
                }
                // `restore_settings` only reads storage and applies view
                // commands; it never calls `save`, so there is no write-back
                // loop even if several tabs save in quick succession.
                let _ = this.update(cx, |app, cx| {
                    app.reload_settings_from_other_tab(cx);
                });
                end_settings_sync(&mut in_flight);
            }
        })
        .detach();
        install_settings_storage_bridge(tx);
    }

    #[cfg(not(target_family = "wasm"))]
    fn spawn_settings_storage_sync(&self, _cx: &mut Context<Self>) {}

    /// Re-applies a remote settings save to the Dashboard, palette and DOM
    /// proof. Uses the same load path as startup so `LAST_LOADED` (the diff
    /// baseline for the next save) advances to the freshly written value too.
    fn reload_settings_from_other_tab(&mut self, cx: &mut Context<Self>) {
        self.restore_settings();
        // A transient status write replaces persistent ones (SECURITY.md §5), so
        // never erase an outstanding corrupt / save-failure notice with the
        // purely informational sync message.
        if should_announce_settings_sync(self.dashboard.status()) {
            self.dashboard.set_status(SETTINGS_SYNC_NOTICE, false);
        }
        self.sync_proof_dom();
        cx.notify();
    }

    /// 表示設定だけを `localStorage` に保存する。
    fn persist_settings(&mut self) {
        let settings = tm_viewmodel::Settings {
            version: tm_viewmodel::SETTINGS_VERSION,
            interval_ms: self.dashboard.interval().as_millis() as u64,
            sort: self.dashboard.sort_spec(),
            details_visible: self.dashboard.details_visible(),
            tree_view: self.dashboard.tree_view(),
            theme: theme(),
        };
        let result = crate::settings::save(&settings);
        if let Err(message) = &result {
            log::warn!("設定を保存できません: {message}");
        }
        if let Some(text) = settings_save_status(&result) {
            // Persistent so the next successful sample cannot silently erase
            // the only signal that a preference change was not saved.
            self.dashboard.set_persistent_status(text);
        }
        // settings.rs 側で保存失敗を PENDING_NOTICE に載せる仕組みが追加されても
        // 既存の load 通知経路（restore_settings）を壊さずにここで拾う。
        // 通常は load_with_notice が既に排出済みなので no-op になる。
        if let Some(notice) = crate::settings::take_pending_notice() {
            self.dashboard.set_persistent_status(notice.text);
        }
    }

    /// 現在のスナップショットを redact 済みの CSV / JSON としてダウンロードする。
    ///
    /// 変換そのものは [`tm_core::export`] の純粋関数へ委譲する。ブラウザは
    /// エージェント側の環境変数（HOME / USER / hostname）を読めないため、
    /// 識別子はスナップショット自身から導出する（[`crate::redaction`]）。
    /// ライブのスナップショットは HTTP 応答時に既に redact 済みで、エクスポート
    /// 関数はさらにプライベートな clone へ識別子非依存の redact（tailnet
    /// ドメインなど）を適用する。DOM proof と同じ `proof_token` 値を渡すことで、
    /// ライブの `process.command` に `--token` の実値が残っていても平文で出力しない。
    /// ライブ由来のスナップショットは wire envelope の `total_process_count`
    /// を `live_total` に保持しているため、それを export へ渡して `?limit=`
    /// による切り詰め後もマシン全体の件数を残す。デモでは `None` となり、
    /// 従来どおり掲載行数を使う。
    fn export_snapshot(&mut self, format: ExportFormat, cx: &mut Context<Self>) {
        let total_process_count = self.live_total.map(|total| total as u64);
        let proof_token = self.proof_token.as_deref();
        let (content, rows) = match self.dashboard.snapshot() {
            Some(snapshot) => (
                export_document_with_token(snapshot, format, total_process_count, proof_token),
                snapshot.processes.len(),
            ),
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
        // 行を選択したらコンテキストメニューを閉じる。右クリックで開いた直後は
        // Select を発行しないので、開いた瞬間に閉じることはない（R1-I19）。
        if matches!(&command, Command::Select(_)) {
            self.row_menu = None;
            if self.menu_proof == MenuProof::Open {
                self.menu_proof = MenuProof::Select;
            }
            self.sync_proof_dom();
        }
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
                // C2-UXKEY (B): the sampler owns the cadence; F5 only asks for a
                // refresh and the fixed notice says so. See MANUAL_REFRESH_NOTICE
                // for why an extra one-shot fetch was not added here.
                Effect::SampleNow => {}
                Effect::Signal { .. } => self.dashboard.set_status(READONLY_SIGNAL_NOTICE, true),
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
    /// 二重挿入になるので、この関数はナビゲーション・Esc・ヘルプの開閉だけを
    /// 担当する（V2-HELP）。
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.to_lowercase();
        let modifiers = event.keystroke.modifiers;
        // 修飾なしだけをヘルプ/シェルショートカットの対象にする。Ctrl/Cmd は
        // 明示的に扱う（Ctrl+F / Ctrl+K）ため、ここで早リターンしない。
        let unmodified = !modifiers.control
            && !modifiers.platform
            && !modifiers.alt
            && !modifiers.shift
            && !modifiers.function;

        // V2-HELP のヘルプオーバーレイが最優先。F1 はトグル、Esc は表示中だけ
        // 閉じる（表示中の Esc が絞り込みを消さない）。修飾キー付きは IME /
        // ショートカット手势としてオーバーレイへ渡さない。
        if unmodified
            && let Some(route) = HelpCloseRoute::from_key(&key)
            && let Some(next_shown) = help_close_state(self.show_help, route)
        {
            self.show_help = next_shown;
            // ヘルプの開閉はコマンド扱い。ここでミラーを更新してから notify し、
            // render の表示遷移で DOM の「閉じる」へフォーカスを移せるようにする。
            self.sync_a11y_dom_now();
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.show_help {
            // ヘルプ表示中は Tab で「閉じる」へ戻し、他のキーは行操作や IME へ
            // 渡さない（ネイティブの help 優先順位と同じ）。
            if unmodified && key == "tab" {
                self.help_close_focus.focus(window, cx);
            }
            cx.stop_propagation();
            return;
        }

        // WCAG 2.1.1: 列見出しにフォーカスがある間の Enter/Space はソート操作。
        // GPUI の focusable div が keyup で既存の `on_click`（SetSortKey）を
        // 発火させるので、ここでは Space=一時停止 / Enter=改行既定へ渡さず、
        // keydown の伝播だけ止めてブラウザ既定動作を防ぐ。
        if is_sort_key_activation(&key, modifiers, self.filter_editing)
            && self
                .sort_header_focus
                .iter()
                .any(|handle| handle.is_focused(window))
        {
            cx.stop_propagation();
            return;
        }

        // Shift+F10 / ContextMenu: 選択行のメニューを開き、最初の項目へ移る。
        // メニュー項目は track_focus 済みなので Enter/Space で既定の on_click が働く。
        if is_row_menu_shortcut(&key, modifiers)
            && !self.filter_editing
            && let (Some(target), Some(index)) = (
                self.dashboard.selected_key(),
                self.dashboard.selected_visible_index(),
            )
        {
            self.open_row_menu(target, index, cx);
            self.row_menu_item_focus.focus(window, cx);
            cx.stop_propagation();
            return;
        }

        // 既存契約: 行メニューが開いていれば Esc はまずメニューを閉じる。
        if unmodified && key == "escape" && self.row_menu.is_some() {
            self.row_menu = None;
            self.menu_proof = MenuProof::None;
            self.sync_proof_dom();
            cx.notify();
            cx.stop_propagation();
            return;
        }

        // V3-KEYS: 唯一の解決点。ネイティブの `key_action` と同じ契約で、
        // 編集中はナビゲーションキーとシェルショートカットを一切解決しない。
        // 戻り値 Unhandled のときは stop_propagation しないので、隠し textarea の
        // 既定動作（キャレット移動・変換候補選択）がそのまま働く。
        // 行メニュー表示中は native の確認ダイアログと同じモーダルとして扱い、
        // F2 / Space / Ctrl+K は Swallowed にして無視する（YA-ROWMENU）。
        let editing_text = self.filter_editing || self.filter_input.is_composing();
        let decision = resolve_web_key(&key, modifiers, editing_text, self.row_menu.is_some());
        if decision == WebKeyDecision::Swallowed {
            // モーダルがキーを消費した。隠し textarea の既定動作（Space の
            // 文字挿入など）まで及ばないよう伝播も止める。
            cx.stop_propagation();
            return;
        }
        let Some(action) = decision.into_action() else {
            // Web 固有の例外: 編集中でない Enter も改行既定を止める（隠し
            // textarea に改行が入ると filter へ混入するため）。編集中は上の
            // resolver が CommitFilter を返すのでここには来ない。
            if unmodified && key == "enter" {
                cx.stop_propagation();
            }
            return;
        };

        match action {
            WebKeyAction::CancelFilter => {
                self.filter_editing = false;
                self.clear_filter(window, cx);
                cx.stop_propagation();
            }
            WebKeyAction::CommitFilter => {
                self.end_filter_editing(window, cx);
                cx.stop_propagation();
            }
            WebKeyAction::BeginFilter => {
                self.begin_filter_editing(window, cx);
                cx.stop_propagation();
            }
            WebKeyAction::ReadOnlyNotice => {
                // Ctrl+K: シグナル要求と同じ読み取り専用通知を出す（V3-KEYS）。
                self.dashboard.set_status(READONLY_SIGNAL_NOTICE, true);
                self.sync_proof_dom();
                cx.notify();
                cx.stop_propagation();
            }
            WebKeyAction::SelectRelative(delta) => {
                self.handle_command(Command::SelectRelative(delta), cx);
                cx.stop_propagation();
            }
            WebKeyAction::TogglePaused => {
                // Web サンプラーに一時停止は無い。無反応より固定通知を出す。
                self.dashboard.set_status(PAUSE_UNSUPPORTED_NOTICE, false);
                self.sync_proof_dom();
                cx.notify();
                cx.stop_propagation();
            }
            WebKeyAction::ToggleDetails => {
                self.handle_command(Command::ToggleDetails, cx);
                cx.stop_propagation();
            }
            WebKeyAction::Refresh => {
                // 判断（C2-UXKEY）: (B) を採用。サンプラーが周期を管理するので
                // データ取得は no-op のまま、要求のみであることを文言で明示する。
                // DOM proof の tick= は実サンプルだけが増やす契約を維持する。
                self.handle_command(Command::Tick, cx);
                self.dashboard.set_status(MANUAL_REFRESH_NOTICE, false);
                self.sync_proof_dom();
                cx.notify();
                cx.stop_propagation();
            }
            WebKeyAction::ShowHelp => {
                // 通常は V2 の help_overlay_key_state が先に F1 を消費する。
                // ここは修飾キー因素でオーバーレイへ届かなかった場合の保険。
                self.show_help = true;
                self.sync_a11y_dom_now();
                cx.notify();
                cx.stop_propagation();
            }
            WebKeyAction::ClearFilter => {
                if self.dashboard.filter().is_active() {
                    self.clear_filter(window, cx);
                }
                cx.stop_propagation();
            }
        }
    }

    /// 絞り込み編集を開始する（Ctrl+F）。ネイティブの
    /// `begin_filter_editing` と同じく確定文字列を入力欄へ引き継ぐ。
    fn begin_filter_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.filter_editing {
            self.filter_input.clear();
            let current = self.dashboard.filter_text().to_string();
            if !current.is_empty() {
                self.filter_input.replace_text(None, &current);
            }
            self.filter_editing = true;
        }
        self.focus_handle.focus(window, cx);
        self.notify_ime_mirror(window);
        cx.notify();
    }

    /// 絞り込み編集を終了する（Enter）。確定文字列は Dashboard に残す。
    fn end_filter_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter_editing = false;
        let outcome = self.filter_input.unmark();
        self.apply_input_outcome(outcome, window, cx);
    }

    /// IME ミラー（ブラウザ側の隠し textarea）へ編集・選択の変化を通知する。
    ///
    /// [`Self::apply_input_outcome`] と同じ経路を、Esc / プリセット適用など
    /// `filter_input` を直接クリアする経路でも使う。これを省くと GPUI 側の
    /// 表示テキストは消えても textarea の値が残り、次の入力で古い文字が
    /// 再挿入される。
    fn notify_ime_mirror(&self, window: &mut Window) {
        window.notify_text_input_changed(&self.focus_handle);
        window.notify_text_selection_changed(&self.focus_handle);
    }

    /// 絞り込み欄を空にして、一覧の絞り込みも解除する。
    fn clear_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter_input.clear();
        self.handle_command(Command::ClearFilter, cx);
        self.notify_ime_mirror(window);
    }

    /// 組み込みフィルタプリセットを適用する（R1-I06）。
    ///
    /// すべてのプリセットは `filter.text` が空なので、IME ミラーの表示テキストも
    /// 同時にクリアしないと「欄には前の検索語が残っているのに一覧はプリセット」
    /// という不一致になる。`ClearFilter` と同じ手順を踏むこと。
    fn apply_filter_preset(
        &mut self,
        id: FilterPresetId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.filter_input.clear();
        self.handle_command(Command::ApplyFilterPreset(id), cx);
        self.notify_ime_mirror(window);
    }

    /// 行メニューを開く。PID と行番号以外の情報は保持しない（R1-I19）。
    fn open_row_menu(&mut self, key: ProcessKey, index: usize, cx: &mut Context<Self>) {
        self.row_menu = Some(RowMenu { key, index });
        self.menu_proof = MenuProof::Open;
        self.sync_proof_dom();
        cx.notify();
    }

    /// 最新スナップショットにメニューの対象が居なくなったらメニューを閉じる。
    ///
    /// PID は再利用されるので数値ではなく完全な [`ProcessKey`] で照合する。
    /// 呼び出し側のサンプラが `cx.notify()` を行うため、ここでは状態のみ更新する。
    fn revalidate_row_menu(&mut self) {
        if row_menu_is_stale(self.dashboard.snapshot(), self.row_menu) {
            self.row_menu = None;
            self.menu_proof = MenuProof::None;
        }
    }

    /// F1 / ヘッダーのヘルプボタンで開くキーボード操作オーバーレイ（V2-HELP）。
    ///
    /// 絶対配置のフルスクリーン層なので既存レイアウト座標は変わらない。
    /// Esc / F1 / 「閉じる」ボタンのいずれでも閉じられ、開いた瞬間は
    /// 「閉じる」へフォーカスが移るためキーボードだけで操作を完結できる。
    fn help_overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.show_help {
            return None;
        }
        let rows: Vec<Div> = HELP_SHORTCUTS
            .iter()
            .map(|(keys, description)| {
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .bg(rgb(palette().surface_hi))
                            .text_xs()
                            .text_color(rgb(palette().text))
                            .child(*keys),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette().subtext))
                            .child(*description),
                    )
            })
            .collect();
        let live = matches!(&self.source, DataSource::Live(_));
        Some(
            div()
                .id("help-overlay")
                .role(Role::Dialog)
                .aria_label(HELP_TITLE)
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(0x000000A6))
                .occlude()
                .child(
                    v_flex()
                        .id("help-panel")
                        .w(px(560.0))
                        .p_4()
                        .gap_3()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(palette().surface))
                        .bg(rgb(palette().mantle))
                        .child(
                            h_flex()
                                .justify_between()
                                .items_center()
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .child(HELP_TITLE),
                                )
                                .child(
                                    div()
                                        .id("help-close")
                                        .role(Role::Button)
                                        .aria_label("ヘルプを閉じる")
                                        .px_3()
                                        .py_1()
                                        .cursor_pointer()
                                        .rounded_md()
                                        .bg(rgb(palette().surface_hi))
                                        .text_xs()
                                        .text_color(rgb(palette().text))
                                        .hover(|style| style.bg(rgb(palette().surface)))
                                        .on_click(cx.listener(|this, _event, window, cx| {
                                            this.show_help = false;
                                            // ミラーからダイアログを除いてから render の
                                            // 表示遷移で DOM フォーカスを復帰させる。
                                            this.sync_a11y_dom_now();
                                            this.focus_handle.focus(window, cx);
                                            cx.notify();
                                        }))
                                        .track_focus(&self.help_close_focus)
                                        .child("閉じる"),
                                ),
                        )
                        .children(rows)
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette().muted))
                                .child("ブラウザ版は読み取り専用です。プロセス操作と確認ダイアログの行はネイティブ版の操作一覧です。"),
                        )
                        .children((!live).then(|| {
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(palette().yellow))
                                        .child("実データに接続する"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette().subtext))
                                        .child(DEMO_MODE_NOTICE),
                                )
                        }))
                        .into_any_element(),
                )
                .into_any_element(),
        )
    }

    /// 行コンテキストメニュー。絶対配置のため既存レイアウト座標を変更しない。
    fn row_context_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.row_menu?;
        let key = menu.key;
        let pid = key.pid;
        Some(
            div()
                .id("row-context-menu")
                .role(Role::Menu)
                .aria_label(format!("行 {} (PID {}) のメニュー", menu.index + 1, pid))
                .absolute()
                .top_0()
                .left_0()
                .mt(px(160.0))
                .ml(px(20.0))
                .w(px(220.0))
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(rgb(palette().surface))
                .bg(rgb(palette().mantle))
                .child(
                    self.menu_item(
                        "menu-select",
                        "行を選択",
                        "行を選択".to_string(),
                        move |this, cx| this.handle_command(Command::Select(Some(menu.key)), cx),
                        cx,
                    )
                    // Shift+F10 / ContextMenu で開いた直後のフォーカス先。
                    // ここから Tab / Enter でメニュー操作を続けられる。
                    .track_focus(&self.row_menu_item_focus),
                )
                .child(self.menu_item(
                    "menu-copy-pid",
                    "PID をコピー",
                    format!("PID {pid} をコピー"),
                    move |this, cx| {
                        this.row_menu = None;
                        // 描画時点の PID ではなく、クリック時点のスナップショットで
                        // 完全な ProcessKey を再検証する（PID 再利用対策）。
                        match row_menu_copy_pid(this.dashboard.snapshot(), key) {
                            Some(live_pid) => {
                                this.menu_proof = MenuProof::CopyPid;
                                this.dashboard
                                    .set_status(row_menu_pid_status(live_pid), false);
                            }
                            None => {
                                this.menu_proof = MenuProof::None;
                                this.dashboard.set_status(STALE_ROW_MENU_NOTICE, false);
                            }
                        }
                        this.sync_proof_dom();
                        cx.notify();
                    },
                    cx,
                ))
                .child(self.menu_item(
                    "menu-close",
                    "閉じる",
                    "メニューを閉じる".to_string(),
                    move |this, cx| {
                        this.row_menu = None;
                        this.menu_proof = MenuProof::None;
                        this.sync_proof_dom();
                        cx.notify();
                    },
                    cx,
                ))
                .into_any_element(),
        )
    }

    /// One accessible menu item with a fixed label and no agent-supplied text.
    fn menu_item(
        &self,
        id: &'static str,
        label: &'static str,
        aria_label: String,
        action: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .role(Role::MenuItem)
            .aria_label(aria_label)
            .text_sm()
            .px_2()
            .py_1()
            .rounded_md()
            .cursor_pointer()
            .hover(|style| style.bg(rgb(palette().surface)))
            .on_click(cx.listener(move |this, _event, _window, cx| action(this, cx)))
            .child(label)
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
        self.notify_ime_mirror(window);
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
                        // ヘルプボタンはテーマ切り替えの左に置く。右端の座標を
                        // CDP 検証で決め打ちしているテーマボタンを動かさないため。
                        div()
                            .id("open-help-web")
                            .role(Role::Button)
                            .aria_label("ヘルプを開く（F1、Esc で閉じる）")
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
                                this.show_help = true;
                                // F1 と同じくコマンド時にミラーを更新する（毎フレームではない）。
                                this.sync_a11y_dom_now();
                                this.focus_handle.focus(window, cx);
                                cx.notify();
                            }))
                            .child("ヘルプ"),
                    )
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
                                .child(usage_value_text(cpu_total)),
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
                                .child(usage_value_text(mem_percent)),
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
            // 重要指標なので小さい kv のままにせず、CPU/メモリの数値と同じ
            // text_xl + BOLD へ昇格させる（色だけでなくサイズでも目立たせる）。
            .child(
                kv("プロセス", format::format_count(process_count))
                    .text_xl()
                    .font_weight(gpui::FontWeight::BOLD),
            );
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

    /// フィルタ行の中央に置くプリセットチップ列（R1-I06）。
    ///
    /// GPUI の要素 id は `(固定名, 添字)` でフレーム内一意にする。チップ自体は
    /// `filter_preset_chip` が描画し、ここでは active 状態の判定とクリックの
    /// `Command::ApplyFilterPreset` 発行だけを担当する。`flex_1 + min_w_0 +
    /// overflow_hidden` で余白を吸収するため、左の検索欄（340 px）と右側の
    /// ボタン群の座標は変わらず、CDP の TOOLBAR_CENTER_Y もそのまま使える。
    fn filter_preset_chips(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let active = self.dashboard.active_filter_preset();
        let chips: Vec<Stateful<Div>> = FILTER_PRESETS
            .iter()
            .enumerate()
            .map(|(index, preset)| {
                let id = preset.id;
                filter_preset_chip(("filter-preset", index), preset, active == Some(id))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _event, window, cx| {
                        this.focus_handle.focus(window, cx);
                        this.apply_filter_preset(id, window, cx);
                    }))
            })
            .collect();
        h_flex()
            .gap_1()
            .flex_1()
            .min_w_0()
            .justify_center()
            .overflow_hidden()
            .id(FILTER_PRESETS_GROUP_ID)
            .role(Role::Group)
            .aria_label("フィルタプリセット")
            .children(chips)
    }

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
            .w(px(FILTER_BOX_WIDTH))
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
                // クリックで編集モードへ入る（ネイティブの検索欄クリックと同じ）。
                // これにより Ctrl+F を押さなくても編集中ガードが働く。
                this.begin_filter_editing(window, cx);
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
            .child(self.filter_preset_chips(cx))
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
                self.sort_cell(
                    "sort-name",
                    "名前",
                    SortKey::Name,
                    &self.sort_header_focus[0],
                    cx,
                )
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
                self.sort_cell(
                    "sort-cpu",
                    "CPU",
                    SortKey::Cpu,
                    &self.sort_header_focus[1],
                    cx,
                )
                .w(px(COL_CPU))
                .flex_shrink_0(),
            )
            .child(
                self.sort_cell(
                    "sort-memory",
                    "メモリ",
                    SortKey::Memory,
                    &self.sort_header_focus[2],
                    cx,
                )
                .w(px(COL_MEMORY))
                .flex_shrink_0(),
            )
            .child(
                self.sort_cell(
                    "sort-pid",
                    "PID",
                    SortKey::Pid,
                    &self.sort_header_focus[3],
                    cx,
                )
                .w(px(COL_PID))
                .flex_shrink_0(),
            )
            .child(
                self.sort_cell(
                    "sort-user",
                    "ユーザー",
                    SortKey::User,
                    &self.sort_header_focus[4],
                    cx,
                )
                .w(px(COL_USER))
                .flex_shrink_0(),
            )
            .child(
                self.sort_cell(
                    "sort-status",
                    "状態",
                    SortKey::Status,
                    &self.sort_header_focus[5],
                    cx,
                )
                .w(px(COL_STATUS))
                .flex_shrink_0(),
            )
            .child(
                self.sort_cell(
                    "sort-threads",
                    "スレッド",
                    SortKey::Threads,
                    &self.sort_header_focus[6],
                    cx,
                )
                .w(px(COL_THREADS))
                .flex_shrink_0(),
            )
    }

    fn sort_cell(
        &self,
        id: &'static str,
        label: &'static str,
        key: SortKey,
        focus: &FocusHandle,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let sort = self.dashboard.sort_spec();
        let active = sort.key == key;
        let text = sort_header_text(label, active, sort.direction);
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
            // マウス操作は従来どおり on_click が担う。track_focus により
            // Tab で到達可能になり、Enter/Space は GPUI の組み込み活性化が
            // 同じ on_click（SetSortKey）を keyup で発火する（WCAG 2.1.1）。
            .track_focus(focus)
            .aria_keyshortcuts("Enter, Space")
            // 隠し DOM ミラーの aria-sort と同じ状態を AccessKit にも出す。
            .a11y_synthetic_children(move |builder| {
                match a11y_sort_accesskit(active, sort.direction) {
                    Some(direction) => builder.parent_node().set_sort_direction(direction),
                    None => builder.parent_node().clear_sort_direction(),
                }
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
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, _event, window, cx| {
                        this.focus_handle.focus(window, cx);
                        this.open_row_menu(key, index, cx);
                    }),
                )
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
        // 初回オンボーディングはデモモード（?agent= 未指定）のときだけ詳細
        // パネル末尾に出す。固定文言のみで既存の DOM 座標・a11y は不変。
        if matches!(&self.source, DataSource::Demo(_)) {
            content = content.child(onboarding_card());
        }
        // cgroup グルーピングは読み取り専用。色ではなくラベルと件数で示し、
        // 既存のセンサー / 詳細セクションの id と座標は変更しない（R1-I12）。
        content = content.child(
            section_title("cgroup グループ")
                .id("details-cgroup-title")
                .role(Role::Heading)
                .aria_level(2),
        );
        let groups = self.dashboard.cgroup_groups();
        if groups.is_empty() {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette().muted))
                    .child("cgroup 情報なし"),
            );
        } else {
            for (index, group) in groups.iter().enumerate() {
                content = content.child(
                    div()
                        .id(("details-cgroup-row", index))
                        .text_xs()
                        .text_color(rgb(palette().text))
                        .child(cgroup_group_text(&group.label, group.members.len())),
                );
            }
        }
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
        // 詳細パネルはこのプロセスが選択されているときだけ描画されるので、
        // Dashboard の選択プロセス時系列をそのままスパークラインへ渡せる。
        let cpu_values = self.dashboard.selected_cpu_values();

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
            .child(
                section_title("CPU 推移")
                    .id("details-cpu-history-title")
                    .role(Role::Heading)
                    .aria_level(2),
            )
            .child(process_sparkline(
                PROCESS_CPU_SPARKLINE_ID,
                process.name.clone(),
                &cpu_values,
                palette().blue,
                28.0,
            ))
            .child(
                div()
                    .id("details-cpu-history-value")
                    .text_xs()
                    .text_color(rgb(palette().muted))
                    .child(format!(
                        "最新 {} / {} サンプル",
                        process_sparkline_value_text(&cpu_values),
                        cpu_values.len()
                    )),
            )
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
        // Read-only power / sampling / cgroup summary (R1-I10..I12). It is
        // appended after the existing fields so every CDP hook and the live
        // region text stay unchanged.
        let power_summary = match snapshot {
            Some(snapshot) => format!(
                "電源 {} / {} / cgroup {} グループ",
                snapshot.power.source.label(),
                snapshot.sampling.profile.label(),
                snapshot.cgroups.len()
            ),
            None => "電源 —".to_string(),
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
                    .child(div().text_color(rgb(palette().dim)).child("|"))
                    .child(power_summary)
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

    /// Builds the exact `body[data-gpui-proof]` payload from the current state.
    ///
    /// Split out from the DOM write so native unit tests can assert on the real
    /// proof produced by `on_sse_ended` instead of hand-concatenated literals
    /// (SA-SSE401). Viewport dimensions are the only browser-only values and
    /// are supplied by the wasm caller; the byte layout is otherwise identical.
    #[cfg(any(target_family = "wasm", test))]
    fn proof_string(&self, viewport_width: u32, viewport_height: u32) -> String {
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
        // `filter` / `ime_field` はユーザー入力由来。`key=value;` の区切り
        // （';' は従来どおり ',' へ）だけでなく '=' と改行も落として、
        // フィールド境界の注入を防ぐ。
        let proof_token = self.proof_token.as_deref();
        let filter = proof_user_text(self.dashboard.filter_text(), proof_token);
        // IME 検証用: 入力欄の表示テキスト（変換中を含む）と変換中フラグ。
        // composition 中は `filter` が変わらず `ime_composing=1` になることを
        // CDP スクリプトが確認する。両値とも実際のトークンを完全一致でマスクする。
        let ime_field = proof_user_text(self.filter_input.text(), proof_token);
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
        let rows = self.dashboard.visible_count();
        // `interval=` keeps the user-configured value byte-for-byte for the
        // existing CDP hooks; the additive `sampling_ms=` reports the adaptive
        // cadence the sampler actually waits for (R1-I11).
        let interval_ms = sampler_wait(self.dashboard.interval()).as_millis() as u64;
        let effective_interval_ms =
            sampler_wait(self.dashboard.effective_interval()).as_millis() as u64;
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
        let menu = if self.row_menu.is_some() {
            MenuProof::Open
        } else {
            self.menu_proof
        };
        let proof = proof_with_menu(&proof, menu);
        let proof = proof_with_preset(&proof, self.dashboard.active_filter_preset());
        let proof = proof_with_sampling(&proof, self.dashboard.snapshot(), effective_interval_ms);
        // Last field on purpose: every pre-existing `key=value;` pair keeps its
        // exact bytes and the new marker is always the final one.
        proof_with_sse(&proof, self.stream_mode)
    }

    /// Writes the compact state summary from [`Self::proof_string`] into
    /// `body[data-gpui-proof]` so the CDP verification script can assert on it.
    ///
    /// `document.title` is intentionally *not* part of the proof: it is a fixed,
    /// human-facing label plus the data source, so no `key=value` field (and no
    /// filter text, tick count, row count, …) ever leaks into the browser tab.
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
        let proof = self.proof_string(viewport_width, viewport_height);
        let live = matches!(&self.source, DataSource::Live(_));
        // タブのタイトルは人間向けの固定文言＋データ源のみ。DOM 証跡の内部
        // フィールド（mode/tick/rows/filter/…）は `body[data-gpui-proof]` に
        // だけ書き、ここには一切露出させない。
        document.set_title(document_title(live));
        if let Some(body) = document.body() {
            let _ = body.set_attribute("data-gpui-proof", &proof);
        }
        self.sync_a11y_dom(&document);
    }

    /// Browser-side `aria-live` mirror plus a hidden structural mirror for the
    /// canvas-only UI.
    ///
    /// The canvas carries no DOM semantics, so a visually hidden
    /// `role="status" aria-live="polite"` node announces status changes while a
    /// second hidden subtree exposes the table, the help dialog and the metric
    /// states to `Accessibility.getFullAXTree`. Both are additive: the
    /// `body[data-gpui-proof]` string written just before this call is
    /// untouched, including its trailing `sse=` field. Only already-redacted
    /// display values are copied, and no `aria-hidden` attribute is ever set.
    ///
    /// Called only from ingest / command paths (through `sync_proof_dom` or
    /// `sync_a11y_dom_now`), never from `render`.
    #[cfg(target_family = "wasm")]
    fn sync_a11y_dom(&self, document: &web_sys::Document) {
        let Some(body) = document.body() else {
            return;
        };
        let Some(live) = hidden_dom_node(document, &body, A11Y_LIVE_DOM_ID) else {
            return;
        };
        if live.get_attribute("role").as_deref() != Some("status") {
            let _ = live.set_attribute("role", "status");
            let _ = live.set_attribute("aria-live", "polite");
            let _ = live.set_attribute("aria-atomic", "true");
        }
        let text = a11y_live_text(
            self.dashboard.status(),
            is_stale(matches!(&self.source, DataSource::Live(_)), self.stale),
        );
        // Avoid re-announcing an unchanged message on every sample.
        if live.text_content().as_deref() != Some(text.as_str()) {
            live.set_text_content(Some(&text));
        }

        let Some(mirror) = hidden_dom_node(document, &body, A11Y_MIRROR_DOM_ID) else {
            return;
        };
        // データ領域とヘルプ領域は分けて更新する。ヘルプ表示中にサンプルが
        // 入ってもダイアログ全体を差し替えず、「閉じる」のフォーカスを保つ。
        let Some(data) = hidden_child_node(document, &mirror, A11Y_MIRROR_DATA_DOM_ID) else {
            return;
        };
        let data_html = a11y_mirror_data_html(
            self.dashboard.snapshot(),
            self.dashboard.visible_indices(),
            self.dashboard.selected_key(),
            self.dashboard.sort_spec(),
        );
        if data.inner_html() != data_html {
            data.set_inner_html(&data_html);
        }
        let Some(help) = hidden_child_node(document, &mirror, A11Y_MIRROR_HELP_DOM_ID) else {
            return;
        };
        // ミラーの「閉じる」/ F1 / Esc を GPUI のヘルプトグルへ橋渡しする。
        // リスナは安定したコンテナに一度だけ張られ、内_html の差し替えで失われない。
        ensure_help_dom_bridge();
        let help_html = a11y_mirror_help_html(self.show_help);
        if help.inner_html() != help_html {
            help.set_inner_html(&help_html);
        }
        self.sync_filter_textarea_a11y(document);
    }

    /// A11y-only refresh for command paths that must not rewrite the DOM proof
    /// (F1 / Esc / help buttons). Native builds have no DOM.
    #[cfg(target_family = "wasm")]
    fn sync_a11y_dom_now(&self) {
        if let Some(document) = web_sys::window().and_then(|window| window.document()) {
            self.sync_a11y_dom(&document);
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn sync_a11y_dom_now(&self) {}

    /// Names the hidden IME mirror as the filter field and publishes a masked
    /// value. The plaintext (which may be an agent token pasted by the user)
    /// is never written to the DOM — only bullet characters and a length.
    #[cfg(target_family = "wasm")]
    fn sync_filter_textarea_a11y(&self, document: &web_sys::Document) {
        // gpui-pre-web の IME ミラーは id を持たない唯一の <textarea>。
        let Ok(Some(textarea)) = document.query_selector("textarea") else {
            return;
        };
        if textarea.get_attribute("aria-label").as_deref() != Some("プロセスの絞り込み") {
            let _ = textarea.set_attribute("aria-label", "プロセスの絞り込み");
        }
        let value = a11y_filter_value_text(self.filter_input.text());
        if textarea.get_attribute("aria-valuetext").as_deref() != Some(value.as_str()) {
            let _ = textarea.set_attribute("aria-valuetext", &value);
        }
    }

    /// Moves the *DOM* focus for a help transition. Called once from the same
    /// transition branch that drives GPUI's `help_close_focus`, so the previous
    /// active element is captured/restored exactly once per open/close cycle.
    #[cfg(target_family = "wasm")]
    fn sync_dom_help_focus(&mut self, change: DomHelpFocus) {
        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return;
        };
        match change {
            DomHelpFocus::Enter => {
                self.dom_help_previous_focus = document
                    .active_element()
                    .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok());
                let Some(close) = document.get_element_by_id(A11Y_HELP_CLOSE_DOM_ID) else {
                    return;
                };
                if let Ok(close) = close.dyn_into::<web_sys::HtmlElement>() {
                    // tabindex=-1: GPUI が Tab の宛先を管理したまま、AT には
                    // ダイアログの「閉じる」がフォーカスされているように見える。
                    let _ = close.focus();
                }
            }
            DomHelpFocus::Leave => {
                if let Some(previous) = self.dom_help_previous_focus.take() {
                    let _ = previous.focus();
                }
            }
            DomHelpFocus::Unchanged => {}
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn sync_dom_help_focus(&mut self, _change: DomHelpFocus) {}

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
        // ヘルプの表示遷移のときだけフォーカスを移動する。キー・ボタンのどちらで
        // 開いても同じ経路を通り、閉じたらルートへ戻って行操作を再開できる。
        // GPUI の `help_close_focus` と DOM の activeElement は同じ遷移判定
        // （`dom_help_focus`）から一度ずつ更新するので、復帰処理が二重化しない。
        let help_focus = dom_help_focus(self.last_help_shown, self.show_help);
        if help_focus != DomHelpFocus::Unchanged {
            if self.show_help {
                self.help_close_focus.focus(window, cx);
            } else {
                self.focus_handle.focus(window, cx);
            }
            self.sync_dom_help_focus(help_focus);
            self.last_help_shown = self.show_help;
        }
        let focused = self.focus_handle.is_focused(window);
        let header = self.header(cx);
        let overview = self.overview();
        let toolbar = self.toolbar(focused, cx);
        let main_area = self.main_area(cx);
        let status_bar = self.status_bar();
        let row_menu = self.row_context_menu(cx);
        let help = self.help_overlay(cx);

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
            .children(row_menu)
            .children(help)
            .child(input_registration)
    }
}

// ------------------------------------------------------------------ helpers

/// Read-only cgroup grouping row: normalised label plus member count (R1-I12).
///
/// The label is already sanitised and redacted at the probe / agent boundaries;
/// this helper only formats it and never embeds user-supplied text in an id.
fn cgroup_group_text(label: &str, members: usize) -> String {
    format!("{label} — {members} 件")
}

/// Pure keyboard contract of the help overlay (V2-HELP).
///
/// `Some(next_state)` means the key belongs to the overlay: F1 toggles it and
/// Esc closes it only while open, so a bare Esc still clears the filter when no
/// overlay is visible. Keeping this decision pure lets the headless tests verify
/// the toggle/close semantics without a GPUI `Window` or `Context`.
fn help_overlay_key_state(show_help: bool, key: &str) -> Option<bool> {
    match key.to_ascii_lowercase().as_str() {
        "f1" => Some(!show_help),
        "escape" if show_help => Some(false),
        _ => None,
    }
}

/// ヘルプを閉じる 3 経路（F1 / Escape / DOM「閉じる」）の純粋な表現。
///
/// GPUI 側のキー経路と、ミラーの DOM ブリッジ経路で同じ遷移関数を共有し、
/// 「閉じる」ボタンがトグル扱いになって即再オープンしないことを保証する。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HelpCloseRoute {
    /// GPUI のキーボードハンドラが受け取る F1（トグル）。
    F1,
    /// GPUI のキーボードハンドラが受け取る Escape（表示中だけ閉じる）。
    Escape,
    /// DOM ミラーの「閉じる」、およびミラーで捕捉したキーからの転送。
    DomButton,
}

impl HelpCloseRoute {
    fn from_key(key: &str) -> Option<Self> {
        match key.to_ascii_lowercase().as_str() {
            "f1" => Some(Self::F1),
            "escape" => Some(Self::Escape),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::F1 => "f1",
            Self::Escape | Self::DomButton => "escape",
        }
    }
}

/// 3 経路すべての表示状態遷移。`None` は「ヘルプの担当ではない」ことを表す。
///
/// DOM 経路はクローズ専用（トグルではない）なので、既に閉じているときに
/// 二重で届いても `Some(true)` を返さず、オーバーレイが即再オープンしない。
fn help_close_state(show_help: bool, route: HelpCloseRoute) -> Option<bool> {
    match route {
        HelpCloseRoute::DomButton => show_help.then_some(false),
        HelpCloseRoute::F1 | HelpCloseRoute::Escape => {
            help_overlay_key_state(show_help, route.key())
        }
    }
}

/// 1 キー入力の解決結果（V3-KEYS）。
///
/// ネイティブの [`app/src/app.rs::KeyAction`](../../app/src/app.rs) と同じ契約を
/// Web 側にも持たせる。純粋関数にすることで、GPUI の Window / Context を
/// 作らずに「編集中はナビゲーションが発火しない」ことを単体テストできる。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WebKeyAction {
    /// 編集中の Esc: 絞り込みを解除して編集モードを抜ける。
    CancelFilter,
    /// 編集中の Enter: 確定して編集モードを抜ける（IME 確定はブラウザが行う）。
    CommitFilter,
    /// Ctrl/Cmd+F: 絞り込み入力へフォーカスする。
    BeginFilter,
    /// Ctrl/Cmd+K: 読み取り専用通知（`Effect::Signal` と同じ文言）。
    ReadOnlyNotice,
    /// 矢印 / Page / Home / End: 選択行をこの行数だけ動かす。
    SelectRelative(i32),
    /// Space: 一時停止相当（Web は未対応なので通知で代替）。
    TogglePaused,
    /// F2: 詳細パネルの表示切替。
    ToggleDetails,
    /// F5: 今すぐ更新（Tick 相当）。
    Refresh,
    /// F1: ヘルプを開く（実際のトグルは V2 の `help_overlay_key_state`）。
    ShowHelp,
    /// 編集中でない Esc: アクティブな絞り込みを解除する。
    ClearFilter,
}

/// 変更文字列を GPUI 状態へ触らずにキーへ対応する操作を解決する（V3-KEYS）。
///
/// `editing_text` は `TaskManagerApp::filter_editing` と IME の未確定状態を
/// OR した値。true の間は文字編集キー（Esc / Enter）だけを返し、矢印・
/// Page・Home/End・Space・ショートカットは `_ => None` に落とす。`on_key_down`
/// は None のとき `stop_propagation` しないので、隠し textarea の既定動作
/// （キャレット移動 / 変換候補選択）がそのまま働く。これはネイティブの
/// `key_action`（app/src/app.rs:2823-2835）と同一の契約である。
fn web_key_action(key: &str, modifiers: Modifiers, editing_text: bool) -> Option<WebKeyAction> {
    if editing_text {
        // 修飾キー付きの編集操作（単語削除など）はプラットフォーム側へ委譲。
        if modifiers.control || modifiers.platform || modifiers.alt || modifiers.function {
            return None;
        }
        return match key {
            "escape" => Some(WebKeyAction::CancelFilter),
            "enter" => Some(WebKeyAction::CommitFilter),
            _ => None,
        };
    }

    // Shift / Alt / Fn は IME 候補選択やテキスト選択の手势なのでショートカットに
    // 変換しない。Ctrl と Cmd は同じ意味（R3-A03）。
    if modifiers.alt || modifiers.shift || modifiers.function {
        return None;
    }
    let primary = modifiers.control || modifiers.platform;
    match (primary, key) {
        (true, "f") => Some(WebKeyAction::BeginFilter),
        (true, "k") => Some(WebKeyAction::ReadOnlyNotice),
        (false, "up") => Some(WebKeyAction::SelectRelative(-1)),
        (false, "down") => Some(WebKeyAction::SelectRelative(1)),
        (false, "pageup") => Some(WebKeyAction::SelectRelative(-10)),
        (false, "pagedown") => Some(WebKeyAction::SelectRelative(10)),
        (false, "home") => Some(WebKeyAction::SelectRelative(-1_000_000)),
        (false, "end") => Some(WebKeyAction::SelectRelative(1_000_000)),
        (false, "space") => Some(WebKeyAction::TogglePaused),
        (false, "f2") => Some(WebKeyAction::ToggleDetails),
        (false, "f5") => Some(WebKeyAction::Refresh),
        (false, "f1") => Some(WebKeyAction::ShowHelp),
        (false, "escape") => Some(WebKeyAction::ClearFilter),
        _ => None,
    }
}

/// キー解決の結果（YA-ROWMENU）。
///
/// 行メニューを native の確認ダイアログと同じモーダルとみなし、メニュー表示中は
/// 詳細トグル / 一時停止通知 / 読み取り専用通知を発火させない。Esc / Enter /
/// 矢印（メニュー項目の移動・実行）やヘルプは従来どおり通す。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WebKeyDecision {
    /// `action` を実行する。
    Run(WebKeyAction),
    /// 行メニュー（モーダル）がキーを消費した。伝播を止めて無視する。
    Swallowed,
    /// バインドなし。隠し textarea / ブラウザの既定動作へ委譲する。
    Unhandled,
}

impl WebKeyDecision {
    fn into_action(self) -> Option<WebKeyAction> {
        match self {
            Self::Run(action) => Some(action),
            _ => None,
        }
    }
}

/// [`web_key_action`] に行メニューのモーダル抑止を重ねた純粋関数。
///
/// `on_key_down` とテストが同じ判定を使うことで、メニュー表示中の F2 / Space /
/// Ctrl+K が詳細・status を書き換えないことを固定できる（YA-ROWMENU）。
/// `row_menu_open = false` のときは従来の resolver と完全に同じ結果になる。
fn resolve_web_key(
    key: &str,
    modifiers: Modifiers,
    editing_text: bool,
    row_menu_open: bool,
) -> WebKeyDecision {
    let Some(action) = web_key_action(key, modifiers, editing_text) else {
        return WebKeyDecision::Unhandled;
    };
    if row_menu_open
        && matches!(
            action,
            WebKeyAction::ToggleDetails | WebKeyAction::TogglePaused | WebKeyAction::ReadOnlyNotice
        )
    {
        return WebKeyDecision::Swallowed;
    }
    WebKeyDecision::Run(action)
}

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

/// Japanese direction name for sort headers (the ↑/↓ glyph alone is not text).
fn a11y_sort_direction(direction: SortDirection) -> &'static str {
    match direction {
        SortDirection::Ascending => "昇順",
        SortDirection::Descending => "降順",
    }
}

/// AccessKit `aria-sort` counterpart of the DOM mirror's `aria-sort` attribute.
///
/// Only the active column publishes a direction; the others clear the property
/// so assistive tech does not announce a stale sort state on an inactive header.
fn a11y_sort_accesskit(active: bool, direction: SortDirection) -> Option<AccessKitSortDirection> {
    if !active {
        return None;
    }
    Some(match direction {
        SortDirection::Ascending => AccessKitSortDirection::Ascending,
        SortDirection::Descending => AccessKitSortDirection::Descending,
    })
}

/// Enter/Space on a *focused* column header is reserved for sorting.
///
/// GPUI's focusable `Div` turns Enter/Space key-up into the existing `on_click`
/// (`Command::SetSortKey`). The shell-level `on_key_down` must therefore not
/// interpret the same keystroke as Space=pause or Enter=newline (WCAG 2.1.1).
fn is_sort_key_activation(key: &str, modifiers: Modifiers, editing: bool) -> bool {
    !editing && !modifiers.modified() && matches!(key, "enter" | "space")
}

/// Shift+F10 or the ContextMenu (a.k.a. Menu) key opens the selected row's menu.
///
/// Web maps the DOM `ContextMenu` key to `"contextmenu"`; native platforms may
/// report the same physical key as `"menu"`, so both spellings are accepted.
fn is_row_menu_shortcut(key: &str, modifiers: Modifiers) -> bool {
    if modifiers.control || modifiers.platform || modifiers.alt || modifiers.function {
        return false;
    }
    if key == "f10" {
        return modifiers.shift;
    }
    !modifiers.shift && matches!(key, "contextmenu" | "menu")
}

/// Visible text of a sortable column header.
///
/// Kept in step with the native shell: the active header carries the shared
/// `SortDirection::arrow()` glyph, so ascending and descending never render
/// identically (the old ▲/▼ pair was tofu in IBM Plex).
fn sort_header_text(label: &str, active: bool, direction: SortDirection) -> String {
    if active {
        format!("{label} {}", direction.arrow())
    } else {
        label.to_string()
    }
}

// ------------------------------------------------ hidden accessibility mirror
//
// The canvas has no DOM, so the browser's AX tree cannot see the table, the
// help dialog or the colour-coded states. These pure builders produce the
// hidden subtree consumed by `sync_a11y_dom`. They stay string-based so the
// structure can be asserted in unit tests without a browser, and every dynamic
// value goes through `escape_html` before `set_inner_html`.

/// DOM focus decision for one help visibility transition.
///
/// The GPUI `help_close_focus` and the DOM `activeElement` are both driven from
/// this single value in `render`, so there is only one "previous focus" cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DomHelpFocus {
    /// No visibility change; leave browser focus alone.
    Unchanged,
    /// Remember the current active element and focus the mirror's close button.
    Enter,
    /// Restore the element remembered on `Enter`.
    Leave,
}

/// Pure transition table used by both the GPUI and the DOM focus paths.
fn dom_help_focus(last_shown: bool, shown: bool) -> DomHelpFocus {
    match (last_shown, shown) {
        (false, true) => DomHelpFocus::Enter,
        (true, false) => DomHelpFocus::Leave,
        _ => DomHelpFocus::Unchanged,
    }
}

/// Columns of the mirrored grid, in canvas header order.
/// `None` is the non-sortable コマンド column (it still needs a columnheader).
const A11Y_MIRROR_COLUMNS: &[(&str, Option<SortKey>)] = &[
    ("名前", Some(SortKey::Name)),
    ("コマンド", None),
    ("CPU", Some(SortKey::Cpu)),
    ("メモリ", Some(SortKey::Memory)),
    ("PID", Some(SortKey::Pid)),
    ("ユーザー", Some(SortKey::User)),
    ("状態", Some(SortKey::Status)),
    ("スレッド", Some(SortKey::Threads)),
];

/// Escapes dynamic text before it is written through `set_inner_html`.
///
/// Process names and sensor labels come from the (already redacted) snapshot;
/// escaping keeps markup in those values from becoming DOM nodes.
fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

/// ARIA value for the active column's `aria-sort`.
fn a11y_sort_aria(direction: SortDirection) -> &'static str {
    match direction {
        SortDirection::Ascending => "ascending",
        SortDirection::Descending => "descending",
    }
}

/// Header row of the mirrored grid.
///
/// Only the active sortable column carries `aria-sort`; its accessible name and
/// visible text repeat the direction so the state is not glyph-only.
/// `columnheader` is valid inside a `role="grid"`, so it is kept as-is.
fn a11y_mirror_header_html(sort: SortSpec) -> String {
    let mut html = String::from("<div role=\"row\">");
    for (label, key) in A11Y_MIRROR_COLUMNS {
        let (sort_attribute, aria_label, text) = match key {
            Some(key) if *key == sort.key => (
                format!(" aria-sort=\"{}\"", a11y_sort_aria(sort.direction)),
                format!(
                    "{label} 列で並び替え、現在 {}",
                    a11y_sort_direction(sort.direction)
                ),
                sort_header_text(label, true, sort.direction),
            ),
            Some(_) => (
                String::new(),
                format!("{label} 列で並び替え"),
                sort_header_text(label, false, sort.direction),
            ),
            None => (String::new(), (*label).to_string(), (*label).to_string()),
        };
        html.push_str(&format!(
            "<div role=\"columnheader\"{sort_attribute} aria-label=\"{}\">{text}</div>",
            escape_html(&aria_label),
            text = escape_html(&text),
        ));
    }
    html.push_str("</div>");
    html
}

/// One mirrored process row: selection state plus a text name that repeats
/// PID / name / CPU / memory and the colour-coded severity word.
///
/// The enclosing `role="grid"` is deliberate: per ARIA, `aria-selected` is only
/// exposed on rows inside `grid`/`treegrid`, so a `role="table"` mirror makes
/// Chrome drop the selected state from the AX tree. With `grid` + `gridcell`
/// (header `columnheader` remains valid) the selected row publishes a
/// `selected=true` AX property. `aria-current="true"` duplicates the state for
/// assistive tech that does not surface `selected`.
fn a11y_mirror_row_html(
    name: &str,
    status: ProcessStatus,
    cpu: f32,
    memory_bytes: u64,
    pid: u32,
    selected: bool,
    row_index: usize,
) -> String {
    let label = a11y_row_label(name, status, cpu, memory_bytes, pid, selected);
    let (selected, current_attribute) = if selected {
        ("true", " aria-current=\"true\"")
    } else {
        ("false", "")
    };
    format!(
        "<div role=\"row\" aria-rowindex=\"{row_index}\" aria-selected=\"{selected}\"{current_attribute} \
         aria-label=\"{}\"><div role=\"gridcell\">{}</div>\
         <div role=\"gridcell\">{}</div><div role=\"gridcell\">{}</div></div>",
        escape_html(&label),
        escape_html(name),
        escape_html(&usage_value_text(cpu)),
        escape_html(&format::format_bytes(memory_bytes)),
    )
}

/// Body rowgroup of the mirrored grid (only the currently visible rows).
fn a11y_mirror_rows_html(
    snapshot: Option<&SystemSnapshot>,
    visible_indices: &[usize],
    selected: Option<ProcessKey>,
) -> String {
    let mut html = String::from("<div role=\"rowgroup\">");
    if let Some(snapshot) = snapshot {
        for (index, process_index) in visible_indices.iter().enumerate() {
            let Some(process) = snapshot.processes.get(*process_index) else {
                continue;
            };
            html.push_str(&a11y_mirror_row_html(
                &process.name,
                process.status,
                process.cpu_percent,
                process.memory_bytes,
                process.key.pid,
                selected == Some(process.key),
                index + 1,
            ));
        }
    }
    html.push_str("</div>");
    html
}

/// Colour-coded usage and temperature states as hidden text.
///
/// Reuses the exact values drawn on the canvas (`usage_value_text` /
/// `temperature_value_text`), so 高/中/低 and 正常/注意/危険/不明 are present
/// in the AX tree without relying on colour perception.
fn a11y_mirror_metrics_html(snapshot: Option<&SystemSnapshot>) -> String {
    let Some(snapshot) = snapshot else {
        return String::new();
    };
    let mut html = String::from("<div role=\"group\" aria-label=\"使用率と温度\">");
    let mut push_metric = |label: &str, value: String| {
        html.push_str(&format!(
            "<div>{}: {}</div>",
            escape_html(label),
            escape_html(&value)
        ));
    };
    push_metric("CPU 使用率", usage_value_text(snapshot.cpu.total_percent));
    push_metric(
        "メモリ使用率",
        usage_value_text(snapshot.memory.used_percent()),
    );
    push_metric(
        "スワップ使用率",
        usage_value_text(snapshot.memory.swap_used_percent()),
    );
    for disk in snapshot.disks.iter().take(2) {
        push_metric(
            &format!("ディスク {} 使用率", disk.mount_point),
            usage_value_text(disk.used_percent()),
        );
    }
    for component in snapshot.components.iter().take(3) {
        let label = if component.label.is_empty() {
            "センサー"
        } else {
            &component.label
        };
        push_metric(
            &format!("{label} 温度"),
            temperature_value_text(
                component.temperature_c,
                component.max_c,
                component.critical_c,
            ),
        );
    }
    html.push_str("</div>");
    html
}

/// Help dialog mirror. Empty when the overlay is closed, so the AX tree has no
/// stale modal. The close button is `tabindex=-1`: GPUI keeps owning Tab, while
/// assistive tech sees a focusable button that `sync_dom_help_focus` targets.
fn a11y_mirror_help_html(show_help: bool) -> String {
    if !show_help {
        return String::new();
    }
    let mut html = format!(
        "<div role=\"dialog\" aria-modal=\"true\" aria-label=\"{title}\">\
         <div role=\"heading\" aria-level=\"1\">{title}</div>",
        title = escape_html(HELP_TITLE),
    );
    for (keys, description) in HELP_SHORTCUTS {
        html.push_str(&format!(
            "<div>{}: {}</div>",
            escape_html(keys),
            escape_html(description),
        ));
    }
    html.push_str(&format!(
        "<div id=\"{}\" role=\"button\" tabindex=\"-1\" aria-label=\"ヘルプを閉じる\">閉じる</div>",
        A11Y_HELP_CLOSE_DOM_ID,
    ));
    html.push_str("</div>");
    html
}

/// Data part of the hidden subtree: the grid (header + visible rows) and the
/// colour-coded metric states. Refreshed on ingest / command, never per frame.
///
/// `role="grid"` (not `table`) is required so Chrome exposes the selected row's
/// `aria-selected=true` as a `selected=true` property in the AX tree.
fn a11y_mirror_data_html(
    snapshot: Option<&SystemSnapshot>,
    visible_indices: &[usize],
    selected: Option<ProcessKey>,
    sort: SortSpec,
) -> String {
    let mut html = String::from("<div role=\"grid\" aria-label=\"プロセス一覧\">");
    html.push_str("<div role=\"rowgroup\">");
    html.push_str(&a11y_mirror_header_html(sort));
    html.push_str("</div>");
    html.push_str(&a11y_mirror_rows_html(snapshot, visible_indices, selected));
    html.push_str("</div>");
    html.push_str(&a11y_mirror_metrics_html(snapshot));
    html
}

/// Whole hidden subtree (data + optional help modal), kept as one pure value
/// so the AX structure can be asserted without a browser.
fn a11y_mirror_html(
    snapshot: Option<&SystemSnapshot>,
    visible_indices: &[usize],
    selected: Option<ProcessKey>,
    sort: SortSpec,
    show_help: bool,
) -> String {
    format!(
        "{}{}",
        a11y_mirror_data_html(snapshot, visible_indices, selected, sort),
        a11y_mirror_help_html(show_help)
    )
}

/// Masked value published on the hidden IME textarea.
///
/// Never returns the plaintext (an agent token may have been pasted into the
/// filter): only bullet characters and a character count reach the DOM.
fn a11y_filter_value_text(text: &str) -> String {
    let count = text.chars().count();
    if count == 0 {
        "未入力".to_string()
    } else {
        let bullets = "•".repeat(count.min(64));
        format!("{bullets}（{count} 文字、伏せ字）")
    }
}

/// Returns an existing hidden node or creates one under `body`.
#[cfg(target_family = "wasm")]
fn hidden_dom_node(
    document: &web_sys::Document,
    body: &web_sys::HtmlElement,
    id: &str,
) -> Option<web_sys::Element> {
    if let Some(element) = document.get_element_by_id(id) {
        return Some(element);
    }
    let element = document.create_element("div").ok()?;
    element.set_attribute("id", id).ok()?;
    element.set_attribute("style", A11Y_HIDDEN_STYLE).ok()?;
    body.append_child(&element).ok()?;
    Some(element)
}

/// Returns an existing child region of the mirror or creates it in place.
#[cfg(target_family = "wasm")]
fn hidden_child_node(
    document: &web_sys::Document,
    parent: &web_sys::Element,
    id: &str,
) -> Option<web_sys::Element> {
    if let Some(element) = document.get_element_by_id(id) {
        return Some(element);
    }
    let element = document.create_element("div").ok()?;
    element.set_attribute("id", id).ok()?;
    parent.append_child(&element).ok()?;
    Some(element)
}

/// Installs the DOM → GPUI help bridge once, on the *stable* help container.
///
/// `gpui-pre-web` attaches its `keydown` listener to the hidden IME textarea, so
/// once DOM focus moves to the mirrored close button, F1 / Escape no longer
/// reach GPUI and the overlay cannot be dismissed. The bridge listens on the
/// help container instead (its `inner_html` is rewritten on open/close but the
/// container itself is never replaced) and forwards an `Escape` keystroke to
/// the textarea. GPUI's existing `on_key_down` → `help_close_state` then closes
/// the overlay exactly once; `stopPropagation` prevents the original DOM event
/// from being handled twice. `Escape` rather than `F1` is forwarded so a stale
/// / duplicated DOM event can never toggle the overlay straight back open.
///
/// Inline JS is used because `web/Cargo.toml` (out of scope here) does not
/// enable the `KeyboardEventInit` web-sys feature needed for a synthetic key.
#[cfg(target_family = "wasm")]
#[wasm_bindgen(inline_js = r#"
export function tmInstallHelpBridge(helpId, closeId, marker) {
  const root = document.getElementById(helpId);
  if (!root || root.getAttribute(marker)) return;
  root.setAttribute(marker, '1');
  const forward = () => {
    const field = document.querySelector('textarea');
    if (!field) return;
    field.dispatchEvent(new KeyboardEvent('keydown', {
      key: 'Escape', code: 'Escape', bubbles: true, cancelable: true,
    }));
  };
  root.addEventListener('click', (event) => {
    const target = event.target;
    if (!target || target.id !== closeId) return;
    event.stopPropagation();
    event.preventDefault();
    forward();
  });
  root.addEventListener('keydown', (event) => {
    if (event.key !== 'F1' && event.key !== 'Escape') return;
    event.stopPropagation();
    event.preventDefault();
    forward();
  });
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = "tmInstallHelpBridge")]
    fn tm_install_help_bridge(help_id: &str, close_id: &str, marker: &str);
}

#[cfg(target_family = "wasm")]
fn ensure_help_dom_bridge() {
    tm_install_help_bridge(
        A11Y_MIRROR_HELP_DOM_ID,
        A11Y_HELP_CLOSE_DOM_ID,
        "data-tm-help-bridge",
    );
}

/// Custom event name used to forward a filtered `storage` event into Rust.
const SETTINGS_STORAGE_EVENT: &str = "tm-settings-storage-changed";

/// Installs the `window` → Rust storage bridge once per page.
///
/// `web-sys` does not enable the `StorageEvent` feature in `web/Cargo.toml`, so
/// the key/source filtering is done in a small inline JS shim (the same pattern
/// already used for the help bridge). The shim forwards only a *signal* - no
/// key, value or storage area ever crosses into Rust. Rust listens for that
/// custom event with a generic `web_sys::Event` closure, which stays alive for
/// the page lifetime because a DOM listener has no RAII owner here.
///
/// Source handling: per the storage spec a write never fires an event in the
/// writing document, and `event.source === window` is an explicit second guard
/// against an echoed/duplicated own save. A `clear()` from another tab has a
/// null key and is treated as a change to our key as well.
#[cfg(target_family = "wasm")]
#[wasm_bindgen(inline_js = r#"
let installed = false;
export function tmInstallSettingsStorageBridge(storageKey, eventName) {
  if (installed) return;
  installed = true;
  window.addEventListener('storage', (event) => {
    if (event.key !== storageKey && event.key !== null) return;
    if (event.storageArea !== window.localStorage) return;
    if (event.source === window) return;
    window.dispatchEvent(new CustomEvent(eventName));
  });
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = "tmInstallSettingsStorageBridge")]
    fn tm_install_settings_storage_bridge(storage_key: &str, event_name: &str);
}

#[cfg(target_family = "wasm")]
fn install_settings_storage_bridge(tx: futures::channel::mpsc::UnboundedSender<()>) {
    use wasm_bindgen::closure::Closure;

    let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
        let _ = tx.unbounded_send(());
    });
    let Some(window) = web_sys::window() else {
        return;
    };
    if window
        .add_event_listener_with_callback(SETTINGS_STORAGE_EVENT, closure.as_ref().unchecked_ref())
        .is_ok()
    {
        // The listener is installed for the lifetime of the page; dropping the
        // closure here would invalidate the JS callback pointer.
        std::mem::forget(closure);
        tm_install_settings_storage_bridge(STORAGE_KEY, SETTINGS_STORAGE_EVENT);
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

/// Wait before the next poll attempt after `failures` consecutive failures.
///
/// Healthy polling keeps the configured `interval` untouched (including a
/// 250 ms floor). Failures grow the wait exponentially — 1 s, 2 s, 4 s, 8 s,
/// 16 s — capped at [`POLL_BACKOFF_MAX`]. A numeric `Retry-After` from a
/// `429` / `5xx` response raises the floor for **one** attempt and is bounded
/// by the same cap, so a broken agent cannot freeze the view. The result is
/// never shorter than `interval`: backoff cannot increase the request rate.
/// Kept pure so the whole contract is testable without a browser (WA-RETRY).
fn poll_retry_wait(interval: Duration, failures: u32, retry_after: Option<Duration>) -> Duration {
    let ceiling = POLL_BACKOFF_MAX.max(interval);
    let exponential = if failures == 0 {
        Duration::ZERO
    } else {
        POLL_BACKOFF_BASE.saturating_mul(2u32.saturating_pow(failures - 1))
    };
    let mut wait = interval.max(exponential);
    if let Some(hint) = retry_after {
        wait = wait.max(hint);
    }
    wait.min(ceiling)
}

/// Consecutive-failure state of the fetch poll only (WA-RETRY).
///
/// Deliberately independent of the SSE `sse_retry_at` cooldown: a stream
/// endpoint failure must not change the poll cadence and vice versa. Kept as a
/// small value type so reset-on-success and the one-shot `Retry-After` hint
/// are unit-testable without a GPUI context.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PollBackoff {
    consecutive_failures: u32,
    retry_after: Option<Duration>,
}

impl PollBackoff {
    /// A successful sample restores the configured cadence immediately.
    fn record_success(&mut self) {
        *self = Self::default();
    }

    /// A failed sample raises the backoff and stores at most one server hint.
    fn record_failure(&mut self, retry_after: Option<Duration>) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.retry_after = retry_after;
    }

    /// Resolves this cycle's wait and consumes the one-shot `Retry-After`.
    fn take_wait(&mut self, interval: Duration) -> Duration {
        let wait = poll_retry_wait(interval, self.consecutive_failures, self.retry_after);
        self.retry_after = None;
        wait
    }
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

/// Sanitises user text for a `key=value;` DOM proof field.
///
/// The legacy `;` → `,` replacement is preserved byte-for-byte. `=`, CR and LF
/// are dropped as well: without this, a filter or IME value such as `a=1;\r\n`
/// could forge a field boundary and break the parser used by the CDP checks.
#[cfg(any(target_family = "wasm", test))]
fn proof_field_text(text: &str) -> String {
    text.chars()
        .filter(|ch| !matches!(ch, '=' | '\r' | '\n'))
        .map(|ch| if ch == ';' { ',' } else { ch })
        .collect()
}

/// `filter` / `ime_field` 用の証跡テキスト。
///
/// CDP 検証が値を参照する契約があるため入力を完全に消すことはできないが、
/// 「機微情報を含む可能性のある値は証跡へ残さない」方針をここで守る。
/// `token` には URL から読み込んだ **実際の** `AgentConfig.token` を渡し、
/// 出現するたびに完全一致で `***` へ置き換える。短いトークンや hex 以外の
/// トークン（`s3cr3t-123` 等）も漏らさず、逆にトークンでない hex 文字列を
/// 推測で消すこともない（長さベースのヒューリスティックは廃止）。
///
/// 置換はサニタイズより前に行うので、`=`, `;`, 改行を含むトークンも漏れない。
/// 既存の `key=value;` キー、区切り、末尾 `sse=` 契約はバイト単位で不変。
#[cfg(any(target_family = "wasm", test))]
fn proof_user_text(text: &str, token: Option<&str>) -> String {
    let redacted = match token {
        Some(token) if !token.is_empty() => text.replace(token, "***"),
        _ => text.to_string(),
    };
    proof_field_text(&redacted)
}

/// Builds the human-facing browser-tab title from the data source alone.
///
/// The DOM proof fields live exclusively in `body[data-gpui-proof]`; the title
/// is deliberately limited to a fixed label plus a demo/live marker. It can
/// therefore never carry a `key=value` proof field, the filter input text, a
/// tick or row count, a PID, an interval, or the agent token.
#[cfg(any(target_family = "wasm", test))]
fn document_title(live: bool) -> &'static str {
    if live {
        "GPUI タスクマネージャー (Web) — ライブ"
    } else {
        "GPUI タスクマネージャー (Web) — デモ"
    }
}

/// The re-entry flag is sticky until success or an explicit config change: a
/// transient follow-up failure must not hide the auth guidance (R49 audit).
fn next_reconnect_required(current: bool, error: &FetchError) -> bool {
    current || crate::agent::requires_reconnect(error)
}

/// Error-only fields written when a live transport fails, plus the status text.
///
/// Only `401` (FetchError::Auth) carries the token re-entry guidance; `403`
/// (Forbidden) is an origin problem and must not raise the DOM proof's
/// `reconnect=` flag (C2-WEB). The flag stays sticky across transient follow-up
/// failures and is cleared only on success or an actual config change.
///
/// There is deliberately no `stale` output: a failed SSE endpoint must not age
/// the snapshot that the same stream just delivered (X2-STALE). Kept pure so
/// that contract is testable without a GPUI context.
fn live_error_fields(
    current_live_error: Option<&str>,
    reconnect_required: bool,
    error: &FetchError,
) -> (Option<String>, bool, String) {
    let newly_required = crate::agent::requires_reconnect(error);
    let reconnect_required = next_reconnect_required(reconnect_required, error);
    let text = error.to_string();
    let live_error = if !newly_required || current_live_error.is_none() {
        Some(text.clone())
    } else {
        current_live_error.map(str::to_string)
    };
    (
        live_error,
        reconnect_required,
        format!("エージェントから取得できません: {text}"),
    )
}

/// Appends the row-context-menu state to the DOM proof string.
///
/// Additive only: the value is one of four fixed identifiers (never a process
/// name, command or token), so every pre-existing `key=value;` pair stays
/// byte-for-byte identical and the headless-Chrome parser keeps working.
#[cfg(any(target_family = "wasm", test))]
fn proof_with_menu(proof: &str, menu: MenuProof) -> String {
    format!("{proof}menu={};", menu.proof_value())
}

/// Fixed ASCII identifier for a filter preset in the DOM proof.
///
/// The Japanese chip label is intentionally *not* mirrored: a fixed identifier
/// keeps the `key=value;` grammar stable and can never carry user text, a
/// hostname or the agent token. Match is exhaustive so a newly added preset
/// forces this mapping to be updated.
#[cfg(any(target_family = "wasm", test))]
fn preset_proof_value(id: FilterPresetId) -> &'static str {
    match id {
        FilterPresetId::All => "all",
        FilterPresetId::HighCpu => "high_cpu",
        FilterPresetId::HighMemory => "high_memory",
        FilterPresetId::LargeVirtualMemory => "large_virtual_memory",
        FilterPresetId::Running => "running",
    }
}

/// Appends the active filter preset to the DOM proof string.
///
/// Additive only: `none` or a fixed identifier from [`preset_proof_value`], so
/// every pre-existing `key=value;` pair stays byte-for-byte identical.
#[cfg(any(target_family = "wasm", test))]
fn proof_with_preset(proof: &str, preset: Option<FilterPresetId>) -> String {
    let value = preset.map(preset_proof_value).unwrap_or("none");
    format!("{proof}preset={value};")
}

/// Appends the adaptive power / sampling / cgroup summary to the DOM proof.
///
/// Additive only: every value is a fixed ASCII identifier or a number, so no
/// agent-derived text, host name or token can enter the proof. The pre-existing
/// `key=value;` pairs remain byte-for-byte identical (R1-I10..I12).
#[cfg(any(target_family = "wasm", test))]
fn proof_with_sampling(
    proof: &str,
    snapshot: Option<&SystemSnapshot>,
    effective_interval_ms: u64,
) -> String {
    let source = snapshot.map_or("none", |snapshot| snapshot.power.source.id());
    let profile = snapshot.map_or("none", |snapshot| snapshot.sampling.profile.id());
    let groups = snapshot.map_or(0, |snapshot| snapshot.cgroups.len());
    format!(
        "{proof}power={source};sampling={profile};sampling_ms={effective_interval_ms};cgroups={groups};"
    )
}

/// Any termination of an SSE subscription returns monitoring to the poll.
///
/// A clean EOF, a transport error and a settings change all leave the stream
/// unable to deliver snapshots, so the legacy path takes over. Kept pure so the
/// fallback contract can be tested without a browser context.
fn stream_mode_after_sse_end(_outcome: &Result<(), FetchError>) -> StreamMode {
    StreamMode::Fallback
}

/// Mode held while a subscription is being established.
///
/// A spawned task / accepted HTTP connection is not proof that frames will
/// arrive, so polling must stay on until the first snapshot is delivered. Kept
/// pure so the "no freeze before the first frame" contract can be unit tested.
fn stream_mode_while_connecting() -> StreamMode {
    StreamMode::Fallback
}

/// How long an active stream may stay silent before polling is resumed.
///
/// Design choice: three times the *applied* refresh interval, with a fixed
/// [`MIN_SSE_IDLE_TIMEOUT`] floor. The floor keeps the grace window independent
/// of the sampler cadence: agent frames arrive on the agent's own 1 s
/// `STREAM_INTERVAL`, so a 250 ms sampler interval must not turn a healthy 1 s
/// frame into an idle timeout. `effective_interval()` already carries the
/// adaptive power floor and `sampler_wait` clamps before multiplying; the final
/// `max` guarantees a window of 3 s .. 30 s.
fn sse_idle_timeout(interval: Duration) -> Duration {
    sampler_wait(interval)
        .saturating_mul(3)
        .max(MIN_SSE_IDLE_TIMEOUT)
}

/// Connecting-state watchdog: a suspended poll must not stay suspended when a
/// stream never delivers a snapshot (for example, it emits only `event: error`).
/// Pure so the regression can be tested without a GPUI entity.
fn connecting_watchdog_expired(
    polls_suspended: bool,
    last_snapshot: Option<Instant>,
    connect_started: Option<Instant>,
    now: Instant,
    interval: Duration,
) -> bool {
    polls_suspended
        && last_snapshot.is_none()
        && connect_started
            .is_some_and(|at| now.saturating_duration_since(at) >= sse_idle_timeout(interval))
}

/// Pure idle-watchdog decision, testable without a GPUI entity.
///
/// `last_snapshot == None` can never count as idle: the stream only becomes
/// `Active` on its first frame, and the timestamp is set in the same callback.
/// `saturating_duration_since` keeps the comparison total even if the clock is
/// observed slightly out of order.
fn sse_idle_expired(
    mode: StreamMode,
    last_snapshot: Option<Instant>,
    now: Instant,
    interval: Duration,
) -> bool {
    mode == StreamMode::Active
        && last_snapshot
            .is_some_and(|at| now.saturating_duration_since(at) >= sse_idle_timeout(interval))
}

/// Whether a frame still belongs to the current subscription.
///
/// A config change, retry or idle timeout bumps `sse_epoch`; a late frame from
/// the superseded stream must not overwrite the newer state. Kept pure so the
/// guard contract is testable on the native host.
fn sse_frame_is_current(frame_epoch: u64, shell_epoch: u64) -> bool {
    frame_epoch == shell_epoch
}

/// Appends the SSE transport marker to the DOM proof.
///
/// Additive only: a fixed `active` / `fallback` token as the final field, so
/// every pre-existing `key=value;` pair stays byte-for-byte identical and no
/// token, endpoint or agent-supplied text can enter the proof.
#[cfg(any(target_family = "wasm", test))]
fn proof_with_sse(proof: &str, mode: StreamMode) -> String {
    format!("{proof}sse={};", mode.proof_value())
}

/// Whether the row menu's full process identity is still in the snapshot.
///
/// A bare PID is not identity: the kernel reuses PIDs. The menu keeps the whole
/// [`ProcessKey`] (PID + start time), so a stale key must never authorise an
/// action after the next sample replaces the process.
fn row_menu_key_is_live(snapshot: Option<&SystemSnapshot>, key: ProcessKey) -> bool {
    snapshot.is_some_and(|snapshot| snapshot.processes.iter().any(|process| process.key == key))
}

/// Whether an open row menu must be closed after a new snapshot arrives.
fn row_menu_is_stale(snapshot: Option<&SystemSnapshot>, menu: Option<RowMenu>) -> bool {
    menu.is_some_and(|menu| !row_menu_key_is_live(snapshot, menu.key))
}

/// PID the CopyPid action may report, or `None` if the process was replaced or
/// vanished before the click. Kept pure so the PID-reuse rule is testable.
fn row_menu_copy_pid(snapshot: Option<&SystemSnapshot>, key: ProcessKey) -> Option<u32> {
    row_menu_key_is_live(snapshot, key).then_some(key.pid)
}

/// Status announced after the row menu's PID action. No clipboard write is
/// attempted in the browser (read-only shell); the numeric PID is the only
/// agent-derived value and it cannot carry separators or control characters.
fn row_menu_pid_status(pid: u32) -> String {
    format!("PID {pid} を選択しました（ブラウザ版はクリップボードへ書き込みません）")
}

/// First-run connection checklist shown in demo mode (R1-I16).
///
/// The strings come from the redaction-safe constants in `crate::agent`; no
/// hostname, token or agent text is ever interpolated here.
fn onboarding_card() -> Stateful<Div> {
    v_flex()
        .id("onboarding-card")
        .role(Role::Group)
        .aria_label("初回接続ガイド")
        .gap_1()
        .child(
            section_title("初回接続ガイド")
                .id("onboarding-title")
                .role(Role::Heading)
                .aria_level(2),
        )
        .children(onboarding_steps().iter().enumerate().map(|(index, step)| {
            div()
                .id(("onboarding-step", index))
                .text_xs()
                .text_color(rgb(palette().subtext))
                .child(*step)
        }))
}

/// Identifiers handed to the pure exporters.
///
/// The browser cannot read the agent host's environment, so identifiers are
/// recovered from the snapshot itself. Live snapshots were already redacted on
/// the HTTP boundary (see [`crate::redaction`]); deriving the values again here
/// means the exporters never run with empty identifiers, and a snapshot from any
/// other source still gets its host / user / home strings masked on export.
fn export_identifiers(snapshot: &SystemSnapshot) -> OwnedIdentifiers {
    identifiers_from_snapshot(snapshot)
}

/// Runs the pure `tm_core` exporter for the requested format.
///
/// Backward-compatible entry point for callers without a wire envelope (demo
/// snapshots and existing tests). Live exports use
/// [`export_document_with_total`] instead.
fn export_document(snapshot: &SystemSnapshot, format: ExportFormat) -> String {
    export_document_with_total(snapshot, format, None)
}

/// Runs the pure exporter while preserving the wire envelope's machine-wide
/// process count.
///
/// `GET /api/v1/snapshot?limit=N` truncates `snapshot.processes` but keeps
/// `total_process_count`; passing it through here stops an export from
/// shrinking that count to the number of published rows. `None` keeps the old
/// row-count fallback for snapshots with no envelope.
fn export_document_with_total(
    snapshot: &SystemSnapshot,
    format: ExportFormat,
    total_process_count: Option<u64>,
) -> String {
    let ids = export_identifiers(snapshot);
    let ids = ids.as_redaction_ids();
    match format {
        ExportFormat::Csv => snapshot_to_csv_with_total(snapshot, &ids, total_process_count),
        ExportFormat::Json => snapshot_to_json_with_total(snapshot, &ids, total_process_count),
    }
}

/// Token-aware export entry point used by the live shell.
///
/// `proof_token` is the exact value the DOM proof hook masks (`#token=` / the
/// agent config). `tm-agent` scrubs its own argv before publishing a snapshot,
/// but reusing the same value here gives the export an independent exact-match
/// boundary; the argv-shape helper additionally covers a `--token <value>`
/// fragment that reaches the browser from any other source. Both passes run on
/// a private clone, so the dashboard snapshot is never mutated. Tools that do
/// not own a token keep using [`export_document_with_total`] unchanged.
fn export_document_with_token(
    snapshot: &SystemSnapshot,
    format: ExportFormat,
    total_process_count: Option<u64>,
    token: Option<&str>,
) -> String {
    let mut masked = snapshot.clone();
    if let Some(token) = token {
        tm_core::redact::scrub_token_from_snapshot(&mut masked, token);
    }
    tm_core::redact::scrub_token_flags_from_snapshot(&mut masked);
    export_document_with_total(&masked, format, total_process_count)
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
/// the `Referer` header. As soon as a token is read it is moved into
/// [`REMEMBERED_TOKEN`] and the `#token=` pair is removed from the address bar
/// and the current session-history entry with `history.replaceState`. Later
/// sampler cycles therefore keep working from the in-memory copy even though
/// the URL (and `location.hash`) no longer carries the secret. The token is
/// never written to the DOM, the title, the console or the agent request URL.
fn agent_config_from_url() -> Option<AgentConfig> {
    #[cfg(target_family = "wasm")]
    {
        let location = web_sys::window()?.location();
        let search = location.search().ok()?;
        if search.is_empty() {
            return None;
        }
        let params = web_sys::UrlSearchParams::new_with_str(&search).ok()?;
        let endpoint = params.get("agent");
        let token = token_from_fragment(&location.hash().unwrap_or_default());
        let remembered = remembered_token();
        let resolved = resolve_agent_config(endpoint, token, remembered.as_ref());
        apply_memory_action(resolved.memory);
        if resolved.scrub_url {
            if let Ok(href) = location.href() {
                if let Some(clean) = href_without_token(&href) {
                    // Best-effort: a failed replaceState still leaves the token
                    // in memory, so reconnection keeps working either way.
                    let _ = tm_strip_url_token(&clean);
                }
            }
        }
        resolved.config
    }
    #[cfg(not(target_family = "wasm"))]
    {
        None
    }
}

/// `#token=` state observed in the current fragment.
#[cfg(any(target_family = "wasm", test))]
#[derive(Clone, Debug, PartialEq, Eq)]
enum UrlToken {
    /// No `token` key in the fragment.
    Absent,
    /// `#token=<non-empty value>`.
    Value(String),
    /// `#token=` with an empty value: an explicit request to forget the token.
    ExplicitEmpty,
}

/// Effect of one URL read on the in-memory token copy.
#[cfg(any(target_family = "wasm", test))]
#[derive(Clone, Debug, PartialEq, Eq)]
enum MemoryAction {
    /// Reuse what is already held.
    Keep,
    /// Replace it with the freshly read token, bound to its endpoint.
    Store { endpoint: String, token: String },
    /// Forget it (explicit `#token=` clear).
    Clear,
}

/// Pure result of merging the URL with the remembered token.
#[cfg(any(target_family = "wasm", test))]
#[derive(Debug, PartialEq, Eq)]
struct ResolvedConfig {
    config: Option<AgentConfig>,
    memory: MemoryAction,
    /// Whether the address bar still contains a `token` key to scrub.
    scrub_url: bool,
}

/// In-memory token, paired with the endpoint it belongs to.
///
/// Keeping the endpoint prevents a token issued to one loopback agent from
/// being reused if `?agent=` is later pointed at a different one. This is a
/// process-local value only: it is never mirrored into the DOM, status text or
/// logs, and it disappears on reload.
#[cfg(any(target_family = "wasm", test))]
static REMEMBERED_TOKEN: OnceLock<RwLock<Option<(String, String)>>> = OnceLock::new();

/// Reads the remembered `(endpoint, token)` pair, if any.
#[cfg(any(target_family = "wasm", test))]
fn remembered_token() -> Option<(String, String)> {
    REMEMBERED_TOKEN
        .get_or_init(|| RwLock::new(None))
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Applies one memory transition from [`resolve_agent_config`].
#[cfg(any(target_family = "wasm", test))]
fn apply_memory_action(action: MemoryAction) {
    let mut guard = REMEMBERED_TOKEN
        .get_or_init(|| RwLock::new(None))
        .write()
        .unwrap_or_else(PoisonError::into_inner);
    match action {
        MemoryAction::Keep => {}
        MemoryAction::Store { endpoint, token } => *guard = Some((endpoint, token)),
        MemoryAction::Clear => *guard = None,
    }
}

/// Reads the `token` key from the fragment, distinguishing "absent" from an
/// explicitly empty value; `UrlSearchParams` percent-decodes the value just as
/// the old inline read did.
#[cfg(target_family = "wasm")]
fn token_from_fragment(hash: &str) -> UrlToken {
    let fragment = hash.strip_prefix('#').unwrap_or(hash);
    let Ok(params) = web_sys::UrlSearchParams::new_with_str(fragment) else {
        return UrlToken::Absent;
    };
    match params.get("token") {
        Some(value) if value.trim().is_empty() => UrlToken::ExplicitEmpty,
        Some(value) => UrlToken::Value(value),
        None => UrlToken::Absent,
    }
}

/// Merges a URL read with the remembered token.
///
/// A token found in the fragment is stored (and the URL flagged for scrubbing);
/// an absent token falls back to the memory copy *only* for the same endpoint;
/// an explicitly empty `#token=` clears both memory and the emitted config.
/// Without `?agent=` the previous behaviour is preserved (no config change).
#[cfg(any(target_family = "wasm", test))]
fn resolve_agent_config(
    endpoint: Option<String>,
    token: UrlToken,
    remembered: Option<&(String, String)>,
) -> ResolvedConfig {
    let Some(endpoint) = endpoint else {
        return ResolvedConfig {
            config: None,
            memory: MemoryAction::Keep,
            scrub_url: false,
        };
    };
    // Same normalisation as `AgentConfig::new`, so the memory key matches the
    // endpoint the Bearer header will actually be sent to.
    let endpoint = endpoint.trim().trim_end_matches('/').to_string();
    if endpoint.is_empty() {
        return ResolvedConfig {
            config: None,
            memory: MemoryAction::Keep,
            scrub_url: false,
        };
    }
    match token {
        UrlToken::Value(value) => {
            let value = value.trim().to_string();
            if value.is_empty() {
                return ResolvedConfig {
                    config: Some(AgentConfig::new(endpoint, None)),
                    memory: MemoryAction::Clear,
                    scrub_url: true,
                };
            }
            ResolvedConfig {
                config: Some(AgentConfig::new(endpoint.clone(), Some(value.clone()))),
                memory: MemoryAction::Store {
                    endpoint,
                    token: value,
                },
                scrub_url: true,
            }
        }
        UrlToken::ExplicitEmpty => ResolvedConfig {
            config: Some(AgentConfig::new(endpoint, None)),
            memory: MemoryAction::Clear,
            scrub_url: true,
        },
        UrlToken::Absent => {
            let token = remembered
                .filter(|(saved_endpoint, _)| saved_endpoint == &endpoint)
                .map(|(_, token)| token.clone());
            ResolvedConfig {
                config: Some(AgentConfig::new(endpoint, token)),
                memory: MemoryAction::Keep,
                scrub_url: false,
            }
        }
    }
}

/// Returns `href` with the fragment's `token` key removed, or `None` when
/// nothing needs scrubbing. Other fragment keys are preserved verbatim and an
/// empty remaining fragment drops the `#` entirely.
#[cfg(any(target_family = "wasm", test))]
fn href_without_token(href: &str) -> Option<String> {
    let (base, fragment) = href.split_once('#')?;
    let mut kept = String::new();
    let mut had_token = false;
    for pair in fragment.split('&') {
        if pair.is_empty() {
            continue;
        }
        let key = pair.split_once('=').map_or(pair, |(key, _)| key);
        if key == "token" {
            had_token = true;
        } else {
            if !kept.is_empty() {
                kept.push('&');
            }
            kept.push_str(pair);
        }
    }
    if !had_token {
        return None;
    }
    Some(if kept.is_empty() {
        base.to_string()
    } else {
        format!("{base}#{kept}")
    })
}

/// Replaces the current URL (and history entry) without a page reload.
///
/// `web/Cargo.toml` is out of scope here and does not enable the `History`
/// feature, so the same inline-JS bridge pattern as the help/storage bridges
/// is used. Only the already-sanitised URL crosses into JS: the token value is
/// never passed as an argument, read back, or logged.
#[cfg(target_family = "wasm")]
#[wasm_bindgen(inline_js = r#"
export function tmStripUrlToken(url) {
  try {
    window.history.replaceState(window.history.state, '', url);
    return true;
  } catch (error) {
    return false;
  }
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = "tmStripUrlToken")]
    fn tm_strip_url_token(url: &str) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs production state transitions on a real (headless) `TaskManagerApp`
    /// and snapshots the observable state afterwards. The instance matches
    /// `TaskManagerApp::new` minus the window-bound sampler / subscription
    /// side effects, so these assertions describe the same state the browser
    /// would expose via `body[data-gpui-proof]`.
    fn run_sse_steps(
        steps: impl FnOnce(&mut TaskManagerApp) + Send + 'static,
    ) -> (bool, Option<String>, Option<String>, String) {
        fn build(cx: &mut Context<TaskManagerApp>) -> TaskManagerApp {
            let platform = PlatformInfo::native_read_only("Linux (エージェント経由)");
            let dashboard = Dashboard::new(platform, DEFAULT_REFRESH_INTERVAL);
            TaskManagerApp {
                dashboard,
                filter_input: FilterInput::new(),
                filter_editing: false,
                source: DataSource::Demo(DemoProbe::new()),
                proof_token: None,
                focus_handle: cx.focus_handle(),
                sort_header_focus: std::array::from_fn(|_| cx.focus_handle()),
                row_menu_item_focus: cx.focus_handle(),
                scroll_handle: UniformListScrollHandle::new(),
                live_total: None,
                live_error: None,
                reconnect_required: false,
                stale: false,
                ticks: 0,
                last_export: None,
                row_menu: None,
                menu_proof: MenuProof::None,
                stream_mode: StreamMode::Fallback,
                sse_task: None,
                sse_poll_guard: None,
                sse_epoch: 0,
                sse_last_snapshot_at: None,
                sse_connect_started_at: None,
                sse_retry_at: None,
                poll_backoff: PollBackoff::default(),
                show_help: false,
                help_close_focus: cx.focus_handle(),
                last_help_shown: false,
            }
        }

        // Headless Linux keeps its event loop alive even after `quit()`, so the
        // app runs on a detached thread and the snapshot travels back through a
        // bounded channel. The callback itself is synchronous: by the time
        // `recv_timeout` returns, every assertion below has observed the real
        // post-callback state.
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            gpui_platform::headless().run(move |cx: &mut gpui::App| {
                let entity = cx.new(build);
                entity.update(cx, |app, _cx| {
                    steps(app);
                    let _ = sender.send((
                        app.reconnect_required,
                        app.live_error.clone(),
                        app.dashboard.status().map(|status| status.text.clone()),
                        app.proof_string(0, 0),
                    ));
                });
                cx.quit();
            });
        });
        receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("headless callback must publish the snapshot")
    }

    /// Ends the stream once through the production callback.
    fn sse_end_state(
        outcome: Result<(), FetchError>,
    ) -> (bool, Option<String>, Option<String>, String) {
        run_sse_steps(move |app| app.on_sse_ended(outcome))
    }

    /// Observable state after a live-mode poll-guard transition.
    #[derive(Debug)]
    struct LiveGuardState {
        guard_held: bool,
        polls_suspended: bool,
        generation: u64,
        active: bool,
        ticks: u64,
        live_error: Option<String>,
        reconnect: bool,
        status: Option<String>,
        proof: String,
    }

    /// Runs the production guard transitions on a live `TaskManagerApp`.
    ///
    /// The `AgentClient` is shared with the caller so the test can observe the
    /// suspension directly. No sampler or SSE task is spawned and no HTTP
    /// request is made: every assertion below is about local state only.
    fn run_live_sse_steps<R>(
        steps: impl FnOnce(&mut TaskManagerApp, Arc<AgentClient>) -> R + Send + 'static,
    ) -> (LiveGuardState, R)
    where
        R: Send + 'static,
    {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            gpui_platform::headless().run(move |cx: &mut gpui::App| {
                let client = Arc::new(AgentClient::new(
                    AgentConfig::new("http://127.0.0.1:18789", None),
                    cx.http_client(),
                    cx.background_executor().clone(),
                ));
                let observed = Arc::clone(&client);
                let entity = cx.new(|test_cx| {
                    let platform = PlatformInfo::native_read_only("Linux (エージェント経由)");
                    let dashboard = Dashboard::new(platform, DEFAULT_REFRESH_INTERVAL);
                    TaskManagerApp {
                        dashboard,
                        filter_input: FilterInput::new(),
                        filter_editing: false,
                        source: DataSource::Live(client),
                        proof_token: None,
                        focus_handle: test_cx.focus_handle(),
                        sort_header_focus: std::array::from_fn(|_| test_cx.focus_handle()),
                        row_menu_item_focus: test_cx.focus_handle(),
                        scroll_handle: UniformListScrollHandle::new(),
                        live_total: None,
                        live_error: None,
                        reconnect_required: false,
                        stale: false,
                        ticks: 0,
                        last_export: None,
                        row_menu: None,
                        menu_proof: MenuProof::None,
                        stream_mode: StreamMode::Fallback,
                        sse_task: None,
                        sse_poll_guard: None,
                        sse_epoch: 0,
                        sse_last_snapshot_at: None,
                        sse_connect_started_at: None,
                        sse_retry_at: None,
                        poll_backoff: PollBackoff::default(),
                        show_help: false,
                        help_close_focus: test_cx.focus_handle(),
                        last_help_shown: false,
                    }
                });
                entity.update(cx, |app, _| {
                    let produced = steps(app, Arc::clone(&observed));
                    let _ = sender.send((
                        LiveGuardState {
                            guard_held: app.sse_poll_guard.is_some(),
                            polls_suspended: observed.polls_suspended(),
                            generation: observed.poll_generation().current(),
                            active: app.stream_mode == StreamMode::Active,
                            ticks: app.ticks,
                            live_error: app.live_error.clone(),
                            reconnect: app.reconnect_required,
                            status: app.dashboard.status().map(|status| status.text.clone()),
                            proof: app.proof_string(0, 0),
                        },
                        produced,
                    ));
                });
                cx.quit();
            });
        });
        receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("headless callback must publish the live-guard snapshot")
    }

    #[test]
    fn token_from_url_is_remembered_and_survives_url_removal() {
        let endpoint = "http://127.0.0.1:8787";
        let token = "s3cr3t-123";
        let first = resolve_agent_config(
            Some(format!("{endpoint}/")),
            UrlToken::Value(token.to_string()),
            None,
        );
        // A URL token is captured immediately and the address bar is flagged
        // for scrubbing in the very same read.
        assert!(first.scrub_url);
        assert_eq!(
            first.memory,
            MemoryAction::Store {
                endpoint: endpoint.to_string(),
                token: token.to_string(),
            }
        );
        let config = first.config.expect("live config");
        assert_eq!(config.endpoint, endpoint);
        assert_eq!(config.token.as_deref(), Some(token));

        let remembered = match first.memory {
            MemoryAction::Store { endpoint, token } => Some((endpoint, token)),
            _ => unreachable!("a URL token must be stored"),
        };
        // The token has disappeared from the URL, but the sampler still gets a
        // usable config from memory (this is what makes reconnect work).
        let second = resolve_agent_config(
            Some(endpoint.to_string()),
            UrlToken::Absent,
            remembered.as_ref(),
        );
        assert!(!second.scrub_url);
        assert_eq!(second.memory, MemoryAction::Keep);
        let reconnected = second.config.expect("live config");
        assert_eq!(reconnected.token.as_deref(), Some(token));

        // The exact-match proof mask is unchanged for a memory-sourced token:
        // no plaintext value reaches the DOM proof.
        let proof = proof_user_text(&format!("filter={token}"), reconnected.token.as_deref());
        assert!(!proof.contains(token));
        assert!(proof.contains("***"));
    }

    #[test]
    fn a_new_url_token_overwrites_the_remembered_one() {
        let endpoint = "http://127.0.0.1:8787";
        let old = (endpoint.to_string(), "old-token".to_string());
        let resolved = resolve_agent_config(
            Some(endpoint.to_string()),
            UrlToken::Value("fresh-token".to_string()),
            Some(&old),
        );
        assert_eq!(
            resolved.config.expect("live config").token.as_deref(),
            Some("fresh-token")
        );
        assert_eq!(
            resolved.memory,
            MemoryAction::Store {
                endpoint: endpoint.to_string(),
                token: "fresh-token".to_string(),
            }
        );
        assert!(resolved.scrub_url);
    }

    #[test]
    fn explicit_empty_token_clears_memory_and_config() {
        let endpoint = "http://127.0.0.1:8787";
        let old = (endpoint.to_string(), "old-token".to_string());
        for empty in [UrlToken::ExplicitEmpty, UrlToken::Value("   ".to_string())] {
            let resolved = resolve_agent_config(Some(endpoint.to_string()), empty, Some(&old));
            assert_eq!(resolved.config.expect("live config").token, None);
            assert_eq!(resolved.memory, MemoryAction::Clear);
            assert!(resolved.scrub_url);
        }
        // After an explicit clear, an absent token must not resurrect the old one.
        let after = resolve_agent_config(Some(endpoint.to_string()), UrlToken::Absent, None);
        assert_eq!(after.config.expect("live config").token, None);
        assert_eq!(after.memory, MemoryAction::Keep);
    }

    #[test]
    fn remembered_token_is_not_reused_for_a_different_endpoint() {
        let token = "loopback-token".to_string();
        let old = ("http://127.0.0.1:8787".to_string(), token.clone());
        let resolved = resolve_agent_config(
            Some("http://127.0.0.1:9999".to_string()),
            UrlToken::Absent,
            Some(&old),
        );
        // The SSRF gate still applies, but the old agent's bearer token must
        // never be attached to a newly configured endpoint.
        assert_eq!(resolved.config.expect("live config").token, None);
        assert_eq!(resolved.memory, MemoryAction::Keep);
        assert!(!resolved.scrub_url);
    }

    #[test]
    fn only_the_token_fragment_key_is_scrubbed_from_the_url() {
        let base = "http://127.0.0.1:8091/?agent=http%3A%2F%2F127.0.0.1%3A8787";
        assert_eq!(
            href_without_token(&format!("{base}#token=abc")),
            Some(base.to_string())
        );
        assert_eq!(
            href_without_token(&format!("{base}#token=abc&view=list")),
            Some(format!("{base}#view=list"))
        );
        assert_eq!(
            href_without_token(&format!("{base}#view=list&token=")),
            Some(format!("{base}#view=list"))
        );
        assert_eq!(href_without_token(&format!("{base}#view=list")), None);
        assert_eq!(href_without_token(base), None);
    }

    #[test]
    fn in_memory_copy_can_be_stored_kept_and_cleared() {
        // Only this test touches the process-local static, so the sequence is
        // deterministic even though the harness runs tests in parallel.
        apply_memory_action(MemoryAction::Clear);
        assert_eq!(remembered_token(), None);
        let pair = (
            "http://127.0.0.1:8787".to_string(),
            "s3cr3t-123".to_string(),
        );
        apply_memory_action(MemoryAction::Store {
            endpoint: pair.0.clone(),
            token: pair.1.clone(),
        });
        assert_eq!(remembered_token(), Some(pair.clone()));
        apply_memory_action(MemoryAction::Keep);
        assert_eq!(remembered_token(), Some(pair));
        apply_memory_action(MemoryAction::Clear);
        assert_eq!(remembered_token(), None);
    }

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
        // The overview cards draw the same word next to the number, mirroring
        // the temperature gauge instead of relying on colour alone.
        assert_eq!(usage_value_text(90.0), "90.0% 高");
        assert_eq!(usage_value_text(70.0), "70.0% 中");
        assert_eq!(usage_value_text(10.0), "10.0% 低");
    }

    #[test]
    fn sort_direction_has_a_textual_name() {
        assert_eq!(a11y_sort_direction(SortDirection::Ascending), "昇順");
        assert_eq!(a11y_sort_direction(SortDirection::Descending), "降順");
    }

    #[test]
    fn sort_header_visible_text_changes_with_direction() {
        // Tofu regression: ▲/▼ were missing from IBM Plex, making the asc/desc
        // screenshots byte-identical. Both shells must show distinct glyphs.
        let asc = sort_header_text("CPU", true, SortDirection::Ascending);
        let desc = sort_header_text("CPU", true, SortDirection::Descending);
        assert_eq!(asc, "CPU ↑");
        assert_eq!(desc, "CPU ↓");
        assert_ne!(asc, desc);
        assert_eq!(
            sort_header_text("CPU", false, SortDirection::Ascending),
            "CPU"
        );
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
    fn consecutive_poll_failures_grow_the_backoff_and_respect_the_cap() {
        let interval = DEFAULT_REFRESH_INTERVAL;
        // Fresh / successful state keeps the configured cadence untouched.
        assert_eq!(poll_retry_wait(interval, 0, None), interval);
        // Five consecutive 503s without a hint: 1, 2, 4, 8, 16 s.
        let expected = [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(8),
            Duration::from_secs(16),
        ];
        for (failures, want) in expected.iter().enumerate() {
            let got = poll_retry_wait(interval, failures as u32 + 1, None);
            assert!(
                got > interval || failures == 0,
                "failure {} must back off",
                failures
            );
            assert_eq!(got, *want, "unexpected wait at failure {}", failures + 1);
        }
        // Beyond the cap the wait stops growing, even for a huge streak.
        for failures in [6, 10, u32::MAX] {
            assert_eq!(
                poll_retry_wait(interval, failures, None),
                POLL_BACKOFF_MAX,
                "failure {failures} must stay capped"
            );
        }
    }

    #[test]
    fn poll_backoff_resets_on_success_and_consumes_retry_after_once() {
        let mut backoff = PollBackoff::default();
        let interval = DEFAULT_REFRESH_INTERVAL;
        for want in [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
        ] {
            backoff.record_failure(None);
            assert_eq!(backoff.take_wait(interval), want);
        }
        // A server hint raises this one attempt but is not applied forever.
        backoff.record_failure(Some(Duration::from_secs(10)));
        assert_eq!(backoff.take_wait(interval), Duration::from_secs(10));
        assert_eq!(backoff.take_wait(interval), Duration::from_secs(8));

        // One success restores the normal cadence from any depth.
        backoff.record_success();
        assert_eq!(backoff, PollBackoff::default());
        assert_eq!(backoff.take_wait(interval), interval);
        backoff.record_failure(Some(Duration::from_secs(10)));
        backoff.record_success();
        assert_eq!(backoff.take_wait(interval), interval);
    }

    #[test]
    fn poll_retry_after_is_bounded_and_never_speeds_up_the_configured_cadence() {
        // A huge hint is clamped by the same ceiling as exponential backoff.
        assert_eq!(
            poll_retry_wait(
                DEFAULT_REFRESH_INTERVAL,
                1,
                Some(Duration::from_secs(3_600))
            ),
            POLL_BACKOFF_MAX
        );
        // A slow user-configured cadence must not be shortened by backoff or
        // by a hint smaller than the interval.
        let slow = MAX_REFRESH_INTERVAL;
        assert_eq!(poll_retry_wait(slow, 0, None), slow);
        assert_eq!(poll_retry_wait(slow, 1, None), slow);
        assert_eq!(poll_retry_wait(slow, 1, Some(Duration::from_secs(2))), slow);
        assert_eq!(
            poll_retry_wait(slow, 1, Some(Duration::from_secs(20))),
            Duration::from_secs(20)
        );
        // Backoff is independent of the SSE cooldown: the two never share state.
        assert_ne!(POLL_BACKOFF_MAX, SSE_RECONNECT_DELAY);
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

    #[test]
    fn dom_proof_filter_fields_cannot_inject_key_value_boundaries() {
        // 既存の ';' -> ',' 置換はバイト単位で維持（旧 CDP フック不変）。
        assert_eq!(proof_field_text("cpu;mem"), "cpu,mem");
        assert_eq!(proof_field_text(""), "");

        let clean = proof_field_text("a=1;\r\nb=2;=;\n");
        assert_eq!(clean, "a1,b2,,");
        assert!(!clean.contains('='));
        assert!(!clean.contains(';'));
        assert!(!clean.contains('\r'));
        assert!(!clean.contains('\n'));

        // filter / ime_field を実際の proof と同じ形へ埋め込むと、どんな
        // ユーザー入力でも key=value; のフィールド数が増えない。
        for raw in ["a=1;\r\nb=2;=;\n", "plain", "a=b;\r\ninject=1", ""] {
            let value = proof_field_text(raw);
            let proof = format!("mode=demo;filter={value};ime_field={value};rows=3;");
            let fields: Vec<&str> = proof.split(';').filter(|part| !part.is_empty()).collect();
            assert_eq!(fields.len(), 4, "{proof}");
            for field in fields {
                assert_eq!(field.matches('=').count(), 1, "{field} in {proof}");
            }
            assert!(proof.starts_with("mode=demo;"), "{proof}");
        }
    }

    #[test]
    fn dom_proof_masks_exact_token_without_changing_contract() {
        // 長さや文字種で推測せず、渡された実際のトークンだけをマスクする。
        let token = "s3cr3t-123";
        let masked = proof_user_text(&format!("filter {token}"), Some(token));
        assert_eq!(masked, "filter ***", "{masked}");
        assert!(!masked.contains(token), "{masked}");

        // 複数出現・埋め込み・前後のユーザー文字もそのまま保持する。
        assert_eq!(
            proof_user_text(&format!("{token}/middle/{token}"), Some(token)),
            "***/middle/***"
        );
        assert_eq!(proof_user_text(&format!("x{token}y"), Some(token)), "x***y");

        // トークンが無い／空なら既存 proof_field_text とバイト同一。
        // 32 文字以上の hex もトークンでなければ消さない（ヒューリスティック廃止）。
        let long_hex = "a".repeat(64);
        for raw in ["cpu;mem", "plain", "a=1;\r\nb=2;=;\n", "", &long_hex] {
            assert_eq!(proof_user_text(raw, None), proof_field_text(raw), "{raw:?}");
        }
        assert!(proof_user_text(&long_hex, Some(token)).contains(&long_hex));

        // 既存キーと末尾 sse= 契約を壊さない。
        let proof = format!(
            "mode=demo;filter={};ime_field={};rows=3;sse={};",
            proof_user_text(token, Some(token)),
            proof_user_text(&format!("x{token}y"), Some(token)),
            StreamMode::Fallback.proof_value(),
        );
        assert!(proof.ends_with("sse=fallback;"), "{proof}");
        assert!(!proof.contains(token), "{proof}");
        let fields: Vec<&str> = proof.split(';').filter(|part| !part.is_empty()).collect();
        assert_eq!(fields.len(), 5, "{proof}");
        for field in fields {
            assert_eq!(field.matches('=').count(), 1, "{field} in {proof}");
        }
    }

    #[test]
    fn dom_proof_masks_short_non_hex_token_after_sanitising_boundaries() {
        // `=`, `;`, 改行を含むトークンもサニタイズ前に完全一致で消す。
        let token = "a=b;\r\nc";
        let input = format!("keep{token};tail=1");
        let masked = proof_user_text(&input, Some(token));
        assert_eq!(masked, "keep***,tail1");
        assert!(!masked.contains('=') && !masked.contains(';'));
        assert!(!masked.contains('\r') && !masked.contains('\n'));
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
        let owned = export_identifiers(&snapshot);
        let ids = owned.as_redaction_ids();
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
    fn export_document_preserves_wire_total_process_count() {
        let snapshot = export_sample_snapshot();
        let rows = snapshot.processes.len();
        assert_eq!(rows, 1);

        // Wire envelope: `?limit=` published one row but the machine has 30.
        let wire = export_document_with_total(&snapshot, ExportFormat::Json, Some(30));
        let value: serde_json::Value = serde_json::from_str(&wire).expect("valid JSON export");
        assert_eq!(value["total_process_count"].as_u64(), Some(30), "{wire}");
        assert_eq!(
            value["snapshot"]["processes"]
                .as_array()
                .map(|processes| processes.len()),
            Some(rows)
        );

        // No envelope (demo snapshot): keep the published row count, and the
        // 2-argument wrapper must stay byte-identical.
        let local = export_document_with_total(&snapshot, ExportFormat::Json, None);
        let value: serde_json::Value = serde_json::from_str(&local).expect("valid JSON export");
        assert_eq!(value["total_process_count"].as_u64(), Some(rows as u64));
        assert_eq!(export_document(&snapshot, ExportFormat::Json), local);

        // CSV has no envelope field: the fixed 12-column contract is intact.
        let csv = export_document_with_total(&snapshot, ExportFormat::Csv, Some(30));
        assert_eq!(csv, export_document(&snapshot, ExportFormat::Csv));
        assert_eq!(csv.lines().count(), rows + 1, "{csv}");
        assert_eq!(csv.lines().nth(1).unwrap().matches(',').count(), 11);
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
    fn export_document_with_token_masks_the_proof_token_everywhere() {
        let token = "super-secret-agent-token";
        let mut snapshot = export_sample_snapshot();
        // Exact-value masking must cover both the argv position and an
        // unrelated part of the command that the argv helper cannot see.
        snapshot.processes[0].command =
            format!("tm-agent --token {token} --once --header Bearer-{token}");
        snapshot.processes[0].name = format!("tm-agent-{token}");
        let before = format!("{snapshot:?}");
        for format in [ExportFormat::Csv, ExportFormat::Json] {
            let text = export_document_with_token(&snapshot, format, Some(30), Some(token));
            assert!(!text.contains(token), "{format:?} leaked token: {text}");
            assert!(text.contains("***"), "{format:?}: {text}");
            // The wire total contract remains intact after masking.
            if format == ExportFormat::Json {
                let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
                assert_eq!(value["total_process_count"].as_u64(), Some(30));
            }
        }
        assert_eq!(
            format!("{snapshot:?}"),
            before,
            "export must not mutate the snapshot"
        );
    }

    #[test]
    fn export_document_with_token_scrubs_argv_token_without_an_exact_value() {
        let token = "argv-only-secret";
        let mut snapshot = export_sample_snapshot();
        snapshot.processes[0].command = format!("tm-agent --token {token} --once");
        for format in [ExportFormat::Csv, ExportFormat::Json] {
            let text = export_document_with_token(&snapshot, format, None, None);
            assert!(
                !text.contains(token),
                "{format:?} leaked argv token: {text}"
            );
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
        assert!(DEMO_MODE_NOTICE.contains("tm-agent"));
        assert!(DEMO_MODE_NOTICE.contains("--redact"));
        assert!(DEMO_MODE_NOTICE.contains("?agent="));
        assert!(DEMO_MODE_NOTICE.contains("#token="));
        // 既定の待受ポートは wire 契約の単一起源（tm_core::DEFAULT_PORT）と
        // 交叉検証する。concat! は数値 const を展開できないため、文字列側は
        // リテラルのまま期待値を wire 定数から組み立ててドリフトを検出する。
        let expected = format!("127.0.0.1:{}", tm_core::DEFAULT_PORT);
        assert!(DEMO_MODE_NOTICE.contains(&expected));
        assert!(!DEMO_MODE_NOTICE.contains("18787"));
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
    fn demo_status_notice_is_persistent_info_not_error() {
        // デモ通知はエラー扱いにしない。詳細（8787 案内）はヘルプ側に残す。
        let mut dashboard = Dashboard::new(PlatformInfo::native("Linux"), DEFAULT_REFRESH_INTERVAL);
        dashboard.set_persistent_info(DEMO_MODE_STATUS_NOTICE);
        let status = dashboard.status().expect("demo status notice");
        assert!(
            !status.is_error,
            "demo mode is guidance, not a failure: {:?}",
            status
        );
        assert!(
            status.persistent,
            "demo notice must survive the first sample"
        );
        assert!(
            status.text.starts_with(tm_viewmodel::STATUS_PREFIX_INFO),
            "{}",
            status.text
        );
        assert!(status.text.ends_with(DEMO_MODE_STATUS_NOTICE));
        assert!(DEMO_MODE_STATUS_NOTICE.contains("F1"));
        assert!(!DEMO_MODE_STATUS_NOTICE.contains(';'));
        assert!(!DEMO_MODE_STATUS_NOTICE.contains('\n'));
        assert!(DEMO_MODE_NOTICE.contains("127.0.0.1:8787"));
    }

    #[test]
    fn demo_export_notice_yields_to_the_standing_guide_on_next_sample() {
        // SC-STATUSRESTORE: mode=demo installs the standing notice, an export
        // confirmation covers it briefly, and the next demo sample must bring
        // the guide back instead of ending on "ステータス: 変更なし".
        let mut demo = Dashboard::new(PlatformInfo::demo(), DEFAULT_REFRESH_INTERVAL);
        demo.set_persistent_info(DEMO_MODE_STATUS_NOTICE);
        let guide = demo.status().cloned().expect("demo guide");

        demo.set_status(export_status_message(ExportFormat::Csv, 12, 4096), false);
        let export_notice = demo.status().cloned().expect("export notice");
        assert!(!export_notice.persistent);
        assert!(export_notice.text.contains("CSV"));
        assert_ne!(export_notice.text, guide.text);

        demo.ingest(tm_core::demo::demo_snapshot(0));
        assert_eq!(demo.status(), Some(&guide));
        let announced = a11y_live_text(demo.status(), false);
        assert!(!announced.contains("ステータス: 変更なし"), "{announced}");
        assert!(announced.contains(DEMO_MODE_STATUS_NOTICE), "{announced}");

        // mode=live never installs the demo guide, so the same clear leaves an
        // empty status instead of surfacing demo guidance.
        let mut live = Dashboard::new(
            PlatformInfo::native_read_only("Linux (エージェント経由)"),
            DEFAULT_REFRESH_INTERVAL,
        );
        live.set_status("エージェントに接続しています…", false);
        live.ingest(tm_core::demo::demo_snapshot(0));
        assert!(live.status().is_none());
        assert_eq!(a11y_live_text(live.status(), false), "ステータス: 変更なし");
    }

    #[test]
    fn f1_toggles_help_and_escape_closes_it() {
        let mut shown = false;
        // F1 opens, another F1 closes (case-insensitive like `on_key_down`).
        shown = help_overlay_key_state(shown, "f1").expect("F1 opens help");
        assert!(shown);
        shown = help_overlay_key_state(shown, "F1").expect("F1 closes help");
        assert!(!shown);

        // F1 reopens; Esc closes only while the overlay is visible.
        shown = help_overlay_key_state(shown, "f1").expect("F1 reopens help");
        assert!(shown);
        shown = help_overlay_key_state(shown, "Escape").expect("Esc closes help");
        assert!(!shown);
        assert_eq!(
            help_overlay_key_state(false, "escape"),
            None,
            "Esc with no overlay must still reach the filter"
        );
        assert_eq!(
            help_overlay_key_state(true, "arrowdown"),
            None,
            "navigation keys must not change the overlay"
        );
    }

    #[test]
    fn help_closes_via_all_three_routes_without_reopening() {
        assert_eq!(HelpCloseRoute::from_key("F1"), Some(HelpCloseRoute::F1));
        assert_eq!(
            HelpCloseRoute::from_key("escape"),
            Some(HelpCloseRoute::Escape)
        );
        assert_eq!(HelpCloseRoute::from_key("arrowdown"), None);

        // F1 / Escape（GPUI 側）と DOM「閉じる」の 3 経路すべてが閉じる。
        for route in [
            HelpCloseRoute::F1,
            HelpCloseRoute::Escape,
            HelpCloseRoute::DomButton,
        ] {
            assert_eq!(help_close_state(true, route), Some(false), "{route:?}");
        }
        // DOM 経路はクローズ専用。二重に届いてもトグルで再オープンしない。
        assert_eq!(help_close_state(false, HelpCloseRoute::DomButton), None);
        assert_eq!(help_close_state(false, HelpCloseRoute::Escape), None);
        // F1 だけは明示的なトグルとして残る（ネイティブ契約を壊さない）。
        assert_eq!(help_close_state(false, HelpCloseRoute::F1), Some(true));

        // いずれの経路で閉じてもフォーカス復帰は dom_help_focus の 1 箇所だけ。
        let after = help_close_state(true, HelpCloseRoute::DomButton).unwrap();
        assert_eq!(dom_help_focus(true, after), DomHelpFocus::Leave);
    }

    #[test]
    fn help_shortcuts_cover_f1_escape_and_native_rows() {
        // C2-UXKEY: 列見出しの Enter/Space と行メニューの Shift+F10 /
        // ContextMenu を追記したので 12 → 14 行。
        assert_eq!(HELP_SHORTCUTS.len(), 14);
        let keys: Vec<_> = HELP_SHORTCUTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.contains(&"F1"));
        assert!(keys.contains(&"Esc"));
        assert!(keys.contains(&"Enter / Space"));
        assert!(keys.contains(&"Shift+F10 / ContextMenu"));
        let f1 = HELP_SHORTCUTS
            .iter()
            .find(|(key, _)| *key == "F1")
            .expect("F1 row");
        assert_eq!(f1.1, "このヘルプを開く / 閉じる");
        let esc = HELP_SHORTCUTS
            .iter()
            .find(|(key, _)| *key == "Esc")
            .expect("Esc row");
        assert!(esc.1.contains("ヘルプを閉じる"));
    }

    #[test]
    fn auth_failure_guidance_is_announced_and_token_free() {
        let guidance = crate::agent::RECONNECT_GUIDANCE;
        let error = FetchError::Auth;
        assert_eq!(error.to_string(), guidance);
        assert!(crate::agent::requires_reconnect(&error));
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
        let auth = FetchError::Auth;
        assert!(next_reconnect_required(false, &auth));
        // A generic follow-up failure must not clear the auth guidance.
        assert!(next_reconnect_required(true, &FetchError::Connect));
        assert!(next_reconnect_required(true, &FetchError::Http(500)));
        // Without a prior auth error, a generic error stays non-reconnect.
        assert!(!next_reconnect_required(false, &FetchError::Connect));
        // 403 is an origin problem: it never sets the flag and never clears a
        // sticky 401 flag either (C2-WEB).
        let forbidden = FetchError::Forbidden;
        assert!(!next_reconnect_required(false, &forbidden));
        assert!(next_reconnect_required(true, &forbidden));
        assert!(
            forbidden
                .to_string()
                .contains("別ホスト／別オリジンからのアクセスはサポートしていません")
        );
    }

    #[test]
    fn settings_save_failure_is_mapped_to_a_fixed_dom_safe_status() {
        assert_eq!(settings_save_status(&Ok(())), None);
        // The underlying message may vary; only the fixed notice is surfaced.
        for detail in [
            "localStorage を利用できません",
            "設定を保存できません（ストレージ容量または権限）",
            "設定を JSON に変換できません: path=/home/secret token=abc",
        ] {
            assert_eq!(
                settings_save_status(&Err(detail.to_string())),
                Some(SETTINGS_SAVE_ERROR_NOTICE)
            );
        }
        assert!(SETTINGS_SAVE_ERROR_NOTICE.contains("設定を保存できません"));
        assert!(!SETTINGS_SAVE_ERROR_NOTICE.contains(';'));
        assert!(!SETTINGS_SAVE_ERROR_NOTICE.contains('\n'));
        assert!(!SETTINGS_SAVE_ERROR_NOTICE.contains("secret"));
        let announced = a11y_live_text(
            Some(&StatusMessage {
                text: SETTINGS_SAVE_ERROR_NOTICE.to_string(),
                is_error: true,
                persistent: true,
            }),
            false,
        );
        assert!(announced.contains("設定を保存できません"), "{announced}");
    }

    #[test]
    fn cross_tab_reload_guard_rejects_reentry() {
        // A burst of storage events must not start a second reload while the
        // first one is still applying settings to the Dashboard.
        let mut in_flight = false;
        assert!(begin_settings_sync(&mut in_flight));
        assert!(in_flight);
        assert!(
            !begin_settings_sync(&mut in_flight),
            "re-entrant storage event must be ignored"
        );
        assert!(
            !begin_settings_sync(&mut in_flight),
            "every event until release must be ignored"
        );
        end_settings_sync(&mut in_flight);
        assert!(!in_flight);
        assert!(
            begin_settings_sync(&mut in_flight),
            "guard must be reusable"
        );
        end_settings_sync(&mut in_flight);
    }

    #[test]
    fn settings_sync_notice_is_fixed_and_dom_safe() {
        // The cross-tab notice follows the same redaction / DOM-proof contract
        // as the existing save-failure notice (no key, value, `;` or newline).
        assert!(SETTINGS_SYNC_NOTICE.contains("設定"));
        assert!(!SETTINGS_SYNC_NOTICE.contains(';'));
        assert!(!SETTINGS_SYNC_NOTICE.contains('\n'));
        assert!(!SETTINGS_SYNC_NOTICE.contains(STORAGE_KEY));
        let announced = a11y_live_text(
            Some(&StatusMessage {
                text: SETTINGS_SYNC_NOTICE.to_string(),
                is_error: false,
                persistent: false,
            }),
            false,
        );
        assert!(announced.contains("設定"), "{announced}");
        assert_eq!(SETTINGS_STORAGE_EVENT, "tm-settings-storage-changed");

        // The sync notice is informational: it may replace no/transient status
        // but must never overwrite a persistent corrupt / save-failure notice.
        assert!(should_announce_settings_sync(None));
        assert!(should_announce_settings_sync(Some(&StatusMessage {
            text: "今すぐ更新を要求しました".to_string(),
            is_error: false,
            persistent: false,
        })));
        assert!(!should_announce_settings_sync(Some(&StatusMessage {
            text: "保存された設定を読み込めませんでした".to_string(),
            is_error: true,
            persistent: true,
        })));
    }

    #[test]
    fn sse_fallback_notice_uses_the_same_persistent_guard_as_cross_tab_sync() {
        // The fallback notice is informational: it replaces no/transient status
        // but must never overwrite a persistent corrupt / save-failure notice.
        assert_eq!(sse_fallback_status(None), Some(SSE_FALLBACK_NOTICE));
        assert_eq!(
            sse_fallback_status(Some(&StatusMessage {
                text: "今すぐ更新を要求しました".to_string(),
                is_error: false,
                persistent: false,
            })),
            Some(SSE_FALLBACK_NOTICE)
        );
        for text in [
            crate::settings::NOTICE_CORRUPT,
            crate::settings::NOTICE_SAVE_FAILED,
        ] {
            assert_eq!(
                sse_fallback_status(Some(&StatusMessage {
                    text: text.to_string(),
                    is_error: true,
                    persistent: true,
                })),
                None,
                "{text}"
            );
        }
        // A persistent *info* notice is protected too, exactly as in the
        // cross-tab guard (the helper does not depend on `is_error`).
        assert_eq!(
            sse_fallback_status(Some(&StatusMessage {
                text: "デモデータで起動しました".to_string(),
                is_error: false,
                persistent: true,
            })),
            None
        );
        assert_eq!(
            should_announce_settings_sync(None),
            sse_fallback_status(None).is_some()
        );
    }

    #[test]
    fn sse_fallback_keeps_a_real_persistent_dashboard_notice() {
        let mut dashboard = Dashboard::new(PlatformInfo::native("Linux"), DEFAULT_REFRESH_INTERVAL);
        dashboard.set_persistent_status(crate::settings::NOTICE_CORRUPT);
        let before = dashboard.status().cloned().expect("persistent notice");
        assert!(before.persistent && before.is_error);
        assert_eq!(sse_fallback_status(dashboard.status()), None);
        // No fallback write happened: the displayed status is unchanged.
        assert_eq!(dashboard.status(), Some(&before));
        assert!(before.text.contains(crate::settings::NOTICE_CORRUPT));

        // A transient status still receives the fixed fallback text.
        dashboard.set_status("一時的なお知らせ", false);
        assert_eq!(
            sse_fallback_status(dashboard.status()),
            Some(SSE_FALLBACK_NOTICE)
        );
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
    fn document_title_is_fixed_and_never_exposes_dom_proof_fields() {
        // タブのタイトルはデータ源だけを示す固定文言。DOM 証跡の内部
        // フィールドやユーザー入力（filter）は一切含まれないこと。
        let demo = document_title(false);
        let live = document_title(true);
        assert_eq!(demo, "GPUI タスクマネージャー (Web) — デモ");
        assert_eq!(live, "GPUI タスクマネージャー (Web) — ライブ");
        assert_ne!(demo, live);
        for title in [demo, live] {
            assert!(title.contains("GPUI タスクマネージャー"), "{title}");
            assert!(title.contains("(Web)"), "{title}");
            // key=value; 形式の証跡フィールドはタイトルに出てこない。
            for field in [
                "mode=",
                "tick=",
                "cpu=",
                "mem=",
                "rows=",
                "total=",
                "filter=",
                "selected_pid=",
                "sort=",
                "dir=",
                "sampled=",
                "vw=",
                "vh=",
                "live_error=",
                "stale=",
                "ime_",
                "theme=",
                "interval=",
                "export=",
                "reconnect=",
            ] {
                assert!(!title.contains(field), "{field} leaked into {title}");
            }
            // 区切り文字・改行・トークン・入力値の可能性のある文字も出ない。
            assert!(!title.contains(';'), "{title}");
            assert!(!title.contains('='), "{title}");
            assert!(!title.contains('\n'), "{title}");
            assert!(!title.contains("super-secret-agent-token"), "{title}");
            assert!(!title.contains("firefox-filter-input"), "{title}");
        }
        assert!(live.contains("ライブ"), "{live}");
        assert!(demo.contains("デモ"), "{demo}");
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

    // --------------------------------------------------------- R1-I16 / I19

    #[test]
    fn proof_with_menu_keeps_legacy_hook_bytes() {
        let legacy = "mode=demo;tick=7;rows=3;theme=dark;";
        for (menu, value) in [
            (MenuProof::None, "none"),
            (MenuProof::Open, "open"),
            (MenuProof::Select, "select"),
            (MenuProof::CopyPid, "copy_pid"),
        ] {
            let proof = proof_with_menu(legacy, menu);
            assert_eq!(proof, format!("{legacy}menu={value};"));
            assert!(proof.starts_with(legacy));
            assert_eq!(proof.matches("menu=").count(), 1);
        }
    }

    #[test]
    fn row_menu_status_is_redaction_and_dom_safe() {
        let status = row_menu_pid_status(4242);
        assert!(status.contains("4242"));
        // Numeric PID only: no path, separator, token or agent-supplied text.
        assert!(!status.contains(';'));
        assert!(!status.contains('\n'));
        assert!(!status.contains('/'));
        assert!(!status.contains("token"));
    }

    #[test]
    fn row_menu_key_validation_rejects_reused_pids_and_missing_processes() {
        let snapshot = export_sample_snapshot();
        let live = tm_core::ProcessKey::new(4242, 1_700_000_000);
        let reused = tm_core::ProcessKey::new(4242, 1_700_000_001);
        let gone = tm_core::ProcessKey::new(9_999, 1_700_000_000);
        assert_eq!(live.pid, reused.pid, "PID 再利用は開始時刻で見分ける");

        assert!(row_menu_key_is_live(Some(&snapshot), live));
        assert!(!row_menu_key_is_live(Some(&snapshot), reused));
        assert!(!row_menu_key_is_live(Some(&snapshot), gone));
        assert!(!row_menu_key_is_live(None, live));

        // ingest 後のメニュー閉鎖判定。
        assert!(!row_menu_is_stale(Some(&snapshot), None));
        assert!(!row_menu_is_stale(
            Some(&snapshot),
            Some(RowMenu {
                key: live,
                index: 0
            })
        ));
        assert!(row_menu_is_stale(
            Some(&snapshot),
            Some(RowMenu {
                key: reused,
                index: 0
            })
        ));
        assert!(row_menu_is_stale(
            None,
            Some(RowMenu {
                key: live,
                index: 0
            })
        ));

        // CopyPid はクリック時点のキーを再検証し、古い PID を出さない。
        assert_eq!(row_menu_copy_pid(Some(&snapshot), live), Some(4242));
        assert_eq!(row_menu_copy_pid(Some(&snapshot), reused), None);
        assert_eq!(row_menu_copy_pid(None, live), None);

        // 固定通知に PID / 区切り文字 / 改行は出ない。
        assert!(!STALE_ROW_MENU_NOTICE.contains("4242"));
        assert!(!STALE_ROW_MENU_NOTICE.contains(';'));
        assert!(!STALE_ROW_MENU_NOTICE.contains('\n'));
    }

    #[test]
    fn onboarding_checklist_is_fixed_and_hook_safe() {
        let steps = onboarding_steps();
        assert_eq!(steps.len(), 4);
        for step in steps {
            assert!(!step.contains(';'), "{step}");
            assert!(!step.contains('\n'), "{step}");
            assert!(!step.contains("secret"), "{step}");
        }
    }

    // ------------------------------------------------- R1-I06 / R1-I08 配線

    #[test]
    fn filter_preset_chip_row_keeps_cdp_geometry_and_legacy_hook_bytes() {
        // プリセット行はツールバー中央の余白に追加するだけなので、CDP が
        // クリック座標を再計算する定数と検索欄の幅は一切変わらない。
        assert_eq!(TOOLBAR_HEIGHT, 36.0);
        assert_eq!(ROOT_PAD, 12.0);
        assert_eq!(OVERVIEW_HEIGHT, 260.0);
        assert_eq!(FILTER_BOX_WIDTH, 340.0);
        assert_eq!(DETAILS_WIDTH, 320.0);
        assert_eq!(FILTER_PRESETS_GROUP_ID, "filter-presets");
        assert_eq!(PROCESS_CPU_SPARKLINE_ID, "process-cpu-sparkline");
        assert_ne!(PROCESS_CPU_SPARKLINE_ID, "cpu-sparkline");

        // 既存フックの key=value; はバイト単位で不変（末尾追加のみ）。
        let legacy = "mode=demo;tick=7;rows=3;theme=dark;";
        let proof = proof_with_preset(&proof_with_menu(legacy, MenuProof::None), None);
        assert!(proof.starts_with(legacy), "{proof}");
        assert_eq!(
            proof,
            "mode=demo;tick=7;rows=3;theme=dark;menu=none;preset=none;"
        );
        assert_eq!(A11Y_LIVE_DOM_ID, "tm-web-a11y-live");
    }

    #[test]
    fn filter_preset_wiring_covers_every_builtin_and_clears_the_text_filter() {
        use crate::widgets::{filter_preset_a11y_label, filter_preset_text};

        assert_eq!(FILTER_PRESETS.len(), 5);
        let mut ids = std::collections::HashSet::new();
        for preset in FILTER_PRESETS {
            assert!(
                ids.insert(preset.id),
                "duplicate preset id: {:?}",
                preset.id
            );
            assert_eq!(preset.label, preset.id.label());
            // 全プリセットの filter.text が空なので、チップ押下時に IME ミラーの
            // 表示テキストもクリアして不一致を作らない（apply_filter_preset）。
            assert!(preset.filter.text.is_empty(), "{:?}", preset.id);
            // active は色ではなく ✓ とアクセシブルなラベルで示す。
            let active = filter_preset_text(preset, true);
            assert!(active.contains('✓'), "{active}");
            assert!(active.contains(preset.label), "{active}");
            assert_eq!(filter_preset_text(preset, false), preset.label);
            assert!(
                filter_preset_a11y_label(preset, true).contains("適用中"),
                "{preset:?}"
            );
        }

        let mut dashboard = Dashboard::new(PlatformInfo::demo(), DEFAULT_REFRESH_INTERVAL);
        for preset in FILTER_PRESETS {
            let effects = dashboard.apply(Command::ApplyFilterPreset(preset.id));
            assert!(effects.is_empty(), "preset command must be effect-free");
            assert_eq!(dashboard.active_filter_preset(), Some(preset.id));
            assert_eq!(dashboard.filter_text(), "");
        }
        // 直接の文字入力は active 印を消す（チップの ✓ が残らない）。
        dashboard.apply(Command::SetFilterText("firefox".to_string()));
        assert_eq!(dashboard.active_filter_preset(), None);
    }

    #[test]
    fn selected_cpu_sparkline_uses_dashboard_series_and_fixed_id() {
        use crate::widgets::process_sparkline_a11y_label;

        let mut dashboard = Dashboard::new(PlatformInfo::demo(), DEFAULT_REFRESH_INTERVAL);
        let mut probe = DemoProbe::new();
        let first = probe.sample().expect("demo snapshot");
        let key = first.processes[0].key;
        let name = first.processes[0].name.clone();
        dashboard.ingest(first);
        dashboard.apply(Command::Select(Some(key)));
        assert!(
            dashboard.selected_cpu_values().is_empty(),
            "選択変更で時系列はリセットされる"
        );
        dashboard.ingest(probe.sample().expect("demo snapshot"));
        let values = dashboard.selected_cpu_values();
        assert_eq!(values.len(), 1);
        // 詳細パネルに渡すラベルにプロセス名・最新値・サンプル数が載ること。
        let label = process_sparkline_a11y_label(&name, &values);
        assert!(label.starts_with(&name), "{label}");
        assert!(
            label.contains(&process_sparkline_value_text(&values)),
            "{label}"
        );
        assert!(label.contains("1 サンプル"), "{label}");
    }

    #[test]
    fn dom_proof_preset_field_is_additive_and_token_free() {
        let token = "super-secret-agent-token";
        let legacy =
            "mode=live;tick=5;rows=3;theme=dark;interval=2000;export=none;reconnect=0;menu=none;";
        for (preset, value) in [
            (None, "none"),
            (Some(FilterPresetId::All), "all"),
            (Some(FilterPresetId::HighCpu), "high_cpu"),
            (Some(FilterPresetId::HighMemory), "high_memory"),
            (
                Some(FilterPresetId::LargeVirtualMemory),
                "large_virtual_memory",
            ),
            (Some(FilterPresetId::Running), "running"),
        ] {
            let proof = proof_with_preset(legacy, preset);
            assert!(proof.starts_with(legacy), "{proof}");
            assert_eq!(proof, format!("{legacy}preset={value};"));
            assert!(!proof.contains(token), "{proof}");
            // 固定 ASCII 識別子のみ（日本語ラベルやユーザー入力は鏡像しない）。
            assert!(!proof.contains(' '), "{proof}");
            assert_eq!(proof.matches("preset=").count(), 1);
        }
    }

    // ------------------------------------------ R1-I10 / R1-I11 / R1-I12 配線

    #[test]
    fn dom_proof_sampling_fields_are_additive_and_token_free() {
        use tm_core::demo::demo_snapshot;

        let token = "super-secret-agent-token";
        let legacy = "mode=demo;tick=7;interval=2000;preset=none;";
        let snapshot = demo_snapshot(0);
        let proof = proof_with_sampling(legacy, Some(&snapshot), 2_500);
        assert!(proof.starts_with(legacy), "{proof}");
        assert_eq!(
            proof,
            "mode=demo;tick=7;interval=2000;preset=none;\
             power=ac;sampling=interactive;sampling_ms=2500;cgroups=2;"
        );
        assert_eq!(
            proof_with_sampling(legacy, None, 1_000),
            format!("{legacy}power=none;sampling=none;sampling_ms=1000;cgroups=0;")
        );
        assert!(!proof.contains(token), "{proof}");
        assert!(!proof.contains(' '), "{proof}");
        assert_eq!(proof.matches("power=").count(), 1);
        assert_eq!(proof.matches("sampling_ms=").count(), 1);
    }

    #[test]
    fn legacy_wire_snapshot_without_new_fields_still_deserialises() {
        // 既存エージェントが power/sampling/cgroups を送らない場合でも、
        // serde(default) で安全に読め、サンプリングを遅くしないこと。
        let original = tm_core::demo::demo_snapshot(0);
        let mut value = serde_json::to_value(&original).expect("serialise");
        let object = value.as_object_mut().expect("snapshot object");
        for field in ["power", "sampling", "cgroups"] {
            assert!(object.remove(field).is_some(), "fixture had {field}");
        }
        let restored: SystemSnapshot = serde_json::from_value(value).expect("legacy wire");
        assert_eq!(restored.power.source.id(), "unknown");
        assert!(restored.power.batteries.is_empty());
        assert!(restored.cgroups.is_empty());
        assert_eq!(restored.sampling.profile.id(), "interactive");
        assert_eq!(restored.processes.len(), original.processes.len());
        assert_eq!(restored.hostname, original.hostname);

        let mut dashboard = Dashboard::new(PlatformInfo::demo(), DEFAULT_REFRESH_INTERVAL);
        dashboard.ingest(restored);
        assert_eq!(dashboard.effective_interval(), DEFAULT_REFRESH_INTERVAL);
    }

    #[test]
    fn cgroup_grouping_is_displayed_from_the_dashboard_snapshot() {
        let mut dashboard = Dashboard::new(PlatformInfo::demo(), DEFAULT_REFRESH_INTERVAL);
        assert!(dashboard.cgroup_groups().is_empty());
        dashboard.ingest(tm_core::demo::demo_snapshot(0));
        let groups = dashboard.cgroup_groups();
        assert_eq!(groups.len(), 2);
        for group in groups {
            let row = cgroup_group_text(&group.label, group.members.len());
            assert!(row.contains(&group.label), "{row}");
            assert!(
                row.contains(&format!("{} 件", group.members.len())),
                "{row}"
            );
            assert!(group.label.chars().all(|ch| !ch.is_control()));
        }
        assert_eq!(
            cgroup_group_text("demo.slice/system.service", 3),
            "demo.slice/system.service — 3 件"
        );
    }

    // ---------------------------------------------------------------- T-SSE

    #[test]
    fn dom_proof_sse_field_is_appended_last_and_token_free() {
        let token = "super-secret-agent-token";
        let legacy = "mode=live;tick=7;interval=2000;cgroups=2;";
        let active = proof_with_sse(legacy, StreamMode::Active);
        assert!(active.starts_with(legacy), "{active}");
        assert_eq!(active, format!("{legacy}sse=active;"));
        let fallback = proof_with_sse(legacy, StreamMode::Fallback);
        assert_eq!(fallback, format!("{legacy}sse=fallback;"));
        assert_eq!(StreamMode::Active.proof_value(), "active");
        assert_eq!(StreamMode::Fallback.proof_value(), "fallback");
        for proof in [&active, &fallback] {
            assert!(!proof.contains(token), "{proof}");
            assert!(!proof.contains(' '), "{proof}");
            assert!(!proof.contains('\n'), "{proof}");
            assert_eq!(proof.matches("sse=").count(), 1, "{proof}");
        }
    }

    #[test]
    fn sse_endpoint_error_keeps_a_just_delivered_snapshot_fresh() {
        // `handle_live_result(Ok(frame))` just ran: with a snapshot already on
        // screen the poll helper reports fresh, i.e. stale=0.
        let stale_after_frame = stale_after_poll(true, true);
        assert!(!stale_after_frame);
        // The poll helper would have set stale=1 for the same failure. That is
        // exactly the call the SSE endpoint path must not make (X2-STALE).
        assert!(stale_after_poll(true, false));
        for error in [
            FetchError::BodyRead,
            FetchError::Parse("SSE データの解析に失敗しました".to_string()),
        ] {
            let (live_error, reconnect_required, status) = live_error_fields(None, false, &error);
            assert!(live_error.is_some(), "{error:?}");
            assert!(!reconnect_required, "{error:?}");
            assert!(
                status.contains("エージェントから取得できません"),
                "{status}"
            );
            // These are the fields the *poll* would write for the same error;
            // the SSE path now writes none of them (WB-SSEERR2), and never
            // touches `stale` in particular.
            assert!(
                !stale_after_frame,
                "{error:?}: stream snapshot stayed fresh"
            );
        }
    }

    #[test]
    fn sse_endpoint_error_keeps_the_401_403_reconnect_contract() {
        // 401 on the *stream* endpoint raises the token re-entry flag in the
        // same callback (SA-SSE401), publishes the guidance, and still leaves
        // the poll-owned `live_error` at 0. The proof is the real one produced
        // by `sync_proof_dom`'s builder, not a hand-concatenated literal.
        let (reconnect, live_error, status, proof) = sse_end_state(Err(FetchError::Auth));
        assert!(reconnect, "SSE 401 must set reconnect=1 immediately");
        assert!(live_error.is_none(), "SSE must not write live_error");
        let status = status.expect("401 must publish a status");
        assert!(
            status.contains(crate::agent::RECONNECT_GUIDANCE),
            "{status}"
        );
        assert!(status.contains("#token="), "{status}");
        assert!(proof.contains("live_error=0;"), "{proof}");
        assert!(proof.contains("reconnect=1;"), "{proof}");
        assert!(proof.ends_with("sse=fallback;"), "{proof}");

        // 403 is an origin problem: neither the poll nor the stream may invent
        // an auth failure for it.
        let forbidden = FetchError::Forbidden;
        assert_eq!(forbidden.to_string(), crate::agent::FORBIDDEN_GUIDANCE);
        assert!(!crate::agent::requires_reconnect(&forbidden));
        let (reconnect, live_error, _status, proof) = sse_end_state(Err(forbidden));
        assert!(!reconnect, "403 must not set reconnect=1");
        assert!(live_error.is_none(), "SSE must not write live_error");
        assert!(proof.contains("live_error=0;"), "{proof}");
        assert!(proof.contains("reconnect=0;"), "{proof}");
        assert!(!proof.contains("#token="), "{proof}");

        // The flag is sticky: a following non-auth stream failure cannot lower
        // it again (R49 audit), even when both observations came from SSE.
        let (reconnect, live_error, _status, proof) = run_sse_steps(|app| {
            app.on_sse_ended(Err(FetchError::Auth));
            app.on_sse_ended(Err(FetchError::Forbidden));
        });
        assert!(reconnect, "a sticky 401 flag survives a following 403");
        assert!(live_error.is_none(), "SSE must not write live_error");
        assert!(proof.contains("reconnect=1;"), "{proof}");
    }

    #[test]
    fn sse_error_only_emits_a_transient_status_not_live_error() {
        // wa-netfail / WB-SSEERR2: a stream endpoint failing with 503 must not
        // blink the DOM proof's `live_error=` marker while the poll is healthy.
        // The effect carries no `live_error` field at all, and the real state
        // observation confirms the marker stays 0 on the production path.
        let effect = sse_error_effect(None, &FetchError::Http(503));
        assert_eq!(
            effect,
            SseErrorEffect {
                transient_status: Some(SSE_FALLBACK_NOTICE),
                reconnect_required: false,
            }
        );
        assert!(!SSE_FALLBACK_NOTICE.contains(';'));
        assert!(!SSE_FALLBACK_NOTICE.contains('\n'));

        let (reconnect, live_error, status, proof) = sse_end_state(Err(FetchError::Http(503)));
        assert!(!reconnect, "503 must not set reconnect=1");
        assert!(live_error.is_none(), "SSE must not write live_error");
        let status = status.expect("503 must publish the fallback notice");
        assert!(
            status.contains(SSE_FALLBACK_NOTICE),
            "unexpected status: {status}"
        );
        assert!(proof.contains("live_error=0;"), "{proof}");
        assert!(proof.contains("reconnect=0;"), "{proof}");
        assert!(proof.ends_with("sse=fallback;"), "{proof}");
    }

    #[test]
    fn poll_5xx_still_sets_the_live_error_marker() {
        // The poll is the sole owner of the DOM proof's `live_error=` marker:
        // a real snapshot-fetch failure must still raise it (WB-SSEERR2).
        for code in [500u16, 502, 503, 504] {
            let error = FetchError::Http(code);
            let (live_error, reconnect_required, status) = live_error_fields(None, false, &error);
            assert!(live_error.is_some(), "{error:?}");
            assert!(
                !reconnect_required,
                "5xx must not set reconnect=1: {error:?}"
            );
            assert!(
                status.contains("エージェントから取得できません"),
                "{status}"
            );
        }
    }

    #[test]
    fn sse_error_never_overwrites_a_persistent_notice() {
        // SECURITY.md §5: a corrupt / save-failure notice wins over the
        // informational fallback emitted for an SSE error (non-regression).
        let persistent = StatusMessage {
            text: "設定ファイルが破損しています".to_string(),
            is_error: true,
            persistent: true,
        };
        assert_eq!(
            sse_error_effect(Some(&persistent), &FetchError::BodyRead),
            SseErrorEffect {
                transient_status: None,
                reconnect_required: false,
            }
        );
        // A transient status, in contrast, may be replaced by the notice.
        let transient = StatusMessage {
            text: "接続設定を再読込しました".to_string(),
            is_error: false,
            persistent: false,
        };
        assert_eq!(
            sse_error_effect(Some(&transient), &FetchError::BodyRead).transient_status,
            Some(SSE_FALLBACK_NOTICE)
        );

        // Real path: the guard still holds after `on_sse_ended` for a non-auth
        // stream failure — the persistent corrupt notice stays on screen and
        // neither `reconnect=` nor `live_error=` moves.
        let (reconnect, live_error, status, proof) = run_sse_steps(|app| {
            app.dashboard
                .set_persistent_status("設定ファイルが破損しています");
            app.on_sse_ended(Err(FetchError::BodyRead));
        });
        assert!(!reconnect, "BodyRead must not set reconnect=1");
        assert!(live_error.is_none(), "SSE must not write live_error");
        assert!(
            status
                .as_deref()
                .is_some_and(|text| text.contains("設定ファイルが破損しています")),
            "persistent notice must win: {status:?}"
        );
        assert!(proof.contains("reconnect=0;"), "{proof}");
        assert!(proof.contains("live_error=0;"), "{proof}");
    }

    #[test]
    fn dom_proof_sse_field_flips_to_fallback_when_the_stream_ends() {
        let base = "mode=live;stale=0;tick=9;";
        let active = proof_with_sse(base, StreamMode::Active);
        assert_eq!(active, format!("{base}sse=active;"));
        // `on_sse_ended` rewrites the mode and calls `sync_proof_dom` before it
        // returns, so the proof changes in the same callback as the error.
        let mode = stream_mode_after_sse_end(&Err(FetchError::BodyRead));
        assert_eq!(mode, StreamMode::Fallback);
        let fallback = proof_with_sse(base, mode);
        assert_eq!(fallback, format!("{base}sse=fallback;"));
        assert_ne!(active, fallback);
        assert_eq!(active.replace("sse=active;", "sse=fallback;"), fallback);
        // Only the transport marker changed: stale stays 0 across the flip.
        assert!(active.contains("stale=0;") && fallback.contains("stale=0;"));
        assert!(!active.contains("stale=1;") && !fallback.contains("stale=1;"));
    }

    #[test]
    fn every_sse_termination_falls_back_to_the_existing_poll() {
        // A clean EOF and every typed error must return to polling so the user
        // is never left staring at a frozen snapshot (R3-S01).
        assert_eq!(stream_mode_after_sse_end(&Ok(())), StreamMode::Fallback);
        for error in [
            FetchError::Auth,
            FetchError::Forbidden,
            FetchError::Http(503),
            FetchError::Timeout,
            FetchError::Busy,
            FetchError::BodyAborted,
            FetchError::SizeLimit,
            FetchError::BodyRead,
            FetchError::Parse("SSE データの解析に失敗しました".to_string()),
            FetchError::Build("bad uri".to_string()),
            FetchError::Connect,
            FetchError::InvalidEndpoint,
            FetchError::SchemaVersion {
                agent: 99,
                web: tm_core::WIRE_SCHEMA_VERSION,
            },
        ] {
            let debug = format!("{error:?}");
            assert_eq!(
                stream_mode_after_sse_end(&Err(error)),
                StreamMode::Fallback,
                "{debug}"
            );
        }
        // The fallback notice stays inside the DOM-proof grammar.
        assert!(!SSE_FALLBACK_NOTICE.contains(';'));
        assert!(!SSE_FALLBACK_NOTICE.contains('\n'));
    }

    #[test]
    fn polling_stays_on_until_the_first_stream_frame_arrives() {
        // The transport is not *proven* until a frame arrives, so the mode
        // stays Fallback (sse=fallback) throughout Connecting. Request-level
        // suppression during that window is handled by the poll guard below;
        // this test keeps the DOM-proof contract unchanged.
        assert_eq!(stream_mode_while_connecting(), StreamMode::Fallback);
        assert_eq!(
            proof_with_sse("mode=live;", stream_mode_while_connecting()),
            "mode=live;sse=fallback;"
        );
    }

    #[test]
    fn sse_connecting_suspends_polls_and_skips_the_sample_silently() {
        let (state, (polled_none, ingested, before, after)) = run_live_sse_steps(|app, client| {
            // Exactly the line `start_sse_stream` executes on subscribe.
            app.sse_poll_guard = Some(client.suspend_polls_while_connecting());
            let before = app.proof_string(0, 0);
            // The guarded fetch returns before any await, so no `http.send`
            // happens and no timer is needed while the guard is alive.
            let polled = futures::executor::block_on(client.fetch_snapshot_guarded());
            let polled_none = matches!(polled, Ok(None));
            let ingested = app.ingest_guarded_poll(polled);
            (polled_none, ingested, before, app.proof_string(0, 0))
        });
        assert!(polled_none, "a Connecting poll must not issue a request");
        assert!(!ingested, "Ok(None) must be a silent skip");
        assert_eq!(before, after, "a skipped tick must not rewrite the proof");
        assert!(state.guard_held && state.polls_suspended);
        assert!(!state.active, "no frame means no Active transport");
        assert_eq!(state.ticks, 0);
        assert!(state.live_error.is_none());
    }

    #[test]
    fn sse_first_frame_confirms_and_resumes_polling() {
        let (state, (before, resumed)) = run_live_sse_steps(|app, client| {
            app.sse_poll_guard = Some(client.suspend_polls_while_connecting());
            let before = client.poll_generation().current();
            // The callback order: confirm + release, then Active, then ingest.
            app.confirm_sse_and_resume_polls();
            let resumed = !client.polls_suspended() && app.sse_poll_guard.is_none();
            app.stream_mode = StreamMode::Active;
            app.ingest_guarded_poll(Ok(Some(SnapshotResponse {
                snapshot: tm_core::demo::demo_snapshot(0),
                total_process_count: 3,
            })));
            (before, resumed)
        });
        assert_ne!(
            state.generation, before,
            "confirm must invalidate an in-flight poll"
        );
        assert!(resumed, "the first frame must release the suspension");
        assert!(!state.guard_held && !state.polls_suspended);
        assert!(state.active);
        assert_eq!(state.ticks, 1, "a real frame is ingested exactly once");
        assert!(
            state.proof.contains("sse=active;"),
            "{proof}",
            proof = state.proof
        );
        assert!(state.live_error.is_none());
    }

    #[test]
    fn sse_error_and_idle_timeout_release_the_guard() {
        let (ended, ()) = run_live_sse_steps(|app, client| {
            // A stream that never delivered still owns the startup guard.
            app.sse_poll_guard = Some(client.suspend_polls_while_connecting());
            app.on_sse_ended(Err(FetchError::BodyRead));
        });
        assert!(
            !ended.guard_held && !ended.polls_suspended,
            "error must resume polls"
        );
        assert!(ended.proof.contains("sse=fallback;"));
        assert!(ended.live_error.is_none(), "WB-SSEERR2");

        let (idle, ()) = run_live_sse_steps(|app, client| {
            // Defensive path: an Active stream that goes idle must not leave a
            // guard behind either.
            app.sse_poll_guard = Some(client.suspend_polls_while_connecting());
            app.stream_mode = StreamMode::Active;
            app.on_sse_idle_timeout();
        });
        assert!(
            !idle.guard_held && !idle.polls_suspended,
            "idle reset must resume polls"
        );
        assert!(idle.proof.contains("sse=fallback;"));
    }

    #[test]
    fn sse_error_with_a_held_guard_keeps_notification_priority() {
        // SECURITY.md §5 / WB-SSEERR2 non-regression with the new guard in
        // place: a persistent save/corrupt notice still wins over the SSE
        // fallback, and neither `reconnect=` nor `live_error=` moves.
        let persistent = StatusMessage {
            text: "設定ファイルが破損しています".to_string(),
            is_error: true,
            persistent: true,
        };
        assert_eq!(
            sse_error_effect(Some(&persistent), &FetchError::BodyRead),
            SseErrorEffect {
                transient_status: None,
                reconnect_required: false,
            }
        );
        let (state, ()) = run_live_sse_steps(|app, client| {
            app.dashboard
                .set_persistent_status("設定ファイルが破損しています");
            app.sse_poll_guard = Some(client.suspend_polls_while_connecting());
            app.on_sse_ended(Err(FetchError::BodyRead));
        });
        assert!(!state.guard_held && !state.polls_suspended);
        assert!(
            state
                .status
                .as_deref()
                .is_some_and(|text| text.contains("設定ファイルが破損しています")),
            "persistent notice must win: {status:?}",
            status = state.status
        );
        assert!(!state.reconnect);
        assert!(state.proof.contains("reconnect=0;"));
        assert!(state.proof.contains("live_error=0;"));
    }

    #[test]
    fn sse_idle_timeout_is_three_times_the_applied_interval_with_a_fixed_floor() {
        // At the default cadence the three-times rule and the floor coincide.
        assert_eq!(
            sse_idle_timeout(DEFAULT_REFRESH_INTERVAL),
            DEFAULT_REFRESH_INTERVAL.saturating_mul(3)
        );
        // Clamp before multiplying, then raise the sub-floor result to 3 s.
        assert_eq!(
            sse_idle_timeout(Duration::from_millis(1)),
            MIN_SSE_IDLE_TIMEOUT
        );
        assert_eq!(
            sse_idle_timeout(Duration::from_secs(3_600)),
            MAX_REFRESH_INTERVAL.saturating_mul(3)
        );
    }

    #[test]
    fn sse_idle_timeout_floor_clamps_sub_three_second_windows() {
        // Every computation below the floor resolves to MIN_SSE_IDLE_TIMEOUT:
        // the sampler's 250 ms lower clamp can never shrink the watchdog.
        for interval_ms in [1, 250, 500, 999] {
            assert_eq!(
                sse_idle_timeout(Duration::from_millis(interval_ms)),
                MIN_SSE_IDLE_TIMEOUT
            );
        }
        // Above the floor the three-times rule still governs.
        assert_eq!(
            sse_idle_timeout(Duration::from_secs(4)),
            Duration::from_secs(12)
        );
    }

    #[test]
    fn sse_idle_watchdog_tolerates_one_second_frames_at_short_intervals() {
        // The agent streams a snapshot every STREAM_INTERVAL (1 s). Whatever the
        // sampler interval is, a 1 s gap must never look like an idle stream,
        // which is the misclassification the fixed floor prevents.
        for interval_ms in [250, 500, 1_000] {
            let interval = Duration::from_millis(interval_ms);
            let mut last_frame = Instant::now();
            for _ in 0..4 {
                assert!(!sse_idle_expired(
                    StreamMode::Active,
                    Some(last_frame),
                    last_frame + Duration::from_secs(1),
                    interval
                ));
                last_frame += Duration::from_secs(1);
            }
        }
    }

    #[test]
    fn sse_idle_watchdog_never_fires_when_the_clock_goes_backwards() {
        // A clock observed out of order (now < last_snapshot) must not be
        // treated as an elapsed idle period (w3-tests finding).
        let now = Instant::now();
        let future_snapshot = now + Duration::from_secs(5);
        for interval_ms in [250, 1_000] {
            assert!(!sse_idle_expired(
                StreamMode::Active,
                Some(future_snapshot),
                now,
                Duration::from_millis(interval_ms)
            ));
        }
    }

    #[test]
    fn sse_idle_watchdog_only_fires_for_an_active_silent_stream() {
        let interval = DEFAULT_REFRESH_INTERVAL;
        let timeout = sse_idle_timeout(interval);
        let first_frame = Instant::now();

        // Before the first frame there is nothing to watch (and polling is on).
        assert!(!sse_idle_expired(
            StreamMode::Fallback,
            None,
            first_frame + timeout * 4,
            interval
        ));
        assert!(!sse_idle_expired(
            StreamMode::Active,
            None,
            first_frame,
            interval
        ));
        // Inside the grace window polling stays suspended.
        assert!(!sse_idle_expired(
            StreamMode::Active,
            Some(first_frame),
            first_frame + timeout - Duration::from_millis(1),
            interval
        ));
        // At and beyond the deadline the poll must take over again.
        assert!(sse_idle_expired(
            StreamMode::Active,
            Some(first_frame),
            first_frame + timeout,
            interval
        ));
        assert!(sse_idle_expired(
            StreamMode::Active,
            Some(first_frame),
            first_frame + timeout * 2,
            interval
        ));
    }

    #[test]
    fn connecting_watchdog_releases_a_poll_suspended_by_only_error_frames() {
        let interval = DEFAULT_REFRESH_INTERVAL;
        let timeout = sse_idle_timeout(interval);
        let started = Instant::now();

        // No guard: the poll is free, so no watchdog needed even if a stream
        // attempt once started long ago.
        assert!(!connecting_watchdog_expired(
            false,
            None,
            Some(started),
            started + timeout * 2,
            interval
        ));
        // A genuine first frame clears the connecting baseline; the normal
        // Active watchdog takes over from there.
        assert!(!connecting_watchdog_expired(
            true,
            Some(started),
            None,
            started + timeout * 2,
            interval
        ));
        // Regression: a stream that emits only `event: error` never produces a
        // snapshot, keeps the guard, and previously suspended polling forever.
        assert!(!connecting_watchdog_expired(
            true,
            None,
            Some(started),
            started + timeout - Duration::from_millis(1),
            interval
        ));
        assert!(connecting_watchdog_expired(
            true,
            None,
            Some(started),
            started + timeout,
            interval
        ));
        assert!(connecting_watchdog_expired(
            true,
            None,
            Some(started),
            started + timeout * 2,
            interval
        ));
        // Once the guard is released (idle timeout ran), no repeated firing.
        assert!(!connecting_watchdog_expired(
            false,
            None,
            None,
            started + timeout * 3,
            interval
        ));
    }

    #[test]
    fn stale_stream_frames_are_rejected_by_the_epoch_guard() {
        let shell_epoch = 7;
        assert!(sse_frame_is_current(shell_epoch, shell_epoch));
        // Both an older and a newer (defensive) epoch count as a stale stream.
        assert!(!sse_frame_is_current(shell_epoch - 1, shell_epoch));
        assert!(!sse_frame_is_current(shell_epoch + 1, shell_epoch));
    }

    // ------------------------------------------------------ V3-KEYS 配線

    fn ctrl() -> Modifiers {
        Modifiers {
            control: true,
            ..Modifiers::default()
        }
    }

    fn cmd() -> Modifiers {
        Modifiers {
            platform: true,
            ..Modifiers::default()
        }
    }

    /// デモスナップショットを取り込み、先頭 2 行の ProcessKey を返す。
    fn dashboard_with_two_visible_rows() -> (Dashboard, tm_core::ProcessKey, tm_core::ProcessKey) {
        let mut dashboard = Dashboard::new(PlatformInfo::demo(), DEFAULT_REFRESH_INTERVAL);
        let snapshot = DemoProbe::new().sample().expect("demo snapshot");
        assert!(snapshot.processes.len() >= 2, "demo fixture is too small");
        dashboard.ingest(snapshot);
        let rows: Vec<_> = dashboard.visible_rows().collect();
        assert!(rows.len() >= 2, "dashboard must show at least two rows");
        let snapshot = dashboard.snapshot().expect("ingested snapshot");
        let first = snapshot.processes[rows[0].index].key;
        let second = snapshot.processes[rows[1].index].key;
        (dashboard, first, second)
    }

    #[test]
    fn editing_home_never_changes_selected_pid() {
        let (mut dashboard, first, second) = dashboard_with_two_visible_rows();
        dashboard.apply(Command::Select(Some(second)));
        let before = dashboard.selected_key();
        assert_eq!(before, Some(second));
        assert_ne!(before, Some(first));

        // 編集中は resolver が None を返す → SelectRelative を送らない。
        // on_key_down は None のとき stop_propagation しないので、隠し
        // textarea のキャレット移動だけが起こる（native と同じ契約）。
        assert_eq!(web_key_action("home", Modifiers::default(), true), None);
        assert_eq!(dashboard.selected_key(), before);

        // 同じ Home が編集中でなければ先頭行へ動くことを確認し、
        // 「ガードが reason で選択が止まっている」ことを証明する。
        let action = web_key_action("home", Modifiers::default(), false);
        assert_eq!(action, Some(WebKeyAction::SelectRelative(-1_000_000)));
        if let Some(WebKeyAction::SelectRelative(delta)) = action {
            dashboard.apply(Command::SelectRelative(delta));
        }
        assert_eq!(dashboard.selected_key(), Some(first));
    }

    #[test]
    fn navigation_and_shortcuts_are_suppressed_while_editing() {
        // キャレット移動 / IME 候補選択と衝突するキーはすべて委譲する。
        for key in [
            "up", "down", "pageup", "pagedown", "home", "end", "space", "f1", "f2", "f5",
        ] {
            assert_eq!(
                web_key_action(key, Modifiers::default(), true),
                None,
                "{key} must not fire while the filter is being edited"
            );
        }
        for modifiers in [ctrl(), cmd()] {
            for key in ["f", "k", "home", "f2", "f5"] {
                assert_eq!(
                    web_key_action(key, modifiers, true),
                    None,
                    "modified shortcut must not fire while editing: {key}"
                );
            }
        }
        // 文字編集キーだけは編集中も解決する（残りはブラウザの IME へ）。
        assert_eq!(
            web_key_action("escape", Modifiers::default(), true),
            Some(WebKeyAction::CancelFilter)
        );
        assert_eq!(
            web_key_action("enter", Modifiers::default(), true),
            Some(WebKeyAction::CommitFilter)
        );
        assert_eq!(web_key_action("a", Modifiers::default(), true), None);
    }

    #[test]
    fn f2_f5_ctrl_k_are_reflected_in_dashboard_state_and_status() {
        let plain = Modifiers::default();

        // F2 → 詳細トグル。DOM proof の a11y/ステータス経由で観測可能な状態。
        assert_eq!(
            web_key_action("f2", plain, false),
            Some(WebKeyAction::ToggleDetails)
        );
        let (mut dashboard, _, _) = dashboard_with_two_visible_rows();
        let before = dashboard.details_visible();
        dashboard.apply(Command::ToggleDetails);
        assert_ne!(dashboard.details_visible(), before);

        // F5 → Tick 相当。C2-UXKEY (B): Web はサンプラーが周期を管理するので
        // 実サンプルは増やさず、「要求のみ」であることを固定 status に残す
        // （DOM の a11y ミラーが同じ文言を読み上げる）。tick= は実サンプルだけ。
        assert_eq!(
            web_key_action("f5", plain, false),
            Some(WebKeyAction::Refresh)
        );
        assert_eq!(dashboard.apply(Command::Tick), vec![Effect::SampleNow]);
        assert!(MANUAL_REFRESH_NOTICE.contains("更新"));
        assert!(MANUAL_REFRESH_NOTICE.contains("要求のみ"));
        assert!(!MANUAL_REFRESH_NOTICE.contains(';'));
        dashboard.set_status(MANUAL_REFRESH_NOTICE, false);
        assert!(
            dashboard
                .status()
                .is_some_and(|status| status.text.contains(MANUAL_REFRESH_NOTICE)),
            "F5 の通知が status に載ること"
        );

        // Ctrl+K → Effect::Signal と同じ読み取り専用通知。
        for modifiers in [ctrl(), cmd()] {
            assert_eq!(
                web_key_action("k", modifiers, false),
                Some(WebKeyAction::ReadOnlyNotice)
            );
        }
        assert_eq!(
            READONLY_SIGNAL_NOTICE,
            "ブラウザ版は読み取り専用です。プロセス操作（シグナル送信）は行いません。"
        );
        assert!(!READONLY_SIGNAL_NOTICE.contains(';'));
        assert!(!READONLY_SIGNAL_NOTICE.contains('\n'));
        dashboard.set_status(READONLY_SIGNAL_NOTICE, true);
        assert!(
            dashboard
                .status()
                .is_some_and(|status| status.text.contains(READONLY_SIGNAL_NOTICE)),
            "Ctrl+K の読み取り専用通知が status に載ること"
        );
        assert!(dashboard.status().is_some_and(|status| status.is_error));
    }

    #[test]
    fn row_menu_is_modal_for_f2_space_and_ctrl_k_until_esc_closes_it() {
        // 判断（YA-ROWMENU）: 行メニューは native の確認ダイアログと同等の
        // モーダル。表示中は F2 / Space / Ctrl+K を消費して無視し、Esc / Enter /
        // メニュー項目のキーボード操作は従来どおり通す。
        let plain = Modifiers::default();
        let (mut dashboard, _, _) = dashboard_with_two_visible_rows();
        let conflicting = [("f2", plain), ("space", plain), ("k", ctrl()), ("k", cmd())];

        // メニュー表示中は 3 種のショートカットがすべて Swallowed。
        // ガードが無ければこれらは詳細トグル / status 変更として観測される。
        for (key, modifiers) in conflicting {
            assert_eq!(
                resolve_web_key(key, modifiers, false, true),
                WebKeyDecision::Swallowed,
                "{key} must be ignored while the row menu is open"
            );
            assert!(matches!(
                web_key_action(key, modifiers, false),
                Some(
                    WebKeyAction::ToggleDetails
                        | WebKeyAction::TogglePaused
                        | WebKeyAction::ReadOnlyNotice
                )
            ));
        }

        // メニュー項目のキーボード操作（矢印移動）と Enter の既定停止は維持。
        // Esc は resolver より前の分岐が行メニューを閉じるため、閉じた後は
        // row_menu_open = false で解決される（下の F2 の確認）。
        assert_eq!(
            resolve_web_key("down", plain, false, true),
            WebKeyDecision::Run(WebKeyAction::SelectRelative(1))
        );
        assert_eq!(
            resolve_web_key("enter", plain, false, true),
            WebKeyDecision::Unhandled
        );

        // on_key_down と同じ判定で実際に detailed / status が変わらないことを固定。
        let apply = |dashboard: &mut Dashboard, action: WebKeyAction| match action {
            WebKeyAction::ToggleDetails => {
                dashboard.apply(Command::ToggleDetails);
            }
            WebKeyAction::TogglePaused => {
                dashboard.set_status(PAUSE_UNSUPPORTED_NOTICE, false);
            }
            WebKeyAction::ReadOnlyNotice => {
                dashboard.set_status(READONLY_SIGNAL_NOTICE, true);
            }
            _ => {}
        };
        let details_before = dashboard.details_visible();
        let status_before = dashboard.status().cloned();
        for (key, modifiers) in conflicting {
            if let WebKeyDecision::Run(action) = resolve_web_key(key, modifiers, false, true) {
                apply(&mut dashboard, action);
            }
        }
        assert_eq!(dashboard.details_visible(), details_before);
        assert_eq!(dashboard.status().cloned(), status_before);

        // Esc で閉じた後（row_menu = None）は F2 が従来どおり詳細を反転する。
        assert_eq!(
            resolve_web_key("f2", plain, false, false),
            WebKeyDecision::Run(WebKeyAction::ToggleDetails)
        );
        dashboard.apply(Command::ToggleDetails);
        assert_ne!(dashboard.details_visible(), details_before);
    }

    #[test]
    fn ctrl_f_and_plain_navigation_keep_native_parity() {
        for modifiers in [ctrl(), cmd()] {
            assert_eq!(
                web_key_action("f", modifiers, false),
                Some(WebKeyAction::BeginFilter)
            );
        }
        let expected = [
            ("up", WebKeyAction::SelectRelative(-1)),
            ("down", WebKeyAction::SelectRelative(1)),
            ("pageup", WebKeyAction::SelectRelative(-10)),
            ("pagedown", WebKeyAction::SelectRelative(10)),
            ("home", WebKeyAction::SelectRelative(-1_000_000)),
            ("end", WebKeyAction::SelectRelative(1_000_000)),
            ("space", WebKeyAction::TogglePaused),
            ("f2", WebKeyAction::ToggleDetails),
            ("f5", WebKeyAction::Refresh),
            ("f1", WebKeyAction::ShowHelp),
            ("escape", WebKeyAction::ClearFilter),
        ];
        for (key, action) in expected {
            assert_eq!(
                web_key_action(key, Modifiers::default(), false),
                Some(action),
                "{key}"
            );
        }
        // F1 の実際のトグルは V2 の純粋ヘルパーが担当（二重処理しない）。
        assert_eq!(help_overlay_key_state(false, "f1"), Some(true));
        assert_eq!(help_overlay_key_state(true, "f1"), Some(false));
        assert_eq!(help_overlay_key_state(true, "escape"), Some(false));
        assert_eq!(help_overlay_key_state(false, "escape"), None);
    }

    // ------------------------------------------------ hidden DOM mirror tests

    #[test]
    fn mirror_header_exposes_single_active_sort_column() {
        let asc = a11y_mirror_header_html(SortSpec {
            key: SortKey::Cpu,
            direction: SortDirection::Ascending,
        });
        assert_eq!(asc.matches("aria-sort=").count(), 1, "{asc}");
        assert!(asc.contains("aria-sort=\"ascending\""), "{asc}");
        assert!(asc.contains("CPU ↑"), "{asc}");
        assert!(asc.contains("現在 昇順"), "{asc}");
        assert_eq!(
            asc.matches("role=\"columnheader\"").count(),
            A11Y_MIRROR_COLUMNS.len(),
            "{asc}"
        );
        let desc = a11y_mirror_header_html(SortSpec {
            key: SortKey::Cpu,
            direction: SortDirection::Descending,
        });
        assert!(desc.contains("aria-sort=\"descending\""), "{desc}");
        assert!(desc.contains("現在 降順"), "{desc}");
        assert!(!asc.contains("aria-hidden"), "{asc}");
    }

    #[test]
    fn mirror_row_exposes_selection_and_escaped_display_values() {
        let selected = a11y_mirror_row_html(
            "<script>alert(1)</script>",
            ProcessStatus::Zombie,
            92.0,
            4_096,
            42,
            true,
            3,
        );
        assert!(selected.contains("role=\"row\""), "{selected}");
        assert_eq!(
            selected.matches("role=\"gridcell\"").count(),
            3,
            "{selected}"
        );
        assert!(!selected.contains("role=\"cell\""), "{selected}");
        assert!(selected.contains("aria-rowindex=\"3\""), "{selected}");
        assert!(selected.contains("aria-selected=\"true\""), "{selected}");
        assert!(selected.contains("aria-current=\"true\""), "{selected}");
        assert!(selected.contains("PID 42"), "{selected}");
        assert!(selected.contains("選択中"), "{selected}");
        assert!(selected.contains("高"), "{selected}");
        assert!(selected.contains("&lt;script&gt;"), "{selected}");
        assert!(!selected.contains("<script>"), "{selected}");
        let unselected = a11y_mirror_row_html("idle", ProcessStatus::Idle, 1.0, 0, 7, false, 1);
        assert!(
            unselected.contains("aria-selected=\"false\""),
            "{unselected}"
        );
        assert!(!unselected.contains("aria-current"), "{unselected}");
        assert!(!unselected.contains("選択中"), "{unselected}");
    }

    #[test]
    fn mirror_help_dialog_exposes_heading_shortcuts_and_close_button() {
        let open = a11y_mirror_help_html(true);
        assert!(open.contains("role=\"dialog\""), "{open}");
        assert!(open.contains("aria-modal=\"true\""), "{open}");
        assert!(open.contains(HELP_TITLE), "{open}");
        assert!(open.contains("role=\"heading\""), "{open}");
        assert!(open.contains("F1"), "{open}");
        assert!(open.contains("Esc"), "{open}");
        assert!(
            open.contains(&format!("id=\"{}\"", A11Y_HELP_CLOSE_DOM_ID)),
            "{open}"
        );
        assert!(open.contains("role=\"button\""), "{open}");
        assert!(open.contains("tabindex=\"-1\""), "{open}");
        assert!(open.contains("閉じる"), "{open}");
        assert!(!open.contains("aria-hidden"), "{open}");
        assert!(a11y_mirror_help_html(false).is_empty());
    }

    #[test]
    fn mirror_grid_contains_header_rows_and_help_in_one_subtree() {
        let snapshot = tm_core::demo::demo_snapshot(0);
        let visible: Vec<usize> = (0..snapshot.processes.len().min(3)).collect();
        let html = a11y_mirror_html(Some(&snapshot), &visible, None, SortSpec::default(), true);
        // ARIA 仕様上 aria-selected は grid/treegrid の row でのみ露出されるため、
        // ミラーは table/cell ではなく grid/gridcell を使う（columnheader は維持）。
        assert!(html.contains("role=\"grid\""), "{html}");
        assert!(!html.contains("role=\"table\""), "{html}");
        assert!(!html.contains("role=\"cell\""), "{html}");
        assert_eq!(
            html.matches("role=\"gridcell\"").count(),
            3 * visible.len(),
            "{html}"
        );
        assert_eq!(
            html.matches("role=\"columnheader\"").count(),
            A11Y_MIRROR_COLUMNS.len(),
            "{html}"
        );
        assert_eq!(html.matches("role=\"rowgroup\"").count(), 2, "{html}");
        assert!(html.contains("role=\"row\""), "{html}");
        assert!(html.contains("role=\"dialog\""), "{html}");
        assert_eq!(
            html.matches("aria-selected=").count(),
            visible.len(),
            "{html}"
        );
        assert!(!html.contains("aria-hidden"), "{html}");
        assert_ne!(A11Y_MIRROR_DOM_ID, A11Y_LIVE_DOM_ID);
        assert_ne!(A11Y_HELP_CLOSE_DOM_ID, A11Y_MIRROR_DOM_ID);
    }

    #[test]
    fn mirror_selected_row_exposes_ax_selected_property_contract() {
        // CDP の verify_ax_roles は名称フォールバックを廃し、AX property の
        // selected=true のみを厳格照合する。ミラー側がその property を生むことを
        // 純粋 HTML レベルで固定する（aria-current は冗長化）。
        let snapshot = tm_core::demo::demo_snapshot(0);
        let selected_key = snapshot.processes[0].key;
        let visible: Vec<usize> = (0..snapshot.processes.len().min(3)).collect();
        let html = a11y_mirror_html(
            Some(&snapshot),
            &visible,
            Some(selected_key),
            SortSpec::default(),
            false,
        );
        assert_eq!(html.matches("aria-selected=\"true\"").count(), 1, "{html}");
        assert_eq!(html.matches("aria-current=\"true\"").count(), 1, "{html}");
        assert_eq!(
            html.matches("aria-selected=\"false\"").count(),
            visible.len() - 1,
            "{html}"
        );
        // 選択行の role は grid 配下の row であり、cell は gridcell である。
        let selected_row = html
            .split("<div role=\"row\"")
            .find(|part| part.contains("aria-selected=\"true\""))
            .expect("選択行が存在する");
        assert!(
            selected_row.contains("aria-current=\"true\""),
            "{selected_row}"
        );
        assert!(selected_row.contains("role=\"gridcell\""), "{selected_row}");
        assert!(!selected_row.contains("role=\"cell\""), "{selected_row}");
        let expected_pid = format!("PID {}", selected_key.pid);
        assert!(selected_row.contains(&expected_pid), "{selected_row}");
    }

    #[test]
    fn mirror_metrics_spell_out_usage_and_temperature_states() {
        assert!(a11y_mirror_metrics_html(None).is_empty());
        let snapshot = tm_core::demo::demo_snapshot(0);
        let html = a11y_mirror_metrics_html(Some(&snapshot));
        let cpu = usage_value_text(snapshot.cpu.total_percent);
        let mem = usage_value_text(snapshot.memory.used_percent());
        assert!(html.contains(&escape_html(&cpu)), "{html}");
        assert!(html.contains(&escape_html(&mem)), "{html}");
        for component in snapshot.components.iter().take(3) {
            let value = temperature_value_text(
                component.temperature_c,
                component.max_c,
                component.critical_c,
            );
            assert!(html.contains(&escape_html(&value)), "{html}");
        }
        // 同じ表示値ヘルパーが色語をテキストに載せることを固定する。
        assert!(usage_value_text(90.0).contains("高"));
        assert!(usage_value_text(70.0).contains("中"));
        assert!(usage_value_text(10.0).contains("低"));
        assert!(temperature_value_text(Some(105.0), Some(90.0), Some(100.0)).contains("危険"));
        assert!(!html.contains("aria-hidden"), "{html}");
    }

    #[test]
    fn mirror_filter_value_is_masked_and_never_plaintext() {
        let token = "dsh_token_abc123";
        let masked = a11y_filter_value_text(token);
        assert!(!masked.contains("token"), "{masked}");
        assert!(!masked.contains("abc123"), "{masked}");
        assert!(masked.contains('•'), "{masked}");
        assert!(
            masked.contains(&format!("{} 文字", token.chars().count())),
            "{masked}"
        );
        assert_eq!(a11y_filter_value_text(""), "未入力");
        assert_eq!(
            a11y_filter_value_text(&"x".repeat(100))
                .matches('•')
                .count(),
            64
        );
    }

    #[test]
    fn help_focus_transition_is_single_sourced() {
        assert_eq!(dom_help_focus(false, false), DomHelpFocus::Unchanged);
        assert_eq!(dom_help_focus(true, true), DomHelpFocus::Unchanged);
        assert_eq!(dom_help_focus(false, true), DomHelpFocus::Enter);
        assert_eq!(dom_help_focus(true, false), DomHelpFocus::Leave);
        // 開閉を往復しても enter / leave は 1 回ずつ（復帰の二重化なし）。
        let mut shown = false;
        let mut changes = 0;
        for next in [true, true, false, false] {
            if dom_help_focus(shown, next) != DomHelpFocus::Unchanged {
                changes += 1;
            }
            shown = next;
        }
        assert_eq!(changes, 2);
    }

    #[test]
    fn escape_html_neutralises_markup_in_snapshot_text() {
        assert_eq!(
            escape_html("a&b<c>\"d\"'"),
            "a&amp;b&lt;c&gt;&quot;d&quot;&#39;"
        );
    }

    #[test]
    fn focused_column_header_enter_space_is_reserved_for_sorting() {
        let plain = Modifiers::default();
        assert!(is_sort_key_activation("enter", plain, false));
        assert!(is_sort_key_activation("space", plain, false));
        // 編集中は IME の確定 / 文字入力へ譲る（既存のガードを維持）。
        assert!(!is_sort_key_activation("enter", plain, true));
        assert!(!is_sort_key_activation("space", plain, true));
        assert!(!is_sort_key_activation(
            "space",
            Modifiers {
                shift: true,
                ..plain
            },
            false
        ));
        assert!(!is_sort_key_activation("f5", plain, false));

        // GPUI の組み込み活性化が呼ぶ on_click と同じコマンドでソートが変わる。
        let (mut dashboard, _, _) = dashboard_with_two_visible_rows();
        let before = dashboard.sort_spec().key;
        let target = if before == SortKey::Name {
            SortKey::Pid
        } else {
            SortKey::Name
        };
        dashboard.apply(Command::SetSortKey(target));
        assert_eq!(dashboard.sort_spec().key, target);
    }

    #[test]
    fn aria_sort_matches_between_dom_mirror_and_accesskit() {
        assert_eq!(
            a11y_sort_accesskit(true, SortDirection::Ascending),
            Some(AccessKitSortDirection::Ascending)
        );
        assert_eq!(
            a11y_sort_accesskit(true, SortDirection::Descending),
            Some(AccessKitSortDirection::Descending)
        );
        assert_eq!(a11y_sort_accesskit(false, SortDirection::Ascending), None);
        assert_eq!(a11y_sort_accesskit(false, SortDirection::Descending), None);

        // 隠し DOM ミラーはアクティブ列にだけ aria-sort を出す（既存契約）。
        let header = a11y_mirror_header_html(SortSpec {
            key: SortKey::Cpu,
            direction: SortDirection::Descending,
        });
        assert_eq!(header.matches("aria-sort=\"descending\"").count(), 1);
        assert!(!header.contains("aria-sort=\"ascending\""));
        let header = a11y_mirror_header_html(SortSpec {
            key: SortKey::Name,
            direction: SortDirection::Ascending,
        });
        assert_eq!(header.matches("aria-sort=\"ascending\"").count(), 1);
    }

    #[test]
    fn row_menu_key_shortcuts_cover_shift_f10_and_context_menu() {
        let plain = Modifiers::default();
        assert!(is_row_menu_shortcut(
            "f10",
            Modifiers {
                shift: true,
                ..plain
            }
        ));
        assert!(is_row_menu_shortcut("contextmenu", plain));
        assert!(is_row_menu_shortcut("menu", plain));
        // 無修飾 F10 や修飾付き ContextMenu は他ショートカットを邪魔しない。
        assert!(!is_row_menu_shortcut("f10", plain));
        assert!(!is_row_menu_shortcut(
            "contextmenu",
            Modifiers {
                control: true,
                ..plain
            }
        ));
        assert!(!is_row_menu_shortcut(
            "contextmenu",
            Modifiers {
                shift: true,
                ..plain
            }
        ));
        // F5 は従来どおり更新要求のまま（行メニューには影響しない）。
        assert!(!is_row_menu_shortcut("f5", plain));
        assert_eq!(
            web_key_action("f5", plain, false),
            Some(WebKeyAction::Refresh)
        );
    }
}

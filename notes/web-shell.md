---
type: Implementation Report
title: ブラウザ版 GPUI タスクマネージャーシェル（web/）の実装と Runtime Proof
description: gpui-pre 0.3.12 の wasm Web バックエンドで、ネイティブ版と共通の tm-core / tm-viewmodel を使う読み取り専用タスクマネージャーシェルを実装し、ヘッドレス Chrome（CDP）で描画・更新・操作・ライブ接続・エラー時を検証した記録
tags: [gpui, wasm, webgpu, webgl2, trunk, task-manager, read-only, runtime-proof]
resource: work/gpui-task-manager/web
generated: 2026-11-14T22:01:18+09:00
status: verified
---

## 結論（要約）

- `work/gpui-task-manager/web/` にブラウザ版シェルを実装した。
  `cargo check --target wasm32-unknown-unknown` と `trunk build --release` が成功し、
  ヘッドレス Chrome 上で画面が描画された。
- デモモードで **描画・1 秒更新・ホイールスクロール・行クリック（詳細パネル）・
  キーボード絞り込み・列クリックソート** を CDP で確認した（全チェック pass）。
- ライブモードで `tm-agent` の実プロセス 500 件（全 600 件超のうち上位）を表示した。
  トークンは **URL フラグメント `#token=`** で受け取り、**`Authorization: Bearer`
  ヘッダ**で送る（クエリ `?token=` は廃止済み）。CORS プリフライトも確認した。
- エージェント停止時は **デモへフォールバックせず**、エラーをステータスバーと表に
  表示することを確認した。
- 3 モードともコンソールに panic / JS 例外は無い（GPUI/wgpu の想定内警告のみ）。
- 公開物（スクリーンショット・証拠 JSON）は `tm-agent --redact` で取得し、
  ホスト名・ユーザー名・ホームパスを伏せている。

## 成果物

| パス | 内容 |
|---|---|
| `work/gpui-task-manager/web/` | 独立 crate（`src/`、`Cargo.toml`、`Trunk.toml`、`index.html`、`assets/fonts/`） |
| `work/gpui-task-manager/web/README.md` | ビルド・起動・agent 連携・既知の制約 |
| `work/gpui-task-manager/web/scripts/cdp_verify_web.py` | ヘッドレス Chrome を CDP 操作する検証スクリプト（デモ / ライブ / エラー時） |
| `work/gpui-task-manager/web/verification/web-shell-evidence-*.json` | 検証の証拠 JSON（状態フック、ネットワーク、コンソール、チェック結果） |
| `work/gpui-task-manager/notes/assets/web-shell-*.png` | スクリーンショット 18 枚（デモ 8 / ライブ 8 / エラー時 2） |
| `work/gpui-task-manager/notes/web-shell.md` | 本ドキュメント |

## 構成と設計判断

### crate 構成

`web/` は親 workspace に含めない独立 crate にした（`Cargo.toml` に空の
`[workspace]` を宣言）。理由:

- ルート workspace はネイティブ（`tm-platform` / `gpui-kit`）を含み、wasm では
  ビルドできない依存を引く。独立させることで `cargo check --target
  wasm32-unknown-unknown` を純粋に保てる。
- `tm-core` / `tm-viewmodel` は path 依存で共有する。両 crate は wasm 対応
  （`tm-core` は `serde` feature を有効化）。

依存は `gpui = { package = "gpui-pre", version = "=0.3.12", default-features = false }`
と `gpui_platform` / `gpui_web` の同一バージョン固定。`gpui-kit` /
`gpui-component` は `wasm_thread`（nightly 必須）を引くため使わず、UI は gpui 素の
`div` で自作した。

### 初期化（`src/lib.rs`）

`gpui_platform::application()` は wasm で `WebPlatform` を既定設定で作るため、
次の 2 点を設定できない。そのためスパイク（`poc-wasm/`）と同じく
`gpui_web::WebPlatform` を直接構築する:

1. `CanvasFontFallback::EmojiAndCjk` — 日本語を Canvas フォールバックで描画する
   （既定の `Emoji` では CJK が描画されない）。
2. `platform.fetch_http_client()` を `Application::with_platform(...)
   .with_http_client(...)` に注入 — 既定は `NullHttpClient` で fetch が失敗する。

`gpui_platform::web_init()`（panic hook + console ログ）を `#[wasm_bindgen(start)]`
の先頭で呼ぶ。フォントは IBM Plex Sans / Mono を `include_bytes!` で埋め込み、
`cx.text_system().add_fonts` でロードする。

### 状態管理

画面状態はネイティブ版と共通の `tm_viewmodel::Dashboard` が持つ。シェルは
`Command`（Select / SetSortKey / SetFilterText / ClearFilter / SelectRelative）を
適用して描画するだけにし、ソート・絞り込み・選択のロジックを再実装しない。

ブラウザ版では次を意図的に制限している:

- `PlatformInfo.supports_process_control = false`（デモ・ライブとも）。
  `Effect::Signal` が返っても実行せず、読み取り専用メッセージを出す。
- シグナル操作の UI（ボタン・確認ダイアログ）を置かない。
- ポップアップ/ダイアログは GPUI Web で使えないため、確認ダイアログ自体が不要。

### レイアウト

ネイティブ版と同じ Catppuccin Mocha 系トークンを `src/theme.rs` に重複定義した
（ネイティブ app crate に依存すると `gpui-kit` を引くため）。画面は

- ヘッダー（固定 52px）
- 概要カード 4 枚（固定 186px）
- ツールバー（固定 36px）
- プロセス表（`uniform_list`）+ 詳細パネル（320px）
- ステータスバー（固定 26px）

の縦積みで、表だけが伸縮する。固定高は CDP 検証スクリプトのクリック座標計算と
共有している（`scripts/cdp_verify_web.py` 冒頭の定数）。

### 検証用フック

GPUI は canvas に描画するため DOM から状態を読めない。CDP 検証用に
`document.title` と `body[data-gpui-proof]` へ状態を書き出す:

```text
mode=demo;tick=5;cpu=32.6;mem=57.2;rows=28;total=28;filter=;selected_pid=900;
sort=CPU;dir=desc;sampled=1760000004;vw=1440;vh=857;live_error=0;
```

これは検証専用の計装で、トークンやエンドポイントは含めない。

## 画面仕様

| 領域 | 内容 |
|---|---|
| ヘッダー | アプリ名、ホスト名、接続モード（デモ / ライブ）、読み取り専用バッジ、OS / カーネル |
| CPU カード | 全体 %、コア数・周波数、履歴スパークライン、per-core バー、Load average |
| メモリカード | 使用率 %、使用/合計、使用率バー、スワップ、履歴スパークライン、空き |
| ネットワークカード | 受信レート + 履歴、送信レート + 履歴 |
| システムカード | ホスト、稼働時間、プロセス数、ディスク使用率（最大 2 マウント） |
| ツールバー | 検索ボックス（フォーカスで青枠）、現在のソート、表示件数 |
| プロセス表 | 名前（状態ドット付き）・コマンド・CPU・メモリ・PID・ユーザー・状態・スレッド |
| 詳細パネル | 名前、状態、PID、ユーザー、CPU、RSS、仮想メモリ、スレッド、親 PID、起動時刻、起動から、コマンド全文、読み取り専用の注記 |
| ステータスバー | データ源、最終更新（UTC）、表示/取得/全件数、エラーまたは操作ヒント |

表は `uniform_list`（行高 30px）で、500 行でも可視範囲のみ描画する。列ヘッダは
`SortKey` にマップされ、同じ列の再クリックで方向が反転する（`Command::SetSortKey`）。
コマンド列は表示専用（ソート対象外）。

キーボードは `on_key_down` で処理する: 文字入力で絞り込み、`Backspace` で削除、
`Esc` で解除、`↑↓/PageUp/PageDown/Home/End` で選択移動。日本語 IME はブラウザ版で対応（`notes/ime.md`）
（ASCII のみ）。

## データ経路

### デモ（既定）

URL クエリが無い場合、`tm_core::demo::DemoProbe` を `DataSource::Demo` として保持し、
`cx.spawn` の単一タスクが 1 秒間隔で `probe.sample()` → `dashboard.ingest()` する。
デモデータは tick の純関数で、同じ tick なら同一内容になる。

### ライブ（`?agent=`）

`?agent=<base URL>` がある場合、`DataSource::Live(Arc<AgentClient>)` になる。
同じサンプラタスクが次を行う:

1. `GET <agent>/api/v1/snapshot?limit=500` を `Authorization: Bearer <token>` 付きで送る
2. `{schema_version, total_process_count, snapshot}` を `serde_json` でパース
3. `schema_version` を `tm_core::WIRE_SCHEMA_VERSION` と照合
4. `dashboard.ingest(snapshot)` で反映（`total_process_count` はステータスバーに表示）

**多重取得しない**: ループは「取得 → ingest → 1 秒待つ」の逐次実行なので、
エージェントが遅い場合も要求が積み上がらない（検証では約 13 秒で 13 回の GET）。

**フォールバックしない**: 取得失敗時は `live_error` を保持してステータスバーと表に
エラーを表示し、直前のスナップショットは保持する。デモデータへは切り替えない。

## 認証・セキュリティ

- **トークンは URL フラグメント `#token=...` で受け取る**。フラグメントは
  サーバにも `Referer` にも送られない。クエリ `?token=` は廃止され、
  エージェントは 401 を返す（Runtime Proof で確認）。
- エージェントへの認証は **`Authorization: Bearer <token>` ヘッダ**のみ。
  リクエスト URL にトークンを含めない（CDP のネットワーク証拠で
  `token_in_url=false` を確認）。
- トークンは DOM・タイトル・コンソール・証拠 JSON に出さない。エラー文字列は
  `redact_token()` でトークン（とその percent-encode 形）を `***` に置換する。
- エージェントは `Host` ヘッダが loopback 以外なら 403、CORS は loopback
  オリジンのみ許可し、プリフライトで `Authorization, Content-Type` を許可する。
  ブラウザ側も loopback 以外へは接続しない想定。
- 公開用スクリーンショットと証拠は `tm-agent --redact` で取得し、ホスト名→`host`、
  ユーザー名→`user`、`/home/<user>`→`/home/user` に置換している。
- 読み取り専用: ブラウザ側は GET のみ。`Effect::Signal` は実行しない。
  エージェントにも操作エンドポイントは無い。

## 検証環境

- Kubuntu 2026.04 / rustc 1.99.1 stable / cargo 1.99.1
- wasm32-unknown-unknown ターゲット導入済み
- trunk 0.22.1（wasm-bindgen CLI 0.2.130 を自動取得）
- Google Chrome 157.0.8103.50（headless=new、CDP 9223、viewport 1440x857）
- 静的配信: `python3 -m http.server 8091 --bind 127.0.0.1 -d dist`
- エージェント: `tm-agent`（`TM_AGENT_TOKEN=test`、`--redact`）
- 検証日: 2026-11-14

## Runtime Proof

### 1. ビルド

```text
$ cargo check --target wasm32-unknown-unknown --manifest-path web/Cargo.toml
    Checking tm-web v0.1.0 (.../work/gpui-task-manager/web)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1m 01s
（2 回目以降は 0.3 秒前後。警告 0）

$ cd web && trunk build --release
    Finished `release` profile [optimized] target(s) in 2.6s
2026-11-14T12:28:20Z  INFO applying new distribution
2026-11-14T12:28:20Z  INFO ✅ success
```

生成物（release、2026-11-16 再計測）:

| ファイル | サイズ | gzip |
|---|---|---|
| `dist/*_bg.wasm` | 7,641,445 B（7.29 MiB） | 3,044,050 B（2.90 MiB） |
| `dist/*.js` | 159,873 B（156 KiB） | 24,504 B |
| `dist/index.html` | 1,320 B | 842 B |

`data-wasm-opt="z"` により wasm-opt（-Oz）を適用（14.8 MB → 7.6 MB）。

### 2. デモモード（CDP 検証）

`python3 cdp_verify_web.py --url http://127.0.0.1:8091/ --label demo` →
**failures: [] / passed: true**。状態フックの時系列:

| step | tick | rows | filter | selected_pid | sort | dir |
|---|---|---|---|---|---|---|
| initial | 4 | 28 | (空) | 900 | CPU | desc |
| after_ticks（2.5 秒後） | 7 | 28 | (空) | 900 | CPU | desc |
| scroll_down | 8 | 28 | (空) | 900 | CPU | desc |
| row_click（3 行目） | 10 | 28 | (空) | 4242 | CPU | desc |
| filter_typed（`fire`） | 12 | 1 | fire | 800 | CPU | desc |
| filter_cleared（Esc） | 13 | 28 | (空) | 800 | CPU | desc |
| sort_cpu | 13 | 28 | (空) | 800 | CPU | asc |
| sort_cpu_flip | 14 | 28 | (空) | 800 | CPU | desc |
| sort_name | 15 | 28 | (空) | 800 | 名前 | asc |
| final | 16 | 28 | (空) | 800 | 名前 | asc |

- 1 秒タイマーで tick が増加（4 → 7 → … → 16）。
- 行クリックで選択 PID が 900 → 4242 に変わり、詳細パネルが切り替わる。
- `fire` 入力で 28 行 → 1 行（firefox）に絞り込まれ、Esc で戻る。
- CPU 列クリックで降順 → 昇順、再クリックで反転、名前列で名前順になる。

### 3. ライブモード（CDP 検証）

`TM_AGENT_TOKEN=test ./target/debug/tm-agent --listen 127.0.0.1:18787 --redact`
を起動し、`?agent=http://127.0.0.1:18787#token=test` を開いて検証 →
**failures: [] / passed: true**。

| 項目 | 結果 |
|---|---|
| モード | `mode=live`、`live_error=0` |
| 表示行数 | 500 行（`total=500`、ステータスバーは「表示 500 / 取得 500 / 全 636 件」） |
| tick | 3 → 5 → 12（1 秒更新） |
| 行クリック | selected_pid 1 → 8564 |
| 絞り込み `fire` | 500 行 → 43 行 |
| ソート | CPU asc ⇄ desc、名前 asc |
| ネットワーク | スナップショット GET 13 回（約 13 秒）、**URL に token 無し**、`Authorization` ヘッダあり |
| CORS | プリフライト（OPTIONS）1 回を観測（`Access-Control-Allow-Headers: Authorization, Content-Type`） |

### 4. エラー時（エージェント停止）

エージェントを停止した状態で同じ URL を開き、`--expect-agent-error` で検証 →
**failures: [] / passed: true**。

| 項目 | 結果 |
|---|---|
| モード | `mode=live`（デモへ切り替わらない） |
| 行数 | 0（デモデータを表示しない） |
| エラー | `live_error=1`、表に「エージェントに接続できません（デモへはフォールバックしません）」、ステータスバーに赤字で接続失敗の要約 |
| ネットワーク | 6 回の GET 試行（1 秒間隔・多重なし）、URL に token 無し |

### 5. コンソール

| モード | 全メッセージ | 想定内（benign） | panic / JS 例外 |
|---|---|---|---|
| デモ | 10 | 3 | 0 |
| ライブ | 14 | 7 | 0 |
| エラー時 | 22 | 9 | 0 |

想定内の例: WebGPU アダプタ無し → WebGL2 フォールバック、dual-source blending
非対応によるサブピクセル AA 無効化、SwiftShader の ReadPixels 警告、
エラー時の `net::ERR_CONNECTION_REFUSED`。panic / `Runtime.exceptionThrown` は無い。

### 6. スクリーンショット

`work/gpui-task-manager/notes/assets/` に保存（1440x857、ライブは `--redact`）:

| モード | ファイル |
|---|---|
| デモ | `web-shell-01-initial-demo.png` 〜 `web-shell-08-final-demo.png` |
| ライブ | `web-shell-01-initial-live.png` 〜 `web-shell-08-final-live.png` |
| エラー時 | `web-shell-01-initial-live-error.png`、`web-shell-09-agent-error-live-error.png` |

各モードの内訳: 01 初期表示 / 02 タイマー更新後 / 03 スクロール後 /
04 行クリック（詳細パネル）/ 05 絞り込み / 06 CPU ソート / 07 名前ソート / 08 最終。

## 既知の制約・未確認事項

- 読み取り専用（シグナル操作なし）。ブラウザから OS のプロセスを直接は読めない。
- 日本語 IME はブラウザ版・ネイティブ版とも対応（`EntityInputHandler` + composition、`notes/ime.md`）。
- headless Chrome は WebGL2（ANGLE + SwiftShader）へフォールバックする。
  WebGPU を強制すると device lost で描画停止（スパイクで確認）。実 GPU の
  WebGPU 描画、Safari / Firefox は未検証。
- wasm が 7.29 MiB（gzip 2.90 MiB、2026-11-16 計測、wasm-opt 適用済み）と大きい。
  初回ロード時間は未計測（localhost のスパイクでは wasm 転送 46 ms だったが、実ネットワークでは
  転送とコンパイルが支配的になる）。
- 単一ウィンドウのみ。ポップアップ/ダイアログ/ファイルダイアログ不可。
- CJK は Canvas フォールバック描画のため、折り返し・字間の品質はネイティブに劣る。
- 接続は loopback 前提。リモート公開用の TLS・認証・レート制限は未実装。
- モバイル（タッチ・ソフトキーボード）は未検証。
- プロセス表のコマンド列は表示専用で、ソート対象外。

## 再現手順

```bash
cd work/gpui-task-manager

cargo check --target wasm32-unknown-unknown --manifest-path web/Cargo.toml
cd web && trunk build --release && cd ..

# 配信（別シェル）
cd web && python3 -m http.server 8091 --bind 127.0.0.1 -d dist

# Chrome（別シェル）
google-chrome --headless=new --no-sandbox --disable-dev-shm-usage \
  --window-size=1440,1000 --remote-debugging-port=9223 \
  --user-data-dir=/tmp/tm-web-chrome about:blank

# デモ
cd web/scripts
python3 cdp_verify_web.py --url http://127.0.0.1:8091/ \
  --out ../../notes/assets --evidence-out ../verification \
  --prefix web-shell --label demo

# ライブ（別シェルで agent を起動してから）
#   TM_AGENT_TOKEN=test ./target/debug/tm-agent --listen 127.0.0.1:18787 --redact
python3 cdp_verify_web.py \
  --url 'http://127.0.0.1:8091/?agent=http://127.0.0.1:18787#token=test' \
  --out ../../notes/assets --evidence-out ../verification \
  --prefix web-shell --label live

# エラー時（agent を停止してから）
python3 cdp_verify_web.py \
  --url 'http://127.0.0.1:8091/?agent=http://127.0.0.1:18787#token=test' \
  --out ../../notes/assets --evidence-out ../verification \
  --prefix web-shell --label live-error --expect-agent-error
```

## 参考

- `work/gpui-task-manager/notes/wasm-spike.md` — GPUI Web の capability・制約・
  推奨構成（本実装の前提）
- `work/gpui-task-manager/poc-wasm/` — スパイク実装（初期化・CJK・fetch の実例）
- `work/gpui-task-manager/web/README.md` — ビルド・起動・agent 連携・制約
- `work/gpui-task-manager/crates/tm-core/` — ドメイン・整形・DemoProbe
- `work/gpui-task-manager/crates/tm-viewmodel/` — Dashboard reducer
- `work/gpui-task-manager/agent/` — 読み取り専用エージェント
- gpui-pre 0.3.12（Apache-2.0）、IBM Plex（SIL OFL 1.1）

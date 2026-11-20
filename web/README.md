# web/ — ブラウザ版 GPUI タスクマネージャーシェル

gpui-pre 0.3.12 の Web バックエンド（`wasm32-unknown-unknown`）で動く、
**読み取り専用**のタスクマネージャー UI です。ネイティブ版と同じ
`tm-core`（ドメイン・整形・DemoProbe）と `tm-viewmodel`（Dashboard reducer）を使い、
データ供給だけを差し替えます。

- 既定: ブラウザ内で `tm_core::demo::DemoProbe` を更新間隔ごとにサンプリング
- ライブ: ループバックの `tm-agent` の `GET /api/v1/stream`（SSE）を優先し、
  未接続・終了・失敗・アイドル時は `GET /api/v1/snapshot?limit=500` の定期取得へ
  フォールバック（DOM proof の `sse=active|fallback` で確認）
- 更新間隔はツールバーの **0.5s / 1s / 2s / 5s プリセット**で切り替えられます。
  AC ・ウィンドウ表示時の下限は 0.5s なので 0.5s プリセットも実効します
  （未変更の既定は 1s）。選択値は `localStorage` の `gpui-task-manager.settings` に
  保存され、起動時に復元されます。適用中の値は DOM proof の `interval=<ミリ秒>`
  フィールド（`interval=500` / `1000` / `2000` / `5000`）で確認できます
  （トークンやホスト名は含まれません）。

実装・検証の詳細は `../notes/web-shell.md` を参照してください。

## 構成

| パス | 内容 |
| --- | --- |
| `src/lib.rs` | エントリ（`WebPlatform` 直接構築、フォントロード、ウィンドウ生成） |
| `src/shell.rs` | 画面（ヘッダー / 概要 / 表 / 詳細 / ステータスバー）、入力、データ経路 |
| `src/settings.rs` | `localStorage` の読み書き（更新間隔プリセット・テーマ、スキーマ互換・破損時隔離） |
| `src/redaction.rs` | ワイヤの生スナップショットから識別子を回収し伏せ字化（表示・エクスポートへ適用） |
| `src/agent.rs` | `tm-agent` クライアント（Authorization ヘッダ、JSON / SSE パース、エラー整形） |
| `src/theme.rs` | Catppuccin Mocha / Latte の配色トークンと切替状態（ネイティブ版と同系） |
| `src/widgets.rs` | card / metric_bar / sparkline / pill / kv などの小さな部品 |
| `index.html` | trunk のエントリ（`data-trunk rel="rust"`） |
| `Trunk.toml` | trunk 設定（serve ポート 8091） |
| `assets/fonts/` | IBM Plex Sans / Mono（SIL OFL 1.1、ライセンス同梱） |
| `scripts/cdp_verify_web.py` | ヘッドレス Chrome を CDP 操作して Runtime Proof を取るスクリプト |
| `verification/` | CDP 検証の証拠 JSON（デモ / ライブ / エラー時） |
| `dist/` | `trunk build` の生成物（git 管理外、CI ではライセンス／通知も同梱） |

この crate は親 workspace に含めない独立 crate です（`Cargo.toml` に空の
`[workspace]` を宣言）。ルートで `cargo test --workspace` しても対象外なので、
検証は `--manifest-path web/Cargo.toml` を指定してください。

## 必要環境

- Rust stable（検証: rustc 1.99.1）+ `rustup target add wasm32-unknown-unknown`
- [trunk](https://trunkrs.dev/) 0.22.1（wasm-bindgen CLI は trunk が自動取得）
- 検証用: Google Chrome **157**（headless、CDP による Runtime Proof 専用）+ Python 3 + `websockets`。下記「既知の制約」の互換マトリクスは Chrome **158** で別途実測しており、目的が異なる（混同しないこと）

## ビルド

```bash
cd .

# 型チェック（wasm ターゲット）
cargo check --target wasm32-unknown-unknown --manifest-path web/Cargo.toml

# 配布物の生成（web/dist/index.html + wasm + JS）
cd web
trunk build --release
```

- 初回は依存 crate のコンパイル込みで 1〜2 分、2 回目以降は数秒です。
- 生成物サイズ（release、2026-11-20 再計測）: wasm 7.58 MiB（7,949,118 B、
  gzip 3,165,445 B）、JS 156 KiB、`index.html` 1.3 KiB。`data-wasm-opt="z"` により
  trunk が wasm-opt（-Oz）を適用します（初回は wasm-opt を自動ダウンロード）。

## 起動（デモモード）

```bash
cd web
python3 -m http.server 8091 --bind 127.0.0.1 -d dist
# ブラウザで http://127.0.0.1:8091/ を開く
```

URL クエリが無ければデモモードです。`tm-core` の決定論的デモデータを
設定した更新間隔（既定 1s、0.5s / 1s / 2s / 5s から切替可）で
`Dashboard::ingest` します（実プロセスは表示されません）。

## ライブ接続（tm-agent）

```bash
cd .
cargo build -p tm-agent
# トークンは省略して自動生成（Linux/Unix のみ、0600 ファイル、保存先は XDG state パス、
# stderr にはファイル名 `token` のみ、再起動でも再利用）。
# Windows では自動生成せず、--token-file / TM_AGENT_TOKEN / --token の明示が必要
# （固定エラーの文言は ../README.md「認証」節を参照）。
# --token は非推奨（/proc/<pid>/cmdline から読まれ得る）。既定の待受ポートは 8787。
./target/debug/tm-agent --redact
token_file="${XDG_STATE_HOME:-$HOME/.local/state}/tm-agent/token"
```

ブラウザで次を開きます（トークンは XDG state パス（Windows は AppData 配下）のファイル内容を **URL フラグメント** に指定）:

```text
http://127.0.0.1:8091/?agent=http://127.0.0.1:8787#token=<トークン>
```

接続仕様:

1. **トークンは `#token=...`（フラグメント）で受け取ります。**
   フラグメントはサーバにも `Referer` にも送られません。
   クエリ `?token=` は廃止されており、エージェントは 401 を返します。
2. エージェントへのリクエストは
   `Authorization: Bearer <token>` ヘッダで認証します（クエリには載せません）。
3. `?agent=` はエージェントのベース URL です。ポーリングでは `limit=500` で取得します。
4. **SSE 優先**: `GET /api/v1/stream` を購読し、最初のフレームを受信した時点で
   `sse=active` となり定期取得を一時停止します。接続前・終了・失敗・アイドル
   （3 秒以上フレーム無し）のときは `sse=fallback` に戻り、設定した更新間隔
   （0.5s / 1s / 2s / 5s）のポーリングへフォールバックします。ポーリング自体は
   **多重取得なし**（前回の応答を待ってから次を発行）です。終了・失敗後は
   5 秒のクールダウンで SSE の再接続を試みます。
5. 取得に失敗した場合はステータスバーと表にエラーを表示し、
   **デモへはフォールバックしません**（直前のスナップショットは保持）。
   これはデータ源の切替えの話であり、上記の SSE → ポーリングのフォールバックとは別です。
6. CORS は loopback オリジンのみ許可され、`Authorization` ヘッダ用の
   プリフライトにもエージェントが対応しています（`Host` ヘッダも loopback 必須）。
7. 更新に失敗して古いスナップショットが画面に残った場合は、ヘッダーと
   ステータスバーに **stale バッジ**（「stale: 表示中のデータは古いです」）を
   表示します（ヘッダーは「stale (旧データ)」）。最初の取得失敗で表示する旧データが
   無い場合は stale 表示を行わず、エラーメッセージのみを出します。
   更新が成功すると stale は解消します。
8. **URL の `?agent=` / `#token=` 変更はページ再読込なしで検知**されます。
   サンプラーが各周期で URL を読み直し、接続先またはトークンの差分を検知すると
   自動的に再接続します。エージェント再起動でトークンが変わった場合も、
   アドレスバーの `#token=` を貼り替えるだけで再接続できます。このとき
   ステータスに固定文言の **RELOAD_NOTICE**
   （「接続設定を再読込しました。エージェントへ再接続しています…」）を表示します。
   なお 401（トークン不正）はポーリング／SSE の別を問わず即時に DOM proof の
   `reconnect=1` と `#token=` のトークン再入力案内として現れます。SSE 経由の
   401 でも次のポーリングを待たずにこれらの状態が設定され、SSE エラーは
   `live_error`（DOM proof の `live_error=` は 0 のまま）を汚しません。

注意:

- トークンを含む URL を共有しないでください。フラグメントはサーバに送られませんが、
  ブラウザの履歴には残ります。
- **Windows ではトークンの自動生成を行いません**。新規生成はできず、既存ファイル
  （`%APPDATA%` / `%USERPROFILE%\AppData\Roaming` 配下）の再利用のみです。
  `--token-file` / `TM_AGENT_TOKEN` / `--token` のいずれかを明示してください
  （固定エラーの文言は `../README.md`「認証」節を参照）。
- `--redact` を付けると、ホスト名（コマンド引数内のホスト名・tailnet ドメインを含む）・ユーザー名・`/home/<user>` が
  `host` / `user` / `/home/user` に置換されます。公開用スクリーンショットは
  このモードで取得しています（`verification/` も同様）。

## 画面

| 領域 | 内容 |
| --- | --- |
| ヘッダー | アプリ名、ホスト名、接続モード（デモ / ライブ）、読み取り専用バッジ |
| 概要 | CPU（全体 % / per-core バー / 履歴 / Load）、メモリ（% / バー / スワップ / 履歴）、ネットワーク（rx/tx レート / 履歴）、システム（ホスト / 稼働時間 / プロセス数 / ディスク）、各メトリクスのトレンドバッジ（上昇 / 下降 / 横ばい / 不明を文字と色で表示）、温度ゲージ（値と `max` / `crit` しきい値、状態「正常 / 注意 / 危険」をテキストで表示） |
| ツールバー | 検索ボックス（キーボード入力）、現在のソート、表示件数、更新間隔プリセット（0.5s / 1s / 2s / 5s、選択値は `localStorage` に保存・起動時に復元）、CSV / JSON エクスポートボタン（伏せ字化済みスナップショットをファイルへダウンロード） |
| プロセス表 | 名前・コマンド・CPU・メモリ・PID・ユーザー・状態・スレッド。`uniform_list` による仮想スクロール。列クリックでソート、行クリックで詳細 |
| 詳細パネル | 選択プロセスの情報（コマンド、親 PID、起動時刻、RSS/仮想メモリ、スレッドなど）。**シグナル操作はありません**（読み取り専用の明示） |
| ステータスバー | データ源、最終更新、件数、エラーメッセージ、操作ヒント |

キーボード:

- 文字入力: 絞り込み（名前・コマンド・ユーザー・PID の AND 検索）
- `Backspace`: 絞り込み文字を削除、`Esc`: 絞り込み解除
- `↑` / `↓` / `PageUp` / `PageDown` / `Home` / `End`: 選択移動
- 列ヘッダのクリック: ソート（同じ列の再クリックで昇順/降順を反転）

## 検証（Runtime Proof）

```bash
cd .

# 1) wasm 型チェック
cargo check --target wasm32-unknown-unknown --manifest-path web/Cargo.toml

# 2) release ビルド
cd web && trunk build --release && cd ..

# 3) 静的配信（別シェル）
cd web && python3 -m http.server 8091 --bind 127.0.0.1 -d dist

# 4) ヘッドレス Chrome（別シェル、CDP 9223）
google-chrome --headless=new --no-sandbox --disable-dev-shm-usage \
  --window-size=1440,1000 --remote-debugging-port=9223 \
  --user-data-dir=/tmp/tm-web-chrome about:blank

# 5) デモモードの検証（描画・タイマー・スクロール・行クリック・絞り込み・ソート）
cd web/scripts
python3 cdp_verify_web.py --url http://127.0.0.1:8091/ \
  --out ../../notes/assets --evidence-out ../verification \
  --prefix web-shell --label demo

# トークンは自動生成ファイルから読む（値をコードへ埋め込まない）
token_file="${XDG_STATE_HOME:-$HOME/.local/state}/tm-agent/token"

# 6) ライブモードの検証（tm-agent を既定 8787 で起動してから）
python3 cdp_verify_web.py \
  --url "http://127.0.0.1:8091/?agent=http://127.0.0.1:8787#token=$(cat "${token_file}")" \
  --out ../../notes/assets --evidence-out ../verification \
  --prefix web-shell --label live

# 7) エージェント停止時の検証（デモへフォールバックしないこと）
python3 cdp_verify_web.py \
  --url "http://127.0.0.1:8091/?agent=http://127.0.0.1:8787#token=$(cat "${token_file}")" \
  --out ../../notes/assets --evidence-out ../verification \
  --prefix web-shell --label live-error --expect-agent-error
```

- スクリーンショット: `../notes/assets/web-shell-*.png`
- 証拠 JSON: `verification/web-shell-evidence-*.json`
  （状態フック、ネットワーク要求、コンソール、チェック結果）
- 検証用の状態フック: アプリは `body[data-gpui-proof]` にのみ `key=value;` 形式の
  状態を書き出します。`document.title` は人間向けの固定文言
  （`GPUI タスクマネージャー (Web) — デモ` / `GPUI タスクマネージャー (Web) — ライブ`）
  とデータ源だけに限定し、DOM proof の内部フィールド（`mode` / `tick` / `rows` /
  `filter` / …）はタイトルへ書き出しません。CSP を無闇に追加してタイトル更新を
  止める必要はありません（フックは body 属性で完結しています）。GPUI は canvas
  描画なので従来 CDP から観測できるのはこのフックだけでしたが、現在は下記のとおり
  AX ツリーも突き合わせます。全キーと値域は下記「DOM proof フィールド一覧」を
  参照してください。
- クリック座標は `src/shell.rs` のレイアウト定数（ヘッダー 52px、概要 260px、
  ツールバー 36px、行 30px など）から計算しています。レイアウトを変えたら
  `scripts/cdp_verify_web.py` の定数も更新してください。

### DOM proof フィールド一覧

`body[data-gpui-proof]` に書き出される現行の全キーです（`web/src/shell.rs:2661-2684`
の `sync_proof_dom()` と `proof_with_*` ヘルパーが根拠）。値は固定識別子・数値・
サニタイズ済みテキストのみで、トークン・ホスト名・エンドポイントは含まれません。
CDP 検証では次の値域でパースしてください（`../notes/web-shell.md` の
「検証用フック」も参照）。

| キー | 値域 | 内容 |
| --- | --- | --- |
| `mode` | `demo` / `live` | データ源 |
| `tick` | 非負整数 | サンプル回数 |
| `cpu` | 小数 1 桁 | CPU 全体使用率（%） |
| `mem` | 小数 1 桁 | メモリ使用率（%） |
| `rows` | 非負整数 | 表示行数 |
| `total` | 非負整数 | プロセス数（スナップショット無しは 0） |
| `filter` | サニタイズ済み文字列（空可） | 絞り込み入力（`;` → `,`、`=`・改行は除去） |
| `selected_pid` | PID の十進数 / `-` | 選択行（未選択は `-`） |
| `sort` | `CPU` / `メモリ` / `仮想メモリ` / `PID` / `名前` / `ユーザー` / `状態` / `スレッド` / `起動時刻` | ソート列の固定ラベル |
| `dir` | `asc` / `desc` | ソート方向 |
| `sampled` | UNIX 秒の整数（スナップショット無しは 0） | 最終サンプル時刻 |
| `vw` | 非負整数 | ビューポート幅（px） |
| `vh` | 非負整数 | ビューポート高さ（px） |
| `live_error` | `0` / `1` | ライブ取得エラーの有無 |
| `stale` | `0` / `1` | 古いスナップショットを表示中か |
| `ime_composing` | `0` / `1` | IME 変換中か |
| `ime_field` | サニタイズ済み文字列（空可） | 入力欄の表示テキスト（未確定文字列を含む） |
| `ime_marked` | `<start>:<end>` / `-` | 変換中範囲（範囲無しは `-`） |
| `ime_selection` | `<start>:<end>` | 選択範囲 |
| `theme` | `dark` / `light` | 適用中のテーマ |
| `interval` | ミリ秒の整数（250〜10000、プリセットは 500 / 1000 / 2000 / 5000） | ユーザー設定の更新間隔（クランプ後） |
| `export` | `csv` / `json` / `none` | 直近のエクスポート形式 |
| `reconnect` | `0` / `1` | 401 による再接続要否（成功まで保持） |
| `menu` | `none` / `open` / `select` / `copy_pid` | 行コンテキストメニューの状態 |
| `preset` | `none` / `all` / `high_cpu` / `high_memory` / `large_virtual_memory` / `running` | 適用中のフィルタプリセット |
| `power` | `none` / `ac` / `battery` / `unknown` | 電源（スナップショット無しは `none`） |
| `sampling` | `none` / `interactive` / `battery` / `battery_saver` / `background` | 適応サンプリングプロファイル |
| `sampling_ms` | ミリ秒の整数 | 適応・クランプ後の実際のサンプラー待機時間 |
| `cgroups` | 非負整数 | cgroup 数（スナップショット無しは 0） |
| `sse` | `active` / `fallback` | ライブ転送状態（初フレーム受信で `active`、終了・失敗・アイドルで `fallback`） |

### DOM proof と AX ロールの対応検証

`body[data-gpui-proof]` は DOM 属性フックにすぎず、canvas 内の GPUI AccessKit
セマンティクスがブラウザのアクセシビリティツリーへ届いている保証はありません。
ミラー実装前は `Accessibility.getFullAXTree` で `columnheader` / `row` / `dialog`
が 0 件なのに DOM proof だけが成功する偽陽性状態が確認されていました。現在は
`web/src/shell.rs` のミラーが role / aria 付き DOM を公開するため、DOM proof 単独
では不十分として AX ツリーも突き合わせます。

- `Accessibility.enable` + `Accessibility.getFullAXTree` で AX ノードを取得し、
  `role` の出現数を集計して proof と比較します。期待値は
  `scripts/cdp_verify_web.py` の `EXPECTED_AX_ROLES`（`table` = 1、
  `columnheader` = 8〔ソート可能 7 列 + コマンド列〕、`row` = proof の `rows` + 1
  〔データ行 + ヘッダ行〕、`dialog` = 0 または 1〔ヘルプ表示の有無〕）で定義しています。
- `selected_pid` / `sort` / `dir` についても、選択行・列見出し・ソート方向が
  proof と一致するかを確認します（対応表は `PROOF_AX_STATE_FIELDS`）。`dir` は
  CDP の AX ツリーが `aria-sort` を property として露出しないため、列見出しの
  名前テキストに含まれる `昇順` / `降順` で照合します。
- 現状は **warning 扱い**です。不一致は証拠 JSON の `accessibility.warnings` と
  最終チェック出力の `warnings` に記録し、`failures` / `passed` には影響させません。
  ミラー実装と期待値・照合方法の修正後、`--self-check` と実 CDP の両方で
  `accessibility.warnings` = 0 件を確認済みです。AX 検証の未カバー項目が残るため
  `PROMOTE_AX_TO_FAILURE` は `False` のままで、残項目が解消されたら `True` へ
  反転して同じチェックをエラーへ昇格します。
- `--self-check`（オフライン）では `EXPECTED_AX_ROLES` の構造と、proof からの
  期待値解決（`rows+1` を含む）・AX ノード集計・名前テキストによる `dir` 照合の
  純粋関数だけを検証します（ブラウザ不要）。

## 既知の制約

- **読み取り専用**: ブラウザからプロセス操作はできません。UI にシグナル送信は
  なく、`tm-agent` も GET のみを提供します。
- **日本語 IME（ブラウザ版は対応済み）**: `gpui-pre-web` の IME ミラー
  （隠し textarea）と `EntityInputHandler` を使い、composition 中の未確定文字列は
  入力欄にのみ表示し、確定文字列だけを絞り込みへ反映します。ヘッドレス Chrome の
  `Input.imeSetComposition` / `Input.insertText` で検証済み（`../notes/ime.md`）。
  ネイティブ版も `EntityInputHandler` で実装済みです（`../notes/ime.md`）。
- **描画は WebGL2 フォールバック**: headless Chrome では WebGPU アダプタが
  取得できず、GPUI が WebGL2（ANGLE + SwiftShader）へ自動フォールバックします。
  WebGPU を強制すると device lost で描画が停止します（スパイクで確認済み）。
  実 GPU の通常 Chrome での WebGPU 描画は未確認です。
- **wasm サイズ**: 7.58 MiB（gzip 3.02 MiB、2026-11-20 計測）。wasm-opt（-Oz）適用後も大きいため、
  初回ロードに時間がかかります。配信時は gzip/brotli を有効にしてください。
- **単一ウィンドウ**: GPUI Web はトップレベルウィンドウ 1 個のみで、
  ポップアップ・ダイアログ・ファイルダイアログは使えません。
- **CJK は Canvas フォールバック描画**: 日本語はブラウザのシステムフォントで
  1 書記素ずつ描画されます。ネイティブのシェーピングに比べ折り返し・字間の
  品質は劣ります。
- **接続は loopback 前提**: エージェントはループバック bind・loopback Origin の
  CORS のみ許可します。リモート公開用の認証・TLS は未実装です。
- ブラウザ互換性（互換マトリクス）は Chrome **158** / Edge 157 / Firefox 160 で実動確認済み
  （2026-11-20 実測）。この 158 は互換マトリクス専用の値で、上記「必要環境」の Chrome **157**
  は CDP による日常のヘッドレス Runtime Proof 用スクリプト（`cdp_verify_*.py`）
  が動作確認済みのバージョンです。用途が異なるため使い分けてください。
  Firefox 160 は WebDriver BiDi での確認で、F1 のみ合成キーを用い、信頼済みキーは未確認です。
  Safari / Brave / モバイル（タッチ・ソフトキーボード）/ 実 GPU の WebGPU は未検証のままです。

## ライセンス

- 本 crate のコード: AGPL-3.0-or-later（リポジトリ本体と同じ）
- `gpui-pre` / `gpui-pre-platform` / `gpui-pre-web` 0.3.12: Apache-2.0
- IBM Plex Sans / Mono: SIL OFL 1.1（`assets/fonts/LICENSE-IBM-Plex.txt`）
- 他プロジェクトからコピーした AGPL/GPL コードは含みません（`poc-wasm` と同じプロジェクトの自作コードのみ再利用）。本 crate 自体は上のとおり AGPL-3.0-or-later です。
- 配布アーカイブ（`web-dist` ジョブの `gpui-task-manager-web`）には
  `LICENSE`（AGPL-3.0 本文）/ `NOTICE.md` / `THIRD_PARTY_NOTICES.md` /
  `LICENSE-IBM-Plex.txt` が同梱されます。`trunk build` の後に `web/dist/` へ
  コピーされ、二重ビルドの SHA256 再現性ゲートにも含まれます。
  `packaging/linux/licenses/crates/` が存在する場合は crate 別ライセンス全文も
  `dist/licenses/crates/` へコピーされます。

## 参考

- `../notes/wasm-spike.md` — GPUI Web の capability・制約・推奨構成（スパイク）
- `../notes/web-shell.md` — 本シェルの実装・検証記録（Runtime Proof）
- `../notes/verification.md` — リポジトリ全体の検証記録

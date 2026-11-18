# web/ — ブラウザ版 GPUI タスクマネージャーシェル

gpui-pre 0.3.12 の Web バックエンド（`wasm32-unknown-unknown`）で動く、
**読み取り専用**のタスクマネージャー UI です。ネイティブ版と同じ
`tm-core`（ドメイン・整形・DemoProbe）と `tm-viewmodel`（Dashboard reducer）を使い、
データ供給だけを差し替えます。

- 既定: ブラウザ内で `tm_core::demo::DemoProbe` を 1 秒ごとにサンプリング
- ライブ: ループバックの `tm-agent` から `GET /api/v1/snapshot?limit=500` を 1 秒ごとに取得

実装・検証の詳細は `../notes/web-shell.md` を参照してください。

## 構成

| パス | 内容 |
|---|---|
| `src/lib.rs` | エントリ（`WebPlatform` 直接構築、フォントロード、ウィンドウ生成） |
| `src/shell.rs` | 画面（ヘッダー / 概要 / 表 / 詳細 / ステータスバー）、入力、データ経路 |
| `src/agent.rs` | `tm-agent` クライアント（Authorization ヘッダ、JSON パース、エラー整形） |
| `src/theme.rs` | Catppuccin Mocha / Latte の配色トークンと切替状態（ネイティブ版と同系） |
| `src/widgets.rs` | card / metric_bar / sparkline / pill / kv などの小さな部品 |
| `index.html` | trunk のエントリ（`data-trunk rel="rust"`） |
| `Trunk.toml` | trunk 設定（serve ポート 8091） |
| `assets/fonts/` | IBM Plex Sans / Mono（SIL OFL 1.1、ライセンス同梱） |
| `scripts/cdp_verify_web.py` | ヘッドレス Chrome を CDP 操作して Runtime Proof を取るスクリプト |
| `verification/` | CDP 検証の証拠 JSON（デモ / ライブ / エラー時） |
| `dist/` | `trunk build` の生成物（git 管理外） |

この crate は親 workspace に含めない独立 crate です（`Cargo.toml` に空の
`[workspace]` を宣言）。ルートで `cargo test --workspace` しても対象外なので、
検証は `--manifest-path web/Cargo.toml` を指定してください。

## 必要環境

- Rust stable（検証: rustc 1.99.1）+ `rustup target add wasm32-unknown-unknown`
- [trunk](https://trunkrs.dev/) 0.22.1（wasm-bindgen CLI は trunk が自動取得）
- 検証用: Google Chrome 157（headless）+ Python 3 + `websockets`

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
- 生成物サイズ（release、2026-11-17 再計測）: wasm 7.29 MiB（7,642,794 B、
  gzip 3,044,134 B）、JS 156 KiB、`index.html` 1.3 KiB。`data-wasm-opt="z"` により
  trunk が wasm-opt（-Oz）を適用します（初回は wasm-opt を自動ダウンロード）。

## 起動（デモモード）

```bash
cd web
python3 -m http.server 8091 --bind 127.0.0.1 -d dist
# ブラウザで http://127.0.0.1:8091/ を開く
```

URL クエリが無ければデモモードです。`tm-core` の決定論的デモデータを
1 秒間隔で `Dashboard::ingest` します（実プロセスは表示されません）。

## ライブ接続（tm-agent）

```bash
cd .
cargo build -p tm-agent
# トークンは環境変数で渡す（--token は非推奨: /proc/<pid>/cmdline から読まれ得る）
TM_AGENT_TOKEN=test ./target/debug/tm-agent --listen 127.0.0.1:18787 --redact
```

ブラウザで次を開きます（トークンは **URL フラグメント**）:

```text
http://127.0.0.1:8091/?agent=http://127.0.0.1:18787#token=test
```

接続仕様:

1. **トークンは `#token=...`（フラグメント）で受け取ります。**
   フラグメントはサーバにも `Referer` にも送られません。
   クエリ `?token=` は廃止されており、エージェントは 401 を返します。
2. エージェントへのリクエストは
   `Authorization: Bearer <token>` ヘッダで認証します（クエリには載せません）。
3. `?agent=` はエージェントのベース URL です。`limit=500` で取得します。
4. 取得は 1 秒間隔・**多重取得なし**（前回の応答を待ってから次を発行）です。
5. 取得に失敗した場合はステータスバーと表にエラーを表示し、
   **デモへはフォールバックしません**（直前のスナップショットは保持）。
6. CORS は loopback オリジンのみ許可され、`Authorization` ヘッダ用の
   プリフライトにもエージェントが対応しています（`Host` ヘッダも loopback 必須）。

注意:

- トークンを含む URL を共有しないでください。フラグメントはサーバに送られませんが、
  ブラウザの履歴には残ります。
- `--redact` を付けると、ホスト名（コマンド引数内のホスト名・tailnet ドメインを含む）・ユーザー名・`/home/<user>` が
  `host` / `user` / `/home/user` に置換されます。公開用スクリーンショットは
  このモードで取得しています（`verification/` も同様）。

## 画面

| 領域 | 内容 |
|---|---|
| ヘッダー | アプリ名、ホスト名、接続モード（デモ / ライブ）、読み取り専用バッジ |
| 概要 | CPU（全体 % / per-core バー / 履歴 / Load）、メモリ（% / バー / スワップ / 履歴）、ネットワーク（rx/tx レート / 履歴）、システム（ホスト / 稼働時間 / プロセス数 / ディスク） |
| ツールバー | 検索ボックス（キーボード入力）、現在のソート、表示件数。ネイティブ版にある更新間隔ボタン（0.5s/1s/2s/5s）は無く、1 秒固定（保存値があれば起動時に復元） |
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

# 6) ライブモードの検証（tm-agent を 18787 で起動してから）
python3 cdp_verify_web.py \
  --url 'http://127.0.0.1:8091/?agent=http://127.0.0.1:18787#token=test' \
  --out ../../notes/assets --evidence-out ../verification \
  --prefix web-shell --label live

# 7) エージェント停止時の検証（デモへフォールバックしないこと）
python3 cdp_verify_web.py \
  --url 'http://127.0.0.1:8091/?agent=http://127.0.0.1:18787#token=test' \
  --out ../../notes/assets --evidence-out ../verification \
  --prefix web-shell --label live-error --expect-agent-error
```

- スクリーンショット: `../notes/assets/web-shell-*.png`
- 証拠 JSON: `verification/web-shell-evidence-*.json`
  （状態フック、ネットワーク要求、コンソール、チェック結果）
- 検証用の状態フック: アプリは `document.title` と
  `body[data-gpui-proof]` に状態（mode / tick / rows / filter / selected_pid /
  sort / live_error / viewport サイズ）を書き出します。GPUI は canvas 描画なので、
  CDP から観測できるのはこのフックだけです。
- クリック座標は `src/shell.rs` のレイアウト定数（ヘッダー 52px、概要 260px、
  ツールバー 36px、行 30px など）から計算しています。レイアウトを変えたら
  `scripts/cdp_verify_web.py` の定数も更新してください。

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
- **wasm サイズ**: 7.29 MiB（gzip 2.90 MiB、2026-11-17 計測）。wasm-opt（-Oz）適用後も大きいため、
  初回ロードに時間がかかります。配信時は gzip/brotli を有効にしてください。
- **単一ウィンドウ**: GPUI Web はトップレベルウィンドウ 1 個のみで、
  ポップアップ・ダイアログ・ファイルダイアログは使えません。
- **CJK は Canvas フォールバック描画**: 日本語はブラウザのシステムフォントで
  1 書記素ずつ描画されます。ネイティブのシェーピングに比べ折り返し・字間の
  品質は劣ります。
- **接続は loopback 前提**: エージェントはループバック bind・loopback Origin の
  CORS のみ許可します。リモート公開用の認証・TLS は未実装です。
- Safari / Firefox、モバイル（タッチ・ソフトキーボード）は未検証です。

## ライセンス

- 本 crate のコード: AGPL-3.0-or-later（リポジトリ本体と同じ）
- `gpui-pre` / `gpui-pre-platform` / `gpui-pre-web` 0.3.12: Apache-2.0
- IBM Plex Sans / Mono: SIL OFL 1.1（`assets/fonts/LICENSE-IBM-Plex.txt`）
- 他プロジェクトからコピーした AGPL/GPL コードは含みません（`poc-wasm` と同じプロジェクトの自作コードのみ再利用）。本 crate 自体は上のとおり AGPL-3.0-or-later です。

## 参考

- `../notes/wasm-spike.md` — GPUI Web の capability・制約・推奨構成（スパイク）
- `../notes/web-shell.md` — 本シェルの実装・検証記録（Runtime Proof）
- `../notes/verification.md` — リポジトリ全体の検証記録

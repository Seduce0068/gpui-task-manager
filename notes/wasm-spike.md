---
type: Spike Report
title: GPUI Web バックエンド（gpui-pre 0.3.12）実現性スパイク
description: ブラウザ上で GPUI のシステムモニタ UI（表・スクロール・検索入力・クリック・定期更新・HTTP fetch・CJK）が動くかを Runtime Proof 付きで検証した記録
tags: [gpui, wasm, webgpu, webgl2, trunk, wasm-bindgen, spike]
resource: ./poc-wasm
generated: 2026-11-14T20:57:11+09:00
status: verified
---

## 結論（要約）

- **動く**。`trunk build --release` が成功し、ヘッドレス Chrome 上で GPUI の UI が描画された。
  テキスト・表・スクロール・検索入力（キーボード）・クリック・1 秒定期更新・
  同一オリジン HTTP fetch・CJK（日本語）表示まで Runtime Proof を取得した。
- 描画は **wgpu**。headless Chrome では WebGPU アダプタが取得できず、
  GPUI が自動で **WebGL2（ANGLE + SwiftShader）** にフォールバックして描画した。
  WebGPU を強制すると初期化は成功するが **device lost** で描画が停止した（後述）。
- **マルチスレッド不要**。`gpui-pre` / `gpui-pre-platform` / `gpui-pre-web` を
  `default-features = false` にして stable で wasm32-unknown-unknown にビルド・実行できた。
- **CJK は `CanvasFontFallback::EmojiAndCjk` が必須**。既定の `Emoji` では CJK は
  Canvas フォールバック対象外になる。`gpui_platform::application()` ではこの設定が
  できないため、`gpui_web::WebPlatform` を直接構築する必要がある。
- 制約: トップレベルウィンドウは 1 個のみ。ブラウザから OS のプロセス情報は取得不可。
  日本語 IME 入力・クリップボードは未検証。

## 検証環境

- Kubuntu 2026.04 / rustc 1.99.1 stable / cargo 1.99.1
- wasm32-unknown-unknown ターゲット導入済み
- trunk 0.22.1（wasm-bindgen CLI 0.2.130 を自動取得）
- Google Chrome 157.0.8103.50（headless=new、CDP で操作）
- 依存: `gpui-pre` / `gpui-pre-platform` / `gpui-pre-web` = 0.3.12（Apache-2.0）
- 表示確認用フォント: IBM Plex Sans / IBM Plex Mono（SIL OFL 1.1、同梱）
- 検証日: 2026-11-14

## 成果物

| パス | 内容 |
|---|---|
| `./poc-wasm/` | 動く PoC crate（`src/lib.rs`、`Cargo.toml`、`index.html`、`Trunk.toml`） |
| `./poc-wasm/scripts/cdp_verify.py` | ヘッドレス Chrome を CDP 操作して証拠を取るスクリプト |
| `./poc-wasm/README.md` | ビルド・配信・検証の手順 |
| `./notes/assets/wasm-spike-*.png` | スクリーンショット（描画・スクロール・クリック・検索） |
| `./notes/assets/wasm-spike-evidence-*.json` | コンソールログ・DOM・状態フックの証拠 JSON |
| `./notes/assets/wasm-spike-backend-variants.json` | `?backend=webgl` / `?backend=webgpu` の切り替え証拠 |
| `./notes/assets/wasm-spike-webgpu-forced-01-initial.png` | 強制 WebGPU 時の描画停止（空白 canvas）の証拠 |
| `./notes/assets/wasm-spike-webgpu-noadapter-01.png` | WebGPU アダプタなし時のエラー表示の証拠 |
| `./notes/assets/wasm-spike-*-console.log` | ヘッドレス Chrome の生ログ |

## 再現手順

### 1. ビルド

```bash
cd ./poc-wasm
cargo check --target wasm32-unknown-unknown
trunk build --release
```

実際の出力（末尾）:

```text
$ cargo check --target wasm32-unknown-unknown
    Checking gpui-web-spike v0.1.0 (.../poc-wasm)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.83s

$ trunk build --release
    Finished `release` profile [optimized] target(s) in 0.28s
2026-11-14T11:56:01.070149Z  INFO applying new distribution
2026-11-14T11:56:01.072343Z  INFO ✅ success
```

初回のフルビルドは dev で約 63 秒、release で約 90 秒（依存 crate のコンパイル込み）。
ログ全文: `notes/assets/wasm-spike-cargo-check.log`、
`notes/assets/wasm-spike-trunk-build-release.log`。

生成物サイズ（release）:

| ファイル | 生サイズ | gzip |
|---|---|---|
| `*_bg.wasm` | 14 MB（14,570,620 bytes） | 3.6 MB（3,581,079 bytes） |
| `*.js`（wasm-bindgen glue） | 154 KB | 24 KB |
| `index.html` | 1.4 KB | - |

`data-wasm-opt="0"` のため wasm-opt 未適用。本番では wasm-opt（`-Oz`）と
gzip/brotli 配信でさらに削減できる見込み。

### 2. 配信とヘッドレス確認

```bash
cd ./poc-wasm/dist
python3 -m http.server 8090 --bind 127.0.0.1
```

ミッション指定のワンショット確認（`--virtual-time-budget` + `--screenshot`）:

```bash
google-chrome --headless=new --no-sandbox --disable-dev-shm-usage \
  --window-size=1200,800 --virtual-time-budget=8000 \
  --screenshot=/abs/path/wasm-spike-oneshot-virtual-time.png \
  --enable-logging=stderr --user-data-dir=/tmp/gpui-spike-chrome \
  "http://127.0.0.1:8090/"
```

このコマンドでも UI が描画された（`wasm-spike-oneshot-virtual-time.png`、1200x800）。

インタラクション検証（クリック・キー入力・ホイール・時系列の証拠取得）は CDP を使用:

```bash
# 別シェルで Chrome を起動
google-chrome --headless=new --no-sandbox --disable-dev-shm-usage \
  --window-size=1200,800 --remote-debugging-port=9222 \
  --user-data-dir=/tmp/gpui-spike-chrome about:blank

cd ./poc-wasm/scripts
python3 cdp_verify.py --url http://127.0.0.1:8090/ \
  --out ../../notes/assets --prefix wasm-spike --label headless-default-release
```

## Runtime Proof

### 1. 描画（スクリーンショット）

- `wasm-spike-01-initial.png`: ヘッダ、CPU/メモリ/クリック/HTTP fetch のカード、
  検索入力、プロセス表、フッターが描画されている。
- 日本語ラベル（システムモニタ、CPU 使用率、メモリ、プロセス名、ユーザー、
  スクロール検証用ダミー行など）がすべて表示されている。
- `wasm-spike-03-scrolled.png`: ホイールで表がスクロールし、行 4〜12 が表示。
- `wasm-spike-04-row-click.png`: 行 128 が選択色になり、クリック回数 1・選択中 PID 128。
- `wasm-spike-05-search-typed.png`: 検索欄に `http` を入力し、`HTTPクライアント` の
  1 行に絞り込まれている（CJK と ASCII の混在フィルタが動作）。
- `wasm-spike-06-search-backspace.png`: Backspace 2 回で `ht` になり 0 件表示。
- `wasm-spike-07-button-click.png`: 「クリックでカウント +1」でクリック回数 2。

### 2. 状態遷移の証拠（DOM 計装 + CDP）

アプリは検証用に `document.title` と `body[data-gpui-proof]` へ状態を書き出している。
CDP で読み取った時系列（`wasm-spike-evidence-headless-default-release.json`）:

| step | tick | clicks | query | selected_pid | http |
|---|---|---|---|---|---|
| initial | 7 | 0 | (空) | - | HTTP 200 |
| after_ticks（2.5 秒後） | 9 | 0 | (空) | - | HTTP 200 |
| scroll_down | 10 | 0 | (空) | - | HTTP 200 |
| row_click | 12 | 1 | (空) | 128 | HTTP 200 |
| search_typed（`http`） | 14 | 1 | http | 128 | HTTP 200 |
| search_backspace（2 回） | 15 | 1 | ht | 128 | HTTP 200 |
| button_click | 16 | 2 | ht | 128 | HTTP 200 |
| final | 18 | 2 | ht | 128 | HTTP 200 |

- **定期更新**: tick が約 1 秒ごとに増加（7 → 9 → 10 → 12 → 14 → 15 → 16 → 18）。
- **HTTP fetch**: 起動直後に `/stats.json` を取得し `HTTP 200` / body 242 bytes。
- **クリック**: 行クリックで `selected_pid=128` と `clicks=1`、ボタンで `clicks=2`。
- **キーボード**: `http` の入力と Backspace による削除が反映。
- **スクロール**: 表の表示行が変化（スクリーンショットで確認）。

### 3. DOM の証拠

CDP で取得した `document.documentElement.outerHTML` から:

```html
<canvas tabindex="-1" width="1200" height="657"
  style="width: 100%; height: 100%; display: block; ..."></canvas>
<textarea spellcheck="false" ... style="position: fixed; ... opacity: 0;
  font-size: 16px;"></textarea>
<!-- + safe-area 計測用 div 4 個 -->
```

- GPUI が `canvas` を 1 枚生成し `body` に追加している（レイアウト viewport は 1200x657）。
- `textarea` は Web プラットフォームの **IME ミラー**。
- canvas 以外の UI 要素は DOM に存在しない（すべて canvas 描画）。

### 4. コンソールログ（描画バックエンド）

Auto（既定）での起動時ログ（`wasm-spike-oneshot-console.log` ほか）:

```text
[INFO] gpui_web_spike: GPUI web spike: start()
[WARN] gpui_web::platform: WebGPU initialization failed; falling back to WebGL2:
       browser WebGPU probe did not return a usable adapter
[WARN] gpui_wgpu::wgpu_context: Dual-source blending not available on this GPU.
       Subpixel text antialiasing will be disabled.
[INFO] gpui_wgpu::wgpu_context: Browser graphics initialized: requested=WebGl,
       selected=Gl, adapter="ANGLE (Google, Vulkan 1.3.0
       (SwiftShader Device (Subzero) (0x0000C0DE)), SwiftShader driver)", ...
[INFO] gpui_web::platform: Browser graphics initialized successfully with Gl
[INFO] gpui_web_spike: アプリケーションフォントをロードしました (IBM Plex Sans/Mono)
[WARN] gpui_wgpu::wgpu_renderer: Failed to poll device during resize: Timeout
[INFO] gpui_web_spike: fetch /stats.json => status=HTTP 200, body_len=242
```

Chrome 側の警告:

```text
Automatic fallback to software WebGL has been deprecated. Please use the
--enable-unsafe-swiftshader flag to opt in to lower security guarantees
for trusted content.
```

- headless Chrome では WebGPU のプローブが「使用可能なアダプタなし」を返し、
  GPUI が WebGL2 にフォールバックして描画している。
- この環境の WebGL2 は ANGLE + SwiftShader（ソフトウェア）。
  dual-source blending 非対応のためサブピクセル AA は無効化される。
- 初回 resize 時の `Failed to poll device during resize: Timeout` は警告のみで、
  描画・操作に影響は見られなかった（要因は未特定）。
- 404 が 1 件出るが、これは favicon.ico の取得失敗でありアプリとは無関係。

### 5. ロード時間（localhost 参考値）

`performance` API による計測:

| リソース | 時間 | decoded |
|---|---|---|
| JS glue | 3 ms | 157,685 bytes |
| wasm | 46 ms | 14,570,620 bytes |
| stats.json | 2 ms | 242 bytes |

`domContentLoaded` 19 ms / `load` 20 ms（localhost・キャッシュなしの参考値。
実ネットワークでは wasm 14MB の転送とコンパイル時間が支配的になる）。

## capability 表（gpui-pre-web 0.3.12）

ソース（`~/.cargo/registry/src/index.crates.io-*/gpui-pre-web-0.3.12/src/`）と
本 PoC の実行結果に基づく。

| 機能 | 結果 | 根拠・備考 |
|---|---|---|
| WebGPU 描画 | △ | 強制時は初期化成功するが headless SwiftShader で device lost。実 GPU は未確認 |
| WebGL2 描画 | ○ | Auto 時のフォールバック先。headless では SwiftShader。本 PoC はこれで描画 |
| テキスト・レイアウト | ○ | taffy + cosmic-text。flex/table 風レイアウトを確認 |
| CJK 表示 | ○（条件付き） | `CanvasFontFallback::EmojiAndCjk` + フォント 1 種のロードが必須。既定 `Emoji` では不可 |
| スクロール | ○ | `overflow_y_scroll` + ホイールイベント。スクリーンショットで確認 |
| クリック | ○ | `on_click`。行選択・カウンタ増加を確認 |
| キーボード入力（ASCII） | ○ | `on_key_down` + `Keystroke.key_char`。`http` 入力と Backspace を確認 |
| 日本語 IME 入力 | 未確認 | `textarea` の IME ミラーはあるが、composition には `EntityInputHandler` 実装が必要 |
| 定期更新 | ○ | `cx.spawn` + `BackgroundExecutor::timer`。1 秒ごとの tick 増加を確認 |
| HTTP fetch | ○（同一オリジン） | `FetchHttpClient`。`/stats.json` を 200 で取得。CORS・認証は未確認 |
| クリップボード | 未確認（実装あり） | `platform.rs` にテキスト/画像の read/write 実装。secure context 前提 |
| マルチウィンドウ | × | トップレベルウィンドウは 1 個のみ（`AlreadyOpen` / `ReopeningUnsupported`） |
| ポップアップ/ダイアログ | × | `PopupNotSupportedError` / `UnsupportedWindowKind` |
| ファイルダイアログ | × | `prompt_for_paths` は "not supported on the web" |
| OS 統合（reveal/open with） | ×（no-op） | `reveal_path` などは no-op。`open_url` は `window.open` |
| マルチスレッド | 未確認（既定 off） | `multithreaded` feature は `wasm_thread`（nightly 必須） |
| アプリメニュー | ×（no-op） | Web ではメニュー未対応 |
| OS プロセス情報取得 | × | ブラウザ sandbox の制約。後述の設計含意を参照 |

補足（ソース確認）:

- `gpui_platform::application()` は wasm では
  `WebPlatform::new_with_backend(true, WebBackendPreference::Auto)` を返す。
  Canvas フォールバックは既定の `Emoji` で、HTTP クライアントは未設定
  （`Application::with_platform` の既定は `NullHttpClient`）。
- `Application::run` は wasm では即座に return し、app を `mem::forget` で保持する。
  エントリでは `gpui_platform::web_init()`（panic hook + console ログ）が必須。
- フォント DB は空で起動する。`cx.text_system().add_fonts(...)` で
  TTF をロードしてからウィンドウを開く必要がある。
- 描画は wgpu の WebGPU / WebGL2。`WebBackendPreference` で固定可能。
- CJK/絵文字の Canvas フォールバックは「1 つの拡張書記素単位」の水平描画のみ対応。
  シェーピングや折り返しの品質はネイティブフォントに劣る。

## 描画バックエンドの切り分け

同じバイナリで `?backend=webgpu` / `?backend=webgl` を切り替えられるようにして検証した。

| 条件 | 起動 | 描画 | ログ |
|---|---|---|---|
| Auto（既定） | ○ | ○ | WebGPU プローブ失敗 → WebGL2 へフォールバック |
| `?backend=webgl` | ○ | ○ | `requested=WebGl, selected=Gl`（SwiftShader） |
| `?backend=webgpu`（Chrome フラグなし） | × | × | アダプタなし。canvas は生成されず、DOM にエラー文を表示 |
| `?backend=webgpu` + `--enable-unsafe-webgpu --use-webgpu-adapter=swiftshader --enable-unsafe-swiftshader` | ○ | **×** | 初期化成功（`BrowserWebGpu`）後に device lost |

- WebGPU アダプタが無い場合、GPUI は panic せず `show_graphics_unavailable_message` で
  DOM にエラー文を表示して停止する（`wasm-spike-webgpu-noadapter-01.png`）:

  ```text
  Failed to initialize browser graphics: No browser graphics backend could be
  initialized. Only WebGPU was tried because the application requested it
  explicitly. WebGPU failure: Failed to request a WebGpu adapter compatible with
  the canvas: No suitable graphics adapter found; ... webgpu found no adapters
  ```

- `?backend=webgl` は Auto と同じ WebGL2 経路で安定して描画する
  （`wasm-spike-webgl-forced-01.png`、`wasm-spike-backend-variants.json`）。

強制 WebGPU 時のログ（`wasm-spike-webgpu-forced-evidence-*.json`）:

```text
[INFO] gpui_web::platform: Browser graphics initialized successfully with BrowserWebGpu
[WARN] WebGPU canvas configured with a different format than is preferred by this device
       ("rgba8unorm"). This requires an extra copy, which may impact performance.
[WARN] A valid external Instance reference no longer exists.
[ERROR] gpui_wgpu::wgpu_context: wgpu device lost: reason=Unknown,
        message=A valid external Instance reference no longer exists.
[ERROR] gpui_wgpu::wgpu_renderer: Browser graphics context was lost;
        rendering has stopped. Reload the page to recover.
```

- 描画は停止し canvas は背景色のみ（`wasm-spike-webgpu-forced-01-initial.png`）。
  一方、タイマー・クリック・fetch などのアプリロジックは動作し続けた。
- 原因は headless Chrome + SwiftShader の WebGPU 実装（wgpu 30 / Chrome 157 の
  組み合わせ）と推測。実 GPU を積んだ通常の Chrome では未検証。
- **本番では WebGPU を必須にせず、既定の Auto（WebGPU → WebGL2）を推奨**。
  CI の headless 検証も WebGL2 前提（必要なら `--enable-unsafe-swiftshader`）にする。

## CJK / フォント

- Web プラットフォームのテキストは cosmic-text がネイティブにシェーピングするが、
  フォント DB は空で起動する。本 PoC は IBM Plex Sans / Mono を `include_bytes!` で
  埋め込み `add_fonts` している。
- CJK は読み込んだフォントにグリフが無いため、`CanvasFontFallback::EmojiAndCjk` により
  Canvas 2D でブラウザのシステムフォントを使って描画される。
  デバッグビルドでは `cosmic_text::font::fallback: Failed to find any fallback for
  [Han] ...` が大量に出るが、これは想定どおり（Canvas 側で代替される）。
- 本番で品質を求める場合の選択肢:
  1. Noto Sans JP などの CJK フォントをサブセット化して埋め込み、native shaping させる
     （推奨。ただしフォントライセンスとサイズの確認が必要）
  2. Canvas フォールバックを継続（手軽。シェーピング・選択・折り返しは近似）
- 既定の `CanvasFontFallback::Emoji` のままでは CJK は描画されない点に注意。

## 本番 web シェルへの推奨構成

1. **依存とピン**
   - `gpui = { package = "gpui-pre", version = "=0.3.12", default-features = false }`
   - `gpui_platform = { package = "gpui-pre-platform", version = "=0.3.12", default-features = false }`
   - `gpui_web = { package = "gpui-pre-web", version = "=0.3.12", default-features = false }`
   - `Cargo.lock` をコミットし、`=` 固定で更新を管理する（gpui-pre は API 破壊が頻繁）。
   - `gpui-kit` / `gpui-component` は `wasm_thread`（nightly 必須）を引くため
     stable wasm では使わない。UI は gpui 素の `div` で自作する。
2. **初期化を 1 箇所に閉じ込める**
   - `WebPlatform::new_with_backend_and_font_fallback(true, Auto, EmojiAndCjk)` を構築
   - `platform.fetch_http_client()` を `Application::with_platform(...).with_http_client(...)`
     に注入（既定は `NullHttpClient` なので fetch が失敗する）
   - `gpui_platform::web_init()` を必ず呼ぶ
   - エントリは cdylib + `#[wasm_bindgen(start)]`
3. **フォント**
   - Latin: OFL フォントを埋め込み `add_fonts`
   - CJK: Noto Sans JP サブセット埋め込み（native shaping）を第一候補、
     Canvas フォールバックをフォールバック手段として併用
4. **データ取得**
   - ブラウザから OS メトリクスは取れない。ローカル agent（Rust）が
     HTTP/SSE/WebSocket でメトリクスを配信し、web シェルは `FetchHttpClient` で取得する。
   - 同一オリジンにリバースプロキシで同居させるか、CORS を明示設定する。
   - 定期更新は `cx.spawn` + `BackgroundExecutor::timer` のポーリングで十分
     （本 PoC は 1 秒間隔で実績）。高頻度が必要なら SSE/WebSocket を検討。
5. **ビルド/配信**
   - trunk + wasm-bindgen。release ビルド + wasm-opt（`-Oz`）+ gzip/brotli 配信。
   - `Content-Type: application/wasm` を返すサーバで配信する
     （Python 3.14 の `http.server` は application/wasm を返すことを確認）。
   - `multithreaded` を使わない限り COOP/COEP は不要。
6. **UI 制約の織り込み**
   - トップレベルウィンドウは 1 個。ポップアップ/ダイアログ不可。
     モーダルは同一ウィンドウ内のレイヤーとして実装する。
   - OS 統合（ファイルダイアログ、reveal、メニュー）は使わない。
   - IME 入力が必要な検索欄は `EntityInputHandler` の実装が必要（未検証）。

## ブラウザから OS プロセスを読めない制約への設計含意

- ブラウザ sandbox から `/proc` 等は読めない。CPU/メモリ/プロセス一覧の取得は
  **必ずサーバ側（ローカル agent）** で行う必要がある。
- 推奨アーキテクチャ:
  - ローカル agent（Rust、デスクトップ側）がメトリクス API を提供
    （例: `/api/stats`, `/api/processes`、SSE/WS で差分配信）。
  - web シェル（GPUI wasm）は `FetchHttpClient` でポーリングし、表を描画する。
  - 操作（kill 等）は agent API 経由 + 認可（トークン/ローカル限定バインド）。
- 本 PoC の `/stats.json` 取得は、この構成の疎通確認に相当する
  （同一オリジン fetch が GPUI の HTTP クライアントで動くことを確認済み）。
- 完全なデモ用途なら、ブラウザ内で疑似データを生成する本 PoC の方式でも成立する。

## 未確認事項

1. 実 GPU（非 headless Chrome / Firefox / Safari）での WebGPU 描画と device lost の再現性。
2. 日本語 IME 入力（`EntityInputHandler` 実装が必要）。変換・確定・未確定文字列の表示。
3. CJK フォント（Noto Sans JP サブセット等）を埋め込んだ native shaping の画質・性能。
4. クリップボードの読み書き（secure context、権限プロンプト）。
5. CORS 越しの fetch、認証付き fetch、SSE/WebSocket の gpui-pre サポート範囲。
6. マルチスレッド（nightly + wasm_thread + COOP/COEP）の実用性と性能。
7. 数千行規模のリスト描画・スクロール性能（`uniform_list` / `list` の web 挙動）。
8. SVG・画像（`img` 要素）の web 描画。
9. WebGPU device lost の原因（wgpu 30 + Chrome 157 headless の組み合わせか、
   アプリ側の使い方か）。
10. Safari / Firefox での WebGL2・フォントフォールバックの挙動。
11. wasm-opt 適用時のサイズ・起動時間（本 PoC は未適用）。
12. モバイル（タッチ・ソフトキーボード）での操作。

## 参考

- 外部プロジェクトの調査ノート（gpui-pre の wasm コンパイル、
  gpui-kit が stable wasm 不可である既存調査）
- 外部プロジェクトの調査ノート（gpui-pre の来歴・ライセンス・
  feature flags・Linux ビルド要件）
- gpui-pre 0.3.12 README（`~/.cargo/registry/src/index.crates.io-*/gpui-pre-0.3.12/README.md`）
- gpui-pre-web 0.3.12 ソース
  （`~/.cargo/registry/src/index.crates.io-*/gpui-pre-web-0.3.12/src/`）:
  `platform.rs`（capabilities）、`window.rs`（canvas/IME ミラー）、
  `text_system.rs`（Canvas フォールバック）、`http_client.rs`（FetchHttpClient）
- gpui-pre-wgpu 0.3.12 ソース: `wgpu_context.rs`（`WebBackendPreference`、
  WebGPU/WebGL 選択とエラーメッセージ）
- 実装コードは本スパイク用の自作。AGPL プロジェクトのソースはコピーしていない。

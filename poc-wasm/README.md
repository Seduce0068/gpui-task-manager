# poc-wasm — GPUI Web バックエンドのスパイク

gpui-pre 0.3.12（Zed GPUI の再公開スナップショット、Apache-2.0）を
wasm32-unknown-unknown にビルドし、ブラウザ上でシステムモニタ風 UI を描画する PoC。
実装コードはすべて本スパイク用の自作。AGPL プロジェクトのソースはコピーしていない。

検証結果・制約・推奨構成は `../notes/wasm-spike.md` にまとめてある。

## 検証対象

- テキスト・表・スクロール・検索入力（キーボード）・クリック・1 秒定期更新
- 同一オリジン HTTP fetch（gpui-pre-web の `FetchHttpClient`）
- CJK（日本語）表示
- 描画バックエンド（WebGPU / WebGL2）の選択と headless Chrome での可否

## 構成

| パス | 内容 |
|---|---|
| `src/lib.rs` | アプリ本体（`#[wasm_bindgen(start)]` エントリ、UI、タイマー、fetch） |
| `Cargo.toml` | 独立 crate（`[workspace]` で親 workspace に入らない） |
| `index.html` | trunk 用エントリ（`data-trunk rel="rust"`） |
| `Trunk.toml` | trunk 設定（port 8090） |
| `public/stats.json` | fetch 検証用のダミー API |
| `assets/fonts/` | IBM Plex Sans / Mono（SIL OFL 1.1、`LICENSE-IBM-Plex.txt` 同梱） |
| `scripts/cdp_verify.py` | ヘッドレス Chrome を CDP 操作して Runtime Proof を取るスクリプト |

## ビルド

```bash
cd poc-wasm
rustup target add wasm32-unknown-unknown   # 導入済みなら不要
cargo check --target wasm32-unknown-unknown
trunk build --release                      # dist/ に index.html + JS + wasm を生成
```

- trunk 0.22.1 / wasm-bindgen CLI は trunk が自動取得（本検証では 0.2.130）。
- `data-wasm-opt="0"` のため wasm-opt は未適用（release wasm 14MB / gzip 3.6MB）。
- `default-features = false` で `multithreaded`（nightly + wasm_thread 必須）を無効化している。

## 配信と確認

```bash
cd dist
python3 -m http.server 8090 --bind 127.0.0.1
```

- 通常表示: <http://127.0.0.1:8090/>
- バックエンド固定（検証用）: `?backend=webgpu` / `?backend=webgl`

## ヘッドレス検証

```bash
# Chrome を CDP 有効で起動（別シェル）
google-chrome --headless=new --no-sandbox --disable-dev-shm-usage \
  --window-size=1200,800 --remote-debugging-port=9222 \
  --user-data-dir=/tmp/gpui-spike-chrome about:blank

# 描画・クリック・キー入力・スクロール・定期更新・fetch の証拠を取得
cd scripts
python3 cdp_verify.py --url http://127.0.0.1:8090/ --out ../../notes/assets \
  --prefix wasm-spike --label headless-default-release
```

スクリーンショットと `*-evidence-*.json`（コンソールログ、DOM、状態フック）は
`notes/assets/` に保存される。

## 検証用フック

- `document.title` と `body[data-gpui-proof]` に tick / CPU / メモリ / クリック数 /
  検索文字列 / 選択 PID / HTTP ステータスを書き出す（CDP から観測するための計装）。
- アプリの実装は GPUI の API（`div`, `on_click`, `on_key_down`, `overflow_y_scroll`,
  `cx.spawn`, `BackgroundExecutor::timer`, `HttpClient`）のみで構成している。

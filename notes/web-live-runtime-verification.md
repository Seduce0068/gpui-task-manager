---
type: Report
title: "Web シェルの実 tm-agent / SSE Runtime Proof"
description: "ヘッドレス Chrome と CDP で Web シェルのライブ接続・SSE スナップショット更新・エージェント停止時の挙動を検証した記録"
tags:
  - runtime-proof
  - web-shell
  - sse
  - tm-agent
  - loopback
generated:
  by: "g1-live"
  at: "2026-11-19T20:17:42+09:00"
verified:
  by: "g1-live"
  at: "2026-11-19T20:17:42+09:00"
status: approved
---

## 概要

Web シェルが実 `tm-agent` に接続し、SSE ストリーム経由でスナップショットが
更新されることを Runtime Proof として確認した。`body[data-gpui-proof]` の
`mode=live` / `sse=active` / `tick` 増加 / `rows>0` を観測し、
エージェント停止後は `sse=fallback` へ遷移するが `mode=live` は維持され、
デモへはフォールバックしないことを確認した。トークン値は出力・記録していない。

更新（2026-11-20）: デモデータ set の変更により現行デモ件数は 30 行となった
（ka-recovery が実測確認）。本文の行数は現行デモ件数 30 行へ更新済みで、旧 28 行は
更新前のデモデータ set による観測値である。30 行への増加はデモデータの変更による
もので、回帰ではない。

## 検証環境

- 実行日時: `2026-11-19T20:13:24+09:00` に `date` で確認、文書作成は
  `2026-11-19T20:17:42+09:00`。
- ビルド: `cargo build --locked -p tm-agent`（終了コード 0、
  workspace manifest の未使用フィールド警告 2 件のみ）。
- Web 配布物: 既存の `web/dist` を再利用。生成時刻が `web/src/` と
  `web/index.html` のいずれより後であることを確認したため、再ビルドは不要。
- エージェント: ループバックの `127.0.0.1:18911`、`--demo --redact`、
  トークンは一時ディレクトリの 0600 ファイル経由で渡した。
- 静的サーバ: ループバックの `127.0.0.1:8093`。
- ブラウザ: headless Chrome（CDP 9333、viewport 1440x1000）。
- CDP 操作: Python の `websockets` 経由で `Runtime` / `Page` / `Network` を有効化し、
  `Runtime.evaluate` で DOM proof を最大 15 秒ポーリングした。

## 確認手順

### 起動

一時ディレクトリに `openssl rand -hex 32` で生成したトークンを 0600 で保存し、
値はシェル変数と CDP の URL 組み立てにのみ使用した。URL は CDP の
`Page.navigate` で渡し、ブラウザのコマンドラインには含めなかった。

```bash
cargo build --locked -p tm-agent
umask 077
token_file="$(mktemp)"
printf '%s\n' "$(openssl rand -hex 32)" > "$token_file"
chmod 600 "$token_file"
./target/debug/tm-agent --token-file "$token_file" \
  --listen 127.0.0.1:18911 --demo --redact &
python3 -m http.server 8093 --bind 127.0.0.1 --directory web/dist &
google-chrome --headless=new --no-sandbox --disable-dev-shm-usage \
  --window-size=1440,1000 --remote-debugging-port=9333 \
  --user-data-dir="$tmpdir/chrome" about:blank &
```

- エージェントの `/healthz` が 200 を返すまで待機して ready とした。
- 静的サーバの `index.html` と CDP の `/json/version` も到達確認した。
- トークンファイルのモードは `600` であることを確認した。

### DOM proof の検証

`http://127.0.0.1:8093/?agent=http://127.0.0.1:18911#token=<値>` を開き、
`body[data-gpui-proof]` を 0.4 秒間隔でポーリングした。主要な時系列は
相対時刻で次のとおり（トークンは含めない）。

| 相対時刻 | mode | sse | tick | rows | total | live_error |
| --- | --- | --- | --- | --- | --- | --- |
| 1.11 秒 | live | fallback | 0 | 0 | 0 | 0 |
| 1.86 秒 | live | active | 2 | 30 | 30 | 0 |
| 2.36 秒 | live | active | 2 | 30 | 30 | 0 |

- 接続確立直後は `sse=fallback` だが、最初のスナップショットフレームで
  `sse=active` へ遷移した（仕様どおり、購読開始だけでは active にならない）。
- `tick` は 0 → 2 に進行し、`rows=30` / `total=30` を確認した。
- `Network.requestWillBeSent` で `/api/v1/stream` へのリクエストを確認し、
  `token=` を含む URL やトークン値を含む URL は 0 件だった（計 8 リクエスト）。
- コンソール 7 件、JS 例外 0 件。panic / RefCell error / トークン値の
  言及はいずれも 0 件だった。

### エージェント停止後

シェルからエージェントへ SIGTERM を送り、終了を確認してから同じページの
DOM proof を再ポーリングした。

| 相対時刻 | mode | sse | tick | rows | live_error | stale |
| --- | --- | --- | --- | --- | --- | --- |
| 0.0 秒 | live | fallback | 23 | 30 | 0 | 0 |
| 0.8 秒 | live | fallback | 23 | 30 | 1 | 1 |

- `sse=fallback` への遷移を確認した。
- `mode=live` は維持され、`mode=demo` は一度も観測されなかった。
- rows は 30 のまま（直前のスナップショットを保持）、
  その後の定期取得失敗で `live_error=1` / `stale=1` になった。
- 停止後のコンソール 8 件、JS 例外 0 件。panic / RefCell error /
  トークン値の言及はいずれも 0 件だった。

### 停止と cleanup

agent / 静的サーバ / Chrome の順に SIGTERM し、必要なら SIGKILL で停止を確認した。
残存プロセススキャンで該当プロセスが 0 件であることを確認し、
ポート 18911 / 8093 / CDP ポートのいずれも空きであることを `curl` で確認した。
トークンファイルと Chrome プロファイルを含む一時ディレクトリは削除した。

## 確認結果

| 確認項目 | 期待 | 観測 | 結果 |
| --- | --- | --- | --- |
| ビルド | 成功 | 終了コード 0（警告 2 件） | OK |
| 起動モード | `mode=live` | 全サンプルで live | OK |
| SSE transport | `sse=active` | 初回フレーム後に active | OK |
| スナップショット更新 | tick 進行・rows>0 | 0 → 2、rows=30 | OK |
| SSE エンドポイント | `/api/v1/stream` を使用 | リクエスト確認 | OK |
| トークン露出 | URL・コンソール・証拠に無し | すべて 0 件 | OK |
| エージェント停止後 | `sse=fallback` | 遷移を確認 | OK |
| デモへのフォールバック | しない | `mode=demo` は 0 件 | OK |
| プロセス・ポート cleanup | 全停止・空き | 残存 0 件、全ポート空き | OK |

## 機密情報の扱い

- トークンは実行時に生成して 0600 ファイルに保存し、`--token-file` で渡した。
- 値は端末出力・ログ・スクリーンショット・証拠 JSON のいずれにも残していない
  （文字列一致は真偽のみを記録）。
- 証拠 JSON にはトークン値、生のホスト名、内部パスを記載していない。
- エージェントは `--redact` で起動し、スナップショットの識別情報はマスクされている。

## 制約・未確認事項

- `--demo` エージェントを使用したため、データは合成値であり、
  ネイティブの real probe はこの検証では扱っていない。
- SSE active の判定は DOM proof の `sse=` フィールドと
  `Network.requestWillBeSent` に基づく。フレーム本文の改ページは行っていない。
- headless Chrome の描画 backend は既定（環境により WebGL2 フォールバック）であり、
  実 GPU の WebGPU 描画は未検証。
- エージェント停止後の再接続（SSE_RECONNECT_DELAY）は 15 秒の観測窓を超えるため、
  この検証では確認していない。

## 成果物と検証

- 証拠 JSON: `notes/assets/web-live-evidence.json`
- ドキュメント: `notes/web-live-runtime-verification.md`
- 検証コマンド: `markdownlint notes/web-live-runtime-verification.md` と
  `bash scripts/check-public-docs.sh` を実行し、両方成功することを確認した。

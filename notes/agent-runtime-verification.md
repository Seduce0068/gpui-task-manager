---
type: Report
title: "tm-agent 実動検証（R1 分析 / R2 Runtime Proof）"
description: "tm-agent を実起動し TCP ソケット経由で主要エンドポイントを確認した結果と制約"
tags:
  - runtime-proof
  - tm-agent
  - loopback
  - authentication
  - verification
generated:
  by: "f1-agent-live"
  at: "2026-11-19T19:53:11+09:00"
verified:
  by: "f1-agent-live"
  at: "2026-11-19T19:53:11+09:00"
status: approved
---

## 概要

読み取り専用ループバックエージェント `tm-agent` を実起動し、TCP ソケット経由で
主要エンドポイント（`/healthz`、`/metrics`、`/api/v1/snapshot`、
`/api/v1/stream`）の挙動を確認した。全項目で期待値どおりの結果を得た。
トークン値はログ・成果物のいずれにも残していない。

## 検証環境

- 日時: `2026-11-19T19:50:32+09:00` に `date --iso-8601=seconds` で確認。
- 対象: `tm-agent 0.1.0`（ワークスペースの Cargo ビルド出力）。
- リッスン先: `127.0.0.1:18901`（ループバックのみ）。
- モード: `--demo --redact`（決定的な合成データ、識別情報マスク）。

## 確認手順

### ビルド

```bash
cargo build --locked -p tm-agent
```

終了コード 0。ワークスペースmanifestの未使用フィールドに対する警告 2 件のみで、
コンパイルエラーはなし。

### トークン作成と起動

一時ディレクトリに `openssl rand -hex 32` で生成したトークンを 0600 で保存し、
`--token-file` 経由で渡した。値は端末・ログに出力していない。

```bash
umask 077
token_file="$(mktemp)"
printf '%s\n' "$(openssl rand -hex 32)" > "$token_file"
chmod 600 "$token_file"
tm-agent --token-file "$token_file" --listen 127.0.0.1:18901 --demo --redact &
```

- 起動前に `curl` でポート 18901 が空きであることを確認した。
- トークンファイルのモードは `600` であることを確認した。
- 起動後、`/healthz` が 200 を返すまで待機して ready と判定した。
- 構造化ログに `agent.listening` が 1 件出力されることを確認した。

### エンドポイント確認

Bearer 付きリクエストでは、トークン値をシェル変数からヘッダに渡し、
値そのものは表示していない。SSE は `curl -N --max-time 3` で受信した。

```bash
curl -sS -D headers -o body -w '%{http_code}' http://127.0.0.1:18901/healthz
curl -sS -D headers -o body -w '%{http_code}' http://127.0.0.1:18901/metrics
curl -sS -D headers -o body -w '%{http_code}' 'http://127.0.0.1:18901/api/v1/snapshot?limit=5'
curl -sS -D headers -o body -w '%{http_code}' -H "Authorization: Bearer $token_value" \
  'http://127.0.0.1:18901/api/v1/snapshot?limit=5'
curl -sS -D headers -o body -w '%{http_code}' -X POST \
  http://127.0.0.1:18901/api/v1/snapshot
curl -sS -D headers -o body -w '%{http_code}' -X OPTIONS \
  -H 'Origin: http://127.0.0.1:8091' http://127.0.0.1:18901/api/v1/stream
curl -sS -N --max-time 3 -D headers -o body -w '%{http_code}' \
  -H "Authorization: Bearer $token_value" http://127.0.0.1:18901/api/v1/stream
curl -sS -D headers -o body -w '%{http_code}' -H 'Host: evil.example' \
  http://127.0.0.1:18901/healthz
```

### 停止と cleanup

検証後にエージェントプロセスを SIGTERM で停止し、終了を確認したうえで
残存プロセスがないことを確認した。最後に `curl` でポート 18901 への接続が
失敗することを確認し、ポートが空いていることを確かめた。一時ディレクトリも削除した。

## 確認結果

| 確認項目 | 期待 | 観測 | 結果 |
| --- | --- | --- | --- |
| `GET /healthz` | 200 | 200、`{"ok":true,"schema_version":1}` | OK |
| `GET /metrics` | 200、ゲージ含有 | 200、`tm_agent_responses_total` と `tm_agent_stream_sessions` を確認 | OK |
| `GET /api/v1/snapshot?limit=5`（トークン無し） | 401 + `unauthorized` | 401、`code=unauthorized`、`WWW-Authenticate: Bearer` | OK |
| 同上（Bearer あり） | 200、redact 済み | 200、`total_process_count=28`、返却 5 件、`hostname=host`、全ユーザー `user` | OK |
| `POST /api/v1/snapshot` | 405 + `Allow` | 405、`Allow: GET, OPTIONS`、`code=method_not_allowed` | OK |
| `OPTIONS /api/v1/stream`（Origin `http://127.0.0.1:8091`） | 204 + CORS | 204、5 種類の CORS ヘッダを確認 | OK |
| `GET /api/v1/stream`（Bearer あり） | 200 + `text/event-stream` | 200、`text/event-stream; charset=utf-8`、`event: snapshot` を 3 フレーム受信 | OK |
| 不正 Host（`evil.example`） | 403 | 403、`code=forbidden` | OK |
| 停止後ポート確認 | 空き | 2 実行とも接続失敗（空き）を確認 | OK |

`/metrics` のゲージは `tm_agent_responses_total{class="2xx"} 2`（計測時点）と
`tm_agent_stream_sessions 0` であり、カウンタとゲージが Prometheus テキスト形式で
公開されていることを確認した。

CORS ヘッダは `Access-Control-Allow-Origin`、`Access-Control-Allow-Methods`、
`Access-Control-Allow-Headers`、`Access-Control-Max-Age`、`Vary` の 5 種類を確認した。
Origin はリクエストの `http://127.0.0.1:8091` がそのまま反映され、ワイルドカードでは
ない。

## 機密情報の扱い

- トークンは実行時に生成して 0600 ファイルへ保存し、`--token-file` で渡した。
- 値は一切出力せず、記録したのは SHA-256 先頭 12 文字のみ（初回実行分）。
- スナップショット本文・SSE 本文・サーバ stderr にトークン値が含まれないことを
  文字列一致（真偽のみ）で確認した。
- redact の効果として、ホスト名は `host`、プロセスユーザーはすべて `user` に
  置換されていた。実行環境のホームパス・ユーザー名・ホスト名が本文に残って
  いないことも真偽のみで確認した。
- 証拠 JSON と本文にはトークン値も生の内部パスも記載していない。

## 制約・未確認事項

- `--demo` のため、データは合成値であり、ネイティブの `/proc` プローブは
  この実行では検証していない。
- シグナルハンドラによる graceful shutdown は実装されていないため、停止は
  SIGTERM で実施した。
- ブラウザ側の CORS 判定は行わず、レスポンスヘッダのみを確認した。
- SSE のフレーム間隔は 1 秒であり、timeout 3 秒では 3 フレームを受信した
  （想定の 1〜2 フレームを上回るが、期待するストリーム動作は確認できた）。
- `/metrics` のカウンタはプロセスローカルであり、再起動で初期化される。

## 成果物と検証

- 証拠 JSON: `notes/assets/agent-runtime-evidence.json`
- ドキュメント: `notes/agent-runtime-verification.md`
- 検証コマンド: `markdownlint-cli2 notes/agent-runtime-verification.md` と
  `bash scripts/check-public-docs.sh` を実行し、両方成功することを確認した。

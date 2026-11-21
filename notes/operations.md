---
type: Playbook
title: 運用 Runbook（メトリクス・リリース・復元）
description: tm-agent の /metrics 意味と推奨アラート、リリース時バージョン更新手順、破損設定の復元手順
tags: [operations, metrics, release, recovery]
---

## メトリクス

`tm-agent` は `GET /metrics` で Prometheus テキストを公開します。エンドポイントは
認証不要ですが、エージェント自身が loopback（`127.0.0.1` / `[::1]`）のみに bind
するため、外部の監視基盤から直接スクレイプできません。

| 名前 | 種別 | 意味 |
| --- | --- | --- |
| `tm_agent_requests_total` | counter | 処理した HTTP リクエスト総数 |
| `tm_agent_responses_total`（`class` ラベル: `2xx`, `3xx`, `4xx`, `5xx`, `unknown`） | counter | ステータスクラス別のレスポンス数 |
| `tm_agent_active_connections` | gauge | 受け付け中の接続数（`MAX_CONNECTIONS` が上限） |
| `tm_agent_stream_sessions` | gauge | アクティブな SSE ストリーム数（`MAX_STREAMS=2` が上限） |
| `tm_agent_uptime_seconds` | gauge | プロセス起動からの経過秒 |

推奨アラート:

- エラー率: `5xx / requests_total` の継続的な上昇。容量拒否（503）は 5xx に
  集約されるため、下記のゲージと併せて原因を切り分ける。
- SSE 枯渇: `tm_agent_stream_sessions >= 2` が継続した場合、新しいブラウザ接続は
  即時 503 となりポーリングへフォールバックする。
- 接続枯渇: `tm_agent_active_connections` が `MAX_CONNECTIONS` 付近で張り付いた場合。

いずれのカウンタもプロセスローカルで、再起動時に初期化されます。外部監視へは
ループバック上のスクレイパを配置して relay するか、既存のローカルエクスポータを
利用してください（bind 先の拡張はセキュリティ設計変更に該当するため、安易に行いません）。

## リリース時バージョン更新

更新対象は次の 3 箇所です。いずれかがずれると DEB のバージョン一致検証や
公開物の表示が壊れます。

1. `Cargo.toml` の `[workspace.package] version`
2. `web/Cargo.toml` の `version`
3. `README.md` 冒頭の製品バージョン

更新後、`bash packaging/linux/build-deb.sh` が cargo metadata から取得した
バージョンとバイナリの `--version` を照合することを確認し、成果物一覧と
`SHA256SUMS`、SBOM を揃えて公開します。差し戻しは公開タグ／成果物を前版へ
戻し、README のバージョン表記も同時に戻します（現状、DEB への GPG 署名と
リポジトリホスティングは未実施です）。

## 破損設定の復元

保存先と退避ファイル名、最新の判定基準は `README.md` の「破損設定の退避と
手動復元」を参照してください。要点は次のとおりです。

- 「最新」はファイル名末尾の 16 桁 hex タイムスタンプ順であり、mtime 順ではありません。
- 退避物は破損を検出した値そのものです。そのまま現行キーへ戻すと再読込時に
  再度 corrupt 経路へ送られ既定値に縮退するため、必要なフィールドだけを抽出し、
  型不正なフィールドは既定値へ置き換えてから現行 JSON へマージしてください。

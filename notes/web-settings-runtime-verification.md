---
type: Report
title: "Web 設定永続化の Runtime Proof（localStorage 保存・再読込・破損退避・復元）"
description: "headless Chrome と CDP で Web 設定の保存・再読込・破損退避・復元を実ブラウザ検証した記録"
tags:
  - runtime-proof
  - web-settings
  - local-storage
  - cdp
  - loopback
generated:
  by: "h1-settings"
  at: "2026-11-19T21:02:06+09:00"
verified:
  by: "h1-settings"
  at: "2026-11-19T21:03:39+09:00"
status: approved
---

## 概要

現在の Web ビルドについて、設定の永続化が実ブラウザで動作することを
Runtime Proof として確認した。既定値起動、列ヘッダ操作による `localStorage` 保存、
フィクスチャ書き込み後の再読込による復元、テーマ切替の双方向反映、
および破損設定の退避と既定値起動のすべてが成功した。
公式スクリプト `web/scripts/cdp_verify_settings.py` は
`{"passed": true, "failures": []}`（終了コード 0）を返し、
破損設定の退避は別途 CDP 直接操作で 9 項目すべてを確認した。

## 検証環境

- 実行日時: `date` で `2026-11-19T20:58:35+09:00` を確認。スクリプトの
  evidence は `2026-11-19T20:59:39+0900` 開始、
  `2026-11-19T20:59:51+0900` 終了。文書作成は
  `2026-11-19T21:02:06+09:00`。
- Web 配布物: 既存の `web/dist`（`index.html`、
  `tm-web-5133422a3c1eab30.js`、`tm-web-8de3cad71d51505_bg.wasm`）を再利用。
  再ビルドは不要だった。
- 静的サーバ: `python3 -m http.server 8094 --bind 127.0.0.1`。
  HTTP ステータス 200 を確認。
- ブラウザ: headless Chrome `158.0.8127.36`。
  `--headless=new --no-sandbox --disable-dev-shm-usage --window-size=1440,1000`
  `--remote-debugging-port=9341 --user-data-dir=/tmp/tm-settings-chrome`
  `--force-device-scale-factor=1`。
- CDP: Python `websockets` 15.0.1 経由で `Page` / `Runtime` を有効化し、
  `Runtime.evaluate` で `body[data-gpui-proof]` を最大 20 秒ポーリング。
- モード: デモモード（`mode=demo`、`sse=fallback`）。SSE / ライブ接続は
  別担当の検証範囲のため、ここでは変更していない。

## 確認手順

### 起動

```bash
cd web/dist && python3 -m http.server 8094 --bind 127.0.0.1 &
google-chrome --headless=new --no-sandbox --disable-dev-shm-usage \
  --window-size=1440,1000 --remote-debugging-port=9341 \
  --user-data-dir=/tmp/tm-settings-chrome --force-device-scale-factor=1 \
  about:blank &
```

起動後に `http://127.0.0.1:8094/` が 200 を返すことと、
CDP の `/json/version` が応答し page target が 1 つ存在することを確認した。

### 設定永続化スクリプト

```bash
python3 web/scripts/cdp_verify_settings.py \
  --port 9341 \
  --url http://127.0.0.1:8094/ \
  --evidence notes/assets/web-settings-evidence.json
```

実行結果は `evidence: notes/assets/web-settings-evidence.json` と
`{"passed": true, "failures": []}`、終了コード 0 だった。
スクリプトは以下を順に実行する。

1. 保存済み設定を削除して再読込し、既定テーマ dark を確認する。
2. DOM から取得した列座標で CPU ヘッダをクリックし、`localStorage` への
   保存とスキーマ必須キー、トークン非含有を確認する。
3. 復元用設定を書き込んで再読込し、ソート・方向・テーマ・更新間隔が
   復元されることを確認する。
4. テーマ切替ボタンで Light → Dark → Light を操作し、状態フックと
   保存値の両方を確認する。

### 破損設定の追加検証

公式スクリプトには破損設定のケースが含まれないため、既存 CDP セッションへ
インラインの Python  heredoc で接続し、以下の操作を追加で実行した
（スクリプトファイルは作成していない）。

1. `gpui-task-manager.settings*` のキーをすべて削除してから対象 URL を開く。
2. `gpui-task-manager.settings` に `{ not json` を書き込んで再読込する。
3. `body[data-gpui-proof]` と `#gpui-settings-notice`、
   `localStorage` のキー一覧と保存値を `Runtime.evaluate` で取得する。
4. 既定値起動、Notice 表示と情報漏えい対策（redaction）、退避キー生成、
   退避物の元バイト保持、既定値の保存を 9 項目で判定する。

## 結果

### スクリプトのチェック結果

| ステップ | 観測値 | 判定 |
| --- | --- | --- |
| fresh | `theme=dark`、`interval=1000`、`sort=CPU`、`dir=desc` | 合格 |
| saved | `version=2`、`interval_ms=1000`、`sort=Cpu/Ascending`、`theme=Dark` | 合格 |
| saved_parsed | 必須キー 6 個すべて揃い、`token` 文字列なし | 合格 |
| restored | `sort=名前`、`dir=asc`、`theme=light`、`interval=5000` | 合格 |
| toggled_dark | 状態フック `theme=dark`、保存値 `theme=Dark` | 合格 |
| toggled_light | 状態フック `theme=light`、保存値 `theme=Light` | 合格 |

保存キーは `gpui-task-manager.settings`。デモデータは `rows=28` で、
ヘッダ座標は DOM の `getBoundingClientRect` と viewport 1440x857 から取得した。
`layout` の値は CDP 由来の数値のみで、内部パスや識別子は含まない。

### 破損退避の検証結果

9 項目すべてが `true`、`all_passed: true` だった。

- アプリが起動し `rows=28` を返す（クラッシュしない）。
- 既定値へ戻る: `theme=dark`、`interval=1000`、`sort=CPU`、`dir=desc`。
- Notice に `.corrupt.<timestamp>` を含む固定文言が表示される。
- Notice に内部パス・トークン・メールアドレス・アカウント識別子が含まれない。
- 退避キー `gpui-task-manager.settings.corrupt.18e9708720bce700` が作成される。
- 退避キーに元の `{ not json` がそのまま保持される。
- 既定値が `gpui-task-manager.settings` へ保存される
  （`version=2`、`interval_ms=1000`、`sort=Cpu/Descending`、`theme=Dark`）。

退避キーのサフィックスは実行時に生成される 16 進タイムスタンプであり、
固定値を前提とした検証ではない。

### 証拠の機密性

`notes/assets/web-settings-evidence.json` を全文スキャンし、
トークン / シークレット / API キー / パスワード、
内部パス（ホームディレクトリや作業ツリー）、
アカウント識別子 / メールアドレス / tailnet ドメイン、
ループバック以外の IP アドレスの各パターンで 0 件一致を確認した。
マスクが必要な値は存在しなかったため、証拠 JSON は未改変である。
記録されている URL は `http://127.0.0.1:8094/` のみ。

### プロセス停止と後始末

起動した 2 つの background job を停止し、停止後に
`ss -ltn` で port 8094 と port 9341 が空きであることを確認した。
`user-data-dir=/tmp/tm-settings-chrome` を指定したプロセスは残っていない。

## 制約

- 公式スクリプト `cdp_verify_settings.py` は保存・復元・切替のみを対象とし、
  破損設定のケースを持たない。破損退避は本報告の追加 CDP 検証で確認した。
  同ロジックのオフライン検証は `web/src/settings.rs` の
  `parse_saved("{ not json")` 単体テストにも存在する（設定キーは
  `gpui-task-manager.settings`、退避キー接頭辞は
  `gpui-task-manager.settings.corrupt.`）。
- デモモードで検証したため、SSE / ライブ接続の挙動は対象外である。
- 証拠 JSON の `started_at` / `finished_at` はスクリプト側の
  `time.strftime("%Y-%m-%dT%H:%M:%S%z")` により `+0900` 形式
  （コロンなし）で記録される。本報告の日時は `date` で確認した
  RFC 3339 のオフセット付き形式を使用している。
- 検証はループバックのみで行い、外部送信・公開操作・グローバル設定変更は
  行っていない。

## 検証

```bash
markdownlint notes/web-settings-runtime-verification.md
bash scripts/check-public-docs.sh
```

- `markdownlint`: 初回は MD024（見出し重複）2 件で失敗したため、結果側の
  見出しを改名し、再実行で終了コード 0（エラー 0 件）を確認した。
- `bash scripts/check-public-docs.sh`: 最終再実行で終了コード 0、
  `public-docs: OK (no internal paths / account identifiers in published docs)`
  を確認した。途中で別担当のファイルが一時的に
  `account-identifier` パターンに一致して失敗したが、
  当該担当の修正後に再実行して解消している。本報告は担当外ファイルを
  修正していない。

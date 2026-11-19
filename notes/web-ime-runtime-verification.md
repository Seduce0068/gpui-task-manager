---
type: Report
title: "ブラウザ版日本語 IME の Runtime Verification（H2-IME）"
description: "現行 web/dist を実ヘッドレス Chrome で検証し、IME の composition・確定・Escape 破棄を含む 32 チェックすべてが成功した記録"
tags:
  - ime
  - japanese-input
  - composition
  - runtime-proof
  - cdp
  - web
generated:
  by: "h2-ime"
  at: "2026-11-19T21:02:32+09:00"
verified:
  by: "h2-ime"
  at: "2026-11-19T21:02:32+09:00"
status: "approved"
---

## 概要

現在のコードから生成された既存の `web/dist` を実ブラウザ（headless Chrome）で配信し、
CDP 経由で日本語 IME の composition / 確定 / Escape 破棄を再現した。
`web/README.md` と `notes/ime.md` が謳う「composition 32 チェック、JS 例外 0」は、
現行コードでも再現した（32/32 OK、Runtime 例外 0、コンソールエラー 0）。

## 環境

| 項目 | 値 |
| --- | --- |
| 検証日時 | 2026-11-19T21:02:32+09:00 |
| ブラウザ | Chrome/158.0.8127.36（headless new・`--no-sandbox --disable-dev-shm-usage`） |
| CDP ポート | 9342（ループバックのみ） |
| 静的サーバ | `127.0.0.1:8095`（`python3 -m http.server`） |
| Python | 3 + `websockets` 15.0.1 |
| 配信物 | 既存の `web/dist`（再ビルドせず再利用） |

`web/dist/index.html` と wasm の更新時刻は `web/src/shell.rs` より後であり、
配信物は現行ソースを反映している。再ビルドは担当外パスへの書き込みを避けるため実施していない。

## 実行コマンド

### 指示されたコマンド（失敗）

```bash
python3 web/scripts/cdp_verify_ime.py --port 9342 --url http://127.0.0.1:8095/ --evidence notes/assets/web-ime-evidence.json
```

終了コード 2。`cdp_verify_ime.py` の argparse には `--port` / `--url` が存在しない。
これらは `web/scripts/cdp_verify_web.py` 側のオプションである。
現行の IME スクリプトは静的サーバと Chrome を自分で起動・停止する設計に変更されており、
`--http-port` / `--cdp-port` / `--dist` を使う。

### 実際に成功したコマンド

```bash
python3 web/scripts/cdp_verify_ime.py \
  --dist web/dist \
  --out /tmp/tm-ime-assets \
  --http-port 8095 \
  --cdp-port 9342 \
  --evidence notes/assets/web-ime-evidence.json
```

`--out` を一時ディレクトリに変更したのは、既存の `notes/assets/ime-*.png` を
担当外パスで上書きしないためである。エビデンス JSON のみ担当パスへ出力した。

## 結果

`checks: 32/32 OK`、`Runtime 例外が無い — 0 件`、`コンソールエラーが無い — 0 件`、
終了コード 0。エビデンスは `notes/assets/web-ime-evidence.json` に保存した。

### 32 チェックの内訳

| 段階 | チェック数 | 主な確認内容 |
| --- | ---: | --- |
| 初期状態 | 4 | 隠し textarea の存在・フォーカス、非変換中、フィルタ空 |
| ASCII 入力 `fire` | 3 | 絞り込み反映、件数減少、非変換中を維持 |
| Backspace | 1 | 1 文字削除（`fir`） |
| Escape 解除 | 2 | フィルタ解除、全件へ復帰 |
| composition `にほんご` | 5 | 変換中フラグ、未確定文字列表示、marked range `0:4`、フィルタ・一覧を更新しない |
| compositionupdate `日本語` | 3 | 候補更新の反映、フィルタ未更新、変換中を維持 |
| compositionend 確定 | 4 | `filter=日本語`、フラグ下降、marked range 消滅、一覧絞り込み |
| 確定後 Escape | 3 | フィルタ・入力欄を空に、全件へ復帰 |
| 変換中 Escape（破棄） | 4 | 変換中フラグ、フィルタ空、入力欄空、次の composition で状態回復（`漢字`） |
| 最終状態 | 1 | フィルタ解除 |
| 例外・エラー | 2 | Runtime 例外 0、コンソールエラー 0 |
| **合計** | **32** | すべて OK |

## エビデンスと機微情報

`notes/assets/web-ime-evidence.json`（25,654 バイト）は JSON として有効で、
32 件の `checks`、11 件の `steps`、`exceptions: []`、`console_errors: []`、
12 件のスクリーンショット名を含む。

機微情報スキャンでは、開発ホストのアカウント識別子・ホームディレクトリ、
tailnet ドメイン、内部作業パス、トークン、`Bearer`、`Authorization`、
秘密情報系キーワードのすべてで一致件数 0 を確認した。
`url` はループバックアドレスのみ、`chrome` はブラウザバージョン文字列のみである。

## 制約

- CDP の合成 IME は Escape で `compositionend` を発火しないため、
  変換中フラグが残る。スクリプトは直後の composition で状態が回復することを確認しており、
  実 IME の Escape 時の `compositionend` 発火は未検証（`notes/ime.md` と同趣旨）。
- 現行スクリプトがサーバと Chrome を自己起動するため、指示手順の外部 background 起動は
  ポート競合を避けるため行わなかった。`--force-device-scale-factor=1` も付与されないが、
  DOM 状態フックによる検証であり結果に影響しない。
- スクリーンショットは一時ディレクトリへ出力したため、リポジトリには保存していない。
- Safari / Firefox、モバイル（タッチ・ソフトキーボード）は未検証。

## 検証

```bash
markdownlint notes/web-ime-runtime-verification.md
bash scripts/check-public-docs.sh
```

## 参照

- `notes/ime.md` — IME 実装と過去の Runtime Proof
- `notes/verification.md` — リポジトリ全体の検証記録
- `web/README.md` — ブラウザ版シェルの仕様と検証手順
- `web/scripts/cdp_verify_ime.py` — 本次で実行した検証スクリプト

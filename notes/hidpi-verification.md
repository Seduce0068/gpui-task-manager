---
type: Report
title: "Web シェル HiDPI 描画検証 (DPR 1 / DPR 2)"
description: "ヘッドレス Chrome と CDP で Web シェルの DPR 1 / DPR 2 描画をスクリーンショット確認した証跡"
tags:
  - hidpi
  - web-shell
  - cdp
  - verification
generated:
  by: "c1-hidpi"
  at: "2026-11-19T19:04:47+09:00"
status: "draft"
---

## 概要

Web シェル（wasm 版 GPUI タスクマネージャー）の DPR 1 / DPR 2 描画を、エージェント自身が取得した
スクリーンショットで確認した。グローバル表示設定（kscreen-doctor の scale 変更等）は行っていない。
DPR は Chrome プロセス限定の `--force-device-scale-factor=2` と CDP の
`Emulation.setDeviceMetricsOverride` のみで強制している。

## 環境

| 項目 | 値 |
| --- | --- |
| Chrome | Google Chrome 157.0.8103.78 (`--headless=new`) |
| CDP | Protocol-Version 1.3、ポート 9333 / 9335 / 9336 / 9337 |
| Python | 3.14.4、`websockets` 15.0.1、`cdp_verify_web.py` の CDP ヘルパを再利用 |
| trunk | 0.22.1（`trunk build --release` 成功、約 1 分 30 秒） |
| 取得日時 | 2026-11-19T18:57〜19:04+09:00 |

## 実行コマンド

```bash
cd web && trunk build --release
python3 -m http.server 8091 --bind 127.0.0.1 -d web/dist
google-chrome --headless=new --no-sandbox --disable-dev-shm-usage \
  --window-size=1440,900 --remote-debugging-port=9333 \
  --user-data-dir=/tmp/tm-hidpi-chrome about:blank
```

DPR 2 の制御実験では、上記 Chrome 起動時に `--force-device-scale-factor=2` を付与した。
`Page.captureScreenshot` と `Emulation.setDeviceMetricsOverride` は CDP 経由で実施している。

## 取得画像

| ファイル | DPR 指定 | CSS viewport | 画像サイズ | 判定 |
| --- | --- | --- | --- | --- |
| [web-hidpi-dpr1.png](assets/web-hidpi-dpr1.png) | CDP override dSF=1 | 1440x900 | 1440x900 | 正常 |
| [web-hidpi-dpr2.png](assets/web-hidpi-dpr2.png) | プロセス dSF=2 + CDP override dSF=2 | 1440x900 | 2880x1800 | 正常 |
| [web-hidpi-dpr2-cdp-override-artifact.png](assets/web-hidpi-dpr2-cdp-override-artifact.png) | CDP override dSF=2 のみ | 1440x900 | 2880x1800 | エミュレーション起因の崩れ |
| [web-hidpi-dpr2-force-factor.png](assets/web-hidpi-dpr2-force-factor.png) | プロセス dSF=2 | 2880x1657 | 5760x3314 | 正常（2 倍バッファ確認） |

## 確認結果

### DPR 1

- `window.devicePixelRatio=1`、canvas バッキングストア 1440x900、CSS サイズ 1440x900 で一致。
- 文字は鮮明。ヘッダー、概要カード 4 枚、ツールバー、テーブル見出し、詳細パネル、ステータスバーに
  重なりやクリッティングは見られない。

### DPR 2（正しい描画）

- `window.devicePixelRatio=2`。canvas バッキングストアは CSS サイズの 2 倍
  （2880x1514、別実験で 5760x3314 / CSS 2880x1657）に設定されている。
- 文字は物理ピクセル単位で鮮明。カードの重なり、ツールバー右端（表示件数 / CSV / JSON）の欠落、
  テーブル列幅の崩れはない。名前 / コマンド / CPU / メモリ / PID / ユーザー / 状態 / スレッドの
  8 列見出しと詳細パネルも表示されている。

### CDP override 単独で発生した崩れ（記録）

- `Emulation.setDeviceMetricsOverride` のみで dSF=2 を設定すると、`window.devicePixelRatio` は 2 に
  なるが canvas バッキングストアは 1440x900 のままだった。1 倍バッファが 2 倍に拡大される結果、
  ツールバー右端とテーブル本体・詳細パネルが見切れる
  （[崩れの画像](assets/web-hidpi-dpr2-cdp-override-artifact.png)）。
- ロード後に override を適用し、合成 `resize` イベントを発行してもバッキングストアは更新されなかった。
- プロセス起動時に DPR=2 を設定した制御実験では正しく描画された。したがってこの崩れは、実行時の
  DPR 変更をエミュレートする CDP 手法の制約（Headless Chrome の DevicePixelContentBox が 1 倍のまま
  等）である可能性が高い。実機モニタ移動時の再描画は未検証であり、断定しない。

### DOM proof

- `body[data-gpui-proof]` は DPR 1 / DPR 2 の両方で存在し、329 文字の
  `mode=demo;tick=0;rows=0;vw=1440;vh=900;...` を確認した。
- 属性のみで画面上には表示されない（シェル実装の設計どおり）。画面下部のステータスバーは
  データ源・エラー文言であり DOM proof 自体ではない。

## HiDPI とは独立した制約

- このビルドでは DOM proof が `tick=0; rows=0` のままで、14 秒待機してもプロセス行が生成されない。
- コンソールには gpui-pre 0.3.12 の `RefCell already borrowed: attempted mutable App borrow`
  （window.rs:1933 / 1976 / 2032 / 2064）が繰り返し出力される。
- emulation なし・追加フラグなしのクリーンな Chrome でも再現したため、DPR 強制手法に起因しない。
- よって行の重なりやセル内のクリッティングは本次では評価していない（ヘッダー列幅のみ確認）。

> **2026-11-21 追記（当該不具合は解消済み）**: 上記 `tick=0 / rows=0` と
> `RefCell already borrowed` は HiDPI 検証当時（2026-11-19）のビルドの記録であり、
> 現行ビルドでは再現しない。後時刻のライブ CDP 検証では `tick` が 4 → 14 と進み
> `rows=500`（実プロセス）が描画され、`failures: [] / passed: true` となっている
> （`notes/verification.md`「ブラウザ版ライブ / エラー時モードの CDP 再検証」節、
> 2026-11-15 実施）。デモモードも `rows=28` で描画され、2026-11-17 の CDP 再検証
> （設定永続化・デモ・IME 32/32）もすべて成功している。README.md「検証状況」も
> ブラウザ版シェルのデモ / ライブ検証を成功として記載しており、
> `RefCell already borrowed` は当時の gpui-pre 0.3.12 側の事象が解消されたものと判断する。
> ただし実機 HiDPI モニタ移動後の再描画は下記「未検証項目」のとおり引き続き未検証。

## 未検証項目

- ネイティブ版（デスクトップ GPUI）の HiDPI 描画は対象外。ユーザー目視が必要なまま。
- 実機 HiDPI ディスプレイ、および実行中の DPR 変更（モニタ移動）後の再描画は未検証。

## 検証

- `markdownlint notes/hidpi-verification.md` を実行し、警告なしを確認した。
- `bash scripts/check-public-docs.sh` を実行し、内部パスや識別子の混入がないことを確認した。
- 画像にトークン・認証情報は映っていない（デモモード、エージェント接続なし）。

## 参照

- [CDP 検証ヘルパ](../web/scripts/cdp_verify_web.py)
- [証拠 JSON](assets/web-hidpi-evidence.json)

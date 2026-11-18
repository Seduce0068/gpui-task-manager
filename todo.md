---
type: Todo
title: "gpui-task-manager プロジェクトTODO"
description: "gpui-task-manager 固有のタスク。横断タスクはグローバルTODOへ。"
tags:
  - todo
  - tasks
  - "gpui-task-manager"
generated:
  by: opencode
  at: "2026-11-17T20:11:47+09:00"
status: current
---

## 進行中

（なし）

## 未着手

### 優先度: 高

- [x] CDP 自己検証を CI ゲートへ接続（R1-V01/G01）— cdp-verify ジョブと
      3 スクリプトの --self-check を追加。実機クリック手順は CI コメントに明記。
- [x] wasm fetch タイムアウトと stale 表示（R2-X01, R3-S01）— 5秒タイムアウト、
      旧スナップショット維持、DOM hook に stale=0|1 を追加。
- [x] `?agent=` 接続先を loopback/http に制限（da48630）。
- [x] 弱い乱数フォールバック廃止とトークン出力の見直し（01accf5 / 170e94a）。
- [x] 設定ファイルの互換性と通知（R2-D01/D02/R5-D02/R3-U01/U02）— version 保持、
      未知フィールド round-trip、SettingsNotice を status / DOM alert へ接続。
- [x] Web 版へ aria / AccessKit の代替表現を追加（R3-A01）— 表・ボタン・ステータスに role / aria-label / live を付与。
- [x] キーボード parity と入力中ナビ抑制（R3-A02/A03/R1-D11/I13）— Ctrl/Cmd 統一、
      純粋 key_action と単一ディスパッチを追加。
- [x] DEB の依存検査と purge 方針（910eef2）。
- [x] agent の構造化ログ / トレース / /metrics / 終了コード分類（R5-O01〜O04）
- [x] markdownlint を check.sh / CI へ配線（R5-C01 の配線部分）。
      ドキュメント数値の陳腐化と AC-12 マトリクスは R5-C02 として分離。
- [x] R5-C02 ドキュメント数値と AC-12 マトリクスを更新 — README / DESIGN /
      notes/* を現行実装へ。RESEARCH.md の2箇所は R1-D02B として分離。
- [x] 成果物ジョブの `needs: gates` と fork PR 除外（4696c41）。
- [x] ライセンス帰属と GPUI pin の訂正（4696c41 / fdf6f62）。
- [ ] SBOM 生成を冪等化し、cargo-vet / advisory ignore の運用を見直す
      （R1S-OPS-04, R1S-OPS-09, R1S-OPS-10）。
- [x] 数値表示と履歴の整合（733b7b0 / b310422）。
- [x] Unicode 不可視文字と NFC 比較（R1S-DATA-01/02/05/06/11）— Cf 全21レンジ拒否、
      UCD 16.0 の NFC を自前実装（新規依存なし・理由をコードに記録）。
- [x] トークンの argv 漏えい対策とファイル権限検証（170e94a）。
- [x] per-signal capability で Stop/Continue を無効化（R2X-P1/P2/P4/R5-I04）—
      Web の details_visible は既に実効化済みと確認。viewmodel 側も防御。
- [x] `unmark()` の EditOutcome 契約と UTF-16 範囲丸め（R2X-S1/S8）— 確定文字列不変
      を維持し、サロゲート途中の selection を修正。
- [x] ワイヤ境界のサニタイズと redact 対象拡張（7880663 / 3fa8ea1 / b7c256b）。

### 優先度: 中

- [x] 仮想メモリのソート・絞り込みに対応（R5-I01）— SortKey::VirtualMemory としきい値フィルタを追加。
- [x] 温度しきい値ゲージ（R5-I03/R1-I03）— 色に加え状態語と aria 数値を表示。
- [x] `supports_*` Capability で取得不可セクションを出し分け（R5-I04）— CDP 座標を維持した説明ノードを追加。
- [x] 設定一時ファイルの nonce と query_param 検証（a232a8e）。
- [x] `ConfirmKill` の paused ガードと pending 保護（38d954f）。
- [x] 接続スレッドの JoinHandle を追跡し期限付き join を実装（R2-X03）— JoinTracker を追加。
- [x] シグナル別確認文言と色非依存 status（R3-U03〜U05）— [情報]/[エラー]
      マーカー、confirmation_note、transient 自動クリアを維持。
- [x] agent と app の CLI を揃える（R5-O05）— --version / --log-level / ヘルプ / 終了コードを統一。
- [x] `dashboard.apply` の Effect 契約を明示（R2-C01/C02）— 呼び出し側が必ず1回実行、
      effect-free 判定ヘルパーとテストを追加。
- [x] Web 版へ更新間隔変更 UI を追加（R1-C01/R2-X02）— プリセット切替とサンプラー反映、DOM hook は追加字段のみ。
- [x] probe 失敗時は暗黙デモへ落とさず EXIT_PROBE、--demo 不正値は exit 2
      （R3-S02/S03）。app::new の到達不能な暗黙分岐は R3-S02B として残す。
- [x] エージェント再起動時の接続再読込と再入力導線（R3-S04/S05）— SharedConfig、
      固定ガイダンス、DOM proof reconnect=0|1 を追加。
- [x] OSC/DCS（+SOS/PM/APC）ペイロード全体を除去し、ZWJ/ZWNJ は可視ギャップ
      を残す方針を明文化（R1S-DATA-03/04）。
- [x] EiB 表記と limit エコーのサニタイズ（a29d76c）。
- [ ] 固定ポートの複数インスタンス方針と dev-setup shim の残置対策を
      文書化・実装する（R3-L03, R3-L04）。
- [ ] `check.sh` のスキップ時挙動を明示し、誤った成功表示を防ぐ
      （R1S-OPS-08）。
- [ ] プローブの全スレッド走査を限定し、mutex 占有を抑える（R4-A01）。
- [ ] wasm レスポンス読み取りの早期 abort と並行 fetch の制限を検討する
      （R4-A02、upstream 確保ではなく累積の問題）。
- [ ] 重複バージョンと feature 差分依存の監査方法を決定する
      （R1-OPS-03, R4-S04, R1S-OPS-03）。
- [x] slowloris 対策と accept/spawn の graceful 化（b95fbf8 / e6b46ff）。
- [ ] schema version の前方/後方互換方針を定義し、ディスク使用率を
      `used_space()` 相当へ修正する（R2X-D3, R2X-D5）。
- [ ] Windows の taskkill エラー意味論、PID 1 保護、Users 更新、
      「空＝非対応」の区別を修正する（R2X-P5, R2X-P6, R2X-P7, R2X-P8）。
- [x] 重複ヘッダ拒否・Host 不一致・stale 表示（dcd9e9c / dde9bcd）。

- [x] R3-S02B `app::new()` の暗黙デモ分岐を除去し明示エラーへ統一

- [ ] R1-D02B RESEARCH.md §8 の未確認4（IME）・8（エージェント並列化）を解消済みとして本文へ反映

### 優先度: 低

- [x] 履歴トレンド差分バッジ（R5-I05）— 純粋ロジックと widget を追加し概要カードへ配線。※R5-I02 は却下済み。
- [x] CSV/JSON 変換の純粋関数を tm-core に追加（R1-I02 のコア部分）。
      UI / CLI 配線は R1-I02-UI として分離。
- [x] R1-I02-UI CSV/JSON エクスポートを CLI / Web の UI へ接続
      （0600 権限、既定 redact、DOM proof は加法変更）。
- [ ] 名前付きフィルタプリセット、選択プロセスの時系列スパークライン
      （R1-I06, R1-I08）。
- [ ] バッテリー／電源状態、適応的サンプリング、cgroup グルーピング
      （R1-I10, R1-I11, R1-I12）。
- [ ] エージェントのストリーミング配信、ブラウザ初回オンボーディング、
      行コンテキストメニュー（R1-I14, R1-I16, R1-I19）。
- [ ] CDP 検証のクリック座標を定数同期からセレクタ取得へ変更する
      （R1S-ARCH-03, R1-OPS-11）。
- [ ] PoC の `cdp_verify.py` に「証拠収集のみ」と明記する（R1S-OPS-12）。
- [ ] HiDPI／モニタ間移動時の表示を実機で確認する（R3-A05）。
- [ ] スクリーンショットと証跡数値の鮮度、性能記述を再測定する
      （R1-G12, R1-G13）。
- [ ] 放置されたローカルブランチ／merged リモートブランチを整理する
      （R1-G14）。

## 完了

- [x] `todo.md` をプロジェクト内へ配置（2026-11-17）
- [x] 全 6 ラウンドのクロスレビュー実施（189 所見、2026-11-17）

## メモ

- グローバルTODO: `~/.config/opencode/todo.md`（複数プロジェクト横断のタスク用）。
- 全所見の根拠・Runtime Proof・重複統合・却下理由は
  `notes/residual-tasks-research.md` を参照。
- `notes/current-state.md`「未解決・今後の候補」に既出の 8 項目
  （`prefers-color-scheme`、同形異義文字、実 IME、431/400/503 の CORS、
  Windows/macOS 実機、優先度変更、GPU 詳細監視、DEB 署名・公開）は
  重複登録していない。
- 却下: R5-I02（`format_process_age` は利用済み）。R2-X02 は
  「妥当（潜在）」へ上書き、R2-F01 は Medium へ格下げ済み。

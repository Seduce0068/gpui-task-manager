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
- [ ] 設定ファイルの破損時バックアップ、version 保持、未知フィールドの
      ラウンドトリップ、GUI への非通知解消（R2-D01, R2-D02, R5-D02,
      R3-U01, R3-U02）。
- [x] Web 版へ aria / AccessKit の代替表現を追加（R3-A01）— 表・ボタン・ステータスに role / aria-label / live を付与。
- [ ] キーボードショートカットの cross-shell parity と入力中のナビ抑制
      （R3-A02, R3-A03, R1-D11, R1-I13）。
- [x] DEB の依存検査と purge 方針（910eef2）。
- [x] agent の構造化ログ / トレース / /metrics / 終了コード分類（R5-O01〜O04）
- [x] markdownlint を check.sh / CI へ配線（R5-C01 の配線部分）。
      ドキュメント数値の陳腐化と AC-12 マトリクスは R5-C02 として分離。
- [ ] R5-C02 設計・調査・テスト件数の陳腐化と AC-12 マトリクスを解消する
      （R1-D01, R1-D02, R1-D03, R1-D16, R5-C03, R5-C04）。
- [x] 成果物ジョブの `needs: gates` と fork PR 除外（4696c41）。
- [x] ライセンス帰属と GPUI pin の訂正（4696c41 / fdf6f62）。
- [ ] SBOM 生成を冪等化し、cargo-vet / advisory ignore の運用を見直す
      （R1S-OPS-04, R1S-OPS-09, R1S-OPS-10）。
- [x] 数値表示と履歴の整合（733b7b0 / b310422）。
- [ ] Unicode の不可視書式文字（Cf / 補助面 VS）を網羅し、フィルタ比較を
      NFC 正規化する（R1S-DATA-01, R1S-DATA-02, R1S-DATA-05,
      R1S-DATA-06, R1S-DATA-11）。
- [x] トークンの argv 漏えい対策とファイル権限検証（170e94a）。
- [ ] Web の `details_visible` 設定を実効化し、Windows の Stop/Continue を
      per-signal capability で隠す（R2X-P1, R2X-P2, R2X-P4, R5-I04）。
- [ ] `unmark()` の EditOutcome 契約と UTF-16 範囲丸め後の selection を修正する
      （R2X-S1, R2X-S8）。純粋ロジックとして確定。
- [x] ワイヤ境界のサニタイズと redact 対象拡張（7880663 / 3fa8ea1 / b7c256b）。

### 優先度: 中

- [x] 仮想メモリのソート・絞り込みに対応（R5-I01）— SortKey::VirtualMemory としきい値フィルタを追加。
- [x] 温度しきい値ゲージ（R5-I03/R1-I03）— 色に加え状態語と aria 数値を表示。
- [ ] `supports_*` Capability で取得不可セクションを出し分ける（R5-I04）。
- [x] 設定一時ファイルの nonce と query_param 検証（a232a8e）。
- [x] `ConfirmKill` の paused ガードと pending 保護（38d954f）。
- [x] 接続スレッドの JoinHandle を追跡し期限付き join を実装（R2-X03）— JoinTracker を追加。
- [x] シグナル別確認文言と色非依存 status（R3-U03〜U05）— [情報]/[エラー]
      マーカー、confirmation_note、transient 自動クリアを維持。
- [x] agent と app の CLI を揃える（R5-O05）— --version / --log-level / ヘルプ / 終了コードを統一。
- [ ] `dashboard.apply` が返す Effect を実行するか、空である前提を明示する
      （R2-C01, R2-C02）。
- [x] Web 版へ更新間隔変更 UI を追加（R1-C01/R2-X02）— プリセット切替とサンプラー反映、DOM hook は追加字段のみ。
- [ ] ネイティブ probe 失敗時のデモ切替と `--demo` 不正値の扱いを統一する
      （R3-S02, R3-S03）。
- [ ] エージェント再起動時の接続設定再読込／利用者再入力の導線を設ける
      （R3-S04, R3-S05）。
- [ ] OSC/DCS ペイロードの除去方針と ZWJ/ZWNJ の扱いを決める
      （R1S-DATA-03, R1S-DATA-04）。
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

### 優先度: 低

- [ ] 履歴のトレンド差分バッジを追加する（R5-I05）。※R5-I02 は却下済み。
- [x] CSV/JSON 変換の純粋関数を tm-core に追加（R1-I02 のコア部分）。
      UI / CLI 配線は R1-I02-UI として分離。
- [ ] R1-I02-UI CSV/JSON エクスポートを CLI / Web の UI へ接続する
      （0600 権限、--redact 既定を維持）。
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

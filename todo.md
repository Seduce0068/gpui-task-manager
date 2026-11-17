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
- [ ] `?agent=` エンドポイントを loopback/http に検証し、Bearer トークンの
      送信先を制限する（R2-P01, R1-V08, R1S-SEC-01）。
- [ ] 弱い乱数フォールバックを廃止し、トークンの stderr 平文出力を見直す
      （R1S-SEC-02, R1-V14, R4-O01, R1S-SEC-03）。
- [ ] 設定ファイルの破損時バックアップ、version 保持、未知フィールドの
      ラウンドトリップ、GUI への非通知解消（R2-D01, R2-D02, R5-D02,
      R3-U01, R3-U02）。
- [x] Web 版へ aria / AccessKit の代替表現を追加（R3-A01）— 表・ボタン・ステータスに role / aria-label / live を付与。
- [ ] キーボードショートカットの cross-shell parity と入力中のナビ抑制
      （R3-A02, R3-A03, R1-D11, R1-I13）。
- [ ] DEB に purge 方針の maintainer スクリプトを追加し、`ldd` の
      "not found" を失敗扱いにする（R3-L01, R1S-OPS-05）。
- [x] agent の構造化ログ / トレース / /metrics / 終了コード分類（R5-O01〜O04）
- [x] markdownlint を check.sh / CI へ配線（R5-C01 の配線部分）。
      ドキュメント数値の陳腐化と AC-12 マトリクスは R5-C02 として分離。
- [ ] R5-C02 設計・調査・テスト件数の陳腐化と AC-12 マトリクスを解消する
      （R1-D01, R1-D02, R1-D03, R1-D16, R5-C03, R5-C04）。
- [ ] 成果物ジョブへ `needs: gates` を付与し、fork PR を成果物生成から
      除外する（R1-OPS-01, R4-O02, R1S-OPS-02）。
- [ ] NOTICE のライセンス帰属（GPUI / gpui-component / IBM Plex）と
      GPUI pin の記載を訂正する（R4-S01, R4-S02, R5-D01, R1-G08）。
- [ ] SBOM 生成を冪等化し、cargo-vet / advisory ignore の運用を見直す
      （R1S-OPS-04, R1S-OPS-09, R1S-OPS-10）。
- [ ] 数値表示と履歴の整合（NaN/負値/demo tick のオーバーフロー）を取る
      （R1S-DATA-07, R1S-DATA-09, R1S-DATA-10）。
- [ ] Unicode の不可視書式文字（Cf / 補助面 VS）を網羅し、フィルタ比較を
      NFC 正規化する（R1S-DATA-01, R1S-DATA-02, R1S-DATA-05,
      R1S-DATA-06, R1S-DATA-11）。
- [ ] `--token` の argv 経由のスナップショット漏えいを塞ぎ、トークンファイルの
      パーミッション検証を追加する（R2X-A1, R2X-A2, R2X-A10）。
- [ ] Web の `details_visible` 設定を実効化し、Windows の Stop/Continue を
      per-signal capability で隠す（R2X-P1, R2X-P2, R2X-P4, R5-I04）。
- [ ] `unmark()` の EditOutcome 契約と UTF-16 範囲丸め後の selection を修正する
      （R2X-S1, R2X-S8）。純粋ロジックとして確定。
- [ ] ワイヤ境界（web/agent.rs）で `sanitize_display` を適用し、`--redact` の
      マスク対象へ `mount_point` / `os_name` / IF 名を追加する（R2X-D1, R2X-D4）。

### 優先度: 中

- [ ] 仮想メモリのソート・絞り込みに対応する（R5-I01）。
- [ ] 温度の `critical_c` / `max_c` を使ったしきい値ゲージを追加する
      （R5-I03, R1-I03）。
- [ ] `supports_*` Capability で取得不可セクションを出し分ける（R5-I04）。
- [ ] 設定一時ファイル名へ nonce を付与し、`query_param` の重複と制御文字を
      拒否する（R2-D03, R2-D05）。
- [ ] `ConfirmKill` の paused ガードと pending の無条件上書きを防ぐ
      （R2-F01, R2-F02）。
- [ ] 接続スレッドの `JoinHandle` を追跡し、終了処理を制御する（R2-X03）。
- [ ] i18n 機構、シグナル別の確認文言、色以外の status 識別、transient
      メッセージの自動クリアを実装する（R3-A04, R3-U03, R3-U04, R3-U05）。
- [x] agent と app の CLI を揃える（R5-O05）— --version / --log-level / ヘルプ / 終了コードを統一。
- [ ] `dashboard.apply` が返す Effect を実行するか、空である前提を明示する
      （R2-C01, R2-C02）。
- [ ] Web 版へ更新間隔変更 UI を追加し、起動時固定の潜在問題を解消する
      （R1-C01, R2-X02）。
- [ ] ネイティブ probe 失敗時のデモ切替と `--demo` 不正値の扱いを統一する
      （R3-S02, R3-S03）。
- [ ] エージェント再起動時の接続設定再読込／利用者再入力の導線を設ける
      （R3-S04, R3-S05）。
- [ ] OSC/DCS ペイロードの除去方針と ZWJ/ZWNJ の扱いを決める
      （R1S-DATA-03, R1S-DATA-04）。
- [ ] バイト表記へ EiB を追加し、`limit` エコーをサニタイズする
      （R1S-DATA-08, R4-A03）。
- [ ] 固定ポートの複数インスタンス方針と dev-setup shim の残置対策を
      文書化・実装する（R3-L03, R3-L04）。
- [ ] `check.sh` のスキップ時挙動を明示し、誤った成功表示を防ぐ
      （R1S-OPS-08）。
- [ ] プローブの全スレッド走査を限定し、mutex 占有を抑える（R4-A01）。
- [ ] wasm レスポンス読み取りの早期 abort と並行 fetch の制限を検討する
      （R4-A02、upstream 確保ではなく累積の問題）。
- [ ] 重複バージョンと feature 差分依存の監査方法を決定する
      （R1-OPS-03, R4-S04, R1S-OPS-03）。
- [ ] エージェントの slowloris 対策（ヘッダ全体の絶対 timeout）と accept
      エラーのバックオフ、spawn 失敗の graceful 化（R2X-A3, R2X-A6, R2X-A7,
      R2X-FR1）。
- [ ] schema version の前方/後方互換方針を定義し、ディスク使用率を
      `used_space()` 相当へ修正する（R2X-D3, R2X-D5）。
- [ ] Windows の taskkill エラー意味論、PID 1 保護、Users 更新、
      「空＝非対応」の区別を修正する（R2X-P5, R2X-P6, R2X-P7, R2X-P8）。
- [ ] 重複ヘッダの拒否、`--listen` と Host 許可リストの不一致、live エラーの
      stale 表示を解消する（R2X-A5, R2X-A9, R2X-S4）。

### 優先度: 低

- [ ] 履歴のトレンド差分バッジを追加する（R5-I05）。※R5-I02 は却下済み。
- [ ] スナップショットの CSV/JSON エクスポート（R1-I02）。
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

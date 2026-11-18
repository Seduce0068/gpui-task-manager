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
- [x] SBOM 冪等化と監査運用（R1S-OPS-04/09/10）— advisory ignore に expiry、
      vet を drift 検出ゲート化。CI で2回生成の同一性を検証。
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
- [x] 固定ポート運用と shim 残置対策（R3-L03/L04）— README に切替手順、dev-setup
      に冪等生成・古い shim の除去を追加。
- [x] `check.sh` のスキップ時挙動を明示（R1S-OPS-08）— SKIP 表示と想定外スキップは
      exit 3、--fast は意図的スキップとして exit 0。
- [x] 全スレッド走査を二段階 refresh へ限定（R4-A01）— 約2353エントリの挿入を回避。
- [x] wasm fetch の早期 abort と並行制限（R4-A02）— FetchGate と generation
      によるチャンク単位 abort を実装。
- [x] 依存バージョン/feature 差分の監査方法（R1-OPS-03/R4-S04/R1S-OPS-03）—
      notes/dependency-audit.md に runbook を追加。
- [x] slowloris 対策と accept/spawn の graceful 化（b95fbf8 / e6b46ff）。
- [x] schema version の前方/後方互換（R2X-D3）— 設定の opaque version 保持と
      wire の旧版受け入れを実装。
- [x] R2X-D5 ディスク使用率を checked 算術の derived 値へ修正。
- [ ] R2X-D5B 真の f_bfree ベース used 値（予約ブロック除外）を取得する —
      sysinfo 0.40.x に API が無く、新規依存または FFI の判断が必要。
- [x] Windows taskkill 意味論と PID 保護（R2X-P5〜P8）— エラー分類、Users 更新、
      capability の実収集反映を追加。
- [x] 重複ヘッダ拒否・Host 不一致・stale 表示（dcd9e9c / dde9bcd）。

- [x] R3-S02B `app::new()` の暗黙デモ分岐を除去し明示エラーへ統一

- [x] R1-D02B RESEARCH.md §8 の未確認4・8を解消済みとして本文へ反映

- [x] R1S-OPS-13 web supply-chain とドキュメントを baseline-unreviewed 方針へ統一

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
- [x] CDP クリック座標を DOM/セレクタ取得へ変更（R1S-ARCH-03/R1-OPS-11）— canvas
      rect 基準の相対座標とフォールバックを追加。
- [x] PoC の `cdp_verify.py` に「証拠収集のみ」と明記（R1S-OPS-12）。
- [ ] HiDPI／モニタ間移動時の表示を実機で確認する（R3-A05）。
- [ ] スクリーンショットと証跡数値の鮮度、性能記述を再測定する
      （R1-G12, R1-G13）。
- [ ] 放置されたローカルブランチ／merged リモートブランチを整理する
      （R1-G14）。

### 優先度: 高 — Twitter 公開前チェック (swarm-project-review R6)

- [x] 公開ドキュメントから内部情報を除去: `README.md:35` の内部ブランチ名
      内部ブランチ名と「クリーンな履歴」運用、`SECURITY.md:12` の
      内部作業パス、`log.md` の旧アカウント名・トークン scope・課金
      情報・内部サービス名・絶対ホームパス、`notes/` のエージェント名・未 push
      状態、`RESEARCH.md` の Apache-2.0 前提。根拠 R3-DE01 / R5-DC01 / R5-UX01。
- [x] DEB 同梱ドキュメントを専用テンプレートへ変更し、生成物 `packaging/linux/build/`
      を git 管理から外す: 現在 `build-deb.sh:84` が開発用 `README.md` を無修正で
      同梱し、内部パス・ブランチ名・古い検証値が入る。同 `:132` の Maintainer に
      個人 GitHub 数値 ID が固定。根拠 R4-DR01 / R1-CF01。
- [x] Web ライブモードの redaction を必須化: `web/src/agent.rs:406` は sanitize
      のみで redact を適用せず、既定 (`--redact` 無し) のエージェントに接続すると
      実ホスト名・ユーザー名・ホームパスが画面と「伏せ字化済み」エクスポートへ残る。
      初回ガイドへ `--redact` 必須を明記し、未 redact 応答はエクスポート拒否または
      明示表示する。根拠 R1-AR02 (high)。
- [ ] `--redact` のマスキング対象を agent / GUI で統一: `crates/tm-core/src/redact.rs:131`
      は `os_name` / `kernel_version` / IF 名 / センサラベルを対象外だが、
      `redact_for_export` はマスクする。agent の `--once` / API / SSE は前者のみを
      呼び、GUI も kernel・cgroup・センサラベルが `display()` を通らない。
      根拠 R4-OB01 / R1-AR01 / R2-ER01。
- [x] エラー・ログ経路から内部 IP / パス / トークンを除去: 非ループバック
      `--listen` の指定値エコー (`agent/src/args.rs:188`)、export 成功/失敗の絶対
      パス (`app/src/main.rs:429,470`)、プローブエラー詳細・トークンファイルパス・
      生成トークンの stderr/JSON ログ (`agent/src/main.rs:319,289,234`)。
      根拠 R1-AP01 / R2-ER01 / R3-AU01 / R4-OB01。

### 優先度: 中 — Twitter 公開前チェック

- [x] CI に公開前シークレットスキャンを追加: 成果物 upload 前に gitleaks /
      trufflehog とバイナリ・dist の `strings` 検査を行い検出で失敗させる。
      根拠 R4-CI01 / R3-DE02。
- [ ] `.gitignore` へ `.env` / `.env.*` / `*.pem` / `*.key` を追加し、
      `!notes/assets/*.log` の再包含を見直す。根拠 R1-CF02。
- [x] CI ゲートを `scripts/check.sh` 一本へ集約し、markdownlint 設定を
      `.markdownlint.json` へ一本化する（現在 3 か所に分散）。根拠 R4-CI01 / R1-CF01。
- [ ] advisory ignore の失効を強制し、cargo-vet の `baseline-unreviewed` のみの
      運用と `advisory-not-detected` 常時 allow を整理する。根拠 R3-DE02 / R4-CI01。
- [ ] GitHub Actions と外部ツールをコミット SHA / 明示バージョンへピン留め。
      根拠 R3-DE02 / R4-CI01。
- [ ] `sanitize_display` で U+2028 / U+2029 を除去し、wire 由来文字列の
      レイアウト崩れと export 混入を防ぐ。根拠 R3-IV02。
- [ ] コード内の開発ホスト固有値を汎用名へ: `/boot/efi` / `/mnt/timeshift` /
      `tailscale0` / `enp4s0` と実測トラフィック量、デモの
      `/home/user/.../gpui-task-manager` と `127.0.0.1:8787`。根拠 R1-AR01。
- [ ] `Dashboard::ingest` で古い/順序外スナップショットを検証し、transient status
      と network rate 基準の汚染を防ぐ。Web の config/generation を原子取得にする。
      根拠 R2-ER02 / R2-BV02。
- [ ] ドキュメントの現状矛盾を解消: `RESEARCH.md` の `/metrics` 欠落、
      `SECURITY.md` の公開前プレースホルダ、wasm 計測日、検証件数、
      設定永続化が 3 項目と書かれた古い記述。根拠 R5-DC01 / R3-DE01。
- [ ] Web の corrupt 設定 DOM 通知を正常 load 時にクリアし、corrupt 値の退避を
      ネイティブと揃える。根拠 R2-DI01。
- [ ] ネイティブ版へ AccessKit の role / aria_label を付与し、ブラウザ版との
      アクセシビリティ格差を解消。根拠 R5-UX01。

### 優先度: 低 — Twitter 公開前チェック

- [ ] ユーザー向け文言の内部用語を置換: `redact` / `stale` / `赤退` /
      `tm-core DemoProbe` 等。根拠 R5-UX01。

### 優先度: 高 — コンボレビュー 2026-11-19

- [x] 自動生成トークンを stderr に表示する旨の古い記載を更新（README/DESIGN/SECURITY/verification 修正）:
      `README.md:125` / `DESIGN.md:269` / `SECURITY.md:61` は「stderr に 1 回表示」
      とあるが実装は値をログに出さない（`agent/src/main.rs:237`）。接続には
      `--token-file` / `TM_AGENT_TOKEN` で渡す必要がある旨を明記。根拠 c5-dc01。
- [x] ネイティブの probe 失敗 stderr を分類済み固定文言へ（probe_error_label 追加）: `app/src/main.rs:58,510,522`
      は `probe_failure_message` / `Display` の生 detail を出力し、内部パス・
      ホスト名が CI ログへ残る。agent の `probe_error_label` 相当を共有化。
      根拠 c4-ob01 / c1-ap01。
- [x] redact を大小文字非依存にする（case_insensitive_ranges 追加）: `crates/tm-core/src/redact.rs:106-121` の
      `String::replace` は完全一致のため `MY-HOST` / `TAIL1234` / 大文字ホームが
      残る。小文字化して位置を求め元文字列を書き換える方式へ。根拠 c3-iv01。
- [x] Web redaction の順序を修正（identify → redact → sanitize）: `web/src/redaction.rs:107` は生識別子を取得し
      対象はサニタイズ後のため、Cf/打ち切り境界で一致せず漏れる。redact →
      sanitize の順序に固定。根拠 c3-iv01。

### 優先度: 中 — コンボレビュー 2026-11-19

- [x] `--once --token` の stdout から自プロセス command のトークンを除去:
      `agent/src/main.rs:364` は `--redact` 時のみ redact するため未指定時に平文が残る。
      `--once` では常にマスク。根拠 c3-au01。
- [x] `--token` 警告文を訂正: `agent/src/main.rs:203` は「同一ユーザー」とあるが
      `/proc/<pid>/cmdline` は world-readable。`TM_AGENT_TOKEN` も environ 経由で
      読める旨を記載し推奨順を `--token-file` 優先へ。根拠 c3-au01。
- [x] 設定保存の CAS / 再読込: `app/src/settings.rs:111` の LAST_LOADED 固定により
      並行タブ／インスタンスが将来 version を書き下げる。保存直前に再読込。
      根拠 c2-di01 / c4-dr01。
- [x] `ControlError::Failed` が taskkill 生出力を保持: `crates/tm-platform/src/lib.rs:719`
      の detail にフルパス/アカウント名が入り得る。固定文言＋exit code のみへ。
      根拠 c2-er01。
- [x] `control()` が 1 PID の refresh で `remove_not_listed_processes=true` を渡し
      全プロセスをキャッシュから削除: `crates/tm-platform/src/lib.rs:553`。false へ。
      根拠 c2-er01。
- [x] README の既定ポート 18787 → 8787 (`README.md:158`)、web/README の
      概要 186px → 260px (`web/README.md:168`)、検証件数 364 → 372 / web 76。
      根拠 d2-fg01 / c5-dc01。
- [x] CI: gitleaks の checksum 検証と `fetch-depth: 0`、成果物ジョブの `needs` に
      deny を追加、check-public-docs の走査対象を動的集合へ＋一致行を redact。
      根拠 c1-cf01 / c4-dr01。
- [x] 第三者ライセンス全文を DEB へ同梱 (`packaging/linux/build-deb.sh:91`)。
      web-dist は IBM Plex OFL をコピー済みで不均一。根拠 c3-de01。
- [x] HTTP エラーへ機械可読 `code` を追加し、400 の raw 値エコーを廃止:
      `agent/src/main.rs:136`。根拠 c1-ap01。
- [x] FetchError を enum 化し 403 を認証失敗と分ける (`web/src/agent.rs:98`)。
      根拠 c1-ap01。
- [x] 設定保存失敗を UI に通知 (`app/src/app.rs:393`, `web/src/shell.rs:391`)。
      根拠 d4-vu01。

### 優先度: 低 — コンボレビュー 2026-11-19

- [x] ネイティブの確認ダイアログ/ヘルプへフォーカス移動と Tab トラップ:
      `app/src/app.rs:1895,2032`。根拠 d4-vu01。
- [x] 温度/負荷表示の `max` / `crit` / `Load` を日本語へ (`app/src/app.rs:1035`)。
      根拠 c5-ux01。
- [x] browser document.title から DOM 証跡フィールドを除去 (`web/src/shell.rs:2068`)。
      根拠 c5-ux01。
- [x] Esc の二重掲載を整理 (`app/src/app.rs:2045`)。根拠 d4-vu01 / c5-ux01。

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

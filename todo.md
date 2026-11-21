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
- [x] R2X-D5B 真の f_bfree ベース used 値（予約ブロック除外）を取得する — 既存 rustix の
      fs feature で safe statvfs を採用し、新規依存・unsafe なしで実装済み（M3-DISK）。
      sysinfo 0.40.x には f_bfree API が無く、旧 derived 妥協案（BB-FBFREE）を経て
      f_bfree 実装へ決着。
- [x] Windows taskkill 意味論と PID 保護（R2X-P5〜P8）— エラー分類、Users 更新、
      capability の実収集反映を追加。
- [x] 重複ヘッダ拒否・Host 不一致・stale 表示（dcd9e9c / dde9bcd）。

- [x] R3-S02B `app::new()` の暗黙デモ分岐を除去し明示エラーへ統一

- [x] R1-D02B RESEARCH.md §8 の未確認4・8を解消済みとして本文へ反映

- [x] R1S-OPS-13 web supply-chain とドキュメントを baseline-unreviewed 方針へ統一

- [x] R1-I06B プリセット / 選択プロセス スパークラインを Web UI へ配線（DOM proof は末尾追加のみ）
- [x] R1-I10B 電源・サンプリング・cgroup を SystemSnapshot / 両 shell へ接続

### 優先度: 低

- [x] 履歴トレンド差分バッジ（R5-I05）— 純粋ロジックと widget を追加し概要カードへ配線。※R5-I02 は却下済み。
- [x] CSV/JSON 変換の純粋関数を tm-core に追加（R1-I02 のコア部分）。
      UI / CLI 配線は R1-I02-UI として分離。
- [x] R1-I02-UI CSV/JSON エクスポートを CLI / Web の UI へ接続
      （0600 権限、既定 redact、DOM proof は加法変更）。
- [x] フィルタプリセットと選択プロセス スパークライン（R1-I06/I08）— ロジックと
      widget を追加。shell.rs への配線は R1-I06B として分離。
- [x] 電源状態/適応サンプリング/cgroup データ（R1-I10〜I12）— core/platform に
      型と probe を追加。SystemSnapshot への組込みは R1-I10B として分離。
- [x] SSE 配信 / 初回オンボーディング / 行メニュー（R1-I14/I16/I19）— DOM proof は
      末尾追加のみ、redact / a11y 契約を維持。
- [x] CDP クリック座標を DOM/セレクタ取得へ変更（R1S-ARCH-03/R1-OPS-11）— canvas
      rect 基準の相対座標とフォールバックを追加。
- [x] PoC の `cdp_verify.py` に「証拠収集のみ」と明記（R1S-OPS-12）。
- [x] HiDPI／モニタ間移動時の表示を実機で確認する（R3-A05）。（現環境は単一モニタ 2560x1440 / scale 1 で再現不能。ユーザー側で DPR 混在構成を用意し目視確認が必要 — R4）。確認手順と環境は notes/environment-verification.md に調査・記録済み（実機目視は未実施）。 ユーザー確認手順（Kubuntu 上で実行）: Web 版は CDP の DPR 1/2 スクリーンショットで検証済み（notes/hidpi-verification.md）。ネイティブ版は未検証。

```bash
echo '=== 現在の出力と scale ==='
kscreen-doctor -o | grep -E 'Output:|Scale:|Geometry:'
echo '=== KScreen / DPI ==='
gsettings get org.gnome.desktop.interface scaling-factor
xrdb -query | grep -i dpi
echo '=== ネイティブ app を scale 変更後に再起動し、文字の鮮明さ・カードの重なり・列幅を目視 ==='
echo '=== Web 版は --force-device-scale-factor=2 の Chrome で canvas のボケと DOM proof の vw/vh を確認 ==='
```

 現環境は単一出力 HDMI-A-1 2560x1440 / scale 1 のため、DPR 混在構成の
 再現には 2 枚目のモニタ接続または KDE 表示設定の scale 変更が必要。

- [x] スクリーンショットと証跡数値の鮮度、性能記述を再測定する（ローカル確認分: 件数 435/90・CI 10 ジョブ・日付を更新。wasm/実機 CDP は最終計測日を明記）（R1-G12, R1-G13）。調査内容は notes/environment-verification.md に記録。性能再測定は実環境・大規模データが必要で未実施。
- [x] 放置されたローカルブランチ／merged リモートブランチを整理する（R1-G14）。調査内容は notes/environment-verification.md に記録済み。削除は破壊的操作のためユーザー承認待ち（未実行）。 削除候補と復元経路:（ユーザー承認を得て 2026-11-19 に実行: fix / backup 3 件を削除。master / アーカイブは reflog 依存のため保持）
- `fix/tm-agent-test-shutdown-race`: `origin/main` マージ済み、リモートに同一 SHA
  (`dab7520`) が残るため `git branch -d fix/tm-agent-test-shutdown-race` は安全。
- `backup/main-pre-integration` / `backup/round31-safe-pre-integration`: 統合前の
  rollback 点。不要になった時点で `git branch -d <name>` を検討。
- `master` / アーカイブ用ブランチ: `bc30c0b` は main の祖先ではなくリモートからも
  到達不能（reflog のみ）。削除しないこと。
いずれも削除は状態変更のため、ユーザー承認後に実行する。

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
- [x] `--redact` のマスキング対象を agent / GUI で統一: `crates/tm-core/src/redact.rs:131`
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
- [x] `.gitignore` へ `.env` / `.env.*` / `*.pem` / `*.key` を追加し、
      `!notes/assets/*.log` の再包含を見直す。根拠 R1-CF02。
- [x] CI ゲートを `scripts/check.sh` 一本へ集約し、markdownlint 設定を
      `.markdownlint.json` へ一本化する（現在 3 か所に分散）。根拠 R4-CI01 / R1-CF01。
- [x] advisory ignore の失効を強制し、cargo-vet の `baseline-unreviewed` のみの
      運用と `advisory-not-detected` 常時 allow を整理する。根拠 R3-DE02 / R4-CI01。
- [x] GitHub Actions と外部ツールをコミット SHA / 明示バージョンへピン留め。
      根拠 R3-DE02 / R4-CI01。
- [x] `sanitize_display` で U+2028 / U+2029 を除去し、wire 由来文字列の
      レイアウト崩れと export 混入を防ぐ。根拠 R3-IV02。
- [x] コード内の開発ホスト固有値を汎用名へ: `/boot/efi` / `/mnt/timeshift` /
      `tailscale0` / `enp4s0` と実測トラフィック量、デモの
      `/home/user/.../gpui-task-manager` と `127.0.0.1:8787`。根拠 R1-AR01。
- [x] `Dashboard::ingest` で古い/順序外スナップショットを検証し、transient status
      と network rate 基準の汚染を防ぐ。Web の config/generation を原子取得にする。
      根拠 R2-ER02 / R2-BV02。
- [x] ドキュメントの現状矛盾を解消: `RESEARCH.md` の `/metrics` 欠落、
      `SECURITY.md` の公開前プレースホルダ、wasm 計測日、検証件数、
      設定永続化が 3 項目と書かれた古い記述。根拠 R5-DC01 / R3-DE01。
- [x] Web の corrupt 設定 DOM 通知を正常 load 時にクリアし、corrupt 値の退避を
      ネイティブと揃える。根拠 R2-DI01。
- [x] ネイティブ版へ AccessKit の role / aria_label を付与し、ブラウザ版との
      アクセシビリティ格差を解消。根拠 R5-UX01。

### 優先度: 低 — Twitter 公開前チェック

- [x] ユーザー向け文言の内部用語を置換: `redact` / `stale` / `赤退` /
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

### 残課題（swarm-coding ラウンド 3 で判明）

- [x] MSRV 1.92 を宣言しているが `sysinfo 0.40.3` が rustc 1.95 を要求し、（sysinfo 0.40.3 が 1.95、gpui-pre-util が slice_as_array を要求。依存変更または MSRV 引き上げの設計判断待ち — AF-CI）（MSRV を 1.95 へ引き上げ、workspace 全体を検証する msrv-1-95 ジョブへ変更 — M2-CI）
      workspace 全体の `cargo +1.92 check` が Cargo に拒否される。CI の msrv-1-92
      ジョブは tm-core / tm-viewmodel にスコープ限定済み。sysinfo を 1.92 対応版へ
      変更するか MSRV を 1.95 へ引き上げる設計判断が必要（af-ci）。

### 優先度: 低 — コンボレビュー 2026-11-19

- [x] ネイティブの確認ダイアログ/ヘルプへフォーカス移動と Tab トラップ:
      `app/src/app.rs:1895,2032`。根拠 d4-vu01。
- [x] 温度/負荷表示の `max` / `crit` / `Load` を日本語へ (`app/src/app.rs:1035`)。
      根拠 c5-ux01。
- [x] browser document.title から DOM 証跡フィールドを除去 (`web/src/shell.rs:2068`)。
      根拠 c5-ux01。
- [x] Esc の二重掲載を整理 (`app/src/app.rs:2045`)。根拠 d4-vu01 / c5-ux01。

### 優先度: 中 — バックログ補充 2026-11-19（swarm-todo-discovery）

- [x] `/metrics` の status-class 合計が `requests_total` と不一致: `agent/src/telemetry.rs:109-125`
      は status 0 (unknown) に counter を持たず、早期リターンのリクエストが
      responses_total から零落ちる。unknown クラス用カウンタを追加し合計一致を
      テストで固定。根拠 e1-st / e5-vu（重複統合）。
- [x] `scripts/check.sh` が `scripts/check-advisory-ignores.sh` を呼んでいない:
      CI の deny ジョブは政策を検査するがローカルは cargo deny check のみ。
      deny セクションへ追加しローカル/CI のゲートを同期。根拠 e4-dp。
- [x] ネイティブの絞り込み欄へ AccessKit の Role::TextInput / aria_label /
      aria_value / aria_description を付与（Web 側は実装済み）。根拠 e5-vu。
- [x] ドキュメントの現状不一致を一括修正: web/README の更新間隔ボタン
      （実装済み）・CSV/JSON エクスポート・stale 表示・トレンド/温度ゲージ、
      DESIGN.md:360 の「設定ファイルを持たない」、README の TM_REDACT=1、
      Web のライブ設定ポーリング再接続。根拠 e2-fg。
- [x] CI のロックファイル整合性: wasm / native-windows / native-macos ジョブが
      `--locked` 未使用。全 cargo check/test へ付与。根拠 e4-dp。
- [x] `deny.toml` の expiry 本文（2027-02-19）と設定値（2027-02-18）を整合。
      根拠 e4-dp（前回 CI 修正で UTC 基準に合わせた経緯をコメントに残す）。

### 優先度: 低 — バックログ補充 2026-11-19

- [x] `app/src/settings.rs:353,365` の陳腐化した `#[allow(dead_code)]` を削除
      （load_with_notice / take_pending_notice は本番呼び出し済み）。根拠 e1-st。
- [x] DEB の Maintainer 既定値 `noreply@example.com` を実際の連絡先へ変更するか
      ビルド時必須化。根拠 e4-dp。
- [x] 403 の FORBIDDEN_GUIDANCE が利用者に変更手段の無い Origin 確認を求める。
      「別オリジンは非対応」という実態へ文言修正。根拠 e5-vu。
- [x] 設定破損退避物の一覧・復元手順・保持上限を追加（現状は退避のみ）。根拠 e5-vu。
- [x] ネイティブ probe 失敗に再試行操作を追加（現状は再起動のみ）。根拠 e5-vu。
- [x] wasm の inline JS ダウンロードを web-sys API へ置換するか、判断理由を ADR 化。
      根拠 e1-st。

### 要判断（設計・外部依存）

- [x] トークン自動生成時の受け渡し経路: 値をログに出さない現状は security 上正しいが、（XDG state 配下へ 0600 保存しパスのみ案内、値は非出力 — M1-TOKEN）
      既定起動ではブラウザ接続が完了しない（README の #token= 案内と矛盾）。
      0600 ファイル保存＋パス案内、またはトークン源未指定時に起動拒否、等の判断が必要。
      根拠 e5-vu。
- [x] 全成果物ジョブの needs に wasm / native-windows / native-macos / msrv-1-92 を追加（gates, deny, wasm, native-windows, native-macos, msrv-1-95 を指定 — M2-CI）
      （現状は [gates, deny] のみで、他プラットフォーム失敗時に成果物が残る）。
      branch protection 側で代替する選択もあり。根拠 e4-dp。
- [x] ツール版（cargo-vet / machete / markdownlint / gitleaks / trunk 等）の
      Dependabot 対象化。mise.toml 等の一元管理が必要。根拠 e4-dp。

### 優先度: 低 — テストギャップ 2026-11-19（D3-TQ01）

- [x] エクスポート JSON が wire スキーマから漂移: `crates/tm-core/src/export.rs:97`
      は「同じ envelope keys」と明言するが `sampled_elapsed` を出力しない。
      形状テストがハードコードキーのため新規フィールドを検出できない。
      出力するか serde envelope との差分比較へ変更。根拠 e3-tq。
- [x] ビューモデルの per-signal capability 拒否分岐にテストが無い
      (`crates/tm-viewmodel/src/lib.rs:829`)。Windows の Stop/Continue で
      pending_kill が作られず固定メッセージが出ることを pin。根拠 e3-tq。
- [x] テーマのグローバル AtomicU8 を並列テストが書き換えフレークしうる
      (`app/src/theme.rs:70`, `web/src/theme.rs:71`)。直列化と復元を追加。根拠 e3-tq。
- [x] SortKey の User/Status/Threads/StartTime の並び替えテストが無い
      (`crates/tm-core/src/query.rs:266`)。9 分岐すべてを pin。根拠 e3-tq。
- [x] CPU busy カウンタ逆行の分岐が未到達 (`crates/tm-core/src/metrics.rs:64`)。
      total 増・busy 減のケースを追加し 0% を固定。根拠 e3-tq。
- [x] truncate_label の境界値（0 文字・多バイト）が未テスト
      (`app/src/app.rs:2563`)。根拠 e3-tq。
- [x] トークンファイルの I/O エラー経路（存在しない/読み取り不能/非 UTF-8）が
      未テストで、フルパス非漏洩契約も検証されていない
      (`agent/src/main.rs:359`)。根拠 e3-tq。
- [x] 並行接続時の warm-up が 1 回だけ消費されることを検証するテストが無い
      (`agent/src/main.rs:948`)。根拠 e3-tq。

### 優先度: 低 — バックログ補充 2026-11-19（第2次）

- [x] `trend_delta` の BytesPerSecond 下降分岐 (`-magnitude`) が未テスト
      (`crates/tm-viewmodel/src/lib.rs:167`)。根拠 k1-tests。
- [x] `is_loopback_http_endpoint` の IPv6 不正形（']' 欠落・空ポート等）が未テスト
      (`web/src/agent.rs:240`)。根拠 k1-tests。
- [x] `ProcessStatus` の label()/id() 全 7 バリアントの pin テストが欠落
      (`crates/tm-core/src/model.rs:110`)。根拠 k1-tests。
- [x] `ExitCategory::as_str()` の 6 カテゴリ文字列が未テスト
      (`agent/src/telemetry.rs:264`)。根拠 k1-tests。
- [x] `query_param` の '=' を持たないキーだけの分岐が未テスト
      (`agent/src/http.rs:357`)。根拠 k1-tests。
- [x] ドキュメント整合（第2次）: DESIGN AC-12 の RESEARCH 判定を「一致」へ、
      notes/web-shell.md の IME「（ASCII のみ）」削除、README/DESIGN の
      Actions 記述を SHA 固定へ、notes/cross-platform.md を履歴スナップショット
      明記、web-shell.md のスクリーンショット枚数を 26 枚へ。根拠 k2-docs。
- [x] `disk.mount_point` のサニタイズ上限が 64 (sanitize) と 240 (native probe)
      で不一致。`MAX_COMMAND_CHARS` へ統一。根拠 k3-bugs。
- [x] `sanitize_display` が上限文字数ちょうど＋末尾空白で不要な省略記号を付ける
      (`crates/tm-core/src/sanitize.rs:85`)。trim 後に判定。根拠 k3-bugs。
- [x] `PlatformInfo` の能力フラグ契約 (`crates/tm-core/src/port.rs:43`) が
      native probe の実挙動と矛盾。ドキュメントを実装へ合わせる。根拠 k3-bugs。

### 優先度: 中 — 新機能統合後レビュー 2026-11-19

- [x] SSE が method 検証／プリフライトをバイパス: `agent/src/main.rs:940` の
      `STREAM_PATH` 分岐が route() を通らないため OPTIONS が 401、POST でも
      ストリーム開始する。GET のみ＋OPTIONS preflight を追加。根拠 q1-sse。
- [x] Authorization 必須 SSE のクライアント認証経路を確定: EventSource は（サーバ側は method/Host/CORS を整備済み。EventSource vs fetch()+ReadableStream の選択と ACA-Credentials 設計は未決定 — q1-sse）（fetch() + ReadableStream を採用、Bearer ヘッダ、generation 変更で再接続、EOF/エラー時にポーリングへフォールバック、DOM proof 末尾に sse= 追加 — T-SSE）
      カスタムヘッダ不可、fetch()+ReadableStream なら CORS preflight と
      ACA-Credentials の設計が必要。根拠 q1-sse（設計判断）。
- [x] 新規 wire enum (PowerSource / BatteryState / SamplingProfile /
      WindowVisibility) へ `serde(other)` を追加し、未知値でスナップショット
      全体が失敗するのを防ぐ。根拠 q2-data。
- [x] redact 後に batteries/cgroups の再ソート・再グループ化: `crates/tm-core/src/redact.rs:232`
      の事後置換でラベル衝突・順序違反が起きる。根拠 q2-data。
- [x] プリセット適用後に文字入力すると構造条件だけ残る:
      `crates/tm-viewmodel/src/lib.rs:820` の SetFilterText が filter.text のみ
      更新し高 CPU 条件が残る。根拠 q3-ui。
- [x] プリセットチップを Role::Button 化＋Tab/Enter のキーボード操作を追加
      (`web/src/widgets.rs:242`, `web/src/shell.rs:604`)。根拠 q3-ui。
- [x] AC online 読み取り失敗を「バッテリー動作」と誤判定しない:
      `crates/tm-platform/src/lib.rs:519` の unwrap_or(0) を区別。根拠 q2-data。
- [x] Esc/プリセットクリア時に IME 隠し textarea へ通知しミラーを同期
      (`web/src/shell.rs:653,664`)。根拠 q3-ui。
- [x] SSE のグレースフルシャットダウン: 無期限 read timeout が 2s join を超過、
      初回 warm-up が probe mutex 保持中に 250ms sleep。根拠 q1-sse。
- [x] SSE の Host 必須化、parse_sse_event_frame の event 重複拒否、
      trace.finish(200) を write_sse_head 後に移動。根拠 q1-sse。
- [x] 行コンテキストメニューのキーボード操作と stale PID 検証
      (`web/src/shell.rs:604,386`)。根拠 q3-ui。
- [x] DOM proof の filter/ime_field から '=' と改行を除去
      (`web/src/shell.rs:2337`)。根拠 q3-ui。

### 優先度: 中 — SSE クライアント差分レビュー 2026-11-19

- [x] SSE 経由の SnapshotEnvelope が redact/sanitize をバイパス（high）:
      `into_response()` で単一ゲート化、8MiB 境界を >= に統一、フレーム分割の
      增量スキャン化。根拠 u1-sec / u2-state。
- [x] stream_mode=Active が接続確立前に設定されポーリングが停止（medium）:
      初フレームまで Fallback 維持、アイドル watchdog（間隔×3）、sse_epoch
      ガードを on_snapshot にも適用。根拠 u2-state。
- [x] ヘッダ到達後無送信ストリームのウォッチドッグ欠落（medium）。根拠 u1-sec。
- [x] generation 変更が read().await 中にブロックされる（low）: ドキュメントを
      実装に合わせ修正。セットアップ中の fetch detach は依存制約をコメント化。
      根拠 u1-sec / u2-state。
- [x] フレーム分割の O(n^2) 再走査（low）。根拠 u1-sec。

### 優先度: 中 — 最終監査 2026-11-19（第3次）

- [x] watchdog の 3× sampler 周期が 250ms 設定で 750ms となりエージェントの
      1s フレームを誤判定、再接続ループに陥る (`web/src/shell.rs:3181`)。
      固定下限（3s 以上）を max 適用し 250/500ms ケースを固定。根拠 w1-diff。
- [x] SSE 端点エラーが直前フレームを stale と誤表示 (`web/src/shell.rs:552`)。
      SSE エラー専用ハンドラで stale を変更しない。根拠 w1-diff。
- [x] SECURITY/DESIGN が SSE を「未実装」と誤記（high）。実装済みの認証・
      Host/CORS・接続枠を追記、AC-12 マトリクスに SSE 行を追加。根拠 w2-public。
- [x] プリセット／電源／cgroup の README/DESIGN 未反映、cgroup の「未実装」
      誤記を訂正。根拠 w2-public。
- [x] DESIGN のトークン永続化記述を SECURITY/README と整合（0600 保存・
      再起動再利用）。根拠 w2-public。
- [x] notes/current-state.md のテスト件数を 485/130 へ更新。根拠 w2-public。
- [x] markdownlint ゲートへ packaging/linux/README.deb.md を追加
      (scripts/check.sh, .github/workflows/ci.yml)。根拠 w2-public。
- [x] テスト追加: SSE 実経路の 403→Forbidden、セットアップ deadline が
      長寿命ストリームを壊さないこと、watchdog 時刻逆行、DOM proof の
      active→fallback 即時更新。根拠 w3-tests。

### 優先度: 中 — 未レビュー領域の探索 2026-11-19（第4次）

- [x] ステータスバーの AccessKit live 文言が --redact を迂回（high）:
      `app/src/app.rs:2153` の aria_value / live ノードへ self.display() 済み
      文字列を渡す。根拠 y1-native。
- [x] cgroup 詳細行の生ラベルが --redact で伏せ字化されない（medium）:
      `app/src/app.rs:2011` を self.display(&group.label) 経由へ。根拠 y1-native。
- [x] SSE の OPTIONS/405 が Host 検証をバイパス（medium）:
      `agent/src/main.rs:963` の分岐前に is_allowed_host を追加。根拠 y3-server。
- [x] shutdown AtomicBool が accept ループから監視されず 8s join がデッドコード（high）: シグナル配線は #![forbid(unsafe_code)] / 依存追加 / blocking accept の理由で見送り、コメントを「accept ループ終了後の後始末」に訂正して graceful を謳わない（Z2-AGENT）。根拠 y3-server。
- [x] DEB 同梱 README が新機能未反映、--skip-build の版数不一致、
      チェックサム manifest 欠落（medium）。根拠 y2-deb。
- [x] 認証失敗 auth.denied に request_id が無く --log-level error で抑制
      (`agent/src/main.rs:1214`)。相関 ID 付与と Level::Error 昇格。根拠 y3-server。
- [x] SSE ハッピーパス（1 フレーム→shutdown→streams 0）のテスト欠落、
      streams ゲージが /metrics に未公開。根拠 y3-server。
- [x] リリースバイナリの --remap-path-prefix / strings 検査が無い
      (Cargo.toml:41, ci.yml:83)。成果物の開発パス混入を防止。根拠 y2-deb。
- [x] release-linux / web-dist / sbom のビルドが --locked でない。根拠 y2-deb。

### メモ — strings ゲート対応の経緯

- release-linux の strings ゲート初回失敗は、remap-path-prefix のルール順序が
  逆で `/build/koppe/.cargo/...` が残ったことと、redact のマスク先
  `/home/user` がデモデータ・テスト期待値に混在していたことが原因。
  一般的な `/home/` `/Users/` を先に、具体的プレフィックスを後に置き、
  マスク先を `<home>` へ統一して全パターン 0 件を確認済み。
  Tailnet ドメインのパターンは、redact 処理に必要なサフィックス定数自体を
  誤検出しないよう、実ホスト名を伴う形式へ絞った。

### 優先度: 中 — 未調査領域の探索 2026-11-19（第5次）

- [x] `deny.toml:23` の `all-features = false` を true へ変更し optional 依存も
      監査対象に（a2-supply が 0.21.0 で成功実証済み）。
- [x] CDP ゲートの py_compile / self-check 対象へ `poc-wasm/scripts/cdp_verify.py`
      を追加。PoC の CDP 切断時 pending future 解除も web 版から移植。
- [x] cdp_verify_ime.py のローカル parse_proof を web 共有版へ置換、
      self-check に現行 DOM proof 全キー（sse/preset/power/sampling/cgroups 等）
      の gold 文字列を追加。
- [x] web/README の SSE 優先＋フォールバック記述、DOM proof キー一覧、
      document.title 固定契約、構成表（settings.rs/redaction.rs）を実装へ同期。
- [x] notes/current-state.md の内訳（agent 104 / app 94）と履歴 485→491 を修正。
- [x] PoC cdp_verify.py の出力先既定値を README と同じ ../../notes/assets へ。
- [x] Dependabot ignore へ sysinfo / rustix の完全一致ピンを追加するか、
      方針文書を整合（a2-supply）。

### メモ — wasm 時刻 panic 回帰の記録

- 2026-11-19 に SSE watchdog (4e0e3db) が sampler 毎周期で
  `std::time::Instant::now()` を呼び、wasm32 で `time not implemented` panic。
  App borrow が解放されず Web デモの tick/rows が 0 固定になった。
- `web-time` 1.1.0（gpui-pre の推移依存）を direct dependency 化し、
  shell.rs の Instant / settings.rs の SystemTime を置換、sampler では
  StreamMode::Active のときだけ watchdog を評価する防御を追加。
- CDP Runtime Proof: tick=3 / rows=28 / borrow|panic|unsupported 0 件
  （修正前 tick=0 / borrow error 2229 件）。

### メモ — CI apt ミラー一時障害

- 2026-11-19 の CI で Ubuntu ミラーが 503 を返し、gates / MSRV の
  `apt-get` が失敗（コードとは無関係、再実行で解消）。
- ci.yml の MSRV / release-linux ジョブへ `Acquire::Retries=5` と
  `Acquire::http::Timeout=30` を追加済み。
- gates ジョブは再利用ワークフロー `Seduce0068/workflows` の `apt_packages`
  入力経由のため、リトライ強化は提供側リポジトリの変更が必要（未対応）。
  AGENTS.md §5 に従い、まず提供側で対応を検討する。

### 優先度: 高 — ユーザビリティ／ドッグフーディング 2026-11-19

- [x] 401 案内／オンボーディングが存在しない「再起動後の新しい #token= URL」
      を要求 (`web/src/agent.rs:96`, `:672`)。実装はトークンファイル再利用のため
      新 URL は発行されない。トークンファイルの内容を #token= に貼る案内へ修正。
      根拠 u1-onboard / u3-messages（独立一致）。
- [x] デモ通知が `[エラー]` 表示で接続案内が右端で切れる
      (`web/src/shell.rs:334`, `web/src/agent.rs:104`)。info 扱い＋短い告知へ
      変更し、詳細は常設ガイドへ。根拠 u1-onboard / u4-layout / u5-dogfood。
- [x] ソート矢印 ▲/▼ が IBM Plex で欠落（tofu）し asc/desc が視覚的に同一
      （CDP 拡大画像 SHA256 一致）。描画確認済みの ↑/↓ または「昇順/降順」
      テキストへ変更。根拠 u5-dogfood。
- [x] Web キーボード操作を native と対称化: F1 ヘルプ、F2 詳細、Space 一時停止、
      Ctrl+F/Ctrl+K の読み取り専用通知。編集中はナビゲーションキーを
      キャレットへ渡す（現在は行選択が動く）。根拠 u1/u2/u5。
- [x] Web に F1／ヘルプボタンでショートカット一覧を追加（native と共通テーブル）。
      根拠 u1-onboard / u2-keyboard / u5-dogfood。

### 優先度: 中 — ユーザビリティ／ドッグフーディング 2026-11-19

- [x] 403 HTTP 本文の内部用語（Host ヘッダ/Origin/ループバック）を
      FORBIDDEN_GUIDANCE と同じ実態案内へ統一 (`agent/src/main.rs:1011` 等)。
      根拠 u3-messages。
- [x] 一時エラー (5xx/429/timeout) に自動再試行と次の行動を案内。
      503 body.message（同時接続数上限）を反映。根拠 u3-messages。
- [x] スキーマ不一致文言を「エージェントとアプリのバージョンが一致しません」
      へ（内部変数名と数値を隠す）。根拠 u3-messages。
- [x] 接続先ポートの混在を解消: 画面案内 8787 / README 例 18787
      (`README.md:134`, `web/src/shell.rs:104`)。根拠 u1-onboard。
- [x] 更新間隔 0.5s が INTERACTIVE_INTERVAL_MS=1000 の下限で実効せず
      1 件/秒のまま。選択不可にするか「1s に制限」と表示。根拠 u5-dogfood。
- [x] デモの「高 CPU」プリセットが常に 0 件（デモ最大 27.2% < 閾値 50%）。
      デモデータに 50% 超を 1 件追加するか空状態に条件を表示。根拠 u5-dogfood。
- [x] プロセス数を小さい kv から text_xl + BOLD へ昇格。
      使用率の深刻度を色だけでなく「高/中/低」テキストでも表示。
      根拠 u4-layout。
- [x] デモ詳細パネルのオンボーディングカードが行未選択時のみ表示になるよう
      調整（選択時に本命情報へ到達可能に）。根拠 u4-layout。

### 優先度: 高 — 敵対的レビュー／実測 2026-11-19

- [x] トークンファイル権限チェックが `mode & 0o044`（read のみ）で
      `0o622` 等の group/other write 可能ファイルを受け付ける。
      エラー文は 0600 を要求しているため矛盾。`0o077` または厳密 `0o600`
      へ修正（`agent/src/main.rs:431`）。根拠 x3-sec。
- [x] 401 案内の「再起動ではトークンは変わりません」が --token /
      TM_AGENT_TOKEN 経路では偽。条件付き文言へ（`web/src/agent.rs:97`）。
      根拠 x3-sec。

### 優先度: 中 — 敵対的レビュー／実測 2026-11-19

- [x] 自動トークン生成の TOCTOU: exists → read → generate → write が
      無ロックで、並列起動でメモリ内トークンとファイルが不一致になり得る
      (`agent/src/main.rs:544`)。create_new 等で atomic 化。根拠 x3-sec。
- [x] `--once --demo` が warm-up sleep 250ms を無条件に実行（全体 251ms の
      99%）。デモ probe は決定的なので sleep をスキップし、実 probe のみ
      待つ分岐へ（`agent/src/main.rs:598`）。根拠 x2-perf。
- [x] SSE 2 スロットが切断後も最大 1 秒保持され、1 秒以内の再接続が
      503 になる。切断検知を早めるか QUICK_RELEASE を検討
      (`agent/src/main.rs:1191`)。根拠 x2-perf。
      → フレーム間の待ちを 100 ms スライスへ分割し、各スライスでソケットを
      probe（read timeout + EOF 検知）して切断と shutdown を早期検知。
      新規依存なし、Bearer / Host/Origin / redact / StreamSlot RAII /
      MAX_STREAMS=2 の契約は維持。回帰テスト
      `stream_slot_is_released_soon_after_client_disconnect` を追加。

### 優先度: 高 — 実AXツリー監査 2026-11-20

- [x] Web の AccessKit セマンティクスがブラウザ AX ツリーに到達しない。
      `Accessibility.getFullAXTree` で columnheader/row/dialog が 0 件、
      フィルタは無名 textbox、使用率の「高/中/低」と温度状態も不在。
      canvas 内の GPUI AccessKit ノードを DOM ミラーへ公開するか、
      主要状態（テーブル見出し・行・選択・ヘルプ・フィルタ値）を
      role/aria 付き DOM として生成する。根拠 za-ax。
- [x] ヘルプ表示時に DOM の activeElement が隠し IME textarea のままで
      「閉じる」へフォーカスが移らない。ヘルプを role=dialog +
      aria-modal 付き DOM ミラーにし、表示時に閉じるボタンへ
      DOM フォーカスを移す（閉じたら復帰）。根拠 za-ax。
- [x] DOM proof と AX ツリーの状態矛盾（rows=30 / dir=desc / selected_pid=2500
      なのに AX に該当ノードなし）。CDP 検証に proof↔AX 一致チェックを追加
      し、偽陽性を防ぐ。根拠 za-ax。

### メモ — エラー回復シナリオ検証 2026-11-20

- SIGTERM 後 fallback 保持、同一 0600 トークンでの自動再接続（5s クールダウン）、
  トークン変更時の 401 案内、0644 権限での起動拒否、トークン非露出、
  ポート cleanup を実動で確認。修正項目なし（z2-recovery）。

### 優先度: 高 — 実AX再検証 2026-11-20

- [x] ヘルプ表示中に F1 / Escape / 「閉じる」クリックでダイアログが閉じない。
      DOM ミラーの閉じるボタンへフォーカスを移した結果、GPUI のキーイベント先
      （隠し IME textarea）へ入力が届かない。DOM ミラーにも閉鎖ハンドラを付け、
      GPUI 側へ Command を送る経路を確保。根拠 ac-axverify。
- [x] 絞り込みへ文字列を入力すると平文が AX textbox value / StaticText と
      DOM textarea.value / data-gpui-proof の filter,ime_field に残る。
      ミラーの aria-valuetext は伏せ字だが入力表面が未マスク。
      トークンが貼り付けられた場合の漏洩経路を塞ぐ。根拠 ac-axverify。
- [x] CDP proof↔AX warning 2 件: row=31（ヘッダ行 role=row を含む）vs proof 30、
      aria-sort が AX property として露出せず dir 照合が失敗。期待値と照合方法を
      実 AX に合わせて修正（構造は正しいため warning 解消でよい）。根拠 ac-axverify。

### 優先度: 中 — CLI/データ往復監査 2026-11-20

- [x] `?limit=N` で切り詰めた wire を export すると、`total_process_count=30`
      が `processes.len()=3` に上書きされ総数が失われる (`export.rs:125`)。
      envelope の総数を明示保持するか「掲載行数」と明記。根拠 ba-roundtrip
      （過去 c2-di01 の指摘が未修正）。
- [x] SSE エンドポイント `/api/v1/stream` が `tm-agent --help` と README に
      未記載（実装・テスト済み）。args.rs:54 のエンドポイント一覧と
      README エージェント節へ追記。根拠 bb-cli。
- [x] GUI が位置引数を黙って無視する一方 tm-agent は終了コード 2 で拒否。
      `app/src/main.rs:335` の _ アームを agent と揃えるか、許容するなら
      ヘルプに明記。根拠 bb-cli。
- [x] GUI の `--version/--help` が他フラグの検証をバイパスし、
      `--bogusflag --version` が exit 0（tm-agent は exit 2）。
      全引数検証後に version/help を処理する順序へ統一。根拠 bb-cli。

### 優先度: 中 — 回帰／設定マイグレーション監査 2026-11-20

- [x] AX ミラーが role="table" のため Chrome が aria-selected の selected state
      を落とす（AX properties に selected 0 件）。role="grid" + gridcell へ変更
      して state を暴露し、CDP 検証も property で厳格化。根拠 da-regress。
- [x] native の保存失敗ステータスだけ transient（web は persistent）。
      `app/src/app.rs:241` を set_persistent_status へ変更しテスト更新。
      根拠 db-migrate。
- [x] web の corrupt 退避キーが 1ms 分解能で衝突し得る。`-N` サフィックスを
      解釈するだけのデッドロジックを生成側へ反映（既存キー列挙）。根拠 db-migrate。

### メモ — 実動確認 2026-11-20

- F1 ヘルプ3経路、64-hex proof マスク、ソート反転と AX 名、選択行同期、
  エクスポート total 保持、CLI exit code、v1→v2 読み込み、未来 version 保持、
  opaque version、corrupt 6 連鎖（5件 prune）、保存失敗通知は実動で確認。
  修正項目は上記 3 件のみ（da-regress / db-migrate）。

### 優先度: 低 — デッドコード／旧記述の整理 2026-11-20

- [x] `sanitize_and_redact_wire_snapshot` を `redact_and_sanitize_wire_snapshot`
      へ改名（実処理順 identify→redact→sanitize と名前を一致）。pub(crate)
      のため外部互換リスクなし。根拠 fb-deadcode。
- [x] export.rs の cherry-pick 痕跡: `redact_text` の unused import と
      `#[allow(unused_imports)]` を削除し、doc リンクを完全パス化。根拠 fb-deadcode。
- [x] `snapshot_to_csv_with_total` の引数名 `_total_process_count` を
      `_ignored_wire_total` 等へ改名し、CSV は envelope 総数非対応である旨を
      API 名/doc で明示（JSON 版と挙動が異なるため誤用防止）。根拠 fb-deadcode。
- [x] 修正済み項目の旧記述を削除: query.rs の `todo 2026-11-19` マーカー、
      DESIGN.md の「フォーカス移動によるキーボード完結は将来の改善項目」。
      根拠 fb-deadcode。
- [x] `DemoProbe::with_tick` が未使用（doc は 'used by tests' だが呼び出し0件）。
      公開ライブラリのため削除せず、doc を「テスト用の設定ヘルパ」と明記
      （`#[cfg(test)]` は外部利用者から API を奪うため適用しない）。根拠 fb-deadcode。

### メモ — 長時間安定性実測 2026-11-20

- 30s＋60s の実測で VmRSS/Threads/fd に単調増加なし、SSE 235 回切断で
  スロット 0 復帰、snapshot p95 1.3ms 以下・時間劣化なし、unknown=0。
  リーク経路なし（fa-stability）。

### 優先度: 高 — マルチクライアント／負荷監査 2026-11-20

- [x] 2タブ／2インスタンスで設定の既知フィールドが lost update。
      merged_map が known fields を呼び出し側で全上書きするため、保存直前に
      再読込しても version/未知フィールドしか守られない。差分マージまたは
      revision CAS で片方の更新が消えないようにする（web/native 両方）。
      根拠 ga-multitab。
- [x] 他タブの保存が UI に反映されず表示と localStorage が乖離（storage
      イベント未購読）。再読込でサイレントに戻るため誤認を招く。
      STORAGE_KEY の storage イベントを購読し再ロード＋通知。根拠 ga-multitab。
- [x] SSE と認証済み snapshot が並行すると warm-up サンプルが2回消費
      （初回1回契約違反）。SSE 経路で discarded sample 成功直後に warmed_up
      を store してから待つ。根拠 gb-stress。
- [x] /metrics の active_connections が問い合わせ元自身を計上しアイドルでも
      1 を返す。自己を除外するか 1 を基準と明記。根拠 gb-stress。

### メモ — 仕様どおりで修正不要 2026-11-20

- 3タブ目の SSE 503→fallback は MAX_STREAMS=2 の graceful degradation、
  sse_epoch ガードは再接続後に古いフレームを正しく無視。9177 リクエストの
  負荷でハング/クラッシュ/リーク/unknown=0 を確認（ga-multitab / gb-stress）。

### 優先度: 高 — 公開準備最終監査 2026-11-20

- [x] リリース成果物（binary + wasm）に開発 worktree パス
      （内部開発 worktree の絶対パス）が混入。remap-path-prefix が
      `/home/` → `/build/` に置換するため既存ゲートがすり抜ける。
      release-linux ゲートへ内部 worktree パターンを追加し、
      wasm/dist も走査対象に含める。根拠 ib-publish。
- [x] README / notes/current-state / DESIGN §12.1 のテスト件数を
      workspace 509（core 174 / vm 80 / platform 44 / agent 111 / app 100）
      / web 167、日付 2026-11-20 へ更新。根拠 ib-publish。
- [x] 実装済みの AX ミラー（role=grid）とマルチタブ三方マージ／storage 同期を
      DESIGN（ADR 追加）と README「できること」へ反映。根拠 ib-publish。
- [x] NOTICE.md の直接依存一覧を修正: console_error_panic_hook は推移依存へ、
      web-time 1.1.0 を直接依存として追加。根拠 ib-publish。
- [x] wasm サイズ記述（7,642,794 B / 2026-11-17「再ビルド未実施」）を
      現行 dist の 7,949,118 B / gzip 3,165,445 B / 2026-11-20 へ更新。
      根拠 ib-publish。

### メモ — Web 長時間安定性 2026-11-20

- 60 秒計測で DOM ノード 164 / AX row 31 / proof キー 30 が一定、tick 単調、
  ヒープは GC ノコギリでネット -830KB、console error 0、2 タブ同期も動作。
  修正項目なし（ia-webstab）。

### 優先度: 高 — サプライチェーン監査 2026-11-20

- [x] heapless 0.9.3 の unsound 脆弱性 9 件（RUSTSEC-2026-0574..0582、
      use-after-free／データ競合等）が修正版 0.9.4 公開済みにもかかわらず
      cargo-deny に検出されず cargo-audit も警告扱いで CI を通過。
      root / web の Cargo.lock を 0.9.4 へ更新（gpui-pre は caret 制約で
      ピン変更不要）。更新後 deny / vet / audit を再検証。根拠 kb-supply。
- [x] deny.toml の advisory ignore 9 件の根拠先「DESIGN.md §14」に追跡記録が
      存在しない（RUSTSEC ID の記載なし）。§14 へ ID / 理由 / レビュー日 /
      expiry の表を追加。根拠 kb-supply。
- [x] .github/dependabot.yml の web 更新定義に rustix / sysinfo の完全ピン
      ignore が無く root と不整合。追加して揃える。根拠 kb-supply。
- [x] 検証ノートの「28 行保持」期待値を現行デモ件数 30 行へ更新
      （notes/web-live-runtime-verification.md 等）。回帰ではない。根拠 ka-recovery。

### メモ — 回復シナリオ再検証 2026-11-20

- 現行ソース再ビルド後、ライブ接続／SIGTERM fallback／同一トークン再起動
  （5 秒クールダウン）／0644 権限拒否／corrupt 退避 -N＋5件 prune／
  2 タブ差分マージの 6 シナリオすべて期待どおり。トークン露出・例外 0 件。
  回帰なし（ka-recovery）。

### 優先度: 中 — ビルド再現性／フレーキネス監査 2026-11-20

- [x] DEB がビルド時刻に依存（SOURCE_DATE_EPOCH 未設定）。同一入力で 23 秒
      隔てた 2 回の SHA256 が不一致。SOURCE_DATE_EPOCH=1700000000 で一致を
      実証済み。build-deb.sh で固定。根拠 mb-repro。
- [x] web/Trunk.toml に wasm-opt 版が未固定（version_131 が暗黙選択）。
      [tools] で wasm_opt / wasm_bindgen を固定。根拠 mb-repro。
- [x] remap-path-prefix が /home/ と /Users/ のみで checkout サブパスと
      CARGO_TARGET_DIR が残る（wasm 5 件、binary 1 件）。リポジトリルートと
      target 全体を置換対象に追加。wasm は wasm-opt --strip-debug 検討。
      根拠 mb-repro。
- [x] trunk 生成 index.html の modulepreload 順序が非決定的（HashSet 由来、
      Trunk 0.22.2 の copy_dir_recursive）。ビルド後にパス順へ正規化する
      ポストフックまたは上流更新。根拠 mb-repro。
- [x] ahash の compile-time-rng（zed-xim 経由）で app バイナリがビルドごとに
      変化（116 バイト差: build-id と乱数シード）。tm-agent は決定的。
      `.cargo/config.toml` の `[env] CONST_RANDOM_SEED` で compile-time シードを
      固定し、同一 target dir の clean ビルド 2 回で app バイナリ SHA256 一致
      （e0f257737dcb05e2fdbabfb52356a43a551f373807ab0dfe5092902255a260f0）を実証。
      zed-xim 側に feature オプションは無く（crates.io 最新 0.4.0-zed）、
      with_seeds / patch は不採用。根拠 mb-repro + oc-ahash、
      詳細は notes/current-state.md「ビルド再現性」。
- [x] CI に公開成果物の二重ビルド＋SHA256 比較ゲートが無い。上記 4 種の
      非決定性を検出できず公開後に事後発見し得る。根拠 mb-repro。

### メモ — フレーキネス 2026-11-20

- workspace 26 回（default 10 / serial 3 / threads16 3 / 他）+ web 10 回
  すべて成功、509/167 pass。時間依存アサート 4 件と PENDING_NOTICE 競合
  ウィンドウ 1 件は speculative（未再現）。app 側 NOTICE_TEST_LOCK 相当の
  導入は予防的価値あり（ma-flaky）。

### 優先度: 中 — 未調査観点の発掘 2026-11-20

- [x] stdout が閉じたパイプで panic（`--once | head` 等）。Rust は SIGPIPE を
      無視するため EPIPE で exit 101 + panic 行を stderr へ出す。CLI 出力を
      EPIPE で安全に終了させる。根拠 od-discovery。
- [x] `--token-file` が FIFO／デバイス等を受け付け起動ハングまたは無制限
      読取り（`std::fs::read_to_string`）。export/settings と同じ
      symlink_metadata の非通常ファイル拒否を追加。根拠 od-discovery。
- [x] 相対 XDG_STATE_HOME で自動トークンが CWD（リポジトリ内）に作られる。
      XDG_CONFIG_HOME のはabsolute 検査と対称に XDG_STATE_HOME も絶対パス
      検査を追加。根拠 od-discovery。
- [x] CSV 数式無害化が全角 '＝'（U+FF1D）starter を通す。
      export.rs:588-621 の neutralise 集合へ全角記号を追加。根拠 od-discovery。

### 優先度: 低 — 未調査観点の発掘 2026-11-20

- [x] 設定 JSON の UTF-8 BOM が破損誤判定される＋巨大ファイル無防御。
      parse_document / web/src/settings.rs で BOM 除去とサイズ上限を検討。
      根拠 od-discovery。
- [x] 自動トークンの絶対パスが stderr に出る（`--redact` 公開スクショでも
      漏れる）。generated_token_message を token_file_name と同じ leaf 名
      規律へ。根拠 od-discovery。

### 優先度: 中 — README再現性／ブラウザ互換監査 2026-11-20

- [x] README クイックスタートのコードブロックに長時間サーバー（cargo run /
      trunk serve）と後続クライアントコマンドが混在し、コピペすると停止する。
      別ブロック＋「ターミナル A / B」明記。wasm 節の `cd poc-wasm` は
      実在しない（`cd ../poc-wasm` が必要）。根拠 rb-readme。
- [x] README 別ポート例が未定義のシェル変数 ${token_file} を参照（新しい
      ターミナルでは未定義で 401）。各ブロック内で再定義。根拠 rb-readme。
- [x] README の curl 例が jq に依存するが前提パッケージ一覧に jq が無い。
      apt 一覧へ追加するか jq 依存を外す。根拠 rb-readme。
      → `curl -sS ... | head -c 500` へ変更し、apt 一覧は据え置き（理由を README に記載）。
- [x] README「トークン場所は stderr で案内」と実装（leaf 名「token」のみ）
      が不一致。記述を修正。根拠 rb-readme。
- [x] README「まず試す（最短）」に Rust 1.95+ の導入手順が無い（cargo
      command not found）。公式手順へのリンクを追加。根拠 rb-readme。
- [x] ブラウザ検証状況の記述が古い: web/README.md:302 と
      notes/verification.md:118 の「Firefox 未検証」、README.md:315 の
      「ヘッドレス Chrome のみ」。Chrome 158 / Edge 157 / Firefox 160 で
      検証成功（Firefox は BiDi・F1 は合成キーで信頼済みキー未確認）を反映。
      Safari / Brave / モバイル / 実 GPU は未検証のまま。根拠 ra-browsers。

### 優先度: 高 — ネイティブAX実検証 2026-11-20

- [x] ネイティブ版の F1 ヘルプダイアログが実 AX ツリーに存在しない
      （Dialog role / 「閉じる」ボタン欠落）。Web 版と同じ
      `.role(Role::Dialog).aria_label(HELP_TITLE)` とボタン role を付与。
      根拠 ta-nativeax。
- [x] ネイティブ版の全ボタン（一時停止／詳細／シグナル／ヘルプ等）が AX から
      欠落。`app/src/ui.rs:115-142` の action_button() へ Role::Button +
      label 由来の aria_label を付与。根拠 ta-nativeax。
- [x] 使用率ゲージ・温度状態・詳細パネルが AX に現れない。Web 版の
      Group + aria_label + ProgressIndicator と同等を native へ付与。
      根拠 ta-nativeax。

### 優先度: 中 — エクスポート実利用 2026-11-20

- [x] CSV の cpu_percent に f32 丸め誤差（4.3799996 等）がそのまま出力され
      可読性が低い。UI と同じ小数 2 桁へ丸めるか、生値である旨を明記。
      根拠 tb-export。
      → 完了: `export.rs` の `push_csv_percent` で出力時のみ小数 2 桁へ丸め
      （4.3799996 → "4.38"、1.9399999 → "1.94"）。JSON / 計算 / ソートは生値維持。
- [x] CSV の sampled_at_unix_millis（13桁 epoch）が表計算で日付にならず、
      README に列の意味・形式の説明が無い。列説明を追記するか ISO-8601 列
      を併設。根拠 tb-export。
      → 完了: README「エクスポート形式（CSV / JSON）」へ 12 列の意味・形式を表で
      記載。ISO-8601 列の併設は固定 12 列契約を壊すため見送り（README に判断記録）。

### メモ — 環境依存エッジ 2026-11-20

- TZ 3 タイムゾーンで --once 出力バイト一致、4 ロケールで日本語/CSV 同一、
  0622 トークン拒否、PID 再利用防御済み、ゼロ幅文字除去済み。
  問題なし（sa-signal / sb-env）。

### 優先度: 中 — 状態機械／カバレッジ監査 2026-11-20

- [x] 通知優先度が経路ごとに不一致。corrupt 退避直後に save 失敗すると（実装: should_announce_save_failure / sse_fallback_status の persistent ガード＋回帰テスト）
      `record_save_failure` (web/src/settings.rs:627) が corrupt 通知を
      無条件上書き、SSE fallback (shell.rs:692/737 の transient set_status)
      が persistent な corrupt/save-failed を消す。cross-tab 同期は
      persistent 保護済みなので矛盾。優先度比較またはキュー化＋回帰テスト。
      根拠 wb-state。
- [x] 行メニュー表示中に F2 が通り詳細パネルがトグルされる問題を修正。
      判断: 行メニューを native の確認ダイアログ（app/src/app.rs:672-687 の
      dialog_first / dialog_command が F2 等を無視）と同等のモーダルとみなす。
      row_menu.is_some() のとき resolve_web_key が F2(ToggleDetails) /
      Space(TogglePaused) / Ctrl+K(ReadOnlyNotice) を Swallowed として返し、
      on_key_down が stop_propagation して無視（隠し textarea の既定動作も
      防止）。Esc（早期分岐でメニューを閉じる）/ Enter / 矢印のメニュー操作は
      従来どおり。テスト row_menu_is_modal_for_f2_space_and_ctrl_k_until_esc_closes_it
      で詳細・status 不変と閉じ後の F2 復帰を固定（web/src/shell.rs）。
      根拠 wb-state（speculative → confirmed）。
- [x] web の HTTP スナップショット取得パイプライン（fetch_snapshot_inner（注: fetch_snapshot_inner の実経路は既存の live CDP 実動検証でカバー済み。SSE エラー分岐4件は XB-SSEERR で追加済み、残りは unit モック化の検討課題）
      の loopback 検証・Bearer・401/403 分類・redact ゲート）と agent SSE
      エラーフレーム（warmup 失敗・mutex poison・JSON 変換失敗）が
      カバレッジ 0。実パイプラインのテスト追加を検討。根拠 wa-coverage。
      YB-FETCH 完了（2026-11-20）: fetch_snapshot_inner から実パイプラインを
      自由関数 fetch_snapshot_once（HttpClient と世代判定を注入）へ抽出し、
      loopback 拒否 / Bearer ヘッダ / 401/403/429/5xx の型と固定文言 /
      8MiB 超 Content-Length の読み取り前拒否 / redact+sanitize ゲート /
      世代変更によるボディ読み取り中止 / parse・schema・transport 失敗を
      unit モックで検証。cargo test --lib 181 件 pass、
      clippy all-targets / wasm pass、agent.rs は fmt 済み。

### メモ — カバレッジ実測 2026-11-20

- cargo-tarpaulin 0.37.6 で計測: workspace 62.31%（core 96.87 / vm 95.11 /
  platform 91.46 / agent 81.11 / app 25.44）、web 31.60%（redact/theme 100、
  settings 85.47、agent 73.10、shell 17.38）。低いのは GPUI 描画関数で
  CDP 実動検証で補完済み。初回 statvfs テスト 1 件の失敗は環境依存フレーク
  （再実行 pass）。根拠 wa-coverage。

### 優先度: 中 — ドキュメント整合最終監査 2026-11-20

- [x] SECURITY.md:185 の「Safari / Firefox 未検証」を更新（Firefox 160 は
      2026-11-20 実動確認済み）。:190 のゼロ幅文字記述も実装（全 Cf 除去）
      へ整合。根拠 za-links。
- [x] DESIGN.md:843 の「ゼロ幅文字は除去しない」記述を削除（:393-394 と
      実装は除去済み）。根拠 za-links。
- [x] notes/web-shell.md:299 の旧制約を更新（Firefox 160 は 2026-11-20 に
      WebDriver BiDi で実動確認済み、Safari と実 GPU WebGPU は未検証）。
      wasm サイズを現行値（7,949,118 B / gzip 3,165,445 B、2026-11-20）へ更新。
      根拠 za-links。
- [x] notes/verification.md:110 のスクリーンショット枚数 18→26 を修正
      （実ファイル 26 枚を確認）。:433 / :473 の Firefox 未検証旧記述へ
      §6「既知の制限・未検証」のブラウザ互換性の項（2026-11-20 実測）への
      参照を追加。根拠 za-links。
- [x] 存在しない notes/assets/task-manager-demo.png 参照を現存画像へ差し替え（notes/review-2026-11-14.md:66 を現存画像へ、:156 は参照を削除）
      （notes/review-2026-11-14.md:66,156、M6 の結論は維持）。当該ファイルは
      担当外のため未変更、親へ報告済み。根拠 za-links。
- [x] notes/residual-tasks-research.md:403 の F1 ヘルプ未実装記述に
      2026-11-19 実装済み（V2-HELP）の解決注記。根拠 za-links。
- [x] README 冒頭に製品バージョン 0.1.0 を明記し、schema_version=1 /
      SETTINGS_VERSION=2 / OKF verified（日付）と別概念である旨を注記。
      Cargo.toml の workspace.package.version と同期。根拠 zb-version。

### メモ — ドキュメント整合（R2 主実行、2026-11-20）

- 担当 4 件（README バージョン / web-shell / verification / residual-tasks）を
  完了。画像参照差し替えは notes/review-2026-11-14.md が担当外のため未変更。
- 検証: `bash scripts/check-public-docs.sh` pass、markdownlint 既存 baseline を
  増やさないことを確認。根拠 za-links / zb-version。

### メモ — バージョン整合 2026-11-20

- Cargo.toml / CLI --version / DEB control すべて 0.1.0（0.1.0-1）で一致、
  schema_version=1 と SETTINGS_VERSION=2 も正しく区別。DEB Maintainer の
  `Name <email>` 形式は架空アドレスを避ける方針で意図的に未変更（low）。
  根拠 zb-version。

### 優先度: 高 — ライセンス同梱完全性 2026-11-20

- [x] DEB に MIT/ISC/BSD/Zlib 等のライセンス全文・著作権表示が同梱されない
      （783 crates の静的リンク依存）。cargo ローカルレジストリからライセンス
      全文を収集して同梱する仕組みを追加。根拠 wa-license。
      完了（R2 za-deblic）: build-deb.sh が `cargo metadata --offline` の
      manifest_path からローカルレジストリの全文 1259 件を licenses/crates/
      へコピー。実測 804 registry packages のうち 724件を .deb で確認。
      残り 80 件は NOT-VENDORED.txt に一覧化し、README.deb.md の
      「cannot ship」を「not vendored yet」へ修正。
- [x] Lucide(ISC)/Feather(MIT) アイコンのライセンス本文（gpui-kit-assets
      の LICENSE-LUCIDE）が未同梱。vendoring して同梱。根拠 wa-license。
      完了（R2 za-deblic）: .deb 内
      licenses/crates/gpui-kit-assets-0.9.1/LICENSE-LUCIDE の存在と ISC
      本文・Feather 由来 MIT 帰属を確認。
- [x] Web 配布アーティファクト（web/dist）に AGPL LICENSE と NOTICE／（CI web-dist が LICENSE/NOTICE/THIRD_PARTY_NOTICES/IBM Plex を dist へコピー、再現性ゲート対象）
      第三者ライセンス本文が含まれない。CI の web-dist へコピー。根拠 wa-license。
- [x] debian/copyright が Files: * で同梱文書を AGPL と誤表示。Files stanza
      を分割（OFL-1.1 / Apache-2.0 / 依存クレート類）。根拠 wa-license。
      （差分: `packaging/linux/copyright` を DEP-5 の 4 stanza へ分割。catch-all
      `Files: *`=AGPL-3.0-or-later、`licenses/LICENSE-IBM-Plex.txt`=OFL-1.1、
      `THIRD_PARTY_NOTICES.md`=Apache-2.0、`licenses/crates/*`=全 family。
      python3-debian で parse と last-match を検証。）
- [x] NOTICE.md の帰属表が依存グラフ（783 crates）を網羅していない。
      主要ライセンス family の列挙または自動生成。根拠 wa-license。
      （差分: `NOTICE.md` へ deny.toml allow/exceptions + root/web の cargo
      metadata から導出した全 18 family を表で追加。個別クレートは
      `licenses/crates/` の同梱全文を参照する旨を明記。）

### メモ — 依存鮮度 2026-11-20

- advisory DB 更新後も cargo audit / deny / vet すべて問題なし、更新可能な
  脆弱依存なし。deny ignore 9 件も 3 か月ルール内（wb-advisory）。

### 優先度: 高 — 配布物開示／SBOM整合 2026-11-20

- [x] web SBOM がホスト x86_64 対象で wasm32 成果物と不一致（wasm 固有
      41 packages 欠落、Linux 固有 76 過剰）。generate-sbom.sh で
      `cargo cyclonedx --target wasm32-unknown-unknown` を明示。根拠 wb-sbom。
      完了（R2 WA-SBOMTGT）: root に `x86_64-unknown-linux-gnu`、web に
      `wasm32-unknown-unknown` を明示。web SBOM は 368 → 332 components
      （各自の metadata.component を合わせ 333）となり wasm32 の
      `cargo metadata --filter-platform` 333 packages と集合一致。
      Linux 固有 `linux-raw-sys` は消滅、wasm-bindgen / web-sys を保持。
- [x] 公開 .deb の `licenses/crates/**`（914 件）と同梱ドキュメントが開示
      ゲートの走査対象外。upload 直前に /tmp へ展開し全ファイルを走査する
      ゲートを追加（値 redact・パターン名のみ）。根拠 wa-vendored。
      （差分: ci.yml release-linux に `Gate - no development paths in DEB
      payload` を追加。展開物は `:?` ガード付き rm -rf で cleanup。）
- [x] web dist の開示ゲートが `*.wasm` / `*.js` のみを走査し、LICENSE /
      NOTICE / `licenses/crates/**` が対象外。`find web/dist -type f` の
      全ファイルへ変更し `strings -a -n 6` を適用。根拠 wa-vendored。
- [x] 64-hex トークン様パターンが全開示ゲートに無い。追加。根拠 wa-vendored。
      （素の `[0-9a-fA-F]{64}` は依存クレートの連結 hex 定数へ誤一致し
      現行ビルドで binary 7 件 / wasm 9 件の偽陽性となるため、
      `\b[0-9a-fA-F]{64}\b` でトークン境界を要求。）
- [x] check-public-docs.sh が Markdown のみ走査し、`LICENSE*` / `COPYING*` /
      `licenses/crates/**` をカバーしない。走査集合へ追加。根拠 wa-vendored。
      完了（R2 WC-DOCSCAN）: git pathspec へ `LICENSE*` / `COPYING*` / `NOTICE*` /
      `THIRD_PARTY_NOTICES*` / `licenses/crates/**` を追加し `sort -u` で重複排除。
      併せて 64-hex パターンを追加（公開済み SHA256/コンテナ ID 10 件は
      allowlist、新規トークンは検出）。bash -n / shellcheck / shfmt -d -i 2 /
      現状ゲート pass、/tmp 陽性対照で2パターン検出を確認。

### 優先度: 中 — SBOM スコープ明示 2026-11-20

- [x] SBOM はホストターゲット・デフォルト feature のスナップショットで
      deny.toml の全ターゲット範囲と非整合。NOTICE / README.deb.md に
      スコープを明記。SBOM 生成の --locked 代替と冪等ゲートを内容ハッシュ
      まで拡張。bom-ref の CI 絶対パスを正規化するか現状明記。
      upsteam author の noreply メールは正当メタデータとして保持方針を明記。
      根拠 wb-sbom。
      完了（R2 WA-SBOMTGT）: NOTICE.md / README.deb.md へターゲット別
      スナップショットである旨と全ターゲット非網羅を明記。ci.yml の
      sbom ジョブへ生成直後の `git diff --exit-code -- Cargo.lock
      web/Cargo.lock` とファイル名＋SHA256 の冪等ゲートを追加
      （SOURCE_DATE_EPOCH で内容も再現可能）。bom-ref の file:// 絶対
      パスは正規化せず現状許容、noreply メールは正当メタデータとして
      保持（開示ゲート対象外）する方針をコメントに明記。

### 優先度: 低 — 極端データ／ライフサイクル監査 2026-11-20

- [x] トークン再利用時も「自動生成しました」とログに出る（ローテーション
      誤認）。resolve_auto_token 前に有効ファイルを判定し、再利用時は
      「既存トークンを再利用しました」を出力。実動で初回=生成／2回目=再利用
      を確認。根拠 wb-lifecycle。

### メモ — 極端データ／ライフサイクル 2026-11-20

- 0 件スナップショット（CSV ヘッダのみ・選択解除）、10000 件（仮想化・
  56-79ms で export）、NaN/Infinity/1e9/u64::MAX（正規化・saturating）、
  1000 文字超/改行/制御文字（切り詰め・RFC4180 エスケープ）すべて破綻なし。
  Web 実ブラウザへの極端データ注入は未実施（静的確認）。根拠 wa-extreme。
- トークン 10 回再利用でハッシュ不変・権限 600/700・一時ファイル 0、
  corrupt 退避 20 回で最新 5 件・ディスク頭打ち、保存成功/失敗ともに
  一時ファイル 0。根拠 wb-lifecycle。

### メモ — シグナル／並行保存監査 2026-11-20

- SIGTERM で自作 sleep が終了、SIGSTOP→T／SIGCONT→R、stale start_time で
  拒否（対象生存）を 2 回連続で実証。UI 経路も同一 control() に到達。
  成功パス不具合 0 件（wa-signal）。
- 実プロセス 3 並行＋一時ハーネス 4 プロセス並行保存 70 ケースで、ファイル
  破損・0600 劣化・一時ファイル残存・panic すべて 0 件。逐次保存では異なる
  4 フィールドがすべて保持されるが、同時保存は 40 ケース中 23 で last-writer-
  wins により更新消失（設計判断どおり、flock 未提供）。app/src/settings.rs の
  モジュール doc に「逐次ケースに限る」旨を明記（wb-concurrent）。

### 優先度: 中 — ネットワーク障害／I/O失敗監査 2026-11-20

- [x] Content-Length 不一致の途中切断が FetchError::BodyRead に分類され、
      再試行案内（TRANSIENT_RETRY_GUIDANCE）が欠ける。5xx/timeout と案内を
      統一。根拠 wa-netfail。（WA-BODYREAD: `web/src/agent.rs` の BodyRead の
      Display へ TRANSIENT_RETRY_GUIDANCE を付与し、BODY_READ_ERROR の固定前缀
      と redact 契約を維持。完全一致アサートを prefix + 案内含有へ更新し、
      transient テストにも BodyRead を追加。）
- [x] SSE 端点だけ 503 のとき、poll は健全（tick 進行・stale=0）なのに
      live_error=1 が点滅する。SSE エラーを poll 由来の live_error と分離
      するか、直近 successful poll があれば反映しないゲート。根拠 wa-netfail。
      → `record_live_error` は poll 経路のみが呼び、SSE 経路は transient な
      fallback status のみ（`SseErrorEffect` は live_error / reconnect を持た
      ない）。persistent 通知ガードと sse=fallback / reconnect=0 を維持。
      根拠 WB-SSEERR2（R2 主実行）。
- [x] FETCH_TIMEOUT が wasm の実リクエストを abort できず回復がピア切断
      まで遅延（最大20s）→ 完了（WB-ABORT: 既存ソリューション調査の結果、
      担当範囲内では実装不可と判断し記録のみ／コード・Cargo.toml は未変更）。
      根拠 wa-netfail。調査結果:
      - web-sys 0.3.107 に `AbortController` / `AbortSignal` feature は存在
        （registry web-sys/Cargo.toml:42-43）。既存 web-sys への feature 追加
        自体は担当範囲内だが、下記の理由から単独では不十分。
      - gpui-pre-web 0.3.12 `FetchHttpClient::send`（http_client.rs:76-102）は
        実 fetch を `spawn_local` へ detach し oneshot で待つだけ。future を
        drop しても receiver が落ちるだけでブラウザの fetch は継続する
        （`with_deadline` agent.rs:616-626 は型付き Timeout を返すのみ）。
        `HttpClient` trait（gpui-pre-http-client:118-126）に signal 受け渡し口
        は無く、`RequestTimeout` extension（同:26,60-62）は定義のみでどの
        実装も参照しない no-op（grep 確認）。
      - wasm-bindgen-futures に abortable ヘルパは無い。`futures::future::Abortable`
        （既存依存）は Rust future を落とすだけなので実 fetch は止まらない。
        gloo-net の abortable fetch は新規クレートで禁止。
      - 自前 transport を作るには js-sys / wasm-bindgen-futures の直接依存追加
        （禁止）＋ gpui-web の fetch/response/stream 実装の複製（100行超・
        native テスト用 cfg 分岐も必須）が必要 → 不採用（車輪の再発明）。
      回避策: 古い fetch はピア側のアイドル切断（最大20s）で自然解放される。
      その間 agent の接続スロットを占有して 503 が増えるため、poll 側の
      クールダウン/バックオフで実害を緩和（WA-RETRY が担当）。redact / DOM
      proof 契約はコード未変更のため非回帰。
- [x] 連続 503/429 で Retry-After が無視され約1req/s で試行。poll 側にも
      クールダウン/バックオフを検討（暴走ではない）。根拠 wa-netfail。
      → 完了（WA-RETRY: R2 主実行）。既存調査で web/ には SSE_RECONNECT_DELAY=5s
      と sampler_wait の間隔クランプのみがあり Retry-After 解釈ヘルパは無かった。
      gpui::http_client が http::HeaderMap を再公開しているため web-sys 不要・
      新規依存なしで delta-seconds を解析可能と判断し、完全解釈を採用。
      agent.rs は 429/5xx の数値 Retry-After（正の整数のみ、30s でクランプ、
      HTTP-date 等は無視）を既存 Http(u16) を壊さない新 variant
      HttpRetryAfter{status,retry_after_secs} に保持（表示文言・reconnect・
      redact 契約は Http と同一）。shell.rs の poll は連続失敗で 1→2→4→8→16→30s
      の指数バックオフ、成功/config 変更で即リセット、Retry-After は 1 試行だけ
      下限を引き上げる。SSE の 5s クールダウンとは別状態（SSE エラーは poll
      間隔に影響せず、成功フレームは共通 ingest でリセット）。
      DOM proof キー・末尾 sse= 契約・persistent 通知ガードは無変更。
      検証: cargo test --manifest-path web/Cargo.toml --lib（190 passed）、
      web/ で cargo clippy --all-targets -- -D warnings、
      cargo clippy --target wasm32-unknown-unknown --lib -- -D warnings、
      cargo fmt --all -- --check（すべて成功）。
- [x] RLIMIT_FSIZE 超過（SIGXFSZ）で export 一時ファイルが残存。シグナル
      終了なのでクリーンアップ不可。→ 完了（WC-SIGXFSZ）: サポート対象外として
      `app/src/main.rs` の write_export doc コメントと
      `packaging/linux/README.deb.md` の「Known limitations」へ明記。signal
      handling は unsafe FFI / 新規依存を避ける方針（AGENTS §15）のため対象外と
      判断。通常の io::Error（権限拒否・ENOSPC 等）は cleanup する旨も併記。
      コード挙動変更なし。根拠 wb-exportio。

### メモ — I/O 失敗 2026-11-20

- /dev/full・権限拒否・上書き・シンボリックリンク・存在しない親ディレクトリ
  はすべて安全に処理（非通常ファイル拒否・0600維持・識別子非露出・panic無し）。
  設定側 persistent 通知と export 側 stderr+exit1 の差は headless モード差で
  不整合ではない。根拠 wb-exportio。

### 優先度: 高 — 実動回帰／最終観点 2026-11-20

- [x] Retry-After が Web ビルドで無視される。agent の CORS レスポンスに
      `Access-Control-Expose-Headers: Retry-After` が無く、ブラウザがヘッダを
      読めない（対照実験で追加すると 4s 間隔で効くことを実証）。add_cors_headers
      / preflight_response / 503 直接書き出し経路へ追加。根拠 wa-regress。
      → 完了（WA-CORS）: `agent/src/http.rs` の `add_cors_headers` /
      `expose_retry_after` を単一起源とし、`agent/src/main.rs` の 503 直接経路・
      SSE head・preflight 経路（preflight は `add_cors_if_absent` 経由）で同一
      ヘッダを付与。`cargo test -p tm-agent`（125 passed）/
      `cargo clippy -p tm-agent --all-targets -- -D warnings`（exit 0）成功。
- [x] 時刻逆行が持続すると is_late の間 last_network / clear_transient_status
      がスキップされ、network rate が累積＋transient error が消えない
      （4 サンプルで線形増加を実証）。基底を進めるか、late 中も通知クリア
      とレート基準更新を行う。根拠 wb-discovery。
      → 完了（WB-CLOCK）: `Dashboard::ingest` で「sampled_at の高水準マーク
      逆行」かつ「累積 network counter が後退していない」場合を clock step
      back と判定し、高水準マーク／last_network／transient クリアを再基底化。
      真に古いサンプル（counter も後退）は従来どおり late として表示のみ。
      新規テスト3件（固定・連続逆行の4サンプルでレート非累積、transient
      error の消去、再基底後の前方進行レート）を追加。
      `cargo test -p tm-viewmodel`（85 passed）/
      `cargo clippy -p tm-viewmodel --all-targets -- -D warnings`（exit 0）/
      `cargo fmt --all -- --check`（exit 0）成功。
- [x] IPv6 loopback `[::1]` が bind/クライアント許可は通るが CORS 許可
      リストで拒否され、ブラウザ接続だけ失敗。is_allowed_origin に `[::1]`
      を追加（curl 200 だが fetch が CORS ブロック）。根拠 wb-discovery。
      → 完了（WA-CORS）: `agent/src/http.rs` の `is_allowed_origin` が
      `http://[::1]` と `http://[::1]:<port>` を許可（port は原本どおり u16
      検証）。opaque host / 括弧なし IPv6 / 他 IPv6 リテラル / 他 origin は
      拒否のまま。socket テスト更新済み。`cargo test -p tm-agent`（125 passed）。
- [x] ネットワーク IF ホットプラグで合計カウンタ基準が古いまま新規 IF の
      累積値を差分とし、1 フレームだけ数十 GB/s に跳ね上がる。IF セット
      変化時に基準を再初期化。根拠 wb-discovery。WC-IFHOT 対応済み
      （`tm-viewmodel/src/lib.rs` で physical IF 名セットを追跡し変化時に
      レート 0 + `last_network` 再初期化、追加・削除の回帰テストを追加。
      cargo test -p tm-viewmodel / clippy / fmt --check すべて通過）。

### 優先度: 低 — 最終観点 2026-11-20

- [x] サスペンド/レジューム後の SSE watchdog が performance.now の前方
      ジャンプを誤検知しうる（speculative、ブラウザ依存）。根拠 wb-discovery。
      → 現状許容（WB-WD 静的レビュー）: ブラウザ依存で実機確認不可。現状の
      saturating_duration_since は前方ジャンプを誤検知しうるが、誤 fallback 後は
      5s クールダウンで自動回復するため実害は一時的。実機で再現した場合に対応する。
      根拠: MDN「Ticking during sleep」は仕様 Level 2 がスリープ中の tick を
      要求する一方、実際に tick するのは Windows ブラウザのみで
      Chrome/Firefox/Safari は非 Windows でバグにより止まる
      （`https://developer.mozilla.org/en-US/docs/Web/API/Performance/now` ）。
      web-time 1.1.0 のクレート文書も同趣旨（~/.cargo/registry/src/.../web-time-1.1.0/src/lib.rs:35-42）。
      対象環境（Kubuntu / Android）では前方ジャンプ自体が生じにくい。
      しきい値方式はしきい値未満のサスペンドを守れず、再基底方式は真の停止検出を
      1 グレース期間遅らせるため、未検証のまま採用せず。web/src/shell.rs は無変更。
- [x] ハードリンク非対応 FS（exFAT 等）で自動トークンの atomic publish が
      失敗し起動不能になりうる（likely）。→ 完了（WA-FSLINK・案内のみ）:
      rename フォールバックは既存トークンを無条件上書きし並行収束を壊すため
      不採用（std に no-replace rename は無く、RENAME_NOREPLACE/linkat は
      libc FFI・新規依存・unsafe が必要で禁止）。hard_link が
      Unsupported/PermissionDenied/InvalidInput/CrossesDevices で失敗した
      場合に --token-file と XDG_STATE_HOME を案内する固定エラーへ変換。
      AlreadyExists は従来どおり Conflict（再読込）で、0o077 チェック・
      TOCTOU 対策・再利用ロジックは無変更。純粋分類関数のテスト2件追加、
      トークン値・パス非露出・0600 契約維持。`cargo test -p tm-agent`
      （127 passed）/ `cargo clippy -p tm-agent --all-targets -- -D warnings`
      （exit 0）/ `cargo fmt --all -- --check`（exit 0）。根拠 wb-discovery。

### メモ — 実動回帰 2026-11-20

- SSE エラー分離・指数バックオフの 30s 上限／成功リセット・console error 0・
  トークン非露出は実ブラウザで確認（wa-regress）。

### メモ — 最終公開監査／CORS実動 2026-11-20

- 最終監査（wb-audit2）: テスト件数 workspace 543（agent 127 / app 111 /
  core 176 / platform 44 / viewmodel 85）/ web 190 が README/current-state/
  DESIGN/index と一致。バージョン 0.1.0 が Cargo.toml/CLI/DEB/README で一致。
  check.sh 17 ゲートが CI 10 ジョブへすべて対応。todo 未完了 0 件。発見なし。
- CORS 実動（wa-cors2）: フェイク 503 に Retry-After / Expose-Headers /
  Allow-Origin が揃う。現行 tm-agent のビジー 503 に Expose-Headers 付与。
  IPv6 [::1] リスナーで CORS 反射 200。不具合 0 件。
- wa-corslive は途中失敗したため wa-cors2 に範囲を絞って再委譲し完了
  （AGENTS.md §7 に従い失敗を無視せず再実行）。

### 優先度: 高 — swarm-limited R1 回帰 2026-11-20

- [x] SSE の 401 が reconnect=1 / トークン案内を即時設定しなくなった
      （2fea1fb の SSE エラー分離で回帰）。requires_reconnect が真のとき
      だけ例外経路を追加し、web/README の契約を復元。根拠 ra-diff。
      → 完了（SA-SSE401）: `SseErrorEffect` に `reconnect_required` を追加し
      `sse_error_effect(status, error)` が `requires_reconnect` で判定。
      `handle_sse_error` は 401 のときだけ sticky な `reconnect_required` と
      `#token=` 入りの案内ステータスを即時設定し、`live_error` は poll 専用
      のまま（DOM proof は `live_error=0` / `reconnect=1`）。
      `web/README.md` の contract を SSE 経由にも拡張して復元。
      `cargo test --manifest-path web/Cargo.toml --lib`（191 passed）/
      web clippy native + wasm / web fmt --check すべて exit 0。
- [x] カウンタ不変の古いサンプル（total_rx/tx が等値のアイドル時）が
      時刻逆行リベースとして誤受理され、transient 通知をearly消去＋
      baseline を上書き（6c035fc で回帰）。counters_advanced を厳密
      増加判定へ。根拠 ra-diff。
      → 完了（SB-CLOCKFIX）: `Dashboard::ingest` の `counters_advanced` を
      `>= && >=` から `> || >` の厳密増加判定へ変更。等値は late のまま
      保持し、transient 通知・`last_ingested_at`・`last_network`・
      `last_network_interfaces` を巻き戻さない。カウンタが明確に進んだ
      時刻逆行は従来どおり再基底（WB-CLOCK 契約維持）。新規テスト3件
      （アイドル等値＋古い sampled_at が late で各状態を保持／カウンタ
      進展逆行の再基底／アイドル連続後の前方進行レート）を追加し、既存
      `late_snapshot_*` および WC-IFHOT 契約テストの非回帰を確認。
      `cargo test -p tm-viewmodel`（88 passed）/
      `cargo clippy -p tm-viewmodel --all-targets -- -D warnings`（exit 0）/
      `cargo fmt --all -- --check`（exit 0）成功。
- [x] 401/403 と SSE エラーの契約テストが純粋関数／リテラル結合だけで
      実経路（on_sse_ended / sync_proof_dom）を通っていない（weak
      assertion）。回帰を検出できるテストへ置換。根拠 ra-diff。
      → 完了（SA-SSE401）: 2 件の weak assertion を実経路へ置換。
      (1) `sse_endpoint_error_keeps_the_401_403_reconnect_contract` は
      headless GPUI の実 `TaskManagerApp` で `on_sse_ended(Auth/Forbidden)`
      を呼び、`proof_string()`（= `sync_proof_dom` が DOM へ書くペイロード）
      から `reconnect=` / `live_error=` / `sse=` を検証。401 の sticky 性も
      実経路で確認。
      (2) `SseErrorEffect` の DOM proof テストもリテラル結合を廃し、503 の
      実経路状態（`live_error=0` / `reconnect=0` / fallback notice）を検証。
      あわせて persistent corrupt 通知ガードの実経路非回帰も追加。
- [x] エクスポート完了後に常設デモ案内が復帰せず「ステータス: 変更なし」
      になる。transient が persistent を置換しない保護 or 復元。根拠
      rc-dogfood。persistent slot を保持し transient 消去時に復元（SC-STATUSRESTORE）。

### 優先度: 中 — swarm-limited R1 発掘 2026-11-20

- [x] Windows で /dev/urandom 直読み＋XDG_STATE_HOME 前提のためトークン
      未指定の既定起動が不能（high、静的確認）。mining: Windows では
      APPDATA/USERPROFILE と OS RNG を使う分岐を検討。根拠 rb-discover。
      （SC-WINTOKEN: 明示エラー化を選択。generate_token に cfg 分岐を追加し、
      Unix は現状維持、Windows / その他は「--token 明示が必要」固定エラー。
      std に CSPRNG API が無く、getrandom は間接依存のみで新規依存追加は
      担当外、Win32 API は unsafe FFI で禁止、時刻/PID フォールバックも
      既に禁止されているため。auto_token_dir は Windows で APPDATA →
      USERPROFILE/AppData/Roaming を絶対パス・仮想 FS 共有検査で選択し、
      Linux 分岐は不変。既存トークンファイルがあれば Windows でも再利用可。）
- [x] stdout の非 EPIPE 書込み失敗（/dev/full / ENOSPC）が無言 exit 0
      （--once 出力喪失）。EPIPE と区別して警告 or 非0終了。根拠 rb-discover。
      → 完了（SD-STDOUT）: `write_stdout_ignoring_broken_pipe` が BrokenPipe
      のみ無視し他は Err。`--once` は exit 1 + 固定警告、`--version` / `--help`
      も非 0 終了。モック書込みテストと実プロセス `> /dev/full` で確認。
- [x] --once が --token-file / TM_AGENT_TOKEN の妥当性を検証せず黙って
      無視（once=0 / server=5）。根拠 rb-discover。
      → 完了（SE-ONCETOKEN）: 方針 (A) を採用。`run_with` の `--once` 早期
      リターン前に `resolve_configured_token` / `env_token` を集約し、サーバー
      モードと同一の検証・終了コード（不正な `--token-file` は両方 exit 5）。
      one-shot は認証リスナーがなく自動生成トークンの作成のみ明示的に
      非対応（副作用回避）。`args::USAGE` に両モード共通の検証と
      自動生成なしを明記。`run_with` へ argv を注入し両モードの終了コードを
      比較する unit テストを追加（トークン値は出力せず）。
      検証: `cargo test -p tm-agent`（131 passed）/
      `cargo clippy -p tm-agent --all-targets -- -D warnings`（exit 0）/
      `cargo fmt --all -- --check`（exit 0）。実プロセスで
      `--demo --once --token-file /nonexistent` と
      サーバーモードが共に exit 5、`--demo --once` は exit 0・1 行 JSON・
      先頭 30 件（デモ全件、上限 500 を下回る）を確認。
- [x] 追跡済み PoC ログ 2 件に内部 worktree パスが残り開示ゲートの
      走査対象外。根拠 rb-discover。
- [x] umask 0777 で XDG_STATE_HOME の中間ディレクトリが 0000 になり（共通 private_dir ヘルパで作成経路へ 0700 を適用。実動 exit 124＝起動継続・ディレクトリ 0700、unit test 2 件追加）
      起動失敗（low）。根拠 rb-discover。

### 優先度: 中 — swarm-limited R1 レビュー 2026-11-21

- [x] Windows では README / --help のトークン自動生成説明が不成立。
      auth.rs の cfg(windows) は固定エラーで生成しないため、
      README:200-201,243-244 と args.rs:64-68 に Windows 例外と
      --token-file / TM_AGENT_TOKEN 必須を明記。根拠 rb-winchk。
      → 完了（SA-WINDOC）: README のワンショット節・認証節と args.rs の USAGE
      認証節へ「Linux/Unix のみ自動生成、Windows は明示指定が必要」を明記。
      auth.rs:54-59 の固定エラー文言をそのまま案内へ一致させ、
      USAGE のテストへ Windows 例外の文言を pin した。
      検証: bash scripts/check-public-docs.sh / markdownlint README.md
      notes/cross-platform.md todo.md --disable MD060 MD055。
- [x] private_dir の既存ファイルへ 0700 を適用してしまう。Path::exists() は
      通常ファイルにも真を返すため、末尾 chmod が既存ファイルを変更。
      dir.is_dir() を確認してから chmod（ファイルは InvalidInput で拒否）。
      agent / app 双方の ensure_private_dir_chain へ chmod_private_dir を
      導入し、EEXIST コンポーネントも is_dir 確認後に chmod。
      根拠 ra-privdir。
- [x] private_dir のコメント「pre-existing directories untouched」が
      リーフ chmod / EEXIST コンポーネントと矛盾。実装に合わせて修正
      （既存祖先は不改変、リーフと EEXIST 成分は 0700 に正規化、
      ファイルは chmod しない）。根拠 ra-privdir。
- [x] テスト品質: umask 0777 回帰テストが実際に umask を変更していない、
      EEXIST 競合テストが 0700 収束を検証していない。テストバイナリを
      `sh -c 'umask 0777; exec "$0" ...'` で再起動して実 umask を適用
      （制御用 0000 ディレクトリで 0777 生效を証明）、競合テストへ
      dir と親チェーン各成分の 0700 アサートを追加。根拠 ra-privdir。

### メモ — クロスプラットフォーム 2026-11-21

- [x] Windows CI は tm-agent を型検査のみ（unit test は Linux gates が担当）で、
      README/notes に明記されていない（low）。根拠 rb-winchk。
      → 完了（SA-WINDOC）: notes/cross-platform.md の現行構成節（§0）へ、
      `native-windows` は workspace 全体を型検査し、tm-agent の unit test は
      Linux `gates` ジョブが担当する旨を明記。履歴スナップショットは不改変。
- [x] reconsider（設計判断・SA-WINDOC 記録）: 共通ワークフローへの委譲は（判断確定: 共通ワークフロー側の Windows サポート未確認かつ別リポジトリ変更は外部送信・公開操作を伴うため現状維持。tm-agent は workspace 型検査でカバー済みで機能欠落なし。要望があれば再開）
      AGENTS §5 の方針確認が必要（likely）なため、コード・CI 設定を変更せず
      記録のみとする。あわせて -D warnings の Windows/wasm 適用は
      tm-platform:86-88 の既存 unused imports で失敗（low）するため別途判断。
      根拠 rb-winchk。

### 優先度: 高 — swarm-project-review + todo-discovery 2026-11-21

- [x] Retry-After 実ヘッダが agent 側に存在せず、expose ヘッダのみ。（R6 coding で実装済み: 521e294）
      web の retry_after_secs/HttpRetryAfter/PollBackoff のヒント経路が
      本番で発火せず指数バックオフのみになる。503 に数値 Retry-After を
      設定し実ソケット経由のテストを追加。根拠 R2-TQ01 / R2-ER01。
- [x] cdp-verify が release-linux / web-dist / sbom の needs から漏れており、（R6 coding で実装済み: 521e294）
      CDP 検証スクリプトの回帰があっても成果物が公開されうる。needs へ
      追加。根拠 R3-DP01 / R4-CI01。
- [x] web クレートの MSRV 宣言 (rust-version 1.95) が CI で検証されていない。（R6 coding で実装済み: 521e294）
      msrv-1-95 ジョブは workspace のみ。web の wasm check を 1.95 で追加
      するか、宣言から検証済みの主張を削除。根拠 R1-CF01。
- [x] ワイヤ型/エラー契約 (SnapshotEnvelope, ErrorCode, 固定ガイダンス) が（tm-core::wire / tm-viewmodel::settings_policy へ集約）
      agent と web で手作業複製され共有クレートが無い。tm-core に wire
      モジュールを設け単一定義化。根拠 R1-AR01。
- [x] 設定永続化ポリシー (version互換/rawマージ/破損隔離/4MiB/BOM/保持5)（tm-core::wire / tm-viewmodel::settings_policy へ集約）
      が app と web で再実装され契約が分離しうる。共有 policy へ抽出。
      根拠 R1-AR01。

### 優先度: 中 — swarm-project-review + todo-discovery 2026-11-21

- [x] エクスポート後の private_dir / Retry-After / CORS の実経路テストが
      弱い（EEXIST 分岐・既存 leaf/ancestor 権限・stream 503 CORS・
      連続 hint・5xx 境界）。根拠 R2-TQ01 / R5-TS01。
- [x] late サンプルで network_rates/history が (0,0) に上書きされる。
      is_late ならレート更新と history.push をスキップ。根拠 R2-BV01。
- [x] 自動トークン read→remove→publish の TOCTOU で並行起動が別トークン
      を持ち得る。adopt 直前の再読込または直列化。根拠 R2-BV01。
- [x] SSE 接続確定前のポールと SSE が同一 probe をサンプリングし、
      sampled_elapsed 短縮でレート過大。接続元分離またはポールにも
      世代ガード。根拠 R2-BV01。
- [x] SSE error イベントが Web 側で Parse エラー化してストリーム終了。
      1 回の回復可能失敗で fallback＋5s クールダウンが発生。Frame::Error
      等でスキップ継続。根拠 R2-ER01。
- [x] トークン公開の部分失敗で旧ファイルも新ファイルも失われる。
      退避/復元または chmod 先試行。根拠 R2-ER01。
- [x] --log-level が RequestTrace を抑制できず、--log-level error でも
      http.request.start/end が出力される。root cause: telemetry::emit が
      直接呼ばれる。根拠 R4-OB01。
- [x] リクエストスコープのエラーイベント (probe.mutex_poisoned 等) に
      request_id が付かず相関できない。根拠 R4-OB01。
- [x] 列ヘッダのソートがマウス専用（WCAG 2.1.1）。フォーカス＋
      Enter/Space＋aria-sort。ネイティブ確認ダイアログの Dialog role 欠落。
      根拠 R5-UX01。
- [x] ライトテーマの muted/dim/塗り背景が WCAG AA 4.5:1 未満。
      コントラスト比の自動検証テスト追加。根拠 R5-UX01。
- [x] F5「今すぐ更新」が Web で実更新せず通知のみ。実 fetch または
      文言を「要求のみ」に変更。根拠 R5-UX01。
- [x] web 行メニューが右クリック専用でキーボードから開けない（WCAG）。
      Shift+F10/ContextMenu 対応。根拠 R5-UX01。
- [x] web-dist の licenses/crates/** が CI で常にスキップされ、DEB と
      ライセンス同梱が非対称。共有スクリプト化または必須アサーション。
      根拠 R4-CI01。
- [x] deny ジョブが -A advisory-not-detected で check.sh とフラグ不一致。
      advisory DB 読み込みをアサート。根拠 R4-CI01。
- [x] gitleaks チェックサムがバイナリと同一オリジンで真正性検証に
      なっていない。repo 内ピン留めまたは署名 attestation。根拠 R4-CI01。
- [x] XDG_CONFIG_HOME パステストが環境変数を設定せず名前だけ検証。
      純粋関数へ切り出し各分岐を明示テスト。根拠 R5-TS01。
- [x] テスト件数 556/191 に対し README 内訳 176+91+44+132+111=554。
      内訳を再計測して合計と揃え DESIGN マトリクスも更新。
      根拠 R1-FG01 / R5-DC01。
- [x] loopback 許可範囲の記述が SECURITY/DESIGN で広い（127.0.0.1/[::1]
      のみが正）。PID0 拒否の記述不整合。根拠 R1-FG01。
- [x] web/README のトークン保存場所案内が「stderr で案内」と矛盾。
      XDG state パスを参照する表現へ。根拠 R4-VU01。
- [x] Windows トークン自動生成の例外が DESIGN/SECURITY/web/README に（c2-docs4 で実装済み: 1a83fa4）
      欠落（README のみ記載）。root cause auth.rs:51。根拠 R5-DC01。
- [x] DEB README の AC サンプリング下限が 1s と記載（実装は 0.5s）。
      README.deb.md:18 を修正。根拠 R5-DC01。

### 優先度: 低 — swarm-project-review + todo-discovery 2026-11-21

- [x] tm-platform が workspace.dependencies に無く path/version 重複。
      deny 重複 warn と CI コメント矛盾。all-features 監査が実ビルドと
      異なる feature 集合。根拠 R1-CF01。
- [x] markdownlint 除外スコープが CI と check.sh で手動同期。
      gitleaks --unshallow 失敗無視。npm markdownlint 推移依存固定なし。
      dependabot gpui-stack グループがデッド設定。根拠 R1-CF01 / R3-DP01。
- [x] admit 失敗の固定 50ms リトライ、stderr BrokenPipe パニック、
      Prometheus カウンタの同時スクレイプ不整合、容量拒否 503 の
      メトリクス区別不可。根拠 R2-ER01 / R4-OB01。
- [x] DOM proof の短いトークン平文露出、XDG パスの '..' 迂回、
      明示トークンの強度要件/レート制限なし、通常 snapshot の Host
      欠落許容。根拠 R3-AU01。
- [x] sysinfo 0.31.4/0.40.3 重複、advisory ignore 根拠が lock 実態と
      矛盾（pastey/web-time 代替あり）。根拠 R3-DP01。
- [x] テーマ切替ボタンの accessible name が状態を示さない、UI 文言の
      i18n 層欠落、温度しきい値用語 (max/crit vs 上限/危険) 混在、
      空状態案内がマウス前提。根拠 R5-UX01。
- [x] 実測ホスト CPU 上限アサートと 800ms health 上限がフレークしうる。
      private_dir 非 Unix 分岐・native ui.rs テスト欠落。根拠 R5-TS01。
- [x] notes/index 未掲載ノート多数、physical-verification の private 記述、
      Chrome 157/158 混在、ADR wasm サイズ要再測定、DESIGN 行番号 stale、
      DESIGN にレビュー作業メモ残存、advisory ignore 8/9 分岐、
      設定永続化リストの theme 欠落、HiDPI ノートの古い不具合記録。
      根拠 R4-VU01 / R5-DC01。
- [x] workflow_dispatch が任意ブランチから成果物生成、外部ツール導入が
     毎回ネットワーク依存でキャッシュなし。根拠 R4-CI01。
- [x] AX ミラー告知に対して CDP の AX 検証が warning 止まり
      (PROMOTE_AX_TO_FAILURE=False)。昇格または注記。根拠 R4-VU01。

### メモ — 却下 / speculative 2026-11-21

- R3-IV01 は新規発見なし。Host 欠落の不整合は r3au01/r1fg01 の
      confirmed 指摘として統合し、iv01 の「意図的」判定は採用しない。
- シグナル可否を OS 表示文字列の部分一致から導出する設計（R1-AR01）
      は実害が未知の speculative として記録。
- 性能・スケール指摘（wasm 初回ロード計測未実施等）は測定根拠が無いため
      medium 以下に留めた。

### 優先度: 中 — バックログ補充コンボ R6 2026-11-21

- [x] ワイヤ共有から漏れたプロトコル定数（SNAPSHOT_PATH / STREAM_PATH /
      DEFAULT_SNAPSHOT_LIMIT=500 / MAX_SNAPSHOT_LIMIT / DEFAULT_PORT=8787）が
      agent・USAGE・web・テストで手書き複製され、片側変更が他側で 404／
      案内乖離を招く。tm_core::wire へ追加し re-export。根拠 R1-RESIDUAL。
- [x] ErrorCode::as_str と全 variant 一覧が agent/src/main.rs と（a9b4ea4 で実装済み）
      args::USAGE とそのテストの三方で手動同期。variant 追加時に片方を
      忘れても全テストが通る。wire へ移し USAGE を同じ配列から生成、
      または交叉検証テスト。根拠 R1-RESIDUAL。
- [x] Web の RECONNECT_GUIDANCE / ONBOARDING_STEPS が「既定は XDG state（a9b4ea4 で実装済み）
      配下」と無条件に案内し、Windows の自動生成例外（auth.rs:51）と
      矛盾。Windows 利用者が存在しない場所を探す。「--token-file で指定
      した場所（Unix 自動生成時は XDG state）」等へ。根拠 R1-RESIDUAL。

### 優先度: 低 — バックログ補充コンボ R6 2026-11-21

- [x] private_dir が agent/app で逐語複製（意図済みだがドリフト検出が（a9b4ea4 で実装済み）
      コピー内部に閉じている）。tm-platform へ一本化するか、両ファイルを
      比較する CI ガードを追加し意図を明文化。根拠 R1-RESIDUAL。
- [x] tm_core::wire / tm_viewmodel::settings_policy が DESIGN §2.2 の（a9b4ea4 で実装済み）
      crate 役割表と §12.1 の AC-12 マトリクスに未反映。単一起源を
      ドキュメント化し、将来の再複製を防止。根拠 R1-RESIDUAL。
- [x] notes/current-state.md:135 が strings 開発パス検査を「未導入」と（a9b4ea4 で実装済み）
      記載するが、ci.yml に 3 箇所実装済み。「実装済み」へ更新し
      SHA256SUMS 未同梱のみ未着手として残す。根拠 R1-RESIDUAL。
- [x] 503 ビジー文言が agent 2 箇所（通常/SSE）と web 固定案内で（SERVICE_UNAVAILABLE_REASON を wire 化し交叉検証）
      手動同期。SERVICE_UNAVAILABLE_GUIDANCE を wire へ移すか、
      共通定数から導出＋cross-crate pin テスト。根拠 R1-RESIDUAL。

### メモ — 却下 2026-11-21

- R1-POSTREF は SnapshotEnvelope generics＋newtype adapter、
  merged_map_with_baseline の欠落復元、SettingsNotice の &'static str
  制約、Retry-After/403 バイト不変の 4 観点を実コードで確認し、
  新規問題なし（findings: []）。共有化リファクタは採用を確定。

### 優先度: 中 — バックログ補充コンボ R6 2026-11-21（第2次）

- [x] SSE error フレーム連続でポーリングが無期限停止。Connecting 中も
      watchdog でガードを解放（sse_connect_started_at と純粋ヘルパ＋
      回帰テスト）。根拠 R1-FRESH。
- [x] 保存失敗通知が再保存成功後も残存。save() 成功時に PENDING_NOTICE
      と DOM ノードをクリア。根拠 R1-FRESH。
- [x] 破損設定の「最新の退避物」判定がコード（ファイル名 hex 順）と
      手順（mtime 順）で不一致。README/SECURITY をファイル名順へ修正
      し、退避物は破損値そのものなので型修正してマージする旨を追記。
      根拠 R1-OPSGAP。
- [x] README クイックスタートに wasm target と trunk 0.22.1 の導入が
      欠落、apt 依存が CI と不一致。両方を追記／整合。根拠 R1-OPSGAP。

### 優先度: 低 — バックログ補充コンボ R6 2026-11-21（第2次）

- [x] /metrics のメトリクス意味・エラー率算出式・SSE 枠アラート閾値が（notes/operations.md を作成）
      未文書化。loopback 限定のため外部監視基盤から直接取得できない
      制約と relay 手順も Runbook に記載。根拠 R1-OPSGAP。
- [x] リリース時のバージョン更新チェックリスト／Runbook が無い（notes/operations.md を作成）
      （Cargo.toml×2 / README:17 / DEB 検証 / 差し戻し）。根拠 R1-OPSGAP。

### 優先度: 中 — バックログ補充コンボ R6 2026-11-21（第3次）

- [x] --token 起動のトークン値が export CSV/JSON に残る。tm-core::redact
      へ scrub_token_from_snapshot / scrub_token_flags_from_snapshot を
      追加し native/web export へ接続。実動 Runtime Proof で平文 0 を確認。
      根拠 R1-PRIVACY。
- [x] #token= がブラウザ session history に残る。history.replaceState で
      URL から除去し、OnceLock メモリへ移して再接続を維持（endpoint 束縛、
      新トークン上書き、空値クリア）。根拠 R1-PRIVACY。

### 優先度: 低 — バックログ補充コンボ R6 2026-11-21（第3次）

- [x] WIRE_SCHEMA_VERSION 引き上げが非対称（旧 web は新 agent の全フレーム（更新順を README、観測リスクを operations.md、桁あふれを sort_key で対応）
      を拒否）。更新順を「web 先→agent 後」と明記するか、拒否時に版番号を
      UI へ。将来の schema_version 型変更が Parse エラーに化ける可能性も
      併せて対応。根拠 R1-FWDCOMPAT。
- [x] 検疫キーの hex が 16 桁固定前提で epoch nanos 桁あふれ時に保持兜が（更新順を README、観測リスクを operations.md、桁あふれを sort_key で対応）
      効かない（speculative、2554 年）。wire の usize が wasm32 で 32bit
      のため将来 u64 超え値が Parse エラーになる可能性も併記。
      根拠 R1-FWDCOMPAT。
- [x] /metrics が無認証で uptime・カウンタ・active/stream ゲージを公開。（更新順を README、観測リスクを operations.md、桁あふれを sort_key で対応）
      同一ホスト他プロセスが利用リズムを推測しうる（low/likely）。
      破損設定の隔離バックアップが生 JSON を永続化する点も併記。
      根拠 R1-PRIVACY。
- [x] 将来スキーマ version 9 で interval_ms 範囲が変わった場合、現在の（future version では clamp をスキップし raw 値を保持）
      クランプ値が version:9 のまま保存され将来ビルドの意味が壊れる
      （speculative）。根拠 R1-FWDCOMPAT。

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
- WB-CONCURRENT 監査メモ（2026-11-20）: 実
  `gpui-task-manager --demo --redact` 3 プロセスを共有 XDG_CONFIG_HOME で
  4 秒並行起動 → 全プロセス生存・stderr 0・settings.json 不変。一時 Rust
  ハーネス（`app/src/settings.rs` を `#[path]` でそのまま使用）で 4 プロセス
  並行保存を 70 ケース実施（barrier 40 / stagger 10 / same-field 20）。
  全 70 ケースで serde parse 成功・権限 0600・一時ファイル 0・
  corrupt 退避 0・保存失敗 0・panic/stderr 0。逐次（120ms ずらし）は
  10/10 で 4 フィールドすべて残存（three-way merge 有効）。同時バリアでは
  17/40 のみ 4 フィールド残存、23/40 で 1 つ以上の変更が last-writer-wins
  で消失（例 barrier-2 で `tree_view` が false に戻る）。同一フィールド競合は
  20/20 で単一値のみ残存（baseline と同値の 1000 は書き込まれない）。
  `SAVE_LOCK` はプロセス内のみ・flock 無しという設計判断どおり破損は無く、
  「異なるフィールドの並行編集も保持」は逐次ケースに限られる。/tmp cleanup 済み。

---
type: Research
title: 未知の残タスク・機能追加案の調査
description: エージェントチームによる多角的調査（ラウンド 1〜5）と最終集約（ラウンド 6）の作業記録
tags: [research, backlog, agent-team, gpui, task-manager]
resource: work/gpui-task-manager
generated: 2026-11-17T19:15:32+09:00
verified: 2026-11-17T20:39:14+09:00
status: completed
---

## 目的

既存の `notes/current-state.md` 「未解決・今後の候補」や `notes/verification.md` に
列挙済みの項目に加え、**まだ文書化されていない残タスク・機能追加案**を、
エージェントチームによる反復調査で洗い出す。ラウンド 1〜5 で観点を変えて探索し、
ラウンド 6 で重複排除・妥当性検証を行って統合する。

## 方法

- 各ラウンドで複数の調査エージェントを並列起動し、観点を変えて探索する。
- 各所見は `FILE:LINE` 等の根拠を添え、既知（KNOWN）/ 新規（NEW）を区別する。
- 妥当性は High / Medium / Low で評価し、推測は明示する。
- 親エージェントは所見を無条件に採用せず、現行コード・文書で事実確認する。

## 既知項目の基準線

`notes/current-state.md` 「未解決・今後の候補」:

- `prefers-color-scheme` 連動
- 同形異義文字のサニタイズ
- ネイティブ版の実 IME 手動確認
- 431 / 400 / 503 の CORS 付与
- Windows / macOS の物理実機検証
- 優先度変更、GPU 詳細監視、DEB 署名・公開

## ラウンド 1 — 全表面走査（完了）

調査観点: コード上の未完実装 / 文書と実装の乖離 / GitHub・git 履歴 /
テスト・CI 検証ギャップ / 機能追加アイデア。5 エージェントを並列起動。

### 1-A. 文書と実装の乖離（rx-docgap / 16 件）

| ID | 所見 | 根拠 | 判定 | 妥当性 |
| --- | --- | --- | --- | --- |
| R1-D01 | DESIGN §1.2 の「非目標」に設定の永続化が残ったまま（実装済みと矛盾） | `DESIGN.md:26-33` vs `:604-620`, `settings.rs:28-44` | NEW | High |
| R1-D02 | RESEARCH §8「未確認事項」が IME・エージェント並列化の解消を反映していない | `RESEARCH.md:271-280` vs `README.md:44-48`, `agent/src/main.rs:215-283` | NEW | High |
| R1-D03 | テスト件数の記載が SECURITY / README / DESIGN で古く相互矛盾 | `SECURITY.md:137,152`, `README.md:171`, `DESIGN.md:94-98` vs `current-state.md:31-33`（201/web 15） | NEW | High |
| R1-D04 | `tm-viewmodel::Settings` の doc comment が保存項目を 3 つのみと記載（実 6 項目） | `settings.rs:1-5` vs `:31-44` | NEW | Medium |
| R1-D05 | `hidepid` 環境の UI 注記がスパイク限定で実アプリ未反映・未検証 | `RESEARCH.md:122-123,277`, `crates-survey.md:156` vs 製品コードになし | NEW | Medium |
| R1-D06 | AC-3 の検証手段「KWin D-Bus」が一度も成功せず設計書未改訂 | `DESIGN.md:406,500` vs `verification.md:51-54` | NEW | Medium |
| R1-D07 | `offset` / 差分更新の「将来対応」が未着手 | `RESEARCH.md:210-211,280` vs `agent/src/main.rs:579-607` | NEW | Medium |
| R1-D08 | Glances / node_exporter アダプタが再評価されず放置 | `RESEARCH.md:247-252,266,279`, `DESIGN.md:443,559` | KNOWN | Medium |
| R1-D09 | 記録データ再生（record/replay）の採用理由が残ったまま再訪されず | `RESEARCH.md:20,182`, `DESIGN.md:252,583` | NEW | Medium |
| R1-D10 | ADR-007 の start_time 非提供表現が `PlatformInfo` に未実装 | `DESIGN.md:576-577`, `port.rs:14-25`, `tm-platform/lib.rs:427-429` | NEW | Medium |
| R1-D11 | 確認ダイアログのキーボード完結（フォーカス移動）が将来項目のまま | `DESIGN.md:240` vs `app/src/app.rs:2157-2161` | NEW | Low |
| R1-D12 | 起動時間目標（数秒以内）に計測根拠なし | `DESIGN.md:420` vs `verification.md` に記録なし | NEW | Low |
| R1-D13 | SECURITY.md が「公開リポジトリ化の際に追記」プレースホルダのまま | `SECURITY.md:1-14` vs `README.md:19`（public 化済み） | NEW | Medium |
| R1-D14 | 折りたたみ状態／cgroup 表示が既知ギャップ一覧から欠落 | `DESIGN.md:706` vs `current-state.md:60-69` | KNOWN | Low |
| R1-D15 | 仮想 IF 除外が Linux 名前提で Windows/macOS で未対応（既知一覧に未反映） | `cross-platform.md:246-247` vs `model.rs:276-292` | NEW | Low |
| R1-D16 | AC-12（設計・調査・README と実装の一致）に検証マトリクス不在 | `DESIGN.md:509`, `review-2026-11-14.md:80-84` | NEW | Medium |

### 1-B. GitHub / git 履歴のシグナル（rx-github / 16 件）

リモート: `github.com/Seduce0068/gpui-task-manager`。issue 0 件、PR #1/#2 は merged 済み、
タグ・リリースなし。コード内 TODO マーカーも 0 件（a6de40f）。

| ID | 所見 | 根拠 | 判定 | 妥当性 |
| --- | --- | --- | --- | --- |
| R1-G01 | モーダル overlay の実機クリック遮断が未検証 | commit `2301668` 本文「実機クリック注入は未検証」（lead 確認済み） | NEW | High |
| R1-G02 | macOS CI のキュー待ち／cancelled が慢性的に再発 | commit `8ee22fe`, `log.md:499-504,565-570,708-712,732` | NEW | High |
| R1-G03 | PR #2 が「CI 緑になってからマージ」を要求したままマージ済み | PR #2 本文, `log.md:345-349,437-446` | NEW | Medium |
| R1-G04 | プロセスツリー cycle 処理が連続修正の脆弱領域（プロパティテスト不足） | `117a736`→`a65fcf2`→`9e5f4d0`→`2d1112d` | NEW | Medium |
| R1-G05 | エージェント並列接続まわりも連続修正の集中領域（ソーク不足） | `2a3f0fe`→`f430a7f`→`cedc118`→`5fec22b`→`dab7520` | NEW | Medium |
| R1-G06 | 数値エッジケース防御が後追いaccumulation（境界値テスト体系化） | `3749354`,`4553dbb`,`e88f344`,`b271d83`,`c848597` | NEW | Medium |
| R1-G07 | PID 再利用の残リスク（pidfd）が将来課題のまま | `verification.md:786`, `git log --grep=pidfd` 0 件 | NEW | Medium |
| R1-G08 | Dependabot が GPUI を pin し続け手動升级が残る／unmaintained 8 件 | `.github/dependabot.yml:34-41,63-69`, PR #1 | NEW | Medium |
| R1-G09 | リモート公開用 TLS・認証・レート制限が未実装 | `web-shell.md:299` | NEW | Medium |
| R1-G10 | wasm サイズと実回線の初回ロード時間が未計測 | `web-shell.md:294-296` | NEW | Low-Med |
| R1-G11 | StartupWMClass の実 Window プロパティが未検証 | commit `90d98b7` | NEW | Low-Med |
| R1-G12 | スクリーンショットと verification.md 数値の乖離（M5）が先送り | `verification.md:797-798`, `review-2026-11-14.md:192` | NEW | Low |
| R1-G13 | 性能記述（sysinfo full tick 31.4ms / cold start 85ms）が未再測定 | `review-2026-11-14.md:180`（o7） | NEW | Low |
| R1-G14 | 放置されたローカルブランチ master / archive/master と merged 済みリモートブランチ | `git branch -a`（`bc30c0b`, `dab7520`） | NEW | Low |
| R1-G15 | issue/tag/release が皆無で残作業がローカル文書にのみ存在 | `list_issues` 0, `git tag -l` 空 | 一部 KNOWN | Low-Med |
| R1-G16 | コード内未解決マーカー 0 件（否定証拠） | commit `a6de40f` | NEW | Low |

### 1-C. テスト / CI / 検証ギャップ（rx-verify / 15 件）

| ID | 所見 | 根拠 | 判定 | 妥当性 |
| --- | --- | --- | --- | --- |
| R1-V01 | CDP ブラウザ実動検証が CI に未接続（UI の実行時挙動が無ゲート） | `ci.yml:56-67,140-157`, `grep cdp_verify .github scripts` 0 件（lead 確認済み） | NEW | High |
| R1-V02 | Windows CI が `tm-platform` テストをスキップ（macOS は実行） | `ci.yml:82` vs `:93`（lead 確認済み） | NEW | High |
| R1-V03 | 実 IME（IBus/fcitx5）手動確認が未実施 | `current-state.md:66`, `ime.md:30,277-279` | KNOWN | High |
| R1-V04 | 物理 Windows/macOS の GUI・IME・描画が未検証 | `current-state.md:68`, `physical-verification.md:146` | KNOWN | High |
| R1-V05 | 実質カバレッジが Chrome のみで Edge/Brave が未分類 | `ime.md:306-307`, `verification.md:117,469` | NEW | High-Med |
| R1-V06 | WebGPU 実 GPU 描画が未検証（ヘッドレスは WebGL2/SwiftShader） | `verification.md:117,469`, `wasm-spike.md:229-236` | NEW | Medium |
| R1-V07 | X11 セッションでの表示が未検証 | `verification.md:114`, `DESIGN.md:705` | NEW | Medium |
| R1-V08 | Host ヘッダ欠落時にループバック検証をバイパス（Negative テスト欠落） | `agent/src/main.rs:323` `if let Some(host)`（lead 確認済み） | NEW | Medium |
| R1-V09 | 431/400/503 の CORS 付与なし（未テスト・未対応） | `verification.md:980`, `current-state.md:67` | KNOWN | Medium |
| R1-V10 | 同形異義文字のサニタイズ未実装・テスト皆無 | `current-state.md:65`, `sanitize.rs:134-297` | KNOWN | Medium |
| R1-V11 | ブラウザシェル本体（`shell.rs` 1,759 行）にユニットテストゼロ | `grep -c cfg(test)` 0 | NEW | Medium |
| R1-V12 | モバイル（タッチ・ソフトキーボード）未検証 | `web-shell.md:300` | NEW | Low-Med |
| R1-V13 | `write_response` の部分書き込み／分割書き込みが未テスト | `agent/src/http.rs:601-610` | NEW | Low |
| R1-V14 | `/dev/urandom` 失敗時フォールバック乱数のテスト欠落 | `auth.rs:50-71,106-180`, `review-2026-11-14.md:104` | NEW | Low |
| R1-V15 | wasm check のローカル/CI ゲート不一致（poc-wasm） | `current-state.md:35` vs `ci.yml:66-67`, `check.sh:19-20` | NEW | Low |

### 1-D. 機能追加アイデア（rx-ideas / 20 件）

| ID | アイデア | 工数 | 判定 | 補足 |
| --- | --- | --- | --- | --- |
| R1-I01 | プロセス単位 I/O レート（`/proc/<pid>/io`） | M | NEW | 競争ギャップ最大、wire v1 更新要 |
| R1-I02 | スナップショット CSV/JSON エクスポート（`--redact` 再利用） | S | NEW | 低工数・高価値、`--redact` 流用可（lead 確認: 実装なし） |
| R1-I03 | 閾値アラート／デスクトップ通知（`critical_c` 未活用） | M | NEW | 能動検知化、I14 と相乗 |
| R1-I04 | 複数ホスト監視ダッシュボード | L | NEW | TLS/認証が前提（G09 と統合） |
| R1-I05 | スナップショット記録／再生と時系列 diff | M | NEW | D09 と同一系統 |
| R1-I06 | 名前付きフィルタビュー／プリセット | S | NEW | ADR-012 と緊張 |
| R1-I07 | プロセスツリー一括操作（サブツリー kill／複数選択） | M | NEW | Web は読み取り専用維持 |
| R1-I08 | 選択プロセスの時系列スパークライン | M | NEW | 全体履歴のみ既存 |
| R1-I09 | per-process ネットワーク使用量（nethogs 的） | L | NEW | 権限・inode 脆さ |
| R1-I10 | バッテリー／電源状態の監視 | S-M | NEW | 11 モニタ比較で差別化 |
| R1-I11 | ウィンドウ非表示時の適応的サンプリング | S | NEW | バッテリー・CPU 削減 |
| R1-I12 | cgroup／コンテナ単位グルーピング（読み取り専用） | M-L | NEW | D14 と関連 |
| R1-I13 | 確認ダイアログのフォーカス移動／キーボード完結 | S | NEW | D11 と同一 |
| R1-I14 | エージェントのストリーミング配信（SSE/WS） | M | NEW | G09 と関連、8 接続上限と共存設計要 |
| R1-I15 | 読み取り専用 CLI / one-shot モード | S | NEW | `handle_cli()` 拡張のみ |
| R1-I16 | ブラウザ版の初回オンボーディング／接続ウィザード | S | NEW | 導入障壁の解消 |
| R1-I17 | ユーザー設定ファイルの拡張（キー／列／閾値） | M | NEW | スキーマ v3、秘密情報規律要 |
| R1-I18 | 外部データソースアダプタ（Glances / node_exporter） | L | NEW | D08 と同一系統 |
| R1-I19 | 行コンテキストメニュー／クリップボード連携 | S-M | NEW | clipboard 実装なし（lead 確認） |
| R1-I20 | Flatpak / AppImage 配布 | L | KNOWN  adjacent | DEB 署名・公開（既知）とは別 |

### 1-E. コード未完実装（rx-code / 5 件）

全 `.rs` を grep 走査し疑わしい生産コードは周辺を通読（全文未読のファイルあり、
その旨は rx-code が明記）。

| ID | 所見 | 根拠 | 判定 | 妥当性 |
| --- | --- | --- | --- | --- |
| R1-C01 | Web 版に更新間隔の変更 UI が無く、サンプラー間隔が起動時固定 | `web/src/shell.rs:123,139,164`（`interval_ms` を 1 回キャプチャ）／toolbar `:806-865` に間隔ボタン無し。対比 native `app/src/app.rs:1061-1083`（lead 確認済み） | NEW | Medium |
| R1-C02 | tm-agent が非 UTF-8 argv で panic（`args_os()` 修正の取りこぼし） | `agent/src/main.rs:89`（`std::env::args()`）vs `app/src/main.rs:23-25`（lead 確認済み） | NEW | Medium-Low |
| R1-C03 | エージェントの module doc が旧方式「header or query」のまま（query は廃止済み） | `agent/src/main.rs:9` vs `agent/src/auth.rs:90-104`（lead 確認済み） | NEW | Low |
| R1-C04 | `#![allow(dead_code, unused_imports)]` による crate 全体の lint 抑制 | `web/src/lib.rs:5`（理由コメント `:1-4` あり、隠蔽の実証なし） | NEW | Low |
| R1-C05 | PoC の固定ダミー行／スワローしたエラー | `poc-wasm/src/lib.rs:614-625`, `poc-crates/sysinfo-probe/src/main.rs:302,329`（スパイク資産） | KNOWN | Low |

### 1-F. 補完レビュー（r1-* チーム / 32 件）

19:26 に別チーム（r1-*）を並列起動し、セキュリティ / アーキテクチャ /
運用 / データ安全性を深掘りした。ID の衝突を避け `R1S-*` を用いる。

#### エージェント起動結果（失敗も記録）

| エージェント | 結果 |
| --- | --- |
| `r1-security` | 失敗（最終応答なし） |
| `r1-sec2`（縮小版で再実行） | 3 件返却 |
| `r1-arch` / `r1-arch2` | 2 回とも失敗（最終応答なし） |
| `r1-arch3`（縮小版で再実行） | 5 件返却 |
| `r1-ops` | 12 件返却 |
| `r1-sanitize` | 11 件返却 |
| `r1-tests` | 失敗（ファイル走査中に中断） |

失敗 5 件は無視せず、縮小版プロンプト再実行と親による直接確認で補完した。
テスト観点は 1-C が既にカバーしているため再起動していない。

#### 親による事実確認（Runtime Proof）（1-F）

- `deny.toml:65` の `multiple-versions = "warn"`（R1S-OPS-03）。
- `scripts/generate-sbom.sh:38,46` の相対 `out_dir` 除外（R1S-OPS-04）。
- `format.rs:13` の UNITS が `PiB` 終了（R1S-DATA-08）。
- `demo.rs:418` が非 saturating な `+ tick`（R1S-DATA-10）。
- `query.rs:228` の `total_cmp`（R1S-DATA-07）。
- `sanitize.rs:92` が `U+FE00..=U+FE0F` のみ（R1S-DATA-02）。
- `port.rs:118-120` の `is_destructive = matches!(Kill)`（R1S-ARCH-04）。
- `agent/src/main.rs:323` の `if let Some(host)`（R1S-SEC-01）。
- `agent/src/main.rs:9` と `auth.rs:92-101` の矛盾（R1S-DOC-01）。

#### 追加所見

セキュリティ（r1-sec2）

- **R1S-SEC-01**（Medium）`agent/src/main.rs:323` — Host 欠落時に
  DNS rebinding 防御を迂回。↔ R1-V08 と同一（重複）。
- **R1S-SEC-02**（Medium）`agent/src/auth.rs:26` — `/dev/urandom` 失敗時の
  time/PID + SplitMix64 は予測可能。↔ R1-V14（テスト欠落）と関連。
- **R1S-SEC-03**（Low）`agent/src/main.rs:120` — 生成トークンを stderr へ
  平文出力。ログ共有で認証情報が漏れる。新規。

アーキテクチャ（r1-arch3）

- **R1S-ARCH-01**（Medium）`web/src/shell.rs:12` — 検証用 DOM フックが
  本番 shell に常設され状態が二重管理。新規。
- **R1S-ARCH-02**（Medium）`crates/tm-viewmodel/src/lib.rs:75` —
  `ConfirmKill` / `CancelKill` が Kill 固有名。新規。
- **R1S-ARCH-03**（Medium）`web/src/shell.rs:38` — レイアウト定数の
  手動同期（R1S-OPS-11 と同一）。
- **R1S-ARCH-04**（Low）`crates/tm-core/src/port.rs:118` —
  シグナル危険度が Kill のみ。新規。
- **R1S-ARCH-05**（Low）`app/src/app.rs:38` — Summary が履歴ベクタを
  毎回 all-copy。新規。

運用・保守（r1-ops）

- **R1S-OPS-01**（Medium）`ci.yml:109` — 成果物ジョブに `needs: gates` 無し。
- **R1S-OPS-02**（Medium）`ci.yml:49` — cargo-machete 未固定で非再現的。
- **R1S-OPS-03**（Medium）`deny.toml:65` — 重複バージョンが warn 止まり。
- **R1S-OPS-04**（Medium）`generate-sbom.sh:38` — 非冪等なコピー蓄積。
- **R1S-OPS-05**（Medium）`build-deb.sh:91` — `ldd` の "not found" 黙除外。
- **R1S-OPS-06**（Medium）`cdp_verify_web.py:92` — CDP 切断処理の未反映。
- **R1S-OPS-07**（Medium）`cdp_verify_ime.py:288` — 起動失敗時にサーバ放置。
- **R1S-OPS-08**（Low）`check.sh:37` — 未導入ツールをスキップし成功表示。
- **R1S-OPS-09**（Low）`deny.toml:29` — advisory ignore に `expire` 無し。
- **R1S-OPS-10**（Medium）`supply-chain/config.toml:7` — 未監査なのに
  `safe-to-deploy` exemption。
- **R1S-OPS-11**（Low）`cdp_verify_web.py:46` — 座標の手動同期
  （R1S-ARCH-03 と同一）。
- **R1S-OPS-12**（Low）`poc-wasm/scripts/cdp_verify.py:440` — 常に 0 を返す。

データ・表示安全性（r1-sanitize）

- **R1S-DATA-01**（Medium）`sanitize.rs:28,79-96` — Unicode Cf 未網羅。
- **R1S-DATA-02**（Medium）`sanitize.rs:92` — 補助面 VS17..256 未対応。
- **R1S-DATA-03**（Low）`sanitize.rs:102-105,125-127` — OSC/DCS ペイロード残存。
- **R1S-DATA-04**（Low）`sanitize.rs:28-29,79-95` — ZWJ/ZWNJ の空白化で
  正当な結合を破壊（推測）。
- **R1S-DATA-05**（Medium）`filter_input.rs:94-105`, `query.rs:262-267` —
  NFC/NFD 正規化なしでフィルタ結果が入力法依存。
- **R1S-DATA-06**（Low）`query.rs:262-267` — `Σ` / `ς` が一致しない（推測）。
- **R1S-DATA-07**（Medium）`query.rs:228` — NaN が先頭ソートされ履歴と不一致。
- **R1S-DATA-08**（Low）`format.rs:13` — `u64::MAX` が PiB 表記で誤読。
- **R1S-DATA-09**（Low）`format.rs:48-50,78-81` — 負値が履歴 clamp と不一致。
- **R1S-DATA-10**（Medium）`demo.rs:418` — 極値 tick でオーバーフロー。
- **R1S-DATA-11**（Low）`sanitize.rs:79-96` — Unicode 版との暗黙依存（推測）。

親による直接発見

- **R1S-DOC-01**（Low）`agent/src/main.rs:9` — 「header or query」とあるが
  query は廃止済み。↔ R1-C03 と同一（重複）。

### ラウンド 1 の中間サマリ

- 構造的ギャップ（最優先候補）: R1-V01（CDP 未 CI 化）, R1-V02（Windows が tm-platform 未テスト）,
  R1-G01（モーダル実機クリック未検証）, R1-D01/D02/D03（設計・調査・テスト件数の陳腐化＝AC-12 不成立）。
- セキュリティ: R1-V08（Host 欠落時の検証バイパス）, R1-G07（pidfd）, R1-G09（リモート TLS/認証）。
- 機能: R1-I01（per-process I/O）, R1-I02（エクスポート）, R1-I03（アラート）。
- 既知との重複が多い項目（R1-V03/V04/V09/V10, R1-D08/D14, R1-I20）はラウンド 6 で集約対象。
- lead による事実確認済み: R1-V01, R1-V02, R1-V08, R1-G01, R1-I02/I19（実装なし）。
- 補完チームで **32 件** 追加（合計 119 件）。明確な重複は
  R1S-SEC-01↔R1-V08, R1S-DOC-01↔R1-C03, R1S-ARCH-03↔R1S-OPS-11 の 3 組。
- 補完チームの妥当性内訳: Medium 17 件 / Low 15 件 / High 0 件。

## ラウンド 2 — 脆弱領域の深掘り（完了）

ラウンド 1 で「連続修正が必要だった領域」「テストゼロの領域」を手がかりに、
rx-code が全体走査を打ち切った分を補完する。観点案:

- 全文通読による未完実装の取りこぼし（意味的に空の実装、スワローした Result）
- データ整合性（サニタイズ境界、PID 再利用、wire schema、設定マイグレーション）
- 状態機械（kill フロー、モーダル、paused、エラー復帰）の遷移漏れ
- エージェントの concurrently / timeout / リソースリーク

起動エージェント: `rx2-fullread` / `rx2-state` / `rx2-data` / `rx2-agent` / `rx2-plat`

### 2-F. 補完レビュー（別チーム / 13 件）

2026-11-17 20:03 に、対象ファイルと応答時間を絞った読み取り専用チームを
4 体並列起動した。すべて完走（失敗 0）。障害時の再起動は不要だった。

| エージェント | 結果 |
| --- | --- |
| `rx2-code`（未完実装 / スワロー Result） | 2 件 |
| `rx2-data`（データ整合性 / マイグレーション） | 5 件 |
| `rx2-fsm`（状態機械の遷移漏れ） | 2 件 |
| `rx2-concurrency`（並行 / timeout / リーク） | 3 件 |

#### 親による事実確認（Runtime Proof）（2-F）

- `web/src/shell.rs:139,164` で `interval_ms` が spawn 時に一度だけ捕捉、
  `restore_settings()` はその前（`:217`）に走る（R2-X02）。
- `crates/tm-viewmodel/src/settings.rs:62` が version を常に v2 上書き。
- `crates/tm-core/src/model.rs:20` の `start_time_secs` は秒精度。
- `agent/src/http.rs:314-326` の `query_param` は最初の一致で return。
- `app/src/app.rs:118-123` の 4 呼び出しはすべて `let _ =`。
- `crates/tm-viewmodel/src/lib.rs:409,519-538` の `ConfirmKill` は paused
  非検査、`app/src/app.rs:199` の `request_sample` は paused で即 return。
- `web/src/shell.rs:1739` + `web/src/agent.rs:105` で任意 endpoint へ
  Bearer トークンを送信しうる（R2-P01）。

#### 所見（2-F）

- **R2-C01**（Medium）`app/src/app.rs:118-123` — 設定適用時に返る
  `Vec<Effect>` を 4 回すべて破棄。現状は空だが将来の Effect が失われる。
- **R2-C02**（Low）`app/src/app.rs:219,246` — `this.update` の Result を
  スワローし、entity 消滅時に status 更新が黙って飛ぶ。
- **R2-D01**（Medium）`crates/tm-viewmodel/src/settings.rs:62` — version を
  常に v2 へ上書きし、将来版の新フィールドを復元不能に失う（コメントと矛盾）。
- **R2-D02**（Medium）`app/src/settings.rs:77` — 破損設定がバックアップなしで
  既定値へ上書き。web 側も warn のみで既定へ戻す。
- **R2-D03**（Low）`app/src/settings.rs:112` — 一時ファイル名が PID のみで
  再利用時に残留 tmp と所有権が混在。
- **R2-D04**（Low）`crates/tm-core/src/model.rs:20` — PID 再利用対策の
  start_time が秒精度で同一秒内の再利用を取りこぼす。
- **R2-D05**（Low）`agent/src/http.rs:314` — 重複クエリの先勝ち、
  デコード制御文字も許す（パラメータポイズニング）。
- **R2-F01**（Medium）`crates/tm-viewmodel/src/lib.rs:409,519-538` /
  `app/src/app.rs:199,254` — ConfirmKill が paused を検査せず信号発行。
  送信後の再 sampling も抑制され、画面とPID再利用判定が古いまま（推測）。
- **R2-F02**（Low）`crates/tm-viewmodel/src/lib.rs:488-517` — RequestSignal が
  未解決 pending を無条件上書き（UI が遮断し通常到達しない推測）。
- **R2-X01**（High）`web/src/agent.rs:110` — wasm fetch に timeout が無く
  サンプラーが無期限停止しうる（実機再現は未検証）。
- **R2-X02**（Medium）`web/src/shell.rs:139` — polling 間隔が起動時固定
  （R1-C01「Web に間隔 UI 無し」と部分重複で実害は限定的）。
- **R2-X03**（Low）`agent/src/main.rs:263` — 接続スレッドの JoinHandle を
  破棄し終了処理を追跡できない。
- **R2-P01**（Medium）`web/src/shell.rs:1739` + `web/src/agent.rs:105` —
  `?agent=` に loopback/http 検証が無く、URL 断片の Bearer トークンを
  任意エンドポイントへ送信。エージェント側の厳格検証との非対称。

#### 補完チームのサマリ

- 最優先: R2-X01（无期限ハング）, R2-P01（トークン送信先未検証）,
  R2-D01（設定マイグレーション破壊）, R2-D02（破損設定の無バックアップ上書き）。
- 重複候補: R2-X02↔R1-C01, R2-D04↔R1-G07, R2-P01↔R1-V08。
- 妥当性: High 1 / Medium 6 / Low 6。

### 2-G. 追加深掘り（2026-11-17 20:xx / 新規 30 件 + 重複 5 件）

`rx2-agent` / `rx2-plat` / `rx2-state` / `rx2-data` / `rx2-fullread2` を並列起動
（fullread は失敗したため `rx2-fullread2` に絞り込み再起動）。2-F とは異なる観点
（シークレット漏えい、per-signal 能力、IME 契約、ワイヤ境界）を担当した。
ID は既存と衝突しないよう `R2X-` 接頭辞を使う。

#### 親による事実確認（Runtime Proof）（2-G）

- `agent/src/main.rs:461-503` の `redact_snapshot` は hostname と
  `process.{user,command,name}` のみ。`mount_point` / `os_name` 等は未対象（R2X-A4）。
- `agent/src/main.rs:144-152` の `read_token_file` は `read_to_string` のみで
  パーミッション検証なし（R2X-A2）。
- `agent/src/args.rs:139` は `is_loopback()` で 127.0.0.2 を受理するが
  `agent/src/http.rs:268` の Host 許可リストは 127.0.0.1 のみ（R2X-A9）。
- `web/src/shell.rs:877` は `details_panel()` を無条件追加。native は
  `app/src/app.rs:1657` で `details_visible()` を検査（R2X-P1）。
- `crates/tm-platform/src/lib.rs:544-547` は Windows の Stop/Continue を
  `Unsupported`、Terminate/Kill を `taskkill /F`（R2X-P2/P4）。
- `app/src/app.rs:441` の PID 1 保護は全 OS 共通（R2X-P6）。
- `crates/tm-viewmodel/src/filter_input.rs:132-138` の `unmark()` は marked を
  take するが text を残し、`:94-106` の `committed_text()` を変化させるのに
  `EditOutcome::Composing` を返す（R2X-S1）。既存テスト `:348-357` のコメント
  「確定は abc」と実装が矛盾。
- 同 `:202-240` はバイト範囲を丸めるが `inserted_start` は未丸め UTF-16 のまま
  （R2X-S8）。
- `web/src/agent.rs:148-167` は deserialize → 完全一致 version チェック。
  `sanitize_display` の呼び出しは `tm-platform` のみで web/app にゼロ（R2X-D1）。
- `crates/tm-platform/src/lib.rs:392-400` は `used = total - available`
  （R2X-D5）。

#### 所見（新規のみ）（2-G）

エージェント / シークレット（rx2-agent）

- **R2X-A1**（High）`agent/src/main.rs:416-455` — `--token` が自プロセスの
  argv 経由で snapshot の `process.command` にエコーされ、`--redact` も除去しない。
- **R2X-A2**（High）`agent/src/main.rs:144-152` — トークンファイルの mode を
  未検証（644 でも起動、同一マシンの他ユーザーが読める）。
- **R2X-A3**（High）`agent/src/http.rs:23,81` — read のたびに timeout が
  リセットされ、8 本の trickle 接続で全スロットを約 2 時間占有（slowloris）。
- **R2X-A4**（Medium）`agent/src/main.rs:461-503` — `--redact` が
  `mount_point` / `os_name` / `kernel_version` / IF 名 / `component.label` を
  マスクせず、home パスがマウントポイントから漏れる。
- **R2X-A5**（Medium）`agent/src/http.rs:44-51` — 重複 Host/Authorization/Origin
  の先頭採用（RFC 違反。R1-V08 の「欠落」とは別ベクタ）。
- **R2X-A6**（Medium）`agent/src/main.rs:224,241-260` — 503 応答が accept
  ループ内で同期書き込みされ zero-window ピアに最大 5 秒ブロック。
- **R2X-A7**（Medium）`agent/src/main.rs:263` — `thread::spawn` 失敗が panic 化し
  エージェント全体が終了（Drop ガードの減算はアンワインドで無意味）。
- **R2X-A8**（Low）`agent/src/main.rs:447-453` — 500 応答が probe エラー文字列を
  エコー（現状到達不能）。
- **R2X-A9**（Low）`agent/src/args.rs:139` vs `http.rs:268` — loopback 許可範囲と
  Host 許可リストの不一致（127.0.0.2 で起動すると全リクエスト 403）。
- **R2X-A10**（Low）`agent/src/main.rs:97-99` — `--once` が `--token` 警告より前
  に return し、argv 露出の警告が出ない。

プラットフォーム / shell パリティ（rx2-plat）

- **R2X-P1**（High）`web/src/shell.rs:870-878,1190-1198` — `details_visible`
  設定を保存・復元するがパネルを常時描画し、設定が完全に無効（R1-D04 の doc
  問題とは別の実装バグ）。
- **R2X-P2**（High）`app/src/app.rs:1426,1506-1573` — Windows で Stop/Continue
  ボタンが常に表示され常に失敗、かつ「プロセス操作は無効」と誤導（実際は
  Terminate は動作）。per-signal 能力フラグが `port.rs:14-25` に無い。
- **R2X-P3**（Medium）`web/src/shell.rs:288-325` — 修飾キーを全て早期 return
  し Ctrl+F / F2 / F5 が欠落（R3-A02 と部分重複、具体的キー欠落は追加情報）。
- **R2X-P4**（Medium）`crates/tm-core/src/port.rs:100-105,119-121` — Windows の
  Terminate は「終了 (SIGTERM)」かつ非破壊表示だが実態は `taskkill /F`。
- **R2X-P5**（Medium）`crates/tm-platform/src/lib.rs:559-571` — taskkill 失敗が
  PermissionDenied ではなく単一 `Failed` にマッピング。
- **R2X-P6**（Medium）`crates/tm-platform/src/lib.rs:441` — PID 1 保護が Unix
  前提で Windows にも適用され、正当なプロセスを永不変で拒否。
- **R2X-P7**（Medium）`crates/tm-platform/src/lib.rs:140,182-198` — Users が
  `new()` 時のみ取得され refresh されず、ユーザー列が陳腐化。
- **R2X-P8**（Medium）`crates/tm-platform/src/lib.rs:192-236` — 収集失敗が空
  リストに化け `supports_*` は true のまま、「空＝非対応」を区別不可。
- **R2X-P9**（Low）`web/src/shell.rs` — F1 ヘルプ／ショートカット一覧が皆無。
- **R2X-P10**（Low）`crates/tm-platform/src/lib.rs:253-255,261` — load_average /
  uptime の 0 フォールバックが実測 0 と区別不能（実機未検証）。
- **R2X-P11**（Low）`poc-wasm/src/lib.rs:43,326` — PoC が本番の model/sanitize/
  query と乖離し将来作業を誤導（R1-C05 は「ダミー行」のみで観点が新規）。
- **R2X-P12**（Low）`web/src/shell.rs:131-169` — SetPaused がデッド遷移
  （R2-S5 と重複、参照先として統合）。
- **R2X-P13**（Low）`crates/tm-platform/src/lib.rs:220-236` — `component_snapshots`
  が毎サンプル `Components` を再生成（計測根拠なし）。

状態機械（rx2-state）

- **R2X-S1**（High）`crates/tm-viewmodel/src/filter_input.rs:132-138` — `unmark()`
  が「確定文字列は不変」契約に違反し、キャンセルした preedit が確定フィルタへ
  漏れる（純粋ロジックとしてコード上確定）。
- **R2X-S2**（Medium）`app/src/app.rs:421-428,325-329` — 変換中の Esc が
  プラットフォーム側 composition を取り消さず入力受付だけ閉じる。
- **R2X-S7**（Low）`crates/tm-viewmodel/src/lib.rs:329-337,596-600` —
  `Select(None)` が selection_hint を更新せず次 Tick で自動再選択。
- **R2X-S8**（Medium）`crates/tm-viewmodel/src/filter_input.rs:202-240` —
  UTF-16 範囲を文字境界へ丸めた後も未丸めオフセットで selection/marked を
  計算し、テキスト長超過の範囲外状態を作る（純粋ロジックとして確定）。

データ整合性（rx2-data）

- **R2X-D1**（Medium）`web/src/agent.rs:148-154` — ワイヤ境界で `sanitize_display`
  未実施。悪意/破損エージェントの制御文字・ANSI がそのまま描画される。
- **R2X-D2**（Low）`crates/tm-platform/src/lib.rs:129-134` — `os_name` /
  `kernel_version` / `os_label` がネイティブ側でも未サニタイズ。
- **R2X-D3**（Medium）`web/src/agent.rs:159-167` — schema version が完全一致
  判定で前方/後方互換とも皆無。加算的 v2 で全ライブデータを拒否。
- **R2X-D5**（Medium）`crates/tm-platform/src/lib.rs:392-400` — ディスク使用率
  が `total - available`（予約ブロックを used に算入）で過大表示。
- **R2X-D7**（Low）`crates/tm-core/src/metrics.rs:105-119` — ネットワーク
  レートの前方ジャンプ（NIC ホットプラグ等）に上限なし。

フォーカス走査（rx2-fullread2 / 否定証拠）

- `unimplemented!` / `todo!` / `unreachable!`: 実ソース 0 件。
- `panic!`: テスト内 2 件のみ。`#[allow(...)]`: clippy 理由付き 2 件のみ。
- **R2X-FR1**（Low）`agent/src/main.rs:222-226` — accept 失敗時にバックオフ
  なく eprintln のみでループ継続（EMFILE 等でビジーループ化、推測含む）。
- **R2X-FR2**（Low）`crates/tm-platform/src/lib.rs:507-510` — 非 Linux で
  thread_count が常に None（`:15` に明記された意図的制限で実質既知）。

#### 重複（既存 ID へ統合）

| 新 ID | 統合先 | 理由 |
| --- | --- | --- |
| R2X-S3 | R2-D01 | SETTINGS_VERSION が常に v2 上書きで migration 不在 |
| R2X-S4 | R3-S01 | live エラーが初回スナップ後 UI に出ない |
| R2X-S5 | R2-S5 / R2X-P12 | web の SetPaused デッド遷移 |
| R2X-S6 | R2-F02 | RequestSignal が pending を無条件上書き |
| R2X-D4 | R2-D01 | 設定 version マイグレーション不在 |
| R2X-D6 | R2-D04 / R1-G07 | start_time 1 秒精度（pidfd 関連） |

#### ラウンド 2 追加分のサマリ

- 新規 30 件（High 6 / Medium 13 / Low 11）＋重複 6 件。
- 最優先: R2X-A1/A2（シークレット漏えい）, R2X-P1（設定が無効）,
  R2X-P2（Windows 誤導）, R2X-S1/S8（IME 純粋ロジック欠陥）。
- `rx2-fullread` は失敗したが `rx2-fullread2` へ絞り込み再起動して回収済み。
- 累計 189 + 30 = 219 件。

## ラウンド 3 — 実利用シナリオと UX ウォークスルー（実施済み）

初回起動、エージェント停止、設定破損、kill 確認、バージョン更新、
マルチモニタ/HiDPI などの現実シナリオを条件付きで追い、
「想定していない経路」の残作業を探す。アクセシビリティ・ローカライゼーションも対象。

### 3-A. シナリオ別レビュー（実施済み 2026-11-17 / 20 件）

読み取り専用エージェント 4 体を並列起動（起動接続 / a11y / ダイアログ /
ライフサイクル）。すべて完走、失敗なし。

#### 親による事実確認（Runtime Proof）（3-A）

- `web/src/shell.rs:189-205` の Err 分岐は status のみ更新し旧 snapshot を維持。
- `app/src/app.rs:103` は NativeProbe 失敗時に DemoProbe を差し込む。
- `app/src/main.rs:26` は `--demo` 完全一致のみ（`--demo=yes` は無視）。
- `web/src/shell.rs:288-311` は修飾キーで return、矢印等を stop_propagation。
- `web/index.html:26` は `<body></body>` で canvas のみ、aria 関連 0 件。
- `app/src/app.rs:406` は `modifiers.control` のみ（platform 修飾子なし）。
- `app/src/settings.rs:77` は eprintln のみで GUI 通知なし。
- `app/src/app.rs:1819` の注意文は全シグナルで「取り消せません」固定。
- `clear_status()` は viewmodel:476 に定義のみで app 側から未呼び出し。
- `packaging/linux/build-deb.sh` の DEBIAN は control のみ（maintainer
  スクリプトなし、`:109`）。

#### 所見（3-A）

起動・接続（rx3-startup）

- **R3-S01**（High）`web/src/shell.rs:199` — 接続失敗後も旧 snapshot が
  「最新」として表示され、有効データの境界が不明。
- **R3-S02**（Medium）`app/src/app.rs:103` — ネイティブ probe 失敗時に
  デモへ自動フォールバック（Web は明示拒否しており一貫性なし）。
- **R3-S03**（Medium）`app/src/main.rs:26` — GUI の `--demo` が不正値を
  黙って無視し実データ起動に落ちる（agent 側は拒否）。
- **R3-S04**（Medium）`web/src/shell.rs:92` — エージェント再起動時に
  接続先設定が再読込されない（推測）。
- **R3-S05**（Low）`web/src/shell.rs:102` — URL 未指定の初回起動が
  通知なしでデモモードになる。

アクセシビリティ（rx3-a11y）

- **R3-A01**（High）`web/index.html:20-27` — canvas のみで aria/accesskit の
  代替表現が皆無。スクリーンリーダーが何も読み上げられない。
- **R3-A02**（Medium）`web/src/shell.rs:288-311` — 入力中も矢印キーを
  横取りし、修飾キーショートカットを全破棄（IME 候補選択への影響は推測）。
- **R3-A03**（Medium）`app/src/app.rs:406,449-451` — control のみ判定で
  macOS の Cmd 修飾キー非対応（GPUI のマッピング依存、推測）。
- **R3-A04**（Medium）`web/index.html:2-6` — lang/title と UI が日本語
  ハードコードで i18n 機構なし。
- **R3-A05**（Low）`web/index.html:5,20-24` — HiDPI / モニタ間移動時の
  スケール追従が見当たらない（実機未検証の推測）。

ダイアログ / 設定 UX（rx3-dialog）

- **R3-U01**（High）`app/src/settings.rs:77` — 破損設定の読み込み失敗が
  stderr のみで GUI に通知されない（R2-D02 と関連）。
- **R3-U02**（Medium）`web/src/settings.rs:41` — parse 失敗が console
  警告だけで画面に反映されない。
- **R3-U03**（Medium）`app/src/app.rs:1819` — Stop / Continue でも
  「この操作は取り消せません」と表示され利用者を誤導。
- **R3-U04**（Medium）`app/src/app.rs:1703` — status の成功/エラーが
  色分けのみで判別される（色覚特性・モノクロ環境で問題）。
- **R3-U05**（Medium）`app/src/app.rs:115` — transient な StatusMessage が
  app 側でクリアされず残存（lib.rs 側の経路は未確認、推測）。

ライフサイクル（rx3-lifecycle）

- **R3-L01**（High）`packaging/linux/build-deb.sh:72-86` — 削除/更新用
  maintainer スクリプトが無く、設定・証跡・ログの残置方針が未定義。
- **R3-L02**（Medium）`build-deb.sh:51-63,109-117` — `--skip-build` 時に
  control の Version と実際のバイナリが不整合になり得る。
- **R3-L03**（Medium）`README.md:89,94` — 固定 loopback ポートで複数
  インスタンス共存不可、切替手順も未記載。
- **R3-L04**（Medium）`scripts/dev-setup.sh:44-45` / `app/build.rs:13-17` —
  ローカル shim 残置で更新後に誤リンクし得る。

#### ラウンド 3 のサマリ

- 最優先: R3-S01（古いデータの提示）, R3-A01（a11y 皆無）,
  R3-U01（設定破損の非通知）, R3-L01（アンインストール方針未定義）。
- 重複候補: R3-U01↔R2-D02（破損設定）, R3-S01↔R2-X01（接続失敗の扱い）,
  R3-S02↔R1-V05（デモ/実データの判別）, R3-L02↔R1-OPS-01（成果物整合）。
- 妥当性: High 4 / Medium 13 / Low 3。累計 132 + 20 = 152 件。

## ラウンド 4 — 敵対的レビュー / 本番運用（実施済み）

障害モード、リソース枯渇、DoS、権限境界、依存・サプライチェーン、
公開リポジトリとしての運用（issue/tag/release、SECURITY ポリシー）を
敵対的視点でレビューする。ラウンド 1〜3 の所見のうち妥当性が曖昧なものも再検証。

### 4-A. 敵対レビューと再検証（実施済み 2026-11-17 / 16 件 + 判定 5 件）

読み取り専用エージェント 4 体を並列起動（敵対入力 / サプライチェーン /
公開運用 / 妥当性再検証）。すべて完走、失敗なし。

#### 親による事実確認（Runtime Proof）（4-A）

- `web/src/agent.rs:123-139` は `Vec::new()` を `take(8MiB)` で読む。
  **R4-A02 の「8MiB を前もって確保」は誤り**（実際は段階的伸長）。
  ただし最大 8MiB までの累積と並行 fetch の懸念は残る。
- `NOTICE.md:11-13` の GPUI 帰属・gpui-component の link 先を確認。
- `web/Cargo.toml:1-25` に `rust-version` が無く、gpui-pre 0.3.12 を直接依存。
- `.github/workflows/ci.yml:7,10,109-175` に pull_request Trigger と
  release-linux / web-dist / sbom ジョブを確認。
- `README.md:88-89` に `TM_AGENT_TOKEN=test` の例を確認。
- `crates/tm-platform/src/lib.rs:187` が `ProcessesToUpdate::All` で更新。

#### 所見（4-A）

敵対入力 / DoS（rx4-adversarial）

- **R4-A01**（Medium）`crates/tm-platform/src/lib.rs:187` — 全プロセス＋
  全スレッド更新が probe mutex を長時間占有し、8 接続すべてをブロックし
  連鎖タイムアウトしうる（実測なしの推測）。
- **R4-A02**（Medium）`web/src/agent.rs:123-139` — 応答上限 8MiB までの累積と
  並行 fetch で wasm ヒープを圧迫しうる。※「 upfront 確保」は誤記を訂正。
- **R4-A03**（Low）`agent/src/main.rs:586` — 不正 `limit` 値がエラーボディへ
  そのままエコーされる（8KiB 上限で増幅は限定的）。

サプライチェーン（rx4-supplychain）

- **R4-S01**（Medium）`NOTICE.md:11` — GPUI 帰属が実際の取得元（gpui-pre
  0.3.12 再公開スナップショット）と不一致。
- **R4-S02**（Medium）`NOTICE.md:13` — gpui-component のライセンス出典 URL が
  gpui-kit と同一で誤り。
- **R4-S03**（Low）`web/Cargo.toml:5` — 独立 workspace に rust-version が無く
  MSRV 未固定（edition 2024 / wasm-bindgen 0.2.120 を要求）。
- **R4-S04**（Low）`deny.toml:22` — 既定 feature のグラフのみ監査され、
  web の feature 差分構成が監査をすり抜ける可能性（推測）。

公開運用 / 権限（rx4-ops）

- **R4-O01**（Medium）`agent/src/auth.rs:50-63` — 弱い乱数フォールバック
  （R1S-SEC-02 / R1-V14 と重複）。
- **R4-O02**（Medium）`.github/workflows/ci.yml:109-137` — fork PR のコードが
  リリース成果物ジョブでビルド・upload される（シークレットは contents: read
  で守られるが、未信頼コードの成果物が「リリース」名で残る）。
- **R4-O03**（Low）`README.md:88-89` — `TM_AGENT_TOKEN=test` のコピペ例が残る。
- **R4-O04**（Low）`SECURITY.md:18` vs `README.md:27-29` — private
  vulnerability reporting の有効化記載が無い（設定はローカルから未確認）。

#### 妥当性再検証（rx4-verify）— 親による上書き判定を含む

| ID | エージェント判定 | 親の最終判定 | 根拠 |
| --- | --- | --- | --- |
| R2-X02 | 誤検知 | **妥当（潜在）** | エージェント自身の根拠が「spawn 時に一度だけ読む」と原所見を裏付ける。Web に間隔 UI が無いため現在の実害は限定的。 |
| R2-P01 | 妥当（High） | 妥当（High） | endpoint に許可リスト検証なし。 |
| R2-F01 | 妥当（High） | 妥当（Medium） | confirm_kill は paused 非検査。ただし到達経路が限定的（UI が modal を遮断）。 |
| R3-S04 | 妥当（Medium） | 妥当（Medium） | `agent_config_from_url()` は new() の 1 回のみ。通常はリロード運用で許容。 |
| R3-A05 | 要実機 | 要実機（Low） | HTML のみでは DPR 追従を確定できず。実機検証が必要。 |

- エージェント判定を無条件採用せず、R2-X02 は親の Runtime Proof で
  「誤検知」→「妥当（潜在）」へ上書きした（理由を表に記録）。
- R2-F01 は到達経路が限定的なため High → Medium に格下げ。

#### ラウンド 4 のサマリ

- 新規 16 件（High 0 / Medium 7 / Low 9）＋再検証 5 件。
- 最優先: R4-O02（fork PR の成果物）, R4-S01/S02（ライセンス帰属の誤り）,
  R4-A01（mutex 占有による連鎖タイムアウト）。
- 重複: R4-O01↔R1S-SEC-02↔R1-V14, R4-S01↔R1-G08（GPUI pin）。
- 累計 152 + 16 = 168 件。

## ラウンド 5 — 新カテゴリと目新しさの最終掃き出し（実施済み）

観測性（ログ/メトリクス）、CLI/自動化、データ民生、ライセンス正確性など
これまでの観点に入っていないカテゴリを追加し、ラウンド 1〜4 で漏れた
機能追加案を出す。各所見は NEW 判定を再度確認する。

### 5-A. 新カテゴリ走査（実施済み 2026-11-17 / 21 件 + 却下 1 件）

読み取り専用エージェント 4 体を並列起動（観測性 / データ民生 / 機能案 /
テスト・ドキュメント網羅）。すべて完走、失敗なし。

#### 親による事実確認（Runtime Proof）（5-A）

- `agent/src/main.rs` に `eprintln!` 10 箇所、時刻/レベル/構造化フィールドなし。
- `/healthz` の `HealthBody` は `{ok, schema_version}` のみでメトリクスなし。
- `app/src/settings.rs:106-129` の `save_to()` は既知フィールドのみ再
  シリアライズ（未知キー保持なし、0600 + PID tmp + atomic rename は適切）。
- `NOTICE.md:9-15` に IBM Plex の記載なし（`web/README.md:24,200` にはあり、
  `web/assets/fonts/LICENSE-IBM-Plex.txt` 同梱）。
- `crates/tm-core/src/format.rs:71` の `format_process_age` は
  `app/src/app.rs:1469,1741` と `web/src/shell.rs:1312` で利用済み。
- `supports_network` / `supports_disks` は app/web の src で未参照。
- `markdownlint` は scripts / .github に 0 件、`notes/index.md` に
  `residual-tasks-research.md` の記載なし。
- `README.md:172` の clippy 行は `undocumented_unsafe_blocks` フラグが
  `notes/current-state.md:33` と不一致。

#### 所見（5-A）

観測性 / CLI（rx5-observability）

- **R5-O01**（High）`agent/src/main.rs:82` — ログが自由形式 eprintln のみで
  構造化・レベル・時刻が欠如。ログ集約での絞り込み・相関ができない。
- **R5-O02**（High）`agent/src/main.rs:196` — `active` カウンタを維持するが
  `/healthz` は `{ok, schema_version}` のみでメトリクス公開の足場が無い。
- **R5-O03**（Medium）`agent/src/main.rs:83` — 全失敗が単一 `exit(1)` で
  終了コードが分類できない（自動化側が stderr 文字列に依存）。
- **R5-O04**（Medium）`agent/src/main.rs:281` — リクエスト単位の
  アクセスログ・相関 ID が皆無（遅延やモード差を事後追跡できない）。
- **R5-O05**（Medium）`agent/src/args.rs:14` — agent 側 CLI に
  `--version` / `--log-level` / `--format` が無く app と不揃い。

データ民生 / ライセンス（rx5-data）

- **R5-D01**（Medium）`NOTICE.md:9` — 同梱の IBM Plex フォント帰属が
  NOTICE に未記載（ライセンス文自体は同梱のため Medium）。
- **R5-D02**（Medium）`app/src/settings.rs:106` — 未知フィールドが次回
  save で黙って失われ、前方互換がラウンドトリップしない
  （R2-D01 と関連するが保存側の独立問題）。
- **R5-D03**（Low）`crates/tm-viewmodel/src/settings.rs:31` — データ保持期間 /
  インポート方針の表現が無い（要件想定は未確認の推測）。

機能案（rx5-ideas）

- **R5-I01**（High）`crates/tm-core/src/model.rs:211` — 仮想メモリの
  ソート・絞り込み対応（SortKey は resident のみ）。
- **R5-I02**（High）→ **却下** — 「`format_process_age` が未活用」という
  根拠は誤り。`app/src/app.rs:1469,1741` と `web/src/shell.rs:1312` で
  利用済み（親の Runtime Proof で確認）。アイデア自体（稼働時間列の
  可読性）は既に実装されているため NEW ではない。
- **R5-I03**（Medium）`crates/tm-core/src/model.rs:330` — 温度の
  `critical_c` / `max_c` を使ったしきい値ゲージ。
- **R5-I04**（Medium）`crates/tm-core/src/port.rs:18` — `supports_*`
  Capability による機能の動的無効化（app/web で未参照を確認）。
- **R5-I05**（Low）`crates/tm-core/src/metrics.rs:161` — `RingBuffer.earliest()`
  を利用した履歴トレンド差分バッジ。

テスト / ドキュメント（rx5-coverage）

- **R5-C01**（High）`.markdownlint.json:1` — 設定のみで check.sh / CI に
  markdownlint ゲート未配線（MD025/MD041 の回帰検出不可）。
- **R5-C02**（Medium）`notes/index.md:1` — `residual-tasks-research.md` が
  索引に未掲載（本ラウンド 6 で対応予定）。
- **R5-C03**（Medium）`README.md:172` — clippy コマンドが
  `current-state.md:33` と不一致（`undocumented_unsafe_blocks` 欠落）。
- **R5-C04**（Low）`README.md:166` — 検証章に doc-test / rustdoc の
  結果行が無い（ゲートは存在する）。

#### ラウンド 5 のサマリ

- 新規 21 件（High 4 / Medium 11 / Low 6）＋却下 1 件（R5-I02）。
- 却下理由を記録し、無条件採用を避けた（親の Runtime Proof 優先）。
- 最優先: R5-O01/O02（観測性の欠如）, R5-C01（markdownlint 未ゲート）,
  R5-I01（仮想メモリのソート欠落）。
- 重複: R5-D02↔R2-D01（設定の前方互換）, R5-C03↔R1-D03（テスト件数/
  コマンドの陳腐化）, R5-D01↔R4-S01/S02（NOTICE 正確性）。
- 累計 168 + 21 = 189 件。

## ラウンド 6 — 集約（実施済み 2026-11-17）

ラウンド 1〜5 の全所見 189 件について、重複統合・妥当性再判定・既知項目の
分離を行い、優先度付き最終バックログをルート `todo.md` へ追記した。

### 6-A. 重複統合（正規 ID → 統合元）

| 正規 ID | 統合元 | 内容 |
| --- | --- | --- |
| R1-V08 | R1S-SEC-01 | Host ヘッダ欠落時の検証迂回 |
| R1-C03 | R1S-DOC-01 | `main.rs:9` の「header or query」記載 |
| R1S-SEC-02 | R1-V14, R4-O01 | 弱い乱数フォールバック |
| R1S-ARCH-03 | R1S-OPS-11, R1-OPS-11 | レイアウト定数の手動同期 |
| R1-C01 | R2-X02 | Web の更新間隔が起動時固定 |
| R1-G07 | R2-D04, R2-D03 | PID 再利用（start_time 秒精度 / tmp 名） |
| R2-D01 | R5-D02 | 設定 version 強制上書きと未知フィールド喪失 |
| R2-D02 | R3-U01, R3-U02 | 破損設定の無バックアップ上書きと非通知 |
| R2-X01 | R3-S01 | 接続失敗時のデータ鮮度（timeout と stale 表示） |
| R4-S01 | R4-S02, R5-D01, R1-G08 | NOTICE のライセンス帰属・GPUI pin |
| R1-D03 | R5-C03 | ドキュメント記載の陳腐化（テスト件数 / clippy） |
| R1-D16 | R5-C01, R5-C04 | ドキュメント検証ゲートと AC-12 マトリクス |
| R1-OPS-01 | R4-O02 | CI ゲートと成果物生成の前置関係 |
| R1S-ARCH-04 | R3-U03 | シグナル危険度と確認文言の不一致 |
| R2-P01 | R1-V08 | loopback 検証の非対称（関連グループ） |

### 6-B. 妥当性の再判定・却下

- **却下: R5-I02** — 「`format_process_age` 未活用」は誤り。
  `app/src/app.rs:1469,1741` / `web/src/shell.rs:1312` で利用済み。
- **上書き: R2-X02** — エージェントは「誤検知」と判定したが、根拠自身が
  「spawn 時に一度だけ読む」と原所見を裏付けたため「妥当（潜在）」に戻す。
- **格下げ: R2-F01** — High → Medium（到達経路が modal 遮断で限定的）。
- **要実機: R3-A05** — HTML のみでは DPR 追従を確定できない。
- **訂正: R4-A02** — 「8MiB を upfront 確保」は誤り（`Vec::new()` は
  段階的伸長）。8MiB までの累積と並行 fetch の懸念は妥当として残す。

### 6-C. 既知項目の分離

以下は `notes/current-state.md`「未解決・今後の候補」に既出のため、
`todo.md` へ重複登録せず参照のみとした: `prefers-color-scheme` 連動、
同形異義文字サニタイズ、実 IME 手動確認、431/400/503 の CORS、
Windows/macOS 物理実機検証、優先度変更、GPU 詳細監視、DEB 署名・公開。

### 6-D. 最終バックログ

- 189 件 → 重複 17 件統合・却下 1 件 → **約 171 件の実所見**。
- これをテーマ別に約 40 項目へグルーピングし、
  ルート `todo.md` の `## 未着手`（高 / 中 / 低）へ追記した。
- 全所見の根拠・Runtime Proof・却下理由は本ファイルに残す。

### 6-E. ラウンド 2 追加分（2-G）の事後統合

189 件集約後に、ラウンド 2 の追加深掘り（2-G）で 30 件を検出した。
重複 6 件は 6-A の既存 ID へ吸収し、新規 30 件を最終バックログへ反映済み。

- シークレット: R2X-A1（argv エコー）/ A2（ファイル mode）/ A4（redact 漏れ）を
  既存の「トークン露出」グループ（R1S-SEC-03, R1S-SEC-04）へ統合。
- 能力モデル: R2X-P1（details_visible 無効）/ P2（Windows Stop/Continue）を
  既存の R5-I04（capability 動的無効化）グループへ統合。
- IME 純粋ロジック: R2X-S1（unmark 契約違反）/ S8（UTF-16 範囲外）は
  新規独立項目として高優先へ追加。
- データ境界: R2X-D1（ワイヤ未サニタイズ）/ D3（schema 互換）/ D5（ディスク
  使用率）を新規独立項目として追加。
- エージェント robustness: R2X-A3/A6/A7/FR1 を既存の接続ライフサイクル
  グループ（R1-G05, R2-X03）へ統合。
- 集約後总数: 189 + 30 = 219 件（重複 17 + 6 = 23 件統合、却下 1 件）。

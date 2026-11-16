---
type: Design
title: GPUI Task Manager 設計書
description: Ubuntu系向けGPUIシステムタスクマネージャーのアーキテクチャ・品質特性・セキュリティ・テスト戦略
tags: [gpui, rust, task-manager, system-monitor, wasm, design]
status: draft
generated: 2026-11-14T20:30:00+09:00
---

## 1. 目的とスコープ

### 1.1 目的

Ubuntu 系デスクトップ（Kubuntu / KDE Plasma / Wayland を第一ターゲット）で動く
システムタスクマネージャーを、GPUI（Zed の GPU アクセラレート UI フレームワーク）
で試作する。同時に、同じコアをブラウザ（wasm）で動かせる構造を示す。

対象とする利用シナリオ:

- プロセス一覧を見て、CPU / メモリ消費の大きいプロセスを特定する。
- プロセスの詳細（PID、ユーザー、コマンド、起動からの経過）を確認する。
- 不要なプロセスを安全に終了する（確認ダイアログ + 保護対象の拒否）。
- システム全体の CPU / メモリ / ネットワーク / ディスクの傾向を俯瞰する。
- ブラウザから同じ画面構成で「読み取り専用」の監視ダッシュボードを見る。

### 1.2 非目標（MVP では扱わない）

- root 権限が必要な操作（優先度変更、cgroup 操作、カーネルパラメータ）。
- リモートホストの監視（SSH / Prometheus 連携）。ただし拡張点は設計に残す。
- Windows / macOS の**物理実機**での検証（OS 別 cfg と GitHub Actions の
  windows-latest / macos-latest を実機相当の検証として使う）。
- 設定の永続化（ネイティブ: ファイル、ブラウザ: localStorage）、自動更新、多言語化（日本語 UI のみ）。
- ブラウザからのプロセス操作（安全側に倒して読み取り専用）。

### 1.3 品質特性と設計手段

| 品質特性 | 設計手段 | 検証方法 |
|---|---|---|
| 保守性 | 依存方向を一方向に固定（core ← viewmodel ← shell）、`#![forbid(unsafe_code)]`、`#![deny(missing_docs)]` | コンパイル・clippy・コードレビュー |
| モジュール性 | 5 crate 構成。プラットフォーム依存は `tm-platform` に隔離 | `cargo tree` / 依存方向レビュー |
| 効率性 | 1 秒間隔の差分サンプリング、固定長履歴、仮想リスト、描画時の全件 clone 禁止 | サンプル所要時間の実測、UI 応答確認 |
| 移植性 | コア・ビューモデルは `std` のみで wasm 対応、OS 依存は trait の裏 | wasm32 ビルド、Linux 実機 |
| 共通部品 | 表示整形・サニタイズ・ソート/フィルタ・履歴を `tm-core` に集約 | 単体テスト（golden） |
| テスト容易性 | 純粋関数 + reducer + `DemoProbe`。I/O は trait 境界のみ | `cargo test` 60 件超 |
| 操作容易性 | キーボード完結（↑↓/Space/Ctrl+F/Ctrl+K/F1）、クリック選択、確認ダイアログ | 起動検証・手動確認 |
| 拡張性 | `SystemProbe` の差し替え、`Command`/`Effect` の追加、エージェントのスキーマ版管理 | 設計レビュー |
| 美しさ | トークン化した配色、カード + テーブル + 詳細パネルの一貫レイアウト | スクリーンショット |
| セキュリティ | unsafe 禁止、読み取り専用ブラウザ、ループバック限定エージェント、PID 再利用防御 | 脅威モデル・テスト・コードレビュー |

---

## 2. アーキテクチャ

### 2.1 レイヤー構成

```text
+---------------------------------------------------------------+
| app/  (native shell)          web/  (wasm shell, browser)     |
|   GPUI native + gpui-kit        GPUI web + demo/agent data     |
+---------------------+----------------+------------------------+
                      |  Command / Effect
+---------------------v----------------v------------------------+
| tm-viewmodel : 画面状態の reducer（純粋・I/O なし）            |
|   Dashboard, Command, Effect, PendingKill                      |
+---------------------+-----------------------------------------+
                      |  SystemSnapshot, ProcessSnapshot, ...
+---------------------v-----------------------------------------+
| tm-core : ドメインモデル / メトリクス / クエリ / ポート        |
|   ProcessKey, SystemSnapshot, RingBuffer, Filter, SystemProbe  |
+---------------------+-----------------------------------------+
                      |  SystemProbe 実装
+---------------------v-----------------------------------------+
| tm-platform : sysinfo + rustix（ネイティブのみ）              |
+---------------------------------------------------------------+

agent/ : tm-platform を読み取り専用 HTTP に変換（ブラウザのライブ用）
```

依存方向の規則:

1. `tm-core` はどの crate にも依存しない（`serde` のみ任意 feature）。
2. `tm-viewmodel` は `tm-core` のみに依存する。
3. `tm-platform` は `tm-core` に依存し、UI には依存しない。
4. `app` / `web` は `tm-core` + `tm-viewmodel` に依存する。`app` だけが
   `tm-platform` を知る。
5. `web` はネイティブ依存（sysinfo / rustix / std::fs）を持たない。

この規則により、ブラウザ版でも同じビューモデル・同じ表示文字列を使える。

### 2.2 crate 一覧

| crate | 役割 | 主な依存 | テスト |
|---|---|---|---|
| `tm-core` | 型・メトリクス・整形・サニタイズ・ソート/フィルタ・ポート | なし（任意 serde） | 47 |
| `tm-viewmodel` | 画面状態 reducer、選択・ダイアログ・更新間隔 | `tm-core` | 13 |
| `tm-platform` | sysinfo 収集、rustix シグナル、PID 再利用防御 | `tm-core`, sysinfo, rustix | 8 |
| `gpui-task-manager` | ネイティブ GPUI シェル | `gpui-kit`, core, viewmodel, platform | UI スモーク |
| `tm-agent` | 読み取り専用ループバック HTTP | `tm-core`, `tm-platform`, serde_json | 単体 + curl |

### 2.3 データフロー

```text
timer (250ms..10s)
   -> Dashboard::apply(Command::Tick) -> [Effect::SampleNow]
   -> background executor: SystemProbe::sample()（UI スレッド外）
   -> Dashboard::ingest(SystemSnapshot)
        -> 前回サンプルとの差分からネットワークレートを計算
        -> 固定長履歴に push
        -> 選択プロセスの維持（消えた場合は近傍へフォールバック）
   -> cx.notify() -> GPUI が再描画
```

ユーザー操作は必ず `Command` に変換し、`Dashboard::apply` が返す `Effect` だけを
シェルが実行する。UI から `SystemProbe` を直接叩かない。

---

## 3. ドメインモデル

### 3.1 主要な型

| 型 | 不変条件 / 意味 |
|---|---|
| `ProcessKey { pid, start_time_secs }` | プロセスの安定識別子。PID 再利用を検出する |
| `Timestamp { unix_millis }` | サンプル時刻。UI 表示は UTC |
| `ProcessSnapshot` | 名前・コマンド・ユーザー・状態・CPU%・RSS・VSZ・スレッド・親 PID |
| `CpuSnapshot` | 全体/コア別使用率、load average、uptime、周波数 |
| `MemorySnapshot` | RAM / swap の使用量。`used_percent()` は 0..=100 に clamp |
| `NetworkSnapshot` | インターフェース別の累積カウンタ。レートは viewmodel で差分計算 |
| `SystemSnapshot` | 1 時点の全体像。`self_pid` は自分自身の保護に使う |

### 3.2 CPU とレートの計算

- CPU はプラットフォームが「前回サンプルからの差分」を返す。sysinfo の
  `refresh_cpu_usage` はこの差分計算を内部で行う。最初のサンプルは 0% になる
  ため、UI は「取得中」を表示できる。
- Linux の `/proc/stat` を直接読む場合に備え、`CpuCounters::percent_since()` を
  core に用意した（テスト済み・飽和・逆転・0 除算を処理）。
- ネットワークは累積カウンタの差分を経過秒で割る。`NetworkCounters::rate_since()`
  は経過 0 秒・非有限値を安全に 0 にする。
- sysinfo の `Process::cpu_usage()` は本機で飽和した（全プロセスがコア数×100% に
  張り付く）。`tm-platform` は `Process::accumulated_cpu_time()` の差分を実測経過
  時間で割る方式に切り替え、プロセス CPU% を自前計算する。全体 CPU は sysinfo の
  差分をそのまま使う。

### 3.3 PID 再利用防御

PID は再利用される。確認ダイアログを開いている間に元のプロセスが終了し、同じ
PID の別プロセスが起動する可能性がある。対策を多層化する。

1. `ProcessKey` は PID と起動時刻の組で識別する。
2. `tm-platform::control` はシグナル送信直前にプロセスを再取得し、起動時刻が
   一致することを確認する。不一致なら `ControlError::NotFound`。
3. `tm-viewmodel` は確認時にスナップショット上に同一キーが存在するか再確認する。
4. PID 1 と自分自身は viewmodel と platform の両方で拒否する。

---

## 4. ビューモデル（状態機械）

`Dashboard` は UI 非依存の reducer である。

- 入力: `Command`（Tick, Select, SelectRelative, SetSortKey, SetSortSpec,
  SetFilterText, ClearFilter, SetInterval, SetPaused, ToggleDetails,
  SetDetailsVisible, ToggleTreeView, SetTreeView, ToggleTreeNode,
  RequestSignal, ConfirmKill, CancelKill）。
- 出力: `Effect`（SampleNow, Signal { key, signal }）。
- 保持状態: 最新スナップショット、フィルタ後ソート済みインデックス、履歴、
  選択キー、確認待ちシグナル、ステータスメッセージ、更新間隔、一時停止、
  ツリー表示の折りたたみ集合（セッションローカル。永続化するのは
  `tree_view` のオン/オフ設定のみ）。
- 表示順は常に「フィルタ → ソート → PID 昇順のタイブレーク」で決定的。
- 選択はキーで保持するため、再ソートやサンプル更新で行が動いても追従する。
  消えた場合は直前の位置に近い行へフォールバックする。
- 確認ダイアログは `PendingKill` として保持し、`ConfirmKill` でのみ
  `Effect::Signal` を出す。`RequestSignal` だけでは何も実行しない。

---

## 5. ネイティブ UI

### 5.1 画面構成

```text
+------------------------------------------------------------------+
| GPUI Task Manager   host · kernel        [一時停止][詳細][ヘルプ] |
+------------------------------------------------------------------+
| [CPU card] [メモリ card] [ネットワーク card] [システム card]      |
+------------------------------------------------------------------+
| [検索ボックス][クリア] 並び: CPU ▼ [0.5s][1s][2s][5s]  表示/全件 |
+--------------------------------------------------+---------------+
| 名前            CPU    メモリ   PID  ユーザー 状態 | プロセス詳細   |
| ● firefox ...  18.4%  1.9 GiB   800  user  実行中 | 名前 / PID    |
| ● code ...      9.7%  1.2 GiB   810  user  待機   | 状態 / ユーザー|
| ...                                              | シグナル操作   |
+--------------------------------------------------+---------------+
| プラットフォーム | 更新間隔 | 最終更新 | メッセージ / キー案内     |
+------------------------------------------------------------------+
```

### 5.2 描画の効率

- プロセス一覧は `uniform_list`（仮想スクロール）。数千行でも表示中の行だけを構築する。
- `render` ではスナップショット全体を clone しない。概要カード用の小さな値だけを
  `Summary` にコピーし、行は遅延構築する。
- 概要のスパークラインは gpui の flex 要素で描く（追加のチャート依存なし）。
- サンプリングは `cx.background_spawn` で実行し、UI スレッドをブロックしない。
- 同時サンプルは `sampling_in_flight` で 1 本に制限する。

### 5.3 操作

| キー | 動作 |
|---|---|
| ↑ / ↓ | 選択行の移動（スクロール追従） |
| PgUp / PgDn | 10 行移動 |
| Home / End | 先頭 / 末尾 |
| Space | 更新の一時停止・再開 |
| Ctrl+F | 絞り込み入力（文字入力・Backspace・Esc で解除） |
| Ctrl+K | 選択プロセスの終了確認 |
| F2 | 詳細パネルの表示切替 |
| F5 | 今すぐ更新 |
| F1 | ヘルプ |
| Esc | ダイアログ / 絞り込みの解除 |

マウスでは行クリックで選択、列ヘッダでソート、詳細パネルのボタンでシグナル操作。

ブラウザ版・ネイティブ版とも絞り込みは日本語 IME（composition）に対応する。
`gpui-pre-web` の IME ミラー（隠し textarea）と、ネイティブの
`window.handle_input` + `EntityInputHandler` を使い、変換中の未確定文字列は入力欄に
だけ表示し、確定文字列だけを `Dashboard` に反映する（`notes/ime.md`）。

### 5.4 表示の安全性

- プロセス名・コマンドは OS から来る攻撃者影響下のデータである。改行・制御文字・
  ANSI エスケープ・bidi／ゼロ幅の不可視書式文字を `tm-core::sanitize` で除去し、文字数で切り詰める。
- ユーザー名やコマンドは `.truncate()` でレイアウト崩れを防ぐ。
- ダイアログは「取り消せません」を明示する。**安全側の既定**として、Enter は
  キャンセル、Ctrl+Enter または「実行」ボタンで確定する（誤操作でプロセスを
  失わないため）。対象の取り違えを防ぐため、PID・ユーザー・起動からの経過・
  コマンドを表示する。フォーカス移動によるキーボード完結は将来の改善項目。

---

## 6. wasm / ブラウザ戦略

### 6.1 制約

ブラウザからは `/proc` を読めない。したがって「ブラウザで本物のプロセス一覧を
読む」ことは原理的に不可能で、次のどちらかが必要になる。

1. ネイティブ側のエージェントから HTTP で取得する（ライブ監視）。
2. 記録済みデータ / 合成データを再生する（デモ・回帰テスト）。

### 6.2 構成

- ビューモデルとコアは wasm32 でそのまま動く（`std` のみ、`serde` 任意）。
- UI は GPUI Web バックエンド（`gpui-pre` + `gpui-pre-platform` を
  `default-features = false`）で構築する。`gpui-kit` は stable では
  `wasm_thread` を要求するため wasm シェルには使わない。
- データは `DemoProbe`（合成）または `tm-agent`（ループバック HTTP）から取得する。
- ブラウザは**読み取り専用**。シグナル操作ボタンは無効化し、`SystemProbe::control`
  は `Unsupported` を返す。

### 6.3 エージェント

- ループバックアドレスのみ bind（既定 127.0.0.1、IPv4/IPv6 の loopback を許可）。認証は `Authorization: Bearer` のみ受け付ける
  （クエリ文字列のトークンは URL 漏洩対策として拒否する）。
- トークンは起動時に `/dev/urandom` から生成して stderr に 1 回表示する。
  `--token-file PATH` または `TM_AGENT_TOKEN` で外部から渡せる（ファイル推奨）。
  `--token` はテスト専用で、argv が `/proc/<pid>/cmdline` から読まれ得るため警告を出す。
- エンドポイントは `GET /healthz` と `GET /api/v1/snapshot` のみ。
- プロセス操作 API を持たない（読み取り専用）。
- `Host` ヘッダが loopback（`127.0.0.1` / `localhost` / `[::1]`）以外なら 403。
  DNS リバインディングの多層防御。
- CORS は `http://127.0.0.1[:port]` / `http://localhost[:port]` のみ許可
  （ポートなしも許可する）。
- リクエストヘッダ上限 8 KiB、1 接続 1 リクエスト、読み取りタイムアウト 1 秒。
- スキーマは `{"schema_version": 1, "snapshot": {...}}`。`WIRE_SCHEMA_VERSION` で
  互換性を管理する。
- プロセスは CPU 降順 → PID 昇順でソートし、`?limit=N`（既定 500、最大 2000）で
  先頭 N 件に絞る。`total_process_count` を併せて返す。実測では 618 プロセスの
  全件 JSON が約 173 KB、既定 500 件で約 137 KB であり、大規模環境や将来の
  フィールド追加でも 1 秒間隔の全件送信を避けるためにページングを維持する。
  （調査時の 2.24 MB はスレッド込み 3,549 エントリ・全項目の旧測定値。）

### 6.4 実証結果（2026-11-14）

- `gpui-pre 0.3.12` + `gpui-pre-platform`（`default-features = false`）で
  stable の wasm32-unknown-unknown にコンパイル成功。
- `trunk build --release` 成功。ヘッドレス Chrome 157 で GPUI が描画し、
  テキスト・表・スクロール・クリック・キーボード入力・1 秒定期更新・HTTP fetch・
  CJK 表示を確認。
- ヘッドレス環境では WebGPU アダプタが取れず **WebGL2（SwiftShader）に
  フォールバック**する。実ブラウザでは WebGPU が使える可能性があるが未確認。
- CJK は `CanvasFontFallback::EmojiAndCjk` が必須。`gpui_platform::application()`
  では設定できないため、`gpui_web::WebPlatform` を直接構築して
  `fetch_http_client` を注入する。
- wasm サイズは wasm-opt（-Oz）適用後 **7.29 MiB（gzip 2.90 MiB、2026-11-17 計測）**。
  `web/index.html` の `data-wasm-opt="z"` で trunk が wasm-opt を自動適用する。

### 6.5 ブラウザ版シェルの実装結果（`web/`）

- `web/` は独立 crate（親 workspace 外）。`tm-core` / `tm-viewmodel` を path 依存で共有。
- `cargo check --target wasm32-unknown-unknown --manifest-path web/Cargo.toml` と
  `trunk build --release` が成功。
- ヘッドレス Chrome 157 + CDP で、デモ / ライブ（`tm-agent --redact`）/
  エージェント停止時の 3 モードを検証。描画・1 秒更新・スクロール・行クリック・
  絞り込み・ソート・エラー表示を確認し、panic / JS 例外は 0。
- ライブ接続は URL フラグメント `#token=` + `Authorization: Bearer` ヘッダ方式。
  クエリ文字列のトークンは使用しない。
- 日本語 IME: `EntityInputHandler` + composition を実装。CDP の
  `Input.imeSetComposition` / `Input.insertText` で「にほんご」→「日本語」の
  変換・確定・Esc 復帰を 32 チェックで検証（JS 例外 0）。`FilterInput` のテストは
  `tm-viewmodel` に移設（13 件、2026-11-17 時点）、`web/` の単体テストは 12 件。
- wasm-opt（-Oz）適用後もデモ / ライブ / エラー時の CDP 検証は pass。
- 証拠: `notes/web-shell.md`、`notes/ime.md`、`notes/assets/web-shell-*.png`、
  `notes/assets/ime-*.png`、`web/verification/*.json`。

---

## 7. セキュリティ設計

### 7.1 資産と信頼境界

| 資産 | 信頼境界 | 脅威 |
|---|---|---|
| プロセス情報 | カーネル → アプリ | 悪意あるプロセス名・コマンドによる表示破壊 |
| シグナル送信 | アプリ → 他プロセス | 誤操作・PID 再利用による誤殺 |
| ブラウザ UI | エージェント → ブラウザ | トークン漏洩・CORS 誤設定・DNS リバインディング |
| 依存クレート | crates.io → ビルド | 供給網攻撃・未保守クレート |

### 7.2 対策

1. **unsafe 禁止**: 全 crate で `#![forbid(unsafe_code)]`。シグナル送信は rustix の
   安全 API を使い、`libc` を直接呼ばない。
2. **最小権限**: setuid しない。root 権限を要求しない。EPERM は明確な日本語
   メッセージにする。
3. **誤操作防止**: シグナルは必ず確認ダイアログを経由。PID 1 / 自分自身を拒否。
   PID + 起動時刻の一致を送信直前に再確認。
4. **読み取り専用ブラウザ**: エージェントに操作 API を置かない。UI 側でも
   `supports_process_control = false` としてボタンを無効化する。
5. **ループバック限定**: エージェントは `127.0.0.1` にのみ bind。`Authorization:
   Bearer` 必須、`Host` ヘッダも loopback に限定。CORS は `http://127.0.0.1[:port]` / `http://localhost[:port]` のみ。
6. **表示サニタイズ**: プロセス名・コマンドライン・コンポーネントラベル等の表示文字列から制御文字・ANSI エスケープ・bidi／ゼロ幅の不可視書式文字を除去し、文字数を制限する。
7. **供給網**: GPUI 系は完全一致ピン + `Cargo.lock` コミット。`cargo audit` に加え
   `cargo-deny`（advisories / bans / licenses / sources）をルート / web の両方で
   実行し、CI で継続検査する。未保守（unmaintained）等の除外は理由付きで `deny.toml` に
   記録し、DESIGN §14 で追跡する。Dependabot（Cargo / web / GitHub Actions、週次）と
   セキュリティアラートを有効化している。GPUI 系は自動更新の対象外（ADR-001 の完全ピン
   運用）。CycloneDX SBOM を `scripts/generate-sbom.sh` と CI `sbom` ジョブで生成する。
   `cargo-vet` は root / web とも初期化済み（全依存を `exempt`＝未監査の明示として記録、
   外部 imports 未使用）。trust root は自リポジトリのみとし、実レビューできた依存から
   `audits.toml` に記録する（`notes/cargo-vet-adoption.md`）。監査記録の捏造は行わない。
   CI は共通 `rust-gates.yml@v1` の `extra_gates` で `cargo vet --locked` を必須化。
8. **秘密情報を扱わない**: 設定ファイル・認証情報・DB を持たない。トークンは
   起動時に生成し、stderr にのみ表示する。

### 7.3 残余リスク

- GPUI 系は個人アカウントが crates.io に再公開したスナップショットであり、
  bus factor = 1。完全ピンで更新を統制するが、upstream のセキュリティ修正を
  自動では取り込めない。
- トークンはプロセス生存中は固定で、再起動すると無効になる（TTL・ローテーションは
  未実装）。ブラウザでは URL フラグメントから読み取って `Authorization` ヘッダで
  送るため、履歴・Referer には残らないが、画面共有やスクリーンショットには写り得る。
  ループバック限定・読み取り専用 API・短い読み取りタイムアウトで影響を限定する。
- PID の `start_time` は秒精度であり、同一秒内の PID 再利用や再取得から
  `kill_process` までの TOCTOU は完全には排除できない。PID + 起動時刻の照合で
  リスクを大幅に下げるが、`pidfd` 系 API による原子化は将来の課題。
- プロセス名の Unicode 双方向制御文字（RTL override 等）とゼロ幅・不可視書式文字は `tm-core::sanitize` で
  除去する（U+202A–U+202E, U+2066–U+2069, U+200E/F, U+061C）。ゼロ幅文字など
  表示に影響する他の書式文字は将来の検討対象。
- ネイティブ版 IME は実装済みだが、実 IME（IBus/fcitx5）での手動確認は未実施
  （`TM_DEBUG_IME=1` のデバッグフックによる状態遷移の検証のみ）。
- Windows のシグナル送信は `%SystemRoot%\System32\taskkill.exe` の絶対パス起動に
  限定し PATH ハイジャックを避けるが、Windows では Stop/Continue 非対応で、
  Terminate/Kill は強制終了になる。macOS の実機検証は CI ランナーのみ。

---

## 8. テスト戦略

### 8.1 ピラミッド

| 層 | 内容 | 実行 |
|---|---|---|
| 単体（純粋） | 型の不変条件、CPU 差分、レート、リングバッファ、整形、サニタイズ、ソート/フィルタ | `cargo test -p tm-core` |
| 状態機械 | Tick/Pause、ソート、フィルタ、選択追従、確認フロー、保護対象、PID 消滅 | `cargo test -p tm-viewmodel` |
| プラットフォーム | 実機サンプルの妥当性、存在しない PID、PID 1 / 自分自身、spawn した子の終了 | `cargo test -p tm-platform` |
| エージェント | JSON 応答、認証、CORS、読み取り専用 | `cargo test -p tm-agent` + curl |
| UI スモーク | Wayland 起動、ウィンドウ確認、スクリーンショット | 手動 + KWin D-Bus |
| wasm | `cargo check --target wasm32-unknown-unknown`、`trunk build`、ブラウザ表示 | スパイク + CDP |
| 入力状態 | `FilterInput` の状態機械（UTF-16・選択・marked range、Web/ネイティブ共有） | `cargo test -p tm-viewmodel`（FilterInput 13 件、viewmodel 全体 37 件） |
| IME（Web） | composition 中は確定しない・確定後に反映・Esc 復帰 | CDP `Input.imeSetComposition`（32 チェック） |
| IME（native） | `EntityInputHandler` 経由の変換中表示と確定反映 | `TM_DEBUG_IME=1` + スクリーンショット |
| クロスプラットフォーム | Windows/macOS ターゲットの check、CI ランナー | `cargo check --target ...` / GitHub Actions |

### 8.2 決定性

- `DemoProbe` は tick の純関数。同じ tick は同じスナップショットになる。
- ソートは PID 昇順のタイブレークで常に決定的。
- 時刻に依存するテストは書かない（`format_clock` は固定入力でテスト）。

### 8.3 手動・実機検証

- Wayland セッションで起動し、KWin D-Bus でウィンドウタイトル / app_id を確認する。
- スクリーンショットを取得して、概要カード・表・詳細パネル・ダイアログを確認する。
- 実プロセスの終了は、自分で spawn した `sleep` に対してのみ自動テストで行う。

---

## 9. 性能設計

| 項目 | 目標 | 手段 |
|---|---|---|
| サンプリング周期 | 250 ms 〜 10 s、既定 1 s | viewmodel で clamp |
| サンプル所要時間 | 1 秒周期に対して十分小さい（実測値を記録） | sysinfo の差分更新、プロセス全件 1 回 |
| 履歴メモリ | 120 サンプル × 4 系列で固定 | `RingBuffer` |
| 描画 | 表示行のみ構築 | `uniform_list`、`Summary` の小さなコピー |
| 起動 | 数秒以内 | 依存の debuginfo 削減、遅延初期化なし |

実測値（2026-11-14、本機）:

| 項目 | 実測 |
|---|---|
| sysinfo full tick（スレッド込み 3,549 エントリ） | 中央値 31.4 ms |
| sysinfo cold start | 85 ms |
| ワイヤ JSON（618 プロセス、limit=2000） | 約 173 KB（旧測定: スレッド込み 3,549 エントリで 2.24 MB） |
| エージェント 1 スナップショット | 約 250 ms（共有プローブで直列化、接続は最大 8 並列） |
| ネイティブ debug バイナリ | 166 MB（debuginfo 削減済み） |
| wasm（release、wasm-opt -Oz） | 7.29 MiB / gzip 2.90 MiB（2026-11-17 計測） |

---

## 10. 拡張性

- **プラットフォーム追加**: `SystemProbe` を実装し `app` の factory に追加する。
  コア・ビューモデル・UI は変更しない。
- **ビュー追加**: `Command` / `Effect` を追加し reducer を拡張する。UI は新しい
  状態を描画するだけ。
- **リモート監視**: `SystemProbe` の HTTP 実装（エージェントのクライアント側）を
  追加すれば、ブラウザと同じデータ経路をネイティブでも使える。
- **エージェント API**: `WIRE_SCHEMA_VERSION` で版管理。Glances / node_exporter
  互換アダプタは「別のプローブ実装」として追加できる。
- **テーマ**: `theme.rs` のパレット（Dark = Catppuccin Mocha / Light = Catppuccin Latte）を
  実行時に切り替える。ネイティブ版・ブラウザ版ともヘッダーの切替ボタンと
  `Settings.theme` の永続化まで実装済み（ブラウザ版は `localStorage`）。

---

## 11. ビルドと配布

### 11.1 Linux（第一ターゲット）

```bash
bash scripts/dev-setup.sh        # libxkbcommon-x11 のローカル shim（-dev が無い場合）
cargo build -p gpui-task-manager
cargo test --workspace
./target/debug/gpui-task-manager          # 実データ
./target/debug/gpui-task-manager --demo   # 合成データ
```

必要パッケージ（Ubuntu 系）: `gcc g++ clang libfontconfig-dev libwayland-dev
libxkbcommon-x11-dev libx11-xcb-dev libssl-dev libzstd-dev libvulkan1` と
Vulkan 対応 GPU ドライバ。

### 11.2 wasm / ブラウザ

```bash
cargo check --target wasm32-unknown-unknown -p tm-core -p tm-viewmodel
cd web && trunk build --release && trunk serve   # ブラウザ版シェル
cd poc-wasm && trunk build --release             # 最小スパイク
```

ブラウザ版は既定でデモデータを表示する。ライブ表示は
`http://127.0.0.1:8091/?agent=http://127.0.0.1:18787#token=<token>` のように
URL フラグメントでトークンを渡し、`fetch` の `Authorization: Bearer` ヘッダで
エージェントに接続する（フラグメントはサーバ・Referer に送られない）。

### 11.3 CI

- 共通ワークフロー（`Seduce0068/workflows/.github/workflows/rust-gates.yml@v1`）で
  fmt → check → test → clippy（`-D warnings`）等の標準ゲートを実行する。
- 追加ジョブ: wasm check、cargo-deny（ルート / web）、native-windows
  （`windows-latest`）、native-macos（`macos-latest`）、sbom（CycloneDX）。
- Actions はバージョンタグで参照し、Dependabot（GitHub Actions 週次）で更新を追従する。必要に応じて commit SHA ピン留めへの移行を検討する。
  `ubuntu-latest` は使用しない。
- GPUI の X11 依存は CI でも `libxkbcommon-x11-dev` を導入する。
- GitHub ラベルで領域・運用課題を分類する。public 化に伴い branch protection を
  有効化し、必須 CI とレビュー運用でゲートする（公開前の `403` は過去の記録）。

---

## 12. 受入条件

| ID | 条件 | 検証 |
|---|---|---|
| AC-1 | `cargo test --workspace` が全件成功する | CI / ローカル |
| AC-2 | `cargo clippy --workspace --all-targets -- -D warnings` が成功する | CI / ローカル |
| AC-3 | Wayland でネイティブウィンドウが起動し、プロセス一覧が表示される | KWin D-Bus + スクリーンショット |
| AC-4 | プロセス行クリックで詳細パネルが更新される | 手動 + スクリーンショット |
| AC-5 | 終了操作が確認ダイアログを経由し、キャンセルで何も起きない | 単体テスト + 手動 |
| AC-6 | PID 1 / 自分自身の操作が拒否される | 単体テスト |
| AC-7 | フィルタ・ソート・選択がサンプル更新後も維持される | 単体テスト |
| AC-8 | 表示文字列のサニタイズ（制御文字・ANSI・長大文字列）が効く | 単体テスト |
| AC-9 | コア・ビューモデルが wasm32 にコンパイルできる | `cargo check --target` |
| AC-10 | ブラウザで GPUI 画面が表示される（スパイク） | ヘッドレス Chrome |
| AC-11 | エージェントが読み取り専用で、トークンなしの要求を拒否する | curl |
| AC-12 | 設計書・調査レポート・README が実装と一致する | レビュー |
| AC-13 | ブラウザ版 / ネイティブ版で日本語 IME の変換中は絞り込まず、確定後に反映される | CDP composition 検証 + `TM_DEBUG_IME=1` |
| AC-14 | Windows / macOS ターゲットでワークスペースが型検査できる | `cargo check --target` + CI |
| AC-15 | cargo-deny がルート / web とも成功する | `cargo deny check` |
| AC-16 | 表示設定が保存され、再起動後に復元される（ネイティブ / ブラウザ） | 実機 + CDP（`cdp_verify_settings.py`）
| AC-17 | root 権限なしで DEB を生成でき、展開バイナリが起動する | `build-deb.sh` + `dpkg-deb -x` |

---

## 13. ADR（Architecture Decision Records）

### ADR-001: GPUI スタックに `gpui-kit 0.9.1` を採用

- 状況: 公式 `gpui` crate は 2025-10 以降更新が止まっている。Zed 本体の GPUI は
  crates.io に公開されていない。
- 決定: `gpui-kit = "=0.9.1"`（内部で `gpui-pre =0.3.12`、Zed GPUI スナップショット）
  を完全一致で固定し、`Cargo.lock` をコミットする。
- 理由: 単一依存で版が揃う、DataTable/VirtualList/Theme などの部品が揃う、
  Apache-2.0、商用アプリでの実績。
- 代替: 公式 `gpui 0.2.2`（古い）、Zed への git 依存（GPL 感染）、`gpui-ce`
  （API 互換性が流動的）。
- 結果: 週次の破壊的変更リスクを受け入れる。更新は隔週でまとめて行う。

### ADR-002: core / viewmodel / shell の 3 層分離

- 決定: `tm-core`（純粋）→ `tm-viewmodel`（reducer）→ shell（GPUI）の一方向依存。
- 理由: ブラウザとネイティブで表示ロジックを共有し、UI なしでテストできる。
- 結果: shell にロジックを置かない規律が必要。コード量は少し増える。

### ADR-003: ネイティブ収集は sysinfo、シグナルは rustix

- 決定: プロセス・CPU・メモリ・ネットワーク・ディスクの収集に `sysinfo 0.40.3`、
  シグナル送信に `rustix 1.1.6`（`process` feature）を使う。
- 理由: sysinfo はクロスプラットフォームで活発。rustix は unsafe なしで
  `kill_process` を提供する。
- 代替: `procfs` 直接パース（Linux 専用・移植性低下）、`nix`（unsafe を含む）、
  `libc`（禁止）。
- 結果: 最初のサンプル CPU% が 0 になるなど sysinfo の仕様を UI で許容する。

### ADR-004: wasm は「シェル分離 + 読み取り専用データ経路」

- 決定: コアとビューモデルを共有し、wasm シェルは `gpui-pre-web` で構築する。
  データはデモまたはエージェント経由の読み取り専用。
- 理由: ブラウザから OS プロセスは読めない。操作も安全側に倒す。
- 結果: wasm シェルはネイティブと別 crate になる（`gpui-kit` の stable wasm 非対応）。

### ADR-005: ブラウザのライブデータはループバック HTTP エージェント

- 決定: `tm-agent` が `127.0.0.1` で読み取り専用 JSON を返す。トークン必須、
  CORS は `http://127.0.0.1[:port]` / `http://localhost[:port]` のみ。
- 代替: WebSocket（双方向は不要）、Glances 互換 API（将来のアダプタ）、
  WebRTC（過剰）。
- 結果: ブラウザとエージェントが同一マシンであることが前提。
- 追加: 実測 618 プロセスで約 173 KB（既定 500 件で約 137 KB）。大規模環境でも
  1 秒間隔の全件送信を避けるため、`?limit=N`（既定 500、最大 2000）でページングし、
  `total_process_count` を返す。

### ADR-006: unsafe 禁止

- 決定: 全 crate で `#![forbid(unsafe_code)]`。
- 理由: セキュリティ・保守性。FFI が必要になった場合は ADR を追加してレビューする。
- 結果: rustix など安全 API の crate を選ぶ必要がある。

### ADR-007: PID 再利用防御

- 決定: `ProcessKey = (pid, start_time)`、送信直前の再確認、PID 1 / 自分自身の拒否、
  確認ダイアログ。
- 結果: 誤殺のリスクを大幅に下げる。start_time を提供しないプラットフォームでは
  防御が弱くなるため、その旨を `PlatformInfo` で表現する（将来）。

### ADR-008: 固定長履歴と仮想リスト

- 決定: 履歴は `RingBuffer`（120 サンプル）、プロセス一覧は `uniform_list`。
- 理由: 長時間稼働でもメモリが増えない。数千行でも描画が軽い。
- 結果: 履歴の永続化・エクスポートは MVP 外。

### ADR-009: 依存最小化

- 決定: `tm-core` の依存は任意の `serde` のみ。UI は `gpui-kit` のみ。wasm 側は
  `wasm-bindgen` / `web-sys` を必要最小限にする。
- 結果: ビルド時間と供給網リスクを抑える。チャートは自前の flex 描画。

### ADR-010: テストは純粋層に集中

- 決定: core / viewmodel に単体テストを厚く置き、platform は実機スモーク、
  UI は起動検証 + スクリーンショット、wasm はビルド + ブラウザ表示で確認する。
- 結果: UI の自動 E2E は MVP 外（Wayland への合成入力が難しいため）。

### ADR-011: ライセンスは Apache-2.0

- 決定: 本プロジェクトのコードは Apache-2.0。GPUI 系（Apache-2.0）と互換。
- 理由: 特許許諾があり、エコシステムと整合する。AGPL の既存試作からコードは
  コピーしていない（調査ノートのみ参照）。
- 結果: 既存の AGPL プロジェクトへ統合する場合は別途ライセンス判断が必要。

### ADR-012: 設定は表示設定のみを XDG に永続化（改訂）

- 決定: **更新間隔・ソート列/方向・詳細パネル表示**を保存し、起動時に復元する。
  スキーマは `tm-viewmodel::Settings` に置き、ネイティブとブラウザで共有する
  （保存媒体だけが違う: ネイティブは
  `$XDG_CONFIG_HOME/gpui-task-manager/settings.json`、Windows は `%APPDATA%`、
  macOS は `~/Library/Application Support`、ブラウザは `localStorage`）。
- 保存対象を表示設定に限定する理由: プロセス情報・フィルタ・トークン・ホスト名などの
  機微情報や一時状態はディスクに残さない。フィルタは「その時の作業」であり、
  起動時に勝手に絞り込まれると対象を見失うため保存しない。
- 実装: 原子的書き込み（一時ファイル + rename）、Unix では `0600`、`version` で
  スキーマ版を保持し、未知フィールドは無視する（前方互換）。値は view-model の
  範囲へクランプする。
- 検証: `TM_DEBUG_SETTINGS=1` のデバッグフックで保存経路を、設定ファイルを置いた
  起動で復元経路を実機確認（`notes/assets/settings-restore.png`）。
  ブラウザは `web/scripts/cdp_verify_settings.py` で保存（列クリック）と復元
  （再読込後のソート）を CDP 検証（`web/verification/web-settings-evidence.json`）。

### ADR-013: ブラウザ UI は gpui 素の div で構成

- 状況: `gpui-kit` / `gpui-component` は stable wasm で `wasm_thread`（nightly 必須）
  を要求するため使えない。
- 決定: ブラウザシェルは `gpui-pre` + `gpui-pre-web` の素の要素で構成し、
  `WebPlatform` を直接構築して CJK フォールバックと HTTP クライアントを設定する。
- 結果: ネイティブ版と見た目を揃えるため、テーマトークンは小さく重複させる。

### ADR-014: 収集は UI スレッド外・1 本に制限

- 決定: `SystemProbe` を `Arc<Mutex<...>>` に置き、`cx.background_spawn` で
  サンプリングする。`sampling_in_flight` で多重実行を防ぐ。
- 理由: sysinfo の tick は 31 ms（本機）で、UI スレッドで実行すると描画が乱れる。
- 結果: サンプル中の操作は次のティックまで待つが、UI は応答し続ける。

### ADR-015: エージェントのトークン入力はファイル / 環境変数を優先

- 状況: `--token` は argv に残り、同一マシンの他プロセスから `/proc/<pid>/cmdline`
  で読まれ得る。起動時生成は安全だが、自動化では受け渡しが必要になる。
- 決定: `--token-file PATH` と `TM_AGENT_TOKEN` を用意し、`--token` はテスト用途に
  限定して警告を出す。トークンファイルは空を拒否し、末尾改行を除去する。
- 結果: 常用時のトークンがプロセス一覧に現れない。ファイルのパーミッション管理は
  運用者の責任であり、SECURITY.md に `chmod 600` を記載する。

### ADR-016: 依存監査・SBOM・自動更新

- 決定: `deny.toml` でライセンス・脆弱性・重複版・供給元を定義し、CI の `deny` ジョブで
  ルート / web の両ワークスペースを検査する。Dependabot は Cargo（ルート / web）と
  GitHub Actions を週次で更新し、GPUI 系は完全ピン運用のため対象外（ignore）にする。
  CycloneDX SBOM をスクリプト/CI で生成する。`cargo-vet` は trust 方針・棚卸し・運用手順
  を定めてから段階導入し、監査記録を捏造しない。
- 結果: `cargo-deny`/Dependabot/SBOM を当面の実効的対策として運用する。未保守等の除外は
  理由付きで管理し、新規の脆弱性・ライセンス違反・不明レジストリは失敗させる。
  GPUI の更新は手動で行う。

### ADR-017: wasm-opt（-Oz）を trunk で適用

- 決定: `web/index.html` の `data-wasm-opt="z"` で wasm-opt を有効化する。
- 結果: wasm 14,799,786 B → 7,589,230 B（gzip 3,656,199 → 3,024,986 B）。
  適用後も CDP 検証（デモ / ライブ / エラー時）は pass。初回ビルドは wasm-opt を
  自動ダウンロードする。2026-11-15 の再計測では 7,635,554 B（gzip 3,042,856 B、
  依存更新やコード追加により微増）。2026-11-16 の再計測では 7,641,445 B
  （gzip 3,044,050 B、ブラウザ版テーマ追加による微増）。2026-11-17 の再計測では
  7,641,970 B（gzip 3,045,139 B、ツリーサイクル修正による微増）。

### ADR-018: Windows / macOS は cfg + CI ランナーで検証

- 状況: 開発ホストは Linux のみで、物理実機の Windows / macOS がない。
- 決定: `tm-platform` を OS 別 cfg（Unix: rustix / Windows: `taskkill` 絶対パス）に
  分岐し、GitHub Actions の windows-latest / macos-latest で `cargo check --workspace`
  と portable テストを実行する。Windows の Stop/Continue は非対応と明記する。
- 結果: 物理実機の GUI 操作・IME は未検証のまま残るが、型検査と単体テストは
  OS ごとに担保できる。

### ADR-019: ネイティブ IME は EntityInputHandler の最小実装

- 状況: `gpui-pre-linux` は Wayland text-input-v3 / X11 XIM を実装しているが、
  アプリ側の `EntityInputHandler` 登録がないと変換中文字列や複数文字確定が捨てられる。
  `gpui-component::Input` への置換は編集モードの意味論を変えるリスクが高い。
- 決定: `FilterInput` を `tm-viewmodel` に共有化し、`TaskManagerApp` に
  `EntityInputHandler` を実装してフィルタ欄の bounds に登録する。既存の
  Ctrl+F / Esc / Enter の意味論は維持し、文字入力だけをハンドラ経由にする。
  実 IME の無い環境向けに `TM_DEBUG_IME=1` のデバッグフックを用意する。
- 結果: ブラウザ版と同じ `FilterInput` を共有しつつ、ネイティブでも変換中表示と
  確定反映を実装。実 IME の候補ウィンドウ・予測変換は未検証。

### ADR-020: Linux 配布は root 不要の DEB 生成スクリプト

- 決定: `packaging/linux/build-deb.sh` が release バイナリから `.deb` を生成する
  （`dpkg-deb --root-owner-group`、`sudo` を一切呼ばない、ホストへインストールしない）。
- 依存関係は `ldd` + `dpkg -S` から自動計算し、Vulkan ローダは `dlopen` のため
  `libvulkan1` を明示追加する。
- 収録物: バイナリ、`.desktop`（`StartupWMClass=gpui-task-manager`）、
  hicolor アイコン（scalable + 256x256）、`README.md` / `NOTICE.md` / `copyright`。
- 未実施: 署名、リポジトリ公開、AppImage/Flatpak、Windows/macOS パッケージ。

---

## 14. 既知の制限・未検証

- Windows / macOS の**物理実機**でのビルド・実行は未検証。OS 別 cfg と CI ランナーで
  型検査・テストを実施する（Windows は Stop/Continue 非対応、Terminate/Kill は強制終了）。
- ネイティブ版 IME は実装済みだが、実 IME（IBus/fcitx5）での手動確認は未実施。
- Wayland 以外（X11 セッション）での表示は未検証。GPUI の x11 feature は有効。
- プロセスツリーはツリー表示トグル・階層インデント・ノード単位の折りたたみ/展開（`tm-core` のフラット化 + `tm-viewmodel` の折りたたみ状態、native/web で親ノードクリック時に選択+トグル）と `tree_view` 設定の永続化を実装済み（`parent_pid` ベース、表示対象内でのフラット化）。折りたたみ状態の永続化、優先度変更、cgroup 表示、GPU 利用率の詳細監視は未実装。
- 温度センサー等の component 収集・データモデルは実装済みで、ネイティブ/ブラウザの概要カードに取得可能な範囲を表示する（取得不可時は「センサーなし」）。詳細 GPU 利用率・ファン回転数等の網羅的監視は未実装。
- ライトテーマは**ネイティブ版・ブラウザ版とも実装済み**（切替ボタン + `Settings.theme` の
  永続化、`SETTINGS_VERSION = 2`）。ブラウザ版のパレットは `web/src/theme.rs` にあり、
  `prefers-color-scheme` 連動などの自動更新は未実装。設定スキーマは
  `tm-viewmodel::Settings` で共有し（更新間隔・ソート・詳細パネル・ツリー表示・テーマ）、
  ネイティブ版はファイル、ブラウザ版は `localStorage` に永続化する。ブラウザ版の
  状態フックは `theme=dark|light` を公開し、CDP で切替・復元を検証する。
- GPUI 系の完全ピンは更新を止めるため、定期的な見直しが必要。
- ブラウザ版の wasm は 7.29 MiB（gzip 2.90 MiB、wasm-opt -Oz 適用済み、2026-11-17 計測）。
- ブラウザ版は単一ウィンドウのみ（ポップアップ / ダイアログ不可）。CJK は
  Canvas フォールバック描画のため、ネイティブのシェーピングより品質が劣る。
- ヘッドレス環境では WebGPU が使えず WebGL2 フォールバックになる。実 GPU での
  WebGPU 描画は未確認。
- エージェントのサンプリングは共有プローブで直列化（1 スナップショット約 250 ms）。
  接続は最大 8 本までスレッドで並列処理し、超過分は即座に 503 を返す。
  アイドル接続が読み取りタイムアウトを待っても他のリクエストをブロックしない。
- `cargo-deny` は導入済み（`deny.toml`、ルート / web の CI ジョブ）。SBOM 生成は
  `scripts/generate-sbom.sh` と CI `sbom` ジョブで運用中。`cargo-vet` は初期化済み
  （root / web の `supply-chain/`、全依存 exempt ベースライン、imports 未使用）で、
  `notes/cargo-vet-adoption.md` に運用方針・imports 候補の一次調査がある
  （監査記録の捏造は行わない）。branch protection は public 化後に有効化済み。
- ネットワーク集計は `lo` を除外する。docker0 / tailscale0 などの仮想 IF は含む。
- ディスクは `total_space() == 0` の擬似マウントを除外する。
- スレッド数が取得できないプロセスは「—」と表示する。
- `tm-agent --once` は既定で先頭 500 件のみを出力する（全件は HTTP の `limit` 指定）。
- `--redact` はホスト名・ユーザー名・ホームパス・コマンド引数内のホスト名 / tailnet ドメインを
  置換するが、プロセス名に含まれるゼロ幅文字・同形異義文字までは除去しない。

---

## 15. 参照

- GPUI: <https://github.com/zed-industries/zed/tree/main/crates/gpui>
- gpui-kit: <https://github.com/longbridge/gpui-kit> / <https://gpui-kit.com>
- sysinfo: <https://github.com/GuillaumeGomez/sysinfo>
- rustix: <https://github.com/bytecodealliance/rustix>
- 既存調査ノート（同一ワークスペース、AGPL）:
  `work/gpui-file-explorer/notes/ecosystem-notes.md`,
  `work/gpui-file-explorer/notes/wasm-browser-notes.md`

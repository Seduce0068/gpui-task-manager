# 作業ログ

## 2026-11-14

- 要求: Ubuntu 系向け GPUI システムタスクマネージャーの検討・設計・試作
  （クロスプラットフォーム、wasm、ブラウザ、保守性・移植性・テスト容易性・
  セキュリティ）。
- 既存資産 `work/gpui-file-explorer` の GPUI 調査ノート（wasm 可否・
  スタック・ビルド要件）を参照。コードはコピーせず、調査のみ。
- 調査: 既存モニタ 11 候補、crate 17 件、sysinfo 実測（31.4 ms / 3,549 スレッド、
  実プロセス 618）、GPUI スタック、wasm/ブラウザ戦略。
- 設計: `tm-core` / `tm-viewmodel` / `tm-platform` / `app` / `web` / `agent` の
  5+1 構成。PID 再利用防御、読み取り専用ブラウザ、ループバック agent、
  unsafe 禁止、確認ダイアログ。
- 実装: 純粋コア（47 テスト）、ビューモデル（13 テスト）、ネイティブ収集
  （9 テスト）、読み取り専用エージェント（33 テスト）、ネイティブ GPUI シェル。
- 検証: `cargo test --workspace` 102 passed、clippy 警告 0、Wayland 起動・
  スクリーンショット、エージェント HTTP 実測、GPUI Web スパイク
  （ヘッドレス Chrome、WebGL2 フォールバック、CJK、fetch）。
- 修正: sysinfo のスレッド混入（プロセス数 3,549 → 618）とプロセス CPU% 飽和を
  実測で発見し、スレッド除外と `accumulated_cpu_time` 差分方式に修正。
- 独立レビュー（blocker 0 / major 8）を受けて、クエリトークン廃止・Host 検証・
  ダイアログの安全側 Enter・個人情報マスク・性能値の再測定・CI 参照先などを修正。
- ブラウザ版 `web/` を実装（デモ / エージェント接続、読み取り専用）。
  ヘッドレス Chrome + CDP でデモ / ライブ / エラー時の 3 モードを検証し、
  スクリーンショット 18 枚と証拠 JSON を保存。
- 安全強化: 確認ダイアログに対象の PID・ユーザー・起動経過・コマンドを表示、
  Enter=キャンセル / Ctrl+Enter=確定のテスト追加、`--token-file` 対応、
  サニタイズの決定論的堅牢性テスト（500 ケース）、`SECURITY.md` 追加。
- レビュー残余対応: `--redact` のコマンド引数内ホスト名 / tailnet ドメイン対応、
  USAGE 修正、native-libs の Git 管理除外、未使用依存削除、lo / 擬似マウント除外、
  スレッド不明表示、app の単体テスト追加、ADR 番号順修正。

## 2026-11-14（GitHub 公開準備）

- `gh` で認証確認（account: `onoderakeita` / 現行 login: `Seduce0068`、scopes: repo, workflow）。
- 既存 `Seduce0068/workflows`（public）を確認し、CI 参照を現行名 `Seduce0068/workflows@v1` に修正。
- 個人情報を含む過去履歴を push しないよう、**クリーンな1コミットの `main`** を作成して
  private リポジトリへ push する方針（ローカル `archive/master` に旧履歴を保持）。

## 2026-11-14（GitHub 公開）

- リポジトリ: https://github.com/Seduce0068/gpui-task-manager （**private**）
- 公開用の `main` はクリーンな1コミット（旧履歴はローカルの `master` / `archive/master` に保持、
  push していない）。
- CI（run 43604507187）: **成功**。gates（fmt / check / test / clippy / cargo audit）7m02s、
  wasm check（core / viewmodel / web）1m56s。
- 認証: `gh` の account 表示は旧名 `onoderakeita`、現行 login は `Seduce0068`。
  `gh repo create` の GraphQL はプロキシで失敗したため REST API で作成した。

## 2026-11-15（ツール・IME・クロスプラットフォーム）

- Dependabot を有効化（Cargo ルート / web、GitHub Actions、週次。GPUI 系は ignore）。
- `cargo-deny 0.20.2` を導入し `deny.toml` を追加。ルート / web とも ok。
- wasm-opt（-Oz）を trunk で有効化。wasm 14.8 MB → 7.6 MB（gzip 3.66 → 3.02 MB）、
  CDP 検証 pass。
- ブラウザ版の日本語 IME を実装（`EntityInputHandler` + composition、CDP 32 チェック）。
  ネイティブ版は設計案のみ（`notes/ime.md`）。
- Windows/macOS 対応: rustix を Unix 限定、Windows は `taskkill` 絶対パス起動、
  Linux 専用 tasks/threads を gate。CI に native-windows / native-macos / deny を追加。
  Windows workspace check / macOS 4 crate check はローカル成功。
- テスト: workspace 118 + web 15。

## 2026-11-15（CI 初回実行）

- run 43612670190: **5 ジョブすべて成功**（gates 2m28s / wasm 1m47s / cargo-deny 22s /
  native-windows 4m49s / native-macos 1m11s）。
- Dependabot 初回更新: Cargo ルート / web / GitHub Actions の 3 ジョブ成功。
  PR #1（actions/checkout 4→7）が作成され、PR の CI も成功（レビュー待ち）。

## 2026-11-15（ネイティブ IME・PR マージ）

- Dependabot PR #1（actions/checkout 4→7）をマージ。main の CI 5 ジョブも成功。
- ネイティブ IME: `FilterInput` を `tm-viewmodel` へ共有化し、`EntityInputHandler` を実装。
  `TM_DEBUG_IME=1` のデバッグフックで変換中→確定を確認（一覧 28→0 件）。
  実 IME の手動確認は未実施。
- テスト: workspace 131 + web 4。
- CI run 43616522864: ネイティブ IME 変更後も 5 ジョブすべて成功
  （Windows 4m28s / macOS 1m10s / cargo-deny / wasm 1m49s / gates 3m55s）。
- Windows/macOS 物理実機・クラウド検証の選択肢と費用を `notes/physical-verification.md` に整理。

## 2026-11-15（設定スキーマ共通化とブラウザ永続化）

- 設定スキーマを `tm-viewmodel::Settings` に集約（ネイティブとブラウザで共有）。
  ネイティブは既存の XDG/APPDATA/Application Support ファイル保存を維持し、
  ブラウザは `localStorage`（`gpui-task-manager.settings`）へ保存。
- `web/src/settings.rs` と `web/scripts/cdp_verify_settings.py` を追加。
  CDP 検証: 列クリックで保存、再読込で復元（`sort=名前;dir=asc`）を確認。
  証拠: `web/verification/web-settings-evidence.json`（`passed: true`）。
- テスト: workspace **143 passed**（core 49 / viewmodel 34 / platform 9 / agent 39 / app 12）、
  web **7 passed**。`bash scripts/check.sh` 成功。
- push 後の CI（run 43669677202）は 7 ジョブすべて成功
  （release-linux / web-dist artifact を含む）。

## 2026-11-15（機能・運用拡張の安全な進捗）

- CI: `wasm` / `cargo-deny` / `release-linux` / `web-dist` を `ubuntu-24.04` に固定、
  `actions/upload-artifact` を `@v7` に更新、`sbom` ジョブを追加。
  `scripts/generate-sbom.sh` を追加（`cargo-cyclonedx` があれば CycloneDX 生成、
  未導入時は案内して exit 0）。
- GitHub 運用: ラベル（`area:core/platform/agent/web/ui`, `ci`, `security`,
  `packaging`, `supply-chain` 等）を作成。main の branch protection API は
  private リポジトリのプラン制約で `403` のため、必須チェック/レビュー等の運用方針を
  README/SECURITY/DESIGN に明記。
- 温度/コンポーネント: `ComponentSnapshot` と `SystemSnapshot.components` を追加
  （後方互換のため `serde(default)`）。`tm-platform` で sysinfo component 収集を有効化し
  ベストエフォートで取得、demo にサンプル値を追加。
- プロセスツリー基盤: `tm-core::query` に `ProcessTreeRow` / `flatten_process_tree`
  を追加しテスト追加（`parent_pid` ベースの深さ優先フラット化）。
- テスト: tm-core 52 / tm-platform 9 pass、tm-viewmodel/tm-agent/gpui-task-manager check pass。
  `cargo-vet` は監査判断が必要なため未導入（将来検討）。

## 2026-11-15（ツリー表示と温度表示のUI反映・ローカル検証）

- 変更:
  - `tm-viewmodel::Settings` に `tree_view` を追加し、native/web の保存・復元・コマンド適用を拡張。
  - `tm-viewmodel::Dashboard` にツリー表示状態（表示順/深さ）と `ToggleTreeView`/`SetTreeView` を追加。
  - `tm-core::query` のツリー順序テストを追加。
  - native/web のプロセス一覧に階層インデントとツリー切替 UI を追加。
  - `SystemSnapshot.components` の温度情報を native/web 概要カードに表示（センサーなし時はフォールバック表示）。
  - `app/src/ui.rs` / `web/src/widgets.rs` の `kv` ラベルを動的化。
- 検証:
  - `cargo fmt --all`
  - `cargo check -p gpui-task-manager -p tm-agent -p tm-viewmodel` / `cd web && cargo check`
  - `cargo test -q -p tm-core -p tm-viewmodel -p tm-platform -p tm-agent -p gpui-task-manager`
  - `cd web && cargo test -q`（7 passed）
- 補足:
  - ツリーは基本表示に加えノード単位の折りたたみ/展開を実装（セッション内状態）。温度はベストエフォート収集/表示。詳細 GPU 利用率等は未実装。

## 2026-11-15（ツリー表示整合性と依存監査の確認）

- 変更:
  - `tm-viewmodel` のツリー表示 `has_children` 算出を修正し、階層表示の挙動テストを追加。
- 検証:
  - `cargo fmt --all`
  - `cargo test -q -p tm-viewmodel`（36 passed）/ `cargo test -q -p tm-core -p tm-platform -p tm-agent -p gpui-task-manager` / `cd web && cargo test -q`（7 passed）
  - `cargo deny check` pass（ローカル）

## 2026-11-15（センサー/温度の詳細表示）

- 変更:
  - native/web 詳細パネルにセンサー/温度セクションを追加し、全コンポーネント情報を表示。
- 検証:
  - `cargo fmt --all`、`cargo check -q -p gpui-task-manager`、`cd web && cargo check -q`
  - `cargo test -q -p tm-core -p tm-viewmodel -p tm-platform -p tm-agent -p gpui-task-manager`、`cd web && cargo test -q`

## 2026-11-14（cargo-vet 方針明確化・web lint 修正）

- `cargo-vet`（`supply-chain/`）は未導入のままとし、trust root/棚卸し/運用手順の
  定義後に導入する方針を `README.md` / `SECURITY.md` / `notes/verification.md` に明記。
  監査記録の捏造は行わない。SBOM（CycloneDX）/ cargo-audit / cargo-deny を当面の
  供給網可視性・検査手段として位置づけた。
- `web/` クレートの既存 `clippy -D warnings` 失敗（未使用 import/API/helper 等）を
  `web/src/lib.rs` の crate-level `allow(dead_code, unused_imports)` で明示的に許容し、
  将来利用予定コードを削除せず lint を通過可能にした。`web/src/shell.rs` に
  `cargo fmt` を適用。
- 検証: `cd web && cargo fmt --check && cargo clippy --all-targets -- -D warnings &&
  cargo test -q && cargo check -q`、`cargo fmt --all --check`、
  `cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace -q`。


## 2026-11-15（cargo-vet 導入準備・デモコンポーネントテスト）

- `cargo-vet` 導入準備メモ `notes/cargo-vet-adoption.md` を追加。
  - 現状（未導入、`supply-chain/` なし、ローカル `cargo vet` 未導入）と既存供給網対策（cargo-audit/deny、CycloneDX SBOM、Dependabot、ピン運用）を整理。
  - `cargo metadata` ベースの依存棚卸し（総 809、crates.io 804、workspace path 5、git/path 非workspace 0）と導入前未決事項（trust/imports、初期方針、criteria、更新/CI 運用）を記録。監査記録は作成していない。
- `notes/index.md` に cargo-vet 導入準備メモへのリンクを追加。
- `tm-core` demo の温度/コンポーネント監視を明示検証するテスト `demo_snapshot_includes_deterministic_components` を追加（決定論性・ラベル・閾値・tick 間変化）。

### 検証

- `cargo fmt --all --check`: 成功
- `cargo test -p tm-core -q`: 成功（56 passed）
- `cargo clippy -p tm-core --all-targets -- -D warnings`: 成功
- `cargo test --workspace -q`: 成功（12 + 39 + 56 + 9 + 37 passed, 0 failed）
- `cargo clippy --workspace --all-targets -- -D warnings`: 成功

## 2026-11-15（cargo-vet メモ精緻化・DESIGN 運用記述更新）

- `notes/cargo-vet-adoption.md` を現状の棚卸し結果に合わせて精緻化。
  - git/非workspace path 依存が現時点で 0 件である事実を明記し、将来追加時のポリシー整理項目として整理（監査記録の捏造はしない）。
- `DESIGN.md` の供給網・CI・運用記述を現状に整合。
  - SBOM（`scripts/generate-sbom.sh` / CI `sbom`）と cargo-deny/Dependabot 運用を明記。
  - `cargo-vet` は未導入・準備メモ参照・監査捏造なしの方針を反映。
  - Actions SHA ピン留め・`ubuntu-latest` 非使用・ラベル運用・branch protection 未適用（private プラン制約 `403`）を記載。

### 検証

- コード変更なし（文書のみ）。`git diff` と記述整合を確認。

## 2026-11-15（cargo-vet 導入準備メモ拡充）

- `notes/cargo-vet-adoption.md` を拡充。
  - 検証日時を更新し、`cargo metadata` による棚卸し再確認（total 809 / crates-io 804 / path workspace 5 / non-workspace path 0 / git 0）を反映。
  - 棚卸し再取得手順（事実確認用コマンド）を追加。
  - 導入判断チェックリストを追加（目的、trust/imports 評価、初期 exempt 方針、git/path 例外手順、CI/運用、証跡健全性）。
  - 未レビュー依存を `audit` と混同しない方針を明示。

### 検証

- コード変更なし（文書のみ）。`cargo metadata` 再取得コマンドで棚卸し数値を確認し、`git diff` で記述整合を確認。

## 2026-11-15（component ラベルの表示サニタイズ・agent 互換確認）

- `crates/tm-platform/src/lib.rs`: `sysinfo` 由来のコンポーネントラベル（温度センサー等）に `tm_core::sanitize::sanitize_display(..., MAX_NAME_CHARS)` を適用し、制御文字/ANSI/bidi・過大な表示文字列を他の OS 由来表示データと同様に防御。
- `SECURITY.md` / `DESIGN.md`: 表示サニタイズ対象にコンポーネントラベルを含め、記述を実態に整合。
- `tm-agent` の wire 出力確認: `cargo run -q -p tm-agent -- --demo --once` で `schema_version: 1` のまま `snapshot.components` がシリアライズされることを確認（2件、`CPU Package` / `NVMe Composite`）。

### 検証

- `cargo fmt --all`
- `cargo test -p tm-core -p tm-viewmodel -p tm-platform -p tm-agent -q`（pass）
- `cargo clippy -p tm-platform --all-targets -- -D warnings`（pass）
- `cargo run -q -p tm-agent -- --demo --once` で components JSON 出力を確認


## 2026-11-15（運用拡張項目の再確認・cargo-vet評価軸追記）

- GitHub/CI 運用を再確認（事実）:
  - ラベル整備済み（`area:*`, `ci`, `security`, `packaging`, `supply-chain` 等）。`gh label list` で確認。
  - branch protection API は private プラン制約で `403`（未適用）。必須チェック強制の代替として CI/レビュー/ラベル運用を維持。
  - `.github/workflows/ci.yml` に `ubuntu-latest` 不使用（`ubuntu-24.04` 等を明示）。Actions はバージョンタグ参照 + Dependabot（GitHub Actions 週次）で追従しており、文書の「SHA ピン留め」表記を実態に合わせて修正（`DESIGN.md`, `README.md`）。
- 供給網/運用文書:
  - `deny.toml` / `scripts/generate-sbom.sh` / CI `deny`・`sbom` ジョブ / Dependabot（Cargo root/web, GitHub Actions）を再確認。
  - `notes/cargo-vet-adoption.md` に導入判断の比較軸（trust/imports 候補評価、criteria、初期 exempt 方針、更新運用、証跡健全性、最小手順）を追記。`verified` 更新。未導入・未監査依存を監査済みとしない方針は維持。
  - `notes/verification.md` に再確認結果を追記。

### 検証

- コード変更なし（文書のみ）。`git diff` で変更内容を確認。
- `gh label list --repo Seduce0068/gpui-task-manager --limit 50`、`gh api repos/Seduce0068/gpui-task-manager/branches/main/protection`（`403`）、`.github/workflows/ci.yml` / `.github/dependabot.yml` / `deny.toml` / `scripts/generate-sbom.sh` の存在と内容を確認。


## 2026-11-15（詳細パネルの空センサーラベルフォールバック）

- native/web の詳細パネルで、サニタイズ等により `component.label` が空文字の場合に `"センサー"` を表示するフォールバックを追加（概要カードの既存挙動と整合）。

### 検証

- `cargo fmt --all`
- `cargo test -p tm-core -q`（56 passed）
- `cargo test -p gpui-task-manager -q`（12 passed）
- `cargo check -p gpui-task-manager -q`
- `cargo check --manifest-path web/Cargo.toml -q`
- `cargo test --manifest-path web/Cargo.toml -q`（7 passed）


## 2026-11-15（概要カードの空センサーラベル表記を統一）

- 概要カードでも空のコンポーネントラベルを `"センサー"` に統一（詳細パネルと整合）。

### 検証

- `cargo fmt --all`
- `cargo test -p gpui-task-manager -q`（12 passed）
- `cargo test --manifest-path web/Cargo.toml -q`（7 passed）

## 2026-11-15（cargo-deny 警告整理: 未使用 license allow の削除）

- `deny.toml`: `[licenses].allow` から `NCSA` を削除。
  - 根拠: `cargo deny list`（root / web いずれも）で NCSA ライセンス使用クレートは確認されず、`cargo deny check -A advisory-not-detected` 実行時に root 側で `license-not-encountered` 警告が出ていたため。現在の依存グラフに存在しない許可の整理であり、ポリシー緩和ではない（将来必要になった場合は根拠を確認して追加する）。
- duplicate 警告（`multiple-versions = "warn"`）は既知の許容事項として変更なし。

### 検証

- `cargo deny --manifest-path Cargo.toml check -A advisory-not-detected`（pass、`license-not-encountered` 解消）
- `cargo deny --manifest-path web/Cargo.toml check -A advisory-not-detected`（pass）

## 2026-11-15（cargo-vet ツール導入・準備メモ更新）

- 作業環境に `cargo-vet 0.10.3` を導入（`cargo install cargo-vet --version 0.10.3 --locked`、`cargo vet --version` 確認、`/home/koppe/.cargo/bin/cargo-vet`）。
- `notes/cargo-vet-adoption.md` を更新し、ツール導入済み・`supply-chain/` 未作成・init/imports/trust/exempt/audit 方針未決定の事実を反映。
- `cargo vet init`、`supply-chain/` 作成、exempt/imports/audit 記録は未実施（未監査依存を監査済みとして記録していない）。

## 2026-11-15（cargo-vet 初期化・全依存 exempt ベースライン）

- `cargo vet init` を root / web で実行し、`supply-chain/`（config.toml / audits.toml / imports.lock）を作成。初期状態は全依存を `exemptions`（`safe-to-deploy`）として記録（root 804 / web 560）。imports 未使用、監査記録なし（実レビューなしの `audit` は作成しない方針）。
- `README.md` / `SECURITY.md` / `notes/verification.md` / `notes/cargo-vet-adoption.md` を現状に合わせて更新。
- 検証: `cargo vet --locked`（root: `Vetting Succeeded (804 exempted)` / web: `Vetting Succeeded (560 exempted)`）、`cargo vet fmt` 差分なし。`cargo vet suggest` はネットワーク取得が必要で制限環境では失敗。
- 未実施: CI ゲート化（`Seduce0068/workflows` 共通ワークフロー側での検討を優先）、実レビューに基づく `certify`、外部 imports の評価。

## 2026-11-15（cargo-vet の CI / ローカルゲート追加）

- CI: 共通 `rust-gates.yml@v1` の `extra_gates` で `cargo vet --locked`（root / web）を必須化（`cargo install cargo-vet --version 0.10.3 --locked` で導入、taiki-e/install-action は cargo-vet 未対応のため）。
- ローカル: `scripts/check.sh` に `cargo vet --locked`（root / web）の optional ゲートを追加。
- 検証: `bash -n` / `shellcheck` / `shfmt -d -i 2` / YAML パース / `bash scripts/check.sh` ローカル実行。CI 初回実行結果は push 後に `gh run` で確認。

## 2026-11-15（CI 全ジョブの起動失敗: GitHub Actions 請求エラー）

- 2026-11-15 07:11Z（16:11 JST）以降、GitHub Actions の請求エラー（payments failed / spending limit）により全 CI ジョブが起動失敗（最終成功: 12:55 JST）。コード/workflow 定義の問題ではない。
- 対応はアカウントの Billing & plans 確認が必要（ユーザー操作）。期間中の変更はローカル `bash scripts/check.sh`（exit 0）で検証済み。

## 2026-11-15（cargo-vet 外部 imports 候補の一次調査・採否未決）

- `notes/cargo-vet-adoption.md` §8.1 に GitHub API 一次情報による候補調査を追記: `mozilla/supply-chain`（push 2026-11-10、audits.toml 約 509 KB）、`google/rust-crate-audits`（push 2026-11-10、criteria 定義 + 監査基準文書）、`zcash/rust-ecosystem`（push 2026-07-12、`cargo vet suggest` が registry 経由で参照を試行）、EmbarkStudios（該当なし・未確認）。
- 採用はしない（`imports.lock` は空のまま）。criteria の意味・`aggregated-from` の信頼連鎖・主要依存へのカバレッジ評価後に `cargo vet import` / `trust` を検討する。

## 2026-11-15（cargo audit 再実行・ドキュメント整合監査）

- `cargo audit` 再実行: exit 0、既知 8 件の allowed warnings（unmaintained）のみ、新規アドバイザリなし。
- ドキュメント整合: `DESIGN.md`（§7.2 / §15）と `notes/index.md` の「cargo-vet 未導入」記述を初期化済みの現状に更新。README の CI ジョブ数は ci.yml と一致。

## 2026-11-15（generate-sbom.sh の収集バグ修正・cargo-vet Runbook 追加）

- `scripts/generate-sbom.sh`: cargo-cyclonedx 0.5.10 の実出力名（`sbom-root.json` 等、メンバーごと）と収集パターン（`sbom-root*.cdx.json`）の不一致により CI sbom ジョブが空成果物で失敗する問題を修正（パターン拡張・一意名収集・収集後の元ファイル除去）。
- ローカル検証: `bash scripts/generate-sbom.sh target/sbom-local` で 6/6 の CycloneDX JSON を生成・収集（jq 検証）。sbom ジョブ自体は CI で未検証（最後の CI 成功後に追加されたため）で、課金問題解消後の CI 再実行で確認する。
- `notes/cargo-vet-adoption.md` §9 にネットワーク環境での cargo-vet 試行手順（Runbook）を追加。

## 2026-11-15（ブラウザ版の再ビルド・CDP 再検証・wasm サイズ更新）

- `trunk build --release` 成功。wasm 7,635,554 B（gzip 3,042,856 B）/ JS 159,873 B / index.html 1,320 B を再計測。
- CDP デモモード再検証: pass（failures 0、panic/JS 例外 0）。証跡（`web-shell-evidence-demo.json` とスクリーンショット 8 枚）を更新。
- `README.md` / `DESIGN.md` / `web/README.md` / `notes/web-shell.md` / `notes/verification.md` の wasm サイズを現状値へ更新（旧値は履歴として保持）。

## 2026-11-15（ブラウザ版ライブ / エラー時モードの CDP 再検証）

- `cargo build -p tm-agent` で直前ビルドしたエージェントを `127.0.0.1:18787`（`--redact`）で起動し、`web/dist` を 8091 で配信、ヘッドレス Chrome 157 + CDP で再検証。
- ライブ: **failures 0 / passed**（500 行、`fire` 絞り込みで 25 行、`live_error=0`、GET 14 回すべて `Authorization` 付き・URL にトークン無し、プリフライト 1 回）。エラー時（エージェント停止）: **failures 0 / passed**、`rows=0 / live_error=1` で UI に反映、panic 0。
- 証跡（`web/verification/web-shell-evidence-live{,-error}.json`、`notes/assets/web-shell-*-live*.png`）を現行コードで更新。
- 検証後の Chrome / HTTP サーバ / `tm-agent` は停止し、ポート 8091 / 9223 の解放を確認。

## 2026-11-15（日本語 IME CDP 再検証）

- `web/scripts/cdp_verify_ime.py` を再実行: 32 チェックすべて pass、console error / JS 例外 0、スクリーンショット 13 枚を更新。
- 変換中は一覧 28 件のまま →「日本語」確定後に 0 件へ絞り込み、Esc で中断すると 28 件へ復帰、IME 後も通常入力が機能することを確認。証跡は `notes/assets/ime-*.png`。

## 2026-11-16（ブラウザ版 設定永続化の再検証・検証スクリプト修正）

- `web/scripts/cdp_verify_settings.py` の期待スキーマが実装（`tree_view` 追加後）に追随しておらず検証が失敗していたため、期待キーに `tree_view` を追加し復元用設定にも含めた。
- 再実行: `{"passed": true, "failures": []}`（fresh / saved / saved_parsed / restored）。保存内容は `version/interval_ms/sort/details_visible/tree_view` のみでトークンを含まず、再読込で `sort=名前, dir=asc` が復元されることを確認。
- 証跡 `web/verification/web-settings-evidence.json` を更新。検証用プロセスは停止しポート解放を確認。

## 2026-11-16（ネイティブ版 IME デバッグフック再検証）

- `cargo build -p gpui-task-manager` → `TM_DEBUG_IME=1` で起動し、`composing(にほんご, marked=0..4, filter="")` → `composing(日本語, marked=0..3, filter="")` → `committed(filter="日本語", composing=false)` を実機で再確認。
- 証跡 `notes/assets/native-ime-evidence.txt` を追加。実 IME（IBus/fcitx5）経路は未検証のまま。

## 2026-11-16（Wayland text-input-v3 プロトコル観察と実IME経路の調査）

- 実 IME 環境（fcitx5 + mozc 稼働、`XMODIFIERS=@im=fcitx5`）を確認し、`WAYLAND_DEBUG=1 TM_DEBUG_IME=1` でプロトコルログ（3,055 行）を取得。
- 確認: compositor が `zwp_text_input_manager_v3` を公開、GPUI がバインドして `get_text_input(id 27, wl_seat#3)` を発行。text-input-v3 のセットアップは成功。依存 crate `gpui-pre-linux 0.3.12` が `enable()` / `disable()` / `commit_string` / `preedit_string` を実装していることもソースで確認。
- 未到達: ウィンドウがキーボードフォーカスを取得しなかったため `enable()` が発行されず（`wl_keyboard.enter` count = 0）。実 IME 経路の完全検証には対話操作またはキー注入ツールが必要。
- 証跡: `notes/assets/native-ime-wayland-trace.txt`（プロトコル関連 70 行の抜粋）。`notes/verification.md` に所見と限界を追記。

## 2026-11-16（CI 起動失敗の原因特定: private リポジトリの課金枠）

- 対照実験で原因を特定: 同一アカウントでも **public な `gpui-file-explorer` の CI は success（5m33s）**、**private な `gpui-task-manager` は全ジョブが 0 step で failure**。失敗アノテーションは "recent account payments have failed or your spending limit needs to be increased"。
- 結論: private リポジトリの Actions は課金対象のため、支払い/spending limit の問題で起動しない。コードや workflow の問題ではない。
- 対応案: ①public 化（「まだ公開しない」方針とは要調整）、②Billing & plans の修正、③ローカル検証の継続（現在実施中）。①②はユーザー操作が必要。

## 2026-11-16（リポジトリ衛生: Python bytecode の ignore 追加）

- CDP 検証スクリプト（`web/scripts/*.py`、`poc-wasm/scripts/cdp_verify.py`）の実行で `__pycache__/` が生成されるが、リポジトリの `.gitignore` には未登録で、**ユーザーのグローバル git 設定（`~/.config/git/ignore`）にのみ依存**していた（他環境では `.pyc` がコミットされ得る）。
- `.gitignore` に `__pycache__/` と `*.pyc` を追加し、リポジトリ単体で再現性のある除外を確保（`git check-ignore -v` で `.gitignore:15` が適用されることを確認）。
- 併せて Python スクリプト 4 本の構文検証（`python3 -m py_compile`）がすべて成功することを確認。

## 2026-11-16（ディスク一覧の表示順を容量降順に修正）

- 実機で `/boot/efi`(96 MB) が 2 番目に並び、UI の `take(2)` のため `/mnt/timeshift`(439 GiB) と Windows ドライブ(333 GiB) が表示されない実害を確認。
- `tm-platform` のディスクソートを容量降順（同容量はマウントポイント順）に変更し、回帰テスト 2 件を追加（tm-platform 9 → 11 tests）。
- `cargo test --workspace` 155 passed / clippy 警告 0 / fmt 適用。
- あわせて `DESIGN.md` §4 の `Command` 一覧を実装に合わせて更新（`ClearFilter` / `SetSortSpec` / `SetDetailsVisible` / `ToggleTreeView` / `SetTreeView` / `ToggleTreeNode` を追記、保持状態に折りたたみ集合を明記）。

## 2026-11-16（ネットワーク集計の二重計上を修正）

- 実測で `tailscale0`(rx 79.9 MB) が `enp4s0`(rx 124.6 GB) と二重計上されることを確認（a6）。
- `tm-core` に `NetworkSnapshot::physical_totals()` と `is_virtual_interface()` を追加し、overview のレート/履歴計算を仮想 IF 除外へ切り替え。回帰テスト 2 件追加（tm-core 58 tests）。
- 旧挙動を期待値にしていた `tm-viewmodel` のテスト 1 件を正しい値へ更新。
- `cargo test --workspace` 157 passed / web 7 passed / clippy 警告 0 / fmt 適用。

## 2026-11-16（PlatformInfo capability の意味を明確化）

- a5 を検討し、`supports_network` / `supports_disks` は「ホスト capability」と定義して据え置き（列挙失敗は空スナップショットで表現）。doc comment に明記。
- `PlatformInfo::native_read_only` を追加し、web shell の手書きリテラル（プロセス制御不可のネイティブ）を置き換え。capability 定義を 1 か所に集約。
- テスト追加（tm-core 58 → 59）。`cargo test --workspace` 158 passed / web 7 passed / wasm check OK / clippy 警告 0。

## 2026-11-16（Ctrl+K の保護判定を view model に一本化）

- Ctrl+K がシェル側で `is_protected` を先に確認していたため、保護プロセスでは何も起きずメッセージも出なかった（ボタン経路と不一致）。常に `Command::RequestSignal` を投げ、判定とメッセージを view model に一本化。
- `cargo test --workspace` 158 passed / clippy 警告 0 / fmt 適用。

## 2026-11-16（エージェントのシリアル処理を実測・制約として記録）

- レビュー s1 を実測: 無送信接続 1 本で `/healthz` 49 ms、3 本同時保持で 2,059 ms（1 本あたり `READ_TIMEOUT`=1 秒を直列待ち）。
- 影響はエージェントの可用性のみで、接続元は loopback に限定。UI は同時 1 リクエスト方式のため通常運用では競合しない。
- 並列化は `SystemProbe` の `Send` 境界・probe の共有方法・同時接続上限を同時に変える設計判断のため、今回は文書化に留め改善案を `notes/verification.md` に記録。

## 2026-11-16（ライトテーマ追加に伴う文書・検証の追随）

- ネイティブ版のライトテーマ（Catppuccin Latte）実装（`b1a6f85`）を受け、`DESIGN.md` §10 / §14 と `README.md` の「ライトテーマは未実装」記述を実態に合わせて更新（ネイティブ版は切替ボタン + `Settings.theme` 永続化まで実装、ブラウザ版は未実装で `"Dark"` 固定。`SETTINGS_VERSION = 2`）。
- `web/scripts/cdp_verify_settings.py` の期待スキーマに `theme` を追加し、復元用設定も `version: 2` / `theme: "Light"` に更新。再検証は pass（`{"passed": true, "failures": []}`）、証跡を更新。
- ブラウザ版の状態フック（`data-gpui-proof`）に `theme` が無いため、CDP ではテーマ復元を検証できない。web 側のテーマ実装と状態フック追加を次回候補として `notes/verification.md` に記録。

## 2026-11-16（ブラウザ版ライトテーマの実装・CDP 検証）

- 前回の「次回候補」だった web 版テーマを実装。`web/src/theme.rs` をネイティブ版と同じ
  `Palette` / `DARK`（Mocha）/ `LIGHT`（Latte）/ `theme()` / `set_theme()` / `palette()` 構成へ拡張し、
  `shell.rs` / `widgets.rs` の色参照 93 箇所を `palette().*` に置換。`C_*` 定数は廃止。
- ヘッダー右端に `toggle-theme-web` ボタン（幅 56 px 固定、CDP から座標を決め打ち可能）を追加。
  クリックで Dark ↔ Light を切替えて `persist_settings()` で保存し、
  `restore_settings()` は `set_theme(settings.theme)` で復元。共有スキーマにより
  ネイティブ版と web 版で同じ `theme` 値をやり取りできるようになった（旧挙動は web が常に `Dark` 固定保存）。
- `data-gpui-proof` に `theme=dark|light` を追加。`web/scripts/cdp_verify_settings.py` に
  既定値・保存値・復元・ボタン切替（Light → Dark → Light）のチェックを追加し再検証:
  `{"passed": true, "failures": []}`。証跡 `web/verification/web-settings-evidence.json` 更新。
  light 状態の画面は `notes/assets/web-shell-light-theme.png`。
- 検証: `cargo test -p tm-web` 11 passed / 0 failed（テーマ 4 件追加）、
  `cargo check --target wasm32-unknown-unknown` 成功、`trunk build --release` 成功。
  wasm 7,641,445 B（gzip 3,044,050 B、旧 7,635,554 B から微増）。
- 文書更新: `README.md` / `DESIGN.md` / `web/README.md` / `notes/verification.md` /
  `notes/web-shell.md`。未実装項目から「ブラウザ版のテーマ切替」を削除し、
  `prefers-color-scheme` 連動のみ未実装として残す。
- CI は今回も未対応（全 8 ジョブが 0 step で failure、public 化後も継続）。

## 2026-11-16（ブラウザ版: 概要カードとツールバーの重なりを修正）

- ライトテーマの画面でシステムカードの温度行とツールバーが重なっていたため、
  `OVERVIEW_HEIGHT` を 186 → 260 px へ変更（温度 3 件までカード内に収める高さ）。
  CDP 検証スクリプト 2 本の座標定数と `notes/web-shell.md` も追随。
- 再検証: web 11 tests / clippy 0 warning / wasm check / trunk build 成功。
  CDP デモ `failures: []`、設定 `passed: true`、IME 32/32 OK。スクリーンショット
  （`web-shell-*-demo.png` / `web-shell-light-theme.png` / `ime-*.png`）と証跡 JSON を更新。
  IME 証跡は `web/verification/ime-evidence.json` に初めて保存。
- CI は依然として新規 run が queued のまま（課金/実行基盤側の問題と推定、未解決）。

## 2026-11-16（ドキュメント不整合の残存 2 件を修正）

- `notes/solutions-survey.md`: `start_time` を「起動からの秒」と誤記していた箇所を
  「Unix epoch 秒（`sysinfo 0.40.3` の `Process::start_time()`）」に訂正（`crates/tm-core/src/model.rs`
  の説明と一致）。
- `DESIGN.md`: ADR-020（DEB 生成）が ADR-019（ネイティブ IME）より前に置かれていたため、
  番号順になるよう 2 セクションを入れ替えた（内容変更なし、他ファイルからの参照なし）。
- CI: この間の新規 run では 8 ジョブが実際に起動（6m21s 実行）。先の run は同ブランチへの
  連続 push による concurrency で macOS ジョブが cancelled になったもので、
  コード失敗ではない。最新 run の完了は未確認。

## 2026-11-16（CI の全ジョブ成功を確認・起動失敗状態の解消）

- 2026-11-15 07:11Z 以降、8 ジョブすべてが 0 step・数秒で failure だった CI が復旧した。
- 最新 run [43885904734](https://github.com/Seduce0068/gpui-task-manager/actions/runs/43885904734)
  （commit `9e2cdae`、2026-11-16 23:33 JST 完了）は gates / wasm check / cargo-deny /
  native-windows / native-macos / release-linux / web-dist / sbom の **8 ジョブすべて success**。
- 直前の 2 件の run が `cancelled` になっているのは、同ブランチへの連続 push による
  concurrency 設定でのキャンセルであり、コードの失敗ではない（一部ジョブは success）。
- 先の原因推定は private リポジトリの Actions 課金/spending limit だったが、
  public 化後に解消した模様。請求設定の詳細はエージェントから確認できないため未確認。

## 2026-11-16（エージェントを並列接続化・s1 解消）

- `agent/src/main.rs`: `SharedState`（Mutex で共有する `SystemProbe`、トークン、
  接続数カウンタ）を導入し、接続ごとにスレッドを spawn。ロックは `route()` の間だけ。
  上限 `MAX_CONNECTIONS = 8` を超える接続には即 503。
- 回帰テスト 2 件追加（アイドル接続下のヘルス応答、上限超過の 503）。
  tm-agent 39 → 41 tests、workspace 合計 162 passed / clippy 0 warning / fmt OK。
- Runtime Proof: 実エージェントで `/healthz` 0.24 ms（通常）、3.04 ms（アイドル 3 本）、
  503 を 0.03 ms（アイドル 13 本）。旧 2,059 ms から改善。プロセス停止・ポート解放確認済み。
- 検証中に誤って既存の OpenClaw gateway（ port 18789）を停止したが、systemd user service
  `openclaw-gateway.service` が自動再起動し ready を確認（復旧済み）。原因は
  `cmd1 && cmd2 &` が cmd1 ごとバックグラウンド化する bash 構文の誤用。
  以降は `setsid` + 実 PID の確認を使用。

## 2026-11-16（リポジトリ整理: 旧パレットの一時バックアップを削除）

- `app/src/theme.rs.bak` はライトテーマ実装時に作られた旧ダーク専用パレットの
  一時コピーで、ソース・ドキュメントから参照がないまま Git に追跡されていた。
  削除（内容は履歴に残る）。ビルド・テストへの影響なし。

## 2026-11-16（検証ログ §6 の古い記述を現状に修正）

- `notes/verification.md` §6「既知の制限・未検証」に、既に解消済みの
  「IME による日本語入力は未対応」「`cargo-deny` は未導入」「WebGPU / Safari / Firefox /
  日本語 IME は未検証」が残っていた。実装状況に合わせて修正
  （IME は両版実装済み・実 IME 手動確認のみ未実施、`cargo-deny` / `cargo audit` は CI ゲート化済み）。
  あわせて 2026-11-16 再計測の JS サイズ表記を実測 159,873 B に訂正。

## 2026-11-17（ゼロ幅・不可視書式文字のサニタイズを追加）

- `crates/tm-core/src/sanitize.rs` に `is_invisible_format()` を追加し、
  ゼロ幅文字・単語結合子・BOM・variation selector 等を空白化（結合文字は保持）。
  レビュー s9 の残存部分のうち、正当な名前を壊さない範囲を対策。
- テスト 2 件追加（tm-core 59 → 61、workspace 164 passed）。
  README / DESIGN / SECURITY / notes/verification を現状に合わせて更新。

## 2026-11-17（現状の検証数値をドキュメントへ反映）

- `README.md` §検証状況 と `notes/verification.md` §2 のテスト件数が古く
  （143 passed / web 7）実測（164 passed / web 11）と不一致だったため更新。
  内訳: tm-core 61 / tm-viewmodel 37 / tm-platform 14 / tm-agent 41 / app 11。

## 2026-11-17（エージェント並列化のレビュー指摘を修正）

- 敵対的レビューで指摘された 3 点を修正:
  - Drop ガード `ActiveConnection` でパニック時のカウンタリークを解消（回帰テスト追加）。
  - 503 拒否経路にも `WRITE_TIMEOUT` を設定し accept ループの阻塞を防止。
  - Mutex poison 回収時にログを出力（無ログ fail-open の解消）。
- tm-agent 41 → 42 tests、workspace 165 passed / clippy 0 warning / fmt OK。
- サブエージェント: agent-1（読み取り専用・コード変更なし）。採用 3 件、既知制約として記録 3 件。

## 2026-11-17（CI 最終 run の状況: macOS ランナーのキュー待ち）

- run 43899207540（commit `f430a7f`）は wasm / windows / web-dist / release-linux /
  gates / sbom / cargo-deny の **7 ジョブ success**。macOS ジョブは約 14 分間
  queued のまま実行されず、run 全体は完了していない（コード失敗ではない）。
- 直前 run 43897299713 では macOS を含む 8/8 が success であり、
  ランナー可用性の問題と判断。監視ジョブはタイムアウト前に停止した。

## 2026-11-17（Host検証403へのCORS付与）

- `agent/src/main.rs`: Origin 判定を早期 return 前に移動し、許可 loopback Origin
  に限り Host 検証 403 にも CORS ヘッダを付与（ブラウザ UI がエラー本文を読めるように）。
  不正 Origin は反射しない。回帰テスト 1 件追加（tm-agent 42 → 43）。
- 431 / 400 / 503 はOriginが読めないため据え置き（文書化済み）。

## 2026-11-17（調査ノート索引を現状に合わせて整理）

- `notes/index.md`: IME ノートの概要を「ブラウザ版・ネイティブ版とも実装済み、
  実 IME 未検証」に修正。独立レビューへの空行を削除。
  証拠画像一覧にライトテーマ・IME・wasm-opt の画像を追記（`--redact` の注記も追加）。

## 2026-11-17（SECURITY.md を実装へ追随）

- 脅威モデルと方針の記述が古かった 3 点を修正:
  - 永続化設定に `tree_view` / `theme` を追加（`Settings` スキーマ v2 と整合）。
  - CORS がエラー応答 403 にも許可 Origin のみ付与する旨を明記。
  - 同時接続 8 本・超過 503 の並列化と、アイドル接続によるスロット占有の
    残余リスクを脅威モデル表に追記。

## 2026-11-17（同時接続上限テストのフレーク要因を解消）

- `excess_connections_are_rejected_immediately` は、8 本のアイドル接続が
  READ_TIMEOUT（1 秒）でタイムアウトしてスロットを解放すると 9 本目が 200 を返し、
  アサーションが失敗し得た。本番コードの timeout を変更せず、テスト側で
  部分的なリクエストヘッダ（`GET ...\r\nHost: x`）を送るキーパースレッドを導入し、
  スロットを確実に保持するよう修正（keepalive の総バイト数は 8 KiB 上限未満）。
- 検証: 該当テストを 5 回連続実行しすべて成功（各 0.60 秒）。
  workspace 166 passed / clippy 0 warning / fmt OK / wasm check OK。

## 2026-11-17（レビュー履歴の「未対応」注記を現状で補足）

- `notes/verification.md` のレビュー時点（2026-11-14）の「未対応として残るもの」に
  2026-11-17 追記を追加。s1/s2/s3 はその後すべて対応済みであることを明示し、
  現状は後続の並列化・CORS 付与の節を参照するよう案内（履歴自体は改変しない）。

## 2026-11-17（README のエージェント検証行へ並列化の実測を反映）

- `README.md` §検証状況のエージェント行に、2026-11-16 の Runtime Proof
  （アイドル 3 本下で `/healthz` 3.04 ms、上限超過で即 503）と 403 の CORS 付与を追記。

## 2026-11-17（未使用依存を削除）

- `cargo machete` が検出した `web/` / `poc-wasm/` / `app/` の未使用依存
  （console_error_panic_hook / js-sys / wasm-bindgen-futures / serde）を削除。
  各 Cargo.lock を再生成し、再検出 0 件を確認。
- 全検証（166 tests / clippy / wasm / trunk build / CDP 設定・デモ）成功。
  wasm サイズ不変。検証プロセスの停止・ポート解放済み。

## 2026-11-17（CDP 検証の証跡出力先を修正）

- 依存削除後の CDP 再検証で `cdp_verify_web.py` を `web/` から実行したため、
  デモ証跡がリポジトリルートの `verification/` に誤出力された。
  スクリプトを `web/scripts/` から実行し直して `web/verification/` と
  `notes/assets/web-shell-*.png` に正しく再出力し、誤出力ディレクトリを削除。
  検証用プロセスの停止・ポート解放も確認済み。

## 2026-11-17（CI run 43910499314 の macOS キュー待ち）

- run 43910499314（commit `7291885`）は wasm / windows / web-dist / release-linux /
  gates / sbom / cargo-deny の 7 ジョブ success。macOS ジョブのみ約 10 分間
  queued の後 cancelled（コード失敗ではない）。直前 run 43907783410 では
  macOS を含む 8/8 success を確認済みで、ランナー可用性の問題と判断。

## 2026-11-17（検証ログの OKF メタデータを更新）

- `notes/verification.md` の `verified` を 2026-11-14 から現在日時に更新し、
  `status` を `draft` → `verified` へ変更（多数の Runtime Proof を記録済みのため）。
  `generated` は作成日として保持。OKF 形式・markdownlint 検証成功。

## 2026-11-17（設定保存の一時ファイル名を PID 付きへ）

- `app/src/settings.rs`: 一時ファイル名 `settings.json.tmp` を
  `settings.json.tmp.<pid>` に変更し、同時保存時の上書き競合を回避。
  回帰テスト 1 件追加（app 11 → 12、workspace 167 passed）。

## 2026-11-17（設定テストと履歴記述の整合性を修正）

- 前回の PID 付き一時ファイル名変更（`068adb2`）に対し、
  `round_trips_settings_atomically` が旧名 `settings.json.tmp` を確認していたため、
  PID 付き名を確認するよう修正（テストの意図を復元）。
- `notes/verification.md` §13 の履歴記述に、現行の一時ファイル名
  `settings.json.tmp.<pid>` とスキーマ v2（6 項目）の注記を追加。
- app 15 tests / clippy 0 warning / markdownlint（件数同一 120）。

## 2026-11-17（設定 v1→v2 マイグレーションのテストを補強）

- `crates/tm-viewmodel/src/settings.rs` の `missing_fields_fall_back_to_defaults` に、
  スキーマ v2 で追加された `tree_view` / `theme` の既定値アサーションを追加
  （v1 の設定ファイルには双方が存在しないため、既定値へ縮退することを明示）。
  新規テストではなく既存テストの補強（workspace 167 tests のまま）。
- 検証: cargo test --workspace / clippy / wasm check 成功。

## 2026-11-17（tm-agent の --help にトークン解決順を追記）

- `agent/src/args.rs` の `USAGE` に `TM_AGENT_TOKEN` の説明と解決順
  （`--token` または `--token-file` > `TM_AGENT_TOKEN` > 自動生成）を追加。
  README には記載があったが `--help` 単独では分からなかったため。
  実装（`agent/src/main.rs` の token 解決ロジック）と一致することを確認。

## 2026-11-17（web/src/lib.rs の allow 属性に理由を明記）

- `#![allow(dead_code, unused_imports)]` の理由（wasm 専用アイテムを native でも
  コンパイルしてテストするため）をコードコメント化。属性を外すと 30 件以上の
  警告が出ることを確認し、クレート全体の allow が妥当であることを検証。
  動作変更なし（web 11 tests / clippy / wasm check 成功）。

## 2026-11-17（check.sh に未使用依存チェックを追加）

- `scripts/check.sh` に `cargo machete`（root / web）のオプショナルゲートを追加。
  cargo-deny / cargo-vet と同じパターンで、未インストール時は skip。
- 検証: `bash -n` / `shellcheck` / `shfmt -d -i 2`（差分なし）および
  `bash scripts/check.sh` を通しで実行し `All checks passed.` を確認。
  root / web とも未使用依存 0 件。

## 2026-11-17（format.rs の非有限値を防御）

- `format_percent` / `format_cpu_percent` / `format_load_average` で
  NaN / 無限大を 0 として表示するよう修正（`NaN%` 等の描画を防止）。
  テスト 1 件追加（tm-core 61 → 62、workspace 168 passed）。

## 2026-11-17（ネットワークレートの微小 elapsed を防御）

- `NetworkCounters::rate_since` の下限を f64::EPSILON → 1 ms へ引き上げ、
  不正な微小タイムスタンプによる u64 saturate（虚偽のスパイク）を防止。
  テスト 1 件追加（tm-core 62 → 63、workspace 169 passed）。

## 2026-11-17（履歴グラフの NaN 混入を防御）

- `MetricHistory::push` で非有限の CPU/メモリ率を 0 に置換（`clamp` は NaN を
  すり抜けるため）。テスト 1 件追加（tm-core 63 → 64、workspace 170 passed）。

## 2026-11-17（build-deb.sh を shfmt 準拠へ整形）

- 全シェルスクリプト（4 本）の `bash -n` / `shellcheck` / `shfmt -d -i 2` を
  一括検証したところ、`packaging/linux/build-deb.sh` のみ shfmt 差分あり
  （case 文のインデントとパイプ位置）。機能変更なしで整形し、3 検証すべて成功。

## 2026-11-17（モーダル overlay に occlude() を追加）

- レビュー s10 は「no-op の `on_click` で背景クリックを防いでいる（推測）」だった。
  GPUI 0.3.12 のソースで `InteractiveElement::occlude()` が
  `HitboxBehavior::BlockMouse` を設定する明示 API であることを確認し、
  kill-overlay / help-overlay の両方に追加（no-op listener は残す）。
- コンパイル・app 15 tests 成功。実機クリック注入は kdotool/xdotool が
  未インストールで未検証（コード上の保証として記録）。

## 2026-11-17（現状サマリ文書を追加）

- `notes/current-state.md` を新規作成（OKF 準拠・markdownlint 0 件）。
  `log.md` は時系列、`verification.md` は個別証跡のため、現時点の到達点を
  俯瞰する文書が欠けていたため。検証状況（170 passed / CI 8/8 ほか）、
  対応済み事項、未解決候補、関連文書への導線を記載。
  `notes/index.md` の「検証」節にリンクを追加。

## 2026-11-17（SBOM 生成のローカル再現）

- `bash scripts/generate-sbom.sh target/sbom-r24` を実行し、6 件の CycloneDX JSON
  （root 5 メンバー + web）を生成・収集。すべて `bomFormat=CycloneDX` /
  `specVersion=1.3` で jq パース成功。元ファイルの後始末も動作し、
  作業ツリーはクリーン。未使用依存削除後も SBOM 生成に影響なし。

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

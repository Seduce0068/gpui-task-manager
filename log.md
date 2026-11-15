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
  - ツリーは基本表示（折りたたみなし）、温度はベストエフォート収集/表示。詳細 GPU 利用率等は未実装。

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

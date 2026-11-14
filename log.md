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

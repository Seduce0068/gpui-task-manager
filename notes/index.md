# 調査・検証ノート索引

GPUI Task Manager の調査・検証資料です。設計判断の根拠は本文を参照してください。

## 調査

- [solutions-survey.md](solutions-survey.md) — 既存 Linux モニタ 11 候補の比較、
  活発度・ライセンス・Web 対応・採用判断、ブラウザ監視プロトコル、GPUI 先行事例
- [crates-survey.md](crates-survey.md) — 収集系 crate の評価（sysinfo / procfs /
  rustix / heim など）、RUSTSEC、sysinfo の制約、実測ログ全文
- [ime.md](ime.md) — 日本語 IME（ブラウザ版・ネイティブ版の実装と Runtime Proof、実 IME 未検証）
- [physical-verification.md](physical-verification.md) — Windows/macOS 物理実機・クラウド検証の選択肢と費用・手順
- [cargo-vet-adoption.md](cargo-vet-adoption.md) — cargo-vet 導入（初期化済み・全依存 exempt ベースライン、運用方針、imports 候補一次調査、監査捏造なし）
- [cross-platform.md](cross-platform.md) — Windows/macOS 対応（cfg 分岐・CI マトリクス・cross-check）
- [wasm-spike.md](wasm-spike.md) — GPUI Web の実現性スパイク（capability 表、
  バックエンド切り分け、推奨構成、未確認事項）
- [residual-tasks-research.md](residual-tasks-research.md) — 全 6 ラウンドの
  クロスレビュー記録（189 所見・根拠・重複統合・妥当性判定・最終バックログ）

## 検証

- [current-state.md](current-state.md) — 2026-11-20 時点の到達点・検証結果・未解決事項の俯瞰（テスト実測 workspace 608 / web 208、件数は README.md「検証状況」と同期）
- [agent-runtime-verification.md](agent-runtime-verification.md) — `tm-agent` 実動検証（TCP ソケット経由の主要エンドポイント、loopback / 認証の Runtime Proof と制約）
- [native-runtime-verification.md](native-runtime-verification.md) — ネイティブ GPUI アプリの Wayland 実起動（ウィンドウ生成の Runtime Proof）
- [environment-verification.md](environment-verification.md) — 実機環境・証跡の確認記録（HiDPI / スクリーンショット鮮度 / ローカルブランチ整理、事実と未確認事項）
- [hidpi-verification.md](hidpi-verification.md) — Web シェルの HiDPI（DPR 1 / DPR 2）描画をヘッドレス Chrome + CDP で確認した証跡（古い `tick=0` 記録には解消注記あり）
- [web-ime-runtime-verification.md](web-ime-runtime-verification.md) — ブラウザ版日本語 IME の実ヘッドレス Chrome 検証（composition / 確定 / Escape 破棄、32 チェック成功）
- [web-settings-runtime-verification.md](web-settings-runtime-verification.md) — Web 設定永続化の Runtime Proof（localStorage 保存・再読込・破損退避・復元）
- [web-live-runtime-verification.md](web-live-runtime-verification.md) — Web シェルの実 `tm-agent` / SSE ライブ接続・更新・エージェント停止時の挙動を CDP で検証
- [adr-ci-tool-version-management.md](adr-ci-tool-version-management.md) — ADR: CI ツール版の手動ピン維持と Dependabot 対象範囲、mise.toml 一元管理の移行メモ
- [adr-wasm-inline-js-download.md](adr-wasm-inline-js-download.md) — ADR: wasm ダウンロードの inline JS 維持判断と web-sys 置換条件
- [dependency-audit.md](dependency-audit.md) — 重複バージョン・feature 差分の監査手順（cargo deny / tree / metadata、既存ゲート非破壊）
- [verification.md](verification.md) — ビルド・テスト・clippy・ネイティブ起動・
  エージェント・wasm の Runtime Proof と既知の制限
- [web-shell.md](web-shell.md) — ブラウザ版シェル（`web/`）の実装と検証
- [review-2026-11-14.md](review-2026-11-14.md) — 独立レビュー（セキュリティ・アーキテクチャ・運用）

## 証拠画像

- `assets/task-manager-native-wayland.png` — ネイティブ版（実データ、Wayland、`--redact` 適用）
- `assets/wasm-spike-*.png` — GPUI Web スパイク（描画・スクロール・入力・fetch）
- `assets/web-shell-*.png` — ブラウザ版シェル（デモ／ライブ／エラー時）
- `assets/web-shell-light-theme.png` — ブラウザ版ライトテーマ（Catppuccin Latte）
- `assets/ime-*.png` / `assets/native-ime-*.png` — 日本語 IME の composition 検証
- `assets/wasmopt-*.png` — wasm-opt 適用後の描画確認

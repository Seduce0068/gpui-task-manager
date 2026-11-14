# 調査・検証ノート索引

GPUI Task Manager の調査・検証資料です。設計判断の根拠は本文を参照してください。

## 調査

- [solutions-survey.md](solutions-survey.md) — 既存 Linux モニタ 11 候補の比較、
  活発度・ライセンス・Web 対応・採用判断、ブラウザ監視プロトコル、GPUI 先行事例
- [crates-survey.md](crates-survey.md) — 収集系 crate の評価（sysinfo / procfs /
  rustix / heim など）、RUSTSEC、sysinfo の制約、実測ログ全文
- [wasm-spike.md](wasm-spike.md) — GPUI Web の実現性スパイク（capability 表、
  バックエンド切り分け、推奨構成、未確認事項）

## 検証

- [verification.md](verification.md) — ビルド・テスト・clippy・ネイティブ起動・
  エージェント・wasm の Runtime Proof と既知の制限
- [web-shell.md](web-shell.md) — ブラウザ版シェル（`web/`）の実装と検証

- [review-2026-11-14.md](review-2026-11-14.md) — 独立レビュー（セキュリティ・アーキテクチャ・運用）

## 証拠画像

- `assets/task-manager-native-wayland.png` — ネイティブ版（実データ、Wayland）
- `assets/wasm-spike-*.png` — GPUI Web スパイク（描画・スクロール・入力・fetch）
- `assets/web-shell-*.png` — ブラウザ版シェル（実装後）

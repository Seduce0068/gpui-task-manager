---
type: Reference
title: 実機環境・証跡の確認記録
description: HiDPI / スクリーンショット鮮度 / ローカルブランチ整理について、エージェントが確認できた事実と未確認事項を記録する
tags: [verification, environment, runtime-proof, task-manager]
resource: gpui-task-manager
generated: 2026-11-18T12:35:38+09:00
verified: 2026-11-19T15:55:00+09:00
status: verified
---

## HiDPI / モニタ間移動（R3-A05）

確認できた事実:

- 実行環境は Wayland セッション（`XDG_SESSION_TYPE=wayland`、
  `WAYLAND_DISPLAY=wayland-0`）で、Xwayland の `DISPLAY=:1` も存在する。
- `xwininfo` と `spectacle` が導入済みで、ウィンドウ列挙とスクリーンショット
  取得の手段自体は利用可能。
- `notes/physical-verification.md` には HiDPI / マルチモニタが未検証として
  記録されている。

未確認:

- 実機の HiDPI スケーリング、モニタ間移動、ウィンドウ装飾の再レイアウトは、
  物理的な複数モニタと GUI 操作を伴うためエージェント単独では検証できない。
- 必要な手順は `notes/physical-verification.md` に記載済み（リサイズ、
  HiDPI、スクロール、列ソート、絞り込みの目視確認）。

## スクリーンショットと証跡数値の鮮度（R1-G12 / R1-G13）

確認できた事実:

- `notes/ime.md` に CDP 操作とスクリーンショットの対応表が存在する。
- `notes/physical-verification.md` にネイティブ版の起動・スクリーンショット
  手順と、Windows/macOS の物理環境が未検証である旨が記録されている。
- テスト件数は README と `notes/current-state.md` / `notes/verification.md`
  で現行値（workspace 359 / web 74）に更新済み。
- 一部の画像は `/tmp` に取得した一時ファイルであり、リポジトリ内の
  証跡とはパスが異なる（`notes/ime.md` の記録）。

未確認:

- 個々のスクリーンショット画像と本文数値の 1 対 1 の対応は、画像が
  一時ディレクトリに保存されているため全面再検証していない。
- 性能記述の再測定（百万件規模の実測など）は `notes/residual-tasks-research.md`
  の調査タスクとして残っている。

## ローカルブランチ整理（R1-G14）

確認できた事実（`git branch --merged main` / `git branch -vv`）:

2026-11-19 時点の実測（`git status --short --branch` / `git branch -vv`）:

- base checkout の `main`: `origin/main` に対して **ahead 2 / behind 53**。
  未 push の 2 コミット（10a8e16, 6eed24d）は古い分岐から生えており、
  `6eed24d` は `origin/main` の祖先ではない。これらをそのまま push すると
  公開済みの監査成果（redaction / gitleaks / CI ゲート等）が失われる。
  新機能は隔離 worktree で cherry-pick 統合し、`integration/base-r1`
  から `origin/main` へ fast-forward push 済み（2026-11-19）。
- `fix/tm-agent-test-shutdown-race`: `origin/main` にマージ済み。リモート
  ブランチが残存（`dab7520`）するため削除しても復元可能。
- `backup/main-pre-integration` / `backup/round31-safe-pre-integration`:
  統合前の rollback 点として作成したローカル ref。前者は 6eed24d、
  後者は 4b5359c（`origin/main` の祖先）を指す。
- `master` とアーカイブ用ブランチ: 同一コミット `bc30c0b` を指すが、
  `main` にはマージされておらず、いかなるリモート ref からも到達不能。
  復元は reflog（既定 90 日 / 到達不能 30 日）のみ。

判断:

- ブランチ削除は破壊的操作のため、ユーザー確認なしには実行しない。
- マージ済みの `fix/tm-agent-test-shutdown-race` は削除候補。リモートに
  同一 SHA が残るため復元は容易だが、削除自体は状態変更のため委任外。
- `backup/*` は rollback 点として意図的に保持中。不要になった時点で
  ユーザー承認を得て削除する。
- `master` / アーカイブ用ブランチは未マージの履歴を含む可能性があり、
  削除前に差分確認が必要。

## 結論

この3項目は「実機・外部環境・破壊的操作」に依存するため、エージェントは
現状を調査・記録するにとどめた。実行にはユーザーの環境操作または
削除の承認が必要である。

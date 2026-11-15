---
type: Reference
title: cargo-vet 導入準備メモ（未導入）
description: gpui-task-manager における cargo-vet（supply-chain 監査）の現状棚卸し、導入前に決める方針、段階的手順案。監査記録は未作成。
tags: [supply-chain, cargo-vet, security, dependencies]
status: draft
generated: 2026-11-15T14:42:00+09:00
verified: 2026-11-15T14:50:00+09:00
---

## 1. 現状（事実）

- **導入状態**: 未導入。リポジトリに `supply-chain/` は存在せず、作業環境に `cargo-vet`（`cargo vet`）も未インストール（`cargo vet` 実行不可を確認）。
- **代替/併用中の供給網対策**:
  - `cargo audit` / `cargo-deny`（advisories/bans/licenses/sources、root/web）
  - CycloneDX SBOM: [`scripts/generate-sbom.sh`](../scripts/generate-sbom.sh) と CI `sbom` ジョブ
  - Dependabot（Cargo root/web、GitHub Actions、週次）とセキュリティアラート
  - GPUI 系・主要依存の完全ピン運用と `Cargo.lock` コミット
- **方針の明文化**: `README.md` / `SECURITY.md` / `notes/verification.md` に「trust 方針・棚卸し・運用手順を定めてから導入、未監査依存を監査済みとして記録しない」を記載済み。

## 2. 依存棚卸し（`cargo metadata` ベース、2026-11-15 時点のローカル lockfile）

> 数値は現在の `Cargo.lock` / 解決グラフに基づく概算。導入時は再取得すること。

- **総パッケージ数**: 809
- **ソース内訳**:
  - crates.io（registry）: 804
  - git: 0（ユニークな name/version/source 組み合わせ: 0）
  - workspace path: 5
  - 非workspace path: 0
- **workspace direct deps（normal/build）**:
  - `tm-core`: `serde`
  - `tm-viewmodel`: `serde`, `tm-core`
  - `tm-platform`: `libc`, `rustix`, `serde`, `sysinfo`, `tm-core`
  - `tm-agent`: `serde`, `serde_json`, `tm-core`, `tm-platform`
  - `gpui-task-manager`: `gpui-kit`, `serde`, `serde_json`, `tm-core`, `tm-platform`, `tm-viewmodel`
- **ライセンス上位（metadata 文字列ベース）**: Apache-2.0 / MIT 系が大半。導入時は `cargo-deny` の licenses 設定・実ファイルと突合する。

### git 依存の扱い（要注意）

`cargo-vet` は registry 中心の運用が基本であり、git/path 依存は監査・imports の扱いが registry クレートと異なる。GPUI 系を含む git 依存が 0 件あるため、導入設計では次を明示する必要がある。

- git 依存を「監査対象外として除外」するか、「固定リビジョン/タグを前提に手動レビュー記録で補う」か
- `Cargo.lock` 固定・ピン更新手順・差分レビュー責任の所在
- registry 公開物への置換可能性（将来）

## 3. 導入前に決めるべき事項（未決）

- [ ] **trust root**: 自組織/個人の audits を置くか、信頼する imports 元（例: Mozilla, Google, Embark 等の公開 audits）を選ぶか。imports は「信頼の委譲」であり盲目的採用はしない。
- [ ] **初期方針**: `cargo vet init` 後に全依存を一括 `exempt` するか、重要度/到達性で優先順位付けするか（exempt は「未監査の既知状態」であり「監査済み」ではない）。
- [ ] **監査基準（criteria）**: `safe-to-run` / `safe-to-deploy` 等の基準をどう使い分けるか。デスクトップアプリ + ローカルエージェント + wasm の性質を反映する。
- [ ] **更新運用**: Dependabot/手動更新時に `cargo vet` を通す担当・手順・CI 失敗時の対応（exempt 追加の可否とレビュー要件）。
- [ ] **CI 統合**: どのジョブ/タイミングで `cargo vet` を必須化するか（ローカル未導入期間との整合）。
- [ ] **git 依存ポリシー**: 上記 2節の扱い。

## 4. 段階的導入案（案・未実施）

1. **準備**: `cargo-vet` を開発環境/CI に導入しバージョンを固定。`supply-chain/` 用ディレクトリ構成を決める。
2. **初期化（方針確定後）**: `cargo vet init` 等で雛形生成。imports を採用する場合は採用元を明示して最小限から試行。
3. **棚卸し可視化**: `cargo vet` の未監査一覧をもとに、直接依存・高リスク領域（ネットワーク、プロセス制御、暗号、unsafe 相当の FFI 境界）から優先度付け。
4. **記録**: 実際にレビューしたものだけ `audit` として記録。未レビューは `exempt`/保留として扱い、理由と期限方針を残す。**捏造しない**。
5. **CI/運用**: `cargo-deny`/`cargo-audit` と並行運用し、更新フローに vet 確認を組み込む。
6. **継続改善**: 依存更新ごとに差分を小さく保ち、git 依存削減やピン更新のレビューを継続。

## 5. リスクと留意点

- パッケージ数が多く（809）、初期コストが高い。形式的な exempt 量産は安心感だけを与えうるため、目的（何を保証したいか）を先に定義する。
- git 依存が 0 件あり、registry 前提の imports だけでは網羅できない。
- `cargo-vet` は `cargo-audit`/`cargo-deny` と目的が重なる部分と異なる部分がある（既知脆弱性 DB、ポリシー/ライセンス/ban、監査連鎖）。置換ではなく補完として位置づける。

## 6. 次の具体アクション候補（安全・小さく進める）

- 開発環境への `cargo-vet` 導入可否（バージョン固定方針）を決める。
- trust/imports の候補を比較し、採用基準を短く文書化する。
- `cargo metadata` から直接依存と git 依存の一覧を定期更新できるスクリプト化を検討する（監査は自動化しない）。

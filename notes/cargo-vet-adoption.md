---
type: Reference
title: cargo-vet 導入準備メモ（未導入）
description: gpui-task-manager における cargo-vet（supply-chain 監査）の現状棚卸し、導入前に決める方針、段階的手順案。監査記録は未作成。
tags: [supply-chain, cargo-vet, security, dependencies]
status: draft
generated: 2026-11-15T14:42:00+09:00
verified: 2026-11-15T15:08:00+09:00
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

### git / path 依存の扱い

`cargo-vet` は registry 中心の運用が基本であり、git/path 依存は監査・imports の扱いが registry クレートと異なる場合がある。現時点の棚卸しでは **git 依存 0 件・非workspace path 依存 0 件**（workspace メンバー 5 件を除く）だが、将来追加される可能性があるため導入設計では次を方針として明示しておく。

- git/path 依存を追加する場合の扱い（原則避ける、必要時は固定リビジョン/タグ・差分レビュー・手動記録で補う等）
- `Cargo.lock` 固定・ピン更新手順・差分レビュー責任の所在

## 3. 導入前に決めるべき事項（未決）

- [ ] **trust root**: 自組織/個人の audits を置くか、信頼する imports 元（例: Mozilla, Google, Embark 等の公開 audits）を選ぶか。imports は「信頼の委譲」であり盲目的採用はしない。
- [ ] **初期方針**: `cargo vet init` 後に全依存を一括 `exempt` するか、重要度/到達性で優先順位付けするか（exempt は「未監査の既知状態」であり「監査済み」ではない）。
- [ ] **監査基準（criteria）**: `safe-to-run` / `safe-to-deploy` 等の基準をどう使い分けるか。デスクトップアプリ + ローカルエージェント + wasm の性質を反映する。
- [ ] **更新運用**: Dependabot/手動更新時に `cargo vet` を通す担当・手順・CI 失敗時の対応（exempt 追加の可否とレビュー要件）。
- [ ] **CI 統合**: どのジョブ/タイミングで `cargo vet` を必須化するか（ローカル未導入期間との整合）。
- [ ] **git/path 依存ポリシー**: 将来追加時の扱い（原則回避、必要時のレビュー/固定手順）。

## 4. 段階的導入案（案・未実施）

1. **準備**: `cargo-vet` を開発環境/CI に導入しバージョンを固定。`supply-chain/` 用ディレクトリ構成を決める。
2. **初期化（方針確定後）**: `cargo vet init` 等で雛形生成。imports を採用する場合は採用元を明示して最小限から試行。
3. **棚卸し可視化**: `cargo vet` の未監査一覧をもとに、直接依存・高リスク領域（ネットワーク、プロセス制御、暗号、unsafe 相当の FFI 境界）から優先度付け。
4. **記録**: 実際にレビューしたものだけ `audit` として記録。未レビューは `exempt`/保留として扱い、理由と期限方針を残す。**捏造しない**。
5. **CI/運用**: `cargo-deny`/`cargo-audit` と並行運用し、更新フローに vet 確認を組み込む。
6. **継続改善**: 依存更新ごとに差分を小さく保ち、git 依存削減やピン更新のレビューを継続。

## 5. リスクと留意点

- パッケージ数が多く（809）、初期コストが高い。形式的な exempt 量産は安心感だけを与えうるため、目的（何を保証したいか）を先に定義する。
- 現時点で git/非workspace path 依存は 0 件だが、将来追加された場合は registry 前提の imports だけでは扱えない可能性がある。
- `cargo-vet` は `cargo-audit`/`cargo-deny` と目的が重なる部分と異なる部分がある（既知脆弱性 DB、ポリシー/ライセンス/ban、監査連鎖）。置換ではなく補完として位置づける。

## 6. 次の具体アクション候補（安全・小さく進める）

- 開発環境への `cargo-vet` 導入可否（バージョン固定方針）を決める。導入時は `Cargo.lock` と同じコミットで再現できるようにする。
- trust/imports の候補を比較し、採用基準を短く文書化する（信頼根拠・更新頻度・criteria 対応・自組織方針との整合）。
- `cargo metadata` から依存・git/path 依存の一覧を定期確認する手順を維持する（監査判断そのものは自動化しない）。
- 初期導入を行う場合でも、未レビュー依存を `exempt` として明示し、`audit` と混同しない。

## 7. 棚卸し再取得手順（事実確認用）

`Cargo.lock` 更新後や導入検討時に再実行し、数値を更新する。

```bash
cd /home/koppe/document/deepseek-harness/default-workspace/work/gpui-task-manager
cargo metadata --format-version 1 > /tmp/gpui-tm-metadata.json
python3 - <<'PY'
import json
with open('/tmp/gpui-tm-metadata.json') as f:
    d = json.load(f)
pkgs = d['packages']
ws = set(d['workspace_members'])
kinds = {}
path_ws = path_nonws = git = 0
for p in pkgs:
    src = p.get('source')
    if src is None:
        kinds['path'] = kinds.get('path', 0) + 1
        if p['id'] in ws:
            path_ws += 1
        else:
            path_nonws += 1
    elif src.startswith('registry+'):
        kinds['crates-io'] = kinds.get('crates-io', 0) + 1
    elif src.startswith('git+'):
        kinds['git'] = kinds.get('git', 0) + 1
        git += 1
    else:
        kinds[src] = kinds.get(src, 0) + 1
print('total', len(pkgs))
print('kinds', kinds)
print('path_workspace', path_ws, 'path_nonworkspace', path_nonws, 'git', git)
PY
```

## 8. 導入判断チェックリスト（未決事項の具体化）

- [ ] **目的**: cargo-vet で何を補完したいか（監査連鎖の可視化、更新時の差分確認補助、imports 活用など）を1文で定義する。
- [ ] **trust/imports 評価**: 候補の公開 audits を採用する場合、運営主体・更新頻度・criteria の意味・自組織の受容範囲を確認し、採用/不採用理由を記録する（盲目的採用しない）。
- [ ] **初期状態**: `init` 後の未監査依存をどう表現するか（exempt の粒度、優先順位、期限/見直し方針）を決める。
- [ ] **git/path 方針**: 現時点 0 件だが、追加時の例外手順（固定リビジョン、差分レビュー、記録場所）を事前定義する。
- [ ] **CI/ローカル運用**: `cargo vet` 実行タイミング、失敗時の対応、Dependabot/手動更新フローとの接続を決める。
- [ ] **証跡の健全性**: 実レビューなしに `audit` を作らない。imports は委譲として扱い、採用根拠を残す。

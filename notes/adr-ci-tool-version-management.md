---
type: ADR
title: CI ツール版は手動ピンを維持する（mise.toml 一元管理は未実装）
description: CI 内の固定ツール版と Dependabot の対象範囲を記録し、将来の一元管理に向けた移行手順をメモとして残す。
tags: [adr, ci, dependabot, toolchain, supply-chain]
generated: 2026-11-19T05:24:39+09:00
status: approved
---

## 1. 現状（Context）

### 1.1 CI 内の固定ツール版

`.github/workflows/ci.yml` に固定されているツール版は次の 7 件である（いずれも 2026-11-19 時点）。

- `cargo-vet` 0.10.3 — `.github/workflows/ci.yml:56`（`cargo install cargo-vet --version 0.10.3 --locked`）
- `cargo-machete` 0.10.1 — `.github/workflows/ci.yml:60`（`cargo install cargo-machete --version 0.10.1 --locked`）
- `markdownlint-cli` 0.50.0 — `.github/workflows/ci.yml:71`（`npm install --global markdownlint-cli@0.50.0`）
- `gitleaks` 8.30.1 — `.github/workflows/ci.yml:85`（`GITLEAKS_VERSION=8.30.1`、公式 tarball を checksum 検証して取得）
- `cargo-deny` 0.21.0 — `.github/workflows/ci.yml:218`（`taiki-e/install-action` の `tool: cargo-deny@0.21.0`）
- `trunk` 0.22.1 — `.github/workflows/ci.yml:274`（`tool: trunk@0.22.1`）
- `cargo-cyclonedx` 0.5.10 — `.github/workflows/ci.yml:301`（`tool: cargo-cyclonedx@0.5.10`）

### 1.2 Dependabot の対象範囲

- `.github/dependabot.yml:16` と `:45` が `cargo`（`/` と `/web`）、`:73` が `github-actions`（`/`）を対象にする。週次で月曜に実行される。
- 上記 7 ツールは `cargo install` / `npm install --global` / リリース tarball の直接取得で導入されており、Dependabot の更新対象には含まれない。したがって自動更新 PR は来ず、**手運用**である。
- `.github/workflows/ci.yml:26` の再利用可能ワークフロー参照と、各ジョブの `actions/checkout` 等は SHA 固定で運用されている（例: `.github/workflows/ci.yml:112`）。github-actions は Dependabot 対象だが、SHA 固定の運用方針は手動確認を伴う。
- リポジトリ内に `mise.toml` は存在せず、ツール版の単一情報源は現状 `ci.yml` 内の上記 7 箇所である。

## 2. 決定（Decision）

7 ツールの版を `ci.yml` 内に直接ピンする現状を維持し、`mise.toml` 等による一元管理は **導入しない（未実装）**。§5 は将来の移行手順のメモであり、実行済みの変更ではない。

## 3. 現状維持の理由

- **監査証跡の近接性**: 版の数字が実際に使うコマンドの直近にあり、`gitleaks` の checksum 検証（`.github/workflows/ci.yml:85-106`）まで同じブロックで読める。レビュー時に版と検証方法を別途追う必要がない。
- **Dependabot との非互換**: 現在の設定は cargo と github-actions のみで、`cargo install` や npm グローバル、tarball の版を追跡する仕組みが無い。mise を入れても自動更新が増えるわけではなく、更新は結局手動になる。
- **共通ワークフローへの委譲**: `gates` ジョブは `Seduce0068/workflows` の `rust-gates.yml` へ委譲している（`.github/workflows/ci.yml:26`）。mise で版を統一するなら、各ジョブと共通ワークフロー側の両方に渡す経路を設計する必要があり、移行期は二重管理になる。
- **supply-chain 要件との整合**: R2 の要件は明示的な版固定とチェックサム検証をゲートの前提としている。情報源を `mise.toml` へ移す過程で、この明示性が一時的に薄くなるリスクを避ける。

## 4. リスクと限界

- 7 箇所の数字を手動で同期するため、更新漏れと版のドリフトが起きやすい。
- Dependabot が Cargo 依存や GitHub Actions を更新しても、ツール版は据え置きのままになる。
- セキュリティ修正を含む新版が出ても、手動レビュー待ちで適用が遅れる可能性がある。
- `cargo-vet` と `cargo-machete` は `cargo install` のため、CI 実行ごとにビルド時間がかかる（キャッシュ戦略は未検討）。

## 5. 将来 mise.toml 等で一元管理する場合の移行手順（メモ・未実装）

実装は行わない。着手時に次の順で検討する。

1. リポジトリ直下に `mise.toml` を追加し、`[tools]` に上記 7 件を正確な版で列挙する。例: `cargo-vet = "0.10.3"`、`cargo-machete = "0.10.1"`、`markdownlint-cli = "0.50.0"`、`gitleaks = "8.30.1"`、`cargo-deny = "0.21.0"`、`trunk = "0.22.1"`、`cargo-cyclonedx = "0.5.10"`。
2. 各ツールが mise のレジストリで提供されているかを実装時に確認する（未提供なら `aqua` 等との併用またはカスタムバックエンドが必要）。未提供のツールは現状の直接取得を残す。
3. 情報源を 1 つに決める。`mise.toml` を正とし、`ci.yml` は版を直接持たず mise 経由で解決する（`mise exec -- <tool>` または `mise which <tool>`）。移行期のみ両方を記載し、切替完了後に直接ピンを削除する。
4. CI に mise を導入する。導入アクションは再利用可能ワークフロー側の対応も検討し、外国製アクションは SHA 固定を維持する。既存の `taiki-e/install-action` の `tool:` 入力を mise 経由へ置き換える。
5. `gitleaks` の checksum 検証（`.github/workflows/ci.yml:85-106`）は mise 取得後も残す。取得元が変わっても検証を省略しない。
6. ローカル開発手順を `README.md` / `AGENTS.md` に追記し、`mise install` を標準化する。開発者の既存環境との差分は `mise current` 等で確認する。
7. 検証: 7 ツールすべてで `--version` が `mise.toml` の版と一致すること、`cargo vet` / `cargo machete` / `markdownlint` / `gitleaks detect` / `cargo deny` / `trunk build` / SBOM 生成が従前どおり通ることを確認する。
8. Dependabot が `mise.toml` を更新できるかは現状未確認である。自動更新を前提にせず、手動レビュー運用を明記する。
9. 完了時に本 ADR を `superseded` とし、一元管理の採用理由・検証結果・ロールバック手順を新しい ADR に記録する。

## 6. 代替案と不採用理由

- **mise.toml を今すぐ導入**: 情報源が 2 つに分かれ、共通ワークフロー側の設計が未定のため不採用（§3）。
- **Dependabot にツール版の追跡を期待する**: 現行設定は cargo / github-actions のみで、`cargo install` 等は対象外。設定を拡張しても追跡できる保証が無いため不採用。
- **版固定を外して latest を使う**: supply-chain 要件（明示版とチェックサム検証）に反し、再現性も失われるため不採用。

## 7. 影響（Consequences）

- 正: 版と検証方法が `ci.yml` の同じ箇所にあり、レビューと監査が容易。supply-chain ゲートの明示性が保たれる。
- 負: ツール更新は手作業であり、7 箇所の同期漏れに注意が必要。
- 負: 一元管理の準備が未完了のため、§5 の手順は着手時に必ず見直す（ツールの提供状況と Dependabot 対応は未確認）。

## 8. 参照

- `.github/workflows/ci.yml:56`、`:60`、`:71`、`:85`、`:218`、`:274`、`:301` — 固定ツール版
- `.github/workflows/ci.yml:26` — 再利用可能ワークフローの SHA 固定
- `.github/dependabot.yml:16`、`:45`、`:73` — cargo / github-actions のみを対象とする設定
- `notes/cargo-vet-adoption.md` — cargo-vet の固定方針（0.10.3）
- `notes/dependency-audit.md` — 依存監査手順

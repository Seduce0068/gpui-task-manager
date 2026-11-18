---
type: Reference
title: cargo-vet 導入メモ（未監査ベースラインによる drift 検出、root/web 統一）
description: gpui-task-manager における cargo-vet（supply-chain 監査）の導入状況、運用方針、段階的レビュー計画。2026-11-18 に root/web とも baseline-unreviewed 方針へ統一。監査記録は未作成（全依存を exempt として明示）。
tags: [supply-chain, cargo-vet, security, dependencies]
status: adopted-initial
generated: 2026-11-15T14:42:00+09:00
verified: 2026-11-18T11:36:00+09:00
---

## 1. 現状（事実）

- **導入状態**: 初期化済み。`cargo-vet 0.10.3`（`cargo install cargo-vet --version 0.10.3 --locked`、2026-11-15）で `cargo vet init` を root / web に実行し、`supply-chain/`（`config.toml` / `audits.toml` / `imports.lock`）を作成。2026-11-18 に root / web とも「未監査ベースラインによる drift 検出」方針へ統一し、全 exempt の `criteria` を `baseline-unreviewed`（root 804 / web 560）に変更、`[policy.<first-party>]` と `[criteria.baseline-unreviewed]` / `[criteria.audited]` を両ストアへ追加した。外部 imports は未使用（`imports.lock` 空）、監査記録なし（`audits.toml` の `[audits]` 空）。`baseline-unreviewed` は「未監査の明示」であり監査済みを意味しない。
- **代替/併用中の供給網対策**:
  - `cargo audit` / `cargo-deny`（advisories/bans/licenses/sources、root/web）。CI の deny ジョブは `-A advisory-not-detected` を指定し、root `deny.toml` の ignore が web グラフに現れない場合のノイズ警告を抑制（2026-11-15、`0ee7c37`）。
  - CycloneDX SBOM: [`scripts/generate-sbom.sh`](../scripts/generate-sbom.sh) と CI `sbom` ジョブ
  - Dependabot（Cargo root/web、GitHub Actions、週次）とセキュリティアラート
  - GPUI 系・主要依存の完全ピン運用と `Cargo.lock` コミット
- **方針の明文化**: `README.md` / `SECURITY.md` / `notes/verification.md` と本メモに「trust 方針・棚卸し・運用手順を定めてから導入し、未監査依存は `baseline-unreviewed` として記録して監査済みと扱わない」を記載済み（2026-11-18 に root / web 統一）。

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

## 3. 導入方針（決定済みと残課題）

- [x] **trust root**: 自リポジトリの `supply-chain/` を唯一の trust root とする。外部 imports（Mozilla/Google/Embark 等の公開 audits）は初期状態では使わない（`imports.lock` 空）。imports は「信頼の委譲」であり、将来採用する場合は §8 の評価軸で候補を確認してからにする。
- [x] **初期方針**: `cargo vet init` により全依存を一括 `exempt` として記録（root 804 / web 560）。2026-11-18 に両ストアの `criteria` を `baseline-unreviewed` へ統一。`baseline-unreviewed` は「未監査の明示」であり「監査済み」ではない。以後は実レビューできた依存から `audits.toml` に記録する。
- [x] **監査基準（criteria）**: 既定の `safe-to-deploy` ではなく、プロジェクト固有の `baseline-unreviewed` を root / web 両ストアで採用（2026-11-18 統一）。`audits.toml` に `[criteria.baseline-unreviewed]`（未監査ベースラインの記録専用で安全性を主張しない）と `[criteria.audited]`（`implies = ["safe-to-deploy", "baseline-unreviewed"]`、実レビュー用ラベル）を定義。`[policy.<first-party>]` で各 first-party crate（root: `gpui-task-manager` / `tm-agent` / `tm-core` / `tm-platform` / `tm-viewmodel`、web: `tm-core` / `tm-viewmodel` / `tm-web`）の依存基準も `baseline-unreviewed` とし、未監査 baseline からの drift を検出する。実監査を記録する場合は `audited`（`safe-to-deploy` を含む）を使い、`baseline-unreviewed` と混同しない。
- [x] **更新運用**: Dependabot/ピン更新でバージョンが変わると `cargo vet --locked` が失敗する。対応は ①実レビュー（`cargo vet inspect` / `diff`）後に `certify`（`audited` を付与）、②レビュー前なら理由を明記した `add-exemption`（`baseline-unreviewed`）。監査記録の捏造はしない。
- [x] **CI 統合**: 共通 `rust-gates.yml@v1` の `extra_gates` 入力で `cargo vet --locked`（root / web）を必須化（共通ワークフロー側の変更は不要）。`cargo-vet` は `taiki-e/install-action` 未対応のため `cargo install cargo-vet --version 0.10.3 --locked` で導入。ローカルは `scripts/check.sh` に optional ゲートとして追加（cargo-vet 未導入環境では skip）。
- [x] **git/path 依存ポリシー**: 原則回避（現状 git 0 件 / 非workspace path 0 件）。追加時は固定リビジョン/タグ・差分レビュー・手動記録で補う。

## 4. 段階的導入状況

1. [x] **準備**: `cargo-vet 0.10.3` を開発環境に導入（バージョン固定・`--locked`）。`supply-chain/` はリポジトリ管理（root / web）。
2. [x] **初期化**: `cargo vet init`（root / web、2026-11-15）。imports は未採用。
3. **棚卸し可視化**: `cargo vet` の未監査一覧をもとに、直接依存・高リスク領域（ネットワーク、プロセス制御、暗号、unsafe 相当の FFI 境界）から優先度付け。
4. **記録**: 実際にレビューしたものだけ `audit` として記録。未レビューは `exempt`/保留として扱い、理由と期限方針を残す。**捏造しない**。
5. **CI/運用**: `cargo-deny`/`cargo-audit` と並行運用し、更新フローに vet 確認を組み込む。
6. **継続改善**: 依存更新ごとに差分を小さく保ち、git 依存削減やピン更新のレビューを継続。

## 5. リスクと留意点

- パッケージ数が多く（809）、初期コストが高い。形式的な exempt 量産は安心感だけを与えうるため、目的（何を保証したいか）を先に定義する。
- 現時点で git/非workspace path 依存は 0 件だが、将来追加された場合は registry 前提の imports だけでは扱えない可能性がある。
- `cargo-vet` は `cargo-audit`/`cargo-deny` と目的が重なる部分と異なる部分がある（既知脆弱性 DB、ポリシー/ライセンス/ban、監査連鎖）。置換ではなく補完として位置づける。

## 6. 次の具体アクション候補（安全・小さく進める）

- CI 統合は実施済み（`extra_gates` で `cargo vet --locked`、root / web）。初回 CI 実行結果を確認し、失敗があれば修正する。
- 実レビューの最初の対象: 直接依存・高リスク領域（`rustix`、`sysinfo`、`libc`、`serde`、`tm-agent` の HTTP/サーバ系依存）から `cargo vet inspect` / `diff` で確認し、`certify` を記録する（ネットワークが必要なためレビュー環境で実施）。
- trust/imports 候補の一次調査は実施済み（§8.1、採否未決）。次は主要依存へのカバレッジ確認（ネットワーク環境で `cargo vet suggest` / `import` を試行）と、criteria の意味・受容範囲の評価を行う。
- `cargo metadata` から依存・git/path 依存の一覧を定期確認する手順を維持する（監査判断そのものは自動化しない）。

## 7. 棚卸し再取得手順（事実確認用）

`Cargo.lock` 更新後や導入検討時に再実行し、数値を更新する。

```bash
cd .   # リポジトリのチェックアウト先
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

## 8. 導入判断の比較軸（trust/imports 候補評価・未決）

本節は導入時の判断材料であり、特定の trust root / imports 元の採用を決定・推奨するものではない。採用前に一次情報（各プロジェクトの公開 `supply-chain/`、criteria 定義、更新履歴、利用条件）を確認する。

| 観点 | 確認項目 | 記録すべき判断 |
|---|---|---|
| 目的 | 何を保証したいか（既知脆弱性対応の補完、監査連鎖、供給網変更の可視性、リリース要件など） | `cargo-audit`/`cargo-deny`/SBOM/Dependabot と重複・補完する範囲 |
| Trust root | 自組織/個人の audits を主とするか、外部 imports を併用するか | 信頼の根拠、責任主体、レビュー可能性 |
| Imports 候補 | 公開 audits の有無、対象エコシステム、criteria（`safe-to-run`/`safe-to-deploy` 等）の意味、更新頻度、カバレッジ | 採用する場合のバージョン/コミット固定方針と更新確認手順。盲目的な委譲を避ける |
| Criteria | 本プロジェクトに必要な基準（デスクトップ UI、ローカル loopback agent、wasm/web 配布物、プロセス制御を含む） | 使用する criteria と、`audit`/`exempt`/`unaudited` 相当の扱い |
| 初期状態 | 全件 `exempt` から始めるか、直接依存・高リスク依存から段階的に記録するか | 期間、優先順位、レビュー担当、期限方針。`exempt` は未監査の明示であり監査済みではない |
| 更新運用 | Dependabot/手動更新時の `cargo vet` 実行、差分確認、記録更新、CI 失敗時の対応 | PR チェック項目、緊急更新時の例外条件 |
| git/path 依存 | 将来の git/path 依存追加時の扱い | 原則回避、必要時は固定リビジョン・差分レビュー・手動根拠記録 |
| 証跡健全性 | 実レビューなしに `audit` を作らない、imports の意味を理解した上で採用する | 記録者、根拠、再確認トリガー |

### 比較作業の最小手順（未実施）

1. 候補ごとに一次情報の所在（公開 `supply-chain/config.toml`/audits、criteria 文書、更新実績）を確認する。
2. 本プロジェクトの依存構成（registry/git/path、直接依存、高リスク領域）との適合を評価する。
3. 採用する場合でも、`cargo-vet` ツールバージョン・imports 参照・自組織方針を固定/記録する。
4. 初期導入 PR では `cargo vet init`/設定/運用文書/CI 方針を分離し、未レビュー依存の監査済み化を行わない。

**2026-11-18 時点の状況**: root / web とも未監査ベースライン（`baseline-unreviewed`）による drift 検出方針へ統一済み（全依存 exempt、root 804 / web 560、imports 未使用、監査記録なし、両ストアで `cargo vet --locked` 成功）。trust/imports 候補の採否は未決で、上記は将来の評価軸。実レビューなしの `audit` 作成は行わない。

### 8.1 外部 imports 候補の一次調査（事実確認のみ・採否未決、2026-11-15）

GitHub API（一次情報）による確認結果。採用判断はこの調査だけでは不十分で、criteria の意味・自プロジェクトの受容範囲・監査連鎖の評価後に `cargo vet import` / `trust` を検討する。当面は imports 未使用（`imports.lock` 空）のまま運用する。

| 候補 | 実在性・活動（GitHub API 確認） | 構成 / criteria | 備考 |
|---|---|---|---|
| [mozilla/supply-chain](https://github.com/mozilla/supply-chain) | 最終 push 2026-11-10、非アーカイブ | ルートに `audits.toml`（約 509 KB の aggregated audits）+ `sources.list`、Apache-2.0/MIT。README に他インスタンスへの import 手順あり | cargo-vet 本家（Mozilla）の公開監査集約 |
| [google/rust-crate-audits](https://github.com/google/rust-crate-audits) | 最終 push 2026-11-10、非アーカイブ | `audits.toml` に `crypto-safe` / `does-not-implement-crypto` / `safe-to-run` / `safe-to-deploy` / `ub-risk-*` 等の criteria 定義（ChromiumOS から `aggregated-from`）+ `auditing_standards.md`（criteria ごとの監査者要件） | 監査基準文書が充実。criteria 追加の参考になる |
| [zcash/rust-ecosystem](https://github.com/zcash/rust-ecosystem) | 最終 push 2026-07-12 | `supply-chain/audits.toml` + `sources.txt` | `cargo vet suggest` が既定 registry 経由で参照を試行した実績あり（本環境ではネットワーク制約で取得失敗） |
| EmbarkStudios | GitHub 検索で cargo-vet 用の公開 audits リポジトリを確認できず（個人 `ctrlaltf2/supply-chain` のみ該当） | — | 一次情報未確認。採用候補からは当面除外 |

- cargo-vet には「cargo-vet Registry」の仕組みがあり、`mini-vet 2.0.0`（crates.io、説明: "A client for the cargo-vet registry. Fetches security reviews for Rust/Cargo crates."）がクライアントとして存在する。`cargo vet suggest` は registry 経由で公開 audits を参照する（2026-11-15 実行時に `zcash` の registry audits 取得を試行）。
- 判断メモ: 現行方針は `safe-to-deploy` を要求する通常の監査ではなく、`baseline-unreviewed` による未監査ベースラインの drift 検出（2026-11-18 に root / web 統一）。Mozilla/Google の audits は `safe-to-deploy` 等の基準で付与されており、`baseline-unreviewed` を直接満たすとは限らない。imports を採用する場合は criteria の対応付け（`audited` 経由等）と `aggregated-from` の連鎖（信頼の委譲）を理解してから評価する。優先度は「自プロジェクトの主要依存がどの程度カバーされるか」の確認後（ネットワークが必要）。

## 9. ネットワーク環境での cargo-vet 試行手順（Runbook・未実施）

`cargo vet` の `suggest` / `inspect` / `diff` / `certify` / `import` / `trust` は crates.io・GitHub からの取得（ネットワーク）が必要。制限環境（本作業環境）では失敗するため、ネットワークが使える環境で次の順に試行する（2026-11-15 時点で未実施）。各サブコマンドの正確な引数は `cargo vet <command> --help` で確認すること。

```bash
cd <repo>

# 1. レビュー候補の一覧（registry 経由の公開 audits 参照も行う）
cargo vet suggest

# 2. 候補のソース確認・バージョン間差分（例: 直接依存・高リスク領域から）
cargo vet inspect rustix 1.1.6
cargo vet diff libc 0.2.171 libc 0.2.172

# 3. 実レビューできた場合のみ記録する
cargo vet certify rustix 1.1.6
cargo vet fmt

# 4. imports の評価試行（採用判断は §8.1 の評価軸で事前評価してから）
git switch -c try-import-mozilla
cargo vet import mozilla/supply-chain
cargo vet --locked
git --no-pager diff supply-chain/   # imports.lock / config.toml の差分を必ずレビュー
# 不採用なら: git switch main && git branch -D try-import-mozilla

# 5. 記録後の整合確認（root / web 両方）
cargo vet --locked
(cd web && cargo vet --locked)
```

注意:

- `certify` は自分が実際にレビューした場合のみ使う。判断できない場合は理由を明記した `add-exemption` のまま残す（捏造しない）。
- `import` は信頼の委譲。`imports.lock` への反映差分を必ずレビューし、採用時のみコミットする。
- web クレートは別ストアだが、方針の差分が出ないよう root / web とも `baseline-unreviewed` に統一済み（2026-11-18）。imports を採用する場合も両ストアで方針を揃え、差がある理由を本メモに記録する。

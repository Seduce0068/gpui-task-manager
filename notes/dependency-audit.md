---
type: Reference
title: 依存バージョン・feature 差分の監査手順
description: gpui-task-manager の重複バージョンと feature 差分依存を検出・レビューする手順。cargo-deny / cargo-vet の既存ゲートを壊さず、新規依存なしで運用する。
tags: [supply-chain, dependencies, cargo-deny, audit]
status: adopted
generated: 2026-11-18T12:19:21+09:00
verified: 2026-11-18T12:19:21+09:00
---

## 1. 目的と適用範囲

重複バージョン（同一 crate の複数バージョンが解決グラフに併存する状態）と feature 差分依存（同一 crate が経路やターゲットごとに異なる feature セットで有効化される状態）を、既存ゲートを壊さずに可視化・レビューするための手順。対象要求は R1-OPS-03 / R4-S04 / R1S-OPS-03。

対象ファイルは `Cargo.toml`、`deny.toml`、`notes/*.md` のみ。新規依存の追加、バージョン制約の変更、`multiple-versions = "deny"` への切替、`skip` / `skip-tree` 例外の追加など、既存の deny / vet ゲートを変える変更は本監査の範囲外とする。

## 2. 前提となる既存ゲート

変更前後で同じ結果になることを確認する baseline。数値は「6. 観測結果」を参照。

- `cargo deny check`（root）: advisories / bans / licenses / sources がすべて ok
- `cargo deny --manifest-path web/Cargo.toml check`（web）: 同上
- `cargo vet --locked`（root / web）: `Vetting Succeeded (804 exempted)`（web は 560 exempted、`notes/cargo-vet-adoption.md` 参照）

`deny.toml` の `[graph] targets` は Linux / Windows / macOS / wasm の 4 triple をカバーし、`[output] feature-depth = 1` が feature 表示の深さを制御する。`[bans]` は `multiple-versions = "warn"`、`workspace-default-features = "allow"`、`external-default-features = "allow"` の既存方針を維持する。

## 3. 重複バージョンの監査手順

1. ゲート結果を取得する。`cargo deny check bans` は重複を `warning[duplicate]` として表示しつつ、終了コード 0（`bans ok`）を返す。ここで "deny" に変えない。
2. 重複の一覧と原因を取得する。

   ```bash
   cargo tree -d --prefix none                 # 重複バージョンと依存経路
   cargo tree -d --prefix none \
     | grep -E '^[A-Za-z0-9_-]+ v[0-9]' \
     | grep -v '(\*)' | sort -u                # 重複エントリの一意化
   ```

3. 新規重複が増えた場合は、どの member / 推移依存がどのバージョンを要求しているかを `cargo tree -i <crate> <version>` で追い、理由（ピン済みスタック由来で代替なし、など）をレビュー記録に残す。既知の重複は `notes/cargo-vet-adoption.md` と `DESIGN.md` §14 のsupply-chain 方針と突合する。
4. 重複を解消する必要が生じた場合は、別タスクでバージョン制約の更新を提案する。本監査タスクでは制約を変更しない。

## 4. feature 差分の監査手順

cargo-deny の `bans` はクレート単位の重複・許可リストを検査し、feature 単位の差分は検出しない。feature 差分は次の手順で可視化する。

1. 指定クレートに至る feature 経路を表示する。

   ```bash
   cargo tree -e features -i <crate> --prefix none
   ```

2. 解決グラフ上の有効 feature を `cargo metadata` で確認する。次のスクリプトは同一名の複数バージョンで feature セットが異なるケースを抽出する（読み取り専用）。

   ```bash
   cargo metadata --format-version 1 --locked > /tmp/gpui-tm-metadata.json
   python3 - <<'PY'
   import json
   with open('/tmp/gpui-tm-metadata.json') as f:
       d = json.load(f)
   pkgs = {p['id']: p for p in d['packages']}
   seen = {}
   for node in d['resolve']['nodes']:
       p = pkgs[node['id']]
       seen.setdefault(p['name'], []).append(
           (p['version'], tuple(sorted(node['features'])))
       )
   for name, entries in sorted(seen.items()):
       if len(entries) > 1:
           print(name, entries)
   PY
   ```

3. ターゲット差を確認する。`deny.toml` の `[graph] targets` と同じ 4 triple で feature 経路を比較する。cfg 依存の feature（例: Linux の `wayland` / `x11`、Windows の `windows`）は triple を変えないと見えない。

   ```bash
   for triple in x86_64-unknown-linux-gnu x86_64-pc-windows-msvc \
                 aarch64-apple-darwin wasm32-unknown-unknown; do
     echo "== $triple =="
     cargo tree -e features --target "$triple" -i <crate> --prefix none | head -n 40
   done
   ```

4. feature の追加・削除が生じる変更は、差分前に `cargo tree -e features -i <crate>` を保存し、変更後と比較して意図した差分かをレビューする。`features` を追加した member がある場合は `Cargo.toml` の `[workspace.dependencies]` コメント方針（shared version の一元管理）と突合する。

## 5. 変更ルールとゲート非破壊

- 新規依存・破壊的変更は禁止。バージョン制約、`features`、`default-features` の既存値を変更しない。
- `deny.toml` の既存ルール値・ignore・licenses・sources は変更しない。本タスクで追加したのはコメントのみ。
- `supply-chain/` の audits / exempt / criteria は変更しない。既存の `baseline-unreviewed` 方針を維持する。
- 監査の結果、ゲート失敗や新規警告が出た場合は、原因を記録して別タスクで対応する。警告を抑止する設定追加は行わない。
- `Cargo.lock` は更新しない（`cargo tree` / `cargo metadata` は読み取りのみ）。

## 6. 観測結果（Runtime Proof）

2026-11-18T12:19:21+09:00 に作業ディレクトリ `gpui-task-manager` で取得。

- ツール: `cargo 1.100.0`、`rustc 1.100.0`、`cargo-deny 0.21.0`、`cargo-vet 0.10.3`、`markdownlint`（npm グローバル）。
- `cargo deny check`（変更前）: `advisories ok, bans ok, licenses ok, sources ok`、終了コード 0。
- `cargo deny --manifest-path web/Cargo.toml check`（変更前）: `advisories ok, bans ok, licenses ok, sources ok`、終了コード 0。
- `cargo vet --locked`（root・変更前）: `Vetting Succeeded (804 exempted)`、終了コード 0。
- `cargo vet --locked`（web・変更前）: `Vetting Succeeded (560 exempted)`、終了コード 0。
- 変更後の再実行でも root / web の `cargo deny check` と `cargo vet --locked` は上記と同一の結果（すべて終了コード 0）だった。
- `cargo tree -d --prefix none`: 重複エントリ 317 行、複数バージョンを持つ crate 名 35 個。
- 代表例 `sysinfo`: `0.40.3` は `default-features = false` で `component` / `disk` / `network` / `system` / `user`（`crates/tm-platform/Cargo.toml` の直接依存）。`0.31.4` は推移依存で `default` / `multithread` / `windows` を含む。同じ crate 名でバージョンと feature セットが異なるため、両方の観点でレビュー対象となる。
- 参考 `serde 1.0.229`: 有効 feature は `alloc` / `default` / `derive` / `rc` / `serde_derive` / `std` に解決される（`[workspace.dependencies]` の `derive` と推移依存の `default` が統合された結果）。
- `markdownlint 'notes/*.md'`（変更前）: 既存警告 353 行（MD012 2 件、MD024 7 件、MD060 344 件）。これらは既存ファイルのbaselineであり、本タスクでは修正しない。

## 7. ネットワーク失敗時の扱い

`cargo deny check` の advisories は RustSec advisory DB の取得・更新に network を必要とする。今回の検証では network 失敗は発生せず、advisories は ok だった。network が使えない環境では次のように記録する。

1. `cargo deny check` のエラー出力（DB fetch 失敗の旨）をそのまま報告に含める。
2. network 不要な `cargo deny check bans licenses sources` と `cargo vet --locked` を別途実行し、それぞれの終了コードを記録する。
3. advisories の結果は「未検証（network 失敗）」と明記し、成功したと書かない。
4. `cargo vet suggest` / `inspect` / `diff` / `certify` / `import` も network 依存のため、失敗時は `notes/cargo-vet-adoption.md` §9 の Runbook に委譲し、監査記録の捏造を避ける。

## 8. 関連資料

- [`notes/index.md`](index.md) — 調査・検証ノートの索引
- [`notes/cargo-vet-adoption.md`](cargo-vet-adoption.md) — supply-chain 監査の trust 方針・棚卸し・Runbook
- `deny.toml` — bans / advisories / licenses / sources の既存ゲート定義
- `Cargo.toml` — shared version と feature の方針コメント

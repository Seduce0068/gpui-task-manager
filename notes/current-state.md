---
type: Reference
title: GPUI Task Manager 現状サマリ
description: 2026-11-20 時点の実装状況・検証結果・未解決事項の俯瞰。時系列の詳細は log.md、個別の根拠は verification.md を参照
tags: [status, verification, runtime-proof, gpui, task-manager]
resource: .
generated: 2026-11-17T03:46:00+09:00
verified: 2026-11-20T08:08:37+09:00
status: verified
---

## 目的

この文書は、時系列で追記される `log.md` と個別の Runtime Proof を記録する
`verification.md` を補い、**現時点の到達点を一覧できる**ようにするものです。
詳細な根拠は各文書を参照してください。

## プロジェクト概要

- Ubuntu 系デスクトップ向けのシステムタスクマネージャー試作（GPUI / Rust）。
- ネイティブ版（Wayland、Kubuntu 2026.04 で実機検証）とブラウザ版
  （`gpui-pre-web` / wasm、読み取り専用）を持つ。
- ドメイン・表示ロジックは `tm-core` / `tm-viewmodel` に集約し、
  シェル（`app/` / `web/`）は描画とコマンド発行のみを行う 3 層構成。
- ライセンス AGPL-3.0-or-later、全 crate で `#![forbid(unsafe_code)]`。

## 検証状況（2026-11-20）

| 項目 | 結果 |
| --- | --- |
| `cargo test --workspace` | **668 passed / 0 failed**（tm-core 176 / tm-viewmodel 94 / tm-platform 44 / tm-agent 153 / app 120、2026-11-20 実測） |
| web ユニットテスト | **232 passed / 0 failed**（fetch 制限 / redaction / プリセット / 電源 / SSE / 行メニュー / オンボーディング等を含む、2026-11-20 実測） |
| 最新のテスト追加（5b5ac04） | ネイティブ AX role 付与 7 テスト（app 103 → 110）と CSV cpu_percent 2 桁丸め（tm-core 175 → 176）の 8 件（workspace 519 → 527 → 531 → 532 → 534、web 232 不変、2026-11-20 実測）。直近以前は 2ba4525 の 6 件（workspace 509 → 519、web 167 → 169） |
| `cargo clippy --workspace --all-targets -- -D warnings -D clippy::undocumented_unsafe_blocks` | 成功（警告 0、root / web とも） |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | 成功（警告 0、root / web とも） |
| `cargo fmt --all --check` | 成功 |
| wasm check | tm-core / tm-viewmodel / web / poc-wasm の 4 ターゲット成功 |
| `trunk build --release` | 成功（wasm 7,949,118 B / gzip 3,165,445 B、2026-11-20 計測） |
| 依存監査 | `cargo audit`（脆弱性 0・既知 8 件のみ）、`cargo deny`（全カテゴリ ok）、`cargo vet`（root 804 / web 560 exempted）、`cargo machete`（未使用 0 件） |
| CI | 現行定義は 10 ジョブ（gates / wasm / msrv-1-95 / cdp-verify / native-windows / native-macos / cargo-deny / release-linux / web-dist / sbom、2026-11-19 定義再確認）。旧記録の「全 8 ジョブ success」は定義変更前のもので、10 ジョブの再実行は未確認 |
| CDP ブラウザ検証 | 設定・デモ・IME（32/32）すべて pass |
| ブラウザ互換性 | Chrome 158 / Edge 157 / Firefox 160 で実動確認成功（2026-11-20 実測。Firefox は WebDriver BiDi、F1 は合成キーのみで信頼済みキー未確認）。Safari / Brave / モバイル / 実 GPU WebGPU は未検証 |
| シェル静的検証 | 4 本すべて `bash -n` / `shellcheck` / `shfmt -d -i 2` 成功 |
| SBOM 生成 | `generate-sbom.sh` で 6 件の CycloneDX 1.3 JSON を生成・jq 検証（2026-11-17） |

### 実測パフォーマンス（x2-perf, 2026-11-19）

敵対的レビュー／実測ラウンドで取得したローカル計測値。トークン値や内部パス
は含めない。いずれも loopback 上の tm-agent / ブラウザ版が対象。

| 対象 | p50 | p95 |
| --- | --- | --- |
| `/healthz` | 0.142 ms | 0.249 ms |
| `/api/v1/snapshot` | 0.281 ms | 0.428 ms |
| SSE 初回ヘッダ | 0.236 ms | 0.998 ms |

- `--once` は 251 ms。デモ warm-up の待ちが全体の大部分を占める。
- 1000 行描画は行数に依存しない（SwiftShader 由来の描画コスト）。
- native GUI 起動は p50 809 ms。
- SSE スロットが切断後も最大 1 秒保持され、1 秒以内の再接続が 503 になる
  事象を確認。フレーム間の待ちを 100 ms スライスへ分割し、各スライスで
  ソケットを probe して切断を早期検知するよう修正（`agent/src/main.rs`）。

### ビルド再現性 — app バイナリ / ahash compile-time-rng（2026-11-20）

- **事象**: zed-xim 0.4.0-zed が ahash 0.8 を
  `features = ["compile-time-rng"], default-features = false` で固定依存し
  （公開 Cargo.toml:102-105 / Cargo.toml.orig）、ahash 0.8.12 の
  `get_fixed_seeds()` が `const_random!(u64)` × 8 をバイナリへ埋め込む
  （`src/random_state.rs:81-99`）。このため app バイナリの clean ビルドで
  SHA256 が変わっていた（mb-repro: 116 バイト差 = 乱数シード + GNU build-id）。
- **対処**: `.cargo/config.toml:44-64` に `[env] CONST_RANDOM_SEED`
  （`force = true`）を追加し compile-time シードを固定。const-random 0.1.18 の README
  「Deterministic builds」が公式に定義する環境変数で、
  const-random-macro 0.1.16 の `src/span.rs:8-17` が `option_env!` 経由で
  SHAKE-256 のシードとして使用する。上流変更・fork・依存 feature の変更・
  unsafe はすべて不要。
- **検証（2026-11-20、同一 `CARGO_TARGET_DIR` を clean して 2 回）**:
  `cargo build --release --locked -p gpui-task-manager` の SHA256 は両方
  `e0f257737dcb05e2fdbabfb52356a43a551f373807ab0dfe5092902255a260f0`。
  別々の target dir で行った 2 回のビルドでも ahash rlib は
  `c86e0c08b907c91f17e8574f7b840b5fae1a5148d3ebf85a71f2ebee5aae4a8e` で一致。
- **既知の制約**: 異なる `CARGO_TARGET_DIR`（例 `/tmp/oc-ahash-b1` と
  `/tmp/oc-ahash-b2`）間のバイナリは一致しない。rlib の依存パスと
  `OUT_DIR` 由来のソース位置が絶対パスとして残るため（実測 217,094 バイト差、
  ただし rodata の文字列の多重集合は同一）。CI の `/home/runner/...` は
  既存の `--remap-path-prefix` で `/build` へ置換されるため、公開ゲートの
  再利用ビルドには影響しない。再現性ゲートは同一パス上で clean ビルドを
  2 回実行して比較する必要がある。
- **不採用候補**: (a) zed-xim の feature オプション — 公開 Cargo.toml の
  `[features]` に無効化手段が無く、ahash 依存はハードコード。
  crates.io 上最新も 0.4.0-zed（2025-10-05 更新）で修正版なし。
  (b) app 側から `RandomState::with_seeds` を渡す — `const fn` だが
  zed-xim 内部の `AHashMap::with_hasher(Default::default())`
  （`zed-xim/src/lib.rs:54`, `zed-xim/src/server/connection.rs:630` 等）には届かない。
  ahash 0.8.12 に `FixedState` 型は存在しない。
  (c) `[patch.crates-io]` で feature を除去 — Cargo の feature は加算的で
  patch はソース差し替えのみ。ahash / zed-xim の fork が必要になり、
  担当範囲外かつ cargo-vet / cargo-deny の supply-chain gate に影響。
  (d) ahash を直接依存に追加して `runtime-rng` を unify — cfg 上は
  `get_fixed_seeds()` の分岐を切り替えられるが、workspace 全体の
  ahash 0.8 利用者へ及ぶ副作用と未使用依存（cargo machete）のリスクがあり
  不採用（未検証）。

## 主な対応済み事項

- **ブラウザ版ライトテーマ**: Catppuccin Latte の切替・保存・復元を実装。
  CDP で既定・保存・復元・ボタン切替を検証。
- **レイアウト**: 概要カードとツールバーの重なりを解消（固定高さ 186 → 260 px）。
- **エージェント**: 接続を最大 8 本まで並列化し、アイドルピアによる阻塞を解消。
  Drop ガード・503 の timeout・Mutex poison のログを追加。
  Host 検証 403 には許可 Origin のみ CORS を付与。
- **表示の安全性**: 制御文字・ANSI・bidi・ゼロ幅不可視文字を除去。
  NaN / 無限値、微小タイムスタンプ、履歴への非有限値混入を防御。
- **UI 安全性**: 確認ダイアログは Enter = キャンセル / Ctrl+Enter = 確定。
  モーダル overlay に `occlude()` を追加し背景クリックを遮断。
- **設定・依存**: 一時ファイル名に PID を付与して同時保存の競合を回避。
  未使用依存を削除し、`check.sh` に machete と `undocumented_unsafe_blocks` の
  ゲートを追加（root / web とも）。
- **テスト**: レビュー指摘のテスト欠落を補い、回帰テストを継続追加（履歴: 143 → 491 → 509 → 519 → 527 → 531 → 532 → 534、現在値は 668。件数は変動するため README.md「検証状況」を参照）。
- **成果物の `strings` 検査（開発パス混入検査）は実装済み**: `.github/workflows/ci.yml`
  の `release-linux` ジョブ（`Gate - no development paths in release binary` と
  `Gate - no development paths in DEB payload`）および `web-dist` ジョブ
  （`Gate - no development paths in web dist`）の各 `strings -a` Gate が、
  開発パス / アカウント識別子 / トークン様 hex を走査して一致時に失敗させる。
  `--remap-path-prefix` で置換された内部 worktree パス（`.agents/worktrees` 等）も
  検出対象（R1-RESIDUAL 反映）。

## 未解決・今後の候補

設計判断や外部環境を伴うため、本研究会では実装していない項目:

- `prefers-color-scheme` 連動（設定スキーマの「明示設定 vs 自動」を定義する必要がある）。
- 同形異義文字（Cyrillic `ѕ` 等）のサニタイズ（正当な名前を壊すリスクの判断が必要）。
- ネイティブ版の実 IME（IBus / fcitx5）手動確認（対話的操作とキー注入ツールが必要）。
- 431 / 400 / 503 の CORS 付与（リクエストが parse できない／未読のため Origin 不明）。
- Windows / macOS の物理実機検証（CI ランナーで代替中）。
- 優先度変更、GPU 詳細監視、DEB 署名・公開。
- 公開成果物（`.deb` / wasm dist / SBOM 等）に `SHA256SUMS` が未同梱。
  ダウンロード側で完全性を検証できるチェックサム manifest が生成・添付
  されていない（実装は本担当範囲外）。

## 関連文書

- 時系列: `log.md`
- 個別の Runtime Proof: `verification.md`
- 設計判断: `../DESIGN.md`
- セキュリティ方針: `../SECURITY.md`
- 独立レビュー: `review-2026-11-14.md`

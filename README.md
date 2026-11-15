# GPUI Task Manager

Ubuntu 系デスクトップ向けの**システムタスクマネージャー**試作です。
[GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui)（Zed の GPU
アクセラレート UI フレームワーク）を `gpui-kit 0.9.1` 経由で使い、プロセス・CPU・
メモリ・ネットワーク・ディスクを監視します。クロスプラットフォームと wasm /
ブラウザ対応を見据えた構成です。

![ネイティブ版（Wayland、実データ、--redact でホスト名等をマスク）](notes/assets/task-manager-native-wayland.png)

![ブラウザ版（GPUI Web / wasm、ライブデータ、--redact 適用）](notes/assets/web-shell-08-final-live.png)

- 第一ターゲット: **Kubuntu 2026.04 / KDE Plasma 6 / Wayland**（実機検証）
- スタック: Rust 1.92+ / `gpui-kit =0.9.1`（`gpui-pre 0.3.12`）/ `sysinfo =0.40.3` / `rustix =1.1.6`
- ライセンス: **Apache-2.0**（第三者表示は [NOTICE.md](NOTICE.md)）

## GitHub

- リポジトリ: <https://github.com/Seduce0068/gpui-task-manager>（現在 **private**）
- CI: [`.github/workflows/ci.yml`](.github/workflows/ci.yml)。Linux の gates は共通の
  `Seduce0068/workflows@v1` に委譲（fmt → check → test → clippy → cargo audit）。
  追加で wasm / cargo-deny / native-windows / native-macos / release-linux /
  web-dist / sbom を実行（Linux 系ジョブは再現性のため `ubuntu-24.04` に固定、
  成果物は `actions/upload-artifact@v7`）。SBOM は
  [`scripts/generate-sbom.sh`](scripts/generate-sbom.sh)（CycloneDX JSON、
  root/web ワークスペース）で生成。
- 運用: Dependabot（Cargo / web / GitHub Actions、週次）、セキュリティアラート、
  プロジェクト用ラベル（`area:*` / `ci` / `security` / `packaging` / `supply-chain` 等）
  を有効化。ブランチ保護は private リポジトリのプラン制約により API 上
  `403`（GitHub Pro または public が必要）で未設定のため、保護ルール相当は
  必須 CI / レビュー運用で補う。`cargo-vet` は trust/audit 方針と
  `supply-chain/` の初期化・インポート・監査運用を定めてから導入する（現時点では
  未導入、監査記録の捏造はしない）。SBOM（CycloneDX）は
  `scripts/generate-sbom.sh` と CI で生成する。
- 公開用の `main` はクリーンな履歴。開発履歴はローカルの `master` / `archive/master` に保持している。

## できること（MVP）

- プロセス一覧: 名前・コマンド・CPU・メモリ・PID・ユーザー・状態・スレッド数（ツリー表示トグル、`parent_pid` ベースの階層インデント）
- 概要カード: CPU（全体 + コア別 + 履歴）、メモリ / スワップ、ネットワーク
  受信・送信レート、ホスト / 稼働時間 / ディスク使用率、温度センサー（取得可能な場合）
- ソート（列クリック）、絞り込み（名前・コマンド・ユーザー・PID）、詳細パネル
- シグナル操作（SIGTERM / SIGKILL / SIGSTOP / SIGCONT）を**確認ダイアログ経由**で実行
- 安全装置: PID 1 と自分自身の拒否、PID + 起動時刻による PID 再利用検出
- キーボード操作: ↑↓ / PgUp/PgDn / Home/End / Space / Ctrl+F / Ctrl+K / F2 / F5 / F1
- **ブラウザ版 / ネイティブ版の日本語 IME（composition）対応**。変換中は未確定文字列を
  入力欄にのみ表示し、確定文字列だけを絞り込みへ反映
- `--demo` で決定的な合成データ起動（スクリーンショット・回帰用）
- **設定の永続化（ネイティブ / ブラウザ両方）**: 更新間隔・ソート列/方向・
  詳細パネルの表示・ツリー表示を保存し、次回起動時に復元。ネイティブは XDG 設定
  (`~/.config/gpui-task-manager/settings.json`、Windows/macOS も対応パス、
  原子的書き込み・0600・スキーマ版付き)、ブラウザは `localStorage`
  (`gpui-task-manager.settings`)。スキーマは `tm-viewmodel::Settings` で共有
- 依存監査（cargo-deny / cargo audit）と Dependabot による週次更新
- ブラウザ版（読み取り専用）: `web/` + `poc-wasm/`。ローカルエージェント経由で
  実データを表示（wasm / GPUI Web）

## クイックスタート（Kubuntu / Wayland）

```bash
# 1. 初回のみ: libxkbcommon-x11 のローカル shim（-dev パッケージが無いホスト向け）
bash scripts/dev-setup.sh

# 2. ビルドとテスト
cargo build -p gpui-task-manager
cargo test --workspace
bash scripts/check.sh        # fmt + clippy + test + wasm コンパイル確認を一括実行

# 3. 起動（実データ / 合成データ）
./target/debug/gpui-task-manager
./target/debug/gpui-task-manager --demo
```

前提パッケージ（Ubuntu 系）:

```bash
sudo apt install -y gcc g++ clang libfontconfig-dev libwayland-dev \
  libxkbcommon-x11-dev libx11-xcb-dev libssl-dev libzstd-dev libvulkan1
```

Vulkan 対応 GPU ドライバが必要です（GTX 1660 SUPER + NVIDIA proprietary で検証）。
`libxkbcommon-x11-dev` を入れられない場合は `scripts/dev-setup.sh` が
`app/native-libs/libxkbcommon-x11.so` を作成します（システム変更なし・root 不要）。

### 読み取り専用エージェント（ブラウザ用）

```bash
cargo run -p tm-agent -- --once --demo      # 1 スナップショットを JSON で出力
# 常用時はトークンを自動生成（stderr に 1 回だけ表示）、--token-file、または TM_AGENT_TOKEN を使う
TM_AGENT_TOKEN=test cargo run -p tm-agent -- --listen 127.0.0.1:18787
curl -s -H 'Authorization: Bearer test' \
  'http://127.0.0.1:18787/api/v1/snapshot?limit=5' | jq .
```

エージェントは `127.0.0.1` のみ bind、`Authorization: Bearer` 必須、読み取り専用、
CORS は localhost オリジンのみ許可します。`--token` はテスト専用です（argv は
同一マシンの他プロセスから読まれ得ます）。常用時は `--token-file` か `TM_AGENT_TOKEN`
を使ってください。

### wasm / ブラウザ

```bash
# ブラウザ版シェル（読み取り専用）
cd web && trunk build --release && trunk serve
# デモ表示:  http://127.0.0.1:8080/
# ライブ表示: http://127.0.0.1:8080/?agent=http://127.0.0.1:18787#token=<token>
#   （トークンは URL フラグメントで渡し、fetch の Authorization ヘッダで送る）

# GPUI Web の最小スパイク（Runtime Proof）
cd poc-wasm && trunk build --release
```

ブラウザ版は `tm-core` / `tm-viewmodel` をネイティブ版と共有し、GPUI Web
（`gpui-pre-web`、WebGPU → WebGL2 自動フォールバック）で描画します。
`tm-agent` に接続しない場合は決定的なデモデータで動作します。プロセス操作は
**提供しません**（読み取り専用）。wasm サイズは **7.24 MiB（gzip 2.89 MiB）**で、
`data-wasm-opt="z"` により wasm-opt（-Oz）を適用しています。日本語 IME は
`EntityInputHandler` + composition で対応済みです（[notes/ime.md](notes/ime.md)）。

## アーキテクチャ

```text
tm-core <── tm-viewmodel <── app (GPUI native) / web (GPUI wasm + demo/agent)
   ^                              ^
   +──── tm-platform (native) ────+   sysinfo + rustix（ネイティブのみ）
```

- **UI にロジックを置かない**: 表示状態は `tm-viewmodel` の reducer が所有し、
  シェルは `Command` を投げて `Effect` を実行するだけです。
- **wasm とネイティブで共通部品を共有**: 整形・サニタイズ・ソート/フィルタ・
  履歴・選択ロジックは `tm-core` / `tm-viewmodel` にあります。
- **安全性**: 全 crate で `#![forbid(unsafe_code)]`。シグナルは rustix の安全 API。
  PID 1 / 自分自身を拒否し、PID 再利用を検出します。

詳細は [DESIGN.md](DESIGN.md)（品質特性・データモデル・セキュリティ・ADR）、
調査は [RESEARCH.md](RESEARCH.md) を参照してください。

## リポジトリ構成

| パス | 内容 |
|---|---|
| `crates/tm-core/` | ドメイン型、CPU/ネットワーク差分、履歴、整形、サニタイズ、ソート/フィルタ、`SystemProbe` port |
| `crates/tm-viewmodel/` | `Dashboard` reducer、`Command` / `Effect`、確認ダイアログ状態、選択追従 |
| `crates/tm-platform/` | `NativeProbe`（sysinfo 収集、rustix シグナル、PID 再利用防御） |
| `app/` | ネイティブ GPUI シェル（概要カード、仮想リスト、詳細パネル、ダイアログ） |
| `web/` | wasm シェル（GPUI Web + デモ / エージェント接続、読み取り専用、設定は localStorage） |
| `poc-wasm/` | GPUI Web の最小スパイク（Runtime Proof・スクリーンショット） |
| `agent/` | ブラウザ用の読み取り専用ループバック HTTP エージェント |
| `notes/` | 調査ノート・wasm スパイク・IME/クロスプラットフォーム検証・レビュー |
| `scripts/` | ビルド補助（`dev-setup.sh`, `check.sh`） |
| `packaging/linux/` | DEB パッケージ（`build-deb.sh`、`.desktop`、アイコン、`copyright`） |
| `deny.toml` / `.github/dependabot.yml` | 依存ポリシー（ライセンス・脆弱性・自動更新） |

## セキュリティ方針（要約）

詳細は [SECURITY.md](SECURITY.md) を参照してください。

- `unsafe` 禁止（`#![forbid(unsafe_code)]`）、setuid なし、root 権限不要
- シグナル操作は確認ダイアログ必須。PID 1 / 自分自身は拒否。PID + 起動時刻の
  一致を送信直前に再確認
- ブラウザ / デモは**読み取り専用**（操作ボタン無効、エージェントに操作 API なし）
- エージェントは `127.0.0.1` のみ bind、トークン必須、CORS は localhost のみ
- OS 由来の文字列は制御文字・ANSI エスケープを除去して表示
- GPUI / sysinfo / rustix は完全一致ピン、`Cargo.lock` をコミット。`cargo audit` に加えて
  `cargo-deny`（ライセンス・脆弱性・重複版）を CI で実行（ルート / web とも ok）

## 検証状況（2026-11-15、Runtime Proof）

| 項目 | 結果 |
|---|---|
| `cargo test --workspace` | **143 passed / 0 failed**（core 49 / viewmodel 34 / platform 9 / agent 39 / app 12） |
| web ユニットテスト | **7 passed / 0 failed** |
| `cargo clippy --workspace --all-targets -- -D warnings` | 成功（警告 0） |
| `cargo fmt --all --check` | 成功 |
| ネイティブ起動（Wayland、実データ） | 成功。約 620 プロセス、CPU/メモリ/ネットワーク表示、stderr 0 行（[画像](notes/assets/task-manager-native-wayland.png)、`--redact` でホスト名（コマンド引数内のホスト名・tailnet ドメインを含む）等をマスク） |
| デモ起動 | 成功（`--demo`、合成データ） |
| エージェント | `/healthz` 200、トークンなし 401、クエリトークン 401、`limit` 動作、CORS、Host 検証 403、405、ループバック強制を確認 |
| wasm スパイク | `trunk build --release` 成功、ヘッドレス Chrome で描画・CJK・入力・fetch を確認（WebGL2 フォールバック） |
| ブラウザ版シェル | `web/` に実装。デモ / ライブ（agent 接続）をヘッドレス Chrome で検証（[画像](notes/assets/web-shell-08-final-live.png)、詳細は [notes/web-shell.md](notes/web-shell.md)） |
| `cargo audit` | 脆弱性 0 / 未保守警告 8 件（GPUI 依存由来） |
| `cargo-deny` | ルート / web とも **advisories / bans / licenses / sources すべて ok**（未保守 8 件は理由付き ignore） |
| Dependabot | Cargo（ルート / web）と GitHub Actions の週次更新を設定。セキュリティアラート / 自動修正を有効化。初回更新ジョブ 3 件実行、PR #1（actions/checkout 4→7）作成・PR CI 成功 |
| wasm-opt | 14,799,786 B → **7,589,230 B**（gzip 3,656,199 → 3,024,986 B）。適用後もブラウザ検証 pass |
| 日本語 IME（ブラウザ） | composition 32 チェックすべて pass、JS 例外 0。詳細 [notes/ime.md](notes/ime.md) |
| 日本語 IME（ネイティブ） | `EntityInputHandler` + 共有 `FilterInput`。`TM_DEBUG_IME=1` で変換中→確定を確認（[画像](notes/assets/native-ime-01-composing.png)）。実 IME は未検証 |
| Windows cross-check | `cargo check --target x86_64-pc-windows-msvc --workspace`（app/GPUI 含む）成功。CI `native-windows` で実機相当検証 |
| macOS cross-check | `tm-core` / `tm-viewmodel` / `tm-platform` / `tm-agent` の macOS ターゲット check 成功。app は CI `native-macos` で検証 |
| CI | 7 ジョブ（gates / wasm / cargo-deny / native-windows / native-macos / release-linux / web-dist） |
| 設定の永続化（ネイティブ） | 保存（0600・原子的）と復元を実機確認（[画像](notes/assets/settings-restore.png)、`TM_DEBUG_SETTINGS=1`） |
| 設定の永続化（ブラウザ） | 列クリックで保存、再読込で復元を CDP で確認（`web/verification/web-settings-evidence.json`） |
| DEB パッケージ | `bash packaging/linux/build-deb.sh` で 11 MB の `.deb` を生成、展開バイナリの `--version` と依存自動計算を確認 |

詳細は [notes/verification.md](notes/verification.md)、ブラウザ版は
[notes/web-shell.md](notes/web-shell.md)、IME は [notes/ime.md](notes/ime.md)、
クロスプラットフォームは [notes/cross-platform.md](notes/cross-platform.md)、
物理実機/クラウド検証の選択肢は [notes/physical-verification.md](notes/physical-verification.md)、
独立レビューは [notes/review-2026-11-14.md](notes/review-2026-11-14.md) を参照してください。

## 既知の制限

- Windows / macOS は**実機では未検証**。コードは OS 別 cfg でコンパイル可能にし、
  GitHub Actions の windows-latest / macos-latest を実機相当の検証として使う
- Windows は POSIX シグナルがないため Stop/Continue 非対応（Terminate/Kill は
  `taskkill /F` による強制終了）
- ブラウザ版は読み取り専用（OS プロセスはブラウザから取得できないため）
- ネイティブ版 IME は実装済みだが、実 IME（IBus/fcitx5）での手動確認は未実施
  （`TM_DEBUG_IME=1` のデバッグフックによる状態遷移の検証のみ）
- プロセスツリー、優先度変更、GPU 監視、ライトテーマは未実装
- DEB は未署名・リポジトリ未公開（ローカル生成のみ）
- ブラウザ wasm は 7.24 MiB（gzip 2.89 MiB、wasm-opt -Oz 適用済み）
- エージェントはシリアル処理（1 スナップショット約 250 ms）
- GPUI 系は個人アカウントが再公開するスナップショット。完全ピンで運用し、更新は
  まとめて行う必要がある

## ライセンス

Apache-2.0。依存クレートのライセンスと帰属は [NOTICE.md](NOTICE.md) を参照。
既存の AGPL プロジェクト（`work/gpui-file-explorer`）からは**コードをコピーして
いません**（調査ノートのみ参照）。

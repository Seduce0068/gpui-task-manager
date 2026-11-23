# GPUI Task Manager

Ubuntu 系デスクトップ向けの**システムタスクマネージャー**試作です。
[GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui)（Zed の GPU
アクセラレート UI フレームワーク）を `gpui-kit 0.9.1` 経由で使い、プロセス・CPU・
メモリ・ネットワーク・ディスクを監視します。クロスプラットフォームと wasm /
ブラウザ対応を見据えた構成です。

![ネイティブ版（Wayland、実データ、--redact でホスト名等をマスク）](notes/assets/task-manager-native-wayland.png)

![ブラウザ版（GPUI Web / wasm、ライブデータ、--redact 適用）](notes/assets/web-shell-08-final-live.png)

- 第一ターゲット: **Kubuntu 2026.04 / KDE Plasma 6 / Wayland**（実機検証）
- スタック: Rust 1.95+ / `gpui-kit =0.9.1`（`gpui-pre 0.3.12`）/ `sysinfo =0.40.3` / `rustix =1.1.6`
- ライセンス: **AGPL-3.0-or-later**（第三者表示は [NOTICE.md](NOTICE.md)、以前の Apache-2.0 本文は [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)）

> **製品バージョン: 0.1.0** — [`Cargo.toml` の `workspace.package.version`](Cargo.toml) と同期する値です。
> この製品バージョンは、wire envelope の `schema_version=1`（`tm_core::WIRE_SCHEMA_VERSION`）、
> 設定スキーマの `SETTINGS_VERSION=2`（`tm_viewmodel::SETTINGS_VERSION`）、
> そして文書メタデータの OKF `verified`（RFC 3339 日時）とは**別概念**であり、
> それぞれ独立に更新されます。混同しないでください。
>
> `WIRE_SCHEMA_VERSION` を上げる場合は**ブラウザ版を先に更新し、tm-agent を後に
> 更新してください**。ブラウザ版は自身の版以下のスナップショットだけを受理する
> ため、新版エージェントを先に起動すると旧ブラウザ版が全フレームを拒否し、
> 表示が停止します（更新後はブラウザのキャッシュもリロードしてください）。

## GitHub

- リポジトリ: <https://github.com/Seduce0068/gpui-task-manager>（**public**）
- CI: [`.github/workflows/ci.yml`](.github/workflows/ci.yml)。Linux の gates は共通の
  `Seduce0068/workflows@v1` に委譲（fmt → check → test → clippy → cargo audit → cargo vet）。
  追加で wasm / msrv-1-95 / cdp-verify / cargo-deny / native-windows /
  native-macos / release-linux / web-dist / sbom を実行（Linux 系ジョブは
  再現性のため `ubuntu-24.04` に固定、成果物は `actions/upload-artifact@v7`）。SBOM は
  [`scripts/generate-sbom.sh`](scripts/generate-sbom.sh)（CycloneDX JSON、
  root/web ワークスペース）で生成。
- 運用: Dependabot（Cargo / web / GitHub Actions、週次）、セキュリティアラート、
  プロジェクト用ラベル（`area:*` / `ci` / `security` / `packaging` / `supply-chain` 等）
  を有効化。ブランチ保護は public リポジトリの機能で有効化し、必須 CI / レビュー運用で補う。
  `cargo-vet` は root / web とも初期化済み（`supply-chain/`、2026-11-18 に両ストアを
  `baseline-unreviewed`（未監査ベースラインによる drift 検出）方針へ統一。全依存を
  `exempt` として記録し、`safe-to-deploy` を主張しない。外部 imports は未使用、
  監査記録の捏造はしない）。SBOM（CycloneDX）は
  `scripts/generate-sbom.sh` と CI で生成する。
- 公開用の `main` はクリーンな履歴。開発履歴はローカル開発ブランチに保持している。

## できること（MVP）

- プロセス一覧: 名前・コマンド・CPU・メモリ・PID・ユーザー・状態・スレッド数（ツリー表示トグル、`parent_pid` ベースの階層インデント）
- 概要カード: CPU（全体 + コア別 + 履歴）、メモリ / スワップ、ネットワーク
  受信・送信レート、ホスト / 稼働時間 / ディスク使用率、温度センサー（取得可能な場合）
- ソート（列クリック）、絞り込み（名前・コマンド・ユーザー・PID）、詳細パネル
- **フィルタプリセット**（すべて / 高 CPU / 高メモリ / 大きな仮想メモリ / 実行中）をワンクリックで適用（`tm-viewmodel` 共通の組み込み定義、ブラウザ版のツールバーにチップ表示）
- **更新間隔プリセット**（0.5s / 1s / 2s / 5s）をボタンで切替・保存（ネイティブ / ブラウザ共通）
- **電源・ウィンドウ状態に応じたサンプリング**: AC 表示時は下限 0.5s（未変更の既定は 1s）、バッテリー 2.5s、低バッテリー（20% 以下）・ウィンドウ非表示 5s。設定した更新間隔の下限として適用し、意図的に遅い設定は維持（ネイティブ / ブラウザ）
- **cgroup の読み取り専用表示**: 正規化したグループラベルとメンバー数、ステータスバーのグループ数を表示。操作・上限変更は不可
- シグナル操作（SIGTERM / SIGKILL / SIGSTOP / SIGCONT）を**確認ダイアログ経由**で実行
- 安全装置: PID 1 と自分自身の拒否、PID + 起動時刻による PID 再利用検出
- キーボード操作: ↑↓ / PgUp/PgDn / Home/End / Space / Ctrl+F / Ctrl+K / F2 / F5 / F1
- **ブラウザ版 / ネイティブ版の日本語 IME（composition）対応**。変換中は未確定文字列を
  入力欄にのみ表示し、確定文字列だけを絞り込みへ反映
- `--demo` で決定的な合成データ起動（スクリーンショット・回帰用）
- `--export csv|json PATH` で伏せ字化済みスナップショットを 0600 で書き出して終了
  （ウィンドウを開かず、CLI 証跡や回帰比較に使える）
- ブラウザ版でもツールバーの **CSV / JSON エクスポートボタン**で伏せ字化済み
  スナップショットをダウンロード可能（ネイティブの `--export` と同じ
  `tm_core::export` を共有し、DOM proof の `export=csv|json` で確認できる）
- **設定の永続化（ネイティブ / ブラウザ両方）**: 更新間隔・ソート列/方向・
  詳細パネルの表示・ツリー表示・**テーマ（Dark / Light）**を保存し、次回起動時に復元。ネイティブは XDG 設定
  (`~/.config/gpui-task-manager/settings.json`、Windows/macOS も対応パス、
  原子的書き込み・0600・スキーマ版付き)、ブラウザは `localStorage`
  (`gpui-task-manager.settings`)。スキーマは `tm-viewmodel::Settings` で共有
- 依存監査（cargo-deny / cargo audit）と Dependabot による週次更新
- ブラウザ版（読み取り専用）: `web/` + `poc-wasm/`。ローカルエージェント経由で
  実データを表示（wasm / GPUI Web）
- **ライトテーマ（Catppuccin Latte）**: ネイティブ版・ブラウザ版ともヘッダーの「ライト/ダーク」ボタンで切替、設定に永続化（ブラウザ版は `localStorage`）
- **スクリーンリーダー向け AX ミラー（ブラウザ版）**: GPUI canvas の AccessKit がブラウザ AX に届かないため、非表示 DOM ミラーを `role=grid` で公開し、行・列・セルを読み上げ可能にする（ADR-022）
- **複数タブ設定同期（ブラウザ版）**: `localStorage` への差分マージ保存と `storage` イベント同期で、複数タブ / インスタンス間の設定変更を調停する（ADR-023）

## エクスポート形式（CSV / JSON）

`--export csv|json PATH`（ネイティブ）とブラウザ版のエクスポートボタンは、どちらも
`tm_core::export` の純粋関数を使い、**伏せ字化・サニタイズ済み**のスナップショットを
書き出します。JSON は `GET /api/v1/snapshot` と同じ wire envelope
（`schema_version` / `total_process_count` / `snapshot`）です。

CSV はヘッダー 1 行 + プロセス 1 行の**固定 12 列**です。各列の意味と形式は次のとおりです。

| # | 列 | 意味 | 形式 |
| --- | --- | --- | --- |
| 1 | `sampled_at_unix_millis` | スナップショット取得時刻（全行で同一。複数ファイルを時系列に連結しても時刻を失わない） | Unix epoch ミリ秒（13 桁、例 `1700000000123`）。**表計算アプリは日付として認識しません**。必要なら `=A2/86400000+DATE(1970,1,1)` 等で変換し、タイムゾーンは利用側で解釈する |
| 2 | `pid` | プロセス ID | 10 進整数 |
| 3 | `start_time_secs` | プロセス起動時刻（PID 再利用検出用） | Unix epoch 秒（10 桁） |
| 4 | `name` | プロセス名（伏せ字化済み） | RFC 4180 クォート。数式 starter は先頭に `'` を付与 |
| 5 | `user` | 実行ユーザー（伏せ字化済み。取得不可は空欄） | 文字列 |
| 6 | `status` | プロセス状態 | `running` / `sleeping` / `idle` / `stopped` / `zombie` / `tracing` / `unknown` |
| 7 | `cpu_percent` | CPU 使用率（全コア合計のため 100 超え可） | **小数点以下 2 桁に丸めた数値**（例 `4.3799996` → `4.38`、`1.9399999` → `1.94`。末尾の `0` は省略）。丸めは出力時のみで、計算・ソートと JSON の数値は生値のまま |
| 8 | `memory_bytes` | 常駐メモリ（RSS） | バイト（10 進整数） |
| 9 | `virtual_memory_bytes` | 仮想メモリ | バイト（10 進整数） |
| 10 | `threads` | スレッド数（取得不可は空欄） | 10 進整数 |
| 11 | `parent_pid` | 親プロセス ID（取得不可は空欄） | 10 進整数 |
| 12 | `command` | コマンドライン（伏せ字化・サニタイズ済み） | RFC 4180 クォート。数式 starter は先頭に `'` を付与 |

テキスト列（`name` / `user` / `command`）はすべて `redact` 済みで、ホスト名・ユーザー名・
ホームパス・内部ドメインは出力されません。制御文字と ANSI エスケープも除去されます。

ISO-8601 形式の日時列を併設する案は**採用しませんでした**。13 桁 epoch の列を
ISO-8601 文字列に置き換えたり 13 列目を追加したりすると、固定 12 列の CSV 契約と
既存のパーサー / 回帰証跡が壊れるためです。日付として扱いたい場合は利用側で
上の式により変換してください（この判断は列の意味を README に明記することで代替しています）。

## クイックスタート（Kubuntu / Wayland）

### Rust の導入（初回のみ）

ビルドには **Rust 1.95+**（`rustc` / `cargo`）が必要です（CI の `msrv-1-95`
ジョブでも同バージョンをゲートしています）。未導入または古い場合は、
[rustup の公式インストール手順](https://rustup.rs/)で stable を入れてください。

```bash
# Kubuntu: 初回導入（既存環境は rustup update で更新）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
. "$HOME/.cargo/env"
rustc --version   # 1.95.0 以降であることを確認
cargo --version

# ブラウザ版をビルド／実動確認する場合のみ（wasm ターゲットと trunk 0.22.1）
rustup target add wasm32-unknown-unknown
cargo install trunk --version 0.22.1 --locked
```

### まず試す（最短）

```bash
# Kubuntu / Wayland: 初回のみ、libxkbcommon-x11 のローカル shim を用意
bash scripts/dev-setup.sh

# ビルドして合成データで起動（エージェント不要・実データの権限不要）
cargo build -p gpui-task-manager
./target/debug/gpui-task-manager --demo
```

### アプリケーションメニューにインストール（開発用）

`scripts/install-local.sh` は root 不要で、ビルドからランチャー登録までを
一括で行います。再ビルド後に再度実行すれば、バイナリとメニューが更新されます。

```bash
bash scripts/install-local.sh              # ビルド + 配置 + メニューキャッシュ更新
bash scripts/install-local.sh --skip-build # 既存の target/release バイナリを再利用
```

配置先は `~/.local/bin/gpui-task-manager`、
`~/.local/share/applications/gpui-task-manager.desktop`、および
`~/.local/share/icons/hicolor/` です。配布用の `.deb` は
`packaging/linux/build-deb.sh` が作成します（こちらは `/usr` 配下へインストール）。

`--demo` は決定的な合成データです。フィルタプリセットの「高 CPU」は 50% 超の
行を表示し、更新間隔プリセットの 0.5s も実効します（未変更の既定は 1s）。
実データを見る場合は `./target/debug/gpui-task-manager` を起動してください。

### 重い検証（必要なときだけ）

```bash
cargo test --workspace
bash scripts/check.sh        # fmt + clippy + test + wasm + web clippy/test + deny + vet + machete + rustdoc + markdownlint を一括実行
```

前提パッケージ（Ubuntu 系）:

```bash
sudo apt install -y gcc g++ clang pkg-config libfontconfig-dev libfreetype-dev \
  libwayland-dev libxkbcommon-dev libxkbcommon-x11-dev libx11-xcb-dev \
  libssl-dev libzstd-dev libvulkan1
```

Vulkan 対応 GPU ドライバが必要です（GTX 1660 SUPER + NVIDIA proprietary で検証）。
`jq` はあえて前提一覧に含めていません。後述のエージェント例は `head -c 500` で
出力を切るため、JSON 整形ツールの追加導入なしでコピペできます。
`libxkbcommon-x11-dev` を入れられない場合は `scripts/dev-setup.sh` が
`app/native-libs/libxkbcommon-x11.so` を作成します（システム変更なし・root 不要）。

### ローカル shim の生成条件と除去

`bash scripts/dev-setup.sh` は次の場合にだけ
`app/native-libs/libxkbcommon-x11.so` を生成（または作り直し）します。

- `pkg-config --exists xkbcommon-x11` が**失敗**する（`-dev` パッケージが無い）
- `libxkbcommon-x11.so.0` がスクリプト既知のシステムパスに**存在**する

shim が無い／壊れている／選択したランタイムと別の実体を指す場合は作り直し、
正しい実体を指す場合は維持します（冪等）。`app/build.rs` は `app/native-libs/`
が存在する限りリンカの検索パスへ追加するため、shim が残ったままだと後から
`libxkbcommon-x11-dev` を導入・更新した際に古い shim へ誤リンクする可能性があります。
**`-dev` 導入後・OS 更新後は、もう一度 `bash scripts/dev-setup.sh` を実行して
shim を除去してください**（`pkg-config` が見つかれば自動除去されます）。

```bash
# Kubuntu / 開発ホスト: 状態確認 → 除去 → 再確認
echo '--- shim の状態（事前確認）---'
ls -l app/native-libs/libxkbcommon-x11.so 2>/dev/null || echo 'shim なし'
echo '--- shim の除去: libxkbcommon-x11-dev 導入後に実行 ---'
bash scripts/dev-setup.sh
echo '--- 再確認 ---'
ls -l app/native-libs/libxkbcommon-x11.so 2>/dev/null || echo 'shim なし（正常）'

# pkg-config が使えず自動除去できない場合の手動除去
# rm -f app/native-libs/libxkbcommon-x11.so
# rmdir app/native-libs 2>/dev/null || true
```

`-dev` パッケージが無いホストで上記を実行すると shim が再作成されます（通常の動作）。
スクリプトはランタイムが見つからない場合も古い shim を除去してから失敗するため、
ビルド時に残置 shim が誤って参照されることはありません。

### 読み取り専用エージェント（ブラウザ用）

ワンショット（サーバーを起動せず JSON を 1 回だけ出力）:

```bash
cargo run -p tm-agent -- --once --demo      # 1 スナップショットを JSON で出力
```

Linux/Unix のサーバーモードではトークンを省略すると自動生成され、0600 で保存されます。
stderr で案内されるのはファイル名 `token` のみで、保存場所は下記の XDG state パスです。
**Windows では自動生成を行いません。** トークンを省略した既定起動は固定エラーで失敗するため、
`--token-file` / `TM_AGENT_TOKEN` / `--token` のいずれかを明示してください（下記「認証」節を参照）。
既定の待受ポートは 8787（画面案内と同じ）です。

**ターミナル A（サーバー）** — 起動したままにしておきます:

```bash
cargo run -p tm-agent
```

**ターミナル B（クライアント）** — 別のターミナルで実行します（`token_file` を
このブロック内で定義するため、新しいターミナルでも動きます）:

```bash
token_file="${XDG_STATE_HOME:-$HOME/.local/state}/tm-agent/token"
curl -sS -H "Authorization: Bearer $(cat "${token_file}")" \
  'http://127.0.0.1:8787/api/v1/snapshot?limit=5' | head -c 500
```

別ポートの明示例（既定の 8787 が競合する並行検証などでだけ指定）:

**ターミナル A（サーバー、別ポート）**:

```bash
cargo run -p tm-agent -- --listen 127.0.0.1:18787
```

**ターミナル B（クライアント、別ポート）** — 新しいターミナルでも動くよう
`token_file` を再定義しています:

```bash
token_file="${XDG_STATE_HOME:-$HOME/.local/state}/tm-agent/token"
curl -sS -H "Authorization: Bearer $(cat "${token_file}")" \
  'http://127.0.0.1:18787/api/v1/snapshot?limit=5' | head -c 500
```

`| jq .` を使わないのは、上記の前提パッケージ一覧に `jq` が無いためです。
`head -c 500` なら追加パッケージなしで先頭 500 バイトを確認できます（全体を
整形して見たい場合は `jq` を別途導入してください）。

エージェントはループバックアドレスのみ bind（既定 `127.0.0.1`）、`Authorization: Bearer` 必須、読み取り専用、
CORS は loopback オリジン（`http://127.0.0.1[:port]` / `http://localhost[:port]`）のみ許可します。`--token` はテスト専用です（argv は
同一マシンの他プロセスから読まれ得ます）。常用時は `--token-file` か `TM_AGENT_TOKEN`
を使ってください。**Linux/Unix では**トークンを省略すると自動生成され、
`$XDG_STATE_HOME/tm-agent/token`（未設定なら `~/.local/state/tm-agent/token`）に 0600 で保存します。
**Windows では自動生成を行いません。** 既存のトークンファイル（`%APPDATA%`／
`%USERPROFILE%\AppData\Roaming` 配下）があれば再利用されますが、新規に生成することはできないため、
トークン未指定の起動は次の固定エラー（`agent/src/auth.rs` と一致）で失敗します:

> Windows では --token 明示が必要です: 安全な乱数源を新規依存・unsafe なしで取得できないため、--token / --token-file / TM_AGENT_TOKEN のいずれかでトークンを指定してください。

Windows では `--token-file` / `TM_AGENT_TOKEN` / `--token` のいずれかを明示してください
（`--token` は argv が同一マシンの他プロセスから読まれ得るため常用には推奨しません）。
stderr で案内されるのはファイル名 `token` のみで、ホームパスを含む保存場所や
トークンの値はログ・stdout に出力されません（stderr にも出力しません）。
再起動時は同じファイルを再利用するため、ブラウザが保持したトークンもそのまま使えます。
接続時は上記 XDG state パス（Windows では `%APPDATA%`／`%USERPROFILE%\AppData\Roaming`
配下）のファイル内容を読み取り、URL フラグメントや `--token-file` に指定してください。

HTTP エンドポイントは `/healthz`（ヘルスチェック）、`/api/v1/snapshot`（スナップショット、
`?limit=N` で件数指定）、`/api/v1/stream`（SSE）、`/metrics`（Prometheus テキスト）の 4 つです。
`/api/v1/stream` は `text/event-stream` で 1 秒間隔のスナップショットを長時間接続で流します。
Bearer トークン必須、同時 2 接続まで、Host / Origin は loopback のみ、認証は
`Authorization` ヘッダのみ（クエリトークン不可）です。ブラウザ版はライブ接続でこの
SSE を優先利用し、接続に失敗するか終了した場合は従来のポーリングへフォールバックします。

### wasm / ブラウザ

ブラウザ版シェル（読み取り専用）:

**ターミナル A（サーバー）** — 起動したままにしておきます:

```bash
cd web && trunk build --release && trunk serve
```

**ターミナル B（クライアント）** — 別のターミナルでサーバー応答を確認し、
ブラウザで下記 URL を開きます:

```bash
curl -sS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8091/
# デモ表示:  http://127.0.0.1:8091/
# ライブ表示: http://127.0.0.1:8091/?agent=http://127.0.0.1:8787#token=<token>
#   （トークンは URL フラグメントで渡し、fetch の Authorization ヘッダで送る）
```

GPUI Web の最小スパイク（Runtime Proof、サーバーは不要。ターミナル A で
`cd web` 済みのシェルをそのまま使うため `../poc-wasm` を指定します。
新しいターミナルなどリポジトリルートにいる場合は
`cd poc-wasm && trunk build --release` としてください）:

```bash
cd ../poc-wasm && trunk build --release
```

ブラウザ版は `tm-core` / `tm-viewmodel` をネイティブ版と共有し、GPUI Web
（`gpui-pre-web`、WebGPU → WebGL2 自動フォールバック）で描画します。
`tm-agent` に接続しない場合は決定的なデモデータで動作します。プロセス操作は
**提供しません**（読み取り専用）。wasm サイズは **7,949,118 B（gzip 3,165,445 B、2026-11-20 計測）**で、
`data-wasm-opt="z"` により wasm-opt（-Oz）を適用しています。日本語 IME は
`EntityInputHandler` + composition で対応済みです（[notes/ime.md](notes/ime.md)）。

### 固定 loopback ポートの運用（複数インスタンスと切替）

エージェントは `127.0.0.1:8787`、ブラウザ版シェル（`trunk serve`）は
`127.0.0.1:8091` を既定の固定 loopback ポートとして使います（trunk 側の既定値は
[`web/Trunk.toml`](web/Trunk.toml)）。複数インスタンス運用では `--listen` で
`127.0.0.1:18787` などの別ポートを明示的に割り当ててください。**同じホストの同じポートに同時に使えるのは
1 インスタンスだけ**です。2 つ目を同じポートで起動すると bind に失敗するため、
別の worktree / ブランチを並行検証する場合は次のようにポートをずらします。

```bash
# Kubuntu / 開発ホスト: 使用中の待ち受けポートを確認
echo '--- 待ち受けポートの確認 ---'
ss -ltnp | grep -E ':(18787|18788|8091|8092)\b' || echo '該当なし'

# 2 つ目のエージェント（--listen で別の loopback ポートを指定。
# トークンは省略して自動生成ファイルを共有）
cargo run -p tm-agent -- --listen 127.0.0.1:18788

# 2 つ目のブラウザ版シェル（trunk serve の --port で Trunk.toml の既定 8091 を上書き）
cd web && trunk serve --port 8092
```

エージェントは `127.0.0.1` / `[::1]` の loopback のみ bind を許可し、`0.0.0.0` や
ホスト名は拒否します。ブラウザ版から 2 つ目のスタックへ接続する場合は URL の
`?agent=` も切り替えたポートに合わせてください。

```text
http://127.0.0.1:8092/?agent=http://127.0.0.1:18788#token=<token>
```

終了時は各プロセスの起動元ターミナルで `Ctrl-C` を使ってください。前述の
クイックスタート手順（既定ポート）は 1 インスタンス前提のままです。

### 破損設定の退避と手動復元

設定が破損していた場合、アプリは元の内容を退避してから既定値で起動します。
退避物は古い順に自動削除され、**最新 5 件だけ**が残ります（削除に失敗しても
起動は継続します）。消える前に必要な設定を戻す手順は次のとおりです。

- ネイティブ版: `~/.config/gpui-task-manager/settings.json.corrupt-<timestamp>`
  を `~/.config/gpui-task-manager/settings.json` へコピーして起動します。
  `<timestamp>` は実際の退避ファイル名の末尾（16 桁の hex）に置き換えてください。

```bash
# Kubuntu / Linux: 最新の退避物を settings.json へ戻して起動
# 退避ファイル名末尾の 16 桁 hex タイムスタンプ順が「最新」の定義（mtime ではない）
backup=$(ls -1 ~/.config/gpui-task-manager/settings.json.corrupt-* | sort | tail -n 1)
cp -- "$backup" ~/.config/gpui-task-manager/settings.json
./target/debug/gpui-task-manager
```

退避物は「破損を検出した値そのもの」です。正常だった設定のスナップショットでは
ないため、そのままコピーすると再読込時に再度 corrupt 経路へ送られ既定値に縮退
します。必要なフィールドだけを抽出し、型が不正なフィールドは既定値へ置き換えて
から現行 JSON へマージしてください。

- ブラウザ版: DevTools（Edge なら F12 →「アプリ」→「ローカルストレージ」）で
  `gpui-task-manager.settings.corrupt.<timestamp>` の値をコピーし、型不正な
  フィールドを修正したうえで `gpui-task-manager.settings` の値へ貼り付けて
  上書きし、ページを再読込します。最新の退避物はキー末尾の `<timestamp>`
  （16 桁 hex）が最大のものを選びます（mtime 順ではありません）。

```text
gpui-task-manager.settings.corrupt.<timestamp>  →  gpui-task-manager.settings
```

詳しくは [SECURITY.md](SECURITY.md) の「破損設定の退避と手動復元」も参照してください。

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
| --- | --- |
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
- 公開用スクリーンショット / 証跡は `--redact`（または環境変数 `TM_REDACT=1`）で
  ホスト名・ユーザー名・ホームパスを伏せ字化（[SECURITY.md](SECURITY.md) の「公開物の衛生」）
- エージェントはループバックアドレスのみ bind（既定 `127.0.0.1`）、トークン必須、CORS は loopback origin のみ
- OS 由来の文字列は制御文字・ANSI エスケープ・bidi／ゼロ幅の不可視書式文字を除去して表示
- GPUI / sysinfo / rustix は完全一致ピン、`Cargo.lock` をコミット。`cargo audit` に加えて
  `cargo-deny`（ライセンス・脆弱性・重複版）を CI で実行（ルート / web とも ok）
- Gitleaks でシークレットスキャン: `.git/hooks/pre-commit` がコミット前にステージ済み
  差分を検査し、CI も最終ゲートとして全履歴を走査する（ローカルフックは回避可能なため）

## 検証状況（2026-11-20、Runtime Proof）

| 項目 | 結果 |
| --- | --- |
| `cargo test --workspace` | **729 passed / 0 failed**（core 211 / viewmodel 143 / platform 44 / agent 158 / app 171、2026-11-20 実測） |
| web ユニットテスト | **258 passed / 0 failed**（fetch 制限 / redaction / プリセット / 電源 / SSE / 行メニュー / オンボーディング等を含む、2026-11-20 実測） |
| `cargo clippy --workspace --all-targets -- -D warnings -D clippy::undocumented_unsafe_blocks` | 成功（警告 0、root / web とも） |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | 成功（警告 0、root / web とも） |
| `cargo fmt --all --check` | 成功 |
| ネイティブ起動（Wayland、実データ） | 成功。約 620 プロセス、CPU/メモリ/ネットワーク表示、stderr 0 行（[画像](notes/assets/task-manager-native-wayland.png)、`--redact` でホスト名（コマンド引数内のホスト名・tailnet ドメインを含む）等をマスク） |
| デモ起動 | 成功（`--demo`、合成データ） |
| エージェント | `/healthz` 200、トークンなし 401、クエリトークン 401、`limit` 動作、CORS（403 含む）、Host 検証 403、405、ループバック強制を確認。並列接続（最大 8）でアイドル 3 本下でも `/healthz` 3.04 ms、超過は即 503（2026-11-16 実測） |
| wasm スパイク | `trunk build --release` 成功、ヘッドレス Chrome で描画・CJK・入力・fetch を確認（WebGL2 フォールバック） |
| ブラウザ版シェル | `web/` に実装。デモ / ライブ（agent 接続）を Chrome **158** / Edge 157 / Firefox 160 の互換マトリクスで検証成功（WebDriver BiDi。F1 は合成キーで検証しており信頼済みキーは未確認）。CDP による日常のヘッドレス Runtime Proof は Chrome **157** で実施しており（[web/README.md](web/README.md)「必要環境」参照）、互換マトリクスの 158 とは用途が異なる。Safari / Brave / モバイル / 実 GPU の WebGPU は未検証のまま（[画像](notes/assets/web-shell-08-final-live.png)、詳細は [notes/web-shell.md](notes/web-shell.md)） |
| `cargo audit` | 脆弱性 0 / 未保守警告 9 件（GPUI 依存由来、内訳は [DESIGN.md](DESIGN.md) §14） |
| `cargo-deny` | ルート / web とも **advisories / bans / licenses / sources すべて ok**（未保守 9 件は理由付き ignore、内訳は [DESIGN.md](DESIGN.md) §14） |
| Dependabot | Cargo（ルート / web）と GitHub Actions の週次更新を設定。セキュリティアラート / 自動修正を有効化。初回更新ジョブ 3 件実行、PR #1（actions/checkout 4→7）作成・PR CI 成功。Actions はコミット SHA 固定（タグはコメント併記）、Dependabot github-actions 週次更新で追従 |
| wasm-opt | 14,799,786 B → **7,589,230 B**（gzip 3,656,199 → 3,024,986 B）。適用後もブラウザ検証 pass。2026-11-15 再計測 7,635,554 B（gzip 3,042,856 B）、2026-11-16 再計測 7,641,445 B（gzip 3,044,050 B）、2026-11-17 再計測 7,641,970 B（gzip 3,045,139 B）、2026-11-17 最終再計測 7,642,794 B（gzip 3,044,134 B）、2026-11-20 現行 dist 7,949,118 B（gzip 3,165,445 B） |
| 日本語 IME（ブラウザ） | composition 32 チェックすべて pass、JS 例外 0。詳細 [notes/ime.md](notes/ime.md) |
| 日本語 IME（ネイティブ） | `EntityInputHandler` + 共有 `FilterInput`。`TM_DEBUG_IME=1` で変換中→確定を確認（[画像](notes/assets/native-ime-01-composing.png)）。実 IME は未検証 |
| Windows cross-check | `cargo check --target x86_64-pc-windows-msvc --workspace`（app/GPUI 含む）成功。CI `native-windows` で実機相当検証 |
| macOS cross-check | `tm-core` / `tm-viewmodel` / `tm-platform` / `tm-agent` の macOS ターゲット check 成功。app は CI `native-macos` で検証 |
| CI | 10 ジョブ（gates / wasm / msrv-1-95 / cdp-verify / native-windows / native-macos / cargo-deny / release-linux / web-dist / sbom、2026-11-19 定義再確認） |
| 設定の永続化（ネイティブ） | 保存（0600・原子的）と復元を実機確認（[画像](notes/assets/settings-restore.png)、`TM_DEBUG_SETTINGS=1`） |
| 設定の永続化（ブラウザ） | 列クリックで保存、再読込で復元を CDP で確認（`web/verification/web-settings-evidence.json`） |
| ライトテーマ（ブラウザ） | 状態フック `theme=dark` が既定、`theme=light` が復元され、切替ボタンで Dark ↔ Light と保存値が連動することを CDP で確認（[画像](notes/assets/web-shell-light-theme.png)、詳細は [notes/verification.md](notes/verification.md)） |
| 概要カードのレイアウト（ブラウザ） | システムカードの温度行とツールバーの重なりを修正（概要高さ 186 → 260 px）。CDP デモ / 設定 / IME（32/32）を再検証し、スクリーンショットで重なり解消を確認 |
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
- 優先度変更、GPU 監視、テーマの自動更新（`prefers-color-scheme` 連動）は未実装
- DEB は未署名・リポジトリ未公開（ローカル生成のみ）。`Maintainer` の既定値は
  `gpui-task-manager contributors (security reports via GitHub Security Advisory)`
  という中立値で、実在しないメールアドレスを含めない（報告方針は
  [SECURITY.md](SECURITY.md) の GitHub Security Advisory を参照）。配布用に
  ビルドする場合は環境変数 `DEB_MAINTAINER` に実在の maintainer 名と連絡先を
  指定する（例: `DEB_MAINTAINER='Name <real-email>' bash packaging/linux/build-deb.sh`）
- ブラウザ wasm は 7,949,118 B（gzip 3,165,445 B、wasm-opt -Oz 適用済み、2026-11-20 計測）
- エージェントのサンプリングは共有プローブで直列化（1 スナップショット約 250 ms）。接続は最大 8 本まで並列処理し、超過は即座に 503（アイドル接続が他のリクエストをブロックしない）
- GPUI 系は個人アカウントが再公開するスナップショット。完全ピンで運用し、更新は
  まとめて行う必要がある

## ライセンス

`AGPL-3.0-or-later`。依存クレートのライセンスと帰属は [NOTICE.md](NOTICE.md) を参照。
以前の Apache-2.0 本文は [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) に保持しています。
Apache-2.0 は GPLv3 互換のため AGPLv3 への変更が可能ですが、NOTICE の保持義務は継続します。
別の AGPL プロジェクトからは**コードをコピーして
いません**（調査ノートのみ参照）。

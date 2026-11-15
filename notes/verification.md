---
type: Reference
title: GPUI Task Manager 検証ログ（2026-11-14）
description: ネイティブ試作・エージェント・wasmスパイクの Runtime Proof と既知の制限
tags: [verification, runtime-proof, gpui, linux, wayland, wasm]
status: draft
generated: 2026-11-14T21:10:00+09:00
verified: 2026-11-14T21:10:00+09:00
---

## 1. 検証環境

| 項目 | 値 |
|---|---|
| OS | Kubuntu 2026.04 / KDE Plasma 6 / Wayland（`XDG_SESSION_TYPE=wayland`, `WAYLAND_DISPLAY=wayland-0`） |
| CPU / RAM / GPU | Ryzen 5 3600（6C/12T）/ 64 GB / GeForce GTX 1660 SUPER |
| Rust | rustc 1.99.1 / cargo 1.99.1 |
| 検証日時 | 2026-11-14 20:00–21:10 JST |
| 作業ディレクトリ | `work/gpui-task-manager` |

## 2. ビルド・静的検証

| コマンド | 結果 |
|---|---|
| `cargo fmt --all --check` | 成功（差分なし） |
| `cargo build -p gpui-task-manager` | 成功（警告 0） |
| `cargo test --workspace` | **143 passed / 0 failed**（tm-core 49 / tm-viewmodel 34 / tm-platform 9 / tm-agent 39 / app 12） |
| web ユニットテスト | **7 passed / 0 failed**（agent 4 + settings 3） |
| `cargo clippy --workspace --all-targets -- -D warnings` | 成功（警告 0） |
| `cargo audit` / `cargo-deny` | 脆弱性 0。未保守 8 件（`zvariant` 系・`instant`・`paste`・`rustls-pemfile`・`rustybuzz`・`ttf-parser` など GPUI 依存由来）は理由付き ignore。cargo-deny はルート / web とも ok |
| バイナリサイズ（debug） | ネイティブ 166 MB / エージェント 7.0 MB（debuginfo 削減設定済み） |

## 3. ネイティブ UI（実データ）

- 起動: `./target/debug/gpui-task-manager`
- 結果: Wayland ネイティブウィンドウが起動し、stderr 0 行。
- 表示内容（スクリーンショット `notes/assets/task-manager-native-wayland.png`、
  `--redact` でホスト名・ユーザー名・ホームパスをマスク済み）:
  - ホスト名 `host`（マスク）、kernel `7.0.0-43-generic`、`Linux (Ubuntu 26.04)`
  - CPU・メモリ・ネットワーク・負荷の各カードが表示される（値はサンプル時点で
    変動するため固定値を書かない。画像を参照）
  - プロセス **約 620 件**。実プロセス（systemd, kthreadd, firefox, chrome,
    zed-editor 等）を表示
  - 詳細パネル: PID 1（systemd）を選択、ユーザー root、スレッド 1、
    メモリ表示。**PID 1 のシグナル操作は無効化**されている
- デモデータ起動: `./target/debug/gpui-task-manager --demo`
  - 合成プロセス 28 件（個人情報を含まない）

### 3.1 ウィンドウ確認の方法

Wayland ネイティブウィンドウは X11 の `xwininfo` では見えない。当初 KWin の
`queryWindowInfo` D-Bus を試したが `NoReply` で失敗したため、**KDE Spectacle の
アクティブウィンドウ撮影**（`spectacle -a -b -n -o <file>`）で表示を確認した。
ウィンドウタイトルはスクリーンショット上で `GPUI Task Manager`。

## 4. エージェント（読み取り専用 HTTP）

### 4.1 ワンショット出力

```console
$ cargo run -q -p tm-agent -- --once --demo | head -c 120
{"schema_version":1,"total_process_count":28,"snapshot":{"sampled_at":...
$ cargo run -q -p tm-agent -- --once | head -c 120
{"schema_version":1,"total_process_count":618,"snapshot":{"sampled_at":...
```

### 4.2 サーバ動作（127.0.0.1:18787、`--token test`）

| リクエスト | 結果 |
|---|---|
| `GET /healthz` | `200 {"ok":true,"schema_version":1}` |
| `GET /api/v1/snapshot`（トークンなし） | `401` |
| `GET /api/v1/snapshot?token=test`（クエリトークン） | `401`（クエリ文字列のトークンは拒否） |
| `GET /api/v1/snapshot` + `Host: evil.example` + 正トークン | `403`（DNS リバインディング多層防御） |
| `GET /api/v1/snapshot?limit=5`（Bearer test） | `200`、`total_process_count=618`、`processes` 5 件、約 137 KB（既定 500 件） |
| `OPTIONS` + `Origin: http://127.0.0.1:5173` | `Access-Control-Allow-Origin` を反射、`Vary: Origin` |
| `POST /api/v1/snapshot` | `405` |
| `--listen 0.0.0.0:18787` | 起動拒否（exit 1、ループバック強制） |

## 5. wasm / ブラウザ（GPUI Web）

`poc-wasm/` のスパイク（詳細: `notes/wasm-spike.md`）:

| 項目 | 結果 |
|---|---|
| `cargo check --target wasm32-unknown-unknown` | 成功（stable、`default-features = false`） |
| `trunk build --release` | 成功（trunk 0.22.1） |
| ヘッドレス Chrome 157 での描画 | 成功（WebGPU アダプタなし → **WebGL2 / SwiftShader にフォールバック**） |
| CJK テキスト | `CanvasFontFallback::EmojiAndCjk` で表示成功 |
| スクロール・クリック・検索入力・1 秒定期更新 | 成功 |
| HTTP fetch | 成功（`/stats.json` → HTTP 200） |
| wasm サイズ（スパイク時点） | 14 MB（gzip 3.6 MB）+ JS 154 KB。後に wasm-opt -Oz を適用し 7.24 MiB（gzip 2.89 MiB、§10） |
| 証拠 | `notes/assets/wasm-spike-*.png`、`wasm-spike-evidence-headless-default-release.json` |

### 5.1 ブラウザ版シェル（`web/`）

`web/` に読み取り専用シェルを実装し、ヘッドレス Chrome 157 + CDP で検証した
（詳細: `notes/web-shell.md`）。

| モード | 結果 |
|---|---|
| デモ（DemoProbe） | 描画・1 秒更新・スクロール・行クリック・絞り込み・ソートを確認（failures 0） |
| ライブ（`tm-agent --redact`） | 実プロセス 500 件（全 636 件中）を表示。行クリック・絞り込み・ソート・更新を確認（failures 0）。`#token=` + `Authorization: Bearer` を CDP の Network 証拠で確認 |
| エージェント停止 | デモへフォールバックせず、`live_error=1` とエラー表示（failures 0） |
| コンソール | panic / JS 例外 0（WebGL2 フォールバック等の想定内警告のみ） |

- `cargo check --target wasm32-unknown-unknown --manifest-path web/Cargo.toml`: 成功
- `cd web && trunk build --release`: 成功（wasm-opt -Oz 適用後 7.24 MiB / gzip 2.89 MiB、JS 155 KiB）
- 証拠: `notes/assets/web-shell-*.png`（18 枚）、`web/verification/web-shell-evidence-*.json`

## 6. 既知の制限・未検証

- Windows / macOS の実機ビルド・実行は未検証。
- X11 セッションでの表示は未検証。
- 合成入力によるキーボード E2E は未実施（手動・スクリーンショット確認のみ）。
- IME による日本語入力は未対応。
- KWin D-Bus の `queryWindowInfo` は本環境で `NoReply`。ウィンドウ確認は Spectacle で代替。
- WebGPU の実 GPU 描画、Safari / Firefox、日本語 IME は未検証。
- エージェントはシリアル処理のため 1 スナップショット約 250 ms。同時リクエストはキューされる。
- `cargo-deny` は未導入（ライセンス・重複版チェックは今後の CI 課題）。
- ライセンス/帰属は `NOTICE.md` に記載。

## 7. 再現手順（要約）

```bash
cd work/gpui-task-manager
bash scripts/dev-setup.sh          # libxkbcommon-x11 のローカル shim（必要なホストのみ）
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build -p gpui-task-manager
./target/debug/gpui-task-manager --demo   # 合成データ
./target/debug/gpui-task-manager          # 実データ（Wayland）
# エージェント
cargo run -p tm-agent -- --once --demo
cargo run -p tm-agent -- --listen 127.0.0.1:18787 --token test
# wasm スパイク
cd poc-wasm && trunk build --release
```

## 8. 独立レビューと対応（2026-11-14）

`notes/review-2026-11-14.md` の独立レビュー（blocker 0 / major 8 / minor 35）を受け、
以下を修正した。

| 指摘 | 対応 |
|---|---|
| M1 クエリ文字列のトークン | `Authorization: Bearer` のみに変更。ブラウザは URL フラグメント + ヘッダ方式 |
| M2 「短命トークン」記述 | 実装に合わせて「プロセス生存中は固定・再起動で無効」に修正 |
| M3 Enter で即確定 | Enter = キャンセル、Ctrl+Enter = 確定に変更。設計書も修正 |
| M4 2.24 MB の性能記述 | 現行実測（618 プロセス約 173 KB / 既定 500 件約 137 KB）に更新。旧値はスレッド込み 3,549 エントリの測定と注記 |
| M5 verification の数値不一致 | 動的数値を削除し画像参照に統一。テスト件数を 106 に更新 |
| M6 個人情報 | ネイティブ / エージェントに `--redact` を実装。スクリーンショット・証拠 JSON・デモデータを汎用化 |
| M7 コミット未作成 | 論理単位のコミットを作成（`Cargo.lock` 含む） |
| M8 CI 参照先 | `Seduce0068/workflows@v1` に修正（旧 `onoderakeita` 名はリダイレクト。Actions の `uses:` は現行名を使う） |
| minor（Host / timeout / user サニタイズ / bidi / overlay / .gitignore / NOTICE 等） | 主要項目を修正。詳細は `notes/review-2026-11-14.md` とコミット履歴を参照 |

再検証: `bash scripts/check.sh` 成功（fmt / clippy / test 108 件 / wasm check）。

### 追加対応（レビュー残余）

- `--redact` がコマンド引数内のホスト名・tailnet ドメイン（切り詰められた
  `https://host.tailXXXX.t…` を含む）も置換するよう修正。実データで
  ローカルホスト名 / tailnet ID / `/home/<user>` / ユーザー名が 0 件であることを確認。
- `tm-agent` の USAGE から `?token=` を削除（実装は拒否）。
- `app/native-libs/` を Git 管理から除外（絶対パス symlink のため）。
- `tm-platform` の未使用 serde 依存を削除。
- ネットワーク集計から `lo`、ディスクから `total=0` の擬似マウントを除外。
- スレッド数不明は「—」表示。`format_clock` / `usage_color` の単体テストを追加。
- ADR の番号順を修正。

## 9. 安全強化（追加ラウンド、2026-11-14）

「安全第一」の方針で、危険操作の防止・認証・プライバシー・テストを追加した。

| 項目 | 内容 | 検証 |
|---|---|---|
| 確認ダイアログの対象表示 | PID・ユーザー・起動からの経過・コマンドを表示し、取り違えを防止 | viewmodel テスト（identity 捕捉） |
| 安全側キー操作 | Enter = キャンセル、Ctrl+Enter = 確定。キー対応を関数化 | app テスト 4 件 |
| Kill シグナルの保持 | 要求したシグナルが確認後も保持される | viewmodel テスト |
| サニタイズ堅牢性 | 制御文字・ANSI・bidi を含む 500 ケースの決定論的検査 | tm-core テスト |
| トークン入力の安全化 | `--token-file` / `TM_AGENT_TOKEN` を追加、`--token` は警告付きテスト用 | args / main テスト + 実機で `/healthz` 200・トークンなし 401・Bearer 200、argv にトークンが現れないことを確認 |
| エージェントの安全テスト | stale start_time・PID 0/1/self・Host 検証・クエリトークン拒否・redact | platform / agent テスト |
| 公開物の衛生 | `--redact` がコマンド引数内のホスト名 / tailnet ドメインも置換 | 実データで 0 件を確認 |
| ポリシー文書 | `SECURITY.md`（安全原則・脅威モデル・残余リスク） | レビュー |

追加後のゲート: `bash scripts/check.sh` 成功（fmt / clippy / test 118 件 / wasm check）。

## 10. ツール導入（2026-11-15）

| ツール | 内容 | 結果 |
|---|---|---|
| Dependabot | `.github/dependabot.yml`（Cargo ルート / web、GitHub Actions、週次）。GPUI 系は完全ピン運用のため ignore。セキュリティアラート / 自動修正を API で有効化 | `vulnerability-alerts.enabled=true` / `automated-security-fixes.enabled=true`。初回更新ジョブ 3 件成功、PR #1（actions/checkout 4→7）作成・PR CI 成功 |
| cargo-deny | `deny.toml`（ライセンス allowlist、脆弱性、重複版 warn、供給元 deny）。ルート / web の両方 | 両方 **advisories / bans / licenses / sources ok** |
| wasm-opt | `web/index.html` の `data-wasm-opt="z"`（trunk が wasm-opt を自動取得） | wasm 14,799,786 → **7,589,230 B**、gzip 3,656,199 → **3,024,986 B** |
| wasm-opt 後の Web 検証 | 既存 CDP スクリプト（デモ） | `{"failures": [], "passed": true}`、スクリーンショット 8 枚（`wasmopt-*.png`） |

## 11. 日本語 IME（ブラウザ版 / ネイティブ版、2026-11-15）

- `gpui-pre-web` の IME ミラー（隠し textarea）+ アプリ側 `EntityInputHandler` で実装。
- 変換中は未確定文字列を入力欄にのみ表示し、`Dashboard` へは確定文字列だけを反映。
- CDP `Input.imeSetComposition` / `Input.insertText` で「にほんご」→「日本語」を再現し、
  **32/32 チェック pass**、JS 例外 0。`FilterInput` の単体テストは `tm-viewmodel` に移設（28 件）、web は 4 件。
- ネイティブ版: `FilterInput` を `tm-viewmodel` に共有化し、`EntityInputHandler` を
  実装。`TM_DEBUG_IME=1` で「にほんご」（変換中・一覧 28 件のまま）→「日本語」
  （確定・一覧 0 件）を確認。`notes/assets/native-ime-*.png`。
  実 IME（IBus/fcitx5）での手動確認は未実施。
- 証拠: `notes/assets/ime-*.png`、`notes/assets/native-ime-*.png`、
  `web/scripts/cdp_verify_ime.py`。

## 12. Windows / macOS（2026-11-15）

| 検証 | 結果 |
|---|---|
| `cargo check --target x86_64-pc-windows-msvc --workspace` | 成功（app/GPUI 含む） |
| `cargo check --target x86_64-pc-windows-msvc -p tm-platform --tests` / clippy | 成功 |
| `cargo check --target aarch64-apple-darwin`（core / viewmodel / platform / agent） | 成功 |
| app の macOS ビルド | Linux ホストでは `gpui-pre-apple` の Metal シェーダ生成が動かず不可（環境制約）。CI `native-macos` で検証 |
| CI ジョブ | `native-windows` / `native-macos` / `deny` を追加。run 43612670190 で **5 ジョブすべて成功**（Windows 4m49s / macOS 1m11s / deny 22s / wasm 1m47s / gates 2m28s） |

- Windows のシグナル送信は `%SystemRoot%\System32\taskkill.exe` の絶対パス起動
  （PATH ハイジャック回避）。Stop/Continue は非対応。
- `thread_count` は 637 プロセスで `/proc/<pid>/status` の `Threads:` と照合し不一致 0。

## 13. 設定の永続化（2026-11-15）

保存対象は表示設定のみ（更新間隔・ソート列/方向・詳細パネル表示）。プロセス情報・
フィルタ・ホスト名・トークンはディスクに書かない。

### 13.1 保存経路（Runtime Proof）

```text
$ XDG_CONFIG_HOME=$(mktemp -d) TM_DEBUG_SETTINGS=1 ./target/debug/gpui-task-manager --demo --redact
TM_DEBUG_SETTINGS: saved interval_ms=1000 key=Cpu direction=Descending details=true path=Some("/tmp/.../gpui-task-manager/settings.json")

$ ls -l /tmp/.../gpui-task-manager/settings.json
-rw------- 1 user user 133 ... settings.json        # 0600
```

- 一時ファイル（`settings.json.tmp`）は残らない（原子的 rename）。
- JSON は `version` / `interval_ms` / `sort` / `details_visible` の 4 項目のみ。

### 13.2 復元経路（Runtime Proof）

`interval_ms=5000` / `sort=Name Ascending` / `details_visible=false` を書いた設定で起動:

- ステータスバー **「更新間隔 5000 ms」**、ツールバー **「並び: 名前 ▲」**、
  一覧は名前昇順（avahi-daemon, bash, bluetoothd, ...）、**詳細パネル非表示**。
- 証拠: `notes/assets/settings-restore.png`。
- 起動後も設定ファイルは変更されない（Tick では書き込まない設計）。

### 13.3 ブラウザ版（localStorage、CDP 検証）

`web/scripts/cdp_verify_settings.py`（ヘッドレス Chrome + CDP）:

| 手順 | 結果 |
|---|---|
| 初期化（保存削除 → 再読込） | `sort=CPU;dir=desc`（既定） |
| CPU 列ヘッダをクリック | `localStorage` に保存。スキーマは `version` / `interval_ms` / `sort` / `details_visible` の 4 キーのみ（トークン等を含まない） |
| `interval_ms=5000, Name/asc, details=false` を書いて再読込 | `sort=名前;dir=asc` に復元 |

- 結果: `{"passed": true, "failures": []}`。証拠: `web/verification/web-settings-evidence.json`。
- スキーマはネイティブと共通（`tm-viewmodel::Settings`）。

### 13.4 自動テスト

`app`（12 件）に往復・原子的書き込み・`0600` 権限・範囲クランプ・未知フィールド無視・
NotFound/InvalidData の扱いを含む。

## 14. Linux DEB パッケージング（2026-11-15）

```bash
bash packaging/linux/build-deb.sh              # release ビルド + .deb 生成（root 不要）
bash packaging/linux/build-deb.sh --skip-build # 既存バイナリから再パッケージ
```

- 出力: `packaging/linux/build/gpui-task-manager_0.1.0-1_amd64.deb`（**11 MB**）。
- 依存は `ldd` + `dpkg -S` から自動生成し、`libvulkan1` を明示追加:
  `libc6, libgcc-s1, libvulkan1, libxau6, libxcb1, libxcb-xkb1, libxdmcp6,
  libxkbcommon0, libxkbcommon-x11-0`
- `dpkg-deb -x` で展開したバイナリの `--version` → `gpui-task-manager 0.1.0`。
- `desktop-file-validate` 成功。`sudo` を呼ばず、ホストへインストールしない。
- 未実施: 署名、リポジトリ公開、AppImage/Flatpak、Windows/macOS パッケージ。

## 15. CI ジョブ追加（2026-11-15）

| ジョブ | 内容 |
|---|---|
| `release-linux` | release バイナリ + DEB を artifact として保存 |
| `web-dist` | trunk でブラウザ版 dist をビルドし artifact として保存 |

- 既存の gates / wasm / cargo-deny / native-windows / native-macos と合わせて 7 ジョブ。
- `taiki-e/install-action` は trunk に対応（TOOLS.md で確認）。

## 2026-11-15（機能・運用拡張の安全な進捗）

- CI/運用: `wasm` / `cargo-deny` / `release-linux` / `web-dist` を `ubuntu-24.04` に固定、
  成果物 upload を `actions/upload-artifact@v7` に更新、SBOM 生成ジョブ
  （`cargo-cyclonedx` + `scripts/generate-sbom.sh`）を追加。GitHub ラベル
  （`area:*`, `ci`, `security`, `packaging`, `supply-chain` 等）を整備。
  private リポジトリの branch protection API は `403`（プラン制約）のため、
  必須 CI/レビュー等は運用で補う方針を文書化。
- モデル/収集: `tm-core` に `ComponentSnapshot` と `SystemSnapshot.components`
  （`serde(default)`）を追加。`tm-platform` で sysinfo `component` feature を有効化し、
  ベストエフォートで温度センサー情報を収集（利用不可環境では空）。
  demo スナップショットにもサンプル component を追加。
- プロセスツリー基盤: `tm-core::query` に `ProcessTreeRow` /
  `flatten_process_tree` を追加（`parent_pid` ベース、選択 subset の深さ優先）。
- 検証: `cargo test -p tm-core -p tm-platform`、主要クレートの `cargo check`
  （tm-viewmodel / tm-agent / gpui-task-manager）を確認。`cargo-vet` は未導入のため
  SBOM/運用ドキュメントで将来検討として扱う。

## 2026-11-15（ツリー表示基盤/UIと温度表示の追加・ローカル検証）

- 実装:
  - `tm-viewmodel::Settings` に `tree_view`（`serde(default)`）を追加し、native/web で復元・保存・コマンド反映。
  - `tm-viewmodel::Dashboard` に `ToggleTreeView` / `SetTreeView`、`visible_depths`、ツリー時の表示順再計算を追加。
  - native/web のプロセス一覧に階層インデント表示とツリー切替 UI を追加（`parent_pid` ベースの `flatten_process_tree` を表示順へ適用）。
  - `tm-core::query` にツリー順序の回帰テストを追加（子が先に並ぶ入力、兄弟順維持）。
  - `SystemSnapshot.components` の温度情報を native/web 概要カードに表示（取得不可時は「センサーなし」、最大/critical があれば併記）。
  - `app/src/ui.rs` / `web/src/widgets.rs` の `kv` ラベルを動的文字列に拡張。
- 検証（ローカル）:
  - `cargo fmt --all`
  - `cargo check -p gpui-task-manager -p tm-agent -p tm-viewmodel`
  - `cd web && cargo check`
  - `cargo test -q -p tm-core`（54 passed）
  - `cargo test -q -p tm-viewmodel`（34 passed）
  - `cargo test -q -p tm-platform`（9 passed）
  - `cargo test -q -p tm-agent`
  - `cargo test -q -p gpui-task-manager`（12 passed）
  - `cd web && cargo test -q`（7 passed）
- 制限/未検証:
  - ツリー表示は折りたたみ UI なしの基本表示。フィルタ/ソート時は表示対象集合に対するフラット化。
  - 温度/コンポーネントはベストエフォート（環境によりセンサーなし）。GPU 利用率の詳細監視は未実装。
  - 実 IME 手動確認、Windows/macOS 実機、WebGPU 実機確認は未実施。

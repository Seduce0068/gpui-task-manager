---
type: Reference
title: GPUI Task Manager 検証ログ（2026-11-14）
description: ネイティブ試作・エージェント・wasmスパイクの Runtime Proof と既知の制限
tags: [verification, runtime-proof, gpui, linux, wayland, wasm]
status: verified
generated: 2026-11-14T21:10:00+09:00
verified: 2026-11-17T01:59:00+09:00
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
| `cargo test --workspace` | **197 passed / 0 failed**（tm-core 84 / tm-viewmodel 37 / tm-platform 11 / tm-agent 50 / app 15、2026-11-17） |
| web ユニットテスト | **12 passed / 0 failed**（theme 4 + settings 3 + agent 4、2026-11-17） |
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
- `cd web && trunk build --release`: 成功（wasm-opt -Oz 適用後 7.24 MiB / gzip 2.89 MiB、JS 155 KiB。2026-11-15 再計測: 7,635,554 B / gzip 3,042,856 B、JS 159,873 B。2026-11-16 再計測: 7,641,445 B / gzip 3,044,050 B、JS 159,873 B）
- 証拠: `notes/assets/web-shell-*.png`（18 枚）、`web/verification/web-shell-evidence-*.json`

## 6. 既知の制限・未検証

- Windows / macOS の実機ビルド・実行は未検証。
- X11 セッションでの表示は未検証。
- 合成入力によるキーボード E2E は未実施（手動・スクリーンショット確認のみ）。
- KWin D-Bus の `queryWindowInfo` は本環境で `NoReply`。ウィンドウ確認は Spectacle で代替。
- WebGPU の実 GPU 描画、Safari / Firefox は未検証。
- 日本語 IME（composition）はブラウザ版・ネイティブ版とも実装済み。ブラウザ版は CDP
  32 チェック pass、ネイティブ版は `TM_DEBUG_IME=1` で状態遷移を確認（詳細は `notes/ime.md`）。
  実 IME（IBus/fcitx5）の手動確認は未実施。
- エージェントのサンプリングは共有プローブで直列化（1 スナップショット約 250 ms）。
  接続は最大 8 本まで並列で、超過分は即 503（後述の 2026-11-16 エントリ参照）。
- `cargo-deny` / `cargo audit` は CI ゲート化済み（ルート / web とも ok、理由付き ignore あり）。
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

- 一時ファイルは残らない（原子的 rename）。当時の名前は `settings.json.tmp` だったが、
  2026-11-17 以降は PID 付き `settings.json.tmp.<pid>`（同時保存の競合回避）。
- 当時の JSON は `version` / `interval_ms` / `sort` / `details_visible` の 4 項目のみ。
  現行スキーマ v2 はこれに `tree_view` / `theme` を加えた 6 項目。

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
  - ツリー表示はノード単位の折りたたみ/展開 UI を実装（折りたたみ状態はセッション内）。フィルタ/ソート時は表示対象集合に対するフラット化。
  - 温度/コンポーネントはベストエフォート（環境によりセンサーなし）。GPU 利用率の詳細監視は未実装。
  - 実 IME 手動確認、Windows/macOS 実機、WebGPU 実機確認は未実施。

## 2026-11-15（ツリー表示の整合性テストと依存監査の再確認）

- 実装/テスト:
  - `tm-viewmodel` の `visible_rows().has_children` を表示順/深さから算出するよう修正。
  - ツリー表示の viewmodel テストを追加（階層フラット化、深さ、`has_children`、トグル/明示設定時の再計算）。
- 検証（ローカル）:
  - `cargo fmt --all`
  - `cargo test -q -p tm-viewmodel`（36 passed）
  - `cargo test -q -p tm-core -p tm-platform -p tm-agent -p gpui-task-manager`
  - `cd web && cargo test -q`（7 passed）
  - `cargo deny check`（advisories ok, bans ok, licenses ok, sources ok）

## 2026-11-15（詳細パネルへのセンサー/温度表示追加）

- 変更:
  - native/web の詳細パネルに「センサー / 温度」セクションを追加し、取得済みコンポーネントを全件表示（ラベル・現在/最大/危険温度、未取得時は「センサーなし」）。
- 検証（ローカル）:
  - `cargo fmt --all`
  - `cargo check -q -p gpui-task-manager`
  - `cd web && cargo check -q`
  - `cargo test -q -p tm-core -p tm-viewmodel -p tm-platform -p tm-agent -p gpui-task-manager`
  - `cd web && cargo test -q`（7 passed）

## cargo-vet（初期化済み・運用方針）

- 現状: `cargo-vet 0.10.3` 導入済み。`supply-chain/` を root（804 exempt）/ web（560 exempt）に作成。`imports.lock` は空（外部 imports 未使用）、`audits.toml` は空（監査記録なし）。
- 方針: trust root は自リポジトリのみ。全依存を `exempt`（未監査の明示）として出発し、実レビューできた依存から `audits.toml` に記録する。未監査依存を監査済みとして記録しない。
- 更新手順: Dependabot/ピン更新でバージョンが変わったら `cargo vet --locked` が失敗する。実レビュー後に `certify`、レビュー前なら理由付き `add-exemption` で対応する。
- 検証: `cargo vet --locked`（root: `Vetting Succeeded (804 exempted)` / web: `Vetting Succeeded (560 exempted)`、2026-11-15）。`cargo vet suggest` は crates.io のソース取得（ネットワーク）が必要で、制限環境では失敗する。
- CI ゲート化: 未実施。共通ワークフロー（`Seduce0068/workflows` `rust-gates.yml@v1`）側での対応可否を先に検討する。
- 当面の代替/併用: `cargo audit`、`cargo-deny`、CycloneDX SBOM（`scripts/generate-sbom.sh` / CI `sbom` ジョブ）。


## 2026-11-15（運用拡張項目の再確認・cargo-vet評価軸追記）

- GitHub 運用（再確認）:
  - ラベル: `gh label list` で標準ラベルに加え `area:*`, `ci`, `security`, `packaging`, `supply-chain`, `dependencies`, `github_actions` 等が存在することを確認。
  - branch protection: `gh api repos/Seduce0068/gpui-task-manager/branches/main/protection` は `403 "Upgrade to GitHub Pro or make this repository public..."`（現行 private プラン制約）。必須チェック強制は未適用のため、CI 通過確認・PR/レビュー運用・ラベル運用で代替。
  - Actions: `.github/workflows/ci.yml` に `ubuntu-latest` はなく、`ubuntu-24.04` / `windows-latest` / `macos-latest` を明示。Actions は現状バージョンタグ参照（例: `actions/checkout@v7`）で、Dependabot（GitHub Actions 週次）で更新追従する構成。文書中の「SHA ピン留め」表記は実態に合わせて修正。
- 供給網/運用整備（確認）:
  - `deny.toml`、`scripts/generate-sbom.sh`、CI `deny`/`sbom` ジョブ、`.github/dependabot.yml`（Cargo root/web + GitHub Actions 週次、GPUI 系は更新 ignore）が存在。
  - `cargo-vet` は未導入（`supply-chain/` なし、ローカル `cargo vet` 未インストール）。`notes/cargo-vet-adoption.md` に導入判断の比較軸（trust/imports、criteria、exempt 初期方針、更新運用、証跡健全性）を追記。未レビュー依存を監査済みとして記録しない方針は維持。
- 機能面（既存確認）:
  - プロセスツリー表示/折りたたみ、`SystemSnapshot.components`（温度/センサー、`serde(default)`）、native/web のセンサー表示、`tm-agent` `schema_version: 1` での components シリアライズは既存実装・検証済み。詳細 GPU 利用率/プロセス別 GPU 指標は未実装。

## 2026-11-15（cargo-vet 初期化・全依存 exempt ベースライン）

- `cargo-vet 0.10.3` を `cargo install cargo-vet --version 0.10.3 --locked` で導入。
- `cargo vet init` を root / web で実行し、`supply-chain/`（config/audits/imports.lock）を作成。imports は未使用、監査記録は空（全依存を `exempt` として明示、root 804 / web 560）。
- 検証: `cargo vet --locked`（root: `Vetting Succeeded (804 exempted)` / web: `Vetting Succeeded (560 exempted)`）、`cargo vet fmt` で差分なし。`cargo vet suggest` は crates.io のソース取得（ネットワーク）が必要で制限環境では失敗（記録のみ）。
- ドキュメント: `README.md` / `SECURITY.md` / `notes/cargo-vet-adoption.md` を現状に合わせて更新。
- 未実施: CI ゲート化（共通ワークフロー側での検討を優先）、実レビューに基づく `certify`、外部 imports の評価・採用。

## 2026-11-15（cargo-vet の CI / ローカルゲート追加）

- CI: `Seduce0068/workflows` 共通 `rust-gates.yml@v1` の `extra_gates` 入力で `cargo install cargo-vet --version 0.10.3 --locked` → `cargo vet --locked`（root / web）を必須化。共通ワークフロー自体の変更は不要（`extra_gates` 既存入力の利用。taiki-e/install-action は cargo-vet 未対応のため cargo install）。
- ローカル: `scripts/check.sh` に `cargo vet --locked`（root / web）の optional ゲートを追加（cargo-vet 未導入環境では skip、cargo-deny と同じパターン）。`shfmt -d -i 2` 準拠。
- 検証: `bash -n` / `shellcheck`（check.sh）/ `shfmt -d -i 2`（差分なし）/ `ci.yml` の YAML パース。`bash scripts/check.sh` をローカル実行して結果確認。
- CI 実行結果の確認は push 後に `gh run` で行う（初回 extra_gates の成否を含む）。

## 2026-11-15（CI 全ジョブの起動失敗: GitHub Actions 請求エラー）

- 事実: 2026-11-15 07:11Z（16:11 JST）以降の push で CI 全ジョブが 3〜5 秒で失敗。アノテーション: `The job was not started because recent account payments have failed or your spending limit needs to be increased. Please check the 'Billing & plans' section in your settings`。最終成功は `refactor: share settings schema and persist web preferences`（2026-11-15T03:55:29Z、12:55 JST）。
- 影響: gates（fmt/check/test/clippy/audit/extra_gates の cargo vet 含む）/ wasm / native-windows / native-macos / deny / release-linux / web-dist / sbom の全ジョブが未起動。コードや workflow 定義の問題ではない（0 step で失敗）。
- 対応: アカウントの Billing & plans（支払い方法 / spending limit）の確認・修正が必要（アカウント設定のためエージェントでは対応不可）。修正後に `gh workflow run` または再 push で再検証する。
- 代替: 上記期間のリポジトリ変更はローカルの標準検証ループ（`bash scripts/check.sh`、exit 0）で検証済み。

## 2026-11-15（cargo audit 再実行・ドキュメント整合監査）

- `cargo audit`: exit 0、既知の 8 件 allowed warnings（unmaintained: instant / paste / rustls-pemfile / rustybuzz / ttf-parser / zbus_names / zvariant / zvariant_derive）のみで新規アドバイザリなし。
- ドキュメント整合監査（本日の変更後）:
  - `DESIGN.md` §7.2 供給網 / §15 残余リスクの「cargo-vet 未導入」記述を初期化済みの現状に更新。
  - `notes/index.md` の cargo-vet-adoption.md の概要説明を更新。
  - `notes/verification.md` の日付付き過去エントリ（:120 / :309 / :377）は履歴として修正しない。
  - README の CI ジョブ数（8 ジョブ）は `.github/workflows/ci.yml` と一致を確認。

## 2026-11-15（generate-sbom.sh のローカル検証・収集バグ修正）

- 発見: `scripts/generate-sbom.sh` は `cargo-cyclonedx 0.5.10` の実出力名 `sbom-root.json`（メンバーごと）に対し、収集パターン `sbom-root*.cdx.json` が不一致のため成果物 0 件になり、CI `sbom` ジョブ（`if-no-files-found: error`）が失敗する。さらに basename の `cp` では root 側 5 ファイルが同名衝突で 1 件に潰れる。**sbom ジョブは最後の CI 成功（2026-11-15 12:55 JST）後の `306ed2b`（13:34 JST）で追加されており、CI で一度も検証されていなかった。**
- 修正: 収集パターンを `sbom-root*.json` / `sbom-web*.json` に拡張、メンバーディレクトリを含む一意名（例: `crates__tm-core__sbom-root.json`）で収集、収集後はスクリプト固有名の元ファイルを除去（汎用名 `bom.json` は対象外）。
- 検証（Runtime Proof）: `cargo-cyclonedx 0.5.10` 導入（`cargo install cargo-cyclonedx --locked`）→ `bash scripts/generate-sbom.sh target/sbom-local` → **6/6 ファイル収集、全件 `bomFormat == CycloneDX`（jq 検証）**、作業ツリーに残骸なし、`bash -n` / `shellcheck` / `shfmt -d -i 2 -bn -ci` OK。
- 補足: `notes/cargo-vet-adoption.md` §9 にネットワーク環境での cargo-vet 試行手順（Runbook）を追加。

## 2026-11-15（ブラウザ版の再ビルド・CDP 再検証と文書の現状合わせ）

- `trunk build --release`（`web/`）: 成功。生成物を再計測: wasm 7,635,554 B（gzip 3,042,856 B）、JS 159,873 B（gzip 24,504 B）、`index.html` 1,320 B（gzip 842 B）。旧記録（7,589,230 B / 3,024,986 B、JS 158,865 B）から微増。
- CDP 再検証（デモモード、ヘッドレス Chrome 157 + CDP 9223、静的配信 8091）: **failures: [] / passed: true**、panic・JS 例外 0、benign console 7 件。状態フックの時系列は既存文書の表と同一挙動（tick 増加、行クリックで選択 PID 変化、`fire` で 28→1 行、Esc で復帰、CPU/名前ソート切替）。証跡を `web/verification/web-shell-evidence-demo.json` / `notes/assets/web-shell-0*-demo.png` に更新（現行コードは `ime_composing` / `ime_field` / `ime_marked` / `ime_selection` を状態フックに含むため、証跡にもこれらが記録される）。
- 文書の現状合わせ: `README.md` / `DESIGN.md`（§8 性能、ADR-017、§14）/ `web/README.md` / `notes/web-shell.md` の wasm サイズ記載を 2026-11-15 計測値に更新（7.24 MiB → 7.28 MiB、gzip 2.89 → 2.90 MiB）。ADR-017 には再計測値を追記し、旧測定の履歴は保持。
- 未実施: 実 GPU の WebGPU、Safari/Firefox。ライブ / エラー時モードは次節で再検証済み。

## 2026-11-15（ブラウザ版ライブ / エラー時モードの CDP 再検証）

前節（デモ再検証）に続き、CI で検証できない 2 モードをローカルで再検証した。実行は「`web/dist` を `python3 -m http.server 8091` で配信 → ヘッドレス Chrome 157（CDP 9223）→ `web/scripts/cdp_verify_web.py`」の手順。検証後は Chrome / HTTP サーバ / `tm-agent` の残留をすべて停止し、ポート 8091 / 9223 の解放を確認。

### ライブモード（`tm-agent` 接続）

`TM_AGENT_TOKEN=test ./target/debug/tm-agent --listen 127.0.0.1:18787 --redact`（`cargo build -p tm-agent` で直前ビルド）を起動し、
`?agent=http://127.0.0.1:18787#token=test` で検証 → **failures: [] / passed: true**、panic・JS 例外 0 件。

| step | tick | rows | filter | selected_pid | sort | dir | live_error |
|---|---|---|---|---|---|---|---|
| initial | 4 | 500 | (空) | 1894747 | CPU | desc | 0 |
| after_ticks | 6 | 500 | (空) | 1894747 | CPU | desc | 0 |
| scroll_down | 7 | 500 | (空) | 1894747 | CPU | desc | 0 |
| row_click | 8 | 500 | (空) | 1894783 | CPU | desc | 0 |
| filter_typed（`fire`） | 10 | 25 | fire | 1995347 | CPU | desc | 0 |
| filter_cleared（Esc） | 11 | 500 | (空) | 1995347 | CPU | desc | 0 |
| sort_cpu | 11 | 500 | (空) | 1995347 | CPU | asc | 0 |
| sort_cpu_flip | 12 | 500 | (空) | 1995347 | CPU | desc | 0 |
| sort_name | 13 | 500 | (空) | 1995347 | 名前 | asc | 0 |
| final | 14 | 500 | (空) | 1995347 | 名前 | asc | 0 |

- 実データで 500 行（`limit=500` = `DEFAULT_PROCESS_LIMIT`）、絞り込みで 25 行、`live_error=0` のまま全ステップ成功。デモ（28 行）と異なり実プロセス数を反映している。
- ネットワーク証跡: `GET /api/v1/snapshot?limit=500` を **14 回**（`preflight_count: 1`）。全 GET に `Authorization` ヘッダあり、**URL にトークンなし**（`token_in_url: false`）。`--redact` によりホスト名は `host` に置換されている。

### エラー時モード（エージェント未起動）

`tm-agent` を停止した状態（ポート 18787 は connection refused）で同 URL を開いて検証 →
**failures: [] / passed: true**、panic 0 件。状態フックは `mode=live;tick=0;rows=0;total=0;selected_pid=-;live_error=1` となり、
**ライブ取得失敗が `live_error=1` として UI に反映される**ことを確認。エラーメッセージはネイティブ版と同じ形（接続拒否 / `net::ERR_CONNECTION_REFUSED` 相当）で、panic やコンソール未処理例外にはなっていない。

### 証跡

- `web/verification/web-shell-evidence-live.json` / `web-shell-evidence-live-error.json`（更新）
- `notes/assets/web-shell-0{1..8}-*-live.png`、`notes/assets/web-shell-01-initial-live-error.png`、`web-shell-09-agent-error-live-error.png`（更新、1440x857 で整合）

### 制約（変わりなし）

- 実 GPU の WebGPU 描画は未確認（ヘッドレスは WebGL2 / SwiftShader へフォールバック）。Safari / Firefox も未検証。
- 物理実機 Windows / macOS の GUI 操作は未検証。

## 2026-11-15（日本語 IME（composition）CDP 再検証）

`web/scripts/cdp_verify_ime.py`（スクリプト自身が静的サーバと Chrome を起動・停止する自己完結型）を再実行し、`Input.imeSetComposition` / `Input.insertText` による変換・確定を再確認した。

- 結果: **32 チェックすべて pass**、`console_errors: []` / `exceptions: []`、スクリーンショット 13 枚、ステップ 11。
- 状態遷移（`filter` / `ime_composing` / `ime_field` / `ime_marked` / `rows`）:

| step | filter | ime_composing | ime_field | ime_marked | rows |
|---|---|---|---|---|---|
| initial | (空) | 0 | (空) | - | 28 |
| ascii-fire | fire | 0 | fire | - | 1 |
| backspace | fir | 0 | fir | - | 1 |
| escape-clear | (空) | 0 | (空) | - | 28 |
| composing | (空) | **1** | **にほんご** | 0:4 | **28** |
| composing-update | (空) | **1** | **日本語** | 0:3 | **28** |
| committed | **日本語** | 0 | 日本語 | - | **0** |
| escape-after-commit | (空) | 0 | (空) | - | 28 |
| composing-escape | (空) | 0 | (空) | - | 28 |
| recovered | 漢字 | 0 | 漢字 | - | 0 |
| final | (空) | 0 | (空) | - | 28 |

- 要点: **変換中（`ime_composing=1`）は絞り込みが確定せず 28 件のまま**で、確定（`committed`）後に初めて `filter=日本語` となり 0 件へ絞り込まれる。Esc による変換中断（`composing-escape`）でも一覧が 28 件に戻る。IME 後も通常入力（`recovered` = 漢字）が機能する。
- 証跡: `notes/assets/ime-0{1..10}-*.png`（1440x857）、`ime-05-composing-filter.png` / `ime-07-committed-filter.png`（1260x138、絞り込み欄の拡大）。詳細は `notes/ime.md`。
- 制約: 実 IME（IBus/fcitx5）での手動確認ではなく、CDP による composition イベント再現。ネイティブ版は `TM_DEBUG_IME=1` のデバッグフックによる検証のみ。

## 2026-11-16（ブラウザ版の設定永続化 CDP 再検証と検証スクリプトのスキーマ追随）

`web/src/shell.rs` / `crates/tm-viewmodel` の変更後に設定永続化の再検証を行った。

- 初回実行で **検証スクリプト側の不備** を検出: `web/scripts/cdp_verify_settings.py` の期待スキーマが
  `{version, interval_ms, sort, details_visible}` のままで、実装に追加された `tree_view`
  （`tm-viewmodel::Settings`、2026-11-15 のツリー表示対応）に追随していなかった
  （実際に保存されるキー: `details_visible, interval_ms, sort, tree_view, version`）。
- 修正: 期待スキーマに `tree_view` を追加し、復元経路で書き込む設定にも `tree_view: true` を含めた。
- 再実行結果: **`{"passed": true, "failures": []}`**、ステップ `fresh → saved → saved_parsed → restored` すべて成功。

| 項目 | 結果 |
|---|---|
| 保存された設定 | `version=1`, `interval_ms=1000`, `sort={Cpu, Ascending}`, `details_visible=true`, `tree_view=false` |
| 復元後の状態フック | `sort=名前`, `dir=asc`, `rows=28`（`localStorage` に書いた設定が再読込で反映） |
| トークン漏えい | `token` を含まないことを確認（プロセス情報・トークンは非保存） |

- 証跡: `web/verification/web-settings-evidence.json`（更新）。
- 検証後の Chrome / HTTP サーバは停止し、ポート 8091 / 9223 の解放を確認。
- 教訓: 実装のスキーマ変更時に検証スクリプトの期待値が取り残される。スキーマ変更時は
  `web/scripts/cdp_verify_settings.py` の期待キーも同時に更新する（今回の修正で解消）。

## 2026-11-16（ネイティブ版 IME デバッグフックの再検証）

`cargo build -p gpui-task-manager`（debug バイナリ、174 MB）をビルドし、`TM_DEBUG_IME=1` で
起動して state 遷移ログを再取得した（Wayland セッション上、`timeout 25` で終了）。

```text
$ timeout 25 env TM_DEBUG_IME=1 ./target/debug/gpui-task-manager
TM_DEBUG_IME: composing field="にほんご" marked=Some(0..4) filter=""
TM_DEBUG_IME: composing field="日本語" marked=Some(0..3) filter=""
TM_DEBUG_IME: committed filter="日本語" composing=false field="日本語"
（exit 124 = timeout による終了。GUI アプリのため常駐が正常）
```

- 変換中（`composing`）はフィールドに未確定文字列（にほんご → 日本語）と marked range が入り、
  **Dashboard の `filter` は空のまま**（一覧は絞り込まれない）。
- 確定（`committed`）で初めて `filter="日本語"` となり、`composing=false` になる。
- 証跡: `notes/assets/native-ime-evidence.txt`（ログ全文）、既存スクリーンショット `native-ime-0{1,2}-*.png`。
- 制約（変更なし）: これはデバッグフックによる `FilterInput` の状態遷移と Dashboard 反映の検証であり、
  **実 IME（IBus / fcitx5）の Wayland text-input-v3 / X11 XIM 経路は未検証**。
  実 IME での手動確認には日本語入力メソッドのインストールと対話操作が必要。


## 2026-11-16（Wayland text-input-v3 プロトコルレベルの観察）

実 IME 環境（fcitx5 + mozc が稼働、`XMODIFIERS=@im=fcitx5`、`GTK_IM_MODULE=fcitx5`）で、
`WAYLAND_DEBUG=1 TM_DEBUG_IME=1 ./target/debug/gpui-task-manager` を実行し、
wayland プロトコルの送受信ログ（3,055 行）を取得した。

### 確認できたこと（正常）

| 項目 | 観測 | 意味 |
|---|---|---|
| compositor の対応 | `wl_registry.global("zwp_text_input_manager_v3", 1)` | KWin が text-input-v3 を公開 |
| GPUI のバインド | `wl_registry.bind(71, "zwp_text_input_manager_v3", 1, id 18)` | GPUI がプロトコルを使用 |
| text_input 取得 | `zwp_text_input_manager_v3#18.get_text_input(id 27, wl_seat#3)` | シートに紐づけて取得 |
| seat capabilities | `wl_seat#3.capabilities(7)` | キーボード・ポインタ等が利用可能 |
| キーボード取得 | `wl_seat#3.get_keyboard(id 26)` | キーボードオブジェクト作成 |

→ **GPUI は Wayland text-input-v3 を正しくセットアップしている**
（依存 crate `gpui-pre-linux 0.3.12` の `src/linux/wayland/client.rs` で
`zwp_text_input_v3` を使用し、`enable()` / `disable()` を実装していることもソースで確認）。

### 観測されなかったこと（今回の実行条件による）

| 項目 | 観測 | 原因 |
|---|---|---|
| `zwp_text_input_v3#27.enable()` | **0 回** | ウィンドウがアクティブでないため |
| `wl_keyboard#26.enter` | **0 回** | 実行したウィンドウがキーボードフォーカスを得ていない |
| `wl_pointer.enter` | 0 回 | 同上 |

依存 crate の実装を読むと、`update_ime_enabled()` は
`state.active == false` のとき早期 return し、
`state.active` は `wl_keyboard.enter` 相当のフォーカスで `true` になる
（`window.rs:1610` の `set_focused`、`client.rs:2097` の `wl_keyboard::Event::Enter`）。
今回の実行ではフォーカスが発生しなかったため、`enable()` まで到達していない。

### 結論（検証の限界と次に必要なこと）

- **正常性の確認**: GPUI ↔ compositor 間で text-input-v3 のセットアップは成功している。
  アプリ側も paint 中に `window.handle_input(...)` を呼び、`filter_editing` のときだけ
  `accepts_text_input` が true を返す実装になっている（`app/src/app.rs`）。
- **未到達**: 実際のキーボードフォーカス下で `enable()` → `preedit_string` /
  `commit_string` が発行される経路は、実行環境でウィンドウをフォーカスして
  キー入力する必要があり、今回の非対話的実行では到達しなかった。
- **実 IME 経路を自動検証するには**、次のいずれかが必要:
  1. キー注入ツール（`ydotool` / `wtype` 等）の導入（※追加インストールは要確認）
  2. 対話セッションで手動操作し、`WAYLAND_DEBUG=1` のログと画面を確認
  3. アプリ側にデバッグ用のバッチ（フォーカス強制＋合成イベント注入）を足す
- 現状の実装・証跡の正直な位置づけ: **`FilterInput` の状態遷移と Dashboard 反映は
  デバッグフックで検証済み**だが、**実 IME からの入力経路は未検証**（虚偽の主張はしない）。

## 2026-11-16（CI 全ジョブ起動失敗の原因特定: private リポジトリの課金枠）

前節で「請求エラー」とだけ記録していた事象について、**対照実験により原因を特定**した。

### 対照実験の結果

同一アカウント（`Seduce0068`）の 2 リポジトリで、同時期の CI 実行結果が分かれた。

| リポジトリ | 公開設定 | 直近の CI | 実行時間 | 結果 |
|---|---|---|---|---|
| `gpui-file-explorer` | **public** | `docs: update test count from 36 to 132`（2026-11-15T16:02:50Z） | 5 分 33 秒 | ✅ **success** |
| `gpui-task-manager` | **private** | `docs: record wayland ...`（2026-11-15T15:57:27Z） | 4 秒 | ❌ failure（0 step） |

失敗ジョブのアノテーション（GitHub API で取得）:

```text
The job was not started because recent account payments have failed or your
spending limit needs to be increased. Please check the 'Billing & plans'
section in your settings
```

### 結論

- **原因は GitHub Actions の課金**: private リポジトリの Actions は課金対象（分数消費）であり、
  支払い失敗 / spending limit 未設定のためジョブが開始されない。
- public リポジトリ（`gpui-file-explorer`）は Actions が無料枠で動くため、同じアカウントでも成功する。
- したがって本件はコード / workflow 定義の問題ではなく、**リポジトリ公開設定と課金設定の問題**。

### 対応の選択肢（いずれもアカウント設定の変更が必要）

1. **`gpui-task-manager` を public 化** — CI が無料枠で動き即解決。ただし「まだ公開しない」方針と衝突するため要判断。
2. **Billing & plans で支払い方法を修正 / spending limit を設定** — private のまま CI を継続。
3. **ローカル検証を継続** — 本セッションで実施してきたとおり、CI で検証できない分は
   ローカルの Runtime Proof（fmt/check/test/clippy/deny/vet/CDP）で代替する。

エージェントからは 1・2 を実行できないため（アカウント設定 / 公開操作）、**ユーザー判断が必要**。
現状は 3 の方針で検証を継続している。

## 2026-11-16（ディスク一覧の表示順を修正: 容量の大きい順）

外部レビュー a7（「ディスクの擬似マウントが未フィルタ」）を確認したところ、`total_space() > 0`
フィルタは既にあり `tmpfs` / `overlay` は除外されていたが、**別の実害**が見つかった。

### 発見（Runtime Proof）

実機のディスク一覧は**マウントポイント名の昇順**でソートされていたため、
UI が `take(2)` で先頭 2 件しか表示しないこともあり、**大容量データ領域が隠れていた**。

```text
修正前（tm-agent /api/v1/snapshot の disks 順）
  0: /                        586 GiB
  1: /boot/efi                  0 GiB   ← 96 MB の EFI パーティションが 2 番目
  2: /mnt/timeshift           439 GiB   ← 表示されない
  3: /run/media/<user>/Windows 333 GiB  ← 表示されない
```

### 修正

- `crates/tm-platform/src/lib.rs`: ディスクを**容量の降順**（同容量はマウントポイント昇順）で
  ソートする `sort_disks_for_display` を追加。UI の表示枠を小さなシステムパーティションが
  占有しないようにする。
- 回帰テストを 2 件追加（容量順の並び、同容量時のタイブレーク）。
  `tm-platform` のテストは 9 → **11 件**。

### 検証（Runtime Proof）

- 修正後（実機）: `/`(586 GiB) → `/mnt/timeshift`(439 GiB) → `/run/media/<user>/Windows`(333 GiB) → `/boot/efi`。UI の `take(2)` で `/` と `/mnt/timeshift` が表示される。
- `cargo test --workspace`: **155 passed / 0 failed**（app 12 / tm-agent 39 / tm-core 56 / tm-platform 11 / tm-viewmodel 37）。
- `cargo clippy --workspace --all-targets -- -D warnings`: 警告 0。
- `cargo fmt --all` 適用済み。

## 2026-11-16（ネットワーク集計から仮想 IF を除外: 二重計上の修正）

外部レビュー a6（「ネットワーク集計が lo / 仮想 IF を含む」）を実測で確認したところ、
**VPN インターフェースによる二重計上**が実際に発生していた。

### 実測（Runtime Proof）

```text
enp4s0      rx=124,641,412,687  tx=28,381,393,862   ← 物理 NIC
tailscale0  rx=    79,901,838   tx=    35,192,774   ← 同じ通信が NIC 上も流れる
docker0 / br-* / lxcbr0 / virbr0  rx=0 tx=0         ← アイドルだが合算対象だった

合計（修正前の overview）: rx=124,721,314,525（tailscale 分を二重計上）
物理のみ（修正後）        : rx=124,641,412,687
```

`lo` は `tm-platform` 側で既に除外されていたが、`tm-platform` のフィルタは `lo` だけで、
`tailscale0` / `docker0` / `br-*` / `lxcbr0` / `virbr0` は集計に含まれていた。

### 修正

- `crates/tm-core/src/model.rs`:
  - `NetworkSnapshot::physical_totals()` を追加（loopback と仮想 IF を除外して合算）。
  - `is_virtual_interface(name)` を追加（`lo`、および `br-` / `docker` / `veth` / `virbr` /
    `lxcbr` / `tailscale` / `tun` / `tap` 接頭辞）。未知の名前は物理として扱う
    （表示から消える方が、稀な過大計上より有害なため）。
  - `SystemSnapshot::network_totals()` を `physical_totals()` に切り替え（overview の
    レート計算・履歴がこの関数を使う）。`totals()` は全 IF 合算として残す。
- 回帰テストを 2 件追加（`tm-core` 56 → **58 件**）:
  仮想 IF 名の判定（正例 9 / 負例 6）、物理 IF のみの合算と全 IF 合算の差。
- 既存テスト `tm-viewmodel::ingest_fills_history_and_rates` は旧挙動（`lo` を合算した
  1_238_663）を期待値に固定していたため、正しい値（1_234_567）に更新した。

### 検証（Runtime Proof）

- `cargo test --workspace`: **157 passed / 0 failed**
  （app 12 / tm-agent 39 / tm-core 58 / tm-platform 11 / tm-viewmodel 37）。
- `cd web && cargo test`: **7 passed / 0 failed**。
- `cargo clippy --workspace --all-targets -- -D warnings`: 警告 0。
- `cargo fmt --all` 適用済み。

## 2026-11-16（capability の意味を明確化・web shell の重複構築を解消）

外部レビュー a5（「`PlatformInfo` の capability が実態より強い」）を検討した。

### 判断

レビューの指摘は「`Networks` / `Disks` の初期化に失敗しても `supports_network` /
`supports_disks` が true を返す」というもの。実装を確認したうえで、**フラグの意味を
「ホストがその機能を持つか」と定義して据え置き**、代わりに意図を doc comment と
API で明確にした。

- 理由: `sysinfo` の列挙失敗は一時的（権限・コンテナ環境など）で、失敗のたびに
  capability を false にすると UI の表示が実行ごとに揺れる。データが無いことは
  空のスナップショット（`disks: []` / `interfaces: []`）で既に表現されている。
- 呼び出し側で「データが有るか」を知りたい場合は snapshot を見る、と明記した。

### 変更

- `crates/tm-core/src/port.rs`:
  - `PlatformInfo::native` の doc comment にフラグの意味（ホスト capability であり、
    列挙失敗は snapshot 側で表現する）を明記。
  - `PlatformInfo::native_read_only` を追加。ネイティブだがプロセス制御を持たない
    バックエンド（ブラウザ経由）を 1 か所で表現する。
  - テスト追加（`tm-core` 58 → **59 件**）: `native_read_only` は制御不可・
    demo ではない・network/disk は対応、を検証。
- `web/src/shell.rs`: 手書きの `PlatformInfo { .. }` リテラルを `native_read_only` に
  置き換え（capability の定義が 1 か所に集約され、乖離しにくくなる）。

### 検証（Runtime Proof）

- `cargo test --workspace`: **158 passed / 0 failed**（tm-core が 59 件に増加）。
- `cd web && cargo test`: **7 passed / 0 failed**、`cd web && cargo check`: 成功。
- `cargo check --target wasm32-unknown-unknown -p tm-core -p tm-viewmodel`: 成功。
- `cargo clippy --workspace --all-targets -- -D warnings` / web clippy: 警告 0。
- `cargo fmt --all` 適用済み。

## 2026-11-16（Ctrl+K の保護判定を view model に一本化）

外部レビュー a3（「UI 側に保護判定の重複」）を確認し、修正した。

### 問題

`app/src/app.rs` の Ctrl+K ハンドラが `!self.dashboard.is_protected(key)` を先に確認して
`Command::RequestSignal` を投げていたため、**保護プロセス（PID 1 / 自分自身）を
選択して Ctrl+K を押すと何も起きず、理由も表示されなかった**。
同じ操作をボタン（`signal-term` など）から行うと view model が
「PID 1 と自分自身は操作できません（安全のため保護されています）」を表示するため、
2 つの経路で挙動が食い違っていた。

### 修正

- `app/src/app.rs`: Ctrl+K は常に `Command::RequestSignal` を投げるようにし、
  保護判定とエラーメッセージは view model（`Dashboard::request_signal`）へ一本化。
  ボタン経路と同じ挙動・同じメッセージになる。
- シェル側の `is_protected` 呼び出しは、ボタンの有効/無効表示（`enabled` の計算）
  のみに残した（表示と実行の両方で保護される）。

### 検証（Runtime Proof）

- 保護フロー自体は `tm-viewmodel::kill_flow_requires_confirmation_and_is_protected` で
  検証済み（PID 1 / 自分自身は `pending_kill` にならず `status.is_error` になる）。
- `cargo test --workspace`: **158 passed / 0 failed**。
- `cargo clippy --workspace --all-targets -- -D warnings`: 警告 0。
- `cargo fmt --all` 適用済み。

## 2026-11-16（外部レビュー指摘の棚卸し: 多くが対応済みであることを確認）

`notes/review-2026-11-14.md` の指摘を 1 件ずつ現行コードと突き合わせ、**未対応とされていた項目の
大半が既に修正・設計変更されている**ことを確認した（虚偽の「対応済み」主張を避けるため、
根拠となるコード・文書の位置を記録する）。

| 指摘 | 現状 | 根拠 |
|---|---|---|
| a3 保護判定の重複 | **今回修正** | `app/src/app.rs` の Ctrl+K を view model 委譲に統一（commit `dd11ab0`） |
| a5 capability が実態より強い | **今回対応** | `PlatformInfo::native_read_only` 追加と意味の明文化（commit `f16c9b2`） |
| a6 ネットワーク二重計上 | **対応済み** | `NetworkSnapshot::physical_totals` と `is_virtual_interface`（commit `931c7a7`） |
| a7 ディスクの擬似マウント | **対応済み** | `total_space() > 0` フィルタ + 容量降順ソート（commit `67748a3`） |
| a9 wire schema 版管理 | **対応済み** | `web/src/agent.rs` が `schema_version != WIRE_SCHEMA_VERSION` を検証 |
| a13 bidi 記述の不一致 | **対応済み** | `DESIGN.md` §7.1 が「bidi 制御文字を除去」と実装一致 |
| a14 古い測定値 | **対応済み** | `DESIGN.md` / `RESEARCH.md` に「旧測定 2.24 MB」と注記 |
| M3 Enter で即確定（安全側既定でない） | **対応済み** | `dialog_command`: `enter` → `CancelKill`、`ctrl+enter` → `ConfirmKill`。`DESIGN.md:237-239` も一致 |
| M6c デモデータに実ユーザー名 | **対応済み** | `crates/tm-core/src/demo.rs` は `user` / `/home/user/...` に匿名化済み |
| s4 トークンの argv 露出 | **対応済み** | `--token-file` を追加し、`--token` の usage に「argv は他プロセスから見える」と明記 |
| s6 Authorization fallback バグ | **設計変更で解消** | クエリトークンを廃止し `Authorization` ヘッダのみ受理。不正ヘッダのテストあり |
| s7 PID 再利用の残余（秒精度/TOCTOU） | **文書化済み** | `DESIGN.md:367-369` に残余リスクとして明記（`pidfd` は将来課題） |

### 今回追加で修正した衛生項目

- `notes/cargo-vet-adoption.md` §7 の棚卸し手順に含まれていた**作業環境の絶対パス**
  （`/home/<user>/...`）を `work/gpui-task-manager` に置き換えた（M6 の方針:
  公開物に個人環境のパスを残さない）。
- `log.md` は追記専用の履歴のため、過去記録として保持（本方針は `~/.agents/AGENTS.md` §8 と整合）。

### 未対応として残るもの

- M5（過去のスクリーンショットと `verification.md` の数値差）: 動的な値のため、
  該当箇所は「画像参照」に寄せて再取得時に更新する方針（画像と文章を同時更新）。
- s1（シリアル処理によるローカル可用性）・s2（Host ヘッダ検証）・s3（CORS 範囲の記述差）:
  エージェントの設計判断が必要なため、次回以降に検討する。

  > **2026-11-17 追記**: 上記はレビュー時点（2026-11-14）の履歴。その後 s1 は接続の
  > 並列化（commit `2a3f0fe` ほか）、s2 は Host 検証 403（commit 履歴）、s3 は
  > 403 への CORS 付与（commit `cedc118`）でそれぞれ対応済み。現状は後続の
  > 「エージェントの並列接続化」および「Host 検証 403 への CORS 付与」の節を参照。

## 2026-11-16（エージェントのシリアル処理によるローカル可用性を実測）

外部レビュー s1（「シリアル処理によるローカル可用性低下」）を実測で再確認した。

### 実測（Runtime Proof）

`READ_TIMEOUT` は既に 5 秒 → **1 秒**に短縮されているため、レビュー当時（4,499 ms 遅延）
より影響は小さいが、**無送信接続の本数に比例して遅延する**性質は残っている。

| 条件 | `/healthz` の応答時間 |
|---|---|
| 無送信接続 1 本 | **49 ms**（1 秒の読み取りタイムアウト内に相手が閉じたため） |
| 無送信接続 3 本（同時保持） | **2,059 ms**（1 本あたり約 1 秒 = `READ_TIMEOUT` 分を直列に待つ） |

- 影響範囲: エージェントの可用性のみ（アプリ本体・システムには影響しない）。
- 前提: 接続できるのは `127.0.0.1` のみ（loopback バインド + `Host` 検証）なので、
  攻撃者は同一マシン上のプロセスに限られる。
- 通常運用: ブラウザ UI は「同時 1 リクエスト、応答を待ってから次を送る」方式
  （`web/src/agent.rs`）なので、この直列処理と競合しない。

### 対応（今回は文書化に留めた理由）

- `serve()` は `probe: &mut dyn SystemProbe` を受け取る設計で、並列化には
  `Mutex<Box<dyn SystemProbe + Send>>` への変更（`SystemProbe` の `Send` 境界追加）と
  同時接続数の上限・スレッドプールが同時に必要になる。
- これは公開 API と実行モデルに関わる変更で、`#![forbid(unsafe_code)]` の下でも
  実装可能だが、並行性の回帰リスクを伴うため、**設計判断として次回以降に分離**した。
- 現状の緩和: `127.0.0.1` バインド、`Host` の loopback 検証、読み取りタイムアウト 1 秒、
  1 接続 1 リクエスト、8 KiB ヘッダ上限。攻撃面は同一ホスト内に限定される。

### 改善案（次回検討）

1. `probe` を `Arc<Mutex<Box<dyn SystemProbe + Send>>>` にし、接続ごとにスレッドを
   spawn する（`sample()` は `&mut self` のためロックで直列化。サンプリングは
   250 ms 程度なので UI の応答性は保たれる）。
2. 同時接続数に上限（例: 8）を設け、超過分は即座に `503` を返して閉じる。
3. あるいは `accept` 直後だけ短い読み取りタイムアウト（例: 250 ms）を使い、
   ヘッダ受信後は通常の 1 秒に切り替える。

## 2026-11-16（ライトテーマ追加後の設定検証スクリプト追随・web 版との差の記録）

並行セッションが **ネイティブ版のライトテーマ（Catppuccin Latte）** を実装した
（commit `b1a6f85`: `app/src/app.rs` に切替ボタン、`app/src/theme.rs` のパレット切替、
`crates/tm-viewmodel/src/settings.rs` の `Settings.theme` と `SETTINGS_VERSION = 2`）。

### 検証スクリプトの追随（前回と同じ種類の不具合を再発見）

設定スキーマに `theme` が加わったため、`web/scripts/cdp_verify_settings.py` の期待キーが
古くなり、検証が失敗した（前回 `tree_view` で同じ問題を経験済み）。

```text
FAIL: 保存スキーマが想定どおり（実際: ['details_visible','interval_ms','sort','theme','tree_view','version']）
```

- 修正: 期待キーに `theme` を追加し、復元用の設定も `version: 2` / `theme: "Light"` に更新。
  冒頭の doc comment（スキーマ一覧）も同期。
- 再検証: **`{"passed": true, "failures": []}`**、保存内容は
  `version=2, interval_ms, sort, details_visible, tree_view, theme("Dark")` で
  トークン・プロセス情報を含まないことを確認。証跡を
  `web/verification/web-settings-evidence.json` に更新。

### web 版ライトテーマの実装と検証（2026-11-16）

- **実装**: `web/src/theme.rs` をネイティブ版と同じ `Palette` / `DARK`（Mocha）/
  `LIGHT`（Latte）/ `theme()` / `set_theme()` / `palette()` 構成へ拡張。`shell.rs` と
  `widgets.rs` の色参照 93 箇所を `palette().*` へ切り替え、ヘッダー右端に
  `toggle-theme-web` ボタン（固定幅 56 px）を追加。クリックで Dark ↔ Light を切り替え、
  `persist_settings()` で `theme` を保存する。`restore_settings()` は
  `set_theme(settings.theme)` で起動時に復元する。共有スキーマ
  （`tm-viewmodel::Settings`）により、ネイティブ版と web 版で同じ `theme` 値をやり取りできる。
- **状態フック**: `data-gpui-proof` に `theme=dark|light` を追加（CDP 検証用）。
- **単体テスト**: `cargo test -p tm-web` は **11 passed / 0 failed**（旧 7 件 + テーマ 4 件）。
  `cargo check --target wasm32-unknown-unknown` 成功。
- **CDP 検証**（`web/scripts/cdp_verify_settings.py`、`--evidence web/verification/web-settings-evidence.json`）:
  `{"passed": true, "failures": []}`。
  - fresh: `theme=dark`（既定）
  - saved: 保存 JSON のキーは `version/interval_ms/sort/details_visible/tree_view/theme` のみで
    `theme="Dark"`、トークン不含
  - restored: `{"theme": "Light"}` を書いて再読込 → `theme=light` と `sort=名前` を復元
  - 切替ボタン: Light → Dark → Light の 2 クリックで `theme=` と保存値が連動
  - 画面: `notes/assets/web-shell-light-theme.png`（light 状態のスクリーンショット）
- 旧状態（web が `theme: "Dark"` を固定保存していた非対称）は解消。
  `prefers-color-scheme` 連動などの自動更新は未実装。
- 再ビルド: `trunk build --release` 成功。wasm 7,641,445 B（gzip 3,044,050 B）、
  JS 159,873 B、`index.html` 1,320 B（2026-11-16 計測、旧 7,635,554 B から微増）。

## 2026-11-16（ブラウザ版: 概要カードの内容オーバーフローを修正）

- 現象: ライトテーマのスクリーンショット（`notes/assets/web-shell-light-theme.png`）で、
  システムカードの温度行（「CPU Packages」「NVMe Composite」）とツールバー
  （「ソート」「ツリー: ON」「表示 N 件」）が重なっていた。原因は概要エリアの
  固定高さ `OVERVIEW_HEIGHT = 186` に対し、システムカードが最大で
  3 行の基本情報 + ディスク 2 件（ラベル+バー）+ 温度 3 件を並べるため。
- 修正: `web/src/shell.rs` の `OVERVIEW_HEIGHT` を 260 px へ変更（3 件の温度でも
  カード内に収まる高さ）。CDP スクリプトの座標定数
  （`cdp_verify_web.py` / `cdp_verify_ime.py` の `OVERVIEW_H`）と
  `notes/web-shell.md` の記載も 260 に追随。
- 再検証（Runtime Proof）:
  - `cargo test -p tm-web` 11 passed / 0 failed、`cargo clippy --all-targets -- -D warnings` 成功、
    `trunk build --release` 成功。wasm サイズは不変（7,641,445 B / gzip 3,044,050 B）。
  - CDP デモモード: `{"failures": [], "passed": true}`。座標計算を更新した
    行クリック・列ソート・スクロールがすべて成功。証跡
    `web/verification/web-shell-evidence-demo.json` とスクリーンショット 8 枚を更新。
  - CDP 設定永続化: `{"passed": true, "failures": []}`（テーマ切替を含む）。
  - CDP IME: **32/32 OK**、Runtime 例外・コンソールエラー 0 件。証跡を
    `web/verification/ime-evidence.json` に初めてリポジトリへ保存（旧 runs は /tmp のみ）。
  - 目視: `notes/assets/web-shell-light-theme.png` で重なりが解消していることを確認。
    トレードオフとして表の表示行数が約 13 → 11 行に減少（カード内容を切らずに収める優先）。

## 2026-11-16（CI 全ジョブ成功の Runtime Proof）

- 長期間 continuum だった CI 起動失敗（8 ジョブが 0 step で数秒 failure）が解消。
  run [43885904734](https://github.com/Seduce0068/gpui-task-manager/actions/runs/43885904734)
  （commit `9e2cdae`）で gates / wasm / cargo-deny / native-windows / native-macos /
  release-linux / web-dist / sbom の **8/8 success** を確認（2026-11-16 23:33 JST 完了）。
- 直前 run の `cancelled` は同ブランチ連続 push による concurrency キャンセル。
  ローカルの `cargo test --workspace`（160 passed）/ clippy / fmt / web 11 tests と整合。

## 2026-11-16（エージェントの並列接続化: s1 の解消）

- 外部レビュー s1 で実測されていた「アイドル接続が `/healthz` を最大 2 秒阻塞する」
  問題を解消した。`agent/src/main.rs` に `SharedState`（`Arc<Mutex<Box<dyn SystemProbe>>>`
  ＋トークン＋`AtomicUsize` の接続数）を導入し、accept ごとにスレッドを spawn。
  共有プローブのロックは完全なリクエストヘッダ読み取り後の `route()` 呼び出し時だけに
  限定し、`read_request()` 中のアイドル待ちは他の接続をブロックしない。
- `MAX_CONNECTIONS = 8`。上限超過分は accept 直後に 503 を返して閉じる
  （loopback 限定だが、無制限のスレッド生成はローカルリソース枯渇のベクタになるため）。
- 回帰テスト 2 件追加（tm-agent 39 → **41 tests**）:
  - `idle_connections_do_not_block_health_check`: アイドル接続 2 本を保持しても
    `/healthz` が 800 ms 以内に応答（旧実装では約 2 秒）。
  - `excess_connections_are_rejected_immediately`: 8 本保持中の 9 本目が 503 を受け取り、
    接続数カウンタが 8 に戻ることを確認。
- **Runtime Proof**（実プロセス、`--demo --listen 127.0.0.1:18793 --token test`）:
  `/healthz` の応答時間は アイドル 0 本 **0.24 ms** / アイドル 3 本 **3.04 ms**
  / アイドル 13 本（上限超過）**0.03 ms + 503**。旧実装のアイドル 3 本 2,059 ms から
  大幅に改善。検証後はプロセスを停止しポート 18793 の解放を確認。
- 全検証: `cargo test --workspace` 162 passed / web 15 passed / clippy 警告 0 /
  fmt OK / wasm check OK。

## 2026-11-17（表示サニタイズ: ゼロ幅・不可視書式文字を追加除去）

- レビュー s9 のうち、過剰除去を避けられる「不可視書式文字」を対策した。
  `crates/tm-core/src/sanitize.rs` の `is_invisible_format()` で
  ZWSP/ZWNJ/ZWJ、word joiner、BOM、variation selector、hangul filler 等を
  空白へ置換（結合文字は保持してアクセント付き名前を壊さない）。
- テスト追加: なりすまし文字列（`pay\u{200b}pal...`）の除去、通常ラテン文字・CJK・
  結合文字の保持、プロパティテスト（500 ケース）への不可視文字注入。
  tm-core 59 → **61 tests**、workspace 合計 164 passed / clippy 0 warning / fmt OK / wasm check OK。

## 2026-11-17（エージェント並列化の敵対的レビュー対応）

- サブエージェントによる読み取り専用レビューで 2 件の構造的指摘を確認・修正:
  1. **[高] パニック時の接続カウンタリーク**: `ActiveConnection`（Drop ガード）を導入し、
     通常終了・パニックアンワインドの両方で `active` を確実に減算。8 回のパニックで
     全接続が 503 になる fail-closed 状態を解消。回帰テスト 1 件追加。
  2. **[中] 503 拒否経路の write timeout 未設定**: 拒否経路は `handle_connection` を
     通らないため、`WRITE_TIMEOUT` を明示設定。受信ウィンドウ 0 のピアが
     accept ループを再び阻塞する問題を防止。
  3. **[中] Mutex poison 回収の無ログ化**: 回収時に `eprintln!` で記録し、
     無ログの fail-open を解消。
- その他の指摘（CORS ヘッダ欠落、8 スロット上限の設計制約、テスト経路の偏り）は
  loopback・同一ユーザー脅威モデルの既知制約として記録し、今回は変更しない。
- 検証: tm-agent 41 → **42 tests**、workspace 165 passed / clippy 0 warning / fmt OK / wasm check OK。
  CI run 43897299713 は 8/8 success（修正前のコミット）。

## 2026-11-17（Host 検証 403 への CORS 付与）

- レビューで既知制約として記録されていた「エラー応答の CORS 欠落」のうち、
  Host 検証 403 を改善。`agent/src/main.rs` の `handle_connection` で Origin 判定を
  早期 return より前に移動し、許可された loopback Origin に限り 403 にも
  `Access-Control-Allow-Origin` / `Vary: Origin` を付与する。不正 Origin は従来どおり反射しない。
- 回帰テスト追加: 許可 Origin + 不正 Host の 403 に CORS が付くこと、
  不正 Origin + 不正 Host の 403 には付かないこと。
- 未対応: 431 / 400（リクエストヘッダが parse できないため Origin 不明）と
  503（accept 直後でリクエストを読んでいない）は CORS なしのまま。UI 側は
  fetch エラーとして検知できるため、セキュリティ境界を緩める理由はない。
- 検証: tm-agent 42 → **43 tests**、workspace 166 passed / clippy 0 warning / fmt OK。

## 2026-11-17（未使用依存を削除・ADR-009 の徹底）

- `cargo machete` で検出された未使用依存を 3 クレートから削除:
  - `web/`: `console_error_panic_hook` / `js-sys` / `wasm-bindgen-futures`
  - `poc-wasm/`: 同上
  - `app/`: `serde`（`serde_json` は利用中のため残す）
  - 各 `Cargo.lock` も再生成。削除後 `cargo machete` は 0 件。
- 検証: workspace 166 passed / clippy 0 warning / fmt OK / wasm check（tm-core,
  tm-viewmodel, web, poc-wasm すべて）成功。`trunk build --release` 成功。
  wasm サイズは不変（7,641,445 B / gzip 3,044,050 B）。
  CDP 設定検証・デモ検証とも `passed: true / failures: []`。証跡は
  `web/verification/*.json` と `notes/assets/web-shell-*.png` に再出力。
  検証用 HTTP サーバ / Chrome は停止し、ポート 8091 / 9223 の解放を確認。

## 2026-11-17（設定の一時ファイル名に PID を付与）

- `app/src/settings.rs` の `save_to` は一時ファイル名を固定
  （`settings.json.tmp`）にしていたため、2 プロセスが同時保存すると
  同じ一時ファイルを上書きし合う可能性があった。`settings.json.tmp.<pid>` に
  変更して競合を回避（原子的 rename と 0600 は従来どおり）。
- テスト追加: 保存後に一時ファイルが残らず、最終ファイルが存在することを確認。
  app 11 → **12 tests**、workspace 167 passed / clippy 0 warning / fmt OK。

## 2026-11-17（表示フォーマットの非有限値を防御）

- `crates/tm-core/src/format.rs` の `format_percent` / `format_cpu_percent` /
  `format_load_average` が `NaN` / `inf` をそのまま表示していたため、
  非有限値を `0` に置換するよう修正（表示レイヤの安全性）。
  通常のカーネル由来データは有限だが、壊れた probe が `NaN%` 等を描画しないようにする。
- テスト追加: `non_finite_numbers_never_reach_the_display`。
  tm-core 61 → **62 tests**、workspace 168 passed / clippy 0 warning / fmt OK。

## 2026-11-17（ネットワークレートの微小 elapsed を防御）

- `crates/tm-core/src/metrics.rs` の `NetworkCounters::rate_since` は
  `f64::EPSILON`（約 2.2e-16 秒）より小さい値のみ弾いていたため、
  例えば 1 ナノ秒の不正なタイムスタンプと大きなバイト差で `u64` レートが
  saturate し、履歴グラフに虚偽のスパイクが立ち得た。
  下限を 1 ミリ秒（`MIN_ELAPSED`）に引き上げ、それ未満は 0 を返すよう修正。
  サンプラーは 1 秒間隔なので正規サンプルに影響なし。
- テスト追加: `network_rate_ignores_sub_millisecond_gaps`。
  tm-core 62 → **63 tests**、workspace 169 passed / clippy 0 warning / fmt OK。

## 2026-11-17（履歴グラフへの非有限値混入を防御）

- `crates/tm-core/src/metrics.rs` の `MetricHistory::push` は `f32::clamp` のみで
  0..=100 に制限していたが、Rust の `clamp` は NaN をそのまま返すため、
  壊れた probe が NaN / inf を履歴へ送るとチャート描画に混入し得た。
  非有限値を 0 に置換するよう修正（前節の format 防御と同じ方針）。
- テスト追加: `history_rejects_non_finite_percentages`。
  tm-core 63 → **64 tests**、workspace 170 passed / clippy 0 warning / fmt OK。

## 2026-11-17（ツリー平坦化のサイクル耐性を修正）

- `crates/tm-core/src/query.rs` の `flatten_process_tree` は、親子関係に循環が
  あるとサイクル参加プロセスが root にならず表示から消えていた
  （再現テストで `rows = []` を確認）。visited ガード付きの walk に変更し、
  root 到達後も未訪問のノードを呼び出し順に追加するよう修正。
  折りたたみ時は子を visited に登録しつつ emit のみ抑制するため、
  隠れた子が再表示される回帰はない（既存テストで確認）。
- テスト追加: `flatten_process_tree_survives_a_parent_cycle`。
  tm-core 64 → **65 tests**、workspace 171 passed / clippy 0 warning / fmt OK。

## 2026-11-17（最新コードでの CDP 再検証と wasm サイズ更新）

- ツリーサイクル修正ほか直近の変更を反映するため `trunk build --release` を
  再実行し、CDP 検証を通しで再実施:
  - 設定永続化: `{"passed": true, "failures": []}`
  - デモモード: `{"failures": [], "passed": true}`
  - IME: 32/32 OK
- wasm 7,641,970 B（gzip 3,045,139 B、旧 7,641,445 B から微増）。
  README / DESIGN / web/README / current-state / web-shell の現状値を更新（旧値は履歴として保持）。
  検証用 HTTP サーバ / Chrome は停止し、ポート 8091 / 9223 の解放を確認。

## 2026-11-17（ツリーサイクル回帰テストを拡充）

- サイクル修正の検証を補強するため 2 テスト追加:
  - 3 ノード cycle（`1→2→3→1`）で全メンバーが表示され、深さが 0/1/2 に収束（無限ループなし）。
  - 通常ルート配下に別の 2 ノード cycle がある場合も、全プロセスが落ちずに表示される。
- tm-core 65 → **67 tests**、workspace 173 passed / clippy 0 warning / fmt OK。

## 2026-11-17（折りたたみ＋親子サイクルの併存を修正）

- サイクル修正のフォールバックパスで collapsed フラグが効いてしまい、
  折りたたみ済みノードがサイクルに含まれると相手側が表示から消えていた。
  `walk` に `ignore_collapsed` を追加し、フォールバックパスでは collapse を
  無視して全 cycle メンバーを表示するよう修正（深さは有限のまま）。
- テスト追加: `flatten_process_tree_cycle_honours_collapsed_nodes`。
  tm-core 67 → **68 tests**、workspace 174 passed / clippy 0 warning / fmt OK。

## 2026-11-17（ツリー平坦化の大規模チェーン回帰テスト）

- サイクル修正で追加したフォールバックパスが計算量に悪影響を与えていないことを
  確認するため、1,000 プロセスの親子チェーンを平坦化するテストを追加。
  全プロセスが 1 回だけ現れ、深さが 0..999 で収束することを検証（0.01 秒で完了）。
- tm-core 68 → **69 tests**、workspace 175 passed / clippy 0 warning / fmt OK。

## 2026-11-17（独立監査で発見した実バグを修正）

サブエージェントによる独立調査（4 エージェント並列）の指摘を裏取りし、以下を修正:

- **tree_view の永続化漏れ（app）**: `new()` が `tree_view` を復元しておらず、
  `ToggleTreeView` / `SetTreeView` も保存対象外だった（web 版は正常）。
  復元リストと persist 判定へ追加。
- **トークンの前後空白（agent）**: `--token` / `TM_AGENT_TOKEN` は trim せず保持
  していたため、`request_token` が trim する提示側と不一致だった。3 ソースを trim 統一。
- **Host 空ポート / IPv6 不正サフィックス（agent）**: `port.chars().all(...)` は
  空文字に `true` を返すため `localhost:` 等が許容され、`[::1]:garbage` も通過していた。
  `valid_port_suffix` で明示検証し拒否テスト追加。
- **更新間隔の復元が実効化されていない（web）**: サンプラーが既定 1 秒固定で
  `settings.interval_ms` を無視していた。`dashboard.interval()` 由来に変更し、
  proof へ `interval=` を追加。CDP で 5000 ms 復元を確認。
- **設定一時ファイルの権限（app）**: `fs::write` 後の chmod では umask 依存の
  0644 一瞬が存在した。Unix では `OpenOptions::mode(0o600)` で生成。
- **CI ゲート**: `clippy_args` に `undocumented_unsafe_blocks`、`extra_gates` に
  `cargo machete`（root / web）を追加（共通ワークフローの input を確認済み）。
- **CDP スクリプト**: 必須キーの包含判定、 json.loads 例外処理、interval 検証を追加。
- **ドキュメント**: trunk serve URL 8080→8091、件数・wasm 日付・branch protection・
  CI ジョブ列挙・audit/vet 記述を現状へ修正。

検証: workspace 176 passed（agent 44）/ clippy 0 warning / fmt OK / YAML パース / CDP 設定検証 pass。

## 2026-11-17（独立監査の残存指摘を追加修正）

- **非 UTF-8 argv（app）**: `std::env::args()` → `args_os()` に変更し、
  不正バイト引数でも `--version` / `--help` が panic しないことを実機確認
  （`$'\xff\xfe' --version` で exit 0）。
- **selection_hint（viewmodel）**: `recompute_visible()` が hint を再同期する前に
  旧値を退避し、選択プロセス消滅時の行位置フォールバックに使用するよう修正。
- **応答サイズ上限（web）**: `body.take(8 MiB).read_to_end` で wasm ヒープを保護。
  定数テスト追加（web 11 → 12 tests）。
- **境界テスト（agent）**: 64 個ヘッダの受理、CRLF 混入 Origin の拒否を追加。
- **CDP スクリプト**: parse_proof は元々 `=` を値側で保持していたことを確認
  （監査指摘は誤検知、誤修正を事実確認で回避）。column_centers の負値クランプ、
  IME reader 例外伝播と子プロセスの確実な後始末を追加。
- **doc**: `AgentConfig.token` が query 渡しと書かれていた箇所を Bearer ヘッダへ修正。
- 検証: workspace 177 passed（agent 45）/ web 12 / clippy 0 warning / fmt OK / py_compile OK。

## 2026-11-17（最新コードで CDP 再検証）

- args_os / サイズ上限ほかの変更を反映したビルドで CDP 設定・デモ・IME（32/32）を再検証し、
  すべて成功。wasm 7,642,794 B（gzip 3,044,134 B）へ微増し現状値を更新。
  検証用プロセスを停止し、全ポート（8091/9223/8097/9229）の解放を確認。

## 2026-11-17（ライブモード End-to-End 再検証）

- 直近の agent 変更（トークン trim 統一・Host 空ポート拒否・並列化）後に、
  tm-agent（127.0.0.1:18787、`--redact`、`TM_AGENT_TOKEN=test`）＋
  web/dist（8091）＋ ヘッドレス Chrome（CDP 9223）でライブ接続を再検証:
  - ライブモード: `{"failures": [], "passed": true}`
    （mode=live / rows=500 / 14 GET すべて Authorization 付き /
     CORS プリフライト 1 回 / トークン URL 非含有）。
  - エラー時モード（`--expect-agent-error`、`--label live-error`）: `passed: true`
    （mode=live のまま / live_error=1 / rows=0、デモへフォールバックしない）。
- 最初にエラー時モードをフラグ無しで実行し 4 件 FAIL が出たが、
  スクリプトの期待モード指定漏れが原因と確認し正しいフラグで再実行して解消。
- 検証用プロセスを停止し、全ポート（8091 / 9223 / 18787）の解放を確認。

## 2026-11-17（HTTP ヘッダ 8 KiB 境界の受理テストを追加）

- `read_request` はヘッダ終端位置が `MAX_HEADER_BYTES`（8 KiB）を超えた場合のみ
  `TooLarge` を返す仕様。431 側（超過拒否）は既存テストでカバーされていたが、
  境界値ちょうどの受理が未検証だった。
- ループバックソケット経由で `read_request` を呼ぶヘルパーを追加し、
  8 KiB ちょうどのヘッダが受理されること／8 KiB 超が `TooLarge` になることを検証。
  途中で不正に切り詰めたテストデータが `Malformed` になったが、実装ではなく
  テストデータの問題と修正。
- tm-agent 45 → **47 tests**、workspace 179 passed / clippy 0 warning / fmt OK。

## 2026-11-17（HTTP 読み取りチャンク境界のテストを追加）

- `read_request` は 1024 バイトのチャンクで読み、`find_header_end` は蓄積済み
  バッファ全体を走査する。終端 CRLF がチャンク境界（1023-1024）をまたぐケースを
  ループバックソケットで明示検証（CRLF の配置をアサーションで固定）。
- tm-agent 47 → **48 tests**、workspace 180 passed / clippy 0 warning / fmt OK。

## 2026-11-17（空接続の read_request 挙動を直接検証）

- `read_request` は peer が何も送らずに閉じた場合 `Ok(None)` を返す仕様。
  main.rs の `empty_connection_is_ignored` は接続が処理されることのみを確認して
  いたため、http.rs の `read_head` ヘルパーで戻り値を直接検証するテストを追加:
  - 空接続 → `Ok(None)`
  - 途中まで送って閉じた場合 → `Err(Malformed)`
- tm-agent 48 → **50 tests**、workspace 182 passed / clippy 0 warning / fmt OK。

## 2026-11-17（表示フォーマットの極端値テストを追加）

- `format_bytes(u64::MAX)` / `format_duration(u64::MAX)` /
  `format_process_age(0, u64::MAX)` / `format_count(u64::MAX)` が
  パニック・オーバーフローせず有限の文字列を返すことを検証。
  壊れた probe が極端値を渡しても描画ループが落ちないことを担保。
- tm-core 69 → **70 tests**、workspace 183 passed / clippy 0 warning / fmt OK。

## 2026-11-17（wire schema 拒否のテストを追加）

- 監査 a9 で「web 側が未知スキーマを拒否」と対応済みと記録されていたが、
  その拒否経路にテストが無かった。`check_schema_version` を純粋関数に切り出し、
  現行バージョンは受理／未知バージョン（+1）はエラーになることを検証。
- web 12 → **13 tests**、workspace 183 passed（ルート側は不変）/ clippy 0 warning / fmt OK。

## 2026-11-17（web 設定の破損 JSON フォールバックを検証）

- `settings::load()` は破損 JSON を既定値へ縮退するが、ブラウザコンテキストが必要で
  単体テストが無かった。JSON パース部分を純粋関数 `parse_saved` に切り出し、
  破損 JSON が既定値（interval 1000 / version 現行 / details_visible true）へ
  縮退することを検証。
- web 13 → **14 tests**、ルート workspace 183 passed（不変）/ clippy 0 warning / fmt OK。

## 2026-11-17（web 設定の前方互換テストを追加）

- 将来のシェルが未知フィールドを書き込んだ設定を、現行ビルドが
  `#[serde(default)]` で読み込めることを検証（既知値保持・version 現行化）。
  native 版の `unknown_fields_are_ignored_for_forward_compatibility` と同等の
  カバレッジを web 側にも確保。
- web 14 → **15 tests**、ルート workspace 183 passed（不変）/ clippy 0 warning / fmt OK。

## 2026-11-17（ソート列ラベル/初期方向/矢印を全バリアント固定）

- CDP の证明フック（`sort=名前` / `dir=asc`）が依存するヘッダー表示値を
  全 8 列で固定（label / default_direction / SortSpec::new）し、
  SortDirection の矢印・反転も検証。
- tm-core 70 → **71 tests**、workspace 184 passed / clippy 0 warning / fmt OK。

## 2026-11-17（status/user フィルタの個別テストを追加）

- 既存の閾値テストは min_cpu/min_memory 同時指定のみで、status / user の
  個別マッチ・非マッチ（user は大小文字無視）が未検証だった。
  `Filter::matches` を直接検証するテストを追加。
- tm-core 71 → **72 tests**、workspace 185 passed / clippy 0 warning / fmt OK。

## 2026-11-17（CPU/メモリ閾値の個別適用を検証）

- 既存の閾値テストは両方同時指定のみだったため、min_cpu_percent と
  min_memory_bytes をそれぞれ単独で指定した場合に正しくフィルタされることを検証。
- tm-core 72 → **73 tests**、workspace 186 passed / clippy 0 warning / fmt OK。

## 2026-11-17（is_active の全フィールド検証を追加）

- 既存の active 判定テストは text のみだった。UI の「絞り込み中」表示に効く
  user / status / min_cpu / min_memory の各フィールドが active と判定されることを検証
  （閾値 0 も明示指定として active）。
- tm-core 73 → **74 tests**、workspace 187 passed / clippy 0 warning / fmt OK。

## 2026-11-17（ProcessKey の Display 形式を固定）

- PID 再利用検出のログで使われる `pid@start_time` 形式と `pid()` アクセサを検証。
  既存の順序テスト（PID → start_time）と合わせ、同一性キーの表現を固定。
- tm-core 74 → **75 tests**、workspace 188 passed / clippy 0 warning / fmt OK。

## 2026-11-17（Timestamp の極端値 saturating を検証）

- `Timestamp::from_secs(u64::MAX)` が秒→ミリ秒変換で wrap せず saturated になることを検証
  （壊れた snapshot の sampled_at が未来の plausible な値に化けないことを担保）。
- tm-core 75 → **76 tests**、workspace 189 passed / clippy 0 warning / fmt OK。

## 2026-11-17（Signal ラベルの全バリアント固定）

- 確認ダイアログで表示される `Signal::label` / `short_label` を全 4 バリアントで固定。
  tm-platform の `map_signal` も全バリアント網羅を確認（将来追加時の panic リスクなし）。
- tm-core 76 → **77 tests**、workspace 190 passed / clippy 0 warning / fmt OK。

## 2026-11-17（大小文字無視ヘルパーの Unicode 挙動を固定）

- `İ` → `i` + combining dot の展開で eq/cmp/contains が一致すること、
  空 needle が常に一致、ASCII 折り目、および `ß` が `ss` に展開されない
  （Unicode lowercase の制約）ことをテストで明示。
- tm-core 77 → **78 tests**、workspace 191 passed / clippy 0 warning / fmt OK。

## 2026-11-17（ローカルゲート scripts/check.sh の失敗を修正し CI へ反映）

- `bash scripts/check.sh` を実行したところ exit 101 で失敗。原因は web 側
  `assert!(MAX_SNAPSHOT_BYTES > 1_000_000);` が clippy::assertions_on_constants
  （-D warnings）に引っかかったこと。ルートの `cargo clippy --workspace` は
  web 別ワークスペースを含まないため見落としていた。
- `const { assert!(...) }` に修正し check.sh は全ゲート exit 0 を確認。
- CI の `extra_gates` に web clippy / web test を追加（これまで web は
  cargo-vet / cargo-machete のみで clippy が走っていなかった）。YAML パース確認済み。

## 2026-11-17（network rate の backward counters を検証）

- インターフェースリセット等でカウンタが戻った場合、saturating_sub が 0 を返し
  巨大な誤レートにならないことを検証（cpu_percent 側には同種テスト済みで対称）。
- tm-core 78 → **79 tests**、workspace 192 passed / clippy 0 warning / fmt OK。

## 2026-11-17（MetricHistory の極端 byte rate を検証）

- network_rx/tx_rate は u64 でチャートへそのまま渡るため、u64::MAX が
  固定長 ring を通って latest/earliest で正しくラウンドトリップすることを検証。
  当初フィールド名を rx_bytes と誤記してコンパイルエラーになったが、
  実定義（network_rx_rate / network_tx_rate）に修正。
- tm-core 79 → **80 tests**、workspace 193 passed / clippy 0 warning / fmt OK。

## 2026-11-17（truncate の 0/1 文字境界を検証）

- `max_chars=0` が 「…」、`max_chars=1` が 1 文字 + 「…」になることを
  日本語/ASCII 両方で固定（ellipsis は max_chars に含まれない）。
- tm-core 80 → **81 tests**、workspace 194 passed / clippy 0 warning / fmt OK。

## 2026-11-17（双方向制御文字の全バリアントを検証）

- `is_bidi_control` の全範囲（LRE/RLE/PDF/LRO/RLO/LRI/RLI/FSI/PDI の 9 文字）が
  すべて空白に中性化されることを網羅検証（プロセス名の並び替えリスクを担保）。
- tm-core 81 → **82 tests**、workspace 195 passed / clippy 0 warning / fmt OK。

## 2026-11-17（その他の不可視書式文字を検証）

- 既存テストがカバーしていない CGJ / Arabic Letter Mark / Hangul Filler /
  Khmer Vowel / Mongolian VS / Halfwidth Hangul Filler が
  `is_invisible_format` で検出されることを検証（前回の bidi 9 文字と合わせ
  サニタイズ対象を網羅）。
- tm-core 82 → **83 tests**、workspace 196 passed / clippy 0 warning / fmt OK。

## 2026-11-17（MetricHistory のゼロ容量境界を検証）

- `MetricHistory::new(0)` が各 series の RingBuffer を capacity 1 にクランプし、
  push がパニックせず最新値を保持することを検証。
- tm-core 83 → **84 tests**、workspace 197 passed / clippy 0 warning / fmt OK。

## 2026-11-17（壊れた intra-doc link を修正し rustdoc ゲートを追加）

- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` を実行したところ
  exit 101。`filter_input.rs` の [`gpui::EntityInputHandler`]（tm-viewmodel は gpui 非依存）
  と [`tm_viewmodel::Dashboard`]（自クレート内なので `crate::Dashboard`）の
  壊れた intra-doc link 2 件を修正。
- 再発防止のため `scripts/check.sh` と CI `extra_gates` に同コマンドを追加。
  shellcheck / shfmt / YAML パース / check.sh 全体（exit 0）で検証済み。

## 2026-11-17（web ワークスペースの rustdoc ゲートを追加）

- web 側の `RUSTDOCFLAGS=-D warnings cargo doc --manifest-path web/Cargo.toml --no-deps`
  を実行し exit 0（壊れたリンクなし）を確認。
- 将来 web 側で壊れた intra-doc link が入っても検出できるよう、
  `scripts/check.sh` と CI `extra_gates` に同コマンドを追加。全体 exit 0 を確認。

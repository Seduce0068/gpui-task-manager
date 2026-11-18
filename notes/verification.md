---
type: Reference
title: GPUI Task Manager 検証ログ（2026-11-14）
description: ネイティブ試作・エージェント・wasmスパイクの Runtime Proof と既知の制限
tags: [verification, runtime-proof, gpui, linux, wayland, wasm]
status: verified
generated: 2026-11-14T21:10:00+09:00
verified: 2026-11-18T10:58:00+09:00
---

## 1. 検証環境

| 項目 | 値 |
|---|---|
| OS | Kubuntu 2026.04 / KDE Plasma 6 / Wayland（`XDG_SESSION_TYPE=wayland`, `WAYLAND_DISPLAY=wayland-0`） |
| CPU / RAM / GPU | Ryzen 5 3600（6C/12T）/ 64 GB / GeForce GTX 1660 SUPER |
| Rust | rustc 1.99.1 / cargo 1.99.1 |
| 検証日時 | 2026-11-14 20:00–21:10 JST |
| 作業ディレクトリ | `.` |

## 2. ビルド・静的検証

| コマンド | 結果 |
|---|---|
| `cargo fmt --all --check` | 成功（差分なし） |
| `cargo build -p gpui-task-manager` | 成功（警告 0） |
| `cargo test --workspace` | 件数は README.md「検証状況」を参照（2026-11-18 時点のスナップショット） |
| web ユニットテスト | 件数は README.md「検証状況」を参照（2026-11-18 時点のスナップショット） |
| `cargo clippy --workspace --all-targets -- -D warnings -D clippy::undocumented_unsafe_blocks` | 成功（警告 0、root / web とも） |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | 成功（警告 0、root / web とも） |
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
cd .
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
| M8 CI 参照先 | `Seduce0068/workflows@v1` に修正（旧アカウント名はリダイレクト。Actions の `uses:` は現行名を使う） |
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

## 2026-11-15（CI 全ジョブの起動失敗: アカウント設定に起因）

- 事実: 2026-11-15 07:11Z（16:11 JST）以降の push で CI 全ジョブが 3〜5 秒で失敗。アノテーション: アカウント設定に起因するジョブ起動エラー。最終成功は `refactor: share settings schema and persist web preferences`（2026-11-15T03:55:29Z、12:55 JST）。
- 影響: gates（fmt/check/test/clippy/audit/extra_gates の cargo vet 含む）/ wasm / native-windows / native-macos / deny / release-linux / web-dist / sbom の全ジョブが未起動。コードや workflow 定義の問題ではない（0 step で失敗）。
- 対応: アカウント設定の確認・修正が必要（アカウント設定のためエージェントでは対応不可）。修正後に `gh workflow run` または再 push で再検証する。
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
| 別の AGPL プロジェクト | **public** | `docs: update test count from 36 to 132`（2026-11-15T16:02:50Z） | 5 分 33 秒 | ✅ **success** |
| `gpui-task-manager` | **private** | `docs: record wayland ...`（2026-11-15T15:57:27Z） | 4 秒 | ❌ failure（0 step） |

失敗ジョブのアノテーション（GitHub API で取得）:

```text
The job was not started due to an account configuration issue.
```

### 結論

- **原因はリポジトリの公開設定とアカウント設定**: private リポジトリでは
  ジョブが開始されない場合がある。
- public リポジトリ（別の AGPL プロジェクト）は同じアカウントでも成功する。
- したがって本件はコード / workflow 定義の問題ではなく、**リポジトリ公開設定とアカウント設定の問題**。

### 対応の選択肢（いずれもアカウント設定の変更が必要）

1. **本リポジトリを public 化** — CI が無料枠で動き即解決。ただし「まだ公開しない」方針と衝突するため要判断。
2. **アカウント設定を確認・修正** — private のまま CI を継続。
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
  （`/home/<user>/...`）をリポジトリ相対パスに置き換えた（M6 の方針:
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

## 2026-11-17（選択行位置フォールバックの回帰テストを追加）

- 初期監査で「修正は適用されたが回帰テストが無い」と記録された `previous_hint`
  （選択プロセス消滅時に同じ行位置へフォールバック）を明示検証するテストを追加。
  中間行（index 2）を選択してそのプロセスを消し、同じ位置の行が選択されることを確認。
- tm-viewmodel 37 → **38 tests**、workspace 198 passed / clippy 0 warning / fmt OK。

## 2026-11-17（HTTP レスポンスヘッダの厳格テストを追加）

- ループバックソケットで write_response の生バイトを検証:
  200 は Content-Type / Content-Length / Cache-Control / nosniff / Connection、
  204 は Content-Type / Content-Length を省略し終端 CRLF CRLF のみ。
  当初 `X-Content-Type-Options` が "Content-Type" に部分一致して誤失敗したが、
  ヘッダ行先頭（`\r\nContent-Type:`）で照合して解消。
- tm-agent 50 → **51 tests**、workspace 199 passed / clippy 0 warning / fmt OK。

## 2026-11-17（生成トークンの小文字 hex を固定）

- `generate_token` が小文字 hex（0-9a-f）のみを生成することを明示検証。
  token は constant_time_eq でバイト比較されるため、大文字化は認証失敗に直結する。
- tm-agent 51 → **52 tests**、workspace 200 passed / clippy 0 warning / fmt OK。

## 2026-11-17（HTTP/1.0 要求行の受け付けを検証）

- parse_request が HTTP/1.0 と HTTP/1.1 の両方を受け付けることを明示検証
  （既存テストは HTTP/2.0 拒否のみで 1.0 受理は未カバー）。レスポンスは常に HTTP/1.1。
- tm-agent 52 → **53 tests**、workspace 201 passed / clippy 0 warning / fmt OK。

## 2026-11-17（クロスレビュー R1S-DATA-07/09/10 を修正）

- R1S-DATA-07: `query.rs` の CPU ソートで NaN が先頭に来ていた。履歴側が
  non-finite を 0 にクランプするのと揃え、NaN を 0 としてソートする
  `cmp_percent` を導入（回帰テスト追加）。
- R1S-DATA-09: `format_cpu_percent` が負値を `-5.0%` と表示し履歴 clamp と
  不一致だったため、下限 0 にフロア（multicore の 100 超は従来どおり許容）。
- R1S-DATA-10: `demo.rs` の `DEMO_BASE_UNIX_SECS + tick` と `uptime_secs` が
  非 saturating で、`u64::MAX` tick で debug overflow panic していた。
  所見は 418 行目のみだったが 430 行目も同種のバグを発見し両方を修正
  （極値 tick の回帰テスト追加）。
- tm-core 84 → **86 tests**、workspace 203 passed / clippy 0 warning / fmt OK。

## 2026-11-17（CI 失敗 3 件を修正）

- クロスレビュー反映後の CI で 3 ジョブが失敗:
  1. `gates`: 私が追加した doc コメントの [`MetricHistory`] リンクが未解決
     （format.rs は metrics を import していない）→ プレーンテキストへ変更。
  2. `cargo-deny`: 新 advisory RUSTSEC-2026-0543（chrono unmaintained）が
     allowlist に無かった。pin 済み GPUI スタックの推移依存であることを理由に
     `deny.toml` へ追加（既存 8 件と同方針）。
  3. `web dist artifact`: `web/Cargo.toml` の `strip = true` が wasm-bindgen の
     target_features セクションを削除し、wasm-opt が memory.copy/fill を
     拒否（bulk-memory）。`strip = false` に変更（wasm-opt -Oz が strip を担当）。
  ローカルで `bash scripts/check.sh` 全ゲート exit 0、`trunk build --release` exit 0、
  `RUSTDOCFLAGS=-D warnings cargo doc`（root/web）両方成功を確認。

## 2026-11-17（弱い乱数フォールバックを廃止：R1S-SEC-02/R4-O01）

- `agent/src/auth.rs` の `/dev/urandom` 失敗時 time/PID + SplitMix64 フォールバックを削除。
  `generate_token()` を `Result<String, String>` に変更し、OS CSPRNG が読めない場合は
  予測可能なトークンを出さずエラーで起動を失敗させる（`parse()` は既に Result のため
  自然に伝播）。回帰テスト 2 件は `.expect("urandom")` に更新。
- 実機確認: `--token test --once --demo` でスナップショット取得成功、トークン省略時は
  `/dev/urandom` から生成され stderr に 1 回表示（従来どおり）。
- 見送り: R1S-SEC-03（stderr 平文出力）は README:88 で「stderr に 1 回だけ表示」と
  明記された意図的な設計であり、廃止にはトークン自動保存等の仕様判断が必要。
- workspace 203 passed / clippy 0 warning / fmt OK。

## 2026-11-17（トークンファイル権限と argv 警告：R2X-A2/A10）

- `read_token_file` が Unix で mode を検証し、group/other に読み取り権がある
  （0644 等）場合は 0600 を要求して起動を拒否。Windows は mode が無いためスキップ。
- `--token` の argv 露出警告を `--once` の早期 return より前に移動し、
  one-shot 実行でも警告が出るようにした（重複警告は削除）。
- 実機確認: `--once --demo --token test` で警告表示、0644 ファイルで拒否メッセージを確認。
- tm-agent 53 → **54 tests**、workspace 204 passed / clippy 0 warning / fmt OK / rustdoc OK。
- 見送り: R2X-A1（`--token` が snapshot の process.command にエコー）は redact 範囲の
  設計判断が必要なため今回は対象外。

## 2026-11-17（--listen と Host 許可リストの不一致を解消：R2X-A9）

- `is_loopback()` は 127.0.0.2 等も受理するため、Agent は起動するが
  `is_allowed_host` がその Host を拒否してブラウザから到達できない不一致があった。
  `--listen` を 127.0.0.1 / [::1] のみに限定（Host 側は緩めない）。
- 実機確認: 127.0.0.2:18890 はエラーで拒否、127.0.0.1:18892 は正常に待ち受け。
- tm-agent 54 tests（件数不変、既存テストへ 127.0.0.2 を追加）、
  workspace 204 passed / clippy 0 warning / fmt OK。

## 2026-11-17（query_param ポイズニング対策と設定 tmp nonce：R2-D05/R2-D03）

- `query_param` がデコード後の制御文字（%0d%0a / %00 等）を含む値を `None` として
  扱うようにした（`parse_process_limit` は None をデフォルト扱い → 攻撃値を無効化）。
  重複クエリは先勝ちを維持し、テストで固定（第 2 の注入値で上書き不可）。
- 設定一時ファイル名に PID + 単調 nonce + 時刻を付与し、PID 再利用時に古い tmp と
  衝突しないようにした。テストは「残 tmp なし」をディレクトリ走査で検証する形へ更新。
- tm-agent 54 → **55 tests**、workspace 205 passed / clippy 0 warning / fmt OK / rustdoc OK。

## 2026-11-17（ヘッダ読み取りの絶対 timeout：R2X-A3）

- 従来は per-read の READ_TIMEOUT(1s) のみで、1 バイトずつ送り続ける
  slowloris 接続が全スロットを長時間占有できた。`HEAD_TIMEOUT(5s)` を導入し、
  `read_request_with_timeout`（テスト用に deadline 注入可能）でヘッダ全体の
  絶対期限を守る。期限超過は 408 Request Timeout を返す。
- `read()` が WouldBlock/TimedOut を返した場合も Timeout として扱うよう修正
  （当初 Malformed になっていた）。テストソケットでも確実に動くよう
  `read_request_with_timeout` 自身が残予算で read timeout を設定。
- 回帰テスト: 部分ヘッダを送って放置し 100ms で Timeout になることを検証。
- tm-agent 55 → **56 tests**、workspace 206 passed / clippy 0 warning / rustdoc OK。

## 2026-11-17（ConfirmKill の paused ガードと pending 上書き防止：R2-F01/F02）

- `confirm_kill` が `paused` 中は信号を発行せず pending を破棄しエラー表示
  （一時停止中は表示が古く PID 再利用判定が信頼できないため）。
- `request_signal` が未解決の pending を無条件上書きせず、先に確定/キャンセルを
  促す（確認ダイアログの対象が勝手に変わるのを防止）。
- 回帰テスト: 2 回目の RequestSignal で対象が変わらないこと、paused 中の
  ConfirmKill が Effect を出さないことを検証。
- tm-viewmodel 38 → **39 tests**、workspace 207 passed / clippy 0 warning / rustdoc OK。

## 2026-11-17（accept ループの堅牢化：R2X-A7/R2X-FR1）

- `thread::spawn` の Result を扱い、失敗時は接続カウンタを手動で減らして
  エージェント全体を panic で落とさないようにした（Drop ガードは worker へ move
  済みなので `state.active.fetch_sub` で直接戻す）。スレッドに名前を付与。
- accept 失敗時に 50ms バックオフを入れ、EMFILE 等の永続エラーで
  ビジーループしないようにした。
- `spawn_connection` を `Result<(), String>` に変更し、呼び出し側でエラーをログ出力。
- workspace 207 passed / clippy 0 warning / rustdoc OK（テスト件数は不変）。

## 2026-11-17（ワイヤ境界と OS 文字列のサニタイズ：R2X-D1/D2）

- tm-core に `sanitize_snapshot(&mut SystemSnapshot)` を追加。hostname / os_name /
  kernel_version / process(name,command,user) / IF 名 / mount_point / component.label
  をまとめて `sanitize_display` に通す（数値・時刻・キーは触らない）。
- web の agent レスポンス処理でデシリアライズ直後に適用し、悪意/破損エージェントの
  ANSI・制御文字が canvas に届かないようにした。
- tm-platform の os_name / kernel_version / os_label にも sanitize_display を適用。
- 回帰テスト: 全文字列フィールドに ANSI/bidi/改行を注入し除去されることを検証。
- tm-core 86 → **87 tests**、workspace 208 passed / clippy 0 warning / rustdoc OK。

## 2026-11-18（エージェントチーム並列調査 → 4 項目を修正）

4 観点（web / agent / viewmodel-app / 設定データ）の調査エージェントを並列起動し、
各推奨項目を親が Runtime Proof で確認してから実装:

- R2X-A5: `parse_request` が重複ヘッダ（大小文字無視）を `Malformed` 拒否。
  Host/Authorization/Origin の「先頭値」依存による解釈差分を排除。回帰テスト追加。
- R1S-DATA-08: バイト表記へ EiB を追加（`u64::MAX` = 16.0 EiB、
  1_024^6 = 1.0 EiB を pin）。
- R2X-P1: web の `details_visible` を実効化（main_area の条件描画＋
  `toggle-details-web` ボタン）。設定の復元・保存は既に配線済みで、既定値 true のため
  既定画面は不変。
- R3-S03: `app/src/main.rs` に pure な `parse_gui_flags` を追加し、
  `--demo=yes` / `--redact=1` を exit 2 で拒否（`args_os()` は維持）。

**子の提案で却下した点**: probe-vm は「app/src/main.rs に #[cfg(test)] を追加して
pure 関数をテスト可能」と提案したが、実際に追加すると GPUI のテストハーネスが
SIGSEGV (signal 11) でクラッシュ（recursion_limit 256/512 でも回避不可）。
app クレートはテスト対象外という既存設計を尊重し、テストモジュールは削除。
実機で `--demo=yes` / `--redact=1` の拒否と `--help` の従来動作を確認。

- tm-agent 56 → **57 tests**、workspace **209 passed** / `scripts/check.sh` 全ゲート成功 /
  clippy / rustdoc（root+web）/ fmt OK。
- 子エージェントはすべて読み取り専用で compile/test 未実行。出力は仮説として扱い、
  上記はすべて親の Runtime Proof で確認済み。

## 2026-11-18（エージェント並列調査 第2弾 → redact マスク追加）

3 観点（CDP CI 接続 / wasm timeout / redact スコープ）の調査エージェントを並列起動。
各出力を仮説として扱い、親がローカルコードで Runtime Proof してから判断:

- **採用**: probe-redact の提案どおり `agent/src/main.rs` の `redact_snapshot` に
  `disks[].mount_point` への `redact_text` 適用を追加（R2X-A4 の仕約内部分）。
  仕約（ホスト名・ユーザー名・ホームパス）の範囲内で、新ロジック・wire 変更なし。
  子の重要指摘どおり **IF 名はマスクしない**（`is_virtual_interface()` が名前で仮想 NIC を
  判定し `physical_totals()` のレート計算が壊れるため。仕約変更＋設計案件）。
- 既存テストへ `$HOME/work/project` → `/home/user/work/project` のアサーションを追加。
  clippy の `useless format!` 指摘を修正。
- **見送り（子の提案を判断で却下）**: probe-cdp の CDP 独立ジョブ追加（実装量大・
  flaky 要因多数、約 2〜4 分の CI 時間増）、probe-timeout の wasm fetch watchdog
  （future 破棄が network abort にならず orphan fetch が残る、設計判断が必要）。

- 実機: `tm-agent --once --demo --redact` で mount_point が伏せられた値になることを確認
  （デモは `/` と `/home` のみで home 配下例なし、ユニットテストでカバー）。
- workspace **209 passed**（件数不変、既存テスト拡張）/ clippy / rustdoc OK。

## 2026-11-18（エージェント並列調査 第3弾 → ?agent= 送信先制限）

3 観点（残所見再走査 / データ境界 / UX）の調査エージェントを並列起動。
probe-residual はさらに 3 体の子エージェントを入れ子で活用し、各出力を
Runtime Proof で取捨選択していた（親はその結果も仮説として現行コードで確認）:

- **採用**: R2-P01 — `web/src/agent.rs` に `is_loopback_http_endpoint()` を追加し、
  `fetch_snapshot()` の入り口で Bearer ヘッダ付与前に接続先を検証。
  `http://` ＋ host が `127.0.0.1` / `localhost` / `[::1]` ＋数値 port のみ許可。
  URL フラグ由来のトークンを任意ホストへ送る経路を遮断（agent 側 Host 検証との非対称も解消）。
  pure 関数のため web の native テストで決定論的に検証（許可 4 件／拒否 7 件）。
- web 15 → **16 tests**、workspace 209 passed / clippy / wasm check / rustdoc OK。

**見送り・却下（子の提案を判断で処理）**:

- probe-ux の R2-C01（Effect 明示）、probe-boundary の `format_load_average` 負値フロアは
  今回は見送り（次ラウンド候補として保持）。
- R2X-D5（ディスク used）は子の追加発見で pinned sysinfo 0.40.3 に `used_space()` が
  無いことが判明し「安全」から「要判断」へ格下げ（子の提案を補正して採用）。
- R4-A02（8MiB upfront 確保）は `body.take(8MiB)` の実装を確認し「対応済み相当」と判定。
- CDP CI 接続 / wasm watchdog / per-signal capability / 更新間隔 UI は設計判断または
  flaky 要因を伴うため再度見送り。

- 新たな不整合として `NOTICE.md:31` が「Apache License 2.0」のまま
  （fdf6f62 で AGPL 移行済み）であることを記録（次ラウンド候補）。

## 2026-11-18（エージェント並列調査 第4弾 → 3 論理変更）

3 観点（ライセンス文書 / サニタイズ / CI 設定）の調査エージェントを並列起動し、
各推奨項目を親が現行コードで Runtime Proof してから実装:

1. **ライセンス表記の整合**（14 ファイルの SPDX ヘッダ + NOTICE.md:31 +
   web/Cargo.toml:8 の実メタデータ）を AGPL-3.0-or-later へ統一。
   fdf6f62 は LICENSE 本文等のみを変更し、SPDX ヘッダ群と独立 workspace の
   `license` メタデータが旧 Apache のまま残っていた。旧 SPDX 残存 0 件を確認。
   THIRD_PARTY_NOTICES.md は旧本文アーカイブとして正当（probe が diff で検証）。
2. **format_load_average の負値フロア**（R1S-DATA-09 残り）: 非有限は 0、
   有限は `max(0.0)` に。probe が /tmp で現行 `"-1.00"` と `-0.0 → "-0.00"`
   を実証。`[-1.0,1.5,NaN]` → `"0.00 1.50 0.00"` を回帰テストで固定。
3. **CI 成果物ジョブの保護**: release-linux / web-dist / sbom の 3 ジョブへ
   fork PR 除外の `if` と `needs: gates` を付与（R4-O02 / R1-OPS-01）、
   cargo-machete を 0.10.1 に pin（R1S-OPS-02）。YAML パース成功。

**見送り**: Windows への tm-platform テスト追加（R1-V02）は初回 CI の実行時
アサーションがローカル検証不可のため次ラウンド以降に保留。macOS cancelled は
外部ランナー供給問題で設定側緩和策なし（probe も同意）。DESIGN の ADR-011 は
履歴として保持し、ADR-021 で AGPL 移行を記録するのは要判断として据え置き。

- workspace 209 passed / clippy / cargo-deny / rustdoc / check.sh 全ゲート成功。

## 2026-11-18（エージェント並列調査 第5弾 → 5 項目を修正）

3 観点（同族バグ / Effect / DEB）の調査エージェントを並列起動し、
各推奨項目を親が現行コードで Runtime Proof してから実装:

1. **R2-C02**: app/src/app.rs の 2 箇所で `let _ = this.update(...)` を
   `if ... .is_err() {}` に変更（entity 消滅後のコールバックを明示）。
   子の提案どおり `return` を付けたが、末尾の文なので clippy の
   `needless_return` に指摘され、コメント入り空ブロックに修正。
2. **R2-C01**: 設定復元 4 コマンドの Effect を明示的に検査（native と web を
   同等化）。契約テスト `settings_restore_commands_emit_no_effects` を
   tm-viewmodel へ追加（app クレートの SIGSEGV 制約を回避）。
3. **format_percent(-0.0) / cmp_percent 負値**: 直近の `format_load_average`
   修正の取り残し。`clamp` は `-0.0` を保持するため `.max(0.0)` を前置。
   ソートも表示と揃え、Ascending の回帰テストで `-5.0` / `-0.0` を固定。
4. **R1S-OPS-05**: build-deb.sh が `ldd` の "not found" を黙ってスキップ
   していた問題を修正。ldd 出力を捕捉して明示検査し、未解決ライブラリは
   ビルド失敗扱い。README に依存・残置方針を追記。
5. **R1-V02**: Windows CI の `cargo test` へ `-p tm-platform` を追加
   （前回保留項目。前回 CI が 8/8 success だったため実施）。

- 子の指摘で却下: 補助面 VS (U+E0100..E01EF) は IVS として正当利用され
  表示差を生み得るため別判断に保留。R3-L01 (postrm) は Debian Policy 上
  `postrm` 追加で `apt remove` 後に rc 状態が残る退行リスクがあるため
  見送り（README に現行方針を明記）。
- tm-core 87 → **88**、tm-viewmodel 39 → **40**、workspace **211 passed** /
  clippy / shellcheck / shfmt / YAML / rustdoc / check.sh 全ゲート成功。

## 2026-11-18（エージェント並列調査 第6弾 → 3 論理変更）

3 観点（同族負値 / ライセンス文書 / 設定バックアップ）の調査エージェントを並列起動し、
各推奨項目を親が現行コードで Runtime Proof してから実装:

1. **負値の正規化を統一**: tm-core に `pub fn normalise_percent()` を追加し
   （非有限・負値・-0.0 → 正の 0）、`cmp_percent` / `Filter::matches` /
   agent の `apply_process_limit` を同じ floor に揃えた。閾値側（min_cpu）は
   負の閾値を壊さないよう触らない。回帰テスト 2 件追加。
   agent の truncate テストは子の提案どおり PID タイブレークで失敗したため、
   `truncate(1)` で「30% が残る」ことを検証する形に修正（修正前は -5% が残る）。
2. **ライセンス文書の残り**: NOTICE.md へ IBM Plex の帰属（SIL OFL 1.1、
   RFN "Plex"、Copyright 2017-2018 IBM）を追加。deny.toml の allow リスト
   コメントを実態（permissive + AGPL + MPL）に修正。web/README.md の
   「本 crate は Apache-2.0」「AGPL/GPL コードは含みません」という誤記を修正。
3. **R2-D02 破損設定バックアップ**: load_from() の InvalidData 分岐で
   `settings.json.corrupt-<epoch_nanos:16hex>` へ rename 退避（上書き禁止・
   連番付与・バックアップ失敗は起動を止めず元の InvalidData を維持）。
   settings.rs の既存テストモジュールは GPUI 非依存で SIGSEGV せず動作
   （8 件成功を確認。SIGSEGV は main.rs に限定されることが判明）。

- 子の指摘で見送り: SECURITY.md:114 の文言は方針判断、copyright の
   common-licenses 記述は Debian 一般の真偽未確認のため保留。

- tm-core 88 → **89**、tm-agent 57 → **58**、app 11 → **12**、
  workspace **214 passed** / clippy / deny / rustdoc / check.sh 全ゲート成功。
  markdownlint は NOTICE.md の表区切りを baseline と同じ形式に揃え増分ゼロ。

## 2026-11-18（エージェント並列調査 第7弾 → 機密情報の生エコー停止ほか）

3 観点（agent HTTP / core モデル / 機密情報）の調査エージェントを並列起動し、
各推奨項目を親が現行コードで Runtime Proof してから実装:

1. **argv／パスの生エコー停止**: `agent/src/args.rs` の `--listen` 不正値と
   不明引数、`app/src/main.rs` の `--demo=<値>` エラー、`app/src/settings.rs` の
   設定読み込み／バックアップ失敗ログからユーザー入力値・フルパスを除去
   （誤用時にトークンや HOME が stderr に出る経路を遮断）。
2. **web エラーの redact**: `web/src/agent.rs` のリクエスト組み立て／JSON 解析
   エラーにも `redact_token()` を適用（status message にトークンが流れる経路を遮断）。
3. **R2X-A8**: 500 応答が probe エラー文字列をエコーしていたのを修正。
   詳細は stderr へ、クライアントには固定文言。モック probe の回帰テスト追加。
4. **正規化ヘルパの重複解消**: `format_percent` / `format_cpu_percent` /
   `format_load_average` / `MetricHistory::push` を `normalise_percent` に統一。
5. テスト固定: `flatten_process_tree` の自己親、`format_duration` の単位切替境界。

**見送り**: R1-V08（Host 欠落）は HTTP/1.0 互換の仕様判断、R1-V09（431/400/503 の
CORS）は parse 前の Origin 取得という設計課題、DOM proof のフィルタ文字列は
CDP 契约上マスク不可のため、いずれも親の判断として保留。

- tm-core 89 → **90**、tm-agent 58 → **59**、workspace **216 passed** /
  clippy / rustdoc / check.sh 全ゲート成功 / markdownlint 増分ゼロ。

## 2026-11-18（エージェント並列調査 第8弾 → 接続カウンタ二重減算を修正）

3 観点（実バグ探索 / redact 共通化 / ドキュメント残）の調査エージェントを並列起動。

- **実バグ（信頼度 高、Runtime Proof 済み）**: 前回 R2X-A7 対応（e6b46ff）で
  `thread::Builder::spawn` 失敗時に `map_err` 内の `fetch_sub` と `_slot` の
  `Drop`（std は spawn 失敗時にクロージャをドロップする）の両方が作動し、
  `active` が `usize::MAX` にラップして以降の全接続が 503 になる。
  `RUST_MIN_STACK=1099511627776` で再現（2 回目の /healthz が 503）、
  `fetch_sub` 削除後は 3 回連続で 503 にならないことを確認。
- **ドキュメント追随**: app 16 / web 16（agent 7 + settings 5 + theme 4）の
  実測へ README / current-state / verification を更新。current-state の
  ライセンス（AGPL）とテスト推移（143 → 364、件数は変動するため README.md を参照）も更新。
- **保留（要判断）**: app 版 redact は HOME/USER のみで hostname/tailnet を
  マスクせず SECURITY.md の約束と不一致（スクリーンショット経由の漏えい）。
  tm-core 共通化の提案を記録し、次ラウンドの実装候補とする。
  ADR-021（AGPL 移行）の追加、ADR-011 の superseded 扱いも判断待ち。

- workspace 216 passed / clippy / markdownlint（増分ゼロ）成功。

## 2026-11-18（エージェント並列調査 第9弾 → app redact を agent と共通化）

2 体のエージェント（redact 調査／実装レビュー）を並列起動し、指摘を
Runtime Proof で確認してから実装:

- **tm-core に `redact` モジュールを追加**: `RedactionIdentifiers { home, user,
  hostname }` と pure な `redact_text` / `redact_snapshot`。`env` 読み取りは
  呼び出し側に残し、no-I/O・wasm 対応を維持。既存 agent の tailnet 3 ヘルパを
  移動し、置換順序を `home → hostname → short_host → user → tailnet →
  subdomains → tokens` に修正（短い user が hostname を壊す問題を解消）。
  空／短い識別子の最小長ガードも追加。
- **agent**: ローカル redact ヘルパを削除し共通関数へ切り替え。既存テストは
  回帰として維持（59 件すべて成功）。
- **app**: `display()` が dashboard の hostname を使って共通 `redact_text` を
  呼ぶようにし、`process.name`（テーブル＋詳細）・`summary().disks.mount` ・
  hostname に適用。SECURITY.md の約束（hostname/tailnet 含む）を満たす。
- **レビュー指摘で回避**: `dashboard.snapshot()` は `Option<&SystemSnapshot>`
  なので app では `&mut` を要求する `redact_snapshot` を使わず pure 関数のみ。
  ids は redact 前に clone して所有させ borrow 競合・順序依存を回避。
  IF 名・センサー名は `is_virtual_interface()` の回帰リスクで意図的に非対象。

- tm-core 90 → **94**、workspace **220 passed** / clippy / rustdoc 成功。
  ネイティブ `--demo --redact` 起動スモークも成功。

## 2026-11-18（エージェント並列調査 第10弾 → ADR-021 と SECURITY 明記）

2 体のエージェント（Host 欠落対策／ADR）を並列起動。

- **R1-V08（Host 欠落）は見送りを維持**: 案 B（HTTP/1.1 のみ Host 必須化）は
  技術的には実装可だが、既存テスト 11 箇所で Host 欠落が期待値の前提になっており
  改変が波及する。主防御は Bearer トークンで、DNS rebinding は Host 付きで到達し
  従来どおり 403 となるため防御効果も限定的（子のレビューを Runtime Proof で確認）。
  代わりに SECURITY.md に「Host 欠落時は検証をスキップするがトークン必須、
  ブラウザの fetch は Host を自動付与（Forbidden header）で変更不可」を明記し、
  誤読リスクを解消。
- **ADR-021 を追加**: commit fdf6f62 の AGPL-3.0-or-later 移行を DESIGN.md の
  ADR 一覧末尾（ADR-020 の後）に記録。ADR-011（Apache）は一切書き換えず履歴として
  保持し、ADR-021 側に関係を記載（repo の ADR-012 は in-place 改訂の前例だが、
  履歴保持の制約を優先）。frontmatter は慣例どおり未更新。
- **SECURITY.md:114** の「ライセンス（Apache-2.0 互換）」を
  `deny.toml` の `[licenses].allow`（AGPL 互換）を参照する文言へ修正。

- workspace 220 passed / markdownlint（DESIGN 51 / SECURITY 9、増分ゼロ）成功。

## 2026-11-18（エージェント並列調査 第11弾 → 一時停止・初回warm-up・503コメント）

3 観点（pause 競合 / 503 ブロック / warm-up）の調査エージェントを並列起動し、
各指摘を Runtime Proof で確認してから実装:

1. **一時停止の競合（実バグ）**: viewmodel の `ingest()` に `paused` ガードを追加。
   サンプリング中に Space で一時停止しても in-flight 結果で画面が動かなくなった。
   `Tick` / `ingest` / `ConfirmKill` を同じ pause 不変条件に統一。pure テスト
   `ingest_is_ignored_while_paused` を追加（app の SIGSEGV 制約を回避）。
   子の副次発見どおり web には paused 参照が 0 件で一時停止が機能していなかった
   が、viewmodel ガードで web も同時に解消。
2. **初回 HTTP の warm-up（実バグ）**: 子が実プロセスで再現（初回は全 CPU 0.0
   で PID 昇順 500 件、73% のビジープロセスが欠落）。`SharedState` に one-shot
   `warmed_up: AtomicBool` を追加し、認証済み初回 snapshot のみ 250ms 空けて
   2 回 sample（捨て＋採用）。認証失敗／healthz／不正 limit は warm-up を消費
   しない。probe mutex は route 全体で保持されるため競合なし。demo は warm-up
   不要（tick が進むため）。回帰テスト `CountingProbe` で sample 回数を固定。
3. **503 コメントの訂正**: 上限超過 503 は accept スレッドで同期的に書かれる。
   子の反証どおり 240 バイトで実害は到達不能（既存計測 0.03ms）かつ worker 化は
   DoS ベクタになるため構造は維持し、コメントを実際の挙動に合わせた。

- tm-agent 59 → **60**、tm-viewmodel 40 → **41**、workspace **222 passed** /
  clippy / rustdoc / check.sh 成功。

## 2026-11-18（エージェント並列調査 第12弾 → デモ/SBOM/web fmt の実バグ）

3 観点（IME / デモ境界 / スクリプト）の調査エージェントを並列起動し、
各指摘を親が Runtime Proof で確認してから実装:

1. **demo のメモリ列が静止（実バグ）**: `noise as u64` は常に 0 なので
   `(noise * 4_096.0) as u64` に変更（スケールしてから整数化）。
   `demo_process_memory_varies_with_tick` で tick 差と不変条件を固定。
2. **`http://[::1]` が拒否（実バグ）**: 前回実装した `is_loopback_http_endpoint`
   が IPv6 リテラルのコロンを port 区切りと誤判定していた。括弧の後ろだけを
   port として扱うよう修正し、allowed リストに `http://[::1]` を追加。
3. **SBOM artifact から root SBOM が欠落（実バグ）**: `find .` は
   `./target/sbom/...` を返すのに除外パターンが `${out_dir}/*` で不一致。
   `out_dir` を `./target/sbom` 形式へ正規化。Stub で再現・修正確認済み。
4. **web rustfmt ゲート漏れ（実バグ）**: `cargo fmt --manifest-path
   web/Cargo.toml --all --check` が exit 1（3 箇所の整形漏れ）だった。
   check.sh と ci.yml の extra_gates に web fmt を追加し、web を整形。

**却下／保留**: IME の「Enter で未確定文字が残る」シナリオは子の静的解析で
表示が Dashboard 確定値に戻ることが判明し却下。R2X-S1（unmark の
`committed_text()` 矛盾）、demo pid 421 の self-parent、補助面 VS、
schema 互換方針、GLIBC バージョン付き Depends は設計判断として保留。

- tm-core 94 → **95**、workspace **223 passed** / web 16 / clippy / check.sh /
  shellcheck / shfmt / YAML 成功。cargo deny は一時 GnuTLS エラー后再試行で成功。

## 2026-11-18（エージェント並列調査 第13弾 → CDP/status/CLI の実バグ）

3 観点（設定状態機械 / web CDP / 認証起動）の調査エージェントを並列起動し、
各指摘を親が Runtime Proof で確認してから実装:

1. **CDP read_loop の pending future 解決漏れ**: `cdp_verify_web.py` は
   read loop 終了時に pending future を放置し `call()` が 120 秒ハング
   （IME 版には完成した修正例あり）。IME 版と同じ `set_exception` を移植。
   settings スクリプトも web を import するので同時に解消。
2. **R3-U05 transient status の未クリア**: `StatusMessage` に `persistent`
   を追加し、`set_persistent_status` / `clear_transient_status` を新設。
   native/web の成功 ingest 時に transient のみクリア（起動エラーは永続）。
   pure テスト `transient_status_is_cleared_but_persistent_status_survives`
   を tm-viewmodel へ追加。
3. **`parse_gui_flags` の未知フラグ無視**: `--redcat` 等の typo が黙って
   redact=false で起動し SECURITY.md の公開物衛生に反するシナリオを修正。
   未知フラグを拒否（値はエコーしない）。usage にも明記。
   app テストは SIGSEGV 制約で追加せず、実バイナリで `--help` / `--version` /
   `--demo=yes` / `--redcat` の全経路を確認。

**却下／保留**: R2-D01（version 保持）/ R5-D02（未知フィールド）は現状
再現不能でマイグレーション方針が必要、R3-U01（破損設定 GUI 通知）は
persistent 受け皿ができたので別途、CDP の固定 sleep・ポート競合・F2/
Space parity は設計判断／CDP 証跡更新を伴うため保留。

- tm-viewmodel 41 → **42**、workspace **224 passed** / clippy / check.sh /
  python py_compile 成功。CLI 全経路の Runtime Proof 成功。

## 2026-11-18（エージェント並列調査 第14弾 → 前回回帰と redact/HTTP 漏れ）

3 観点（描画境界 / 不変条件 / HTTP 完全性）の調査エージェントを並列起動し、
各指摘を親が Runtime Proof で確認してから実装:

1. **前回実装の回帰（2 エージェントが独立に指摘）**: R21 の paused ガードと
   R22 の transient クリアが交錯し、一時停止中に in-flight サンプルが完了すると
   `ingest` は no-op なのに `clear_transient_status()` が「一時停止しました」
   を消していた。`clear_transient_status()` を `ingest` 内（paused ガード
   通過後）へ移動し app/web の重複呼び出しを削除。demo/live 両方を同時解消。
   `pause_notice_survives_in_flight_ingest` で回帰固定。
2. **`--redact` の漏れ**: 詳細パネルのユーザー、確認ダイアログのユーザー名と
   プロセス名の 3 箇所が `self.display()` を通しておらず SECURITY.md の
   約束（公開スクリーンショット）に反していた。3 箇所を統一。
3. **HTTP ヘッダ名の空白／obs-fold**: `name.trim()` により `Host : x` や
   CRLF で始まる折り返しの `Authorization` 行が受理されていた
   （RFC 9110 §5.1 MUST 違反、   子が /tmp で Runtime Proof）。trim を廃止し、ヘッダ値の C0/DEL も拒否。
   実機で両ケースが 400、正規 healthz が 200 であることを確認。

**保留**: persistent status の上書き（1 行ステータスで最新エラー優先という
意図があり得るため設計判断）、再開直後の network rate、CDP の固定 sleep /
ポート競合 / F2 parity、R2-D01 / R5-D02。

- tm-agent 60 → **61**、tm-viewmodel 42 → **43**、workspace **226 passed** /
  clippy / check.sh / 実機 HTTP確認 成功。

## 2026-11-18（エージェント並列調査 第15弾 → 永続通知とスクロール追従）

3 観点（履歴時間軸 / コア不変条件 / app 状態）の調査エージェントを並列起動。
別セッションが agent/web/CI/scripts を編集中だったため、調査・実装は
`crates/` と `app/` に限定し競合を回避（別セッションの作業は commit
`8acb10c` として反映済み）。

1. **永続 status の pause/resume 消滅（実バグ）**: `SetPaused` が永続
   起動警告（「デモデータで起動しました」）を transient で上書きしていた。
   persistent なら上書きしないガードを追加。
2. **再開直後の network rate 混入**: 同じ分岐で `last_network = None` とし、
   paused 中に受け取ったバイトが再開直後の瞬間レートに化けるのを防止。
   pure テスト `persistent_notice_survives_pause_resume_and_rate_starts_fresh`
   を追加。
3. **選択行のスクロール追従（実バグ）**: ingest で選択がフォールバック移動
   したとき `scroll_to_item` が呼ばれず選択行が画面外に残っていた。
   選択キーが変わった時だけ `ScrollStrategy::Nearest` で追従。

**却下／保留**: 非単調 sampled_at のガード（配信順序と絡む設計判断）、
温度値の正規化契約（実害観測例なし）、status_bar の生テキスト
（識別子を含むエラー文言は現状確認できず）、破損設定の GUI 通知
（文言合成と解除方針の設計判断）。

- tm-viewmodel 43 → **44**、workspace **235 passed** / 全テスト成功。

## 2026-11-18（エージェント並列調査 第16弾 → 境界値・設定・ドキュメント）

3 観点（数値精度 / app 状態 / ドキュメント）の調査エージェントを並列起動。
別セッションが agent/web/CI/app/src/main.rs を活発に編集中だったため、
調査・実装は crates/・app/src/settings.rs・ルートドキュメントに限定し競合回避。

1. **format_bytes の繰り上がり（実バグ、子が Runtime Proof）**: 1,048,575
   バイトが `1024.0 KiB` と表示されていた（本来 `1.0 MiB`）。`{:.1}` で
   1024.0 に丸まる分を上位単位へキャリー。format_rate にも波及する。
2. **settings.json がディレクトリのとき設定永続化が壊れる（実バグ）**:
   `EISDIR` は InvalidData ではないため退避されず、save が毎回失敗して
   一時ファイルが蓄積。path.is_dir() も退避対象に追加し、save 失敗時に
   一時ファイルを削除。pure テスト 2 件追加。
3. **ドキュメントの /metrics 欠落**: DESIGN.md:271 / SECURITY.md:42-43 が
   別セッション追加の認証不要 `/metrics` を欠き「全リクエスト Bearer 必須」
   という根拠が事実と矛盾。3 エンドポイントと認証例外を明記。
   README のテスト件数 234→235、check.sh 説明に rustdoc/markdownlint 追記。

**保留**: WireGuard IF（wg0）の二重計上（設計判断）、連番枯渇の非上書き化
（到達困難）、load average の極端値（現実シナリオなし）、persistent status
の運用記述（配置場所の判断）。

- tm-core 95 / app 18（テスト 2 件追加）、workspace 237 passed / clippy /
  markdownlint（増分ゼロ）成功。

## 2026-11-18（エージェント並列調査 第17弾 → バックアップ保全とVS17）

3 観点（サニタイズ / app 状態 / ドキュメント）の調査エージェントを並列起動。
別セッションが crates/tm-core/src/query.rs・tm-viewmodel・app/src/main.rs・
web 等を活発に編集中だったため、実装は競合しない sanitize.rs・settings.rs
に限定。調査中に別セッションが commit 0693fb2（仮想メモリソート等）を打ち、
テスト件数は 261 / web 36 に更新された。

1. **backup_candidate の連番枯渇（実バグ）**: 旧コードは `counter=65535` の
   とき最終候補 `-65535` を存在チェックせず rename し、既存バックアップを
   黙って上書きしていた（子がシミュレーションで実証）。存在判定を注入した
   pure ヘルパ `backup_candidate` に切り出し、u64 の無制限カウンタで
   未使用パスを選択。回帰テスト追加（65536 番目に進むことを固定）。
2. **補助面 VS17..256 のサニタイズ**: is_invisible_format が
   U+E0100..U+E01EF をカバーしていなかった。既存方針（VS1-16 を空白化、
   PID とペアで識別）に従い 1 行追加＋境界テスト。

**保留**: 温度の NaN°C（app/web の表示配線が必要で別セッションと競合）、
redact の 2 文字ユーザ名（漏らさない優先か整合性かの設計判断）、
Ctrl+K の読み取り専用ガード、demo の base_memory=0 ジッタ、
SECURITY/DESIGN のテスト件数（別セッションが 261 に追随中）。

- 担当範囲: app 28 passed / tm-core の該当テスト成功 / clippy / rustfmt OK。
  `crates/tm-core/src/export.rs`（別セッションの未コミット作業）のテスト
  1 件失敗は担当外のため修正せず。

## 2026-11-18（エージェント並列調査 第18弾 → 設定保全・demo・SECURITY）

3 観点（コア / 設定 / ドキュメント）の調査エージェントを並列起動。別セッション
が app/src/main.rs・tm-viewmodel・web を活発に編集中だったため、実装は競合しない
settings.rs・demo.rs・SECURITY.md に限定。調査中に別セッションが 9e8ca34
（温度ゲージ、CSV/JSON 変換、status 整理）をコミット。

1. **非UTF-8 パスでバックアップが壊れ破損データが失われる（実バグ）**:
   `.display().to_string()` が 0xFF を U+FFFD に化け、rename 先が別ディレクトリ
   になって ENOENT → その後の save で破損データが上書き消失していた。
   backup_candidate / backup_corrupt_file を OsString / Path ベースに変更。
2. **umask で 0600 保証が破られる**: `.mode(0o600)` は umask でマスクされる
   （umask 0777 で 0000）。open 直後に `set_permissions(0o600)`（fchmod、
   umask 非依存）を追加し次回起動の EACCES を防止。
3. **demo の base_memory=0 プロセスが非ゼロ RSS**: kthreadd / zombie-proc が
   tick ごとに RSS/VM が揺れていた。CPU と同じパターンで 0 固定。回帰テスト
   `zero_rss_demo_processes_stay_at_zero` 追加。
4. **SECURITY.md §5 に status 運用を追記**: `[情報]`/`[エラー]` 接頭辞、
   transient は成功 ingest 時のみクリア（paused 中は維持）、persistent は
   サンプル・pause/resume をまたいで保持。3 回以上保留だった項目を解消。

**保留**: Cf タグ文字（Cf 全般の設計判断）、redact 2 文字ユーザ名（漏らさない
優先で現状維持、恒真テストは別途修正）、温度の normalise_temperature helper
（export.rs 側の引き継ぎ必要）、symlink 扱い（設計判断）、テスト件数
（別セッションが 276/42 に追随中で数字を書かない方式を検討）。

- 担当範囲: app 35 passed / tm-core 110 passed / clippy / rustfmt / markdownlint
  （増分ゼロ）成功。

## 2026-11-18（エージェント並列調査 第19弾 → CSV数式対策と文書の件数参照化）

3 観点（コア / 設定CLI / 文書）の調査エージェントを並列起動。別セッションが
app/src/app.rs・tm-core/port.rs・sanitize.rs を編集中だったため、実装は競合しない
export.rs・DESIGN.md・SECURITY.md に限定。

1. **CSV 数式インジェクションの抜け道（実バグ、子が Runtime Proof）**:
   csv_text の無害化集合が `\n` / 空白 / VT / FF を欠き、`"\n=1+1"` や
   `" =1+1"` がスプレッドシートで数式として解釈され得た。先頭文字集合に
   追加し、pure テスト `csv_neutralises_leading_whitespace_and_newline_before_formulas`
   を追加。RFC 4180 の引用判定（`\n`/`\r` 両方）との非対称を解消。
2. **テスト件数を README 参照に集約**: DESIGN.md / SECURITY.md の
   「60 件超 / 47 / 13 / 8 / 175 件 / 28 件 / 12 件」を削除し、
   README.md「検証状況」を参照する文言に変更。件数は別セッションが頻繁に
   更新するため（現行 316 / web 57）、数字を書かない方式で陳腐化を防止。

**見送り**: `--export` の umask 0777 下 mode 0000（app/src/main.rs に別セッション
の未コミット変更が混在し、巻き込みリスクがあるため次ラウンドに回す）、
symlink の settings.json 置換（設計判断）、TM_REDACT 不正値（要判断）、
Select(None) の復活（到達性限定）、DESIGN §1.2 の非目標矛盾（次点）。

- tm-core 129 passed / clippy / markdownlint（増分ゼロ）成功。
  CI の rustfmt との食い違いを避けるため import 並び順は HEAD のまま維持。

## 2026-11-18（エージェント並列調査 第20弾 → UTF-8境界・CSV構造・選択クリア）

3 観点（エクスポート / コア / 状態機械）の調査エージェントを並列起動。別セッション
が多数のファイルを編集中だったため、実装は競合しない3ファイルに限定。

1. **redact_tailnet_domains の UTF-8 境界パニック（2 エージェントが独立に
   実 API 再現）**: `rfind` のバイト index に +1 してスライスしていたため、
   ドメイン区切り直前に日本語があると `byte index is not a char boundary` で
   パニック。`char_indices().rev().find()` + `len_utf8()` に変更。
   `--redact` / agent snapshot / export の全経路が対象。回帰テスト追加。
2. **CSV アポストロフ位置が RFC 4180 引用を壊す（前回修正の不備）**:
   アポストロフを引用の外側に置いていたため、RFC 4180 パーサが引用を認識せず
   カンマで列が分割され、後続フィールドが `=` で始まり数式として解釈され得た。
   引用の内側の直後にアポストロフを置く方式に変更。既存期待値を更新し、
   列分割しないことを pure テストで固定。
3. **Select(None) の選択復活（API 契約違反）**: `ensure_selection_visible()`
   と `ingest()` の fallback の2系統で `selected=None` が復活していた。
   `had_snapshot`/`had_selection` で3 states（初回／明示クリア／消滅）を区別し、
   明示クリアのみ維持。回帰テスト追加。

**却下／保留**: persistent status の上書き方針（要判断）、serde enum 境界
（query.rs は別セッション編集対象）、symlink 扱い（設計判断）、
RequestSignal の paused ガード（低優先）。

- tm-core 131 passed / viewmodel 63 passed / clippy 成功。
  rustfmt で変わった既存行の import 順・文字列整形は HEAD 形式に戻し、
  CI の rustfmt との食い違いを回避。

## 2026-11-18（エージェント並列調査 第21弾 → status集約・redact契約・CLI表記）

3 観点（状態機械 / コア / 設定CLI）の調査エージェントを並列起動。別セッション
が多数のファイルを編集中だったため、実装は競合しない3ファイルに限定。
前回の rustfmt 問題を踏まえ、直接 rustfmt は実行していない。

1. **status 書き込みの集約（3 回以上保留の決着）**: request_signal /
   confirm_kill の 8 箇所の直接 `StatusMessage` 代入を `set_status()` に集約。
   生成される StatusMessage は等価で挙動変更ゼロ。SECURITY.md §5 の
   「自動遷移は永続を保持／明示メッセージは上書き」という契約を doc に明記。
   将来 persistent 保護方針を採用しても変更点が1箇所。
2. **redact 全フィールド網羅テスト**: secret_snapshot の既知 secret 検査は
   `..default()` で将来フィールドを空文字で呑み込むため漏えいを検出できない。
   `SECRETS` 共有定数化＋`redact_for_export` の Debug dump で全フィールドを
   検査するテストを追加（新フィールド追加時に漏えいを検出）。
3. **CLI 表記**: help の終了コード「0/2/4」→「0/1/2/4」（EXIT_INTERNAL=1
   が実際に返る）、ライセンス「Apache-2.0」→「AGPL-3.0-or-later」。
   CSV 列数コメントの誤記（13列→12列+埋め込みカンマ1）も訂正。

**却下／保留**: persistent status 保護方針 A/B（SECURITY §5 で上書きは許可
済みと判明、現状維持を明文化）、network baseline スパイク（app.rs 側の
in-flight 連携が必要）、serde enum 境界、symlink 扱い。

- tm-core 132 / viewmodel 63 / app 61 passed / clippy / markdownlint 増分ゼロ。

## 2026-11-18（エージェント並列調査 第22弾 → status redact と SSE 文書）

3 観点（コア / app / 文書）の調査エージェントを並列起動。別セッションが
agent/web/crates を活発に編集し、10a8e16（プリセット・電源・SSE）自体が
未コミット差分でコンパイル不能だったため、クリーンな worktree（d55552b）
を作成して本来の checkout を汚さずに実装・検証した。

1. **status bar / overlay の --redact 漏れ（実バグ）**: シグナル送信失敗時に
   `ControlError::Failed` の詳細（taskkill のパス等）が status テキストとして
   生表示されていた。status bar と overlay の 2 箇所を `self.display()` 経由に。
2. **NaN の CPU 色矛盾（軽微）**: `usage_color(cpu.min(100.0))` は NaN を
   無視して 100.0（赤）を返す一方、ラベルは `0.0%` だった。NaN を green に
   フォールバックする `usage_color(cpu)` に変更し表示と色を整合。
3. **SECURITY.md の SSE 方針追記**: 「読み取りタイムアウト 1 秒・同時接続
   8 本」とだけ書かれ、SSE はタイムアウト無し・別枠であることを運用者が
   誤認する状態を解消。Bearer/Host/CORS は共通、クエリトークン禁止、
   専用枠・切断まで配信を明記（数値・パスは書かない）。
4. **DESIGN.md**: エンドポイント「3 つ」→ SSE を含む記述に変更。テスト件数の
   数値（334/57）を削除し README「検証状況」参照へ集約（陳腐化防止）。

**見送り**: export.rs の redact 全フィールドテストが新規型（power/cgroup）を
exercise しない虚偽確信（新規型はまだ SystemSnapshot に未接続、別セッション
の migration 時に同時修正が必要）、app への power/preset/cgroup 未配線
（設計判断）、SECURITY の SSE 検証項目（実装側テスト未確認）。

- app 61 passed / clippy / fmt / markdownlint（増分ゼロ）成功。

## 2026-11-18（エージェント並列調査 第23弾 → 温度の正規化と全TODO検証）

3 観点（新機能レビュー / 境界監査 / TODO整合）の調査エージェントを並列起動。
元 checkout は別セッションの 10a8e16/6eed24d（未 push）で diverge 状態のため、
前回同様 worktree（cb1d95a）で実装・検証した。

1. **温度の正規化バグ（明確な実バグ、3 回以上保留）**: `export.rs` の
   `push_f32` が温度にも `normalise_percent` を適用し、100°C 超 → 100、
   負温度 → 0 に改変していた。tm-core に `normalise_temperature`（非有限のみ
   0、負・100 超は保持）を追加し温度出力を専用ヘルパに分離。回帰テストで
   120.5°C / -20°C / NaN の JSON 出力を固定。

**監査で判明した重要事実**:
- SSE はサーバ側 `/api/v1/stream` のみで、クライアント接続コードは 0 行。
  さらに OPTIONS プリフライトが 401 で失敗するため「全 TODO 完了」は
  過剰宣言（配線とプリフライト修正はセットで必要、別セッションの範囲）。
- redact 全フィールド網羅テストと sanitize カバレッジは新機能側が追従済みで
  第22弾の虚偽確信は解消。SSE エラー/status もクライアント未接続で影響なし。
- 残る要判断: WireGuard wg0、symlink settings、serde enum 境界、
  request_signal の paused ガード、恒真テスト、app への preset 未配線、
  adaptive interval の即時反映、sysfs online 欠落時の誤判定。

- tm-core 133 passed / clippy / fmt 成功。

## 2026-11-18（エージェント並列調査 第24弾 → redact 契約テストの実質化）

probe-todofinal が「全 TODO 完了」を検証し、紅真テスト（常に真で何も検証
していない）を指摘。worktree（cb1d95a → ac685f2）で修正:

1. `redact.rs:180-183` の `assert!(!out.contains("java") || out.contains("java"))`
   を `assert!(out.contains("java"))` に変更。5 文字ユーザ名 `alice` が
   無関係語 `java` を壊さないことを実際に検証。
2. 最小長ガードの境界テスト `minimum_length_guards_are_exact` を追加。
   home/hostname は 3 文字未満維持・3 文字以上置換、user は 1 文字維持・
   2 文字置換、tail トークンは 5 hex 維持・6 hex 置換を固定。
   子のテスト案で `/ab`（スラッシュ含む 3 文字）の期待値が誤っていたため
   実装（>= 3）に合わせて訂正。

- tm-core 134 passed / clippy / fmt 成功。

## 2026-11-18（エージェント並列調査 第25弾 → 仮想IF判定とSSE文書の整合）

3 観点（状態機械 / データ境界 / 文書）の調査エージェントを並列起動。
worktree（round31-safe、HEAD a96e823）で実装・検証。

1. **仮想 IF 判定の漏れ（実バグ）**: `is_virtual_interface()` の
   `VIRTUAL_PREFIXES` に WireGuard `wg` と macOS `utun` が無く、`lo0` も
   loopback 判定から漏れていた。tailscale0 と同種の二重計上が起きる。
   `wg`/`utun` を追加し `lo`/`lo0` を明示判定（`starts_with("lo")` は
   `local0` を誤判定するため避ける）。`tm-platform` の重複 `name == "lo"`
   判定を共通 pure ヘルパに集約。`local0` が物理のままであることを固定。
2. **SSE 文書の矛盾**: 第22弾で DESIGN/SECURITY に追加した詳細な SSE 方針は、
   このブランチに SSE が未実装（`route()` は 3 パスのみ）ため運用者が
   `/api/v1/stream` へ接続して 404 になる／誤監査を招く状態だった。
   「このブランチでは未実装・ポーリングが唯一のライブ経路」と明記。
3. **README 件数**: 実測 362（core 134 / viewmodel 63 / platform 26 /
   agent 78 / app 61）に追随。

**見送り**: settings の未知 enum 値が Corrupt 扱いになる前方互換契約の破壊
（viewmodel/settings.rs の Lenient デシリアライザで直せるが契約解釈に幅あり、
次の候補）、request_signal の paused ガード（低優先・UX 不一致のみ）、
bond0（実機確認が必要な設計判断）。

- tm-core 134 / tm-platform 26 passed / clippy / fmt / markdownlint 増分ゼロ。

## 2026-11-18（エージェント並列調査 第26弾 → settings 未知 enum の寛容化）

probe-vmfinal が「文書化された前方互換契約の破壊」と判定した項目を修正。
worktree（round31-safe）で実装・検証。

1. **未知 enum 値が settings 全体を無効化する問題**: `#[serde(default)]` は
   フィールド欠落しか吸収せず、`sort.key="FutureKey"` や `theme="FutureTheme"`
   は解析失敗 → native は設定退避＋赤警告、web は `LAST_LOADED` 消失 →
   次回 save で将来版 `version` と未知フィールドが失われていた。
   `crates/tm-viewmodel/src/settings.rs` に `lenient()` デシリアライザ
   （`Value` 経由で失敗時 `Default`）を追加し `sort` / `theme` に適用。
   部分 struct（`{"key":"Pid"}`）も既定 direction へフォールバック。
   `serde_json` を dev-dependency から通常依存へ昇格。
   `normalized()` の version 強制は触らず、shell の `clamped()` 契約を維持。
2. **README 件数**: 実測 363（viewmodel 63 → 64）に追随。

**見送り**: request_signal の paused ガード（低優先・UX 不一致のみ）、
bond0 の仮想 IF 追加（実機確認が必要）、diverge 解消（rebase は破壊的
操作で委任外、別セッションの未 push 2 コミットは触れない）。

- viewmodel 64 passed / workspace 363 passed / clippy / fmt / machete / deny
  成功。

## 2026-11-18（エージェント並列調査 第27弾 → request_signal の paused ガード）

第25弾 probe-vmfinal が「低優先・要判断」として残した UX 不一致を解消。
worktree（round31-safe）で実装・検証。

- 一時停止中に Ctrl+K／詳細パネルの終了ボタンを押すと、古い snapshot で
  確認ダイアログが開いていた。確定時に `confirm_kill` の paused ガードが
  拒否するため信号発行はされないが、「開けるのに確定できない」矛盾があった。
- `request_signal()` に paused ガードを追加（capability／保護チェックの後ろ、
  pending 上書き防止の前）。メッセージは `confirm_kill` と同一。
- 回帰テスト `request_signal_is_rejected_while_paused` を追加（停止中は
  pending が作られず、再開後は通常どおりダイアログが開くことを固定）。

**見送り**: bond0 の仮想 IF 追加（実機確認が必要）、diverge 解消（rebase は
破壊的操作で委任外、別セッションの未 push 2 コミットは触れない）。

- viewmodel 65 passed / workspace 364 passed / clippy / fmt 成功。

## 2026-11-18（エージェント並列調査 第28弾 → web lock と weak assertion）

3 観点（テスト品質 / panic 経路 / 設定依存）の調査エージェントを並列起動。
worktree で実装・検証。

1. **web/Cargo.lock の陳腐化（実 CI 破壊、親の Runtime Proof で確認）**: 第26弾で
   `serde_json` を dev-dependency から通常依存へ昇格した際、`web/Cargo.lock` の
   `tm-viewmodel` 依存リストが更新されず、`(cd web && cargo vet --locked)` が
   exit 255、`cargo metadata --locked` が exit 101 で失敗していた。
   1 行追加で解消（vet 560 exempted 成功を確認）。子の「CI が壊れる」という
   報告は実 CI 8/8 success と矛盾したが、ローカルの --locked は確実に失敗
   しており修正した（AGENTS §7 に従い親の Runtime Proof を優先）。
2. **weak assertion の修正（probe-testquality）**:
   - `sorting_by_virtual_memory_defaults_to_descending` の
     `spec.direction.arrow() == SortDirection::Descending.arrow()`（同語反復）を
     `"▼"` との比較に変更。
   - `user_inside_hostname_does_not_break_redaction` の部分 `contains("java")`
     を完全出力 `"https://host host java"` との assert_eq に変更（期待値を
     実装に合わせて訂正）。
   - `auth.rs` の認証トークン再比較（dedicated テストと重複）を削除。
3. **panic 経路監査**: ユーザー入力/wire から到達可能なクラッシュは 0 件。
   UTF-16 境界変換は `floor_byte_index` で防御済み（将来リスクとして
   `utf16_index` の内部丸めを記録）。

- workspace 364 passed / clippy / fmt / cargo vet --locked（web）/ markdownlint
  増分ゼロ成功。`tm-platform` の実プロセステストが 1 回 flaky に失敗したが
  再実行で成功（変更と無関係）。

**見送り**: bond0 の仮想 IF 追加（実機確認が必要）、diverge 解消（rebase は
破壊的操作で委任外）。

## 2026-11-18（エージェント並列調査 第29弾 → CI/security 文書と weak assertion）

3 観点（CI 整合 / エッジ / 文書）の調査エージェントを並列起動。worktree で実装・検証。

1. **ci.yml:150 コメントの誤った安全感**: 「advisory ignore は 3 か月で expire し
   期限切れは check を失敗させる」とあったが、Commit 5ae8290 で expiry キーは
   削除され deny.toml は「手動 3 か月レビュー」に変更済み。コメントを実態へ修正。
2. **format.rs の weak assertion**: `format_duration(u64::MAX).ends_with('h')` と
   `format_count(u64::MAX as usize).contains(',')` を、完全出力
   `"213503982334601d 7h"` / `"18,446,744,073,709,551,615"` との assert_eq に変更。
3. **notes のテスト件数**: verification.md / current-state.md の 334/57（古い）を
   「README.md『検証状況』を参照」方式へ置換。DESIGN の AC-12 マトリクス判定と
   実態が一致するようにした。
4. **SECURITY.md の未来形**: 「公開リポジトリ化する際は」→「public 化済みのため
   サポート対象バージョンと連絡先を早急に追記する（現状未追記）」に修正。

**見送り**: check.sh の `cargo audit` / `--locked` 追加（CI との完全同期だが
ローカル開発での lock 更新義務が生じる副作用あり、要判断）、
Timestamp::elapsed_secs_since の契約明記（未使用 API、設計判断）、
export.rs:711 の恒真テスト（修正範囲がやや広い）、bond0、diverge 解消。

- workspace 364 passed / clippy / fmt / yaml / markdownlint 増分ゼロ成功。

## 2026-11-18（エージェント並列調査 第30弾 → API doc と round-trip 契約）

2 観点（API 契約 / weak assertion）の調査エージェントを並列起動。worktree で実装・検証。

1. **export.rs の CSV 数式テストを実質化**（前回見送り分）: `csv.lines()` が
   引用内改行で行を分割し 2 つ目の `contains` が恒真だった。CSV 全体の
   suffix 比較へ変更し、4 ケースの完全フィールド形状を固定。
2. **Timestamp::elapsed_secs_since の契約明記**: 秒丸め先行のため
   `(unix_millis 差)/1000` と最大 1 秒ずれることを doc に明記し、
   秒境界をまたぐケース（1900ms→2100ms → 1）をテストで固定。
3. **redact_snapshot の doc 誤記を修正**（probe-apicontract）:
   「every display string」とあったが実際は hostname / process user/name/
   command / disk mount_point のみ。IF 名・センサラベル・OS/kernel は
   意図的に対象外で `redact_for_export()` が補完することを明記。
   doc を信じた呼び出し側が識別子を漏らすプライバシー影響を解消。
4. **settings round-trip テストの完全 JSON 比較化**（probe-weakfinal）:
   部分 `contains` 2 行を完全シリアライズ文字列との assert_eq に置換。
   フィールド名変更・スキーマ変更を検出可能に。

- tm-core 134 / tm-viewmodel 65 passed / clippy / fmt / rustdoc 成功。

## 2026-11-18（エージェント並列調査 第31弾 → probe ロック範囲とヘルプ終了コード）

3 観点（エラーメッセージ / ホットパス / 並行性）の調査エージェントを並列起動。
worktree で実装・検証。

1. **probe Mutex の過剰取得を修正**（probe-concurrency・実バグ）: 全リクエストで
   `worker_state.probe.lock()` していたため、初回 warm-up の 250ms sleep 中に
   `/healthz` / `/metrics` まで待たされていた。`route()` の probe 引数を
   `Option<&mut dyn SystemProbe>` に変更し、SNAPSHOT_PATH のときだけロック。
   probe 不要なエンドポイント（health/metrics/OPTIONS/405/404）はロック外で
   dispatch される。hotpath エージェントも同じmutex保持時間を指摘していた。
2. **help の終了コード自矛盾を修正**（probe-errmsgs）: 見出しは「0/1/2/4」なのに
   一覧に「3 ネットワークエラー」「5 認証・トークンエラー」が残っていた。
   これらは tm-agent 固有で gpui-task-manager は返さないことを明記。
   テスト期待値も新しい文言へ更新。

**見送り**: nfc_fold の sort_by_cached_key 最適化（性能改善だが計測根拠なし、
AGENTS §3 に従い機能バグと区別）、--redact の GUI 側 user/kernel/label ギャップ
（app/app.rs の変更が必要で rustfmt 既存行リスクあり、次ラウンド候補）、
JoinTracker の panic 統計、RequestTrace::finish のタイミング（計測精度のみ）。

- workspace 364 passed / clippy / fmt 成功。

## 2026-11-18（エージェント並列調査 第32弾 → GUI redact の漏れを三段階で塞ぐ）

probe-errmsgs（第31弾）が最優先で指摘し rustfmt リスクで見送った
「--redact の GUI 側ギャップ」を、直接 rustfmt を使わず edit のみで実装。
worktree（round31-safe）で検証。

1. 詳細パネルの process.user → `self.redact` 時は "user" 固定
   （display() はログイン名しか置換せず、root 等が残っていた）。
2. テーブル行の user 列 → 同上。
3. summary.kernel → `self.display()` 経由。
4. component.label（2 箇所）→ `self.display()` 経由。
5. probe-redactfinal の最終監査で残っていた確認ダイアログの user も
   同上の "user" 固定へ修正（テーブル／詳細との不整合を解消）。

エージェント側 `redact_snapshot()` と export 側 `redact_for_export()` は
全 process.user を "user" に置換しており、GUI のみが約束を満たして
いなかった。公開スクリーンショットの機密性約束を充足。

**見送り**: フィルタ入力のエコー（ユーザー入力で IME 変換中文字列を
書き換えると候補位置がずれる、設計判断）、ステータスバーの OS ラベル
（汎用 OS 名で実害なし）。

- app 61 passed / clippy / fmt 成功。

## 2026-11-18（エージェント並列調査 第33弾 → wire/demo 契約の整合）

3 観点（wire 境界 / テストデータ / 設定移行）の調査エージェントを並列起動。
worktree で実装・検証。

1. **未知 ProcessStatus が全 snapshot を拒否する問題**（probe-wire2・3 回連続
   先送り項目）: `ProcessStatus::Unknown` に `#[cfg_attr(feature = "serde",
   serde(other))]` を付与。将来 agent が新しい status を返しても Err ではなく
   `Unknown`（表示「不明」）へ縮退。Serialize 出力は不変。回帰テスト追加。
2. **demo と real probe の順序乖離**（probe-testdata）: demo の disks を容量
   降順（`/home` → `/`）へ、processes を `(pid, start_time_secs)` 昇順へ変更。
   real の `sort_disks_for_display` / PID 順と同じ契約に整合。UI はテーブル
   側でソートするため画面は不変だが、`--export` の行順が real と揃う。
   共通契約テスト `demo_matches_native_probe_ordering_contracts` を追加。
3. `tm-core` に serde_json を dev-dependency 追加（回帰テスト用）。
   root Cargo.lock に 1 行追加、web vet --locked は 560 exempted 成功。

**見送り**: 未知 enum/サブフィールドの save 時上書き（保持 vs migrate の
設計判断、probe-migration）、primitive 型不一致の corrupt 経路（契約解釈）、
再度の version チェック前倒し（範囲が広い）。

- workspace 366 passed / clippy / fmt / machete / deny / web vet --locked 成功。

## 2026-11-18（エージェント並列調査 第34弾 → collapse 選択と前方互換の契約）

2 観点（未調査観点 / 前方互換設計）の調査エージェントを並列起動。worktree で実装・検証。

1. **collapse で選択行が無関係な先頭へ飛ぶ実バグ**（probe-lastdrop）:
   tree view で子を選択して親を collapse すると、`flatten_process_tree` が
   子を visible から除外し `ensure_selection_visible` が `visible.first()`
   へフォールバックしていた。`ToggleTreeNode` で collapse する際、
   選択が対象ノードの子孫なら選択をそのノード自身へ付け替える
   （`is_descendant` pure ヘルパを追加、cycle と missing parent を防御）。
   既存ツリーテストに選択アサーションを追加（33 回の監査をすり抜けていた）。
2. **前方互換の二層契約を明記**（probe-fwdcompat 推奨 (b)）:
   `settings.rs` のモジュール doc に「未知トップレベル／future version は
   round-trip、known field 内の未知値は既定値へ migrate して保存」を明記。
   既存テストに「既定化が serialize 表現へ現れる」ことを追加。
   (a) の raw 保持は `merged_map`（app/web）変更＋dirty tracking が必要で
   worktree 外かつリスク大のため却下。

**見送り**: expand 後の子選択復元（`selection_hint` 拡張、設計判断）、
TOCTOU 窓の pidfd 化解消（カーネルレベル変更）、app/web の Corruption 条項
drift（worktree 外）。

- workspace 366 passed / clippy / fmt / rustdoc 成功。

## 2026-11-18（エージェント並列調査 第35弾 → export CPU と ADR の実態）

3 観点（ADR / 用語 / 出力形式）の調査エージェントを並列起動。worktree で実装・検証。

1. **`--export` の初回サンプルで全 CPU 0.0（実バグ）**: real probe の
   初回 sample は前回カウンタが無く全プロセス 0.0 を返す。HTTP 側と
   `agent --once` は warm-up 済みだったが `--export` だけ取り残されていた。
   `run_export` で real のときだけ 1 回捨てサンプル＋250ms 待機を追加
   （demo は初回から非零のためスキップ）。`export_warmup_interval()` を
   定数化し 250ms を回帰テストで固定。app 62 passed。
2. **ADR の実態乖離を訂正**（probe-adrfinal）:
   - ADR-001「更新は隔週」→ dependabot 実態の週次へ
   - ADR-008「エクスポートは MVP 外」→ スナップショット CSV/JSON は実装済み
   - ADR-012「未知フィールドは無視」→ 二層前方互換契約を明記
   - ADR-017 wasm 計測日付 11-18 → 実態の 11-17 へ訂正＋要再測定
3. **README の「阻塞」を「ブロック」へ**（probe-terminology、DESIGN と統一）。
   ステータスバーの「Space 一時停止」を「Space 表示の一時停止」へ明記
   （SIGSTOP 操作との同名衝突を回避）。

**見送り**: 更新側ラベル全体の「表示を一時停止」化（viewmodel の文言テストと
SIGSTOP 側 DOM フックへの波及）、CSV 数式無害化の Unicode 空白拡張
（実経路で sanitize が先に trim し到達不可）、MAC アドレスの redact、
bond0、diverge 解消。

- workspace 367 passed / clippy / fmt / markdownlint 増分ゼロ成功。
  `split_task_refresh_preserves_thread_counts` が並列実行で 2 回 flaky 失敗
  したが、単体実行と全体再実行で成功（変更と無関係）。

## 2026-11-18（エージェント並列調査 第36弾 → --token の redact 漏えい）

2 観点（agent CLI / export I/O）の調査エージェントを並列起動。worktree で実装・検証。

1. **`--token` が redact 済み出力に残る機密漏えい（probe-agentcli B3、
   R2X-A1 として先送り済み）**: tm-platform が argv を `process.command` に
   コピーするため `tm-agent --token <secret> --redact --once` の JSON に
   トークンが残っていた。`redact_snapshot()` ヘルパに解決済み token を渡し、
   `process.command` / `process.name` から `***` へ置換。`--once` は
   `config.token`、HTTP は `SharedState.token` を渡す。回帰テスト追加。
   置換対象は argv 由来の 2 フィールドに限定（他フィールドは設計判断）。

**見送り（次ラウンド候補）**: `--export /dev/stdout` が root／コンテナで
`/dev/stdout` シンボリックリンクを 0600 通常ファイルで置換して破壊する
実バグ（probe-exportio 候補7、Runtime Proof あり。symlink/fifo の方針と
typed error 化 exit 2 分類も同時に判断）、token-file の 0o620 受理
（完全性の設計判断）、IPv6 origin の CORS、--once の serialize 失敗分類。
0600 付与・umask 非依存・原子性・EXIT_PROBE 分類は健全で現状維持。

- agent 79 passed / workspace 368 passed / clippy / fmt 成功。

## 2026-11-18（エージェント並列調査 第37弾 → export の特殊出力先を拒否）

第36弾 probe-exportio が Runtime Proof 付きで指摘した `/dev/stdout` 破壊を修正。
worktree（round31-safe）で実装・検証。

- `write_export()` の冒頭で `symlink_metadata()` を実行し、存在する出力先が
  通常ファイルでない（ディレクトリ／symlink／FIFO／デバイス）場合は拒否。
  存在しない場合のみ一時ファイル生成を許可。さらに `/dev` / `/proc` / `/sys`
  配下も明示拒否（将来作られるデバイスノードも保護）。
- `rename(2)` は symlink 自身を置換するため、root／コンテナで
  `--export csv /dev/stdout` すると `/dev/stdout` が 0600 通常ファイルに
  置換されて破壊されていた（親の `stat` で root:root の symlink を確認）。
- 回帰テスト `write_export_rejects_special_destinations` を追加
  （`/dev/stdout`・`/proc/self/status`・symlink を拒否、symlink 先は無傷）。

**見送り**: typed error 化による exit 2 分類（既存の directory テストが
exit 1 を固定しており契約変更になるため）、`sync_all()` によるクラッシュ
永続性（要件次第）、token-file の 0o620、IPv6 origin、bond0、diverge 解消。
0600／umask 非依存／原子的書き込みは健全で現状維持。

- app 63 passed / workspace 369 passed / clippy / fmt 成功。

## 2026-11-18（エージェント並列調査 第38弾 → 設定 I/O ガードと CLI 記載）

3 観点（設定 I/O / 配布 / CLI 情報）の調査エージェントを並列起動。worktree で実装・検証。

1. **設定ライターへの I/O ガード移植**（probe-settingsio、第37弾の export 版）:
   `app/src/settings.rs` に ①`read_document` 入口の `symlink_metadata` 検査
   （FIFO／デバイス／symlink は corrupt 退避、起動ハングを防止）、②一時ファイル
   `create_new(true)`（symlink 追従による任意ファイル破壊を拒否）、③XDG_CONFIG_HOME
   と HOME の絶対パス検査＋dev/proc/sys 拒否、を追加。回帰テスト 2 件。
   0600／umask 非依存／BACKUP 退避順序は健全で現状維持。
2. **tm-agent の `--log-level` ヘルプに注記**（probe-cliinfo A-1）:
   debug/trace が info と同じ出力であることを明記（app 側には既に同趣の
   注記があり、2 バイナリで契約説明を揃えた）。テスト期待値も更新。
3. **README に `--export csv|json PATH` を記載**（A-3）。

**見送り**: 第三者ライセンス同梱（probe-packaging 5b/5c、DEP-5 複数節と
OFL ライセンス同行は法的判断を含む）、Depends のバージョン下限（設計判断）、
TM_DEBUG_SETTINGS のフルパス（opt-in、要注意）、web localStorage の
エラー混同、gpui-task-manager の --help 早期ショートカット、bond0、diverge。

- app 65 passed / workspace 371 passed / clippy / fmt / README lint 増分ゼロ。
  `split_task_refresh_preserves_thread_counts` が並列実行で flaky 失敗したが
  再実行で成功（変更と無関係）。

## 2026-11-18（エージェント並列調査 第39弾 → 配布物の事故・ライセンス同行）

第38弾 probe-packaging の軽微な確定事項を実装（法的判断を伴う DEP-5 拡充は
見送り、事実誤記とライセンス同行のみ扱う）。worktree で検証。

1. `packaging/linux/README.md` の同梱一覧が `{README,NOTICE,copyright}` の
   3 ファイルだったが、build-deb.sh は `THIRD_PARTY_NOTICES.md` と
   `LICENSE` も install していた。5 ファイルへ訂正。
2. web 成果物に OFL-1.1 の IBM Plex フォント 3 個を `include_bytes!` で
   埋め込みながら `LICENSE-IBM-Plex.txt` が同行していなかった。
   CI の web dist ビルド後に `cp` を追加。

子は `web/Trunk.toml` の `[[copy]]` を提案したが、ローカルの
`trunk 0.22.1 config generate-schema` でトップレベルに `copy` キーが
存在しないことを確認し却下（Runtime Proof を優先）、CI ステップで対応。

**見送り**: DEP-5 複数節への第三者クレート帰属追加と MIT/ISC/BSD 通知文の
同梱（法的判断）、Depends のバージョン下限（設計判断）、bond0、diverge。

- workspace 371 passed / clippy / fmt / yaml / markdownlint 増分ゼロ成功。

## 2026-11-18（エージェント並列調査 第40弾 → flaky と wall clock レート）

2 観点（flaky 根本原因 / 新観点バグ）の調査エージェントを並列起動。worktree で実装・検証。

1. **`split_task_refresh_preserves_thread_counts` の flaky を修正**（probe-flakyroot、
   実測 30 回中 8 回失敗＝27%、シリアル 0/10）: 原因は
   `refresh_processes_specifics(Some, false, ...)` の `remove_dead=false` で
   終了 PID が `tasks=None` のまま残り snapshot へ入ること。第2引数を
   `true` に変更し、さらに Linux の `thread_count()` を `Some(1)` フォールバック
  付きに硬化。修正後 30 回連続成功（失敗 0/30）。
2. **ネットワークレートの wall clock 依存を修正**（probe-newangle 候補1）:
   NTP で時刻が 999ms 戻ると 1000ms の実経过が 1ms と計算され、
   `MIN_ELAPSED` ガードを通過して約 1000 倍のスパイクになる。
   `SystemSnapshot` に `sampled_elapsed: Duration`（serde default）を追加し、
   platform は `Instant` 由来の elapsed_ms、demo は 1 秒を設定。
   viewmodel は wall clock 差ではなくこの値で rate を計算。
   `last_network` から Timestamp を削除。回帰テストで逆行を固定。

**見送り**: web の `reconnect_required` 上書き（候補2、1 行＋クリア条件、
次ラウンド候補）、CPU% の elapsed 取得タイミング（候補3、要判断）、
`--once` serialize 失敗の exit 分類（候補4、発生確率極低）、
永続通知の上書き（設計どおり）、nonce ラップアラウンド（三重条件）。

- workspace 372 passed / clippy / fmt 成功 / flaky 30 回連続成功。

## 2026-11-18（エージェント並列調査 第41弾 → reconnect ガイダンスの維持）

第40弾 probe-newangle 候補2（次ラウンド候補）を実装。worktree で検証。

- `web/src/shell.rs` の `handle_live_result` が、ポーリング失敗のたびに
  `reconnect_required = requires_reconnect(&error)` で上書きしていた。
  認証拒否後にエージェントがまだ応答しない間の汎用エラーで `false` に
  戻り、ユーザーが取るべき唯一の導線（新しい #token= を貼り付け）が消えていた。
- `next_reconnect_required(current, error)` pure ヘルパを追加して sticky 化。
  クリアは成功時（342 行目）と設定変更時（304 行目）のみ。
  `live_error` も認証エラーを汎用エラーで上書きしないよう保護。
- 回帰テスト `reconnect_guidance_is_sticky_across_transient_failures` を追加。

**見送り**: CPU% の elapsed 取得タイミング（要判断）、`--once` serialize
失敗の exit 分類（発生確率極低）、永続通知の上書き（設計どおり）、
bond0、diverge 解消。

- web 76 passed / workspace 372 passed / clippy / fmt 成功。

## 2026-11-18（エージェント並列調査 第42弾 → デモ可視性と elapsed  timing）

2 観点（デモ可視性 / 残タスク）の調査エージェントを並列起動。worktree で実装・検証。

1. **デモデータの可視性を向上**（probe-demovisible、ユーザーが `--demo` の
   16GiB を実機 64GB と誤認）:
   - トップバーの `pill("DEMO", ...)` を `pill("デモデータ", ...)` に変更。
   - メモリカードの値の直前に `※ デモ用の固定値` を `demo_mode` 時のみ表示。
   - ウィンドウタイトルを `--demo` 時は `GPUI Task Manager [デモデータ]` に。
   Web 版は既に persistent notice／ヘッダバッジ／ステータスで明示済みのため変更なし。
2. **CPU% の elapsed 取得タイミングを修正**（probe-finalbacklog 推奨）:
   `sample_native()` が `refresh()` 前に `Instant::now()` を取っていたため、
   カウンタ読取（refresh 中）との間に refresh 時間の揺れが入り CPU% と
   `sampled_elapsed`（ネットワークレート）が数% ずれていた。
   `Instant::now()` と elapsed 計算を refresh 直後に移動。

**見送り**: `--once` serialize 失敗の exit 分類（到達不能）、DEB Depends
バージョン下限（設計判断）、web localStorage 読取エラー（設計どおり）、
TM_DEBUG_SETTINGS フルパス（opt-in）、non-UTF8 argv の panic（型変更が必要）、
bond0、diverge 解消。

- workspace 372 passed / clippy / fmt 成功。

## 2026-11-18（エージェント並列調査 第43弾 → non-UTF8 argv の panic）

probe-argv が将来候補として挙げた `tm-agent` の non-UTF8 argv panic を修正。
worktree で実装・検証。

- `agent/src/main.rs` の `std::env::args()` を `args_os()` に変更。
  非 UTF-8 引数は生バイトをエコーせず固定文言の Usage エラー（exit 2）に変換。
  app 側（args_os + to_string_lossy）と異なり、認証トークンを黙って
  U+FFFD に置換しないよう strict に拒否する（agent/src/args.rs の parse
  シグネチャは変更せず、呼び出し側 1 箇所のみの最小修正）。
- Runtime Proof: `tm-agent $'\xff\xfe' --once` →
  `{"category":"usage","exit_code":2}`、panic せず生バイト非出力。
  通常の `--version` は exit 0 で従来どおり。

**見送り**: `args::parse` の `AsRef<OsStr>` 総称化（将来的に --token-file の
非 UTF-8 パスを lossless に扱いたい場合の拡張）、--version の exit code
を app に揃える（設計判断）、DEB Depends 下限、bond0、diverge。

- agent 79 / workspace 372 passed / clippy / fmt 成功。

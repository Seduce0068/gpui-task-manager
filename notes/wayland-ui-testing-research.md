---
type: Reference
title: Kubuntu KDE Wayland でのキー操作・UI テスト方法の調査
description: GPUI Task Manager の Wayland ネイティブウィンドウに対する入力注入、UI テスト、画面取得の実現手段を出典付きで整理した調査レポート
tags: [wayland, kde, kwin, testing, input-injection, gpui, research]
resource: .
generated: 2026-11-22T00:48:44+09:00
verified: 2026-11-22T00:48:44+09:00
status: verified
---

## 調査範囲と除外範囲

Kubuntu 2026.04 / KDE Plasma（KWin 6.6.7）/ Wayland セッション（`WAYLAND_DISPLAY=wayland-0`、
`XDG_SESSION_TYPE=wayland`、`DISPLAY=:1` は Xwayland）を対象とした。調査手法は
`swarm-research` の制限プロファイル（同時起動上限 3・深度 1・累計制限なし）で、
R2 の 3 観点並列調査、R3 の 2 観点深掘り、R4 の敵対的検証を行った。

除外したもの: Windows / macOS 固有の入力 API、Safari / Firefox のブラウザ自動化、
本番リリース署名・配布（別タスク）。Web 版の CDP 検証は既存実装があるため
背景としてのみ扱い、Wayland ネイティブの手段を主眼とした。

## 確認済み事実

### 1. kdotool はキー／マウス入力に使えない

ローカルの kdotool v0.3.0 は `key` / `mousemove` / `click` が `Unknown command` で失敗する
（`kdotool -n key a` → exit 1）。上流 README の "Won't support" 節が
`You can use ydotool, dotool, etc. for these: Keyboard commands / Mouse commands` と明記しており、
設計上の非対応である。一方で `windowactivate` / `windowraise` / `windowmove` / `windowsize` /
`windowclose` / `kwinscript` は利用可能で、KWin のスクリプト API を D-Bus 経由で使う
KDE Plasma 6 専用ツールである。

- 出典（ローカル）: `kdotool --version`, `kdotool -n key a`, `kdotool --help`（取得 2026-11-22T00:33:59+09:00）
- 出典（一次）: kdotool 0.3.0 README 139-144 行、`https://raw.githubusercontent.com/jinliu/kdotool/master/README.md#L139-L144` （取得 2026-11-22T00:34:30+09:00）

### 2. xdotool / wtype / wsend はこの環境で使えない

xdotool 3.20160805.1 は installed だが X11 の XTEST 拡張に依存し、Wayland ネイティブの
ウィンドウ検索・入力は動作しない（上流 README: `Wayland has some X11 compatibility, but
for the purposes of xdotool, many things do not work correctly`）。wtype は
`zwp_virtual_keyboard_v1` に依存し、KWin が当該 unstable プロトコルを実装していないため
KDE で失敗する（issue #45 が 2026-05 時点で open）。wsend は apt 候補なし。

- 出典（一次）: `https://raw.githubusercontent.com/jordansissel/xdotool/master/README.md#L3,L70` （取得 2026-11-22T00:35:10+09:00）
- 出典（一次/コミュニティ）: `https://raw.githubusercontent.com/atx/wtype/master/man/wtype.1#L15-L18` 、`https://github.com/atx/wtype/issues/45` （取得 2026-11-22T00:35:40+09:00）
- 出典（ローカル）: `command -v ydotool/wtype/wsend` → すべて exit 1、`apt-cache policy wsend` → 候補なし

### 3. KWin の EIS / libei が最も有望な入力経路

本環境の KWin 6.6.7 は libei/EIS プラグイン（`eis.so`）を有効にしており、
D-Bus `org.kde.KWin.EIS.RemoteDesktop.connectToEIS(int capabilities, int& cookie)`
が `QDBusUnixFileDescriptor` を返す。capabilities は 1=keyboard、2=pointer、4=touch。
`libei1` / `libeis1` 1.5.0-3 が installed 済み。

**R2 の「承認ダイアログが出る」という主張は誤り**で、R4 の検証で以下が確定した:
直接 D-Bus 呼び出しは `DbusEisContext` を生成して fd を返すだけであり、
`allowConnection()` は既定で `true`。prompter を起動するコードパスは存在しない
（v6.6.7 には `kwin_eis_prompter` 自体が無く、master の `XWaylandEisContext::connectionRequested()`
が `PROMPTER_BIN` を起動するのは Xwayland ソケット経由の場合のみ）。
したがって承認ダイアログは XDG Desktop Portal の `RemoteDesktop.Start()` や
Xwayland の EIS ソケット経由で初めて現れる。

- 出典（一次、v6.6.7）: `https://raw.githubusercontent.com/KDE/kwin/v6.6.7/src/plugins/eis/eisbackend.cpp:connectToEIS` 、`https://raw.githubusercontent.com/KDE/kwin/v6.6.7/src/plugins/eis/eiscontext.h:32-35` （取得 2026-11-22T00:47:01+09:00）
- 出典（一次、ローカル D-Bus）: `qdbus6 org.kde.KWin /org/kde/KWin/EIS/RemoteDesktop` → `connectToEIS`（取得 2026-11-22T00:34:27+09:00）

### 4. libei は短命 CLI に不向きで、常駐デーモンが必要

libei の README は `not designed for short-lived fire-and-forget-type applications like xdotool` と明記し、
`the protocol does not allow for a libei client to connect, send all requests in bulk and exit` とする。
デバイスネゴシエーション完了までクライアントプロセスの生存が必要なため、
EIS 接続を保持する常駐デーモン＋CLI からの IPC という構成が妥当。

- 出典（一次）: `https://gitlab.freedesktop.org/libinput/libei/-/raw/main/README.md` （取得 2026-11-22T00:39:00+09:00）

### 5. Rust から libei を扱う手段は存在する

Pure Rust の `reis` 0.7.1（2026-07-30 published、MIT、downloads 266469、yanked=false）が
crates.io に実在し、`#![forbid(unsafe_code)]` を宣言しているため、本プロジェクトの
unsafe 禁止規約と互換。zbus の `OwnedFd` は 2 段で `UnixStream` に変換可能
（`UnixStream::from(std::os::fd::OwnedFd::from(zbus_fd))`）。
車輪の再発明を避ける候補として `wdotool` 0.5.3（xdotool 互換の Wayland 自動化 CLI＋ライブラリ、
libei/ポータル経由、KWin Scripting backend、`prime` による常駐化）と `eitype` 0.2.2 も存在する。

- 出典（一次）: `https://crates.io/api/v1/crates/reis/0.7.1` 、`https://docs.rs/crate/reis/0.7.1/source/src/lib.rs:19` 、`https://docs.rs/zbus/6.5.0/zbus/struct.OwnedFd.html` （取得 2026-11-22T00:47:01+09:00）
- 出典（コミュニティ）: `https://raw.githubusercontent.com/cushycush/wdotool/main/README.md` 、`https://raw.githubusercontent.com/Adam-D-Lewis/eitype/main/README.md` （取得 2026-11-22T00:40:00+09:00）

### 6. 画面取得は KWin ScreenShot2 または spectacle

Wayland ネイティブの画面取得は `org.kde.KWin.ScreenShot2` の
`CaptureActiveScreen` / `CaptureActiveWindow` / `CaptureScreen` 等を使う。
`spectacle -b -n -o <path>` はそのユーザー向け CLI として機能し、本リポジトリでも
証跡取得に利用済み。`import -window root` は Xwayland ルートしか捕捉できず失敗する
（実測 exit 1）。

- 出典（ローカル）: `qdbus6 org.kde.KWin /org/kde/KWin/ScreenShot2`, `spectacle --help`, `import -window root png:-`（取得 2026-11-22T00:34:27〜00:35:15+09:00）

### 7. GPUI のテスト基盤は Linux の headless で使える（ただし要 test-support）

gpui-pre 0.3.12 は `test-support` feature で `TestAppContext` と `VisualTestContext` を公開する。
**R2 の「VisualTestContext は macOS 限定」は誤認**で、macOS 限定なのは別型
`VisualTestAppContext`（実 Mac レンダリング用）であり、`VisualTestContext` は
`#[cfg(any(test, feature = "test-support"))]` で OS 非依存に公開される（R4 で確定）。
`TestAppContext` は `NoopTextSystem` の `TestPlatform` を使い、実ディスプレイを要求しない。
`simulate_keystrokes("ctrl-f")` は `Keystroke::parse_with_os` → `dispatch_keystroke` →
`run_until_parked()` の順で実イベント経路を通る。`debug_bounds(selector)` は対象要素に
`.debug_selector(|| "...".into())` を付け、debug build + test-support + 描画済みであることが必要。

- 出典（一次）: gpui-pre-0.3.12 `examples/testing.rs:9,233-234,266-268`、`src/app.rs:38-41`、`src/app/test_context.rs:622-633,865-874,1026-1033`（取得 2026-11-22T00:39:12+09:00）
- 出典（一次、R4 検証）: `src/app/visual_test_context.rs:12-16,20-21`（取得 2026-11-22T00:45:06+09:00）

本リポジトリは test-support を有効化しておらず、CI の `cargo test` も
`-p tm-core -p tm-viewmodel -p tm-platform` のみで app クレートのテストを走らせていない。
PoC を CI ゲートに乗せるには `app/Cargo.toml` への dev-dependency
（`gpui = { package = "gpui-pre", version = "=0.3.12", features = ["test-support"] }`）
と CI のテスト対象拡張が必要（`proptest` / `postage` / `futures-concurrency` が Cargo.lock に新規追加される）。

### 8. ydotool は root 権限が必要でこの環境では即利用不可

ydotool は uinput で仮想入力デバイスを作るため Wayland ネイティブにも効くが、
v1.0.0 以降 is mandatory な `ydotoold` が `/dev/uinput` への書き込み権限を要求する。
本環境の `/dev/uinput` は `crw------- root root` で、ユーザー `testuser` は `input` グループに
属していない。apt 候補は 1.0.4-3。導入には権限昇格／udev 設定が必要。

- 出典（一次）: `https://raw.githubusercontent.com/ReimuNotMoe/ydotool/master/README.md#L77,L83,L91` （取得 2026-11-22T00:34:45+09:00）
- 出典（ローカル）: `ls -l /dev/uinput`, `id`, `apt-cache policy ydotool`（取得 2026-11-22T00:34:44+09:00）

## 推奨される次の一手（優先順）

1. **即効性: wdotool の試用** — `cargo install wdotool` または apt で導入し、
   `--backend libei` を明示して初回のみ承認、restore_token を `$XDG_STATE_HOME/wdotool/portal.token`
   に保存する。kdotool でフォーカス確保 → wdotool でキー送信の組み合わせ。
   ただし KDE backend は実機未検証と明記されているため、libei backend を使う。
2. **本格実装: KWin 直接 D-Bus + reis 常駐デーモン** — `connectToEIS(1|2)` で fd を取得し、
   `UnixStream::from(OwnedFd::from(zbus_fd))` を `reis::ei::Context::new` に渡す。
   デーモンは接続を維持し、CLI は Unix socket 経由でキー／ポインタ要求を送る。
   終了時に `disconnect(cookie)` を必ず呼ぶ。KWin の autotest
   `autotests/integration/libei_test.cpp` がイベント列の実装例。
3. **UI テストの自動化: gpui-pre test-support** — app に dev-dependency を追加し、
   `#[gpui::test]` + `VisualTestContext::simulate_keystrokes` でキー経路をテスト。
   `debug_selector` / `debug_bounds` で行やダイアログのジオメトリを検証。
   CI の `cargo test` 対象に `-p gpui-task-manager` を追加。
4. **画面証跡: spectacle / ScreenShot2** — `spectacle -a -b -n -o <file>` で
   アクティブウィンドウ撮影。DOM proof や状態フックを正とし、画像は目視証跡として扱う。

## 未確認事項

- wdotool / eitype の本環境（Kubuntu 2026.04 + KWin 6.6.7）での実機動作は未検証。
- `connectToEIS` の fd を reis に渡す Rust 側の完全な実装例は、KWin autotest（C++）と
  kde-mcp の prober（unsafe 版）のみで、safe Rust の完成例は見つからなかった。
- KWin master の `XWaylandEisContext` が v6.6.7 に存在しないなど、バージョンによる
  構造差がある。参照する際はブランチ／タグを明示すること。
- AT-SPI 経由で外部から GPUI ウィジェットを操作する手段は、リポジトリ内に
  コード／テストが見当たらず未確認。

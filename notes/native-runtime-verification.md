---
type: Report
title: "ネイティブ GPUI アプリ実動検証"
description: "Wayland セッションで GPUI デモアプリを実起動し、ウィンドウ生成を Runtime Proof として記録した検証結果"
tags:
  - gpui
  - wayland
  - runtime-proof
  - verification
generated:
  by: "f2-native-live"
  at: "2026-11-19T19:55:50+09:00"
verified:
  by: "f2-native-live"
  at: "2026-11-19T19:55:50+09:00"
status: "approved"
---

## 結論

ネイティブ GPUI アプリは Wayland セッションで実起動し、ウィンドウが生成された。
コンポジタ (KWin) のウィンドウ一覧にデモデータ版のタイトルが出現し、プロセス終了とともに消えた。
stderr は 0 行で、panic / `RefCell` error / 内部パス露出は認められなかった。
停止は SIGTERM で実施し、起動したプロセスは残存していない。

## 環境

検証時刻は `date` コマンドで確認した値 (RFC 3339、オフセット付き) に基づく。

| 項目 | 値 |
| --- | --- |
| 検証時刻 | 2026-11-19T19:55:50+09:00 |
| セッション | `XDG_SESSION_TYPE=wayland` |
| Wayland | `WAYLAND_DISPLAY=wayland-0` |
| Xwayland | `DISPLAY=:1` (存在するが本アプリのウィンドウはここに出ていない) |
| ランタイムディレクトリ | `/run/user/1000` |
| コンポジタ | KWin (`kwin_wayland`) |
| モニタ / スケール | 単一モニタ 2560x1440 / scale 1 は別タスクの確認結果を背景情報として引用。本タスクでは再検証していない |

## 確認手順と結果

### 1. ビルド

```bash
cargo build --locked -p gpui-task-manager
```

終了コード 0。`Finished dev profile` (16.64 秒)。警告は workspace の未使用フィールド
(`authors` / `description`) のみで、ビルド失敗やコンパイルエラーはなかった。

### 2. 起動

```bash
nohup ./target/debug/gpui-task-manager --demo --redact \
  > /tmp/f2-native-app.stdout 2> /tmp/f2-native-app.stderr < /dev/null &
```

1 回目 PID は 3117979、2 回目 PID は 3133288 (2026-11-19T19:54:23+09:00 起動)。
外部データは `--demo` により不要で、`--redact` により表示は伏せ字である。

### 3. ウィンドウ確認

最大 15 秒のポーリングでは、プロセスは全期間 alive、stderr は常に 0 行だった。
`xwininfo -root -tree` に GPUI のウィンドウは表示されず、
`xprop -root` の `_NET_CLIENT_LIST` は Bitwarden 由来の 2 ウィンドウのみだった。
`kdotool` はこの環境にインストールされていない (`command not found`)。

Wayland 側の証拠を `/proc/<pid>/fd` とコンポジタで確認した。

- fd 9 (`socket:[71182063]`) のピア inode 71186559 は `/run/user/1000/wayland-0` (KWin 側ソケット) だった。
- fd 19 は `/memfd:wayland-cursor-rs (deleted)` で、Wayland クライアントのカーソル用メモリである。
- `/proc/<pid>/maps` に `libwayland-client.so.0.24.0` がマップされていた。
- `/dev/dri/renderD128` を 4 回、NVIDIA デバイスと `/dev/udmabuf` をオープンし、NVIDIA の GLCache ファイルが生成されていた (GPU レンダリング路上)。
- fd 22 は Xwayland ソケットに接続されていたが、X11 ツリーにアプリのウィンドウは存在しない。

決定的な証拠として、KWin の `org.kde.krunner1.Match("GPUI")` を実行したところ、
起動中は `GPUI Task Manager [デモデータ]` (`gpui-task-manager`) が返った。
SIGTERM 後は同エントリが消え、別セッション由来の非デモタイトル `GPUI Task Manager` のみが残った。
すなわちコンポジタが本プロセスのウィンドウを生成・破棄している。

なお、ソース上 `gpui_kit::open_window(...).expect("failed to open the task manager window")`
が失敗すれば panic して終了するため、プロセスが 15 秒以上生存し stderr が空であることも
ウィンドウ生成成功と整合する。

### 4. stderr 確認

両起動とも停止前後の stderr / stdout は 0 バイト・0 行だった。
`panic`、`RefCell` 関連エラー、内部ディレクトリパスの露出は認められなかった。

### 5. 停止と cleanup

```bash
kill -TERM <pid>
```

両回とも SIGTERM 後 2 秒でプロセスが消え、`ps` で PID を確認できない状態になった。
起動した 2 つの PID は残存していない。
別バイナリパスの `--redact` プロセスが 1 件存在したが、本タスクが起動したものではないため停止していない。

## 未検証事項

HiDPI スケーリング (scale 変更・2 モニタ構成) とモニタ間ドラッグ移動は本タスクでは未検証である。
本検証は単一モニタ・scale 1 のセッションにおけるウィンドウ生成とプロセス健全性に限る。

## 検証コマンド

```bash
markdownlint notes/native-runtime-verification.md
bash scripts/check-public-docs.sh
```

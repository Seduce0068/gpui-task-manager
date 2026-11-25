---
type: Reference
title: GPUI システムタスクマネージャー: 既存ソリューション調査（2026-11-14）
description: GPUI ベースの Ubuntu 向けシステムタスクマネージャーを新規設計するための既存ソリューション・GPUI 先行事例・ブラウザ監視プロトコルの一次情報調査。比較表、採用/不採用判断、Runtime Proof を含む。
tags:
  - gpui
  - system-monitor
  - task-manager
  - survey
  - linux
  - wasm
generated: 2026-11-14T20:18:42+09:00
verified: 2026-11-14T20:18:42+09:00
status: draft
---

## 0. 結論（設計チームへの推奨）

1. **収集層は `sysinfo = "=0.40.3"` を中核**に据える。Linux 実測で per-core CPU / メモリ / スワップ / ネットワーク / ディスク / プロセス（cmdline・user・status・start_time・kill）がすべて取得できた。full tick の中央値は **約 31ms**（約 3.5 千 プロセス時）。
2. **UI は `gpui-kit = "=0.9.1"`**（既存調査どおり）。`AreaChart` + `DataTable`（仮想スクロール）が gpui-kit の `examples/system_monitor` で実証済み。ライセンス Apache-2.0 で再利用可能。
3. **既存モニタのコードは流用しない**（GPL-3.0 / AGPL-3.0 が中心）。Mission Center（機能スコープ）、Argus（GPUI タスクマネージャのアーキテクチャ）、btop/bottom（情報設計）を**設計参考**に限定する。
4. **ブラウザでは /proc を読めない**。`sysinfo` は wasm32 にコンパイルは通るが `System::new()` が `Err(Unsupported)` を返す（実測）。ブラウザ対応は **(a) ローカル agent の HTTP/WS 経由**を主、(b) 記録 JSONL の再生をデモ用に併用、の二段構えにする。
5. **プロトコルは独自スキーマ `gpui-task-manager/snapshot/v1` を主**とし、Glances REST API v4 は**取り込み専用の任意互換**に留める。Glances 形式を主スキーマにすると psutil 固有の巨大 JSON（`/all`）を引き継ぐことになる。
6. **プロセス表は必ずページング/差分**にする。約 3.5 千 プロセス全件の JSON は **約 2MB** だった。1 秒間隔で全件送る設計はブラウザでは破綻する。
7. **プロセス操作は `(pid, start_time)` の組で同一性検証してから**シグナル送出（PID 再利用対策）。kill は `sysinfo::Process::kill_with` で実証済み、`rustix 1.1.6` の `kill_process` を代替に使える。
8. **収集は UI スレッドの外**（専用スレッド + チャネル）に置く。gpui-kit の example は UI スレッドで `refresh_all()` しており、31ms/tick のスパイクが描画に乗るためそのままは採用しない。
9. **サンプリング間隔は既定 1 秒**、非フォーカス時 2〜5 秒、プロセス詳細はオンデマンド。CPU 使用率は 200ms 未満の間隔では正しく出ない（`MINIMUM_CPU_UPDATE_INTERVAL = 200ms`）。
10. **ライセンス方針**: 本体は Apache-2.0 系（gpui-kit）で構成し、GPL 系既存アプリは仕様参考のみ。Glances（LGPL-3.0）はプロトコル仕様の参照に留め、コードをリンクしない。

## 1. 調査の前提と方法

- 調査日: **2026-11-14**（Asia/Tokyo）。数値・日付はすべてこの日に取得。
- 環境: Kubuntu 2026.04 / Wayland / 汎用デスクトップCPU / 十分な RAM / rustc 1.99.1 / cargo 1.99.1。
- 一次情報: GitHub REST API（`gh api`、認証済み）、GitLab REST API（gitlab.gnome.org / invent.kde.org / gitlab.com）、crates.io API（`curl -A 'research-agent/1.0'`）、rustsec.org、各リポジトリの README / Cargo.toml / 公式ドキュメント、crate tarball。
- 表記: **【検証済】** = 一次情報を取得して確認。**【推測】** = 取得情報からの推定。**【未確認】** = 未検証。
- 数値の意味: ★ = 取得時点の stars。contributors は GitHub API の `per_page=1` + Link ヘッダの最終ページ数（GitLab は contributors API が 100 件上限のため「100以上」）。
- 既存調査（外部プロジェクトの調査ノート）は手掛かりとして参照。**AGPL/GPL コードのコピーは行っていない**。

## 2. 比較表（11 候補）

### 2.1 基本属性と活発度

| 候補 | 言語 / UI スタック | ライセンス | プラットフォーム | Web / wasm | ★ | 最終コミット | 最新リリース | contrib. |
|---|---|---|---|---:|---|---:|---|---:|---:|
| btop | C++23 / 端末（ncurses 系） | Apache-2.0 | Linux, macOS, BSD 系 | なし | 35,392 | 2026-11-11 | v1.4.8 (2026-10-25) | 191 |
| bottom (btm) | Rust / ratatui + crossterm | MIT | Linux, macOS, Windows | なし | 14,191 | 2026-11-14 | 0.15.0 (2026-11-01) | 98 |
| zenith | Rust / ratatui + crossterm | MIT | Linux, macOS | なし | 3,070 | 2026-10-01 | 0.15.1 (2026-09-02) | 23 |
| Mission Center | Rust / GTK4 + libadwaita | GPL-3.0 | Linux | なし | 673（GitLab） | 2026-11-03 | v1.2.0 (2026-07-26) | 100以上 |
| Resources | Rust / GTK4 + libadwaita | GPL-3.0 | Linux | なし | 1,276 | 2026-05-12 | v1.10.2 (2026-03-06) | 未取得 |
| GNOME System Monitor | C++ / gtkmm4 + libadwaita + libgtop2 | GPL-2.0 | Linux | なし | 92（GitHub ミラー） | 2026-11-11 | 51.1 (2026-10-06) | 100以上 |
| Plasma System Monitor | QML + C++ / Qt6・Kirigami・ksystemstats | GPL-2.0-only OR GPL-3.0-only OR KDE-Accepted-GPL（混在） | Linux | なし | 80（GitHub ミラー） | 2026-11-14 | 単独リリースなし（Plasma 同梱、版 未確認） | 未取得 |
| htop | C / ncurses | GPL-2.0 | Linux, macOS, BSD 系 ほか | なし | 8,458 | 2026-11-13 | 3.5.3 (2026-08-16) | 282 |
| Glances | Python 3 / psutil・Jinja2 Web UI | LGPL-3.0 | Linux, macOS, Windows, BSD 系 | **あり**（Web UI + REST v4） | 33,951 | 2026-11-14 | v4.5.9 (2026-11-07) | 287 |
| netdata | C（agent）+ Go（go.d）+ Web dashboard | GPL-3.0+（agent）/ UI は NCUL1 | Linux, macOS, Windows ほか | **あり**（Web dashboard） | 81,408 | 2026-11-14 | v2.12.3 (2026-11-04) | 701 |
| Cockpit | C / Python / JavaScript（Web 管理 UI） | LGPL-2.1（+GPL-3.0, BSD-3, MIT 等の混在） | Linux（サーバ） | **あり**（ブラウザが主 UI） | 15,384 | 2026-11-13 | 372 (2026-11-10) | 264 |

補足:

- Resources の GitHub リポジトリは **archived = true**（2026-05-12）。README に「GNOME Incubator（gitlab.gnome.org/GNOME/Incubator/resources）へ移管」と明記【検証済】。
- Mission Center の正式ホストは GitLab（gitlab.com/mission-center-devs/mission-center）。GitHub に公式ミラーは確認できず（未確認）。
- GNOME System Monitor / Plasma System Monitor の GitHub リポジトリはミラーのため stars は参考値。活発度は GitLab 側の最終コミットで判断した。
- netdata のライセンスは README の表で「Netdata Agent = GPL v3+ / Netdata UI = NCUL1」と明記【検証済】。

### 2.2 機能マトリクス（README / man page / 公式 docs 由来）

| 候補 | per-core CPU | メモリ/スワップ | ネットワーク | ディスク | プロセス操作 | ツリー表示 | GPU / その他 |
|---|---|---|---|---|---|---|---|
| btop | ○ | ○ | ○ | ○（IO + 空き容量） | ○（任意シグナル） | ○ | バッテリー、プリセット |
| bottom | ○（平均 + per-core グラフ） | ○ | ○ | ○（IO + 容量） | ○（kill シグナル） | ○ | 温度、バッテリー |
| zenith | ○（グラフ） | ○ | ○ | ○（IO + 空き容量） | ○（シグナル + nice 変更） | 一覧 + フィルタ（ツリーは未確認） | NVIDIA GPU、ズーム/履歴スクロール、バッテリー |
| Mission Center | ○（per-thread） | ○（内訳表示） | ○（IF 情報含む） | ○ | ○（プロセス/アプリ終了） | 未確認（アプリ単位集約はあり） | GPU（NVTOP）、ファン、バッテリー、systemd/OpenRC |
| Resources | ○ | ○ | ○ | ○ | ○（アプリ/プロセス終了） | 未確認 | GPU（nvml-wrapper）、NPU、バッテリー |
| GNOME System Monitor | ○ | ○ | ○（履歴グラフ） | ○（ファイルシステム） | ○（kill / renice） | ○ | プロセス依存ツリー、アイコン |
| Plasma System Monitor | ○（センサー） | ○ | ○ | ○ | ○（プロセスページ） | 未確認 | カスタムページ、ksystemstats センサー |
| htop | ○（メーター） | ○ | ✗ | ○（IO） | ○（シグナル、nice、affinity） | ○ | ユーザー/スレッド表示 |
| Glances | ○ | ○ | ○ | ○ | 未確認（監視特化） | ✗ | sensors/GPU/containers/アラート、REST/Web |
| netdata | ○ | ○ | ○ | ○ | ✗（監視特化） | ✗ | 収集プラグイン多数、アラート、ML |
| Cockpit | ○ | ○ | ○ | ○ | 未確認（systemd 管理が主） | ✗ | サービス/ストレージ/ログ/コンテナ管理 |

○ = README・man page・公式 docs に記載あり。✗ = 記載なし/対象外。未確認 = 一次情報で確認できなかった。

### 2.3 採用判断（本プロジェクト = GPUI デスクトップ + wasm デモ）

| 候補 | 判断 | 理由（設計への使い方） |
|---|---|---|
| btop | **不採用（参考）** | C++23/端末実装で依存利用不可。ただし「グラフ + プロセス表 + ツリー + シグナルメニュー」を 1 画面に収める情報設計は参考。Apache-2.0 のため移植は可能だが C++→Rust のコストが高い。 |
| bottom | **不採用（参考 + 収集層の裏付け）** | TUI のため部品流用不可。**`sysinfo = "=0.40.3"` を採用している**（Cargo.toml 実査）ことが、我々の収集層選定の外部裏付けになる。kill シグナル/ツリー/温度/バッテリーの機能セットも参考。 |
| zenith | **不採用（UX 参考）** | TUI。**ズーム可能な履歴チャートとスクロールバック**、per-process ディスク使用量は採用したい UX。更新頻度は 12 か月で 46 コミットと低め、heim の fork 依存も保守上の懸念。 |
| Mission Center | **不採用（機能スコープの最有力参照）** | GPL-3.0 のためコード流用不可。機能スコープ（per-thread CPU、メモリ内訳、IF 情報、GPU、サービス、ミニファイ表示）は我々の目標設定に最も近い。**収集を別プロセス（magpie, nng-c IPC）に分離**している点は、ブラウザ向けローカル agent 設計の先行例。 |
| Resources | **不採用（アプリ単位 UX 参考）** | リポジトリ archived（GNOME Incubator へ移管）。GPL-3.0。**「プロセス」ではなく「アプリ単位で集約して見せる」**UX と、`nix`/`nvml-wrapper`/`plotters` の依存構成が参考。 |
| GNOME System Monitor | **不採用（最小機能セットの基準）** | GPL-2.0 / C++。プロセス依存ツリー・アイコン・renice/kill という「標準機能」の基準として参照。 |
| Plasma System Monitor | **不採用（センサー抽象化の参考）** | QML + ksystemstats。**「センサーを抽象化してカスタムページに配置」**というデータモデルは、将来のダッシュボード機能の参考。KDE 依存のため流用不可。 |
| htop | **不採用（プロセス操作 UX 参考）** | C/ncurses。ツリー操作・シグナル選択メニュー・nice/affinity の操作体系を参考。 |
| Glances | **条件付き採用（プロトコル互換候補）** | LGPL-3.0。**REST API v4 が公開仕様（OpenAPI あり）**で、外部バックエンド/取り込み元として互換を検討する価値がある。ただし主スキーマにはしない（§4.6）。コードのリンクはしない。 |
| netdata | **不採用（将来の外部バックエンド候補）** | 重量級（agent + 独自エコシステム）。GPL-3.0+/NCUL1。ローカル agent を自作する代わりに netdata を同梱するのは過剰。 |
| Cockpit | **不採用（アーキテクチャ参考）** | LGPL-2.1。サーバ管理ツールで権限モデルが違う。**ブラウザ UI ↔ ローカル agent（WebSocket の JSON チャネル、特権は superuser チャネルで分離）**という構成は、本プロジェクトの wasm 対応設計の先行例。 |

**総括**: 既存の Linux モニタは「端末 TUI（btop/bottom/zenith/htop）」「GTK/Qt デスクトップ（Mission Center/Resources/GNOME/Plasma）」「Web 管理（Glances/netdata/Cockpit）」の 3 系統。GPUI ネイティブ + wasm という組み合わせの既存ソリューションは **Argus（Windows 向け、AGPL）以外に確認できなかった**（§5.2）。したがって「新規に作る」判断は妥当で、既存からは**機能仕様とアーキテクチャのみ**を取り込む。

## 3. 各候補の詳細と採用判断

### 3.1 btop（aristocratos/btop）

- 言語/UI: C++23、端末 UI（README のバッジで c++23 を明記）【検証済】。
- 機能: マウス対応、プロセス詳細、フィルタ、ソート、**ツリー表示、任意シグナル送出**、プロセス一覧の一時停止、ネットワーク自動スケールグラフ、ディスク IO、バッテリーメーター、プリセット【検証済: README Features】。
- 活発度: ★35,392 / 最終コミット 2026-11-11 / v1.4.8（2026-10-25）/ contributors 191 / 直近 12 か月のコミット 100 以上【検証済】。
- ライセンス: Apache-2.0【検証済: GitHub API】。
- プラットフォーム: Linux / macOS / FreeBSD / NetBSD / OpenBSD（README バッジ）。
- Web/wasm: なし。
- 判断: **不採用（参考）**。C++/ncurses の別プロセスであり、Rust アプリに組み込む形での再利用はできない。情報レイアウト（上部グラフ、下部プロセス表、右側メニュー）を UX 参考にする。

### 3.2 bottom / btm（ClementTsang/bottom）

- 言語/UI: Rust、ratatui 0.30 + crossterm 0.29【検証済: Cargo.toml】。
- 機能: CPU（平均 + per-core）・RAM/swap・ネットワーク・温度・ディスク IO の時系列グラフ（ズーム可）、ディスク容量、温度センサー表、バッテリー、**プロセス表（ソート/検索/kill シグナル/ツリーモード）**、設定ファイル/テーマ、基本モード【検証済: README Features】。
- 活発度: ★14,191 / 最終コミット 2026-11-14 / 0.15.0（2026-11-01）/ contributors 98【検証済】。
- ライセンス: MIT。
- プラットフォーム: Linux / macOS / Windows。
- Web/wasm: なし。
- 判断: **不採用（参考 + 収集層の裏付け）**。Cargo.toml で `sysinfo = { version = "=0.40.3", ... features = ["component","disk","network","system","user"] }` を確認【検証済】。同じ sysinfo を採用する TUI が活発に保守されていることは、収集層の安定性の傍証になる。

### 3.3 zenith（bvaisvil/zenith）

- 言語/UI: Rust、ratatui 0.29 + crossterm 0.28、`sysinfo = "0.37"`、heim の fork（`bvaisvil/heim` branch `zenith_changes`）【検証済: Cargo.toml】。
- 機能: CPU/メモリ/ネットワーク/ディスクのグラフ、ディスク空き容量チャート、NIC IP、CPU 周波数、**ズーム可能な履歴スクロールバック**、フィルタ可能なプロセス表（per-process ディスク使用量）、**シグナル送出と nice 変更**、バッテリー、NVIDIA GPU（`--features nvidia`）、root 時の delay accounting【検証済: README Features】。
- 活発度: ★3,070 / 最終コミット 2026-10-01 / 0.15.1（2026-09-02）/ contributors 23 / 直近 12 か月 46 コミット【検証済】。
- ライセンス: MIT。
- プラットフォーム: Linux / macOS。
- Web/wasm: なし。
- 判断: **不採用（UX 参考）**。履歴のスクロールバックと per-process ディスク使用量は採用したい。heim の fork 依存はメンテ上のリスク要因であり、我々は sysinfo + procfs で代替する。

### 3.4 Mission Center（mission-center-devs/mission-center, GitLab）

- 言語/UI: Rust + GTK4/libadwaita。Cargo.toml に `adw`（libadwaita）、`nng-c` を確認【検証済】。`subprojects/magpie` という収集用サブプロジェクトを持ち、`magpie_client` から IPC する構成【検証済: リポジトリツリー, BUILD.md】。
- 機能: per-thread CPU、システム/プロセス/スレッド/ハンドル数、稼働時間、クロック（base/current）、キャッシュサイズ、RAM/swap とメモリ内訳、ディスク、ネットワーク（IF 名・接続種別・無線速度・MAC/IP）、GPU（NVTOP、エンコーダ/デコーダ/電力）、ファン、バッテリー、**アプリ/プロセス単位のリソース内訳**、systemd/OpenRC サービス、ミニファイ表示、HW アクセラレーション描画【検証済: README Features】。
- 制約（README 明記）: per-process ネットワーク監視は nethogs の手動セットアップが必要、Intel GPU は Broadwell 以降のみ、libadwaita のためシステムテーマに追従しない【検証済】。
- 活発度: GitLab ★673 / 最終コミット 2026-11-03 / v1.2.0（2026-07-26）/ contributors 100 以上【検証済】。
- ライセンス: GPL-3.0（COPYING で確認）【検証済】。
- プラットフォーム: Linux。
- Web/wasm: なし。
- 判断: **不採用（機能スコープの最有力参照）**。GPL-3.0 のためコードは流用できない。ただし「収集を別プロセスに分離して IPC する」構成は、ブラウザ向けローカル agent の設計にそのまま通じる。

### 3.5 Resources（nokyan/resources）

- 言語/UI: Rust + GTK4/libadwaita。Cargo.toml に `gtk4`、`libadwaita`、`nix`（signal/sched）、`nvml-wrapper`、`plotters`、`neli-wifi` を確認【検証済】。
- 機能: CPU/メモリ/GPU/NPU/ネットワーク IF/ブロックデバイス/バッテリーの表示、アプリ一覧、プロセス一覧、**グラフィカルアプリとプロセスの終了**【検証済: README】。
- 活発度: ★1,276 / 最終コミット 2026-05-12 / **archived（2026-05-12）** / v1.10.2（2026-03-06）【検証済: GitHub API】。
- ライセンス: GPL-3.0（LICENSE で確認）【検証済】。
- プラットフォーム: Linux（Flatpak が公式配布）。
- Web/wasm: なし。
- 判断: **不採用（アプリ単位 UX 参考）**。リポジトリがアーカイブされ GNOME Incubator へ移管済み。プロセスを「アプリ」に集約する UX と、`nvml-wrapper` による GPU 取得は参考。

### 3.6 GNOME System Monitor

- 言語/UI: C++（gtkmm4 + libadwaita + libgtop2）【検証済: README のビルド依存】。
- 機能: プロセス依存の**ツリー表示**、プロセスアイコン、非表示プロセス、CPU/メモリ/スワップの履歴グラフ、**kill / renice（root 権限要）**【検証済: README】。
- 活発度: GitLab 最終コミット 2026-11-11 / リリース 51.1（2026-10-06）/ contributors 100 以上【検証済】。
- ライセンス: GPL-2.0（COPYING）【検証済】。
- プラットフォーム: Linux。
- Web/wasm: なし。
- 判断: **不採用（最小機能セットの基準）**。「タスクマネージャとして最低限これだけは出す」基準として参照。

### 3.7 Plasma System Monitor（KDE）

- 言語/UI: QML + C++（Qt6、Kirigami、`ksystemstats` デーモン、faces システム）【検証済: README】。ライセンスヘッダは `GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL`（main.cpp）、リポジトリの LICENSES には BSD-3-Clause / CC0-1.0 / LGPL-2.0-or-later も含む【検証済】。
- 機能: センサー情報とプロセス情報の監視、**ページの自由なカスタマイズ**（faces システム）【検証済: README】。
- 活発度: invent.kde.org 最終コミット 2026-11-14。単独リリースはなく Plasma リリースに同梱（正確な版は未確認）。
- プラットフォーム: Linux（KDE Plasma）。
- Web/wasm: なし。
- 判断: **不採用（センサー抽象化の参考）**。KDE 依存のため流用不可。データモデル（センサー → カスタムページ）は将来のダッシュボード設計に参考。

### 3.8 htop（htop-dev/htop）

- 言語/UI: C、ncurses。
- 機能: **ツリービュー**、任意シグナルの送出（タグ付きプロセスへの一括送出含む）、nice/autogroup 優先度変更、**CPU affinity 設定**、per-core メーター、ディスク IO【検証済: htop.1.in】。
- 活発度: ★8,458 / 最終コミット 2026-11-13 / 3.5.3（2026-08-16）/ contributors 282【検証済】。
- ライセンス: GPL-2.0。
- プラットフォーム: Linux / macOS / BSD 系 ほか。
- Web/wasm: なし。
- 判断: **不採用（プロセス操作 UX 参考）**。シグナル選択メニュー、ツリーの折りたたみ、affinity 設定の操作体系を参考。

### 3.9 Glances（nicolargo/glances）

- 言語/UI: Python 3（psutil、Jinja2 Web UI）。依存は `psutil>=5.6.7`, `requests`, `jinja2` ほか【検証済: pyproject.toml】。
- 機能: CPU（per-core 含む）/メモリ/スワップ/ディスク IO/FS/ネットワーク/センサー/GPU/NPU/コンテナ/アラート等のプラグイン、**REST API v4**、Web UI、クライアント/サーバモード、MCP サーバ【検証済: README 目次と API docs】。
- 活発度: ★33,951 / 最終コミット 2026-11-14 / v4.5.9（2026-11-07）/ contributors 287【検証済】。
- ライセンス: LGPL-3.0（COPYING）【検証済】。
- プラットフォーム: Linux / macOS / Windows / BSD 系。
- Web/wasm: **あり**（Web UI + REST API。ブラウザから利用可能）。
- 判断: **条件付き採用（プロトコル互換候補）**。API 仕様は §4.2。LGPL-3.0 のためコードをリンクせず、必要なら「Glances サーバを外部データソースとして読む」形にする。

### 3.10 netdata

- 言語/UI: C の agent + Go の go.d プラグイン + Web dashboard【推測: リポジトリ言語は Go、README のアーキテクチャ記述から】。
- 機能: 毎秒のメトリクス収集、アラート、ML、可視化、親子ストリーミング、多数のコレクタ【検証済: README】。
- 活発度: ★81,408 / 最終コミット 2026-11-14 / v2.12.3（2026-11-04）/ contributors 701【検証済】。
- ライセンス: agent = GPL-3.0+、UI = NCUL1【検証済: README の表】。
- プラットフォーム: Linux / macOS / Windows / コンテナ / K8s。
- Web/wasm: **あり**（Web dashboard）。
- 判断: **不採用（将来の外部バックエンド候補）**。重量級で、単体デスクトップアプリに同梱する対象ではない。

### 3.11 Cockpit（cockpit-project/cockpit）

- 言語/UI: C（cockpit-ws / cockpit-bridge）+ Python（パッケージ）+ JavaScript（Web UI）【推測: リポジトリ構成と README から。正確な比率は未確認】。
- 機能: ブラウザからのシステム管理（サービス、ストレージ、ネットワーク、ログ、コンテナ等）【検証済: README】。
- 活発度: ★15,384 / 最終コミット 2026-11-13 / リリース 372（2026-11-10）/ contributors 264【検証済】。
- ライセンス: LGPL-2.1（LICENSES ディレクトリに LGPL-2.1.txt ほか）【検証済】。
- プラットフォーム: Linux（サーバ）。
- Web/wasm: **あり**（ブラウザが主 UI）。
- 判断: **不採用（アーキテクチャ参考）**。ブラウザ ↔ ローカル agent の通信は「WebSocket 上の JSON チャネル」で、`superuser` オプションで特権操作を分離する【検証済: cockpit-channels.adoc】。この権限分離モデルは wasm 版の設計に参考になる。

## 4. ブラウザ / wasm でのシステム監視: 先行例とプロトコル

### 4.1 制約（実測に基づく）

- ブラウザから `/proc` は読めない。`sysinfo` は **wasm32-unknown-unknown にコンパイルは通る**が、`unknown` バックエンドの `SystemInner::new()` が `Err(Error::Unsupported)` を返すため**実行時に何も取得できない**【検証済: `poc-crates/sysinfo-wasm-check` の `cargo check --target wasm32-unknown-unknown` 成功 + `src/unknown/system.rs` の実装】。
- したがって wasm 版のデータ源は (a) ローカル agent の HTTP/WS、(b) 記録データの再生、(c) デモ用の合成データ、のいずれかになる。

### 4.2 Glances REST API v4（一次情報）

- ルート URL: `http://localhost:61208/api/4`（`--bind`/`--port`/`url_prefix` で変更可）。起動は `glances -w --disable-webui` など【検証済: <https://glances.readthedocs.io/en/latest/api/restful.html>】。
- エンドポイント（公式 docs の目次）: `/all`, `/alert`, `/amps`, `/cloud`, `/connections`, `/containers`, `/core`, `/cpu`, `/diskio`, `/folders`, `/fs`, `/gpu`, `/help`, `/ip`, `/irq`, `/load`, `/mem`, `/memswap`, `/mpp`, `/network`, `/now`, `/npu`, `/percpu`, `/ports`, `/processcount`, `/processlist`, `/programlist`, `/psutilversion`, `/quicklook`, `/raid`, `/sensors`, `/smart`, `/system`, `/uptime`, `/version`, `/vms`, `/wifi`【検証済】。
- 認証: HTTP Basic と JWT Bearer の両対応（`--password` で有効化、`POST /api/4/token` でトークン取得）【検証済】。
- 更新: `/all` または各プラグイン取得時に自動更新。Web UI の既定リフレッシュは 2 秒（`?refresh=5` で変更可）【検証済】。
- `processlist` は psutil 由来の詳細フィールド（`cmdline` 配列、`cpu_percent`、`cpu_times` の dict、`memory_info` など）を返す【検証済: docs の例】。
- OpenAPI 仕様: `https://raw.githubusercontent.com/nicolargo/glances/refs/heads/develop/docs/api/openapi.json`【検証済: docs に記載】。
- 評価: **取り込み元としては優秀**（実績・仕様公開・認証あり）。ただし `/all` は「very big dictionary」と明記されており、プロセス全件を毎秒送る用途には向かない。主スキーマにはしない。

### 4.3 node_exporter（Prometheus 形式）

- `:9100` でハードウェア/OS メトリクスを Prometheus テキスト形式で公開。collector はプラグイン式。Linux/macOS/BSD 対応、Windows は別 exporter【検証済: README】。
- 最新リリース v1.12.1（2026-07-14）、contributors 508【検証済: GitHub API】。
- 評価: **時系列メトリクスの標準形式**として参考になるが、pull 型の scrape 前提で、プロセス一覧・kill・ツリー表示には向かない。ローカル agent の `/metrics` 互換出力を将来追加する余地はある。

### 4.4 Cockpit（WebSocket JSON チャネル）

- `cockpit.channel(options)` で「payload タイプ」を指定してチャネルを開き、`channel.send()` / `onmessage` で JSON メッセージを交換する。`superuser: "require"` で root 権限チャネルを要求できる【検証済: doc/modules/guide/pages/cockpit-channels.adoc】。
- ブラウザは cockpit-ws と通信し、cockpit-bridge がシステム側の API（D-Bus、ファイル、HTTP など）をチャネルとして公開する【推測: ドキュメント構成から。プロトコルの完全な仕様は未確認】。
- 評価: **「ブラウザ UI + ローカル agent + 権限分離」**の先行例として、wasm 版の設計に最も近い。ただし Cockpit 自体を組み込むのではなく、設計原則（読み取りと特権操作の分離、チャネル多重化）を参考にする。

### 4.5 記録データの再生

- スナップショットを JSONL（1 行 1 サンプル）で保存し、wasm デモではファイルを読み込んで再生する方式。`serde`/`serde_json` で実装可能。
- 長所: サーバ不要でブラウザデモが完結、再現性が高い。短所: ライブ監視ではない。
- 実測: 約 3.5 千 プロセスを含む 1 スナップショットは **約 2MB**。記録時はプロセスを上位 N 件に絞る/差分のみ保存するなどの圧縮が必要。

### 4.6 推奨プロトコル（設計チーム向け）

**主スキーマは独自の `gpui-task-manager/snapshot/v1`（JSON）とし、Glances 互換は取り込み専用の任意機能にする。**

理由:

1. Glances の `processlist` は psutil 固有のフィールド体系（`cpu_times` の dict 等）で、Rust 側の DTO と二重管理になる。
2. `/all` は全プラグインを一括で返す巨大 JSON で、毎秒の配信には向かない（実測の全件 約 2MB と同種の問題）。
3. Glances は pull 型で、ブラウザへの push（WS/SSE）を前提にしていない。
4. 一方で、Glances サーバが既に動いている環境では「データ源として接続できる」価値があるため、`ingest` アダプタとして `GET /api/4/all` を読む互換は残す。

推奨 API（ローカル agent、Rust + axum 想定）:

| エンドポイント | 方式 | 内容 |
|---|---|---|
| `GET /api/v1/health` | HTTP | バージョン、収集間隔、権限モード（user/root） |
| `GET /api/v1/snapshot` | HTTP | 全体サマリ（CPU per-core、メモリ、ネット、ディスク、プロセス数） |
| `GET /api/v1/processes?sort=cpu&order=desc&limit=100&offset=0&q=` | HTTP | ページング付きプロセス一覧 |
| `GET /api/v1/processes/{pid}?start_time=` | HTTP | プロセス詳細（cmdline/exe/cwd/environ 等） |
| `GET /api/v1/history?from=&to=&step=` | HTTP | リングバッファの時系列 |
| `GET /api/v1/stream` | WebSocket または SSE | 既定 1 秒のサマリ配信（プロセス全件は送らない） |
| `POST /api/v1/processes/{pid}/signal` | HTTP | body に `{signal, start_time}`。**start_time 不一致なら 409**（PID 再利用対策） |
| `GET /api/v1/ingest/glances` | HTTP | 任意: 外部 Glances サーバへのプロキシ（互換） |

- wasm 側のクライアント実装: `gloo-net 0.7`（`json`/`websocket`/`http`/`eventsource` feature を確認済み）、`web-sys 0.3.107`（`WebSocket`/`EventSource` feature あり）、`wasm-bindgen 0.2.130`。
- セキュリティ: agent は既定で **127.0.0.1 バインド + 起動時にランダムトークン発行**。CORS はデモ配信元のみ許可。シグナル送出は「自分の UID のプロセス」に限定し、root 権限が必要な操作は別モード（polkit 等）に分離する。Cockpit の `superuser` 分離を参考。

## 5. GPUI 系の先行事例

### 5.1 gpui-kit `examples/system_monitor`（最重要・ライセンス Apache-2.0）

- リポジトリ: `longbridge/gpui-kit`、`examples/system_monitor/src/main.rs`（628 行）。★20,647、最終 push 2026-11-14【検証済: GitHub API】。
- 依存: `gpui-kit`（workspace）、`sysinfo = "0.37"`、`smol`、`battery`【検証済: Cargo.toml】。
- UI 部品: `chart::AreaChart`、`table::{Column, ColumnSort, DataTable, TableDelegate, TableState}`、`tab::{Tab, TabBar}`、`progress::Progress`、`Icon`、`TitleBar`、`ThemeMode`【検証済: main.rs の import】。
- 実装パターン: `INTERVAL = 500ms`、`MAX_DATA_POINTS = 120`、`cx.spawn` + `smol::Timer` でループし、**UI スレッド上で `sys.refresh_all()` / `disks.refresh(true)` を実行**。プロセス表は PID / Name / CPU% / Memory の 4 列（ソート可）【検証済】。
- 評価: **チャート + 仮想テーブル + タブという本プロジェクトの UI 構成が、そのまま gpui-kit で成立する実証**。ただし収集を UI スレッドで行う点と、sysinfo 0.37 固定である点はそのまま採用しない（0.40.3 へ更新し、収集は別スレッドへ）。

### 5.2 Argus（Ray-lea/Argus）: GPUI 製タスクマネージャの先行例

- 概要: 「现代 Windows 任务管理器」— Rust + GPUI Kit のネイティブ Windows タスクマネージャ。★1,570 / 作成 2026-10-12 / 最終コミット 2026-11-14 / v0.5.0（2026-11-12）/ **AGPL-3.0-or-later**【検証済: GitHub API, README】。
- アーキテクチャ（`docs/architecture/README.md`）【検証済】:
  - `crates/argus-core`: Windows ネイティブの収集、データモデル、履歴、システム操作。
  - `crates/argus-service`: 任意の特権サービス、固定センサーモジュール、タスクマネージャ補助。
  - `crates/argus-app`: サンプリングのスケジューリング、設定、UI 設定、タスク起動。
  - `src-gpui`: GPUI ビュー、ウィンドウ、トレイ、ショートカット、更新。
  - サンプリング間隔は「フォアグラウンド / 最小化 / トレイ」で切り替え。スナップショット変換とプロセスグルーピングはバックグラウンドで行い、UI は最新結果を受け取ってソート・行選択を保持する。**プロセスを変更する前に PID と作成時刻を照合する**（PID 再利用対策）。チャートはサンプル時刻を使い、表示ティックで描画。
- 機能: プロセス（アプリ/ツリー/一覧）、性能（per-core CPU・周波数・温度・電力・メモリ・ディスク・ネットワーク・GPU）、7 日間の履歴、ファイルロックの調査、プロセスツリーの終了【検証済: README の機能表】。
- Web: 公式サイト（argustask.com）で「ブラウザ上で全インタラクティブ UI を試せる」と明記【検証済: README】。実装形態（GPUI wasm か別実装か）は**未確認**。リポジトリ内に wasm/trunk 関連ファイルは見当たらなかった【検証済: git tree 検索】。
- 判断: **コードは AGPL-3.0 のため流用不可**。ただし「収集層と UI の分離」「フォーカス状態で間隔を変える」「PID+作成時刻の照合」「履歴リングバッファ」は、そのまま設計に取り込む価値がある。

### 5.3 その他の GPUI アプリ（topic:gpui）

- `gh search repos --topic gpui`（stars 降順、2026-11-14）で確認した上位: zed-industries/zed（95,005）、AprilNEA/OpenLogi（25,697）、longbridge/gpui-kit（20,648）、zeronsh/zeron（3,647）、remorses/gpuix（2,950）、sonorahq/sonora（2,808）、nyakang/nyaterm（2,422）、sheprd-labs/sheprd（2,295）、66HEX/frame（2,288）、vicanso/zedis（2,206）、AnalyseDeCircuit/oxideterm（1,864）、MatthiasGrandl/Loungy（1,731）、Ray-lea/Argus（1,570）【検証済】。
- **監視系の専用アプリは Argus 以外に確認できなかった**（`gpui monitor` 検索は 0 件）。GPUI 自体は実用アプリで広く使われており、「GPUI でタスクマネージャを作る」ことの技術的リスクは低い。
- `ibisjp/ibis-office` はリポジトリ説明で「Web (Wasm)」対応をうたっており、GPUI の wasm 実用化の傍証になる（詳細は未確認）。

### 5.4 UI 部品の再利用可否

| 部品 | gpui-kit の提供 | 本プロジェクトでの扱い |
|---|---|---|
| 時系列チャート（CPU/メモリ/ネット） | `chart::{LineChart, AreaChart, BarChart}` ほか【検証済: ソース一覧】 | そのまま利用。履歴スクロールバックは自前で実装（zenith の UX を参考） |
| プロセス表 | `table::{DataTable, TableState, TableDelegate, Column, ColumnSort}`（仮想スクロール） | そのまま利用。ページング/差分更新を delegate 側で実装 |
| タブ/サイドバー/ツールバー/ステータスバー | `tab`, `sidebar`, `toolbar`, `status_bar`, `title_bar` | そのまま利用 |
| ツリー表示 | `tree`（ファイルツリー例あり） | プロセスツリーに転用可能（要検証） |
| ダイアログ/通知/入力 | `dialog`, `notification`, `input` | そのまま利用（シグナル確認ダイアログ等） |
| 数値/容量フォーマット | `gpui-base` のユーティリティ | 必要に応じて利用 |

## 6. Runtime Proof（要約）

環境: Kubuntu 2026.04 / 汎用デスクトップCPU / 十分な RAM / rustc 1.99.1。負荷平均は中程度、スレッド込み約 3.5 千エントリの**多忙なデスクトップ**での測定。実行コマンドと丸めた参考ログ:

```bash
cd ./poc-crates/sysinfo-probe
cargo run --release
```

```text
[timing] System::new_all() cold start: 約 85ms
[timing] full tick  (refresh_all + disks + networks): median=約 31ms min=約 30ms max=約 37ms n=7
[timing] targeted tick (cpu+memory+processes only):   median=約 24ms min=約 21ms max=約 27ms n=7
[timing] disks.refresh only: median=約 0.9ms / networks.refresh only: median=約 1.2ms
process_count=約 3500
kill_with(Signal::Term) -> Some(true)
after kill: still_in_sysinfo=true child.try_wait()=Ok(Some(ExitStatus(unix_wait_status(15))))
snapshot json bytes=約 2.2MB (processes=約 3500)
```

全出力は `./notes/crates-survey.md` の付録 A、生ログは `./poc-crates/sysinfo-probe/run.log` を参照。

| 項目 | 実測値 |
|---|---|
| `System::new_all()` コールドスタート | 約 85ms |
| full tick（`refresh_all` + disks + networks）中央値 | **約 31ms**（min 約 30 / max 約 37, n=7） |
| targeted tick（CPU + メモリ + プロセス）中央値 | **約 24ms**（min 約 21 / max 約 27, n=7） |
| `Disks::new_with_refreshed_list()` / `refresh` | 約 1.1ms / 約 0.9ms |
| `Networks::new_with_refreshed_list()` / `refresh` | 約 1.3ms / 約 1.2ms |
| プロセス数（スレッド込み） | 約 3.5 千 |
| per-core CPU | 複数コア分を取得（例: 数十% 台） |
| メモリ / スワップ | 合計・使用量とも数十 GB 台、swap も同規模（丸めた参考値） |
| ネットワーク | 複数 IF（物理 IF / 仮想 IF / loopback 等）を取得 |
| ディスク | 複数エントリ（うち疑似マウントが過半、total=0） |
| プロセス詳細 | cmdline / user / status / start_time / run_time / exe / cwd / parent / disk_usage を取得 |
| user=None の割合 | 約 1%（約 3.5 千件中、他ユーザー/カーネルスレッド由来） |
| cmdline が空 | 約 9%（同、カーネルスレッド等） |
| exe=None | 約 21%（同） |
| `/proc` マウント | `proc /proc proc rw,nosuid,nodev,noexec,relatime`（hidepid なし） |
| kill 実証 | 自前の `sleep 30` に `kill_with(Signal::Term)` → `Some(true)`、`ExitStatus(unix_wait_status(15))` |
| スナップショット JSON | **約 2.2 MB**（約 3.5 千 プロセス、全 cmdline 込み） |

設計上の含意:

1. 1 秒間隔なら収集コストは 1 コアの約 3%、500ms なら約 6%。UI スレッドから追い出すのが前提。
2. 全件 JSON は 約 2MB。プロセス表はページング、ストリームはサマリのみ、が必須。
3. ディスク一覧は overlay/lxcfs 等の擬似マウントを除外するフィルタが必要。
4. hidepid が有効な環境では他ユーザーのプロセスが見えない。起動時に `/proc/mounts` を確認して UI に注記する。
5. `start_time` は Unix epoch 秒（u64、`sysinfo 0.40.3` の `Process::start_time()`）。`(pid, start_time)` を同一性キーにする。
6. ゾンビは再 reap されるまで sysinfo 上に残る（実測: kill 後 300ms の時点で `still_in_sysinfo=true`）。プロセス操作後の表示は「消えたこと」を待つポーリングが必要。

## 7. 未確認事項

1. Mission Center のプロセスツリー表示の有無、Plasma System Monitor のツリー表示/操作の詳細（README では確認できず）。
2. Plasma System Monitor の正確なバージョン（Plasma 同梱のため単独リリースなし）。
3. netdata の言語構成比（C/Go/JS の正確な内訳）と dashboard のライセンス境界。
4. Argus のブラウザ体験デモの実装形態（GPUI wasm か、別実装のモックか）。
5. GPUI の wasm バックエンド（`gpui-pre-web`）で、本プロジェクトのプロセス表 UI が実用速度で描画できるか（別タスク `notes/wasm-spike.md` の検証結果を参照）。
6. Glances REST API の完全なレスポンススキーマ（OpenAPI JSON の全量取得は未実施。docs の例と目次で確認）。
7. Cockpit のプロトコル完全仕様（channels の payload タイプ一覧、ブリッジの実装詳細）。
8. GNOME System Monitor の GitLab contributors 数（API 上限 100 で打ち切り）。
9. htop のネットワーク表示（プラグイン依存の有無）。
10. 各候補の Windows/macOS 品質（本調査は Linux 環境のみで実機確認していない）。

## 8. 参照 URL（すべて 2026-11-14 取得）

- btop: <https://github.com/aristocratos/btop> / <https://raw.githubusercontent.com/aristocratos/btop/main/README.md> / <https://github.com/aristocratos/btop/releases/tag/v1.4.8>
- bottom: <https://github.com/ClementTsang/bottom> / <https://raw.githubusercontent.com/ClementTsang/bottom/main/Cargo.toml> / <https://github.com/ClementTsang/bottom/releases/tag/0.15.0>
- zenith: <https://github.com/bvaisvil/zenith> / <https://raw.githubusercontent.com/bvaisvil/zenith/master/README.md> / <https://raw.githubusercontent.com/bvaisvil/zenith/master/Cargo.toml>
- Mission Center: <https://gitlab.com/mission-center-devs/mission-center> / <https://gitlab.com/mission-center-devs/mission-center/-/raw/main/README.md> / <https://gitlab.com/mission-center-devs/mission-center/-/raw/main/COPYING>
- Resources: <https://github.com/nokyan/resources> / <https://raw.githubusercontent.com/nokyan/resources/main/README.md> / <https://raw.githubusercontent.com/nokyan/resources/main/Cargo.toml>
- GNOME System Monitor: <https://gitlab.gnome.org/GNOME/gnome-system-monitor> / <https://gitlab.gnome.org/GNOME/gnome-system-monitor/-/raw/main/README.md>
- Plasma System Monitor: <https://invent.kde.org/plasma/plasma-systemmonitor> / <https://invent.kde.org/plasma/plasma-systemmonitor/-/raw/master/README.md>
- htop: <https://github.com/htop-dev/htop> / <https://raw.githubusercontent.com/htop-dev/htop/main/htop.1.in> / <https://github.com/htop-dev/htop/releases/tag/3.5.3>
- Glances: <https://github.com/nicolargo/glances> / <https://glances.readthedocs.io/en/latest/api/restful.html> / <https://raw.githubusercontent.com/nicolargo/glances/master/COPYING>
- netdata: <https://github.com/netdata/netdata> / <https://raw.githubusercontent.com/netdata/netdata/master/README.md> / <https://github.com/netdata/netdata/releases/tag/v2.12.3>
- Cockpit: <https://github.com/cockpit-project/cockpit> / <https://raw.githubusercontent.com/cockpit-project/cockpit/main/doc/modules/guide/pages/cockpit-channels.adoc> / <https://github.com/cockpit-project/cockpit/releases/tag/372>
- node_exporter: <https://github.com/prometheus/node_exporter> / <https://github.com/prometheus/node_exporter/releases/tag/v1.12.1>
- gpui-kit: <https://github.com/longbridge/gpui-kit> / <https://github.com/longbridge/gpui-kit/blob/main/examples/system_monitor/src/main.rs>
- Argus: <https://github.com/Ray-lea/Argus> / <https://github.com/Ray-lea/Argus/blob/main/docs/architecture/README.md> / <https://github.com/Ray-lea/Argus/releases/tag/v0.5.0>
- sysinfo: <https://github.com/GuillaumeGomez/sysinfo> / <https://crates.io/api/v1/crates/sysinfo> / <https://github.com/GuillaumeGomez/sysinfo/blob/master/README.md>
- procfs: <https://github.com/eminence/procfs> / <https://crates.io/api/v1/crates/procfs>
- rustix: <https://github.com/bytecodealliance/rustix> / <https://rustsec.org/advisories/RUSTSEC-2026-0486.html>
- nix: <https://github.com/nix-rust/nix> / <https://rustsec.org/advisories/RUSTSEC-2021-0119.html>
- heim: <https://github.com/heim-rs/heim> / <https://crates.io/api/v1/crates/heim>
- wasm クレート: <https://crates.io/api/v1/crates/wasm-bindgen> / <https://crates.io/api/v1/crates/web-sys> / <https://crates.io/api/v1/crates/gloo-net>
- 既存調査: 外部プロジェクトの調査ノート
- Runtime Proof: `./poc-crates/sysinfo-probe/`, `./poc-crates/sysinfo-wasm-check/`

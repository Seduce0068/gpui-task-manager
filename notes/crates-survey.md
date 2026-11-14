---
type: Reference
title: GPUI タスクマネージャー: 収集系 crate 評価と sysinfo 実測（2026-11-14）
description: sysinfo 0.40.3 を中核とする収集 crate の最新版・ライセンス・RUSTSEC・保守状況の評価と、release ビルドでの実測ログ（サンプル取得時間、取得項目、kill、wasm コンパイル確認）。推奨依存リスト付き。
tags:
  - rust
  - crates
  - sysinfo
  - procfs
  - rustix
  - wasm
  - survey
generated: 2026-11-14T20:18:42+09:00
verified: 2026-11-14T20:18:42+09:00
status: draft
---

## 0. 推奨依存リスト

### 0.1 デスクトップ版（GPUI ネイティブ）

```toml
[dependencies]
# UI: 既存調査（work/gpui-file-explorer/notes/ecosystem-notes.md）の第一候補
gpui-kit = "=0.9.1"  # チャート・仮想テーブル・タブ等。Apache-2.0

# 収集: 本調査で実測済みの中核
sysinfo = { version = "=0.40.3", default-features = false, features = [
    "system", "disk", "network", "user", "multithread",
] }

# OS 操作（kill 等）の安全 API。RUSTSEC-2026-0486 修正版を固定
rustix = { version = "=1.1.6", features = ["process"] }

# wire format / 履歴 JSONL / 設定
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# 必要になった時点で追加:
# procfs = "0.18"          # Linux 固有の追加情報（/proc/[pid]/io, smaps_rollup, cgroup 等）
# sysinfo features に "component"（温度センサー）や "gpu"（NVIDIA NVML のみ）を追加
```

- `sysinfo` の既定 features は `component, disk, gpu, network, system, user`。本プロジェクトでは GPU/温度を初期スコープ外にするなら `default-features = false` で上記 5 つに絞り、ビルドサイズと依存を削る。
- `multithread` は `rayon` を有効化し、プロセス走査を並列化する。3,549 プロセス環境での実測（§4）を踏まえ、既定で有効にする。
- `rustix` は `kill_process(pid, Signal)` などの安全 API を提供する（`unsafe` をアプリ側に書かない）。`sysinfo` の `kill_with` でも足りるが、シグナル送出を収集層から分離したい場合に使う。

### 0.2 ローカル agent（ブラウザ向け、ネイティブ実行）

```toml
[dependencies]
axum = "0.8"                 # HTTP API
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "signal"] }
tokio-tungstenite = "0.30"   # WebSocket 配信を使う場合（SSE のみなら不要）
sysinfo = { version = "=0.40.3", default-features = false, features = [
    "system", "disk", "network", "user", "multithread",
] }
rustix = { version = "=1.1.6", features = ["process"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

- CORS 用の `tower-http` 等は本調査では未評価（必要時に追加評価）。
- agent は既定で `127.0.0.1` バインド + 起動時トークン発行を推奨（§4.6 参照）。

### 0.3 wasm クライアント（ブラウザ UI から agent を読む）

```toml
[dependencies]
wasm-bindgen = "0.2.130"
web-sys = { version = "0.3.107", features = [
    "Window", "WebSocket", "MessageEvent", "EventSource", "Worker", "console",
] }
gloo-net = { version = "0.7", features = ["json", "websocket", "http", "eventsource"] }
gloo-timers = "0.4"
js-sys = "0.3.107"
serde = { version = "1", features = ["derive"] }
serde-wasm-bindgen = "0.6.5"  # 任意（JsValue 直接変換で足りるなら不要）
```

### 0.4 採用しない crate

| crate | 理由 |
|---|---|
| `heim` | 最終リリース 2020-12-15、リポジトリ最終 push 2024-07-14 で実質休眠。zenith は fork を git 依存して回避している。 |
| `nix` | 不要（`rustix` + `procfs` で足りる）。採用する場合も 0.31.3 は既知 advisory の影響なし。 |
| wasm での `sysinfo` | コンパイルは通るが実行時に `Err(Unsupported)`。ブラウザではデータ源にならない（§2.5）。 |
| wasm での `reqwest` | wasm では fetch のみで WebSocket 非対応。ブラウザ側は `gloo-net` を使う。 |

## 1. crate 評価表（2026-11-14 取得）

数値は crates.io API の取得時点。DL は累計 / 直近 90 日。RUSTSEC は rustsec.org のパッケージページを確認（404 = advisory なし）。

| crate | 最新安定版（公開日） | ライセンス | DL（累計 / 90日） | RUSTSEC | 判断 |
|---|---|---|---|---:|---|---|
| `sysinfo` | **0.40.3**（2026-11-11） | MIT | 247.2M / 63.0M | なし（パッケージページ 404） | **採用（中核）** |
| `procfs` | 0.18.0（2025-08-30） | MIT OR Apache-2.0 | 96.6M / 24.0M | なし（404） | 条件付き採用（Linux 追加情報） |
| `rustix` | **1.1.6**（2026-10-27） | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | 1,404.3M / 366.6M | RUSTSEC-2026-0486（>=1.0.0, <1.1.6、1.1.6 で修正） | **採用（=1.1.6 固定）** |
| `nix` | 0.31.3（2026-05-11） | MIT | 979.4M / 250.9M | RUSTSEC-2021-0119（^0.20.2 / ^0.21.2 で修正済み） | 代替候補（今回は不採用） |
| `heim` | 0.0.11（2020-12-15） | Apache-2.0 OR MIT | 0.55M / 0.03M | なし | **不採用（休眠）** |
| `serde` | 1.0.229（2026-07-18） | MIT OR Apache-2.0 | 1,706.7M / 431.9M | 未調査（主要 crate） | **採用** |
| `serde_json` | 1.0.154（2026-10-11） | MIT OR Apache-2.0 | 1,613.3M / 434.1M | 未調査（主要 crate） | **採用** |
| `wasm-bindgen` | 0.2.130（2026-10-30） | MIT OR Apache-2.0 | 659.0M / 178.7M | 未調査 | **採用（wasm）** |
| `web-sys` | 0.3.107（2026-10-30） | MIT OR Apache-2.0 | 472.5M / 128.0M | 未調査 | **採用（wasm）** |
| `js-sys` | 0.3.107（2026-10-30） | MIT OR Apache-2.0 | 613.4M / 166.5M | 未調査 | **採用（wasm）** |
| `gloo-net` | 0.7.0（2026-03-25） | MIT OR Apache-2.0 | 27.0M / 6.0M | 未調査 | **採用（wasm HTTP/WS/SSE）** |
| `gloo-timers` | 0.4.0（2026-03-25） | MIT OR Apache-2.0 | 129.3M / 28.1M | 未調査 | **採用（wasm タイマー）** |
| `serde-wasm-bindgen` | 0.6.5（2024-02-27） | MIT | 92.7M / 19.4M | 未調査 | 条件付き（2024 年から更新なし） |
| `axum` | 0.8.9（2026-04-14） | MIT | 592.0M / 165.0M | 未調査 | **採用（agent HTTP）** |
| `tokio` | 1.54.1（2026-10-21） | MIT | 1,191.8M / 309.0M | 未調査 | **採用（agent ランタイム）** |
| `tokio-tungstenite` | 0.30.0（2026-07-11） | MIT | 360.8M / 116.4M | 未調査 | 条件付き（WS 配信時） |
| `reqwest` | 0.13.5（2026-09-08） | MIT OR Apache-2.0 | 912.5M / 266.4M | 未調査 | 条件付き（ネイティブ client 用） |

補足:

- `sysinfo` は **MIT**（README / Cargo.toml の `license = "MIT"`）。`procfs` は `MIT OR Apache-2.0`、`rustix` は三重ライセンス（いずれも Apache-2.0 互換）。
- `rustix` の RUSTSEC-2026-0486 は「xattr 関数がゼロ長バッファで未書き込みバイトを初期化済み扱いにする」Unsoundness。影響は `>=1.0.0, <1.1.6` で、**1.1.6 で修正済み**（2026-10-27 公開）。採用時は `=1.1.6` 以上を固定する。
- `nix` の RUSTSEC-2021-0119 は `getgrouplist` の OOB write（CVE-2021-45707）。0.31.3 は影響範囲外。
- `sysinfo` / `procfs` は rustsec.org にパッケージページが存在しない（= 登録された advisory なし）。

## 2. `sysinfo` 0.40.3 の詳細

### 2.1 基本情報と features

- 最新安定版: **0.40.3（2026-11-11 公開）**。`rust-version = "1.95"`、`edition = "2024"`、ライセンス MIT【検証済: crate tarball の Cargo.toml】。
- 対応 OS（README）: Android, FreeBSD, NetBSD, iOS, Linux, macOS, Raspberry Pi, Windows【検証済】。非対応 OS では「コンパイルは通るが何もしない」と明記。
- features（0.40.3）:
  - 既定: `component`, `disk`, `gpu`, `network`, `system`, `user`
  - 追加: `multithread`（rayon）、`linux-netdevs`、`linux-tmpfs`、`serde`、`c-interface`、`apple-sandbox`、`apple-app-store`、`debug`、`unknown-ci`
- 保守状況: GitHub ★2,761 / 最終 push 2026-11-12 / 直近 12 か月のコミット 100 以上 / 最新リリース 2026-11-11【検証済】。週次〜隔週の活発なリリース。
- 同一 crate の採用実績: bottom が `=0.40.3`、zenith が 0.37、Mission Center 系の一部も利用（Argus の古い lock では 0.31.4）【検証済: 各 Cargo.toml / Cargo.lock】。

### 2.2 取得できる項目（Linux 実測）

`poc-crates/sysinfo-probe` の release 実行で確認した項目:

| 分類 | API | 実測結果（2026-11-14、本機） |
|---|---|---|
| システム | `System::host_name/name/os_version/kernel_version/uptime/boot_time/load_average/physical_core_count/distribution_id` | host, Ubuntu 26.04, kernel 7.0.0-43-generic, uptime 522800s, load 8.39/7.99/7.31, physical 6 / logical 12 |
| CPU | `sys.cpus()`, `Cpu::usage()`, `Cpu::brand()`, `Cpu::frequency()`, `sys.global_cpu_usage()` | Ryzen 5 3600, 3949MHz, 12 コア分の使用率（0〜100） |
| メモリ | `total_memory/used_memory/available_memory/free_memory/total_swap/used_swap` | total 67.35GB / used 49.04GB / available 18.31GB / swap 68.72GB 中 21.14GB 使用 |
| ネットワーク | `Networks::new_with_refreshed_list()`, `NetworkData::{total_received,total_transmitted,received,transmitted,total_packets_received,total_packets_transmitted,total_errors_on_received,mac_address,mtu,ip_networks,transmit_link_speed,receive_link_speed,operational_state}` | 8 IF（enp4s0, tailscale0, docker0, lxcbr0, virbr0, lo, br-*, br-*）。enp4s0 は累計 rx 98.1GB / tx 13.7GB |
| ディスク | `Disks::new_with_refreshed_list()`, `Disk::{name,mount_point,file_system,kind,total_space,available_space,is_removable,is_read_only,usage}` | 12 エントリ。`/` は ext4/SSD 629.7GB 中 100.4GB 空き。overlay/lxcfs の疑似マウント 9 件（total=0）も列挙される |
| プロセス | `sys.processes()`, `Process::{name,cmd,exe,cwd,user_id,status,start_time,run_time,cpu_usage,memory,virtual_memory,disk_usage,parent,tasks,thread_kind,environ}` | 3,549 プロセス。cmdline/user/status/start_time すべて取得（例: firefox-bin, user=user, Sleep, start_time=1794132006, cpu=214.01%） |
| ユーザー | `Users::new_with_refreshed_list()`, `get_user_by_id(&Uid)` | uid → ユーザー名の解決が可能（例: user） |
| シグナル | `Process::{kill, kill_with, kill_and_wait, kill_with_and_wait}`, `Signal` | 自前 `sleep 30` に `kill_with(Signal::Term)` → `Some(true)`、終了 status 15 を確認 |
| コンテナ | `System::cgroup_limits()` | 未実測（機能は存在。コンテナ内の CPU/メモリ上限取得に使える） |
| 温度 | `Components::new_with_refreshed_list()`（`component` feature） | 本 PoC では feature を外して未実測 |
| GPU | `Gpus`（`gpu` feature） | 未実測。Linux 実装は NVML を動的ロードするため **NVIDIA のみ**【検証済: `src/unix/linux/gpu.rs`】 |

### 2.3 制約（設計に効くもの）

1. **CPU 使用率の更新間隔**: Linux の `MINIMUM_CPU_UPDATE_INTERVAL = 200ms`。これより短い間隔で refresh しても意味のある値にならない。サンプリングは 200ms 以上（推奨 1 秒）にする。
2. **プロセスの CPU 使用率は 100% を超える**: マルチスレッドの合計値（実測: firefox-bin 214.01%）。UI では「コア数で正規化した全体比」と「タスクマネージャ流の合計%」を区別して表示する。
3. **プロセスの cmdline/exe/user に欠損がある**: 実測で user=None 33/3,549、cmdline 空 325、exe=None 756。カーネルスレッドや権限制約によるもので、UI は「不明」表示を前提にする。
4. **`hidepid` の影響**: 本機の `/proc` は `hidepid` なし（`proc /proc proc rw,nosuid,nodev,noexec,relatime`）のため他ユーザーのプロセスも見える。`hidepid=1/2` の環境では他ユーザーの情報が制限される（sysinfo の Linux 実装も `hidepid=invisible` を考慮したコメントを持つ）。起動時に `/proc/mounts` を読み、制限モードを UI に注記する。
5. **per-process ネットワークは取得できない**: インターフェース単位のみ。per-process は eBPF/nethogs 等が必要（Mission Center も nethogs の手動セットアップと明記）。
6. **ディスク一覧に疑似マウントが混ざる**: overlay, lxcfs, tmpfs 等を除外するフィルタが必要（実測 12 件中 9 件が疑似マウント）。
7. **ネットワーク一覧に仮想 IF が混ざる**: docker0, lxcbr0, virbr0, tailscale0, lo 等。UI で「物理 IF」と「仮想 IF」を分類するか、フィルタを用意する。
8. **PID 再利用**: `start_time()` は起動からの秒（u64）。`(pid, start_time)` を同一性キーにし、シグナル送出前に照合する。Argus も「PID と作成時刻を照合してから変更する」と明記している。
9. **ゾンビの扱い**: kill 後 300ms 時点で sysinfo 上にまだ存在（`still_in_sysinfo=true`）。子プロセスは reap するまで残るため、UI は「消えるまでポーリング」する。
10. **シグナル送出の権限**: 他ユーザーのプロセスには EPERM。root 権限や polkit が必要な操作は別モードに分離する。
11. **`System` インスタンスの再利用**: README が「同じインスタンスを保持して refresh する方が良い」と明記。毎回 `new_all()` しない（コールドスタート 85ms）。
12. **JSON サイズ**: 3,549 プロセスの全項目 JSON は 2.24MB。全件送信は避け、ページング/上位 N 件/差分にする。

### 2.4 RUSTSEC とライセンス

- RUSTSEC: `sysinfo` に登録 advisory なし（rustsec.org/packages/sysinfo.html は 404）【検証済】。
- ライセンス: MIT（アプリ側 Apache-2.0 と互換）【検証済】。

### 2.5 wasm（wasm32-unknown-unknown）対応

- `poc-crates/sysinfo-wasm-check` で `cargo check --target wasm32-unknown-unknown` が**成功**（sysinfo 0.40.3、features = `system`）【検証済】。
- しかしソース上の `unknown` バックエンドは `SystemInner::new()` が `Err(Error::Unsupported)` を返し、他のメソッドは `unreachable!()`【検証済: `src/unknown/system.rs`】。
- 結論: **コンパイルは通るが実行時に使えない**。ブラウザでは agent 経由か記録再生が必須。

## 3. 代替 crate の評価

### 3.1 `procfs`（Linux 専用の補完）

- 最新 0.18.0（2025-08-30）、`MIT OR Apache-2.0`、rustc 1.70+、Linux のみ【検証済: crates.io / README】。
- GitHub ★450、最終 push 2026-06-26。直近のリリースはやや間隔が空くが、`/proc` の仕様は安定しておりリスクは低い。
- 用途: sysinfo が提供しない Linux 固有情報（`/proc/[pid]/io`、`smaps_rollup`、cgroup、fd 数、`/proc/net/dev`、pressure stall information 等）。
- 判断: **条件付き採用**。初期スコープでは不要。必要になった時点で追加し、sysinfo との二重読みを避ける。

### 3.2 `rustix`（安全な OS API）

- 最新 **1.1.6**（2026-10-27）、`Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT`、★2,162、最終 push 2026-11-12【検証済】。
- `rustix::process::{kill_process, kill_process_group, kill_current_process_group}` を提供（`process` feature）【検証済: ソース】。`unsafe` をアプリ側に書かずにシグナル送出ができる。
- **RUSTSEC-2026-0486**（xattr の Unsound、影響 `>=1.0.0,<1.1.6`）→ **1.1.6 固定で回避**【検証済: rustsec.org】。
- 判断: **採用**。`sysinfo` の kill と重複するが、収集層と操作層を分離する設計に合わせて使う。

### 3.3 `nix`

- 最新 0.31.3（2026-05-11）、MIT、★3,092、最終 push 2026-11-14【検証済】。
- `nix::sys::signal::kill(pid, signal)` を提供。ただし `unsafe` を避けたい本プロジェクトでは `rustix` を優先。
- RUSTSEC-2021-0119 は 0.31.3 に影響しない。
- 判断: **代替候補（今回は不採用）**。Resources は `nix`（signal/sched）を採用しているが、我々は依存を増やさない。

### 3.4 `heim`

- 最新 0.0.11（**2020-12-15**）、`Apache-2.0 OR MIT`、GitHub ★932、最終 push **2024-07-14**【検証済: crates.io / GitHub API】。
- 非同期ベースのクロスプラットフォーム収集ライブラリだが、5 年以上リリースがなく実質休眠。zenith は `bvaisvil/heim` の fork を git 依存して回避している。
- 判断: **不採用**（メンテ停止・Rust エコシステムの更新に追随していない）。

### 3.5 `serde` / `serde_json`

- `serde 1.0.229`（2026-07-18）/ `serde_json 1.0.154`（2026-10-11）。MIT OR Apache-2.0。DL は 10 億超でデファクト【検証済: crates.io】。
- 用途: agent の wire format、履歴の JSONL 保存、設定ファイル。`sysinfo` 自身の `serde` feature は使わず、アプリ側の DTO でスキーマを固定する（バージョン間の互換性を自分で制御するため）。

### 3.6 wasm 系 crate

- `wasm-bindgen 0.2.130` / `web-sys 0.3.107` / `js-sys 0.3.107`（いずれも 2026-10-30）【検証済】。
- `web-sys` に `WebSocket` / `EventSource` / `Worker` / `Window` feature が存在することを確認【検証済】。
- `gloo-net 0.7.0`（2026-03-25）の既定 features は `json`, `websocket`, `http`, `eventsource`【検証済: crates.io の features】。ブラウザ側の HTTP/WS/SSE クライアントはこれで足りる。
- `gloo-timers 0.4.0`（2026-03-25）: wasm のタイマー。
- `serde-wasm-bindgen 0.6.5`（2024-02-27）: 2024 年から更新なし。安定・小さいが、必要なら `wasm-bindgen` の `JsValue` 直接変換で代替可能。**条件付き採用**。
- 判断: wasm クライアントは `gloo-net` + `web-sys` + `gloo-timers` の組み合わせを推奨。GPUI 側の wasm バックエンド（`gpui-pre-web`）は別タスクの検証（`notes/wasm-spike.md`）に従う。

## 4. 実測方法（再現手順）

```bash
# 1) sysinfo のサンプリングコストと取得項目（release）
cd work/gpui-task-manager/poc-crates/sysinfo-probe
cargo run --release

# 2) wasm32-unknown-unknown へのコンパイル可否
cd work/gpui-task-manager/poc-crates/sysinfo-wasm-check
cargo check --target wasm32-unknown-unknown
```

- `sysinfo-probe` は `[workspace]` を空宣言した独立 crate（親 workspace に入らない）。
- 測定は `System::new_all()` を 1 回 → 250ms スリープを挟んで 7 回の tick を計測し、中央値を採用。
- 負荷状況: load average 8 前後、プロセス数 3,549（Firefox 複数 + Docker コンテナ）という**多忙なデスクトップ**での値。空闲時の値はこれより小さくなる可能性が高い（未確認）。

## 5. 未確認事項

1. `sysinfo` の `component`（温度）と `gpu`（NVIDIA NVML）feature の実機動作（本 PoC では feature を外した）。
2. `sysinfo` の Windows/macOS 実装品質（Linux のみ実測）。
3. `hidepid=1/2` 環境での欠損率（本機は hidepid なしのため未測定）。
4. `procfs` を併用した場合の追加取得項目の実測（`/proc/[pid]/io` 等）。
5. `cgroup_limits()` のコンテナ内での挙動。
6. `axum` / `tokio` / `gloo-net` のバージョン組み合わせでのビルド検証（本調査ではバージョン確認のみ）。
7. `serde` / `wasm-bindgen` 系の RUSTSEC 個別確認（rustsec.org のパッケージページは未取得）。

## 6. 参照 URL（すべて 2026-11-14 取得）

- sysinfo: <https://crates.io/api/v1/crates/sysinfo> / <https://github.com/GuillaumeGomez/sysinfo> / <https://github.com/GuillaumeGomez/sysinfo/blob/master/README.md> / <https://crates.io/api/v1/crates/sysinfo/0.40.3/download（tarball> 実査）
- procfs: <https://crates.io/api/v1/crates/procfs> / <https://github.com/eminence/procfs> / <https://raw.githubusercontent.com/eminence/procfs/master/README.md>
- rustix: <https://crates.io/api/v1/crates/rustix> / <https://rustsec.org/advisories/RUSTSEC-2026-0486.html> / <https://github.com/bytecodealliance/rustix>
- nix: <https://crates.io/api/v1/crates/nix> / <https://rustsec.org/advisories/RUSTSEC-2021-0119.html> / <https://github.com/nix-rust/nix>
- heim: <https://crates.io/api/v1/crates/heim> / <https://github.com/heim-rs/heim>
- serde / serde_json: <https://crates.io/api/v1/crates/serde> / <https://crates.io/api/v1/crates/serde_json>
- wasm 系: <https://crates.io/api/v1/crates/wasm-bindgen> / <https://crates.io/api/v1/crates/web-sys> / <https://crates.io/api/v1/crates/gloo-net> / <https://crates.io/api/v1/crates/gloo-timers>
- agent 系: <https://crates.io/api/v1/crates/axum> / <https://crates.io/api/v1/crates/tokio> / <https://crates.io/api/v1/crates/tokio-tungstenite>
- 既存調査: `work/gpui-file-explorer/notes/ecosystem-notes.md`（gpui-kit 0.9.1 / gpui-pre 0.3.12 の来歴）、`work/gpui-file-explorer/notes/wasm-browser-notes.md`
- Runtime Proof ソース: `work/gpui-task-manager/poc-crates/sysinfo-probe/src/main.rs`, `work/gpui-task-manager/poc-crates/sysinfo-wasm-check/src/main.rs`

## 付録 A: `cargo run --release` の全出力

環境: Kubuntu 2026.04 / Ryzen 5 3600（6C/12T）/ RAM 64GB / rustc 1.99.1 / sysinfo 0.40.3。実行: `cargo run --release`（`poc-crates/sysinfo-probe`）。

```text
   Compiling sysinfo-probe v0.1.0 (/home/user/document/deepseek-harness/default-workspace/work/gpui-task-manager/poc-crates/sysinfo-probe)
    Finished `release` profile [optimized] target(s) in 0.62s
     Running `target/release/sysinfo-probe`
== sysinfo probe: crate version 0.40.3 ==
[timing] System::new_all() cold start: 85.027234ms
[timing] Disks::new_with_refreshed_list(): 1.080602ms
[timing] Networks::new_with_refreshed_list(): 1.249091ms
[timing] full tick  (refresh_all + disks + networks): median=31.440903ms min=30.27202ms max=36.892232ms n=7
[timing] targeted tick (cpu+memory+processes only):   median=24.280293ms min=20.861438ms max=27.408202ms n=7
[timing] disks.refresh only: median=898.574µs / networks.refresh only: median=1.174251ms

== system ==
host=Ok("<host>") os=Ok("Ubuntu") os_version=Ok("26.04") kernel=Ok("7.0.0-43-generic")
uptime=Ok(522800) boot_time_unix=Ok(1794131936) physical_cores=Ok(6) logical_cpus=12
load_avg 1/5/15 = 8.39/7.99/7.31
cpu_brand="AMD Ryzen 5 3600 6-Core Processor" cpu_frequency_mhz=3949
global_cpu_percent=48.41
per_core_cpu_percent (12 cores) = [42.3, 53.8, 57.7, 61.5, 51.9, 44.4, 42.3, 46.2, 36.0, 37.0, 57.7, 48.0]

== memory ==
total=67351371776 used=49043570688 available=18307801088 free=2297499648 swap_total=68719472640 swap_used=21142663168

== networks ==
if=br-19b60f1f19dc total_rx=0 total_tx=0 rx_since_last=0 tx_since_last=0 pkts_rx=0 pkts_tx=0 err_rx=0 mac=3e:30:94:23:14:2b mtu=1500
if=docker0 total_rx=0 total_tx=0 rx_since_last=0 tx_since_last=0 pkts_rx=0 pkts_tx=0 err_rx=0 mac=ce:b0:0e:1c:43:48 mtu=1500
if=tailscale0 total_rx=57831855 total_tx=25423078 rx_since_last=224 tx_since_last=0 pkts_rx=386304 pkts_tx=389380 err_rx=0 mac=00:00:00:00:00:00 mtu=1280
if=lxcbr0 total_rx=0 total_tx=0 rx_since_last=0 tx_since_last=0 pkts_rx=0 pkts_tx=0 err_rx=0 mac=10:66:6a:00:00:00 mtu=1500
if=enp4s0 total_rx=98132741593 total_tx=13711775029 rx_since_last=4470 tx_since_last=3887 pkts_rx=95518386 pkts_tx=37564405 err_rx=0 mac=d4:5d:64:ee:7f:8f mtu=1280
if=lo total_rx=116447834135 total_tx=116447834135 rx_since_last=3977 tx_since_last=3977 pkts_rx=47468427 pkts_tx=47468427 err_rx=0 mac=00:00:00:00:00:00 mtu=65536
if=br-9a7ce1e92d28 total_rx=0 total_tx=0 rx_since_last=0 tx_since_last=0 pkts_rx=0 pkts_tx=0 err_rx=0 mac=5e:d0:b0:f1:95:50 mtu=1500
if=virbr0 total_rx=0 total_tx=0 rx_since_last=0 tx_since_last=0 pkts_rx=0 pkts_tx=0 err_rx=0 mac=52:54:00:ab:71:77 mtu=1500

== disks ==
disk="/dev/nvme0n1p7" mount="/" fs="ext4" kind=SSD total=629697183744 available=100447502336 removable=false read_only=false
disk="/dev/nvme0n1p2" mount="/boot/efi" fs="vfat" kind=SSD total=100663296 available=60939264 removable=false read_only=false
disk="/dev/sdb1" mount="/mnt/timeshift" fs="ext4" kind=SSD total=471415693312 available=331495960576 removable=true read_only=false
disk="lxcfs" mount="/var/lib/lxcfs" fs="fuse.lxcfs" kind=Unknown(-1) total=0 available=0 removable=false read_only=false
disk="overlay" mount="/var/lib/docker/rootfs/overlayfs/2363ec980e8503da19476f7f564f4dfb790b14efa2c0a7779d6abe6d95dd8da1" fs="overlay" kind=Unknown(-1) total=0 available=0 removable=false read_only=false
disk="overlay" mount="/var/lib/docker/rootfs/overlayfs/cb3f2761da4750b3a80f43bd4d3030f1ec4d5a2930a9c86e11235f1bba9f02a4" fs="overlay" kind=Unknown(-1) total=0 available=0 removable=false read_only=false
disk="overlay" mount="/var/lib/docker/rootfs/overlayfs/1f1ad0723fe1a01e297849abf262e3a5d4545640ffbe82b2cd76147478705265" fs="overlay" kind=Unknown(-1) total=0 available=0 removable=false read_only=false
disk="overlay" mount="/var/lib/docker/rootfs/overlayfs/761a3a4d11561be86a2ef894e937b2130447f2da5b826e3061384ab03d6459d0" fs="overlay" kind=Unknown(-1) total=0 available=0 removable=false read_only=false
disk="overlay" mount="/var/lib/docker/rootfs/overlayfs/4552193caac72682c0937f5b6358775a71f865532d9c8604bf2b0e604e96bc65" fs="overlay" kind=Unknown(-1) total=0 available=0 removable=false read_only=false
disk="overlay" mount="/var/lib/docker/rootfs/overlayfs/c9c452aed8386d79f5949016c193c338bcf43f5fe9f514bb94d55d4be33fcabe" fs="overlay" kind=Unknown(-1) total=0 available=0 removable=false read_only=false
disk="overlay" mount="/var/lib/docker/rootfs/overlayfs/c3408f5bdb41df4a3859ce73cdf763ffb26b152cdf872a36b2d5aae708f28c30" fs="overlay" kind=Unknown(-1) total=0 available=0 removable=false read_only=false
disk="overlay" mount="/var/lib/docker/rootfs/overlayfs/530f50b2009a67ea73b07074c2a91cfa7e4f1fee8c30132890924c90ba8eeb29" fs="overlay" kind=Unknown(-1) total=0 available=0 removable=false read_only=false

== processes ==
process_count=3549
top 5 by cpu:
pid=6977 name="firefox-bin" user=Some("user") status=Sleep start_time=1794132006 run_time=522730 cpu=214.01 mem=4018008064 cmdline="/home/user/downloads/firefox/firefox --sm-client-id 1031c3692fc2e9000179380566300041380770000"
pid=8545 name="Isolated Web Co" user=Some("user") status=Run start_time=1794132011 run_time=522725 cpu=76.43 mem=2690158592 cmdline="/home/user/downloads/firefox/firefox-bin -contentproc -isForBrowser -prefsHandle 0:37179 -prefMapHandle 1:304037 -jsInitHandle 2:161360 -parentBuildID 20261102144711 -sandboxReporter 3 -ipcHandle 4 -initialChannelId {08180635-df7a-4c53-90f7-94b9bb463c8d} -parentPid 6977 -crashHelperPid 7054 -crashHelper 5 -crashReporter 6 -greomni /home/user/downloads/firefox/omni.ja -appomni /home/user/downloads/firefox/browser/omni.ja -appDir /home/user/downloads/firefox/browser 13 tab"
pid=8577 name="Isolated Web Co" user=Some("user") status=Sleep start_time=1794132011 run_time=522725 cpu=72.61 mem=2446483456 cmdline="/home/user/downloads/firefox/firefox-bin -contentproc -isForBrowser -prefsHandle 0:37179 -prefMapHandle 1:304037 -jsInitHandle 2:161360 -parentBuildID 20261102144711 -sandboxReporter 3 -ipcHandle 4 -initialChannelId {ebcd3eb9-b28e-4bea-8333-3f3e8e4b22b3} -parentPid 6977 -crashHelperPid 7054 -crashHelper 5 -crashReporter 6 -greomni /home/user/downloads/firefox/omni.ja -appomni /home/user/downloads/firefox/browser/omni.ja -appDir /home/user/downloads/firefox/browser 16 tab"
pid=7226 name="TaskCon~ller #0" user=Some("user") status=Run start_time=1794132006 run_time=522730 cpu=57.32 mem=4019830784 cmdline="/home/user/downloads/firefox/firefox --sm-client-id 1031c3692fc2e9000179380566300041380770000"
pid=4012693 name="sysinfo-probe" user=Some("user") status=Sleep start_time=1794654731 run_time=5 cpu=57.32 mem=35463168 cmdline="target/release/sysinfo-probe"
field coverage: user=None 33/3549 / cmdline empty 325/3549 / exe=None 756/3549

/proc mount line: proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0

== kill test (own child only) ==
spawned pid=4012817 before=Some(("sleep", 1794654736, "Sleep"))
kill_with(Signal::Term) -> Some(true)
after kill: still_in_sysinfo=true child.try_wait()=Ok(Some(ExitStatus(unix_wait_status(15))))

== json ==
snapshot json bytes=2241066 (processes=3549)
head: {"schema":"gpui-task-manager/snapshot/v1","captured_at_unix":1794654737,"sysinfo_crate_version":"0.40.3","host_name":"<host>","os_name":"Ubuntu","os_version":"26.04","kernel_version":"7.0.0-43-generic","uptime_secs":522800,"boot_time_unix":1794131936,"load_avg":[8.39,7.99,7.31],"physical_cores":6,"logical_cpus":12,"cpu_brand":"AMD Ryzen 5 3600 6-Core Processor","cpu_frequency_mhz":3949,"global_cpu_percent":37.35294,"per_core_cpu_percent":[42.307693,53.846157,57.692307,61.538464,51.851852,44.444443,42.307693,46.15385,36.0,37.03704,57.692307,48.0],"memory_total":673513717

DONE
```

## 付録 B: wasm32-unknown-unknown コンパイル確認

`poc-crates/sysinfo-wasm-check` にて:

```text
    Checking memchr v2.8.3
    Checking sysinfo v0.40.3
    Checking sysinfo-wasm-check v0.1.0 (/home/user/document/deepseek-harness/default-workspace/work/gpui-task-manager/poc-crates/sysinfo-wasm-check)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.53s
```

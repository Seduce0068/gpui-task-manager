---
type: Research
title: GPUI Task Manager 調査レポート
description: Ubuntu系向けGPUIシステムタスクマネージャーの既存ソリューション・crate・GPUIスタック・wasm/ブラウザ戦略の調査結果（2026-11-14）
tags: [gpui, rust, task-manager, system-monitor, wasm, survey]
status: draft
generated: 2026-11-14T21:05:00+09:00
verified: 2026-11-14T21:05:00+09:00
---

## 0. 結論

### 0.1 採用スタック

| 層 | 採用 | 根拠 |
|---|---|---|
| UI（ネイティブ） | `gpui-kit = "=0.9.1"`（`gpui-pre 0.3.12` = Zed GPUI スナップショット） | Apache-2.0。部品が揃い、商用アプリ実績あり。既存の GPUI 試作で実機ビルド・Wayland 起動を確認済み |
| 収集 | `sysinfo = "=0.40.3"` | MIT。per-core CPU / メモリ / スワップ / ネットワーク / ディスク / プロセス（cmdline・user・status・start_time）を 1 パスで取得。bottom も同版を採用 |
| シグナル | `rustix = "=1.1.6"`（`process` feature） | 安全 API で `kill_process`。RUSTSEC-2026-0486 の修正版を完全ピン |
| シリアライズ | `serde` / `serde_json` | wire format と将来の記録再生 |
| ブラウザ UI | `gpui-pre` + `gpui-pre-platform`（`default-features = false`）+ `gpui-pre-web` | stable で wasm32 にコンパイル可能（既存調査）。GPUI Web の描画をヘッドレス Chrome で確認済み |
| ブラウザのデータ | デモ（合成）+ 読み取り専用ループバック agent | ブラウザから `/proc` は読めない。agent はトークン認証・CORS 制限・読み取り専用 |

### 0.2 重要な実測値

| 項目 | 値 | 出典 |
|---|---|---|
| sysinfo full tick（スレッド込み 3,549 エントリ） | 中央値 31.4 ms | `notes/crates-survey.md` §4 / `poc-crates/sysinfo-probe` |
| sysinfo cold start | 85 ms | 同上 |
| 実プロセス数（本機） | 約 620（`ps -e` と一致） | 同上。**sysinfo は `with_tasks()` 有効時、Linux でスレッドもプロセス map に入れる**ため、スレッド除外が必要（試作で修正済み） |
| ワイヤ JSON（618 プロセス、limit=2000） | 約 173 KB（既定 500 件で約 137 KB） | 実測。旧測定 2.24 MB はスレッド込み 3,549 エントリ・全項目の値 |
| プロセス CPU% の飽和 | sysinfo の `cpu_usage()` が本機で全プロセス 1200%（コア数×100）に飽和 | collector-dev の実測。`accumulated_cpu_time()` の差分÷経過時間で自前計算に変更 |
| CPU 更新の最小間隔 | 200 ms（`MINIMUM_CPU_UPDATE_INTERVAL`） | sysinfo 仕様。既定 1 秒サンプリングは妥当 |
| 対象 GPU / セッション | GTX 1660 SUPER / Wayland / 12 論理コア / RAM 64 GB | 本機 |

### 0.3 既存ソリューションの再利用判断

Linux の既存モニタは **GPL/AGPL が中心**で、コードの再利用はライセンス上できない。
一方で、機能仕様・UX・アーキテクチャは設計参考として価値が高い。

- 機能スコープの参照: **Mission Center**（per-thread CPU、メモリ内訳、収集プロセス分離）
- 情報設計の参照: **btop / bottom**（上部グラフ + 下部プロセス表）
- UX の参照: **zenith**（履歴スクロールバック）、**htop**（シグナル選択）
- アーキテクチャの参照: **Cockpit**（ブラウザ UI + ローカル agent + 権限分離）、**Mission Center**（収集の別プロセス化）
- プロトコルの参照: **Glances REST API v4**（取り込み専用の任意互換。主スキーマにはしない）
- GPUI の先行例: **Argus**（AGPL、Windows 向け）。収集/UI 分離、PID+作成時刻照合、履歴リングバッファを参考（コード流用不可）

新規実装とする判断は妥当。GPUI + ネイティブ + wasm の既存タスクマネージャは Argus 以外に確認できなかった。

---

## 1. 調査方法と前提

- 調査日: **2026-11-14**（Asia/Tokyo）。数値・日付はすべてこの日に取得。
- 環境: Kubuntu 2026.04 / KDE Plasma 6 / Wayland / Ryzen 5 3600（6C/12T）/
  RAM 64 GB / GTX 1660 SUPER / rustc 1.99.1。
- 一次情報: GitHub REST API（`gh api`）、GitLab REST API、crates.io API、
  rustsec.org、各リポジトリの README / Cargo.toml / 公式 docs、crate tarball。
- 外部プロジェクトの調査ノートは手掛かりとして参照。**AGPL/GPL コードの
  コピーは行っていない**。
- 表記: **【検証済】** = 一次情報を取得して確認。**【推測】** = 取得情報からの推定。
  **【未確認】** = 未検証。

---

## 2. 既存ソリューション（11 候補）

### 2.1 比較表（要約）

| 候補 | 言語 / UI | ライセンス | Web/wasm | 活発度（最終更新 / 最新リリース） | 判断 |
|---|---|---|---|---|---|
| btop | C++23 / TUI | Apache-2.0 | なし | 2026-11-11 / v1.4.8 | 参考（情報設計） |
| bottom | Rust / ratatui | MIT | なし | 2026-11-14 / 0.15.0 | 参考（sysinfo 採用の裏付け） |
| zenith | Rust / ratatui | MIT | なし | 2026-10-01 / 0.15.1 | 参考（履歴 UX） |
| Mission Center | Rust / GTK4 | GPL-3.0 | なし | 2026-11-03 / v1.2.0 | 参考（機能スコープ） |
| Resources | Rust / GTK4 | GPL-3.0 | なし | 2026-05-12 / v1.10.2（archived） | 参考（アプリ単位 UX） |
| GNOME System Monitor | C++ / gtkmm4 | GPL-2.0 | なし | 2026-11-11 / 51.1 | 参考（標準機能） |
| Plasma System Monitor | QML + C++ | GPL 系 | なし | 2026-11-14 / Plasma 同梱 | 参考（センサー抽象化） |
| htop | C / ncurses | GPL-2.0 | なし | 2026-11-13 / 3.5.3 | 参考（操作体系） |
| Glances | Python / psutil | LGPL-3.0 | **あり**（Web UI + REST v4） | 2026-11-14 / v4.5.9 | 条件付き（プロトコル互換候補） |
| netdata | C + Go / Web | GPL-3.0+ / NCUL1 | **あり** | 2026-11-14 / v2.12.3 | 不採用（重量級） |
| Cockpit | C / Python / JS | LGPL-2.1 ほか | **あり** | 2026-11-13 / 372 | 参考（ブラウザ + agent 構成） |

詳細な stars / contributors / 機能マトリクスは
[`notes/solutions-survey.md`](notes/solutions-survey.md) §2 を参照。

### 2.2 採用しなかった理由（共通）

1. **ライセンス**: 主要な Linux モニタは GPL/AGPL。コードを取り込むと本プロジェクトの
   Apache-2.0 方針と両立しない。
2. **UI スタックが異なる**: TUI（ratatui/ncurses）や GTK/Qt であり、GPUI に組み込めない。
3. **wasm 非対応**: いずれもネイティブ専用で、ブラウザ戦略の参考にならない。

---

## 3. 収集 crate の評価

### 3.1 評価表（2026-11-14 取得）

| crate | 最新版（公開日） | ライセンス | RUSTSEC | 判断 |
|---|---|---|---|---|
| `sysinfo` | **0.40.3**（2026-11-11） | MIT | なし | **採用（中核）** |
| `procfs` | 0.18.0（2025-08-30） | MIT OR Apache-2.0 | なし | 条件付き（Linux 追加情報） |
| `rustix` | **1.1.6**（2026-10-27） | Apache-2.0 WITH LLVM-exception OR MIT | RUSTSEC-2026-0486（<1.1.6） | **採用（=1.1.6 固定）** |
| `nix` | 0.31.3（2026-05-11） | MIT | 既知分は修正済み | 代替候補（不採用） |
| `heim` | 0.0.11（2020-12-15） | Apache-2.0 OR MIT | なし | **不採用（休眠）** |
| `serde` | 1.0.229（2026-07-18） | MIT OR Apache-2.0 | なし | 採用 |
| `serde_json` | 1.0.154（2026-10-11） | MIT OR Apache-2.0 | なし | 採用 |
| `wasm-bindgen` | 0.2.130（2026-10-30） | MIT OR Apache-2.0 | なし | 採用（web シェル） |
| `web-sys` / `js-sys` | 0.3.107（2026-10-30） | MIT OR Apache-2.0 | なし | 採用（web シェル） |
| `gloo-net` | 0.7.0（2026-03-25） | MIT OR Apache-2.0 | なし | 条件付き（agent 通信） |

詳細は [`notes/crates-survey.md`](notes/crates-survey.md) §1–3 を参照。

### 3.2 sysinfo 0.40.3 の制約（設計に効くもの）

1. **CPU 更新の最小間隔は 200 ms**。既定 1 秒サンプリングは妥当。
2. **プロセスの CPU% は 100% を超える**（マルチスレッド合計）。UI はそのまま表示し、
   色分けは 100% で clamp する。
3. **cmdline / exe / user に欠損がある**（カーネルスレッド、権限制約）。「不明」表示を前提。
4. **`hidepid` の影響**: `hidepid=1/2` では他ユーザーの情報が制限される。UI に注記する
   余地を残す（MVP では未実装）。
5. **per-process ネットワークは取得できない**（インターフェース単位のみ）。
6. **ディスク一覧に疑似マウントが混ざる**（overlay / lxcfs / tmpfs）。フィルタが必要。
7. **ネットワーク一覧に仮想 IF が混ざる**（docker0, vpn0, lo 等）。
8. **PID 再利用**: `start_time()` を併用して同一性を検証する。
9. **ゾンビ**: kill 後も reap されるまで残る。UI は消えるまでポーリングする。
10. **`System` インスタンスを再利用**する（コールドスタート 85 ms を毎回払わない）。
11. **スレッド混入**: `with_tasks()` を有効にすると Linux ではスレッドも process map に
    入る。試作ではスレッド行を除外し、`threads` フィールドに件数を入れるよう修正した。
12. **JSON サイズ**: 実装後のワイヤ形式で 618 プロセス約 173 KB（既定 500 件で約 137 KB）。旧調査の 2.24 MB はスレッド込み 3,549 エントリ・全項目の測定値。大規模環境でもページング / 上位 N 件で送る。

### 3.3 実測（`poc-crates/sysinfo-probe`）

- release ビルドで `System::new_all()` + `refresh_all()` を 10 回実行。
- 中央値 **31.4 ms**（3,549 エントリ時）、cold start 85 ms。
- per-core CPU（12）、メモリ / スワップ、ネットワーク 8 IF、ディスク 12 エントリ、
  プロセスの cmdline / user / status / start_time を確認。
- `kill_with(Signal::Term)` で自分で spawn した `sleep 30` の終了を確認。

---

## 4. GPUI スタック

### 4.1 採用: `gpui-kit = "=0.9.1"`

- 実体: `gpui-pre 0.3.12`（Zed commit `d2a4fb6` のスナップショット、Apache-2.0）+
  `gpui-pre-platform` + `gpui-component` + `gpui-kit-assets`（Lucide アイコン）。
- 理由:
  1. 単一依存で版が揃い、型不一致事故が起きにくい。
  2. DataTable（仮想スクロール）・VirtualList・Theme・Dialog などの部品が揃う。
  3. Longbridge Pro での商用実績、週次リリース、3 OS の CI。
  4. Apache-2.0 で、Zed アプリ本体の GPL コードを引かない。
- リスク:
  - pre-1.0 / 週次破壊的変更。`=` 完全ピン + `Cargo.lock` コミットで統制する。
  - `gpui-pre-*` は個人アカウント（huacnlee）による再公開。bus factor = 1。
  - 独立したセキュリティ監査は未確認。
- 代替: 公式 `gpui 0.2.2`（2025-10 で更新停止）、Zed git 依存（GPL 感染）、
  `gpui-ce`（API 互換性が流動的）、`gpui-fast`（実験的。MVP では使わない）。

### 4.2 既存 GPUI 試作からの知見

- Linux では `gpui-pre-platform` の `wayland` / `x11` feature を明示する必要がある。
  `gpui-kit` 経由なら既定で有効。
- `libxkbcommon-x11-dev` が無いホストではリンクが失敗する。`scripts/dev-setup.sh` で
  ローカル symlink を作成して回避（システム変更なし・root 不要）。
- 起動は `gpui_kit::application().with_assets(...).run(|cx| { gpui_kit::init(cx); ... })`。
- 非同期は `cx.spawn` / `cx.background_spawn` + `background_executor().timer(...)`。
- 仮想リストは `uniform_list` + `UniformListScrollHandle`。

---

## 5. wasm / ブラウザ

### 5.1 原理的制約

- ブラウザから `/proc` は読めない。
- `sysinfo` は wasm32-unknown-unknown に**コンパイルは通る**が、`unknown` バックエンドの
  `SystemInner::new()` が `Err(Unsupported)` を返すため**実行時に使えない**【検証済:
  `poc-crates/sysinfo-wasm-check`】。
- したがって wasm 版のデータ源は (a) ローカル agent の HTTP、(b) 記録データの再生、
  (c) 合成データ、のいずれかになる。

### 5.2 GPUI Web バックエンドの実証（Runtime Proof）

`poc-wasm/` に最小の GPUI Web アプリを作り、stable ツールチェーンで wasm32 に
ビルドしてヘッドレス Chrome で実行した。

- 依存: `gpui-pre = "=0.3.12"` + `gpui-pre-platform = "=0.3.12"` を
  `default-features = false`（`wasm_thread` を避けるため）。`gpui-kit` は stable では
  `wasm_thread` を要求するため wasm シェルには使わない。
- `cargo check --target wasm32-unknown-unknown`: 成功。
- `trunk build --release`: 成功（trunk 0.22.1）。
- ヘッドレス Chrome 157 で実行:
  - WebGPU はアダプタが取得できず、**WebGL2（SwiftShader）へフォールバック**して描画成功。
  - GPUI のフォントロード、CJK テキスト描画、クリック、スクロール、タイマー更新、
    `fetch`（`/stats.json` → HTTP 200）を確認。
  - スクリーンショット 8 枚を `notes/assets/wasm-spike-*.png` に保存。
- 制約: ヘッドレス環境では WebGPU が使えず WebGL2 フォールバックになる。実ブラウザ
  （Edge / Chrome）では WebGPU が使える可能性が高い（未確認）。

### 5.3 ブラウザ戦略

1. **主経路**: ネイティブの `tm-agent`（127.0.0.1、トークン認証、読み取り専用）から
   `GET /api/v1/snapshot?limit=N` で取得する。
2. **デモ経路**: `DemoProbe` の合成データで、サーバなしでも UI を確認できる。
3. **プロセス操作**: ブラウザでは提供しない（読み取り専用）。エージェントにも操作 API を
   置かない。
4. **ページング**: 大規模環境でも 1 秒間隔の全件送信を避けるため、`limit`（既定 500、最大 2000）と
   `total_process_count` を返す。将来は `offset` と差分更新を追加する。

---

## 6. プロトコル

### 6.1 独自スキーマ `gpui-task-manager/snapshot/v1`

```json
{
  "schema_version": 1,
  "total_process_count": 618,
  "snapshot": {
    "sampled_at": { "unix_millis": 1794657411364 },
    "hostname": "...",
    "os_name": "Ubuntu",
    "kernel_version": "7.0.0-43-generic",
    "cpu": { "total_percent": 56.2, "per_core_percent": [51.4, ...], "load_average": [8.4, 8.0, 7.3], "uptime_secs": 522800 },
    "memory": { "total_bytes": 67300000000, "used_bytes": 49000000000, "available_bytes": 18300000000, "swap_total_bytes": 68700000000, "swap_used_bytes": 21100000000 },
    "processes": [ { "key": { "pid": 1, "start_time_secs": 1794132006 }, "name": "systemd", "command": "/sbin/init", "user": "root", "status": "Sleeping", "cpu_percent": 0.1, "memory_bytes": 16800000, "virtual_memory_bytes": 26500000, "threads": 1, "parent_pid": null } ],
    "network": { "interfaces": [ { "name": "eth0", "received_bytes": 98100000000, "transmitted_bytes": 13700000000 } ] },
    "disks": [ { "mount_point": "/", "total_bytes": 629700000000, "available_bytes": 100400000000, "used_bytes": 529300000000 } ],
    "self_pid": 12345
  }
}
```

- `schema_version` は `tm_core::WIRE_SCHEMA_VERSION`。
- プロセスは CPU 降順 → PID 昇順でソートして `limit` 件に絞る。
- 認証（`/api/v1/snapshot`）: `Authorization: Bearer <token>` のみ。クエリ文字列のトークンは拒否する
  （URL 漏洩対策）。ブラウザは URL フラグメント `#token=` から読み取り、ヘッダで送る。
- CORS: `http://127.0.0.1[:port]` / `http://localhost[:port]` の Origin のみ反射。
  preflight は `Authorization` ヘッダを許可する。
- `Host` ヘッダが loopback 以外なら 403（DNS リバインディング多層防御）。
- エンドポイント: `GET /healthz`, `GET /metrics`, `GET /api/v1/snapshot` の 3 つ（読み取り専用）。
  `/healthz` と `/metrics` は認証不要で、`/metrics` は Prometheus テキスト形式の
  カウンタ/ゲージのみを返し（マシンデータなし）、リスナー自体が loopback 限定。
  Bearer トークンが必要なのは `/api/v1/snapshot` のみ（[`DESIGN.md`](DESIGN.md) 6.3 節、
  [`SECURITY.md`](SECURITY.md) 参照）。

### 6.2 Glances / node_exporter との関係

- Glances REST API v4 は OpenAPI 仕様があり実績も豊富だが、`/all` は巨大で
  psutil 固有のフィールド体系。**取り込み専用の任意アダプタ**として将来対応する。
- node_exporter（Prometheus 形式）は pull 型の時系列メトリクス向けで、プロセス一覧には
  向かない。本実装では `/metrics` を Prometheus テキスト形式で実装済み（認証不要、
  カウンタ/ゲージのみ、loopback 限定）だが、プロセス一覧は `/api/v1/snapshot` が担う。

---

## 7. 採用判断まとめ

| 項目 | 決定 | 主な理由 |
|---|---|---|
| 新規実装 vs 既存利用 | 新規実装 | GPUI + wasm の既存タスクマネージャが事実上存在しない。既存は GPL/AGPL 中心 |
| UI | gpui-kit 0.9.1 | 部品・実績・ライセンス。完全ピンで運用 |
| 収集 | sysinfo 0.40.3 | クロスプラットフォーム・活発・実測で十分な速度 |
| シグナル | rustix 1.1.6 | unsafe なし。RUSTSEC 修正版固定 |
| wasm UI | gpui-pre + gpui-pre-web | stable でビルド可能。実ブラウザ描画を実証 |
| ブラウザのデータ | agent + デモ | ブラウザから `/proc` は不可。読み取り専用で安全側 |
| プロトコル | 独自 v1 + Glances 互換は将来 | サイズ・pull 型・DTO 二重管理を避ける |
| 既存コード | 不使用 | GPL/AGPL。仕様・UX・アーキテクチャのみ参考 |

---

## 8. 未確認事項

以下は調査時点（2026-11-14）の未確認事項を履歴として残す。うち 4 と 8 はその後解消済みのため、現状は各項目の注記と参照先を参照（R1-D02 反映、2026-11-18）。

1. WebGPU が実ブラウザ（Edge / Chrome、非ヘッドレス）で利用できるか。
2. Wayland 以外（X11 セッション）での GPUI 表示・入力。
3. Windows / macOS の実機ビルド・実行（sysinfo は対応、GPUI も CI では 3 OS 検証）。
4. IME による日本語入力（絞り込みは ASCII 前提）。
   **【解消済み（2026-11-15）】** ブラウザ版・ネイティブ版とも `gpui::EntityInputHandler` を実装し、変換中は未確定文字列を入力欄にのみ表示し、確定文字列だけを絞り込みへ反映する。ヘッドレス Chrome + CDP の 32 チェックすべて pass と `TM_DEBUG_IME=1` フックによる Runtime Proof を取得済み。実 IME（IBus/fcitx5）での手動確認は未実施で、同旨を [`README.md`](README.md) の制約事項へ記載。根拠: [`notes/ime.md`](notes/ime.md)。
5. `hidepid=1/2` 環境での表示制限の挙動。
6. GPUI 系スナップショットの独立セキュリティ監査（未実施）。
7. Glances 互換アダプタの実装コスト（未実装）。
8. ブラウザ agent の並列化・差分更新（現状はシリアルで 1 スナップショット約 250 ms）。
   **【並列化は解消済み（2026-11-16〜17）】** 接続を最大 8 本まで並列処理し、超過は即座に 503 を返す。サンプリングは共有プローブで直列のまま（1 スナップショット約 250 ms）だが、リクエストヘッダ待ちで他接続を阻塞しない。根拠: [`README.md`](README.md)、[`notes/current-state.md`](notes/current-state.md)、`agent/src/main.rs`。なお `offset` / 差分更新は未着手で、§5.3 の将来対応のまま（R1-D07）。

---

## 9. 参照

### 既存ソリューション

- btop: <https://github.com/aristocratos/btop>
- bottom: <https://github.com/ClementTsang/bottom>
- zenith: <https://github.com/bvaisvil/zenith>
- Mission Center: <https://gitlab.com/mission-center-devs/mission-center>
- Resources: <https://github.com/nalabelle/resources>
- GNOME System Monitor: <https://gitlab.gnome.org/GNOME/gnome-system-monitor>
- Plasma System Monitor: <https://invent.kde.org/plasma/plasma-systemmonitor>
- htop: <https://github.com/htop-dev/htop>
- Glances: <https://github.com/nicolargo/glances> / <https://glances.readthedocs.io/en/latest/api/restful.html>
- netdata: <https://github.com/netdata/netdata>
- Cockpit: <https://github.com/cockpit-project/cockpit>

### crate / 一次情報

- sysinfo: <https://crates.io/crates/sysinfo> / <https://github.com/GuillaumeGomez/sysinfo>
- rustix: <https://crates.io/crates/rustix> / <https://github.com/bytecodealliance/rustix>
- RUSTSEC-2026-0486: <https://rustsec.org/advisories/RUSTSEC-2026-0486>
- procfs: <https://crates.io/crates/procfs>
- heim: <https://crates.io/crates/heim>
- Glances OpenAPI: <https://raw.githubusercontent.com/nicolargo/glances/refs/heads/develop/docs/api/openapi.json>

### GPUI

- GPUI: <https://github.com/zed-industries/zed/tree/main/crates/gpui>
- gpui-kit: <https://github.com/longbridge/gpui-kit> / <https://gpui-kit.com>
- Argus（参考・AGPL）: <https://github.com/Ray-lea/Argus>

### 詳細ノート

- [`notes/solutions-survey.md`](notes/solutions-survey.md): 11 候補の詳細比較・プロトコル・GPUI 先行事例
- [`notes/crates-survey.md`](notes/crates-survey.md): crate 評価・sysinfo 制約・実測ログ全文
- [`notes/wasm-spike.md`](notes/wasm-spike.md): GPUI Web スパイクの手順・証拠
- [`DESIGN.md`](DESIGN.md): 設計書

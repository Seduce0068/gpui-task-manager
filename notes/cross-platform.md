---
type: Reference
title: Windows / macOS 対応と CI マトリクス（tm-platform クロスプラットフォーム化）
description: tm-platform を Windows / macOS でコンパイル可能にした変更点（rustix の Unix 限定化、シグナル送信の OS 分岐、スレッド数取得の Linux gate）と、CI への native-windows / native-macos / cargo-deny ジョブ追加、ローカル cross-check の Runtime Proof、未検証事項
tags: [cross-platform, windows, macos, sysinfo, rustix, taskkill, ci, cargo-deny, runtime-proof]
resource: work/gpui-task-manager
generated: 2026-11-15T00:25:12+09:00
status: draft
---

## 1. 結論（要約）

- `crates/tm-platform` を Windows / macOS でコンパイル可能にした。rustix は
  `[target.'cfg(unix)'.dependencies]` に限定し、シグナル送信は Unix（rustix）と
  Windows（`taskkill /F` を絶対パスで起動）に分岐した。
- Linux 専用だった tasks / threads 取得は `#[cfg(target_os = "linux")]` に gate し、
  他 OS は `threads: None`（UI は「—」表示）とした。
- CI に `native-windows` / `native-macos` / `deny` ジョブを追加した。既存の Linux
  gates と wasm ジョブ、`concurrency` / `permissions` は変更していない。
- ローカル検証: Windows は app（GPUI）を含む workspace 全体の `cargo check` が成功。
  macOS は GPUI の Metal シェーダ生成が Linux ホストでは動かないため app を含む
  workspace check は不可（`tm-core` / `tm-viewmodel` / `tm-platform` / `tm-agent`
  は成功）。実機相当の検証は GitHub Actions の windows-latest / macos-latest。
- `cargo deny --manifest-path Cargo.toml check` は成功。web ワークスペース側は
  `tm-web` の license 未設定と path 依存の wildcard で失敗中（Lead が
  `web/Cargo.toml` を修正予定）。

## 2. 変更点

### 2.1 `crates/tm-platform/Cargo.toml`

- `rustix` を `[target.'cfg(unix)'.dependencies]` へ移動した。rustix の `process`
  モジュールは `#[cfg(not(windows))]` で公開されており（rustix 1.1.6
  `src/lib.rs`）、Windows ビルドでは依存自体を外すのが正しい。
- `cargo tree --target x86_64-pc-windows-msvc -p tm-platform` に rustix が
  現れないこと、macOS ターゲットでは現れることを確認した。
- sysinfo の依存・features（`system` / `disk` / `network` / `user`）は変更なし。

### 2.2 シグナル送信（`NativeProbe::control`）

共通の安全規則は全 OS で維持している。

- PID 0 / PID 1 / 自分自身は `ControlError::Protected`。
- Unix のみ `pid > i32::MAX` を `NotFound`（Windows の PID は `u32`）。
- 送信直前に start time を再取得し、`ProcessKey` と一致しなければ `NotFound`。
- 上記を通過した後にプラットフォーム別の `control_process()` を呼ぶ。

`#[cfg(unix)]` の `control_process()`:

- rustix の `kill_process` で `SIGTERM` / `SIGKILL` / `SIGSTOP` / `SIGCONT` を送る。
- `Errno::PERM` → `PermissionDenied`、`Errno::SRCH` → `NotFound`、その他 → `Failed`。

`#[cfg(windows)]` の `control_process()`:

- Windows に POSIX シグナルはないため、`Terminate` と `Kill` はどちらも
  `taskkill /F`（強制終了）として扱う。`Stop` / `Continue` は
  `ControlError::Unsupported`。
- `taskkill.exe` は **`%SystemRoot%\System32\taskkill.exe` の絶対パス**で起動する
  （`SystemRoot` が未設定・空なら `C:\Windows\System32\taskkill.exe`）。`PATH`
  解決をしないことで PATH ハイジャックの余地をなくす。参考として、sysinfo の
  `Process::kill_with` は `Command::new("taskkill.exe")` で PATH 解決するため
  採用しなかった（sysinfo 0.40.3 `src/windows/process.rs`）。
- 引数は `/PID <pid> /F` のみ。シェルを経由せず、PID は数値文字列。
- `CREATE_NO_WINDOW`（`0x0800_0000`）を指定し、GUI アプリから起動しても
  コンソールウィンドウを出さない。
- 起動失敗・非ゼロ終了は `ControlError::Failed`（stderr を含める）。

`#[cfg(not(any(unix, windows)))]` のフォールバックは常に `Unsupported`。

### 2.3 tasks / threads の Linux gate

- `process_refresh_kind()` の `.with_tasks()` を `#[cfg(target_os = "linux")]` に
  gate した（他 OS では no-op のため）。
- `thread_count()` ヘルパーを追加し、Linux は `tasks().len() + 1`、それ以外は
  `None` を返す。
- ユーザーランドスレッドをプロセス一覧から除外するフィルタは全 OS でコンパイル
  可能（非 Linux では `thread_kind()` が常に `None` なので no-op）。

`tasks().len() + 1` の根拠と実測:

- sysinfo 0.40.3 の Linux 実装 `get_proc_tasks()` は `/proc/<pid>/task` を列挙し、
  主スレッド（`*pid != parent_pid`）を除外して `tasks` を作る。よって `+1` で
  `Threads:`（= `ps -o nlwp`）と一致する。
- 一時 example で 637 プロセスを走査し、`tasks().len() + 1` と
  `/proc/<pid>/status` の `Threads:` を比較した結果、**不一致 0**。
  例: pid 6977 は 171 スレッド、pid 8545 は 65 スレッドで一致した。
  別時刻の `ps -o nlwp` とも一致を確認（pid 6977: 175 = `/proc` Threads 175）。
- 検証用 example は確認後に削除し、コミットには含めていない。

### 2.4 `ProcessStatus` とその他 sysinfo API の OS 差

- `sysinfo::ProcessStatus` は全 variant を全プラットフォームで宣言しており、
  variant に `cfg` が付いていない（sysinfo 0.40.3 `src/common/system.rs`）。
  そのため既存の網羅 `match` は Windows / macOS でもそのままコンパイルできる。
  cfg 分岐は不要と判断し、根拠をコメントに残した。
- `System::load_average()` / `Cpu::frequency()` / `Users` は対応 OS すべてで API が
  提供される（`Users` は feature `user` で有効、未対応 OS は空リスト）。fallback は
  不要と判断。macOS で frequency が 0 の場合は既存の
  `average_frequency_mhz()` が `None` を返す。
- `process.user_id()` と `Users::get_user_by_id()` は Windows でも型検査できる
  （実行時の SID 解決は未検証）。

### 2.5 テストの OS 別 gate

- 子プロセス spawn を使うテスト（`sleep` / `true`）と `ChildGuard` /
  `wait_for_process` は `#[cfg(unix)]`。
- `control_rejects_a_stale_start_time` と `terminate_ends_a_child_process` は
  `#[cfg(target_os = "linux")]` から `#[cfg(unix)]` に拡大し、macOS CI でも実行する。
- `sample_reports_a_plausible_machine` / `two_samples_keep_cpu_percentages_in_range`
  / `platform_info_describes_the_native_backend` / PID 保護・範囲外 PID のテストは
  全 OS で実行する。

### 2.6 `.github/workflows/ci.yml`

```yaml
  native-windows:
    name: native check (windows-latest)
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Check workspace
        run: cargo check --workspace
      - name: Test portable crates
        run: cargo test -p tm-core -p tm-viewmodel

  native-macos:
    name: native check (macos-latest)
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Check workspace
        run: cargo check --workspace
      - name: Test portable and platform crates
        run: cargo test -p tm-core -p tm-viewmodel -p tm-platform

  deny:
    name: cargo-deny (licenses / advisories / duplicates)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: taiki-e/install-action@cargo-deny
      - name: Check native workspace
        run: cargo deny --manifest-path Cargo.toml check
      - name: Check web workspace
        run: cargo deny --manifest-path web/Cargo.toml check
```

- `native-windows` / `native-macos` は app（GPUI）を含む workspace 全体を型検査し、
  OS 固有の cfg 分岐が壊れていないことを確認する。macOS では spawn 系の
  `tm-platform` テストも実行する。
- `deny` は `cargo-deny 0.20.2` の `--manifest-path` をグローバルオプションとして
  使い、ルートと web の 2 ワークスペースを検査する。ポリシーは Lead が用意した
  ルートの `deny.toml`。

## 3. ローカル検証（Runtime Proof）

### 3.1 環境

| 項目 | 値 |
| --- | --- |
| ホスト | Kubuntu 2026.04 / KDE Plasma 6 / Wayland |
| rustc | 1.99.1 (58cf8f9c3 2026-10-13) |
| cargo | 1.99.1 (5f94df478 2026-08-27) |
| cargo-deny | 0.20.2 |
| クロスターゲット | `x86_64-pc-windows-msvc` / `aarch64-apple-darwin`（`rustup target add` で追加） |
| sysinfo / rustix | 0.40.3 / 1.1.6（完全固定） |

### 3.2 結果

| # | コマンド | 結果 |
| --- | --- | --- |
| 1 | `cargo check --target x86_64-pc-windows-msvc -p tm-core -p tm-viewmodel -p tm-platform -p tm-agent` | 成功（2.43s） |
| 2 | `cargo check --target aarch64-apple-darwin -p tm-core -p tm-viewmodel -p tm-platform -p tm-agent` | 成功（2.93s） |
| 3 | `cargo check --target <両ターゲット> -p tm-platform --tests` | 成功 |
| 4 | `cargo clippy --target <linux/windows/macos> -p tm-platform --all-targets -- -D warnings` | 成功（3 ターゲットとも） |
| 5 | `cargo check --target x86_64-pc-windows-msvc --workspace` | 成功（app/GPUI 含む。初回 1m18s、taskkill 変更後 1.32s） |
| 6 | `cargo check --target aarch64-apple-darwin --workspace` | 失敗（環境制約。3.3 参照） |
| 7 | `cargo check --workspace`（Linux） | 成功 |
| 8 | `cargo test --workspace`（Linux） | 成功（118 tests: app 6 / agent 39 / core 49 / platform 9 / viewmodel 15） |
| 9 | `cargo test -p tm-platform`（Linux） | 成功（9 tests） |
| 10 | `cargo fmt -p tm-platform -- --check` | 成功 |
| 11 | `cargo deny --manifest-path Cargo.toml check` | 成功（advisories / bans / licenses / sources すべて ok） |
| 12 | `cargo deny --manifest-path web/Cargo.toml check` | 失敗（exit 6。2.6 の注記参照） |
| 13 | `cargo tree --target x86_64-pc-windows-msvc -p tm-platform` | rustix なし（0 件）。macOS ターゲットでは 1 件 |

### 3.3 macOS workspace check がローカルで失敗する理由

`cargo check --target aarch64-apple-darwin --workspace` は `gpui-pre-apple 0.3.12`
で失敗する。

```text
error: couldn't read .../target/aarch64-apple-darwin/debug/build/gpui-pre-apple-*/out/stitched_shaders.metal:
       No such file or directory
  --> gpui-pre-apple-0.3.12/src/metal_renderer.rs:42:35
```

`gpui-pre-apple` の `build.rs` は
`#[cfg(any(target_os = "macos", target_os = "ios"))] fn main()` となっており、
クロスコンパイル時は build script が **ホスト（Linux）向け**にコンパイルされるため
`main()` が空になり、Metal シェーダの生成物が作られない。これはコード側の問題では
なく、macOS 実機（Xcode の Metal ツールチェーン）でのみ解消できる環境制約である。
したがって app（GPUI）を含む macOS 検証は CI の macos-latest を実機相当とする。
GPUI に依存しない `tm-core` / `tm-viewmodel` / `tm-platform` / `tm-agent` は
macOS ターゲットで成功している。

### 3.4 `cargo deny` の web ワークスペースについて

`cargo deny --manifest-path web/Cargo.toml check` は次の 2 件で失敗する（exit 6）。

- `error[unlicensed]: tm-web = 0.1.0 is unlicensed`
  （`web/Cargo.toml` に `license` がない）
- `error[wildcard]: found 2 wildcard dependencies for crate 'tm-web'`
  （`tm-core` / `tm-viewmodel` の path 依存に `version` がない）

`web/Cargo.toml` と `deny.toml` は Lead のスコープのため、本タスクでは変更して
いない。Lead が `web/Cargo.toml` を修正した後に再実行して確認する。

## 4. 未検証事項

1. **Windows 実機 / CI での実行**: サンプリング値（CPU・メモリ・ネットワーク・
   ディスク）、`taskkill /F` の権限挙動（管理者権限が必要なプロセスは失敗する）、
   `Terminate` が強制終了になること、ユーザー列の SID 解決、GPUI アプリの起動。
2. **macOS 実機 / CI での実行**: `threads: None`（「—」表示）、`frequency_mhz` が
   `None` になる可能性、`Users` の解決、`SIGSTOP` / `SIGCONT`、spawn 系テスト
   （`sleep` と SIGTERM）が macos-latest で通るか。
3. **GPUI app の macOS コンパイル**: ローカルでは不可（3.3）。CI の
   macos-latest が初回確認になる。
4. **CI 3 ジョブ**: push 後に初回実行される。ローカルで確認できたのは
   Windows の workspace check と cargo-deny（ルート）まで。
5. **web の cargo-deny**: Lead の `web/Cargo.toml` 修正後に再実行する。
6. **Windows の `control_reports_not_found_for_out_of_range_pid`**: Windows で
   `u32::MAX` を渡すテストは CI が初実行（ローカルは Linux のみ）。

## 5. 既知の制約

- Windows は `Stop` / `Continue` 非対応（`ControlError::Unsupported`）。
  `Terminate` も graceful ではなく `taskkill /F` の強制終了で、POSIX の SIGTERM と
  は意味が異なる。
- Windows の `taskkill` は対象プロセスの権限に依存する。管理者権限が必要な
  プロセスへの操作は `ControlError::Failed` になる。
- macOS / Windows は `threads: None`（スレッド数を取得できない）。
- `ProcessStatus` は sysinfo が全 variant を全 OS で宣言するため cfg 分岐は不要。
  将来 sysinfo が variant を cfg 化した場合は再検討が必要。
- ネットワークの `lo` 除外は Linux の IF 名を前提にした判定で、Windows / macOS では
  一致しない（実害はないが、仮想 IF の除外は未実装）。
- Windows の PID 上限チェック（`pid > i32::MAX`）は Unix 専用。Windows は `u32`
  全体を許容する。
- `taskkill` を起動するため、プロセス操作 1 回ごとに子プロセスが生成される
  （sysinfo の `kill_with` と同じ特性）。

## 6. 参照

- `crates/tm-platform/src/lib.rs` — `control_process()` / `taskkill_path()` /
  `thread_count()` / `process_refresh_kind()`
- sysinfo 0.40.3 — `src/windows/process.rs`（`kill_with` の `taskkill.exe` 呼び出し）、
  `src/unix/linux/process.rs`（`tasks` 収集と主スレッド除外）、
  `src/common/system.rs`（`ProcessStatus` / `Process::tasks` / `thread_kind` /
  `load_average`）、`src/macros.rs`（`declare_signals`）
- rustix 1.1.6 — `src/lib.rs` の `#[cfg(not(windows))] pub mod process;`
- `DESIGN.md` §2（依存方向）、§14（既知の制限）
- `.github/workflows/ci.yml`

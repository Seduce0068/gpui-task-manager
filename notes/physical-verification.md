---
type: Playbook
title: Windows / macOS 物理実機での検証方法（2026-11-15 調査）
description: GPUI タスクマネージャーを物理 Windows / macOS 実機で検証するための選択肢（自前ハード、クラウド Mac/GPU VM、CI ランナー）と、費用・手順・チェックリスト・注意点を一次情報で整理した実務メモ
tags: [verification, windows, macos, physical-machine, cloud-mac, gpu-vm, ci, gpui]
resource: work/gpui-task-manager
generated: 2026-11-15T01:20:00+09:00
status: draft
---

## 1. 結論（先に要点）

- **物理実機の代わりに「実機相当」をどこまで担保できるか**を分けて考える。
  - ビルド・型検査・単体テスト: **GitHub Actions の windows-latest / macos-latest で担保済み**
    （本リポジトリの CI run `43612670190` で両方成功。macOS は arm64 ランナー）。
  - GPU 描画・ウィンドウ・IME・プロセス操作の**実行時挙動**: CI ランナーでは検証できない。
    物理実機または GPU 付きクラウド VM が必要。
- **Windows の最短ルート**: 既存 PC に Windows 11 を**別ディスクでネイティブ導入**するのが
  最も忠実（Vulkan + MS-IME + 実プロセス操作）。難しければ Azure の GPU VM
  （NVadsA10_v5、NVIDIA A10 + GRID ライセンス）を RDP で使う。
- **macOS の最短ルート**: Apple Silicon の Mac mini を購入/借用するのが最も確実。
  短期なら **MacStadium の M4 Mac mini（$149/月〜）**、時間単位に近い検証なら
  **AWS EC2 Mac（mac-m4.metal、Dedicated Host、最低 24 時間）**。
- **Linux ホスト上の Windows/macOS VM は推奨しない**。Windows VM は GPU パススルーなしでは
  Vulkan がソフトウェア描画になり GPUI の実行時検証にならない。macOS VM は Apple 製
  ハードウェア前提で、本機（Ryzen/Linux）では実用的でない。
- 検証時は `--demo` と `--redact` を使い、**自分で起動した子プロセス以外は kill しない**。
  クラウドは使い捨てアカウント/リソースで行い、終了時に必ず停止・削除する。

## 1.1 無料でできる範囲（2026-11-15 確認）

| 対象 | 無料でできるか | 方法・条件 |
|---|---|---|
| Windows 物理実機 | **できる** | Microsoft Evaluation Center の **Windows 11 Enterprise 評価版（90 日）**。プロダクトキー不要で ISO をダウンロードでき、既存 PC の空きディスクにクリーンインストールして検証できる。90 日経過後はデスクトップが黒くなり毎時シャットダウンされるため、評価用途に限定する。 |
| Windows（VM） | できるが GPU 検証の代替にならない | 同じ評価版 ISO を VM に入れることは可能。ただし GPU パススルーなしでは Vulkan がソフトウェア描画になり、GPUI の実行時検証にはならない。 |
| Windows（クラウド GPU） | **無料では難しい** | Azure/AWS の GPU VM は時間課金。無料クレジットは期間・金額限定。 |
| macOS 物理実機 | **借用できれば無料** | 知人・家族・勤務先の Mac を借りる。新規購入は有料。 |
| macOS クラウド | **無料枠なし** | MacStadium / AWS EC2 Mac は有料（MacStadium M4.S $149/月〜、AWS は最低 24 時間）。 |
| macOS CI（ビルド・テスト） | 条件付きで無料 | GitHub Actions の標準ランナーは**public リポジトリでは無料**。private リポジトリは GitHub Free の月額 $12 分の included compute 内なら追加課金なし（macOS は $0.062/分、Windows は $0.010/分、Linux は $0.006/分）。**GUI・IME・プロセス操作は不可**。 |

- 本リポジトリは private。CI 1 回あたりの概算は Linux 約 6 分（$0.04）+ Windows 約 5 分（$0.05）+ macOS 約 2 分（$0.12）≒ **$0.2/回**で、GitHub Free の $12/月の範囲なら月 50 回程度まで無料。
- **無料で「全部」はできない**: macOS の対話 GUI/IME を無料で新規に用意する公式ルートはない。
  現実的な無料構成は「Windows は評価版で物理検証、macOS は借用 or CI でビルド/テストのみ」。

## 2. 現在どこまで検証済みか（2026-11-15 時点）

| 検証層 | 内容 | 状態 |
|---|---|---|
| Linux 実機 | Wayland 起動、プロセス一覧、シグナル操作、IME 以外の UI | 検証済み |
| Windows 型検査 | `cargo check --target x86_64-pc-windows-msvc --workspace`（app/GPUI 含む） | 成功（ローカル） |
| Windows CI | windows-latest で workspace check + portable tests | **成功**（run 43612670190、4m49s） |
| macOS 型検査 | core / viewmodel / platform / agent の cross-check | 成功（app は Linux ホストで Metal シェーダ生成不可） |
| macOS CI | macos-latest（arm64）で workspace check + tests | **成功**（run 43612670190、1m11s） |
| ブラウザ | デモ / ライブ / IME を CDP で検証 | 検証済み |
| **物理 Windows/macOS** | GUI 描画、IME、プロセス操作、HiDPI、パッケージ | **未検証** |

## 3. Windows の選択肢

### 3.1 推奨: 既存 PC へのネイティブ Windows 11（別ディスク）

- 本機は Ryzen 5 3600 + GTX 1660 SUPER + SSD/HDD/USB SSD 構成。**空きの内部 SSD/HDD に
  Windows 11 をクリーンインストール**し、BIOS/UEFI のブートメニューで切り替える。
- 公式 ISO: Microsoft の [Windows 11 ダウンロード](https://www.microsoft.com/ja-jp/software-download/windows11)
  （2026-11-15 時点の現行リリースは **Windows 11 2026 更新 / バージョン 26H2**、ライセンス必要）。
- ビルド要件（Zed の Windows ビルド手順より）: rustup、Visual Studio または Build Tools の
  「Desktop development with C++」、Windows SDK、CMake。GPUI/Zed は Windows で
  **Vulkan** をグラフィックス API として使うため、NVIDIA の最新ドライバと Vulkan ランタイムが必要。
- 手順の概要:
  1. 重要データをバックアップし、既存 Linux ディスクを物理的に外すか、インストール先を
     明確に分離する（ブートローダー破損防止）。
  2. Windows 11 ISO から USB メディアを作成してインストール。NVIDIA ドライバを導入。
  3. Rust stable + VS Build Tools（C++/Windows SDK/CMake）を導入。
  4. `cargo build -p gpui-task-manager` → `.\target\debug\gpui-task-manager.exe --demo` で起動。
  5. 実データ起動、プロセス一覧、`sleep` 相当の子プロセスを自分で起動して Terminate、
     Ctrl+F で MS-IME の日本語入力、リサイズ/HiDPI を確認。
- 注意: ディスク選択・ブートローダー・ライセンス・BitLocker/セキュアブート。**破壊的操作の
  前に必ずバックアップ**。

### 3.2 クラウド GPU Windows VM（RDP）

- **Azure NVadsA10_v5**: NVIDIA A10 GPU（1/6〜2 GPU）＋ GRID ライセンス。GPU 加速の
  仮想デスクトップ/グラフィックス向けで、Windows VM を RDP で操作できる。
  <https://learn.microsoft.com/en-us/azure/virtual-machines/sizes/gpu-accelerated/nvadsa10v5-series>
- **AWS EC2 G4dn（Windows）**: NVIDIA T4 の GPU インスタンス。Windows AMI + RDP で利用可能。
  <https://aws.amazon.com/ec2/instance-types/g4/>
- 長所: 物理機を用意せず GPU 付き Windows を試せる。短所: RDP セッション特有の表示、
  GPU ドライバ/GRID 設定、時間課金。**料金はリージョン・サイズで変動するため公式の
  料金計算ツールで確認**（本ノートでは断定しない）。

### 3.3 その他

- 中古の Windows ノート/デスクトップを用意する（最も安価で実機に近い）。
- Windows 11 VM を Linux 上で動かす場合、GPU パススルー（VFIO）なしでは Vulkan が
  ソフトウェア実装になり、GPUI の描画検証にはならない。GPU パススルーは単一 GPU 構成では
  ホストのグラフィックスを停止する必要があり、本機では推奨しない。

## 4. macOS の選択肢

### 4.1 推奨: Apple Silicon Mac mini の購入/借用

- Apple 公式の Mac mini（2026 年時点は M6 / M5 Pro 搭載モデル）。
  <https://www.apple.com/jp/mac-mini/>
- ビルド要件（Zed の macOS ビルド手順より）: Xcode（Metal Toolchain を含む）+
  Command Line Tools + CMake。macOS 26 SDK 以上でバンドル可能、生成アプリは macOS 12 以降。
  <https://github.com/zed-industries/zed/blob/main/docs/src/development/macos.md>
- 日本語 IME（macOS の日本語入力）をそのまま試せる。GUI・Metal 描画・プロセス操作の
  実機検証に最も適する。価格は Apple ストアの構成次第（本ノートでは断定しない）。

### 4.2 クラウド Mac（短期・月単位）

- **MacStadium**: M4 Mac mini の月額が公開されている（2026-11-15 取得）。
  - M4.S（M4 10コア / 16GB / 256GB SSD）: **$149/月**
  - M4.M（M4 10コア / 24GB / 512GB SSD）: **$249/月**
  - 上位: M4.L $349/月、Mac Studio もあり。月単位・前払い。VDI/リモートデスクトップも提供。
  <https://macstadium.com/pricing>
- **AWS EC2 Mac**: Dedicated Host 単位の課金で、**最低 24 時間**の割り当てが必要。
  M4 Mac は `mac-m4.metal`（M4 10コア / 10 GPU / 24GiB）。SSH または Apple Remote Desktop
  で接続でき、起動完了まで約 6〜20 分。Spot/Reserved は不可、On-Demand のみ。
  <https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/ec2-mac-instances.html>
- **GitHub Actions の macos-latest（arm64）**: ビルドと単体テストは既に成功しているが、
  対話的 GUI・IME・プロセス操作の検証には使えない。

### 4.3 非推奨

- Linux ホスト上の macOS VM（QEMU/UTM 等）は Apple 製ハードウェア前提のため、
  本機では実用的でない。Hackintosh はサポート外で、検証の再現性もない。

## 5. 推奨プラン（予算・目的別）

| 目的 | Windows | macOS |
|---|---|---|
| 最小コストで一度だけ実機確認 | 既存 PC の空きディスクに Windows 11（ISO は無料、ライセンスは既存/購入） | 知人/借用の Mac、または MacStadium M4.S を 1 か月（$149） |
| 継続的に開発・リリース検証 | 既存 PC デュアルブート + CI | Mac mini 購入（長期的に最も安価になり得る） |
| 短期に GPU/IME を重点確認 | Azure NVadsA10_v5 の Windows VM（時間課金） | AWS EC2 Mac `mac-m4.metal`（最低 24 時間） |
| ビルド回帰のみ | GitHub Actions windows-latest（導入済み） | GitHub Actions macos-latest arm64（導入済み） |

## 6. 実機チェックリスト（Windows / macOS 共通）

1. `cargo build -p gpui-task-manager`（release も）が成功する。
2. `--demo` でウィンドウが表示され、概要カード・プロセス表・詳細パネルが描画される。
3. 実データ起動でプロセス数が OS のツール（タスク マネージャー / `ps`）と大きく乖離しない。
4. GPU バックエンド: Windows は Vulkan、macOS は Metal で初期化できる（ログ/エラー確認）。
5. 自分で起動した子プロセス（例: `timeout` / `sleep`）を選択し、確認ダイアログ経由で
   Terminate できる。PID 1 / 自分自身は拒否される。
6. Ctrl+F で **日本語 IME** の変換・確定・Esc 復帰ができる（Windows は MS-IME、
   macOS は日本語入力）。変換中に一覧が絞り込まれないこと。
7. ウィンドウのリサイズ、HiDPI、スクロール、列ソート、絞り込みが破綻しない。
8. `--redact` でスクリーンショットを取得し、ホスト名・ユーザー名・ホームパスが伏せられる。
9. 終了時にクラッシュログ・GPU エラーが残らない。
10. クラウド VM は検証後に停止・削除し、課金を止める。

## 7. 費用・期間の目安（取得できた一次情報のみ）

| 項目 | 値 | 出典 |
|---|---|---|
| MacStadium M4.S Mac mini | $149/月（M4 10コア / 16GB / 256GB） | MacStadium Pricing（2026-11-15 取得） |
| MacStadium M4.M Mac mini | $249/月（M4 10コア / 24GB / 512GB） | 同上 |
| AWS EC2 Mac | Dedicated Host 単位、最低 24 時間。`mac-m4.metal`（M4 10コア / 10 GPU / 24GiB） | AWS EC2 Mac ドキュメント（2026-11-15 取得） |
| Windows 11 ISO | 公式ダウンロード可（ライセンスは別途） | Microsoft Windows 11 ダウンロード（2026-11-15 取得） |
| Azure NVadsA10_v5 | NVIDIA A10 + GRID ライセンス。料金はリージョン/サイズで変動 | Microsoft Learn（2026-11-15 取得） |

- AWS EC2 Mac の**実際の料金**、Azure GPU VM の**実際の時間単価**、Apple Mac mini の
  **日本価格**は、取得時点のページで動的に変わるため本ノートでは断定しない。
  見積もりは各公式の料金ページ/計算ツールで確認すること。

## 8. 未確認事項

1. 物理 Windows 実機での GPUI/Vulkan 描画（GTX 1660 SUPER + NVIDIA proprietary driver）。
2. 物理 macOS 実機での Metal 描画と日本語 IME の挙動。
3. Windows の `taskkill /F` による Terminate の権限挙動（管理者権限が必要なプロセス）。
4. macOS の SIGSTOP/SIGCONT、プロセス一覧の `threads: None` 表示。
5. Windows/macOS の実機での HiDPI・ウィンドウ装飾・パッケージ（DEB/MSI/DMG）。
6. クラウド GPU VM の RDP セッションで GPUI が期待どおり描画されるか。
7. Azure/AWS/Apple の実際の料金（変動するため）。

## 9. 参照（2026-11-15 取得）

- Microsoft: Windows 11 Enterprise 評価版（90 日、無料）
  <https://www.microsoft.com/en-us/evalcenter/evaluate-windows-11-enterprise>
- Microsoft: Windows 11 のダウンロード <https://www.microsoft.com/ja-jp/software-download/windows11>
- Microsoft Learn: NVadsA10_v5 sizes series
  <https://learn.microsoft.com/en-us/azure/virtual-machines/sizes/gpu-accelerated/nvadsa10v5-series>
- AWS: Amazon EC2 Mac instances（User Guide）
  <https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/ec2-mac-instances.html>
- AWS: EC2 G4 instances <https://aws.amazon.com/ec2/instance-types/g4/>
- MacStadium: Pricing <https://macstadium.com/pricing>
- Apple: Mac mini <https://www.apple.com/jp/mac-mini/>
- GitHub Docs: GitHub Actions billing（Free の included compute は $12/月）
  <https://docs.github.com/en/billing/concepts/product-billing/github-actions>
- GitHub Docs: Actions runner pricing（Linux $0.006 / Windows $0.010 / macOS $0.062 per minute）
  <https://docs.github.com/en/billing/reference/actions-runner-pricing>
- GitHub Docs: GitHub-hosted runners
  <https://docs.github.com/en/actions/reference/runners/github-hosted-runners>
- Zed: Building Zed for Windows
  <https://github.com/zed-industries/zed/blob/main/docs/src/development/windows.md>
- Zed: Building Zed for macOS
  <https://github.com/zed-industries/zed/blob/main/docs/src/development/macos.md>
- 本リポジトリの CI（windows-latest / macos-latest 成功）
  <https://github.com/Seduce0068/gpui-task-manager/actions/runs/43612670190>

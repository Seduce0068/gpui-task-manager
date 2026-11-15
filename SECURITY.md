---
type: Policy
title: GPUI Task Manager セキュリティポリシー
description: 試作版 GPUI タスクマネージャーの安全原則・脅威モデル・安全な使い方・脆弱性報告・残余リスク
tags: [security, policy, task-manager, gpui, rust]
status: draft
generated: 2026-11-14T22:35:00+09:00
verified: 2026-11-14T22:35:00+09:00
---

## 1. 適用範囲とサポート

本ドキュメントは `gpui-task-manager` 試作（`work/gpui-task-manager/`）に適用する。
**試作段階のため、セキュリティ修正のサポート期間は定めていない**。公開リポジトリ化する
際は、サポート対象バージョンと連絡先をこのファイルに追記すること。

## 2. 脆弱性の報告

- 公開リポジトリでは **GitHub Security Advisory（private vulnerability reporting）** を使う。
- 公開 issue に、悪用可能な詳細・ローカルのホスト名・トークン・ログを含めない。
- 報告には、再現手順、影響範囲、想定される攻撃条件、可能なら修正案を含める。
- 本試作は「ローカル単一ユーザーのデスクトップ監視」を前提とする。リモート公開・
  マルチユーザー運用は想定外であり、その構成での脆弱性報告は設計変更として扱う。

## 3. 安全原則（コードで強制している不変条件）

1. **`unsafe` 禁止**: 全 crate で `#![forbid(unsafe_code)]`。OS 操作は rustix の安全 API。
2. **最小権限**: setuid しない。root 権限を要求しない。EPERM は明示的なエラーにする。
3. **危険操作の多層防御**:
   - PID 0 / PID 1 / 自分自身は viewmodel と platform の両方で拒否する。
   - プロセス同一性は `(pid, start_time)`。シグナル送信直前に起動時刻を再取得して照合し、
     不一致なら `NotFound` として送信しない（PID 再利用対策）。
   - シグナル操作は必ず確認ダイアログを経由する。**Enter はキャンセル**、
     Ctrl+Enter または明示クリックで確定する。
   - ダイアログには PID・ユーザー・起動からの経過・コマンドを表示し、対象の取り違えを防ぐ。
4. **読み取り専用ブラウザ**: ブラウザ版にシグナル操作を置かない。エージェントにも操作
   API を置かない。UI は `supports_process_control = false` でボタンを無効化する。
5. **ループバック限定のエージェント**:
   - `127.0.0.1` のみ bind（`--listen` はループバック以外を拒否）。
   - 認証は `Authorization: Bearer` のみ。クエリ文字列のトークンは拒否する
     （URL 履歴・Referer・スクリーンショットからの漏洩対策）。
   - `Host` ヘッダが loopback 以外なら 403（DNS リバインディング多層防御）。
   - CORS は loopback オリジンのみ反射。ヘッダ上限 8 KiB、読み取りタイムアウト 1 秒。
   - トークンは起動時生成（`/dev/urandom`）または `--token-file` / `TM_AGENT_TOKEN`。
     `--token` は argv 経由で他プロセスから見えるため警告を出す。
6. **表示の安全性**: OS 由来の文字列（プロセス名・コマンド・ユーザー名・ホスト名・
   マウントポイント等）は制御文字・ANSI エスケープ・Unicode bidi 制御文字を除去し、
   文字数で切り詰める。
7. **秘密情報を保持しない**: 認証情報・DB を保存しない。トークンはメモリ上と
   起動ログ（1 回）のみ。永続化するのは**表示設定だけ**
   （更新間隔・ソート・詳細パネル）で、プロセス情報・フィルタ・ホスト名・
   ユーザー名はディスクに書かない。ネイティブの設定ファイルは原子的に書き込み、
   Unix では `0600` で作成する。ブラウザは同じスキーマを `localStorage` に保存し、
   アクセスできない場合は既定値へ縮退する（パニックしない）。
8. **公開物の衛生**: スクリーンショット・証拠 JSON は `--redact` を使い、ホスト名・
   ユーザー名・ホームパス・コマンド引数内のホスト名 / tailnet ドメインを伏せる。
9. **入力（IME）**: ブラウザ版・ネイティブ版とも composition 中の未確定文字列を
   アプリ状態に反映せず、確定文字列だけを絞り込みに渡す。確定文字列はユーザー自身の
   入力であり、OS 由来の表示文字列とは別扱いとする。
10. **クロスプラットフォーム**: Windows は `%SystemRoot%\System32\taskkill.exe` の
    絶対パス起動のみ（PATH 解決をしない）。Stop/Continue は非対応。macOS は rustix の
    安全 API を使う。

## 4. 脅威モデル（要約）

| 資産 | 信頼境界 | 主な脅威 | 対策 |
|---|---|---|---|
| プロセス/センサー情報 | カーネル → アプリ | 悪意あるプロセス名・コマンドライン・コンポーネントラベルによる表示破壊 | サニタイズ・文字数制限・truncate |
| シグナル送信 | アプリ → 他プロセス | 誤操作・PID 再利用による誤殺 | 確認ダイアログ・PID+起動時刻照合・PID 0/1/self 拒否 |
| ブラウザ UI | エージェント → ブラウザ | トークン漏洩・CORS 誤設定・DNS リバインディング | Bearer ヘッダ・フラグメント・Host 検証・loopback CORS |
| 依存クレート | crates.io → ビルド | 供給網攻撃・未保守 crate | 完全ピン・`Cargo.lock`・`cargo audit` |

## 5. 安全な使い方（推奨設定）

```bash
# ネイティブ版（危険操作は確認ダイアログ経由）
./target/debug/gpui-task-manager

# エージェント: トークンはファイルか環境変数で渡す（--token はテスト用）
printf '%s' "$(openssl rand -hex 32)" > ~/.config/tm-agent.token
chmod 600 ~/.config/tm-agent.token
./target/debug/tm-agent --listen 127.0.0.1:18787 --token-file ~/.config/tm-agent.token

# 公開用スクリーンショットは redact
./target/debug/gpui-task-manager --redact
./target/debug/tm-agent --once --redact
```

- エージェントを `0.0.0.0` やリモートに公開しない（現状 TLS / リモート認証は未実装）。
- トークンを URL クエリ・シェル履歴・共有ドキュメントに残さない。
- ブラウザ版は読み取り専用であることを前提に運用する。

## 6. 依存関係ポリシー

- GPUI 系（`gpui-kit` / `gpui-pre*`）、`sysinfo`、`rustix` は**完全一致ピン**。
  `Cargo.lock` をコミットする。
- `rustix` は RUSTSEC-2026-0486 の修正版 `=1.1.6` に固定。
- `cargo audit` と `cargo-deny`（advisories / bans / licenses / sources）を CI と
  ローカル `scripts/check.sh` で実行する（root / web）。既知の未保守・重複は
  `deny.toml` で理由付きに管理する。
- **SBOM**: `scripts/generate-sbom.sh`（CycloneDX JSON, root/web）と CI の `sbom`
  ジョブで供給網可視性を確保する。
- **cargo-vet**: 依存監査（`cargo vet` / `supply-chain/`）は導入候補だが、
  現時点では未導入。trust root（監査者/インポート元）の選定、既存依存の棚卸し、
  監査/免除の運用責任を文書化してから初期化する。未監査依存を「監査済み」と
  して記録しない（cargo-vet の監査記録を捏造しない）。導入時は CI ゲート化と
  更新手順（Dependabot/ピン更新時の vet 対応）も合わせて定義する。
- Dependabot（Cargo ルート / web、GitHub Actions、週次）とセキュリティアラート /
  自動修正を有効化する。GPUI 系は完全ピン運用のため自動更新の対象外。
- 依存追加は最小限とし、ライセンス（Apache-2.0 互換）と保守状況を確認する。
- **運用制約**: private リポジトリのプラン制約により GitHub ブランチ保護 API は
  `403`（GitHub Pro または public が必要）で未設定。公開/プラン変更までは
  必須 CI・レビュー運用で `main` を保護する。

## 7. 残余リスク

- PID の `start_time` は秒精度のため、同一秒内の PID 再利用や、再取得から
  `kill_process` までの TOCTOU を完全には排除できない。`pidfd` による原子化は将来課題。
- GPUI 系は個人アカウントが crates.io に再公開するスナップショットで、bus factor = 1。
  独立したセキュリティ監査は未実施。
- ブラウザのトークンは URL フラグメントから読み取るため、画面共有・スクリーンショット・
  同一ユーザーの他プロセスには露出し得る。読み取り専用 API と loopback 限定で影響を抑える。
- ネイティブ版 IME は実装済みだが実 IME（IBus/fcitx5）での手動確認は未実施。実 GPU の WebGPU、
  Safari / Firefox、Windows / macOS の物理実機は未検証（Windows/macOS は CI ランナーで
  型検査・単体テストを実施）。
- Windows のシグナル送信は `taskkill /F` による強制終了で、POSIX の SIGTERM/SIGSTOP
  とは意味が異なる。
- `--redact` は既知の識別子（ホスト名・ユーザー名・ホームパス・tailnet ドメイン）を
  置換するが、ゼロ幅文字・同形異義文字によるなりすましは完全には防がない。

## 8. 検証

```bash
cd work/gpui-task-manager
bash scripts/check.sh          # fmt / clippy / test 143 件 / web 7 件 / wasm / cargo-deny
cargo audit
```

安全関連の主なテスト:

- `tm-platform`: PID 0/1/self の拒否、stale start_time の拒否（子プロセスが生存したまま）、
  範囲外 PID、reap 済み PID、spawn した子の SIGTERM。
- `tm-viewmodel`: 確認前は Effect を出さない、確認後にのみ Signal を出す、PID 1/self の拒否、
  Kill シグナルの保持、対象の identity 捕捉。
- `gpui-task-manager`: Enter=キャンセル / Ctrl+Enter=確定、その他キーの無視。
- `tm-core`: 制御文字・ANSI・bidi の除去、500 ケースの決定論的サニタイズ検査。
- `tm-agent`: クエリトークン拒否、Host 検証、Bearer 認証、CORS、`--token-file`、
  `--redact`（コマンド引数内ホスト名・tailnet ドメインを含む）。
- `tm-viewmodel`: `FilterInput` の UTF-16 / selection / marked range の状態機械
  （Web/ネイティブ共有、28 件）。`web` は agent 4 件。
  IME は CDP の composition 32 チェック + ネイティブ `TM_DEBUG_IME=1` 検証
  （確定前は反映しない）。
- クロスプラットフォーム: Windows ターゲットの workspace check、macOS 4 crate の
  check、CI の windows-latest / macos-latest。
- `app`: 設定ファイルの往復・原子的書き込み・`0600` 権限・範囲クランプ・
  未知フィールド無視（12 件）。デバッグフック `TM_DEBUG_SETTINGS=1` で保存経路を
  実行時確認し、設定ファイルを置いた起動で復元を確認。

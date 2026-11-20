---
type: Policy
title: GPUI Task Manager セキュリティポリシー
description: 試作版 GPUI タスクマネージャーの安全原則・脅威モデル・安全な使い方・脆弱性報告・残余リスク
tags: [security, policy, task-manager, gpui, rust]
status: draft
generated: 2026-11-14T22:35:00+09:00
---

## 1. 適用範囲とサポート

本ドキュメントは `gpui-task-manager` 試作に適用する。
**試作段階のため、セキュリティ修正のサポート期間は定めていない**。サポート対象は
最新の `main` のみとし、報告先は GitHub Security Advisory（private vulnerability
reporting）に限定する。

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
   - ループバックアドレスのみ bind（既定 `127.0.0.1`、`--listen` は `127.0.0.1` と `[::1]` のみ許可。`127.0.0.2` などの loopback や `0.0.0.0` は拒否）。
   - 認証は `Authorization: Bearer` のみ。クエリ文字列のトークンは拒否する
     （URL 履歴・Referer・スクリーンショットからの漏洩対策）。
   - `Host` ヘッダが loopback 以外なら 403（DNS リバインディング多層防御）。
     Host が無いリクエスト（主に HTTP/1.0 の素の TCP クライアント）は検証を
     スキップするが、`/healthz` と `/metrics` を除く全リクエストに Bearer
     トークンが必要なため実害はない（`/metrics` はカウンタ/ゲージのみで
     マシンデータを返さない）。
     ブラウザの fetch は Host を自動付与し（Forbidden header のため JS から
     変更不可）、DNS リバインディングは常に Host 付きで到達し 403 となる。
   - CORS は loopback オリジンのみ反射（エラー応答の 403 にも許可 Origin のみ付与）。
     ヘッダ上限 8 KiB、通常要求の読み取りタイムアウト 1 秒。
   - 長時間ストリーム `GET /api/v1/stream`（SSE）は**実装済み**。Bearer 認証
     （ヘッダのみ・クエリトークン拒否）、Host 検証（Host ヘッダ必須、loopback 以外は
     403）、loopback CORS（エラー応答にも許可 Origin のみ）を通常経路と同じく適用する。
     通常要求の読み取りタイムアウト 1 秒は SSE には適用せず（接続存続中はアイドル）、
     書き込みタイムアウトのみで各イベント送信を制限する。専用接続枠 `MAX_STREAMS`
     （2 枠）を設け、超過は即座に 503 として通常の `/api/v1/snapshot` ポーリングを
     圧迫しない。
   - 接続は最大 8 本まで並列処理し、超過は即座に 503（アイドルピアが他の要求を阻塞しない）。
   - トークンは Linux/Unix では起動時生成（`/dev/urandom`）または `--token-file` / `TM_AGENT_TOKEN`。
     自動生成分は XDG state ディレクトリ配下（`~/.local/state/tm-agent/token` 等）に 0600 で保存し、
     再起動時に再利用する。値も保存場所も出力せず、stderr にはファイル名 `token` のみ案内する
     （保存先は README.md「読み取り専用エージェント」の XDG state パスを参照）。
     **Windows では自動生成を行わない**。既存のトークンファイル（`%APPDATA%` /
     `%USERPROFILE%\AppData\Roaming` 配下）があれば再利用するが新規生成はできず、
     トークン未指定の起動は固定エラーで失敗するため、`--token-file` / `TM_AGENT_TOKEN` /
     `--token` のいずれかを明示する必要がある（固定エラーの文言は README.md「認証」節を参照、
     `agent/src/auth.rs:51-60` が根拠）。`--token` は argv 経由で他プロセスから見えるため警告を出す。
6. **表示の安全性**: OS 由来の文字列（プロセス名・コマンド・ユーザー名・ホスト名・
   マウントポイント等）は制御文字・ANSI エスケープ・Unicode bidi／ゼロ幅の不可視書式文字を除去し、
   文字数で切り詰める。
7. **秘密情報を保持しない**: 認証情報・DB を保存しない。トークンの**値**はログ・stdout に
   出力しない（接続には `--token-file` または `TM_AGENT_TOKEN` を明示指定してもよい）。
   Linux/Unix の自動生成トークンだけは XDG state ディレクトリ配下の 0600 ファイル（ディレクトリは 0700）に
   保存し、stderr にはファイル名 `token` のみ案内して再起動時に再利用する（保存先は README.md の XDG state パスを参照）。Windows は自動生成非対応のため既存ファイルの再利用のみで、明示指定が必須（上記 5 を参照）。
   ディスクに永続化するのはこのトークンファイルと**表示設定だけ**
   （更新間隔・ソート・詳細パネル・ツリー表示・テーマ）で、プロセス情報・フィルタ・ホスト名・
   ユーザー名はディスクに書かない。ネイティブの設定ファイルは原子的に書き込み、
   Unix では `0600` で作成する。ブラウザは同じスキーマを `localStorage` に保存し、
   アクセスできない場合は既定値へ縮退する（パニックしない）。
8. **公開物の衛生**: スクリーンショット・証拠 JSON は `--redact`（または環境変数
   `TM_REDACT=1`）を使い、ホスト名・ユーザー名・ホームパス・コマンド引数内の
   ホスト名 / tailnet ドメインを伏せる。
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
| エージェント可用性 | 同一ホストのプロセス | アイドル接続・部分ヘッダによるスロット占有 | 読み取りタイムアウト 1 秒・同時接続 8 本・超過 503（loopback 限定のため残存リスクは低い） |
| 依存クレート | crates.io → ビルド | 供給網攻撃・未保守 crate | 完全ピン・`Cargo.lock`・`cargo audit` |

## 5. 安全な使い方（推奨設定）

```bash
# ネイティブ版（危険操作は確認ダイアログ経由）
./target/debug/gpui-task-manager

# エージェント: トークンはファイルか環境変数で渡す（--token はテスト用）
printf '%s' "$(openssl rand -hex 32)" > ~/.config/tm-agent.token
chmod 600 ~/.config/tm-agent.token
./target/debug/tm-agent --listen 127.0.0.1:18787 --token-file ~/.config/tm-agent.token

# 公開用スクリーンショットは redact（--redact または環境変数 TM_REDACT=1）
./target/debug/gpui-task-manager --redact
TM_REDACT=1 ./target/debug/gpui-task-manager
./target/debug/tm-agent --once --redact
```

- エージェントを `0.0.0.0` やリモートに公開しない（現状 TLS / リモート認証は未実装）。
- トークンを URL クエリ・シェル履歴・共有ドキュメントに残さない。
- ブラウザ版は読み取り専用であることを前提に運用する。
- ステータスバーのメッセージは `[情報]` / `[エラー]` の接頭辞で種別を示し、
  `tm-viewmodel::StatusMessage::persistent` で永続性を区別する。一時
  (transient) メッセージは次に正常に `ingest` されたスナップショットで自動的に
  消える（一時停止中は消えない）。起動警告などの永続 (persistent) メッセージは
  正常サンプルや一時停止／再開をまたいで保持され、明示的なクリアか別の
  メッセージでの上書きまで残る。永続メッセージは設定・起動状態の注意として扱い、
  一時メッセージを恒久的な状態と誤認しないこと。

### 破損設定の退避と手動復元

破損した設定は上書きせず退避してから既定値で起動する。退避物は**最新 5 件**だけ
保持し、それより古いものは起動時に削除する（削除に失敗しても起動を継続する）。
退避物には表示設定以外は保存されないが、必要な設定を戻す場合は手動で復元する。

- ネイティブ: `~/.config/gpui-task-manager/settings.json.corrupt-<timestamp>` を
  `~/.config/gpui-task-manager/settings.json` へコピーして起動する。

```bash
# Kubuntu / Linux: 最新の退避物を settings.json へ戻して起動
backup=$(ls -1t ~/.config/gpui-task-manager/settings.json.corrupt-* | head -n 1)
cp -- "$backup" ~/.config/gpui-task-manager/settings.json
./target/debug/gpui-task-manager
```

- ブラウザ: DevTools（Edge なら F12 →「アプリ」→「ローカルストレージ」）で
  `gpui-task-manager.settings.corrupt.<timestamp>` の値を
  `gpui-task-manager.settings` へコピーし、ページを再読込する。

```text
gpui-task-manager.settings.corrupt.<timestamp>  →  gpui-task-manager.settings
```

復元後は退避物を削除してよい。保持上限を超えた退避物は自動削除されるため、
長期保存が必要な場合は別途バックアップする。

## 6. 依存関係ポリシー

- GPUI 系（`gpui-kit` / `gpui-pre*`）、`sysinfo`、`rustix` は**完全一致ピン**。
  `Cargo.lock` をコミットする。
- `rustix` は RUSTSEC-2026-0486 の修正版 `=1.1.6` に固定。
- `cargo audit` と `cargo-deny`（advisories / bans / licenses / sources）を CI で
  実行する。ローカルは `scripts/check.sh`（deny / vet / machete）と別途
  `cargo audit`（root / web）。既知の未保守・重複は `deny.toml` で理由付きに管理する。
- **SBOM**: `scripts/generate-sbom.sh`（CycloneDX JSON, root/web）と CI の `sbom`
  ジョブで供給網可視性を確保する。
- **cargo-vet**: root / web とも初期化済み（`cargo-vet 0.10.3`、`supply-chain/`）。
  2026-11-18 に両ストアを `baseline-unreviewed`（未監査ベースラインによる drift
  検出）方針へ統一。全依存を `exempt`（root 804 / web 560、`criteria = "baseline-unreviewed"`）
  として記録し、`safe-to-deploy` を主張しない。外部 imports は未使用
  （盲目的な信頼委譲を避ける）。未監査依存を「監査済み」として記録しない
  （監査記録の捏造をしない）。実レビュー時は `audited`（`safe-to-deploy` を含む）
  を付与し、`baseline-unreviewed` と混同しない。検証は `cargo vet --locked`
  （root / web）。依存更新時は ①実レビュー後に `certify`、②レビュー前なら理由付き
  `add-exemption` の順で対応する。CI ゲート化は共通ワークフローの `extra_gates`
  で実装済み（`cargo vet --locked`、root / web）。
- Dependabot（Cargo ルート / web、GitHub Actions、週次）とセキュリティアラート /
  自動修正を有効化する。GPUI 系は完全ピン運用のため自動更新の対象外。
- 依存追加は最小限とし、ライセンスは `deny.toml` の `[licenses].allow`
  （AGPL-3.0-or-later と互換なライセンスの一覧）で確認し、保守状況も確認する。
- **公開運用**: 本リポジトリは public。ブランチ保護は public リポジトリの機能で有効化し、必須 CI / レビュー運用で `main` を保護する。

## 7. 残余リスク

- PID の `start_time` は秒精度のため、同一秒内の PID 再利用や、再取得から
  `kill_process` までの TOCTOU を完全には排除できない。`pidfd` による原子化は将来課題。
- GPUI 系は個人アカウントが crates.io に再公開するスナップショットで、bus factor = 1。
  独立したセキュリティ監査は未実施。
- ブラウザのトークンは URL フラグメントから読み取るため、画面共有・スクリーンショット・
  同一ユーザーの他プロセスには露出し得る。読み取り専用 API と loopback 限定で影響を抑える。
- ネイティブ版 IME は実装済みだが実 IME（IBus/fcitx5）での手動確認は未実施。ブラウザ版シェルは
  Chrome 158 / Edge 157 / Firefox 160 で実動確認済み（2026-11-20、WebDriver BiDi。F1 は
  Firefox で合成キーにより検証し、信頼済みキーは未確認）。未検証は Safari / Brave / モバイル /
  実 GPU の WebGPU、Windows / macOS の物理実機（Windows/macOS は CI ランナーで
  型検査・単体テストを実施）。
- Windows のシグナル送信は `taskkill /F` による強制終了で、POSIX の SIGTERM/SIGSTOP
  とは意味が異なる。
- `--redact` は既知の識別子（ホスト名・ユーザー名・ホームパス・tailnet ドメイン）を
  置換する。ゼロ幅・不可視書式文字（U+200B–U+200F、Unicode 16.0 の全 Cf）は
  `tm-core::sanitize` で除去済みで、残るなりすましリスクは同形異義文字（homoglyph）のみ。

## 8. 検証

```bash
cd .
bash scripts/check.sh          # fmt / clippy / test / web test / wasm / deny / vet / machete（件数は README.md を参照）
cargo audit
```

安全関連の主なテスト:

- `tm-platform`: PID 0/1/self の拒否、stale start_time の拒否（子プロセスが生存したまま）、
  範囲外 PID、reap 済み PID、spawn した子の SIGTERM。
- `tm-viewmodel`: 確認前は Effect を出さない、確認後にのみ Signal を出す、PID 1/self の拒否、
  Kill シグナルの保持、対象の identity 捕捉。
- `gpui-task-manager`: Enter=キャンセル / Ctrl+Enter=確定、その他キーの無視。
- `tm-core`: 制御文字・ANSI・bidi／ゼロ幅不可視文字の除去、500 ケースの決定論的サニタイズ検査。
- `tm-agent`: クエリトークン拒否、Host 検証、Bearer 認証、CORS、`--token-file`、
  `--redact`（コマンド引数内ホスト名・tailnet ドメインを含む）。
- `tm-viewmodel`: `FilterInput` の UTF-16 / selection / marked range の状態機械
  （Web/ネイティブ共有。件数は README.md「検証状況」を参照）。`web` にも agent テストあり。
  IME は CDP の composition 32 チェック + ネイティブ `TM_DEBUG_IME=1` 検証
  （確定前は反映しない）。
- クロスプラットフォーム: Windows ターゲットの workspace check、macOS 4 crate の
  check、CI の windows-latest / macos-latest。
- `app`: 設定ファイルの往復・原子的書き込み・`0600` 権限・範囲クランプ・
  未知フィールド無視。デバッグフック `TM_DEBUG_SETTINGS=1` で保存経路を
  実行時確認し、設定ファイルを置いた起動で復元を確認。

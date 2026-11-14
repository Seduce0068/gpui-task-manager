---
type: Implementation Report
title: 日本語 IME 対応（ブラウザ版は EntityInputHandler 実装・Runtime Proof 付き、ネイティブは設計案）
description: gpui-pre-web 0.3.12 の IME ミラー（隠し textarea）を利用してブラウザ版シェルの絞り込み欄へ日本語 IME 対応を追加し、ヘッドレス Chrome + CDP の Input.imeSetComposition / Input.insertText で「にほんご」→「日本語」の変換・確定・Esc 復帰を検証した記録。ネイティブ版は未実装で、設計案と未対応理由を記載する
tags: [gpui, wasm, ime, japanese-input, composition, entity-input-handler, runtime-proof]
resource: work/gpui-task-manager/web
generated: 2026-11-15T00:36:34+09:00
status: verified
---

## 結論（要約）

- **ブラウザ版（`web/`）は日本語 IME 対応を実装し、Runtime Proof を取得した。**
  独自の隠し `<input>` は作らず、`gpui-pre-web` 0.3.12 が既に持つ
  **IME ミラー（隠し `<textarea>`）+ composition イベント購読**に乗り、
  アプリ側に `gpui::EntityInputHandler` を実装する方式にした。
- IME 変換中（composition 中）は入力欄にだけ未確定文字列を青＋下線で表示し、
  `Dashboard` の絞り込みには **確定文字列だけ** を反映する。
  「にほんご」変換中は一覧が全 28 件のまま、「日本語」確定後に 0 件へ絞り込まれる
  ことを CDP で確認した。
- `cargo check --target wasm32-unknown-unknown` / `trunk build --release` 成功。
  ヘッドレス Chrome + CDP の **32 チェックすべて pass**、JS 例外・コンソール
  エラー 0 件。単体テスト **11 件 pass**。
- **ネイティブ版（`app/`）は未実装。** `gpui-pre-linux` は Wayland
  text-input-v3 / X11 XIM に対応しており、`EntityInputHandler` を実装すれば
  動く見込みだが、実 IME の合成入力で検証できないこと・時間対効果・
  既存の検証済み挙動を壊すリスクから、設計案と未対応理由を本ノートに記録した
  （[ネイティブ版（設計案・未実装）](#ネイティブ版設計案未実装)）。
- `web/Cargo.toml` に `license = "Apache-2.0"` と path 依存の `version` を追加した
  （cargo-deny 対応、Team Lead 依頼）。IME 用の `web-sys` feature 追加は **不要**
  だった（DOM 操作はすべて `gpui-pre-web` 内で完結するため）。

## 調査結果（実装方針の根拠）

### `gpui-pre-web` は IME を既に実装している

`~/.cargo/registry/src/index.crates.io-*/gpui-pre-web-0.3.12/src/` を読んだ結果:

| 要素 | 実装 | 根拠 |
|---|---|---|
| 隠し `<textarea>`（IME ミラー） | あり | `src/ime_mirror.rs`（618 行）。`position: fixed; 1px; opacity: 0` で body に追加 |
| `compositionstart` / `update` / `end` 購読 | あり | `src/events.rs` の `register_composition_*` |
| `beforeinput` / `input` の差分取り込み | あり | `src/events.rs` の `register_input`。prefix/suffix diff で編集形状を復元 |
| `selectionchange` 取り込み | あり | `src/events.rs` の `register_selection_change` |
| プラットフォーム → GPUI の入力ハンドラ呼び出し | あり | `src/window.rs` の `with_input_handler` / `set_input_handler` |
| アプリ側の受け口 | **未実装（アプリの責務）** | `gpui::EntityInputHandler` を実装し `Window::handle_input` で登録する |

つまり **自前で DOM の隠し input を作る必要はない**。`web/src/shell.rs` の
`TaskManagerApp` に `EntityInputHandler` を実装し、paint 中に
`window.handle_input(&focus_handle, ElementInputHandler::new(bounds, view), cx)`
を呼べばよい（`gpui-pre` の `src/input.rs` にある `ElementInputHandler` の
標準的な使い方）。`wasm-spike.md` の「composition には `EntityInputHandler`
実装が必要」という指摘と一致する。

### 入力経路（Web）

1. 隠し `<textarea>` が DOM フォーカスを持つ（GPUI のフォーカスとは別）。
2. ハードウェアキー: textarea の `keydown` → GPUI の `KeyDownEvent` として配送
   → 文字入力は web 層が `replace_text_in_range(None, key_char)` を呼ぶ。
3. IME: `compositionstart` → `compositionupdate`（変換中の文字列全体）
   → `replace_and_mark_text_in_range`、`compositionend` → `replace_text_in_range`
   と `unmark_text`。
4. Backspace / Delete / 貼り付け: ブラウザの既定動作が textarea を編集し、
   `input` イベントの差分が `replace_text_in_range(Some(range), text)` として届く。

このため `on_key_down` から **文字の直接組み立てと Backspace 処理を削除**した
（残すと二重挿入・二重削除になる）。`on_key_down` は Esc・Enter・一覧ナビゲーション
（↑↓/PageUp/PageDown/Home/End）だけを担当し、消費したキーは
`cx.stop_propagation()` でブラウザ既定動作を止める。

## 実装

### 変更ファイル

| パス | 内容 |
|---|---|
| `web/src/filter_input.rs` | **新規**。絞り込み欄の状態機械（UTF-16・選択・marked range）と単体テスト 11 件。DOM / GPUI 非依存 |
| `web/src/shell.rs` | `EntityInputHandler` 実装、入力ハンドラ登録用 canvas、`on_key_down` 整理、変換中表示、DOM proof へ IME 状態を追加 |
| `web/src/lib.rs` | 純粋ロジックをホスト（`cargo test`）で検証できるよう、wasm 専用エントリ（`run` / `start` / `load_fonts`）を `#[cfg(target_family = "wasm")]` で分離 |
| `web/Cargo.toml` | `license = "Apache-2.0"`、path 依存の `version` 追加（cargo-deny 対応） |

### `FilterInput`（純粋ロジック）

- 保持する状態: `text`（表示テキスト = 確定 + 変換中）、`selection`
  （UTF-16）、`marked`（IME 変換中範囲、UTF-16）。
- `committed_text()`: marked 範囲を取り除いた **確定済み文字列**。
  これだけを `Command::SetFilterText` / `Command::ClearFilter` に渡す。
- `display_parts()`: 表示用に `(変換前, 変換中, 変換後)` へ 3 分割。
- 編集操作: `replace_text`（確定・キー入力・貼り付け）、`replace_and_mark`
  （compositionupdate）、`unmark`、`clear`、`set_selection`、`text_for_range`。
- サロゲートペア（絵文字など）を含む UTF-16 オフセットでも panic しないよう、
  文字境界へ丸めるヘルパー（`floor_byte_index`）を使用。

| 状態 | 表示 | Dashboard のフィルタ |
|---|---|---|
| 通常入力（ASCII） | そのまま | 即時反映 |
| composition 中（marked あり） | 未確定文字列を青＋下線、「変換中」表示 | **更新しない** |
| composition 確定 | 確定文字列を通常色 | 確定文字列を反映 |
| Esc | 入力欄を空に | `Command::ClearFilter` |
| Backspace / Delete | ブラウザ既定 → input 差分取り込み | 1 文字削除を反映 |

### `TaskManagerApp` の `EntityInputHandler` 実装

- `text_for_range` / `selected_text_range` / `marked_text_range` /
  `text_length_utf16` は `FilterInput` の状態を UTF-16 で返す。
- `replace_text_in_range` / `replace_and_mark_text_in_range` / `unmark_text` /
  `set_selected_text_range` は `FilterInput` を更新し、確定したときだけ
  Dashboard へ反映する。
- `accepts_text_input` は常に `true`（ブラウザ版にモードは無く、絞り込み欄が
  常に編集対象）。`text_input_configuration` は `autocorrect: false,
  suggestions: false`。
- 入力ハンドラは `render` の最後に置いた透明 canvas の paint で毎フレーム登録する
  （`Window::handle_input` は paint 中に呼ぶのが GPUI の作法）。

### 変換中表示と DOM proof

- ツールバーの絞り込み欄は `display_parts()` の 3 分割を子要素として描画し、
  変換中だけ `text_color(C_BLUE)` + `underline()`、右側のヒントを「変換中」に
  切り替える（確定後は「入力可」）。
- CDP 検証用に `body[data-gpui-proof]` へ次のフィールドを追加した
  （既存フィールドは変更なし）。
  `ime_composing`（0/1）、`ime_field`（表示テキスト）、
  `ime_marked`（UTF-16 範囲 or `-`）、`ime_selection`（UTF-16 範囲）。
- `document.title` の既存フォーマットは変えていない（既存 CDP スクリプトとの互換）。

## 検証（Runtime Proof）

実行環境: Linux（Kubuntu）、Chrome 157.0.8103.50 ヘッドレス、trunk 0.22.1、
wasm32-unknown-unknown。日時 2026-11-15 00:25–00:31 JST。

### ビルド・単体テスト

```bash
# 1. 純粋ロジックの単体テスト（ホストで実行）
cargo test --manifest-path work/gpui-task-manager/web/Cargo.toml --lib -- filter_input
# → test result: ok. 11 passed; 0 failed

# 2. wasm ターゲットの型チェック
cargo check --target wasm32-unknown-unknown \
  --manifest-path work/gpui-task-manager/web/Cargo.toml
# → Finished `dev` profile ...（warning なし）

# 3. リリースビルド
cd work/gpui-task-manager/web && trunk build --release
# → ✅ success
#   dist/tm-web-*.wasm: 7,589,230 bytes（gzip 3,024,986 bytes）
#   wasm-opt は Team Lead が有効化済み（web/index.html）

# 4. clippy（wasm ターゲット）
cargo clippy --target wasm32-unknown-unknown \
  --manifest-path work/gpui-task-manager/web/Cargo.toml
# → Finished `dev` profile（warning なし）

# 5. rustfmt
rustfmt --check --edition 2024 work/gpui-task-manager/web/src/{lib.rs,shell.rs,filter_input.rs}
# → 差分なし

# 6. ネイティブ版（app/ は未変更。回帰確認のみ）
cd work/gpui-task-manager && cargo build -p gpui-task-manager
# → Finished `dev` profile（exit 0）
```

### CDP による IME 検証

静的配信はスクリプトが起動（`python3 -m http.server`）。Chrome は
`--headless=new --no-sandbox --disable-dev-shm-usage --window-size=1440,1000
--remote-debugging-port=<port>`。

```bash
python3 web/scripts/cdp_verify_ime.py \
  --dist work/gpui-task-manager/web/dist \
  --out work/gpui-task-manager/notes/assets \
  --http-port 8097 --cdp-port 9227 \
  --evidence /tmp/ime-evidence.json
```

> 検証スクリプトは書き込みスコープ（`web/src/`・`web/Cargo.toml`・`app/src/`・
> `notes/ime.md`・`notes/assets/ime-*.png`）外のため `/tmp` に置いた。
> 既存の `web/scripts/cdp_verify_web.py` の CDP ヘルパーを参考にしている。
> リポジトリへ常設する場合は `web/scripts/` へ移す（Team Lead 判断）。

CDP の操作と確認内容（全 32 チェック pass、JS 例外 0、コンソールエラー 0）:

| # | 操作（CDP） | 確認したこと | スクリーンショット |
|---|---|---|---|
| 0 | 初期表示 | 隠し textarea が存在し **フォーカス済み**（IME の受け口）、`ime_composing=0`、フィルタ空 | `ime-01-initial.png` |
| 1 | `dispatchKeyEvent` で `fire` | ASCII 入力が `filter=fire` へ反映、一覧 28→1 件 | `ime-02-ascii-fire.png` |
| 2 | `Backspace` | `filter=fir`（ブラウザ既定 → input 差分取り込み） | `ime-03-backspace.png` |
| 3 | `Escape` | フィルタ解除・一覧 28 件へ復帰 | `ime-04-escape-clear.png` |
| 4 | `Input.imeSetComposition("にほんご")` | `ime_composing=1`、`ime_field=にほんご`、`ime_marked=0:4`、**`filter` は空のまま**、一覧 28 件のまま | `ime-05-composing.png` / `ime-05-composing-filter.png`（拡大） |
| 5 | `Input.imeSetComposition("日本語")` | 候補更新が入力欄へ反映、`filter` は空のまま | `ime-06-composing-update.png` |
| 6 | `Input.insertText("日本語")` | compositionend → `filter=日本語`、`ime_composing=0`、一覧 0 件 | `ime-07-committed.png` / `ime-07-committed-filter.png`（拡大） |
| 7 | `Escape`（確定後） | フィルタ解除・入力欄も空・一覧 28 件へ復帰 | `ime-08-escape-after-commit.png` |
| 8 | composition 中の `Escape` | フィルタ空のまま・入力欄も空。その後 composition をやり直すと反映される（状態回復） | `ime-09-composing-escape.png` |
| 9 | 最終状態 | フィルタ解除済み | `ime-10-final.png` |

拡大画像（`ime-05-composing-filter.png`）では、絞り込み欄に
**「にほんご」が青＋下線・右側に「変換中」** と表示され、一覧は絞り込まれて
いないことが視認できる。`ime-07-committed-filter.png` では確定後の
**「日本語」** が通常色で表示され、一覧は「一致するプロセスがありません」になる。

### 既存機能の回帰確認

IME 対応で `on_key_down` から文字入力・Backspace を外したため、既存の CDP 検証
スクリプト（`web/scripts/cdp_verify_web.py`）をそのまま実行して回帰が無いことを
確認した。

```bash
# 静的配信（dist）と Chrome を起動したうえで
python3 web/scripts/cdp_verify_web.py --port 9228 \
  --url http://127.0.0.1:8098/ \
  --out /tmp/regression-assets --evidence-out /tmp/regression-evidence \
  --prefix ime-regression --label demo
# → {"failures": [], "passed": true}
```

描画・1 秒更新・スクロール・行クリック（詳細パネル）・キーボード絞り込み・
列クリックソートの全チェックが pass した（スクリーンショットは `/tmp` に取得し、
リポジトリへは追加していない）。

### ネイティブ版の起動確認（IME は未検証）

```bash
cd work/gpui-task-manager
./target/debug/gpui-task-manager --demo &
spectacle -a -b -n -e -S -o /tmp/native-active.png   # Wayland のアクティブウィンドウ
```

デモデータでウィンドウが描画されることを確認した
（`notes/assets/ime-native-01-demo.png`、合成データのみ）。
**実 IME の合成入力は未検証**（後述）。

## ネイティブ版（設計案・未実装）

### 現状の挙動と根拠

- `app/src/app.rs` の絞り込みは `KeyDownEvent.keystroke.key_char` を直接
  連結する独自実装で、`EntityInputHandler` を実装していない。
- `gpui-pre-linux` 0.3.12 は IME に対応している:
  - Wayland: `zwp_text_input_v3` の `PreeditString` / `CommitString` を処理
    （`src/linux/wayland/client.rs` 2300–2370 付近）。
  - X11: XIM（`src/linux/x11/xim_handler.rs`）。
- ただし `WaylandWindow::handle_ime`（`src/linux/wayland/window.rs` 1479）は
  `state.input_handler` が無いと **何もしない**。つまり入力ハンドラ未登録の
  現状では、`SetMarkedText`（変換中プレビュー）と複数文字の
  `InsertText`（例:「日本語」）は **捨てられる**。1 文字の `CommitString` だけは
  `key_char` を持つ合成 `KeyDownEvent` として届くため偶然動く
  （`client.rs` 2320 付近）。
- 結論: ネイティブは現状「1 文字確定のみ動く・変換中プレビューなし・
  複数文字確定は失われる」状態で、日本語 IME としては不十分。

### 設計案 A（推奨）: ネイティブにも `EntityInputHandler` を実装

1. `FilterInput`（`web/src/filter_input.rs`）を共有 crate へ移す
   （例: `crates/tm-viewmodel/src/filter_input.rs`。DOM 非依存なのでそのまま使える）。
   書き込みスコープ外のため今回は移動していない。
2. `app/src/app.rs` の `TaskManagerApp` に `EntityInputHandler` を実装する。
   `accepts_text_input` は `self.filter_editing` を返し、Ctrl+F の編集中だけ
   IME を有効化する。Ctrl+F 時に `FilterInput` を Dashboard の現在値で初期化する。
3. `render` のルートに透明 canvas を追加し、paint で
   `window.handle_input(&self.focus_handle, ElementInputHandler::new(bounds, cx.entity()), cx)`
   を呼ぶ（web 版と同じ）。
4. `on_key_down` から `key_char` 連結と Backspace 処理を削除し、
   Esc（解除）/ Enter（編集終了）/ ↑↓ 等だけ残す。文字編集はハンドラ経由にする。
5. 絞り込み欄の表示を `display_parts()` の 3 分割にし、変換中は下線＋色分け。
   `window.notify_text_input_changed` / `notify_text_selection_changed` を呼ぶ。
6. 検証: 実 IBus/fcitx5 環境で「にほんご」→「日本語」の変換・確定・Esc を
   手動確認する必要がある（CDP に相当する合成入力が無い）。

### 設計案 B: `gpui_component::input::{Input, InputState, InputEvent}` へ置換

- `gpui-kit 0.9.1` の `gpui_component::input` は IME 対応のテキスト入力部品で、
  `InputState` を `Input::new(&state)` で描画し `InputEvent::Change` を購読する。
- 長所: キャレット・選択・スクロール・変換中表示が部品側で完結する。
- 短所: フォーカス・編集モードの意味論が既存の「Ctrl+F で編集開始、Esc で解除、
  Enter で終了」と異なる。`InputState` の初期化 API・イベント購読・テーマ統合の
  再設計が必要で、検証済みのネイティブ挙動（モーダル・シグナル送信・デモ/ライブ）
  を壊すリスクが高い。今回は採用しない。

### 未対応の理由

1. **検証手段**: 実 IME（IBus/fcitx5）の合成入力をエージェントから再現する手段が
   無く、Wayland text-input-v3 の composition を注入するツールも無い。
   「未検証の実装」を検証済み扱いにしないため、設計案の記録に留めた。
2. **時間対効果**: Web（必須）の実装・検証を優先した。ネイティブは
   `app.rs` が約 1,500 行の単一ファイルで、フィルタの意味論変更を伴う。
3. **共有ロジックの置き場所**: `FilterInput` は現在 `web/src/` にあり、
   ネイティブから使うには共有 crate への移動が必要（今回の書き込みスコープ外）。
4. **回帰リスク**: ネイティブは既に Runtime Proof 取得済みで、IME 対応の
   ついでに壊す価値が無い。

## 制約・既知の制限

- **変換候補ウィンドウの位置**: 隠し textarea は 1px・不可視で画面左上に固定される
  ため、変換候補ウィンドウはブラウザ既定の位置（左上付近）に出る。
  `bounds_for_range` は `None` を返しており、候補位置の制御はしていない。
- **CDP の合成 IME は Escape で `compositionend` を発火しない**。
  そのため「変換中に Esc」の直後の ASCII 入力は、web 層の `is_composing` が
  残って飲み込まれる（検証では次の composition で状態が回復することを確認）。
  実 IME は Escape で composition を終了して `compositionend` を発火するため
  問題ない見込みだが、**実 IME での確認は未実施**。
- **実 IME（IBus/fcitx5）での手動確認は未実施**。CDP の
  `Input.imeSetComposition` / `Input.insertText` は Chrome の IME 経路を
  再現するが、実 IME 固有の挙動（候補ウィンドウ、予測変換、確定タイミング）は
  完全には一致しない。
- **Android / iOS のソフトキーボード**: `gpui-pre-web` の touch 入力経路は
  実装されているが、本検証はデスクトップ Chrome のみ。未検証。
- **Safari / Firefox**: 未検証（`gpui-pre-web` の対応状況も未確認）。
- **ネイティブ**: 未実装（上記設計案）。実 IME 検証も未実施。
- `web/index.html` / `web/Trunk.toml` / `web/README.md` / ルート `Cargo.toml` /
  `.github/` / `notes/verification.md` は変更していない（Team Lead の担当）。
- 本ノートは markdownlint の MD013 / MD025 / MD041 を満たす。MD060
  （テーブルのパイプ揃え）は既存ノート（`web-shell.md` など）と同じ表スタイルで、
  リポジトリ既存の baseline と同じ警告が残る（本変更由来ではない）。

## 参照

- `~/.cargo/registry/src/index.crates.io-*/gpui-pre-0.3.12/src/input.rs`
  （`EntityInputHandler` / `ElementInputHandler`）
- `~/.cargo/registry/src/index.crates.io-*/gpui-pre-web-0.3.12/src/ime_mirror.rs`
  （IME ミラー）、`src/events.rs`（composition / input / selectionchange 購読）
- `~/.cargo/registry/src/index.crates.io-*/gpui-pre-linux-0.3.12/src/linux/wayland/window.rs`
  （`WaylandWindow::handle_ime`）、`src/linux/wayland/client.rs`（text-input-v3）
- 既存ノート: [wasm-spike.md](wasm-spike.md)（IME ミラーの指摘）、
  [web-shell.md](web-shell.md)（ブラウザ版シェルの実装）
- Chrome DevTools Protocol: [Input.imeSetComposition](https://chromedevtools.github.io/devtools-protocol/tot/Input/#method-imeSetComposition)、
  [Input.insertText](https://chromedevtools.github.io/devtools-protocol/tot/Input/#method-insertText)

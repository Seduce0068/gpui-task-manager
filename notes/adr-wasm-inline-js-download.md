---
type: ADR
title: wasm ダウンロードは inline JS を維持する（web-sys 置換は未実装）
description: wasm 版のテキストダウンロードを web-sys API へ置換しない判断と、将来置換する際の条件を記録する。
tags: [adr, wasm, web-sys, wasm-bindgen, download]
generated: 2026-11-19T05:24:39+09:00
status: approved
---

## 1. 現状（Context）

- `web/src/shell.rs:2550-2563` に `#[wasm_bindgen(inline_js = ...)]` の文字列として `tmDownloadText(filename, mime, text)` が埋め込まれている。
- 処理は `Blob` 生成 → `URL.createObjectURL` → `document.createElement('a')` → `href` / `download` / `rel='noopener'` 設定 → `body.appendChild` → `click()` → `remove()` → `setTimeout(..., 0)` で `revokeObjectURL` という短い shim である。
- `web/src/shell.rs:2564-2567` が `extern "C"` の `tm_download_text` を `js_name = "tmDownloadText"` で束縛し、`web/src/shell.rs:2575-2578` の wasm 用 `download_text` がこれに委譲する。ネイティブ版（`web/src/shell.rs:2580-2581`）は DOM が無いため no-op である。
- 呼び出しは `web/src/shell.rs:457` のエクスポート処理 1 箇所のみ。CDP 検証スクリプトにはダウンロード操作の検証が無く、ブラウザ上での実効確認は未実施である。
- `web/Cargo.toml:40-47` の `web-sys` feature は `console` / `Document` / `Location` / `Storage` / `UrlSearchParams` / `Window` の 6 つのみ。`Url` / `HtmlAnchorElement` / `Blob` は有効になっていない。
- `web/src/shell.rs:2571-2574` のコメントにも、feature が無いため inline JS を採用した旨が既に書かれている。本 ADR はその判断を設計記録として明示するものである。

## 2. 決定（Decision）

現状の inline JS を維持し、web-sys API への置換は **行わない**。これは「未実装」を正直に記録するものであり、置換が完了したことを意味しない。後述の条件が揃った時点で改めて検討し、その際は本 ADR を `superseded` として後継 ADR を新規作成する。

## 3. 判断の根拠（3 つの論点）

### 3.1 feature 追加の影響範囲

`web-sys` の `Url` / `HtmlAnchorElement` / `Blob` を有効にすると、`web/Cargo.toml:40-47` の feature 一覧を変更し、wasm ターゲットの再ビルドと再検証が必要になる。実際には `Node` / `HtmlElement` といった補助 feature や `wasm-bindgen` の `JsCast` も必要になる可能性があり、必要な feature の確定は実装時に行う。`web/` は独立 workspace かつ release プロファイルが `opt-level = "s"` / `lto = "thin"` でサイズ最適化されており（`web/Cargo.toml:55-62`）、1 機能のために web-sys の surface を広げる費用対効果が現状は見合わない。なお feature 変更が `web/Cargo.lock` のパッケージ集合を実際に変えるかは未確認で、導入時に確認する。

### 3.2 inline JS の限界

埋め込みコードはダウンロードの 1 経路だけを担い、イベント処理は `click()` のみ、後始末は `remove()` と `setTimeout(..., 0)` による `revokeObjectURL` だけである。汎用 DOM ヘルパーへ拡張する要求も、複数ファイル種別のダウンロードを同時に扱う要求も現時点では存在しない。したがって「限界はあるが、現状の要求集合に対しては過不足がない」と判断した。限界が顕在化した場合は §5 の条件で置換を検討する。

### 3.3 型チェックされないリスク

inline JS は文字列リテラルであるため、Rust の型検査・clippy・`cargo check --target wasm32-unknown-unknown` の対象外であり、誤りはブラウザ実行時まで顕在化しない。これは明示的に受容した既知のリスクであって、inline JS の利点ではない。現状は、
呼び出し側を `download_text` という 3 引数の純粋なラッパーに閉じ込め（`web/src/shell.rs:2575-2581`）、native 側のテストで変換経路を検証することで影響範囲を抑えている。ブラウザ側の自動検証が無いことは §6 の負の影響として記録する。

## 4. 代替案と不採用理由

- **web-sys API で即時置換**: `Url::create_object_url_with_blob` と `HtmlAnchorElement` による実装は技術的に可能と見られるが、§3.1 の影響範囲と §5 の検証整備が先に必要なため、今回は不採用。
- **`data:` URL へ変更**: `Blob` / `URL` を排して文字列を直接 `href` に埋める案は、大型エクスポートでの URL 長・メモリ消費と MIME／ファイル名の制御劣化が懸念され不採用。
- **ネイティブ側のファイル保存処理へ寄せる**: wasm ビルドには DOM が無く、ブラウザの sandbox 内ではネイティブの保存 API を呼べないため不採用。

## 5. 将来 web-sys へ置換する条件

以下のすべてを満たしたときにのみ置換を再検討する。

1. `web/Cargo.toml` に `Url` / `HtmlAnchorElement` / `Blob` と必要な補助 feature を追加し、最小構成を特定できること。
2. `cargo check --locked --target wasm32-unknown-unknown --manifest-path web/Cargo.toml` と `trunk build --release` が通ること。
3. ブラウザ上でダウンロードを検証する手段を用意すること。現状は CDP スクリプトにダウンロード検証が無いため、新規スクリプトの追加または手動 Runtime Proof の記録が必要である。
4. `download_text` のシグネチャと `web/src/shell.rs:457` の呼び出し側を変えず、native 側のテストが継続して通ること。
5. 本 ADR を `superseded` に更新し、置換の根拠・検証結果・ロールバック手順を新しい ADR に記録すること。

## 6. 影響（Consequences）

- 正: `web/Cargo.toml` の feature 追加が不要で、wasm のビルド構成と release プロファイルを変更せずに済む。判断と根拠が本 ADR に集約される。
- 負: inline JS は Rust の型検査対象外であり、ブラウザでだけ失敗し得る。ダウンロードの自動検証が無いため、回帰検知は手動に依存する。
- 負: 将来置換する際は §5 の検証整備から始める必要があり、作業量は先送りではなく繰り延べである。

## 7. 参照

- `web/src/shell.rs:2550-2567` — inline JS と `extern "C"` 束縛
- `web/src/shell.rs:2571-2581` — wasm / native の `download_text`
- `web/src/shell.rs:457` — エクスポート処理からの呼び出し
- `web/Cargo.toml:40-47` — `web-sys` feature 一覧
- `web/Cargo.toml:49-62` — dev / release プロファイル
- `notes/web-shell.md` — ブラウザ版シェルの実装と検証の概要

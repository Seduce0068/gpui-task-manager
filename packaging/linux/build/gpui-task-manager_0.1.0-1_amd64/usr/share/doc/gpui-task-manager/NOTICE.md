<!-- SPDX-License-Identifier: Apache-2.0 -->
# NOTICE — 第三者ソフトウェアの帰属

本プロジェクトは以下のオープンソースソフトウェアを利用しています。
各ライセンスの全文は各プロジェクトの配布物を参照してください。

## 直接依存

| ソフトウェア | ライセンス | 用途 |
|---|---|---|
| [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) | Apache-2.0 | GPU アクセラレート UI フレームワーク |
| [gpui-kit](https://github.com/longbridge/gpui-kit) 0.9.1 | Apache-2.0 | GPUI のコンポーネント集（`gpui-pre 0.3.12` スナップショットを含む） |
| [gpui-component](https://github.com/longbridge/gpui-kit) | Apache-2.0 | テーマ・部品 |
| [Lucide](https://lucide.dev)（gpui-kit-assets 経由） | ISC（Feather 由来部分は MIT） | アイコン |
| [sysinfo](https://github.com/GuillaumeGomez/sysinfo) 0.40.3 | MIT | システム情報収集 |
| [rustix](https://github.com/bytecodealliance/rustix) 1.1.6 | Apache-2.0 WITH LLVM-exception OR MIT | 安全なシグナル API |
| [serde](https://serde.rs) / [serde_json](https://github.com/serde-rs/json) | Apache-2.0 OR MIT | JSON シリアライズ |
| [wasm-bindgen](https://github.com/rustwasm/wasm-bindgen) | Apache-2.0 OR MIT | wasm バインディング（web シェル） |
| [web-sys](https://github.com/rustwasm/wasm-bindgen) / [js-sys](https://github.com/rustwasm/wasm-bindgen) | Apache-2.0 OR MIT | ブラウザ API（web シェル） |
| [console_error_panic_hook](https://github.com/rustwasm/console_error_panic_hook) | Apache-2.0 OR MIT | ブラウザコンソールへの panic 出力 |
| [anyhow](https://github.com/dtolnay/anyhow) / [futures](https://github.com/rust-lang/futures-rs) / [log](https://github.com/rust-lang/log) | Apache-2.0 OR MIT | web シェルのエラー処理・非同期・ログ |

## 参考にした調査（コードはコピーしていない）

- `work/gpui-file-explorer`（AGPL-3.0-or-later）の調査ノート
  （`notes/ecosystem-notes.md`, `notes/wasm-browser-notes.md`）。
  設計上の知見のみ参照し、ソースコードは取り込んでいません。

## ライセンス全文

本プロジェクト自身のコードは Apache License 2.0（[LICENSE](LICENSE)）です。

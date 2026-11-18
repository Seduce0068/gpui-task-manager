<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
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

## バンドル済みアセット

| アセット | ライセンス | 帰属・同梱先 |
| --- | --- | --- |
| IBM Plex Sans / Mono（3 TTF） | SIL OFL 1.1（Reserved Font Name "Plex"） | Copyright 2017-2018 IBM Corporation。ライセンス全文は `web/assets/fonts/LICENSE-IBM-Plex.txt`（`poc-wasm/assets/fonts/` にも同梱） |

## 参考にした調査（コードはコピーしていない）

- 別の AGPL プロジェクトの調査ノート（AGPL-3.0-or-later）。
  設計上の知見のみ参照し、ソースコードは取り込んでいません。

## ライセンス全文

本プロジェクト自身のコードは AGPL-3.0-or-later（[LICENSE](LICENSE)）です。

配布パッケージの著作権表示は `/usr/share/doc/gpui-task-manager/copyright`、
同梱ライセンス全文は同 `licenses/`、未同梱ライセンスの確認先は同 `README.md`
を参照してください。

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
| [web-time](https://github.com/daxpedda/web-time) 1.1.0 | MIT OR Apache-2.0 | wasm32 のブラウザ互換時刻（web シェル） |
| [anyhow](https://github.com/dtolnay/anyhow) / [futures](https://github.com/rust-lang/futures-rs) / [log](https://github.com/rust-lang/log) | Apache-2.0 OR MIT | web シェルのエラー処理・非同期・ログ |

## 推移依存（gpui-pre-web / gpui-pre-platform 経由）

| ソフトウェア | ライセンス | 用途 |
| --- | --- | --- |
| [console_error_panic_hook](https://github.com/rustwasm/console_error_panic_hook) | Apache-2.0 OR MIT | ブラウザコンソールへの panic 出力 |

## ライセンス family 一覧（全依存）

上記の表は主要コンポーネントのみを列挙しているが、実際の静的リンク依存は
root と web の 2 ワークスペースを合わせて約 783 crates にのぼる。以下は
`deny.toml` の `[licenses].allow` / `exceptions` と `cargo metadata` の
`license` 値（root + web）から導出した全ライセンス family の一覧で、
主要コンポーネントに限定せず網羅することを目的とする。
個別クレート名と各クレートの正確な SPDX 式は、配布パッケージの
`licenses/crates/` に同梱されるライセンス全文（ビルド時に生成）を参照。

| family（SPDX） | 区分 | 根拠・備考 |
| --- | --- | --- |
| `0BSD` | 許容（permissive） | `deny.toml` allow。`0BSD` / `0BSD OR MIT OR Apache-2.0` として出現 |
| `AGPL-3.0-or-later` | コピーレフト（アプリ本体） | `deny.toml` allow。プロジェクト自身のライセンス |
| `Apache-2.0` | 許容 | `deny.toml` allow。OR 式の代替として最も多く出現 |
| `Apache-2.0 WITH LLVM-exception` | 例外付き許容 | `deny.toml` exceptions（`rustix` / `linux-raw-sys`）。平文 `Apache-2.0` 代替も許容 |
| `BSD-2-Clause` | 許容 | `deny.toml` allow |
| `BSD-3-Clause` | 許容 | `deny.toml` allow |
| `BSL-1.0` | 許容 | `deny.toml` allow。`Apache-2.0 OR BSL-1.0` の代替として出現 |
| `CC0-1.0` | パブリックドメイン宣言 | `deny.toml` allow |
| `GPL-2.0-only` | 非選択の OR 代替 | `Apache-2.0 OR GPL-2.0-only`。許容側（`Apache-2.0`）を選択 |
| `ISC` | 許容 | `deny.toml` allow。Lucide アイコン等 |
| `LGPL-2.1-or-later` | 非選択の OR 代替 | `MIT OR Apache-2.0 OR LGPL-2.1-or-later`。許容側を選択 |
| `MIT` | 許容 | `deny.toml` allow |
| `MIT-0` | 許容 | `deny.toml` allow。`CC0-1.0 OR MIT-0 OR Apache-2.0` 等として出現 |
| `MPL-2.0` | 弱コピーレフト | `deny.toml` allow |
| `NCSA` | AND 条件 | `(MIT OR Apache-2.0) AND NCSA`。Unicode データ由来 |
| `Unicode-3.0` | 許容（Unicode データ） | `deny.toml` allow。各ワークスペースで 22 crates が単独 license 値とする |
| `Unlicense` | パブリックドメイン宣言 | `deny.toml` allow。`Unlicense OR MIT` / `Unlicense/MIT` として出現 |
| `Zlib` | 許容 | `deny.toml` allow |

`GPL-2.0-only` と `LGPL-2.1-or-later` は `cargo metadata` の OR 式に現れるが
選択されない代替であり、`NCSA` は AND 条件として許容ライセンスと併存する。
いずれも `cargo deny check` の許可範囲内で、個別の適用条件は同梱ライセンス
全文で確認できる。

## SBOM のスコープ

CycloneDX SBOM（CI 成果物 `gpui-task-manager-sbom`、`target/sbom/*.json`）は
2 つのワークスペースを**それぞれ 1 つのターゲット**で解決したスナップショット
である。

| BOM | ワークスペース | ターゲット |
| --- | --- | --- |
| `sbom-root*.json` | ルート（native） | `x86_64-unknown-linux-gnu` |
| `sbom-web*.json` | `web/`（ブラウザ） | `wasm32-unknown-unknown` |

各 BOM は既定 feature と当該ターゲットの依存クロージャのみを含む。したがって
`deny.toml` が監査する全ターゲット（Windows / macOS / aarch64 等）やオプション
feature を網羅するものではない。別ターゲット固有の依存を確認するには、その
ターゲットで SBOM を再生成するか、`cargo deny check` /
`cargo metadata --locked --filter-platform <target>` を参照すること。
SBOM の依存集合は生成時に同じターゲットの `cargo metadata` と照合され、
target の取り違え（例: web をホスト既定で生成する）を検出する。

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
同梱ライセンス全文は同 `licenses/`（クレート別は `licenses/crates/`、
個別クレート名・SPDX 式は各ファイルを参照）、未同梱ライセンスの確認先は同
`README.md` を参照してください。

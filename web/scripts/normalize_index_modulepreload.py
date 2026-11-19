#!/usr/bin/env python3
"""Trunk 生成の index.html の modulepreload 順を href のパス順へ正規化する。

Trunk 0.22.x は modulepreload タグを HashSet 由来の順序で書き出すため、
同一入力の 2 回ビルドで index.html の SHA256 が変動する（wasm / JS 本体は
毎回一致する）。タグ全体を移動するので integrity (SRI) / crossorigin
属性はそのまま保持される。

使い方:
  web/Trunk.toml の post_build フックから無引数で呼ばれる。
  対象は $TRUNK_STAGING_DIR/index.html（未設定時は web/dist/index.html）。
  Trunk は finalize 済み HTML を staging に書いてから post_build フックを
  実行し、その後に staging -> dist へ move するため staging を書き換える。
  --check を付けると書き込まず、未正規化なら終了コード 1 を返す。
"""

from __future__ import annotations

import argparse
import os
import re
import sys
from pathlib import Path

LINK_RE = re.compile(r"<link\b[^>]*>", re.IGNORECASE)
REL_RE = re.compile(r"""\brel\s*=\s*(?:"modulepreload"|'modulepreload')""", re.IGNORECASE)
HREF_RE = re.compile(r"""\bhref\s*=\s*("[^"]*"|'[^']*')""", re.IGNORECASE)


def default_target() -> Path:
    staging = os.environ.get("TRUNK_STAGING_DIR")
    if staging:
        return Path(staging) / "index.html"
    # 手動実行時: このファイルのある web/scripts/ の親 (web/) 配下の dist。
    return Path(__file__).resolve().parent.parent / "dist" / "index.html"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--file",
        type=Path,
        default=None,
        help="対象 index.html（既定: $TRUNK_STAGING_DIR/index.html または web/dist/index.html）",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="書き込まず、パス順でなければ終了コード 1",
    )
    return parser.parse_args()


def fail(message: str) -> int:
    print(f"normalize-index: {message}", file=sys.stderr)
    return 1


def main() -> int:
    args = parse_args()
    target = args.file or default_target()
    try:
        text = target.read_text(encoding="utf-8")
    except OSError as exc:
        return fail(f"cannot read {target}: {exc}")

    tags = [m for m in LINK_RE.finditer(text) if REL_RE.search(m.group(0))]
    if len(tags) < 2:
        # 0 個または 1 個なら順序の問題は無い（冪等）。
        return 0

    first, last = tags[0], tags[-1]
    span = text[first.start() : last.end()]

    # 安全側: タグ間に modulepreload 以外のマークアップが挟まっていたら並べ替えない。
    for other in LINK_RE.finditer(span):
        if not REL_RE.search(other.group(0)):
            return fail("non-modulepreload <link> is interleaved; refusing to reorder")
    if LINK_RE.sub("", span).strip():
        return fail("unexpected markup between modulepreload tags; refusing to reorder")

    keyed: list[tuple[str, str]] = []
    for tag in tags:
        href = HREF_RE.search(tag.group(0))
        if href is None:
            return fail("modulepreload tag without href; refusing to reorder")
        keyed.append((href.group(1).strip("\"'"), tag.group(0)))
    keyed.sort(key=lambda item: item[0])

    # 区切りは空文字に正規化する（ビルド間の空白差にも依存しない）。
    normalized = "".join(tag for _, tag in keyed)
    rebuilt = text[: first.start()] + normalized + text[last.end() :]

    if rebuilt == text:
        print(f"normalize-index: OK ({len(tags)} modulepreload tags already sorted: {target})")
        return 0
    if args.check:
        return fail(f"modulepreload tags are not in path order: {target}")
    try:
        target.write_text(rebuilt, encoding="utf-8")
    except OSError as exc:
        return fail(f"cannot write {target}: {exc}")
    print(f"normalize-index: sorted {len(tags)} modulepreload tags by href -> {target}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

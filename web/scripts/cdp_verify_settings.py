#!/usr/bin/env python3
"""ブラウザ版の設定永続化（localStorage）の Runtime Proof 取得スクリプト。

確認すること:
1. 列ヘッダをクリックすると `localStorage` に表示設定が保存される（保存経路）。
2. `localStorage` に設定を書いてから再読込すると、ソートが復元される（復元経路）。
3. 保存される JSON は `tm-viewmodel::Settings` のスキーマ（version / interval_ms /
   sort / details_visible / tree_view / theme）に一致し、プロセス情報やトークンを
   含まない。
4. ヘッダーのテーマ切替ボタンで Dark ↔ Light が切り替わり、状態フックの `theme=`
   と localStorage の両方に反映される（復元経路も `theme=light` を確認する）。

前提（別シェルで起動しておく）:
    cd work/gpui-task-manager/web/dist && python3 -m http.server 8091 --bind 127.0.0.1
    google-chrome --headless=new --no-sandbox --disable-dev-shm-usage \\
      --window-size=1440,1000 --remote-debugging-port=9223 \\
      --user-data-dir=/tmp/tm-web-chrome about:blank

使い方:
    # CI ゲート: ブラウザ/サーバ不要のオフライン自己検証（失敗時は終了コード 1）
    python3 web/scripts/cdp_verify_settings.py --self-check

    # 実機 CDP 検証（開発ホストで手動実行。上記の Chrome / 静的サーバが前提）
    python3 web/scripts/cdp_verify_settings.py \\
      --url http://127.0.0.1:8091/ --evidence /tmp/settings-evidence.json

クリック座標の基準は DOM（canvas の getBoundingClientRect / window.innerWidth）から
取得し、src/shell.rs レイアウト定数との手工同期を廃止している。状態フック
body[data-gpui-proof] は従来どおり記録・検証する。
"""

from __future__ import annotations

import argparse
import asyncio
import json
import pathlib
import time

import cdp_verify_web as helper

STORAGE_KEY = "gpui-task-manager.settings"

# tm-viewmodel::Settings のスキーマ（#[serde(default)] で未来のキーは許容）。
REQUIRED_SETTING_KEYS = {
    "version",
    "interval_ms",
    "sort",
    "details_visible",
    "tree_view",
    "theme",
}
# 復元経路のオフライン検証で使う入力。main() と self_check() で共有する。
RESTORED_SETTINGS = {
    "version": 2,
    "interval_ms": 5000,
    "sort": {"key": "Name", "direction": "Ascending"},
    "details_visible": False,
    "tree_view": True,
    "theme": "Light",
}

# ヘッダー右端のテーマ切替ボタン（src/shell.rs のレイアウト定数と同期）。
# ボタンは右グループの最後の子で固定幅 56 px、ヘッダー高さ 52 px。
ROOT_PAD = 12.0
HEADER_H = 52.0
THEME_BTN_W = 56.0
THEME_BTN_CENTER_Y = ROOT_PAD + HEADER_H / 2.0


def theme_button_x(viewport_width: float) -> float:
    return viewport_width - ROOT_PAD - THEME_BTN_W / 2.0


async def wait_for_proof(cdp: helper.CDP, timeout: float = 20.0) -> dict:
    deadline = time.time() + timeout
    while time.time() < deadline:
        env = await cdp.evaluate(helper.PROOF_EXPR)
        if isinstance(env, dict):
            proof = helper.parse_proof(env.get("proof"))
            if proof.get("rows") is not None:
                return proof
        await asyncio.sleep(0.4)
    return {}


def self_check() -> int:
    """オフライン自己検証（CI ゲート）。ブラウザもサーバも起動しない。

    設定スキーマの必須キー・復元用フィクスチャ・テーマボタン座標と、
    web/scripts 内からの共有ヘルパー import を検証する。
    """
    failures: list[str] = []

    def check(condition: bool, message: str) -> None:
        if not condition:
            failures.append(message)
            print(f"  FAIL: {message}")

    check(STORAGE_KEY == "gpui-task-manager.settings", "localStorage キーが維持されている")
    check(
        REQUIRED_SETTING_KEYS.issubset(RESTORED_SETTINGS.keys()),
        f"復元用設定が必須キーを持つ（実際: {sorted(RESTORED_SETTINGS.keys())}）",
    )
    check(RESTORED_SETTINGS.get("version") == 2, "復元用設定の version が 2")
    check(RESTORED_SETTINGS.get("theme") == "Light", "復元用設定の theme が Light")
    check(
        "token" not in json.dumps(RESTORED_SETTINGS).lower(),
        "復元用設定にトークンが含まれない",
    )
    check(theme_button_x(1440.0) == 1400.0, "テーマボタンの x 座標が計算できる")
    check(theme_button_x(1000.0) == 960.0, "テーマボタンの x 座標が幅に追従する")
    check(THEME_BTN_CENTER_Y == 38.0, "テーマボタンの y 座標がヘッダー内")
    check(
        helper.parse_proof("theme=light;rows=3").get("theme") == "light",
        "共有 parse_proof を web/scripts から import できる",
    )
    check(
        set(helper.column_centers(1440.0)) >= {"name", "cpu"},
        "共有 column_centers を web/scripts から import できる",
    )
    check(
        helper.center_from_rect({"left": 100.0, "top": 0.0, "width": 56.0, "height": 36.0})
        == (128.0, 18.0),
        "共有 center_from_rect で DOM rect から座標を取得できる",
    )
    check(
        helper.CANVAS_SELECTOR == "canvas" and helper.FILTER_SELECTOR == "textarea",
        "共有 DOM セレクタが維持されている",
    )
    check(
        "getBoundingClientRect" in helper.rect_expr(helper.CANVAS_SELECTOR),
        "共有 rect 取得式が DOM 座標を使う",
    )

    passed = not failures
    print(
        json.dumps(
            {"script": "cdp_verify_settings.py", "mode": "self-check",
             "passed": passed, "failures": failures},
            ensure_ascii=False,
        )
    )
    return 0 if passed else 1


async def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=9223)
    parser.add_argument("--url", default="http://127.0.0.1:8091/")
    parser.add_argument("--evidence", default="/tmp/settings-evidence.json")
    parser.add_argument(
        "--self-check",
        action="store_true",
        help="CDP を使わずオフラインで設定スキーマを検証する（CI ゲート用）",
    )
    args = parser.parse_args()

    if args.self_check:
        return self_check()

    cdp = helper.CDP()
    await cdp.connect(helper.pick_page_target(args.port))
    await cdp.call("Page.enable")
    await cdp.call("Runtime.enable")

    failures: list[str] = []
    evidence: dict = {
        "url": args.url,
        "started_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "steps": [],
    }

    def check(condition: bool, message: str) -> None:
        if not condition:
            failures.append(message)
            print(f"  FAIL: {message}")

    # 1) 初期化: 保存済み設定を消してから読み込む。
    await cdp.call("Page.navigate", {"url": "about:blank"})
    await asyncio.sleep(0.5)
    await cdp.call("Page.navigate", {"url": args.url})
    await asyncio.sleep(1.0)
    await cdp.evaluate(f"localStorage.removeItem({json.dumps(STORAGE_KEY)});")
    await cdp.call("Page.reload", {"ignoreCache": True})
    proof = await wait_for_proof(cdp)
    check(bool(proof), "状態フックが取得できる")
    check(proof.get("theme") == "dark", f"既定テーマが dark（実際: {proof.get('theme')}）")
    evidence["steps"].append({"step": "fresh", "proof": proof})

    # 2) 保存経路: 列ヘッダをクリックして localStorage に書かれることを確認する。
    # 座標基準を DOM から取得する（状態フックの vw には依存しない）。
    viewport_width, viewport_height = await helper.viewport_size(cdp)
    check(viewport_width > 0 and viewport_height > 0, "viewport サイズを DOM から取得できる")
    canvas_rect = await helper.element_rect(cdp, helper.CANVAS_SELECTOR)
    ox, oy = float(canvas_rect["left"]), float(canvas_rect["top"])
    centers = helper.column_centers(viewport_width)
    evidence["layout"] = {
        "column_centers": centers,
        "header_y": helper.HEADER_Y,
        "dom_geometry": {
            "viewport": {"width": viewport_width, "height": viewport_height},
            "canvas_rect": canvas_rect,
            "hook_vw": proof.get("vw"),
        },
    }
    await helper.click(cdp, ox + centers["cpu"], oy + helper.HEADER_Y)
    await asyncio.sleep(1.2)
    raw = await cdp.evaluate(f"localStorage.getItem({json.dumps(STORAGE_KEY)})")
    evidence["steps"].append({"step": "saved", "stored": raw})
    check(isinstance(raw, str) and bool(raw), "列クリックで設定が保存される")
    saved: dict = {}
    if isinstance(raw, str) and raw:
        try:
            saved = json.loads(raw)
        except json.JSONDecodeError as error:
            check(False, f"保存された設定が JSON ではない: {error}")
    check(
        REQUIRED_SETTING_KEYS.issubset(saved.keys()),
        f"必須キーが保存される（実際: {sorted(saved.keys())}）",
    )
    check("token" not in json.dumps(saved).lower(), "トークンが保存されていない")
    check(saved.get("theme") == "Dark", f"保存されたテーマが Dark（実際: {saved.get('theme')}）")
    evidence["steps"].append({"step": "saved_parsed", "settings": saved})

    # 3) 復元経路: 設定を書いて再読込し、ソートが反映されることを確認する。
    restored = RESTORED_SETTINGS
    await cdp.evaluate(
        f"localStorage.setItem({json.dumps(STORAGE_KEY)}, {json.dumps(json.dumps(restored))});"
    )
    await cdp.call("Page.reload", {"ignoreCache": True})
    proof = await wait_for_proof(cdp)
    evidence["steps"].append({"step": "restored", "proof": proof})
    check(proof.get("sort") == "名前", f"ソート列が復元される（実際: {proof.get('sort')}）")
    check(proof.get("dir") == "asc", f"ソート方向が復元される（実際: {proof.get('dir')}）")
    check(proof.get("theme") == "light", f"テーマが復元される（実際: {proof.get('theme')}）")
    check(
        proof.get("interval") == "5000",
        f"更新間隔が復元される（実際: {proof.get('interval')}）",
    )

    # 4) 切替ボタン: Light → Dark → Light と状態フックと保存値の両方を確認する。
    theme_x = ox + theme_button_x(viewport_width)
    theme_y = oy + THEME_BTN_CENTER_Y
    for expected, stored in (("dark", "Dark"), ("light", "Light")):
        await helper.click(cdp, theme_x, theme_y)
        await asyncio.sleep(0.8)
        proof = await wait_for_proof(cdp)
        evidence["steps"].append({"step": f"toggled_{expected}", "proof": proof})
        check(
            proof.get("theme") == expected,
            f"切替ボタンで theme={expected}（実際: {proof.get('theme')}）",
        )
        raw = await cdp.evaluate(f"localStorage.getItem({json.dumps(STORAGE_KEY)})")
        current: dict = {}
        if isinstance(raw, str) and raw:
            try:
                current = json.loads(raw)
            except json.JSONDecodeError as error:
                check(False, f"切替後の設定が JSON ではない: {error}")
        check(
            current.get("theme") == stored,
            f"切替後の保存テーマが {stored}（実際: {current.get('theme')}）",
        )

    evidence["failures"] = failures
    evidence["passed"] = not failures
    evidence["finished_at"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    path = pathlib.Path(args.evidence)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(evidence, ensure_ascii=False, indent=2))

    print(f"evidence: {path}")
    print(json.dumps({"passed": not failures, "failures": failures}, ensure_ascii=False))
    return 0 if not failures else 1


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))

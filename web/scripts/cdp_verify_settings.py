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
    python3 web/scripts/cdp_verify_settings.py \\
      --url http://127.0.0.1:8091/ --evidence /tmp/settings-evidence.json
"""

from __future__ import annotations

import argparse
import asyncio
import json
import pathlib
import time

import cdp_verify_web as helper

STORAGE_KEY = "gpui-task-manager.settings"

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


async def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=9223)
    parser.add_argument("--url", default="http://127.0.0.1:8091/")
    parser.add_argument("--evidence", default="/tmp/settings-evidence.json")
    args = parser.parse_args()

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
    viewport_width = float(proof.get("vw", "0") or 0)
    check(viewport_width > 0, "viewport 幅を取得できる")
    centers = helper.column_centers(viewport_width)
    evidence["layout"] = {"column_centers": centers, "header_y": helper.HEADER_Y}
    await helper.click(cdp, centers["cpu"], helper.HEADER_Y)
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
    # Required keys must be present; extra keys are allowed for forward
    # compatibility (the shared schema uses `#[serde(default)]`).
    required = {"version", "interval_ms", "sort", "details_visible", "tree_view", "theme"}
    check(
        required.issubset(saved.keys()),
        f"必須キーが保存される（実際: {sorted(saved.keys())}）",
    )
    check("token" not in json.dumps(saved).lower(), "トークンが保存されていない")
    check(saved.get("theme") == "Dark", f"保存されたテーマが Dark（実際: {saved.get('theme')}）")
    evidence["steps"].append({"step": "saved_parsed", "settings": saved})

    # 3) 復元経路: 設定を書いて再読込し、ソートが反映されることを確認する。
    restored = {
        "version": 2,
        "interval_ms": 5000,
        "sort": {"key": "Name", "direction": "Ascending"},
        "details_visible": False,
        "tree_view": True,
        "theme": "Light",
    }
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
    theme_x = theme_button_x(viewport_width)
    for expected, stored in (("dark", "Dark"), ("light", "Light")):
        await helper.click(cdp, theme_x, THEME_BTN_CENTER_Y)
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

#!/usr/bin/env python3
"""GPUI Web スパイクの Runtime Proof 取得スクリプト。

**証拠収集のみ**（R1S-OPS-12）。操作検証（成否判定）ではない。
ヘッドレス Chrome を CDP（Chrome DevTools Protocol）で操作し、
描画・クリック・キー入力・ホイールスクロール・定期更新・HTTP fetch の証拠を
スクリーンショットとコンソールログとして保存する。CDP 入力は画面状態の証拠を
残すための補助であり、操作そのものの成功・失敗をゲートしない。

クリック座標の基準は DOM（canvas の getBoundingClientRect / textarea セレクタ）から
取得する。レイアウト定数との手工同期は行わない。状態フック
body[data-gpui-proof] は従来どおり証拠として記録する。

使い方:
    python3 cdp_verify.py --url http://127.0.0.1:8090/ --out ../../notes/assets \
        --prefix wasm-spike

    # CI ゲート: ブラウザ/サーバ不要のオフライン自己検証（失敗時は終了コード 1）
    python3 cdp_verify.py --self-check

Chrome 側は別途起動しておく:
    google-chrome --headless=new --no-sandbox --disable-dev-shm-usage \
        --window-size=1200,800 --remote-debugging-port=9222 about:blank
"""

from __future__ import annotations

import argparse
import asyncio
import base64
import json
import pathlib
import time
import urllib.request

# websockets は CDP 接続時だけ読み込む: --self-check（CI ゲート）は不要。
CANVAS_SELECTOR = "canvas"
FILTER_SELECTOR = "textarea"  # 検索欄の DOM 表現（存在する場合はセレクタ取得）

# 旧固定座標（1200x800）を DOM rect の比率へ変換した係数。
WHEEL_POINT = (0.5, 470.0 / 800.0)
ROW_POINT = (400.0 / 1200.0, 396.0 / 800.0)
SEARCH_POINT = (0.5, 313.0 / 800.0)
BUTTON_POINT = (1080.0 / 1200.0, 622.0 / 800.0)


def rect_expr(selector: str) -> str:
    """セレクタの getBoundingClientRect() を返す JS 式を組み立てる。"""
    return """(() => {
  const el = document.querySelector(%s);
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return {
    x: r.x, y: r.y, width: r.width, height: r.height,
    left: r.left, top: r.top, right: r.right, bottom: r.bottom,
  };
})()""" % json.dumps(selector)


def center_from_rect(rect: dict | None) -> tuple[float, float] | None:
    """DOM rect の中心座標を返す（ブラウザ不要の純粋関数）。"""
    if not rect:
        return None
    return (
        float(rect["left"]) + float(rect["width"]) / 2.0,
        float(rect["top"]) + float(rect["height"]) / 2.0,
    )


def relative_point(rect: dict, fx: float, fy: float) -> tuple[float, float]:
    """DOM rect の左上と比率から座標を計算する（固定座標の置き換え）。"""
    return (
        float(rect["left"]) + float(rect["width"]) * fx,
        float(rect["top"]) + float(rect["height"]) * fy,
    )


async def element_rect(cdp: "CDP", selector: str) -> dict:
    rect = await cdp.evaluate(rect_expr(selector))
    if not isinstance(rect, dict):
        raise RuntimeError(f"DOM 要素が見つかりません: {selector}")
    return rect


class CDP:
    def __init__(self) -> None:
        self.ws = None
        self.events: list[dict] = []
        self.pending: dict[int, asyncio.Future] = {}
        self._id = 0
        self._reader: asyncio.Task | None = None

    async def connect(self, ws_url: str) -> None:
        import websockets

        self.ws = await websockets.connect(ws_url, max_size=128 * 1024 * 1024)
        self._reader = asyncio.create_task(self._read_loop())

    async def _read_loop(self) -> None:
        try:
            async for raw in self.ws:
                message = json.loads(raw)
                if "id" in message and message["id"] in self.pending:
                    future = self.pending.pop(message["id"])
                    if not future.done():
                        future.set_result(message)
                else:
                    self.events.append(message)
        except Exception as error:  # noqa: BLE001 - 検証スクリプトなので握りつぶして記録
            self.events.append({"method": "__reader_closed__", "error": repr(error)})
            self._fail_pending(error)

    def _fail_pending(self, error: Exception) -> None:
        """reader 終了時に in-flight の call() を例外で解除する（R2-CDP）。

        ページ/ブラウザが消えた場合に call() が 120 秒タイムアウトまで
        ハングするのを防ぐ。web/scripts/cdp_verify_web.py と同じ処理。
        """
        for future in self.pending.values():
            if not future.done():
                future.set_exception(error)

    async def call(self, method: str, params: dict | None = None):
        self._id += 1
        future = asyncio.get_event_loop().create_future()
        self.pending[self._id] = future
        await self.ws.send(
            json.dumps({"id": self._id, "method": method, "params": params or {}})
        )
        message = await asyncio.wait_for(future, timeout=120)
        if "error" in message:
            raise RuntimeError(f"{method} failed: {message['error']}")
        return message.get("result", {})

    async def evaluate(self, expression: str, await_promise: bool = False):
        result = await self.call(
            "Runtime.evaluate",
            {
                "expression": expression,
                "returnByValue": True,
                "awaitPromise": await_promise,
            },
        )
        if "exceptionDetails" in result:
            return {"__exception__": result["exceptionDetails"]}
        return result.get("result", {}).get("value")

    async def drain(self) -> None:
        # イベントを読むために軽い評価を1回投げる（リーダーが受信済みの分を処理する）
        await self.evaluate("1")
        await asyncio.sleep(0.1)
        await self.evaluate("1")


def pick_page_target(port: int) -> str:
    url = f"http://127.0.0.1:{port}/json/list"
    with urllib.request.urlopen(url, timeout=10) as response:
        targets = json.load(response)
    for target in targets:
        if target.get("type") == "page":
            return target["webSocketDebuggerUrl"]
    raise RuntimeError(f"page target が見つかりません: {targets}")


async def screenshot(cdp: CDP, path: pathlib.Path) -> None:
    result = await cdp.call("Page.captureScreenshot", {"format": "png"})
    path.write_bytes(base64.b64decode(result["data"]))
    print(f"  screenshot: {path}")


async def click(cdp: CDP, x: float, y: float) -> None:
    await cdp.call(
        "Input.dispatchMouseEvent",
        {"type": "mouseMoved", "x": x, "y": y, "button": "none"},
    )
    await asyncio.sleep(0.05)
    await cdp.call(
        "Input.dispatchMouseEvent",
        {
            "type": "mousePressed",
            "x": x,
            "y": y,
            "button": "left",
            "buttons": 1,
            "clickCount": 1,
        },
    )
    await asyncio.sleep(0.05)
    await cdp.call(
        "Input.dispatchMouseEvent",
        {
            "type": "mouseReleased",
            "x": x,
            "y": y,
            "button": "left",
            "buttons": 0,
            "clickCount": 1,
        },
    )


async def type_text(cdp: CDP, text: str) -> None:
    for character in text:
        code = ord(character.upper())
        key = character
        await cdp.call(
            "Input.dispatchKeyEvent",
            {
                "type": "keyDown",
                "key": key,
                "code": f"Key{character.upper()}",
                "text": character,
                "unmodifiedText": character,
                "windowsVirtualKeyCode": code,
                "nativeVirtualKeyCode": code,
            },
        )
        await cdp.call(
            "Input.dispatchKeyEvent",
            {
                "type": "keyUp",
                "key": key,
                "code": f"Key{character.upper()}",
                "windowsVirtualKeyCode": code,
                "nativeVirtualKeyCode": code,
            },
        )
        await asyncio.sleep(0.06)


async def press_key(cdp: CDP, key: str, code: str, vk: int) -> None:
    await cdp.call(
        "Input.dispatchKeyEvent",
        {
            "type": "keyDown",
            "key": key,
            "code": code,
            "windowsVirtualKeyCode": vk,
            "nativeVirtualKeyCode": vk,
        },
    )
    await cdp.call(
        "Input.dispatchKeyEvent",
        {
            "type": "keyUp",
            "key": key,
            "code": code,
            "windowsVirtualKeyCode": vk,
            "nativeVirtualKeyCode": vk,
        },
    )
    await asyncio.sleep(0.1)


async def wheel(cdp: CDP, x: float, y: float, delta_y: float) -> None:
    await cdp.call(
        "Input.dispatchMouseEvent",
        {
            "type": "mouseWheel",
            "x": x,
            "y": y,
            "deltaX": 0,
            "deltaY": delta_y,
        },
    )
    await asyncio.sleep(0.4)


def console_texts(events: list[dict]) -> list[dict]:
    messages = []
    for event in events:
        method = event.get("method")
        params = event.get("params", {})
        if method == "Runtime.consoleAPICalled":
            args = []
            for arg in params.get("args", []):
                value = arg.get("value")
                if value is None:
                    value = arg.get("description") or arg.get("type")
                args.append(value)
            messages.append(
                {
                    "source": "console",
                    "type": params.get("type"),
                    "text": " ".join(str(arg) for arg in args),
                    "timestamp": params.get("timestamp"),
                }
            )
        elif method == "Log.entryAdded":
            entry = params.get("entry", {})
            messages.append(
                {
                    "source": "log",
                    "type": entry.get("level"),
                    "text": f"[{entry.get('source')}] {entry.get('text')}",
                    "timestamp": entry.get("timestamp"),
                }
            )
        elif method == "Runtime.exceptionThrown":
            details = params.get("exceptionDetails", {})
            exception = details.get("exception", {})
            messages.append(
                {
                    "source": "exception",
                    "type": "error",
                    "text": exception.get("description") or details.get("text"),
                    "timestamp": details.get("timestamp"),
                }
            )
    return messages


ENV_PROBE = """
(() => {
  const canvas = document.querySelector('canvas');
  const probe = document.createElement('canvas');
  const webgl2 = !!probe.getContext('webgl2');
  return {
    userAgent: navigator.userAgent,
    webgpu: !!navigator.gpu,
    webgl2,
    canvas: canvas ? {
      width: canvas.width,
      height: canvas.height,
      clientWidth: canvas.clientWidth,
      clientHeight: canvas.clientHeight,
      connected: canvas.isConnected,
    } : null,
    title: document.title,
    proof: document.body ? document.body.getAttribute('data-gpui-proof') : null,
    bodyChildren: document.body ? document.body.children.length : 0,
  };
})()
"""


def self_check() -> int:
    """オフライン自己検証（CI ゲート）。ブラウザもサーバも起動しない。

    DOM rect 由来の座標計算とセレクタ取得式、状態フック式に加え、
    reader クローズ時に pending future が例外で解除されることを検証する。
    実機の証拠収集は通常実行（--url / --port）で行う。
    """
    failures: list[str] = []

    def check(condition: bool, message: str) -> None:
        if not condition:
            failures.append(message)
            print(f"  FAIL: {message}")

    rect = {
        "left": 10.0, "top": 20.0, "width": 1200.0, "height": 800.0,
        "right": 1210.0, "bottom": 820.0,
    }
    check(center_from_rect(rect) == (610.0, 420.0), "rect から中心座標が計算できる")
    check(center_from_rect(None) is None, "要素が無い場合は None になる")
    check(
        relative_point(rect, *WHEEL_POINT) == (610.0, 490.0),
        "DOM rect 比率からスクロール座標が計算できる",
    )
    check(
        relative_point(rect, *ROW_POINT) == (410.0, 416.0),
        "DOM rect 比率から行クリック座標が計算できる",
    )
    check(
        relative_point(rect, *BUTTON_POINT) == (1090.0, 642.0),
        "DOM rect 比率からボタン座標が計算できる",
    )
    check(CANVAS_SELECTOR == "canvas" and FILTER_SELECTOR == "textarea", "DOM セレクタが維持されている")
    dom_expr = rect_expr(FILTER_SELECTOR)
    check("querySelector" in dom_expr and '"textarea"' in dom_expr, "rect 取得式にセレクタが埋め込まれる")
    check("getBoundingClientRect" in dom_expr, "getBoundingClientRect で DOM 座標を取得する")
    check("data-gpui-proof" in ENV_PROBE, "DOM 状態フック式が維持されている")

    # CDP 切断時のハング解消: reader クローズで pending future が例外解除されること
    # （websockets / ブラウザ不要のオフライン検証）。
    loop = asyncio.new_event_loop()
    try:
        cdp = CDP()
        pending = loop.create_future()
        cdp.pending[1] = pending
        cdp._fail_pending(RuntimeError("reader closed"))
        check(pending.done(), "reader クローズ時に pending future が done になる")
        check(
            pending.done() and isinstance(pending.exception(), RuntimeError),
            "reader クローズ時に pending が例外で解除される",
        )
    finally:
        loop.close()

    passed = not failures
    print(
        json.dumps(
            {"script": "cdp_verify.py", "mode": "self-check",
             "passed": passed, "failures": failures},
            ensure_ascii=False,
        )
    )
    return 0 if passed else 1


async def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=9222)
    parser.add_argument("--url", default="http://127.0.0.1:8090/")
    parser.add_argument("--out", default="../../notes/assets")
    parser.add_argument("--prefix", default="wasm-spike")
    parser.add_argument("--label", default="default", help="Chrome 起動条件のラベル")
    parser.add_argument("--wait", type=float, default=5.0)
    parser.add_argument(
        "--self-check",
        action="store_true",
        help="CDP を使わずオフラインで DOM 座標ロジックを検証する（CI ゲート用）",
    )
    args = parser.parse_args()

    if args.self_check:
        return self_check()

    out_dir = pathlib.Path(args.out).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)

    ws_url = pick_page_target(args.port)
    cdp = CDP()
    await cdp.connect(ws_url)
    await cdp.call("Page.enable")
    await cdp.call("Runtime.enable")
    await cdp.call("Log.enable")

    evidence: dict = {
        "label": args.label,
        "url": args.url,
        "started_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "steps": [],
    }

    await cdp.call("Page.navigate", {"url": args.url})
    await asyncio.sleep(args.wait)
    await cdp.drain()

    env = await cdp.evaluate(ENV_PROBE)
    evidence["env"] = env
    # クリック座標の基準を DOM から取得する（固定座標 / レイアウト定数に依存しない）。
    canvas_rect = await element_rect(cdp, CANVAS_SELECTOR)
    evidence["layout"] = {"canvas_rect": canvas_rect}
    title_initial = await cdp.evaluate("document.title")
    proof_initial = await cdp.evaluate("document.body.getAttribute('data-gpui-proof')")
    dom_html = await cdp.evaluate("document.documentElement.outerHTML")
    evidence["steps"].append(
        {
            "step": "initial",
            "title": title_initial,
            "proof": proof_initial,
            "dom_html": dom_html,
        }
    )
    await screenshot(cdp, out_dir / f"{args.prefix}-01-initial.png")

    # 1 秒タイマーによる更新の確認（tick が増える）
    await asyncio.sleep(2.5)
    await cdp.drain()
    title_after_ticks = await cdp.evaluate("document.title")
    proof_after_ticks = await cdp.evaluate(
        "document.body.getAttribute('data-gpui-proof')"
    )
    evidence["steps"].append(
        {
            "step": "after_ticks",
            "title": title_after_ticks,
            "proof": proof_after_ticks,
        }
    )
    await screenshot(cdp, out_dir / f"{args.prefix}-02-after-ticks.png")

    # ホイールスクロール（DOM の canvas rect を基準に表のスクロール領域へ）
    scroll_x, scroll_y = relative_point(canvas_rect, *WHEEL_POINT)
    await wheel(cdp, scroll_x, scroll_y, 360)
    await cdp.drain()
    evidence["steps"].append(
        {
            "step": "scroll_down",
            "title": await cdp.evaluate("document.title"),
        }
    )
    await screenshot(cdp, out_dir / f"{args.prefix}-03-scrolled.png")
    # 先頭に戻す（後続の行クリック座標を安定させるため）
    await wheel(cdp, scroll_x, scroll_y, -720)
    await cdp.drain()

    # プロセス行クリック（DOM の canvas rect 比率から算出）
    row_x, row_y = relative_point(canvas_rect, *ROW_POINT)
    await click(cdp, row_x, row_y)
    await asyncio.sleep(0.6)
    await cdp.drain()
    evidence["steps"].append(
        {
            "step": "row_click",
            "title": await cdp.evaluate("document.title"),
            "proof": await cdp.evaluate(
                "document.body.getAttribute('data-gpui-proof')"
            ),
        }
    )
    await screenshot(cdp, out_dir / f"{args.prefix}-04-row-click.png")

    # 検索入力（DOM の textarea があればセレクタで取得、無ければ canvas 比率）
    try:
        search_center = center_from_rect(await element_rect(cdp, FILTER_SELECTOR))
        if search_center is None:
            raise RuntimeError(f"DOM 要素の座標が取得できません: {FILTER_SELECTOR}")
        search_x, search_y = search_center
        search_anchor = FILTER_SELECTOR
    except RuntimeError:
        search_x, search_y = relative_point(canvas_rect, *SEARCH_POINT)
        search_anchor = "canvas-relative-fallback"
    evidence["steps"].append(
        {"step": "search_anchor", "selector": search_anchor, "x": search_x, "y": search_y}
    )
    await click(cdp, search_x, search_y)
    await asyncio.sleep(0.4)
    await type_text(cdp, "http")
    await asyncio.sleep(0.5)
    await cdp.drain()
    evidence["steps"].append(
        {
            "step": "search_typed",
            "title": await cdp.evaluate("document.title"),
            "proof": await cdp.evaluate(
                "document.body.getAttribute('data-gpui-proof')"
            ),
        }
    )
    await screenshot(cdp, out_dir / f"{args.prefix}-05-search-typed.png")

    await press_key(cdp, "Backspace", "Backspace", 8)
    await press_key(cdp, "Backspace", "Backspace", 8)
    await asyncio.sleep(0.4)
    await cdp.drain()
    evidence["steps"].append(
        {
            "step": "search_backspace",
            "title": await cdp.evaluate("document.title"),
            "proof": await cdp.evaluate(
                "document.body.getAttribute('data-gpui-proof')"
            ),
        }
    )
    await screenshot(cdp, out_dir / f"{args.prefix}-06-search-backspace.png")

    # +1 ボタンのクリック（DOM の canvas rect 比率から算出）
    button_x, button_y = relative_point(canvas_rect, *BUTTON_POINT)
    await click(cdp, button_x, button_y)
    await asyncio.sleep(0.5)
    await cdp.drain()
    evidence["steps"].append(
        {
            "step": "button_click",
            "title": await cdp.evaluate("document.title"),
            "proof": await cdp.evaluate(
                "document.body.getAttribute('data-gpui-proof')"
            ),
        }
    )
    await screenshot(cdp, out_dir / f"{args.prefix}-07-button-click.png")

    # 最終状態（tick 更新後の値）
    await asyncio.sleep(2.0)
    await cdp.drain()
    evidence["steps"].append(
        {
            "step": "final",
            "title": await cdp.evaluate("document.title"),
            "proof": await cdp.evaluate(
                "document.body.getAttribute('data-gpui-proof')"
            ),
        }
    )
    await screenshot(cdp, out_dir / f"{args.prefix}-08-final.png")

    evidence["console"] = console_texts(cdp.events)
    evidence["raw_event_count"] = len(cdp.events)
    evidence["finished_at"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")

    evidence_path = out_dir / f"{args.prefix}-evidence-{args.label}.json"
    evidence_path.write_text(
        json.dumps(evidence, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    print(f"evidence: {evidence_path}")

    # 要点を標準出力にも出す（ログとして保存しやすいように）
    print("---- env ----")
    print(json.dumps(env, ensure_ascii=False, indent=2))
    print("---- console ----")
    for message in evidence["console"]:
        print(f"[{message['source']}/{message['type']}] {message['text']}")
    print("---- steps ----")
    for step in evidence["steps"]:
        print(json.dumps(step, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))

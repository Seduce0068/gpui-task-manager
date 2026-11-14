#!/usr/bin/env python3
"""GPUI Web スパイクの Runtime Proof 取得スクリプト。

ヘッドレス Chrome を CDP（Chrome DevTools Protocol）で操作し、
描画・クリック・キー入力・ホイールスクロール・定期更新・HTTP fetch の証拠を
スクリーンショットとコンソールログとして保存する。

使い方:
    python3 cdp_verify.py --url http://127.0.0.1:8090/ --out ../notes/assets \
        --prefix wasm-spike

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

import websockets


class CDP:
    def __init__(self) -> None:
        self.ws = None
        self.events: list[dict] = []
        self.pending: dict[int, asyncio.Future] = {}
        self._id = 0
        self._reader: asyncio.Task | None = None

    async def connect(self, ws_url: str) -> None:
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


async def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=9222)
    parser.add_argument("--url", default="http://127.0.0.1:8090/")
    parser.add_argument("--out", default="../notes/assets")
    parser.add_argument("--prefix", default="wasm-spike")
    parser.add_argument("--label", default="default", help="Chrome 起動条件のラベル")
    parser.add_argument("--wait", type=float, default=5.0)
    args = parser.parse_args()

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

    # ホイールスクロール（表のスクロール領域で下方向へ）
    await wheel(cdp, 600, 470, 360)
    await cdp.drain()
    evidence["steps"].append(
        {
            "step": "scroll_down",
            "title": await cdp.evaluate("document.title"),
        }
    )
    await screenshot(cdp, out_dir / f"{args.prefix}-03-scrolled.png")
    # 先頭に戻す（後続の行クリック座標を安定させるため）
    await wheel(cdp, 600, 470, -720)
    await cdp.drain()

    # プロセス行クリック（表の行選択 + クリック回数）
    await click(cdp, 400, 396)
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

    # 検索入力（クリックでフォーカス → キー入力 → Backspace）
    await click(cdp, 600, 313)
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

    # +1 ボタンのクリック
    await click(cdp, 1080, 622)
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

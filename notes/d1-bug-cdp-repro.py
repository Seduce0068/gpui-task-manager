#!/usr/bin/env python3
"""D1-BUG: reproduce sampler stall / App borrow errors via CDP (read-only investigation)."""
import asyncio
import json
import sys
import time
import urllib.request

import websockets

PORT = 9340
BASE = "http://127.0.0.1:%d" % PORT


async def cdp_targets():
    data = json.load(urllib.request.urlopen(BASE + "/json/list", timeout=5))
    return data


async def main():
    targets = await cdp_targets()
    page = next(t for t in targets if t.get("type") == "page")
    ws_url = page["webSocketDebuggerUrl"]
    async with websockets.connect(ws_url, max_size=None) as ws:
        seq = [0]
        pending = {}
        events = []
        started = time.time()

        async def send(method, params=None):
            seq[0] += 1
            mid = seq[0]
            fut = asyncio.get_running_loop().create_future()
            pending[mid] = fut
            await ws.send(json.dumps({"id": mid, "method": method, "params": params or {}}))
            return await fut

        async def recv_loop():
            try:
                async for raw in ws:
                    msg = json.loads(raw)
                    if "id" in msg and msg["id"] in pending:
                        pending.pop(msg["id"]).set_result(msg)
                    elif "method" in msg:
                        events.append({
                            "t_ms": round((time.time() - started) * 1000),
                            "method": msg["method"],
                            "params": msg.get("params", {}),
                        })
            except websockets.ConnectionClosed:
                pass

        rx = asyncio.create_task(recv_loop())
        await send("Runtime.enable")
        await send("Log.enable")
        await send("Page.enable")
        # fresh navigation so the clock starts at load
        await send("Page.navigate", {"url": "http://127.0.0.1:8092/"})

        samples = []
        for _ in range(60):
            await asyncio.sleep(0.25)
            res = await send("Runtime.evaluate", {
                "expression": "(document.body&&document.body.getAttribute('data-gpui-proof'))||''",
                "returnByValue": True,
            })
            proof = res.get("result", {}).get("result", {}).get("value", "")
            t_ms = round((time.time() - started) * 1000)
            samples.append({"t_ms": t_ms, "proof": proof})

        rx.cancel()
        try:
            await rx
        except asyncio.CancelledError:
            pass

        out = {
            "captured_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            "console_and_log_events": events,
            "proof_samples": samples,
        }
        with open("notes/assets/d1-bug-cdp-repro.json", "w") as f:
            json.dump(out, f, ensure_ascii=False, indent=2)
        print("samples:", len(samples))
        for s in samples[::4]:
            print(s["t_ms"], s["proof"][:90])
        print("--- console (error/warning) ---")
        for e in events:
            if e["method"] in ("Runtime.consoleAPICalled", "Log.entryAdded"):
                p = e["params"]
                lvl = p.get("type") or p.get("entry", {}).get("level")
                txt = p.get("args") or p.get("entry", {}).get("text")
                if lvl in ("error", "warning") or (isinstance(txt, str) and "borrow" in txt):
                    print(e["t_ms"], lvl, str(txt)[:500])


if __name__ == "__main__":
    asyncio.run(main())

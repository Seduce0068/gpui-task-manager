#!/usr/bin/env python3
"""日本語 IME（composition）対応の Runtime Proof 取得スクリプト。

ヘッドレス Chrome を CDP で操作し、`Input.imeSetComposition` /
`Input.insertText` で「にほんご」→「日本語」の変換・確定を再現して、
ブラウザ版 GPUI タスクマネージャーの絞り込み欄へ反映されることを確認する。

前提: `trunk build --release` 済みの `web/dist` があること。
スクリプト自身が静的サーバと Chrome を起動・停止する。

使い方（リポジトリルートから）:
    # CI ゲート: ブラウザ/サーバ不要のオフライン自己検証（失敗時は終了コード 1）
    python3 web/scripts/cdp_verify_ime.py --self-check

    # 実機 CDP 検証（このスクリプト自身が静的サーバと Chrome を起動・停止する）
    python3 web/scripts/cdp_verify_ime.py \
        --dist web/dist \
        --out notes/assets \
        --evidence /tmp/ime-evidence.json

詳細は notes/ime.md を参照。

証拠画像の clip 座標は DOM の textarea（IME ミラー）rect から取得し、
src/shell.rs レイアウト定数との手工同期を廃止している。状態フック
body[data-gpui-proof] は従来どおり解析・検証する。
"""

from __future__ import annotations

import argparse
import asyncio
import base64
import json
import pathlib
import shutil
import signal
import subprocess
import sys
import time
import urllib.request

# --- src/shell.rs のレイアウト定数と同期させること -------------------------
ROOT_PAD = 12.0
GAP = 12.0
HEADER_H = 52.0
OVERVIEW_H = 260.0
TOOLBAR_H = 36.0
TOOLBAR_TOP = ROOT_PAD + HEADER_H + GAP + OVERVIEW_H + GAP
FILTER_X = ROOT_PAD
FILTER_W = 340.0
FILTER_H = 30.0

# --- DOM/セレクタ取得（レイアウト定数との手工同期を廃止） ------------------
CANVAS_SELECTOR = "canvas"
FILTER_SELECTOR = "textarea"  # IME ミラー = 絞り込み欄の DOM 表現


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


def filter_crop(rect: dict) -> dict:
    """textarea の DOM rect から絞り込み欄の拡大 clip を計算する。"""
    return {
        "x": float(rect["left"]) - 8.0,
        "y": float(rect["top"]) - 8.0,
        "width": float(rect["width"]) + 16.0,
        "height": float(rect["height"]) + 16.0,
        "scale": 3,
    }


async def element_rect(cdp: CDP, selector: str) -> dict:
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
        # CDP 接続時だけ読み込む: --self-check（CI ゲート）は websockets 不要。
        import websockets

        self.ws = await websockets.connect(ws_url, max_size=128 * 1024 * 1024)
        self._reader = asyncio.create_task(self._read_loop())

    async def _read_loop(self) -> None:
        try:
            while True:
                message = json.loads(await self.ws.recv())
                if "id" in message and message["id"] in self.pending:
                    self.pending.pop(message["id"]).set_result(message)
                else:
                    self.events.append(message)
        except Exception as error:  # noqa: BLE001 - verification harness
            self.events.append({"method": "__reader_closed__", "error": repr(error)})
            # Unblock any in-flight call instead of letting it wait 120 s.
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

    async def evaluate(self, expression: str):
        result = await self.call(
            "Runtime.evaluate", {"expression": expression, "returnByValue": True}
        )
        if "exceptionDetails" in result:
            return {"__exception__": result["exceptionDetails"]}
        return result.get("result", {}).get("value")

    async def drain(self) -> None:
        await self.evaluate("1")
        await asyncio.sleep(0.1)
        await self.evaluate("1")


PROOF_EXPR = """
(() => {
  const canvas = document.querySelector('canvas');
  const textarea = document.querySelector('textarea');
  return {
    title: document.title,
    proof: document.body ? document.body.getAttribute('data-gpui-proof') : null,
    activeElement: document.activeElement ? document.activeElement.tagName : null,
    textarea: textarea ? {
      value: textarea.value,
      focused: document.activeElement === textarea,
      readOnly: textarea.readOnly,
      style: {
        position: textarea.style.position,
        width: textarea.style.width,
        height: textarea.style.height,
        opacity: textarea.style.opacity,
      },
    } : null,
    canvas: canvas ? {
      width: canvas.width,
      height: canvas.height,
      clientWidth: canvas.clientWidth,
      clientHeight: canvas.clientHeight,
    } : null,
    bodyChildren: document.body ? document.body.children.length : 0,
  };
})()
"""


def parse_proof(proof: str | None) -> dict:
    fields: dict = {}
    if not proof:
        return fields
    for part in proof.split(";"):
        if not part:
            continue
        key, _, value = part.partition("=")
        fields[key] = value
    return fields


async def state(cdp: CDP) -> dict:
    raw = await cdp.evaluate(PROOF_EXPR)
    fields = parse_proof(raw.get("proof") if isinstance(raw, dict) else None)
    return {"raw": raw, "proof": fields}


async def screenshot(cdp: CDP, path: pathlib.Path, clip: dict | None = None) -> None:
    params: dict = {"format": "png"}
    if clip:
        params["clip"] = {**clip, "scale": clip.get("scale", 1)}
    result = await cdp.call("Page.captureScreenshot", params)
    path.write_bytes(base64.b64decode(result["data"]))
    print(f"  screenshot: {path.name}")


async def click(cdp: CDP, x: float, y: float) -> None:
    await cdp.call("Input.dispatchMouseEvent", {"type": "mouseMoved", "x": x, "y": y, "button": "none"})
    await asyncio.sleep(0.05)
    await cdp.call(
        "Input.dispatchMouseEvent",
        {"type": "mousePressed", "x": x, "y": y, "button": "left", "buttons": 1, "clickCount": 1},
    )
    await asyncio.sleep(0.05)
    await cdp.call(
        "Input.dispatchMouseEvent",
        {"type": "mouseReleased", "x": x, "y": y, "button": "left", "buttons": 0, "clickCount": 1},
    )


async def key(cdp: CDP, name: str, code: str, vk: int, text: str | None = None) -> None:
    down: dict = {
        "type": "keyDown",
        "key": name,
        "code": code,
        "windowsVirtualKeyCode": vk,
        "nativeVirtualKeyCode": vk,
    }
    if text is not None:
        down["text"] = text
        down["unmodifiedText"] = text
    await cdp.call("Input.dispatchKeyEvent", down)
    await cdp.call(
        "Input.dispatchKeyEvent",
        {"type": "keyUp", "key": name, "code": code, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk},
    )
    await asyncio.sleep(0.2)


async def type_ascii(cdp: CDP, text: str) -> None:
    for character in text:
        await key(cdp, character, f"Key{character.upper()}", ord(character.upper()), text=character)


async def ime_compose(cdp: CDP, text: str, replace_len: int) -> None:
    """compositionstart / compositionupdate を発生させる。"""
    await cdp.call(
        "Input.imeSetComposition",
        {
            "text": text,
            "selectionStart": len(text),
            "selectionEnd": len(text),
            "replacementStart": 0,
            "replacementEnd": replace_len,
        },
    )
    await asyncio.sleep(0.35)


async def ime_commit(cdp: CDP, text: str) -> None:
    """compositionend を発生させて確定する（Chrome の IME 確定と同じ経路）。"""
    await cdp.call("Input.insertText", {"text": text})
    await asyncio.sleep(0.5)


def start_http_server(dist: pathlib.Path, port: int) -> subprocess.Popen:
    process = subprocess.Popen(
        [sys.executable, "-m", "http.server", str(port), "--bind", "127.0.0.1"],
        cwd=dist,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    for _ in range(40):
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=1):
                return process
        except Exception:
            time.sleep(0.25)
    raise RuntimeError("静的サーバが起動しません")


def start_chrome(port: int, profile: pathlib.Path) -> tuple[subprocess.Popen, str]:
    shutil.rmtree(profile, ignore_errors=True)
    binary = shutil.which("google-chrome") or shutil.which("chromium") or shutil.which("microsoft-edge")
    if binary is None:
        raise RuntimeError("Chrome / Chromium が見つかりません")
    process = subprocess.Popen(
        [
            binary,
            "--headless=new",
            "--no-sandbox",
            "--disable-dev-shm-usage",
            "--window-size=1440,1000",
            f"--remote-debugging-port={port}",
            f"--user-data-dir={profile}",
            "about:blank",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    for _ in range(80):
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/json/version", timeout=1) as response:
                version = json.load(response)
                browser = version.get("Browser", "")
                print(f"  chrome: {browser}")
                return process, browser
        except Exception:
            time.sleep(0.25)
    raise RuntimeError("Chrome が起動しません")


def pick_page_target(port: int) -> str:
    with urllib.request.urlopen(f"http://127.0.0.1:{port}/json/list", timeout=10) as response:
        targets = json.load(response)
    for target in targets:
        if target.get("type") == "page":
            return target["webSocketDebuggerUrl"]
    raise RuntimeError(f"page target が見つかりません: {targets}")


def self_check() -> int:
    """オフライン自己検証（CI ゲート）。ブラウザもサーバも起動しない。

    IME 検証で使う状態フック解析・ツールバー座標・フック式だけを検証する。
    実機の composition 挙動は通常実行（サーバ/Chrome を自己起動）で確認する。
    """
    failures: list[str] = []

    def check(condition: bool, message: str) -> None:
        if not condition:
            failures.append(message)
            print(f"  FAIL: {message}")

    parsed = parse_proof("tick=2;filter=fire;ime_composing=1;ime_marked=0:4")
    check(parsed.get("tick") == "2", "tick が解析できる")
    check(parsed.get("ime_composing") == "1", "変換中フラグが解析できる")
    check(parsed.get("ime_marked") == "0:4", "marked range が解析できる")
    check(parse_proof(None) == {}, "空のフックは空辞書")
    check(TOOLBAR_TOP == 348.0, "ツールバー上端がレイアウト定数と一致する")
    check(FILTER_X == ROOT_PAD, "絞り込み欄の x がルート余白と一致する")
    check(FILTER_W > 0 and FILTER_H > 0, "絞り込み欄のサイズが正")
    check(
        "data-gpui-proof" in PROOF_EXPR
        and "textarea" in PROOF_EXPR
        and "activeElement" in PROOF_EXPR,
        "IME 状態フック式が維持されている",
    )

    # DOM/セレクタ取得: rect 中心と clip 算出をオフラインで検証する。
    rect = {
        "left": 12.0, "top": 348.0, "width": 340.0, "height": 30.0,
        "right": 352.0, "bottom": 378.0,
    }
    check(center_from_rect(rect) == (182.0, 363.0), "rect から中心座標が計算できる")
    check(center_from_rect(None) is None, "要素が無い場合は None になる")
    check(
        filter_crop(rect)
        == {"x": 4.0, "y": 340.0, "width": 356.0, "height": 46.0, "scale": 3},
        "DOM rect から証拠 clip が計算できる",
    )
    check(CANVAS_SELECTOR == "canvas" and FILTER_SELECTOR == "textarea", "DOM セレクタが維持されている")
    check("getBoundingClientRect" in rect_expr(FILTER_SELECTOR), "getBoundingClientRect で DOM 座標を取得する")

    passed = not failures
    print(
        json.dumps(
            {"script": "cdp_verify_ime.py", "mode": "self-check",
             "passed": passed, "failures": failures},
            ensure_ascii=False,
        )
    )
    return 0 if passed else 1


async def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dist", default="work/gpui-task-manager/web/dist")
    parser.add_argument("--out", default="work/gpui-task-manager/notes/assets")
    parser.add_argument("--prefix", default="ime")
    parser.add_argument("--http-port", type=int, default=8091)
    parser.add_argument("--cdp-port", type=int, default=9223)
    parser.add_argument("--evidence", default="/tmp/ime-evidence.json")
    parser.add_argument(
        "--self-check",
        action="store_true",
        help="CDP を使わずオフラインで状態フック/座標を検証する（CI ゲート用）",
    )
    args = parser.parse_args()

    if args.self_check:
        return self_check()

    dist = pathlib.Path(args.dist).resolve()
    out = pathlib.Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)

    http_server = start_http_server(dist, args.http_port)
    chrome, browser_version = start_chrome(args.cdp_port, pathlib.Path("/tmp/tm-ime-chrome"))
    cdp = CDP()
    checks: list[dict] = []
    steps: list[dict] = []

    def check(name: str, condition: bool, detail: str = "") -> None:
        checks.append({"name": name, "ok": bool(condition), "detail": detail})
        print(f"  [{'OK' if condition else 'NG'}] {name}{(' — ' + detail) if detail else ''}")

    try:
        await cdp.connect(pick_page_target(args.cdp_port))
        await cdp.call("Page.enable")
        await cdp.call("Runtime.enable")
        await cdp.call("Log.enable")
        await cdp.call("Page.navigate", {"url": f"http://127.0.0.1:{args.http_port}/"})

        baseline = None
        for _ in range(120):
            await asyncio.sleep(0.5)
            current = await state(cdp)
            proof = current["proof"]
            if proof.get("tick") and int(proof["tick"]) >= 2:
                baseline = current
                break
        if baseline is None:
            raise RuntimeError("アプリが描画されませんでした")
        raw = baseline["raw"]
        print(f"  initial proof: {raw['proof']}")
        print(f"  activeElement: {raw['activeElement']} / textarea: {raw['textarea']}")

        await screenshot(cdp, out / f"{args.prefix}-01-initial.png")
        steps.append({"step": "initial", **baseline})
        check(
            "隠し textarea（IME ミラー）が存在する",
            raw.get("textarea") is not None,
            f"value={raw.get('textarea', {}).get('value')!r}",
        )
        check(
            "IME ミラーがフォーカスされている（IME 入力の受け口）",
            raw.get("textarea", {}).get("focused") is True,
            f"activeElement={raw.get('activeElement')}",
        )
        check("初期状態は変換中でない", baseline["proof"].get("ime_composing") == "0")
        baseline_rows = int(baseline["proof"].get("rows", "0"))
        check("初期状態のフィルタは空", baseline["proof"].get("filter") == "")

        # --- 1. ASCII 入力（既存機能の回帰確認） ---
        print("\n[1] ASCII 入力 'fire'")
        await type_ascii(cdp, "fire")
        await asyncio.sleep(0.4)
        current = await state(cdp)
        await screenshot(cdp, out / f"{args.prefix}-02-ascii-fire.png")
        steps.append({"step": "ascii-fire", **current})
        check("ASCII 入力が絞り込みへ反映される", current["proof"].get("filter") == "fire", current["proof"].get("filter", ""))
        check("ASCII 入力で一覧が絞り込まれる", int(current["proof"].get("rows", "0")) < baseline_rows)
        check("ASCII 入力は変換中ではない", current["proof"].get("ime_composing") == "0")

        # --- 2. Backspace（ブラウザ既定 → input イベント取り込み） ---
        print("\n[2] Backspace")
        await key(cdp, "Backspace", "Backspace", 8)
        await asyncio.sleep(0.4)
        current = await state(cdp)
        await screenshot(cdp, out / f"{args.prefix}-03-backspace.png")
        steps.append({"step": "backspace", **current})
        check("Backspace で 1 文字消える", current["proof"].get("filter") == "fir", current["proof"].get("filter", ""))

        # --- 3. Esc で解除 ---
        print("\n[3] Escape で解除")
        await key(cdp, "Escape", "Escape", 27)
        await asyncio.sleep(0.4)
        current = await state(cdp)
        await screenshot(cdp, out / f"{args.prefix}-04-escape-clear.png")
        steps.append({"step": "escape-clear", **current})
        check("Esc でフィルタが解除される", current["proof"].get("filter") == "")
        check("Esc で一覧が全件へ戻る", int(current["proof"].get("rows", "0")) == baseline_rows)

        # --- 4. IME composition: にほんご（変換中） ---
        print("\n[4] IME composition 'にほんご'（変換中）")
        await ime_compose(cdp, "にほんご", 0)
        current = await state(cdp)
        await screenshot(cdp, out / f"{args.prefix}-05-composing.png")
        crop = filter_crop(await element_rect(cdp, FILTER_SELECTOR))
        await screenshot(cdp, out / f"{args.prefix}-05-composing-filter.png", clip=crop)
        steps.append({"step": "composing", **current})
        check("変換中フラグが立つ", current["proof"].get("ime_composing") == "1")
        check("変換中文字列が入力欄へ表示される", current["proof"].get("ime_field") == "にほんご", current["proof"].get("ime_field", ""))
        check("変換中は marked range がある", current["proof"].get("ime_marked") == "0:4", current["proof"].get("ime_marked", ""))
        check("変換中はフィルタを更新しない", current["proof"].get("filter") == "")
        check("変換中は一覧を絞り込まない", int(current["proof"].get("rows", "0")) == baseline_rows)

        # --- 5. compositionupdate: 日本語（変換中） ---
        print("\n[5] compositionupdate '日本語'（変換中）")
        await ime_compose(cdp, "日本語", 4)
        current = await state(cdp)
        await screenshot(cdp, out / f"{args.prefix}-06-composing-update.png")
        steps.append({"step": "composing-update", **current})
        check("候補更新が入力欄へ反映される", current["proof"].get("ime_field") == "日本語", current["proof"].get("ime_field", ""))
        check("候補更新中もフィルタは更新しない", current["proof"].get("filter") == "")
        check("候補更新中も変換中フラグが立つ", current["proof"].get("ime_composing") == "1")

        # --- 6. compositionend: 確定 ---
        print("\n[6] compositionend '日本語'（確定）")
        await ime_commit(cdp, "日本語")
        current = await state(cdp)
        await screenshot(cdp, out / f"{args.prefix}-07-committed.png")
        crop = filter_crop(await element_rect(cdp, FILTER_SELECTOR))
        await screenshot(cdp, out / f"{args.prefix}-07-committed-filter.png", clip=crop)
        steps.append({"step": "committed", **current})
        check("確定文字列がフィルタへ反映される", current["proof"].get("filter") == "日本語", current["proof"].get("filter", ""))
        check("確定後は変換中フラグが下がる", current["proof"].get("ime_composing") == "0")
        check("確定後は marked range が消える", current["proof"].get("ime_marked") == "-", current["proof"].get("ime_marked", ""))
        check("確定文字列で一覧が絞り込まれる", int(current["proof"].get("rows", "0")) < baseline_rows)

        # --- 7. Esc で解除（確定後） ---
        print("\n[7] Escape で解除（確定後）")
        await key(cdp, "Escape", "Escape", 27)
        await asyncio.sleep(0.4)
        current = await state(cdp)
        await screenshot(cdp, out / f"{args.prefix}-08-escape-after-commit.png")
        steps.append({"step": "escape-after-commit", **current})
        check("確定後の Esc でフィルタが解除される", current["proof"].get("filter") == "")
        check("確定後の Esc で入力欄も空になる", current["proof"].get("ime_field") == "")
        check("確定後の Esc で一覧が全件へ戻る", int(current["proof"].get("rows", "0")) == baseline_rows)

        # --- 8. 変換中の Esc（変換破棄） ---
        print("\n[8] 変換中に Escape（変換破棄）")
        await ime_compose(cdp, "にほんご", 0)
        composing = await state(cdp)
        await key(cdp, "Escape", "Escape", 27)
        await asyncio.sleep(0.5)
        current = await state(cdp)
        await screenshot(cdp, out / f"{args.prefix}-09-composing-escape.png")
        steps.append({"step": "composing-escape", "before": composing, **current})
        check("変換中は変換中フラグが立つ", composing["proof"].get("ime_composing") == "1")
        check("変換中の Esc でフィルタが空のまま", current["proof"].get("filter") == "")
        check("変換中の Esc で入力欄が空になる", current["proof"].get("ime_field") == "")
        # 注: CDP の合成 IME は Escape で compositionend を発火しないため、
        # ブラウザ側の is_composing が残り、直後の ASCII 入力は飲み込まれる。
        # 実 IME は compositionend を発火する（未検証）。ここでは composition を
        # 明示的に終了させて状態が回復することを確認する。
        await ime_compose(cdp, "かんじ", 0)
        await ime_commit(cdp, "漢字")
        recovered = await state(cdp)
        steps.append({"step": "recovered", **recovered})
        check("変換破棄後も次の composition が反映される", recovered["proof"].get("filter") == "漢字", recovered["proof"].get("filter", ""))

        await key(cdp, "Escape", "Escape", 27)
        await asyncio.sleep(0.4)
        final = await state(cdp)
        await screenshot(cdp, out / f"{args.prefix}-10-final.png")
        steps.append({"step": "final", **final})
        check("最終状態でフィルタが解除されている", final["proof"].get("filter") == "")

        # --- コンソールエラー / 例外 ---
        exceptions = [event for event in cdp.events if event.get("method") == "Runtime.exceptionThrown"]
        errors = []
        for event in cdp.events:
            if event.get("method") == "Log.entryAdded":
                entry = event.get("params", {}).get("entry", {})
                if entry.get("level") == "error":
                    errors.append(entry.get("text", ""))
        check("Runtime 例外が無い", not exceptions, f"{len(exceptions)} 件")
        check("コンソールエラーが無い", not errors, f"{len(errors)} 件: {errors[:3]}")

        evidence = {
            "url": f"http://127.0.0.1:{args.http_port}/",
            "chrome": browser_version,
            "checks": checks,
            "steps": steps,
            "exceptions": exceptions,
            "console_errors": errors,
            "screenshots": sorted(path.name for path in out.glob(f"{args.prefix}-*.png")),
        }
        pathlib.Path(args.evidence).write_text(
            json.dumps(evidence, ensure_ascii=False, indent=2), encoding="utf-8"
        )
        failed = [check for check in checks if not check["ok"]]
        print(f"\nchecks: {len(checks) - len(failed)}/{len(checks)} OK")
        print(f"evidence: {args.evidence}")
        return 0 if not failed else 1
    finally:
        try:
            await cdp.ws.close()
        except Exception:
            pass
        for process in (chrome, http_server):
            try:
                process.send_signal(signal.SIGTERM)
                process.wait(timeout=10)
            except Exception:  # noqa: BLE001 - always clean up both children
                process.kill()


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))

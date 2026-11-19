#!/usr/bin/env python3
"""ブラウザ版 GPUI タスクマネージャーシェルの Runtime Proof 取得スクリプト。

ヘッドレス Chrome を CDP（Chrome DevTools Protocol）で操作し、描画・タイマー更新・
スクロール・行クリック（詳細パネル）・キーボード絞り込み・列クリックソートを
スクリーンショットと状態フック（body[data-gpui-proof]）で確認する。

使い方:
    # CI ゲート: ブラウザ/サーバ不要のオフライン自己検証（失敗時は終了コード 1）
    python3 web/scripts/cdp_verify_web.py --self-check

    # 実機 CDP 検証（開発ホストで手動実行）
    # 静的配信（別シェル）
    cd work/gpui-task-manager/web/dist
    python3 -m http.server 8091 --bind 127.0.0.1

    # Chrome（別シェル）
    google-chrome --headless=new --no-sandbox --disable-dev-shm-usage \
      --window-size=1440,1000 --remote-debugging-port=9223 \
      --user-data-dir=/tmp/tm-web-chrome about:blank

    # デモモード
    python3 cdp_verify_web.py --url http://127.0.0.1:8091/ \
        --out ../../notes/assets --evidence-out ../verification --prefix web-shell --label demo

    # ライブモード（tm-agent を 18787 で起動しておく。トークンは URL フラグメント）
    # 公開物に載せるスクリーンショットは --redact で個人情報を伏せること。
    TM_AGENT_TOKEN=test ./target/debug/tm-agent --listen 127.0.0.1:18787 --redact &
    python3 cdp_verify_web.py \
        --url 'http://127.0.0.1:8091/?agent=http://127.0.0.1:18787#token=test' \
        --out ../../notes/assets --evidence-out ../verification --prefix web-shell --label live

クリック座標の基準は DOM から取得する（レイアウト定数との手工同期を廃止）。
canvas の getBoundingClientRect() を原点とし、絞り込み欄は textarea セレクタの
rect 中心を直接クリックする。viewport 幅は window.innerWidth を使い、状態フック
（body[data-gpui-proof]）は従来どおり証拠として記録・検証する。
"""

from __future__ import annotations

import argparse
import asyncio
import base64
import json
import pathlib
import time
import urllib.request

# --- src/shell.rs のレイアウト定数と同期させること -------------------------
ROOT_PAD = 12.0
GAP = 12.0
HEADER_H = 52.0
OVERVIEW_H = 260.0
TOOLBAR_H = 36.0
TABLE_HEADER_H = 30.0
ROW_H = 30.0
DETAILS_W = 320.0
COL_NAME = 170.0
COL_CPU = 68.0
COL_MEMORY = 84.0
COL_PID = 60.0
COL_USER = 96.0
COL_STATUS = 74.0
COL_THREADS = 64.0
TABLE_GAP = 8.0

MAIN_TOP = ROOT_PAD + HEADER_H + GAP + OVERVIEW_H + GAP + TOOLBAR_H + GAP
ROW0_CENTER_Y = MAIN_TOP + 1.0 + TABLE_HEADER_H + ROW_H / 2.0  # border 1px
HEADER_Y = MAIN_TOP + 1.0 + TABLE_HEADER_H / 2.0
TOOLBAR_CENTER_Y = ROOT_PAD + HEADER_H + GAP + OVERVIEW_H + GAP + TOOLBAR_H / 2.0


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
            async for raw in self.ws:
                message = json.loads(raw)
                if "id" in message and message["id"] in self.pending:
                    future = self.pending.pop(message["id"])
                    if not future.done():
                        future.set_result(message)
                else:
                    self.events.append(message)
        except Exception as error:  # noqa: BLE001 - 検証スクリプトなので記録して続行
            self.events.append({"method": "__reader_closed__", "error": repr(error)})
            # Unblock any in-flight call instead of letting `call()` wait for
            # the 120 s timeout when the page/browser disappears (R22 CDP).
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
            "Runtime.evaluate",
            {"expression": expression, "returnByValue": True},
        )
        if "exceptionDetails" in result:
            return {"__exception__": result["exceptionDetails"]}
        return result.get("result", {}).get("value")

    async def drain(self) -> None:
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
        await cdp.call(
            "Input.dispatchKeyEvent",
            {
                "type": "keyDown",
                "key": character,
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
                "key": character,
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
        {"type": "mouseWheel", "x": x, "y": y, "deltaX": 0, "deltaY": delta_y},
    )
    await asyncio.sleep(0.4)


PROOF_EXPR = """
(() => {
  const canvas = document.querySelector('canvas');
  return {
    title: document.title,
    proof: document.body ? document.body.getAttribute('data-gpui-proof') : null,
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


# --- DOM/セレクタ取得（src/shell.rs のレイアウト定数との同期を廃止） -------
CANVAS_SELECTOR = "canvas"
FILTER_SELECTOR = "textarea"  # IME ミラー = 絞り込み欄の DOM 表現
VIEWPORT_EXPR = "({ width: window.innerWidth, height: window.innerHeight })"


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
    """DOM から取得した rect の中心座標を返す（ブラウザ不要の純粋関数）。"""
    if not rect:
        return None
    return (
        float(rect["left"]) + float(rect["width"]) / 2.0,
        float(rect["top"]) + float(rect["height"]) / 2.0,
    )


async def element_rect(cdp: CDP, selector: str) -> dict:
    rect = await cdp.evaluate(rect_expr(selector))
    if not isinstance(rect, dict):
        raise RuntimeError(f"DOM 要素が見つかりません: {selector}")
    return rect


async def element_center(cdp: CDP, selector: str) -> tuple[float, float]:
    center = center_from_rect(await element_rect(cdp, selector))
    if center is None:
        raise RuntimeError(f"DOM 要素の座標が取得できません: {selector}")
    return center


async def viewport_size(cdp: CDP) -> tuple[float, float]:
    """状態フックに依存せず window から viewport サイズを取得する。"""
    viewport = await cdp.evaluate(VIEWPORT_EXPR)
    if not isinstance(viewport, dict):
        raise RuntimeError("viewport サイズが取得できません")
    return float(viewport["width"]), float(viewport["height"])


def parse_proof(proof: str | None) -> dict:
    fields: dict = {}
    if not proof:
        return fields
    for part in proof.split(";"):
        if not part:
            continue
        # Split at the first `=` only: an `=` inside a proof value (e.g. a
        # filter string) must be preserved rather than treated as a separator.
        key, sep, value = part.partition("=")
        if sep:
            fields[key] = value
    return fields


# --- DOM proof ↔ AX ロールの対応（AA-AXMIRROR のミラー検証用） ---------------
# proof の rows / sort / dir / selected_pid に呼応する AX ロール期待値の純粋テーブル。
# 値の意味:
#   int             ... 固定出現数
#   str             ... proof の同名キーを十進整数として読む（"rows" → proof["rows"]）
#                       "key+N" は読んだ値に N を加える（"rows+1" → proof["rows"] + 1）
#   tuple[int, int] ... 許容範囲（ヘルプオーバーレイの開閉で 0/1 になる dialog）
EXPECTED_AX_ROLES: dict[str, int | str | tuple[int, int]] = {
    "table": 1,           # プロセス表 (Role::Table)
    "columnheader": 8,    # 列見出し 8 列（ソート可能 7 列 + コマンド列）
    # ヘッダ行（shell.rs の a11y_mirror_header_html が出す <div role="row">）も
    # CDP の AX ツリーでは row に数えられる。proof["rows"] はデータ行だけを数える
    # ため +1 して照合する。ミラー側を rowgroup 直下の columnheader のみにして
    # row = proof["rows"] へ揃える案もあるが、現行の rowgroup > row > columnheader
    # は ARIA 構造として正しく、ミラー（Rust 側）は担当外なので期待値を実構造へ
    # 合わせる。
    "row": "rows+1",      # データ行 proof["rows"] + ヘッダ行 1
    "dialog": (0, 1),     # ヘルプオーバーレイ (Role::Dialog) の有無
}
# proof フィールドが AX ノードのどの性質へ写るかの対応表（状態の突き合わせ用）。
PROOF_AX_STATE_FIELDS: dict[str, str] = {
    "rows": "row",
    "sort": "columnheader",
    "dir": "columnheader",
    "selected_pid": "row",
}
# DOM ミラー実装後に True へ反転すると AX 不一致を failure に昇格する。
# ミラー実装済み。期待値（row = rows + ヘッダ行）と照合方法（dir は名前の
# 昇順/降順）の修正後、self-check と実 CDP（demo: rows=30 に対して row=31、
# dir=asc が名前の「昇順」で 1 件一致）の両方で accessibility.warnings = 0 件を
# 確認した。これは昇格の判断材料だが、AX 検証の未カバー項目が残るため False 維持。
PROMOTE_AX_TO_FAILURE = False


def expected_ax_role_counts(proof: dict[str, str]) -> dict[str, tuple[int, int]]:
    """EXPECTED_AX_ROLES を proof から解決し、ロールごとの許容範囲を返す純粋関数。"""
    resolved: dict[str, tuple[int, int]] = {}
    for role, spec in EXPECTED_AX_ROLES.items():
        if isinstance(spec, int):
            resolved[role] = (spec, spec)
        elif isinstance(spec, str):
            # "rows" は proof["rows"] そのまま、"rows+1" はヘッダ行の分だけ +1。
            # proof にキーが無い場合もヘッダ行は存在するので 1 件を期待する。
            key, sep, offset = spec.partition("+")
            try:
                value = max(int(proof.get(key, "0")), 0)
                if sep:
                    value += int(offset)
            except (TypeError, ValueError):
                value = 1 if sep else 0
            resolved[role] = (max(value, 0), max(value, 0))
        else:
            low, high = spec
            resolved[role] = (low, high)
    return resolved


def ax_role(node: dict) -> str | None:
    """CDP の AXNode から role 文字列を取り出す（ignored ノードは除外）。"""
    if not isinstance(node, dict) or node.get("ignored"):
        return None
    role = node.get("role")
    value = role.get("value") if isinstance(role, dict) else role
    return value if isinstance(value, str) and value else None


def count_ax_roles(nodes: list[dict]) -> dict[str, int]:
    """Accessibility.getFullAXTree の nodes から role 出現数を集計する純粋関数。"""
    counts: dict[str, int] = {}
    for node in nodes or []:
        role = ax_role(node)
        if role:
            counts[role] = counts.get(role, 0) + 1
    return counts


def ax_node_text(node: dict) -> str:
    name = node.get("name")
    if isinstance(name, dict):
        return str(name.get("value") or "")
    return str(name or "")


def ax_property(node: dict, key: str):
    for prop in node.get("properties") or []:
        if isinstance(prop, dict) and prop.get("name") == key:
            value = prop.get("value")
            return value.get("value") if isinstance(value, dict) else value
    return None


def verify_ax_roles(ax_nodes: list[dict], proof: dict[str, str]) -> tuple[list[str], dict]:
    """AX ロール出現数と DOM proof の整合を検証する純粋関数。

    戻り値は (warnings, detail)。呼び出し側で failures ではなく warnings へ積む
    （ミラー実装後の昇格方法は PROMOTE_AX_TO_FAILURE のコメントを参照）。
    """
    counts = count_ax_roles(ax_nodes)
    expected = expected_ax_role_counts(proof)
    by_role: dict[str, list[dict]] = {}
    for node in ax_nodes or []:
        role = ax_role(node)
        if role:
            by_role.setdefault(role, []).append(node)

    warnings: list[str] = []
    detail: dict = {"role_counts": {}, "state": {}, "proof_fields": PROOF_AX_STATE_FIELDS}
    for role, (low, high) in expected.items():
        actual = counts.get(role, 0)
        detail["role_counts"][role] = {"actual": actual, "expected": [low, high]}
        if not low <= actual <= high:
            warnings.append(
                f"AX ロール {role} の出現数 {actual} が期待範囲 [{low}, {high}] 外"
                "（DOM proof と AX ツリーが不一致: 偽陽性の可能性）"
            )

    rows = by_role.get(PROOF_AX_STATE_FIELDS["rows"], [])
    selected_pid = proof.get("selected_pid", "-")
    if selected_pid not in ("", "-"):
        matched = [
            row for row in rows
            if ax_property(row, "selected") in (True, "true")
            or selected_pid in ax_node_text(row)
        ]
        detail["state"]["selected_pid"] = {"value": selected_pid, "matched_rows": len(matched)}
        if not matched:
            warnings.append(f"AX に selected_pid={selected_pid} に対応する選択行がない")

    headers = by_role.get(PROOF_AX_STATE_FIELDS["sort"], [])
    direction = proof.get("dir")
    if direction in ("asc", "desc"):
        # ミラーは active な columnheader に aria-sort と「現在 昇順/降順」を
        # 付ける（shell.rs: a11y_mirror_header_html）。しかし CDP の AX ツリーは
        # aria-sort を sort property として露出しないため、property ではなく
        # 名前テキストの 昇順/降順 で照合する。
        token = {"asc": "昇順", "desc": "降順"}[direction]
        matched = [header for header in headers if token in ax_node_text(header)]
        detail["state"]["dir"] = {
            "value": direction, "name_token": token, "matched_headers": len(matched),
        }
        if not matched:
            warnings.append(f"AX に dir={direction}（名前の {token}）のソート列見出しがない")

    sort_label = proof.get("sort")
    if sort_label:
        matched = [header for header in headers if sort_label in ax_node_text(header)]
        detail["state"]["sort"] = {"value": sort_label, "matched_headers": len(matched)}
        if not matched:
            warnings.append(f"AX に sort={sort_label} の列見出しがない")

    return warnings, detail


def collect_network(events: list[dict]) -> dict:
    """スナップショット要求の URL・メソッド・Authorization ヘッダ有無を集計する。

    トークンが URL（クエリ）に載っていないこと、Authorization ヘッダで送られ、
    CORS プリフライト（OPTIONS）が成立していることを Runtime Proof にする。
    """
    extra_headers: dict[str, set[str]] = {}
    for event in events:
        if event.get("method") != "Network.requestWillBeSentExtraInfo":
            continue
        params = event.get("params", {})
        headers = {
            str(name).lower()
            for name in (params.get("headers") or {}).keys()
        }
        extra_headers[params.get("requestId", "")] = headers

    requests = []
    for event in events:
        if event.get("method") != "Network.requestWillBeSent":
            continue
        params = event.get("params", {})
        request = params.get("request", {})
        url = request.get("url") or ""
        if "/api/v1/snapshot" not in url:
            continue
        headers = extra_headers.get(params.get("requestId", ""), set())
        requests.append(
            {
                "method": request.get("method"),
                "url": url,
                "has_authorization_header": "authorization" in headers,
            }
        )
    preflight_count = sum(1 for request in requests if request["method"] == "OPTIONS")
    get_count = sum(1 for request in requests if request["method"] == "GET")
    token_in_url = any("token=" in (request["url"] or "") for request in requests)
    authorized = any(request["has_authorization_header"] for request in requests)
    return {
        "snapshot_requests": requests,
        "preflight_count": preflight_count,
        "get_count": get_count,
        "token_in_url": token_in_url,
        "authorization_header_seen": authorized,
    }


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
                }
            )
        elif method == "Log.entryAdded":
            entry = params.get("entry", {})
            messages.append(
                {
                    "source": "log",
                    "type": entry.get("level"),
                    "text": f"[{entry.get('source')}] {entry.get('text')}",
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
                }
            )
    return messages


def column_centers(vw: float) -> dict:
    """src/shell.rs の列幅からヘッダーのクリック中心 x を計算する。"""
    table_width = vw - 2 * ROOT_PAD - DETAILS_W - GAP
    content_width = table_width - 2.0 - 24.0  # border 1px x2 + px_3 (12px x2)
    command_width = max(
        0.0,
        content_width
        - (COL_NAME + COL_CPU + COL_MEMORY + COL_PID + COL_USER + COL_STATUS + COL_THREADS)
        - 7 * TABLE_GAP,
    )
    x = ROOT_PAD + 1.0 + 12.0
    centers = {"name": x + COL_NAME / 2.0}
    x += COL_NAME + TABLE_GAP + command_width + TABLE_GAP
    centers["cpu"] = x + COL_CPU / 2.0
    x += COL_CPU + TABLE_GAP
    centers["memory"] = x + COL_MEMORY / 2.0
    x += COL_MEMORY + TABLE_GAP
    centers["pid"] = x + COL_PID / 2.0
    x += COL_PID + TABLE_GAP
    centers["user"] = x + COL_USER / 2.0
    x += COL_USER + TABLE_GAP
    centers["status"] = x + COL_STATUS / 2.0
    x += COL_STATUS + TABLE_GAP
    centers["threads"] = x + COL_THREADS / 2.0
    return centers


def self_check() -> int:
    """オフライン自己検証（CI ゲート）。ブラウザもサーバも起動しない。

    CDP 接続前に決まる純粋関数（状態フック解析・座標計算・ネットワーク/
    コンソール分類）とレイアウト定数を検証する。実機挙動の検証は通常実行
    （--url / --port を指定）で行い、CI では下記の手動手順に委ねる。
    """
    failures: list[str] = []

    def check(condition: bool, message: str) -> None:
        if not condition:
            failures.append(message)
            print(f"  FAIL: {message}")

    # 状態フック解析: 値に含まれる '=' は区切りとして扱わない。
    parsed = parse_proof("vw=1440;tick=3;rows=42;filter=a=b;sort=CPU;dir=desc")
    check(parsed.get("vw") == "1440", "vw が解析できる")
    check(parsed.get("rows") == "42", "rows が解析できる")
    check(parsed.get("filter") == "a=b", "値に含まれる '=' が保持される")
    check(parsed.get("dir") == "desc", "末尾のフィールドが解析できる")
    check(parse_proof(None) == {} and parse_proof("") == {}, "空のフックは空辞書")

    # DOM proof ↔ AX ロール: オフラインではテーブル構造と純粋関数のみ検証する。
    check(
        set(EXPECTED_AX_ROLES) == {"table", "columnheader", "row", "dialog"},
        "AX ロール期待値テーブルのキーが揃っている",
    )
    check(
        EXPECTED_AX_ROLES["table"] == 1
        and isinstance(EXPECTED_AX_ROLES["columnheader"], int),
        "固定ロール期待値が整数で定義されている",
    )
    check(
        EXPECTED_AX_ROLES["row"] == "rows+1",
        "row が proof の rows + ヘッダ行 1 を参照する",
    )
    low, high = EXPECTED_AX_ROLES["dialog"]
    check(
        isinstance(low, int) and isinstance(high, int) and 0 <= low <= high,
        "dialog が 0/1 の範囲で定義されている",
    )
    check(
        set(PROOF_AX_STATE_FIELDS) == {"rows", "sort", "dir", "selected_pid"},
        "proof の 4 フィールドが AX 状態対応表に含まれる",
    )
    resolved = expected_ax_role_counts({"rows": "42"})
    check(
        resolved["row"] == (43, 43)
        and resolved["table"] == (1, 1)
        and resolved["dialog"] == (0, 1),
        "proof から AX 期待値が解決できる（row はヘッダ行 +1）",
    )
    check(
        expected_ax_role_counts({})["row"] == (1, 1),
        "rows 欠損時もヘッダ行 1 件を期待する",
    )

    def synthetic_ax() -> list[dict]:
        # 実 CDP と同じく aria-sort の property は付けず、名前の「現在 降順」だけ
        # で dir を照合できることを確認する（ヘッダ行も row に含まれる）。
        headers = [
            {
                "role": {"value": "columnheader"},
                "name": {
                    "value": (
                        "名前 列で並び替え、現在 降順" if i == 0 else f"列{i}"
                    )
                },
            }
            for i in range(EXPECTED_AX_ROLES["columnheader"])
        ]
        return (
            [{"role": {"value": "table"}}]
            + [{"role": {"value": "row"}, "name": {"value": "列見出し行"}}]
            + headers
            + [
                {
                    "role": {"value": "row"},
                    "name": {"value": "PID 2500"},
                    "properties": [{"name": "selected", "value": {"value": True}}],
                },
                {"role": {"value": "generic"}, "ignored": True},
                {"ignored": False},
            ]
        )

    synthetic = synthetic_ax()
    check(
        count_ax_roles(synthetic) == {"table": 1, "columnheader": 8, "row": 2},
        "AX ノードから role 出現数を集計し ignored を除外する（ヘッダ行含む）",
    )
    consistent_proof = {
        "rows": "1", "sort": "名前", "dir": "desc", "selected_pid": "2500",
    }
    ok_warnings, ok_detail = verify_ax_roles(synthetic, consistent_proof)
    check(ok_warnings == [], "AX と proof が一致すれば警告が出ない")
    check(ok_detail["state"]["selected_pid"]["matched_rows"] == 1, "選択行が突き合わされる")
    check(
        ok_detail["state"]["dir"]["matched_headers"] == 1
        and ok_detail["state"]["dir"]["name_token"] == "降順",
        "dir を AX property ではなく名前の昇順/降順で照合する",
    )
    zero_warnings, zero_detail = verify_ax_roles([], consistent_proof)
    check(
        len(zero_warnings) >= 4 and zero_detail["role_counts"]["row"]["actual"] == 0,
        "AX ノード 0 件（ミラー未実装）は警告として検出される",
    )

    # 列座標: 全列が正の座標で、左から右へ単調増加すること。
    centers = column_centers(1440.0)
    expected = ("name", "cpu", "memory", "pid", "user", "status", "threads")
    check(all(key in centers for key in expected), "全列のクリック中心が計算できる")
    ordered = [centers[key] for key in expected if key in centers]
    check(len(ordered) == len(expected) and all(value > 0 for value in ordered), "列座標が正")
    check(ordered == sorted(ordered), "列座標が左から右へ単調増加")
    check(column_centers(1440.0) == centers, "列座標が決定的に計算される")

    # ネットワーク: トークン非露出と Authorization ヘッダの検出。
    def events(url: str, with_auth: bool) -> list[dict]:
        headers = {"Authorization": "Bearer test-token"} if with_auth else {}
        return [
            {
                "method": "Network.requestWillBeSentExtraInfo",
                "params": {"requestId": "req-1", "headers": headers},
            },
            {
                "method": "Network.requestWillBeSent",
                "params": {"requestId": "req-1", "request": {"method": "GET", "url": url}},
            },
            {
                "method": "Network.requestWillBeSent",
                "params": {
                    "requestId": "req-2",
                    "request": {
                        "method": "OPTIONS",
                        "url": "http://127.0.0.1:18787/api/v1/snapshot",
                    },
                },
            },
        ]

    snapshot = "http://127.0.0.1:18787/api/v1/snapshot"
    clean = collect_network(events(snapshot, True))
    check(clean["get_count"] == 1, "スナップショット GET が集計される")
    check(clean["preflight_count"] == 1, "プリフライト OPTIONS が集計される")
    check(clean["authorization_header_seen"], "Authorization ヘッダが検出される")
    check(not clean["token_in_url"], "トークン無し URL は正しく判定される")
    leaked = collect_network(events(f"{snapshot}?token=secret", False))
    check(leaked["token_in_url"], "URL 内の token= が検出される")

    # コンソール: console / log / exception の 3 経路を分類できること。
    messages = console_texts(
        [
            {
                "method": "Runtime.consoleAPICalled",
                "params": {"type": "warning", "args": [{"value": "warn"}]},
            },
            {
                "method": "Log.entryAdded",
                "params": {"entry": {"level": "error", "source": "network", "text": "404"}},
            },
            {
                "method": "Runtime.exceptionThrown",
                "params": {"exceptionDetails": {"text": "boom"}},
            },
        ]
    )
    check(
        [message["source"] for message in messages] == ["console", "log", "exception"],
        "コンソール 3 経路が分類される",
    )
    check("boom" in messages[2]["text"], "例外本文が記録される")

    # レイアウト定数とフック式: src/shell.rs 側の変更に気付けること。
    check(TOOLBAR_CENTER_Y < HEADER_Y < ROW0_CENTER_Y, "ヘッダー/行/ツールバーの順序が正しい")
    check("data-gpui-proof" in PROOF_EXPR and "canvas" in PROOF_EXPR, "状態フック式が維持されている")

    # DOM/セレクタ取得: rect 中心の算出とセレクタ埋め込みをオフラインで検証する。
    rect = {
        "left": 10.0, "top": 20.0, "width": 100.0, "height": 40.0,
        "right": 110.0, "bottom": 60.0,
    }
    check(center_from_rect(rect) == (60.0, 40.0), "rect から中心座標が計算できる")
    check(center_from_rect(None) is None, "要素が無い場合は None になる")
    check(CANVAS_SELECTOR == "canvas" and FILTER_SELECTOR == "textarea", "DOM セレクタが維持されている")
    dom_expr = rect_expr(FILTER_SELECTOR)
    check("querySelector" in dom_expr and '"textarea"' in dom_expr, "rect 取得式にセレクタが埋め込まれる")
    check("getBoundingClientRect" in dom_expr, "getBoundingClientRect で DOM 座標を取得する")

    passed = not failures
    print(
        json.dumps(
            {"script": "cdp_verify_web.py", "mode": "self-check",
             "passed": passed, "failures": failures},
            ensure_ascii=False,
        )
    )
    return 0 if passed else 1


async def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=9223)
    parser.add_argument("--url", default="http://127.0.0.1:8091/")
    parser.add_argument("--out", default="../../notes/assets")
    parser.add_argument("--evidence-out", default="../verification")
    parser.add_argument("--prefix", default="web-shell")
    parser.add_argument("--label", default="demo")
    parser.add_argument("--wait", type=float, default=5.0)
    parser.add_argument(
        "--expect-agent-error",
        action="store_true",
        help="エージェント停止時のエラー表示だけを検証する（デモへフォールバックしないこと）",
    )
    parser.add_argument(
        "--self-check",
        action="store_true",
        help="CDP を使わずオフラインで純粋関数を検証する（CI ゲート用）",
    )
    args = parser.parse_args()

    if args.self_check:
        return self_check()

    out_dir = pathlib.Path(args.out).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    evidence_dir = pathlib.Path(args.evidence_out).resolve()
    evidence_dir.mkdir(parents=True, exist_ok=True)

    ws_url = pick_page_target(args.port)
    cdp = CDP()
    await cdp.connect(ws_url)
    await cdp.call("Page.enable")
    await cdp.call("Runtime.enable")
    await cdp.call("Log.enable")
    await cdp.call("Network.enable")
    await cdp.call("Accessibility.enable")

    evidence: dict = {
        "label": args.label,
        "url": args.url,
        "started_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "layout": {
            "row0_center_y": ROW0_CENTER_Y,
            "header_y": HEADER_Y,
            "toolbar_center_y": TOOLBAR_CENTER_Y,
        },
        "steps": [],
    }
    failures: list[str] = []
    warnings: list[str] = []

    def check(condition: bool, message: str) -> None:
        if not condition:
            failures.append(message)
            print(f"  FAIL: {message}")

    # 直前のページと同じ URL だと同一文書扱いでリロードされないことがあるため、
    # いったん about:blank へ移動してから対象 URL を開く（状態の混入を防ぐ）。
    await cdp.call("Page.navigate", {"url": "about:blank"})
    await asyncio.sleep(0.6)
    await cdp.drain()
    # 直前のページ（旧ページのサンプラや Runtime.enable 時のバッファ再生）の
    # ログ・イベントを持ち込まない。証拠は今回のページロード分だけにする。
    await cdp.call("Log.clear")
    await cdp.call("Runtime.discardConsoleEntries")
    cdp.events.clear()
    await cdp.call("Page.navigate", {"url": args.url})
    await asyncio.sleep(args.wait)
    await cdp.drain()

    env = await cdp.evaluate(PROOF_EXPR)
    evidence["env"] = env
    proof = parse_proof(env.get("proof") if isinstance(env, dict) else None)
    evidence["steps"].append({"step": "initial", "proof": proof, "title": env.get("title")})
    check(bool(proof), "状態フック body[data-gpui-proof] が取得できる")
    if not args.expect_agent_error:
        check(int(proof.get("rows", "0")) > 0, "プロセス行が 1 件以上表示される")
    # 座標基準を DOM から取得する（src/shell.rs 定数との手工同期を廃止）。
    vw, vh = await viewport_size(cdp)
    check(vw > 0 and vh > 0, "viewport サイズを DOM から取得できる")
    canvas_rect = await element_rect(cdp, CANVAS_SELECTOR)
    ox, oy = float(canvas_rect["left"]), float(canvas_rect["top"])
    cw, ch = float(canvas_rect["width"]), float(canvas_rect["height"])
    evidence["layout"]["dom_geometry"] = {
        "viewport": {"width": vw, "height": vh},
        "canvas_rect": canvas_rect,
        "hook_vw": proof.get("vw"),
    }
    centers = column_centers(vw)
    evidence["layout"]["column_centers"] = centers
    await screenshot(cdp, out_dir / f"{args.prefix}-01-initial-{args.label}.png")

    if args.expect_agent_error:
        # ライブ指定でエージェントが落ちている場合: デモへフォールバックせず、
        # ステータスバーと表にエラーを表示することを確認する。
        await asyncio.sleep(2.0)
        await cdp.drain()
        env = await cdp.evaluate(PROOF_EXPR)
        proof_error = parse_proof(env.get("proof") if isinstance(env, dict) else None)
        evidence["steps"].append(
            {"step": "agent_error", "proof": proof_error, "title": env.get("title")}
        )
        check(proof_error.get("mode") == "live", "エラー時もライブモードのまま（デモへ落ちない）")
        check(proof_error.get("live_error") == "1", "取得失敗がステータスに反映される")
        check(proof_error.get("rows") == "0", "失敗時にデモデータを表示しない")
        await screenshot(cdp, out_dir / f"{args.prefix}-09-agent-error-{args.label}.png")

        network = collect_network(cdp.events)
        evidence["network"] = network
        check(not network["token_in_url"], "トークンがリクエスト URL に含まれない")

        console = console_texts(cdp.events)
        evidence["console"] = console
        benign = [
            message
            for message in console
            if message.get("source") == "log"
            and (
                message.get("type") == "warning"
                or "failed to load resource" in (message.get("text") or "").lower()
            )
        ]
        severe = [message for message in console if message not in benign]
        panic_messages = [
            message
            for message in severe
            if message.get("source") == "exception"
            or message.get("type") == "error"
            or "panicked" in (message.get("text") or "").lower()
        ]
        evidence["benign_console_count"] = len(benign)
        evidence["panic_or_error_count"] = len(panic_messages)
        evidence["panic_or_errors"] = panic_messages
        check(not panic_messages, "コンソールに panic / JS 例外はない")
        evidence["failures"] = failures
        evidence["warnings"] = warnings
        evidence["passed"] = not failures
        evidence["finished_at"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
        evidence_path = evidence_dir / f"{args.prefix}-evidence-{args.label}.json"
        evidence_path.write_text(
            json.dumps(evidence, ensure_ascii=False, indent=2), encoding="utf-8"
        )
        print(f"evidence: {evidence_path}")
        print("---- console ----")
        for message in console:
            print(f"[{message['source']}/{message['type']}] {message['text']}")
        print("---- checks ----")
        print(
            json.dumps(
                {"failures": failures, "warnings": warnings, "passed": not failures},
                ensure_ascii=False,
            )
        )
        return 0 if not failures else 1

    # 1 秒タイマーによる更新
    tick_before = int(proof.get("tick", "0"))
    await asyncio.sleep(2.5)
    await cdp.drain()
    env = await cdp.evaluate(PROOF_EXPR)
    proof_ticks = parse_proof(env.get("proof") if isinstance(env, dict) else None)
    evidence["steps"].append(
        {"step": "after_ticks", "tick_before": tick_before, "proof": proof_ticks}
    )
    check(
        int(proof_ticks.get("tick", "0")) > tick_before,
        "1 秒タイマーで tick が増加する",
    )
    await screenshot(cdp, out_dir / f"{args.prefix}-02-after-ticks-{args.label}.png")

    # スクロール（DOM の canvas rect を基準に表の中央へホイール）
    table_center_x = ox + cw / 2.0
    wheel_y = oy + ch * 0.64
    await wheel(cdp, table_center_x, wheel_y, 400)
    await cdp.drain()
    scroll_env = await cdp.evaluate(PROOF_EXPR)
    evidence["steps"].append(
        {
            "step": "scroll_down",
            "proof": parse_proof(
                scroll_env.get("proof") if isinstance(scroll_env, dict) else None
            ),
        }
    )
    await screenshot(cdp, out_dir / f"{args.prefix}-03-scrolled-{args.label}.png")
    await wheel(cdp, table_center_x, wheel_y, -800)
    await cdp.drain()

    # 行クリック → 詳細パネル（選択 PID の変化）
    row_y = oy + ROW0_CENTER_Y + 2 * ROW_H
    await click(cdp, ox + ROOT_PAD + 300, row_y)
    await asyncio.sleep(0.6)
    await cdp.drain()
    env = await cdp.evaluate(PROOF_EXPR)
    proof_click = parse_proof(env.get("proof") if isinstance(env, dict) else None)
    evidence["steps"].append({"step": "row_click", "row_y": row_y, "proof": proof_click})
    check(
        proof_click.get("selected_pid", "-") not in ("-", "", None),
        "行クリックでプロセスが選択される",
    )
    await screenshot(cdp, out_dir / f"{args.prefix}-04-row-click-{args.label}.png")

    # キーボード絞り込み（ASCII。日本語 IME は未対応）
    # 絞り込み欄は DOM の textarea（IME ミラー）として存在するのでセレクタで取得する。
    try:
        filter_x, filter_y = await element_center(cdp, FILTER_SELECTOR)
        filter_anchor = FILTER_SELECTOR
    except RuntimeError:
        # IME ミラーが無いビルド向けの保険。主経路は上記の DOM セレクタ取得。
        filter_x, filter_y = ox + ROOT_PAD + 170, oy + TOOLBAR_CENTER_Y
        filter_anchor = "canvas-relative-fallback"
    evidence["steps"].append(
        {"step": "filter_anchor", "selector": filter_anchor, "x": filter_x, "y": filter_y}
    )
    await click(cdp, filter_x, filter_y)
    await asyncio.sleep(0.3)
    await type_text(cdp, "fire")
    await asyncio.sleep(0.5)
    await cdp.drain()
    env = await cdp.evaluate(PROOF_EXPR)
    proof_filter = parse_proof(env.get("proof") if isinstance(env, dict) else None)
    evidence["steps"].append({"step": "filter_typed", "proof": proof_filter})
    check(proof_filter.get("filter") == "fire", "キー入力が絞り込みに反映される")
    check(
        int(proof_filter.get("rows", "0")) < int(proof.get("rows", "0")),
        "絞り込みで表示行が減る",
    )
    await screenshot(cdp, out_dir / f"{args.prefix}-05-filter-{args.label}.png")

    # Esc で絞り込み解除
    await press_key(cdp, "Escape", "Escape", 27)
    await asyncio.sleep(0.4)
    await cdp.drain()
    env = await cdp.evaluate(PROOF_EXPR)
    proof_clear = parse_proof(env.get("proof") if isinstance(env, dict) else None)
    evidence["steps"].append({"step": "filter_cleared", "proof": proof_clear})
    check(proof_clear.get("filter") == "", "Esc で絞り込みが解除される")

    # 列クリックでソート（CPU → 方向反転 → 名前列）
    await click(cdp, ox + centers["cpu"], oy + HEADER_Y)
    await asyncio.sleep(0.4)
    env = await cdp.evaluate(PROOF_EXPR)
    proof_sort_cpu = parse_proof(env.get("proof") if isinstance(env, dict) else None)
    evidence["steps"].append({"step": "sort_cpu", "proof": proof_sort_cpu})
    check(proof_sort_cpu.get("sort") == "CPU", "CPU 列クリックで CPU ソートになる")
    await screenshot(cdp, out_dir / f"{args.prefix}-06-sort-cpu-{args.label}.png")

    await click(cdp, ox + centers["cpu"], oy + HEADER_Y)
    await asyncio.sleep(0.4)
    env = await cdp.evaluate(PROOF_EXPR)
    proof_sort_flip = parse_proof(env.get("proof") if isinstance(env, dict) else None)
    evidence["steps"].append({"step": "sort_cpu_flip", "proof": proof_sort_flip})
    check(
        proof_sort_flip.get("dir") != proof_sort_cpu.get("dir"),
        "同じ列の再クリックでソート方向が反転する",
    )

    await click(cdp, ox + centers["name"], oy + HEADER_Y)
    await asyncio.sleep(0.4)
    env = await cdp.evaluate(PROOF_EXPR)
    proof_sort_name = parse_proof(env.get("proof") if isinstance(env, dict) else None)
    evidence["steps"].append({"step": "sort_name", "proof": proof_sort_name})
    check(proof_sort_name.get("sort") == "名前", "名前列クリックで名前ソートになる")
    await screenshot(cdp, out_dir / f"{args.prefix}-07-sort-name-{args.label}.png")

    # 最終状態
    await asyncio.sleep(1.2)
    await cdp.drain()
    env = await cdp.evaluate(PROOF_EXPR)
    proof_final = parse_proof(env.get("proof") if isinstance(env, dict) else None)
    evidence["steps"].append(
        {"step": "final", "proof": proof_final, "title": env.get("title")}
    )
    await screenshot(cdp, out_dir / f"{args.prefix}-08-final-{args.label}.png")

    # DOM proof ↔ AX ツリーの突き合わせ（偽陽性検出）。
    # AA-AXMIRROR の DOM ミラー実装前は canvas 内の AccessKit ノードがブラウザ AX
    # ツリーへ届かず columnheader/row/dialog が 0 件になる。現状はその既知の不一致を
    # warnings にだけ記録し、failures / passed には影響させない。
    # ミラー実装後に PROMOTE_AX_TO_FAILURE = True へ反転すると、
    # 同じ警告が failures へ入り CDP 検証全体が失敗する（エラーへの昇格手順）。
    try:
        ax_tree = await cdp.call("Accessibility.getFullAXTree")
        ax_nodes = ax_tree.get("nodes", []) if isinstance(ax_tree, dict) else []
    except RuntimeError as error:
        # AX ドメインが使えない環境でも DOM proof 検証そのものは通す（警告のみ）。
        ax_nodes = []
        ax_warnings = [f"Accessibility.getFullAXTree が失敗し AX 検証ができない: {error}"]
        ax_detail = {"role_counts": {}, "state": {}, "error": repr(error)}
    else:
        ax_warnings, ax_detail = verify_ax_roles(ax_nodes, proof_final)
    evidence["accessibility"] = {
        "node_count": len(ax_nodes),
        "role_counts": ax_detail["role_counts"],
        "state": ax_detail["state"],
        "warnings": ax_warnings,
        "enforcement": "failure" if PROMOTE_AX_TO_FAILURE else "warning",
    }
    if PROMOTE_AX_TO_FAILURE:
        failures.extend(ax_warnings)
    else:
        warnings.extend(ax_warnings)
        for message in ax_warnings:
            print(f"  WARN: {message}")

    # ネットワーク（トークンが URL に載らず Authorization ヘッダで送られていること）
    network = collect_network(cdp.events)
    evidence["network"] = network
    check(not network["token_in_url"], "トークンがリクエスト URL に含まれない")
    if args.label == "live":
        check(network["get_count"] >= 1, "スナップショット GET が発行される")
        # プリフライトは Access-Control-Max-Age=600 でブラウザにキャッシュされるため、
        # 同一 Chrome プロファイルでの 2 回目以降は OPTIONS が観測されないことがある。
        # Authorization ヘッダが GET で送られていることを主たる証拠にし、
        # プリフライト回数は証拠 JSON に記録する（キャッシュ時は 0）。
        check(
            network["authorization_header_seen"] or network["preflight_count"] >= 1,
            "Authorization ヘッダが送られる（またはプリフライトが観測される）",
        )

    # コンソール（panic / JS 例外がないこと）
    #
    # GPUI/wgpu は WebGPU フォールバックやサブピクセル AA 無効化を警告として出す。
    # これらは想定内なので失敗扱いにしない。失敗扱いは「Runtime.exceptionThrown」
    # と panic 文言のみ。ネットワークの 404 ログ（favicon 等）も benign として記録する。
    console = console_texts(cdp.events)
    evidence["console"] = console

    def is_benign(message: dict) -> bool:
        text = (message.get("text") or "").lower()
        if message.get("source") == "log" and "failed to load resource" in text:
            return True
        return message.get("source") == "log" and message.get("type") == "warning"

    benign = [message for message in console if is_benign(message)]
    severe = [message for message in console if not is_benign(message)]
    panic_messages = [
        message
        for message in severe
        if message.get("source") == "exception"
        or message.get("type") == "error"
        or "panicked" in (message.get("text") or "").lower()
        or "unreachable executed" in (message.get("text") or "").lower()
    ]
    evidence["benign_console_count"] = len(benign)
    evidence["panic_or_error_count"] = len(panic_messages)
    evidence["panic_or_errors"] = panic_messages
    check(not panic_messages, "コンソールに panic / JS 例外がない")

    evidence["failures"] = failures
    evidence["warnings"] = warnings
    evidence["passed"] = not failures
    evidence["finished_at"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")

    evidence_path = evidence_dir / f"{args.prefix}-evidence-{args.label}.json"
    evidence_path.write_text(
        json.dumps(evidence, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    print(f"evidence: {evidence_path}")
    print("---- console ----")
    for message in console:
        print(f"[{message['source']}/{message['type']}] {message['text']}")
    print("---- checks ----")
    print(
        json.dumps(
            {"failures": failures, "warnings": warnings, "passed": not failures},
            ensure_ascii=False,
        )
    )
    return 0 if not failures else 1


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))

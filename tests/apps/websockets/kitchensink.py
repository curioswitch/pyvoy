from __future__ import annotations

import os
import sys
from time import perf_counter_ns
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from asgiref.typing import ASGIReceiveCallable, ASGISendCallable, WebSocketScope

ACCEPT = {"type": "websocket.accept", "headers": (), "subprotocol": None}


async def app(
    scope: WebSocketScope, receive: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    path = scope["path"]

    msg = await receive()
    if msg["type"] != "websocket.connect":
        raise AssertionError(msg["type"])

    if path == "/reject":
        # Close before accepting -> server rejects the upgrade (HTTP 403).
        await send({"type": "websocket.close", "code": 1008})
        return
    if path == "/raise-before":
        msg = "boom before accept"
        raise RuntimeError(msg)
    if path == "/return-before-accept":
        # Return after connect without accepting or closing -> the server drops
        # the connection mid-handshake.
        return

    await send(ACCEPT)

    if path == "/raise-after":
        msg = "boom after accept"
        raise RuntimeError(msg)
    if path == "/close":
        await send({"type": "websocket.close", "code": 1001, "reason": "bye"})
        return
    if path == "/bad-send":
        # Neither text nor bytes -> server raises.
        await send({"type": "websocket.send"})
        return
    if path == "/double-accept":
        await send(ACCEPT)
        return
    if path == "/close-nocode":
        # No code, explicit None reason -> server defaults to 1000 / empty reason.
        await send({"type": "websocket.close", "reason": None})
        return
    if path == "/return":
        # Accept then return without closing -> server drops the connection.
        return
    if path == "/backpressure":
        # Send incompressible 1 MiB messages, timing each send. When the client
        # stops reading, the downstream write buffer fills and the watermark
        # callbacks gate process_send_events, so at least one `await send` blocks
        # until the client drains. The per-send durations are reported in a final
        # text message so the test can assert at least one long pause occurred.
        waits: list[int] = []
        for _ in range(100):
            chunk = os.urandom(1024 * 1024)
            start = perf_counter_ns()
            await send({"type": "websocket.send", "bytes": chunk})
            waits.append(perf_counter_ns() - start)
        await send({"type": "websocket.send", "text": ",".join(str(w) for w in waits)})
        return
    if path == "/close-then-send":
        # Sending after close raises ClientDisconnectedError
        await send({"type": "websocket.close", "code": 1000})
        await send({"type": "websocket.send", "text": "too late"})
        return
    if path == "/bad-event":
        await send({"type": "websocket.frobnicate"})
        return
    if path == "/recv-after-disconnect":
        while True:
            m = await receive()
            if m["type"] == "websocket.disconnect":
                second = await receive()
                print(  # noqa: T201
                    f"recv-after-disconnect: second={second['type']}",
                    file=sys.stderr,
                    flush=True,
                )
                return

    # Default: echo until the client disconnects.
    while True:
        m = await receive()
        if m["type"] == "websocket.receive":
            await send(
                {
                    "type": "websocket.send",
                    "text": m.get("text"),
                    "bytes": m.get("bytes"),
                }
            )
        elif m["type"] == "websocket.disconnect":
            return

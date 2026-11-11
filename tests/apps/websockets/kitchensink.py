from __future__ import annotations

import json
import os
import sys
from time import perf_counter_ns
from typing import TYPE_CHECKING

import anyio

if TYPE_CHECKING:
    from asgiref.typing import ASGIReceiveCallable, ASGISendCallable, WebSocketScope

ACCEPT = {"type": "websocket.accept", "headers": (), "subprotocol": None}


async def _send_denial(send: ASGISendCallable, status: int, body: bytes) -> None:
    await send(
        {
            "type": "websocket.http.response.start",
            "status": status,
            "headers": [(b"content-type", b"text/plain"), (b"x-denied", b"yes")],
        }
    )
    await send({"type": "websocket.http.response.body", "body": body})


async def _deny(path: str, send: ASGISendCallable) -> None:
    if path == "/deny":
        await _send_denial(send, 401, b"go away")
        return
    if path == "/deny-empty":
        await send(
            {"type": "websocket.http.response.start", "status": 404, "headers": ()}
        )
        await send({"type": "websocket.http.response.body"})
        return
    if path == "/deny-streaming":
        await send(
            {"type": "websocket.http.response.start", "status": 403, "headers": ()}
        )
        for chunk in (b"go ", b"away "):
            await send(
                {
                    "type": "websocket.http.response.body",
                    "body": chunk,
                    "more_body": True,
                }
            )
        await send({"type": "websocket.http.response.body", "body": b"now"})
        return
    if path == "/deny-content-length":
        await send(
            {
                "type": "websocket.http.response.start",
                "status": 403,
                "headers": [(b"content-length", b"7")],
            }
        )
        await send(
            {"type": "websocket.http.response.body", "body": b"go ", "more_body": True}
        )
        await send({"type": "websocket.http.response.body", "body": b"away"})
        return
    if path == "/deny-invalid-status":
        try:
            await send(
                {"type": "websocket.http.response.start", "status": 101, "headers": ()}
            )
        except ValueError as e:
            await _send_denial(send, 400, str(e).encode())
        return
    if path == "/deny-body-before-start":
        try:
            await send({"type": "websocket.http.response.body", "body": b"oops"})
        except RuntimeError as e:
            await _send_denial(send, 400, str(e).encode())
        return
    if path == "/deny-then-close":
        await send(
            {"type": "websocket.http.response.start", "status": 403, "headers": ()}
        )
        try:
            await send({"type": "websocket.close", "code": 1000})
        except RuntimeError as e:
            await send(
                {"type": "websocket.http.response.body", "body": str(e).encode()}
            )
        return
    if path == "/deny-then-send":
        await _send_denial(send, 403, b"denied")
        try:
            await send({"type": "websocket.send", "text": "too late"})
        except OSError:
            print("deny-then-send: disconnected", file=sys.stderr, flush=True)  # noqa: T201
        return
    if path == "/deny-raise-mid-body":
        await send(
            {"type": "websocket.http.response.start", "status": 403, "headers": ()}
        )
        await send(
            {"type": "websocket.http.response.body", "body": b"part", "more_body": True}
        )
        msg = "boom mid denial"
        raise RuntimeError(msg)
    msg = f"unknown denial path {path}"
    raise AssertionError(msg)


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
    if path == "/send-before-accept":
        await send({"type": "websocket.send", "text": "too early"})
        return
    if path == "/subprotocol":
        offered = list(scope["subprotocols"])
        await send(
            {
                "type": "websocket.accept",
                "headers": (),
                "subprotocol": offered[0] if offered else None,
            }
        )
        await send({"type": "websocket.send", "text": ",".join(offered)})
        return

    if path.startswith("/deny"):
        await _deny(path, send)
        return

    await send(ACCEPT)

    if path == "/extensions":
        await send(
            {
                "type": "websocket.send",
                "text": json.dumps(sorted(scope.get("extensions", {}).keys())),
            }
        )
        return
    if path == "/accept-then-deny":
        try:
            await send(
                {"type": "websocket.http.response.start", "status": 403, "headers": ()}
            )
        except RuntimeError as e:
            await send({"type": "websocket.send", "text": str(e)})
            await send({"type": "websocket.close", "code": 1000})
        return
    if path == "/raise-after":
        msg = "boom after accept"
        raise RuntimeError(msg)
    if path == "/close":
        await send({"type": "websocket.close", "code": 1001, "reason": "bye"})
        return
    if path == "/invalid-close-code":
        try:
            await send({"type": "websocket.close", "code": 1005})
        except ValueError:
            await send({"type": "websocket.send", "text": "rejected"})
            await send({"type": "websocket.close", "code": 1000})
        return
    if path == "/scheme":
        await send({"type": "websocket.send", "text": scope["scheme"]})
        return
    if path == "/root-path":
        await send({"type": "websocket.send", "text": scope["root_path"]})
        return
    if path == "/tls":
        await send(
            {
                "type": "websocket.send",
                "text": json.dumps(scope.get("extensions", {}).get("tls")),
            }
        )
        return
    if path == "/peer":
        client = scope["client"]
        server_addr = scope["server"]
        await send(
            {
                "type": "websocket.send",
                "text": f"{client[0]}:{client[1]}|{server_addr[0]}:{server_addr[1]}",
            }
        )
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
    if path == "/slow-recv":
        # Stall before reading so a flooding client fills the buffer and the
        # server applies request backpressure; the client's send() then pauses
        # until we start draining.
        await anyio.sleep(0.5)
        while True:
            m = await receive()
            if m["type"] == "websocket.disconnect":
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

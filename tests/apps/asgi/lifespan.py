from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from asgiref.typing import ASGIReceiveCallable, ASGISendCallable, Scope


async def _send_success(send: ASGISendCallable) -> None:
    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [(b"content-type", b"text/plain")],
            "trailers": False,
        }
    )
    await send({"type": "http.response.body", "body": b"Ok", "more_body": False})


async def normal(
    scope: Scope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    if scope["type"] == "websocket":
        msg = "Websockets not supported"
        raise RuntimeError(msg)

    if scope["type"] == "lifespan":
        while True:
            msg = await recv()
            if msg["type"] == "lifespan.startup":
                scope.get("state", {})["counter"] = 0
                await send({"type": "lifespan.startup.complete"})
            elif msg["type"] == "lifespan.shutdown":
                print(  # noqa: T201
                    f"Got counter: {scope.get('state', {}).get('counter')}", flush=True
                )
                await send({"type": "lifespan.shutdown.complete"})
                return

    scope.get("state", {})["counter"] += 1
    await _send_success(send)


async def startup_failed(
    scope: Scope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    if scope["type"] == "websocket":
        msg = "Websockets not supported"
        raise RuntimeError(msg)

    if scope["type"] == "lifespan":
        while True:
            msg = await recv()
            if msg["type"] == "lifespan.startup":
                await send(
                    {
                        "type": "lifespan.startup.failed",
                        "message": "I failed to startup",
                    }
                )
            elif msg["type"] == "lifespan.shutdown":
                print(  # noqa: T201
                    f"Got counter: {scope.get('state', {}).get('counter')}", flush=True
                )
                await send({"type": "lifespan.shutdown.complete"})
                return

    scope.get("state", {})["counter"] += 1
    await _send_success(send)


async def shutdown_failed(
    scope: Scope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    if scope["type"] == "websocket":
        msg = "Websockets not supported"
        raise RuntimeError(msg)

    if scope["type"] == "lifespan":
        while True:
            msg = await recv()
            if msg["type"] == "lifespan.startup":
                scope.get("state", {})["counter"] = 0
                await send({"type": "lifespan.startup.complete"})
            elif msg["type"] == "lifespan.shutdown":
                await send(
                    {
                        "type": "lifespan.shutdown.failed",
                        "message": "I failed to shutdown",
                    }
                )
                return

    scope.get("state", {})["counter"] += 1
    await _send_success(send)

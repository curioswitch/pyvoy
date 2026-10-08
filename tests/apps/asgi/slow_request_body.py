"""A deliberately slow-consuming ASGI app used to exercise request-body
backpressure (see tests/test_request_body_backpressure.py).

Unlike tests/apps/asgi/kitchensink.py's _large_bodies, which drains its
receive() loop as fast as possible, this app inserts a delay between each
receive() call to simulate an app whose per-chunk processing (e.g. a
synchronous database insert) falls behind the rate the client sends data --
exactly the scenario that exposes the lack of real request-body backpressure
this test suite covers.
"""

from __future__ import annotations

import asyncio
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from asgiref.typing import ASGIReceiveCallable, ASGISendCallable, Scope


async def app(
    scope: Scope,  # noqa: ARG001 - required by the ASGI calling convention
    receive: ASGIReceiveCallable,
    send: ASGISendCallable,
) -> None:
    total = 0
    while True:
        event = await receive()
        if event["type"] == "http.request":
            total += len(event.get("body", b""))
            await asyncio.sleep(0.01)
            if not event.get("more_body", False):
                break
        elif event["type"] == "http.disconnect":
            break

    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [(b"content-type", b"text/plain")],
        }
    )
    await send(
        {
            "type": "http.response.body",
            "body": str(total).encode(),
            "more_body": False,
        }
    )

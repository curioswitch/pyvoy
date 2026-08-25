from __future__ import annotations

import atexit
import os
from pathlib import Path
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from asgiref.typing import ASGIReceiveCallable, ASGISendCallable, Scope


def _write_marker() -> None:
    Path(os.environ["PYVOY_TEST_ATEXIT_PATH"]).write_text("atexit hook ran\n")


atexit.register(_write_marker)


async def app(
    scope: Scope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    if scope["type"] == "lifespan":
        while True:
            msg = await recv()
            if msg["type"] == "lifespan.startup":
                await send({"type": "lifespan.startup.complete"})
            elif msg["type"] == "lifespan.shutdown":
                await send({"type": "lifespan.shutdown.complete"})
                return

    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [(b"content-type", b"text/plain")],
            "trailers": False,
        }
    )
    await send({"type": "http.response.body", "body": b"Ok", "more_body": False})

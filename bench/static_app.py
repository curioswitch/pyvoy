from __future__ import annotations

import os
from typing import TYPE_CHECKING, cast

from starlette.applications import Starlette
from starlette.routing import Mount
from starlette.staticfiles import StaticFiles

if TYPE_CHECKING:
    from asgiref.typing import (
        ASGIReceiveCallable,
        ASGISendCallable,
        ASGISendEvent,
        Scope,
    )

# The directory of files to serve, set by run_static_benchmark before launching.
_STATIC_DIR = os.environ.get("BENCH_STATIC_DIR", ".")

# A Starlette application that serves the benchmark files at /static, so a
# request goes through the full Python ASGI stack per file.
app = Starlette(
    routes=[Mount("/static", app=StaticFiles(directory=_STATIC_DIR), name="static")]
)


async def noop(
    _scope: Scope, _receive: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    """A placeholder ASGI app for the native static-mount configurations, where the
    server serves files itself and this application is never reached."""
    await send(
        cast(
            "ASGISendEvent",
            {"type": "http.response.start", "status": 404, "headers": []},
        )
    )
    await send(
        cast(
            "ASGISendEvent",
            {"type": "http.response.body", "body": b"", "more_body": False},
        )
    )

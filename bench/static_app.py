from __future__ import annotations

import os
from typing import TYPE_CHECKING, Any, cast

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
_files = Starlette(
    routes=[Mount("/static", app=StaticFiles(directory=_STATIC_DIR), name="static")]
)


async def app(
    scope: Scope, receive: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    # Our benchmark is comparing pure python serving to native mounting so
    # force-disable pathsend to always use pure python.
    extensions = cast("dict[str, Any]", scope).get("extensions")
    if isinstance(extensions, dict):
        extensions.pop("http.response.pathsend", None)
    await _files(scope, receive, send)


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

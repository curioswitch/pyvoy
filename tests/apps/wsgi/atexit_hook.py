from __future__ import annotations

import atexit
import os
from pathlib import Path
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    import sys
    from collections.abc import Iterable

    if sys.version_info >= (3, 11):
        from wsgiref.types import StartResponse, WSGIEnvironment
    else:
        from _typeshed.wsgi import StartResponse, WSGIEnvironment


def _write_marker() -> None:
    Path(os.environ["PYVOY_TEST_ATEXIT_PATH"]).write_text("atexit hook ran\n")


atexit.register(_write_marker)


def app(
    _environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    start_response("200 OK", [("content-type", "text/plain")])
    return [b"Ok"]

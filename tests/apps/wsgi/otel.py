from __future__ import annotations

import json
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from collections.abc import Iterable
    from wsgiref.types import StartResponse, WSGIEnvironment


# Reports the trace context the application was given.
def app(environ: WSGIEnvironment, start_response: StartResponse) -> Iterable[bytes]:
    body = json.dumps(
        {
            "traceparent": environ.get("HTTP_TRACEPARENT"),
            "tracestate": environ.get("HTTP_TRACESTATE"),
        }
    ).encode()
    start_response(
        "200 OK",
        [("content-type", "application/json"), ("content-length", str(len(body)))],
    )
    return [body]

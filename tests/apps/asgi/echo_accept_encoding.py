from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from asgiref.typing import ASGIReceiveCallable, ASGISendCallable, Scope


# Reports the Accept-Encoding header the application was given, padded so the
# response is long enough to be worth compressing.
async def app(scope: Scope, _recv: ASGIReceiveCallable, send: ASGISendCallable) -> None:
    if scope["type"] != "http":
        return
    accept_encoding = dict(scope["headers"]).get(b"accept-encoding", b"<absent>")
    body = b"accept-encoding=" + accept_encoding + b" " + b"padding " * 8
    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [
                (b"content-type", b"text/plain"),
                (b"content-length", str(len(body)).encode()),
            ],
            "trailers": False,
        }
    )
    await send({"type": "http.response.body", "body": body, "more_body": False})

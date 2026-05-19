from __future__ import annotations

import traceback
from typing import TYPE_CHECKING

from pyqwest import Client, HTTPVersion

from pyvoy.asgi.httpclient import HTTPTransport

from .cases import client

if TYPE_CHECKING:
    from asgiref.typing import ASGIReceiveCallable, ASGISendCallable, Scope

client_h1c = Client(HTTPTransport("backend_h1c"))
client_h1 = Client(HTTPTransport("backend_h1"))
client_h2c = Client(HTTPTransport("backend_h2c"))
client_h2 = Client(HTTPTransport("backend_h2"))


async def app(
    scope: Scope, _receive: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    assert scope["type"] == "http"  # noqa: S101
    headers = {k.decode(): v.decode() for k, v in scope["headers"]}
    scheme = headers["x-test-scheme"]
    http_version_str = headers["x-test-http-version"]
    match http_version_str:
        case "h1":
            http_version = HTTPVersion.HTTP1
        case "h2":
            http_version = HTTPVersion.HTTP2
        case _:
            http_version = None
    match (scheme, http_version_str):
        case ("http", "h1"):
            http_client = client_h1c
        case ("https", "h1"):
            http_client = client_h1
        case ("http", "h2"):
            http_client = client_h2c
        case ("https", "h2"):
            http_client = client_h2
        case _:
            msg = f"Unknown scheme and HTTP version combination: {scheme} {http_version_str}"
            raise RuntimeError(msg)
    url = f"http{'s' if scheme == 'https' else ''}://localhost"

    try:
        match headers["x-test-case"]:
            case "client_get":
                await client.get(http_client, url)
            case "client_post":
                await client.post(http_client, url, http_version)
            case _:
                msg = f"Unknown test case: {headers['x-test-case']}"
                raise RuntimeError(msg)  # noqa: TRY301
    except Exception:
        response_body = traceback.format_exc().encode()
        await send(
            {
                "type": "http.response.start",
                "status": 500,
                "headers": [
                    (b"content-type", b"text/plain"),
                    (b"content-length", str(len(response_body)).encode()),
                ],
                "trailers": False,
            }
        )
        await send(
            {"type": "http.response.body", "body": response_body, "more_body": False}
        )
        return

    await send(
        {"type": "http.response.start", "status": 200, "headers": [], "trailers": False}
    )
    await send({"type": "http.response.body", "body": b"", "more_body": False})

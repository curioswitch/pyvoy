from __future__ import annotations

import traceback
from typing import TYPE_CHECKING

from pyqwest import Client, HTTPVersion

from pyvoy.asgi.httpclient import HTTPTransport

from .cases import client, errors

if TYPE_CHECKING:
    from asgiref.typing import ASGIReceiveCallable, ASGISendCallable, Scope

client_h1c = Client(HTTPTransport("backend_h1c"))
client_h1 = Client(HTTPTransport("backend_h1"))
client_h2c = Client(HTTPTransport("backend_h2c"))
client_h2 = Client(HTTPTransport("backend_h2"))
client_unavailable = Client(HTTPTransport("backend_unavailable"))


async def app(
    scope: Scope, _receive: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    assert scope["type"] == "http"  # noqa: S101
    headers = {k.decode(): v.decode() for k, v in scope["headers"]}
    scheme = headers["x-test-scheme"]
    http_version_str = headers["x-test-http-version"]
    extra = headers.get("x-test-extra", "")
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
    url = f"http{'s' if scheme == 'https' else ''}://localhost:9999"

    try:
        match headers["x-test-case"]:
            case "client_basic":
                await client.basic(http_client, url, http_version, 9999)
            case "client_iterable_body":
                await client.iterable_body(http_client, url)
            case "client_empty_request":
                await client.empty_request(http_client, url)
            case "client_bidi":
                await client.test_bidi(http_client, url, http_version)
            case "client_large_body":
                await client.large_body(http_client, url, http_version)
            case "client_readall":
                await client.readall(http_client, url)
            case "client_execute":
                await client.execute(http_client, url)
            case "client_execute_json":
                await client.execute_json(http_client, url)
            case "client_get":
                await client.get(http_client, url)
            case "client_post":
                await client.post(http_client, url, http_version)
            case "client_delete":
                await client.delete(http_client, url)
            case "client_head":
                await client.head(http_client, url)
            case "client_options":
                await client.options(http_client, url)
            case "client_patch":
                await client.patch(http_client, url)
            case "client_put":
                await client.put(http_client, url)
            case "client_nihongo":
                await client.nihongo(http_client, url)
            case "client_json_content":
                await client.json_content(http_client, url, extra)
            case "client_json_content_existing_content_type":
                await client.json_content_existing_content_type(http_client, url)
            case "client_close_no_read":
                await client.close_no_read(http_client, url)
            case "client_close_pending_read":
                await client.close_pending_read(http_client, url)
            case "client_request_content_error":
                await client.request_content_error(http_client, url)
            case "client_response_error":
                await client.response_error(http_client, url)
            case "errors_request_timeout":
                await errors.request_timeout(http_client, url)
            case "errors_response_content_timeout":
                await errors.response_content_timeout(http_client, url)
            case "errors_connection_error":
                await errors.connection_error(client_unavailable, url)
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

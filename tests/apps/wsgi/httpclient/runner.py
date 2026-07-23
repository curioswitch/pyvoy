from __future__ import annotations

import asyncio
import traceback
from typing import TYPE_CHECKING

from pyqwest import HTTPVersion, SyncClient
from pyvoy.wsgi.httpclient import HTTPTransport

from tests.apps.httpclient.cases import client, errors, tls, validation

if TYPE_CHECKING:
    from collections.abc import Callable, Iterable

client_h1c = SyncClient(HTTPTransport("backend_h1c"))
client_h1 = SyncClient(HTTPTransport("backend_h1"))
client_h2c = SyncClient(HTTPTransport("backend_h2c"))
client_h2 = SyncClient(HTTPTransport("backend_h2"))
client_unavailable = SyncClient(HTTPTransport("backend_unavailable"))


def app(
    environ: dict[str, str],
    start_response: Callable[[str, list[tuple[str, str]]], object],
) -> Iterable[bytes]:
    scheme = environ["HTTP_X_TEST_SCHEME"]
    http_version_str = environ["HTTP_X_TEST_HTTP_VERSION"]
    extra = environ.get("HTTP_X_TEST_EXTRA", "")
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
        match environ["HTTP_X_TEST_CASE"]:
            case "client_basic":
                asyncio.run(client.basic(http_client, url, http_version, 9999))
            case "client_iterable_body":
                asyncio.run(client.iterable_body(http_client, url))
            case "client_empty_request":
                asyncio.run(client.empty_request(http_client, url))
            case "client_bidi":
                asyncio.run(client.test_bidi(http_client, url, http_version))
            case "client_large_body":
                asyncio.run(client.large_body(http_client, url, http_version))
            case "client_readall":
                asyncio.run(client.readall(http_client, url))
            case "client_execute":
                asyncio.run(client.execute(http_client, url))
            case "client_execute_json":
                asyncio.run(client.execute_json(http_client, url))
            case "client_get":
                asyncio.run(client.get(http_client, url))
            case "client_post":
                asyncio.run(client.post(http_client, url, http_version))
            case "client_delete":
                asyncio.run(client.delete(http_client, url))
            case "client_head":
                asyncio.run(client.head(http_client, url))
            case "client_options":
                asyncio.run(client.options(http_client, url))
            case "client_patch":
                asyncio.run(client.patch(http_client, url))
            case "client_put":
                asyncio.run(client.put(http_client, url))
            case "client_nihongo":
                asyncio.run(client.nihongo(http_client, url))
            case "client_json_content":
                asyncio.run(client.json_content(http_client, url, extra))
            case "client_json_content_existing_content_type":
                asyncio.run(client.json_content_existing_content_type(http_client, url))
            case "client_request_content_error":
                asyncio.run(client.request_content_error(http_client, url))
            case "client_response_error":
                asyncio.run(client.response_error(http_client, url))
            case "errors_request_timeout":
                asyncio.run(errors.request_timeout(http_client, url))
            case "errors_response_content_timeout":
                asyncio.run(errors.response_content_timeout(http_client, url))
            case "errors_connection_error":
                asyncio.run(errors.connection_error(client_unavailable, url))
            case "tls_mtls":
                asyncio.run(tls.mtls(http_client, url))
            case "transport_invalid_option":
                validation.transport_invalid_option()
            case _:
                msg = f"Unknown test case: {environ['HTTP_X_TEST_CASE']}"
                raise RuntimeError(msg)  # noqa: TRY301
    except Exception:
        response_body = traceback.format_exc().encode()
        start_response(
            "500 Internal Server Error",
            [
                ("content-type", "text/plain"),
                ("content-length", str(len(response_body))),
            ],
        )
        return [response_body]

    start_response("200 OK", [])
    return [b""]

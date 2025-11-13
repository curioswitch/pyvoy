from __future__ import annotations

import asyncio
import subprocess
from asyncio import StreamReader
from typing import TYPE_CHECKING

import httpx
import pytest
import pytest_asyncio

from pyvoy import Interface, PyvoyServer

if TYPE_CHECKING:
    from collections.abc import AsyncIterator

from ._util import assert_logs_contains, find_logs_lines


@pytest_asyncio.fixture(scope="module")
async def server_asgi() -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.asgi.kitchensink", stderr=subprocess.STDOUT, stdout=subprocess.PIPE
    ) as server:
        yield server


@pytest_asyncio.fixture(scope="module")
async def server_wsgi() -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.wsgi.kitchensink",
        interface="wsgi",
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        yield server


@pytest.fixture
def logs_asgi(server_asgi: PyvoyServer) -> StreamReader:
    assert server_asgi.stdout is not None
    return server_asgi.stdout


@pytest.fixture
def logs_wsgi(server_wsgi: PyvoyServer) -> StreamReader:
    assert server_wsgi.stdout is not None
    return server_wsgi.stdout


@pytest.fixture
def logs(
    interface: Interface, logs_asgi: StreamReader, logs_wsgi: StreamReader
) -> StreamReader:
    match interface:
        case "asgi":
            return logs_asgi
        case "wsgi":
            return logs_wsgi


@pytest_asyncio.fixture
async def url_asgi(server_asgi: PyvoyServer) -> AsyncIterator[str]:
    yield f"http://{server_asgi.listener_address}:{server_asgi.listener_port}"


@pytest_asyncio.fixture
async def url_wsgi(server_wsgi: PyvoyServer) -> AsyncIterator[str]:
    yield f"http://{server_wsgi.listener_address}:{server_wsgi.listener_port}"


@pytest.fixture(params=["asgi", "wsgi"])
def interface(request: pytest.FixtureRequest) -> Interface:
    return request.param


@pytest.fixture
def url(interface: Interface, url_asgi: str, url_wsgi: str) -> str:
    match interface:
        case "asgi":
            return url_asgi
        case "wsgi":
            return url_wsgi


@pytest_asyncio.fixture
async def client() -> AsyncIterator[httpx.AsyncClient]:
    async with httpx.AsyncClient() as client:
        yield client


@pytest.mark.asyncio
async def test_headers_only(url: str, client: httpx.AsyncClient) -> None:
    response = await client.get(
        f"{url}/headers-only",
        headers=(("Accept", "text/plain"), ("Multiple", "v1"), ("Multiple", "v2")),
    )
    assert response.status_code == 200, response.text
    assert response.headers["x-animal"] == "bear"
    assert response.headers["content-type"] == "text/plain"
    assert response.content == b""


@pytest.mark.asyncio
async def test_request_body(url: str, client: httpx.AsyncClient) -> None:
    response = await client.post(f"{url}/request-body", content="Bear please")
    assert response.status_code == 200, response.text
    assert response.headers["x-animal"] == "bear"
    assert response.headers["content-type"] == "text/plain"
    assert response.content == b""


@pytest.mark.asyncio
async def test_response_body(url: str, client: httpx.AsyncClient) -> None:
    response = await client.get(f"{url}/response-body")
    assert response.status_code == 200, response.text
    assert response.headers["content-type"] == "text/plain"
    assert response.content == b"Hello world!"


@pytest.mark.asyncio
async def test_request_and_response_body(url: str, client: httpx.AsyncClient) -> None:
    response = await client.post(
        f"{url}/request-and-response-body", content="Bear please"
    )
    assert response.status_code == 200, response.text
    assert response.headers["content-type"] == "text/plain"
    assert response.content == b"Yogi Bear"


@pytest.mark.asyncio
async def test_large_bodies(url: str, client: httpx.AsyncClient) -> None:
    response = await client.post(f"{url}/large-bodies", content=b"A" * 1_000_000)
    assert response.status_code == 200, response.text
    assert response.headers["content-type"] == "text/plain"
    assert response.content == b"B" * 1_000_000


@pytest.mark.asyncio
async def test_empty_request_body_http2(url: str) -> None:
    async with httpx.AsyncClient(http2=True, http1=False) as client:
        response = await client.post(f"{url}/controlled", content=b"")
    assert response.status_code == 200, response.text
    assert response.content == b""


@pytest.mark.asyncio
async def test_exception_before_response(
    url: str, client: httpx.AsyncClient, logs: StreamReader
) -> None:
    response = await client.get(f"{url}/exception-before-response")
    assert response.status_code == 500
    assert response.headers["content-type"] == "text/plain; charset=utf-8"
    assert response.content == b"Internal Server Error"
    await assert_logs_contains(
        logs,
        [
            "Traceback (most recent call last):",
            "RuntimeError: We have failed before the response",
        ],
    )


# Because we only flush headers when the first body message is available,
# both below tests end up behaving the same but it may still be worth keeping
# them separate in case of regressions.


@pytest.mark.asyncio
async def test_exception_after_response_headers(
    url: str, client: httpx.AsyncClient, logs: StreamReader
) -> None:
    with pytest.raises(httpx.RemoteProtocolError) as exc_info:
        await client.get(f"{url}/exception-after-response-headers")
    assert "peer closed connection without sending complete message body" in str(
        exc_info.value
    )
    await assert_logs_contains(
        logs,
        [
            "Traceback (most recent call last):",
            "RuntimeError: We have failed after response headers",
        ],
    )


@pytest.mark.asyncio
async def test_exception_after_response_body(
    url: str, client: httpx.AsyncClient, logs: StreamReader
) -> None:
    with pytest.raises(httpx.RemoteProtocolError) as exc_info:
        await client.get(f"{url}/exception-after-response-body")
    assert "peer closed connection without sending complete message body" in str(
        exc_info.value
    )
    await assert_logs_contains(
        logs,
        [
            "Traceback (most recent call last):",
            "RuntimeError: We have failed after response body",
        ],
    )


# Not possible to model with WSGI
@pytest.mark.asyncio
async def test_exception_after_response_complete(
    url_asgi: str, client: httpx.AsyncClient, logs_asgi: StreamReader
) -> None:
    response = await client.get(f"{url_asgi}/exception-after-response-complete")
    assert response.status_code == 200, response.text
    assert response.content == b"Hello World!!!"
    await assert_logs_contains(
        logs_asgi,
        [
            "Traceback (most recent call last):",
            "RuntimeError: We have failed after response complete",
        ],
    )


# TODO: WSGI for client disconnect cases
@pytest.mark.asyncio
# Filter logic has a fast path for empty content so we need to test both.
@pytest.mark.parametrize("content", [b"", b"hello"])
async def test_client_closed_before_response(
    url_asgi: str, logs_asgi: StreamReader, content: bytes
) -> None:
    # httpx seems to response.aclose() until there are response headers, so
    # we use a timeout instead.
    with pytest.raises(httpx.TimeoutException):
        async with (
            httpx.AsyncClient(timeout=0.1) as client,
            client.stream(
                "GET", f"{url_asgi}/client-closed-before-response", content=content
            ) as response,
        ):
            await response.aclose()
    read_lines = await assert_logs_contains(
        logs_asgi,
        [
            "send raised OSError as expected",
            "client-closed-before-response assertions passed",
        ],
    )
    assert "Traceback (most recent call last):" not in read_lines
    # Hard to assert this precisely but wait for a bit of time for more logs
    # to see if the exception appears.
    _, read_lines = await asyncio.wait_for(
        find_logs_lines(logs_asgi, ["not happening"]), timeout=0.5
    )
    assert "Traceback (most recent call last):" not in read_lines


@pytest.mark.asyncio
async def test_client_closed_after_response_start(
    url_asgi: str, client: httpx.AsyncClient, logs_asgi: StreamReader
) -> None:
    try:
        async with client.stream(
            "GET", f"{url_asgi}/client-closed-after-response-start"
        ) as response:
            await response.aclose()
    except httpx.TimeoutException:
        pass
    await assert_logs_contains(
        logs_asgi,
        [
            "send raised OSError as expected",
            "client-closed-after-response-start assertions passed",
        ],
    )


@pytest.mark.asyncio
async def test_client_closed_after_trailers_start(
    url_asgi: str, client: httpx.AsyncClient, logs_asgi: StreamReader
) -> None:
    async with client.stream(
        "GET",
        f"{url_asgi}/client-closed-after-trailers-start",
        headers={"te": "trailers"},
    ) as response:
        await response.aclose()
    await assert_logs_contains(
        logs_asgi,
        [
            "send raised OSError as expected",
            "client-closed-after-trailers-start assertions passed",
        ],
    )


@pytest.mark.asyncio
async def test_asgi_bad_app_missing_type(
    url_asgi: str, client: httpx.AsyncClient
) -> None:
    response = await client.get(f"{url_asgi}/bad-app-missing-type")
    assert response.status_code == 200, response.text


@pytest.mark.asyncio
async def test_asgi_bad_app_start_missing_status(
    url_asgi: str, client: httpx.AsyncClient
) -> None:
    response = await client.get(f"{url_asgi}/bad-app-start-missing-status")
    assert response.status_code == 200, response.text


@pytest.mark.asyncio
async def test_asgi_bad_app_body_before_start(
    url_asgi: str, client: httpx.AsyncClient
) -> None:
    response = await client.get(f"{url_asgi}/bad-app-body-before-start")
    assert response.status_code == 200, response.text


@pytest.mark.asyncio
async def test_asgi_bad_app_start_after_start(
    url_asgi: str, client: httpx.AsyncClient
) -> None:
    response = await client.get(f"{url_asgi}/bad-app-start-after-start")
    assert response.status_code == 200, response.text


@pytest.mark.asyncio
async def test_asgi_bad_app_body_after_complete(
    url_asgi: str, client: httpx.AsyncClient, logs_asgi: StreamReader
) -> None:
    response = await client.get(f"{url_asgi}/bad-app-body-after-complete")
    assert response.status_code == 200, response.text
    await assert_logs_contains(
        logs_asgi, ["bad-app-body-after-complete assertions passed"]
    )


# While httpx doesn't support trailers, it doesn't matter since we only check if
# the handler raised an exception.
@pytest.mark.asyncio
async def test_asgi_bad_app_start_instead_of_trailers(
    url_asgi: str, client: httpx.AsyncClient
) -> None:
    response = await client.get(
        f"{url_asgi}/bad-app-start-instead-of-trailers", headers={"te": "trailers"}
    )
    assert response.status_code == 200, response.text


@pytest.mark.asyncio
async def test_asgi_bad_app_invalid_header_name(
    url_asgi: str, client: httpx.AsyncClient
) -> None:
    response = await client.get(f"{url_asgi}/bad-app-invalid-header-name")
    assert response.status_code == 200, response.text


@pytest.mark.asyncio
async def test_asgi_bad_app_invalid_header_value(
    url_asgi: str, client: httpx.AsyncClient
) -> None:
    response = await client.get(f"{url_asgi}/bad-app-invalid-header-value")
    assert response.status_code == 200, response.text


@pytest.mark.asyncio
async def test_standard_logs(
    url: str, client: httpx.AsyncClient, logs: StreamReader
) -> None:
    response = await client.get(f"{url}/print-logs")
    assert response.status_code == 200, response.text

    await assert_logs_contains(
        logs, ["This is a stdout print", "This is a stderr print"]
    )


@pytest.mark.asyncio
async def test_all_the_headers(url: str, client: httpx.AsyncClient) -> None:
    # We memoize header names when passing to the app and test as many as we can.
    # Skipped headers are handled by envoy and not passed to our filter.
    headers = [
        ("accept", "accept"),
        ("accept-charset", "accept-charset"),
        ("accept-encoding", "accept-encoding"),
        ("accept-language", "accept-language"),
        ("accept-ranges", "accept-ranges"),
        ("access-control-allow-credentials", "access-control-allow-credentials"),
        ("access-control-allow-headers", "access-control-allow-headers"),
        ("access-control-allow-methods", "access-control-allow-methods"),
        ("access-control-allow-origin", "access-control-allow-origin"),
        ("access-control-expose-headers", "access-control-expose-headers"),
        ("access-control-max-age", "access-control-max-age"),
        ("access-control-request-headers", "access-control-request-headers"),
        ("access-control-request-method", "access-control-request-method"),
        ("age", "age"),
        ("allow", "allow"),
        ("alt-svc", "alt-svc"),
        ("authorization", "authorization"),
        ("cache-control", "cache-control"),
        ("cache-status", "cache-status"),
        ("cdn-cache-control", "cdn-cache-control"),
        # Skip connection
        ("content-disposition", "content-disposition"),
        ("content-encoding", "content-encoding"),
        ("content-language", "content-language"),
        # Skip content-length
        ("content-location", "content-location"),
        ("content-range", "content-range"),
        ("content-security-policy", "content-security-policy"),
        ("content-security-policy-report-only", "content-security-policy-report-only"),
        ("content-type", "content-type"),
        ("cookie", "cookie"),
        ("dnt", "dnt"),
        ("date", "date"),
        ("etag", "etag"),
        ("expect", "expect"),
        ("expires", "expires"),
        ("forwarded", "forwarded"),
        ("from", "from"),
        # Skip host
        ("if-match", "if-match"),
        ("if-modified-since", "if-modified-since"),
        ("if-none-match", "if-none-match"),
        ("if-range", "if-range"),
        ("if-unmodified-since", "if-unmodified-since"),
        ("last-modified", "last-modified"),
        ("link", "link"),
        ("location", "location"),
        ("max-forwards", "max-forwards"),
        ("origin", "origin"),
        ("pragma", "pragma"),
        ("proxy-authenticate", "proxy-authenticate"),
        ("proxy-authorization", "proxy-authorization"),
        ("public-key-pins", "public-key-pins"),
        ("public-key-pins-report-only", "public-key-pins-report-only"),
        ("range", "range"),
        ("referer", "referer"),
        ("referrer-policy", "referrer-policy"),
        ("refresh", "refresh"),
        ("retry-after", "retry-after"),
        ("sec-websocket-accept", "sec-websocket-accept"),
        ("sec-websocket-extensions", "sec-websocket-extensions"),
        ("sec-websocket-key", "sec-websocket-key"),
        ("sec-websocket-protocol", "sec-websocket-protocol"),
        ("sec-websocket-version", "sec-websocket-version"),
        ("server", "server"),
        ("set-cookie", "set-cookie"),
        ("strict-transport-security", "strict-transport-security"),
        # Skip te
        ("trailer", "trailer"),
        # Skip transfer-encoding
        ("user-agent", "user-agent"),
        # Skip upgrade
        ("upgrade-insecure-requests", "upgrade-insecure-requests"),
        ("vary", "vary"),
        ("via", "via"),
        ("warning", "warning"),
        ("www-authenticate", "www-authenticate"),
        ("x-content-type-options", "x-content-type-options"),
        ("x-dns-prefetch-control", "x-dns-prefetch-control"),
        ("x-frame-options", "x-frame-options"),
        ("x-xss-protection", "x-xss-protection"),
        ("x-pyvoy", "x-pyvoy"),
        ("x-pyvoy", "x-pyvoy-2"),
    ]
    response = await client.get(f"{url}/all-the-headers", headers=headers)
    assert response.status_code == 200, response.text


@pytest.mark.asyncio
async def test_nihongo(url: str, client: httpx.AsyncClient) -> None:
    response = await client.get(f"{url}/日本語")
    assert response.status_code == 200, response.text


@pytest.mark.asyncio
async def test_scope_content(url: str, client: httpx.AsyncClient) -> None:
    response = await client.post(
        f"{url}/echo-scope", content=b"Hello", headers={"content-type": "text/plain"}
    )
    assert response.status_code == 200, response.text
    assert response.headers["x-scope-content-length"] == "5"
    assert response.headers["x-scope-content-type"] == "text/plain"


@pytest.mark.asyncio
async def test_scope_query(url: str, client: httpx.AsyncClient) -> None:
    response = await client.get(f"{url}/echo-scope?message=hello&lang=英語")
    assert response.status_code == 200, response.text
    assert response.headers["x-scope-query"] == "message=hello&lang=%E8%8B%B1%E8%AA%9E"


@pytest.mark.asyncio
# Skip CONNECT since it's only for websockets
@pytest.mark.parametrize(
    "method", ["OPTIONS", "GET", "POST", "PUT", "DELETE", "HEAD", "TRACE", "PATCH"]
)
async def test_scope_http_method(
    url: str, client: httpx.AsyncClient, method: str
) -> None:
    response = await client.request(method, f"{url}/echo-scope")
    assert response.status_code == 200, response.text
    assert response.headers["x-scope-method"] == method


@pytest.mark.asyncio
@pytest.mark.parametrize("http2", [False, True])
async def test_scope_http_version(url: str, http2: bool, interface: Interface) -> None:
    async with httpx.AsyncClient(http2=http2, http1=not http2) as client:
        response = await client.get(f"{url}/echo-scope")
    assert response.status_code == 200, response.text
    match http2, interface:
        case True, "asgi":
            expected = "2"
        case False, "asgi":
            expected = "1.1"
        case True, "wsgi":
            expected = "HTTP/2"
        case False, "wsgi":
            expected = "HTTP/1.1"
    assert response.headers["x-scope-http-version"] == expected


@pytest.mark.asyncio
async def test_wsgi_readline(url_wsgi: str, client: httpx.AsyncClient) -> None:
    content = b"Hello\nWorld\nGoodbye"
    response = await client.post(f"{url_wsgi}/readline", content=content)
    assert response.status_code == 200, response.text


@pytest.mark.asyncio
async def test_wsgi_iterlines(url_wsgi: str, client: httpx.AsyncClient) -> None:
    content = b"Animal\nBear\nCat"
    response = await client.post(f"{url_wsgi}/iterlines", content=content)
    assert response.status_code == 200, response.text


@pytest.mark.asyncio
async def test_wsgi_readlines(url_wsgi: str, client: httpx.AsyncClient) -> None:
    content = b"Food\nPizza\nBurrito"
    response = await client.post(f"{url_wsgi}/readlines", content=content)
    assert response.status_code == 200, response.text


@pytest.mark.asyncio
async def test_wsgi_write_callable(url_wsgi: str, client: httpx.AsyncClient) -> None:
    response = await client.get(f"{url_wsgi}/write-callable")
    assert response.status_code == 200, response.text
    assert response.text == "Hello World and Goodbye!"


@pytest.mark.asyncio
async def test_errors_output(
    url_wsgi: str, client: httpx.AsyncClient, logs_wsgi: asyncio.StreamReader
) -> None:
    response = await client.get(f"{url_wsgi}/errors-output")
    assert response.status_code == 200, response.text
    await assert_logs_contains(
        logs_wsgi,
        [
            "Hello World",
            "Goodbye Earth",
            "Hello again",
            "Animal: Bear",
            "Food: Pizza",
            "Drink: Beer",
        ],
    )

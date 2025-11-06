import asyncio
import subprocess
from asyncio import StreamReader
from collections.abc import AsyncIterator

import httpx
import pytest
import pytest_asyncio

from pyvoy import Interface, PyvoyServer


async def _find_logs_lines(
    logs: StreamReader, expected_lines: list[str]
) -> tuple[set[str], list[str]]:
    found_lines: set[str] = set()
    read_lines: list[str] = []
    try:
        async for line in logs:
            read_lines.append(line.decode())
            decoded_line = line.decode().rstrip()
            if decoded_line in expected_lines:
                found_lines.add(decoded_line)
            if len(found_lines) == len(expected_lines):
                break
    except asyncio.CancelledError:
        pass
    return found_lines, read_lines


async def assert_logs_contains(logs: StreamReader, expected_lines: list[str]) -> None:
    found_lines, read_lines = await asyncio.wait_for(
        _find_logs_lines(logs, expected_lines), timeout=5.0
    )
    missing_lines = set(expected_lines) - found_lines
    assert not missing_lines, (
        f"Missing log lines: {missing_lines}\nRead lines: {''.join(read_lines)}"
    )


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
    assert response.headers["content-type"] == "text/plain"
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
    try:
        async with (
            httpx.AsyncClient(timeout=0.001) as client,
            client.stream(
                "GET", f"{url_asgi}/client-closed-before-response", content=content
            ) as response,
        ):
            await response.aclose()
    except httpx.TimeoutException:
        pass
    await assert_logs_contains(
        logs_asgi,
        [
            "send raised OSError as expected",
            "client-closed-before-response assertions passed",
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
async def test_standard_logs(
    url: str, client: httpx.AsyncClient, logs: StreamReader
) -> None:
    response = await client.get(f"{url}/print-logs")
    assert response.status_code == 200, response.text

    await assert_logs_contains(
        logs, ["This is a stdout print", "This is a stderr print"]
    )

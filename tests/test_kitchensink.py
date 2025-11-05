from collections.abc import AsyncIterator

import httpx
import pytest
import pytest_asyncio

from pyvoy import Interface, PyvoyServer


@pytest_asyncio.fixture(scope="module")
async def url_asgi() -> AsyncIterator[str]:
    async with PyvoyServer(
        "tests.apps.asgi.kitchensink", stderr=None, stdout=None
    ) as server:
        yield f"http://{server.listener_address}:{server.listener_port}"


@pytest_asyncio.fixture(scope="module")
async def url_wsgi() -> AsyncIterator[str]:
    async with PyvoyServer(
        "tests.apps.wsgi.kitchensink", interface="wsgi", stderr=None, stdout=None
    ) as server:
        yield f"http://{server.listener_address}:{server.listener_port}"


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
async def test_exception_before_response(url: str, client: httpx.AsyncClient) -> None:
    response = await client.get(f"{url}/exception-before-response")
    assert response.status_code == 500
    assert response.headers["content-type"] == "text/plain; charset=utf-8"
    assert response.content == b"Internal Server Error"


# Because we only flush headers when the first body message is available,
# both below tests end up behaving the same but it may still be worth keeping
# them separate in case of regressions.


@pytest.mark.asyncio
async def test_exception_after_response_headers(
    url: str, client: httpx.AsyncClient
) -> None:
    with pytest.raises(httpx.RemoteProtocolError) as exc_info:
        await client.get(f"{url}/exception-after-response-headers")
    assert "peer closed connection without sending complete message body" in str(
        exc_info.value
    )


@pytest.mark.asyncio
async def test_exception_after_response_body(
    url: str, client: httpx.AsyncClient
) -> None:
    with pytest.raises(httpx.RemoteProtocolError) as exc_info:
        await client.get(f"{url}/exception-after-response-body")
    assert "peer closed connection without sending complete message body" in str(
        exc_info.value
    )


# Not possible to model with WSGI
@pytest.mark.asyncio
async def test_exception_after_response_complete(
    url_asgi: str, client: httpx.AsyncClient
) -> None:
    response = await client.get(f"{url_asgi}/exception-after-response-complete")
    assert response.status_code == 200, response.text
    assert response.headers["content-type"] == "text/plain"
    assert response.content == b"Hello World!!!"
    # TODO: Check server logs


@pytest.mark.asyncio
async def test_asgi_bad_app_missing_type(
    url_asgi: str, client: httpx.AsyncClient
) -> None:
    response = await client.get(f"{url_asgi}/bad-app-missing-type")
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

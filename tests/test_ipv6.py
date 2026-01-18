from __future__ import annotations

from typing import TYPE_CHECKING

import pytest
import pytest_asyncio

from pyvoy import Interface, PyvoyServer

if TYPE_CHECKING:
    from collections.abc import AsyncIterator

    from pyqwest import Client


@pytest_asyncio.fixture(scope="module")
async def server_asgi() -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.asgi.kitchensink", address="::1", port=0
    ) as server:
        yield server


@pytest_asyncio.fixture(scope="module")
async def server_wsgi() -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.wsgi.kitchensink", interface="wsgi", address="::1", port=0
    ) as server:
        yield server


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
async def url_asgi(server_asgi: PyvoyServer) -> AsyncIterator[str]:
    yield f"http://localhost:{server_asgi.listener_port}"


@pytest_asyncio.fixture
async def url_wsgi(server_wsgi: PyvoyServer) -> AsyncIterator[str]:
    yield f"http://localhost:{server_wsgi.listener_port}"


@pytest.mark.asyncio
async def test_scope_content(url: str, client: Client) -> None:
    port = url.split(":")[-1]
    response = await client.get(f"{url}/echo-scope")
    assert response.status == 200, response.text()
    assert response.headers["x-scope-server-address"] == "::1"
    assert response.headers["x-scope-server-port"] == port

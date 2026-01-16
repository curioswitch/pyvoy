from __future__ import annotations

import subprocess
from typing import TYPE_CHECKING

import pytest
import pytest_asyncio
from pyqwest import Client

from pyvoy import Interface, Mount, PyvoyServer

if TYPE_CHECKING:
    from collections.abc import AsyncIterator


@pytest_asyncio.fixture(scope="module")
async def server() -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        [
            Mount(app="tests.apps.asgi.kitchensink", path="/asgi", interface="asgi"),
            Mount(app="tests.apps.wsgi.kitchensink", path="/wsgi", interface="wsgi"),
        ],
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        yield server


@pytest_asyncio.fixture
async def url_asgi(server: PyvoyServer) -> AsyncIterator[str]:
    yield f"http://{server.listener_address}:{server.listener_port}/asgi"


@pytest_asyncio.fixture
async def url_wsgi(server: PyvoyServer) -> AsyncIterator[str]:
    yield f"http://{server.listener_address}:{server.listener_port}/wsgi"


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


@pytest.mark.asyncio
async def test_controlled(url: str, client: Client) -> None:
    response = await client.get(f"{url}/controlled")
    assert response.status == 200, response.text()
    assert response.headers["content-type"] == "text/plain"
    assert response.text() == ""

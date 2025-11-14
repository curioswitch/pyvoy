from __future__ import annotations

import subprocess
from typing import TYPE_CHECKING

import pytest
import pytest_asyncio

from pyvoy import PyvoyServer

from ._util import assert_logs_contains

if TYPE_CHECKING:
    from asyncio import StreamReader
    from collections.abc import AsyncIterator

    import httpx


@pytest_asyncio.fixture(scope="module")
async def server_asgi() -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.asgi.kitchensink",
        root_path="/root",
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        yield server


@pytest_asyncio.fixture(scope="module")
async def server_wsgi() -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.wsgi.kitchensink",
        interface="wsgi",
        root_path="/root",
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        yield server


@pytest_asyncio.fixture
async def url_asgi(server_asgi: PyvoyServer) -> AsyncIterator[str]:
    yield f"http://{server_asgi.listener_address}:{server_asgi.listener_port}"


@pytest_asyncio.fixture
async def url_wsgi(server_wsgi: PyvoyServer) -> AsyncIterator[str]:
    yield f"http://{server_wsgi.listener_address}:{server_wsgi.listener_port}"


@pytest.fixture
def logs_wsgi(server_wsgi: PyvoyServer) -> StreamReader:
    assert server_wsgi.stdout is not None
    return server_wsgi.stdout


@pytest.mark.asyncio
async def test_asgi_root_path(url_asgi: str, client: httpx.AsyncClient) -> None:
    response = await client.get(f"{url_asgi}/root/echo-scope")
    assert response.status_code == 200, response.text
    assert response.headers["x-scope-root-path"] == "/root"
    assert response.headers["x-scope-path"] == "/root/echo-scope"


# ASGI servers don't really validate or process root_path, it's just a hint to the app
# or middleware.
@pytest.mark.asyncio
async def test_asgi_root_path_not_match(
    url_asgi: str, client: httpx.AsyncClient
) -> None:
    response = await client.get(f"{url_asgi}/echo-scope")
    assert response.status_code == 200, response.text
    assert response.headers["x-scope-root-path"] == "/root"
    assert response.headers["x-scope-path"] == "/echo-scope"


@pytest.mark.asyncio
async def test_wsgi_root_path(url_wsgi: str, client: httpx.AsyncClient) -> None:
    response = await client.get(f"{url_wsgi}/root/echo-scope")
    assert response.status_code == 200, response.text
    assert response.headers["x-scope-root-path"] == "/root"
    assert response.headers["x-scope-path"] == "/echo-scope"


@pytest.mark.asyncio
async def test_wsgi_root_path_not_match(
    url_wsgi: str, client: httpx.AsyncClient, logs_wsgi: StreamReader
) -> None:
    response = await client.get(f"{url_wsgi}/echo-scope")
    assert response.status_code == 500, response.text
    await assert_logs_contains(
        logs_wsgi, ["Request path '/echo-scope' does not start with root path '/root'"]
    )

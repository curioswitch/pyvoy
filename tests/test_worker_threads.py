from __future__ import annotations

import asyncio
import subprocess
import sys
from typing import TYPE_CHECKING

import pytest
import pytest_asyncio

from pyvoy import Interface, PyvoyServer

if TYPE_CHECKING:
    from collections.abc import AsyncIterator

    import httpx


@pytest_asyncio.fixture(scope="module")
async def server_asgi() -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.asgi.kitchensink",
        worker_threads=8,
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        yield server


@pytest_asyncio.fixture(scope="module")
async def server_wsgi() -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.wsgi.kitchensink",
        interface="wsgi",
        worker_threads=8,
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
async def test_many_requests(url: str, client: httpx.AsyncClient) -> None:
    if not sys.version_info >= (3, 11):
        pytest.skip(
            "asyncio.TaskGroup requires Python 3.11+ and it's not important enough to make this test work on 3.10"
        )

    async def send_request() -> None:
        response = await client.get(
            f"{url}/headers-only",
            headers=(("Accept", "text/plain"), ("Multiple", "v1"), ("Multiple", "v2")),
        )
        assert response.status_code == 200, response.text
        assert response.headers["x-animal"] == "bear"
        assert response.headers["content-type"] == "text/plain"
        assert response.content == b""

    async with asyncio.TaskGroup() as tg:
        for _ in range(100):
            tg.create_task(send_request())

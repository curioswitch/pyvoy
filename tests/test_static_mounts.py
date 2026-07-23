from __future__ import annotations

import subprocess
from pathlib import Path
from typing import TYPE_CHECKING

import pytest
import pytest_asyncio

from pyvoy import PyvoyServer, StaticMount

if TYPE_CHECKING:
    from collections.abc import AsyncIterator

    from pyqwest import Client

STATIC_DIR = str(Path(__file__).parent / "apps" / "static")


@pytest_asyncio.fixture(scope="module")
async def server() -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.asgi.kitchensink",
        static_mounts=[
            StaticMount(
                path="/static", root=STATIC_DIR, cache_control="public, max-age=60"
            )
        ],
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        yield server


@pytest_asyncio.fixture
async def url(server: PyvoyServer) -> str:
    return f"http://{server.listener_address}:{server.listener_port}"


@pytest.mark.asyncio
async def test_static_file_served(url: str, client: Client) -> None:
    response = await client.get(f"{url}/static/app.js")
    assert response.status == 200, response.text()
    assert "javascript" in response.headers["content-type"]
    assert "pyvoy" in response.text()


@pytest.mark.asyncio
async def test_strip_prefix_default(url: str, client: Client) -> None:
    # Mounted at /static with the default strip_prefix, so /static/sub/nested.txt
    # resolves to <root>/sub/nested.txt.
    response = await client.get(f"{url}/static/sub/nested.txt")
    assert response.status == 200, response.text()
    assert response.text().strip() == "nested content"


@pytest.mark.asyncio
async def test_directory_index(url: str, client: Client) -> None:
    response = await client.get(f"{url}/static/")
    assert response.status == 200, response.text()
    assert "index" in response.text()


@pytest.mark.asyncio
async def test_cache_control_option(url: str, client: Client) -> None:
    response = await client.get(f"{url}/static/app.js")
    assert response.headers["cache-control"] == "public, max-age=60"


@pytest.mark.asyncio
async def test_app_serves_non_static(url: str, client: Client) -> None:
    # The application is the catch-all mount, so non-static paths reach it.
    response = await client.get(f"{url}/controlled")
    assert response.status == 200, response.text()


@pytest.mark.asyncio
async def test_directory_deny(client: Client) -> None:
    async with PyvoyServer(
        [],
        static_mounts=[StaticMount(path="/", root=STATIC_DIR, directory="deny")],
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        response = await client.get(
            f"http://{server.listener_address}:{server.listener_port}/"
        )
        assert response.status in (403, 404), response.text()
        # An explicit file is still served with directory listing denied.
        response = await client.get(
            f"http://{server.listener_address}:{server.listener_port}/app.js"
        )
        assert response.status == 200, response.text()


def test_websockets_static_only_rejected() -> None:
    server = PyvoyServer(
        [], static_mounts=[StaticMount("/", STATIC_DIR)], websockets=True
    )
    with pytest.raises(ValueError, match="websockets require an application mount"):
        server.get_envoy_config()


def test_no_mounts_rejected() -> None:
    server = PyvoyServer([])
    with pytest.raises(ValueError, match="at least one mount"):
        server.get_envoy_config()

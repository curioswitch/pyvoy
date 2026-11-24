from __future__ import annotations

import asyncio
import json
import subprocess
from typing import TYPE_CHECKING

import pytest
import pytest_asyncio
import websockets
from websockets.exceptions import InvalidStatus

from pyvoy import Interface, Mount, PyvoyServer

if TYPE_CHECKING:
    from collections.abc import AsyncIterator

    from pyqwest import Client

    from pyvoy import Loop


@pytest_asyncio.fixture(scope="module")
async def server(loop: Loop | None) -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        [
            Mount(app="tests.apps.asgi.kitchensink", path="/asgi", interface="asgi"),
            Mount(app="tests.apps.wsgi.kitchensink", path="/wsgi", interface="wsgi"),
        ],
        loop=loop,
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


def test_empty_mounts_rejected() -> None:
    server = PyvoyServer([])
    with pytest.raises(ValueError, match="at least one mount"):
        server.get_envoy_config()


def _find_named_filter(obj: object, name: str) -> dict | None:
    if isinstance(obj, dict):
        if obj.get("name") == name and "typed_config" in obj:
            return obj
        for value in obj.values():
            found = _find_named_filter(value, name)
            if found is not None:
                return found
    elif isinstance(obj, list):
        for item in obj:
            found = _find_named_filter(item, name)
            if found is not None:
                return found
    return None


def _pyvoy_filter_configs(obj: object) -> list[dict]:
    configs: list[dict] = []
    if isinstance(obj, dict):
        typed_config = obj.get("typed_config", {})
        if obj.get("name") == "pyvoy" and "filter_config" in typed_config:
            configs.append(json.loads(typed_config["filter_config"]["value"]))
        for value in obj.values():
            configs.extend(_pyvoy_filter_configs(value))
    elif isinstance(obj, list):
        for value in obj:
            configs.extend(_pyvoy_filter_configs(value))
    return configs


def test_websockets_enabled_for_asgi_mounts_only() -> None:
    server = PyvoyServer(
        [
            Mount(app="tests.apps.wsgi.kitchensink", path="/", interface="wsgi"),
            Mount(app="tests.apps.asgi.kitchensink", path="/asgi", interface="asgi"),
        ],
        websockets=True,
    )
    configs = {
        config["root_path"]: config
        for config in _pyvoy_filter_configs(server.get_envoy_config())
    }
    assert configs["/asgi"]["websockets"] is True
    assert "websockets" not in configs["/"]


def test_websockets_with_single_wsgi_app_rejected() -> None:
    server = PyvoyServer(
        "tests.apps.wsgi.kitchensink", interface="wsgi", websockets=True
    )

    with pytest.raises(
        ValueError, match="websockets require an application using the ASGI interface"
    ):
        server.get_envoy_config()


@pytest.mark.asyncio
async def test_websockets_routed_to_mount(loop: Loop | None) -> None:
    async with PyvoyServer(
        [
            Mount(app="tests.apps.wsgi.kitchensink", path="/", interface="wsgi"),
            Mount(
                app="tests.apps.websockets.kitchensink", path="/ws", interface="asgi"
            ),
        ],
        loop=loop,
        websockets=True,
        lifespan=False,
        stderr=subprocess.STDOUT,
        stdout=subprocess.DEVNULL,
    ) as server:
        url = f"ws://127.0.0.1:{server.listener_port}"
        async with websockets.connect(f"{url}/ws/echo", open_timeout=5) as ws:
            await ws.send("hello")
            assert await asyncio.wait_for(ws.recv(), timeout=5) == "hello"
        # The WSGI mount doesn't serve WebSockets, so the handshake fails.
        with pytest.raises(InvalidStatus):
            await websockets.connect(f"{url}/echo", open_timeout=5)

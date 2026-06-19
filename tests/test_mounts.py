from __future__ import annotations

import json
import subprocess
from typing import TYPE_CHECKING

import pytest
import pytest_asyncio

from pyvoy import Interface, Mount, PyvoyServer

if TYPE_CHECKING:
    from collections.abc import AsyncIterator

    from pyqwest import Client


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


def test_websockets_with_multiple_mounts_uses_first_app() -> None:
    # WebSockets are handled per-connection before HTTP path routing, so with
    # multiple mounts they bind to the first mount's app (not the last).
    server = PyvoyServer(
        [
            Mount(app="tests.apps.asgi.kitchensink", path="/a", interface="asgi"),
            Mount(app="tests.apps.wsgi.kitchensink", path="/b", interface="wsgi"),
        ],
        websockets=True,
    )
    config = server.get_envoy_config()
    ws_filter = _find_named_filter(config, "pyvoy-ws")
    assert ws_filter is not None
    ws_config = json.loads(ws_filter["typed_config"]["filter_config"]["value"])
    assert ws_config["app"] == "tests.apps.asgi.kitchensink"

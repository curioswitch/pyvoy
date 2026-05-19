from __future__ import annotations

from typing import TYPE_CHECKING

import pytest
import pytest_asyncio

from pyvoy import PyvoyServer

if TYPE_CHECKING:
    from collections.abc import AsyncIterator


@pytest_asyncio.fixture(scope="module")
async def backend_asgi() -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer("tests.apps.asgi.httpclient.kitchensink") as server:
        yield server


@pytest_asyncio.fixture(scope="module")
async def runner_asgi(backend_asgi: PyvoyServer) -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.asgi.httpclient.runner",
        env={
            "TEST_URL": f"http://{backend_asgi.listener_address}:{backend_asgi.listener_port}"
        },
        clusters=[
            {
                "name": "backend",
                "type": "STATIC",
                "connect_timeout": "5s",
                "lb_policy": "ROUND_ROBIN",
                "load_assignment": {
                    "cluster_name": "backend",
                    "endpoints": [
                        {
                            "lb_endpoints": [
                                {
                                    "endpoint": {
                                        "address": {
                                            "socket_address": {
                                                "address": backend_asgi.listener_address,
                                                "port_value": backend_asgi.listener_port,
                                            }
                                        }
                                    }
                                }
                            ]
                        }
                    ],
                },
            }
        ],
    ) as server:
        yield server


@pytest_asyncio.fixture
async def url_asgi(runner_asgi: PyvoyServer) -> AsyncIterator[str]:
    yield f"http://{runner_asgi.listener_address}:{runner_asgi.listener_port}"


@pytest.fixture
def url(url_asgi: str) -> str:
    return url_asgi

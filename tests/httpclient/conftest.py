from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING

import pytest
import pytest_asyncio
import trustme

from pyvoy import Cluster, HTTPVersion, PyvoyServer

if TYPE_CHECKING:
    from collections.abc import AsyncIterator


@dataclass
class Certs:
    ca: bytes
    server_cert: bytes
    server_key: bytes


@pytest.fixture(scope="module")
def ca() -> trustme.CA:
    return trustme.CA()


@pytest.fixture(scope="module")
def certs(ca: trustme.CA) -> Certs:
    server = ca.issue_cert("localhost")
    return Certs(
        ca=ca.cert_pem.bytes(),
        server_cert=server.cert_chain_pems[0].bytes(),
        server_key=server.private_key_pem.bytes(),
    )


@pytest_asyncio.fixture(scope="module")
async def backend_asgi() -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.asgi.httpclient.kitchensink", lifespan=False
    ) as server:
        yield server


@pytest_asyncio.fixture(scope="module")
async def runner_asgi(backend_asgi: PyvoyServer) -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.asgi.httpclient.runner",
        lifespan=False,
        env={
            "TEST_URL": f"http://{backend_asgi.listener_address}:{backend_asgi.listener_port}"
        },
        clusters=[
            Cluster(
                name="backend_h1c",
                address=f"{backend_asgi.listener_address}:{backend_asgi.listener_port}",
                http_version=HTTPVersion.HTTP1,
            ),
            Cluster(
                name="backend_h2c",
                address=f"localhost:{backend_asgi.listener_port}",
                http_version=HTTPVersion.HTTP2,
            ),
        ],
    ) as server:
        yield server


@pytest_asyncio.fixture
async def url_asgi(runner_asgi: PyvoyServer) -> AsyncIterator[str]:
    yield f"http://{runner_asgi.listener_address}:{runner_asgi.listener_port}"


@pytest.fixture
def url(url_asgi: str) -> str:
    return url_asgi

from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING

import pytest
import pytest_asyncio
import trustme

from pyvoy import Cluster, HTTPVersion, PyvoyServer, TLSConfig

if TYPE_CHECKING:
    from collections.abc import AsyncIterator


@dataclass
class Certs:
    ca: bytes
    cert: bytes
    key: bytes


@pytest.fixture(scope="module")
def ca() -> trustme.CA:
    return trustme.CA()


@pytest_asyncio.fixture(scope="module")
async def backend_asgi(ca: trustme.CA) -> AsyncIterator[PyvoyServer]:
    cert = ca.issue_cert("localhost")
    async with PyvoyServer(
        "tests.apps.asgi.httpclient.kitchensink",
        lifespan=False,
        tls_port=0,
        tls_key=cert.private_key_pem.bytes(),
        tls_cert=cert.cert_chain_pems[0].bytes(),
        tls_ca_cert=ca.cert_pem.bytes(),
    ) as server:
        yield server


@pytest_asyncio.fixture(scope="module")
async def runner_asgi(
    backend_asgi: PyvoyServer, ca: trustme.CA
) -> AsyncIterator[PyvoyServer]:
    cert = ca.issue_cert(
        common_name="someclient",
        organization_name="curioswitch",
        organization_unit_name="tests",
    )
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
                name="backend_h1",
                address=f"{backend_asgi.listener_address}:{backend_asgi.listener_port_tls}",
                http_version=HTTPVersion.HTTP1,
                tls=TLSConfig(
                    key=cert.private_key_pem.bytes(),
                    cert=cert.cert_chain_pems[0].bytes(),
                    ca_cert=ca.cert_pem.bytes(),
                ),
            ),
            Cluster(
                name="backend_h2c",
                address=f"localhost:{backend_asgi.listener_port}",
                http_version=HTTPVersion.HTTP2,
            ),
            Cluster(
                name="backend_h2",
                address=f"{backend_asgi.listener_address}:{backend_asgi.listener_port_tls}",
                http_version=HTTPVersion.HTTP2,
                tls=TLSConfig(
                    key=cert.private_key_pem.bytes(),
                    cert=cert.cert_chain_pems[0].bytes(),
                    ca_cert=ca.cert_pem.bytes(),
                ),
            ),
            Cluster(
                name="backend_unavailable",
                address="localhost:9999",
                http_version=HTTPVersion.HTTP1,
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


@pytest.fixture(params=["http", "https"])
def http_scheme(request: pytest.FixtureRequest) -> str:
    return request.param


@pytest.fixture(params=["h1", "h2"])
def http_version(request: pytest.FixtureRequest) -> str:
    return request.param

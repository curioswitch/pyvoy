from __future__ import annotations

import ssl
import subprocess
from dataclasses import dataclass
from typing import TYPE_CHECKING

import pytest
import pytest_asyncio
import trustme
from pyqwest import Client, HTTPTransport, HTTPVersion

from pyvoy import Interface, PyvoyServer

if TYPE_CHECKING:
    from collections.abc import AsyncIterator


@dataclass
class Certs:
    ca: bytes
    server_cert: bytes
    server_key: bytes


@pytest.fixture(scope="module")
def certs() -> Certs:
    ca = trustme.CA()
    server = ca.issue_cert("localhost")
    return Certs(
        ca=ca.cert_pem.bytes(),
        server_cert=server.cert_chain_pems[0].bytes(),
        server_key=server.private_key_pem.bytes(),
    )


@pytest_asyncio.fixture(scope="module")
async def server_asgi(certs: Certs) -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.asgi.kitchensink",
        port=0,
        tls_port=0,
        tls_key=certs.server_key,
        tls_cert=certs.server_cert,
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        yield server


@pytest_asyncio.fixture(scope="module")
async def server_wsgi(certs: Certs) -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.wsgi.kitchensink",
        interface="wsgi",
        port=0,
        tls_port=0,
        tls_key=certs.server_key,
        tls_cert=certs.server_cert,
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
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


@pytest.fixture(params=[HTTPVersion.HTTP1, HTTPVersion.HTTP2, HTTPVersion.HTTP3])
def http_version(request: pytest.FixtureRequest) -> HTTPVersion:
    return request.param


@pytest_asyncio.fixture
async def url_asgi(
    server_asgi: PyvoyServer, http_version: HTTPVersion
) -> AsyncIterator[str]:
    yield f"https://localhost:{server_asgi.listener_port_tls if http_version != HTTPVersion.HTTP3 else server_asgi.listener_port_quic}"


@pytest_asyncio.fixture
async def url_wsgi(
    server_wsgi: PyvoyServer, http_version: HTTPVersion
) -> AsyncIterator[str]:
    yield f"https://localhost:{server_wsgi.listener_port_tls if http_version != HTTPVersion.HTTP3 else server_wsgi.listener_port_quic}"


@pytest_asyncio.fixture
async def client(certs: Certs, http_version: HTTPVersion) -> AsyncIterator[Client]:
    ssl_ctx = ssl.create_default_context()
    ssl_ctx.load_verify_locations(cadata=certs.ca.decode())
    async with HTTPTransport(
        tls_ca_cert=certs.ca, http_version=http_version
    ) as transport:
        yield Client(transport)


@pytest.mark.asyncio
async def test_scope_content(
    url: str, client: Client, http_version: HTTPVersion, interface: Interface
) -> None:
    response = await client.get(f"{url}/echo-scope")
    assert response.status == 200, response.text()
    assert response.headers.get("x-scope-tls-version") == str(0x0304)
    assert response.headers.get("x-scope-tls-client-cert-name") == ""
    match http_version, interface:
        case HTTPVersion.HTTP3, "asgi":
            expected = "3"
        case HTTPVersion.HTTP2, "asgi":
            expected = "2"
        case HTTPVersion.HTTP1, "asgi":
            expected = "1.1"
        case HTTPVersion.HTTP3, "wsgi":
            expected = "HTTP/3"
        case HTTPVersion.HTTP2, "wsgi":
            expected = "HTTP/2"
        case HTTPVersion.HTTP1, "wsgi":
            expected = "HTTP/1.1"
    assert response.headers["x-scope-http-version"] == expected

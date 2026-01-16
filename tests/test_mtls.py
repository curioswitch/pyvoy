from __future__ import annotations

import ssl
import subprocess
from dataclasses import dataclass
from typing import TYPE_CHECKING

import pytest
import pytest_asyncio
import trustme
from pyqwest import Client, HTTPTransport
from trustme import LeafCert

from pyvoy import Interface, PyvoyServer

if TYPE_CHECKING:
    from collections.abc import AsyncIterator


@dataclass
class Certs:
    ca: bytes
    server_cert: bytes
    server_key: bytes
    client: LeafCert


@pytest.fixture(scope="module")
def certs() -> Certs:
    ca = trustme.CA()
    server = ca.issue_cert("localhost")
    client = ca.issue_cert(
        common_name="someclient",
        organization_name="pyvoy",
        organization_unit_name="tests",
    )
    return Certs(
        ca=ca.cert_pem.bytes(),
        server_cert=server.cert_chain_pems[0].bytes(),
        server_key=server.private_key_pem.bytes(),
        client=client,
    )


@pytest_asyncio.fixture(scope="module")
async def server_asgi(certs: Certs) -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.asgi.kitchensink",
        tls_key=certs.server_key,
        tls_cert=certs.server_cert,
        tls_ca_cert=certs.ca,
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        yield server


@pytest_asyncio.fixture(scope="module")
async def server_wsgi(certs: Certs) -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.wsgi.kitchensink",
        interface="wsgi",
        tls_key=certs.server_key,
        tls_cert=certs.server_cert,
        tls_ca_cert=certs.ca,
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


@pytest_asyncio.fixture
async def url_asgi(server_asgi: PyvoyServer) -> AsyncIterator[str]:
    yield f"https://localhost:{server_asgi.listener_port}"


@pytest_asyncio.fixture
async def url_wsgi(server_wsgi: PyvoyServer) -> AsyncIterator[str]:
    yield f"https://localhost:{server_wsgi.listener_port}"


@pytest_asyncio.fixture
async def client(certs: Certs) -> AsyncIterator[Client]:
    ssl_ctx = ssl.create_default_context()
    ssl_ctx.load_verify_locations(cadata=certs.ca.decode())
    certs.client.configure_cert(ssl_ctx)
    async with HTTPTransport(
        tls_ca_cert=certs.ca,
        tls_cert=certs.client.cert_chain_pems[0].bytes(),
        tls_key=certs.client.private_key_pem.bytes(),
    ) as transport:
        yield Client(transport)


@pytest.mark.asyncio
async def test_scope_content(url: str, client: Client) -> None:
    response = await client.get(f"{url}/echo-scope")
    assert response.status == 200, response.text()
    assert response.headers.get("x-scope-tls-version") == str(0x0304)
    assert (
        response.headers.get("x-scope-tls-client-cert-name")
        == "CN=someclient,OU=tests,O=pyvoy"
    )

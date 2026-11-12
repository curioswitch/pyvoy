from __future__ import annotations

import subprocess
from dataclasses import dataclass
from typing import TYPE_CHECKING

import pytest
import pytest_asyncio
import trustme
import websockets
from pyqwest import Client, HTTPTransport, HTTPVersion

from pyvoy import Interface, PyvoyServer

if TYPE_CHECKING:
    from collections.abc import AsyncIterator


# Tests connect from 127.0.0.1, trusted unless otherwise configured.
FORWARDED_HEADERS = {
    "x-forwarded-for": "9.9.9.9, 1.2.3.4, 10.0.0.1",
    "x-forwarded-proto": "https",
}


@dataclass(frozen=True)
class Case:
    forwarded_allow_ips: list[str] | None
    # None means the peer's address and port.
    client: str | None
    scheme: str


CASES = {
    # By default, only loopback is trusted.
    "default": Case(None, "10.0.0.1", "https"),
    "none": Case([], None, "http"),
    "all": Case(["*"], "10.0.0.1", "https"),
    # 10.0.0.1 is a trusted proxy so the client is before it.
    "cidrs": Case(["127.0.0.1", "10.0.0.0/8"], "1.2.3.4", "https"),
    "untrusted": Case(["10.0.0.0/8"], None, "http"),
}


@pytest.fixture(scope="module", params=list(CASES))
def case(request: pytest.FixtureRequest) -> Case:
    return CASES[request.param]


@pytest.fixture(scope="module", params=["asgi", "wsgi"])
def interface(request: pytest.FixtureRequest) -> Interface:
    return request.param


@pytest_asyncio.fixture(scope="module")
async def server(case: Case, interface: Interface) -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        f"tests.apps.{interface}.kitchensink",
        interface=interface,
        forwarded_allow_ips=case.forwarded_allow_ips,
        lifespan=False,
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        yield server


@pytest.mark.parametrize(
    "http_version", [HTTPVersion.HTTP1, HTTPVersion.HTTP2], ids=["h1", "h2"]
)
@pytest.mark.asyncio
async def test_forwarded_headers(
    server: PyvoyServer, case: Case, http_version: HTTPVersion
) -> None:
    async with HTTPTransport(http_version=http_version) as transport:
        response = await Client(transport).get(
            f"http://127.0.0.1:{server.listener_port}/echo-scope",
            headers=FORWARDED_HEADERS,
        )
    assert response.status == 200, response.text()
    assert response.headers["x-scope-scheme"] == case.scheme
    if case.client is None:
        assert response.headers["x-scope-client-address"] == "127.0.0.1"
        assert int(response.headers["x-scope-client-port"]) > 0
    else:
        assert response.headers["x-scope-client-address"] == case.client
        assert response.headers["x-scope-client-port"] == "0"


@pytest.mark.parametrize(
    "http_version", [HTTPVersion.HTTP1, HTTPVersion.HTTP2], ids=["h1", "h2"]
)
@pytest.mark.asyncio
async def test_no_forwarded_headers(
    server: PyvoyServer, http_version: HTTPVersion
) -> None:
    async with HTTPTransport(http_version=http_version) as transport:
        response = await Client(transport).get(
            f"http://127.0.0.1:{server.listener_port}/echo-scope"
        )
    assert response.status == 200, response.text()
    assert response.headers["x-scope-scheme"] == "http"
    assert response.headers["x-scope-client-address"] == "127.0.0.1"
    assert int(response.headers["x-scope-client-port"]) > 0


@pytest.mark.asyncio
async def test_invalid_forwarded_proto(server: PyvoyServer) -> None:
    async with HTTPTransport() as transport:
        response = await Client(transport).get(
            f"http://127.0.0.1:{server.listener_port}/echo-scope",
            headers={"x-forwarded-proto": "https, http"},
        )
    assert response.status == 200, response.text()
    assert response.headers["x-scope-scheme"] == "http"


@pytest_asyncio.fixture(scope="module")
async def ws_server(case: Case) -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.websockets.kitchensink",
        websockets=True,
        forwarded_allow_ips=case.forwarded_allow_ips,
        lifespan=False,
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        yield server


@pytest.mark.asyncio
async def test_websocket_forwarded_headers(ws_server: PyvoyServer, case: Case) -> None:
    url = f"ws://127.0.0.1:{ws_server.listener_port}"
    async with websockets.connect(
        f"{url}/peer", additional_headers=FORWARDED_HEADERS
    ) as ws:
        peer = await ws.recv(decode=True)
    client_host, client_port = peer.split("|")[0].rsplit(":", 1)
    async with websockets.connect(
        f"{url}/scheme", additional_headers=FORWARDED_HEADERS
    ) as ws:
        scheme = await ws.recv()

    assert scheme == ("wss" if case.scheme == "https" else "ws")
    if case.client is None:
        assert client_host == "127.0.0.1"
        assert int(client_port) > 0
    else:
        assert client_host == case.client
        assert client_port == "0"


@pytest.mark.asyncio
async def test_untrusted_tls_scheme() -> None:
    ca = trustme.CA()
    server_cert = ca.issue_cert("localhost")
    async with PyvoyServer(
        "tests.apps.asgi.kitchensink",
        tls_key=server_cert.private_key_pem.bytes(),
        tls_cert=server_cert.cert_chain_pems[0].bytes(),
        forwarded_allow_ips=[],
        lifespan=False,
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        assert server.listener_port_quic is not None
        for http_version, port in (
            (HTTPVersion.HTTP1, server.listener_port),
            (HTTPVersion.HTTP2, server.listener_port),
            (HTTPVersion.HTTP3, server.listener_port_quic),
        ):
            async with HTTPTransport(
                tls_ca_cert=ca.cert_pem.bytes(), http_version=http_version
            ) as transport:
                response = await Client(transport).get(
                    f"https://localhost:{port}/echo-scope",
                    headers={"x-forwarded-proto": "http"},
                )
            assert response.status == 200, response.text()
            assert response.headers["x-scope-scheme"] == "https", http_version


def test_forwarded_allow_ips_normalized() -> None:
    server = PyvoyServer(
        "tests.apps.asgi.kitchensink", forwarded_allow_ips=["10.1.2.3/8", "::1", "*"]
    )
    assert server._forwarded_allow_ips == ["10.0.0.0/8", "::1/128", "*"]


def test_forwarded_allow_ips_invalid() -> None:
    with pytest.raises(ValueError, match="got 'localhost'"):
        PyvoyServer("tests.apps.asgi.kitchensink", forwarded_allow_ips=["localhost"])
    with pytest.raises(TypeError, match="not a string"):
        PyvoyServer("tests.apps.asgi.kitchensink", forwarded_allow_ips="127.0.0.1")

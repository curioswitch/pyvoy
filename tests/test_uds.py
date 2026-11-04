from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path
from typing import TYPE_CHECKING

import pytest
import pytest_asyncio
import trustme
from pyqwest import Client, HTTPTransport, HTTPVersion

from pyvoy import Interface, PyvoyServer

if TYPE_CHECKING:
    from collections.abc import AsyncIterator, Iterator

    from pyvoy import Loop


pytestmark = pytest.mark.skipif(
    sys.platform == "win32", reason="pyqwest only supports Unix sockets on Unix"
)


@pytest.fixture(scope="module")
def socket_dir() -> Iterator[Path]:
    # pytest's tmp_path can exceed the socket path limit of macOS.
    with tempfile.TemporaryDirectory() as tmp_dir:
        yield Path(tmp_dir)


@pytest_asyncio.fixture(scope="module")
async def server_asgi(
    socket_dir: Path, loop: Loop | None
) -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.asgi.kitchensink",
        loop=loop,
        uds=socket_dir / "asgi.sock",
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        yield server


@pytest_asyncio.fixture(scope="module")
async def server_wsgi(socket_dir: Path) -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.wsgi.kitchensink",
        interface="wsgi",
        uds=socket_dir / "wsgi.sock",
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        yield server


@pytest.fixture(params=["asgi", "wsgi"])
def interface(request: pytest.FixtureRequest) -> Interface:
    return request.param


@pytest.fixture
def server(
    interface: Interface, server_asgi: PyvoyServer, server_wsgi: PyvoyServer
) -> PyvoyServer:
    match interface:
        case "asgi":
            return server_asgi
        case "wsgi":
            return server_wsgi


def test_listener(server: PyvoyServer, interface: Interface) -> None:
    assert server.listener_address.endswith(f"{interface}.sock")
    assert server.listener_port == 0
    assert server.listener_port_tls is None
    assert server.listener_port_quic is None
    assert Path(server.listener_address).stat().st_mode & 0o777 == 0o666


@pytest.mark.parametrize(
    "http_version", [HTTPVersion.HTTP1, HTTPVersion.HTTP2], ids=["h1", "h2"]
)
@pytest.mark.asyncio
async def test_scope_content(
    server: PyvoyServer, interface: Interface, http_version: HTTPVersion
) -> None:
    async with HTTPTransport(
        uds=server.listener_address, http_version=http_version
    ) as transport:
        response = await Client(transport).get("http://localhost/echo-scope")
    assert response.status == 200, response.text()
    match interface:
        case "asgi":
            assert response.headers["x-scope-server-address"] == (
                server.listener_address
            )
            assert response.headers["x-scope-server-port"] == "None"
            expected_http_version = "2" if http_version == HTTPVersion.HTTP2 else "1.1"
        case "wsgi":
            assert response.headers["x-scope-server-address"] == "localhost"
            assert response.headers["x-scope-server-port"] == "80"
            expected_http_version = (
                "HTTP/2" if http_version == HTTPVersion.HTTP2 else "HTTP/1.1"
            )
    assert response.headers["x-scope-http-version"] == expected_http_version


@pytest.mark.asyncio
async def test_wsgi_server_from_host(server_wsgi: PyvoyServer) -> None:
    async with HTTPTransport(uds=server_wsgi.listener_address) as transport:
        response = await Client(transport).get("http://example.com:8080/echo-scope")
    assert response.status == 200, response.text()
    assert response.headers["x-scope-server-address"] == "example.com"
    assert response.headers["x-scope-server-port"] == "8080"


@pytest.mark.parametrize("interface", ["asgi", "wsgi"])
@pytest.mark.asyncio
async def test_tls(socket_dir: Path, interface: Interface) -> None:
    ca = trustme.CA()
    cert = ca.issue_cert("localhost")
    async with (
        PyvoyServer(
            f"tests.apps.{interface}.kitchensink",
            interface=interface,
            uds=socket_dir / f"tls-{interface}.sock",
            tls_key=cert.private_key_pem.bytes(),
            tls_cert=cert.cert_chain_pems[0].bytes(),
            stderr=subprocess.STDOUT,
            stdout=subprocess.PIPE,
        ) as server,
        HTTPTransport(
            uds=server.listener_address, tls_ca_cert=ca.cert_pem.bytes()
        ) as transport,
    ):
        assert server.listener_port_tls is None
        assert server.listener_port_quic is None
        response = await Client(transport).get("https://localhost/echo-scope")
    assert response.status == 200, response.text()
    assert response.headers["x-scope-scheme"] == "https"
    assert response.headers["x-scope-tls-version"] == str(0x0304)
    if interface == "wsgi":
        assert response.headers["x-scope-server-port"] == "443"

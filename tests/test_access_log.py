from __future__ import annotations

import asyncio
import json
import re
import subprocess
import urllib.request
from typing import TYPE_CHECKING

import pytest
import pytest_asyncio
from pyqwest import Client, HTTPTransport, HTTPVersion

from pyvoy import Interface, PyvoyServer

if TYPE_CHECKING:
    from collections.abc import AsyncIterator


@pytest.fixture(scope="module", params=["asgi", "wsgi"])
def interface(request: pytest.FixtureRequest) -> Interface:
    return request.param


@pytest_asyncio.fixture(scope="module")
async def server(interface: Interface) -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        f"tests.apps.{interface}.kitchensink",
        interface=interface,
        access_log=True,
        lifespan=False,
        stdout=subprocess.PIPE,
    ) as server:
        yield server


async def _read_access_log(server: PyvoyServer, path: str) -> str:
    """Returns the access log line for the path, skipping other requests."""
    assert server.stdout is not None
    while True:
        # Envoy's default flush interval of 10s would time out.
        line = (await asyncio.wait_for(server.stdout.readline(), 5)).decode()
        if f" {path} " in line:
            return line


@pytest.mark.parametrize(
    ("http_version", "protocol"),
    [(HTTPVersion.HTTP1, "HTTP/1.1"), (HTTPVersion.HTTP2, "HTTP/2")],
    ids=["h1", "h2"],
)
@pytest.mark.asyncio
async def test_access_log(
    server: PyvoyServer, http_version: HTTPVersion, protocol: str
) -> None:
    async with HTTPTransport(http_version=http_version) as transport:
        client = Client(transport)
        # Envoy flushes the first log line immediately, so the interesting one
        # is the second.
        await client.get(f"http://127.0.0.1:{server.listener_port}/headers-only")
        path = f"/response-body?version={protocol}"
        response = await client.get(
            f"http://127.0.0.1:{server.listener_port}{path}",
            headers={
                "user-agent": "pyvoy-test/1.0",
                "referer": "https://example.com/",
                # 127.0.0.1 is trusted by default.
                "x-forwarded-for": "9.9.9.9",
            },
        )
    assert response.status == 200, response.text()

    line = await _read_access_log(server, path)
    assert re.fullmatch(
        rf'9\.9\.9\.9 - - \[\d{{2}}/\w{{3}}/\d{{4}}:\d{{2}}:\d{{2}}:\d{{2}} \+0000\] "GET {re.escape(path)} {protocol}" 200 12 "https://example\.com/" "pyvoy-test/1\.0"\n',
        line,
    ), line


@pytest.mark.asyncio
async def test_access_log_missing_headers(server: PyvoyServer) -> None:
    path = "/response-body?missing-headers"
    async with HTTPTransport(http_version=HTTPVersion.HTTP1) as transport:
        response = await Client(transport).get(
            f"http://127.0.0.1:{server.listener_port}{path}"
        )
    assert response.status == 200, response.text()

    line = await _read_access_log(server, path)
    assert line.startswith("127.0.0.1 - - [")
    assert line.endswith(f'"GET {path} HTTP/1.1" 200 12 "-" "-"\n')


@pytest.mark.parametrize(
    ("additional_envoy_args", "expected"),
    [(None, "0.100s"), (["--file-flush-interval-msec", "2000"], "2s")],
)
@pytest.mark.asyncio
async def test_access_log_flush_interval(
    additional_envoy_args: list[str] | None, expected: str
) -> None:
    async with PyvoyServer(
        "tests.apps.asgi.kitchensink",
        access_log=True,
        lifespan=False,
        additional_envoy_args=additional_envoy_args,
    ) as server:
        response = await asyncio.to_thread(
            urllib.request.urlopen, f"http://{server._admin_address}/server_info"
        )
        server_info = json.loads(response.read())
    assert server_info["command_line_options"]["file_flush_interval"] == expected


def _connection_managers(config: dict) -> list[dict]:
    return [
        network_filter["typed_config"]
        for listener in config["static_resources"]["listeners"]
        for filter_chain in listener["filter_chains"]
        for network_filter in filter_chain["filters"]
        if network_filter["name"] == "envoy.filters.network.http_connection_manager"
    ]


@pytest.mark.parametrize("access_log", [True, False])
def test_access_log_config(*, access_log: bool) -> None:
    server = PyvoyServer(
        "tests.apps.asgi.kitchensink",
        access_log=access_log,
        tls_port=0,
        tls_key=b"key",
        tls_cert=b"cert",
    )

    connection_managers = _connection_managers(server.get_envoy_config())
    # Plaintext, TLS, and HTTP/3.
    assert len(connection_managers) == 3
    for connection_manager in connection_managers:
        assert ("access_log" in connection_manager) == access_log

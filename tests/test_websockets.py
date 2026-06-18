from __future__ import annotations

import asyncio
import json
import os
import subprocess
from pathlib import Path
from tempfile import TemporaryDirectory
from time import perf_counter_ns
from typing import TYPE_CHECKING

import pytest
import pytest_asyncio
import websockets
from websockets.exceptions import ConnectionClosed, InvalidHandshake, InvalidStatus

from pyvoy import PyvoyServer

from ._util import assert_logs_contains

if TYPE_CHECKING:
    from collections.abc import AsyncIterator


def _is_docker_unavailable() -> bool:
    # Check docker CLI is available and supports Linux containers
    try:
        result = subprocess.run(
            ["docker", "info", "--format", "{{.OSType}}"],
            capture_output=True,
            text=True,
            check=True,
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        return True
    else:
        return result.stdout.strip() != "linux"


@pytest_asyncio.fixture(scope="module")
async def echo_server() -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.websockets.echo",
        # Bind all interfaces for access from Docker
        address="0.0.0.0",  # noqa: S104
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
        websockets=True,
        lifespan=False,
    ) as server:
        yield server


# Higher compression cases deal with huge payloads that are very slow with autobahn's
# Docker runner. We keep them opt-in only for now.
cases = [
    pytest.param(
        [
            "1.*",
            "2.*",
            "3.*",
            "4.*",
            "5.*",
            "6.*",
            "7.*",
            "9.*",
            "10.*",
            "12.1.11",
            "12.2.1",
            "13.1.1",
            "13.2.1",
            "13.3.1",
            "13.4.1",
            "13.5.1",
            "13.6.1",
            "13.7.1",
        ],
        id="fast",
    ),
    pytest.param(["12.*", "13.*"], id="slow", marks=pytest.mark.slow),
]


@pytest.mark.skipif(
    _is_docker_unavailable(), reason="requires Docker with Linux containers"
)
@pytest.mark.parametrize("cases", cases)
def test_autobahn(cases: list[str], echo_server: PyvoyServer) -> None:
    config = {
        "servers": [{"url": f"ws://host.docker.internal:{echo_server.listener_port}"}],
        "outdir": "/reports",
        "cases": cases,
        "exclude-cases": [],
    }

    # Use repository root for temp directory for colima compatibility
    temp_root = Path(__file__).parent.parent / "out"
    temp_root.mkdir(exist_ok=True)
    with TemporaryDirectory(dir=temp_root) as tempdir:
        config_dir = Path(tempdir) / "config"
        config_dir.mkdir(exist_ok=True)
        config_path = config_dir / "config.json"
        config_path.write_text(json.dumps(config))

        reports_dir = Path(tempdir) / "reports"
        reports_dir.mkdir(exist_ok=True)

        subprocess.run(
            [
                "docker",
                "run",
                # Needed for Linux
                "--add-host",
                "host.docker.internal:host-gateway",
                "--env",
                "PYTHONUNBUFFERED=1",
                "-v",
                f"{config_dir}:/config",
                "-v",
                f"{reports_dir}:/reports",
                "crossbario/autobahn-testsuite:25.10.1",
                "wstest",
                "-m",
                "fuzzingclient",
                "--spec",
                "/config/config.json",
            ],
            check=True,
            stdout=subprocess.DEVNULL,
        )

        report = json.loads((reports_dir / "index.json").read_text())
        # wstest always exits 0 — even when it cannot reach the server
        ran = sum(len(results) for results in report.values())
        assert ran > 0, "autobahn ran no cases; server unreachable from the container?"
        for _, results in report.items():
            for case, result in results.items():
                assert result["behavior"] in ("OK", "INFORMATIONAL", "NON-STRICT"), (
                    f"{case} failed with behavior {result['behavior']}"
                )


@pytest_asyncio.fixture(scope="module")
async def server() -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.websockets.kitchensink",
        websockets=True,
        lifespan=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.STDOUT,
    ) as server:
        yield server


def _url(server: PyvoyServer, path: str) -> str:
    return f"ws://127.0.0.1:{server.listener_port}{path}"


def _close_code(exc: ConnectionClosed) -> int | None:
    # None when the peer closed without a close frame (abnormal closure).
    rcvd = getattr(exc, "rcvd", None)
    return rcvd.code if rcvd is not None else None


@pytest.mark.asyncio
async def test_echo_and_clean_close(server: PyvoyServer) -> None:
    async with websockets.connect(_url(server, "/echo")) as ws:
        await ws.send("hello")
        assert await ws.recv() == "hello"
        await ws.send(b"\x00\x01\x02")
        assert await ws.recv() == b"\x00\x01\x02"


@pytest.mark.asyncio
async def test_subprotocol(server: PyvoyServer) -> None:
    async with websockets.connect(
        _url(server, "/subprotocol"), subprotocols=["chat", "superchat"]
    ) as ws:
        assert ws.subprotocol == "chat"
        assert await ws.recv() == "chat,superchat"


@pytest.mark.asyncio
async def test_server_initiated_close(server: PyvoyServer) -> None:
    async with websockets.connect(_url(server, "/close")) as ws:
        with pytest.raises(ConnectionClosed) as ei:
            await ws.recv()
    assert _close_code(ei.value) == 1001


@pytest.mark.asyncio
async def test_reject_before_accept(server: PyvoyServer) -> None:
    with pytest.raises(InvalidStatus) as ei:
        await websockets.connect(_url(server, "/reject"))
    assert ei.value.response.status_code == 403


@pytest.mark.asyncio
async def test_app_error_before_accept(server: PyvoyServer) -> None:
    with pytest.raises(InvalidStatus) as ei:
        await websockets.connect(_url(server, "/raise-before"))
    assert ei.value.response.status_code == 500


@pytest.mark.asyncio
async def test_app_error_after_accept(server: PyvoyServer) -> None:
    async with websockets.connect(_url(server, "/raise-after")) as ws:
        with pytest.raises(ConnectionClosed) as ei:
            await ws.recv()
    assert _close_code(ei.value) == 1011


@pytest.mark.asyncio
async def test_bad_send_closes(server: PyvoyServer) -> None:
    async with websockets.connect(_url(server, "/bad-send")) as ws:
        with pytest.raises(ConnectionClosed):
            await ws.recv()


@pytest.mark.asyncio
async def test_double_accept_closes(server: PyvoyServer) -> None:
    async with websockets.connect(_url(server, "/double-accept")) as ws:
        with pytest.raises(ConnectionClosed):
            await ws.recv()


@pytest.mark.asyncio
async def test_close_without_code(server: PyvoyServer) -> None:
    async with websockets.connect(_url(server, "/close-nocode")) as ws:
        with pytest.raises(ConnectionClosed):
            await ws.recv()


@pytest.mark.asyncio
async def test_return_before_accept(server: PyvoyServer) -> None:
    # The app returns before accepting (and without closing); the server drops
    # the connection mid-handshake rather than completing the upgrade.
    with pytest.raises(InvalidHandshake):
        await asyncio.wait_for(
            websockets.connect(_url(server, "/return-before-accept")), timeout=5
        )


@pytest.mark.asyncio
async def test_return_without_close(server: PyvoyServer) -> None:
    # The app returns after accepting without sending a close.
    # We match uvicorn behavior, aborting the connection.
    async with websockets.connect(_url(server, "/return")) as ws:
        with pytest.raises(ConnectionClosed) as ei:
            await asyncio.wait_for(ws.recv(), timeout=5)
    assert _close_code(ei.value) != 1000


@pytest.mark.asyncio
async def test_send_after_close(server: PyvoyServer) -> None:
    # The app closes then sends again; the second send raises
    # ClientDisconnectedError server-side, which is swallowed cleanly.
    async with websockets.connect(_url(server, "/close-then-send")) as ws:
        with pytest.raises(ConnectionClosed) as ei:
            await ws.recv()
    assert _close_code(ei.value) == 1000


@pytest.mark.asyncio
async def test_unknown_event(server: PyvoyServer) -> None:
    async with websockets.connect(_url(server, "/bad-event")) as ws:
        with pytest.raises(ConnectionClosed):
            await ws.recv()


@pytest.mark.asyncio
async def test_recv_after_disconnect() -> None:
    # Start a new PyvoyServer since we need to check its logs for confirming
    # the app exited.
    async with PyvoyServer(
        "tests.apps.websockets.kitchensink",
        websockets=True,
        lifespan=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    ) as srv:
        async with websockets.connect(_url(srv, "/recv-after-disconnect")) as ws:
            await ws.send("hi")
        assert srv.stdout is not None
        await assert_logs_contains(
            srv.stdout, ["recv-after-disconnect: second=websocket.disconnect"]
        )


@pytest.mark.asyncio
async def test_response_backpressure(server: PyvoyServer) -> None:
    async with websockets.connect(
        _url(server, "/backpressure"), max_size=None, max_queue=1, compression=None
    ) as ws:
        # Pause reading messages to cause the server side to back up and pause.
        await asyncio.sleep(0.3)
        waits: list[int] | None = None
        while True:
            msg = await ws.recv()
            if isinstance(msg, str):
                waits = [int(w) for w in msg.split(",")]
                break
    assert waits is not None
    assert len(waits) == 100
    assert max(waits) >= 50_000_000


@pytest.mark.asyncio
async def test_request_backpressure(server: PyvoyServer) -> None:
    # The opposite of response backpressure: the app stalls before reading, so a
    # client flooding 1 MiB messages fills the buffer and the server applies
    # request backpressure -- the client's send() pauses (websockets pauses
    # rather than failing) until the app drains.
    waits: list[int] = []
    async with websockets.connect(
        _url(server, "/slow-recv"), max_size=None, compression=None
    ) as ws:
        for _ in range(40):
            chunk = os.urandom(1024 * 1024)
            start = perf_counter_ns()
            await ws.send(chunk)
            waits.append(perf_counter_ns() - start)
    assert max(waits) >= 100_000_000


@pytest.mark.asyncio
async def test_abrupt_drop(server: PyvoyServer) -> None:
    # Drop the TCP connection without a close handshake -> server delivers a
    # disconnect with code 1005 and the stream sees EOF.
    ws = await websockets.connect(_url(server, "/echo"))
    await ws.send("x")
    assert await ws.recv() == "x"
    ws.transport.close()

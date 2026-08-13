from __future__ import annotations

import asyncio
import json
import subprocess
import sys
from typing import TYPE_CHECKING, cast

import pytest

from pyvoy import PyvoyServer

if TYPE_CHECKING:
    from pyqwest import Client


@pytest.mark.asyncio
async def test_wait_returns_child_exit_status() -> None:
    class Process:
        async def wait(self) -> int:
            return 23

    server = PyvoyServer("tests.apps.asgi.kitchensink")
    server._process = cast("asyncio.subprocess.Process", Process())

    assert await server.wait() == 23


@pytest.mark.asyncio
async def test_wait_before_start_returns_none() -> None:
    server = PyvoyServer("tests.apps.asgi.kitchensink")

    assert await server.wait() is None


@pytest.mark.parametrize("worker_threads", [0, -1, sys.maxsize + 1])
def test_worker_threads_must_be_positive(worker_threads: int) -> None:
    with pytest.raises(ValueError, match="worker_threads must be between 1 and"):
        PyvoyServer("tests.apps.asgi.kitchensink", worker_threads=worker_threads)


@pytest.mark.parametrize("worker_threads", [True, 1.5])
def test_worker_threads_must_be_an_integer(worker_threads: object) -> None:
    with pytest.raises(TypeError, match="worker_threads must be an integer"):
        PyvoyServer(
            "tests.apps.asgi.kitchensink",
            worker_threads=worker_threads,  # pyright: ignore[reportArgumentType]
        )


@pytest.mark.parametrize("websockets_max_message_size", [-1, sys.maxsize + 1])
def test_websockets_max_message_size_must_be_in_range(
    websockets_max_message_size: int,
) -> None:
    with pytest.raises(
        ValueError, match="websockets_max_message_size must be between 0 and"
    ):
        PyvoyServer(
            "tests.apps.asgi.kitchensink",
            websockets_max_message_size=websockets_max_message_size,
        )


def test_websockets_max_message_size_allows_zero() -> None:
    server = PyvoyServer("tests.apps.asgi.kitchensink", websockets_max_message_size=0)

    config = server.get_envoy_config()
    http_filter = config["static_resources"]["listeners"][0]["filter_chains"][0][
        "filters"
    ][0]["typed_config"]["http_filters"][0]
    pyvoy_config = json.loads(http_filter["typed_config"]["filter_config"]["value"])
    assert pyvoy_config["websockets_max_message_size"] == 0


@pytest.mark.parametrize("websockets_max_message_size", [True, 1.5])
def test_websockets_max_message_size_must_be_an_integer(
    websockets_max_message_size: object,
) -> None:
    with pytest.raises(
        TypeError, match="websockets_max_message_size must be an integer"
    ):
        PyvoyServer(
            "tests.apps.asgi.kitchensink",
            websockets_max_message_size=websockets_max_message_size,  # pyright: ignore[reportArgumentType]
        )


@pytest.mark.asyncio
async def test_start_twice_rejected(client: Client) -> None:
    server = PyvoyServer(
        "tests.apps.asgi.kitchensink",
        lifespan=False,
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    )
    await server.start()
    process = server._process
    assert process is not None

    try:
        with pytest.raises(RuntimeError, match="already running or starting"):
            await server.start()
        assert server._process is process
        assert process.returncode is None

        with pytest.raises(RuntimeError, match="already running or starting"):
            async with server:
                pass
        assert server._process is process
        assert process.returncode is None

        response = await client.get(
            f"http://{server.listener_address}:{server.listener_port}/controlled"
        )
        assert response.status == 200, response.text()
    finally:
        await server.stop()

    assert process.returncode is not None
    assert await server.wait() == process.returncode


@pytest.mark.asyncio
async def test_concurrent_start_rejected(monkeypatch: pytest.MonkeyPatch) -> None:
    start_entered = asyncio.Event()
    allow_start = asyncio.Event()

    async def delayed_start(_server: PyvoyServer) -> None:
        start_entered.set()
        await allow_start.wait()

    monkeypatch.setattr(PyvoyServer, "_start", delayed_start)
    server = PyvoyServer("tests.apps.asgi.kitchensink")
    first_start = asyncio.create_task(server.start())
    await start_entered.wait()

    try:
        with pytest.raises(RuntimeError, match="already running or starting"):
            await server.start()
    finally:
        allow_start.set()
        await first_start

    assert not server._starting

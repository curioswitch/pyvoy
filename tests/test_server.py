from __future__ import annotations

import asyncio
import json
import os
import subprocess
import sys
import urllib.request
from pathlib import Path
from typing import TYPE_CHECKING, cast

import pytest

from pyvoy import PyvoyServer

if TYPE_CHECKING:
    from pyqwest import Client

    from pyvoy import Loop


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


@pytest.mark.parametrize("io_threads", [0, -1, sys.maxsize + 1])
def test_io_threads_must_be_positive(io_threads: int) -> None:
    with pytest.raises(ValueError, match="io_threads must be between 1 and"):
        PyvoyServer("tests.apps.asgi.kitchensink", io_threads=io_threads)


@pytest.mark.parametrize("io_threads", [True, 1.5, "4"])
def test_io_threads_must_be_an_integer(io_threads: object) -> None:
    with pytest.raises(TypeError, match="io_threads must be an integer or 'auto'"):
        PyvoyServer(
            "tests.apps.asgi.kitchensink",
            io_threads=io_threads,  # pyright: ignore[reportArgumentType]
        )


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("kwargs", "expected"),
    [({}, os.cpu_count()), ({"io_threads": 1}, 1), ({"io_threads": 2}, 2)],
)
async def test_io_threads(kwargs: dict[str, object], expected: int) -> None:
    async with PyvoyServer(
        "tests.apps.asgi.kitchensink",
        lifespan=False,
        **kwargs,  # pyright: ignore[reportArgumentType]
    ) as server:
        response = await asyncio.to_thread(
            urllib.request.urlopen, f"http://{server._admin_address}/server_info"
        )
        server_info = json.loads(response.read())
    assert server_info["command_line_options"]["concurrency"] == expected


@pytest.mark.asyncio
async def test_async_task_batch_size(client: Client, loop: Loop | None) -> None:
    # With one task per drain, concurrent requests that queue together are run
    # across several passes through the event loop.
    async with PyvoyServer(
        "tests.apps.asgi.kitchensink",
        lifespan=False,
        loop=loop,
        env={"PYVOY_ASYNC_TASK_BATCH_SIZE": "1"},
    ) as server:
        url = f"http://{server.listener_address}:{server.listener_port}/controlled"
        responses = await asyncio.gather(*(client.get(url) for _ in range(50)))
    assert [response.status for response in responses] == [200] * 50


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


def test_uds_rejects_tls_port() -> None:
    with pytest.raises(ValueError, match="tls_port cannot be used with uds"):
        PyvoyServer("tests.apps.asgi.kitchensink", uds="pyvoy.sock", tls_port=0)


def test_uds_listener_without_http3() -> None:
    server = PyvoyServer(
        "tests.apps.asgi.kitchensink",
        uds=Path("pyvoy.sock"),
        tls_key=b"key",
        tls_cert=b"cert",
    )

    listeners = server.get_envoy_config()["static_resources"]["listeners"]
    assert [listener["name"] for listener in listeners] == ["listener"]
    assert listeners[0]["address"] == {"pipe": {"path": "pyvoy.sock", "mode": 0o666}}
    assert "transport_socket" in listeners[0]["filter_chains"][0]


def _pyvoy_filter_configs(config: dict) -> dict[str, dict]:
    """Returns the pyvoy filter configs in an Envoy config by filter name."""
    filters: dict[str, dict] = {}

    def visit(value: object) -> None:
        if isinstance(value, dict):
            typed_config = value.get("typed_config")
            if value.get("name") in ("pyvoy", "pyvoy-ws") and isinstance(
                typed_config, dict
            ):
                filters[value["name"]] = json.loads(
                    typed_config["filter_config"]["value"]
                )
            for item in value.values():
                visit(item)
        elif isinstance(value, list):
            for item in value:
                visit(item)

    visit(config)
    return filters


def test_loop_unset_by_default() -> None:
    server = PyvoyServer("tests.apps.asgi.kitchensink", websockets=True)

    filters = _pyvoy_filter_configs(server.get_envoy_config())
    assert set(filters) == {"pyvoy", "pyvoy-ws"}
    for pyvoy_config in filters.values():
        assert "loop" not in pyvoy_config


def test_loop_configures_all_filters() -> None:
    server = PyvoyServer("tests.apps.asgi.kitchensink", websockets=True, loop="trio")

    filters = _pyvoy_filter_configs(server.get_envoy_config())
    assert set(filters) == {"pyvoy", "pyvoy-ws"}
    for pyvoy_config in filters.values():
        assert pyvoy_config["loop"] == "trio"


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

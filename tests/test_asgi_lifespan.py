from __future__ import annotations

import asyncio
import subprocess
from typing import TYPE_CHECKING

import pytest

from pyvoy import PyvoyServer

if TYPE_CHECKING:
    from pyqwest import Client

    from pyvoy import AsyncLibrary


async def _read_logs(stream: asyncio.StreamReader, logs: list[str]):
    async for line in stream:
        logs.append(line.decode())


@pytest.mark.asyncio
async def test_normal(client: Client, io: AsyncLibrary):
    logs: list[str] = []
    async with PyvoyServer(
        "tests.apps.asgi.lifespan:normal",
        io=io,
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        assert server.stdout is not None
        logs_task = asyncio.create_task(_read_logs(server.stdout, logs))

        url = f"http://{server.listener_address}:{server.listener_port}"

        for _ in range(5):
            response = await client.get(url)
            assert response.status == 200
            assert response.text() == "Ok"

    await logs_task
    assert any("Got counter: 5" in log_line for log_line in logs), (
        f"Logs: {''.join(logs)}"
    )


@pytest.mark.asyncio
async def test_state_isolation(client: Client, io: AsyncLibrary):
    async with PyvoyServer(
        "tests.apps.asgi.lifespan:state_isolation",
        io=io,
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        url = f"http://{server.listener_address}:{server.listener_port}"
        bodies = [(await client.get(url)).text() for _ in range(3)]

    assert bodies == ["1,1", "2,1", "3,1"], bodies


@pytest.mark.asyncio
async def test_normal_lifespan_disabled(client: Client, io: AsyncLibrary):
    logs: list[str] = []
    async with PyvoyServer(
        "tests.apps.asgi.lifespan:normal",
        io=io,
        lifespan=False,
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        assert server.stdout is not None
        logs_task = asyncio.create_task(_read_logs(server.stdout, logs))

        url = f"http://{server.listener_address}:{server.listener_port}"

        for _ in range(5):
            response = await client.get(url)
            assert response.status == 200
            assert response.text() == "Ok"

    await logs_task
    assert not any("Got counter: 5" in log_line for log_line in logs), (
        f"Logs: {''.join(logs)}"
    )


@pytest.mark.asyncio
async def test_startup_failed(io: AsyncLibrary):
    server = PyvoyServer(
        "tests.apps.asgi.lifespan:startup_failed",
        io=io,
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    )
    with pytest.raises(RuntimeError):
        await server.start()
    assert server.stdout is not None
    logs: list[str] = []
    logs_task = asyncio.create_task(_read_logs(server.stdout, logs))

    await logs_task
    assert any("I failed to startup" in log_line for log_line in logs), (
        f"Logs: {''.join(logs)}"
    )


@pytest.mark.asyncio
async def test_startup_failed_no_msg(io: AsyncLibrary):
    server = PyvoyServer(
        "tests.apps.asgi.lifespan:startup_failed_no_msg",
        io=io,
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    )
    with pytest.raises(RuntimeError):
        await server.start()
    assert server.stdout is not None
    logs: list[str] = []
    logs_task = asyncio.create_task(_read_logs(server.stdout, logs))

    await logs_task
    assert any("Application startup failed" in log_line for log_line in logs), (
        f"Logs: {''.join(logs)}"
    )


@pytest.mark.asyncio
async def test_shutdown_failed(client: Client, io: AsyncLibrary):
    logs: list[str] = []
    async with PyvoyServer(
        "tests.apps.asgi.lifespan:shutdown_failed",
        io=io,
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        assert server.stdout is not None
        logs_task = asyncio.create_task(_read_logs(server.stdout, logs))

        url = f"http://{server.listener_address}:{server.listener_port}"

        for _ in range(5):
            response = await client.get(url)
            assert response.status == 200
            assert response.text() == "Ok"

    await logs_task
    assert any("I failed to shutdown" in log_line for log_line in logs), (
        f"Logs: {''.join(logs)}"
    )


@pytest.mark.asyncio
async def test_shutdown_failed_no_msg(client: Client, io: AsyncLibrary):
    logs: list[str] = []
    async with PyvoyServer(
        "tests.apps.asgi.lifespan:shutdown_failed_no_msg",
        io=io,
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        assert server.stdout is not None
        logs_task = asyncio.create_task(_read_logs(server.stdout, logs))

        url = f"http://{server.listener_address}:{server.listener_port}"

        for _ in range(5):
            response = await client.get(url)
            assert response.status == 200
            assert response.text() == "Ok"

    await logs_task
    assert any("Application shutdown failed" in log_line for log_line in logs), (
        f"Logs: {''.join(logs)}"
    )


@pytest.mark.asyncio
async def test_return_without_events(client: Client, io: AsyncLibrary):
    async with PyvoyServer(
        "tests.apps.asgi.lifespan:return_without_events", io=io
    ) as server:
        url = f"http://{server.listener_address}:{server.listener_port}"

        # This isn't an error case so we don't have anything to assert other than standard
        # success.
        response = await client.get(url)
        assert response.status == 200
        assert response.text() == "Ok"


@pytest.mark.asyncio
async def test_return_without_events_during_shutdown(client: Client, io: AsyncLibrary):
    async with PyvoyServer(
        "tests.apps.asgi.lifespan:return_without_events_during_shutdown",
        io=io,
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        url = f"http://{server.listener_address}:{server.listener_port}"

        # This isn't an error case so we don't have anything to assert other than standard
        # success.
        response = await client.get(url)
        assert response.status == 200
        assert response.text() == "Ok"


@pytest.mark.asyncio
async def test_exception_during_shutdown(client: Client, io: AsyncLibrary):
    logs: list[str] = []
    async with PyvoyServer(
        "tests.apps.asgi.lifespan:exception_during_shutdown",
        io=io,
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        assert server.stdout is not None
        logs_task = asyncio.create_task(_read_logs(server.stdout, logs))

        url = f"http://{server.listener_address}:{server.listener_port}"

        for _ in range(5):
            response = await client.get(url)
            assert response.status == 200
            assert response.text() == "Ok"

    await logs_task
    assert any("Failing hard during shutdown" in log_line for log_line in logs), (
        f"Logs: {''.join(logs)}"
    )


@pytest.mark.asyncio
async def test_immediate_exception(client: Client, io: AsyncLibrary):
    async with PyvoyServer(
        "tests.apps.asgi.lifespan:immediate_exception", io=io
    ) as server:
        url = f"http://{server.listener_address}:{server.listener_port}"

        # This isn't an error case so we don't have anything to assert other than standard
        # success.
        response = await client.get(url)
        assert response.status == 200
        assert response.text() == "Ok"


@pytest.mark.asyncio
async def test_lifespan_optional_not_supported(client: Client, io: AsyncLibrary):
    async with PyvoyServer("tests.apps.asgi.kitchensink", io=io) as server:
        url = f"http://{server.listener_address}:{server.listener_port}"

        # This isn't an error case so we don't have anything to assert other than standard
        # success.
        response = await client.post(
            f"{url}/request-and-response-body", content=b"Bear please"
        )
        assert response.status == 200
        assert response.text() == "Yogi Bear"


@pytest.mark.asyncio
async def test_lifespan_required_not_supported(io: AsyncLibrary):
    server = PyvoyServer(
        "tests.apps.asgi.kitchensink",
        io=io,
        lifespan=True,
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    )
    with pytest.raises(RuntimeError):
        await server.start()
    assert server.stdout is not None
    logs: list[str] = []
    logs_task = asyncio.create_task(_read_logs(server.stdout, logs))

    await logs_task
    assert any(
        "Application startup failed. Exiting." in log_line for log_line in logs
    ), f"Logs: {''.join(logs)}"

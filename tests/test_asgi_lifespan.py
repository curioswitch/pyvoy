import asyncio
import subprocess

import httpx
import pytest

from pyvoy import PyvoyServer


async def _read_logs(stream: asyncio.StreamReader, logs: list[str]):
    async for line in stream:
        logs.append(line.decode())


@pytest.mark.asyncio
async def test_lifespan_normal():
    logs: list[str] = []
    async with (
        PyvoyServer(
            "tests.apps.asgi.lifespan:normal",
            stderr=subprocess.STDOUT,
            stdout=subprocess.PIPE,
        ) as server,
        httpx.AsyncClient() as client,
    ):
        assert server.stdout is not None
        logs_task = asyncio.create_task(_read_logs(server.stdout, logs))

        url = f"http://{server.listener_address}:{server.listener_port}"

        for _ in range(5):
            response = await client.get(url)
            assert response.status_code == 200
            assert response.text == "Ok"

    await logs_task
    assert any("Got counter: 5" in log_line for log_line in logs), (
        f"Logs: {''.join(logs)}"
    )


@pytest.mark.asyncio
async def test_lifespan_startup_failed():
    logs: list[str] = []
    async with PyvoyServer(
        "tests.apps.asgi.lifespan:startup_failed",
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        assert server.stdout is not None
        logs_task = asyncio.create_task(_read_logs(server.stdout, logs))

    await logs_task
    assert any("I failed to startup" in log_line for log_line in logs), (
        f"Logs: {''.join(logs)}"
    )


@pytest.mark.asyncio
async def test_lifespan_shutdown_failed():
    logs: list[str] = []
    async with (
        PyvoyServer(
            "tests.apps.asgi.lifespan:shutdown_failed",
            stderr=subprocess.STDOUT,
            stdout=subprocess.PIPE,
        ) as server,
        httpx.AsyncClient() as client,
    ):
        assert server.stdout is not None
        logs_task = asyncio.create_task(_read_logs(server.stdout, logs))

        url = f"http://{server.listener_address}:{server.listener_port}"

        for _ in range(5):
            response = await client.get(url)
            assert response.status_code == 200
            assert response.text == "Ok"

    await logs_task
    assert any("I failed to shutdown" in log_line for log_line in logs), (
        f"Logs: {''.join(logs)}"
    )

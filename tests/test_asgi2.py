from __future__ import annotations

import httpx
import pytest

from pyvoy import PyvoyServer


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "app",
    [
        "ASGI2Class",
        "asgi2_func",
        "asgi2_func_marked",
        "asgi3_func_marked",
        "asgi2_callable",
        "asgi3_callable",
    ],
)
async def test_asgi2_compat(app: str):
    async with (
        PyvoyServer(f"tests.apps.asgi.asgi2:{app}") as server,
        httpx.AsyncClient() as client,
    ):
        url = f"http://{server.listener_address}:{server.listener_port}"
        response = await client.get(url)
        assert response.status_code == 200
        assert response.text == "Ok"

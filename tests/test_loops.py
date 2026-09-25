from __future__ import annotations

import importlib.util
import subprocess
from typing import TYPE_CHECKING, get_args

import pytest

from pyvoy import Loop, PyvoyServer

from ._util import gil_enabled

if TYPE_CHECKING:
    from pyqwest import Client


def _loop_param(loop: Loop) -> object:
    if loop == "zuvloop" and not gil_enabled():
        return pytest.param(
            loop, marks=pytest.mark.skip(reason="zuvloop hangs on free-threaded Python")
        )
    installed = importlib.util.find_spec(loop) is not None
    return pytest.param(
        loop, marks=pytest.mark.skipif(not installed, reason=f"{loop} is not installed")
    )


# The full suite only runs on the default loop and trio, so this covers that
# each other loop is used when configured and can serve a request.
@pytest.mark.asyncio
@pytest.mark.parametrize("loop", [_loop_param(loop) for loop in get_args(Loop)])
async def test_configured_loop(loop: Loop, client: Client) -> None:
    async with PyvoyServer(
        "tests.apps.asgi.kitchensink",
        loop=loop,
        lifespan=False,
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
    ) as server:
        url = f"http://{server.listener_address}:{server.listener_port}"
        response = await client.get(f"{url}/event-loop")
        assert response.status == 200, response.text()
        assert response.text() == loop

        response = await client.post(
            f"{url}/request-and-response-body", content=b"Bear please"
        )
        assert response.status == 200, response.text()
        assert response.content == b"Yogi Bear"

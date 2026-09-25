# Mirrors pyqwest's tests/test_errors.py, run inside a pyvoy application against
# pyvoy's Envoy-backed transports. Differences from pyqwest are marked GAP.
from __future__ import annotations

from typing import cast

import anyio
import pytest
from anyio import to_thread
from pyqwest import Client, ReadError, SyncClient, WriteError

from ._util import SyncRequestBody, hanging_body


async def request_timeout(client: Client | SyncClient, url: str) -> None:
    method = "POST"
    url = f"{url}/echo"
    # Even with a timeout of zero, headers may still return before timeout,
    # though rarely. There's no way to trigger header timeout deterministically
    # so we just allow it to fail within response handling some times, and
    # try to increase the chance of that by running this test a few times.
    for _ in range(10):
        with pytest.raises(TimeoutError):
            if isinstance(client, SyncClient):

                def run():
                    request_content = SyncRequestBody()
                    with client.stream(
                        method, url, content=request_content, timeout=0
                    ) as resp:
                        next(resp.content)

                await to_thread.run_sync(run)
            else:
                with anyio.fail_after(0):
                    async with client.stream(
                        method, url, content=hanging_body()
                    ) as resp:
                        await anext(resp.content)


async def response_content_timeout(client: Client | SyncClient, url: str) -> None:
    method = "POST"
    url = f"{url}/echo"
    # Anecdotally, the above test will have one of its runs timeout on the response body
    # in many cases, but check explicitly for good measure.
    with pytest.raises(TimeoutError):
        if isinstance(client, SyncClient):

            def run():
                request_content = SyncRequestBody()
                with client.stream(
                    method, url, content=request_content, timeout=0.03
                ) as resp:
                    assert resp.status == 200
                    next(resp.content)

            await to_thread.run_sync(run)
        else:
            with anyio.fail_after(0.03):
                async with client.stream(method, url, content=hanging_body()) as resp:
                    assert resp.status == 200
                    await anext(resp.content)


# GAP: Envoy always synthesizes as 503 response for our callouts, so we never raise
# ConnectionError like in pyqwest. User code should generally not be affected
# since in practice, both are almost always handled the same way.
async def connection_error(client: Client | SyncClient, url: str) -> None:
    url = f"{url}/echo"
    if isinstance(client, SyncClient):
        res = await to_thread.run_sync(client.get, url)
    else:
        res = await client.get(url)
    assert res.status == 503


async def request_not_bytes(client: Client | SyncClient, url: str) -> None:
    method = "POST"
    url = f"{url}/echo"
    # This can also surface either on read or write side based on timing
    with pytest.raises((ReadError, WriteError)):
        if isinstance(client, SyncClient):

            def request_content_sync():
                yield cast("bytes", 10)

            def run():
                with client.stream(method, url, content=request_content_sync()) as resp:
                    next(resp.content)

            await to_thread.run_sync(run)
        else:

            async def request_content():
                yield cast("bytes", 10)

            async with client.stream(method, url, content=request_content()) as resp:
                await anext(resp.content)

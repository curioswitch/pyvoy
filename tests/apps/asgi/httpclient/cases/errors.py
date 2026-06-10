from __future__ import annotations

import asyncio
import sys
from typing import TYPE_CHECKING

import pytest
from pyqwest import Client, SyncClient

from ._util import SyncRequestBody

if TYPE_CHECKING:
    from collections.abc import AsyncIterator


async def request_body(queue: asyncio.Queue) -> AsyncIterator[bytes]:
    while True:
        item: bytes | None = await queue.get()
        if item is None:
            return
        yield item


async def request_timeout(client: Client | SyncClient, url: str) -> None:
    assert sys.version_info >= (3, 11)

    method = "POST"
    url = f"{url}/echo"
    # Even with a timeout of zero, headers may still return before timeout,
    # though rarely. There's no way to trigger header timeout deterministically
    # so we just allow it to fail within response handling some times, and
    # try to increase the chance of that by running this test a few times.
    for _ in range(10):
        with pytest.raises((TimeoutError, asyncio.TimeoutError)):
            if isinstance(client, SyncClient):

                def run():
                    request_content = SyncRequestBody()
                    with client.stream(
                        method, url, content=request_content, timeout=0
                    ) as resp:
                        next(resp.content)

                await asyncio.to_thread(run)
            else:
                queue = asyncio.Queue()
                async with asyncio.timeout(0):
                    async with client.stream(
                        method, url, content=request_body(queue)
                    ) as resp:
                        await anext(resp.content)


async def response_content_timeout(client: Client | SyncClient, url: str) -> None:
    assert sys.version_info >= (3, 11)

    method = "POST"
    url = f"{url}/echo"
    # Anecdotally, the above test will have one of its runs timeout on the response body
    # in many cases, but check explicitly for good measure.
    with pytest.raises((TimeoutError, asyncio.TimeoutError)):
        if isinstance(client, SyncClient):

            def run():
                request_content = SyncRequestBody()
                with client.stream(
                    method, url, content=request_content, timeout=0.03
                ) as resp:
                    assert resp.status == 200
                    next(resp.content)

            await asyncio.to_thread(run)
        else:
            queue = asyncio.Queue()
            async with asyncio.timeout(0.03):
                async with client.stream(
                    method, url, content=request_body(queue)
                ) as resp:
                    assert resp.status == 200
                    await anext(resp.content)


# GAP: Envoy always synthesizes as 503 response for our callouts, so we never raise
# ConnectionError like in pyqwest. User code should generally not be affected
# since in practice, both are almost always handled the same way.
@pytest.mark.asyncio
async def connection_error(client: Client | SyncClient, url: str) -> None:
    url = f"{url}/echo"
    if isinstance(client, SyncClient):

        def run():
            client.get(url)

        res = await asyncio.to_thread(run)
    else:
        res = await client.get(url)
    assert res.status == 503

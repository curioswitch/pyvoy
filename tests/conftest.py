from __future__ import annotations

from collections.abc import AsyncIterator

import pytest_asyncio
from pyqwest import Client, HTTPTransport, HTTPVersion


@pytest_asyncio.fixture
async def client() -> Client:
    return Client()


@pytest_asyncio.fixture
async def client_http2() -> AsyncIterator[Client]:
    async with HTTPTransport(http_version=HTTPVersion.HTTP2) as transport:
        yield Client(transport=transport)

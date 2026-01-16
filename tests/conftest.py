from __future__ import annotations

from typing import TYPE_CHECKING

import pytest_asyncio
from pyqwest import Client, HTTPTransport, HTTPVersion

if TYPE_CHECKING:
    from collections.abc import AsyncIterator


@pytest_asyncio.fixture
async def client() -> Client:
    return Client()


@pytest_asyncio.fixture
async def client_http2() -> AsyncIterator[Client]:
    async with HTTPTransport(http_version=HTTPVersion.HTTP2) as transport:
        yield Client(transport=transport)

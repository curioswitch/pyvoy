from __future__ import annotations

from typing import TYPE_CHECKING

import pytest
import pytest_asyncio
from pyqwest import Client, HTTPTransport, HTTPVersion

if TYPE_CHECKING:
    from collections.abc import AsyncIterator


def pytest_addoption(parser: pytest.Parser) -> None:
    parser.addoption(
        "--full",
        action="store_true",
        default=False,
        help="run full test suite (including slow tests)",
    )


def pytest_collection_modifyitems(
    config: pytest.Config, items: list[pytest.Item]
) -> None:
    if config.getoption("--full"):
        # --full given in cli: do not skip slow tests
        return
    skip_slow = pytest.mark.skip(reason="need --full option to run")
    for item in items:
        if "slow" in item.keywords:
            item.add_marker(skip_slow)


@pytest_asyncio.fixture
async def client() -> Client:
    return Client()


@pytest_asyncio.fixture
async def client_http2() -> AsyncIterator[Client]:
    async with HTTPTransport(http_version=HTTPVersion.HTTP2) as transport:
        yield Client(transport=transport)

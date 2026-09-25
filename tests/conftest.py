from __future__ import annotations

from typing import TYPE_CHECKING

import pytest
import pytest_asyncio
from pyqwest import Client, HTTPTransport, HTTPVersion

if TYPE_CHECKING:
    from collections.abc import AsyncIterator

    from pyvoy import AsyncLibrary


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
    # WSGI applications do not run on an event loop, so a test parametrized
    # over both interfaces only needs its WSGI case once.
    kept: list[pytest.Item] = []
    dropped: list[pytest.Item] = []
    for item in items:
        callspec = getattr(item, "callspec", None)
        params = callspec.params if callspec is not None else {}
        if (
            params.get("interface") == "wsgi"
            and params.get("io", "asyncio") != "asyncio"
        ):
            dropped.append(item)
        else:
            kept.append(item)
    if dropped:
        config.hook.pytest_deselected(items=dropped)
        items[:] = kept

    if config.getoption("--full"):
        # --full given in cli: do not skip slow tests
        return
    skip_slow = pytest.mark.skip(reason="need --full option to run")
    for item in items:
        if "slow" in item.keywords:
            item.add_marker(skip_slow)


# PyvoyServer runs applications in its Envoy subprocess, so tests always run
# on asyncio and this selects the async library the server runs ASGI
# applications on.
@pytest.fixture(scope="session", params=["asyncio", "trio"])
def io(request: pytest.FixtureRequest) -> AsyncLibrary:
    return request.param


@pytest_asyncio.fixture
async def client() -> Client:
    return Client()


@pytest_asyncio.fixture
async def client_http2() -> AsyncIterator[Client]:
    async with HTTPTransport(http_version=HTTPVersion.HTTP2) as transport:
        yield Client(transport=transport)

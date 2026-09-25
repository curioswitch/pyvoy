from __future__ import annotations

import importlib.util
from typing import TYPE_CHECKING

import pytest
import pytest_asyncio
from pyqwest import Client, HTTPTransport, HTTPVersion

from ._util import gil_enabled

if TYPE_CHECKING:
    from collections.abc import AsyncIterator

    from pyvoy import Loop


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
    # over both interfaces only needs its WSGI case on the default loop.
    kept: list[pytest.Item] = []
    dropped: list[pytest.Item] = []
    for item in items:
        callspec = getattr(item, "callspec", None)
        params = callspec.params if callspec is not None else {}
        if params.get("interface") == "wsgi" and params.get("loop") is not None:
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
# on asyncio and this selects the event loop the server runs ASGI applications
# on. None is the default event loop. zuvloop is not a pyvoy dependency and
# only supports Python 3.14+, so it runs when installed. It hangs on
# free-threaded Python for now, so it is skipped there. Other event loops only
# get a smoke test in test_loops.py.
_suite_loops: list[Loop | None] = [
    None,
    *(
        ["zuvloop"]
        if importlib.util.find_spec("zuvloop") is not None and gil_enabled()
        else []
    ),
    "trio",
]


@pytest.fixture(
    scope="session",
    params=_suite_loops,
    ids=[loop or "default" for loop in _suite_loops],
)
def loop(request: pytest.FixtureRequest) -> Loop | None:
    return request.param


@pytest_asyncio.fixture
async def client() -> Client:
    return Client()


@pytest_asyncio.fixture
async def client_http2() -> AsyncIterator[Client]:
    async with HTTPTransport(http_version=HTTPVersion.HTTP2) as transport:
        yield Client(transport=transport)

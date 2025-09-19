import asyncio
import threading
import time
from collections.abc import Iterator
from typing import Any

import httpx
import pytest
import uvicorn
import uvloop
from hypercorn.asyncio import serve as hypercorn_serve
from hypercorn.config import Config as HypercornConfig
from hypercorn.logging import Logger as HypercornLogger
from pytest_benchmark.fixture import BenchmarkFixture

from tests.apps.kitchensink import app as kitchensink_app


@pytest.fixture(scope="module")
def kitchensink_uvicorn_url() -> Iterator[str]:
    config = uvicorn.Config("tests.apps.kitchensink:app", port=0, log_level="warning")
    server = uvicorn.Server(config)
    thread = threading.Thread(target=server.run)
    thread.daemon = True
    thread.start()
    while not server.started:
        pass
    yield f"http://localhost:{server.servers[0].sockets[0].getsockname()[1]}"
    server.should_exit = True
    # Don't join the thread since it can take some time


class PortCapturingLogger(HypercornLogger):
    port = -1

    def __init__(self, conf: HypercornConfig) -> None:
        super().__init__(conf)

    async def info(self, message: str, *args: Any, **kwargs: Any) -> None:
        if "Running on" in message:
            _, _, rest = message.partition("//127.0.0.1:")
            port, _, _ = rest.partition(" ")
            self.port = int(port)
        await super().info(message, *args, **kwargs)


@pytest.fixture(scope="module")
def kitchensink_hypercorn_url() -> Iterator[str]:
    conf = HypercornConfig()
    conf.bind = ["127.0.0.1:0"]
    conf._log = PortCapturingLogger(conf)

    shutdown = asyncio.Event()

    async def run_hypercorn() -> None:
        await hypercorn_serve(
            kitchensink_app,  # pyright:ignore[reportArgumentType] - some incompatibility in type
            conf,
            mode="asgi",
            shutdown_trigger=shutdown.wait,
        )

    thread = threading.Thread(target=uvloop.run, args=(run_hypercorn(),))
    thread.daemon = True
    thread.start()

    port = -1
    for _ in range(100):
        port = conf._log.port
        if port != -1:
            break
        time.sleep(0.01)

    yield f"http://localhost:{port}"
    shutdown.set()
    # Don't join the thread since it can take some time


@pytest.fixture(params=["pyvoy", "hypercorn", "uvicorn"])
def server(request: pytest.FixtureRequest) -> str:
    return request.param


@pytest.fixture
def url(
    server,
    kitchensink_url: str,
    kitchensink_hypercorn_url: str,
    kitchensink_uvicorn_url: str,
) -> str:
    match server:
        case "pyvoy":
            return kitchensink_url
        case "hypercorn":
            return kitchensink_hypercorn_url
        case "uvicorn":
            return kitchensink_uvicorn_url
        case _:
            msg = f"Unknown param {server}"
            raise ValueError(msg)


@pytest.mark.parametrize("http2", [False, True], ids=["http/1.1", "http/2"])
def test_headers_only(
    url: str, http2: bool, server: str, benchmark: BenchmarkFixture
) -> None:
    if server == "uvicorn" and http2:
        pytest.skip("uvicorn does not support http/2")
    with httpx.Client(http2=http2, http1=not http2) as client:
        benchmark(
            client.get,
            f"{url}/headers-only",
            headers=(("Accept", "text/plain"), ("Multiple", "v1"), ("Multiple", "v2")),
        )


@pytest.mark.parametrize("http2", [False, True], ids=["http/1.1", "http/2"])
def test_request_and_response_body(
    url: str, http2: bool, server: str, benchmark: BenchmarkFixture
) -> None:
    if server == "uvicorn" and http2:
        pytest.skip("uvicorn does not support http/2")
    with httpx.Client(http2=http2, http1=not http2) as client:
        benchmark(
            client.post, f"{url}/request-and-response-body", content="Bear please"
        )


@pytest.mark.parametrize("http2", [False, True], ids=["http/1.1", "http/2"])
def test_large_bodies(
    url: str, http2: bool, server: str, benchmark: BenchmarkFixture
) -> None:
    if server == "uvicorn" and http2:
        pytest.skip("uvicorn does not support http/2")
    with httpx.Client(http2=http2, http1=not http2) as client:
        benchmark(client.post, f"{url}/large-bodies", content=b"A" * 1_000_000)

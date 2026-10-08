"""Regression tests for request-body backpressure.

Without real backpressure, on_request_body() always returns
StopIterationAndBuffer, which Envoy's own documentation says "can not push
back on streaming data via watermarks". When the ASGI app's receive() calls
fall behind the rate a client sends data, request body chunks accumulate in
Envoy's per-filter buffer completely unbounded until the connection's overall
buffer limit is hit, at which point Envoy gives up and resets the stream with
a 413 -- regardless of how large that limit is configured, for a large enough
request.

These tests use a small per_connection_buffer_limit_bytes and a deliberately
slow-consuming app (tests/apps/asgi/slow_request_body.py) to reproduce this
deterministically with a modest request body, and verify that configuring
request_body_high_watermark/request_body_low_watermark keeps the backlog
bounded well under the connection buffer limit instead of exceeding it.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

import pytest
import pytest_asyncio

from pyvoy import PyvoyServer

if TYPE_CHECKING:
    from collections.abc import AsyncIterator

    from pyqwest import Client

# Chosen so that, without backpressure, the (fast) client sending this much
# data far outpaces the (artificially slowed) app's receive() calls long
# enough to build up a backlog well past _BUFFER_LIMIT_BYTES before the
# request completes.
_BODY_SIZE = 2_000_000

# Deliberately small so the test is fast and deterministic: large enough to
# comfortably hold our high watermark below plus headroom for one in-flight
# chunk, small enough that the unbounded-buffering bug reproduces reliably
# well before all of _BODY_SIZE has been sent.
_BUFFER_LIMIT_BYTES = 262_144  # 256 KiB

_HIGH_WATERMARK_BYTES = 32_768  # 32 KiB
_LOW_WATERMARK_BYTES = 8_192  # 8 KiB


class _ServerWithBufferLimit(PyvoyServer):
    """Applies a small per_connection_buffer_limit_bytes to every listener,
    matching the escape hatch real deployments use (pyvoy's Python API has
    no typed field for it), so the test can reproduce the bug with a modest
    body size instead of needing a multi-megabyte+ payload.
    """

    def get_envoy_config(self) -> dict:
        config = super().get_envoy_config()
        for listener in config.get("static_resources", {}).get("listeners", []):
            if "udp_listener_config" not in listener:
                listener["per_connection_buffer_limit_bytes"] = _BUFFER_LIMIT_BYTES
        return config


@pytest_asyncio.fixture
async def server_with_backpressure() -> AsyncIterator[PyvoyServer]:
    async with _ServerWithBufferLimit(
        "tests.apps.asgi.slow_request_body",
        lifespan=False,
        request_body_high_watermark=_HIGH_WATERMARK_BYTES,
        request_body_low_watermark=_LOW_WATERMARK_BYTES,
    ) as server:
        yield server


@pytest_asyncio.fixture
async def server_without_backpressure() -> AsyncIterator[PyvoyServer]:
    # A high watermark far above _BUFFER_LIMIT_BYTES means the new
    # on_request_body() check never trips, reproducing the original,
    # unbounded-buffering behavior for comparison.
    async with _ServerWithBufferLimit(
        "tests.apps.asgi.slow_request_body",
        lifespan=False,
        request_body_high_watermark=_BUFFER_LIMIT_BYTES * 10,
    ) as server:
        yield server


@pytest.mark.asyncio
async def test_large_slow_body_with_backpressure_succeeds(
    server_with_backpressure: PyvoyServer, client: Client
) -> None:
    url = f"http://{server_with_backpressure.listener_address}:{server_with_backpressure.listener_port}"
    response = await client.post(url, content=b"A" * _BODY_SIZE)
    assert response.status == 200, response.text()
    assert response.content == str(_BODY_SIZE).encode()


@pytest.mark.asyncio
async def test_large_slow_body_without_backpressure_fails_with_413(
    server_without_backpressure: PyvoyServer, client: Client
) -> None:
    # Demonstrates the test actually exercises the bug this feature fixes:
    # with backpressure effectively disabled, the same slow-consuming app and
    # buffer limit reproduce the original "Payload Too Large" failure.
    url = f"http://{server_without_backpressure.listener_address}:{server_without_backpressure.listener_port}"
    response = await client.post(url, content=b"A" * _BODY_SIZE)
    assert response.status == 413, response.text()

from __future__ import annotations

from contextlib import contextmanager
from typing import TYPE_CHECKING, cast

if TYPE_CHECKING:
    from collections.abc import Awaitable, Iterator
    from typing import Any

    from asgiref.typing import ASGIReceiveCallable, ASGISendCallable, Scope


async def _send_success(send: ASGISendCallable) -> None:
    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [(b"content-type", b"text/plain")],
            "trailers": False,
        }
    )
    await send({"type": "http.response.body", "body": b"Ok", "more_body": False})


@contextmanager
def _assert_raises(
    expected_exception_type: type[BaseException], expected_message_substring: str
) -> Iterator[None]:
    try:
        yield
    except expected_exception_type as e:
        if expected_message_substring not in str(e):
            msg = f"Expected exception message to contain '{expected_message_substring}', but got: {e!s}"
            raise AssertionError(msg) from e
    else:
        msg = f"Expected exception of type {expected_exception_type.__name__} was not raised."
        raise AssertionError(msg)


async def normal(
    scope: Scope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    if scope["type"] == "websocket":
        msg = "Websockets not supported"
        raise RuntimeError(msg)

    if scope["type"] == "lifespan":
        while True:
            msg = await recv()
            if msg["type"] == "lifespan.startup":
                scope.get("state", {})["counter"] = 0
                with _assert_raises(
                    RuntimeError,
                    "Expected lifespan message 'lifespan.startup.complete' or 'lifespan.startup.failed'",
                ):
                    await send({"type": "lifespan.shutdown.complete"})
                await send({"type": "lifespan.startup.complete"})
            elif msg["type"] == "lifespan.shutdown":
                with _assert_raises(
                    RuntimeError, "lifespan receive called after shutdown completed."
                ):
                    await recv()
                with _assert_raises(
                    RuntimeError,
                    "Expected lifespan message 'lifespan.shutdown.complete' or 'lifespan.shutdown.failed'",
                ):
                    await send({"type": "lifespan.startup.complete"})

                print(  # noqa: T201
                    f"Got counter: {scope.get('state', {}).get('counter')}", flush=True
                )
                await send({"type": "lifespan.shutdown.complete"})
                with _assert_raises(
                    RuntimeError, "lifespan send called after shutdown completed."
                ):
                    await send({"type": "lifespan.shutdown.complete"})
                return

    scope.get("state", {"counter": 0})["counter"] += 1
    await _send_success(send)


async def startup_failed(
    scope: Scope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    if scope["type"] == "websocket":
        msg = "Websockets not supported"
        raise RuntimeError(msg)

    if scope["type"] == "lifespan":
        while True:
            msg = await recv()
            if msg["type"] == "lifespan.startup":
                await send(
                    {
                        "type": "lifespan.startup.failed",
                        "message": "I failed to startup",
                    }
                )
            elif msg["type"] == "lifespan.shutdown":
                print(  # noqa: T201
                    f"Got counter: {scope.get('state', {}).get('counter')}", flush=True
                )
                await send({"type": "lifespan.shutdown.complete"})
                return

    scope.get("state", {})["counter"] += 1
    await _send_success(send)


async def startup_failed_no_msg(
    scope: Scope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    if scope["type"] == "websocket":
        msg = "Websockets not supported"
        raise RuntimeError(msg)

    if scope["type"] == "lifespan":
        while True:
            msg = await recv()
            if msg["type"] == "lifespan.startup":
                await send(cast("Any", {"type": "lifespan.startup.failed"}))
            elif msg["type"] == "lifespan.shutdown":
                print(  # noqa: T201
                    f"Got counter: {scope.get('state', {}).get('counter')}", flush=True
                )
                await send({"type": "lifespan.shutdown.complete"})
                return

    scope.get("state", {})["counter"] += 1
    await _send_success(send)


async def shutdown_failed(
    scope: Scope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    if scope["type"] == "websocket":
        msg = "Websockets not supported"
        raise RuntimeError(msg)

    if scope["type"] == "lifespan":
        while True:
            msg = await recv()
            if msg["type"] == "lifespan.startup":
                scope.get("state", {})["counter"] = 0
                await send({"type": "lifespan.startup.complete"})
            elif msg["type"] == "lifespan.shutdown":
                await send(
                    {
                        "type": "lifespan.shutdown.failed",
                        "message": "I failed to shutdown",
                    }
                )
                return

    scope.get("state", {})["counter"] += 1
    await _send_success(send)


async def shutdown_failed_no_msg(
    scope: Scope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    if scope["type"] == "websocket":
        msg = "Websockets not supported"
        raise RuntimeError(msg)

    if scope["type"] == "lifespan":
        while True:
            msg = await recv()
            if msg["type"] == "lifespan.startup":
                scope.get("state", {})["counter"] = 0
                await send({"type": "lifespan.startup.complete"})
            elif msg["type"] == "lifespan.shutdown":
                await send(cast("Any", {"type": "lifespan.shutdown.failed"}))
                return

    scope.get("state", {})["counter"] += 1
    await _send_success(send)


async def return_without_events(
    scope: Scope, _recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    if scope["type"] == "websocket":
        msg = "Websockets not supported"
        raise RuntimeError(msg)

    if scope["type"] == "lifespan":
        return

    await _send_success(send)


async def exception_during_shutdown(
    scope: Scope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    if scope["type"] == "websocket":
        msg = "Websockets not supported"
        raise RuntimeError(msg)

    if scope["type"] == "lifespan":
        while True:
            msg = await recv()
            if msg["type"] == "lifespan.startup":
                scope.get("state", {})["counter"] = 0
                await send({"type": "lifespan.startup.complete"})
            elif msg["type"] == "lifespan.shutdown":
                msg = "Failing hard during shutdown"
                raise RuntimeError(msg)

    scope.get("state", {})["counter"] += 1
    await _send_success(send)


async def return_without_events_during_shutdown(
    scope: Scope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    if scope["type"] == "websocket":
        msg = "Websockets not supported"
        raise RuntimeError(msg)

    if scope["type"] == "lifespan":
        while True:
            msg = await recv()
            if msg["type"] == "lifespan.startup":
                scope.get("state", {})["counter"] = 0
                await send({"type": "lifespan.startup.complete"})
            elif msg["type"] == "lifespan.shutdown":
                return

    scope.get("state", {})["counter"] += 1
    await _send_success(send)


# While not a realistic app, it is possible and we handle it properly.
def immediate_exception(
    scope: Scope, _recv: ASGIReceiveCallable, _send: ASGISendCallable
) -> Awaitable[None]:
    if scope["type"] == "websocket":
        msg = "Websockets not supported"
        raise RuntimeError(msg)

    if scope["type"] == "lifespan":
        msg = "Immediate exception from ASGI app"
        raise RuntimeError(msg)

    return _send_success(_send)


# Interesting that all ASGI server seem to treat a non-async function
# as ASGI2. It seems easy to check the function signature itself but we
# follow the same conventions, notably of asgiref.
immediate_exception._asgi_single_callable = True  # noqa: SLF001

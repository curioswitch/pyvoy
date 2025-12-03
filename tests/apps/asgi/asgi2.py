from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable

    from asgiref.typing import ASGIReceiveCallable, ASGISendCallable, Scope


async def _send_failure(msg: str, send: ASGISendCallable) -> None:
    await send(
        {
            "type": "http.response.start",
            "status": 500,
            "headers": [(b"content-type", b"text/plain")],
            "trailers": False,
        }
    )
    await send(
        {
            "type": "http.response.body",
            "body": b"Assertion Failure: " + msg.encode(),
            "more_body": False,
        }
    )


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


# Sanity checks the scope and responses.
async def _real_app(
    scope: Scope, _recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    if scope["type"] != "http":
        return
    if scope.get("method") != "GET":
        await _send_failure("Expected method GET", send)
        return
    await _send_success(send)


class ASGI2Class:
    def __init__(self, scope: Scope) -> None:
        self._scope = scope

    async def __call__(
        self, _recv: ASGIReceiveCallable, send: ASGISendCallable
    ) -> None:
        await _real_app(self._scope, _recv, send)


def asgi2_func(
    scope: Scope,
) -> Callable[[ASGIReceiveCallable, ASGISendCallable], Awaitable[None]]:
    async def app(_recv: ASGIReceiveCallable, send: ASGISendCallable) -> None:
        await _real_app(scope, _recv, send)

    return app


# Meaningless but for code coverage
asgi2_func._asgi_single_callable = False  # noqa: SLF001


# This function would be auto detected anyways but we need to test manual marking too.
def asgi2_func_marked(
    scope: Scope,
) -> Callable[[ASGIReceiveCallable, ASGISendCallable], Awaitable[None]]:
    async def app(_recv: ASGIReceiveCallable, send: ASGISendCallable) -> None:
        await _real_app(scope, _recv, send)

    return app


asgi2_func_marked._asgi_double_callable = True  # noqa: SLF001


# This function would be auto detected anyways but we need to test manual marking too.
async def asgi3_func_marked(
    scope: Scope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    await _real_app(scope, recv, send)


asgi3_func_marked._asgi_double_callable = False  # noqa: SLF001
asgi3_func_marked._asgi_single_callable = True  # noqa: SLF001


class ASGI2Callable:
    def __call__(
        self, scope: Scope
    ) -> Callable[[ASGIReceiveCallable, ASGISendCallable], Awaitable[None]]:
        async def app(_recv: ASGIReceiveCallable, send: ASGISendCallable) -> None:
            await _real_app(scope, _recv, send)

        return app


asgi2_callable = ASGI2Callable()


class ASGI3Callable:
    async def __call__(
        self, scope: Scope, recv: ASGIReceiveCallable, send: ASGISendCallable
    ) -> None:
        await _real_app(scope, recv, send)


asgi3_callable = ASGI3Callable()

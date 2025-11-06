import asyncio
import sys
from collections import defaultdict
from typing import TYPE_CHECKING, Any, cast

if TYPE_CHECKING:
    # We don't use asgiref code so only import from it for type checking
    from asgiref.typing import ASGIReceiveCallable, ASGISendCallable, HTTPScope, Scope
else:
    ASGIReceiveCallable = "asgiref.typing.ASGIReceiveCallable"
    ASGISendCallable = "asgiref.typing.ASGISendCallable"
    HTTPScope = "asgiref.typing.HTTPScope"
    Scope = "asgiref.typing.Scope"


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


async def _headers_only(
    scope: HTTPScope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    if scope["method"] != "GET":
        await _send_failure('scope["method"] != "GET"', send)
        return

    headers = defaultdict(list)
    for name, value in scope.get("headers", []):
        headers[name].append(value)
    if headers.get(b"accept") != [b"text/plain"]:
        await _send_failure('headers.get(b"accept") != [b"text/plain"]', send)
        return
    if headers.get(b"multiple") != [b"v1", b"v2"]:
        await _send_failure('headers.get(b"multiple") != [b"v1", b"v2"]', send)
        return

    msg = await recv()
    if msg["type"] != "http.request":
        await _send_failure('msg["type"] != "http.request"', send)
        return
    if msg.get("body", b"") != b"":
        await _send_failure('msg.get("body", b"") != b""', send)
        return
    if msg.get("more_body", False):
        await _send_failure('msg.get("more_body", False)', send)
        return

    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [(b"content-type", b"text/plain"), (b"x-animal", b"bear")],
            "trailers": False,
        }
    )
    await send({"type": "http.response.body", "body": b"", "more_body": False})


async def _request_body(
    scope: HTTPScope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    if scope["method"] != "POST":
        await _send_failure('scope["method"] != "POST"', send)
        return

    body = b""

    for _ in range(10):
        msg = await recv()
        if msg["type"] != "http.request":
            await _send_failure('msg["type"] != "http.request"', send)
            return
        body += msg.get("body", b"")
        if not msg.get("more_body", False):
            break

    if body != b"Bear please":
        await _send_failure('body != b"Bear please"', send)
        return

    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [(b"content-type", b"text/plain"), (b"x-animal", b"bear")],
            "trailers": False,
        }
    )
    await send({"type": "http.response.body", "body": b"", "more_body": False})


async def _response_body(scope: HTTPScope, send: ASGISendCallable) -> None:
    if scope["method"] != "GET":
        await _send_failure('scope["method"] != "GET"', send)
        return

    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [(b"content-type", b"text/plain")],
            "trailers": False,
        }
    )
    await send({"type": "http.response.body", "body": b"Hello ", "more_body": True})
    await send({"type": "http.response.body", "body": b"world!", "more_body": False})


async def _request_and_response_body(
    scope: HTTPScope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    if scope["method"] != "POST":
        await _send_failure('scope["method"] != "POST"', send)
        return

    body = b""

    for _ in range(10):
        msg = await recv()
        if msg["type"] != "http.request":
            await _send_failure('msg["type"] != "http.request"', send)
            return
        body += msg.get("body", b"")
        if not msg.get("more_body", False):
            break

    if body != b"Bear please":
        await _send_failure('body != b"Bear please"', send)
        return

    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [(b"content-type", b"text/plain")],
            "trailers": False,
        }
    )
    await send({"type": "http.response.body", "body": b"Yogi ", "more_body": True})
    await send({"type": "http.response.body", "body": b"Bear", "more_body": False})


async def _large_bodies(recv: ASGIReceiveCallable, send: ASGISendCallable) -> None:
    body = b""

    for _ in range(10000):
        msg = await recv()
        if msg["type"] != "http.request":
            await _send_failure('msg["type"] != "http.request"', send)
            return
        body += msg.get("body", b"")
        if not msg.get("more_body", False):
            break

    if body != b"A" * 1_000_000:
        await _send_failure('body != b"A" * 1_000_000', send)
        return

    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [(b"content-type", b"text/plain")],
            "trailers": False,
        }
    )

    for i in range(1000):
        chunk = b"B" * 1000
        more_body = i < 999
        await send(
            {"type": "http.response.body", "body": chunk, "more_body": more_body}
        )


async def _trailers_only(send: ASGISendCallable) -> None:
    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [
                (b"content-type", b"text/plain"),
                (b"trailer", b"x-first,x-second"),
            ],
            "trailers": True,
        }
    )
    await send({"type": "http.response.body", "body": b"", "more_body": False})
    await send(
        {
            "type": "http.response.trailers",
            "headers": [(b"x-first", b"last"), (b"x-second", b"first")],
            "more_trailers": False,
        }
    )


async def _response_and_trailers(send: ASGISendCallable) -> None:
    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [
                (b"content-type", b"text/plain"),
                (b"trailer", b"x-first,x-second"),
            ],
            "trailers": True,
        }
    )
    await send({"type": "http.response.body", "body": b"Hello ", "more_body": True})
    await send({"type": "http.response.body", "body": b"Bear", "more_body": False})
    await send(
        {
            "type": "http.response.trailers",
            "headers": [(b"x-first", b"last")],
            "more_trailers": True,
        }
    )
    await send(
        {
            "type": "http.response.trailers",
            "headers": [(b"x-second", b"first")],
            "more_trailers": False,
        }
    )


async def _recv_to_newline(recv: ASGIReceiveCallable) -> bytes:
    body = b""
    while True:
        msg = await recv()
        if msg["type"] != "http.request":
            msg = 'msg["type"] != "http.request"'
            raise RuntimeError(msg)
        body += msg.get("body", b"")
        if not msg.get("more_body", False):
            return body
        if body and body[-1] == b"\n"[0]:
            return body[:-1]


async def _bidi_stream(recv: ASGIReceiveCallable, send: ASGISendCallable) -> None:
    await send(
        {
            "type": "http.response.start",
            "status": 202,
            "headers": [
                (b"content-type", b"text/plain"),
                (b"x-animal", b"bear"),
                (b"trailer", b"x-result,x-time"),
            ],
            "trailers": True,
        }
    )
    await send(
        {"type": "http.response.body", "body": b"Who are you?", "more_body": True}
    )
    msg = await _recv_to_newline(recv)
    await send(
        {
            "type": "http.response.body",
            "body": b"Hi " + msg + b". What do you want to do?",
            "more_body": True,
        }
    )
    msg = await _recv_to_newline(recv)
    await send(
        {
            "type": "http.response.body",
            "body": b"Let's " + msg + b"!",
            "more_body": False,
        }
    )
    await send(
        {
            "type": "http.response.trailers",
            "headers": [(b"x-result", b"great")],
            "more_trailers": True,
        }
    )
    await send(
        {
            "type": "http.response.trailers",
            "headers": [(b"x-time", b"fast")],
            "more_trailers": False,
        }
    )


async def _exception_before_response() -> None:
    msg = "We have failed before the response"
    raise RuntimeError(msg)


async def _exception_after_response_headers(
    _recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [(b"content-type", b"text/plain")],
            "trailers": False,
        }
    )
    await send({"type": "http.response.body", "body": b"", "more_body": True})
    msg = "We have failed after response headers"
    raise RuntimeError(msg)


async def _exception_after_response_body(send: ASGISendCallable) -> None:
    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [(b"content-type", b"text/plain")],
            "trailers": False,
        }
    )
    await send(
        {"type": "http.response.body", "body": b"Hello World!!!", "more_body": True}
    )
    msg = "We have failed after response body"
    raise RuntimeError(msg)


async def _exception_after_response_complete(send: ASGISendCallable) -> None:
    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [(b"content-type", b"text/plain")],
            "trailers": False,
        }
    )
    await send(
        {"type": "http.response.body", "body": b"Hello World!!!", "more_body": False}
    )
    msg = "We have failed after response complete"
    raise RuntimeError(msg)


async def _controlled(
    scope: HTTPScope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    headers = scope["headers"]
    sleep_ms = 0
    response_bytes = 0
    for name, value in headers:
        match name:
            case b"x-sleep-ms":
                sleep_ms = int(value)
            case b"x-response-bytes":
                response_bytes = int(value)

    body = b""

    while True:
        msg = await recv()
        body += msg.get("body", b"")
        if not msg.get("more_body", False):
            break

    if sleep_ms > 0:
        await asyncio.sleep(sleep_ms / 1000.0)

    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [
                (b"content-type", b"text/plain"),
                (b"content-length", str(response_bytes).encode()),
            ],
            "trailers": False,
        }
    )

    if response_bytes > 0:
        chunk = b"A" * response_bytes
        await send({"type": "http.response.body", "body": chunk, "more_body": False})
    else:
        await send({"type": "http.response.body", "body": b"", "more_body": False})


async def _bad_app_missing_type(
    _scope: HTTPScope, _recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    try:
        # We don't await since this error is evaluated eagerly being a parameter error.
        # In other words, it will always fail even if send call and await are separated.
        # This differs from errors for wrong order, etc.
        send(
            cast(
                "Any",
                {
                    "status": 200,
                    "headers": [(b"content-type", b"text/plain")],
                    "trailers": False,
                },
            )
        )
    except RuntimeError as e:
        if str(e) != "Unexpected ASGI message, missing 'type'.":
            await _send_failure(
                "str(e) != \"Unexpected ASGI message, missing 'type'.\"", send
            )
            return
        await _send_success(send)
    else:
        await _send_failure("No exception raised for missing type", send)


async def _bad_app_start_missing_status(
    _scope: HTTPScope, _recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    try:
        # We don't await since this error is evaluated eagerly being a parameter error.
        # In other words, it will always fail even if send call and await are separated.
        # This differs from errors for wrong order, etc.
        send(
            cast(
                "Any",
                {
                    "type": "http.response.start",
                    "headers": [(b"content-type", b"text/plain")],
                    "trailers": False,
                },
            )
        )
    except RuntimeError as e:
        if (
            str(e)
            != "Unexpected ASGI message, missing 'status' in 'http.response.start'."
        ):
            await _send_failure(
                f"{e!s} != \"Unexpected ASGI message, missing 'status' in 'http.response.start'.\"",
                send,
            )
            return
        await _send_success(send)
    else:
        await _send_failure("No exception raised for missing status", send)


async def _bad_app_body_before_start(
    _scope: HTTPScope, _recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    # Verify no exception until awaited.
    awaitable = send({"type": "http.response.body", "body": b"", "more_body": False})
    try:
        await awaitable
    except RuntimeError as e:
        if (
            str(e)
            != "Expected ASGI message 'http.response.start', but got 'http.response.body'."
        ):
            await _send_failure(
                "str(e) != \"Expected ASGI message 'http.response.start', but got 'http.response.body'.\"",
                send,
            )
            return
        await _send_success(send)
    else:
        await _send_failure("No exception raised for body before start", send)


async def _bad_app_start_after_start(
    _scope: HTTPScope, _recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [(b"content-type", b"text/plain")],
            "trailers": False,
        }
    )
    # Verify no exception until awaited.
    awaitable = send(
        {
            "type": "http.response.start",
            "status": 202,
            "headers": [(b"content-type", b"text/plain")],
            "trailers": False,
        }
    )
    # As we've already sent a status code, failures will reset the stream, which leads to the unit test failing.
    try:
        await awaitable
    except RuntimeError as e:
        if (
            str(e)
            != "Expected ASGI message 'http.response.body', but got 'http.response.start'."
        ):
            await _send_failure(
                "str(e) != \"Expected ASGI message 'http.response.body', but got 'http.response.start'.\"",
                send,
            )
            return
        await send({"type": "http.response.body", "body": b"", "more_body": False})
    else:
        await _send_failure("No exception raised for body before start", send)


async def _bad_app_body_after_complete(
    _scope: HTTPScope, _recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [(b"content-type", b"text/plain")],
            "trailers": False,
        }
    )
    await send({"type": "http.response.body", "body": b"", "more_body": False})
    # Verify no exception until awaited.
    awaitable = send({"type": "http.response.body", "body": b"", "more_body": False})
    # The request was already completed successfully on the client side so we can't propagate
    # assertion failures through it and use logs instead.
    try:
        await awaitable
    except RuntimeError as e:
        if (
            str(e)
            != "Unexpected ASGI message 'http.response.body' sent, after response already completed."
        ):
            print(  # noqa: T201
                f"{e!s} != \"Unexpected ASGI message 'http.response.body' sent, after response already completed.\"",
                file=sys.stderr,
            )
            return
        print(  # noqa: T201
            "Assertions passed", file=sys.stderr
        )
    else:
        print(  # noqa: T201
            "No exception raised for body before start", file=sys.stderr
        )


async def _bad_app_start_instead_of_trailers(
    _scope: HTTPScope, _recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [(b"content-type", b"text/plain")],
            "trailers": True,
        }
    )
    await send({"type": "http.response.body", "body": b"", "more_body": False})
    # Verify no exception until awaited.
    awaitable = send(
        {
            "type": "http.response.start",
            "status": 202,
            "headers": [(b"content-type", b"text/plain")],
            "trailers": False,
        }
    )
    # As we've already sent a status code, failures will reset the stream, which leads to the unit test failing.
    try:
        await awaitable
    except RuntimeError as e:
        if (
            str(e)
            != "Expected ASGI message 'http.response.trailers', but got 'http.response.start'."
        ):
            await _send_failure(
                "str(e) != \"Expected ASGI message 'http.response.trailers', but got 'http.response.start'.\"",
                send,
            )
            return
        await send(
            {"type": "http.response.trailers", "headers": [], "more_trailers": False}
        )
    else:
        await _send_failure("No exception raised for body before start", send)


async def _print_logs(
    _scope: HTTPScope, _recv: ASGIReceiveCallable, _send: ASGISendCallable
) -> None:
    print("This is a stdout print", flush=True)  # noqa: T201
    print("This is a stderr print", file=sys.stderr)  # noqa: T201
    await _send_success(_send)


async def app(
    scope: HTTPScope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    match scope["path"]:
        case "/headers-only":
            await _headers_only(scope, recv, send)
        case "/request-body":
            await _request_body(scope, recv, send)
        case "/response-body":
            await _response_body(scope, send)
        case "/request-and-response-body":
            await _request_and_response_body(scope, recv, send)
        case "/large-bodies":
            await _large_bodies(recv, send)
        case "/trailers-only":
            await _trailers_only(send)
        case "/response-and-trailers":
            await _response_and_trailers(send)
        case "/bidi-stream":
            await _bidi_stream(recv, send)
        case "/exception-before-response":
            await _exception_before_response()
        case "/exception-after-response-headers":
            await _exception_after_response_headers(recv, send)
        case "/exception-after-response-body":
            await _exception_after_response_body(send)
        case "/exception-after-response-complete":
            await _exception_after_response_complete(send)
        case "/controlled":
            await _controlled(scope, recv, send)
        case "/bad-app-missing-type":
            await _bad_app_missing_type(scope, recv, send)
        case "/bad-app-start-missing-status":
            await _bad_app_start_missing_status(scope, recv, send)
        case "/bad-app-body-before-start":
            await _bad_app_body_before_start(scope, recv, send)
        case "/bad-app-start-after-start":
            await _bad_app_start_after_start(scope, recv, send)
        case "/bad-app-body-after-complete":
            await _bad_app_body_after_complete(scope, recv, send)
        case "/bad-app-start-instead-of-trailers":
            await _bad_app_start_instead_of_trailers(scope, recv, send)
        case "/print-logs":
            await _print_logs(scope, recv, send)
        case _:
            await _send_failure("unknown path", send)

import asyncio
from collections import defaultdict
from typing import TYPE_CHECKING

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
    msg = await recv()
    if msg["type"] == "http.request":
        await send(
            {
                "type": "http.response.body",
                "body": b"Hi " + msg["body"] + b". What do you want to do?",
                "more_body": True,
            }
        )
    msg = await recv()
    if msg["type"] == "http.request":
        await send(
            {
                "type": "http.response.body",
                "body": b"Let's " + msg["body"] + b"!",
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
    msg = "We have failed hard"
    raise RuntimeError(msg)


async def _exception_after_response_headers(
    recv: ASGIReceiveCallable, send: ASGISendCallable
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
    msg = "We have failed hard"
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
    msg = "We have failed hard"
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
    msg = "We have failed hard"
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
        case _:
            await _send_failure("unknown path", send)

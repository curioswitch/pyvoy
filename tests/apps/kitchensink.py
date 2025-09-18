from collections import defaultdict

from asgiref.typing import ASGIReceiveCallable, ASGISendCallable, HTTPScope


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


# Since there isn't any Python HTTP library supporting bidirectional streaming,
# we will test it outside of Python.
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
        case _:
            await _send_failure("unknown path", send)

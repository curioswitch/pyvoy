from __future__ import annotations

import asyncio
import sys
from collections import defaultdict
from typing import TYPE_CHECKING, Any, TypeVar, cast

if TYPE_CHECKING:
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


class AssertionFailed(Exception):
    pass


V = TypeVar("V")


async def _assert_dict_value(
    actual: dict[str, V] | HTTPScope, key: str, expected: V, send: ASGISendCallable
) -> None:
    obj = "scope" if actual.get("type") == "http" else "headers"
    if key not in actual:
        await _send_failure(f'{obj}["{key}"] not found', send)
        raise AssertionFailed
    if actual[key] != expected:
        await _send_failure(f'{obj}["{key}"] != {expected!r}, is {actual[key]!r}', send)
        raise AssertionFailed


async def _assert_scope_address(
    address: tuple[str, int | None] | None, key: str, send: ASGISendCallable
) -> None:
    if address is None:
        await _send_failure(f'scope["{key}"] is None', send)
        raise AssertionFailed
    host, port = address
    if host != "127.0.0.1":
        await _send_failure(f'scope["{key}"][0] != "127.0.0.1", is {host!r}', send)
    if port is None or port <= 0:
        await _send_failure(f'scope["{key}"][1] <= 0, is {port!r}', send)
        raise AssertionFailed


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
    try:
        await _assert_dict_value(scope, "type", "http", send)
        await _assert_dict_value(
            cast("dict[str, str]", scope["asgi"]), "version", "3.0", send
        )
        await _assert_dict_value(
            cast("dict[str, str]", scope["asgi"]), "spec_version", "2.5", send
        )
        await _assert_dict_value(scope, "http_version", "1.1", send)
        await _assert_dict_value(scope, "method", "GET", send)
        await _assert_dict_value(scope, "scheme", "http", send)
        await _assert_dict_value(scope, "path", "/headers-only", send)
        await _assert_dict_value(scope, "raw_path", b"/headers-only", send)
        await _assert_dict_value(scope, "query_string", b"", send)
        await _assert_dict_value(scope, "root_path", "", send)
        await _assert_scope_address(scope.get("client"), "client", send)
        await _assert_scope_address(scope.get("server"), "server", send)
        if "state" in scope:
            await _send_failure('scope["state"] should not be present', send)
            return None
    except AssertionFailed:
        return None

    headers = defaultdict(list)
    for name, value in scope.get("headers", []):
        headers[name].append(value)
    if headers.get(b"accept") != [b"text/plain"]:
        return await _send_failure('headers.get(b"accept") != [b"text/plain"]', send)
    if headers.get(b"multiple") != [b"v1", b"v2"]:
        return await _send_failure('headers.get(b"multiple") != [b"v1", b"v2"]', send)

    msg = await recv()
    if msg["type"] != "http.request":
        return await _send_failure('msg["type"] != "http.request"', send)
    if msg.get("body", b"") != b"":
        return await _send_failure('msg.get("body", b"") != b""', send)
    if msg.get("more_body", False):
        return await _send_failure('msg.get("more_body", False)', send)

    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [(b"content-type", b"text/plain"), (b"x-animal", b"bear")],
            "trailers": False,
        }
    )
    await send({"type": "http.response.body", "body": b"", "more_body": False})
    return None


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
    await send(cast("Any", {"type": "http.response.body"}))
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
    # A bit lazy, but add coverage for handling of default values in dict here
    # instead of adding yet another test handler.
    await send(cast("Any", {"type": "http.response.start", "status": 200}))
    await send(cast("Any", {"type": "http.response.body", "body": b"Hello World!!!"}))
    msg = "We have failed after response complete"
    raise RuntimeError(msg)


async def _client_closed_before_response(
    _scope: HTTPScope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    # Client libraries tend to be well behaved and close the request properly, so we can't
    # avoid empty body messages. We go ahead and consume them, and should eventually reach
    # the disconnect.
    for _ in range(100):
        msg = await recv()
        if msg["type"] != "http.request":
            break
        await asyncio.sleep(0.1)
    if msg["type"] != "http.disconnect":
        msg = f'{msg["type"]} != "http.disconnect"'
        raise RuntimeError(msg)
    try:
        await _send_success(send)
    except OSError:
        print("send raised OSError as expected", file=sys.stderr)  # noqa: T201
    except Exception as e:
        msg = f"Unexpected exception type: {e!s}"
        raise RuntimeError(msg) from e
    try:
        await _send_success(send)
    except OSError:
        print("send raised OSError as expected", file=sys.stderr)  # noqa: T201
    except Exception as e:
        msg = f"Unexpected exception type: {e!s}"
        raise RuntimeError(msg) from e
    print("client-closed-before-response assertions passed", file=sys.stderr)  # noqa: T201

    # An escaped client disconnection error should not be logged.
    await _send_success(send)


async def _client_closed_after_response_start(
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
    for _ in range(10000):
        try:
            await send({"type": "http.response.body", "body": b"", "more_body": True})
        except OSError:
            print("send raised OSError as expected", file=sys.stderr)  # noqa: T201
            break
        await asyncio.sleep(0.1)
    print("client-closed-after-response-start assertions passed", file=sys.stderr)  # noqa: T201


async def _client_closed_after_trailers_start(
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
    for _ in range(10000):
        try:
            await send(
                {"type": "http.response.trailers", "headers": [], "more_trailers": True}
            )
        except OSError:
            print("send raised OSError as expected", file=sys.stderr)  # noqa: T201
            break
        await asyncio.sleep(0.1)
    print("client-closed-after-trailers-start assertions passed", file=sys.stderr)  # noqa: T201


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
            "bad-app-body-after-complete assertions passed", file=sys.stderr
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
        await send(cast("Any", {"type": "http.response.trailers"}))
    else:
        await _send_failure("No exception raised for body before start", send)


async def _bad_app_invalid_header_name(
    _scope: HTTPScope, _recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    try:
        send(
            {
                "type": "http.response.start",
                "status": 200,
                "headers": [(b"\x00\x01\x02", b"text/plain")],
                "trailers": False,
            }
        )
    except ValueError as e:
        if str(e) != "invalid HTTP header name":
            await _send_failure(f'{e!s} != "invalid HTTP header name"', send)
            return
        await _send_success(send)
    else:
        await _send_failure("No exception raised for invalid header name", send)


async def _bad_app_invalid_header_value(
    _scope: HTTPScope, _recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    try:
        send(
            {
                "type": "http.response.start",
                "status": 200,
                "headers": [(b"content-type", b"\x00\x01\x02")],
                "trailers": False,
            }
        )
    except ValueError as e:
        if str(e) != "failed to parse header value":
            await _send_failure(f'{e!s} != "failed to parse header value"', send)
            return
        await _send_success(send)
    else:
        await _send_failure("No exception raised for invalid header value", send)


async def _print_logs(
    _scope: HTTPScope, _recv: ASGIReceiveCallable, _send: ASGISendCallable
) -> None:
    print("This is a stdout print", flush=True)  # noqa: T201
    print("This is a stderr print", file=sys.stderr)  # noqa: T201
    await _send_success(_send)


async def _all_the_headers(
    scope: HTTPScope, _recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    headers: dict[str, list[str]] = defaultdict(list)
    for name, value in scope["headers"]:
        headers[name.decode()].append(value.decode())

    try:
        await _assert_dict_value(headers, "accept", ["accept"], send)
        await _assert_dict_value(headers, "accept-charset", ["accept-charset"], send)
        await _assert_dict_value(headers, "accept-encoding", ["accept-encoding"], send)
        await _assert_dict_value(headers, "accept-language", ["accept-language"], send)
        await _assert_dict_value(headers, "accept-ranges", ["accept-ranges"], send)
        await _assert_dict_value(
            headers,
            "access-control-allow-credentials",
            ["access-control-allow-credentials"],
            send,
        )
        await _assert_dict_value(
            headers,
            "access-control-allow-headers",
            ["access-control-allow-headers"],
            send,
        )
        await _assert_dict_value(
            headers,
            "access-control-allow-methods",
            ["access-control-allow-methods"],
            send,
        )
        await _assert_dict_value(
            headers,
            "access-control-allow-origin",
            ["access-control-allow-origin"],
            send,
        )
        await _assert_dict_value(
            headers,
            "access-control-expose-headers",
            ["access-control-expose-headers"],
            send,
        )
        await _assert_dict_value(
            headers, "access-control-max-age", ["access-control-max-age"], send
        )
        await _assert_dict_value(
            headers,
            "access-control-request-headers",
            ["access-control-request-headers"],
            send,
        )
        await _assert_dict_value(
            headers,
            "access-control-request-method",
            ["access-control-request-method"],
            send,
        )
        await _assert_dict_value(headers, "age", ["age"], send)
        await _assert_dict_value(headers, "allow", ["allow"], send)
        await _assert_dict_value(headers, "alt-svc", ["alt-svc"], send)
        await _assert_dict_value(headers, "authorization", ["authorization"], send)
        await _assert_dict_value(headers, "cache-control", ["cache-control"], send)
        await _assert_dict_value(headers, "cache-status", ["cache-status"], send)
        await _assert_dict_value(
            headers, "cdn-cache-control", ["cdn-cache-control"], send
        )
        # Skip connection
        await _assert_dict_value(
            headers, "content-disposition", ["content-disposition"], send
        )
        await _assert_dict_value(
            headers, "content-encoding", ["content-encoding"], send
        )
        await _assert_dict_value(
            headers, "content-language", ["content-language"], send
        )
        # Skip content-length
        await _assert_dict_value(
            headers, "content-location", ["content-location"], send
        )
        await _assert_dict_value(headers, "content-range", ["content-range"], send)
        await _assert_dict_value(
            headers, "content-security-policy", ["content-security-policy"], send
        )
        await _assert_dict_value(
            headers,
            "content-security-policy-report-only",
            ["content-security-policy-report-only"],
            send,
        )
        await _assert_dict_value(headers, "content-type", ["content-type"], send)
        await _assert_dict_value(headers, "cookie", ["cookie"], send)
        await _assert_dict_value(headers, "dnt", ["dnt"], send)
        await _assert_dict_value(headers, "date", ["date"], send)
        await _assert_dict_value(headers, "etag", ["etag"], send)
        await _assert_dict_value(headers, "expect", ["expect"], send)
        await _assert_dict_value(headers, "expires", ["expires"], send)
        await _assert_dict_value(headers, "forwarded", ["forwarded"], send)
        await _assert_dict_value(headers, "from", ["from"], send)
        # Skip host
        await _assert_dict_value(headers, "if-match", ["if-match"], send)
        await _assert_dict_value(
            headers, "if-modified-since", ["if-modified-since"], send
        )
        await _assert_dict_value(headers, "if-none-match", ["if-none-match"], send)
        await _assert_dict_value(headers, "if-range", ["if-range"], send)
        await _assert_dict_value(
            headers, "if-unmodified-since", ["if-unmodified-since"], send
        )
        await _assert_dict_value(headers, "last-modified", ["last-modified"], send)
        await _assert_dict_value(headers, "link", ["link"], send)
        await _assert_dict_value(headers, "location", ["location"], send)
        await _assert_dict_value(headers, "max-forwards", ["max-forwards"], send)
        await _assert_dict_value(headers, "origin", ["origin"], send)
        await _assert_dict_value(headers, "pragma", ["pragma"], send)
        await _assert_dict_value(
            headers, "proxy-authenticate", ["proxy-authenticate"], send
        )
        await _assert_dict_value(
            headers, "proxy-authorization", ["proxy-authorization"], send
        )
        await _assert_dict_value(headers, "public-key-pins", ["public-key-pins"], send)
        await _assert_dict_value(
            headers,
            "public-key-pins-report-only",
            ["public-key-pins-report-only"],
            send,
        )
        await _assert_dict_value(headers, "range", ["range"], send)
        await _assert_dict_value(headers, "referer", ["referer"], send)
        await _assert_dict_value(headers, "referrer-policy", ["referrer-policy"], send)
        await _assert_dict_value(headers, "refresh", ["refresh"], send)
        await _assert_dict_value(headers, "retry-after", ["retry-after"], send)
        await _assert_dict_value(
            headers, "sec-websocket-accept", ["sec-websocket-accept"], send
        )
        await _assert_dict_value(
            headers, "sec-websocket-extensions", ["sec-websocket-extensions"], send
        )
        await _assert_dict_value(
            headers, "sec-websocket-key", ["sec-websocket-key"], send
        )
        await _assert_dict_value(
            headers, "sec-websocket-protocol", ["sec-websocket-protocol"], send
        )
        await _assert_dict_value(
            headers, "sec-websocket-version", ["sec-websocket-version"], send
        )
        await _assert_dict_value(headers, "server", ["server"], send)
        await _assert_dict_value(headers, "set-cookie", ["set-cookie"], send)
        await _assert_dict_value(
            headers, "strict-transport-security", ["strict-transport-security"], send
        )
        # Skip te
        await _assert_dict_value(headers, "trailer", ["trailer"], send)
        # Skip transfer-encoding
        await _assert_dict_value(headers, "user-agent", ["user-agent"], send)
        # Skip upgrade
        await _assert_dict_value(
            headers, "upgrade-insecure-requests", ["upgrade-insecure-requests"], send
        )
        await _assert_dict_value(headers, "vary", ["vary"], send)
        await _assert_dict_value(headers, "via", ["via"], send)
        await _assert_dict_value(headers, "warning", ["warning"], send)
        await _assert_dict_value(
            headers, "www-authenticate", ["www-authenticate"], send
        )
        await _assert_dict_value(
            headers, "x-content-type-options", ["x-content-type-options"], send
        )
        await _assert_dict_value(
            headers, "x-dns-prefetch-control", ["x-dns-prefetch-control"], send
        )
        await _assert_dict_value(headers, "x-frame-options", ["x-frame-options"], send)
        await _assert_dict_value(
            headers, "x-xss-protection", ["x-xss-protection"], send
        )
        await _assert_dict_value(headers, "x-pyvoy", ["x-pyvoy", "x-pyvoy-2"], send)
    except AssertionFailed:
        return

    await _send_success(send)


async def _nihongo(
    scope: HTTPScope, _recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    try:
        await _assert_dict_value(
            scope, "raw_path", b"/%E6%97%A5%E6%9C%AC%E8%AA%9E", send
        )
    except AssertionFailed:
        return

    await _send_success(send)


async def _echo_scope(
    scope: HTTPScope, _recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    headers = dict(scope["headers"])
    extensions = scope["extensions"]
    assert extensions is not None  # noqa: S101

    await send(
        {
            "type": "http.response.start",
            "status": 200,
            "headers": [
                (b"x-scope-method", scope["method"].encode()),
                (b"x-scope-scheme", scope["scheme"].encode()),
                (b"x-scope-query", scope["query_string"]),
                (b"x-scope-content-length", headers.get(b"content-length", b"")),
                (b"x-scope-content-type", headers.get(b"content-type", b"")),
                (b"x-scope-http-version", scope["http_version"].encode()),
                (b"x-scope-path", scope["path"].encode()),
                (b"x-scope-root-path", scope["root_path"].encode()),
                (
                    b"x-scope-tls-version",
                    str(extensions.get("tls", {}).get("tls_version", "")).encode(),
                ),
                (
                    b"x-scope-tls-client-cert-name",
                    str(extensions.get("tls", {}).get("client_cert_name", "")).encode(),
                ),
            ],
            "trailers": False,
        }
    )
    await send({"type": "http.response.body", "body": b"", "more_body": False})


async def app(
    scope: HTTPScope, recv: ASGIReceiveCallable, send: ASGISendCallable
) -> None:
    path = scope["path"]
    if root_path := scope.get("root_path"):
        path = path.removeprefix(root_path)
    match path:
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
        case "/client-closed-before-response":
            await _client_closed_before_response(scope, recv, send)
        case "/client-closed-after-response-start":
            await _client_closed_after_response_start(scope, recv, send)
        case "/client-closed-after-trailers-start":
            await _client_closed_after_trailers_start(scope, recv, send)
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
        case "/bad-app-invalid-header-name":
            await _bad_app_invalid_header_name(scope, recv, send)
        case "/bad-app-invalid-header-value":
            await _bad_app_invalid_header_value(scope, recv, send)
        case "/print-logs":
            await _print_logs(scope, recv, send)
        case "/all-the-headers":
            await _all_the_headers(scope, recv, send)
        case "/日本語":
            await _nihongo(scope, recv, send)
        case "/echo-scope":
            await _echo_scope(scope, recv, send)
        case _:
            await _send_failure(f"unknown path: {scope['path']}", send)

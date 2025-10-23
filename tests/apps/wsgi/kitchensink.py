import time
from collections.abc import Iterable
from typing import cast
from wsgiref.types import InputStream as WSGIInputStream
from wsgiref.types import StartResponse, WSGIEnvironment


def _failure(msg: str, start_response: StartResponse) -> Iterable[bytes]:
    start_response("500 Internal Server Error", [("content-type", "text/plain")])
    return [b"Assertion Failure: " + msg.encode()]


def _headers_only(
    environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    if environ["REQUEST_METHOD"] != "GET":
        return _failure('environ["REQUEST_METHOD"] != "GET"', start_response)

    headers = {}
    for key, value in environ.items():
        if key.startswith("HTTP_"):
            headers[key[5:].replace("_", "-").lower()] = value
    if headers.get("accept") != "text/plain":
        return _failure('headers.get("accept") != "text/plain"', start_response)
    if headers.get("multiple") != "v1,v2":
        return _failure('headers.get("multiple") != "v1,v2"', start_response)

    request_body = cast("WSGIInputStream", environ["wsgi.input"])
    body = request_body.read()
    if body != b"":
        return _failure("body != b''", start_response)

    start_response("200 OK", [("content-type", "text/plain"), ("x-animal", "bear")])
    return []


def _request_body(
    environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    if environ["REQUEST_METHOD"] != "POST":
        return _failure('environ["REQUEST_METHOD"] != "POST"', start_response)

    request_body = cast("WSGIInputStream", environ["wsgi.input"])
    body = b""

    for _ in range(10):
        chunk = request_body.read(2)
        if not chunk:
            break
        body += chunk
    if request_body.read(1) != b"":
        return _failure("request_body.read(1) != b''", start_response)

    if body != b"Bear please":
        return _failure("body != b'Bear please'", start_response)

    start_response("200 OK", [("content-type", "text/plain"), ("x-animal", "bear")])
    return []


def _response_body(
    environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    if environ["REQUEST_METHOD"] != "GET":
        return _failure('environ["REQUEST_METHOD"] != "GET"', start_response)

    start_response("200 OK", [("content-type", "text/plain")])
    return [b"Hello ", b"world!"]


def _request_and_response_body(
    environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    if environ["REQUEST_METHOD"] != "POST":
        return _failure('environ["REQUEST_METHOD"] != "POST"', start_response)

    request_body = cast("WSGIInputStream", environ["wsgi.input"])
    body = request_body.read()
    if request_body.read() != b"":
        return _failure("request_body.read(1) != b''", start_response)

    if body != b"Bear please":
        return _failure("body != b'Bear please'", start_response)

    start_response("200 OK", [("content-type", "text/plain")])
    return [b"Yogi ", b"Bear"]


def _large_bodies(
    environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    request_body = cast("WSGIInputStream", environ["wsgi.input"])

    body = b""
    for _ in range(10000):
        chunk = request_body.read(1000)
        if not chunk:
            break
        body += chunk
    if request_body.read() != b"":
        return _failure("request_body.read(1) != b''", start_response)

    if body != b"A" * 1_000_000:
        return _failure(f"body != b'A' * 1_000_000 (len: {len(body)})", start_response)

    start_response("200 OK", [("content-type", "text/plain")])

    for _ in range(1000):
        yield b"B" * 1000


def _read_to_newline(body_stream: WSGIInputStream) -> bytes:
    body = b""
    while True:
        chunk = body_stream.read(1024)
        if not chunk:
            return body
        body += chunk
        if body and body[-1] == b"\n"[0]:
            return body[:-1]


def _bidi_stream(
    environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    start_response(
        "202 Accepted", [("content-type", "text/plain"), ("x-animal", "bear")]
    )

    request_body = cast("WSGIInputStream", environ["wsgi.input"])
    yield b"Who are you?"
    body = _read_to_newline(request_body)
    yield b"Hi " + body + b". What do you want to do?"
    body = request_body.read()
    yield b"Let's " + body + b"!"


def _exception_before_response(
    _environ: WSGIEnvironment, _start_response: StartResponse
) -> Iterable[bytes]:
    msg = "We have failed hard"
    raise RuntimeError(msg)


def _exception_after_response_headers(
    _environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    start_response("200 OK", [("content-type", "text/plain")])
    yield b""
    msg = "We have failed hard"
    raise RuntimeError(msg)


def _exception_after_response_body(
    _environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    start_response("200 OK", [("content-type", "text/plain")])
    yield b"Hello World!!!"
    msg = "We have failed hard"
    raise RuntimeError(msg)


def _controlled(
    environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    sleep_ms = int(environ.get("HTTP_X_SLEEP_MS", 0))
    response_bytes = int(environ.get("HTTP_X_RESPONSE_BYTES", 0))

    cast("WSGIInputStream", environ["wsgi.input"]).read()

    if sleep_ms > 0:
        time.sleep(sleep_ms / 1000.0)

    start_response(
        "200 OK",
        [("content-type", "text/plain"), ("content-length", str(response_bytes))],
    )
    if response_bytes > 0:
        chunk = b"A" * response_bytes
        return [chunk]
    return []


def app(environ: WSGIEnvironment, start_response: StartResponse) -> Iterable[bytes]:
    match environ["PATH_INFO"]:
        case "/headers-only":
            return _headers_only(environ, start_response)
        case "/request-body":
            return _request_body(environ, start_response)
        case "/response-body":
            return _response_body(environ, start_response)
        case "/request-and-response-body":
            return _request_and_response_body(environ, start_response)
        case "/large-bodies":
            return _large_bodies(environ, start_response)
        case "/bidi-stream":
            return _bidi_stream(environ, start_response)
        case "/exception-before-response":
            return _exception_before_response(environ, start_response)
        case "/exception-after-response-headers":
            return _exception_after_response_headers(environ, start_response)
        case "/exception-after-response-body":
            return _exception_after_response_body(environ, start_response)
        case "/controlled":
            return _controlled(environ, start_response)
        case _:
            return _failure(f"Unknown path {environ['PATH_INFO']}", start_response)

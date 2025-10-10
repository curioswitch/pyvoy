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
        case _:
            return _failure(f"Unknown path {environ['PATH_INFO']}", start_response)

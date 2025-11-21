from __future__ import annotations

import sys
import time
from typing import TYPE_CHECKING, TypeVar, cast
from wsgiref.validate import validator

if TYPE_CHECKING:
    from collections.abc import Iterable

    if sys.version_info >= (3, 11):
        from wsgiref.types import ErrorStream as WSGIErrorStream
        from wsgiref.types import InputStream as WSGIInputStream
        from wsgiref.types import StartResponse, WSGIEnvironment
    else:
        from _typeshed.wsgi import ErrorStream as WSGIErrorStream
        from _typeshed.wsgi import InputStream as WSGIInputStream
        from _typeshed.wsgi import StartResponse, WSGIEnvironment


def _failure(msg: str, start_response: StartResponse) -> Iterable[bytes]:
    start_response("500 Internal Server Error", [("content-type", "text/plain")])
    return [b"Assertion Failure: " + msg.encode()]


class AssertionFailed(Exception):
    def __init__(self, response: Iterable[bytes]) -> None:
        super().__init__()
        self.response = response


K = TypeVar("K")
V = TypeVar("V")


def _assert_dict_value(
    actual: dict[K, V], key: K, expected: V, start_response: StartResponse
) -> None:
    if key not in actual:
        raise AssertionFailed(
            _failure(f"Key {key!r} not found in headers", start_response)
        )
    if actual[key] != expected:
        raise AssertionFailed(
            _failure(
                f"Value for key {key!r} is {actual[key]!r}, expected {expected!r}",
                start_response,
            )
        )


def _success(start_response: StartResponse) -> Iterable[bytes]:
    start_response("200 OK", [("content-type", "text/plain")])
    return [b"Ok"]


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
    if request_body.read(0) != b"":
        return _failure("request_body.read(0) != b''", start_response)

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
    # While wsgi typing doesn't allow None for size, servers conventionally accept it
    # and we should to.
    body = request_body.read(cast("int", None))
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


def _bidi_stream(
    environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    start_response(
        "202 Accepted", [("content-type", "text/plain"), ("x-animal", "bear")]
    )

    request_body = cast("WSGIInputStream", environ["wsgi.input"])
    yield b"Who are you?"
    body = request_body.readline()[:-1]
    yield b"Hi " + body + b". What do you want to do?"
    body = request_body.read()
    yield b"Let's " + body + b"!"


def _exception_before_response(
    _environ: WSGIEnvironment, _start_response: StartResponse
) -> Iterable[bytes]:
    msg = "We have failed before the response"
    raise RuntimeError(msg)


def _exception_after_response_headers(
    _environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    start_response("200 OK", [("content-type", "text/plain")])
    yield b""
    msg = "We have failed after response headers"
    raise RuntimeError(msg)


def _exception_after_response_body(
    _environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    start_response("200 OK", [("content-type", "text/plain")])
    yield b"Hello World!!!"
    msg = "We have failed after response body"
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


def _print_logs(
    _environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    print("This is a stdout print", flush=True)  # noqa: T201
    print("This is a stderr print", file=sys.stderr)  # noqa: T201
    start_response("200 OK", [("content-type", "text/plain"), ("x-animal", "bear")])
    return []


def _all_the_headers(
    environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    try:
        host = cast("str", environ.get("HTTP_HOST", ""))
        if not host:
            return _failure("HTTP_HOST not found in environ", start_response)
        if not host.startswith("127.0.0.1:"):
            return _failure(
                f"HTTP_HOST does not start with '127.0.0.1:', was: {host!r}",
                start_response,
            )
        _assert_dict_value(environ, "HTTP_ACCEPT", "accept", start_response)
        _assert_dict_value(
            environ, "HTTP_ACCEPT_CHARSET", "accept-charset", start_response
        )
        _assert_dict_value(
            environ, "HTTP_ACCEPT_ENCODING", "accept-encoding", start_response
        )
        _assert_dict_value(
            environ, "HTTP_ACCEPT_LANGUAGE", "accept-language", start_response
        )
        _assert_dict_value(
            environ, "HTTP_ACCEPT_RANGES", "accept-ranges", start_response
        )
        _assert_dict_value(
            environ,
            "HTTP_ACCESS_CONTROL_ALLOW_CREDENTIALS",
            "access-control-allow-credentials",
            start_response,
        )
        _assert_dict_value(
            environ,
            "HTTP_ACCESS_CONTROL_ALLOW_HEADERS",
            "access-control-allow-headers",
            start_response,
        )
        _assert_dict_value(
            environ,
            "HTTP_ACCESS_CONTROL_ALLOW_METHODS",
            "access-control-allow-methods",
            start_response,
        )
        _assert_dict_value(
            environ,
            "HTTP_ACCESS_CONTROL_ALLOW_ORIGIN",
            "access-control-allow-origin",
            start_response,
        )
        _assert_dict_value(
            environ,
            "HTTP_ACCESS_CONTROL_EXPOSE_HEADERS",
            "access-control-expose-headers",
            start_response,
        )
        _assert_dict_value(
            environ,
            "HTTP_ACCESS_CONTROL_MAX_AGE",
            "access-control-max-age",
            start_response,
        )
        _assert_dict_value(
            environ,
            "HTTP_ACCESS_CONTROL_REQUEST_HEADERS",
            "access-control-request-headers",
            start_response,
        )
        _assert_dict_value(
            environ,
            "HTTP_ACCESS_CONTROL_REQUEST_METHOD",
            "access-control-request-method",
            start_response,
        )
        _assert_dict_value(environ, "HTTP_AGE", "age", start_response)
        _assert_dict_value(environ, "HTTP_ALLOW", "allow", start_response)
        _assert_dict_value(environ, "HTTP_ALT_SVC", "alt-svc", start_response)
        _assert_dict_value(
            environ, "HTTP_AUTHORIZATION", "authorization", start_response
        )
        _assert_dict_value(
            environ, "HTTP_CACHE_CONTROL", "cache-control", start_response
        )
        _assert_dict_value(environ, "HTTP_CACHE_STATUS", "cache-status", start_response)
        _assert_dict_value(
            environ, "HTTP_CDN_CACHE_CONTROL", "cdn-cache-control", start_response
        )
        # Skip connection
        _assert_dict_value(
            environ, "HTTP_CONTENT_DISPOSITION", "content-disposition", start_response
        )
        _assert_dict_value(
            environ, "HTTP_CONTENT_ENCODING", "content-encoding", start_response
        )
        _assert_dict_value(
            environ, "HTTP_CONTENT_LANGUAGE", "content-language", start_response
        )
        # Skip content-length
        _assert_dict_value(
            environ, "HTTP_CONTENT_LOCATION", "content-location", start_response
        )
        _assert_dict_value(
            environ, "HTTP_CONTENT_RANGE", "content-range", start_response
        )
        _assert_dict_value(
            environ,
            "HTTP_CONTENT_SECURITY_POLICY",
            "content-security-policy",
            start_response,
        )
        _assert_dict_value(
            environ,
            "HTTP_CONTENT_SECURITY_POLICY_REPORT_ONLY",
            "content-security-policy-report-only",
            start_response,
        )
        # Doesn't follow the HTTP_ prefix convention
        _assert_dict_value(environ, "CONTENT_TYPE", "content-type", start_response)
        _assert_dict_value(environ, "HTTP_COOKIE", "cookie", start_response)
        _assert_dict_value(environ, "HTTP_DNT", "dnt", start_response)
        _assert_dict_value(environ, "HTTP_DATE", "date", start_response)
        _assert_dict_value(environ, "HTTP_ETAG", "etag", start_response)
        _assert_dict_value(environ, "HTTP_EXPECT", "expect", start_response)
        _assert_dict_value(environ, "HTTP_EXPIRES", "expires", start_response)
        _assert_dict_value(environ, "HTTP_FORWARDED", "forwarded", start_response)
        _assert_dict_value(environ, "HTTP_FROM", "from", start_response)
        # Skip host
        _assert_dict_value(environ, "HTTP_IF_MATCH", "if-match", start_response)
        _assert_dict_value(
            environ, "HTTP_IF_MODIFIED_SINCE", "if-modified-since", start_response
        )
        _assert_dict_value(
            environ, "HTTP_IF_NONE_MATCH", "if-none-match", start_response
        )
        _assert_dict_value(environ, "HTTP_IF_RANGE", "if-range", start_response)
        _assert_dict_value(
            environ, "HTTP_IF_UNMODIFIED_SINCE", "if-unmodified-since", start_response
        )
        _assert_dict_value(
            environ, "HTTP_LAST_MODIFIED", "last-modified", start_response
        )
        _assert_dict_value(environ, "HTTP_LINK", "link", start_response)
        _assert_dict_value(environ, "HTTP_LOCATION", "location", start_response)
        _assert_dict_value(environ, "HTTP_MAX_FORWARDS", "max-forwards", start_response)
        _assert_dict_value(environ, "HTTP_ORIGIN", "origin", start_response)
        _assert_dict_value(environ, "HTTP_PRAGMA", "pragma", start_response)
        _assert_dict_value(
            environ, "HTTP_PROXY_AUTHENTICATE", "proxy-authenticate", start_response
        )
        _assert_dict_value(
            environ, "HTTP_PROXY_AUTHORIZATION", "proxy-authorization", start_response
        )
        _assert_dict_value(
            environ, "HTTP_PUBLIC_KEY_PINS", "public-key-pins", start_response
        )
        _assert_dict_value(
            environ,
            "HTTP_PUBLIC_KEY_PINS_REPORT_ONLY",
            "public-key-pins-report-only",
            start_response,
        )
        _assert_dict_value(environ, "HTTP_RANGE", "range", start_response)
        _assert_dict_value(environ, "HTTP_REFERER", "referer", start_response)
        _assert_dict_value(
            environ, "HTTP_REFERRER_POLICY", "referrer-policy", start_response
        )
        _assert_dict_value(environ, "HTTP_REFRESH", "refresh", start_response)
        _assert_dict_value(environ, "HTTP_RETRY_AFTER", "retry-after", start_response)
        _assert_dict_value(
            environ, "HTTP_SEC_WEBSOCKET_ACCEPT", "sec-websocket-accept", start_response
        )
        _assert_dict_value(
            environ,
            "HTTP_SEC_WEBSOCKET_EXTENSIONS",
            "sec-websocket-extensions",
            start_response,
        )
        _assert_dict_value(
            environ, "HTTP_SEC_WEBSOCKET_KEY", "sec-websocket-key", start_response
        )
        _assert_dict_value(
            environ,
            "HTTP_SEC_WEBSOCKET_PROTOCOL",
            "sec-websocket-protocol",
            start_response,
        )
        _assert_dict_value(
            environ,
            "HTTP_SEC_WEBSOCKET_VERSION",
            "sec-websocket-version",
            start_response,
        )
        _assert_dict_value(environ, "HTTP_SERVER", "server", start_response)
        _assert_dict_value(environ, "HTTP_SET_COOKIE", "set-cookie", start_response)
        _assert_dict_value(
            environ,
            "HTTP_STRICT_TRANSPORT_SECURITY",
            "strict-transport-security",
            start_response,
        )
        # Skip te
        _assert_dict_value(environ, "HTTP_TRAILER", "trailer", start_response)
        # Skip transfer-encoding
        _assert_dict_value(environ, "HTTP_USER_AGENT", "user-agent", start_response)
        # Skip upgrade
        _assert_dict_value(
            environ,
            "HTTP_UPGRADE_INSECURE_REQUESTS",
            "upgrade-insecure-requests",
            start_response,
        )
        _assert_dict_value(environ, "HTTP_VARY", "vary", start_response)
        _assert_dict_value(environ, "HTTP_VIA", "via", start_response)
        _assert_dict_value(environ, "HTTP_WARNING", "warning", start_response)
        _assert_dict_value(
            environ, "HTTP_WWW_AUTHENTICATE", "www-authenticate", start_response
        )
        _assert_dict_value(
            environ,
            "HTTP_X_CONTENT_TYPE_OPTIONS",
            "x-content-type-options",
            start_response,
        )
        _assert_dict_value(
            environ,
            "HTTP_X_DNS_PREFETCH_CONTROL",
            "x-dns-prefetch-control",
            start_response,
        )
        _assert_dict_value(
            environ, "HTTP_X_FRAME_OPTIONS", "x-frame-options", start_response
        )
        _assert_dict_value(
            environ, "HTTP_X_XSS_PROTECTION", "x-xss-protection", start_response
        )
        _assert_dict_value(environ, "HTTP_X_PYVOY", "x-pyvoy,x-pyvoy-2", start_response)
    except AssertionFailed as e:
        return e.response

    return _success(start_response)


def _nihongo(
    _environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    # Can't actually get raw path in WSGI
    return _success(start_response)


def _echo_scope(
    environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    start_response(
        "200 OK",
        [
            ("content-type", "text/plain"),
            ("x-scope-method", environ["REQUEST_METHOD"]),
            ("x-scope-scheme", environ.get("wsgi.url_scheme", "")),
            ("x-scope-query", environ.get("QUERY_STRING", "")),
            ("x-scope-content-length", environ.get("CONTENT_LENGTH", "")),
            ("x-scope-content-type", environ.get("CONTENT_TYPE", "")),
            ("x-scope-http-version", environ.get("SERVER_PROTOCOL", "")),
            ("x-scope-path", environ.get("PATH_INFO", "")),
            ("x-scope-root-path", environ.get("SCRIPT_NAME", "")),
            ("x-scope-tls-version", environ.get("wsgi.ext.tls.tls_version", "")),
            (
                "x-scope-tls-client-cert-name",
                environ.get("wsgi.ext.tls.client_cert_name", ""),
            ),
        ],
    )
    return [b"Ok"]


def _readline(
    environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    request_body = cast("WSGIInputStream", environ["wsgi.input"])
    line = request_body.readline()
    if line != b"Hello\n":
        return _failure("line != b'Hello\\n'", start_response)
    line = request_body.readline(0)
    if line != b"":
        return _failure("line != b''", start_response)
    line = request_body.readline(2)
    if line != b"Wo":
        return _failure("line != b'Wo'", start_response)
    line = request_body.readline(0)
    if line != b"":
        return _failure("line != b''", start_response)
    line = request_body.readline(-1)
    if line != b"rld\n":
        return _failure("line != b'rld\\n'", start_response)
    # While wsgi typing doesn't allow None for size, servers conventionally accept it
    # and we should to.
    line = request_body.readline(cast("int", None))
    if line != b"Goodbye":
        return _failure("line != b'Goodbye'", start_response)

    return _success(start_response)


def _iterlines(
    environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    request_body = cast("WSGIInputStream", environ["wsgi.input"])
    lines = list(iter(request_body))
    if lines != [b"Animal\n", b"Bear\n", b"Cat"]:
        return _failure(
            "lines != [b'Hello\\n', b'World\\n', b'Goodbye']", start_response
        )

    return _success(start_response)


def _readlines(
    environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    request_body = cast("WSGIInputStream", environ["wsgi.input"])
    lines = request_body.readlines()
    if lines != [b"Food\n", b"Pizza\n", b"Burrito"]:
        return _failure(
            "lines != [b'Hello\\n', b'World\\n', b'Goodbye']", start_response
        )

    return _success(start_response)


def _write_callable(
    _environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    write = start_response("200 OK", [("content-type", "text/plain")])
    write(b"Hello")
    write(b" World")
    return [b" and Goodbye!"]


def _errors_output(
    environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    errors = cast("WSGIErrorStream", environ["wsgi.errors"])

    n = errors.write("Hello World\n")
    if n != len("Hello World\n"):
        return _failure("n != len('Hello World\\n')", start_response)
    n = errors.write("")
    if n != 0:
        return _failure("n != 0", start_response)
    errors.write("Goodbye Earth\n\n\nHello again\n")

    errors.write("Animal: ")
    errors.write("Bear\n")
    errors.write("Food: ")
    errors.write("Pizza\nDrink: ")
    errors.write("Beer\n\n\n")

    errors.write("Country: \n\n")
    errors.flush()
    errors.write("Japan")

    errors.writelines(["Line 1\n", "", "Line 2", "Line 3\n"])

    return _success(start_response)


def _multiple_start_response(
    _environ: WSGIEnvironment, start_response: StartResponse
) -> Iterable[bytes]:
    write = start_response("200 OK", [("content-type", "text/plain")])
    try:
        start_response("200 OK", [("content-type", "text/plain")])
    except RuntimeError as e:
        if str(e) != "start_response called twice without exc_info":
            start_response(
                "500 Internal Server Error",
                [("content-type", "text/plain")],
                sys.exc_info(),
            )
            return [b"str(e) != 'start_response called twice without exc_info'"]
    else:
        start_response(
            "500 Internal Server Error",
            [("content-type", "text/plain")],
            # WSGI only defines how to handle the presence or absence of exc_info, not
            # if it is filled with None as it is here. We take advantage of it to
            # allow changing the headers here.
            sys.exc_info(),
        )
        return [b"Expected RuntimeError for double start_response not raised"]

    try:
        msg = "ignored"
        raise ValueError(msg)  # noqa: TRY301
    except ValueError:
        start_response(
            "200 OK",
            [("content-type", "text/plain"), ("x-animal", "bear")],
            sys.exc_info(),
        )

    write(b"Ok?")

    try:
        msg = "ignored"
        raise ValueError(msg)  # noqa: TRY301
    except ValueError:
        try:
            start_response(
                "200 OK",
                [("content-type", "text/plain"), ("x-animal", "cat")],
                sys.exc_info(),
            )
        except ValueError as e:
            if str(e) != msg:
                return [f" No, expected same ValueError thrown but got {e!s}".encode()]

    return [b" Yes"]


def _no_start_response(
    _environ: WSGIEnvironment, _start_response: StartResponse
) -> Iterable[bytes]:
    return [b"OK"]


def app(environ: WSGIEnvironment, start_response: StartResponse) -> Iterable[bytes]:
    path = cast("str", environ["PATH_INFO"]).encode("latin-1").decode("utf-8")
    match path:
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
        case "/print-logs":
            return _print_logs(environ, start_response)
        case "/all-the-headers":
            return _all_the_headers(environ, start_response)
        case "/日本語":
            return _nihongo(environ, start_response)
        case "/echo-scope":
            # Most checks by the wsgiref validator are about the application, not server.
            # Some are quite strict such as not allowing .read() calls even though it is
            # supported by WSGI including within its type definitions. We're mostly interested
            # in environ and type checks and it is enough to apply it only to this handler to
            # have more flexibility in others.
            return validator(_echo_scope)(environ, start_response)
        case "/readline":
            return _readline(environ, start_response)
        case "/iterlines":
            return _iterlines(environ, start_response)
        case "/readlines":
            return _readlines(environ, start_response)
        case "/write-callable":
            return _write_callable(environ, start_response)
        case "/errors-output":
            return _errors_output(environ, start_response)
        case "/multiple-start-response":
            return _multiple_start_response(environ, start_response)
        case "/no-start-response":
            return _no_start_response(environ, start_response)
        case _:
            return _failure(f"Unknown path {environ['PATH_INFO']}", start_response)

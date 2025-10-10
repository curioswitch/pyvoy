from collections.abc import Iterator

import httpx
import pytest

from pyvoy import Interface, PyvoyServer


@pytest.fixture(scope="module")
def url_asgi() -> Iterator[str]:
    with PyvoyServer("tests.apps.asgi.kitchensink") as server:
        yield f"http://{server.listener_address}:{server.listener_port}"


@pytest.fixture(scope="module")
def url_wsgi() -> Iterator[str]:
    with PyvoyServer("tests.apps.wsgi.kitchensink", interface="wsgi") as server:
        yield f"http://{server.listener_address}:{server.listener_port}"


@pytest.fixture(params=["asgi", "wsgi"])
def interface(request: pytest.FixtureRequest) -> Interface:
    return request.param


@pytest.fixture
def url(interface: Interface, url_asgi: str, url_wsgi: str) -> str:
    match interface:
        case "asgi":
            return url_asgi
        case "wsgi":
            return url_wsgi


def test_headers_only(url: str) -> None:
    response = httpx.get(
        f"{url}/headers-only",
        headers=(("Accept", "text/plain"), ("Multiple", "v1"), ("Multiple", "v2")),
    )
    assert response.status_code == 200, response.text
    assert response.headers["x-animal"] == "bear"
    assert response.headers["content-type"] == "text/plain"
    assert response.content == b""


def test_request_body(url: str) -> None:
    response = httpx.post(f"{url}/request-body", content="Bear please")
    assert response.status_code == 200, response.text
    assert response.headers["x-animal"] == "bear"
    assert response.headers["content-type"] == "text/plain"
    assert response.content == b""


def test_response_body(url: str) -> None:
    response = httpx.get(f"{url}/response-body")
    assert response.status_code == 200, response.text
    assert response.headers["content-type"] == "text/plain"
    assert response.content == b"Hello world!"


def test_request_and_response_body(url: str) -> None:
    response = httpx.post(f"{url}/request-and-response-body", content="Bear please")
    assert response.status_code == 200, response.text
    assert response.headers["content-type"] == "text/plain"
    assert response.content == b"Yogi Bear"


def test_large_bodies(url: str) -> None:
    response = httpx.post(f"{url}/large-bodies", content=b"A" * 1_000_000)
    assert response.status_code == 200, response.text
    assert response.headers["content-type"] == "text/plain"
    assert response.content == b"B" * 1_000_000


def test_empty_request_body_http2(url: str) -> None:
    with httpx.Client(http2=True, http1=False) as client:
        response = client.post(f"{url}/controlled", content=b"")
    assert response.status_code == 200, response.text
    assert response.content == b""


def test_exception_before_response(url: str) -> None:
    response = httpx.get(f"{url}/exception-before-response")
    assert response.status_code == 500
    assert response.headers["content-type"] == "text/plain; charset=utf-8"
    assert response.content == b"Internal Server Error"


# Because we only flush headers when the first body message is available,
# both below tests end up behaving the same but it may still be worth keeping
# them separate in case of regressions.


def test_exception_after_response_headers(url: str) -> None:
    with pytest.raises(httpx.RemoteProtocolError) as exc_info:
        httpx.get(f"{url}/exception-after-response-headers")
    assert "peer closed connection without sending complete message body" in str(
        exc_info.value
    )


def test_exception_after_response_body(url: str) -> None:
    with pytest.raises(httpx.RemoteProtocolError) as exc_info:
        httpx.get(f"{url}/exception-after-response-body")
    assert "peer closed connection without sending complete message body" in str(
        exc_info.value
    )


# Not possible to model with WSGI
def test_exception_after_response_complete(url_asgi: str) -> None:
    response = httpx.get(f"{url_asgi}/exception-after-response-complete")
    assert response.status_code == 200, response.text
    assert response.headers["content-type"] == "text/plain"
    assert response.content == b"Hello World!!!"
    # TODO: Check server logs

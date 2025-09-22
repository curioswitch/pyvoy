import httpx
import pytest


def test_headers_only(kitchensink_url: str) -> None:
    response = httpx.get(
        f"{kitchensink_url}/headers-only",
        headers=(("Accept", "text/plain"), ("Multiple", "v1"), ("Multiple", "v2")),
    )
    assert response.status_code == 200, response.text
    assert response.headers["x-animal"] == "bear"
    assert response.headers["content-type"] == "text/plain"
    assert response.content == b""


def test_request_body(kitchensink_url: str) -> None:
    response = httpx.post(f"{kitchensink_url}/request-body", content="Bear please")
    assert response.status_code == 200, response.text
    assert response.headers["x-animal"] == "bear"
    assert response.headers["content-type"] == "text/plain"
    assert response.content == b""


def test_response_body(kitchensink_url: str) -> None:
    response = httpx.get(f"{kitchensink_url}/response-body")
    assert response.status_code == 200, response.text
    assert response.headers["content-type"] == "text/plain"
    assert response.content == b"Hello world!"


def test_request_and_response_body(kitchensink_url: str) -> None:
    response = httpx.post(
        f"{kitchensink_url}/request-and-response-body", content="Bear please"
    )
    assert response.status_code == 200, response.text
    assert response.headers["content-type"] == "text/plain"
    assert response.content == b"Yogi Bear"


def test_large_bodies(kitchensink_url: str) -> None:
    response = httpx.post(f"{kitchensink_url}/large-bodies", content=b"A" * 1_000_000)
    assert response.status_code == 200, response.text
    assert response.headers["content-type"] == "text/plain"
    assert response.content == b"B" * 1_000_000


def test_empty_request_body_http2(kitchensink_url: str) -> None:
    with httpx.Client(http2=True, http1=False) as client:
        response = client.post(f"{kitchensink_url}/controlled", content=b"")
    assert response.status_code == 200, response.text
    assert response.content == b""


def test_exception_before_response(kitchensink_url: str) -> None:
    response = httpx.get(f"{kitchensink_url}/exception-before-response")
    assert response.status_code == 500
    assert response.headers["content-type"] == "text/plain; charset=utf-8"
    assert response.content == b"Internal Server Error"


def test_exception_after_response_headers(kitchensink_url: str) -> None:
    with pytest.raises(httpx.RemoteProtocolError) as exc_info:
        httpx.post(
            f"{kitchensink_url}/exception-after-response-headers", content="Bear please"
        )
    assert "peer closed connection without sending complete message body" in str(
        exc_info.value
    )


def test_exception_after_response_body(kitchensink_url: str) -> None:
    with pytest.raises(httpx.RemoteProtocolError) as exc_info:
        httpx.get(f"{kitchensink_url}/exception-after-response-body")
    assert "peer closed connection without sending complete message body" in str(
        exc_info.value
    )


def test_exception_after_response_complete(kitchensink_url: str) -> None:
    response = httpx.get(f"{kitchensink_url}/exception-after-response-complete")
    assert response.status_code == 200, response.text
    assert response.headers["content-type"] == "text/plain"
    assert response.content == b"Hello World!!!"
    # TODO: Check server logs

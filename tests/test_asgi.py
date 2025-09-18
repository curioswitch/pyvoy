from collections.abc import Iterator

import httpx
import pytest

from pyvoy import PyvoyServer


@pytest.fixture
def kitchensink_url() -> Iterator[str]:
    with PyvoyServer("tests.apps.kitchensink") as server:
        yield f"http://{server.listener_address}:{server.listener_port}"


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

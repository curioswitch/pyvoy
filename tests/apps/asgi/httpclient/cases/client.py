from __future__ import annotations

from pyqwest import Client, HTTPVersion


def supports_trailers(http_version: HTTPVersion | None, url: str) -> bool:
    # Currently reqwest trailers patch does not apply to HTTP/3.
    return http_version != HTTPVersion.HTTP1 or (
        http_version is None and url.startswith("https://")
    )


async def get(client: Client, url: str) -> None:
    url = f"{url}/echo"
    resp = await client.get(url, params={"foo": "bar"})
    assert resp.status == 200
    assert resp.headers["x-echo-method"] == "GET"
    assert resp.headers["x-echo-query-string"] == "foo=bar"
    assert resp.content == b""
    assert len(resp.trailers) == 0


async def post(client: Client, url: str, http_version: HTTPVersion | None) -> None:
    url = f"{url}/echo"
    headers = [("content-type", "text/plain"), ("te", "trailers")]
    req_content = b"Hello, World!"
    resp = await client.post(url, headers, req_content, params={"foo": "bar"})
    assert resp.status == 200
    assert resp.headers["x-echo-method"] == "POST"
    assert resp.headers["x-echo-query-string"] == "foo=bar"
    assert resp.headers["x-echo-content-type"] == "text/plain"
    assert resp.headers.getall("x-echo-content-type") == ["text/plain"]
    assert resp.content == b"Hello, World!"
    if supports_trailers(http_version, url):
        assert resp.trailers["x-echo-trailer"] == "last info"
    else:
        assert len(resp.trailers) == 0

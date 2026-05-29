from __future__ import annotations

import asyncio
import json
from collections.abc import AsyncIterator
from urllib.parse import parse_qs

from pyqwest import Client, FullResponse, Headers, HTTPVersion, SyncClient


def supports_trailers(http_version: HTTPVersion | None, url: str) -> bool:
    # Currently reqwest trailers patch does not apply to HTTP/3.
    return http_version != HTTPVersion.HTTP1 or (
        http_version is None and url.startswith("https://")
    )


async def request_body(queue: asyncio.Queue) -> AsyncIterator[bytes]:
    while True:
        item: bytes | None = await queue.get()
        if item is None:
            return
        yield item


async def basic(
    client: Client | SyncClient,
    url: str,
    _http_version: HTTPVersion | None,
    server_port: int,
) -> None:
    method = "POST"
    url = f"{url}/echo"
    headers = [
        ("content-type", "text/plain"),
        ("x-hello", "rust"),
        ("x-hello", "python"),
    ]
    req_content = b"Hello, World!"
    if isinstance(client, SyncClient):

        def run():
            with client.stream(
                method, url, headers, req_content, params={"foo": "bar"}
            ) as resp:
                content = b"".join(resp.content)
            return (resp, content)

        resp, content = await asyncio.to_thread(run)
    else:
        async with client.stream(
            method, url, headers, req_content, params={"foo": "bar"}
        ) as resp:
            content = b""
            async for chunk in resp.content:
                content += chunk
    assert resp.status == 200
    assert resp.headers["x-echo-host"] == f"localhost:{server_port}"
    assert resp.headers["x-echo-method"] == "POST"
    assert resp.headers["x-echo-query-string"] == "foo=bar"
    assert resp.headers["x-echo-content-type"] == "text/plain"
    assert resp.headers.getall("x-echo-content-type") == ["text/plain"]
    assert resp.headers["x-echo-x-hello"] == "rust"
    assert resp.headers.getall("x-echo-x-hello") == ["rust", "python"]
    assert content == b"Hello, World!"
    # Didn't send te so should be no trailers
    assert len(resp.trailers) == 0
    # GAP: Dynamic modules do not currently expose stream info to populate resp.http_version
    # if http_version is not None:
    #     assert resp.http_version == http_version
    # else:
    #     if url.startswith("https://"):
    #         # Currently it seems HTTP/3 is not added to ALPN and must be explicitly
    #         # set when creating a Client.
    #         assert resp.http_version == HTTPVersion.HTTP2
    #     else:
    #         assert resp.http_version == HTTPVersion.HTTP1


async def iterable_body(client: Client | SyncClient, url: str) -> None:
    method = "POST"
    url = f"{url}/echo"
    if isinstance(client, SyncClient):

        def run():
            with client.stream(method, url, content=[b"Hello, ", b"World!"]) as resp:
                content = b"".join(resp.content)
            return (resp, content)

        resp, content = await asyncio.to_thread(run)
    else:

        async def req_content() -> AsyncIterator[bytes]:
            yield b"Hello, "
            yield b"World!"

        async with client.stream(method, url, content=req_content()) as resp:
            content = b""
            async for chunk in resp.content:
                content += chunk
    assert resp.status == 200
    assert content == b"Hello, World!"


async def empty_request(client: Client | SyncClient, url: str) -> None:
    method = "GET"
    url = f"{url}/echo"
    if isinstance(client, SyncClient):

        def run():
            with client.stream(method, url) as resp:
                content = b"".join(resp.content)
            return (resp, content)

        resp, content = await asyncio.to_thread(run)
    else:
        async with client.stream(method, url) as resp:
            content = b""
            async for chunk in resp.content:
                content += chunk
    assert resp.status == 200
    assert content == b""


async def test_bidi(
    async_client: Client, url: str, http_version: HTTPVersion | None
) -> None:
    client = async_client
    queue = asyncio.Queue()

    async with client.stream(
        "POST",
        f"{url}/echo",
        headers=Headers({"content-type": "text/plain", "te": "trailers"}),
        content=request_body(queue),
    ) as resp:
        assert resp.status == 200
        content = resp.content
        await queue.put(b"Hello!")
        chunk = await anext(content)
        assert chunk == b"Hello!"
        await queue.put(b" World!")
        chunk = await anext(content)
        assert chunk == b" World!"
        await queue.put(None)
        chunk = await anext(content, None)
        assert chunk is None
        if supports_trailers(http_version, url):
            assert resp.trailers["x-echo-trailer"] == "last info"
        else:
            assert len(resp.trailers) == 0


async def large_body(
    client: Client | SyncClient, url: str, http_version: HTTPVersion | None
) -> None:
    method = "POST"
    url = f"{url}/echo"
    headers = Headers(
        [
            ("content-type", "text/plain"),
            ("x-hello", "rust"),
            ("x-hello", "python"),
            ("te", "trailers"),
        ]
    )
    if isinstance(client, SyncClient):

        def run():
            with client.stream(method, url, headers, [b"Hello!"] * 100) as resp:
                content = b"".join(resp.content)
            return (resp, content)

        resp, content = await asyncio.to_thread(run)
    else:

        async def async_req_content() -> AsyncIterator[bytes]:
            for _ in range(100):
                yield b"Hello!"

        async with client.stream(method, url, headers, async_req_content()) as resp:
            content = b""
            async for chunk in resp.content:
                content += chunk
    assert resp.status == 200
    assert resp.headers["x-echo-content-type"] == "text/plain"
    assert resp.headers.getall("x-echo-content-type") == ["text/plain"]
    assert resp.headers["x-echo-x-hello"] == "rust"
    assert resp.headers.getall("x-echo-x-hello") == ["rust", "python"]
    assert content == b"Hello!" * 100, len(content)
    if supports_trailers(http_version, url):
        assert resp.trailers["x-echo-trailer"] == "last info"
    else:
        assert len(resp.trailers) == 0


async def readall(client: Client | SyncClient, url: str) -> None:
    method = "POST"
    url = f"{url}/read_all"
    headers = Headers([("content-type", "text/plain")])
    if isinstance(client, SyncClient):

        def run():
            with client.stream(method, url, headers, [b"Hello!"] * 100) as resp:
                content = b"".join(resp.content)
            return (resp, content)

        resp, content = await asyncio.to_thread(run)
    else:

        async def async_req_content() -> AsyncIterator[bytes]:
            for _ in range(100):
                yield b"Hello!"

        async with client.stream(method, url, headers, async_req_content()) as resp:
            content = b""
            async for chunk in resp.content:
                content += chunk
    assert resp.status == 200
    assert content == b"Hello!" * 100, len(content)


async def execute(client: Client | SyncClient, url: str) -> None:
    method = "POST"
    url = f"{url}/echo"
    headers = [
        ("content-type", "text/plain"),
        ("x-hello", "rust"),
        ("x-hello", "python"),
    ]
    req_content = b"Hello, World!"
    if isinstance(client, SyncClient):
        resp = await asyncio.to_thread(
            client.execute, method, url, headers, req_content, params={"foo": "bar"}
        )
    else:
        resp = await client.execute(
            method, url, headers, req_content, params={"foo": "bar"}
        )
    assert resp.status == 200
    assert resp.headers["x-echo-method"] == "POST"
    assert resp.headers["x-echo-query-string"] == "foo=bar"
    assert resp.headers["x-echo-content-type"] == "text/plain"
    assert resp.headers.getall("x-echo-content-type") == ["text/plain"]
    assert resp.headers["x-echo-x-hello"] == "rust"
    assert resp.headers.getall("x-echo-x-hello") == ["rust", "python"]
    assert resp.content == b"Hello, World!"
    assert resp.text() == "Hello, World!"
    assert len(resp.trailers) == 0


async def execute_json(client: Client | SyncClient, url: str) -> None:
    method = "POST"
    url = f"{url}/echo"
    headers = [
        ("content-type", "text/plain"),
        ("x-hello", "rust"),
        ("x-hello", "python"),
    ]
    req_content_obj = {"message": "Hello, World!"}
    req_content = json.dumps(req_content_obj).encode("utf-8")
    if isinstance(client, SyncClient):
        resp = await asyncio.to_thread(
            client.execute, method, url, headers, req_content
        )
    else:
        resp = await client.execute(method, url, headers, req_content)
    assert resp.status == 200
    assert resp.headers["x-echo-method"] == "POST"
    assert resp.headers["x-echo-content-type"] == "text/plain"
    assert resp.headers.getall("x-echo-content-type") == ["text/plain"]
    assert resp.headers["x-echo-x-hello"] == "rust"
    assert resp.headers.getall("x-echo-x-hello") == ["rust", "python"]
    assert resp.content == req_content
    assert resp.json() == req_content_obj
    assert len(resp.trailers) == 0


async def get(client: Client | SyncClient, url: str) -> None:
    url = f"{url}/echo"
    if isinstance(client, SyncClient):
        resp = await asyncio.to_thread(client.get, url, params={"foo": "bar"})
    else:
        resp = await client.get(url, params={"foo": "bar"})
    assert resp.status == 200
    assert resp.headers["x-echo-method"] == "GET"
    assert resp.headers["x-echo-query-string"] == "foo=bar"
    assert resp.content == b""
    assert len(resp.trailers) == 0


async def post(
    client: Client | SyncClient, url: str, http_version: HTTPVersion | None
) -> None:
    url = f"{url}/echo"
    headers = [("content-type", "text/plain"), ("te", "trailers")]
    req_content = b"Hello, World!"
    if isinstance(client, SyncClient):
        resp = await asyncio.to_thread(
            client.post, url, headers, req_content, params={"foo": "bar"}
        )
    else:
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


async def delete(client: Client | SyncClient, url: str) -> None:
    url = f"{url}/echo"
    if isinstance(client, SyncClient):
        resp = await asyncio.to_thread(client.delete, url, params={"foo": "bar"})
    else:
        resp = await client.delete(url, params={"foo": "bar"})
    assert resp.status == 200
    assert resp.headers["x-echo-method"] == "DELETE"
    assert resp.headers["x-echo-query-string"] == "foo=bar"
    assert resp.content == b""
    assert len(resp.trailers) == 0


async def head(client: Client | SyncClient, url: str) -> None:
    url = f"{url}/echo"
    if isinstance(client, SyncClient):
        resp = await asyncio.to_thread(client.head, url, params={"foo": "bar"})
    else:
        resp = await client.head(url, params={"foo": "bar"})
    assert resp.status == 200
    assert resp.headers["x-echo-method"] == "HEAD"
    assert resp.headers["x-echo-query-string"] == "foo=bar"
    assert resp.content == b""
    assert len(resp.trailers) == 0


async def options(client: Client | SyncClient, url: str) -> None:
    url = f"{url}/echo"
    if isinstance(client, SyncClient):
        resp = await asyncio.to_thread(client.options, url, params={"foo": "bar"})
    else:
        resp = await client.options(url, params={"foo": "bar"})
    assert resp.status == 200
    assert resp.headers["x-echo-method"] == "OPTIONS"
    assert resp.headers["x-echo-query-string"] == "foo=bar"
    assert resp.content == b""
    assert len(resp.trailers) == 0


async def patch(client: Client | SyncClient, url: str) -> None:
    url = f"{url}/echo"
    headers = [("content-type", "text/plain")]
    req_content = b"Hello, World!"
    if isinstance(client, SyncClient):
        resp = await asyncio.to_thread(
            client.patch, url, headers, req_content, params={"foo": "bar"}
        )
    else:
        resp = await client.patch(url, headers, req_content, params={"foo": "bar"})
    assert resp.status == 200
    assert resp.headers["x-echo-method"] == "PATCH"
    assert resp.headers["x-echo-query-string"] == "foo=bar"
    assert resp.headers["x-echo-content-type"] == "text/plain"
    assert resp.headers.getall("x-echo-content-type") == ["text/plain"]
    assert resp.content == b"Hello, World!"
    assert len(resp.trailers) == 0


async def put(client: Client | SyncClient, url: str) -> None:
    url = f"{url}/echo"
    headers = [("content-type", "text/plain")]
    req_content = b"Hello, World!"
    if isinstance(client, SyncClient):
        resp = await asyncio.to_thread(
            client.put, url, headers, req_content, params={"foo": "bar"}
        )
    else:
        resp = await client.put(url, headers, req_content, params={"foo": "bar"})
    assert resp.status == 200
    assert resp.headers["x-echo-method"] == "PUT"
    assert resp.headers["x-echo-query-string"] == "foo=bar"
    assert resp.headers["x-echo-content-type"] == "text/plain"
    assert resp.headers.getall("x-echo-content-type") == ["text/plain"]
    assert resp.content == b"Hello, World!"
    assert len(resp.trailers) == 0


async def nihongo(client: Client | SyncClient, url: str) -> None:
    url = f"{url}/日本語 英語?q=テスト&ほげ=fo%26o"
    if isinstance(client, SyncClient):
        resp = await asyncio.to_thread(client.get, url)
    else:
        resp = await client.get(url)
    assert resp.status == 200
    qs = parse_qs(resp.headers["x-echo-query-string"])
    assert qs["q"] == ["テスト"]
    assert qs["ほげ"] == ["fo&o"]


async def json_content(client: Client | SyncClient, url: str, method: str) -> None:
    url = f"{url}/echo"
    content = {"message": "Hello, World!"}
    if isinstance(client, SyncClient):
        match method:
            case "POST":
                resp = await asyncio.to_thread(client.post, url, content=content)
            case "PUT":
                resp = await asyncio.to_thread(client.put, url, content=content)
            case "PATCH":
                resp = await asyncio.to_thread(client.patch, url, content=content)
            case "EXECUTE_POST":
                resp = await asyncio.to_thread(
                    client.execute, "POST", url, content=content
                )
            case "STREAM_POST":

                def run():
                    with client.stream("POST", url, content=content) as resp:
                        resp_content = b"".join(resp.content)
                    return FullResponse(
                        resp.status, resp.headers, resp_content, resp.trailers
                    )

                resp = await asyncio.to_thread(run)
    else:
        match method:
            case "POST":
                resp = await client.post(url, content=content)
            case "PUT":
                resp = await client.put(url, content=content)
            case "PATCH":
                resp = await client.patch(url, content=content)
            case "EXECUTE_POST":
                resp = await client.execute("POST", url, content=content)
            case "STREAM_POST":
                async with client.stream("POST", url, content=content) as resp:
                    resp_content = b""
                    async for chunk in resp.content:
                        resp_content += chunk
                resp = FullResponse(
                    resp.status, resp.headers, resp_content, resp.trailers
                )
    assert resp.status == 200
    assert resp.headers["content-type"] == "application/json"
    assert resp.content == b'{"message": "Hello, World!"}'
    assert resp.json() == content


async def json_content_existing_content_type(
    client: Client | SyncClient, url: str
) -> None:
    url = f"{url}/echo"
    content = {"message": "Hello, World!"}
    if isinstance(client, SyncClient):
        resp = await asyncio.to_thread(
            client.post, url, headers={"content-type": "text/plain"}, content=content
        )
    else:
        resp = await client.post(
            url, headers={"content-type": "text/plain"}, content=content
        )
    assert resp.status == 200
    assert resp.headers["content-type"] == "text/plain"
    assert resp.content == b'{"message": "Hello, World!"}'

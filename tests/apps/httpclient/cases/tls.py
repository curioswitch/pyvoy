from __future__ import annotations

import asyncio

import pytest
from pyqwest import Client, SyncClient

pytestmark = [
    pytest.mark.parametrize("http_scheme", ["https"], indirect=True),
    pytest.mark.parametrize("http_version", ["h2"], indirect=True),
]


@pytest.mark.asyncio
async def mtls(client: Client | SyncClient, url: str) -> None:
    method = "POST"
    url = f"{url}/echo"
    headers = [("content-type", "text/plain")]
    req_content = b"Hello, World!"

    if isinstance(client, SyncClient):

        def run():
            with client.stream(method, url, headers, req_content) as resp:
                content = b"".join(resp.content)
            return resp, content

        resp, content = await asyncio.to_thread(run)
    else:
        async with client.stream(method, url, headers, req_content) as resp:
            content = b""
            async for chunk in resp.content:
                content += chunk

    assert resp.status == 200
    assert (
        resp.headers["x-echo-tls-client-name"] == "CN=someclient,OU=tests,O=curioswitch"
    )
    assert content == b"Hello, World!"

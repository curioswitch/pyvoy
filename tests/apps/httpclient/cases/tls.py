# Mirrors pyqwest's tests/test_tls.py, run inside a pyvoy application against
# pyvoy's Envoy-backed transports, whose upstreams carry the TLS configuration.
from __future__ import annotations

from anyio import to_thread
from pyqwest import Client, SyncClient


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

        resp, content = await to_thread.run_sync(run)
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

from __future__ import annotations

import sys
from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from pyqwest import Client


pytestmark = [
    pytest.mark.parametrize("http_scheme", ["http"], indirect=True),
    pytest.mark.parametrize("http_version", ["h2"], indirect=True),
]


async def _run_test(
    case: str,
    url: str,
    client: Client,
    http_scheme: str,
    http_version: str,
    extra: str = "",
) -> None:
    response = await client.get(
        url,
        headers={
            "x-test-case": case,
            "x-test-scheme": http_scheme,
            "x-test-http-version": http_version,
            "x-test-extra": extra,
        },
    )
    assert response.status == 200, response.text()


# The timeout cases rely on per-request timeouts. For ASGI these use asyncio
# cancellation; the WSGI transport instead delegates timeouts to Envoy's stream
# timeout configured on the transport, so these are only exercised against ASGI.
@pytest.mark.asyncio
@pytest.mark.skipif(
    sys.version_info < (3, 11), reason="asyncio.timeout requires Python 3.11+"
)
async def test_request_timeout(
    url_asgi: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test(
        "errors_request_timeout", url_asgi, client, http_scheme, http_version
    )


@pytest.mark.asyncio
@pytest.mark.skipif(
    sys.version_info < (3, 11), reason="asyncio.timeout requires Python 3.11+"
)
async def test_response_content_timeout(
    url_asgi: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test(
        "errors_response_content_timeout", url_asgi, client, http_scheme, http_version
    )


@pytest.mark.asyncio
async def test_connection_error(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test("errors_connection_error", url, client, http_scheme, http_version)

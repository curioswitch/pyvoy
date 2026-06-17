from __future__ import annotations

from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from pyqwest import Client

pytestmark = [
    pytest.mark.parametrize("http_scheme", ["https"], indirect=True),
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


@pytest.mark.asyncio
async def test_mtls(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test("tls_mtls", url, client, http_scheme, http_version)

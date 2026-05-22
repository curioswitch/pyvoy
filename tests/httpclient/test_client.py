from __future__ import annotations

from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from pyqwest import Client


async def _run_test(
    case: str, url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    response = await client.get(
        url,
        headers={
            "x-test-case": case,
            "x-test-scheme": http_scheme,
            "x-test-http-version": http_version,
        },
    )
    assert response.status == 200, response.text()


@pytest.mark.asyncio
async def test_basic(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test("client_basic", url, client, http_scheme, http_version)


@pytest.mark.asyncio
async def test_iterable_body(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test("client_iterable_body", url, client, http_scheme, http_version)


@pytest.mark.asyncio
async def test_empty_request(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test("client_empty_request", url, client, http_scheme, http_version)


@pytest.mark.asyncio
async def test_bidi(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test("client_bidi", url, client, http_scheme, http_version)


@pytest.mark.asyncio
async def test_large_body(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test("client_large_body", url, client, http_scheme, http_version)


@pytest.mark.asyncio
async def test_readall(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test("client_readall", url, client, http_scheme, http_version)


@pytest.mark.asyncio
async def test_get(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test("client_get", url, client, http_scheme, http_version)


@pytest.mark.asyncio
async def test_post(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test("client_post", url, client, http_scheme, http_version)

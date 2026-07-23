from __future__ import annotations

from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from pyqwest import Client


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
async def test_execute(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test("client_execute", url, client, http_scheme, http_version)


@pytest.mark.asyncio
async def test_execute_json(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test("client_execute_json", url, client, http_scheme, http_version)


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


@pytest.mark.asyncio
async def test_delete(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test("client_delete", url, client, http_scheme, http_version)


@pytest.mark.asyncio
async def test_head(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test("client_head", url, client, http_scheme, http_version)


@pytest.mark.asyncio
async def test_options(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test("client_options", url, client, http_scheme, http_version)


@pytest.mark.asyncio
async def test_patch(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test("client_patch", url, client, http_scheme, http_version)


@pytest.mark.asyncio
async def test_put(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test("client_put", url, client, http_scheme, http_version)


@pytest.mark.asyncio
async def test_nihongo(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test("client_nihongo", url, client, http_scheme, http_version)


# TODO: test_content_encoding


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "method", ["POST", "PUT", "PATCH", "EXECUTE_POST", "STREAM_POST"]
)
async def test_json_content(
    url: str, client: Client, http_scheme: str, http_version: str, method: str
) -> None:
    await _run_test(
        "client_json_content", url, client, http_scheme, http_version, extra=method
    )


@pytest.mark.asyncio
async def test_json_content_existing_content_type(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test(
        "client_json_content_existing_content_type",
        url,
        client,
        http_scheme,
        http_version,
    )


# close_no_read and close_pending_read rely on asyncio-specific request
# generators and _read_pending so are only exercised against ASGI.
@pytest.mark.asyncio
async def test_close_no_read(
    url_asgi: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test("client_close_no_read", url_asgi, client, http_scheme, http_version)


@pytest.mark.asyncio
async def test_close_pending_read(
    url_asgi: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test(
        "client_close_pending_read", url_asgi, client, http_scheme, http_version
    )


@pytest.mark.asyncio
async def test_request_content_error(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test(
        "client_request_content_error", url, client, http_scheme, http_version
    )


@pytest.mark.asyncio
async def test_response_error(
    url: str, client: Client, http_scheme: str, http_version: str
) -> None:
    await _run_test("client_response_error", url, client, http_scheme, http_version)

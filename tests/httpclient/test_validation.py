from __future__ import annotations

from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from pyqwest import Client


@pytest.mark.asyncio
async def test_transport_invalid_option(url: str, client: Client) -> None:
    response = await client.get(
        url,
        headers={
            "x-test-case": "transport_invalid_option",
            "x-test-scheme": "http",
            "x-test-http-version": "h1",
        },
    )
    assert response.status == 200, response.text()

from __future__ import annotations

from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from pyqwest import Client


@pytest.mark.asyncio
async def test_get(url: str, client: Client) -> None:
    response = await client.get(url, headers={"x-test-case": "client_get"})
    assert response.status == 200, response.text()


@pytest.mark.asyncio
async def test_post(url: str, client: Client) -> None:
    response = await client.get(url, headers={"x-test-case": "client_post"})
    assert response.status == 200, response.text()

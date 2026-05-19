from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from collections.abc import AsyncIterator, Awaitable, Callable


async def forward_bytes(
    gen: AsyncIterator[bytes], receiver: Callable[[bytes | None], Awaitable[None]]
) -> None:
    async for chunk in gen:
        await receiver(chunk)
    await receiver(None)

from __future__ import annotations

from collections.abc import AsyncIterator, Awaitable, Callable


async def forward_bytes(
    gen: AsyncIterator[bytes], receiver: Callable[[bytes | None], Awaitable[None]]
) -> None:
    print(f"Starting to forward bytes from generator {gen}")
    async for chunk in gen:
        print(f"Forwarding chunk of size {len(chunk)}")
        await receiver(chunk)
    await receiver(None)

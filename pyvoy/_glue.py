from __future__ import annotations

import asyncio
from typing import TYPE_CHECKING

from pyqwest import WriteError

if TYPE_CHECKING:
    from collections.abc import AsyncIterator, Awaitable, Callable


async def forward_bytes(
    gen: AsyncIterator[bytes],
    receiver: Callable[[bytes | int | Exception | None], Awaitable[None]],
) -> None:
    try:
        async for chunk in gen:
            await receiver(chunk)
        await receiver(None)
    except asyncio.CancelledError:
        print("forward_bytes: CancelledError caught, stopping forwarding")
        await receiver(1)
    except Exception as e:
        print(f"forward_bytes: Exception caught: {e}")
        err = WriteError(str(e))
        err.__cause__ = e
        await receiver(err)
    finally:
        try:
            aclose = gen.aclose  # type: ignore[attr-defined]
        except AttributeError:
            pass
        else:
            await aclose()

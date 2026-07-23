from __future__ import annotations

import asyncio
import contextlib
import inspect
import types
from typing import TYPE_CHECKING

from pyqwest import WriteError

if TYPE_CHECKING:
    from collections.abc import AsyncIterator, Awaitable, Callable, Iterator


def close_request_iterator(itr: Iterator[bytes]) -> None:
    # Used to unblock a streaming request body iterator that is still being read,
    # for example when the request times out. Running generators cannot be closed
    # reliably (on some Python versions it can hang), so leave them be.
    if (
        isinstance(itr, types.GeneratorType)
        and inspect.getgeneratorstate(itr) == inspect.GEN_RUNNING
    ):
        return
    try:
        close = itr.close  # type: ignore[attr-defined]
    except AttributeError:
        pass
    else:
        with contextlib.suppress(Exception):
            close()


async def forward_bytes(
    gen: AsyncIterator[bytes],
    receiver: Callable[[bytes | int | Exception | None], Awaitable[None]],
) -> None:
    try:
        async for chunk in gen:
            await receiver(chunk)
        await receiver(None)
    except asyncio.CancelledError:
        await receiver(1)
    except Exception as e:
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

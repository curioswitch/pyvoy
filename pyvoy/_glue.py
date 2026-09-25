from __future__ import annotations

import asyncio
import contextlib
import inspect
import sys
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


def _is_cancellation(exception: BaseException) -> bool:
    if isinstance(exception, asyncio.CancelledError):
        return True
    # Only checked when the application runs on trio, which will have imported it.
    trio = sys.modules.get("trio")
    return trio is not None and isinstance(exception, trio.Cancelled)


async def forward_bytes(
    gen: AsyncIterator[bytes],
    receiver: Callable[[bytes | int | Exception | None], Awaitable[None]],
) -> None:
    """Forwards a request body to Envoy.

    `receiver` takes each chunk, then `None` at the end of the body, an
    exception to fail the request with, or `1` to reset it on cancellation.
    """
    try:
        async for chunk in gen:
            if not isinstance(chunk, (bytes, bytearray, memoryview)):
                msg = f"request body must yield bytes, got {type(chunk).__name__}"
                raise TypeError(msg)  # noqa: TRY301
            await receiver(chunk)
    except Exception as e:
        err = WriteError(str(e))
        err.__cause__ = e
        await receiver(err)
    except BaseException as e:
        if _is_cancellation(e):
            await receiver(1)
            raise
        # The body was interrupted, for example by an exception that is not an
        # Exception, so the request cannot be completed.
        err = WriteError("Request body ended before it was complete")
        err.__cause__ = e
        await receiver(err)
    else:
        await receiver(None)
    finally:
        try:
            aclose = gen.aclose  # type: ignore[attr-defined]
        except AttributeError:
            pass
        else:
            await aclose()

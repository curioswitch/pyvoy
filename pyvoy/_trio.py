"""An event loop that runs ASGI applications on trio.

pyvoy's native executor drives ASGI applications through a small part of the
asyncio event loop API: it schedules callbacks from other threads, creates
futures that Envoy completes and tasks that run the application. This module
implements that part on trio so the same executor runs applications inside
`trio.run`.

This module is imported only when trio is configured, so trio stays an
optional dependency.
"""

from __future__ import annotations

import concurrent.futures
import contextlib
import logging
import sys
import threading
from typing import TYPE_CHECKING, Any

import trio

if sys.version_info < (3, 11):
    # A dependency of trio on these versions.
    from exceptiongroup import BaseExceptionGroup

if TYPE_CHECKING:
    from collections.abc import Callable, Coroutine, Generator
    from types import TracebackType

_logger = logging.getLogger(__name__)


class Future:
    """A result set by the executor that an application awaits.

    It is only set and awaited on the trio thread. As with an asyncio future,
    it is cancelled when the awaiting task is cancelled before it is set, and
    setting it afterwards is ignored.
    """

    __slots__ = (
        "_cancelled",
        "_event",
        "_exception",
        "_exception_tb",
        "_result",
        "_waiter",
    )

    def __init__(self) -> None:
        self._event = trio.Event()
        self._cancelled = False
        self._result: object = None
        self._exception: BaseException | None = None
        self._exception_tb: TracebackType | None = None
        self._waiter: Generator[Any, None, object] | None = None

    def done(self) -> bool:
        return self._cancelled or self._event.is_set()

    def cancelled(self) -> bool:
        return self._cancelled

    def set_result(self, result: object) -> None:
        if self.done():
            return
        self._result = result
        self._event.set()

    def set_exception(self, exception: BaseException) -> None:
        if self.done():
            return
        self._exception = exception
        # The executor reuses exception instances across requests, so each
        # raise restores the original traceback instead of growing it.
        self._exception_tb = exception.__traceback__
        self._event.set()

    def __await__(self) -> Generator[Any, None, object]:
        # `anext(iterator, default)` calls this again each time the task is
        # resumed, so a pending future keeps handing out the one wait it
        # started rather than parking the task on a new one every time.
        if self._waiter is None:
            if self._event.is_set():
                return self._finished().__await__()
            self._waiter = self._wait().__await__()
        return self._waiter

    async def _wait(self) -> object:
        try:
            try:
                await self._event.wait()
            except BaseException:
                if not self._event.is_set():
                    self._cancelled = True
                raise
            return await self._finished()
        finally:
            self._waiter = None

    async def _finished(self) -> object:
        if self._exception is not None:
            raise self._exception.with_traceback(self._exception_tb)
        return self._result


class Task:
    """An application coroutine running in the event loop's nursery."""

    __slots__ = ("_callbacks", "_exception", "_result", "_scope", "_token")

    def __init__(self, token: trio.lowlevel.TrioToken) -> None:
        self._token = token
        self._scope = trio.CancelScope()
        self._callbacks: list[Callable[[Task], object]] = []
        self._result: object = None
        self._exception: BaseException | None = None

    def add_done_callback(self, callback: Callable[[Task], object]) -> None:
        self._callbacks.append(callback)

    def result(self) -> object:
        if self._exception is not None:
            raise self._exception
        return self._result

    def cancel(self) -> None:
        """Cancels the task. Called on the trio thread."""
        self._scope.cancel()

    def cancel_soon(self) -> None:
        """Cancels the task from any thread."""
        with contextlib.suppress(trio.RunFinishedError):
            self._token.run_sync_soon(self._scope.cancel)

    async def _run(self, coro: Coroutine[Any, Any, object]) -> None:
        # An exception from one application must not cancel the others in the
        # nursery, so it is stored for the done callbacks as asyncio does. A
        # trio cancellation, whether from `cancel` or the loop stopping,
        # propagates to the scope and the callbacks never run, as with a
        # pending asyncio task when its loop stops.
        with self._scope:
            try:
                self._result = await coro
            except trio.Cancelled:
                raise
            except BaseExceptionGroup as group:
                cancelled, errors = group.split(trio.Cancelled)
                if errors is not None:
                    self._exception = errors
                if cancelled is not None:
                    raise cancelled from None
            except BaseException as e:
                self._exception = e
            for callback in self._callbacks:
                _run_callback(callback, (self,))


class EventLoop:
    """The subset of the asyncio event loop API the executor uses."""

    def __init__(self) -> None:
        self._started = threading.Event()
        self._token: trio.lowlevel.TrioToken | None = None
        self._nursery: trio.Nursery | None = None
        self._stop: trio.Event | None = None

    def run_forever(self) -> None:
        """Runs trio on the calling thread until `stop` is called."""
        trio.run(self._main)

    async def _main(self) -> None:
        self._stop = trio.Event()
        async with trio.open_nursery() as nursery:
            self._nursery = nursery
            self._token = trio.lowlevel.current_trio_token()
            self._started.set()
            await self._stop.wait()
            nursery.cancel_scope.cancel()

    def stop(self) -> None:
        """Stops the loop, cancelling running tasks. Called on the trio thread."""
        assert self._stop is not None  # noqa: S101
        self._stop.set()

    def call_soon_threadsafe(
        self, callback: Callable[..., object], *args: object
    ) -> None:
        """Schedules a callback on the trio thread from any thread.

        Raises trio.RunFinishedError, a RuntimeError, once the loop has
        stopped, as asyncio does for a closed loop.
        """
        if self._token is None:
            # Only possible just after the loop thread starts.
            self._started.wait()
        assert self._token is not None  # noqa: S101
        self._token.run_sync_soon(_run_callback, callback, args)

    def create_future(self) -> Future:
        return Future()

    def create_task(self, coro: Coroutine[Any, Any, object]) -> Task:
        """Starts a coroutine on the trio thread.

        As with asyncio, the task runs with a copy of the caller's context.
        """
        assert self._nursery is not None  # noqa: S101
        assert self._token is not None  # noqa: S101
        task = Task(self._token)
        self._nursery.start_soon(task._run, coro)  # noqa: SLF001
        return task

    def run_coroutine_threadsafe(
        self, coro: Coroutine[Any, Any, object]
    ) -> concurrent.futures.Future[object]:
        """Starts a coroutine from any thread, like asyncio.run_coroutine_threadsafe."""
        future: concurrent.futures.Future[object] = concurrent.futures.Future()

        def complete(task: Task) -> None:
            try:
                future.set_result(task.result())
            except BaseException as e:
                future.set_exception(e)

        def start() -> None:
            self.create_task(coro).add_done_callback(complete)

        self.call_soon_threadsafe(start)
        return future


def new_event_loop() -> EventLoop:
    return EventLoop()


def _run_callback(callback: Callable[..., object], args: tuple[object, ...]) -> None:
    # An exception escaping a trio callback ends the whole run, so it is logged
    # instead, as asyncio logs a failing callback.
    try:
        callback(*args)
    except Exception:
        _logger.exception("Exception in callback %r", callback)

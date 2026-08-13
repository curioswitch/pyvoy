from __future__ import annotations

import asyncio
import signal
import sys
from typing import TYPE_CHECKING

import pytest

from pyvoy import Mount, PyvoyServer, StaticMount, Upstream
from pyvoy import _cli as cli
from pyvoy._server import StartupError

if TYPE_CHECKING:
    from collections.abc import Callable
    from pathlib import Path


@pytest.mark.parametrize(("return_code", "expected_exit_code"), [(23, 23), (-9, 137)])
@pytest.mark.asyncio
async def test_child_runtime_failure_exits_nonzero(
    return_code: int,
    expected_exit_code: int,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    async def start(_server: PyvoyServer) -> None:
        return

    async def wait(_server: PyvoyServer) -> int:
        return return_code

    async def stop(_server: PyvoyServer) -> None:
        return

    monkeypatch.setattr(PyvoyServer, "start", start)
    monkeypatch.setattr(PyvoyServer, "wait", wait)
    monkeypatch.setattr(PyvoyServer, "stop", stop)
    server = PyvoyServer("tests.apps.asgi.kitchensink")
    server._listener_address = "127.0.0.1"
    server._listener_port = 8000

    with pytest.raises(SystemExit) as exc_info:
        await cli._run_server(server, handle_sigterm=False)

    assert exc_info.value.code == expected_exit_code
    assert (
        f"Envoy server exited unexpectedly with code {return_code}."
        in capsys.readouterr().err
    )


@pytest.mark.asyncio
async def test_child_clean_exit_succeeds(monkeypatch: pytest.MonkeyPatch) -> None:
    async def start(_server: PyvoyServer) -> None:
        return

    async def wait(_server: PyvoyServer) -> int:
        return 0

    async def stop(_server: PyvoyServer) -> None:
        return

    monkeypatch.setattr(PyvoyServer, "start", start)
    monkeypatch.setattr(PyvoyServer, "wait", wait)
    monkeypatch.setattr(PyvoyServer, "stop", stop)
    server = PyvoyServer("tests.apps.asgi.kitchensink")
    server._listener_address = "127.0.0.1"
    server._listener_port = 8000

    await cli._run_server(server, handle_sigterm=False)


@pytest.mark.asyncio
async def test_reload_cancellation_is_clean(monkeypatch: pytest.MonkeyPatch) -> None:
    wait_started = asyncio.Event()
    stop_called = asyncio.Event()

    async def start(_server: PyvoyServer) -> None:
        return

    async def wait(_server: PyvoyServer) -> int:
        wait_started.set()
        await asyncio.Future()
        return 0

    async def stop(_server: PyvoyServer) -> None:
        stop_called.set()

    monkeypatch.setattr(PyvoyServer, "start", start)
    monkeypatch.setattr(PyvoyServer, "wait", wait)
    monkeypatch.setattr(PyvoyServer, "stop", stop)
    server = PyvoyServer("tests.apps.asgi.kitchensink")
    server._listener_address = "127.0.0.1"
    server._listener_port = 8000

    server_task = asyncio.create_task(cli._run_server(server, handle_sigterm=False))
    await wait_started.wait()
    server_task.cancel()
    await server_task

    assert stop_called.is_set()


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX signal handling")
@pytest.mark.asyncio
async def test_sigterm_shutdown_is_clean(monkeypatch: pytest.MonkeyPatch) -> None:
    wait_started = asyncio.Event()
    process_exited = asyncio.Event()
    signal_handler: Callable[[], None] | None = None

    class Loop:
        def add_signal_handler(
            self, sig: signal.Signals, callback: Callable[[], None]
        ) -> None:
            nonlocal signal_handler
            assert sig == signal.SIGTERM
            signal_handler = callback

        def remove_signal_handler(self, sig: signal.Signals) -> bool:
            assert sig == signal.SIGTERM
            return True

    async def start(_server: PyvoyServer) -> None:
        return

    async def wait(_server: PyvoyServer) -> int:
        wait_started.set()
        await process_exited.wait()
        return -signal.SIGTERM

    async def stop(_server: PyvoyServer) -> None:
        process_exited.set()

    monkeypatch.setattr(PyvoyServer, "start", start)
    monkeypatch.setattr(PyvoyServer, "wait", wait)
    monkeypatch.setattr(PyvoyServer, "stop", stop)
    monkeypatch.setattr(cli.asyncio, "get_running_loop", Loop)
    server = PyvoyServer("tests.apps.asgi.kitchensink")
    server._listener_address = "127.0.0.1"
    server._listener_port = 8000

    server_task = asyncio.create_task(cli._run_server(server))
    await wait_started.wait()
    assert signal_handler is not None
    signal_handler()
    await server_task


def test_startup_failure_exits_nonzero(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    async def fail_start(_server: PyvoyServer) -> None:
        msg = "startup failed"
        raise StartupError(msg)

    monkeypatch.setattr(PyvoyServer, "start", fail_start)
    monkeypatch.setattr(sys, "argv", ["pyvoy", "tests.apps.asgi.kitchensink"])

    with pytest.raises(SystemExit) as exc_info:
        cli.main()

    assert exc_info.value.code == 1
    assert (
        capsys.readouterr().err
        == "Failed to start Envoy server, see logs for details.\n"
    )


def test_repeated_multi_value_flags_are_accumulated(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    captured_server: PyvoyServer | None = None

    def capture_config(server: PyvoyServer) -> dict[str, object]:
        nonlocal captured_server
        captured_server = server
        return {}

    static_one = tmp_path / "static-one"
    static_two = tmp_path / "static-two"
    monkeypatch.setattr(PyvoyServer, "get_envoy_config", capture_config)
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "pyvoy",
            "primary.app",
            "--additional-mount",
            "first.app=/one=asgi",
            "--additional-mount",
            "second.app=/two=wsgi",
            "--static-mount",
            f"/static-one={static_one}",
            "--static-mount",
            f"/static-two={static_two}",
            "--upstream",
            "first=http://first.example:8001",
            "--upstream",
            "second=http://second.example:8002",
            "--print-envoy-config",
        ],
    )

    cli.main()

    assert captured_server is not None
    assert captured_server._app == [
        Mount(app="primary.app", path="", interface="asgi"),
        Mount(app="first.app", path="/one", interface="asgi"),
        Mount(app="second.app", path="/two", interface="wsgi"),
    ]
    assert captured_server._static_mounts == [
        StaticMount(path="/static-one", root=static_one),
        StaticMount(path="/static-two", root=static_two),
    ]
    assert captured_server._upstreams == [
        Upstream(name="first", address="first.example:8001"),
        Upstream(name="second", address="second.example:8002"),
    ]


@pytest.mark.parametrize(
    ("flag", "value", "minimum"),
    [
        ("--worker-threads", "0", 1),
        ("--worker-threads", "-1", 1),
        ("--worker-threads", str(sys.maxsize + 1), 1),
        ("--websockets-max-message-size", "-1", 0),
        ("--websockets-max-message-size", str(sys.maxsize + 1), 0),
    ],
)
def test_numeric_settings_must_be_in_range(
    flag: str,
    value: str,
    minimum: int,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    monkeypatch.setattr(
        sys, "argv", ["pyvoy", "tests.apps.asgi.kitchensink", flag, value]
    )

    with pytest.raises(SystemExit) as exc_info:
        cli.main()

    assert exc_info.value.code == 2
    assert (
        f"argument {flag}: must be between {minimum} and {sys.maxsize}"
        in capsys.readouterr().err
    )


def test_websockets_max_message_size_allows_zero(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    captured_server: PyvoyServer | None = None

    def capture_config(server: PyvoyServer) -> dict[str, object]:
        nonlocal captured_server
        captured_server = server
        return {}

    monkeypatch.setattr(PyvoyServer, "get_envoy_config", capture_config)
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "pyvoy",
            "tests.apps.asgi.kitchensink",
            "--websockets-max-message-size",
            "0",
            "--print-envoy-config",
        ],
    )

    cli.main()

    assert captured_server is not None
    assert captured_server._websockets_max_message_size == 0

from __future__ import annotations

from typing import TYPE_CHECKING

import pytest

from pyvoy import PyvoyServer

if TYPE_CHECKING:
    from pathlib import Path

    from pyvoy import Interface, Loop


@pytest.mark.asyncio
@pytest.mark.parametrize("interface", ["asgi", "wsgi"])
async def test_atexit_hooks_run_on_shutdown(
    interface: Interface, tmp_path: Path, loop: Loop | None
) -> None:
    marker = tmp_path / "atexit.txt"
    async with PyvoyServer(
        f"tests.apps.{interface}.atexit_hook",
        loop=loop,
        interface=interface,
        env={"PYVOY_TEST_ATEXIT_PATH": str(marker)},
    ):
        assert not marker.exists()

    assert marker.read_text() == "atexit hook ran\n"

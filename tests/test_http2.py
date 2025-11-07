from __future__ import annotations

# Because there aren't any Python HTTP clients that support HTTP/2 fully,
# we have tests written in Go for exercising advanced functionality.
# Go must be installed to run these tests.
import subprocess
from pathlib import Path

import pytest


@pytest.fixture(autouse=True)
def chdir(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.chdir(Path(__file__).parent / "go")


# While ideally we can separate execution per test, the overhead isn't worth it.
def test_advanced_http2() -> None:
    result = subprocess.run(
        ["go", "test", "-count=1", "-v", "."],
        check=False,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stdout + "\n" + result.stderr

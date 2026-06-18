from __future__ import annotations

import json
import subprocess
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import TYPE_CHECKING

import pytest
import pytest_asyncio

from pyvoy import PyvoyServer

if TYPE_CHECKING:
    from collections.abc import AsyncIterator


def _is_docker_unavailable() -> bool:
    # Check docker CLI is available and supports Linux containers
    try:
        result = subprocess.run(
            ["docker", "info", "--format", "{{.OSType}}"],
            capture_output=True,
            text=True,
            check=True,
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        return True
    else:
        return result.stdout.strip() != "linux"


@pytest_asyncio.fixture(scope="module")
async def server() -> AsyncIterator[PyvoyServer]:
    async with PyvoyServer(
        "tests.apps.websockets.echo",
        # Bind all interfaces for access from Docker
        address="0.0.0.0",  # noqa: S104
        stderr=subprocess.STDOUT,
        stdout=subprocess.PIPE,
        websockets=True,
        lifespan=False,
    ) as server:
        yield server


# Higher compression cases deal with huge payloads that are very slow with autobahn's
# Docker runner. We keep them opt-in only for now.
cases = [
    pytest.param(
        [
            "1.*",
            "2.*",
            "3.*",
            "4.*",
            "5.*",
            "6.*",
            "7.*",
            "9.*",
            "10.*",
            "12.1.11",
            "12.2.1",
            "13.1.1",
            "13.2.1",
            "13.3.1",
            "13.4.1",
            "13.5.1",
            "13.6.1",
            "13.7.1",
        ],
        id="fast",
    ),
    pytest.param(["12.*", "13.*"], id="slow", marks=pytest.mark.slow),
]


@pytest.mark.skipif(
    _is_docker_unavailable(), reason="requires Docker with Linux containers"
)
@pytest.mark.parametrize("cases", cases)
def test_autobahn(cases: list[str], server: PyvoyServer) -> None:
    config = {
        "servers": [{"url": f"ws://host.docker.internal:{server.listener_port}"}],
        "outdir": "/reports",
        "cases": cases,
        "exclude-cases": [],
    }

    # Use repository root for temp directory for colima compatibility
    temp_root = Path(__file__).parent.parent / "out"
    temp_root.mkdir(exist_ok=True)
    with TemporaryDirectory(dir=temp_root) as tempdir:
        config_dir = Path(tempdir) / "config"
        config_dir.mkdir(exist_ok=True)
        config_path = config_dir / "config.json"
        config_path.write_text(json.dumps(config))

        reports_dir = Path(tempdir) / "reports"
        reports_dir.mkdir(exist_ok=True)

        subprocess.run(
            [
                "docker",
                "run",
                # Needed for Linux
                "--add-host",
                "host.docker.internal:host-gateway",
                "--env",
                "PYTHONUNBUFFERED=1",
                "-v",
                f"{config_dir}:/config",
                "-v",
                f"{reports_dir}:/reports",
                "crossbario/autobahn-testsuite:25.10.1",
                "wstest",
                "-m",
                "fuzzingclient",
                "--spec",
                "/config/config.json",
            ],
            check=True,
            stdout=subprocess.DEVNULL,
        )

        report = json.loads((reports_dir / "index.json").read_text())
        # wstest always exits 0 — even when it cannot reach the server
        ran = sum(len(results) for results in report.values())
        assert ran > 0, "autobahn ran no cases; server unreachable from the container?"
        for _, results in report.items():
            for case, result in results.items():
                assert result["behavior"] in ("OK", "INFORMATIONAL", "NON-STRICT"), (
                    f"{case} failed with behavior {result['behavior']}"
                )

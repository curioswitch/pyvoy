from __future__ import annotations

import platform
import subprocess
import sys
from pathlib import Path

# Check envoy version minimums and update when needed
MAC_OS_TARGET = "15_0"
GLIBC_TARGET = "2_31"


def main() -> None:
    subprocess.run(["uv", "build", "--wheel"], check=True)

    dist_dir = Path(__file__).parent / ".." / "dist"
    built_wheel = next(dist_dir.glob("*-py3-none-any.whl"))

    python_tag = f"cp{sys.version_info.major}{sys.version_info.minor}"

    match sys.platform:
        case "darwin":
            platform_tag = f"macosx_{MAC_OS_TARGET}_arm64"
        case "linux":
            match platform.machine().lower():
                case "x86_64" | "amd64":
                    platform_tag = f"manylinux_{GLIBC_TARGET}_x86_64"
                case "aarch64" | "arm64":
                    platform_tag = f"manylinux_{GLIBC_TARGET}_aarch64"
                case _:
                    msg = f"Unsupported architecture: {platform.machine()}"
                    raise RuntimeError(msg)
        case "win32":
            platform_tag = "win_amd64"
        case _:
            msg = f"Unsupported platform: {sys.platform}"
            raise RuntimeError(msg)

    subprocess.run(
        [
            sys.executable,
            "-m",
            "wheel",
            "tags",
            "--remove",
            "--python-tag",
            python_tag,
            "--abi-tag",
            python_tag,
            "--platform-tag",
            platform_tag,
            built_wheel,
        ],
        check=True,
    )


if __name__ == "__main__":
    main()

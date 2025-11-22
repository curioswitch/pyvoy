from __future__ import annotations

import subprocess
import sys
from pathlib import Path
from shutil import copyfileobj

bin_dir = Path(__file__).parent.parent / "pyvoy" / "_bin"


def main() -> None:
    print("Building libpyvoy...")  # noqa: T201
    pyvoy_path = bin_dir / "libpyvoy.so"
    pyvoy_path.unlink(missing_ok=True)
    subprocess.run(["cargo", "build", "--release"], check=True)

    release_dir = Path(__file__).parent.parent / "target" / "release"
    if sys.platform == "darwin":
        libpyvoy_path = release_dir / "libpyvoy.dylib"
        proc = subprocess.run(
            ["otool", "-L", libpyvoy_path], stdout=subprocess.PIPE, check=True
        )
        for line in proc.stdout.splitlines():
            if b"libpython" not in line:
                continue
            libpython_path = Path(line.strip().split(b" ")[0].decode())
            subprocess.run(
                [
                    "install_name_tool",
                    "-change",
                    str(libpython_path),
                    libpython_path.name,
                    str(libpyvoy_path),
                ],
                check=True,
            )
    else:
        libpyvoy_path = release_dir / "libpyvoy.so"

    with libpyvoy_path.open("rb") as src, pyvoy_path.open("wb") as dst:
        copyfileobj(src, dst)


if __name__ == "__main__":
    main()

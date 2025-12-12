from __future__ import annotations

import subprocess
import sys
from pathlib import Path
from shutil import copyfileobj, rmtree

bin_dir = Path(__file__).parent.parent / "pyvoy" / "_bin"


def main() -> None:
    print("Building libpyvoy...")  # noqa: T201
    if sys.platform == "win32":
        pyvoy_path = bin_dir / "libpyvoy.dll"
    else:
        pyvoy_path = bin_dir / "libpyvoy.so"
    pyvoy_path.unlink(missing_ok=True)

    target_dir = Path(__file__).parent.parent / "target"
    pyversion_file = target_dir / "python-version.txt"
    if not pyversion_file.exists() or pyversion_file.read_text() != sys.version:
        # PyO3 does not seem to incremently compile correctly when changing Python
        # versions so we track it ourselves.
        rmtree(target_dir, ignore_errors=True)
        target_dir.mkdir(parents=True, exist_ok=True)
        pyversion_file.write_text(sys.version)

    subprocess.run(["cargo", "build", "--release"], check=True)

    release_dir = target_dir / "release"
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
    elif sys.platform == "win32":
        libpyvoy_path = release_dir / "pyvoy.dll"
    else:
        libpyvoy_path = release_dir / "libpyvoy.so"

    with libpyvoy_path.open("rb") as src, pyvoy_path.open("wb") as dst:
        copyfileobj(src, dst)
    print(pyvoy_path)
    for root, dirs, files in bin_dir.walk():
        print(root, dirs, files)


if __name__ == "__main__":
    main()

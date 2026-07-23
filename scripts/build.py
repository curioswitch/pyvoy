from __future__ import annotations

import hashlib
import platform
import subprocess
import sys
from pathlib import Path
from shutil import copyfileobj, rmtree

from pyqwest import SyncClient

bin_dir = Path(__file__).parent.parent / "pyvoy" / "_bin"

version_marker = Path(__file__).parent.parent / "out" / "envoy-files-version.txt"

ENVOY_FILES_VERSION = "v0.1.1"


def envoy_files_asset() -> str:
    """Returns the envoy-files release asset name for the current platform."""
    match sys.platform:
        case "darwin":
            return "libenvoy_files-macos-aarch64.dylib"
        case "win32":
            return "envoy_files-windows-x86_64.dll"
        case "linux":
            match platform.machine().lower():
                case "x86_64" | "amd64":
                    return "libenvoy_files-linux-x86_64.so"
                case "aarch64" | "arm64":
                    return "libenvoy_files-linux-aarch64.so"
                case _:
                    msg = f"Unsupported architecture: {platform.machine()}"
                    raise RuntimeError(msg)
        case _:
            msg = f"Unsupported platform: {sys.platform}"
            raise RuntimeError(msg)


def download_envoy_files() -> None:
    """Downloads the envoy-files dynamic module into _bin.

    Envoy resolves dynamic modules by the filename lib<name>.so on POSIX (including
    macOS, matching how we rename libpyvoy) and <name>.dll on Windows, so we always
    install as libenvoy_files.so / envoy_files.dll regardless of the asset name.
    """
    if sys.platform == "win32":
        dest = bin_dir / "envoy_files.dll"
    else:
        dest = bin_dir / "libenvoy_files.so"

    if (
        dest.exists()
        and version_marker.exists()
        and version_marker.read_text().strip() == ENVOY_FILES_VERSION
    ):
        print(f"envoy-files {ENVOY_FILES_VERSION} already downloaded.")  # noqa: T201
        return

    asset = envoy_files_asset()
    base_url = (
        "https://github.com/curioswitch/envoy-files/releases/download/"
        f"{ENVOY_FILES_VERSION}/{asset}"
    )

    print(f"Downloading envoy-files {ENVOY_FILES_VERSION}...")  # noqa: T201
    client = SyncClient()
    expected_sha = _get(client, f"{base_url}.sha256").split()[0].lower()
    response = client.get(base_url)
    if response.status != 200:
        msg = f"Failed to download {base_url}: {response.status} {response.text()}"
        raise RuntimeError(msg)
    content = response.content
    actual_sha = hashlib.sha256(content).hexdigest()
    if actual_sha != expected_sha:
        msg = (
            f"Checksum mismatch for {asset}: expected {expected_sha}, got {actual_sha}"
        )
        raise RuntimeError(msg)
    dest.write_bytes(content)
    version_marker.parent.mkdir(parents=True, exist_ok=True)
    version_marker.write_text(ENVOY_FILES_VERSION)


def _get(client: SyncClient, url: str) -> str:
    response = client.get(url)
    if response.status != 200:
        msg = f"Failed to download {url}: {response.status} {response.text()}"
        raise RuntimeError(msg)
    return response.text()


def main() -> None:
    download_envoy_files()

    print("Building libpyvoy...")  # noqa: T201
    if sys.platform == "win32":
        pyvoy_path = bin_dir / "pyvoy.dll"
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


if __name__ == "__main__":
    main()

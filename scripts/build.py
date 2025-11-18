import platform
import subprocess
import sys
import tarfile
import urllib.request
from io import BytesIO
from pathlib import Path
from shutil import copyfileobj

import toml

bin_dir = Path(__file__).parent.parent / "pyvoy" / "_bin"


def _download_envoy() -> None:
    toml_path = Path(__file__).parent.parent / "Cargo.toml"
    cargo_toml = toml.loads(toml_path.read_text())

    if cargo_toml["dependencies"]["envoy-proxy-dynamic-modules-rust-sdk"].get("path"):
        print("Using local Envoy build, skipping download.")  # noqa: T201
        return

    version = cargo_toml["dependencies"]["envoy-proxy-dynamic-modules-rust-sdk"]["tag"]
    if version is None:
        msg = "Envoy version not found in Cargo.toml"
        raise RuntimeError(msg)

    machine = platform.machine().lower()
    match machine:
        case "x86_64":
            machine = "amd64"
        case "aarch64":
            machine = "arm64"
    url = f"https://github.com/tetratelabs/archive-envoy/releases/download/{version}/envoy-{version}-{sys.platform}-{machine}.tar.xz"

    envoy_path = bin_dir / "envoy"

    download_envoy = True
    if envoy_path.exists():
        res = subprocess.run(
            [str(envoy_path), "--version"], check=True, capture_output=True, text=True
        )
        version_string = res.stdout.strip()
        if f"/{version[1:]}/" in version_string:
            download_envoy = False

    if download_envoy:
        print("Downloading Envoy binary...")  # noqa: T201
        envoy_path.unlink(missing_ok=True)

        with urllib.request.urlopen(url) as response:  # noqa: S310
            archive_bytes = response.read()

        with tarfile.open(fileobj=BytesIO(archive_bytes), mode="r:xz") as archive:
            envoy_file = archive.extractfile(
                f"envoy-{version}-{sys.platform}-{machine}/bin/envoy"
            )
            if envoy_file is None:
                msg = "envoy binary not found in the archive"
                raise RuntimeError(msg)
            with envoy_path.open("wb") as f:
                copyfileobj(envoy_file, f)
        envoy_path.chmod(0o755)


def main() -> None:
    _download_envoy()

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

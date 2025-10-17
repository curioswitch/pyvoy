import platform
import subprocess
import sys
import tarfile
import urllib.request
from io import BytesIO
from pathlib import Path
from shutil import copyfileobj

import toml


def main() -> None:
    toml_path = Path(__file__).parent.parent / "Cargo.toml"
    cargo_toml = toml.loads(toml_path.read_text())
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

    bin_dir = Path(__file__).parent.parent / "pyvoy" / "_bin"
    envoy_path = bin_dir / "envoy"
    envoy_path.unlink(missing_ok=True)
    pyvoy_path = bin_dir / "libpyvoy.so"
    pyvoy_path.unlink(missing_ok=True)

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

    subprocess.run(["cargo", "build", "--release"], check=True)  # noqa: S607
    ext = ".dylib" if sys.platform == "darwin" else ".so"
    with (
        (Path(__file__).parent.parent / "target" / "release" / f"libpyvoy{ext}").open(
            "rb"
        ) as src,
        pyvoy_path.open("wb") as dst,
    ):
        copyfileobj(src, dst)


if __name__ == "__main__":
    main()

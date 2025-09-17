from pathlib import Path


def get_envoy_path() -> Path:
    return Path(__file__).parent / "envoy"


def get_upstream_path() -> Path:
    return Path(__file__).parent / "envoy-upstream"


def get_pyvoy_dir_path() -> Path:
    return Path(__file__).parent

from pathlib import Path


def get_envoy_path() -> Path:
    return Path(__file__).parent / "envoy"


def get_pyvoy_dir_path() -> Path:
    return Path(__file__).parent

from __future__ import annotations

import argparse
import sys
from pathlib import Path


class DebugArgs:
    port: int | None
    ppid: int | None
    client: str | None
    client_access_token: str | None
    debug_mode: str | None
    preimport: str | None
    multiprocess: bool
    skip_notify_stdin: bool
    json_dap_http: bool
    command: list[str]


def enable_pydev(debug_args: list[str]) -> None:
    """Processes debug arguments orginally sent to the pyvoy CLI and enables pydevd using them."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int)
    parser.add_argument("--ppid", type=int)
    parser.add_argument("--client", type=str)
    parser.add_argument("--client-access-token", type=str)
    parser.add_argument("--debug-mode", type=str)
    parser.add_argument("--preimport", type=str)
    parser.add_argument("--multiprocess", action="store_true")
    parser.add_argument("--skip-notify-stdin", action="store_true")
    parser.add_argument("--json-dap-http", action="store_true")
    parser.add_argument("command", nargs="*")
    args, _ = parser.parse_known_args(debug_args, namespace=DebugArgs())
    if not args.command:
        msg = "No pydevd.py command found, cannot enable debugging."
        raise RuntimeError(msg)
    pydevd_path = Path(args.command[-1])
    if pydevd_path.name != "pydevd.py":
        return
    if "debugpy" in pydevd_path.parts:
        # debugpy removes itself from the path so we need to re-add it to load pydevd.
        # For now, we don't bother with removing it afterwards.
        debugpy_path = pydevd_path
        while debugpy_path.name != "debugpy":
            debugpy_path = debugpy_path.parent
        sys.path.insert(0, str(debugpy_path.parent))
        # This sets up debugpy's vendored pydevd on the path.
        import debugpy.server  # pyright: ignore[reportMissingImports]  # noqa: F401, PLC0415

        # This is debugpy-only
        from _pydevd_bundle.pydevd_defaults import (  # pyright: ignore[reportMissingImports]  # noqa: PLC0415
            PydevdCustomization,
        )

        if args.debug_mode:
            PydevdCustomization.DEBUG_MODE = args.debug_mode
        if args.preimport:
            PydevdCustomization.PREIMPORT = args.preimport

    import pydevd  # pyright: ignore[reportMissingImports]  # noqa: PLC0415

    pydevd_kwargs = {}
    if args.client:
        pydevd_kwargs["host"] = args.client
    if args.port:
        pydevd_kwargs["port"] = args.port
    if args.multiprocess:
        pydevd_kwargs["patch_multiprocessing"] = True
    if args.client_access_token:
        pydevd_kwargs["client_access_token"] = args.client_access_token
    if args.skip_notify_stdin:
        pydevd_kwargs["notify_stdin"] = False
    if args.json_dap_http:
        pydevd_kwargs["protocol"] = "dap"
    if args.ppid:
        pydevd_kwargs["ppid"] = args.ppid

    pydevd.settrace(suspend=False, **pydevd_kwargs)

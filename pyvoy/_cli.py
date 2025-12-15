from __future__ import annotations

import argparse
import asyncio
import signal
import sys
from argparse import ArgumentDefaultsHelpFormatter, ArgumentParser
from pathlib import Path
from typing import get_args

import yaml
from envoy import get_envoy_path

from ._server import Interface, LogLevel, Mount, PyvoyServer, get_envoy_environ
from ._watcher import watch


class CLIArgs:
    app: str
    address: str
    port: int
    print_envoy_config: bool
    print_envoy_entrypoint: bool
    tls_port: int | None
    tls_key: str | None
    tls_cert: str | None
    tls_ca_cert: str | None
    tls_disable_http3: bool
    tls_require_client_certificate: bool
    interface: Interface
    root_path: str
    additional_mount: list[str]
    log_level: LogLevel
    worker_threads: int
    lifespan: bool | None
    reload: bool
    reload_dirs: list[str]
    reload_includes: list[str]
    reload_excludes: list[str]


async def amain() -> None:
    parser = ArgumentParser(
        description="Run a pyvoy server", formatter_class=ArgumentDefaultsHelpFormatter
    )
    parser.add_argument(
        "app",
        help="the app to run as 'module:attr' or just 'module', which implies 'app' for 'attr'",
    )
    parser.add_argument(
        "--address", help="the address to listen on", type=str, default="127.0.0.1"
    )
    parser.add_argument(
        "--port",
        help="the port to listen on (0 for random). Will use TLS if --tls-key/cert are provided without --tls-port",
        type=int,
        default=8000,
    )
    parser.add_argument(
        "--tls-port",
        help="a TLS port to listen on in addition to the plaintext port (0 for random)",
        type=int,
        default=None,
    )
    parser.add_argument(
        "--tls-key",
        help="path to the TLS private key file or the private key in PEM format",
        type=str,
        default=None,
    )
    parser.add_argument(
        "--tls-cert",
        help="path to the TLS certificate file or the certificate in PEM format",
        type=str,
        default=None,
    )
    parser.add_argument(
        "--tls-ca-cert",
        help="path to the TLS CA certificate file or the CA certificate in PEM format",
        type=str,
        default=None,
    )

    parser.add_argument(
        "--tls-disable-http3",
        help="disable HTTP/3 support",
        action="store_true",
        default=False,
    )

    parser.add_argument(
        "--tls-require-client-certificate",
        help="require client certificate for TLS connections",
        action="store_true",
        default=True,
    )

    parser.add_argument(
        "--interface",
        help="the Python application interface to use",
        choices=get_args(Interface),
        type=str,
        default="asgi",
    )

    parser.add_argument(
        "--root-path",
        help="the root path the application is mounted at, for example when using a reverse proxy",
        type=str,
        default="",
    )

    parser.add_argument(
        "--additional-mount",
        help="additional application mounts in the form 'app=path=interface'",
        type=str,
        nargs="+",
        default=[],
    )

    parser.add_argument(
        "--log-level",
        help="the Envoy log level",
        choices=get_args(LogLevel),
        type=str,
        default="error",
    )

    parser.add_argument(
        "--worker-threads",
        help="number of worker threads to use (default: 1 for ASGI, 200 for WSGI)",
        type=int,
        default=argparse.SUPPRESS,
    )

    parser.add_argument(
        "--lifespan",
        help="whether to require or disable ASGI lifespan support. Unset means auto-detect.",
        action=argparse.BooleanOptionalAction,
        default=None,
    )

    parser.add_argument(
        "--reload",
        help="enable auto-reloading on code changes",
        action="store_true",
        default=False,
    )

    parser.add_argument(
        "--reload-dirs",
        help="directories to watch for reload",
        type=str,
        nargs="+",
        default=["."],
    )

    parser.add_argument(
        "--reload-includes",
        help="file patterns to include for reload",
        type=str,
        nargs="+",
        default=["*.py"],
    )

    parser.add_argument(
        "--reload-excludes",
        help="file patterns to exclude from reload",
        type=str,
        nargs="+",
        default=[".*", "*.py[cod]", "*.sw.*", "~*"],
    )

    parser.add_argument(
        "--print-envoy-config",
        help="print the generated Envoy config to stdout and exit",
        action="store_true",
        default=False,
    )

    parser.add_argument(
        "--print-envoy-entrypoint",
        help="print a shell script to execute Envoy with environment for pyvoy, generally for running in Dockerfile",
        action="store_true",
        default=False,
    )

    argv = sys.argv[1:]
    additional_envoy_args = []
    try:
        separator_idx = argv.index("--")
        additional_envoy_args = argv[separator_idx + 1 :]
        argv = argv[:separator_idx]
    except ValueError:
        pass

    args = parser.parse_args(argv, namespace=CLIArgs())

    if args.additional_mount:
        mounts = [Mount(app=args.app, path=args.root_path, interface=args.interface)]
        for mount_str in args.additional_mount:
            try:
                app, path, interface = mount_str.split("=", 2)
            except ValueError:
                print(  # noqa: T201
                    f"Invalid additional mount format: {mount_str}, expected 'app=path=interface'",
                    file=sys.stderr,
                )
                sys.exit(1)
            mounts.append(Mount(app=app, path=path, interface=interface))
        app = mounts
    else:
        app = args.app

    server = PyvoyServer(
        app,
        address=args.address,
        port=args.port,
        stdout=None,
        stderr=None,
        tls_port=args.tls_port,
        tls_key=Path(args.tls_key) if args.tls_key else None,
        tls_cert=Path(args.tls_cert) if args.tls_cert else None,
        tls_ca_cert=Path(args.tls_ca_cert) if args.tls_ca_cert else None,
        tls_enable_http3=not args.tls_disable_http3,
        tls_require_client_certificate=args.tls_require_client_certificate,
        interface=args.interface,
        root_path=args.root_path,
        log_level=args.log_level,
        worker_threads=getattr(args, "worker_threads", None),
        lifespan=args.lifespan,
        additional_envoy_args=additional_envoy_args,
    )

    if args.print_envoy_config:
        print(yaml.dump(server.get_envoy_config()))  # noqa: T201
        return

    if args.print_envoy_entrypoint:
        # Assume Python environment variables are correctly set by the user.
        env = get_envoy_environ()
        print(f"""#!/bin/sh

# Generated Envoy entrypoint for pyvoy

{" ".join(f'{k}="{v}"' for k, v in env.items())} exec {get_envoy_path()} "$@"
""")  # noqa: T201
        return

    if args.reload:
        while True:
            server_task = asyncio.create_task(_run_server(server))
            try:
                async for changed_files in watch(
                    [Path(d) for d in args.reload_dirs],
                    args.reload_includes,
                    args.reload_excludes,
                ):
                    print(  # noqa: T201
                        f"Changes detected in {', '.join(changed_files)}, reloading pyvoy..."
                    )
                    server_task.cancel()
                    await server_task
                    server_task = asyncio.create_task(_run_server(server))
            except asyncio.CancelledError:
                server_task.cancel()
                await server_task
                return
    else:
        await _run_server(server)


async def _run_server(server: PyvoyServer) -> None:
    async with server:
        if server.stopped:
            print(  # noqa: T201
                "Failed to start Envoy server, see logs for details.", file=sys.stderr
            )
            return
        print(  # noqa: T201
            f"pyvoy listening on {server.listener_address}:{server.listener_port}{' (TLS on ' + str(server.listener_port_tls) + ')' if server.listener_port_tls else ''}",
            file=sys.stderr,
        )

        async def shutdown() -> None:
            print("Shutting down pyvoy...")  # noqa: T201
            await server.stop()

        if sys.platform != "win32":
            asyncio.get_event_loop().add_signal_handler(
                signal.SIGTERM, lambda: asyncio.ensure_future(shutdown())
            )
        try:
            await server.wait()
        except asyncio.CancelledError:
            if sys.platform != "win32":
                asyncio.get_event_loop().remove_signal_handler(signal.SIGTERM)
            await shutdown()


def main() -> None:
    asyncio.run(amain())


if __name__ == "__main__":
    main()

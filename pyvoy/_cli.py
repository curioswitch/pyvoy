import asyncio
import signal
import sys
from argparse import ArgumentDefaultsHelpFormatter, ArgumentParser
from pathlib import Path

import yaml

from ._server import Interface, PyvoyServer


class CLIArgs:
    app: str
    address: str
    port: int
    print_envoy_config: bool
    tls_port: int | None
    tls_key: str | None
    tls_cert: str | None
    tls_ca_cert: str | None
    tls_disable_http3: bool
    interface: Interface
    root_path: str


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
        "--port", help="the port to listen on (0 for random)", type=int, default=8000
    )
    parser.add_argument(
        "--tls-port",
        help="the TLS port to listen on in addition to the plaintext port (0 for random)",
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
        "--interface",
        help="the Python application interface to use",
        choices=["asgi", "wsgi"],
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
        "--print-envoy-config",
        help="print the generated Envoy config to stdout and exit",
        action="store_true",
        default=False,
    )

    args = parser.parse_args(namespace=CLIArgs())

    server = PyvoyServer(
        args.app,
        address=args.address,
        port=args.port,
        stdout=None,
        stderr=None,
        tls_port=args.tls_port,
        tls_key=Path(args.tls_key) if args.tls_key else None,
        tls_cert=Path(args.tls_cert) if args.tls_cert else None,
        tls_ca_cert=Path(args.tls_ca_cert) if args.tls_ca_cert else None,
        tls_enable_http3=not args.tls_disable_http3,
        interface=args.interface,
        root_path=args.root_path,
    )

    if args.print_envoy_config:
        print(yaml.dump(server.get_envoy_config()))  # noqa: T201
        return

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

        asyncio.get_event_loop().add_signal_handler(
            signal.SIGTERM, lambda: asyncio.ensure_future(shutdown())
        )
        try:
            await server.wait()
        except asyncio.CancelledError:
            await shutdown()


def main() -> None:
    asyncio.run(amain())


if __name__ == "__main__":
    main()

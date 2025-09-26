import signal
import sys
from argparse import ArgumentDefaultsHelpFormatter, ArgumentParser
from types import FrameType

from ._server import PyvoyServer


class CLIArgs:
    app: str
    address: str
    port: int
    print_envoy_config: bool


def main() -> None:
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
        "--print-envoy-config",
        help="print the generated Envoy config to stdout and exit",
        action="store_true",
        default=False,
    )

    args = parser.parse_args(namespace=CLIArgs())

    def exit_python(_s: int, _f: FrameType | None) -> None:
        print("Shutting down pyvoy...")  # noqa: T201
        sys.exit(0)

    signal.signal(signal.SIGTERM, exit_python)

    if args.print_envoy_config:
        # TODO: Cleanup
        PyvoyServer(
            args.app,
            address=args.address,
            port=args.port,
            print_startup_logs=True,
            print_envoy_config=True,
        ).start()
        return

    with PyvoyServer(
        args.app, address=args.address, port=args.port, print_startup_logs=True
    ) as server:
        print(  # noqa: T201
            f"pyvoy listening on {server.listener_address}:{server.listener_port}",
            file=sys.stderr,
        )
        while True:
            line = server.output.readline()
            if line:
                print(line, end="", file=sys.stderr)  # noqa: T201


if __name__ == "__main__":
    main()

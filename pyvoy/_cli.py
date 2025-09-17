from argparse import ArgumentDefaultsHelpFormatter, ArgumentParser

from ._server import PyvoyServer


class CLIArgs:
    app: str
    address: str
    port: int


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

    args = parser.parse_args(namespace=CLIArgs())

    with PyvoyServer(
        args.app, address=args.address, port=args.port, print_startup_logs=True
    ) as server:
        print(f"pyvoy listening on {server.listener_address}:{server.listener_port}")  # noqa: T201
        try:
            while True:
                line = server.output.readline()
                if line:
                    print(line, end="")  # noqa: T201
        except KeyboardInterrupt:
            print("Shutting down pyvoy...")  # noqa: T201


if __name__ == "__main__":
    main()

import time
from argparse import ArgumentParser

from ._server import PyvoyServer


def main() -> None:
    parser = ArgumentParser(description="Run a pyvoy server")
    parser.add_argument(
        "app",
        nargs=1,
        help="the app to run as 'module:attr' or just 'module', implying 'app' for 'attr'",
    )

    args = parser.parse_args()

    with PyvoyServer(args.app[0]) as server:
        print(f"pyvoy listening on port: {server.listener_port}")
        try:
            while True:
                time.sleep(4000000)
        except KeyboardInterrupt:
            print("Shutting down pyvoy...")


if __name__ == "__main__":
    main()

import sys
import time

from ._server import PyvoyServer


def main() -> None:
    with PyvoyServer() as server:
        print(f"pyvoy listening on port: {server.listener_port}")
        try:
            while True:
                time.sleep(4000000)
        except KeyboardInterrupt:
            print("Shutting down pyvoy...")
            pass


if __name__ == "__main__":
    main()

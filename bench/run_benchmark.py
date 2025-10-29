import argparse
import json
import subprocess
import sys
import time
import urllib.request
from dataclasses import dataclass
from enum import Enum


@dataclass
class AppServer:
    name: str
    args: list[str]


APP = "tests.apps.asgi.kitchensink:app"

PYVOY = AppServer("pyvoy", ["pyvoy", APP])
HYPERCORN = AppServer("hypercorn", ["hypercorn", "--worker-class", "uvloop", APP])
GRANIAN = AppServer(
    "granian", ["granian", "--interface", "asgi", "--loop", "uvloop", APP]
)
UVICORN = AppServer("uvicorn", ["uvicorn", "--loop", "uvloop", APP])


class Protocol(Enum):
    HTTP1 = "http/1.1"
    HTTP2 = "h2"


class Args(argparse.Namespace):
    short: bool


def main() -> None:
    parser = argparse.ArgumentParser(description="Conformance server")
    parser.add_argument(
        "--short",
        action=argparse.BooleanOptionalAction,
        help="Run a short version of the tests, generally to verify benchmark harness",
    )
    args = parser.parse_args(namespace=Args())

    # Run vegeta once at start to avoid go downloading messages during benchmarks
    subprocess.run(
        ["go", "run", "github.com/tsenart/vegeta/v12@v12.12.0", "--version"],  # noqa: S607
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    for app_server in (PYVOY, HYPERCORN, GRANIAN, UVICORN):
        with subprocess.Popen(  # noqa: S603
            app_server.args, stdout=subprocess.PIPE, stderr=subprocess.PIPE
        ) as server:
            # Wait for server to start
            started = False
            for _ in range(100):
                try:
                    with urllib.request.urlopen(
                        "http://localhost:8000/controlled"
                    ) as resp:
                        if resp.status == 200:
                            started = True
                            break
                except Exception:  # noqa: S110
                    pass
                time.sleep(0.1)
            if server.returncode is not None or not started:
                server.terminate()
                stdout, stderr = server.communicate()
                msg = f"Server {app_server.name} failed to start\n{stderr.decode()}\n{stdout.decode()}"
                raise RuntimeError(msg)
            for protocol in (Protocol.HTTP2, Protocol.HTTP1):
                if protocol != Protocol.HTTP1 and app_server == UVICORN:
                    continue
                for sleep in (0, 10, 200, 500, 1000):
                    for response_size in (0, 100, 10000, 100000):
                        if args.short and (
                            protocol != Protocol.HTTP2 or sleep > 0 or response_size > 0
                        ):
                            continue
                        print(  # noqa: T201
                            f"Running benchmark for {app_server.name} with protocol={protocol.value} sleep={sleep}ms response_size={response_size}\n",
                            flush=True,
                        )
                        target = {
                            "method": "GET",
                            "url": "http://localhost:8000/controlled",
                            "header": {
                                "X-Sleep-Ms": [str(sleep)],
                                "X-Response-Bytes": [str(response_size)],
                            },
                        }
                        vegeta_args = [
                            "go",
                            "run",
                            "github.com/tsenart/vegeta/v12@v12.12.0",
                            "attack",
                            "-format=json",
                            "-rate=0",
                            "-max-workers=30",
                            "-duration=5s",
                        ]
                        if protocol == Protocol.HTTP2:
                            vegeta_args.append("-h2c")
                        vegeta = subprocess.Popen(  # noqa: S603
                            vegeta_args,
                            stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE,
                            stderr=sys.stderr,
                        )
                        vegeta_result, _ = vegeta.communicate(
                            f"{json.dumps(target)}\n".encode()
                        )
                        subprocess.run(
                            [  # noqa: S607
                                "go",
                                "run",
                                "github.com/tsenart/vegeta/v12@v12.12.0",
                                "report",
                            ],
                            check=True,
                            input=vegeta_result,
                        )
                        print("\n", flush=True)  # noqa: T201
            server.terminate()
            server.wait()


if __name__ == "__main__":
    main()

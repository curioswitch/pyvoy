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
HYPERCORN = AppServer("hypercorn", ["hypercorn", APP])
GRANIAN = AppServer("granian", ["granian", "--interface", "asgi", APP])
UVICORN = AppServer("uvicorn", ["uvicorn", APP])


class Protocol(Enum):
    HTTP1 = "http/1.1"
    HTTP2 = "h2"


def main() -> None:
    for app_server in (PYVOY, HYPERCORN, GRANIAN, UVICORN):
        with subprocess.Popen(  # noqa: S603
            app_server.args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
        ) as server:
            # Wait for server to start
            for _ in range(100):
                try:
                    with urllib.request.urlopen(
                        "http://localhost:8000/controlled"
                    ) as resp:
                        if resp.status == 200:
                            break
                except Exception:  # noqa: S110
                    pass
                time.sleep(0.1)
            for protocol in (Protocol.HTTP2, Protocol.HTTP1):
                if protocol != Protocol.HTTP1 and app_server == UVICORN:
                    continue
                for sleep in (0, 1, 10, 50, 100, 200, 500, 1000):
                    for response_size in (0, 1, 10, 100, 1000, 10000, 100000):
                        print(  # noqa: T201
                            f"Running benchmark for {app_server.name} with protocol={protocol.value} sleep={sleep}ms response_size={response_size}\n"
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
                            stdout=sys.stdout,
                            stderr=sys.stderr,
                        )
                        print("\n")  # noqa: T201
            server.terminate()
            server.wait()


if __name__ == "__main__":
    main()

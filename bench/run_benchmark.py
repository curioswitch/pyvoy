import json
import subprocess
import sys
import time
import urllib.request
from dataclasses import dataclass


@dataclass
class AppServer:
    name: str
    args: list[str]


PYVOY = AppServer("pyvoy", ["pyvoy", "tests.apps.kitchensink:app"])
HYPERCORN = AppServer("hypercorn", ["hypercorn", "tests.apps.kitchensink:app"])
GRANIAN = AppServer(
    "granian", ["granian", "--interface", "asgi", "tests.apps.kitchensink:app"]
)


def main() -> None:
    for app_server in (PYVOY, HYPERCORN, GRANIAN):
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
            for sleep in (0, 1, 10, 50, 100, 200, 500, 1000):
                for response_size in (0, 1, 10, 100, 1000, 10000, 100000):
                    print(  # noqa: T201
                        f"Running benchmark for {app_server.name} with sleep={sleep}ms response_size={response_size}\n"
                    )
                    target = {
                        "method": "GET",
                        "url": "http://localhost:8000/controlled",
                        "header": {
                            "X-Sleep-Ms": [str(sleep)],
                            "X-Response-Bytes": [str(response_size)],
                        },
                    }
                    vegeta = subprocess.Popen(
                        [  # noqa: S607
                            "go",
                            "run",
                            "github.com/tsenart/vegeta/v12@v12.12.0",
                            "attack",
                            "-h2c",
                            "-format=json",
                            "-rate=0",
                            "-max-workers=10",
                            "-duration=5s",
                        ],
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

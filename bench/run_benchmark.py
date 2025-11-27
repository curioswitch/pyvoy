from __future__ import annotations

import argparse
import json
import subprocess
import sys
import threading
import time
import urllib.request
from dataclasses import dataclass
from enum import Enum

import psutil


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
UVICORN = AppServer("uvicorn", ["uvicorn", "--no-access-log", "--loop", "uvloop", APP])


class Protocol(Enum):
    HTTP1 = "http/1.1"
    HTTP2 = "h2"


class Args(argparse.Namespace):
    short: bool


@dataclass
class ResourceMeasurement:
    cpu_percent: float
    rss: int


@dataclass
class ResourceAggregate:
    min: ResourceMeasurement
    max: ResourceMeasurement
    avg: ResourceMeasurement


class ResourceMonitor:
    def __init__(self, pid: int) -> None:
        self._process = psutil.Process(pid)
        self._started = False
        self._measurements: list[ResourceMeasurement] = []
        self._lock = threading.Lock()

        self._started = False
        self._thread: threading.Thread | None = None

        # Initialize cpu percent measurement
        self._record()

    def start(self) -> None:
        self._started = True
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self._started = False
        if self._thread is not None:
            self._thread.join()
            self._thread = None

    def clear(self) -> None:
        with self._lock:
            self._measurements.clear()

    def aggregate(self) -> ResourceAggregate:
        with self._lock:
            measurements = self._measurements.copy()
        min_cpu = min(m.cpu_percent for m in measurements)
        max_cpu = max(m.cpu_percent for m in measurements)
        avg_cpu = sum(m.cpu_percent for m in measurements) / len(measurements)
        min_rss = min(m.rss for m in measurements)
        max_rss = max(m.rss for m in measurements)
        avg_rss = sum(m.rss for m in measurements) / len(measurements)
        return ResourceAggregate(
            min=ResourceMeasurement(min_cpu, min_rss),
            max=ResourceMeasurement(max_cpu, max_rss),
            avg=ResourceMeasurement(avg_cpu, int(avg_rss)),
        )

    def _record(self) -> ResourceMeasurement:
        with self._process.oneshot():
            cpu_percent = self._process.cpu_percent(interval=None)
            rss = self._process.memory_info().rss
        return ResourceMeasurement(cpu_percent, rss)

    def _run(self) -> None:
        while self._started:
            with self._lock:
                self._measurements.append(self._record())
            time.sleep(0.5)


def main() -> None:
    parser = argparse.ArgumentParser(description="Conformance server")
    parser.add_argument(
        "--short",
        action=argparse.BooleanOptionalAction,
        help="Run a short version of the tests",
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
        if not sys._is_gil_enabled() and app_server == GRANIAN:  # noqa: SLF001
            # Granian hangs on free-threaded for some reason
            continue
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

            pid = server.pid
            if app_server in (GRANIAN, HYPERCORN, PYVOY):
                # find the worker process
                parent = psutil.Process(server.pid)
                children = parent.children()
                pid = children[-1].pid

            monitor = ResourceMonitor(pid)
            monitor.start()

            for protocol in (Protocol.HTTP2, Protocol.HTTP1):
                if protocol != Protocol.HTTP1 and app_server == UVICORN:
                    continue
                for sleep in (0, 10, 200, 500, 1000):
                    for response_size in (0, 100, 10000, 100000):
                        if args.short and (sleep > 0 or response_size > 0):
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

                        monitor.clear()
                        vegeta = subprocess.Popen(  # noqa: S603
                            vegeta_args,
                            stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE,
                            stderr=sys.stderr,
                        )

                        vegeta_result, _ = vegeta.communicate(
                            f"{json.dumps(target)}\n".encode()
                        )

                        resource = monitor.aggregate()

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

                        print(  # noqa: T201
                            f"\nCPU Percentage\t[Min, Max, Avg]\t\t{resource.min.cpu_percent:.2f}, {resource.max.cpu_percent:.2f}, {resource.avg.cpu_percent:.2f}%"
                        )
                        print(  # noqa: T201
                            f"Memory RSS\t[Min, Max, Avg]\t\t{resource.min.rss}, {resource.max.rss}, {resource.avg.rss}\n"
                        )

                        print("\n", flush=True)  # noqa: T201
            monitor.stop()
            server.terminate()
            server.communicate()


if __name__ == "__main__":
    main()

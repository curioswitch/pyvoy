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

from ._results import BenchmarkResults


@dataclass
class AppServer:
    name: str
    args: list[str]
    asgi_args: list[str] | None
    asgi_args_nogil: list[str]
    wsgi_args: list[str] | None


ASGI_APP = "tests.apps.asgi.kitchensink:app"
WSGI_APP = "tests.apps.wsgi.kitchensink:app"

PYVOY = AppServer(
    "pyvoy",
    ["pyvoy"],
    [ASGI_APP],
    ["--worker-threads", "8"],
    ["--interface", "wsgi", WSGI_APP],
)
HYPERCORN = AppServer(
    "hypercorn",
    ["hypercorn", "--worker-class", "uvloop"],
    [ASGI_APP],
    ["--workers", "8"],
    [WSGI_APP],
)
GRANIAN = AppServer(
    "granian",
    ["granian", "--loop", "uvloop"],
    ["--interface", "asgi", ASGI_APP],
    ["--workers", "8"],
    ["--interface", "wsgi", "--blocking-threads", "200", WSGI_APP],
)
GUNICORN = AppServer(
    "gunicorn", ["gunicorn", "--reuse-port", "--threads", "200"], None, [], [WSGI_APP]
)
UVICORN = AppServer(
    "uvicorn",
    ["uvicorn", "--no-access-log", "--loop", "uvloop"],
    [ASGI_APP],
    ["--workers", "8"],
    None,
)


class Protocol(Enum):
    HTTP1 = "h1"
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


def response_sizes(sleep: int) -> tuple[int, ...]:
    if sleep <= 10:
        return (0, 100, 10000, 100000)
    # Past 10ms latency, response size has no perceivable effect on throughput so
    # we reduce bench time by checking just one.
    return (10000,)


def main() -> None:
    parser = argparse.ArgumentParser(description="Conformance server")
    parser.add_argument(
        "--short",
        action=argparse.BooleanOptionalAction,
        help="Run a short version of the tests",
    )
    args = parser.parse_args(namespace=Args())

    benchmark_results = BenchmarkResults()

    # Run vegeta once at start to avoid go downloading messages during benchmarks
    subprocess.run(
        ["go", "run", "github.com/tsenart/vegeta/v12@v12.12.0", "--version"],  # noqa: S607
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )

    for app_server in (PYVOY, GRANIAN, GUNICORN, HYPERCORN, UVICORN):
        for interface in ("asgi", "wsgi"):
            if not sys._is_gil_enabled() and app_server == GRANIAN:  # noqa: SLF001
                # granian hangs on free-threaded for some reason
                continue
            match interface:
                case "asgi":
                    if app_server.asgi_args is None:
                        continue
                    more_args = app_server.asgi_args
                case "wsgi":
                    if app_server.wsgi_args is None:
                        continue
                    more_args = app_server.wsgi_args
            if not sys._is_gil_enabled() and interface == "asgi":  # noqa: SLF001
                more_args.extend(app_server.asgi_args_nogil)

            with subprocess.Popen(  # noqa: S603
                [*app_server.args, *more_args],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
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
                    if protocol != Protocol.HTTP1 and app_server in (GUNICORN, UVICORN):
                        continue
                    for sleep in (0, 10, 200, 500, 1000):
                        for response_size in response_sizes(sleep):
                            if args.short and (sleep > 0 or response_size > 0):
                                continue
                            print(  # noqa: T201
                                f"Running benchmark for {app_server.name} with interface={interface} protocol={protocol.value} sleep={sleep}ms response_size={response_size}\n",
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

                            # Print text report to console
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

                            # Get JSON report for parsing
                            json_report = subprocess.run(
                                [  # noqa: S607
                                    "go",
                                    "run",
                                    "github.com/tsenart/vegeta/v12@v12.12.0",
                                    "report",
                                    "-type=json",
                                ],
                                check=True,
                                input=vegeta_result,
                                capture_output=True,
                            )
                            vegeta_json = json.loads(json_report.stdout)

                            benchmark_results.store_result(
                                protocol.value,
                                interface,
                                app_server.name,
                                sleep,
                                response_size,
                                vegeta_json,
                                resource.avg.cpu_percent,
                                resource.avg.rss,
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

    # Lazy import since some dependencies disable the GIL, and it seems to get
    # propagated to subprocesses through environment if it happens above.
    from . import _charts  # noqa: PLC0415

    _charts.generate_charts(benchmark_results)


if __name__ == "__main__":
    main()

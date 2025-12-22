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


def _format_bytes(value: float) -> str:
    size = float(value)
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if size < 1024 or unit == "TB":
            if unit == "B":
                return f"{int(size)}{unit}"
            return f"{size:.2f}{unit}"
        size /= 1024
    return f"{size:.2f}TB"


def _format_ms(seconds: float) -> str:
    return f"{float(seconds) * 1000:.2f}ms"


def _num(value: object, default: float = 0.0) -> float:
    return value if isinstance(value, (int, float)) else default


def _print_oha_report(oha_json: dict) -> None:
    summary = oha_json.get("summary", {})
    rps = oha_json.get("rps", {})
    latency = oha_json.get("latencyPercentiles", {})
    status_codes = oha_json.get("statusCodeDistribution", {})
    errors = oha_json.get("errorDistribution", {})

    total_requests = 0
    if isinstance(status_codes, dict):
        total_requests += sum(status_codes.values())
    if isinstance(errors, dict):
        total_requests += sum(errors.values())

    success_rate = summary.get("successRate")
    success_rate_str = (
        f"{success_rate * 100:.2f}%"
        if isinstance(success_rate, (int, float))
        else "n/a"
    )

    print("Summary")  # noqa: T201
    print(  # noqa: T201
        f"  Total requests:\t{total_requests}\n"
        f"  Success rate:\t\t{success_rate_str}\n"
        f"  Requests/sec:\t\t{_num(summary.get('requestsPerSec')):.2f}\n"
    )

    if isinstance(rps, dict):
        stddev = rps.get("stddev")
        stddev_str = f"{stddev:.2f}" if isinstance(stddev, (int, float)) else "n/a"
        print(  # noqa: T201
            "\nRPS [mean, stddev, max, min]\n"
            f"  {_num(rps.get('mean')):.2f}, {stddev_str},"
            f" {_num(rps.get('max')):.2f}, {_num(rps.get('min')):.2f}"
        )

    if isinstance(latency, dict) and latency:
        percentile_keys = ("p50", "p75", "p90", "p95", "p99")
        percentiles = ", ".join(
            f"{key}={_format_ms(_num(latency.get(key)))}"
            for key in percentile_keys
            if key in latency
        )
        if percentiles:
            print(f"\nLatency percentiles\t{percentiles}")  # noqa: T201

    if isinstance(status_codes, dict) and status_codes:
        codes = ", ".join(
            f"{code}={count}"
            for code, count in sorted(
                status_codes.items(), key=lambda item: int(item[0])
            )
        )
        print(f"\nStatus codes\t\t{codes}")  # noqa: T201

    if isinstance(errors, dict) and errors:
        error_lines = "\n".join(
            f"  {error}: {count}" for error, count in errors.items()
        )
        print(f"\nErrors\n{error_lines}")  # noqa: T201


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
    None,
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
    server: str | None
    protocol: str | None
    interface: str | None
    sleep: int | None
    request_size: int | None
    response_size: int | None


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


def app_servers(server_arg: str | None) -> tuple[AppServer, ...]:
    match server_arg:
        case "pyvoy":
            return (PYVOY,)
        case "granian":
            return (GRANIAN,)
        case "gunicorn":
            return (GUNICORN,)
        case "hypercorn":
            return (HYPERCORN,)
        case "uvicorn":
            return (UVICORN,)
        case _:
            return (PYVOY, GRANIAN, GUNICORN, HYPERCORN, UVICORN)


def protocols(protocol_arg: str | None) -> tuple[Protocol, ...]:
    match protocol_arg:
        case "h1":
            return (Protocol.HTTP1,)
        case "h2":
            return (Protocol.HTTP2,)
        case _:
            return (Protocol.HTTP1, Protocol.HTTP2)


def interfaces(interface_arg: str | None) -> tuple[str, ...]:
    match interface_arg:
        case "asgi":
            return ("asgi",)
        case "wsgi":
            return ("wsgi",)
        case _:
            return ("asgi", "wsgi")


def sleeps(sleep_arg: int | None) -> tuple[int, ...]:
    if sleep_arg is not None:
        return (sleep_arg,)
    return (0, 10, 200, 500, 1000)


def request_sizes(request_size_arg: int | None, sleep: int) -> tuple[int, ...]:
    if request_size_arg is not None:
        return (request_size_arg,)
    if sleep <= 10:
        return (0, 1000)
    # Past 10ms latency, request size has no perceivable effect on throughput so
    # we reduce bench time by checking just one.
    return (1000,)


def response_sizes(response_size_arg: int | None, sleep: int) -> tuple[int, ...]:
    if response_size_arg is not None:
        return (response_size_arg,)
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
    parser.add_argument(
        "--server",
        type=str,
        default=None,
        help="Run benchmark for a specific server only",
    )
    parser.add_argument(
        "--protocol",
        type=str,
        default=None,
        help="Run benchmark for a specific protocol only",
    )
    parser.add_argument(
        "--interface",
        type=str,
        default=None,
        help="Run benchmark for a specific interface only",
    )
    parser.add_argument(
        "--sleep",
        type=int,
        default=None,
        help="Run benchmark for a specific sleep only",
    )
    parser.add_argument(
        "--request-size",
        type=int,
        default=None,
        help="Run benchmark for a specific request size only",
    )
    parser.add_argument(
        "--response-size",
        type=int,
        default=None,
        help="Run benchmark for a specific response size only",
    )
    args = parser.parse_args(namespace=Args())

    benchmark_results = BenchmarkResults()

    for app_server in app_servers(args.server):
        for interface in interfaces(args.interface):
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

                for protocol in protocols(args.protocol):
                    if protocol != Protocol.HTTP1 and app_server in (GUNICORN, UVICORN):
                        continue
                    for sleep in sleeps(args.sleep):
                        for request_size in request_sizes(args.request_size, sleep):
                            for response_size in response_sizes(
                                args.response_size, sleep
                            ):
                                if args.short and (
                                    sleep > 0 or request_size > 0 or response_size > 0
                                ):
                                    continue
                                print(  # noqa: T201
                                    f"Running benchmark for {app_server.name} with interface={interface} protocol={protocol.value} sleep={sleep}ms request_size={request_size} response_size={response_size}\n",
                                    flush=True,
                                )
                                oha_args = [
                                    "oha",
                                    "-z",
                                    "5s",
                                    "-c",
                                    "10",
                                    "--no-tui",
                                    "--output-format",
                                    "json",
                                    "-m",
                                    "GET",
                                    "-H",
                                    f"X-Sleep-Ms: {sleep}",
                                    "-H",
                                    f"X-Response-Bytes: {response_size}",
                                ]
                                if protocol == Protocol.HTTP2:
                                    oha_args.extend(["--http-version", "2"])
                                if request_size > 0:
                                    oha_args.extend(["-d", "a" * request_size])
                                oha_args.append("http://localhost:8000/controlled")

                                monitor.clear()
                                oha_run = subprocess.run(  # noqa: S603
                                    oha_args, check=True, capture_output=True, text=True
                                )

                                resource = monitor.aggregate()

                                # Print text report to console
                                oha_json = json.loads(oha_run.stdout)
                                _print_oha_report(oha_json)

                                benchmark_results.store_result(
                                    protocol.value,
                                    interface,
                                    app_server.name,
                                    sleep,
                                    request_size,
                                    response_size,
                                    oha_json,
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

    if args == parser.parse_args([], namespace=Args()):
        # Lazy import since some dependencies disable the GIL, and it seems to get
        # propagated to subprocesses through environment if it happens above.
        from . import _charts  # noqa: PLC0415

        _charts.generate_charts(benchmark_results)


if __name__ == "__main__":
    main()

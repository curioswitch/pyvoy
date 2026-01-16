from __future__ import annotations

from collections import defaultdict
from dataclasses import dataclass


@dataclass(frozen=True)
class ChartKey:
    protocol: str
    interface: str
    tls_mode: str
    sleep: int

    def filename(self, key: str) -> str:
        sleep = "high" if self.sleep == 200 else f"{self.sleep:04d}"
        return f"{self.interface}_{self.protocol}_{self.tls_mode}_{sleep}ms_{key}.png"


@dataclass(frozen=True)
class ChartValue:
    label: str
    label_value: str
    app_server: str
    load_result: dict
    avg_cpu: float
    avg_ram: int


class BenchmarkResults:
    _results: defaultdict[ChartKey, list[ChartValue]]

    def __init__(self) -> None:
        self._results = defaultdict(list)

    def get(self) -> dict[ChartKey, list[ChartValue]]:
        return self._results

    def store_result(
        self,
        protocol: str,
        interface: str,
        tls_mode: str,
        app_server: str,
        sleep: int,
        request_size: int,
        response_size: int,
        load_result: dict,
        avg_cpu: float,
        avg_ram: int,
    ) -> None:
        key = ChartKey(protocol, interface, tls_mode, min(sleep, 200))
        label, label_value = (
            ("sleep", str(sleep))
            if sleep >= 200
            else ("request/response_size", f"{request_size}/{response_size}")
        )
        value = ChartValue(
            label, label_value, app_server, load_result, avg_cpu, avg_ram
        )
        self._results[key].append(value)

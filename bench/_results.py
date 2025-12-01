from __future__ import annotations

from collections import defaultdict
from dataclasses import dataclass


@dataclass(frozen=True)
class ChartKey:
    protocol: str
    interface: str
    sleep: int

    def filename(self, key: str) -> str:
        sleep = "high" if self.sleep == 200 else f"{self.sleep:04d}"
        return f"{self.interface}_{self.protocol}_{sleep}ms_{key}.png"


@dataclass(frozen=True)
class ChartValue:
    label: str
    label_value: int
    app_server: str
    vegeta_result: dict
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
        app_server: str,
        sleep: int,
        response_size: int,
        vegeta_result: dict,
        avg_cpu: float,
        avg_ram: int,
    ) -> None:
        key = ChartKey(protocol, interface, min(sleep, 200))
        label, label_value = (
            ("sleep", sleep) if sleep >= 200 else ("response_size", response_size)
        )
        value = ChartValue(
            label, label_value, app_server, vegeta_result, avg_cpu, avg_ram
        )
        self._results[key].append(value)

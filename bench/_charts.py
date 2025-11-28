from __future__ import annotations

from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path

import matplotlib.pyplot as plt
import pandas as pd
import seaborn as sns


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

    def generate_charts(self) -> None:
        for key, values in self._results.items():
            self._generate_chart(key, values)

    def _generate_chart(self, key: ChartKey, values: list[ChartValue]) -> None:
        data = []
        label = ""
        for value in values:
            label = value.label
            vegeta = value.vegeta_result
            data.append(
                {
                    "app_server": value.app_server,
                    label: f"{value.label_value}{'B' if label == 'response_size' else 'ms'}",
                    "throughput": vegeta["throughput"],
                }
            )

        df = pd.DataFrame(data)

        plt.figure(figsize=(14, 8))
        sns.set_style("whitegrid")

        ax = sns.barplot(
            data=df, x=label, y="throughput", hue="app_server", palette="Set2"
        )

        ax.set_xlabel(label.replace("_", " ").title(), fontsize=12)
        ax.set_ylabel("throughput (req/s)", fontsize=12)
        ax.set_title(
            f"{key.interface}+{key.protocol} throughput {f'(sleep: {key.sleep}ms)' if label == 'response_size' else '(response size: 10KB)'}",
            fontsize=14,
            fontweight="bold",
        )
        plt.xticks(rotation=45, ha="right")
        plt.legend(title="App Server", bbox_to_anchor=(1.05, 1), loc="upper left")
        plt.tight_layout()

        filename = Path(__file__).parent / "charts" / key.filename("throughput")
        plt.savefig(filename, dpi=300, bbox_inches="tight")
        plt.close()

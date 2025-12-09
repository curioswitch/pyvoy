from __future__ import annotations

from pathlib import Path
from typing import TYPE_CHECKING

import matplotlib.pyplot as plt
import pandas as pd
import seaborn as sns

if TYPE_CHECKING:
    from ._results import BenchmarkResults, ChartKey, ChartValue


def generate_charts(results: BenchmarkResults) -> None:
    for key, values in results.get().items():
        _generate_chart(key, values)


def _generate_chart(key: ChartKey, values: list[ChartValue]) -> None:
    data = []
    label = ""
    for value in values:
        label = value.label
        vegeta = value.vegeta_result
        data.append(
            {
                "app_server": value.app_server,
                label: f"{value.label_value}{'B' if label == 'request/response_size' else 'ms'}",
                "throughput": vegeta["throughput"],
            }
        )

    df = pd.DataFrame(data)

    plt.figure(figsize=(14, 8))
    sns.set_style("whitegrid")

    ax = sns.barplot(data=df, x=label, y="throughput", hue="app_server", palette="Set2")

    ax.set_xlabel(label.replace("_", " ").title(), fontsize=12)
    ax.set_ylabel("throughput (req/s)", fontsize=12)
    ax.set_title(
        f"{key.interface}+{key.protocol} throughput {f'(sleep: {key.sleep}ms)' if label == 'request/response_size' else '(response size: 10KB)'}",
        fontsize=14,
        fontweight="bold",
    )
    plt.xticks(rotation=45, ha="right")
    plt.legend(title="App Server", bbox_to_anchor=(1.05, 1), loc="upper left")
    plt.tight_layout()

    filename = Path(__file__).parent / "charts" / key.filename("throughput")
    plt.savefig(filename, dpi=300, bbox_inches="tight")
    plt.close()

#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, Tuple


@dataclass
class Summary:
    rows: int = 0
    min_detection: float = 1.0
    max_false_positive: float = 0.0
    max_mean_error: float = 0.0
    max_p95_error: float = 0.0

    def update(self, detection: float, false_positive: float, mean_error: float, p95_error: float) -> None:
        self.rows += 1
        self.min_detection = min(self.min_detection, detection)
        self.max_false_positive = max(self.max_false_positive, false_positive)
        self.max_mean_error = max(self.max_mean_error, mean_error)
        self.max_p95_error = max(self.max_p95_error, p95_error)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Generate markdown summary from north tick timing CSV.")
    parser.add_argument("csv_path", type=Path)
    parser.add_argument("output_md", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    grouped: Dict[Tuple[str, str], Summary] = {}

    with args.csv_path.open(newline="", encoding="utf-8") as fh:
        for row in csv.DictReader(fh):
            key = (row["mode"], row["scenario"])
            summary = grouped.setdefault(key, Summary())
            summary.update(
                detection=float(row["detection_rate"]),
                false_positive=float(row["false_positive_rate"]),
                mean_error=float(row["mean_abs_error_samples"]),
                p95_error=float(row["p95_abs_error_samples"]),
            )

    lines = [
        "# North Tick Timing Metrics Summary",
        "",
        "| mode | scenario | rows | min detection | max false+ | max mean err | max p95 err |",
        "| --- | --- | ---: | ---: | ---: | ---: | ---: |",
    ]

    for mode, scenario in sorted(grouped.keys()):
        s = grouped[(mode, scenario)]
        lines.append(
            f"| {mode} | {scenario} | {s.rows} | {s.min_detection:.6f} | {s.max_false_positive:.6f} | {s.max_mean_error:.6f} | {s.max_p95_error:.6f} |"
        )

    args.output_md.parent.mkdir(parents=True, exist_ok=True)
    args.output_md.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

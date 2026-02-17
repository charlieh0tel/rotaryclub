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

@dataclass(frozen=True)
class Limits:
    min_detection: float
    max_false_positive: float
    max_mean_error: float
    max_p95_error: float


BASELINE_LIMITS: Dict[Tuple[str, str], Limits] = {
    ("dpll", "clean"): Limits(0.95, 0.05, 1.0, 2.0),
    ("simple", "clean"): Limits(0.95, 0.05, 1.0, 2.0),
    ("dpll", "noisy_jittered"): Limits(0.90, 0.08, 1.3, 2.5),
    ("simple", "noisy_jittered"): Limits(0.90, 0.08, 1.3, 2.5),
    ("dpll", "dropout_burst"): Limits(0.88, 0.10, 1.4, 2.6),
    ("simple", "dropout_burst"): Limits(0.88, 0.10, 1.4, 2.6),
    ("dpll", "impulsive_interference"): Limits(0.85, 0.15, 1.5, 2.8),
    ("simple", "impulsive_interference"): Limits(0.30, 0.15, 1.5, 2.8),
    ("dpll", "long_drift"): Limits(0.97, 0.03, 0.8, 1.5),
    ("simple", "long_drift"): Limits(0.97, 0.03, 0.8, 1.5),
}


def strict_limits(limits: Limits) -> Limits:
    return Limits(
        min_detection=limits.min_detection + 0.02,
        max_false_positive=limits.max_false_positive - 0.02,
        max_mean_error=limits.max_mean_error - 0.15,
        max_p95_error=limits.max_p95_error - 0.25,
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Generate markdown summary from north tick timing CSV.")
    parser.add_argument("csv_path", type=Path)
    parser.add_argument("output_md", type=Path)
    parser.add_argument("--profile", choices=["baseline", "strict"], default="baseline")
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
        f"- Profile: `{args.profile}`",
        "- This markdown file is the detailed metrics artifact generated from CSV.",
        "- CI step-summary status notes are separate and only indicate pass/fail state.",
        "- Threshold dimensions: detection_rate (min), false_positive_rate (max), mean_abs_error_samples (max), p95_abs_error_samples (max).",
        "",
        "## Threshold Profile",
        "",
    ]

    if args.profile == "baseline":
        lines.extend([
            "Using baseline thresholds:",
            "",
        ])
    else:
        lines.extend([
            "Using strict thresholds derived from baseline by:",
            "",
            "- `min_detection + 0.02`",
            "- `max_false_positive - 0.02`",
            "- `max_mean_error - 0.15`",
            "- `max_p95_error - 0.25`",
            "",
        ])

    lines.extend([
        "| mode | scenario | threshold set | min detection | max false+ | max mean err | max p95 err |",
        "| --- | --- | --- | ---: | ---: | ---: | ---: |",
    ])

    for mode, scenario in sorted(BASELINE_LIMITS.keys()):
        base = BASELINE_LIMITS[(mode, scenario)]
        lim = base if args.profile == "baseline" else strict_limits(base)
        if scenario == "impulsive_interference" and mode == "simple":
            threshold_set = "impulsive_interference_simple_mode"
        else:
            threshold_set = scenario
        lines.append(
            f"| {mode} | {scenario} | {threshold_set} | {lim.min_detection:.6f} | {lim.max_false_positive:.6f} | {lim.max_mean_error:.6f} | {lim.max_p95_error:.6f} |"
        )

    lines.extend([
        "",
        "## Metrics",
        "",
        "| mode | scenario | rows | min detection | max false+ | max mean err | max p95 err |",
        "| --- | --- | ---: | ---: | ---: | ---: | ---: |",
    ])

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

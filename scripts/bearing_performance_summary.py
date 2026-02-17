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
    min_success_rate: float = 1.0
    max_mean_us_per_sample: float = 0.0
    max_p95_us_per_sample: float = 0.0

    def update(self, success_rate: float, mean_us_per_sample: float, p95_us_per_sample: float) -> None:
        self.rows += 1
        self.min_success_rate = min(self.min_success_rate, success_rate)
        self.max_mean_us_per_sample = max(self.max_mean_us_per_sample, mean_us_per_sample)
        self.max_p95_us_per_sample = max(self.max_p95_us_per_sample, p95_us_per_sample)


@dataclass(frozen=True)
class Limits:
    min_success_rate: float
    max_mean_us_per_sample: float
    max_p95_us_per_sample: float


BASELINE_LIMITS: Dict[Tuple[str, str], Limits] = {
    ("correlation", "clean"): Limits(1.0, 0.26, 0.37),
    ("correlation", "noisy"): Limits(1.0, 0.26, 0.37),
    ("correlation", "dc_offset"): Limits(1.0, 0.27, 0.38),
    ("correlation", "multipath_like"): Limits(1.0, 0.27, 0.39),
    ("zero_crossing", "clean"): Limits(1.0, 0.18, 0.25),
    ("zero_crossing", "noisy"): Limits(1.0, 0.18, 0.25),
    ("zero_crossing", "dc_offset"): Limits(1.0, 0.19, 0.26),
    ("zero_crossing", "multipath_like"): Limits(1.0, 0.20, 0.28),
}


def strict_limits(limits: Limits) -> Limits:
    return Limits(
        min_success_rate=limits.min_success_rate,
        max_mean_us_per_sample=limits.max_mean_us_per_sample * 0.90,
        max_p95_us_per_sample=limits.max_p95_us_per_sample * 0.90,
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Generate markdown summary from bearing performance CSV."
    )
    parser.add_argument("csv_path", type=Path)
    parser.add_argument("output_md", type=Path)
    parser.add_argument("--profile", choices=["baseline", "strict"], default="baseline")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    grouped: Dict[Tuple[str, str], Summary] = {}

    with args.csv_path.open(newline="", encoding="utf-8") as fh:
        for row in csv.DictReader(fh):
            key = (row["method"], row["scenario"])
            summary = grouped.setdefault(key, Summary())
            summary.update(
                success_rate=float(row["success_rate"]),
                mean_us_per_sample=float(row["mean_us_per_sample"]),
                p95_us_per_sample=float(row["p95_us_per_sample"]),
            )

    lines = [
        "# Bearing Performance Summary",
        "",
        f"- Profile: `{args.profile}`",
        "- Scope: bearing calculators only (correlation and zero-crossing), not end-to-end north+bearing pipeline.",
        "- This markdown file is the detailed metrics artifact generated from CSV.",
        "- CI step-summary status notes are separate and only indicate pass/fail state.",
        "- Threshold dimensions: success_rate (min), mean_us_per_sample (max), p95_us_per_sample (max).",
        "",
        "## Threshold Profile",
        "",
    ]

    if args.profile == "baseline":
        lines.extend(["Using baseline thresholds:", ""])
    else:
        lines.extend(
            [
                "Using strict thresholds derived from baseline by:",
                "",
                "- `min_success_rate unchanged`",
                "- `max_mean_us_per_sample * 0.90`",
                "- `max_p95_us_per_sample * 0.90`",
                "",
            ]
        )

    lines.extend(
        [
            "| method | scenario | threshold set | min success | max mean us/sample | max p95 us/sample |",
            "| --- | --- | --- | ---: | ---: | ---: |",
        ]
    )
    for method, scenario in sorted(BASELINE_LIMITS.keys()):
        base = BASELINE_LIMITS[(method, scenario)]
        lim = base if args.profile == "baseline" else strict_limits(base)
        threshold_set = f"{scenario}_baseline" if args.profile == "baseline" else f"{scenario}_strict"
        lines.append(
            f"| {method} | {scenario} | {threshold_set} | {lim.min_success_rate:.6f} | {lim.max_mean_us_per_sample:.9f} | {lim.max_p95_us_per_sample:.9f} |"
        )

    lines.extend(
        [
            "",
            "## Metrics",
            "",
            "| method | scenario | rows | min success | max mean us/sample | max p95 us/sample |",
            "| --- | --- | ---: | ---: | ---: | ---: |",
        ]
    )
    for method, scenario in sorted(grouped.keys()):
        s = grouped[(method, scenario)]
        lines.append(
            f"| {method} | {scenario} | {s.rows} | {s.min_success_rate:.6f} | {s.max_mean_us_per_sample:.9f} | {s.max_p95_us_per_sample:.9f} |"
        )

    args.output_md.parent.mkdir(parents=True, exist_ok=True)
    args.output_md.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

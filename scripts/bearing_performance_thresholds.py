#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, Tuple

EPSILON = 1e-6


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
        description="Validate bearing performance CSV against profile thresholds."
    )
    parser.add_argument("csv_path", type=Path)
    parser.add_argument("--profile", choices=["baseline", "strict"], default="baseline")
    parser.add_argument("--override-min-success-rate", type=float, default=None)
    parser.add_argument("--override-max-mean-us-per-sample", type=float, default=None)
    parser.add_argument("--override-max-p95-us-per-sample", type=float, default=None)
    parser.add_argument("--failed-rows-out", type=Path, default=None)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    rows = list(csv.DictReader(args.csv_path.open(newline="", encoding="utf-8")))
    failures: list[str] = []
    failed_rows: list[dict[str, str]] = []

    for row in rows:
        method = row["method"]
        scenario = row["scenario"]
        key = (method, scenario)
        if key not in BASELINE_LIMITS:
            failures.append(f"FAIL unknown method/scenario row: {row}")
            failed_rows.append(
                {
                    **row,
                    "min_success_rate": "",
                    "max_mean_us_per_sample": "",
                    "max_p95_us_per_sample": "",
                    "reason": "unknown method/scenario",
                }
            )
            continue

        limits = BASELINE_LIMITS[key]
        if args.profile == "strict":
            limits = strict_limits(limits)

        min_success_rate = (
            args.override_min_success_rate
            if args.override_min_success_rate is not None
            else limits.min_success_rate
        )
        max_mean_us_per_sample = (
            args.override_max_mean_us_per_sample
            if args.override_max_mean_us_per_sample is not None
            else limits.max_mean_us_per_sample
        )
        max_p95_us_per_sample = (
            args.override_max_p95_us_per_sample
            if args.override_max_p95_us_per_sample is not None
            else limits.max_p95_us_per_sample
        )

        success_rate = float(row["success_rate"])
        mean_us_per_sample = float(row["mean_us_per_sample"])
        p95_us_per_sample = float(row["p95_us_per_sample"])

        if (
            success_rate + EPSILON < min_success_rate
            or mean_us_per_sample - EPSILON > max_mean_us_per_sample
            or p95_us_per_sample - EPSILON > max_p95_us_per_sample
        ):
            failures.append(
                "FAIL row: "
                f"{row} "
                f"(success={success_rate:.6f} mean_us_per_sample={mean_us_per_sample:.9f} "
                f"p95_us_per_sample={p95_us_per_sample:.9f}; "
                f"limits success>={min_success_rate:.2f} "
                f"mean_us_per_sample<={max_mean_us_per_sample:.9f} "
                f"p95_us_per_sample<={max_p95_us_per_sample:.9f})"
            )
            failed_rows.append(
                {
                    **row,
                    "min_success_rate": f"{min_success_rate:.6f}",
                    "max_mean_us_per_sample": f"{max_mean_us_per_sample:.9f}",
                    "max_p95_us_per_sample": f"{max_p95_us_per_sample:.9f}",
                    "reason": "threshold exceeded",
                }
            )

    if args.failed_rows_out is not None:
        args.failed_rows_out.parent.mkdir(parents=True, exist_ok=True)
        fieldnames = [
            "method",
            "scenario",
            "buffer_size",
            "iterations",
            "measured_count",
            "success_rate",
            "mean_us",
            "p95_us",
            "max_us",
            "mean_us_per_sample",
            "p95_us_per_sample",
            "min_success_rate",
            "max_mean_us_per_sample",
            "max_p95_us_per_sample",
            "reason",
        ]
        with args.failed_rows_out.open("w", newline="", encoding="utf-8") as fh:
            writer = csv.DictWriter(fh, fieldnames=fieldnames)
            writer.writeheader()
            for failed in failed_rows:
                writer.writerow(failed)

    if failures:
        for failure in failures:
            print(failure)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, Tuple


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
}


def stricten(limits: Limits) -> Limits:
    return Limits(
        min_detection=limits.min_detection + 0.02,
        max_false_positive=limits.max_false_positive - 0.02,
        max_mean_error=limits.max_mean_error - 0.15,
        max_p95_error=limits.max_p95_error - 0.25,
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Validate north tick timing CSV against thresholds."
    )
    parser.add_argument("csv_path", type=Path)
    parser.add_argument("--profile", choices=["baseline", "strict"], default="baseline")
    parser.add_argument("--override-min-det", type=float, default=None)
    parser.add_argument("--override-max-fp", type=float, default=None)
    parser.add_argument("--override-max-mean", type=float, default=None)
    parser.add_argument("--override-max-p95", type=float, default=None)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    rows = list(csv.DictReader(args.csv_path.open(newline="", encoding="utf-8")))
    failures = []

    for row in rows:
        mode = row["mode"]
        scenario = row["scenario"]
        key = (mode, scenario)
        if key not in BASELINE_LIMITS:
            failures.append(f"FAIL unknown mode/scenario row: {row}")
            continue

        limits = BASELINE_LIMITS[key]
        if args.profile == "strict":
            limits = stricten(limits)

        min_det = args.override_min_det if args.override_min_det is not None else limits.min_detection
        max_fp = args.override_max_fp if args.override_max_fp is not None else limits.max_false_positive
        max_mean = args.override_max_mean if args.override_max_mean is not None else limits.max_mean_error
        max_p95 = args.override_max_p95 if args.override_max_p95 is not None else limits.max_p95_error

        detection = float(row["detection_rate"])
        false_pos = float(row["false_positive_rate"])
        mean_err = float(row["mean_abs_error_samples"])
        p95_err = float(row["p95_abs_error_samples"])

        if detection < min_det or false_pos > max_fp or mean_err > max_mean or p95_err > max_p95:
            failures.append(
                "FAIL row: "
                f"{row} "
                f"(det={detection:.6f} fp={false_pos:.6f} mean={mean_err:.6f} p95={p95_err:.6f}; "
                f"limits det>={min_det:.2f} fp<={max_fp:.2f} mean<={max_mean:.2f} p95<={max_p95:.2f})"
            )

    if failures:
        for failure in failures:
            print(failure)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Regenerate a gate's BASELINE_LIMITS literal from a fresh metrics JSONL.

There is one limit policy, applied mechanically, so the numbers cannot rot
into archaeology: for a max-direction accuracy metric the limit is the worst
measured row (value plus three inflated standard errors) times 1.5; for a
false-positive rate it is the worst plus 0.01; for a min-direction rate it is
the best-case floor minus 0.02. Timing columns are excluded -- their limits
carry measured cross-session host variance and are set by hand where they
live. Limits at or above 180 degrees are physical-maximum exemptions and are
preserved.

Usage:
    scripts/rederive_limits.py north  <gate_north_tick.jsonl>
    scripts/rederive_limits.py bearing <gate_bearing.jsonl>
    scripts/rederive_limits.py pipeline <gate_pipeline.jsonl>

Prints the replacement literal; splicing it into the report script is a
copy-paste the diff then documents.
"""

from __future__ import annotations

import json
import math
import sys
from collections import defaultdict

GATES = {
    "north": {
        "key": ("mode", "scenario"),
        "max_metrics": [
            "false_positive_rate",
            "mean_abs_error_samples",
            "p95_abs_error_samples",
        ],
        "min_metrics": ["detection_rate"],
    },
    "bearing": {
        "key": ("method", "scenario"),
        "max_metrics": [
            "mean_abs_bearing_error_deg",
            "p95_abs_bearing_error_deg",
            "max_abs_bearing_error_deg",
        ],
        "min_metrics": ["success_rate"],
    },
    "pipeline": {
        "key": ("north_mode", "bearing_method", "scenario"),
        "max_metrics": [
            "false_positive_rate",
            "mean_abs_bearing_error_deg",
            "p95_abs_bearing_error_deg",
            "max_abs_bearing_error_deg",
            "mean_abs_tick_error_samples",
            "p95_abs_tick_error_samples",
        ],
        "min_metrics": ["bearing_success_rate", "detection_rate"],
    },
}

RATE_METRICS = {"false_positive_rate"}
PHYSICAL_MAX_DEG = 180.0


def sig(value: float, figures: int = 3) -> float:
    if value == 0:
        return 0.0
    from math import ceil, floor, log10

    magnitude = floor(log10(abs(value)))
    factor = 10 ** (magnitude - figures + 1)
    return ceil(value / factor) * factor


def main() -> int:
    gate = GATES[sys.argv[1]]
    rows = []
    for line in open(sys.argv[2], encoding="utf-8"):
        record = json.loads(line)
        if record.get("kind") != "meta":
            rows.append(record)

    worst: dict[tuple, dict[str, float]] = defaultdict(dict)
    for row in rows:
        key = tuple(row[k] for k in gate["key"])
        draws = row.get("draws", 1) or 1
        inflation = 1 + 1 / math.sqrt(2 * max(draws - 1, 1)) if draws > 1 else 1.0
        cell = worst[key]
        for metric in gate["max_metrics"]:
            se = float(row.get(f"{metric}_se", 0) or 0)
            value = float(row[metric]) + 3 * se * inflation
            cell[metric] = max(cell.get(metric, 0.0), value)
        for metric in gate["min_metrics"]:
            se = float(row.get(f"{metric}_se", 0) or 0)
            value = float(row[metric]) - 3 * se * inflation
            cell[metric] = min(cell.get(metric, 1.0), value)

    print("BASELINE_LIMITS = {")
    for key in sorted(worst):
        cell = worst[key]
        key_repr = ", ".join(f'"{part}"' for part in key)
        print(f"    ({key_repr}): {{")
        for metric in gate["min_metrics"]:
            limit = max(0.0, cell[metric] - 0.02)
            print(f'        "{metric}": {round(limit, 3)},')  # noqa: B907
        for metric in gate["max_metrics"]:
            need = cell[metric]
            if metric.startswith("max_abs_bearing") and need >= PHYSICAL_MAX_DEG * 0.97:
                # A bound the data already brushes against 180 cannot fail;
                # declare the physical maximum and let the support check's
                # exemption handle it.
                limit = 181.0
            elif metric in RATE_METRICS:
                limit = round(need + 0.01, 4)
            else:
                limit = round(max(sig(need * 1.5), 0.02), 4)
            print(f'        "{metric}": {limit},')
        print("    },")
    print("}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

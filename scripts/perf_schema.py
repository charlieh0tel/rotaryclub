#!/usr/bin/env python3
from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Callable, Dict, Iterable, List, Mapping, Sequence, Tuple


@dataclass(frozen=True)
class MetricSpec:
    name: str
    direction: str  # "min" or "max"
    strict_transform: Callable[[float], float]
    display_name: str
    fmt: str = "{:.6f}"

    def validate(self) -> None:
        if self.direction not in {"min", "max"}:
            raise ValueError(f"invalid direction for {self.name}: {self.direction}")

    def format_value(self, value: float) -> str:
        return self.fmt.format(value)


def apply_profile_limits(
    baseline: Mapping[Tuple[str, str], Mapping[str, float]],
    metrics: Sequence[MetricSpec],
    profile: str,
) -> Dict[Tuple[str, str], Dict[str, float]]:
    if profile not in {"baseline", "strict"}:
        raise ValueError(f"unsupported profile: {profile}")

    out: Dict[Tuple[str, str], Dict[str, float]] = {}
    for key, limits in baseline.items():
        row_out: Dict[str, float] = {}
        for spec in metrics:
            spec.validate()
            base = limits[spec.name]
            row_out[spec.name] = base if profile == "baseline" else spec.strict_transform(base)
        out[key] = row_out
    return out


def summarize_rows(
    rows: Iterable[Mapping[str, str]],
    group_keys: Sequence[str],
    metrics: Sequence[MetricSpec],
) -> Dict[Tuple[str, ...], Dict[str, float]]:
    summary: Dict[Tuple[str, ...], Dict[str, float]] = {}
    for row in rows:
        key = tuple(row[k] for k in group_keys)
        if key not in summary:
            summary[key] = {"rows": 0.0}
            for spec in metrics:
                summary[key][spec.name] = 1.0 if spec.direction == "min" else 0.0
        summary[key]["rows"] += 1.0
        for spec in metrics:
            value = float(row[spec.name])
            if spec.direction == "min":
                summary[key][spec.name] = min(summary[key][spec.name], value)
            else:
                summary[key][spec.name] = max(summary[key][spec.name], value)
    return summary


def evaluate_row_against_limits(
    row: Mapping[str, str],
    limits: Mapping[str, float],
    metrics: Sequence[MetricSpec],
    epsilon: float,
) -> List[str]:
    violations: List[str] = []
    for spec in metrics:
        observed = float(row[spec.name])
        limit = limits[spec.name]
        # NaN compares false against every limit, so a non-finite metric
        # would otherwise pass; treat it as an unconditional violation.
        if not math.isfinite(observed):
            violations.append(spec.name)
        elif spec.direction == "min":
            if observed + epsilon < limit:
                violations.append(spec.name)
        else:
            if observed - epsilon > limit:
                violations.append(spec.name)
    return violations


def coverage_failures(
    rows: Sequence[Mapping[str, str]],
    key_fn: Callable[[Mapping[str, str]], Tuple[str, ...]],
    expected_keys: Iterable[Tuple[str, ...]],
) -> List[str]:
    """Fail when the metric rows do not cover the expected matrix.

    A harness regression that emits no rows (or silently skips scenarios)
    must not pass just because every row that *does* exist is within limits.
    Exact-duplicate rows are also rejected.
    """
    failures: List[str] = []
    if not rows:
        failures.append("FAIL no metric rows produced")
        return failures

    seen = {key_fn(row) for row in rows}
    for key in sorted(set(expected_keys) - seen):
        failures.append(f"FAIL missing metric rows for {key}")

    counts: Dict[Tuple[Tuple[str, str], ...], int] = {}
    for row in rows:
        row_id = tuple(sorted(row.items()))
        counts[row_id] = counts.get(row_id, 0) + 1
    duplicates = sum(count - 1 for count in counts.values() if count > 1)
    if duplicates:
        failures.append(f"FAIL {duplicates} duplicate metric row(s)")

    return failures


def render_markdown_table(
    headers: Sequence[str],
    aligns: Sequence[str],
    rows: Sequence[Sequence[str]],
) -> List[str]:
    if len(headers) != len(aligns):
        raise ValueError("headers and aligns length mismatch")

    sep_parts = []
    for align in aligns:
        if align == "left":
            sep_parts.append("---")
        elif align == "right":
            sep_parts.append("---:")
        else:
            raise ValueError(f"unsupported align: {align}")

    lines = [
        "| " + " | ".join(headers) + " |",
        "| " + " | ".join(sep_parts) + " |",
    ]
    for row in rows:
        lines.append("| " + " | ".join(row) + " |")
    return lines

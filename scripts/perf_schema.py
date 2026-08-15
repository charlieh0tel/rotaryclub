#!/usr/bin/env python3
from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Callable, Dict, Iterable, List, Mapping, Optional, Sequence, Tuple
from pathlib import Path


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
        # NaN compares false against every limit, so a non-finite observation
        # or a non-finite limit (e.g. a `nan` CLI override) would otherwise
        # silently pass; treat either as an unconditional violation.
        if not math.isfinite(observed) or not math.isfinite(limit):
            violations.append(spec.name)
        elif spec.direction == "min":
            if observed + epsilon < limit:
                violations.append(spec.name)
        else:
            if observed - epsilon > limit:
                violations.append(spec.name)
    return violations


def fine_coverage_failures(
    rows: Sequence[Mapping[str, str]],
    group_key_fn: Callable[[Mapping[str, str]], Tuple[str, ...]],
    fine_key_fn: Callable[[Mapping[str, str]], Tuple[str, ...]],
    expected_count: Callable[[Tuple[str, ...]], int],
) -> List[str]:
    """Fail when a group is missing rows of its finer matrix dimensions.

    The coarse coverage check only ensures each (mode, scenario) key appears;
    a harness regression emitting just one buffer/chunk size per scenario
    would still pass. This requires each group to carry its full expected set
    of distinct fine keys (e.g. buffer sizes, chunk-size/offset pairs).
    """
    from collections import defaultdict

    fine: Dict[Tuple[str, ...], set] = defaultdict(set)
    for row in rows:
        fine[group_key_fn(row)].add(fine_key_fn(row))

    failures: List[str] = []
    for group in sorted(fine.keys()):
        want = expected_count(group)
        got = len(fine[group])
        if got != want:
            failures.append(
                f"FAIL group {group} has {got} distinct fine-key row(s), expected {want}"
            )
    return failures


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


def assert_metrics_are_fresh(csv_path: Path, roots: Sequence[str] = ("src", "examples")) -> None:
    """Refuse to evaluate a metrics CSV older than the code that produces it.

    These harnesses write their CSV to stdout and the report script redirects
    it into place, so running the example directly -- `cargo run --example x`
    with the output going anywhere else -- performs the whole measurement and
    leaves the file untouched. Reading it afterwards returns whatever the last
    report run wrote, which is indistinguishable from a fresh result and was
    once real output, so nothing about it looks wrong.

    That cost three isolation runs on a question about which noise seed
    changed a gate row: each one edited a source file, ran the example
    directly, and read a CSV that no longer had anything to do with the code
    under test. All three returned the same number and the conclusion drawn
    from them was that the seed did not matter.

    Comparing mtimes catches exactly that, because the giveaway is always the
    same: source newer than the artifact derived from it.
    """
    if not csv_path.exists():
        raise SystemExit(f"{csv_path} does not exist; run the `run` subcommand first")
    csv_mtime = csv_path.stat().st_mtime
    newest: Optional[Tuple[float, Path]] = None
    for root in roots:
        for path in Path(root).rglob("*.rs"):
            mtime = path.stat().st_mtime
            if newest is None or mtime > newest[0]:
                newest = (mtime, path)
    if newest is not None and newest[0] > csv_mtime:
        raise SystemExit(
            f"{csv_path} is older than {newest[1]}, so it does not describe the current\n"
            f"code. Re-run the `run` subcommand. If you ran the example by hand, note that\n"
            f"it writes its CSV to stdout and this file is only updated by the redirect the\n"
            f"`run` subcommand performs."
        )

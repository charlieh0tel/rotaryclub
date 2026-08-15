#!/usr/bin/env python3
from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Callable, Dict, Iterable, List, Mapping, Optional, Sequence, Tuple
import json
import subprocess
import sys
from datetime import datetime, timezone
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
        # Cells arrive as real numbers now that the metrics are JSON rather
        # than strings parsed out of a CSV, so they are stringified here
        # instead of being assumed to have been already.
        lines.append("| " + " | ".join(str(cell) for cell in row) + " |")
    return lines


def _git(*args: str) -> str:
    try:
        return subprocess.run(
            ["git", *args], check=True, capture_output=True, text=True
        ).stdout.strip()
    except Exception:
        return ""


def write_metrics(path: Path, harness: str, rows_from: Callable[[object], None]) -> None:
    """Write a JSONL metrics file: a meta record, then the harness's rows.

    The meta record is what makes staleness detectable exactly rather than by
    inference. These harnesses print their rows to stdout and this is the only
    place that redirects them into a file, so running the example by hand
    leaves the file untouched and reading it afterwards returns the previous
    run -- which is indistinguishable from a fresh result and was real output
    once. Stamping the commit it came from turns that into a mismatch anyone
    can see.
    """
    path.parent.mkdir(parents=True, exist_ok=True)
    meta = {
        "kind": "meta",
        "harness": harness,
        "git_sha": _git("rev-parse", "HEAD"),
        "git_dirty": bool(_git("status", "--porcelain")),
        "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
    }
    with path.open("w", encoding="utf-8") as handle:
        handle.write(json.dumps(meta) + "\n")
        handle.flush()
        rows_from(handle)


def read_metrics(path: Path) -> Tuple[Dict[str, object], List[Dict[str, object]]]:
    """Read a JSONL metrics file into its meta record and its rows."""
    if not path.exists():
        raise SystemExit(f"{path} does not exist; run the `run` subcommand first")
    meta: Dict[str, object] = {}
    rows: List[Dict[str, object]] = []
    with path.open(encoding="utf-8") as handle:
        for number, line in enumerate(handle, start=1):
            line = line.strip()
            if not line:
                continue
            try:
                record = json.loads(line)
            except json.JSONDecodeError as error:
                raise SystemExit(f"{path}:{number}: not JSON ({error})") from error
            if record.get("kind") == "meta":
                meta = record
            else:
                rows.append(record)
    return meta, rows


def assert_metrics_are_current(path: Path, meta: Mapping[str, object]) -> None:
    """Refuse to evaluate metrics produced by a different commit.

    Replaces an mtime comparison, which could only infer this. The commit the
    numbers came from is now recorded in the file, so the check is exact --
    and it still fires in the case that motivated it, where the example was
    run by hand and the file left describing older code.

    A dirty tree cannot be identified by SHA alone, so that is reported rather
    than trusted: it is the state most likely to be mid-experiment.
    """
    if not meta:
        raise SystemExit(
            f"{path} has no meta record, so what produced it is unknown. "
            f"Re-run the `run` subcommand."
        )
    current = _git("rev-parse", "HEAD")
    recorded = meta.get("git_sha")
    if current and recorded and current != recorded:
        raise SystemExit(
            f"{path} was produced at {str(recorded)[:12]} but HEAD is "
            f"{current[:12]}, so it does not describe the current code. "
            f"Re-run the `run` subcommand.\n"
            f"Note that the harness prints to stdout; only `run` redirects it "
            f"into this file."
        )
    if meta.get("git_dirty"):
        print(
            f"note: {path} was produced from a dirty tree at "
            f"{str(recorded)[:12]}, so the commit does not identify it",
            file=sys.stderr,
        )



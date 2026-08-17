#!/usr/bin/env python3
from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Callable, Dict, Iterable, List, Mapping, Sequence, Tuple
import json
import subprocess
from datetime import datetime, timezone
import hashlib
from pathlib import Path


@dataclass(frozen=True)
class MetricSpec:
    name: str
    direction: str  # "min" or "max"
    display_name: str
    fmt: str = "{:.6f}"

    def validate(self) -> None:
        if self.direction not in {"min", "max"}:
            raise ValueError(f"invalid direction for {self.name}: {self.direction}")

    def format_value(self, value: float) -> str:
        return self.fmt.format(value)


# There is deliberately one profile. A "strict" profile derived by
# transforming the baseline limits ran in CI as advisory-only (its failures
# were swallowed by an exit 0), which is governance theater: a limit either
# means something or it is noise. The baseline limits are re-derived at
# measured-plus-margin instead, so the one gate that exists can fail.


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


def unsupported_metrics(
    row: Mapping[str, object],
    limits: Mapping[str, float],
    metrics: Sequence[MetricSpec],
    *,
    exempt: Sequence[str] = (),
    physical_max: Mapping[str, float] = {},
    sigmas: float = 3.0,
) -> List[str]:
    """Metrics whose spread is too large for this row to support a verdict.

    A limit check answers "is the value inside the limit". This answers the
    question underneath it: is the measurement precise enough for that answer
    to mean anything. A row passing by less than its own noise has not passed;
    it has been rounded in the right direction.

    The test is that `sigmas` standard errors fit inside the margin from the
    value to its limit. Three things it deliberately does not flag:

    Rows already over their limit, which the limit check reports; saying it
    twice helps nobody.

    Metrics named in `exempt`. The timing columns belong here: their spread is
    machine load, and no number of draws averages that away.

    Limits that cannot be crossed. A bearing error cannot exceed 180 degrees,
    so a limit at 181 is a check that never fails, and its arithmetic margin
    -- 177 against 181 -- looks tight while meaning nothing. Declare the
    physical maximum and those stop distorting the answer; leaving them in put
    the pipeline gate's draw requirement at 14 rather than 1.3.

    The standard error is inflated by its own uncertainty before the
    comparison, since with a handful of draws the estimate of the spread is
    itself a rough one -- about 40 percent at four draws.
    """
    draws = float(row.get("draws", 0) or 0)
    failures: List[str] = []
    for spec in metrics:
        if spec.name in exempt:
            continue
        raw_se = row.get(f"{spec.name}_se")
        if raw_se is None:
            # A missing spread column is not evidence of support; it is the
            # absence of the evidence this check runs on. Skipping here made
            # the whole check disappear the day a harness stopped emitting
            # SE columns, with no diagnostic -- demonstrated by stripping
            # them and moving a value to within 0.001 of its limit: PASS.
            failures.append(f"{spec.name} (no {spec.name}_se column)")
            continue
        se = float(raw_se)
        if not math.isfinite(se):
            failures.append(spec.name)
            continue
        limit = limits[spec.name]
        cap = physical_max.get(spec.name)
        if cap is not None and spec.direction == "max" and limit >= cap:
            continue
        value = float(row[spec.name])
        margin = (value - limit) if spec.direction == "min" else (limit - value)
        if margin <= 0.0:
            continue
        inflation = 1.0 + 1.0 / math.sqrt(2.0 * (draws - 1.0)) if draws > 1.0 else float("inf")
        if sigmas * se * inflation > margin:
            failures.append(spec.name)
    return failures


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


def source_digest(roots: Sequence[str] = ("src", "metrics")) -> str:
    """Content hash of the code that produces the metrics.

    Exact in both directions: unchanged code keeps its digest across a commit,
    and an edit changes it whether or not anything was committed.

    Covers the library, the instruments, and the manifests that pin the
    dependencies -- the filter designer and the noise generator live in
    crates, so a lockfile bump changes every measured number just as surely
    as an edit to src/ does. `examples` is deliberately not covered: nothing
    there feeds a gate. The report scripts are not covered either: they hold
    the limits, and changing a limit does not change what was measured.
    """
    digest = hashlib.sha256()
    for manifest in ("Cargo.toml", "Cargo.lock", "metrics/Cargo.toml"):
        path = Path(manifest)
        if not path.is_file():
            raise SystemExit(
                f"source_digest: {manifest!r} is missing. Run these scripts "
                f"from the repository root."
            )
        digest.update(path.as_posix().encode())
        digest.update(path.read_bytes())
    for root in roots:
        # rglob on a missing directory yields nothing and raises nothing, so
        # running from the wrong working directory would hash the empty string
        # and compare it against itself -- a guard that passes for any change.
        if not Path(root).is_dir():
            raise SystemExit(
                f"source_digest: {root!r} is not a directory. Run these scripts "
                f"from the repository root; the freshness check cannot work "
                f"from anywhere else."
            )
        for path in sorted(Path(root).rglob("*.rs")):
            digest.update(path.as_posix().encode())
            digest.update(path.read_bytes())
    return digest.hexdigest()


def write_metrics(path: Path, harness: str, rows_from: Callable[[object], None]) -> None:
    """Write a JSONL metrics file: a meta record, then the harness's rows.

    The harnesses print their rows to stdout and this is the only place that
    redirects them into a file, so running an example by hand leaves the file
    describing whatever ran last. The meta record makes that detectable.
    """
    path.parent.mkdir(parents=True, exist_ok=True)
    meta = {
        "kind": "meta",
        "harness": harness,
        "git_sha": _git("rev-parse", "HEAD"),
        "git_dirty": bool(_git("status", "--porcelain")),
        "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "source_digest": source_digest(),
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
    """Refuse to evaluate metrics that different code produced.

    Keyed on `source_digest` rather than the commit: committing changes no
    source bytes, so a SHA comparison would call every file stale as soon as
    it was committed, and a dirty tree has no SHA identifying its contents.
    """
    if not meta:
        raise SystemExit(
            f"{path} has no meta record, so what produced it is unknown. "
            f"Re-run the `run` subcommand."
        )
    recorded = meta.get("source_digest")
    if not recorded:
        raise SystemExit(f"{path} predates source digests. Re-run the `run` subcommand.")
    current = source_digest()
    if recorded != current:
        raise SystemExit(
            f"{path} was produced from different source than is checked out "
            f"({str(recorded)[:12]} against {current[:12]}), so it does not "
            f"describe the current code. Re-run the `run` subcommand.\n"
            f"Note that the harness prints to stdout; only `run` redirects it "
            f"into this file."
        )



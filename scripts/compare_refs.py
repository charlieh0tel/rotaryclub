#!/usr/bin/env python3
"""Run one metrics harness against two git states and diff the columns.

Both sides go through the report script's own `run`, so they cannot be
produced by different invocations.

Usage:

    scripts/compare_refs.py system_pipeline                  # HEAD vs working tree
    scripts/compare_refs.py bearing_performance --base HEAD~1
    scripts/compare_refs.py north_tick_timing --columns detection_rate

Uncommitted changes are the default "after". They are stashed to measure the
"before" and restored afterwards, including when the run fails.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import Any, Dict, List, Tuple

HARNESSES = {
    "system_pipeline": (
        "scripts/system_pipeline_report.py",
        "target/system-pipeline-perf/gate_pipeline.jsonl",
        ("north_mode", "bearing_method", "scenario", "buffer_size"),
    ),
    "bearing_performance": (
        "scripts/bearing_performance_report.py",
        "target/bearing-perf/gate_bearing.jsonl",
        ("method", "scenario", "buffer_size"),
    ),
    "north_tick_timing": (
        "scripts/north_tick_timing_report.py",
        "target/timing-metrics/gate_north_tick.jsonl",
        ("mode", "scenario", "chunk_size", "start_offset_s"),
    ),
}


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], check=True, capture_output=True, text=True
    ).stdout.strip()


def working_tree_is_dirty() -> bool:
    return bool(git("status", "--porcelain"))


def run_harness(script: str, metrics_path: str) -> List[Dict[str, Any]]:
    subprocess.run([sys.executable, script, "run"], check=True)
    rows = []
    with Path(metrics_path).open(encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            record = json.loads(line)
            if record.get("kind") != "meta":
                rows.append(record)
    return rows


def numeric_columns(rows: List[Dict[str, Any]]) -> List[str]:
    if not rows:
        return []
    out = []
    for name, value in rows[0].items():
        # A standard error is not a metric; it qualifies one. It is consumed
        # alongside its metric below, not diffed on its own.
        if name.endswith("_se"):
            continue
        try:
            float(value)
        except (TypeError, ValueError):
            continue
        out.append(name)
    return out


def key_of(row: Dict[str, Any], keys: Tuple[str, ...]) -> Tuple[str, ...]:
    return tuple(str(row.get(k, "")) for k in keys)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("harness", choices=sorted(HARNESSES))
    parser.add_argument(
        "--base",
        default=None,
        help="git ref for the 'before' side. Default: HEAD, with the working "
        "tree as 'after'. Given a ref, both sides are checked out.",
    )
    parser.add_argument(
        "--head",
        default=None,
        help="git ref for the 'after' side. Default: the working tree.",
    )
    parser.add_argument(
        "--columns",
        default=None,
        help="comma-separated columns to diff. Default: every numeric column.",
    )
    parser.add_argument(
        "--threshold",
        type=float,
        default=0.02,
        help="relative change below which a difference is not printed",
    )
    args = parser.parse_args()

    script, csv_path, keys = HARNESSES[args.harness]

    stashed = False
    original = git("rev-parse", "--abbrev-ref", "HEAD")
    try:
        if args.head is None:
            if args.base is not None:
                raise SystemExit(
                    "--base with a working-tree 'after' is ambiguous; pass --head too"
                )
            if not working_tree_is_dirty():
                raise SystemExit(
                    "working tree is clean, so there is nothing to compare against "
                    "HEAD. Pass --base and --head to compare two refs."
                )
            print("after:  working tree", file=sys.stderr)
            after = run_harness(script, csv_path)
            print("before: HEAD (stashing)", file=sys.stderr)
            git("stash", "push", "-q", "-u")
            stashed = True
            before = run_harness(script, csv_path)
        else:
            base = args.base or "HEAD"
            if working_tree_is_dirty():
                raise SystemExit(
                    "working tree is dirty; commit or stash before comparing two refs"
                )
            print(f"after:  {args.head}", file=sys.stderr)
            git("checkout", "-q", args.head)
            after = run_harness(script, csv_path)
            print(f"before: {base}", file=sys.stderr)
            git("checkout", "-q", base)
            before = run_harness(script, csv_path)
    finally:
        if args.head is not None:
            git("checkout", "-q", original)
        if stashed:
            git("stash", "pop", "-q")

    columns = (
        args.columns.split(",") if args.columns else numeric_columns(after)
    )
    # A non-unique key pairs the wrong rows and reports their difference as a
    # change, so reject one rather than silently producing noise.
    for side, rows in (("before", before), ("after", after)):
        missing = [k for k in keys if rows and k not in rows[0]]
        if missing:
            raise SystemExit(f"{args.harness}: key columns {missing} are not in the output")
        seen = {key_of(r, keys) for r in rows}
        if len(seen) != len(rows):
            raise SystemExit(
                f"{args.harness}: {keys} does not identify rows uniquely on the "
                f"{side} side ({len(rows)} rows, {len(seen)} distinct keys)"
            )
    index: Dict[Tuple[str, ...], Dict[str, Any]] = {key_of(r, keys): r for r in before}

    moved = 0
    unchanged = 0
    for row in after:
        key = key_of(row, keys)
        old = index.get(key)
        if old is None:
            print(f"  {'/'.join(key)}: only in 'after'")
            continue
        for column in columns:
            try:
                x, y = float(old[column]), float(row[column])
            except (KeyError, TypeError, ValueError):
                continue
            if abs(y - x) <= abs(x) * args.threshold:
                unchanged += 1
                continue
            # A difference inside the runs' own sampling noise is not a
            # change: three combined standard errors covers what two
            # honest draws of the same code can disagree by.
            try:
                se = (
                    float(old.get(f"{column}_se", 0) or 0) ** 2
                    + float(row.get(f"{column}_se", 0) or 0) ** 2
                ) ** 0.5
            except (TypeError, ValueError):
                se = 0.0
            if se > 0 and abs(y - x) <= 3 * se:
                unchanged += 1
                continue
            moved += 1
            noise = f"  (3se={3 * se:.3g})" if se > 0 else ""
            print(
                f"  {'/'.join(key):<48} {column:<30} {x:>12.5g} -> {y:<12.5g}{noise}"
            )

    print(
        f"\n{moved} values moved by more than {args.threshold:.0%}, {unchanged} did not.",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

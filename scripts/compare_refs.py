#!/usr/bin/env python3
"""Run one metrics harness against two git states and diff the columns.

Answers "did my change move this number, and which way", which is the question
that kept coming up and kept being answered by hand: stash, run, copy the CSV
somewhere, unstash, run, copy again, compare. Done that way the two halves
drift apart. Once they were not even run the same way -- one through the
report script and one by invoking cargo directly -- and the difference between
the two invocations was read as a difference between the two code states.

So both sides go through the report script's own `run`, from this one place,
and neither side can be produced differently from the other.

Usage:

    scripts/compare_refs.py system_pipeline                  # HEAD vs working tree
    scripts/compare_refs.py bearing_performance --base HEAD~1
    scripts/compare_refs.py north_tick_timing --columns detection_rate

Uncommitted changes are the default "after", since that is usually what is
being asked about. They are stashed to measure the "before" and restored
afterwards, including when the run fails.
"""

from __future__ import annotations

import argparse
import csv
import subprocess
import sys
from pathlib import Path
from typing import Dict, List, Optional, Tuple

HARNESSES = {
    "system_pipeline": (
        "scripts/system_pipeline_report.py",
        "target/system-pipeline-perf/system_pipeline_performance_metrics.csv",
        ("north_mode", "bearing_method", "scenario", "buffer_size"),
    ),
    "bearing_performance": (
        "scripts/bearing_performance_report.py",
        "target/bearing-perf/bearing_performance_metrics.csv",
        ("method", "scenario", "buffer_size"),
    ),
    "north_tick_timing": (
        "scripts/north_tick_timing_report.py",
        "target/timing-metrics/north_tick_timing_metrics.csv",
        ("mode", "scenario", "chunk_size", "start_time_secs"),
    ),
}


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], check=True, capture_output=True, text=True
    ).stdout.strip()


def working_tree_is_dirty() -> bool:
    return bool(git("status", "--porcelain"))


def run_harness(script: str, csv_path: str) -> List[Dict[str, str]]:
    subprocess.run([sys.executable, script, "run"], check=True)
    with Path(csv_path).open(newline="", encoding="utf-8") as handle:
        return list(csv.DictReader(handle))


def numeric_columns(rows: List[Dict[str, str]]) -> List[str]:
    if not rows:
        return []
    out = []
    for name, value in rows[0].items():
        try:
            float(value)
        except (TypeError, ValueError):
            continue
        out.append(name)
    return out


def key_of(row: Dict[str, str], keys: Tuple[str, ...]) -> Tuple[str, ...]:
    return tuple(row.get(k, "") for k in keys)


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
    index: Dict[Tuple[str, ...], Dict[str, str]] = {
        key_of(r, keys): r for r in before
    }

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
            moved += 1
            print(f"  {'/'.join(key):<48} {column:<30} {x:>12.5g} -> {y:<12.5g}")

    print(
        f"\n{moved} values moved by more than {args.threshold:.0%}, {unchanged} did not.",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

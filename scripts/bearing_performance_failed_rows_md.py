#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Render top bearing performance threshold failures from CSV as markdown."
    )
    parser.add_argument("failed_rows_csv", type=Path)
    parser.add_argument("--title", default="Bearing Performance Threshold Failures (Top Rows)")
    parser.add_argument("--max-rows", type=int, default=10)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    print(f"## {args.title}")
    print("")

    if not args.failed_rows_csv.exists():
        print(f"`{args.failed_rows_csv}` not found.")
        return 0

    rows = list(csv.DictReader(args.failed_rows_csv.open(newline="", encoding="utf-8")))
    if not rows:
        print("No threshold failures.")
        return 0

    print(f"Threshold failures: {len(rows)} row(s)")
    print("")
    print("| method | scenario | buffer | success | min success | mean us/sample | max mean | p95 us/sample | max p95 | reason |")
    print("| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |")
    for row in rows[: args.max_rows]:
        print(
            "| {method} | {scenario} | {buffer_size} | {success_rate} | {min_success_rate} | {mean_us_per_sample} | {max_mean_us_per_sample} | {p95_us_per_sample} | {max_p95_us_per_sample} | {reason} |".format(
                **row
            )
        )

    if len(rows) > args.max_rows:
        print("")
        print(f"Showing first {args.max_rows} rows.")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())

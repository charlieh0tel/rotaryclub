#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
import subprocess
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run bearing performance metrics example, validate thresholds, and write markdown summary."
    )
    parser.add_argument("--profile", choices=["baseline", "strict"], default="baseline")
    parser.add_argument("--out-dir", type=Path, default=Path("target/bearing-perf"))
    parser.add_argument("--override-min-success-rate", type=float, default=None)
    parser.add_argument("--override-max-mean-us-per-sample", type=float, default=None)
    parser.add_argument("--override-max-p95-us-per-sample", type=float, default=None)
    return parser.parse_args()


def append_failed_rows_section(summary_path: Path, failed_rows_path: Path, max_rows: int = 10) -> None:
    if not failed_rows_path.exists():
        return

    rows = list(csv.DictReader(failed_rows_path.open(newline="", encoding="utf-8")))
    with summary_path.open("a", encoding="utf-8") as out:
        out.write("\n## Threshold Check\n\n")
        if not rows:
            out.write("No threshold failures.\n")
            return

        out.write(f"Threshold failures: {len(rows)} row(s)\n\n")
        out.write("| method | scenario | buffer | success | min success | mean us/sample | max mean | p95 us/sample | max p95 | reason |\n")
        out.write("| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n")
        for row in rows[:max_rows]:
            out.write(
                "| {method} | {scenario} | {buffer_size} | {success_rate} | {min_success_rate} | {mean_us_per_sample} | {max_mean_us_per_sample} | {p95_us_per_sample} | {max_p95_us_per_sample} | {reason} |\n".format(
                    **row
                )
            )
        if len(rows) > max_rows:
            out.write(f"\nShowing first {max_rows} rows.\n")


def main() -> int:
    args = parse_args()
    out_dir = args.out_dir
    profile = args.profile
    out_dir.mkdir(parents=True, exist_ok=True)

    csv_path = out_dir / "bearing_performance_metrics.csv"
    summary_path = out_dir / f"bearing_performance_{profile}_summary.md"
    failed_rows_path = out_dir / f"bearing_performance_{profile}_failed_rows.csv"

    print("Running bearing performance metrics example...")
    with csv_path.open("w", encoding="utf-8") as out:
        subprocess.run(
            ["cargo", "run", "--release", "--example", "bearing_performance_metrics"],
            check=True,
            stdout=out,
        )
    print(f"Wrote {csv_path}")

    subprocess.run(
        [
            "python3",
            "scripts/bearing_performance_summary.py",
            str(csv_path),
            str(summary_path),
            "--profile",
            profile,
        ],
        check=True,
    )
    print(f"Wrote {summary_path}")

    thresholds_cmd = [
        "python3",
        "scripts/bearing_performance_thresholds.py",
        str(csv_path),
        "--profile",
        profile,
        "--failed-rows-out",
        str(failed_rows_path),
    ]
    if args.override_min_success_rate is not None:
        thresholds_cmd.extend(["--override-min-success-rate", str(args.override_min_success_rate)])
    if args.override_max_mean_us_per_sample is not None:
        thresholds_cmd.extend(
            ["--override-max-mean-us-per-sample", str(args.override_max_mean_us_per_sample)]
        )
    if args.override_max_p95_us_per_sample is not None:
        thresholds_cmd.extend(
            ["--override-max-p95-us-per-sample", str(args.override_max_p95_us_per_sample)]
        )

    threshold_result = subprocess.run(thresholds_cmd)
    append_failed_rows_section(summary_path, failed_rows_path)
    print(f"Wrote {failed_rows_path}")
    if threshold_result.returncode != 0:
        raise SystemExit(threshold_result.returncode)

    print(f"Bearing performance thresholds ({profile}): PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

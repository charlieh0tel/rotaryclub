#!/usr/bin/env python3
from __future__ import annotations

import argparse
import subprocess
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run north tick timing metrics example, validate thresholds, and write markdown summary."
    )
    parser.add_argument("--profile", choices=["baseline", "strict"], default="baseline")
    parser.add_argument("--out-dir", type=Path, default=Path("target/timing-metrics"))
    parser.add_argument("--override-min-det", type=float, default=None)
    parser.add_argument("--override-max-fp", type=float, default=None)
    parser.add_argument("--override-max-mean", type=float, default=None)
    parser.add_argument("--override-max-p95", type=float, default=None)
    return parser.parse_args()


def run(cmd: list[str], env: dict[str, str] | None = None) -> None:
    subprocess.run(cmd, check=True, env=env)


def main() -> int:
    args = parse_args()
    profile = args.profile
    out_dir = args.out_dir
    out_dir.mkdir(parents=True, exist_ok=True)

    csv_path = out_dir / "north_tick_timing_metrics.csv"
    summary_path = out_dir / "north_tick_timing_metrics_summary.md"

    print("Running north tick timing metrics example...")
    with csv_path.open("w", encoding="utf-8") as out:
        subprocess.run(
            ["cargo", "run", "--release", "--example", "north_tick_timing_metrics"],
            check=True,
            stdout=out,
        )
    print(f"Wrote {csv_path}")

    thresholds_cmd = [
        "python3",
        "scripts/north_tick_timing_thresholds.py",
        str(csv_path),
        "--profile",
        profile,
    ]

    if args.override_min_det is not None:
        thresholds_cmd.extend(["--override-min-det", str(args.override_min_det)])
    if args.override_max_fp is not None:
        thresholds_cmd.extend(["--override-max-fp", str(args.override_max_fp)])
    if args.override_max_mean is not None:
        thresholds_cmd.extend(["--override-max-mean", str(args.override_max_mean)])
    if args.override_max_p95 is not None:
        thresholds_cmd.extend(["--override-max-p95", str(args.override_max_p95)])

    run(["python3", "scripts/north_tick_timing_summary.py", str(csv_path), str(summary_path)])
    print(f"Wrote {summary_path}")
    run(thresholds_cmd)
    print(f"North tick timing metrics thresholds ({profile}): PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

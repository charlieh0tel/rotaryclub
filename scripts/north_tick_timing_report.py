#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
import subprocess
from pathlib import Path
from typing import Dict, Tuple

from perf_schema import (
    MetricSpec,
    apply_profile_limits,
    evaluate_row_against_limits,
    render_markdown_table,
    summarize_rows,
)

EPSILON = 1e-3

METRICS = [
    MetricSpec("detection_rate", "min", lambda x: x + 0.02, "detection_rate", "{:.6f}"),
    MetricSpec("false_positive_rate", "max", lambda x: x - 0.02, "false_positive_rate", "{:.6f}"),
    MetricSpec(
        "mean_abs_error_samples",
        "max",
        lambda x: x - 0.15,
        "mean_abs_error_samples",
        "{:.6f}",
    ),
    MetricSpec(
        "p95_abs_error_samples",
        "max",
        lambda x: x - 0.25,
        "p95_abs_error_samples",
        "{:.6f}",
    ),
]

SCENARIO_DEFAULTS: Dict[str, Dict[str, float]] = {
    "clean": {
        "detection_rate": 0.95,
        "false_positive_rate": 0.05,
        "mean_abs_error_samples": 1.0,
        "p95_abs_error_samples": 2.0,
    },
    "noisy_jittered": {
        "detection_rate": 0.90,
        "false_positive_rate": 0.08,
        "mean_abs_error_samples": 1.3,
        "p95_abs_error_samples": 2.5,
    },
    "dropout_burst": {
        "detection_rate": 0.88,
        "false_positive_rate": 0.10,
        "mean_abs_error_samples": 1.4,
        "p95_abs_error_samples": 2.6,
    },
    "impulsive_interference": {
        "detection_rate": 0.85,
        "false_positive_rate": 0.15,
        "mean_abs_error_samples": 1.5,
        "p95_abs_error_samples": 2.8,
    },
    "long_drift": {
        "detection_rate": 0.97,
        "false_positive_rate": 0.03,
        "mean_abs_error_samples": 0.8,
        "p95_abs_error_samples": 1.5,
    },
    "freq_step": {
        "detection_rate": 0.93,
        "false_positive_rate": 0.08,
        "mean_abs_error_samples": 1.2,
        "p95_abs_error_samples": 2.3,
    },
}

MODE_SCENARIO_OVERRIDES: Dict[Tuple[str, str], Dict[str, float]] = {
    ("simple", "impulsive_interference"): {"detection_rate": 0.30},
}

BASELINE_LIMITS: Dict[Tuple[str, str], Dict[str, float]] = {}
for mode in ("dpll", "simple"):
    for scenario, defaults in SCENARIO_DEFAULTS.items():
        merged = dict(defaults)
        merged.update(MODE_SCENARIO_OVERRIDES.get((mode, scenario), {}))
        BASELINE_LIMITS[(mode, scenario)] = merged


def paths(out_dir: Path, profile: str) -> tuple[Path, Path, Path]:
    return (
        out_dir / "north_tick_timing_metrics.csv",
        out_dir / f"north_tick_timing_{profile}_summary.md",
        out_dir / f"north_tick_timing_{profile}_failed_rows.csv",
    )


def run_example(csv_path: Path) -> None:
    csv_path.parent.mkdir(parents=True, exist_ok=True)
    with csv_path.open("w", encoding="utf-8") as out:
        subprocess.run(
            ["cargo", "run", "--release", "--example", "north_tick_timing_metrics"],
            check=True,
            stdout=out,
        )


def evaluate_thresholds(
    rows: list[dict[str, str]],
    profile: str,
    overrides: dict[str, float | None],
) -> tuple[list[str], list[dict[str, str]]]:
    profile_limits = apply_profile_limits(BASELINE_LIMITS, METRICS, profile)
    failures: list[str] = []
    failed_rows: list[dict[str, str]] = []

    for row in rows:
        key = (row["mode"], row["scenario"])
        if key not in BASELINE_LIMITS:
            failures.append(f"FAIL unknown mode/scenario row: {row}")
            failed_rows.append(
                {
                    **row,
                    **{f"limit_{m.name}": "" for m in METRICS},
                    "reason": "unknown mode/scenario",
                }
            )
            continue

        limits = dict(profile_limits[key])
        if overrides["detection_rate"] is not None:
            limits["detection_rate"] = float(overrides["detection_rate"])
        if overrides["false_positive_rate"] is not None:
            limits["false_positive_rate"] = float(overrides["false_positive_rate"])
        if overrides["mean_abs_error_samples"] is not None:
            limits["mean_abs_error_samples"] = float(overrides["mean_abs_error_samples"])
        if overrides["p95_abs_error_samples"] is not None:
            limits["p95_abs_error_samples"] = float(overrides["p95_abs_error_samples"])

        violations = evaluate_row_against_limits(row, limits, METRICS, EPSILON)
        if violations:
            observed = " ".join(f"{m.name}={m.format_value(float(row[m.name]))}" for m in METRICS)
            limits_text = " ".join(f"limit_{m.name}={m.format_value(limits[m.name])}" for m in METRICS)
            failures.append(
                f"FAIL row: {row} ({observed}; {limits_text}; violations={','.join(violations)})"
            )
            failed_rows.append(
                {
                    **row,
                    **{f"limit_{m.name}": m.format_value(limits[m.name]) for m in METRICS},
                    "reason": "threshold exceeded",
                }
            )

    return failures, failed_rows


def write_failed_rows_csv(rows: list[dict[str, str]], failed_rows_path: Path, input_rows: list[dict[str, str]]) -> None:
    failed_rows_path.parent.mkdir(parents=True, exist_ok=True)
    input_fields = list(input_rows[0].keys()) if input_rows else []
    limit_fields = [f"limit_{m.name}" for m in METRICS]
    fieldnames = input_fields + limit_fields + ["reason"]
    with failed_rows_path.open("w", newline="", encoding="utf-8") as fh:
        writer = csv.DictWriter(fh, fieldnames=fieldnames)
        writer.writeheader()
        for row in rows:
            writer.writerow(row)


def build_summary_lines(rows: list[dict[str, str]], profile: str) -> list[str]:
    grouped = summarize_rows(rows, group_keys=["mode", "scenario"], metrics=METRICS)
    profile_limits = apply_profile_limits(BASELINE_LIMITS, METRICS, profile)

    lines = [
        "# North Tick Timing Metrics Summary",
        "",
        f"- Profile: `{profile}`",
        "- This markdown file is the detailed metrics artifact generated from CSV.",
        "- CI step-summary status notes are separate and only indicate pass/fail state.",
        "",
        "## Threshold Profile",
        "",
    ]
    if profile == "baseline":
        lines.extend(["Using baseline thresholds.", ""])
    else:
        lines.extend(
            [
                "Using strict thresholds derived from metric transforms:",
                "",
                "- `detection_rate + 0.02`",
                "- `false_positive_rate - 0.02`",
                "- `mean_abs_error_samples - 0.15`",
                "- `p95_abs_error_samples - 0.25`",
                "",
            ]
        )

    threshold_headers = ["mode", "scenario", "threshold set"] + [f"limit {m.display_name}" for m in METRICS]
    threshold_aligns = ["left", "left", "left"] + ["right"] * len(METRICS)
    threshold_rows = []
    for mode, scenario in sorted(BASELINE_LIMITS.keys()):
        threshold_set = "impulsive_interference_simple_mode" if (mode, scenario) == ("simple", "impulsive_interference") else scenario
        lim = profile_limits[(mode, scenario)]
        threshold_rows.append([mode, scenario, threshold_set] + [m.format_value(lim[m.name]) for m in METRICS])
    lines.extend(render_markdown_table(threshold_headers, threshold_aligns, threshold_rows))

    lines.extend(["", "## Metrics", ""])
    metric_headers = ["mode", "scenario", "rows"] + [m.display_name for m in METRICS]
    metric_aligns = ["left", "left", "right"] + ["right"] * len(METRICS)
    metric_rows = []
    for mode, scenario in sorted(grouped.keys()):
        s = grouped[(mode, scenario)]
        metric_rows.append([mode, scenario, str(int(s["rows"]))] + [m.format_value(s[m.name]) for m in METRICS])
    lines.extend(render_markdown_table(metric_headers, metric_aligns, metric_rows))
    return lines


def append_failed_rows_md(lines: list[str], failed_rows_path: Path, max_rows: int) -> list[str]:
    lines.extend(["", "## Threshold Check", ""])
    if not failed_rows_path.exists():
        lines.append(f"`{failed_rows_path}` not found.")
        return lines
    rows = list(csv.DictReader(failed_rows_path.open(newline="", encoding="utf-8")))
    if not rows:
        lines.append("No threshold failures.")
        return lines
    lines.append(f"Threshold failures: {len(rows)} row(s)")
    lines.append("")
    headers = (
        ["mode", "scenario", "chunk", "offset"]
        + [m.display_name for m in METRICS]
        + [f"limit {m.display_name}" for m in METRICS]
        + ["reason"]
    )
    aligns = ["left", "left", "right", "right"] + ["right"] * (len(METRICS) * 2) + ["left"]
    table_rows = []
    for row in rows[:max_rows]:
        table_rows.append(
            [
                row.get("mode", ""),
                row.get("scenario", ""),
                row.get("chunk_size", ""),
                row.get("start_offset_s", ""),
                *[row.get(m.name, "") for m in METRICS],
                *[row.get(f"limit_{m.name}", "") for m in METRICS],
                row.get("reason", ""),
            ]
        )
    lines.extend(render_markdown_table(headers, aligns, table_rows))
    if len(rows) > max_rows:
        lines.extend(["", f"Showing first {max_rows} rows."])
    return lines


def write_summary(csv_path: Path, summary_path: Path, profile: str, failed_rows_path: Path | None, max_rows: int) -> None:
    rows = list(csv.DictReader(csv_path.open(newline="", encoding="utf-8")))
    lines = build_summary_lines(rows, profile)
    if failed_rows_path is not None:
        lines = append_failed_rows_md(lines, failed_rows_path, max_rows)
    summary_path.parent.mkdir(parents=True, exist_ok=True)
    summary_path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def print_failed_rows_md(failed_rows_path: Path, title: str, max_rows: int) -> None:
    lines = [f"## {title}", ""]
    append_failed_rows_md(lines, failed_rows_path, max_rows)
    print("\n".join(lines))


def cmd_run(args: argparse.Namespace) -> int:
    csv_path, _, _ = paths(args.out_dir, args.profile)
    print("Running north tick timing metrics example...")
    run_example(csv_path)
    print(f"Wrote {csv_path}")
    return 0


def cmd_check(args: argparse.Namespace) -> int:
    csv_path, _, failed_rows_path = paths(args.out_dir, args.profile)
    rows = list(csv.DictReader(csv_path.open(newline="", encoding="utf-8")))
    overrides = {
        "detection_rate": args.override_min_det,
        "false_positive_rate": args.override_max_fp,
        "mean_abs_error_samples": args.override_max_mean,
        "p95_abs_error_samples": args.override_max_p95,
    }
    failures, failed_rows = evaluate_thresholds(rows, args.profile, overrides)
    write_failed_rows_csv(failed_rows, failed_rows_path, rows)
    print(f"Wrote {failed_rows_path}")
    if failures:
        for failure in failures:
            print(failure)
        return 1
    print(f"North tick timing metrics thresholds ({args.profile}): PASS")
    return 0


def cmd_summary(args: argparse.Namespace) -> int:
    csv_path, summary_path, failed_rows_path = paths(args.out_dir, args.profile)
    include_failed = failed_rows_path if args.include_failed_rows else None
    write_summary(csv_path, summary_path, args.profile, include_failed, args.max_rows)
    print(f"Wrote {summary_path}")
    return 0


def cmd_failed_rows(args: argparse.Namespace) -> int:
    print_failed_rows_md(args.failed_rows_csv, args.title, args.max_rows)
    return 0


def cmd_ci(args: argparse.Namespace) -> int:
    csv_path, summary_path, failed_rows_path = paths(args.out_dir, args.profile)
    print("Running north tick timing metrics example...")
    run_example(csv_path)
    print(f"Wrote {csv_path}")

    rows = list(csv.DictReader(csv_path.open(newline="", encoding="utf-8")))
    overrides = {
        "detection_rate": args.override_min_det,
        "false_positive_rate": args.override_max_fp,
        "mean_abs_error_samples": args.override_max_mean,
        "p95_abs_error_samples": args.override_max_p95,
    }
    failures, failed_rows = evaluate_thresholds(rows, args.profile, overrides)
    write_failed_rows_csv(failed_rows, failed_rows_path, rows)
    print(f"Wrote {failed_rows_path}")

    write_summary(csv_path, summary_path, args.profile, failed_rows_path, args.max_rows)
    print(f"Wrote {summary_path}")

    if failures:
        for failure in failures:
            print(failure)
        return 1
    print(f"North tick timing metrics thresholds ({args.profile}): PASS")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="North tick timing report tool")
    sub = parser.add_subparsers(dest="command", required=True)

    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--profile", choices=["baseline", "strict"], default="baseline")
    common.add_argument("--out-dir", type=Path, default=Path("target/timing-metrics"))

    for name in ("run", "check", "summary", "ci"):
        p = sub.add_parser(name, parents=[common])
        if name in {"check", "ci"}:
            p.add_argument("--override-min-det", type=float, default=None)
            p.add_argument("--override-max-fp", type=float, default=None)
            p.add_argument("--override-max-mean", type=float, default=None)
            p.add_argument("--override-max-p95", type=float, default=None)
        if name in {"summary", "ci"}:
            p.add_argument("--max-rows", type=int, default=10)
        if name == "summary":
            p.add_argument("--include-failed-rows", action="store_true")

    pf = sub.add_parser("failed-rows")
    pf.add_argument("failed_rows_csv", type=Path)
    pf.add_argument("--title", default="Threshold Failures (Top Rows)")
    pf.add_argument("--max-rows", type=int, default=10)
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    if args.command == "run":
        return cmd_run(args)
    if args.command == "check":
        return cmd_check(args)
    if args.command == "summary":
        return cmd_summary(args)
    if args.command == "failed-rows":
        return cmd_failed_rows(args)
    if args.command == "ci":
        return cmd_ci(args)
    raise ValueError(f"unsupported command: {args.command}")


if __name__ == "__main__":
    raise SystemExit(main())

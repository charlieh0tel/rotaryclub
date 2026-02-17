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

EPSILON = 1e-6

METRICS = [
    MetricSpec("bearing_success_rate", "min", lambda x: min(1.0, x + 0.001), "bearing_success_rate", "{:.6f}"),
    MetricSpec("detection_rate", "min", lambda x: min(1.0, x + 0.001), "detection_rate", "{:.6f}"),
    MetricSpec("false_positive_rate", "max", lambda x: max(0.0, x - 0.001), "false_positive_rate", "{:.6f}"),
    MetricSpec("mean_us_per_sample", "max", lambda x: x * 0.98, "mean_us_per_sample", "{:.9f}"),
    MetricSpec("p95_us_per_sample", "max", lambda x: x * 0.98, "p95_us_per_sample", "{:.9f}"),
    MetricSpec(
        "mean_abs_bearing_error_deg",
        "max",
        lambda x: x * 0.98,
        "mean_abs_bearing_error_deg",
        "{:.6f}",
    ),
    MetricSpec(
        "p95_abs_bearing_error_deg",
        "max",
        lambda x: x * 0.98,
        "p95_abs_bearing_error_deg",
        "{:.6f}",
    ),
    MetricSpec(
        "max_abs_bearing_error_deg",
        "max",
        lambda x: x,
        "max_abs_bearing_error_deg",
        "{:.6f}",
    ),
    MetricSpec(
        "mean_abs_tick_error_samples",
        "max",
        lambda x: x * 0.98,
        "mean_abs_tick_error_samples",
        "{:.6f}",
    ),
    MetricSpec(
        "p95_abs_tick_error_samples",
        "max",
        lambda x: x * 0.98,
        "p95_abs_tick_error_samples",
        "{:.6f}",
    ),
]

BASELINE_LIMITS: Dict[Tuple[str, str, str], Dict[str, float]] = {}

# Mode+method timing defaults
MODE_METHOD_DEFAULTS: Dict[Tuple[str, str], Dict[str, float]] = {
    ("dpll", "correlation"): {
        "bearing_success_rate": 0.99,
        "detection_rate": 0.995,
        "false_positive_rate": 0.01,
        "mean_us_per_sample": 0.75,
        "p95_us_per_sample": 0.90,
        "mean_abs_bearing_error_deg": 15.0,
        "p95_abs_bearing_error_deg": 35.0,
        "max_abs_bearing_error_deg": 60.0,
        "mean_abs_tick_error_samples": 0.5,
        "p95_abs_tick_error_samples": 1.0,
    },
    ("simple", "correlation"): {
        "bearing_success_rate": 0.99,
        "detection_rate": 0.995,
        "false_positive_rate": 0.01,
        "mean_us_per_sample": 0.75,
        "p95_us_per_sample": 0.90,
        "mean_abs_bearing_error_deg": 15.0,
        "p95_abs_bearing_error_deg": 35.0,
        "max_abs_bearing_error_deg": 65.0,
        "mean_abs_tick_error_samples": 0.5,
        "p95_abs_tick_error_samples": 1.0,
    },
    ("dpll", "zero_crossing"): {
        "bearing_success_rate": 0.99,
        "detection_rate": 0.995,
        "false_positive_rate": 0.01,
        "mean_us_per_sample": 0.50,
        "p95_us_per_sample": 0.60,
        "mean_abs_bearing_error_deg": 15.0,
        "p95_abs_bearing_error_deg": 35.0,
        "max_abs_bearing_error_deg": 60.0,
        "mean_abs_tick_error_samples": 0.5,
        "p95_abs_tick_error_samples": 1.0,
    },
    ("simple", "zero_crossing"): {
        "bearing_success_rate": 0.99,
        "detection_rate": 0.995,
        "false_positive_rate": 0.01,
        "mean_us_per_sample": 0.50,
        "p95_us_per_sample": 0.60,
        "mean_abs_bearing_error_deg": 15.0,
        "p95_abs_bearing_error_deg": 35.0,
        "max_abs_bearing_error_deg": 65.0,
        "mean_abs_tick_error_samples": 0.5,
        "p95_abs_tick_error_samples": 1.0,
    },
}

SCENARIOS = ["clean", "noisy_jittered", "harmonic_contaminated", "low_snr_dc"]

for north_mode in ("dpll", "simple"):
    for bearing_method in ("correlation", "zero_crossing"):
        for scenario in SCENARIOS:
            BASELINE_LIMITS[(north_mode, bearing_method, scenario)] = dict(
                MODE_METHOD_DEFAULTS[(north_mode, bearing_method)]
            )

# Scenario-specific overrides
for north_mode in ("dpll", "simple"):
    for bearing_method in ("correlation", "zero_crossing"):
        BASELINE_LIMITS[(north_mode, bearing_method, "low_snr_dc")].update(
            {
                "bearing_success_rate": 0.95,
                "detection_rate": 0.97,
                "false_positive_rate": 0.05,
            }
        )

BASELINE_LIMITS[("dpll", "correlation", "low_snr_dc")].update(
    {
        "mean_abs_bearing_error_deg": 14.0,
        "p95_abs_bearing_error_deg": 32.0,
        "max_abs_bearing_error_deg": 170.0,
    }
)
BASELINE_LIMITS[("dpll", "zero_crossing", "low_snr_dc")].update(
    {
        "mean_abs_bearing_error_deg": 14.0,
        "p95_abs_bearing_error_deg": 32.0,
        "max_abs_bearing_error_deg": 170.0,
    }
)
BASELINE_LIMITS[("simple", "correlation", "low_snr_dc")].update(
    {
        "mean_abs_bearing_error_deg": 75.0,
        "p95_abs_bearing_error_deg": 170.0,
        "max_abs_bearing_error_deg": 181.0,
    }
)
BASELINE_LIMITS[("simple", "zero_crossing", "low_snr_dc")].update(
    {
        "mean_abs_bearing_error_deg": 75.0,
        "p95_abs_bearing_error_deg": 170.0,
        "max_abs_bearing_error_deg": 181.0,
    }
)


def paths(out_dir: Path, profile: str) -> tuple[Path, Path, Path]:
    return (
        out_dir / "system_pipeline_performance_metrics.csv",
        out_dir / f"system_pipeline_performance_{profile}_summary.md",
        out_dir / f"system_pipeline_performance_{profile}_failed_rows.csv",
    )


def run_example(csv_path: Path) -> None:
    csv_path.parent.mkdir(parents=True, exist_ok=True)
    with csv_path.open("w", encoding="utf-8") as out:
        subprocess.run(
            ["cargo", "run", "--release", "--example", "system_pipeline_performance_metrics"],
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
        key = (row["north_mode"], row["bearing_method"], row["scenario"])
        if key not in BASELINE_LIMITS:
            failures.append(f"FAIL unknown key row: {row}")
            failed_rows.append(
                {
                    **row,
                    **{f"limit_{m.name}": "" for m in METRICS},
                    "reason": "unknown north_mode/bearing_method/scenario",
                }
            )
            continue

        limits = dict(profile_limits[key])
        for metric_name, value in overrides.items():
            if value is not None:
                limits[metric_name] = float(value)

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
    grouped = summarize_rows(rows, group_keys=["north_mode", "bearing_method", "scenario"], metrics=METRICS)
    profile_limits = apply_profile_limits(BASELINE_LIMITS, METRICS, profile)

    lines = [
        "# System Pipeline Performance Summary",
        "",
        f"- Profile: `{profile}`",
        "- Scope: full stack (north tracking + bearing calculation).",
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
                "- `bearing_success_rate + 0.001`",
                "- `detection_rate + 0.001`",
                "- `false_positive_rate - 0.001`",
                "- `*_us_per_sample * 0.98`",
                "- `mean/p95 bearing_error * 0.98`",
                "- `max_abs_bearing_error_deg unchanged`",
                "- `*_tick_error_samples * 0.98`",
                "",
            ]
        )

    threshold_headers = ["north", "bearing", "scenario", "threshold set"] + [f"limit {m.display_name}" for m in METRICS]
    threshold_aligns = ["left", "left", "left", "left"] + ["right"] * len(METRICS)
    threshold_rows = []
    for north_mode, bearing_method, scenario in sorted(BASELINE_LIMITS.keys()):
        lim = profile_limits[(north_mode, bearing_method, scenario)]
        threshold_rows.append(
            [north_mode, bearing_method, scenario, f"{north_mode}_{bearing_method}_{scenario}_{profile}"]
            + [m.format_value(lim[m.name]) for m in METRICS]
        )
    lines.extend(render_markdown_table(threshold_headers, threshold_aligns, threshold_rows))

    lines.extend(["", "## Metrics", ""])
    metric_headers = ["north", "bearing", "scenario", "rows"] + [m.display_name for m in METRICS]
    metric_aligns = ["left", "left", "left", "right"] + ["right"] * len(METRICS)
    metric_rows = []
    for north_mode, bearing_method, scenario in sorted(grouped.keys()):
        s = grouped[(north_mode, bearing_method, scenario)]
        metric_rows.append(
            [north_mode, bearing_method, scenario, str(int(s["rows"]))]
            + [m.format_value(s[m.name]) for m in METRICS]
        )
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
        ["north", "bearing", "scenario", "buffer"]
        + [m.display_name for m in METRICS]
        + [f"limit {m.display_name}" for m in METRICS]
        + ["reason"]
    )
    aligns = ["left", "left", "left", "right"] + ["right"] * (len(METRICS) * 2) + ["left"]
    table_rows = []
    for row in rows[:max_rows]:
        table_rows.append(
            [
                row.get("north_mode", ""),
                row.get("bearing_method", ""),
                row.get("scenario", ""),
                row.get("buffer_size", ""),
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
    print("Running system pipeline performance example...")
    run_example(csv_path)
    print(f"Wrote {csv_path}")
    return 0


def cmd_check(args: argparse.Namespace) -> int:
    csv_path, _, failed_rows_path = paths(args.out_dir, args.profile)
    rows = list(csv.DictReader(csv_path.open(newline="", encoding="utf-8")))
    overrides = {
        "bearing_success_rate": args.override_min_bearing_success,
        "detection_rate": args.override_min_detection_rate,
        "false_positive_rate": args.override_max_false_positive,
        "mean_us_per_sample": args.override_max_mean_us_per_sample,
        "p95_us_per_sample": args.override_max_p95_us_per_sample,
        "mean_abs_bearing_error_deg": args.override_max_mean_bearing_error_deg,
        "p95_abs_bearing_error_deg": args.override_max_p95_bearing_error_deg,
        "max_abs_bearing_error_deg": args.override_max_bearing_error_deg,
        "mean_abs_tick_error_samples": args.override_max_mean_tick_error_samples,
        "p95_abs_tick_error_samples": args.override_max_p95_tick_error_samples,
    }
    failures, failed_rows = evaluate_thresholds(rows, args.profile, overrides)
    write_failed_rows_csv(failed_rows, failed_rows_path, rows)
    print(f"Wrote {failed_rows_path}")
    if failures:
        for failure in failures:
            print(failure)
        return 1
    print(f"System pipeline performance thresholds ({args.profile}): PASS")
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
    print("Running system pipeline performance example...")
    run_example(csv_path)
    print(f"Wrote {csv_path}")

    rows = list(csv.DictReader(csv_path.open(newline="", encoding="utf-8")))
    overrides = {
        "bearing_success_rate": args.override_min_bearing_success,
        "detection_rate": args.override_min_detection_rate,
        "false_positive_rate": args.override_max_false_positive,
        "mean_us_per_sample": args.override_max_mean_us_per_sample,
        "p95_us_per_sample": args.override_max_p95_us_per_sample,
        "mean_abs_bearing_error_deg": args.override_max_mean_bearing_error_deg,
        "p95_abs_bearing_error_deg": args.override_max_p95_bearing_error_deg,
        "max_abs_bearing_error_deg": args.override_max_bearing_error_deg,
        "mean_abs_tick_error_samples": args.override_max_mean_tick_error_samples,
        "p95_abs_tick_error_samples": args.override_max_p95_tick_error_samples,
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
    print(f"System pipeline performance thresholds ({args.profile}): PASS")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="System pipeline performance report tool")
    sub = parser.add_subparsers(dest="command", required=True)

    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--profile", choices=["baseline", "strict"], default="baseline")
    common.add_argument("--out-dir", type=Path, default=Path("target/system-pipeline-perf"))

    for name in ("run", "check", "summary", "ci"):
        p = sub.add_parser(name, parents=[common])
        if name in {"check", "ci"}:
            p.add_argument("--override-min-bearing-success", type=float, default=None)
            p.add_argument("--override-min-detection-rate", type=float, default=None)
            p.add_argument("--override-max-false-positive", type=float, default=None)
            p.add_argument("--override-max-mean-us-per-sample", type=float, default=None)
            p.add_argument("--override-max-p95-us-per-sample", type=float, default=None)
            p.add_argument("--override-max-mean-bearing-error-deg", type=float, default=None)
            p.add_argument("--override-max-p95-bearing-error-deg", type=float, default=None)
            p.add_argument("--override-max-bearing-error-deg", type=float, default=None)
            p.add_argument("--override-max-mean-tick-error-samples", type=float, default=None)
            p.add_argument("--override-max-p95-tick-error-samples", type=float, default=None)
        if name in {"summary", "ci"}:
            p.add_argument("--max-rows", type=int, default=10)
        if name == "summary":
            p.add_argument("--include-failed-rows", action="store_true")

    pf = sub.add_parser("failed-rows")
    pf.add_argument("failed_rows_csv", type=Path)
    pf.add_argument("--title", default="System Pipeline Threshold Failures (Top Rows)")
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

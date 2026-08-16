#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path
from typing import Dict, Tuple

from perf_schema import (
    assert_metrics_are_current,
    read_metrics,
    write_metrics,
    coverage_failures,
    fine_coverage_failures,
    MetricSpec,
    apply_profile_limits,
    evaluate_row_against_limits,
    unsupported_metrics,
    render_markdown_table,
    summarize_rows,
)

# Timing spread is machine load, which no number of draws averages away, so
# those columns are not asked whether they support a verdict.
SUPPORT_EXEMPT = ()
# A bearing error cannot exceed 180 degrees. A limit at or above that cannot
# be crossed, so its margin is not a real one and must not demand precision.
PHYSICAL_MAX = {}

EPSILON = 1e-3

# Strict transforms are relative rather than absolute offsets: the baseline
# limits now sit close to measurement (a 1.5x margin on the error columns),
# and subtracting a fixed 0.25 samples from a p95 limit of 0.19 would demand
# a negative error.
METRICS = [
    MetricSpec("detection_rate", "min", lambda x: x + 0.01, "detection_rate", "{:.6f}"),
    MetricSpec(
        "false_positive_rate",
        "max",
        lambda x: x * 0.75,
        "false_positive_rate",
        "{:.6f}",
    ),
    MetricSpec(
        "mean_abs_error_samples",
        "max",
        lambda x: x / 1.25,
        "mean_abs_error_samples",
        "{:.6f}",
    ),
    MetricSpec(
        "p95_abs_error_samples",
        "max",
        lambda x: x / 1.25,
        "p95_abs_error_samples",
        "{:.6f}",
    ),
]

# Derived from a fresh run of the gate after the truth went fractional
# (band-limited pulses at fractional epochs; commit history has the
# measured worsts). Margin: detection -0.02, false positives +0.02,
# error columns x1.5. The old per-scenario defaults sat 3-35x above
# measurement and could not fail.
BASELINE_LIMITS: Dict[Tuple[str, str], Dict[str, float]] = {
    ("dpll", "clean"): {
        "detection_rate": 0.979,
        "false_positive_rate": 0.02,
        "mean_abs_error_samples": 0.24,
        "p95_abs_error_samples": 0.84,
    },
    ("dpll", "dropout_burst"): {
        "detection_rate": 0.978,
        "false_positive_rate": 0.09,
        "mean_abs_error_samples": 0.52,
        "p95_abs_error_samples": 0.99,
    },
    ("dpll", "freq_step"): {
        "detection_rate": 0.979,
        "false_positive_rate": 0.02,
        "mean_abs_error_samples": 0.45,
        "p95_abs_error_samples": 1.08,
    },
    ("dpll", "impulsive_interference"): {
        "detection_rate": 0.978,
        "false_positive_rate": 0.03,
        "mean_abs_error_samples": 0.51,
        "p95_abs_error_samples": 0.99,
    },
    ("dpll", "long_drift"): {
        "detection_rate": 0.979,
        "false_positive_rate": 0.02,
        "mean_abs_error_samples": 0.05,
        "p95_abs_error_samples": 0.29,
    },
    ("dpll", "noisy_jittered"): {
        "detection_rate": 0.978,
        "false_positive_rate": 0.03,
        "mean_abs_error_samples": 0.51,
        "p95_abs_error_samples": 0.99,
    },
    ("simple", "clean"): {
        "detection_rate": 0.979,
        "false_positive_rate": 0.02,
        "mean_abs_error_samples": 0.12,
        "p95_abs_error_samples": 0.19,
    },
    ("simple", "dropout_burst"): {
        "detection_rate": 0.975,
        "false_positive_rate": 0.02,
        "mean_abs_error_samples": 0.16,
        "p95_abs_error_samples": 0.3,
    },
    ("simple", "freq_step"): {
        "detection_rate": 0.98,
        "false_positive_rate": 0.02,
        "mean_abs_error_samples": 0.18,
        "p95_abs_error_samples": 0.36,
    },
    ("simple", "impulsive_interference"): {
        "detection_rate": 0.299,
        "false_positive_rate": 0.03,
        "mean_abs_error_samples": 0.17,
        "p95_abs_error_samples": 0.32,
    },
    ("simple", "long_drift"): {
        "detection_rate": 0.98,
        "false_positive_rate": 0.02,
        "mean_abs_error_samples": 0.12,
        "p95_abs_error_samples": 0.19,
    },
    ("simple", "noisy_jittered"): {
        "detection_rate": 0.975,
        "false_positive_rate": 0.02,
        "mean_abs_error_samples": 0.16,
        "p95_abs_error_samples": 0.31,
    },
}

# Distinct (chunk_size, start_offset) rows expected per scenario. Most sweep
# the full chunk x offset matrix; freq_step and long_drift use reduced sets.
DEFAULT_FINE_COUNT = 18
EXPECTED_FINE_COUNT = {
    "freq_step": 2,
    "long_drift": 2,
}


def paths(out_dir: Path, profile: str) -> tuple[Path, Path, Path]:
    return (
        out_dir / "gate_north_tick.jsonl",
        out_dir / f"north_tick_timing_{profile}_summary.md",
        out_dir / f"north_tick_timing_{profile}_failed_rows.jsonl",
    )


def run_example(metrics_path: Path) -> None:
    write_metrics(
        metrics_path,
        "north_tick_timing",
        lambda handle: subprocess.run(
            ["cargo", "run", "--release", "-p", "rotaryclub-metrics", "--bin", "gate_north_tick"],
            check=True,
            stdout=handle,
        ),
    )


def evaluate_thresholds(
    rows: list[dict[str, str]],
    profile: str,
    overrides: dict[str, float | None],
) -> tuple[list[str], list[dict[str, str]]]:
    profile_limits = apply_profile_limits(BASELINE_LIMITS, METRICS, profile)
    failures: list[str] = []
    failed_rows: list[dict[str, str]] = []

    failures.extend(
        coverage_failures(rows, lambda row: (row["mode"], row["scenario"]), BASELINE_LIMITS.keys())
    )
    failures.extend(
        fine_coverage_failures(
            rows,
            lambda row: (row["mode"], row["scenario"]),
            lambda row: (row["chunk_size"], row["start_offset_s"]),
            lambda group: EXPECTED_FINE_COUNT.get(group[1], DEFAULT_FINE_COUNT),
        )
    )

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
        unsupported = unsupported_metrics(
            row,
            limits,
            METRICS,
            exempt=SUPPORT_EXEMPT,
            physical_max=PHYSICAL_MAX,
        )
        if unsupported and not violations:
            # Passing by less than the row's own noise is not passing. Either
            # the draw count is too low for this margin or the value has
            # drifted close enough to its limit that the verdict is a coin
            # toss; both want attention before the gate starts flapping.
            failures.append(
                f"FAIL row: {row} (within limits but not supported by the "
                f"measurement: {','.join(unsupported)}; raise the draw count "
                f"or widen the margin)"
            )
            failed_rows.append({**row, "reason": "unsupported by the measurement"})
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


def write_failed_rows(rows: list[dict[str, str]], failed_rows_path: Path, input_rows: list[dict[str, str]]) -> None:
    failed_rows_path.parent.mkdir(parents=True, exist_ok=True)
    input_fields = list(input_rows[0].keys()) if input_rows else []
    limit_fields = [f"limit_{m.name}" for m in METRICS]
    fieldnames = input_fields + limit_fields + ["reason"]
    with failed_rows_path.open("w", encoding="utf-8") as fh:
        for row in rows:
            fh.write(json.dumps(row) + "\n")


def build_summary_lines(rows: list[dict[str, str]], profile: str) -> list[str]:
    grouped = summarize_rows(rows, group_keys=["mode", "scenario"], metrics=METRICS)
    profile_limits = apply_profile_limits(BASELINE_LIMITS, METRICS, profile)

    lines = [
        "# North Tick Timing Metrics Summary",
        "",
        f"- Profile: `{profile}`",
        "- This markdown file is the detailed metrics artifact generated from the JSONL metrics.",
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
    _, rows = read_metrics(failed_rows_path)
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


def write_summary(metrics_path: Path, summary_path: Path, profile: str, failed_rows_path: Path | None, max_rows: int) -> None:
    meta, rows = read_metrics(metrics_path)
    assert_metrics_are_current(metrics_path, meta)
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
    metrics_path, _, _ = paths(args.out_dir, args.profile)
    print("Running north tick timing metrics example...")
    run_example(metrics_path)
    print(f"Wrote {metrics_path}")
    return 0


def cmd_check(args: argparse.Namespace) -> int:
    metrics_path, _, failed_rows_path = paths(args.out_dir, args.profile)
    meta, rows = read_metrics(metrics_path)
    assert_metrics_are_current(metrics_path, meta)
    overrides = {
        "detection_rate": args.override_min_det,
        "false_positive_rate": args.override_max_fp,
        "mean_abs_error_samples": args.override_max_mean,
        "p95_abs_error_samples": args.override_max_p95,
    }
    failures, failed_rows = evaluate_thresholds(rows, args.profile, overrides)
    write_failed_rows(failed_rows, failed_rows_path, rows)
    print(f"Wrote {failed_rows_path}")
    if failures:
        for failure in failures:
            print(failure)
        return 1
    print(f"North tick timing metrics thresholds ({args.profile}): PASS")
    return 0


def cmd_summary(args: argparse.Namespace) -> int:
    metrics_path, summary_path, failed_rows_path = paths(args.out_dir, args.profile)
    include_failed = failed_rows_path if args.include_failed_rows else None
    write_summary(metrics_path, summary_path, args.profile, include_failed, args.max_rows)
    print(f"Wrote {summary_path}")
    return 0


def cmd_failed_rows(args: argparse.Namespace) -> int:
    print_failed_rows_md(args.failed_rows_path_arg, args.title, args.max_rows)
    return 0


def cmd_ci(args: argparse.Namespace) -> int:
    metrics_path, summary_path, failed_rows_path = paths(args.out_dir, args.profile)
    print("Running north tick timing metrics example...")
    run_example(metrics_path)
    print(f"Wrote {metrics_path}")

    meta, rows = read_metrics(metrics_path)
    assert_metrics_are_current(metrics_path, meta)
    overrides = {
        "detection_rate": args.override_min_det,
        "false_positive_rate": args.override_max_fp,
        "mean_abs_error_samples": args.override_max_mean,
        "p95_abs_error_samples": args.override_max_p95,
    }
    failures, failed_rows = evaluate_thresholds(rows, args.profile, overrides)
    write_failed_rows(failed_rows, failed_rows_path, rows)
    print(f"Wrote {failed_rows_path}")

    write_summary(metrics_path, summary_path, args.profile, failed_rows_path, args.max_rows)
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
    pf.add_argument("failed_rows_path_arg", type=Path)
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

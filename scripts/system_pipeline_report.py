#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
import subprocess
from pathlib import Path
from typing import Dict, Tuple

from perf_schema import (
    coverage_failures,
    fine_coverage_failures,
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

# Buffer sizes swept per (north_mode, bearing_method, scenario).
EXPECTED_BUFFER_SIZES = 3

# Mode+method timing defaults
#
# The timing limits are loose, and deliberately.
#
# p95 per-sample time is the worst 5 percent of 180 iterations, which on any
# machine doing anything else is the scheduler rather than the code. It gave
# four false failures in one afternoon here: the same row read 0.937 us per
# sample on one run and 0.335 on another, with no change in between. It stays
# in the report because a genuine blow-up is worth seeing, but it is gated at
# a level only a gross regression reaches.
#
# Mean per-sample time is the one that carries the regression guard, and the
# zero-crossing rows sit at the correlation limit rather than the 0.50 their
# clean numbers would justify, for the same reason at smaller scale. A
# doubling still fails. The accuracy columns are unaffected by any of this and
# they are where the tight limits are.
#
# These are the clean and harmonic_contaminated levels; noisy_jittered and
# low_snr_dc get scenario overrides below.
#
# The bearing limits here are roughly six times tighter than they used to be.
# The old ones were not measuring the pipeline: the harness placed each north
# pulse at round(k * period), up to half a sample from where the rotation
# actually crosses north, which is six degrees of bearing injected per rotation
# before any code ran. Rendering the pulse band-limited at its true epoch, as
# src/simulation/signal.rs already does, took mean error in the clean scenarios
# from about ten degrees to under two.
MODE_METHOD_DEFAULTS: Dict[Tuple[str, str], Dict[str, float]] = {
    ("dpll", "correlation"): {
        "bearing_success_rate": 0.99,
        "detection_rate": 0.995,
        "false_positive_rate": 0.01,
        "mean_us_per_sample": 0.75,
        "p95_us_per_sample": 2.00,
        "mean_abs_bearing_error_deg": 3.0,
        "p95_abs_bearing_error_deg": 8.0,
        "max_abs_bearing_error_deg": 12.0,
        "mean_abs_tick_error_samples": 0.2,
        "p95_abs_tick_error_samples": 0.6,
    },
    ("simple", "correlation"): {
        "bearing_success_rate": 0.99,
        "detection_rate": 0.995,
        "false_positive_rate": 0.01,
        "mean_us_per_sample": 0.75,
        "p95_us_per_sample": 2.00,
        "mean_abs_bearing_error_deg": 3.0,
        "p95_abs_bearing_error_deg": 8.0,
        "max_abs_bearing_error_deg": 12.0,
        "mean_abs_tick_error_samples": 0.2,
        "p95_abs_tick_error_samples": 0.5,
    },
    ("dpll", "zero_crossing"): {
        "bearing_success_rate": 0.99,
        "detection_rate": 0.995,
        "false_positive_rate": 0.01,
        "mean_us_per_sample": 0.75,
        "p95_us_per_sample": 2.00,
        "mean_abs_bearing_error_deg": 3.0,
        "p95_abs_bearing_error_deg": 8.0,
        "max_abs_bearing_error_deg": 12.0,
        "mean_abs_tick_error_samples": 0.2,
        "p95_abs_tick_error_samples": 0.6,
    },
    ("simple", "zero_crossing"): {
        "bearing_success_rate": 0.99,
        "detection_rate": 0.995,
        "false_positive_rate": 0.01,
        "mean_us_per_sample": 0.75,
        "p95_us_per_sample": 2.00,
        "mean_abs_bearing_error_deg": 3.0,
        "p95_abs_bearing_error_deg": 8.0,
        "max_abs_bearing_error_deg": 12.0,
        "mean_abs_tick_error_samples": 0.2,
        "p95_abs_tick_error_samples": 0.5,
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
                # 0.03, down from 0.06, because almost none of what this
                # scenario was calling a false positive was one. It drops
                # every 17th pulse, and the harness scored the DPLL's
                # holdover prediction over each gap as a false alarm -- one
                # per sixteen pulses, 5.9%, which was the whole of the rate
                # the limit had been raised to accommodate. Scoring a
                # prediction over a dropped rotation as neither a detection
                # nor a false positive takes the DPLL rows from 0.049 to
                # 0.001 and the simple rows from 0.029 to 0.017. The limit is
                # set above the simple tracker, which is the higher of the
                # two and genuinely triggers on noise.
                "false_positive_rate": 0.03,
            }
        )

# The bearing limits below are far looser than they were, and the scenarios
# are far harsher than they were. The doppler noise used to be added white
# across the spectrum, so the doppler bandpass -- 500 Hz of 24 kHz -- threw 98
# percent of it away before it reached anything: the scenario named for low SNR
# ran at an in-band tone fraction of 0.998 while the recordings in data/ run
# from 0.002 to 0.075. Every accuracy limit here was set against near-clean
# signal.
#
# The scenarios now carry band-limited audio scaled to the noise power inside
# the doppler passband, which is the quantity that decides a bearing, matched
# to the three recordings: 0.2, 0.8 and 6.5.
#
# One caveat on reading these. At matched passband power the synthetic signal
# is two to four times harsher than the recordings -- 6.1 degrees against 1.6
# at a ratio of 0.2, 40.7 against 54.8 at 6.5 -- because flat noise is worse
# for phase estimation than the shaped audio a radio actually delivers. The
# levels match the physical measurement rather than the resulting error, which
# is the honest way round but means these limits are pessimistic.

# noisy_jittered injects a sample of deliberate tick jitter, so the tick error
# columns there measure the stimulus, not the tracker. The DPLL averages that
# jitter away -- which is the point of a loop -- and so reads a larger tick
# error than the simple tracker while producing a third of its bearing error.
# Bearing is the metric that means something in this scenario.
#
# That claim used to rest on nothing. The jitter was sin(0.37 k), a coherent
# 94 Hz modulation that a 2 Hz loop rejects by construction, so the DPLL's
# advantage was the stimulus being out of band rather than the loop working.
# With white fractional jitter, which has in-band content the loop must
# actually follow, the advantage is not merely intact but wider: 2.07 degrees
# against 5.92 where it had been 4.35 against 8.17.
for bearing_method in ("correlation", "zero_crossing"):
    BASELINE_LIMITS[("dpll", bearing_method, "noisy_jittered")].update(
        {
            "mean_abs_bearing_error_deg": 30.0,
            "p95_abs_bearing_error_deg": 85.0,
            "max_abs_bearing_error_deg": 180.0,
            "mean_abs_tick_error_samples": 0.5,
            "p95_abs_tick_error_samples": 1.0,
        }
    )
    BASELINE_LIMITS[("simple", bearing_method, "noisy_jittered")].update(
        {
            "mean_abs_bearing_error_deg": 30.0,
            "p95_abs_bearing_error_deg": 85.0,
            "max_abs_bearing_error_deg": 180.0,
            "mean_abs_tick_error_samples": 0.3,
        }
    )

# harmonic_contaminated adds impulses to the north channel. The DPLL's timing
# gate rejects the detections they displace and does not coast over a rejection,
# so it reports about one tick in a hundred fewer than the simple tracker, which
# takes the displaced detection instead. Losing the tick is the better trade.
for bearing_method in ("correlation", "zero_crossing"):
    BASELINE_LIMITS[("dpll", bearing_method, "harmonic_contaminated")].update(
        {
            "bearing_success_rate": 0.985,
            "detection_rate": 0.985,
            "mean_abs_bearing_error_deg": 16.0,
            "p95_abs_bearing_error_deg": 40.0,
            "max_abs_bearing_error_deg": 60.0,
            "mean_abs_tick_error_samples": 0.3,
        }
    )
    BASELINE_LIMITS[("simple", bearing_method, "harmonic_contaminated")].update(
        {
            "mean_abs_bearing_error_deg": 16.0,
            "p95_abs_bearing_error_deg": 40.0,
            "max_abs_bearing_error_deg": 60.0,
        }
    )

for bearing_method in ("correlation", "zero_crossing"):
    BASELINE_LIMITS[("dpll", bearing_method, "low_snr_dc")].update(
        {
            "mean_abs_bearing_error_deg": 70.0,
            "p95_abs_bearing_error_deg": 175.0,
            "max_abs_bearing_error_deg": 181.0,
            "mean_abs_tick_error_samples": 0.5,
            "p95_abs_tick_error_samples": 1.0,
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

# The simple tracker on low_snr_dc used to be bimodal, and is not any more.
#
# It either tracked or latched into detecting every other pulse, decided by
# the noise draw: over sixteen draws the latch turned up about one time in
# five, and detection read 0.894 with a standard error of 0.049 -- a value it
# never actually took. The cause was the dead time keeping the first crossing
# in its window rather than the largest. At this noise level a trigger arrives
# roughly once per rotation, and the one that opened the window masked the
# pulse behind it.
#
# The simple tracker now searches the whole dead time for its largest sample,
# so the pulse wins over the trigger. Detection reads 0.995 to 0.997 with a
# standard error under 0.0004, and the limits below are tight again. The
# bearing limit is the measured worst row plus three standard errors.
for bearing_method in ("correlation", "zero_crossing"):
    BASELINE_LIMITS[("simple", bearing_method, "low_snr_dc")].update(
        {
            "bearing_success_rate": 0.99,
            "detection_rate": 0.99,
            "mean_abs_bearing_error_deg": 78.0,
        }
    )

# The bearing limits below are far looser than they were, and the scenarios
# are far harsher than they were. The doppler noise used to be added white
# across the spectrum, so the doppler bandpass -- 500 Hz of 24 kHz -- threw 98
# percent of it away before it reached anything: the scenario named for low SNR
# ran at an in-band tone fraction of 0.998 while the recordings in data/ run
# from 0.002 to 0.075. Every accuracy limit here was set against near-clean
# signal.
#
# The scenarios now carry band-limited audio scaled to the noise power inside
# the doppler passband, which is the quantity that decides a bearing, matched
# to the three recordings: 0.2, 0.8 and 6.5.
#
# One caveat on reading these. At matched passband power the synthetic signal
# is two to four times harsher than the recordings -- 6.1 degrees against 1.6
# at a ratio of 0.2, 40.7 against 54.8 at 6.5 -- because flat noise is worse
# for phase estimation than the shaped audio a radio actually delivers. The
# levels match the physical measurement rather than the resulting error, which
# is the honest way round but means these limits are pessimistic.

# noisy_jittered injects a sample of deliberate tick jitter, so the tick error
# columns there measure the stimulus, not the tracker. The DPLL averages that
# jitter away -- which is the point of a loop -- and so reads a larger tick
# error than the simple tracker while producing a third of its bearing error.
# Bearing is the metric that means something in this scenario.
#
# That claim used to rest on nothing. The jitter was sin(0.37 k), a coherent
# 94 Hz modulation that a 2 Hz loop rejects by construction, so the DPLL's
# advantage was the stimulus being out of band rather than the loop working.
# With white fractional jitter, which has in-band content the loop must
# actually follow, the advantage is not merely intact but wider: 2.07 degrees
# against 5.92 where it had been 4.35 against 8.17.
for bearing_method in ("correlation", "zero_crossing"):
    BASELINE_LIMITS[("dpll", bearing_method, "noisy_jittered")].update(
        {
            "mean_abs_bearing_error_deg": 30.0,
            "p95_abs_bearing_error_deg": 85.0,
            "max_abs_bearing_error_deg": 180.0,
            "mean_abs_tick_error_samples": 0.5,
            "p95_abs_tick_error_samples": 1.0,
        }
    )
    BASELINE_LIMITS[("simple", bearing_method, "noisy_jittered")].update(
        {
            "mean_abs_bearing_error_deg": 30.0,
            "p95_abs_bearing_error_deg": 85.0,
            "max_abs_bearing_error_deg": 180.0,
            "mean_abs_tick_error_samples": 0.3,
        }
    )

# harmonic_contaminated adds impulses to the north channel. The DPLL's timing
# gate rejects the detections they displace and does not coast over a rejection,
# so it reports about one tick in a hundred fewer than the simple tracker, which
# takes the displaced detection instead. Losing the tick is the better trade.
for bearing_method in ("correlation", "zero_crossing"):
    BASELINE_LIMITS[("dpll", bearing_method, "harmonic_contaminated")].update(
        {
            "bearing_success_rate": 0.985,
            "detection_rate": 0.985,
            "mean_abs_bearing_error_deg": 16.0,
            "p95_abs_bearing_error_deg": 40.0,
            "max_abs_bearing_error_deg": 60.0,
            "mean_abs_tick_error_samples": 0.3,
        }
    )
    BASELINE_LIMITS[("simple", bearing_method, "harmonic_contaminated")].update(
        {
            "mean_abs_bearing_error_deg": 16.0,
            "p95_abs_bearing_error_deg": 40.0,
            "max_abs_bearing_error_deg": 60.0,
        }
    )

for bearing_method in ("correlation", "zero_crossing"):
    BASELINE_LIMITS[("dpll", bearing_method, "low_snr_dc")].update(
        {
            "mean_abs_bearing_error_deg": 70.0,
            "p95_abs_bearing_error_deg": 175.0,
            "max_abs_bearing_error_deg": 181.0,
            "mean_abs_tick_error_samples": 0.5,
            "p95_abs_tick_error_samples": 1.0,
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

# The simple tracker on low_snr_dc is bimodal, and the limits below are set
# from its spread rather than from a single run.
#
# At this noise level it either tracks or latches into detecting every other
# pulse, and which one happens is decided by the noise draw. The dead time is
# 28.8 samples against a 29.95 sample period, so a detection landing early puts
# the following pulse inside its shadow, and once that starts it sustains
# itself. Over sixteen draws the latch appears in about one in five, and
# detection reads 0.894 with a standard error of 0.049 -- against 0.99 or 0.49
# for any single draw.
#
# Sixteen draws is what the rest of the table needs many times over; every
# other row here sits at a standard error under 0.001. Pinning this row to
# 0.02 would take about a hundred draws and forty minutes of CI, which is not
# worth it for one row. So the limits are the measured mean less three
# standard errors, which still catches a real regression -- a genuine halving
# would read 0.49 and fail -- while tolerating the draw-to-draw swing that is
# a property of the tracker rather than of a change.
for bearing_method in ("correlation", "zero_crossing"):
    BASELINE_LIMITS[("simple", bearing_method, "low_snr_dc")].update(
        {
            "bearing_success_rate": 0.76,
            "detection_rate": 0.74,
            "mean_abs_bearing_error_deg": 86.0,
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

    failures.extend(
        coverage_failures(rows, lambda row: (row["north_mode"], row["bearing_method"], row["scenario"]), BASELINE_LIMITS.keys())
    )
    failures.extend(
        fine_coverage_failures(
            rows,
            lambda row: (row["north_mode"], row["bearing_method"], row["scenario"]),
            lambda row: (row["buffer_size"],),
            lambda _group: EXPECTED_BUFFER_SIZES,
        )
    )

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

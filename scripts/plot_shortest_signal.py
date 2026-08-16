#!/usr/bin/env python3
"""Plot the shortest detectable signal from `metric_shortest_signal --jsonl`.

    cargo run --release -p rotaryclub-metrics --bin metric_shortest_signal -- --jsonl > s.jsonl
    scripts/plot_shortest_signal.py s.jsonl -o shortest_signal.png

The answer is T90: the shortest burst yielding a usable bearing in 90 percent
of draws. It is marked on each curve rather than left to be traced, since a
reader who has to find the crossing has been handed the evidence and not the
answer.

Faceted by channel condition rather than by buffer size. In-band SNR moves the
answer by more than an order of magnitude and buffer size barely moves it, so
buffer is the colour within a panel and not the structure of the figure.

Nothing is checked in -- regenerate when the numbers change, so a stale image
cannot outlive the measurement it came from.
"""

from __future__ import annotations

import argparse
import json
from collections import defaultdict
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.patheffects as path_effects  # noqa: E402
import matplotlib.pyplot as plt  # noqa: E402

# Fixed order, never cycled. Validated for colour-vision deficiency against the
# chart surface; series are direct-labelled as well, which the low-contrast
# slot requires.
SERIES_COLOURS = ["#2a78d6", "#eb6834", "#1baf7a"]
SURFACE = "#fcfcfb"
INK = "#1a1a19"
MUTED = "#6b6b68"
GRID = "#e2e2df"
FLOOR_FILL = "#eceae6"
REQUIRED_RATE = 0.90
# Bandpass settling plus one filled work buffer. Nothing is detectable below
# this however good the channel, so it is shaded rather than left as empty
# axis a reader could mistake for headroom.
FLOOR_MS = 25.0


def load(path: Path):
    curves = defaultdict(list)
    controls = {}
    meta = {}
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line:
            continue
        row = json.loads(line)
        key = (row["snr_db"], row["buffer_size"])
        if row.get("control"):
            controls[key] = row["rate"]
            continue
        curves[key].append((row["duration_ms"], row["rate"], row["rate_se"]))
        meta.setdefault("error_limit_deg", row.get("error_limit_deg"))
        meta.setdefault("stated_limit_deg", row.get("stated_limit_deg"))
        meta.setdefault("draws", row.get("draws"))
    for key in curves:
        curves[key].sort()
    return curves, controls, meta


def t90(points):
    """Shortest measured duration reaching the required rate."""
    for duration, rate, _ in points:
        if rate >= REQUIRED_RATE:
            return duration
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("jsonl", type=Path)
    parser.add_argument("-o", "--out", type=Path, default=Path("shortest_signal.png"))
    args = parser.parse_args()

    curves, controls, meta = load(args.jsonl)
    snrs = sorted({k[0] for k in curves}, reverse=True)
    buffers = sorted({k[1] for k in curves})

    fig, axes = plt.subplots(1, len(snrs), figsize=(4.6 * len(snrs), 4.8), sharey=True)
    if len(snrs) == 1:
        axes = [axes]
    fig.patch.set_facecolor(SURFACE)
    worst_control = max(controls.values()) if controls else 0.0

    for ax, snr in zip(axes, snrs):
        ax.set_facecolor(SURFACE)
        ax.axvspan(0, FLOOR_MS, color=FLOOR_FILL, zorder=0)
        ax.axhline(REQUIRED_RATE, color=MUTED, lw=1, ls=(0, (4, 3)), zorder=1)

        placed: list[float] = []
        for colour, buffer_size in zip(SERIES_COLOURS, buffers):
            points = curves.get((snr, buffer_size))
            if not points:
                continue
            xs = [p[0] for p in points]
            ys = [p[1] for p in points]
            es = [p[2] for p in points]
            ax.errorbar(
                xs,
                ys,
                yerr=es,
                color=colour,
                lw=2,
                marker="o",
                ms=5,
                elinewidth=1,
                capsize=2,
                label=f"{buffer_size}",
                zorder=3,
            )

            # The answer, marked: a drop line from the crossing to the axis
            # with the duration on it.
            crossing = t90(points)
            if crossing is None:
                continue
            ax.plot(
                [crossing, crossing],
                [0, REQUIRED_RATE],
                color=colour,
                lw=1,
                ls=(0, (2, 2)),
                alpha=0.8,
                zorder=2,
            )
            ax.plot(
                [crossing],
                [REQUIRED_RATE],
                marker="o",
                ms=9,
                color=colour,
                mec=SURFACE,
                mew=1.6,
                zorder=5,
            )
            # Two crossings close together would print on top of each other,
            # so a label lifts clear of any already placed nearby. Where they
            # coincide exactly the second is dropped instead of stacked: two
            # identical numbers one above the other read as two answers.
            if any(abs(crossing - x) < 1e-6 for x in placed):
                continue
            row = sum(1 for x in placed if 0.6 < crossing / x < 1.7)
            placed.append(crossing)
            # Not colour-coded, and it does not need to be: each label sits
            # directly under its own drop line, which is in the series colour.
            ax.annotate(
                f"{crossing:.0f} ms",
                (crossing, 0.0),
                textcoords="offset points",
                xytext=(0, 6 + 15 * row),
                ha="center",
                fontsize=10,
                color=INK,
                zorder=6,
            ).set_path_effects(
                [path_effects.withStroke(linewidth=3, foreground=SURFACE)]
            )

        ax.set_xscale("log")
        ax.set_xlim(15, 3400)
        ax.set_ylim(-0.06, 1.06)
        ticks = [20, 50, 100, 200, 500, 1000, 2000]
        ax.set_xticks(ticks)
        ax.set_xticklabels([str(t) for t in ticks])
        ax.minorticks_off()
        ax.set_xlabel("burst duration (ms)", fontsize=9, color=MUTED)
        ax.set_title(f"{snr:+.0f} dB in-band SNR", fontsize=11, color=INK, pad=8)
        ax.grid(True, axis="y", color=GRID, lw=0.8, zorder=0)
        ax.set_axisbelow(True)
        for side in ("top", "right"):
            ax.spines[side].set_visible(False)
        for side in ("left", "bottom"):
            ax.spines[side].set_color(GRID)
        ax.tick_params(colors=MUTED, labelsize=8)

    axes[0].set_ylabel(
        "fraction of bursts yielding a usable bearing", fontsize=9, color=MUTED
    )
    # High in the panel, where no curve reaches this far left.
    axes[0].text(
        FLOOR_MS * 0.78,
        0.86,
        "detector floor",
        ha="center",
        va="center",
        rotation=90,
        fontsize=7.5,
        color=MUTED,
    ).set_path_effects([path_effects.withStroke(linewidth=3, foreground=FLOOR_FILL)])
    axes[0].annotate(
        "90%",
        (0.99, REQUIRED_RATE),
        xycoords=("axes fraction", "data"),
        ha="right",
        va="bottom",
        fontsize=8,
        color=MUTED,
    )
    handles, labels = axes[0].get_legend_handles_labels()
    fig.legend(
        handles,
        labels,
        title="buffer (samples)",
        frameon=False,
        fontsize=8,
        title_fontsize=8,
        loc="upper right",
        bbox_to_anchor=(0.995, 1.005),
        ncol=len(labels) or 1,
    )

    fig.suptitle(
        "Shortest transmission yielding a usable bearing",
        fontsize=13,
        color=INK,
        x=0.012,
        ha="left",
        y=0.985,
    )
    fig.text(
        0.012,
        0.93,
        f"bearing within {meta.get('error_limit_deg', 10):.0f}° of truth and stating at most "
        f"{meta.get('stated_limit_deg', 10):.0f}°, over {meta.get('draws', '?')} noise draws",
        fontsize=9,
        color=MUTED,
        ha="left",
    )
    fig.text(
        0.012,
        0.895,
        f"The same criterion over a 2000 ms window with no transmission present is met "
        f"{worst_control:.2f} of the time, so these rates are not counting bearings taken on noise",
        fontsize=9,
        color=INK,
        ha="left",
    )
    fig.tight_layout(rect=(0, 0, 1, 0.875))
    fig.savefig(args.out, dpi=160, facecolor=SURFACE)
    print(f"wrote {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

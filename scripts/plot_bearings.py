#!/usr/bin/env python3
"""Plot bearings over time from CSV data, comparing correlation and zero-crossing methods."""

import argparse
import sys
from pathlib import Path

import numpy as np
import pandas as pd
import matplotlib.pyplot as plt
from matplotlib.gridspec import GridSpec


def load_and_prepare(source, min_confidence, min_coherence):
    """Load CSV and prepare dataframe with time column."""
    if isinstance(source, str) or isinstance(source, Path):
        df = pd.read_csv(source)
    else:
        df = pd.read_csv(source)

    if len(df) == 0:
        df['time_s'] = pd.Series(dtype=float)
        return df, df

    if 'ts' in df.columns:
        df['ts'] = pd.to_datetime(df['ts'])
        df['time_s'] = (df['ts'] - df['ts'].iloc[0]).dt.total_seconds()

    mask = (df['confidence'] >= min_confidence) & (df['coherence'] >= min_coherence)
    return df, df[mask]


def circular_diff(a, b):
    """Compute circular difference between bearings, result in [-180, 180]."""
    diff = a - b
    return ((diff + 180) % 360) - 180


def main():
    parser = argparse.ArgumentParser(
        description='Plot bearings over time, comparing correlation and zero-crossing methods.')
    parser.add_argument('csv_file', nargs='?',
                        help='Path to single CSV file (legacy mode)')
    parser.add_argument('--correlation', '-c', type=str,
                        help='Path to correlation method CSV')
    parser.add_argument('--zero-crossing', '-z', type=str,
                        help='Path to zero-crossing method CSV')
    parser.add_argument('--output', '-o', type=str,
                        help='Output file path')
    parser.add_argument('--min-confidence', type=float, default=0.5,
                        help='Minimum confidence threshold (0.0-1.0, default: 0.5)')
    parser.add_argument('--min-coherence', type=float, default=0.5,
                        help='Minimum coherence threshold (0.0-1.0, default: 0.5)')
    parser.add_argument('--no-show', action='store_true',
                        help='Do not display the plot (just save)')
    args = parser.parse_args()

    compare_mode = args.correlation or args.zero_crossing

    if compare_mode:
        plot_comparison(args)
    else:
        plot_single(args)


def plot_single(args):
    """Original single-file plotting mode."""
    if args.csv_file:
        df, df_filtered = load_and_prepare(
            args.csv_file, args.min_confidence, args.min_coherence)
    else:
        df, df_filtered = load_and_prepare(
            sys.stdin, args.min_confidence, args.min_coherence)

    print(f"Plotting {len(df_filtered)}/{len(df)} points "
          f"(confidence >= {args.min_confidence}, coherence >= {args.min_coherence})")

    fig, axes = plt.subplots(2, 1, figsize=(12, 8), sharex=True)

    ax1 = axes[0]
    ax1.scatter(df_filtered['time_s'], df_filtered['bearing'], s=1, alpha=0.5, label='Smoothed')
    ax1.scatter(df_filtered['time_s'], df_filtered['raw'], s=1, alpha=0.3, label='Raw')
    ax1.set_ylabel('Bearing (degrees)')
    ax1.set_ylim(0, 360)
    ax1.set_yticks([0, 90, 180, 270, 360])
    ax1.legend(loc='upper right')
    ax1.grid(True, alpha=0.3)
    ax1.set_title('Bearing Over Time')

    ax2 = axes[1]
    ax2.scatter(df['time_s'], df['confidence'], s=1, alpha=0.5, label='Confidence')
    ax2.scatter(df['time_s'], df['coherence'], s=1, alpha=0.5, label='Coherence')
    ax2.set_xlabel('Time (seconds)')
    ax2.set_ylabel('Quality Metric')
    ax2.set_ylim(0, 1)
    ax2.legend(loc='upper right')
    ax2.grid(True, alpha=0.3)

    plt.tight_layout()
    if args.output:
        output_path = Path(args.output)
    elif args.csv_file:
        input_path = Path(args.csv_file)
        output_path = input_path.parent / f"{input_path.stem}_plot.png"
    else:
        output_path = Path("/tmp/bearings_plot.png")
    plt.savefig(output_path, dpi=150)
    print(f"Saved plot to {output_path}")
    if not args.no_show:
        plt.show()


def plot_comparison(args):
    """Plot correlation and zero-crossing methods with hexbin density and difference."""

    df_corr = df_corr_f = None
    df_zc = df_zc_f = None

    if args.correlation:
        df_corr, df_corr_f = load_and_prepare(
            args.correlation, args.min_confidence, args.min_coherence)
        if len(df_corr) == 0:
            print("Correlation: no data")
            df_corr = df_corr_f = None
        else:
            print(f"Correlation: {len(df_corr_f)}/{len(df_corr)} points after filtering")

    if args.zero_crossing:
        df_zc, df_zc_f = load_and_prepare(
            args.zero_crossing, args.min_confidence, args.min_coherence)
        if len(df_zc) == 0:
            print("Zero-crossing: no data")
            df_zc = df_zc_f = None
        else:
            print(f"Zero-crossing: {len(df_zc_f)}/{len(df_zc)} points after filtering")

    # Create figure with custom grid layout
    fig = plt.figure(figsize=(14, 12))
    gs = GridSpec(3, 3, figure=fig, width_ratios=[4, 4, 1], height_ratios=[1, 1, 1],
                  hspace=0.3, wspace=0.05)

    # Determine common time range
    time_min, time_max = 0, 1
    if df_corr_f is not None and len(df_corr_f) > 0:
        time_min = min(time_min, df_corr_f['time_s'].min())
        time_max = max(time_max, df_corr_f['time_s'].max())
    if df_zc_f is not None and len(df_zc_f) > 0:
        time_min = min(time_min, df_zc_f['time_s'].min())
        time_max = max(time_max, df_zc_f['time_s'].max())

    # Row 1: Correlation hexbin + histogram
    ax_corr = fig.add_subplot(gs[0, 0:2])
    ax_corr_hist = fig.add_subplot(gs[0, 2], sharey=ax_corr)

    if df_corr_f is not None and len(df_corr_f) > 0:
        hb = ax_corr.hexbin(df_corr_f['time_s'], df_corr_f['bearing'],
                           gridsize=(100, 36), cmap='Blues', mincnt=1)
        ax_corr_hist.hist(df_corr_f['bearing'], bins=72, range=(0, 360),
                         orientation='horizontal', color='tab:blue', alpha=0.7)
    ax_corr.set_ylabel('Bearing (degrees)')
    ax_corr.set_ylim(0, 360)
    ax_corr.set_yticks([0, 90, 180, 270, 360])
    ax_corr.set_xlim(time_min, time_max)
    ax_corr.grid(True, alpha=0.3)
    ax_corr.set_title('Correlation Method')
    ax_corr_hist.set_xticks([])
    plt.setp(ax_corr_hist.get_yticklabels(), visible=False)

    # Row 2: Zero-crossing hexbin + histogram
    ax_zc = fig.add_subplot(gs[1, 0:2], sharex=ax_corr)
    ax_zc_hist = fig.add_subplot(gs[1, 2], sharey=ax_zc)

    if df_zc_f is not None and len(df_zc_f) > 0:
        hb = ax_zc.hexbin(df_zc_f['time_s'], df_zc_f['bearing'],
                         gridsize=(100, 36), cmap='Oranges', mincnt=1)
        ax_zc_hist.hist(df_zc_f['bearing'], bins=72, range=(0, 360),
                       orientation='horizontal', color='tab:orange', alpha=0.7)
    ax_zc.set_ylabel('Bearing (degrees)')
    ax_zc.set_ylim(0, 360)
    ax_zc.set_yticks([0, 90, 180, 270, 360])
    ax_zc.grid(True, alpha=0.3)
    ax_zc.set_title('Zero-Crossing Method')
    ax_zc_hist.set_xticks([])
    plt.setp(ax_zc_hist.get_yticklabels(), visible=False)

    # Row 3: Method difference plot
    ax_diff = fig.add_subplot(gs[2, 0:2], sharex=ax_corr)
    ax_diff_hist = fig.add_subplot(gs[2, 2])

    if df_corr_f is not None and df_zc_f is not None and len(df_corr_f) > 0 and len(df_zc_f) > 0:
        # Merge on nearest timestamp
        df_corr_f = df_corr_f.copy()
        df_zc_f = df_zc_f.copy()
        df_corr_f['time_bin'] = (df_corr_f['time_s'] * 100).round() / 100
        df_zc_f['time_bin'] = (df_zc_f['time_s'] * 100).round() / 100

        merged = pd.merge(df_corr_f[['time_bin', 'bearing']],
                         df_zc_f[['time_bin', 'bearing']],
                         on='time_bin', suffixes=('_corr', '_zc'))

        if len(merged) > 0:
            merged['diff'] = circular_diff(merged['bearing_corr'], merged['bearing_zc'])

            ax_diff.hexbin(merged['time_bin'], merged['diff'],
                          gridsize=(100, 36), cmap='RdBu_r', mincnt=1,
                          extent=[time_min, time_max, -180, 180])
            ax_diff.axhline(y=0, color='black', linestyle='-', linewidth=0.5)

            ax_diff_hist.hist(merged['diff'], bins=72, range=(-180, 180),
                             orientation='horizontal', color='gray', alpha=0.7)

            # Print summary stats
            mean_diff = merged['diff'].mean()
            std_diff = merged['diff'].std()
            within_5 = (merged['diff'].abs() <= 5).mean() * 100
            within_10 = (merged['diff'].abs() <= 10).mean() * 100
            print(f"Method difference: mean={mean_diff:.1f}°, std={std_diff:.1f}°, "
                  f"within ±5°: {within_5:.0f}%, within ±10°: {within_10:.0f}%")

    ax_diff.set_xlabel('Time (seconds)')
    ax_diff.set_ylabel('Difference (degrees)')
    ax_diff.set_ylim(-180, 180)
    ax_diff.set_yticks([-180, -90, 0, 90, 180])
    ax_diff.grid(True, alpha=0.3)
    ax_diff.set_title('Correlation − Zero-Crossing')
    ax_diff_hist.set_ylim(-180, 180)
    ax_diff_hist.set_xticks([])
    ax_diff_hist.set_yticks([])

    if args.output:
        output_path = Path(args.output)
    else:
        output_path = Path("/tmp/bearings_comparison.png")

    plt.savefig(output_path, dpi=150, bbox_inches='tight')
    print(f"Saved plot to {output_path}")
    if not args.no_show:
        plt.show()


if __name__ == '__main__':
    main()

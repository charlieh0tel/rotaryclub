#!/usr/bin/env python3
"""Plot bearings over time from CSV data, comparing correlation and zero-crossing methods."""

import argparse
import sys
from pathlib import Path

import pandas as pd
import matplotlib.pyplot as plt


def load_and_prepare(source, min_confidence, min_coherence):
    """Load CSV and prepare dataframe with time column."""
    if isinstance(source, str) or isinstance(source, Path):
        df = pd.read_csv(source)
    else:
        df = pd.read_csv(source)

    if 'ts' in df.columns:
        df['ts'] = pd.to_datetime(df['ts'])
        df['time_s'] = (df['ts'] - df['ts'].iloc[0]).dt.total_seconds()

    mask = (df['confidence'] >= min_confidence) & (df['coherence'] >= min_coherence)
    return df, df[mask]


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
    plt.show()


def plot_comparison(args):
    """Plot correlation and zero-crossing methods side by side."""
    fig, axes = plt.subplots(2, 2, figsize=(14, 10))

    datasets = []

    if args.correlation:
        df_corr, df_corr_f = load_and_prepare(
            args.correlation, args.min_confidence, args.min_coherence)
        datasets.append(('Correlation', df_corr, df_corr_f, 'tab:blue'))
        print(f"Correlation: {len(df_corr_f)}/{len(df_corr)} points after filtering")

    if args.zero_crossing:
        df_zc, df_zc_f = load_and_prepare(
            args.zero_crossing, args.min_confidence, args.min_coherence)
        datasets.append(('Zero-Crossing', df_zc, df_zc_f, 'tab:orange'))
        print(f"Zero-crossing: {len(df_zc_f)}/{len(df_zc)} points after filtering")

    # Top left: Correlation bearing
    ax_corr = axes[0, 0]
    if args.correlation:
        ax_corr.scatter(df_corr_f['time_s'], df_corr_f['bearing'], s=1, alpha=0.5, c='tab:blue')
        ax_corr.scatter(df_corr_f['time_s'], df_corr_f['raw'], s=1, alpha=0.2, c='tab:cyan')
    ax_corr.set_ylabel('Bearing (degrees)')
    ax_corr.set_ylim(0, 360)
    ax_corr.set_yticks([0, 90, 180, 270, 360])
    ax_corr.grid(True, alpha=0.3)
    ax_corr.set_title('Correlation Method')

    # Top right: Zero-crossing bearing
    ax_zc = axes[0, 1]
    if args.zero_crossing:
        ax_zc.scatter(df_zc_f['time_s'], df_zc_f['bearing'], s=1, alpha=0.5, c='tab:orange')
        ax_zc.scatter(df_zc_f['time_s'], df_zc_f['raw'], s=1, alpha=0.2, c='tab:red')
    ax_zc.set_ylim(0, 360)
    ax_zc.set_yticks([0, 90, 180, 270, 360])
    ax_zc.grid(True, alpha=0.3)
    ax_zc.set_title('Zero-Crossing Method')

    # Bottom left: Overlay comparison
    ax_overlay = axes[1, 0]
    if args.correlation:
        ax_overlay.scatter(df_corr_f['time_s'], df_corr_f['bearing'], s=1, alpha=0.5,
                          c='tab:blue', label='Correlation')
    if args.zero_crossing:
        ax_overlay.scatter(df_zc_f['time_s'], df_zc_f['bearing'], s=1, alpha=0.5,
                          c='tab:orange', label='Zero-Crossing')
    ax_overlay.set_xlabel('Time (seconds)')
    ax_overlay.set_ylabel('Bearing (degrees)')
    ax_overlay.set_ylim(0, 360)
    ax_overlay.set_yticks([0, 90, 180, 270, 360])
    ax_overlay.legend(loc='upper right')
    ax_overlay.grid(True, alpha=0.3)
    ax_overlay.set_title('Overlay Comparison')

    # Bottom right: Quality metrics
    ax_qual = axes[1, 1]
    if args.correlation:
        ax_qual.scatter(df_corr['time_s'], df_corr['confidence'], s=1, alpha=0.4,
                       c='tab:blue', label='Corr confidence')
        ax_qual.scatter(df_corr['time_s'], df_corr['coherence'], s=1, alpha=0.4,
                       c='tab:cyan', label='Corr coherence')
    if args.zero_crossing:
        ax_qual.scatter(df_zc['time_s'], df_zc['confidence'], s=1, alpha=0.4,
                       c='tab:orange', label='ZC confidence')
        ax_qual.scatter(df_zc['time_s'], df_zc['coherence'], s=1, alpha=0.4,
                       c='tab:red', label='ZC coherence')
    ax_qual.set_xlabel('Time (seconds)')
    ax_qual.set_ylabel('Quality Metric')
    ax_qual.set_ylim(0, 1)
    ax_qual.legend(loc='upper right', fontsize='small')
    ax_qual.grid(True, alpha=0.3)
    ax_qual.set_title('Quality Metrics')

    plt.tight_layout()

    if args.output:
        output_path = Path(args.output)
    else:
        output_path = Path("/tmp/bearings_comparison.png")

    plt.savefig(output_path, dpi=150)
    print(f"Saved plot to {output_path}")
    plt.show()


if __name__ == '__main__':
    main()

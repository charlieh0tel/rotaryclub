#!/usr/bin/env python3
"""Plot bearings over time from CSV data."""

import argparse
from pathlib import Path

import pandas as pd
import matplotlib.pyplot as plt


def main():
    parser = argparse.ArgumentParser(description='Plot bearings over time from CSV data.')
    parser.add_argument('csv_file', help='Path to the CSV file')
    parser.add_argument('--min-confidence', type=float, default=0.5,
                        help='Minimum confidence threshold (0.0-1.0, default: 0.5)')
    parser.add_argument('--min-coherence', type=float, default=0.5,
                        help='Minimum coherence threshold (0.0-1.0, default: 0.5)')
    args = parser.parse_args()

    df = pd.read_csv(args.csv_file)

    # Handle both old (time_s) and new (ts) column formats
    if 'ts' in df.columns:
        df['ts'] = pd.to_datetime(df['ts'])
        df['time_s'] = (df['ts'] - df['ts'].iloc[0]).dt.total_seconds()

    # Filter by confidence and coherence thresholds
    mask = (df['confidence'] >= args.min_confidence) & (df['coherence'] >= args.min_coherence)
    df_filtered = df[mask]

    print(f"Plotting {len(df_filtered)}/{len(df)} points "
          f"(confidence >= {args.min_confidence}, coherence >= {args.min_coherence})")

    fig, axes = plt.subplots(2, 1, figsize=(12, 8), sharex=True)

    # Top plot: bearing over time (filtered)
    ax1 = axes[0]
    ax1.scatter(df_filtered['time_s'], df_filtered['bearing'], s=1, alpha=0.5, label='Smoothed')
    ax1.scatter(df_filtered['time_s'], df_filtered['raw'], s=1, alpha=0.3, label='Raw')
    ax1.set_ylabel('Bearing (degrees)')
    ax1.set_ylim(0, 360)
    ax1.set_yticks([0, 90, 180, 270, 360])
    ax1.legend(loc='upper right')
    ax1.grid(True, alpha=0.3)
    ax1.set_title('Bearing Over Time')

    # Bottom plot: confidence/coherence
    ax2 = axes[1]
    ax2.scatter(df['time_s'], df['confidence'], s=1, alpha=0.5, label='Confidence')
    ax2.scatter(df['time_s'], df['coherence'], s=1, alpha=0.5, label='Coherence')
    ax2.set_xlabel('Time (seconds)')
    ax2.set_ylabel('Quality Metric')
    ax2.set_ylim(0, 1)
    ax2.legend(loc='upper right')
    ax2.grid(True, alpha=0.3)

    plt.tight_layout()
    input_path = Path(args.csv_file)
    output_path = input_path.parent / f"{input_path.stem}_plot.png"
    plt.savefig(output_path, dpi=150)
    print(f"Saved plot to {output_path}")
    plt.show()


if __name__ == '__main__':
    main()

#!/usr/bin/env python3
"""Plot bearings over time from CSV data."""

import sys
import pandas as pd
import matplotlib.pyplot as plt


def main():
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <csv_file>", file=sys.stderr)
        sys.exit(1)

    csv_file = sys.argv[1]
    df = pd.read_csv(csv_file)

    # Handle both old (time_s) and new (ts) column formats
    if 'ts' in df.columns:
        df['ts'] = pd.to_datetime(df['ts'])
        df['time_s'] = (df['ts'] - df['ts'].iloc[0]).dt.total_seconds()

    fig, axes = plt.subplots(2, 1, figsize=(12, 8), sharex=True)

    # Top plot: bearing over time
    ax1 = axes[0]
    ax1.scatter(df['time_s'], df['bearing'], s=1, alpha=0.5, label='Smoothed')
    ax1.scatter(df['time_s'], df['raw'], s=1, alpha=0.3, label='Raw')
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
    plt.savefig(csv_file.replace('.csv', '_plot.png'), dpi=150)
    print(f"Saved plot to {csv_file.replace('.csv', '_plot.png')}")
    plt.show()


if __name__ == '__main__':
    main()

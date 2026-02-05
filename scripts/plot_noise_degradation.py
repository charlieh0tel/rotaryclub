#!/usr/bin/env python3
"""
Plot noise degradation curves for RDF bearing estimation.

Reads CSV data from stdin (from noise_analysis binary) and creates
a 4-panel plot showing error vs different noise parameters.

Usage:
    cargo run --bin noise_analysis --features test-utils | python scripts/plot_noise_degradation.py
"""

import sys
import csv
import matplotlib.pyplot as plt
from collections import defaultdict


def parse_csv(input_stream):
    """Parse CSV data into a dictionary grouped by noise type."""
    data = defaultdict(lambda: {"param": [], "zc_error": [], "corr_error": []})

    reader = csv.DictReader(input_stream)
    for row in reader:
        noise_type = row["noise_type"]
        data[noise_type]["param"].append(float(row["parameter"]))
        data[noise_type]["zc_error"].append(float(row["zc_error"]))
        data[noise_type]["corr_error"].append(float(row["corr_error"]))

    return data


def plot_panel(ax, params, zc_errors, corr_errors, title, xlabel, ylabel="Max Error (degrees)"):
    """Plot a single panel with both methods."""
    ax.plot(params, zc_errors, "b-o", label="Zero-Crossing", markersize=5, linewidth=2, alpha=0.8)
    ax.plot(params, corr_errors, "r--s", label="Correlation", markersize=4, linewidth=1.5, alpha=0.8, markerfacecolor='none')
    ax.set_title(title)
    ax.set_xlabel(xlabel)
    ax.set_ylabel(ylabel)
    ax.legend()
    ax.grid(True, alpha=0.3)
    ax.set_ylim(bottom=0)


def main():
    data = parse_csv(sys.stdin)

    fig, axes = plt.subplots(2, 2, figsize=(12, 10))
    fig.suptitle("RDF Bearing Estimation: Noise Degradation Analysis", fontsize=14)

    if "awgn" in data:
        plot_panel(
            axes[0, 0],
            data["awgn"]["param"],
            data["awgn"]["zc_error"],
            data["awgn"]["corr_error"],
            "Additive White Gaussian Noise",
            "SNR (dB)",
        )
        axes[0, 0].invert_xaxis()

    if "fading" in data:
        plot_panel(
            axes[0, 1],
            data["fading"]["param"],
            data["fading"]["zc_error"],
            data["fading"]["corr_error"],
            "Rayleigh Fading",
            "Doppler Spread (Hz)",
        )

    if "multipath" in data:
        plot_panel(
            axes[1, 0],
            data["multipath"]["param"],
            data["multipath"]["zc_error"],
            data["multipath"]["corr_error"],
            "Multipath Interference",
            "Delay (% of rotation period)",
        )

    if "impulse" in data:
        plot_panel(
            axes[1, 1],
            data["impulse"]["param"],
            data["impulse"]["zc_error"],
            data["impulse"]["corr_error"],
            "Impulse Noise",
            "Impulse Rate (Hz)",
        )

    plt.tight_layout()
    plt.savefig("noise_degradation.png", dpi=150)
    print("Saved plot to noise_degradation.png", file=sys.stderr)
    plt.show()


if __name__ == "__main__":
    main()

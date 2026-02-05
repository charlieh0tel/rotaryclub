#!/usr/bin/env python3
"""
Plot noise degradation curves for RDF bearing estimation with error bars.

Reads CSV data from stdin (from noise_analysis binary) and creates
a 4-panel plot showing error vs different noise parameters.

Usage:
    cargo run --release --bin noise_analysis --features test-utils | python scripts/plot_noise_degradation.py
"""

import sys
import csv
import matplotlib.pyplot as plt
from collections import defaultdict


def parse_csv(input_stream):
    """Parse CSV data into a dictionary grouped by noise type."""
    data = defaultdict(
        lambda: {
            "param": [],
            "zc_mean": [],
            "zc_std": [],
            "corr_mean": [],
            "corr_std": [],
        }
    )

    reader = csv.DictReader(input_stream)
    for row in reader:
        noise_type = row["noise_type"]
        data[noise_type]["param"].append(float(row["parameter"]))
        data[noise_type]["zc_mean"].append(float(row["zc_mean"]))
        data[noise_type]["zc_std"].append(float(row["zc_std"]))
        data[noise_type]["corr_mean"].append(float(row["corr_mean"]))
        data[noise_type]["corr_std"].append(float(row["corr_std"]))

    return data


def plot_panel(ax, params, zc_mean, zc_std, corr_mean, corr_std, title, xlabel, ylabel="Mean Error (degrees)"):
    """Plot a single panel with both methods and error bars."""
    ax.errorbar(
        params, zc_mean, yerr=zc_std,
        fmt="b-o", label="Zero-Crossing", markersize=4, linewidth=1.5,
        capsize=2, capthick=1, alpha=0.8
    )
    ax.errorbar(
        params, corr_mean, yerr=corr_std,
        fmt="r--s", label="Correlation", markersize=3, linewidth=1.5,
        capsize=2, capthick=1, alpha=0.8, markerfacecolor="none"
    )
    ax.set_title(title)
    ax.set_xlabel(xlabel)
    ax.set_ylabel(ylabel)
    ax.legend()
    ax.grid(True, alpha=0.3)
    ax.set_ylim(bottom=0)


def main():
    data = parse_csv(sys.stdin)

    fig, axes = plt.subplots(2, 2, figsize=(12, 10))
    fig.suptitle("RDF Bearing Estimation: Noise Degradation Analysis (N=10 trials)", fontsize=14)

    if "awgn" in data:
        plot_panel(
            axes[0, 0],
            data["awgn"]["param"],
            data["awgn"]["zc_mean"],
            data["awgn"]["zc_std"],
            data["awgn"]["corr_mean"],
            data["awgn"]["corr_std"],
            "Additive White Gaussian Noise",
            "SNR (dB)",
        )
        axes[0, 0].invert_xaxis()

    if "fading" in data:
        plot_panel(
            axes[0, 1],
            data["fading"]["param"],
            data["fading"]["zc_mean"],
            data["fading"]["zc_std"],
            data["fading"]["corr_mean"],
            data["fading"]["corr_std"],
            "Rayleigh Fading",
            "Doppler Spread (Hz)",
        )

    if "multipath" in data:
        plot_panel(
            axes[1, 0],
            data["multipath"]["param"],
            data["multipath"]["zc_mean"],
            data["multipath"]["zc_std"],
            data["multipath"]["corr_mean"],
            data["multipath"]["corr_std"],
            "Multipath Interference",
            "Delay (% of rotation period)",
        )

    if "impulse" in data:
        plot_panel(
            axes[1, 1],
            data["impulse"]["param"],
            data["impulse"]["zc_mean"],
            data["impulse"]["zc_std"],
            data["impulse"]["corr_mean"],
            data["impulse"]["corr_std"],
            "Impulse Noise",
            "Impulse Rate (Hz)",
        )

    plt.tight_layout()
    plt.savefig("noise_degradation.png", dpi=150)
    print("Saved plot to noise_degradation.png", file=sys.stderr)
    plt.show()


if __name__ == "__main__":
    main()

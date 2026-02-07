#!/usr/bin/env python3
"""Measure audio levels in WAV files."""

import argparse
import math
import sys

import numpy as np
import soundfile as sf


def analyze(path, channel_names=None):
    data, rate = sf.read(path, dtype="float32")

    if data.ndim == 1:
        channels = [("Mono", data)]
    else:
        n_ch = data.shape[1]
        if channel_names and len(channel_names) == n_ch:
            names = channel_names
        else:
            names = [f"Ch{i}" for i in range(n_ch)]
        channels = [(names[i], data[:, i]) for i in range(n_ch)]

    print(f"{path}: {data.shape[0]} frames, {len(channels)}ch, {rate}Hz")

    for name, ch in channels:
        dc = float(np.mean(ch))
        peak = float(np.max(np.abs(ch)))
        rms = float(np.sqrt(np.mean(ch * ch)))
        ac_rms = float(np.sqrt(np.mean((ch - dc) ** 2)))
        peak_db = 20 * math.log10(peak) if peak > 0 else float("-inf")
        rms_db = 20 * math.log10(rms) if rms > 0 else float("-inf")
        over_1 = int(np.sum(np.abs(ch) > 1.0))
        over_09 = int(np.sum(np.abs(ch) > 0.9))
        crest = peak / rms if rms > 0 else float("inf")

        print(f"  {name}:")
        print(f"    dc  ={dc:+.6f}")
        print(f"    peak={peak:.4f} ({peak_db:+.1f} dBFS)")
        print(f"    rms ={rms:.4f} ({rms_db:+.1f} dBFS)  ac_rms={ac_rms:.4f}")
        print(f"    crest factor={crest:.1f} ({20*math.log10(crest):+.1f} dB)")
        print(f"    samples >0.9={over_09}  >1.0={over_1}")


def main():
    parser = argparse.ArgumentParser(description="Measure audio levels in WAV files")
    parser.add_argument("files", nargs="+", help="WAV files to analyze")
    parser.add_argument(
        "--channels",
        nargs="*",
        help="Channel names (e.g. Doppler NorthTick)",
    )
    args = parser.parse_args()

    for path in args.files:
        try:
            analyze(path, args.channels)
        except Exception as e:
            print(f"{path}: error - {e}", file=sys.stderr)
        if path != args.files[-1]:
            print()


if __name__ == "__main__":
    main()

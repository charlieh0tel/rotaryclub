#!/usr/bin/env python3
"""Compare pulse-centroid weighting schemes against a fitted rotation model.

The centroid estimators differ only in how each sample in the window is
weighted before the first moment is taken. Three choices interact:

  * the exponent -- amplitude (x) or energy (x^2);
  * whether the bipolar highpassed pulse is clipped to its positive part
    before weighting, or used as-is;
  * the window half-width.

Which combination wins is not fixed. It depends on how wide the pulse is once
filtered, and therefore on the highpass cutoff: a narrow pulse needs its
neighbours to interpolate between, so concentrating weight on the peak throws
information away, while a wider pulse has skirts worth down-weighting. Run this
after changing the cutoff, the filter length, or the capture hardware.

Scores are RMS bearing error of the per-tick estimate against a fitted
constant rotation rate, optionally after a second-order DPLL. This mirrors
`north_hpf_sweep` but sweeps weighting rather than cutoff, and unlike the Rust
binary it can measure schemes that are not in the shipped enum.

Usage:
    python3 scripts/centroid_weighting_report.py                    # both cutoffs
    python3 scripts/centroid_weighting_report.py --cutoff 1000      # one cutoff
    python3 scripts/centroid_weighting_report.py --loop             # add DPLL curves
"""
from __future__ import annotations

import argparse
import wave
from pathlib import Path
from typing import Dict, List, Sequence, Tuple

import numpy as np

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_WAV = (
    REPO_ROOT / "data" / "wouxun_KG-UV3D_10ft_away_16kbps_stereo_440.800_test1.wav"
)

# Nominal rotation rate of the switcher, in Hz. Used only to seed the pick
# spacing and the integer tick numbering; the actual rate is fitted.
ROTATION_HZ_NOMINAL = 1602.564

# Cutoffs worth comparing: the branch default, and the value the earlier
# estimator measurements were taken at.
DEFAULT_CUTOFFS = (1000.0, 5000.0)

# Weighting schemes. Name -> (exponent, clip-to-positive).
SCHEMES: Dict[str, Tuple[int, bool]] = {
    "amplitude clipped": (1, True),
    "energy clipped": (2, True),
    "energy unclipped": (2, False),
    "amplitude rectified": (1, False),  # |x|, i.e. full-wave rather than clipped
}

WIDTHS = (2, 3, 4, 5, 6, 8)
LOOP_BANDWIDTHS = (0.5, 2.0, 8.0, 20.0)


def read_channel(path: Path, channel: int) -> Tuple[float, np.ndarray]:
    with wave.open(str(path), "rb") as handle:
        rate = handle.getframerate()
        frames = handle.getnframes()
        channels = handle.getnchannels()
        raw = np.frombuffer(handle.readframes(frames), dtype=np.int16)
    samples = raw.astype(float) / 32768.0
    return float(rate), samples.reshape(-1, channels)[:, channel]


def highpass(x: np.ndarray, rate: float, cutoff: float, taps: int = 63) -> np.ndarray:
    """Windowed-sinc highpass by spectral inversion, matching FirHighpass."""
    n = np.arange(taps) - (taps - 1) / 2
    lowpass = np.sinc(2 * cutoff / rate * n) * np.hamming(taps)
    lowpass /= lowpass.sum()
    hp = -lowpass
    hp[(taps - 1) // 2] += 1.0
    return np.convolve(x, hp, mode="same")


def find_picks(y: np.ndarray, rate: float) -> np.ndarray:
    """Crude peak picking: threshold, then the local extremum in each period."""
    period = rate / ROTATION_HZ_NOMINAL
    threshold = 0.35 * np.abs(y).max()
    candidates = np.where(np.abs(y) > threshold)[0]
    picks: List[int] = []
    last = -1e9
    for i in candidates:
        if i - last > 0.7 * period:
            lo, hi = max(0, i - 4), min(len(y), i + 10)
            j = lo + int(np.argmax(np.abs(y[lo:hi])))
            picks.append(j)
            last = j
    return np.array(sorted(set(picks)))


def centroid_offset(y: np.ndarray, j: int, half_width: int, exponent: int, clip: bool) -> float:
    """Fractional offset of the weighted first moment from the peak index.

    An unclipped odd exponent leaves signed weights, so the denominator can pass
    through zero and throw the moment far outside the window; that case is
    rejected rather than allowed to dominate the RMS.
    """
    lo, hi = max(0, j - half_width), min(len(y), j + half_width + 1)
    segment = y[lo:hi]
    indices = np.arange(lo, hi)
    if clip:
        weights = np.clip(segment, 0.0, None) ** exponent
    elif exponent % 2 == 0:
        weights = segment**exponent
    else:
        weights = np.abs(segment) ** exponent
    total = weights.sum()
    if abs(total) < 1e-12:
        return 0.0
    offset = float((indices * weights).sum() / total) - j
    return offset if abs(offset) <= half_width else 0.0


def parabolic_offset(v: np.ndarray, j: int) -> float:
    if j <= 0 or j + 1 >= len(v):
        return 0.0
    a, b, c = v[j - 1], v[j], v[j + 1]
    denominator = a - 2 * b + c
    if abs(denominator) < 1e-15:
        return 0.0
    return float(np.clip(0.5 * (a - c) / denominator, -1, 1))


def build_template(y: np.ndarray, picks: np.ndarray, offsets: np.ndarray, W: int = 8) -> np.ndarray:
    """Average the pulses onto a common sub-sample grid by sinc resampling."""
    n = np.arange(-W, W + 1)
    accumulator = np.zeros(2 * W + 1)
    count = 0
    for j, d in zip(picks, offsets):
        if j - W - 8 < 0 or j + W + 9 >= len(y):
            continue
        m = np.arange(-W - 8, W + 9)
        accumulator += np.sinc((n[:, None] + d) - m[None, :]) @ y[j - W - 8 : j + W + 9]
        count += 1
    template = accumulator / max(count, 1)
    peak = np.abs(template).max()
    return template / peak if peak > 0 else template


def matched_filter_offsets(y: np.ndarray, picks: np.ndarray, template: np.ndarray, W: int = 8) -> np.ndarray:
    out = []
    for j in picks:
        lo, hi = j - W - 3, j + W + 4
        if lo < 0 or hi >= len(y):
            out.append(np.nan)
            continue
        c = np.correlate(y[lo:hi], template, mode="valid")
        k = int(np.argmax(np.abs(c)))
        c = c * np.sign(c[k])
        out.append((k - 3) + parabolic_offset(c, k))
    return np.array(out)


def fit_rotation(epochs: np.ndarray, rate: float) -> Tuple[float, np.ndarray]:
    """Fit samples-per-rotation and return the modelled epoch of each tick."""
    period = rate / ROTATION_HZ_NOMINAL
    k = np.round((epochs - epochs[0]) / period)
    fit = np.polyfit(k, epochs, 1)
    for _ in range(5):
        k = np.round((epochs - fit[1]) / fit[0])
        fit = np.polyfit(k, epochs, 1)
    return float(fit[0]), np.polyval(fit, k)


def run_dpll(epochs: np.ndarray, rate: float, f0: float, bandwidth: float, zeta: float = 0.707) -> np.ndarray:
    """Second-order loop, emitting the NCO prediction for each tick."""
    wn = 2 * np.pi * bandwidth / f0
    kp = 2 * zeta * wn
    ki = wn * wn / (rate / f0)
    freq = 2 * np.pi * f0 / rate
    phase = 0.0
    out = np.empty(len(epochs))
    last = epochs[0]
    for i, t in enumerate(epochs):
        phase += freq * (t - last)
        last = t
        phase = (phase + np.pi) % (2 * np.pi) - np.pi
        error = (-phase + np.pi) % (2 * np.pi) - np.pi
        out[i] = t + error / freq
        freq += ki * error
        phase = (phase + kp * error + np.pi) % (2 * np.pi) - np.pi
    return out


def measure(path: Path, cutoff: float, skip: int, with_loop: bool) -> None:
    rate, x = read_channel(path, 1)
    y = highpass(x, rate, cutoff)
    picks = find_picks(y, rate)
    picks = picks[(picks > 40) & (picks < len(y) - 40)]

    parabolic = np.array([parabolic_offset(np.abs(y), j) for j in picks])
    template = build_template(y, picks, parabolic)
    mf = matched_filter_offsets(y, picks, template)
    finite = np.isfinite(mf)
    picks, mf = picks[finite], mf[finite]

    period, model = fit_rotation(picks.astype(float) + mf, rate)
    f0 = rate / period

    def bearing_rms(residual: np.ndarray) -> float:
        # Clip so that a handful of gross outliers cannot set the scale.
        return float(np.sqrt(np.mean(np.clip(residual, -3, 3) ** 2)) / period * 360.0)

    print(f"\n=== {path.name}   highpass {cutoff:g} Hz ===")
    print(f"rate {f0:.4f} Hz   ticks {len(picks)}   one sample = {360.0/period:.1f} deg")
    print("\nraw per-tick RMS bearing error, degrees")
    header = "".join(f"  w={w:<5}" for w in WIDTHS)
    print(f"{'scheme':<21}{header}")

    best: Dict[str, Tuple[int, float]] = {}
    for name, (exponent, clip) in SCHEMES.items():
        row = []
        for w in WIDTHS:
            offsets = np.array([centroid_offset(y, j, w, exponent, clip) for j in picks])
            row.append(bearing_rms((picks + offsets - model)[skip:]))
        best[name] = (WIDTHS[int(np.argmin(row))], min(row))
        print(f"{name:<21}" + "".join(f"{v:7.3f} " for v in row))

    print(f"{'hard limiter':<21}{bearing_rms((picks - model)[skip:]):7.3f}  (no interpolation)")
    print(f"{'matched filter':<21}{bearing_rms((picks + mf - model)[skip:]):7.3f}  (template from this capture)")

    if not with_loop:
        return

    print("\nafter a second-order DPLL, at each scheme's best half-width")
    print(f"{'scheme':<21}" + "".join(f"{b:>8g} Hz" for b in LOOP_BANDWIDTHS))
    for name, (exponent, clip) in SCHEMES.items():
        w = best[name][0]
        offsets = np.array([centroid_offset(y, j, w, exponent, clip) for j in picks])
        epochs = picks + offsets
        curve = [
            bearing_rms((run_dpll(epochs, rate, f0, bw) - model)[skip:])
            for bw in LOOP_BANDWIDTHS
        ]
        print(f"{name + f' w={w}':<21}" + "".join(f"{c:9.3f} " for c in curve))


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--wav", type=Path, default=DEFAULT_WAV, help="capture to measure")
    parser.add_argument(
        "--cutoff",
        type=float,
        action="append",
        help="highpass cutoff in Hz; repeatable (default: 1000 and 5000)",
    )
    parser.add_argument(
        "--skip",
        type=int,
        default=3000,
        help="ticks to discard before scoring, to let the loop settle",
    )
    parser.add_argument("--loop", action="store_true", help="also report DPLL curves")
    args = parser.parse_args(argv)

    for cutoff in args.cutoff or list(DEFAULT_CUTOFFS):
        measure(args.wav, cutoff, args.skip, args.loop)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

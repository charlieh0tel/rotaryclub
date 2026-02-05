#!/usr/bin/env python3
"""Analyze WAV files to find the actual Doppler frequency and north tick quality."""

import argparse
import glob
import sys

import numpy as np
import soundfile as sf
from scipy.signal import butter, filtfilt, find_peaks


def find_peak_frequency(filepath, bandpass_low=1400, bandpass_high=1800):
    """Find the dominant frequency in a WAV file's left channel."""
    data, rate = sf.read(filepath)
    if len(data.shape) > 1:
        left = data[:, 0].astype(float)
    else:
        left = data.astype(float)

    nyq = rate / 2
    b, a = butter(4, [bandpass_low / nyq, bandpass_high / nyq], btype='band')
    segment = left[:min(rate * 2, len(left))]
    filtered = filtfilt(b, a, segment)

    n_fft = len(filtered) * 4
    fft = np.abs(np.fft.fft(filtered, n_fft))
    freqs = np.fft.fftfreq(n_fft, 1 / rate)

    mask = (freqs > bandpass_low) & (freqs < bandpass_high)
    peak_idx = np.argmax(fft[mask])

    return freqs[mask][peak_idx], fft[mask][peak_idx]


def analyze_north_tick(filepath, highpass_cutoff=5000.0):
    """Analyze north tick channel for rotation rate and lock quality."""
    data, rate = sf.read(filepath)
    if len(data.shape) < 2:
        return None, None, None

    right = data[:, 1].astype(float)

    # Highpass filter to isolate tick transients
    nyq = rate / 2
    b, a = butter(2, highpass_cutoff / nyq, btype='high')
    filtered = filtfilt(b, a, right)

    # Find peaks (north ticks)
    threshold = np.std(filtered) * 3
    peaks, _ = find_peaks(np.abs(filtered), height=threshold, distance=int(rate / 2000))

    if len(peaks) < 3:
        return None, None, None

    # Calculate inter-tick intervals
    intervals = np.diff(peaks)
    mean_interval = np.mean(intervals)
    rotation_freq = rate / mean_interval

    # Phase error variance (normalized by mean interval)
    interval_errors = (intervals - mean_interval) / mean_interval
    phase_error_var = np.var(interval_errors)

    # Lock quality: based on phase error std and frequency stability
    phase_std = np.std(interval_errors)
    phase_score = max(0, 1 - phase_std * 10)  # Scale so 0.1 std -> 0 quality

    freq_cv = np.std(intervals) / mean_interval  # Coefficient of variation
    freq_score = max(0, 1 - freq_cv * 100)

    lock_quality = 0.7 * phase_score + 0.3 * freq_score

    return rotation_freq, phase_error_var, lock_quality


def main():
    parser = argparse.ArgumentParser(description='Analyze Doppler frequency and north tick quality')
    parser.add_argument('files', nargs='*', help='WAV files to analyze (default: data/*.wav)')
    parser.add_argument('--bandpass-low', type=float, default=1400,
                        help='Bandpass lower cutoff (default: 1400)')
    parser.add_argument('--bandpass-high', type=float, default=1800,
                        help='Bandpass upper cutoff (default: 1800)')
    args = parser.parse_args()

    files = args.files if args.files else sorted(glob.glob('data/*.wav'))

    if not files:
        print("No WAV files found", file=sys.stderr)
        sys.exit(1)

    print(f"{'File':<40} {'Doppler (Hz)':>12} {'NorthTick (Hz)':>14} {'PhaseVar':>10} {'LockQual':>10}")
    print("-" * 90)

    for f in files:
        try:
            doppler_freq, _ = find_peak_frequency(f, args.bandpass_low, args.bandpass_high)
            north_freq, phase_var, lock_qual = analyze_north_tick(f)

            name = f.split('/')[-1]
            north_str = f"{north_freq:14.1f}" if north_freq else "N/A".rjust(14)
            phase_str = f"{phase_var:10.6f}" if phase_var is not None else "N/A".rjust(10)
            lock_str = f"{lock_qual:10.3f}" if lock_qual is not None else "N/A".rjust(10)

            print(f"{name:<40} {doppler_freq:>12.1f} {north_str} {phase_str} {lock_str}")
        except Exception as e:
            print(f"{f}: error - {e}", file=sys.stderr)


if __name__ == '__main__':
    main()

#!/usr/bin/env python3
"""Analyze WAV files to find north tick signals and their relationship to doppler frequency."""

import argparse
import glob
import sys

import numpy as np
import scipy.io.wavfile as wav
from scipy.signal import butter, filtfilt, find_peaks


def find_doppler_frequency(signal, rate, bandpass_low=1300, bandpass_high=1900):
    """Find the dominant doppler frequency via FFT."""
    nyq = rate / 2
    b, a = butter(4, [bandpass_low / nyq, bandpass_high / nyq], btype='band')
    segment = signal[:min(rate * 2, len(signal))]
    filtered = filtfilt(b, a, segment)

    n_fft = len(filtered) * 4
    fft = np.abs(np.fft.fft(filtered, n_fft))
    freqs = np.fft.fftfreq(n_fft, 1 / rate)

    mask = (freqs > bandpass_low) & (freqs < bandpass_high)
    peak_idx = np.argmax(fft[mask])

    return freqs[mask][peak_idx]


def find_tick_rate(signal, rate, doppler_freq, hp_cutoffs=[3000, 5000, 8000], thresholds=[2, 3, 4, 5]):
    """Find periodic tick signal that's a submultiple of doppler frequency."""
    nyq = rate / 2
    best_match = None
    best_ratio_diff = 999

    for hp_freq in hp_cutoffs:
        if hp_freq >= nyq:
            continue

        b, a = butter(2, hp_freq / nyq, btype='high')
        hp = filtfilt(b, a, signal)

        for mult in thresholds:
            threshold = np.std(hp) * mult
            min_distance = int(rate / 2000)
            peaks, _ = find_peaks(np.abs(hp), height=threshold, distance=min_distance)

            if len(peaks) < 10:
                continue

            intervals = np.diff(peaks)
            mean_interval = np.mean(intervals)
            tick_freq = rate / mean_interval
            interval_std = np.std(intervals)

            # Reject if std is too high (> 5% of mean interval)
            if interval_std > mean_interval * 0.05:
                continue

            # Check if ratio to doppler is close to an integer (1-10)
            ratio = doppler_freq / tick_freq
            for expected_ratio in range(1, 11):
                ratio_diff = abs(ratio - expected_ratio)
                if ratio_diff < 0.15 and ratio_diff < best_ratio_diff:
                    best_ratio_diff = ratio_diff
                    best_match = {
                        'tick_freq': tick_freq,
                        'ratio': ratio,
                        'expected_ratio': expected_ratio,
                        'interval_std': interval_std,
                        'n_ticks': len(peaks),
                        'hp_cutoff': hp_freq,
                        'threshold_mult': mult,
                    }

    return best_match


def analyze_file(filepath, bandpass_low=1300, bandpass_high=1900):
    """Analyze a single WAV file for doppler and tick signals."""
    rate, data = wav.read(filepath)

    if len(data.shape) < 2:
        return None

    results = {'filepath': filepath, 'rate': rate, 'channels': []}

    for ch_idx, ch_name in [(0, 'left'), (1, 'right')]:
        channel = data[:, ch_idx].astype(float)

        doppler_freq = find_doppler_frequency(channel, rate, bandpass_low, bandpass_high)
        tick_info = find_tick_rate(channel, rate, doppler_freq)

        ch_result = {
            'name': ch_name,
            'doppler_freq': doppler_freq,
            'tick': tick_info,
        }
        results['channels'].append(ch_result)

    return results


def main():
    parser = argparse.ArgumentParser(description='Analyze north tick signals in WAV files')
    parser.add_argument('files', nargs='*', help='WAV files to analyze (default: data/*.wav)')
    parser.add_argument('--bandpass-low', type=float, default=1300,
                        help='Doppler bandpass lower cutoff (default: 1300)')
    parser.add_argument('--bandpass-high', type=float, default=1900,
                        help='Doppler bandpass upper cutoff (default: 1900)')
    parser.add_argument('-v', '--verbose', action='store_true',
                        help='Show detailed tick detection info')
    args = parser.parse_args()

    files = args.files if args.files else sorted(glob.glob('data/*.wav'))

    if not files:
        print("No WAV files found", file=sys.stderr)
        sys.exit(1)

    print(f"{'File':<40} {'Ch':<6} {'Doppler':>10} {'Tick':>10} {'Ratio':>8} {'Std':>8}")
    print("-" * 88)

    for f in files:
        try:
            results = analyze_file(f, args.bandpass_low, args.bandpass_high)
            if results is None:
                print(f"{f}: not stereo", file=sys.stderr)
                continue

            name = f.split('/')[-1][:38]

            for ch in results['channels']:
                doppler = ch['doppler_freq']
                tick = ch['tick']

                if tick:
                    tick_freq = tick['tick_freq']
                    ratio_str = f"1/{tick['expected_ratio']}"
                    std_str = f"{tick['interval_std']:.1f}"
                    print(f"{name:<40} {ch['name']:<6} {doppler:>10.1f} {tick_freq:>10.1f} {ratio_str:>8} {std_str:>8}")

                    if args.verbose:
                        print(f"    HP>{tick['hp_cutoff']}Hz @ {tick['threshold_mult']}x std, {tick['n_ticks']} ticks")
                else:
                    print(f"{name:<40} {ch['name']:<6} {doppler:>10.1f} {'--':>10} {'--':>8} {'--':>8}")

        except Exception as e:
            print(f"{f}: error - {e}", file=sys.stderr)


if __name__ == '__main__':
    main()

# Rotary Club - Pseudo Doppler Radio Direction Finding

Rust implementation of a pseudo doppler RDF system that calculates bearing angles (0-360°) from stereo audio.

Now includes a gui:  [Video](https://youtu.be/nQoKVjQKTF8)

## Installation

Prebuilt Debian (`.deb`) packages are available on [GitHub Releases](https://github.com/charlieh0tel/rotaryclub/releases).

### From Source

```bash
# Clone and build
git clone https://github.com/charlieh0tel/rotaryclub.git
cd rotaryclub
cargo build --release

# Run directly
./target/release/rotaryclub
```

## Quick Start

```bash
# Run with default settings (correlation method, DPLL tracking)
rotaryclub
# Or from source: cargo run

# Use zero-crossing method instead
rotaryclub --method zero-crossing

# Use simple north tracking mode
rotaryclub --north-mode simple

# Swap left/right channels if wired differently
rotaryclub --swap-channels

# Increase output rate to 20 Hz
rotaryclub --output-rate 20

# Apply north offset calibration (e.g., antenna rotated 45° from true north)
rotaryclub --north-offset 45

# Enable info logging
rotaryclub -v

# Enable debug logging
rotaryclub -vv

# Combine options
rotaryclub --method correlation --north-mode dpll --north-offset 45 -v

# Compare pulse estimators (the tracking loop hides most of the difference)
rotaryclub --north-estimator hard-limiter
```

### Testing (from source)

```bash
# Test with WAV file
rotaryclub -i data/doppler-test-2023-04-10-ft-70d.wav

# Generate synthetic test signals (example utility)
cargo run --example generate_test_wav

# End-to-end synthetic pipeline test
cargo run --example synthetic_rdf
```

## Usage

The program reads stereo audio:
- **Left channel**: FM radio audio (contains Doppler tone)
- **Right channel**: North timing reference pulses

Output:
```
Bearing: 137.5° (raw: 136.8°) confidence: 0.95
```

#### Output Measures

See [METRICS.md](METRICS.md) for what each of these can and cannot see, what the CI
gates cover, and how to add a metric without fooling yourself.

- `bearing`: Smoothed azimuth estimate in degrees, wrapped to `[0, 360)`.
- `raw`: Instantaneous unsmoothed azimuth estimate in degrees, wrapped to `[0, 360)`.
- `confidence`: Quality score in `[0, 1]`, derived from `bearing_uncertainty_deg` as
  `1 / (1 + (uncertainty / half_confidence_deg)^2)`. It reads 0.5 at the configured
  half-confidence point, six degrees by default, and 0 when the uncertainty could not
  be estimated at all — which is the absence of a claim, not a claim of a bad bearing.
- `bearing_uncertainty_deg`: Estimated one-sigma uncertainty of this bearing, in degrees.
  Two terms in quadrature: the doppler tone against the noise it sits in — `1/(snr·looks)`
  with `looks` counted against the filter's measured noise-equivalent bandwidth and
  deflated by the residual's measured lag correlation, plus a threshold correction where
  outliers leave the linear regime — and the timing variance of the north tick actually
  emitted. Empty when it cannot be estimated. This is precision rather than accuracy: a
  displacement every estimate shares — a reflection, a mis-set north offset, a detection
  bias the DPLL follows perfectly — is invisible to it. Measured against the bearing
  scatter at precision scale (contiguous 40 ms windows on carrier-bearing stretches), it
  runs at 1.2 to 1.4 on all three recordings and about 1.2 to 1.8 on synthetic signal —
  slightly conservative everywhere.

  Scale matters when checking it: fading and multipath wander the bearing over seconds,
  and at long windows that wander lands in measured scatter as apparent understatement of
  a thing the figure never claimed. Seventy percent of the ft-70d capture is receiver
  hiss between overs, and a bearing measured on hiss is uniformly distributed; gate on
  carrier before comparing.

  It is largely blind to reflections. Noise moves the SNR this is derived from, so it sees
  noise by construction; a reflection puts the bearing between two paths while the tone
  stays strong. With a reflected path 0.45 of the direct one, filtering at 0.5 confidence
  improves median error by 5 percent while discarding 42 percent of the bearings, against
  23 to 58 percent on a clean channel, and rank correlation against actual error falls from
  0.40 to 0.13. See `probe_confidence_multipath`.
- `snr_db`: Estimated in-band Doppler SNR (dB), computed from correlated signal power versus residual power.
- `signal_strength`: Carrier-presence metric in `[0, 1]`: the fraction of in-band amplitude
  that projects onto the tick-locked reference, behind an absolute in-band power gate. Both
  methods report the same quantity. (Zero-crossing used to report crossing density, which
  only discriminated through the old leaky bandpass.)
- `resultant_length`: Mean resultant length of the Doppler phase, `[0, 1]` — 1 when the
  looks all point the same way, 0 when they are scattered. KR6DD averages the per-zero-crossing
  phase vectors directly; here it is derived from the estimated SNR instead, as
  `exp(-1/(2·snr))`, the resultant length a von Mises phase at that SNR has. So it is not
  an independent measurement: on this receiver it is a restatement of `snr_db` on a
  bounded scale, and it will not report scatter that the SNR estimate did not already see.
- `tone_peak`: Largest positive sample of the filtered Doppler signal, in full-scale
  units `[0, 1]`.

  The last two are KR6DD's metrics, defined in his `RPiDDFengine` and carried in the
  KN5R "C" sentence as *angle vector average magnitude* and *FIR filtered Doppler tone
  peak value*. They are carried here unscaled and reported on every output. That format wants them as 0-999 and
  0-999 thousandths respectively; the scaling belongs to the formatter, not the pipeline.

#### North Tracking Quality Measures

- `lock_quality`: DPLL-only lock score in `[0, 1]`: `1 - phase_error_std_dev / (pi/sqrt(3))`,
  clamped. The divisor is the standard deviation of a uniformly distributed phase error, so
  a loop learning nothing from its detections reads 0 and a tight lock reads near 1. It once
  blended in a frequency-stability term, which rewarded exactly the failure the field exists
  to catch — a stuck oscillator has the steadiest frequency of all, and read 0.596 while
  tracking nothing. What it still cannot claim: a loop locked onto a coherent interferer
  reads high; this is loop-against-input coherence, never input-against-truth.
- `phase_error_variance`: Rolling variance (rad^2) of DPLL phase error; lower indicates tighter phase lock.
- Windowing: rolling statistics are computed over the last 128 detected ticks.
- Availability: in `--north-mode dpll` these fields are populated; in `--north-mode simple` they are not produced (`null`/empty in JSON/CSV).

### CLI Options

```
-m, --method <METHOD>            Bearing calculation method
                                 [correlation (default) | zero-crossing]

-n, --north-mode <NORTH_MODE>    North tick tracking mode
                                 [dpll (default) | simple]

    --north-estimator <NAME>     Sub-sample estimator for the reference pulse
                                 [energy-centroid (default) | amplitude-centroid
                                  | hard-limiter]
                                 The centroids resolve the pulse below one
                                 sample; hard-limiter reports the peak index,
                                 which is 12 degrees of bearing at 48 kHz.

-s, --swap-channels              Swap left/right channels

-r, --output-rate <OUTPUT_RATE>  Output rate in Hz [default: 10.0]
                                 Throttled in signal time, so file input
                                 emits at this rate too; raise it to see
                                 more of a recording's ticks

-o, --north-offset <DEGREES>     North reference offset in degrees [default: 0.0]
                                 Added to all bearings for calibration

-f, --format <FORMAT>            Output format [default: text]
                                 [text | kn5r | json | csv]

-i, --input <INPUT>              Input WAV file (default: live device capture)

-v, --verbose                    Increase logging (-v=info, -vv=debug, -vvv=trace)

    --rotation <ROTATION>        Rotation frequency (e.g. 1602, 1602hz, 624us)

    --remove-dc                  Remove DC offset from audio

    --dump-audio <PATH>          Dump captured audio to WAV file

    --north-tick-gain <DB>       North tick input gain in dB [default: 0]

    --device <NAME>              Select input device by substring match

    --list-devices               List available input devices and exit

-h, --help                       Print help
```

## Examples

```bash
cargo run --example audio_loopback      # Verify audio input
cargo run --example filter_test         # Test DSP filters
cargo run --example synthetic_rdf       # Test with generated signals
cargo run --example compute_rotation    # Measure rotation frequency
cargo run --example analyze_channels    # Identify which channel is which
```

## Plotting

The `scripts/plot_bearings.py` script visualizes bearing data from CSV output.

```bash
# Generate CSV from a WAV file
rotaryclub -i recording.wav -f csv > recording.csv

# Plot with the default confidence threshold (0.5)
python3 scripts/plot_bearings.py recording.csv

# Custom thresholds: confidence, and a ceiling on the stated uncertainty
python3 scripts/plot_bearings.py recording.csv --min-confidence 0.7 --max-uncertainty 4.0
```

Requires `pandas` and `matplotlib`.

## Configuration

Channel assignment and signal processing parameters are in `src/config.rs`. See DESIGN.md for details.

## Building

```bash
# Build release binary
cargo build --release

# Run tests
cargo test

# Run north-tick timing gate (writes CSV + Markdown summary under target/timing-metrics/)
python3 scripts/north_tick_timing_report.py ci --profile baseline

# Baseline artifacts:
# - target/timing-metrics/gate_north_tick.jsonl
# - target/timing-metrics/north_tick_timing_baseline_summary.md
# - target/timing-metrics/north_tick_timing_baseline_failed_rows.jsonl

# Build Debian package (requires cargo-deb)
cargo install cargo-deb
cargo deb
# Creates target/debian/rotaryclub_*.deb
```

**Requirements:**
- Rust 1.85+ (edition 2024)
- Linux with ALSA support
- libasound2-dev (for building)

## Documentation

See [DESIGN.md](DESIGN.md) for system architecture, signal processing details, and theory of operation.

## License

Apache License 2.0. See [LICENSE](LICENSE) for details.

## References

- [Doppler Radio Direction Finding - Wikipedia](https://en.wikipedia.org/wiki/Doppler_radio_direction_finding)
- [Pseudo-Doppler RDF Systems](https://radiodirectionfinding.wordpress.com/)

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

- `bearing`: Smoothed azimuth estimate in degrees, wrapped to `[0, 360)`.
- `raw`: Instantaneous unsmoothed azimuth estimate in degrees, wrapped to `[0, 360)`.
- `confidence`: Combined quality score in `[0, 1]` from weighted normalized SNR, coherence, and signal strength.
- `snr_db`: Estimated in-band Doppler SNR (dB), computed from correlated signal power versus residual power.
- `coherence`: Phase-consistency metric in `[0, 1]` (correlation: sub-window circular phase variance; zero-crossing: crossing-interval regularity).
- `signal_strength`: Carrier-presence metric in `[0, 1]` (correlation-energy ratio for correlation method; observed/expected crossing density for zero-crossing method).

#### North Tracking Quality Measures

- `lock_quality`: DPLL-only lock score in `[0, 1]`, computed as weighted phase and frequency stability:
  `phase_weight * phase_score + frequency_weight * freq_score`.
- `phase_score`: `1 - (phase_error_std_dev / pi)`, clamped to `[0, 1]`.
- `freq_score`: `1 - (100 * freq_coeff_of_variation)`, clamped to `[0, 1]`, where `freq_coeff_of_variation = freq_std_dev / freq_mean`.
- `phase_error_variance`: Rolling variance (rad^2) of DPLL phase error; lower indicates tighter phase lock.
- Windowing: rolling statistics are computed over the last 128 detected ticks.
- Availability: in `--north-mode dpll` these fields are populated; in `--north-mode simple` they are not produced (`null`/empty in JSON/CSV).

### CLI Options

```
-m, --method <METHOD>            Bearing calculation method
                                 [correlation (default) | zero-crossing]

-n, --north-mode <NORTH_MODE>    North tick tracking mode
                                 [dpll (default) | simple]

-s, --swap-channels              Swap left/right channels

-r, --output-rate <OUTPUT_RATE>  Output rate in Hz [default: 10.0]

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

# Plot with default thresholds (confidence >= 0.5, coherence >= 0.5)
python3 scripts/plot_bearings.py recording.csv

# Custom thresholds
python3 scripts/plot_bearings.py recording.csv --min-confidence 0.7 --min-coherence 0.6
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
python3 scripts/check_north_tick_timing_metrics.py --profile baseline

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

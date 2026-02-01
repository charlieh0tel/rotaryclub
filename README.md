# Rotary Club - Pseudo Doppler Radio Direction Finding

Rust implementation of a pseudo doppler RDF system that calculates bearing angles (0-360°) from stereo audio.

## Installation

### From Debian Package

```bash
# Install the .deb package
sudo dpkg -i rotaryclub_0.1.0-1_amd64.deb

# Or if you need to install dependencies
sudo apt install -f
sudo dpkg -i rotaryclub_0.1.0-1_amd64.deb

# Run the installed binary
rotaryclub --help
```

### From Source

```bash
# Clone and build
git clone https://github.com/yourusername/rotaryclub.git
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

# Enable debug logging
rotaryclub -v

# Combine options
rotaryclub --method correlation --north-mode dpll --north-offset 45 -v
```

### Testing (from source)

```bash
# Test with WAV file
cargo run --example play_wav_file data/doppler-test-2023-04-10-ft-70d.wav

# Generate synthetic test signals
cargo run --example generate_test_wav
cargo run --example play_wav_file test_bearing_090.wav
```

## Usage

The program reads stereo audio:
- **Left channel**: FM radio audio (contains Doppler tone)
- **Right channel**: North timing reference pulses

Output:
```
Bearing: 137.5° (raw: 136.8°) confidence: 0.95
```

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

-v, --verbose                    Increase logging (-v=debug, -vv=trace)

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

## Configuration

Channel assignment and signal processing parameters are in `src/config.rs`. See DESIGN.md for details.

## Building

```bash
# Build release binary
cargo build --release

# Run tests
cargo test

# Build Debian package (requires cargo-deb)
cargo install cargo-deb
cargo deb
# Creates target/debian/rotaryclub_0.1.0-1_amd64.deb
```

**Requirements:**
- Rust 1.70+
- Linux with ALSA support
- libasound2-dev (for building)

## Documentation

See [DESIGN.md](DESIGN.md) for system architecture, signal processing details, and theory of operation.

## License

Apache License 2.0. See [LICENSE](LICENSE) for details.

## References

- [Doppler Radio Direction Finding - Wikipedia](https://en.wikipedia.org/wiki/Doppler_radio_direction_finding)
- [Pseudo-Doppler RDF Systems](https://radiodirectionfinding.wordpress.com/)

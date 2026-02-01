# Rotary Club - Pseudo Doppler Radio Direction Finding

A Rust implementation of a pseudo doppler radio direction finding (RDF) system that processes stereo audio to calculate bearing angles (0-360°).

## Overview

This program reads stereo audio samples where:
- One channel contains FM audio with a Doppler tone from an electronically-switched antenna array
- The other channel contains a "north tick" timing reference pulse

The system extracts the phase relationship between these signals to compute the bearing angle of the radio transmitter.

### How It Works

1. **Antenna Array**: Hardware switches between antenna elements in a circular pattern at ~500 Hz
2. **Doppler Effect**: This creates a ~500 Hz tone in the FM receiver audio
3. **Phase Detection**: Zero-crossings of the filtered Doppler tone are detected
4. **Bearing Calculation**: Time offset between north tick and zero-crossing reveals bearing angle

## Configuration

### Channel Assignment

By default:
- **Left channel**: FM audio / Doppler tone
- **Right channel**: North tick reference pulse

To swap channels, edit `src/config.rs` in the `AudioConfig::default()` implementation:

```rust
doppler_channel: ChannelRole::Left,      // or ChannelRole::Right
north_tick_channel: ChannelRole::Right,  // or ChannelRole::Left
```

### Signal Processing Parameters

Key parameters in `src/config.rs`:

```rust
// Audio settings
sample_rate: 48000 Hz
buffer_size: 1024 samples

// Doppler tone extraction
bandpass: 400-600 Hz  // Adjust for your rotation frequency
filter_order: 4

// North tick detection
highpass_cutoff: 2000 Hz  // Remove low-frequency noise
threshold: 0.3            // Peak detection threshold (0.0-1.0)

// Output
smoothing_window: 5     // Moving average window size
output_rate_hz: 10.0    // Display update rate
```

## Usage

### 1. Live Audio Capture (Hardware)

Capture from the default audio input device:

```bash
cargo run
```

The program will display bearing measurements in real-time:

```
Bearing: 137.5° (raw: 136.8°) confidence: 0.95
Bearing: 138.2° (raw: 137.1°) confidence: 0.96
```

### 2. Test with Pre-recorded WAV Files

This is the recommended way to develop and test without hardware.

#### Generate Test Files

Create synthetic RDF signals at various bearings:

```bash
cargo run --example generate_test_wav
```

This creates 8 test files:
- `test_bearing_000.wav` (0°)
- `test_bearing_045.wav` (45°)
- `test_bearing_090.wav` (90°)
- ... and so on

#### Play Back WAV File

Process a WAV file through the RDF system:

```bash
cargo run --example play_wav_file test_bearing_090.wav
```

Output:

```
=== WAV File RDF Test ===
File: test_bearing_090.wav

WAV file info:
  Sample rate: 48000 Hz
  Channels: 2
  Duration: 5.00s

Processing...

Time (s)   Bearing (°)     Raw Bearing (°) Confidence
-------------------------------------------------------
0.11       89.2            88.5            0.85
0.21       90.1            89.8            0.91
0.31       89.8            90.2            0.93
...

Statistics:
  Average bearing: 89.7°
  Std deviation: 1.2°
  Range: 4.5°
```

### 3. Other Examples

#### Audio Loopback Test

Verify audio capture is working:

```bash
cargo run --example audio_loopback
```

Shows RMS levels for each channel in real-time.

#### Filter Frequency Response

Test the bandpass and highpass filters:

```bash
cargo run --example filter_test
```

Displays attenuation at various frequencies.

#### Synthetic Signal Test

Test the complete pipeline with generated signals:

```bash
cargo run --example synthetic_rdf
```

Tests bearing calculation at 8 compass points.

## Building

### Requirements

- Rust 1.70+ (2021 edition)
- Linux (ALSA support)
- Audio input device (for live capture)

### Compile

```bash
cargo build --release
```

The optimized binary will be in `target/release/rotaryclub`.

### Run Tests

```bash
cargo test
```

## Calibration

When connecting to real hardware:

1. **Verify Audio**: Run `audio_loopback` to check RMS levels on both channels
2. **Adjust Threshold**: Set `north_tick.threshold` to ~70% of pulse amplitude
3. **Check Rotation**: Verify detected frequency matches antenna spec (~500 Hz)
4. **Tune Bandpass**: Adjust center frequency to match actual Doppler tone
5. **Set Hysteresis**: Increase if spurious zero-crossings detected
6. **Calibrate Offset**: Use known transmitter location to determine angular offset
7. **Optimize Smoothing**: Adjust window size for desired response/stability trade-off

### Enable Debug Logging

```bash
RUST_LOG=debug cargo run
```

Shows rotation frequency detection and other diagnostic info.

## Architecture

```
src/
├── main.rs              # Entry point, main processing loop
├── config.rs            # Configuration parameters
├── error.rs             # Error types
├── lib.rs               # Library exports
├── audio/
│   ├── buffer.rs        # Ring buffer for stereo samples
│   └── capture.rs       # CPAL audio input
├── signal_processing/
│   ├── filters.rs       # Butterworth bandpass/highpass IIR filters
│   ├── detector.rs      # Zero-crossing and peak detection
│   └── math.rs          # Phase/bearing conversion, smoothing
└── rdf/
    ├── north_ref.rs     # North tick detection and tracking
    └── bearing.rs       # Bearing calculation from Doppler phase
```

### Signal Processing Pipeline

```
Audio Input (Stereo)
       │
       ├─────────────────┬────────────────────┐
       │                 │                    │
       ▼                 ▼                    ▼
   Split by         Left/Doppler       Right/North Tick
   Config           Channel            Channel
                       │                    │
                       ▼                    ▼
                  Bandpass             Highpass
                  400-600 Hz           2000 Hz
                       │                    │
                       ▼                    ▼
                  Zero-Crossing        Peak Detection
                  Detection            (Rising Edge)
                       │                    │
                       ▼                    ▼
                  Phase Offset  ───────  North Reference
                  Calculation            Timestamp
                       │
                       ▼
                  Bearing Angle
                  (0-360°)
                       │
                       ▼
                  Moving Average
                  Smoothing
```

## Dependencies

- `cpal` - Cross-platform audio I/O
- `iir_filters` - Digital filter design (scipy-compatible)
- `hound` - WAV file reading/writing
- `crossbeam-channel` - Thread-safe message passing
- `audio_thread_priority` - Real-time thread scheduling

## Performance

- Latency: <100ms (21ms buffering + ~50μs DSP)
- CPU: <5% on modern systems
- Memory: <1MB resident

## Troubleshooting

### No bearing measurements

Check:
- North tick channel has visible pulses (use `audio_loopback`)
- Doppler channel has signal (use `audio_loopback`)
- Channel assignment is correct (doppler vs. north tick)
- Threshold is appropriate for pulse amplitude
- Sample rate matches audio device

### Inaccurate bearings

- Increase filter order for sharper bandpass
- Adjust smoothing window (larger = more stable, smaller = faster response)
- Check rotation frequency matches expected (~500 Hz)
- Calibrate angular offset using known transmitter

### Audio glitches

- Reduce buffer size for lower latency (may increase CPU)
- Run with real-time priority: `sudo <command>`
- Close other audio applications

## License

This project is open source. See LICENSE file for details.

## References

- [Doppler Radio Direction Finding - Wikipedia](https://en.wikipedia.org/wiki/Doppler_radio_direction_finding)
- [Pseudo-Doppler RDF Systems](https://radiodirectionfinding.wordpress.com/)
- Butterworth Filter Design (scipy.signal.butter compatible)

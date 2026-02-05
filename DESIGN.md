# Rotary Club - Design Document

## System Overview

Pseudo doppler radio direction finding system that calculates bearing
angles (0-360°) from stereo audio:
- **Left channel**: FM radio audio containing 1602 Hz Doppler tone
- **Right channel**: North reference timing pulses

## Theory of Operation

A circular array of 4 antennas switches electronically at 1602 Hz (156
µs per antenna). This creates a Doppler shift in the received
signal. The phase of the Doppler tone relative to the north reference
pulse reveals the transmitter bearing:

```
bearing = (phase_offset / 2π) × 360°
```

## Hardware Specifications

- **Antenna switching:** 1602 Hz (4 antennas × 156 µs = 624 µs period)
- **North pulse:** 20 µs wide (< 1 sample at 48 kHz!)
- **Sample rate:** 48 kHz stereo
- **Measured rotation:** 1601 Hz (99.9% accurate)

## Signal Processing

### North Tick Detection (Right Channel)
1. Highpass filter at 5 kHz (isolate 20µs pulse transients)
2. Peak detection with 0.15 threshold and 0.6ms minimum spacing
3. Rotation tracking (configurable):
   - **DPLL mode** (default): Digital PLL locks onto rotation frequency for smooth tracking
   - **Simple mode**: Exponential smoothing of period measurements

### Doppler Tone Extraction (Left Channel)
1. AGC (Automatic Gain Control) normalizes signal amplitude to 0.5 RMS
2. Bandpass filter 1500-1700 Hz (extract Doppler tone)
3. Phase extraction (configurable method):
   - **Correlation mode** (default): I/Q demodulation via correlation with sin/cos at rotation frequency. More accurate and robust to noise.
   - **Zero-crossing mode**: Zero-crossing detection with 0.01 hysteresis. Simpler but less accurate.
4. Calculate phase offset from north tick
5. Convert to bearing: `(phase_offset / 2π) × 360°`
6. Moving average smoothing (window size: 5)

## Configuration

Key tunable parameters in `config.rs`:

```rust
// AGC
target_rms: 0.5, attack_time_ms: 10.0, release_time_ms: 100.0

// Doppler processing
expected_freq: 1602.0, bandpass: 1500-1700 Hz, filter_order: 4
method: Correlation  // or ZeroCrossing

// North tick detection
highpass_cutoff: 5000.0 Hz, threshold: 0.15, min_interval_ms: 0.6
mode: Dpll  // or Simple

// Output
smoothing_window: 5, output_rate_hz: 10.0
```

Channel assignment is configurable via `ChannelRole` enum.

## Design Decisions

- **IIR filters**: Lower latency and fewer coefficients than
  FIR. Butterworth provides flat passband.
- **Bearing extraction methods**: Two options available:
  - **Correlation (default)**: I/Q demodulation, robust to noise
  - **Zero-crossing**: Sub-sample interpolation, lower CPU usage
- **DPLL for north tracking**: Locks onto rotation frequency, tolerates missed pulses,
  provides smooth frequency estimates
- **48 kHz sample rate**: Standard audio hardware
  support. Alternative: 96/192 kHz would better capture 20µs pulse but
  increases CPU load.
- **Single processing thread**: Simple architecture, audio callback
  uses lock-free channel.

## Performance

Test file (11.6s, moving radio source):
- **Rotation detection:** 1601.0 Hz (99.9% accurate)
- **Measurement rate:** 265 bearings/sec
- **Confidence:** 0.90-1.00
- **Latency:** <100ms
- **CPU usage:** <5%

## Known Limitations

1. **North pulse subsampling**: 20µs pulse < 1 sample at 48kHz. Relies
   on high-frequency content (mitigated by DPLL tracking).
2. **No multipath handling**: Reflections can distort phase measurements.

## Future Enhancements

1. **Correlation-based phase detection**: More robust to noise than zero-crossing
2. **Better confidence metrics**: SNR estimation, coherence measurement
3. **Calibration system**: Phase offsets, amplitude compensation,
   temperature drift

Note: Adaptive thresholding for north tick detection is not a priority since
the north reference is a controlled signal with predictable amplitude, and
the DPLL provides robust tracking even with occasional missed pulses.

## References

### Theory
- [Doppler Radio Direction Finding - Wikipedia](https://en.wikipedia.org/wiki/Doppler_radio_direction_finding)
- [Pseudo-Doppler RDF Systems](https://radiodirectionfinding.wordpress.com/)

## Signal Timing Diagram

```
Time →
═════════════════════════════════════════════════════════════

North Pulse (Right Channel):
    ↑20µs↑         ↑20µs↑         ↑20µs↑
    ▁▁█▁▁▁▁▁▁▁▁▁▁▁▁▁▁█▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁█▁
    ←──── 624µs ────→←──── 624µs ────→
         (1602 Hz)        (1602 Hz)

Doppler Tone (Left Channel):
    ╱╲    ╱╲    ╱╲    ╱╲    ╱╲    ╱╲
   ╱  ╲  ╱  ╲  ╱  ╲  ╱  ╲  ╱  ╲  ╱  ╲
  ╱    ╲╱    ╲╱    ╲╱    ╲╱    ╲╱    ╲
  ←─── ~0.6ms ───→ (1602 Hz sine wave)

Antenna Switching:
  Ant1  Ant2  Ant3  Ant4  Ant1  Ant2 ...
  ├156µs┤156µs┤156µs┤156µs┤
  ←────── 624µs ──────→ (complete rotation)

Phase Offset → Bearing:
  ┌─ North Tick
  │     ┌─ Zero Crossing
  │     │
  ▼     ▼
  ├─────┤ = phase offset
  └─────→ bearing = (offset/period) × 360°
```

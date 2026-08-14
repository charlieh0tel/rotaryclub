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
1. Highpass filter at 1 kHz (isolate 20µs pulse transients)
2. Peak detection with 0.15 threshold and 0.6ms minimum spacing
3. Sub-sample pulse estimation (configurable): the detected peak is an
   integer sample index, and one sample at 48 kHz is 12° of bearing, so the
   arrival time is estimated below the sample grid.
   - **Energy centroid** (default): first moment weighted by sample value squared
   - **Amplitude centroid**: first moment weighted by sample value
   - **Hard limiter**: the peak index alone, quantized to whole samples

   The two centroids differ only in how weight spreads across the pulse.
   Amplitude weighting gives the skirts more say and wins on a narrow pulse;
   energy weighting concentrates on the peak and wins on a wider one.
   Measured on the captures in `data/` at the 1 kHz cutoff, unclipped energy
   leads at 0.44 degrees against amplitude's 0.52. The figures once quoted
   here -- 0.69 against 0.89, reversing to 1.57 against 0.60 at 5 kHz -- were
   the clipped energy centroid, which is not what ships: clipping helps an odd
   weighting exponent and harms an even one, and the energy centroid stopped
   clipping. The reversal with cutoff went with it.
4. Rotation tracking (configurable):
   - **DPLL mode** (default): Digital PLL locks onto rotation frequency for smooth tracking
   - **Simple mode**: Exponential smoothing of period measurements

Both trackers consume and emit fractional sample times. The DPLL reports the
tick time its own oscillator predicts rather than the raw detection, which is
where most of the timing accuracy comes from: quantization error dithers as
the pulse walks across the sample interval, so averaging over the loop's
memory removes it. The oscillator also coasts through dropouts, emitting
ticks for rotations that produced no detection with lock quality falling as
the coasting budget is spent, and gates detections that disagree with the
tracked rotation by more than the tracker's own timing spread.

### Doppler Tone Extraction (Left Channel)
1. AGC (Automatic Gain Control) normalizes signal amplitude to 0.3 RMS
2. Bandpass filter 1350-1850 Hz (extract Doppler tone)
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
target_rms: 0.3, attack_time_ms: 10.0, release_time_ms: 100.0

// Doppler processing
expected_freq: 1602.56, bandpass: 1350-1850 Hz
method: Correlation  // or ZeroCrossing

// North tick detection
highpass_cutoff: 1000.0 Hz, threshold: 0.15, min_interval_ms: 0.6
mode: Dpll  // or Simple
estimator: EnergyCentroid  // or AmplitudeCentroid, HardLimiter
max_coast_ms: 1000.0, gate_sigma: 3.0
// DPLL tracking band: 1400-1650 Hz. min_interval_ms must stay shorter
// than the period at frequency_max_hz; conflicting values are a config
// error (0.6 ms supports up to ~1666 Hz).

// Output
smoothing_window: 5, output_rate_hz: 10.0
```

Channel assignment is configurable via `ChannelRole` enum.

## Design Decisions

- **FIR filters**: Linear phase (constant group delay) so tick timing and
  Doppler phase survive filtering; the known delay is compensated
  explicitly in the north trackers.
- **Bearing extraction methods**: Two options available:
  - **Correlation (default)**: I/Q demodulation, robust to noise
  - **Zero-crossing**: Sub-sample interpolation, lower CPU usage
- **DPLL for north tracking**: Locks onto rotation frequency, tolerates missed pulses,
  provides smooth frequency estimates
- **The simple tracker stays simple**: it times each pulse independently and
  averages the intervals, with no oscillator to carry the rotation between
  them. That costs it real robustness -- under combined hum, clipping and
  baseline drift it gives up about half the pulses where the loop gives up
  none, because a disturbance that costs it detections also degrades the
  spacing guard the survivors are judged against. Closing that gap would mean
  rebuilding what the DPLL already is. It is kept as a fallback and a
  comparison point, and `test_north_tick_detection_under_hum_clipping_and_drift`
  records where it stands.
- **Pulse estimator separate from the loop**: they solve different problems.
  The estimator decides where one pulse arrived; the loop decides what the
  rotation is doing. Against a tight loop the estimator choice is worth
  little — measured 0.05° with the centroid against 0.10° with the hard
  limiter — but the centroid costs almost nothing and is what keeps a
  commensurate rotation rate from becoming a fixed bearing offset.
- **Delay compensation is per-estimator**: each estimator reports a different
  point on the same filtered pulse, so its delay is referenced to the point
  it would report for an impulse arriving exactly on a sample. Emitted tick
  times, and north-offset calibrations made against them, therefore do not
  move when the estimator changes.
- **48 kHz sample rate**: Standard audio hardware
  support. Alternative: 96/192 kHz would better capture 20µs pulse but
  increases CPU load. WAV input at other rates is supported: the file's
  header rate is propagated into the DSP configuration.
- **Single processing thread**: Simple architecture. The real-time audio
  callback never blocks: it try_sends into a bounded channel and drops
  (with accounting) if the consumer lags; stream errors propagate
  through the same channel.

## Performance

Test file (11.6s, moving radio source):
- **Rotation detection:** 1601.0 Hz (99.9% accurate)
- **Measurement rate:** 265 bearings/sec
- **Confidence:** signal-dependent; see `bearing_uncertainty_deg`. A clean
  synthetic signal reads about 0.97 and a bearing forty degrees out reads
  0.02. The 0.90-1.00 once quoted here described the weighted-sum score that
  floored near 0.59 whatever the signal did, and is not comparable.
- **Latency:** <100ms
- **CPU usage:** <5%

## Known Limitations

1. **North pulse subsampling**: 20µs pulse < 1 sample at 48kHz. Relies on
   high-frequency content, sub-sample estimation, and DPLL averaging.
2. **Commensurate rotation rates**: the averaging above works because the
   pulse arrives at a different point between samples on each rotation. If
   the rotation rate were commensurate with the sample clock — 1600.000 Hz
   at 48 kHz, exactly 30 samples per rotation — it would land in the same
   place every time, and a quantized estimator's error would become a fixed
   offset of up to half a sample that no amount of loop averaging removes.
   Measured: 3.6° with the hard limiter, 0.6° with the centroid, and zero
   jitter in both cases, so no confidence metric would reveal it. The 624 µs
   hardware period gives 29.952 samples per rotation and avoids this.
3. **Holdover accuracy**: coasting integrates the rate estimate, so error
   accumulates every rotation it has to predict — several samples across a
   few hundred milliseconds if the loop is still settling.
4. **No multipath handling**: Reflections can distort phase measurements.

## Future Enhancements

1. **Calibration system**: Phase offsets, amplitude compensation,
   temperature drift

(Correlation-based phase detection, previously listed here, is implemented and
is the default bearing method. The coherence metric that was listed alongside
it has been removed: it was normalised against the circular variance of a
whole turn, so it read 0.99 on a bearing that was tens of degrees wrong.
Confidence now derives from an estimated bearing uncertainty in degrees.)

Note: adaptive thresholding for north tick detection is not a priority, and
this is measured rather than assumed. `examples/north_threshold_sweep` sweeps
pulse amplitude and channel noise against the detection threshold, in both
tracking modes.

Two things decide the threshold and they pull opposite ways.

The amplitude cliff moves with the threshold, at roughly amplitude = 1.6 x
threshold. The detector threshold is absolute and the filtered pulse peak
scales with amplitude, so this is what it has to do. In DPLL mode, with no
noise, detection is total down to these amplitudes and collapses below them:

  threshold  0.10  0.15  0.20  0.25  0.30  0.40
  cliff      0.15  0.25  0.32  0.42  0.50  0.60

Against the 0.8 the configuration expects, the shipped 0.15 therefore has a
factor of 3.2 in hand on receiver level.

Noise margin runs the other way. At the expected amplitude, detection and
false positive rate against true RMS noise on the north channel:

  noise      0.00       0.05       0.10       0.20       0.30       0.40
  0.15  1.00/0.00  1.00/0.00  0.99/0.00  0.92/0.02  0.67/0.33  0.45/0.55
  0.25  1.00/0.00  1.00/0.00  0.99/0.00  0.95/0.00  0.79/0.18  0.57/0.41

So a higher threshold buys nothing until the channel carries 0.2 RMS and
becomes worth having at 0.3, by which point neither value is usable. Raising
0.15 to 0.25 would trade a factor of 3.2 on level for a factor of 1.9, to gain
detection in a regime where the false positive rate has already reached 0.18.
0.25 and 0.20 both also fail `test_north_tick_detection_under_hum_clipping_and_drift`.
0.15 stays.

An earlier version of this section reported a wide plateau from 0.10 to 0.40
and a shared amplitude cliff at 0.3 regardless of threshold. Both were wrong.
The sweep behind them ran in Simple mode, which is not what ships, and scaled
its noise by a third of the labelled figure, so every noise column was worth
three times less than it said.

What the sweep does not excuse is silence: below the cliff detection goes to
zero rather than degrading, and nothing reports it.

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

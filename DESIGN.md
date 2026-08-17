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
2. Peak detection at a fraction of the expected pulse height, 0.6ms minimum
   spacing. The fraction follows the gain control rather than the tracking
   mode: 0.323 where the north AGC runs, 0.19361 where it does not (0.25 and
   0.15 of full scale at the default pulse and filter). See the threshold
   section below for why they differ.
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
expected_freq: 1602.56, bandpass: 1350-1850 Hz (1023 taps; the filter's
// noise-equivalent bandwidth is measured from the taps and feeds the
// uncertainty's look count, so an unrealizable design costs accuracy but
// cannot miscalibrate the stated figure)
method: Correlation  // or ZeroCrossing

// North tick detection
highpass_cutoff: 1000.0 Hz, min_interval_ms: 0.6
threshold_fraction: None  // resolved per gain control: 0.323 with the north
                          // AGC, 0.19361 without; Some(x) overrides both
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

  What the loop is worth, measured with the doppler channel quiet so the north
  channel is the limiting term, over eight noise draws: 2.08 degrees of bearing
  against the simple tracker's 2.98 at 0.01 RMS of north noise, 2.07 against
  4.43 at 0.05, and 3.26 against 84.65 at 0.2, where the simple tracker also
  gives up a third of its bearings. The advantage is small when the channel is
  clean and decisive when it is not.

  It does not show at all in the `noisy_jittered` pipeline scenario, which is
  where it used to be quoted from. That scenario carries doppler noise at 0.8,
  the middle of what the recordings measure, and at that level the doppler
  channel decides the bearing almost entirely: both trackers read about 22
  degrees. A scenario has to be limited by the thing under test before it can
  measure it.
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
- **Measurement rate:** 1602 bearings/sec computed, one per north tick;
  18,621 over the 11.62 s file. What a consumer sees is `--output-rate`,
  10 Hz by default, since the rest are averaged into it. The 265 once quoted
  here matched neither number and is not a rate this pipeline produces.
- **Confidence:** signal-dependent; see `bearing_uncertainty_deg`. A clean
  synthetic signal reads near 1.0 and a bearing forty degrees out reads
  0.02. Earlier versions quoted 0.97, capped by a reference term charged at
  raw detection scatter -- about twenty-six times the emitted tick's error --
  and before that 0.90-1.00 from a weighted-sum score that floored near 0.59
  whatever the signal did; neither is comparable.
- **Latency:** <100ms — unverified. No harness measures end-to-end latency;
  the gates measure per-sample processing time, which is a different thing.
- **CPU usage:** <5% — unverified, and load-dependent enough that the timing
  columns in the gates fail outright on a loaded machine.

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
this is measured rather than assumed. `sweep_threshold` sweeps
pulse amplitude and channel noise against the detection threshold, in both
tracking modes.

Two things decide the threshold and they pull opposite ways.

The amplitude cliff moves with the threshold, at roughly amplitude = 1.6 x
threshold. The filtered pulse peak scales with the amplitude the receiver
delivers, so this is what it has to do.

The threshold is now expressed as a fraction of the pulse height the
configuration expects, rather than as an absolute level, which is why the
figures below read in absolutes: they were measured before the change and the
change reproduces them exactly. The shipped 0.19361 is 0.15 of full scale
against the default 0.8 pulse through the default filter. What the fraction
fixed is a bookkeeping failure rather than anything in this table -- an
absolute threshold met a signal that scaled with `gain_db` while itself
staying put, so attenuation silently defeated detection. Without gain control,
with no noise, detection is total down to these amplitudes and collapses below
them:

  threshold  0.10  0.15  0.20  0.25  0.30  0.40
  cliff      0.15  0.25  0.32  0.42  0.50  0.60

Against the 0.8 the configuration expects, the shipped 0.15 therefore has a
factor of 3.2 in hand on receiver level.

Noise margin runs the other way, and is the one place the two tables come
from different trackers: the cliff above is the tracker without gain control,
where the cliff exists at all, and the noise figures below are the DPLL, whose
coasting is what carries it through a missed pulse. At the expected amplitude,
detection and false positive rate against true RMS noise on the north channel:

  noise      0.00       0.05       0.10       0.20       0.30       0.40
  0.15  1.00/0.00  1.00/0.00  0.99/0.00  0.92/0.02  0.67/0.33  0.45/0.55
  0.25  1.00/0.00  1.00/0.00  0.99/0.00  0.95/0.00  0.79/0.18  0.57/0.41

So a higher threshold buys nothing until the channel carries 0.2 RMS and
becomes worth having at 0.3, by which point neither value is usable. Raising
0.15 to 0.25 would trade a factor of 3.2 on level for a factor of 1.9, to gain
detection in a regime where the false positive rate has already reached 0.18.
0.25 and 0.20 both also fail `test_north_tick_detection_under_hum_clipping_and_drift`.
0.15 stays.

The north AGC changes half of this and not the other half. Where it runs, it
normalises the level the threshold meets, so the amplitude cliff stops being a
cost: at a threshold of 0.25 detection then holds at 0.92 or better down to a
pulse of 0.15, against zero below 0.42 without it. The noise margin a higher
threshold buys is unaffected and still real -- 0.95 against 0.90 at 0.2 RMS,
0.75 against 0.67 at 0.3.

The threshold now follows the gain control, which resolves that. A tracker
whose AGC holds the pulse at the expected height can afford a high threshold,
because what a high threshold costs is level margin and the AGC is what
supplies it; a tracker that takes the level it is given cannot. So the default
is 0.323 of the expected pulse where the AGC runs and 0.19361 where it does
not -- an absolute 0.25 and 0.15 at the default pulse and filter -- and a DPLL
with its AGC switched off gets the conservative value, because without gain
control it has the same exposure the simple tracker does.

The split is what the measurement asks for. At 0.323 the simple tracker fails
detection under combined hum, clipping and baseline drift, 0.37 against a floor
of 0.45, while the loop passes every disturbance in that test. Through the
system pipeline the change touches DPLL rows only and improves the noisy ones:
detection on `low_snr_dc` goes from 0.974 to 0.988 and its tick error from
0.355 to 0.351 samples, with the largest movement the other way being a bearing
error of 40.73 degrees becoming 40.79.

Re-measured as a fraction, over sixteen independent noise draws, the shipped
value holds. The question was whether the derived 0.19361 could become a round
0.20 now that the knob is dimensionless. It cannot, for the same reason the
absolute could not be raised: in DPLL mode the change is invisible, because the
AGC normalises the level and the amplitude cliff hardly moves, but the simple
tracker has no gain control and its cliff is steep enough that three percent
crosses it -- at a pulse of 0.23 its detection falls from 0.92 to 0.47. Those
cells carry no noise, so they are exact rather than a draw. Rounding the other
way, to 0.1875, buys a little level margin and costs a little noise margin
(0.86 detection at 0.2 RMS against 0.87), which is not an improvement either.

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

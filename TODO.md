# TODO

- File rotation for --dump-audio to bound disk usage on very long recordings
  (memory growth is fixed: dumps stream to disk incrementally)
- Add criterion benchmarks for DSP pipeline (FIR filters, AGC, I/Q correlation)

## Bearing Confidence

- [x] Decide what confidence means, and make it that. Done.
      `ConfidenceMetrics::bearing_uncertainty_deg` estimates a one-sigma
      bearing uncertainty in degrees from the spread of the phase estimates
      and the timing scatter of the reference, and confidence is now
      1 / (1 + (sigma / half) ^ 2) against a configured half-confidence
      point, six degrees by default. Signal strength became a validity gate
      for zero crossing and left the score.
      What it does not do, and cannot: see a displacement every estimate
      shares. It is precision, not accuracy. `bearing_uncertainty_test`
      asserts what it does claim -- growth as the signal degrades, and never
      reading below the scatter it describes.
      Two reductions that are correct in theory were measured and rejected,
      both of which make the figure understate: dividing the reference term
      by the loop averaging (755 ticks at the shipped bandwidth, and the
      reported tick really is 27 times better than one detection, but as the
      signal degrades the tick's error becomes a displacement the loop
      follows rather than scatter it averages away), and dividing the phase
      spread by the root of the estimate count (they share a tick, a filter
      state and an AGC gain, so they are not independent).

- [x] The zero-crossing bearing method has a systematic bias that grows with
      noise. Investigated: there is no such bias. The measurement that showed
      one was made with noise that was half DC offset.
      Every synthetic noise source in the repo built a sample as
      `(x >> 33) as u32` over `u32::MAX`, which is 31 bits divided by a
      32-bit maximum, so the range was [-1, 0) rather than [-1, 1). A
      residual DC through the doppler bandpass shifts a sinusoid's zero
      crossings, which is why the effect scaled with the noise setting, held
      steady across seeds, changed sign with the filter width, and spared the
      correlation method -- correlating against sin and cos at the tone
      frequency rejects DC.
      With the generator fixed the two methods measure within a few
      hundredths of a degree of each other at every level: 0.44 against 0.47
      at noise 0.3, 10.31 against 10.34 at noise 1.0.
      Worth remembering how it was caught. The detector, the AGC, the
      passband centre, the run length and the crossing-selection latch were
      all cleared first, and a plain sign-change scan reproduced the biased
      answer exactly -- so the bias was in the waveform, not the code reading
      it. What settled it was reimplementing the same measurement in numpy
      and getting no bias, then asking what differed about the input.

## North Tick Tracking

- [x] A slow AGC on the north channel. Done, off by default:
      `north_tick.agc.enabled`. Peak-referenced rather than RMS, because the
      pulse is a 1.2-sample event every 30 and an RMS reference would both
      demand an amplitude of 1.5 and track the rotation rate.
      One thing the design as written here got wrong. Gating adaptation purely
      on detections is correct once pulses are arriving, but a receiver quiet
      enough to need the gain is quiet enough that nothing is detected, so the
      first version could never rescue anything: it measured 0.000 detection
      with the AGC on and off alike at a pulse amplitude of 0.15. The way out
      is that a pulse train and a noise floor do not look alike -- a peak
      twenty-five times the mean absolute value against about four -- so
      before the first detection the gain adapts to the buffer peak, and only
      when the buffer looks like pulses by that measure.
      Enabling it leaves the tick count on all three captures in `data/`
      exactly unchanged, and it recovers a pulse of 0.15 that the fixed-gain
      detector misses entirely.

- [x] The north detection threshold has less margin than the sweep that chose
      it showed. Re-measured twice, and the answer changed in between.
      The first re-measurement, before the north AGC existed, found that the
      amplitude at which detection collapses tracks the threshold at about 1.6
      times it, so 0.15 detects down to a pulse of 0.25 and 0.25 only to 0.42.
      Against the 0.8 expected that is a factor of 3.2 on receiver level
      against 1.9, and it was the reason to leave the threshold alone.
      The AGC removes that cost, because it normalises the level the threshold
      meets. With it running, detection at a threshold of 0.25 holds at 0.92
      or better down to a pulse of 0.15, where before it was zero below 0.42:

        thresh\amp  1.00  0.80  0.60  0.50  0.42  0.35  0.30  0.25  0.20  0.15
        0.15        1.00  1.00  1.00  1.00  1.00  1.00  1.00  1.00  0.97  0.99
        0.25        1.00  1.00  1.00  1.00  1.00  0.98  0.92  0.99  0.99  0.99

      and the noise margin that a higher threshold buys is real:

        thresh\noise    0.00   0.05   0.10   0.20   0.30   0.40
        0.15            1.00   1.00   0.98   0.90   0.67   0.45
        0.25            1.00   1.00   0.99   0.95   0.75   0.57

      0.15 stays anyway, because the AGC is DPLL-only and the threshold is
      not. In simple mode the cliff is exactly where it was, and both 0.20 and
      0.25 fail `test_north_tick_detection_under_hum_clipping_and_drift`,
      which is a simple-mode floor. A default that suits one tracker and
      quietly costs the other its level margin is worse than one that is
      merely conservative for the first.
      For a DPLL-only deployment, 0.25 is available and is worth about a
      quarter of the detection rate at 0.3 RMS of channel noise. Whether the
      threshold should follow the tracking mode, or be expressed as a fraction
      of `expected_pulse_amplitude` now that the AGC makes that meaningful, is
      the question this leaves behind.

- [ ] Price what the highpass is for, with a capture that bleeds audio into
      the north channel. That is the only argument for filtering high, and no
      capture in `data/` exhibits it, so nothing measured so far can say
      whether 1 kHz is safe generally or only on these two radios. Needs
      hardware.

- [x] The coasting budget punishes a phase offset as though it were a rate
      error. Investigated and wrong: the budget is right, and the reasoning
      that said otherwise had the causality backwards.
      A predicted tick does advance from the last measured tick by `period`
      and never uses the oscillator's phase, so a standing phase offset looks
      like it should cost nothing. But for a second-order loop a standing
      offset is precisely the observable that says the integrator has not
      converged, which is to say the rate is still slightly wrong. At 0.5 Hz
      the rate is off by 0.0004 samples per rotation -- invisible over the
      four rotations the budget allows, and worth three samples, 35 degrees of
      bearing, over five seconds. Replacing the term with a test on the drift
      of the mean phase error let those loops coast freely and put exactly
      that error into the holdover.
      The budget is conservative rather than correct: at 0.5 Hz it prices the
      per-rotation error at 0.116 samples against a true 0.0004, so it is
      318 times short. That costs holdover only at bandwidths below 1 Hz,
      which the sweep disqualified on acquisition anyway. If it is ever worth
      tightening, the drift signal is real but sits at the noise floor of a
      128-tick window; it would need a longer window to be usable, which
      trades against how fast the budget can react to a genuine rate change.
      `test_coasting_stops_before_its_error_escapes_the_bound` now pins the
      bound itself, and fails against the change described above.

- [x] Extend the comparison to N configurations, not two. `src/bin/config_sweep.rs`
      takes any number of `--axis key=v1,v2,...` and runs the cross product,
      over configuration keys and stimulus alike. `--list-axes` lists both.
      The stimulus axes name the physical quantity rather than a knob:
      `doppler_noise` is passband noise power against the tone, which the
      recordings measure at 0.199, 0.793 and 6.579, and `north_noise` is an
      RMS against their floor of about 0.0006. That naming is the point of the
      exercise. One signal is built per distinct stimulus and shared across
      the configuration axes, so two configurations are never compared against
      different noise.

- [x] Add a mode that runs two configurations over the same signal and reports
      the difference. `src/bin/config_compare.rs`: both sides start from the
      shipped defaults and take dotted `key=value` overrides, so a comparison
      records exactly what it changed. `--list-keys` lists what it accepts.

## Bearing Calculator

- [x] Chase the residual bearing error that remains with the timing trim at
      zero. Closed: it was mostly the measurement, not the pipeline.
      Two artifacts were stacked on top of each other. The perf harness placed
      each north pulse at `round(k * period)`, up to half a sample from where
      the rotation crosses north, which is six degrees of bearing injected per
      rotation before any code ran; and both probes ran for half a second
      against a loop whose bandwidth was 1 Hz, so they spent most of their
      length acquiring and reported the transient as steady-state error.
      With pulses at their true epochs the clean scenarios fell from about ten
      degrees of mean bearing error to under two. Sweeping run length takes
      the residual from -1.07 degrees at half a second to -0.28 at two and
      about -0.2 from five seconds on, and the tracker's mean tick error over
      ten seconds falls from -0.017 samples in the first third to -0.000 in
      the last.
      The earlier split of the residual between the north tracker and the
      bearing path does not survive: it was scored against the rounded pulses,
      and there is no separate bearing-path bias to find. What remains at five
      to ten seconds is about 0.2 degrees, at the correlation floor.

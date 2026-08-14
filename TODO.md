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

- [ ] A slow AGC on the north channel. Pulse amplitude varies 3.7x across the
      two radios in `data/`, 0.21 to 0.78, against a configured
      `expected_pulse_amplitude` of 0.8, and the existing AGC does not reach
      this channel: it runs only on the doppler buffer, while the north path
      applies a static `gain_db` that defaults to unity.
      It must be referenced to the pulse peak, not to RMS. The pulse is a
      1.2-sample event every 30, a duty cycle of 0.04, so the RMS of a clean
      pulse train is 0.2 times the pulse amplitude and the doppler AGC's
      target of 0.3 implies an amplitude of 1.5. Pointed at the three
      captures it asks for 1.9x to 7.0x and drives the pulses to 1.06, 1.50
      and 1.50 -- clipping on all three. An RMS reference also tracks the
      rotation rate, since the duty cycle does.
      Two things it has to get right: adaptation gated on detections, because
      a peak tracker with no pulses ramps gain into the noise floor and
      manufactures its own, and `QuietChannelWatch` already knows when
      nothing is arriving; and a time constant of seconds, which the hardware
      allows because the tick does not change once it is running.
      What it buys beyond the level itself: normalising the peak to
      `expected_pulse_amplitude` turns the threshold into a genuine fraction
      of the pulse, which is what it already reads as. The amplitude cliff
      sits at about 1.6 times the threshold and only matters because the
      incoming level is unknown; normalise it and that becomes guaranteed
      rather than assumed. Wants a fixed-gain escape hatch for bench work.

- [ ] The north detection threshold has less margin than the sweep that chose
      it showed. That sweep ran with noise carrying a seventh of the in-band
      energy it claimed, so its noise 0.10 column is about noise 0.04 in real
      terms. Re-run with the generator fixed, at the shipped threshold of 0.15
      and full pulse amplitude:

        north noise rms   0.05   0.10   0.20   0.40
        detection         1.00   1.00   0.50   0.49

      and sweeping the threshold at north noise 0.20, where 0.15 fails:

        threshold         0.05   0.10   0.15   0.25   0.40   0.60
        detection         0.37   0.50   0.50   1.00   1.00   0.23

      Raising the threshold *improves* detection, which says the failure is
      early triggering on noise followed by the dead time masking the real
      pulse -- detection pinned at almost exactly one half is every other
      pulse lost, which is what a dead time slightly longer than a rotation
      produces when it starts early.
      Not a reason to move the default on its own. The real captures detect
      18628 of 18630 and 121073 of 121074, and the pipeline scenario carries
      north noise near rms 0.1, where 0.15 still takes everything. But the
      margin above the shipped value is about one doubling of channel noise,
      not the comfortable band the original sweep implied, and 0.25 holds a
      factor of four further out at no measured cost in the harness.
      Worth settling with a capture from a genuinely noisy channel, which is
      the same thing the highpass question needs.

- [x] Sweep `highpass_cutoff` again, with the current estimator and a finer
      grid. Done, on all three captures, 250 Hz to 3 kHz in fine steps at the
      shipped 63 taps. The shipped 1 kHz is already at the optimum, which is a
      broad basin from 1000 to 1250 Hz. Fit residual for the shipped energy
      centroid, in degrees:

        cutoff   none   500   750  1000  1250  1500  2000  3000
        ft-70d  3.576 3.506 3.478 3.472 3.472 3.484 3.503 3.512
        wouxun1 0.460 0.535 0.494 0.441 0.438 0.453 0.536 0.562
        wouxun3 0.358 0.422 0.378 0.337 0.335 0.356 0.451 0.507

      1250 Hz beats 1000 by 0.002 to 0.003 degrees, which is not a reason to
      move. The hint that the filter still costs timing at 1 kHz is refuted:
      filtering at 1 kHz beats not filtering at all on every capture.
      Filter length was swept with it. 63 taps is the right choice: 31 is
      worse everywhere, and 127 helps ft-70d by 0.1 degrees while hurting the
      wouxun captures by 0.01.
      The estimator and cutoff do interact, as expected. The amplitude
      centroid at 2 kHz beats the energy centroid at 1250 on both wouxun
      captures, 0.367 against 0.438 and 0.290 against 0.335, and ties on
      ft-70d. That pairing is worth knowing but not worth taking: it is half a
      percent of a sample, it is measured on three captures from two radios,
      and the amplitude centroid is markedly worse than the energy centroid
      everywhere below 1500 Hz, so the pair is sharp where the current one is
      flat.
      Note what this metric is. Residuals are scored against a fitted constant
      rate, so it measures jitter and absorbs any constant delay, and it
      includes whatever the radio itself contributes.

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

- [ ] Extend the comparison to N configurations, not two. `config_compare`
      takes an A and a B, but most real questions are grids: the threshold
      needed threshold against amplitude against noise against tracking mode,
      and the loop bandwidth needed bandwidth against noise. Both were
      hand-rolled into one-off examples, and that is exactly how the same
      mislabelled noise axis came to live in two of them independently, and
      how one of them came to sweep in a tracking mode that does not ship.
      A framework that takes a set of axes, runs the cross product against one
      generator, and prints the table would have made both errors visible at
      the point they were written, because the generator and the axis labels
      would be in one place instead of copied.

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

# TODO

- File rotation for --dump-audio to bound disk usage on very long recordings
  (memory growth is fixed: dumps stream to disk incrementally)
- Add criterion benchmarks for DSP pipeline (FIR filters, AGC, I/Q correlation)

## Bearing Confidence

- [ ] Decide what confidence means, and make it that. Agreed direction: a
      calibrated bearing uncertainty in degrees, testable against the harness
      ground truth -- does a stated +/- 1.2 degrees contain the truth as often
      as it claims -- with any 0..1 score derived from it. `signal_strength`
      to become a validity gate rather than a weighted term: it answers "did
      we get data", not "is this good", and sits at 0.995 in heavy noise
      because the detector keeps finding crossings in garbage.
      Nothing outside this repo reads `confidence`, so the scale is free to
      change. Inside it: text, JSON and CSV output carry it, the GUI uses it
      for needle brightness, and `TRAIL_CONFIDENCE_THRESHOLD` drops trail
      points below 0.5 -- a threshold that never trips today, because the two
      near-constant terms floor the score at about 0.59. Give the score real
      range and that cutoff starts acting, so it needs retuning with it.

      Three things measured while fixing correlation's granularity, which the
      rework has to account for:

      Coherence is still saturated. It moves the right way now and it is
      monotone, but the normalization is against the circular variance of a
      full turn, so the useful range is squeezed into the top fraction of a
      percent:

        noise     0.0    0.3    0.6    1.0    1.5    2.0
        bearing  0.16   0.25   0.43   0.72   6.54  39.82   degrees
        coher. 1.0000 1.0000 0.9999 0.9996 0.9991 0.9984

      Coherence measures precision, not accuracy, and cannot do otherwise.
      It scores how well the Doppler phase agrees with itself *relative to the
      north tick*. A mistimed tick shifts every rotation equally and leaves
      coherence untouched. At noise 2.0 the tick is 0.74 samples out, which is
      8.9 degrees, and the bearing error is largely bias: the circular mean of
      the error is 23 degrees against a mean absolute of 39.8. An uncertainty
      built only on Doppler phase scatter will be confidently wrong exactly
      when the reference is wrong.

      So the reference has to enter the score. `NorthTick::lock_quality`
      already exists, is already computed, and is not used by the confidence
      at all. It is the obvious candidate for the term that covers what
      coherence structurally cannot see.

## North Tick Tracking

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

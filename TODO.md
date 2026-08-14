# TODO

- File rotation for --dump-audio to bound disk usage on very long recordings
  (memory growth is fixed: dumps stream to disk incrementally)
- Add criterion benchmarks for DSP pipeline (FIR filters, AGC, I/Q correlation)
- Improve zero-crossing coherence metric (sub-window phase variance like correlation method)

## North Tick Tracking

- [ ] Sweep `highpass_cutoff` again, with the current estimator and a finer
      grid. The sweep that chose 1 kHz predates both the unclipped energy
      centroid and the estimator-window fix, and sampled nothing between 500
      and 2000 Hz. There is a hint the filter still costs timing even at
      1 kHz: fitting a band-limited impulse to the *unfiltered* north channel
      gives 0.37 degrees against 0.78 for a matched filter on the highpassed
      signal. One `north_hpf_sweep` run answers it.
      Moving back to 5 kHz has already been tried and measured worse on every
      axis: timing 0.52 degrees against 0.44, simple-tracker jitter doubling
      to 0.20 samples, coast coverage falling from 700 rotations to 568, and
      the `low_snr_dc` false-positive rate rising past its gate limit. 3 kHz
      measures the lowest false-positive rate of any tried, 0.046.
      Whatever moves, move both: the cutoff sets how wide the filter leaves
      the pulse, which is what decides the weighting, so the cutoff and the
      estimator are one choice rather than two.

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

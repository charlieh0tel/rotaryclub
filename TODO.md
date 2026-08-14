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

- [ ] The coasting budget punishes a phase offset as though it were a rate
      error. `coast_budget_samples` derives a per-rotation error from the
      larger of the frequency scatter and the mean phase error above its noise
      floor. But a predicted tick advances from the last measured tick by
      `period`; it never uses the oscillator's phase. So a loop sitting at a
      constant phase offset predicts perfectly well, and the budget cuts it off
      anyway. Measured with `coast_budget_probe`: at 0.5 Hz the mean phase
      error is -0.0276 rad, which prices holdover at four rotations, while the
      coasted ticks it does emit are accurate to 0.001 samples. At 1 Hz the
      offset falls under its noise floor, the term vanishes, and holdover runs
      to the cap.
      What the term is trying to catch -- a rate that is steadily wrong with
      little scatter -- shows up as a mean phase error that *drifts*, not one
      that merely sits away from zero. Its trend is the quantity to test.
      This is why bandwidths below 1 Hz are currently unusable, and they are
      exactly the ones with the best steady-state timing.

- [ ] Add a mode that runs two configurations over the same signal and reports
      the difference. Every comparison so far -- estimator, highpass cutoff,
      loop bandwidth, phase correction on and off -- has meant editing a
      default, rebuilding, and diffing two report files by hand, which is slow
      and has twice produced numbers from a build that was one iteration stale.

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

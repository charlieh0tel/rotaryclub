# TODO

- File rotation for --dump-audio to bound disk usage on very long recordings
  (memory growth is fixed: dumps stream to disk incrementally)
- Add criterion benchmarks for DSP pipeline (FIR filters, AGC, I/Q correlation)
- Improve zero-crossing coherence metric (sub-window phase variance like correlation method)

## North Tick Tracking

- [ ] Revisit `highpass_cutoff` when there is a capture that bleeds audio
      into the north channel. It sits at 1 kHz. Moving it back to 5 kHz was
      tried and measured worse on every axis available: per-tick timing 0.52
      degrees against 0.44, the simple tracker's jitter doubling from 0.10 to
      0.20 samples, coast coverage falling from 700 rotations to 568 because
      the budget is earned from how well the rate is known, and the
      `low_snr_dc` false-positive rate rising from 0.048 to 0.051, which trips
      the performance gate. Intermediate cutoffs pass: 3 kHz has the lowest
      false-positive rate measured, 0.046, at 0.56 degrees of timing.
      The one thing none of this prices is audio bleed, which is the only
      argument for filtering high, and no capture exhibits it.

- [ ] Measure loop bandwidth against both acquisition and holdover, then pick
      one. At 2 Hz steady-state error is identical to 1 Hz -- 0.28 to 0.31
      degrees either way -- and acquisition halves from 0.84 s. But the
      coasting budget is derived from the scatter of the frequency estimate,
      and a wider loop makes that scatter worse, so holdover may shorten. The
      two now pull in opposite directions and the tradeoff has never been
      measured.

- [ ] Measure the estimator against a gentler highpass, or none. The cutoff
      sweep that chose 1 kHz predates both the unclipped energy centroid and
      the fix to the estimator's window, so its grid is stale, and it sampled
      nothing between 500 and 2000 Hz. There is a hint the filter still costs
      timing at 1 kHz: fitting a band-limited impulse to the *unfiltered*
      north channel gives 0.37 degrees against 0.78 for a matched filter on
      the highpassed signal. One `north_hpf_sweep` run with the unclipped
      column answers it.
- [ ] Price what the highpass is for. Every capture in `data/` comes from two
      radios and none of them bleeds audio into the north channel, which is
      the only argument for a high cutoff -- so the sweep that lowered it to
      1 kHz could not measure the thing the setting exists to prevent. A
      capture that does bleed would tell us whether 1 kHz is safe generally
      or only here.
- [ ] Treat the cutoff and the estimator as one choice, not two. The cutoff
      decides how wide the filter leaves the pulse, which decides which
      weighting wins: at 5 kHz amplitude weighting led 0.60 degrees to 1.57,
      at 1 kHz energy weighting leads. Any future change to either should
      re-measure both.

## Bearing Calculator

- [ ] Chase the residual bearing error that remains with the timing trim at
      zero: about one degree at 48 kHz, which is 0.08 samples, and of the
      opposite sign at 96 kHz. Identical in the correlation and zero-crossing
      methods, so it is in the shared path rather than either estimator, and
      the north tracker is not the source -- its own bias measures 0.001
      samples. `examples/bearing_convention_probe` measures it directly and
      shows it is unaffected by pulse placement, pulse shape, tick jitter or
      noise, which rules those out as causes. Nor is it the doppler bandpass
      length: sweeping that filter from 0.7 ms to 5.3 ms moves the residual by
      0.05 degrees. It behaves like a fixed delay of about two microseconds --
      roughly constant in time rather than in samples, which is why 96 kHz
      shows a similar angle rather than half of one -- so look for something
      in the doppler path with a time constant.

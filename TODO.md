# TODO

- File rotation for --dump-audio to bound disk usage on very long recordings
  (memory growth is fixed: dumps stream to disk incrementally)
- Add criterion benchmarks for DSP pipeline (FIR filters, AGC, I/Q correlation)
- Improve zero-crossing coherence metric (sub-window phase variance like correlation method)

## North Tick Tracking

- [ ] Measure loop bandwidth against both acquisition and holdover, then pick
      one. At 2 Hz steady-state error is identical to 1 Hz -- 0.28 to 0.31
      degrees either way -- and acquisition halves from 0.84 s. But the
      coasting budget is derived from the scatter of the frequency estimate,
      and a wider loop makes that scatter worse, so holdover may shorten. The
      two now pull in opposite directions and the tradeoff has never been
      measured.
- [ ] Report a north channel that is too quiet to detect. The threshold
      sweep in `examples/north_threshold_sweep` shows the shipped threshold
      has wide margin -- detection holds from a pulse amplitude of 1.0 down
      to 0.3 against the 0.8 expected -- but below that cliff detection goes
      to zero rather than degrading, and nothing says so. A warning when
      detected pulse amplitude runs far below `expected_pulse_amplitude`
      would turn a silent failure into an obvious one.

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
      noise, which rules those out as causes.

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
- [ ] Sweep `threshold` against `expected_pulse_amplitude`. Both are
      inherited and neither was ever measured. They are absolute, so they
      assume a signal level: a receiver delivering half this amplitude sits
      near the threshold with nothing warning you. The sweep says whether the
      current pair has margin, and whether adaptive thresholding would buy
      anything -- DESIGN.md currently argues it would not, on the grounds
      that the reference amplitude is predictable, which is an assumption
      rather than a measurement.

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

- [ ] Resolve the half-sample convention, with two harnesses that disagree.
      `doppler.north_tick_timing_adjustment` defaults to half a sample. Two
      measurements say opposite things about whether it should.
      Against `generate_test_signal`, whose north pulse was gated on rotation
      phase and so landed at `ceil(epoch)` -- half a sample late -- the trim
      was cancelling that artifact. Fixing the generator to place a
      band-limited pulse at the true epoch and setting the trim to zero gives
      0.16 degrees of bearing error against 6.18 with the trim, and makes the
      bearing tests discriminate: the trim then fails two of them.
      Against `examples/system_pipeline_performance_metrics`, which builds its
      own signal with pulses at `round(k * period)` and unbiased jitter, the
      opposite holds: removing the trim moves p95 bearing error from 30.84
      degrees to 35.58 and trips the perf gate. Neither generator is biased,
      so something in the shared bearing path carries the half sample, and the
      two harnesses cannot both be right.
      Both changes -- the generator fix and the trim removal -- were made and
      then reverted on the north-timing branch, because shipping half of a
      coupled pair is worse than shipping neither. They belong together, here,
      with the perf gate as one of the tests.
- [ ] Chase the residual bearing error that remains once
      `north_tick_timing_adjustment` is zero: about 0.09 samples at 48 kHz
      and 0.25 at 96 kHz, sample-rate dependent, identical in the
      correlation and zero-crossing methods, so it is in the shared path
      rather than either estimator. Under 1.5 degrees. The north tracker is
      not the source; its own bias measures 0.001 samples.

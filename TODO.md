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

- [ ] Measure loop bandwidth against both acquisition and holdover, then pick
      one. At 2 Hz steady-state error is identical to 1 Hz -- 0.28 to 0.31
      degrees either way -- and acquisition halves from 0.84 s. But the
      coasting budget is derived from the scatter of the frequency estimate,
      and a wider loop makes that scatter worse, so holdover may shorten. The
      two pull in opposite directions and the tradeoff has never been
      measured.

## Bearing Calculator

- [ ] Chase the residual bearing error that remains with the timing trim at
      zero: -1.09 degrees at 48 kHz and -1.33 at 96, which is 0.08 samples and
      0.22 respectively. Same sign at both rates; an earlier note claiming the
      sign flipped was measured before the estimator and cutoff changes and no
      longer holds.
      `examples/bearing_convention_probe` measures it directly. Ruled out so
      far: pulse placement, pulse shape, tick jitter and noise, none of which
      move it by more than half a degree; and the doppler bandpass length,
      where a 4x sweep from 0.7 ms to 5.3 ms moves it by 0.05 degrees. The
      north tracker is not the source either -- its own bias measures 0.001
      samples, and the residual is identical in the correlation and
      zero-crossing methods, so it is in the path they share.
      What it looks like: a fixed delay of about two microseconds. Roughly
      constant in time rather than in samples, which is why 96 kHz shows a
      similar angle instead of half of one. Look for something in the doppler
      path with a time constant -- the AGC is the obvious candidate.

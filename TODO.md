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
      Also ruled out: the AGC, whose gain pinned to unity moves the residual
      by 0.001 degrees; and the correlation's forward extrapolation of the
      reference across a buffer, since a sixteenfold sweep of buffer size
      moves it by 0.07 degrees.
      It splits in two. `examples/bearing_convention_probe` now times the
      north tracker alone against the same signal: it reports ticks +0.048
      samples late, which is +0.57 degrees, independent of whether the pulse
      is impulsive or band-limited. That accounts for a little over half of
      the -1.07, leaving about -0.5 degrees in the bearing path proper.
      The tracker half is the sharper lead, because it should be zero: for an
      impulse the estimator's centroid and the delay compensation's reference
      are computed the same way over the same window, so they ought to cancel
      exactly. Start by checking whether the detected peak index is the
      response's maximum tap -- the peak-search window and threshold crossing
      are what could pick a different sample.

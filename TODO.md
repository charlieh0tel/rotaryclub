# TODO

- File rotation for --dump-audio to bound disk usage on very long recordings
  (memory growth is fixed: dumps stream to disk incrementally)
- Add criterion benchmarks for DSP pipeline (FIR filters, AGC, I/Q correlation)
- Improve zero-crossing coherence metric (sub-window phase variance like correlation method)

## North Tick Tracking

- [ ] Find why a low `highpass_cutoff` breaks whole-sample shift invariance,
      then lower the default. `north_hpf_sweep` measures ~3x better centroid
      timing at 1 kHz than at the current 5 kHz on the captures in `data/`,
      with no detection loss at any cutoff tried including none, so the win
      is real and unclaimed. At 1 kHz `test_whole_sample_shift_invariance`
      fails at 0.048 samples against a 0.01 bound.
      Ruled out: the centroid window being too narrow for a longer-ringing
      filter. Deriving the half-width from the impulse response does not work
      either -- 95% of the positive energy sits in the peak tap alone, giving
      a half-width of 1, while 99.9% jumps to 30 samples (a full rotation,
      88 degrees of error). The energy distribution is bimodal, so coverage
      is the wrong criterion.
      Most likely suspect: the peak search window grows at low cutoff (the
      threshold crossing moves much earlier relative to the peak), so more
      peaks are deferred across buffer boundaries and resolved at negative
      indices, where the estimator falls back to the peak index. Instrument
      the fallback rate per cutoff first, so the cause is measured rather
      than guessed.
- [ ] Scale the coasting budget to lock quality rather than the fixed
      `max_coast_ms`: coast further when the rate estimate is well
      established, less when it is not. A still-settling estimate drifts
      about 8 samples over a 300 ms coast, while a settled one should hold
      far better. Sweep dropout length against timing error at several lock
      ages first and set the scaling from that curve.
- [ ] Track down the half-sample convention that
      `doppler.north_tick_timing_adjustment` compensates. Its 0.5 default is
      not quantization compensation as suspected -- with sub-sample tick
      timing in place, setting it to zero makes bearing error worse and fails
      three noise-robustness tests. Likely window centering or a group-delay
      convention on the bearing side; remove the trim once found. Bearing
      work, so a separate branch.
- [ ] Extend false-positive sweeps to hum, clipping, and DC drift variants
- [ ] Add config guardrails for threshold/FIR/gain ranges with actionable error messages
      (done for DPLL frequency band inputs and min_interval_ms vs frequency_max_hz)
- [ ] Quantify DPLL lock and reacquisition performance (lock time, dropout recovery, step response limits)

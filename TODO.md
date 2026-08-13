# TODO

- File rotation for --dump-audio to bound disk usage on very long recordings
  (memory growth is fixed: dumps stream to disk incrementally)
- Add criterion benchmarks for DSP pipeline (FIR filters, AGC, I/Q correlation)
- Improve zero-crossing coherence metric (sub-window phase variance like correlation method)

## North Tick Tracking

- [ ] Re-measure the low `highpass_cutoff` case, then lower the default.
      `north_hpf_sweep` measures ~3x better centroid timing at 1 kHz than at
      the current 5 kHz on the captures in `data/`, with no detection loss at
      any cutoff tried including none, so the win is real and unclaimed. At
      1 kHz `test_whole_sample_shift_invariance` failed at 0.048 samples
      against a 0.01 bound.
      Ruled out: the centroid window being too narrow for a longer-ringing
      filter. Deriving the half-width from the impulse response does not work
      either -- 95% of the positive energy sits in the peak tap alone, giving
      a half-width of 1, while 99.9% jumps to 30 samples (a full rotation,
      88 degrees of error). The energy distribution is bimodal, so coverage
      is the wrong criterion.
      The suspect recorded here previously -- the estimator falling back to
      the peak index near a buffer edge -- has since been fixed, so the
      failure may already be gone. Re-measure before investigating further.
- [ ] Give the centroid estimator and the detection gate tests that fail when
      they are disabled. Today the whole suite passes with the estimator
      swapped for `HardLimiter`, and passes with gating switched off, so
      neither feature is actually covered.
      `test_commensurate_rate_produces_constant_offset` bounds bias at 0.75
      samples where the centroid measures 0.0025, and it exercises a single
      sub-sample phase which happens to be that estimator's zero-bias point.
      Sweeping phase at a commensurate rate shows the centroid halves the
      quantization bias rather than removing it, leaving about +/-0.24
      samples.
- [ ] Scale the coasting budget to lock quality rather than the fixed
      `max_coast_ms`: coast further when the rate estimate is well
      established, less when it is not. A still-settling estimate drifts
      about 8 samples over a 300 ms coast, while a settled one should hold
      far better. Sweep dropout length against timing error at several lock
      ages first and set the scaling from that curve.
- [ ] Reconcile the detection gate with the detector dead time. The gate is
      documented as rejecting interference that "can land anywhere in the
      rotation", but `min_interval_ms` is 96% of the rotation period, so an
      early impulse is never detected at all and the gate's one-sample floor
      covers what little window remains. It can only reject late detections.
      Either narrow the dead time or describe what the gate actually does.
- [ ] Size the end-of-buffer coasting guard against the detector's deferral
      window rather than the rotation period. A crossing near a buffer end
      resolves in the next buffer at a negative index, so a predicted tick
      can in principle land inside the dead time before a real detection.
      Not observed in probes across chunk sizes 32 to 100000; suspicion only.
- [ ] Extend false-positive sweeps to hum, clipping, and DC drift variants
- [ ] Add config guardrails for threshold/FIR/gain ranges with actionable error messages
      (done for DPLL frequency band inputs and min_interval_ms vs frequency_max_hz)
- [ ] Quantify DPLL lock and reacquisition performance (lock time, dropout recovery, step response limits)

## Bearing Calculator

- [ ] Chase the residual bearing error that remains once
      `north_tick_timing_adjustment` is zero: about 0.09 samples at 48 kHz
      and 0.25 at 96 kHz, sample-rate dependent, identical in the
      correlation and zero-crossing methods, so it is in the shared path
      rather than either estimator. Under 1.5 degrees. The north tracker is
      not the source; its own bias measures 0.001 samples.
- [ ] Express `north_tick_timing_adjustment` in microseconds rather than
      samples, so a calibration made at one sample rate holds at another.

# TODO

- File rotation for --dump-audio to bound disk usage on very long recordings
  (memory growth is fixed: dumps stream to disk incrementally)
- Add criterion benchmarks for DSP pipeline (FIR filters, AGC, I/Q correlation)
- Improve zero-crossing coherence metric (sub-window phase variance like correlation method)

## North Tick Tracking

- [ ] Harden the simple tracker against baseline disturbance. Under combined
      hum, clipping and drift it detects about half the pulses where the DPLL
      detects nearly all of them, and an initial DC offset stepping into a
      zero-state highpass drops it to 0.22. Its period estimate averages
      measured intervals, so a disturbance that costs it detections also
      degrades the spacing guard the remaining ones are judged against.
      `test_north_tick_detection_under_hum_clipping_and_drift` records the
      current behaviour.

## Bearing Calculator

- [ ] Chase the residual bearing error that remains once
      `north_tick_timing_adjustment` is zero: about 0.09 samples at 48 kHz
      and 0.25 at 96 kHz, sample-rate dependent, identical in the
      correlation and zero-crossing methods, so it is in the shared path
      rather than either estimator. Under 1.5 degrees. The north tracker is
      not the source; its own bias measures 0.001 samples.
- [ ] Express `north_tick_timing_adjustment` in microseconds rather than
      samples, so a calibration made at one sample rate holds at another.

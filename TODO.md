# TODO

- File rotation for --dump-audio to bound disk usage on very long recordings
  (memory growth is fixed: dumps stream to disk incrementally)
- Add criterion benchmarks for DSP pipeline (FIR filters, AGC, I/Q correlation)
- Improve zero-crossing coherence metric (sub-window phase variance like correlation method)

## North Tick Tracking

- [ ] Narrow the detector dead time so the gate can act on more than late
      detections. `min_interval_ms` covers 96% of the rotation, so an impulse
      arriving early is never detected at all. The gate's floor and its
      description were corrected to match what it can reach; widening that
      reach means shortening the dead time, which trades against low-SNR
      detection and needs its own measurement.
- [ ] Give the detection gate a test that discriminates at the shipped loop
      bandwidth. The current one runs at 60 Hz because a narrow loop absorbs
      a handful of displaced detections whether they are gated or not, so
      what the gate is worth in the shipped configuration is unmeasured.
- [ ] Size the end-of-buffer coasting guard against the detector's deferral
      window rather than the rotation period. A crossing near a buffer end
      resolves in the next buffer at a negative index, so a predicted tick
      can in principle land inside the dead time before a real detection.
      Not observed in probes across chunk sizes 32 to 100000; suspicion only.
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

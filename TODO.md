# TODO

- File rotation for --dump-audio to bound disk usage on very long recordings
  (memory growth is fixed: dumps stream to disk incrementally)
- Add criterion benchmarks for DSP pipeline (FIR filters, AGC, I/Q correlation)
- Improve zero-crossing coherence metric (sub-window phase variance like correlation method)

## North Tick Tracking

- [ ] Scale the centroid window to the highpass impulse response, then lower
      `highpass_cutoff`. `north_hpf_sweep` measures ~3x better centroid timing
      at 1 kHz than at the current 5 kHz on the captures in `data/`, with no
      detection loss, but a longer-ringing filter outgrows the fixed window
      and boundary fallbacks start to show up as a whole-sample shift
      dependence.
- [ ] Quantify holdover accuracy against coast length and pick
      `max_coast_ms` from it; a rate estimate that is still settling drifts
      several samples over a 300 ms coast.

- [x] Measure end-to-end north tick timing latency/jitter vs synthetic ground truth across chunk sizes and chunk-boundary phase offsets
- [x] Add CSV + markdown timing artifacts and CI reporting for threshold failures (including failed-row artifact)
- [x] Add realistic false-positive sweeps for impulsive interference/dropout/noise with separate detection/FP metrics
- [ ] Extend false-positive sweeps to hum, clipping, and DC drift variants
- [x] Add long-duration drift timing scenario
- [x] Add frequency-step timing scenario
- [ ] Add config guardrails for threshold/FIR/gain ranges with actionable error messages
      (done for DPLL frequency band inputs and min_interval_ms vs frequency_max_hz)
- [ ] Quantify DPLL lock and reacquisition performance (lock time, dropout recovery, step response limits)

## Bearing Calculator

- [x] Add guardrails for degenerate bearing inputs (empty buffer and non-finite north-tick fields) to avoid NaN outputs
- [x] Add dedicated bearing regression tests for degenerate inputs and bounded/finiteness metrics
- [x] Run bearing regression tests explicitly in CI
- [x] Add bearing-only performance metrics benchmark artifact (baseline + strict profiles)
- [x] Add bearing performance threshold checks and failed-row markdown reporting
- [x] Add end-to-end system-level performance bars (north tracker + bearing calculator combined)

# TODO

- File rotation for --dump-audio in live capture mode to avoid unbounded memory growth
- Add criterion benchmarks for DSP pipeline (FIR filters, AGC, I/Q correlation)
- Improve zero-crossing coherence metric (sub-window phase variance like correlation method)

## North Tick Tracking

- [x] Measure end-to-end north tick timing latency/jitter vs synthetic ground truth across chunk sizes and chunk-boundary phase offsets
- [x] Add CSV + markdown timing artifacts and CI reporting for threshold failures (including failed-row artifact)
- [x] Add realistic false-positive sweeps for impulsive interference/dropout/noise with separate detection/FP metrics
- [ ] Extend false-positive sweeps to hum, clipping, and DC drift variants
- [x] Add long-duration drift timing scenario
- [x] Add frequency-step timing scenario
- [ ] Add config guardrails for threshold/min_interval/FIR/gain ranges with actionable error messages
- [ ] Quantify DPLL lock and reacquisition performance (lock time, dropout recovery, step response limits)

## Bearing Calculator

- [x] Add guardrails for degenerate bearing inputs (empty buffer and non-finite north-tick fields) to avoid NaN outputs
- [x] Add dedicated bearing regression tests for degenerate inputs and bounded/finiteness metrics
- [x] Run bearing regression tests explicitly in CI

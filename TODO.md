# TODO

- File rotation for --dump-audio in live capture mode to avoid unbounded memory growth
- Add criterion benchmarks for DSP pipeline (FIR filters, AGC, I/Q correlation)
- Improve zero-crossing coherence metric (sub-window phase variance like correlation method)

## North Tick Tracking

- [ ] Measure end-to-end north tick timing latency/jitter vs synthetic ground truth across chunk sizes and chunk-boundary phase offsets
- [ ] Add realistic false-positive sweeps (impulsive noise, hum, clipping, DC drift) with separate detection/FP metrics
- [ ] Add config guardrails for threshold/min_interval/FIR/gain ranges with actionable error messages
- [ ] Quantify DPLL lock and reacquisition performance (lock time, dropout recovery, step response limits)

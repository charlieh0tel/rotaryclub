# TODO

- File rotation for --dump-audio in live capture mode to avoid unbounded memory growth
- Add criterion benchmarks for DSP pipeline (FIR filters, AGC, I/Q correlation)
- Improve zero-crossing coherence metric (sub-window phase variance like correlation method)
- Remove static north-tick timing bias by carrying fractional detector/filter delay into `NorthTick`
- Use pulse peak-time indexing (or matched-filter peak) instead of threshold-crossing index for north ticks
- Gate DPLL fractional phase/timing correction by lock quality or phase-variance threshold during acquisition

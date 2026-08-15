# Metrics

What this receiver measures about itself, what those measurements cannot see,
what is missing, and how to add one without fooling yourself.

Per-field definitions of the output metrics live in README under *Output
Measures*. This file is about which question each one answers, and about
method.

## What each metric answers, and what it is blind to

| Metric | Answers | Blind to |
| --- | --- | --- |
| `bearing_uncertainty_deg` | How far this bearing is likely to be from the average of many like it | Anything every estimate shares. A reflection, a mis-set north offset, a systematic timing error. It is precision, not accuracy |
| `confidence` | The same question on a 0–1 scale against a configured half point | The same things, plus: it is a monotone function of the uncertainty and carries no independent information |
| `snr_db` | How much of the buffer's power correlated with the rotation | Whether the correlated part points anywhere sensible |
| `signal_strength` | Whether there is a carrier at all | Whether the bearing derived from it agrees with itself |
| `resultant_length` † | Whether the looks agreed with each other | How strong they were. Derived from `snr_db`, so under a reflection it reads high where a directly measured one would read low |
| `tone_peak` † | Absolute level of the filtered Doppler signal | Everything else. It is a level |
| `lock_quality` | Whether the north DPLL is tracking | The Doppler side entirely |

† Defined by KR6DD; see [Attribution](#attribution).

Two of these deserve emphasis because they are easy to over-read.

**Uncertainty is precision, not accuracy.** It is derived from the
signal-to-noise ratio, so it sees noise by construction and cannot see a
displacement shared by every estimate. Measured against observed scatter it
runs about 1.06 on the recordings and 1.09 on synthetic signal, so the scale is
right; that is a separate property from what it can detect at all.

**Confidence is blind to reflections.** With a reflected path 0.45 of the
direct one, filtering at 0.5 confidence improves median error by 5 percent
while discarding 42 percent of the bearings, against 23 to 58 percent on a
clean channel, and its rank correlation against actual error falls from 0.40 to
0.13. A reflection leaves the tone strong and moves where it points, which is
exactly what an SNR-derived figure cannot register.

## Attribution

Two of the metrics above are not this project's. `resultant_length` and
`tone_peak` are **KR6DD's**, defined in his `RPiDDFengine` and carried in the
"C" sentence as *angle vector average magnitude* and *FIR filtered Doppler tone
peak value*. The sentence format is his as well; the copy in
`kn5r-rdf/docs/data-format.md` is headed "Data format from KR6DD".

What is specifically his, and worth stating because it is a design choice
rather than an implementation detail:

**Using the resultant length of the phase vectors as the bearing quality
figure.** The mean resultant length is a standard circular statistic, but
choosing it as *the* quality number a direction finder reports — maximal when
every zero crossing agrees on the angle, zero when they are scattered — is his,
and it is a better choice than the obvious alternatives. Signal strength says
whether a carrier is present, not whether the bearing derived from it holds
together; a coherence says the second thing, and the second thing is what a
hunter needs.

**Reporting an absolute tone level alongside it.** The two answer different
questions and neither substitutes for the other, which is why sending signal
strength in the magnitude field was wrong here rather than merely imprecise.

This receiver computes `resultant_length` from the signal-to-noise ratio rather
than from per-crossing vectors, so it is an estimate of his quantity rather
than the same computation. That is our compromise, not his design.

The wider format and the consuming ecosystem — the "S", "B" and "T" sentences,
the collection and display side — come from the KN5R-RDF project.

## What the gates measure

Three harnesses, run in CI, each with limits in `scripts/*_report.py`.

| Harness | Covers | Draws per cell |
| --- | --- | --- |
| `bearing_performance_metrics` | Bearing accuracy and throughput per method, scenario and buffer size | 8 |
| `north_tick_timing_metrics` | Tick detection and timing per tracking mode, scenario and chunk size | 8 |
| `system_pipeline_performance_metrics` | The whole stack: detection, bearing error, tick error, throughput | 16 |

Scenarios are `clean`, `noisy_jittered`, `harmonic_contaminated` and
`low_snr_dc`, with the interference scaled to the passband noise the recordings
measure (0.2, 0.8, 6.5 against the tone).

Metrics are written as JSONL with a meta record carrying a hash of the sources
that produced them; `check` refuses a file that a different version of the code
produced. `scripts/compare_refs.py` runs one harness against two git states so
both sides are produced identically.

## Missing: shortest detectable signal

The most useful metric this project does not have. A hunter wants to know
whether a half-second transmission yields a bearing.

Suggested definition:

> The shortest burst duration T for which, over N independent noise
> realisations, at least 90 percent yield a reported bearing within E degrees of
> truth with a stated uncertainty at or below U.

Four parts of that are load-bearing:

**One reported bearing, not any bearing in the burst.** A burst yields many
candidates, and requiring merely that one be good is nearly free — a uniformly
random bearing lands within 30 degrees about 17 percent of the time. Score the
aggregate: the circular median over the burst, or the smoothed output at its
end.

**Both an accuracy and a self-report criterion.** E alone measures the
estimator; U alone measures whether it knows. Confident and wrong is worse than
uncertain and right.

**A rate over draws, not a single run**, with the distribution checked for
bimodality before any mean is quoted.

**A carrier-absent control at the same T.** Run the identical criterion against
squelch-open receiver noise with no transmission. The metric means nothing
above its own false-alarm floor, and bearings computed on hiss look like
bearings.

What it depends on, and so what to sweep: in-band Doppler SNR, and buffer size.
Those two are the whole surface. Bigger buffers give better bearings — scatter
13.0 degrees at 128 samples against 3.5 at 2048 — and a longer minimum
duration, so shortest-detectable and best-bearing pull against each other with
buffer size as the knob. A single number hides that; a table of T against SNR
for two or three buffer sizes does not.

Two conditions to state rather than sweep. The north channel is generated
locally and fed in, so it is not a limiting factor and the DPLL is locked
whether or not anyone is transmitting: the floor is the Doppler path alone,
bandpass settling plus a filled work buffer, around 25 ms at the default buffer
size. And set `smoothing_window` to 1, or the first outputs of the burst
measure the smoother's memory rather than detection.

## Measuring without fooling yourself

Rules that earned their place.

**One realisation is not a measurement.** Average over independent noise draws
and report the standard error of the mean. Size the count from the measured
spread rather than picking a round number; most rows here settle under 0.001
with 8 to 16.

**Check for bimodality before quoting a mean.** A mean is only a summary if the
distribution has one mode. Print the per-draw values for any cell whose error
bar is wide — a metric that reads 0.894 with a standard error of 0.049 may be
taking the values 0.99 and 0.49 and never 0.894.

**Every detection metric needs a null control.** Run the same criterion against
input with nothing in it. Without that, a metric measures how often the
pipeline emits something, not how often it is right.

**Gate on carrier presence when measuring against recordings.** Seventy percent
of `doppler-test-2023-04-10-ft-70d.wav` is receiver hiss between overs.
Ungated, its uncertainty calibration reads 0.73; above a 6 dB floor, 1.06.

**Sweep the regime the hardware occupies, and say when you leave it.** The
recordings put north-channel noise around 0.0006 RMS and passband interference
between 0.2 and 6.5. A sweep to 0.2 RMS on the north channel is exploring a
condition no hardware has produced, and conclusions drawn there should say so.

**Vary the seed, but know what that does and does not cover.** Independent
draws estimate the expectation over the noise model. They do not test the noise
model: every draw here assumes band-limited interference with a log-normal
envelope, and no number of them will reveal that assumption to be wrong.

**State the prediction before measuring**, including its direction. Two
predictions recorded here came out backwards, and both would have been easy to
rationalise afterwards.

**Read only artifacts your command produced.** The metrics harnesses print to
stdout and only the report script redirects that into a file; running an
example by hand leaves the file describing whatever ran last, which is
indistinguishable from a fresh result.

## Where the numbers come from

Measurement tools, none of which run in CI:

| Example | Measures |
| --- | --- |
| `signal_census` | Synthetic signal against the recordings: pulse amplitude, width, floor, jitter, in-band fraction, harmonics |
| `interference_census` | Interference shape and time structure in the Doppler passband |
| `dropout_census` | What else changes when the rotation tone weakens |
| `uncertainty_reference_probe` | Which scatter the stated uncertainty should be calibrated against, gated on carrier |
| `confidence_under_multipath` | Whether filtering on confidence improves the bearings kept |
| `north_threshold_sweep` | Detection and false positives against pulse amplitude, noise and threshold |
| `north_loop_bandwidth_sweep` | Loop bandwidth against acquisition, steady state and holdover |
| `north_agc_probe` | AGC behaviour against north-channel noise, per tracking mode |
| `coast_budget_probe` | How far the coasting budget lets the loop predict |
| `zero_crossing_bias_probe` | Bias in the zero-crossing estimator across seeds and filter widths |
| `north_hpf_sweep` (bin) | Highpass cutoff against per-tick timing, on the captures |
| `config_sweep` (bin) | Any configuration or stimulus axis against bearing and tick error, with `--seeds` and `--jsonl` |

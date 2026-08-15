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

## What "in-band SNR" means here

Everything measured against channel conditions uses the power of the
interference *inside the Doppler passband*, 1350 to 1850 Hz, against the power
of the rotation tone. The captures measure +7, +1 and −8 dB — ratios of 0.199,
0.793 and 6.579 the other way up.

It is deliberately not the ratio over the whole channel. Real FM audio has most
of its energy well below the passband, where it does no harm, so matching total
power with flat noise puts about ten times too much where it hurts: at the
cleanest recording's whole-channel ratio, flat noise produced 20.7 degrees of
bearing error against the 1.6 that recording achieves.

Two different situations land on the same number, and no metric here separates
them. The tone's amplitude is set by the antenna switching depth, which the
hardware fixes; the interference is whatever else the receiver delivers in that
band. So −8 dB is either a loud talker on a strong signal, whose voice
deviation swamps the switching deviation, or a weak signal whose receiver noise
fills the band. A duration or an error quoted at −8 dB answers both questions
at once, and only because the in-band ratio happens to match.

## What the north channel is, and is not

The north reference is generated by the antenna switcher and fed in, so it is
not a limiting factor in service and should not be treated as one when
choosing what to measure. Its level is ours to set correctly; where the
captures show a four-fold spread in pulse amplitude — 0.21, 0.44 and 0.78
against an expected 0.8 — that is levels having been set wrong, not an
environmental variable. The AGC exists to be forgiving about it, not because
the level is unknowable.

Three consequences.

**The loop is locked whether or not anyone is transmitting.** The switcher
does not stop when the transmitter does, so acquisition happens once at
power-on rather than per transmission. Any metric about responding to a signal
should assume a locked loop; the 0.52 second acquisition figure is a power-on
number and quoting it as detection latency would be wrong by an order of
magnitude.

**Sweeping north-channel noise measures an unreachable regime.** The captures
put the floor near 0.0006 RMS. Conclusions drawn at 0.05 or 0.2 — including
the simple tracker losing every other pulse at 0.10 — describe conditions no
hardware has produced, and should say so.

**The one real risk is bleed from the Doppler channel.** It is not expected and
no capture shows it, but it is the only mechanism that would put significant
energy into the north channel, and it is what the highpass cutoff exists to
reject. It is therefore also the only route by which the north-noise results
above could become relevant.

## What the gates measure

Three harnesses, run in CI, each with limits in `scripts/*_report.py`.

| Harness | Covers | Draws per cell |
| --- | --- | --- |
| `gate_bearing` | Bearing accuracy and throughput per method, scenario and buffer size | 8 |
| `gate_north_tick` | Tick detection and timing per tracking mode, scenario and chunk size | 8 |
| `gate_pipeline` | The whole stack: detection, bearing error, tick error, throughput | 4 |

Scenarios are `clean`, `noisy_jittered`, `harmonic_contaminated` and
`low_snr_dc`, with the interference scaled to the in-band SNR the recordings
measure (+7, +1 and −8 dB).

Metrics are written as JSONL with a meta record carrying a hash of the sources
that produced them; `check` refuses a file that a different version of the code
produced. `scripts/compare_refs.py` runs one harness against two git states so
both sides are produced identically.

## Shortest detectable signal

The shortest burst duration T for which, over N independent noise
realisations, at least 90 percent yield a reported bearing within E degrees of
truth with a stated uncertainty at or below U.

Measured by `metric_shortest_signal` at E = U = 10 degrees, over 24 draws,
with the burst embedded in squelch-open hiss and the north loop already
locked. Detection rate by burst length:

| Buffer | In-band SNR | 25 ms | 50 ms | 100 ms | 200 ms | 400 ms | 800 ms | 1600 ms | **T90** | Control |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 256 | +7 dB | 0.79 | 0.92 | 1.00 | 1.00 | 1.00 | 1.00 | 1.00 | **50 ms** | 0.00 |
| 256 | +1 dB | 0.54 | 0.67 | 0.79 | 0.96 | 1.00 | 1.00 | 1.00 | **200 ms** | 0.00 |
| 256 | −8 dB | 0.33 | 0.33 | 0.42 | 0.71 | 0.83 | 0.92 | 1.00 | **800 ms** | 0.00 |
| 1024 | +7 dB | 0.75 | 0.92 | 0.96 | 0.96 | 1.00 | 1.00 | 1.00 | **50 ms** | 0.00 |
| 1024 | +1 dB | 0.50 | 0.67 | 0.88 | 0.96 | 1.00 | 1.00 | 1.00 | **200 ms** | 0.00 |
| 1024 | −8 dB | 0.12 | 0.25 | 0.42 | 0.62 | 0.92 | 0.92 | 1.00 | **400 ms** | 0.00 |

The three levels are what the captures measure: +7, +1 and −8 dB, or ratios of
0.2, 0.8 and 6.5 of interference power to tone power. So a 50 ms transmission
on the cleanest channel recorded, 200 ms on the middle one, and 400 to 800 ms
on the worst — where the rotation tone sits 8 dB *below* the audio on top of
it.

The control column is the same criterion applied to the same window with no
burst in it. It reads zero throughout, so these rates sit above a zero
false-alarm floor.

Two things to read carefully.

**Buffer size barely matters, which was not expected.** The reasoning was that
bigger buffers give better bearings and a longer minimum duration, so the two
would trade. They do, but only at the extremes: at 25 ms the smaller buffer
wins on every noise level (0.79 against 0.75, 0.33 against 0.12), since a 1024
sample buffer is 21 ms and barely fits inside the burst at all. From 400 ms up
the larger buffer wins at the worst noise. In between there is nothing to
choose, and the knob is not the lever it was expected to be.

**The stated uncertainty is that of the aggregate, not of one buffer.** The
score is a median over the burst's bearings, so it is judged against what that
median claims, which is the per-buffer figure over the root of the number of
looks. Judging an aggregate against a single buffer's uncertainty makes the
criterion insensitive to duration -- the first version of this measurement did
that and reported nothing detectable at all above the cleanest noise level.
The division is optimistic about the reference term, which is common to every
look and does not average away, so read these durations as a floor.

## Measuring without fooling yourself

Rules that earned their place.

**One realisation is not a measurement.** Average over independent noise draws
and report the standard error of the mean.

**Size the draw count, do not pick it.** For a gate the requirement is that
noise must not trip a limit, so the count needed is (3 sd / margin)^2 for the
worst row and metric, with the margin measured from the value to its limit.
Guessing costs real time: sixteen draws on the pipeline gate turned out to be
twelve times more than anything needed, and cutting to four took that gate from
about seven minutes to twenty-three seconds. Two things must be left out of
that sum -- timing columns, whose spread is machine load rather than noise, and
checks that cannot fail, such as a bearing-error limit above the 180 degrees a
bearing error can reach. Including the latter put the requirement at 14 instead
of 1.3.

The count is tied to the limits. Tightening one shrinks its margin and raises
the count needed, so re-run the sizing when a limit moves.

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

Everything that measures the receiver lives in the `rotaryclub-metrics`
workspace member, run as `cargo run -p rotaryclub-metrics --bin <name>`. It is
not a default workspace member, so `cargo build` and the packaging that wraps
it are unaffected, and nothing here is installable.

The prefix says what a thing is: `gate_` is enforced by CI against limits in
`scripts/`, `census_` compares the synthetic channel against the recordings,
`sweep_` walks a parameter, `probe_` answers one question, `metric_` produces a
headline number. `examples/` is for demonstrating the receiver, `tests/` for
testing it, `benches/` for the one thing that measures speed.

The gates:

| Instrument | Measures |
| --- | --- |
| `gate_bearing` | Bearing accuracy and throughput per method, scenario and buffer size |
| `gate_north_tick` | Tick detection and timing per tracking mode, scenario and chunk size |
| `gate_pipeline` | The whole stack: detection, bearing error, tick error, throughput |

The rest, none of which run in CI:

| Instrument | Measures |
| --- | --- |
| `census_signal` | Synthetic signal against the recordings: pulse amplitude, width, floor, jitter, in-band fraction, harmonics |
| `census_interference` | Interference shape and time structure in the Doppler passband |
| `census_dropout` | What else changes when the rotation tone weakens |
| `sweep_config` | Any configuration or stimulus axis against bearing and tick error, with `--seeds` and `--jsonl` |
| `sweep_threshold` | Detection and false positives against pulse amplitude, noise and threshold |
| `sweep_loop_bandwidth` | Loop bandwidth against acquisition, steady state and holdover |
| `sweep_hpf` | Highpass cutoff against per-tick timing, on the captures |
| `probe_uncertainty_reference` | Which scatter the stated uncertainty should be calibrated against, gated on carrier |
| `probe_confidence_multipath` | Whether filtering on confidence improves the bearings kept |
| `probe_agc` | AGC behaviour against north-channel noise, per tracking mode |
| `probe_coast_budget` | How far the coasting budget lets the loop predict |
| `probe_zero_crossing_bias` | Bias in the zero-crossing estimator across seeds and filter widths |
| `metric_shortest_signal` | Shortest burst yielding a usable bearing, against noise and buffer size |
| `compare_config` | Two configurations against one generated signal |

`scripts/compare_refs.py` runs a gate against two git states; `benches/north_hotspots`
is the one thing here that measures speed rather than accuracy.

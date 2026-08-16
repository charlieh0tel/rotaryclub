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
of the rotation tone. The three conditions used throughout are +7, +1 and −8 dB
— ratios of 0.199, 0.793 and 6.579 the other way up.

Those three are defined conditions, not measured properties of the three
recordings, and earlier versions of this document said otherwise. They were
introduced as what the captures measure, from an FFT that was never checked in;
`metric_in_band_snr` now performs that measurement, calibrated against
synthetic signals whose ratio is exact (it recovers the three conditions to
within 0.1 dB), and does not agree: over whole files the captures give −9.5,
+2.6 and +2.4 dB.

There is no per-recording number sharper than that, and the instrument itself
demonstrated why. A recording is transmissions separated by squelch noise, so
the per-segment ratio inside one file spans two to four orders of magnitude —
ft-70d runs 0.014 at its tenth percentile and 146 at its ninetieth. Which
single number a recording "has" is decided entirely by which segments are
counted; the original selection rule behind +7/+1/−8 was never recorded, which
is why the triple could not be reproduced. A "median over the loudest half"
summary was tried and retired: it failed its own known-answer calibration by
3.4 dB, and moved 7 dB when the measurement window was merely halved with the
same segments selected. Quote a percentile band with the rule stated, never a
single number.

Nothing measured against these conditions is invalidated: the generators set
their ratio by construction — exactly, since the scale is now set by the same
in-band estimator the calibration validates — so a row labelled −8 dB was
produced at −8 dB whatever any recording does. What does not survive is the
claim that the three conditions are the three captures. −8 dB is a fair
description of ft-70d as a whole; +7 dB describes no whole recording here.
Read them as a span of plausible channels.

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
| `gate_pipeline` | The whole stack: detection, bearing error, tick error, throughput | 8 |

Scenarios are `clean`, `noisy_jittered`, `harmonic_contaminated` and
`low_snr_dc`, with the interference scaled to the three defined in-band SNRs
(+7, +1 and −8 dB); see above for what those do and do not say about the
recordings.

Metrics are written as JSONL with a meta record carrying a hash of the sources
that produced them; `check` refuses a file that a different version of the code
produced. `scripts/compare_refs.py` runs one harness against two git states so
both sides are produced identically.

## Shortest detectable signal

The shortest burst duration T for which, over N independent noise
realisations, at least 90 percent yield a reported bearing within E degrees of
truth with a stated uncertainty at or below U.

Measured by `metric_shortest_signal` at E = U = 10 degrees, over 48 draws at
each of 13 log-spaced durations, with the burst embedded in squelch-open hiss
and the north loop already locked.

| In-band SNR | 256 sample buffer | 1024 sample buffer |
| ---: | ---: | ---: |
| +7 dB | **95 ms** | 140 ms |
| +1 dB | **200 ms** | 300 ms |
| −8 dB (about ft-70d) | **940 ms** | 940 ms |

So a tenth of a second on a clean channel, a fifth of a second at +1 dB, and
just under a second at −8 dB, where the rotation tone sits below the audio on
top of it.

The clean-channel cells doubled from an earlier version of this table (45 and
65 ms) when two small-sample leaks in the stated-uncertainty test were closed
— bursts too short to estimate their own look-correlation now assume the
worst measured one instead of independence, and the lag-1 estimator's small-n
bias is corrected — and the generated conditions became exact rather than
0.6 dB easy. Those cells are decided by the stated test, so they moved most;
the weak-channel cells are error-limited and did not move. T90's resolution
is one buffer (5.3 ms at 256, 21.3 ms at 1024): each cell now carries the
number of chunks it scored, and adjacent cells sharing that count are the
same measurement.

The control is the same criterion applied to a window of the longest
duration, 2000 ms, with no transmission in it — the worst case, since a
longer window offers more chunks and a smaller stated uncertainty for an
aggregate of noise to hide behind. It reads 0.02 (1 draw in 48) at both
buffer sizes, so the detection rates above sit far clear of the false-alarm
floor. With no burst present the channel is the same hiss whatever the SNR
label says, so the control varies only with buffer size. An earlier version
of this measurement reported 0.00 from a zero-length control window that
scored no chunks at all; 0.02 is the number that could actually have moved.

`scripts/plot_shortest_signal.py` draws the curves from the harness's JSONL,
marking each crossing.

Two things to read carefully.

**The small buffer is never worse.** It wins at +7 and +1 dB and ties at −8.
An earlier version of this table had the large buffer winning on the two
weaker channels, 140 ms against 200 and 640 against 940, and explained it as
integration being what a weak signal needs. That reversal was an artifact of
the paragraph below: the large buffer's looks are the more correlated of the
two, so over-crediting raw look count flattered it most. Correcting that
removed the effect entirely rather than shrinking it.

**The stated uncertainty is that of the aggregate, and its looks are not
independent.** The score is a median over the burst's bearings, so it is judged
against what that median claims — the per-buffer figure divided by the root of
the number of looks. Judging it against a single buffer's uncertainty instead
makes the criterion insensitive to duration; the first version of this
measurement did that and reported nothing detectable above the cleanest
channel.

But the looks are one per rotation, 0.6 ms apart, while the work buffer spans
several rotations, so consecutive bearings are computed from mostly the same
samples. Their lag-1 correlation measures 0.886. Dividing by the root of the
raw count therefore claimed 3.4 to 5.0 times the precision the aggregate
actually has, measured against the scatter of the median across draws — and an
AR(1) at that correlation predicts 4.06, so the discrepancy is fully accounted
for. The harness now divides by the root of the effective count,
`n(1−r)/(1+r)`, with r measured from the burst in hand. That agrees with the
observed scatter to within 20 percent, conservative on a clean channel and
about a fifth optimistic at the worst.

It remains optimistic about the reference term, which is common to every look
and does not average away at all, so read these durations as a floor.

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

**And let the gate check that for you.** Each harness emits a standard error
beside every metric, and `check` refuses a row that passes by less than three
of them: within limits, but not by more than its own noise. That turns the
sizing rule from something to remember into something enforced, and it catches
three things at once — a tightened limit whose margin no longer supports the
draw count, a value drifting toward its limit before it crosses, and a metric
gone bimodal, whose inflated spread demands draws it does not have.

Two exclusions, declared per gate. Timing columns, whose spread is load. And
limits that cannot be crossed, which need a declared physical maximum: a
bearing error cannot exceed 180 degrees, so a limit at 181 is not a margin.

It found five rows on first run, all of them maxima or 95th percentiles, which
are the most volatile things measured here. Two gates' limits were widened
from the observed spread and the pipeline gate went from four draws to eight.

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
| `probe_confidence_multipath` | Whether filtering on confidence improves the bearings kept |
| `probe_agc` | AGC behaviour against north-channel noise, per tracking mode |
| `probe_coast_budget` | How far the coasting budget lets the loop predict |
| `probe_zero_crossing_bias` | Bias in the zero-crossing estimator across seeds and filter widths |

`probe_uncertainty_reference` is retired: the calibration measurement it made
lives in `tests/bearing_uncertainty_test.rs`, which windows inside contiguous
carrier runs. The probe kept the older windowing that spans the silence
between overs (15 to 40 percent off), halved its buffers through a
frame-count confusion, and never propagated the file's sample rate -- three
defects in a second implementation of a measurement the test already makes.
| `metric_shortest_signal` | Shortest burst yielding a usable bearing, against noise and buffer size; `--jsonl` for `scripts/plot_shortest_signal.py` |
| `compare_config` | Two configurations against one generated signal |

`scripts/compare_refs.py` runs a gate against two git states; `benches/north_hotspots`
is the one thing here that measures speed rather than accuracy.

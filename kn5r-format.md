# The KN5R-RDF "C" sentence

The format and both quality metrics in it are **KR6DD's**, defined in his
`RPiDDFengine`. The "S", "B" and "T" sentences and the collection side come
from the KN5R-RDF project.

`src/output/kn5r.rs` emits the KN5R-RDF "C" sentence so this receiver can feed
displays and logging built for that format. What the fields contain is
defined by the sources below; two of the three had previously been guessed
wrong here.

Both sources are local: `../kn5r-rdf/docs/data-format.md` gives the layout, and
KR6DD's `../kr6dd-rdf-rs/RPiDDFengine20260510.4th` — with a Rust port beside it
in `rpiddf-rs/src/main.rs` — defines what goes in the fields. Where they differ
in emphasis the engine decides: the format document names the fields, the
engine is what actually fills them.

## The layout

```
C  AAAA  MMM  TTT  UUUUUUUUUUUUUUU
   |     |    |    |
   |     |    |    +-- Unix time, milliseconds, 15 digits
   |     |    +------- FIR filtered Doppler tone peak value, 0-999
   |     +------------ angle vector average magnitude, 0-999
   +------------------ bearing x 10, 0-3599
```

26 characters. The same three data fields also appear in the "S" sentence,
where the timestamp is replaced by 4 digits of milliseconds-into-the-batch.

## What the fields are

**Magnitude is a coherence, not a level.** The engine computes the vector sum
of the per-zero-crossing phase vectors, takes its length, and divides by how
many crossings there were:

```forth
sec-avg-vec-x f@ fdup f* sec-avg-vec-y f@ fdup f* f+ kfsqrt
fround>s 999 #section_zero+crossings */
```

That is the mean resultant length: 1 when every crossing agrees on the angle,
0 when they are scattered. Its own comment calls it a "quality factor".

We had been sending normalised signal strength, which is a different quantity:
a strong tone pointing inconsistently reads near full scale on that and low on
this. It now derives the resultant length from the
signal-to-noise ratio, since scatter of sigma gives a resultant length of
exp(-sigma^2 / 2) and one look at a power ratio r scatters by 1 / sqrt(r).

**Tone peak is an absolute level.** The engine keeps a running maximum of its
FIR output over the batch section, with that output scaled to plus or minus
one, and sends it as thousandths:

```forth
( | fval ) f1000 f@ f*        \ scaled up to show 3 decimals
( fval*1000 ) fround>s maxtonepeak kmax to maxtonepeak
```

We had been sending the SNR against a notional 40 dB full scale, a ratio where
the field wants a level. It now carries the largest positive sample of
the filtered Doppler buffer, which is the same quantity.

**Both scales are linear**, and neither is in decibels. Magnitude is linear in
the resultant length; tone peak is linear in amplitude, in thousandths of full
scale.

## The remaining answers

**There is no "no bearing" convention.** The engine has a
`min-acceptable-vec-mag` in its config, but nothing reads it to suppress
output: every section emits a sentence and the magnitude field simply goes
low. So going silent, which is what we do when there is no measurable tone, is
not what the format expects. A consumer distinguishing "no signal" from "the
process died" has only the magnitude to go on.

**The rate is 20 Hz.** The engine divides each second into twenty batch
sections — `20 constant #batch-buf-sections` — and sends one sentence per
section. Our default was 10 and is now 20.

**The timestamp is capture time, not emit time**, and specifically the *end*
of the batch section the bearing was measured over:
`attach-batch-section-end-time-msecs`.

It is also not read from the clock per sentence. `docs/notes-on-timestamps.md`
records the reasoning: a `utime` call has unpredictable latency, so repeated
calls add jitter to the timestamps. The engine counts samples against the
48 kHz crystal, good to about 50 ppm, and calls `utime` once a day as an
anchor.

We send emit time from the wall clock, so we differ on both counts. For a live
display it hardly matters; for logging and post-processing it folds in the
pipeline latency and the clock-call jitter.

## What is still not matched

The timestamp. Fixing it means carrying a capture time along each buffer
rather than reading the clock at the point of emission, which is a change to
the pipeline rather than to this formatter.

## Telling a bearing from squelch noise

The pipeline reports a bearing for every buffer, including buffers holding
nothing but squelch-open hiss, so anything plotting these sentences needs a
rule for which ones to draw. The sentence carries two fields that could serve,
and the one that looks right is the wrong one.

Rates below are over about 48,000 hiss sentences and six seconds per channel
condition, at the three in-band SNRs the recordings span. "False alarm" is the
fraction of hiss sentences a rule admits; the rest is the fraction of real
signal it keeps.

| rule | false alarm | +7 dB | +1 dB | −8 dB |
| --- | ---: | ---: | ---: | ---: |
| magnitude > 0 | 25.7% | 100% | 99.6% | 90.0% |
| magnitude ≥ 100 | 1.0% | 100% | 98.6% | 69.1% |
| magnitude ≥ 300 | 0.0% | 100% | 96.1% | 44.2% |
| **tone peak ≥ 320** | **1.6%** | **100%** | **99.6%** | **99.2%** |

Magnitude is the natural field to reach for and it does not work at the weak
end. A quarter of hiss sentences carry a nonzero magnitude, and raising the
threshold far enough to reject them costs a third of the bearings on the worst
channel. That is structural rather than bad luck: magnitude here is the mean
resultant length, which this receiver derives from the estimated SNR as
`exp(-1/(2·snr))`, a compressed restatement of SNR that flattens towards zero
exactly where the decision is hard.

Tone peak separates cleanly because it is an in-band amplitude, which is what
presence is. The cost is that it is an absolute level: the threshold above is
calibrated to the 0.9 RMS hiss the captures show, and a receiver at a
different gain or squelch setting needs a different number. Magnitude is
dimensionless and portable, and useless; tone peak discriminates and is not
portable. A consumer wanting both would need a field the sentence does not
have.

This describes our output. Whether KR6DD's engine populates these fields
identically at low SNR has not been checked against his code.

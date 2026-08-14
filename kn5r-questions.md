# Questions for the KN5R-RDF side

`src/output/kn5r.rs` emits the KN5R-RDF "C" sentence so this receiver can feed
displays and logging built for that format. The layout is understood from the
field widths alone; what the fields are supposed to *contain* is not documented
anywhere available here. The reference in the source points at a
`docs/data-format.md` in <https://github.com/kn5r/kn5r-rdf>, which is not
vendored into this repo, and nothing in DESIGN.md or README describes it.

What we emit today is therefore a set of guesses. They are consistent and
stable, so a consumer will see plausible numbers whether or not they mean the
right thing, which is the part worth checking.

## What we emit

```
C  AAAA  MMM  TTT  UUUUUUUUUUUUUUU
   |     |    |    |
   |     |    |    +-- Unix time, milliseconds, 15 digits
   |     |    +------- "tone peak", 0-999
   |     +------------ "magnitude", 0-999
   +------------------ bearing x 10, 0-3599
```

Example: `C3469960084001663117493011` is 346.9 degrees, magnitude 960, tone
084. The timestamp is zero-padded to 15 digits, making the sentence 26
characters.

## The questions

1. **What is "tone peak" meant to measure?**

   This is the one we are least sure of. It was fed from a phase-coherence
   metric, which turned out to sit near full scale regardless of signal
   quality, so it carried no information. It now derives from the estimated
   signal-to-noise ratio of the Doppler tone, scaled so 40 dB reads 999:

   ```
   clean signal    ~999
   moderate noise  ~370
   unusable        ~220
   ```

   Is that the intended quantity? Plausible alternatives are the absolute
   audio level of the recovered tone, the depth of its modulation, or a
   peak-hold of the tone amplitude rather than a ratio.

2. **What is "magnitude" meant to measure, and how does it differ from tone
   peak?**

   We send a normalised signal strength: for the correlation method, the
   square root of the fraction of buffer power that correlated with the
   rotation reference, so it is linear in amplitude rather than power. If
   magnitude is meant to be an absolute level and tone peak a quality figure,
   ours may be the wrong way round.

3. **Is 0-999 a linear scale, and of what?**

   We map linearly from a normalised 0-1. If consumers expect decibels, or a
   log scale, or a raw ADC count, the numbers will look wrong in a way that is
   hard to notice: they will still move in the right direction.

4. **Is there a convention for "no bearing"?**

   We simply emit nothing when there is no measurable Doppler tone. If the
   format has a way to say "receiving, but no bearing" -- a reserved angle, a
   zero magnitude -- we should use it rather than going silent, because going
   silent is indistinguishable from the process having died.

5. **What rate are consumers expecting?**

   Ours is configurable and defaults to 10 Hz. Worth knowing if displays
   assume something specific, or if there is a maximum they cope with.

6. **Is the timestamp expected to be capture time or emit time?**

   We send emit time. For a live display the difference is small; for logging
   and post-processing it is not, since it folds in the pipeline latency.

## Until these are answered

The mapping in `kn5r.rs` carries a comment saying it is unverified. Anyone
relying on the numeric fields for anything other than the bearing itself
should treat them as approximate and directionally correct rather than
calibrated.

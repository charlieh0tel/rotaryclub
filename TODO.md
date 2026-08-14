# TODO

- File rotation for --dump-audio to bound disk usage on very long recordings
  (memory growth is fixed: dumps stream to disk incrementally)
- Add criterion benchmarks for DSP pipeline (FIR filters, AGC, I/Q correlation)

## Measured with config_sweep

Five grids run once the sweep tool existed. Two of them found bugs in the
tool itself before finding anything about the pipeline, both of the same
shape: an axis that silently did nothing, visible because the rows came out
identical. `apply_rotation` rewrites the dead time and the loop's frequency
bounds from the defaults, and it was being called after the axes were
applied; and the generator took its pulse amplitude from a constant rather
than from the impairment. Identical rows are the tell, and worth checking
for on every sweep.

- [x] The uncertainty should come from the SNR, not from the phase spread.
      Done. The figure was the spread of the per-rotation phase estimates over
      an independent count, and it understated the bearing scatter everywhere:
      measured against a standard deviation the ratio ran 0.53 to 0.91 and
      centred on 0.69.

      The independent count was not the fault. Set the buffer to one filter
      length so the count is 1.01 and the averaging term disappears, and the
      figure still understated by 40 to 47 percent. The spread itself was too
      small: the per-window phases agreed with each other more closely than
      the bearing agreed with truth, because the interference is common-mode
      within a buffer. The doppler passband is 500 Hz, so in-band noise
      decorrelates over about 96 samples -- at a 128-sample buffer that is 1.3
      independent realisations, one coherent perturbation shifting every
      window together. A spread taken within the buffer cannot see it, and it
      varies buffer to buffer, so it landed in the bearing scatter instead.
      That also explains why it was mistaken for a bias question: the
      perturbation is shared, but it is not constant, so the mean bias stayed
      small.

      It now composes two terms in quadrature: 1 / sqrt(r n) for the doppler
      tone at a signal-to-noise power ratio r over n independent looks, and
      the reference timing variance the tick reports. All of the sub-window
      phase machinery is gone, along with `circular_mean_phase`,
      `wrap_phase_diff` and `MAX_PHASE_VARIANCE`.

      Not 1 / sqrt(2 r n), which is what the analysis above predicted and what
      is usually quoted: that is the complex-exponential result, and a real
      sinusoid carries its power at both +f and -f. Implemented with the two
      it read 0.73 against the scatter across a sixteenfold buffer range and a
      thirtyfold noise range -- flat, which is what says a constant is wrong
      rather than a model. Without it the same sweep reads 0.87 to 1.20,
      centred on 1.02.

      Calibration ended up at about 1.3 on synthetic signal and about 0.65 on
      the recordings, against 0.53 to 0.91 centred on 0.69 before. It errs
      cautious where the noise is flat and slightly optimistic where it is
      shaped, which is the same direction the perf scenarios differ from the
      captures.

      Two measurement traps found on the way, both in the calibration tests
      rather than the code. They took the scatter inside 64-report windows
      about the window mean, which subtracts the reference term off with the
      mean -- an error in the north epoch displaces a whole run of bearings
      together -- and so measured the doppler term alone, reading 2.2 times
      low against known truth. And they divided a median over all reports by a
      median over windows; both distributions are strongly skewed on real
      signal, so that landed in the quiet part of the run and read 0.16 where
      pairing the two inside each window reads 0.6 to 0.7.
      `examples/uncertainty_reference_probe` prints all of these side by side.

      The buffer-length drift predicted from the offline table -- overstating
      at small buffers, understating at large -- does not survive the
      implementation, and is not the residual. Over 40 cells, five buffer
      lengths against two noise levels and four bearings, the ratio means
      1.07 and runs 0.83 to 1.35, and by buffer it humps rather than drifts:
      0.95, 1.16, 1.23, 1.21, 1.12 at noise 0.2 and 0.90, 1.01, 1.15, 0.99,
      1.01 at 6.5. End to end that is 0.84 to 0.89, the small buffer reading
      lower, which is the opposite direction. Most of the variation is not a
      trend at all: the four bearings within one cell spread by 0.1 to 0.2,
      about as much as the buffer axis does.

      What is real is the shortest buffer. At 128 the four bearings give 0.91
      to 1.00 and at 512 they give 1.15 to 1.29, which do not overlap, so the
      figure runs about a quarter low there. 128 samples against a 96-sample
      noise correlation length is 1.33 looks and the code clamps at 1: below
      about one correlation time the independent-looks model has nothing left
      to describe, so independence is over-counted and the figure comes out
      low. Not worth correcting yet, and correcting it would mean fitting a
      constant to flat synthetic noise, which is the thing least like the
      channel this runs on.

- [ ] The uncertainty reads about 1.3 on synthetic signal and about 0.65 on
      the recordings. That two-fold domain gap is now the dominant error in
      the figure, several times the 1.25 residual at short buffers, and it
      cannot be closed from this end: flat noise scatters a per-rotation phase
      estimate more than shaped audio does, and three captures from two radios
      is not enough to say what real interference does to it. Wants more
      recordings, ideally at known bearings.

- [x] The estimator and the highpass cutoff were thought to trade against
      each other, with an answer that depended on the noise. Measured over
      twelve independent noise realisations with common random numbers, there
      is no trade and no dependence.

      On tick timing the amplitude centroid wins where the recordings sit,
      0.0022 samples against 0.0037 at a 0.0006 RMS north channel, twelve
      times out of twelve, and by 0.0004 at 0.01. Above that the two are
      indistinguishable: at 0.05 the difference is 0.0004 samples with a
      confidence interval of -0.0007 to 0.0015, and at 0.2 it is 0.049 with an
      interval of -0.086 to 0.184.

      On bearing they are indistinguishable everywhere. The scatter difference
      is under 0.02 degrees against a scatter of 10.9, and its interval spans
      zero at every noise level.

      The reversal this item was built on -- 0.0141 against 0.0130 at 0.05 --
      does not exist. It was one noise draw, and it sat outside the spread of
      the twelve that followed.

      The recordings outrank all of the above and say the two are a tie at the
      shipped cutoff. `scripts/centroid_weighting_report.py` and
      `src/bin/north_hpf_sweep.rs` compare the estimators on the captures, and
      the latter over 121,073 ticks puts amplitude at 0.704 degrees per tick
      against energy at 0.688 -- two percent. At 5 kHz the ordering reverses
      and the gap is real, 0.664 against 1.624. Nothing recommends a change,
      so the shipped energy centroid stays.

      Worth recording how this was nearly got wrong. The synthetic result was
      briefly written up as overturning the capture measurement behind the
      shipped default, on the grounds that it too was a single draw. It was
      not: it came from the two capture harnesses above, which were missed
      because the search for them looked in `examples/` and at
      `north_hpf_sweep --help`, and they live in `scripts/` and behind no
      flag. A synthetic measurement does not overturn a capture measurement
      whatever its error bars, and the two are not even in disagreement --
      they are separated by roughly a hundredfold in tick error, because a
      real channel jitters and a generated one does not.

      The cutoff half is settled the same way. 1250 Hz looked to dominate the
      shipped 1000, never worse and 13 percent better on bearing scatter at
      0.2 RMS; across six realisations that becomes 0.604 against 0.624 with a
      seed-to-seed spread of 0.37 to 0.79, which is a draw. 1000 stays.

      Two harness defects had to be fixed before any of this could be
      measured, and they are the real result of this item. `config_sweep` had
      no way to vary the noise at all, so every row it had ever printed was
      one realisation; and the generator's seed was folded in before the
      finalizer, so nearby seeds produced correlated streams -- 0.97 between
      seeds 1 and 2. Sweeping `bearing` does not substitute for a seed: it
      redraws the doppler noise and leaves the north channel identical.

- [x] `gate_sigma` had never had a default chosen by measurement. Now it has,
      and the answer is that it does not matter: the shipped 3.0 stays.

      Where the hardware lives the gate is inert. At the 0.0006 RMS north
      channel the recordings measure, and at 0.01, sweeping 1.5 to 6.0 over
      twelve realisations moves tick error not at all -- 0.0037 samples at
      every setting, to four decimal places -- leaves bearing scatter at
      10.864 throughout, and delivers the same 9615 bearings from 2.0 upward.
      A gate that never fires has no default worth arguing about.

      Above that it does something, but not to the bearing. At 0.05 RMS a
      tighter gate loses on both counts at once, 0.0129 samples against 0.0108
      and 640 fewer bearings, so the trade this item described does not exist
      there. At 0.2 the direction reverses and tighter trends better on
      timing, but over twelve realisations that is t = -1.6 and not a result;
      6.0 is significantly worse at t = 3.1. Bearing scatter shows no
      significant dependence on the gate at any noise level tried.

      The figures this item was built on -- 0.235 samples against 0.321 at
      0.2 RMS, a twenty-seven percent win for sigma 2 -- were one realisation
      from the era when the generator's seeds were correlated. Paired over
      twelve independent ones the same comparison reads 0.528 against 0.560.
      This was the item flagged as most likely to dissolve when the seed
      defect was found, and it dissolved.

## Bearing Confidence

- [x] Decide what confidence means, and make it that. Done.
      `ConfidenceMetrics::bearing_uncertainty_deg` estimates a one-sigma
      bearing uncertainty in degrees from the spread of the phase estimates
      and the timing scatter of the reference, and confidence is now
      1 / (1 + (sigma / half) ^ 2) against a configured half-confidence
      point, six degrees by default. Signal strength became a validity gate
      for zero crossing and left the score.
      What it does not do, and cannot: see a displacement every estimate
      shares. It is precision, not accuracy. `bearing_uncertainty_test`
      asserts what it does claim -- growth as the signal degrades, and never
      reading below the scatter it describes.
      Two reductions that are correct in theory were measured and rejected,
      both of which make the figure understate: dividing the reference term
      by the loop averaging (755 ticks at the shipped bandwidth, and the
      reported tick really is 27 times better than one detection, but as the
      signal degrades the tick's error becomes a displacement the loop
      follows rather than scatter it averages away), and dividing the phase
      spread by the root of the estimate count (they share a tick, a filter
      state and an AGC gain, so they are not independent).

- [x] The zero-crossing bearing method has a systematic bias that grows with
      noise. Investigated: there is no such bias. The measurement that showed
      one was made with noise that was half DC offset.
      Every synthetic noise source in the repo built a sample as
      `(x >> 33) as u32` over `u32::MAX`, which is 31 bits divided by a
      32-bit maximum, so the range was [-1, 0) rather than [-1, 1). A
      residual DC through the doppler bandpass shifts a sinusoid's zero
      crossings, which is why the effect scaled with the noise setting, held
      steady across seeds, changed sign with the filter width, and spared the
      correlation method -- correlating against sin and cos at the tone
      frequency rejects DC.
      With the generator fixed the two methods measure within a few
      hundredths of a degree of each other at every level: 0.44 against 0.47
      at noise 0.3, 10.31 against 10.34 at noise 1.0.
      Worth remembering how it was caught. The detector, the AGC, the
      passband centre, the run length and the crossing-selection latch were
      all cleared first, and a plain sign-change scan reproduced the biased
      answer exactly -- so the bias was in the waveform, not the code reading
      it. What settled it was reimplementing the same measurement in numpy
      and getting no bias, then asking what differed about the input.

## North Tick Tracking

- [x] A slow AGC on the north channel. Done, off by default:
      `north_tick.agc.enabled`. Peak-referenced rather than RMS, because the
      pulse is a 1.2-sample event every 30 and an RMS reference would both
      demand an amplitude of 1.5 and track the rotation rate.
      One thing the design as written here got wrong. Gating adaptation purely
      on detections is correct once pulses are arriving, but a receiver quiet
      enough to need the gain is quiet enough that nothing is detected, so the
      first version could never rescue anything: it measured 0.000 detection
      with the AGC on and off alike at a pulse amplitude of 0.15. The way out
      is that a pulse train and a noise floor do not look alike -- a peak
      twenty-five times the mean absolute value against about four -- so
      before the first detection the gain adapts to the buffer peak, and only
      when the buffer looks like pulses by that measure.
      Enabling it leaves the tick count on all three captures in `data/`
      exactly unchanged, and it recovers a pulse of 0.15 that the fixed-gain
      detector misses entirely.

- [x] The north detection threshold has less margin than the sweep that chose
      it showed. Re-measured twice, and the answer changed in between.
      The first re-measurement, before the north AGC existed, found that the
      amplitude at which detection collapses tracks the threshold at about 1.6
      times it, so 0.15 detects down to a pulse of 0.25 and 0.25 only to 0.42.
      Against the 0.8 expected that is a factor of 3.2 on receiver level
      against 1.9, and it was the reason to leave the threshold alone.
      The AGC removes that cost, because it normalises the level the threshold
      meets. With it running, detection at a threshold of 0.25 holds at 0.92
      or better down to a pulse of 0.15, where before it was zero below 0.42:

        thresh\amp  1.00  0.80  0.60  0.50  0.42  0.35  0.30  0.25  0.20  0.15
        0.15        1.00  1.00  1.00  1.00  1.00  1.00  1.00  1.00  0.97  0.99
        0.25        1.00  1.00  1.00  1.00  1.00  0.98  0.92  0.99  0.99  0.99

      and the noise margin that a higher threshold buys is real:

        thresh\noise    0.00   0.05   0.10   0.20   0.30   0.40
        0.15            1.00   1.00   0.98   0.90   0.67   0.45
        0.25            1.00   1.00   0.99   0.95   0.75   0.57

      0.15 stays anyway, because the AGC is DPLL-only and the threshold is
      not. In simple mode the cliff is exactly where it was, and both 0.20 and
      0.25 fail `test_north_tick_detection_under_hum_clipping_and_drift`,
      which is a simple-mode floor. A default that suits one tracker and
      quietly costs the other its level margin is worse than one that is
      merely conservative for the first.
      For a DPLL-only deployment, 0.25 is available and is worth about a
      quarter of the detection rate at 0.3 RMS of channel noise. Whether the
      threshold should follow the tracking mode, or be expressed as a fraction
      of `expected_pulse_amplitude` now that the AGC makes that meaningful, is
      the question this leaves behind.

- [x] Decide how the detection threshold should be expressed. Done, in three
      steps, and both options in this item turned out to be right.

      It is now a fraction of the pulse height the detector expects rather
      than an absolute level. An absolute threshold met a signal that scaled
      with `gain_db` while itself staying put, so attenuation silently
      defeated detection and validation had grown a check for exactly that;
      derived, the failure cannot be expressed. The change was made on its own
      and verified to alter nothing: 96 rows across both performance
      harnesses, every accuracy and rate column, not one differing value.

      The awkward 0.19361 stays rather than rounding to 0.20. In DPLL mode the
      difference is invisible, but the simple tracker's amplitude cliff is
      steep enough that three percent crosses it -- detection at a pulse of
      0.23 falls from 0.92 to 0.47 -- and those cells carry no noise, so they
      are exact rather than a draw.

      And it follows the tracking mode after all, though the predicate is the
      AGC rather than the mode as such: a high threshold costs level margin
      and the AGC is what supplies it, so a DPLL with its AGC off gets the
      conservative value too. 0.323 where gain-controlled, 0.19361 where not.
      At 0.323 the simple tracker fails detection under hum, clipping and
      drift, 0.37 against a floor of 0.45, where the loop passes everything;
      through the system pipeline the split touches DPLL rows only and
      improves the noisy ones.

- [ ] Sixteen files carry their own copy of the synthetic noise generator, and
      they have diverged twice. The first divergence shifted the output range
      and put a DC offset carrying a seventh of the in-band energy through
      twelve of them; the second was the seed mixing, which was fixed in the
      library and in none of the copies. Neither is live now -- every
      remaining copy uses a single fixed seed, and the seed defect only bites
      when nearby seeds are compared -- so this is a hazard rather than a bug.
      `simulation::noise_at` is public for the purpose and one harness has
      been moved onto it. The rest should follow.

- [ ] Price what the highpass is for, with a capture that bleeds audio into
      the north channel. That is the only argument for filtering high, and no
      capture in `data/` exhibits it, so nothing measured so far can say
      whether 1 kHz is safe generally or only on these two radios. Needs
      hardware.

- [x] The coasting budget punishes a phase offset as though it were a rate
      error. Investigated and wrong: the budget is right, and the reasoning
      that said otherwise had the causality backwards.
      A predicted tick does advance from the last measured tick by `period`
      and never uses the oscillator's phase, so a standing phase offset looks
      like it should cost nothing. But for a second-order loop a standing
      offset is precisely the observable that says the integrator has not
      converged, which is to say the rate is still slightly wrong. At 0.5 Hz
      the rate is off by 0.0004 samples per rotation -- invisible over the
      four rotations the budget allows, and worth three samples, 35 degrees of
      bearing, over five seconds. Replacing the term with a test on the drift
      of the mean phase error let those loops coast freely and put exactly
      that error into the holdover.
      The budget is conservative rather than correct: at 0.5 Hz it prices the
      per-rotation error at 0.116 samples against a true 0.0004, so it is
      318 times short. That costs holdover only at bandwidths below 1 Hz,
      which the sweep disqualified on acquisition anyway. If it is ever worth
      tightening, the drift signal is real but sits at the noise floor of a
      128-tick window; it would need a longer window to be usable, which
      trades against how fast the budget can react to a genuine rate change.
      `test_coasting_stops_before_its_error_escapes_the_bound` now pins the
      bound itself, and fails against the change described above.

- [x] Extend the comparison to N configurations, not two. `src/bin/config_sweep.rs`
      takes any number of `--axis key=v1,v2,...` and runs the cross product,
      over configuration keys and stimulus alike. `--list-axes` lists both.
      The stimulus axes name the physical quantity rather than a knob:
      `doppler_noise` is passband noise power against the tone, which the
      recordings measure at 0.199, 0.793 and 6.579, and `north_noise` is an
      RMS against their floor of about 0.0006. That naming is the point of the
      exercise. One signal is built per distinct stimulus and shared across
      the configuration axes, so two configurations are never compared against
      different noise.

- [x] Add a mode that runs two configurations over the same signal and reports
      the difference. `src/bin/config_compare.rs`: both sides start from the
      shipped defaults and take dotted `key=value` overrides, so a comparison
      records exactly what it changed. `--list-keys` lists what it accepts.

## Bearing Calculator

- [x] Chase the residual bearing error that remains with the timing trim at
      zero. Closed: it was mostly the measurement, not the pipeline.
      Two artifacts were stacked on top of each other. The perf harness placed
      each north pulse at `round(k * period)`, up to half a sample from where
      the rotation crosses north, which is six degrees of bearing injected per
      rotation before any code ran; and both probes ran for half a second
      against a loop whose bandwidth was 1 Hz, so they spent most of their
      length acquiring and reported the transient as steady-state error.
      With pulses at their true epochs the clean scenarios fell from about ten
      degrees of mean bearing error to under two. Sweeping run length takes
      the residual from -1.07 degrees at half a second to -0.28 at two and
      about -0.2 from five seconds on, and the tracker's mean tick error over
      ten seconds falls from -0.017 samples in the first third to -0.000 in
      the last.
      The earlier split of the residual between the north tracker and the
      bearing path does not survive: it was scored against the rounded pulses,
      and there is no separate bearing-path bias to find. What remains at five
      to ten seconds is about 0.2 degrees, at the correlation floor.

# TODO

- File rotation for --dump-audio to bound disk usage on very long recordings
  (memory growth is fixed: dumps stream to disk incrementally)
- Add criterion benchmarks for DSP pipeline (FIR filters, AGC, I/Q correlation)

## Measured with sweep_config

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
      `probe_uncertainty_reference` prints all of these side by side.

      The buffer-length drift is real and monotonic, which took three
      measurements to establish. The offline table predicted the figure would
      overstate at small buffers and understate at large. A first pass with
      the `bearing` axis standing in for independent noise said there was no
      trend at all, only a hump, and that was recorded here as a correction.
      It was the wrong instrument: sweeping bearing redraws the doppler noise
      and reuses the north channel, so the cells were not independent.

      Measured properly, eight independent draws per cell across three buffer
      lengths and three noise levels, the ratio of stated to actual runs 0.92
      at 128 samples, 1.19 at 512 and 1.42 at 2048, at every noise level and
      with no overlap between the groups. Mean 1.18 over the nine cells. So
      the figure understates at short buffers and overstates at long, which is
      the opposite of what the offline table predicted, and it is a trend
      rather than scatter.

      That says the effective independence grows faster with buffer length
      than the model's buffer-over-correlation-time, not more slowly. Worth
      pinning down if this is taken further; the gap to the recordings below
      is still the larger error.

- [ ] Discuss the multipath metrics. Deferred deliberately; this records the
      shape of it so the thread is not lost.

      `resultant_length` is derived from the signal-to-noise ratio, not
      measured from per-rotation phase vectors, because those were removed
      when the uncertainty was re-derived. That is a faithful estimate of
      KR6DD's quantity under noise and misleading under a reflection, which is
      the case it would most be wanted for: the tone stays strong, so an
      SNR-derived resultant length reads high exactly where a measured one
      would read low.

      The same machinery would serve the item below. A directly measured
      resultant length is a coherence, and a bearing whose looks disagree is
      the observable that the stated uncertainty is currently missing. So
      "measure the phases" and "make confidence see multipath" are one change,
      not two, and worth deciding together rather than separately.

      What it costs is re-introducing per-look phase accumulation, which was
      taken out for good reasons: it was the thing that made the old
      uncertainty understate, and the old coherence metric read 0.99 on
      bearings tens of degrees wrong. Bringing it back to compute a different
      quantity is defensible, but it is the same code that was wrong before,
      so it wants care rather than enthusiasm.

- [ ] Confidence does not see multipath, and in an environment with
      reflections that is the error that matters. Measured with a reflected
      path 0.45 of the direct one, filtering at 0.5 confidence improves the
      median error by 5 percent while throwing away 42 percent of the
      bearings; on a clean channel the same filter improves it by 23 to 58
      percent. Rank correlation against actual error falls from 0.40 to 0.13.
      `probe_confidence_multipath` measures it.

      This follows from the derivation rather than being a defect in it: the
      figure comes from the signal-to-noise ratio, and a reflection leaves the
      tone strong while moving where it points. Noise it sees by construction;
      this it cannot.

      There is a way to catch it that does not require recognising multipath
      as such. A bearing whose recent scatter far exceeds its own stated
      uncertainty is being degraded by something the figure does not model,
      whatever that something is, and the ratio of the two is measurable
      online -- it is exactly what the calibration measures offline, where it
      reads 1.1 without a reflection and 0.66 with one. Inflating the stated
      figure by that ratio would make it self-correcting against any
      unmodelled error rather than against multipath specifically.

      Not done, because it changes what a shipped number means for the third
      time and wants deciding rather than doing.

- [ ] The every-other-pulse failure is reduced, not gone. The arbitration
      window fixed it at the noise level the pipeline gate runs at -- 0.08
      RMS, 0.995 detection over sixteen draws, no spread -- and
      `probe_agc` finds it again just above. At 0.10 RMS ten of twelve
      draws read exactly 0.50 and two read 1.00; at 0.20, eight read about
      0.45 and four about 0.90. Still bimodal, still exactly half when it
      bites.
      So the fix moved the onset rather than removing the mechanism, and the
      shipped tracker is clear of it only by margin. The two harnesses build
      their north channel differently, so part of the difference in onset may
      be construction rather than level; that is the first thing to settle.
      Simple-tracker only. The DPLL is smooth across the same sweep, and its
      AGC is worth 0.902 to 0.943 detection at 0.20 RMS with false positives
      falling 0.025 to 0.001.

- [x] The synthetic-versus-real calibration gap does not exist. Closed.

      Stated uncertainty against observed scatter read 1.18 on synthetic
      signal and 0.65 on the captures. The 0.65 is an artifact: seventy
      percent of the ft-70d capture has no carrier on it, since it was
      recorded by keying up several times while walking around the array, and
      a bearing measured on receiver hiss is a uniformly distributed number.
      Gated above 6 dB the same capture reads 1.06 to 1.08, against synthetic
      signal at 1.09.

      `bearing_uncertainty_test` now gates on carrier presence.

      The multipath model added to close the gap is kept as a synthetic stress
      case and documented as claimed of nothing; it is the only impairment
      here that makes a bearing ambiguous rather than imprecise. The
      burstiness is kept on its own evidence: the recordings' interference
      correlates 0.90 to 0.94 window to window against 0.002 for stationary
      noise.





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
      `metrics/src/bin/sweep_hpf.rs` compare the estimators on the captures, and
      the latter over 121,073 ticks puts amplitude at 0.704 degrees per tick
      against energy at 0.688 -- two percent. At 5 kHz the ordering reverses
      and the gap is real, 0.664 against 1.624. Nothing recommends a change,
      so the shipped energy centroid stays.

      Worth recording how this was nearly got wrong. The synthetic result was
      briefly written up as overturning the capture measurement behind the
      shipped default, on the grounds that it too was a single draw. It was
      not: it came from the two capture harnesses above, which were missed
      because the search for them looked in `examples/` and at
      `sweep_hpf --help`, and they live in `scripts/` and behind no
      flag. A synthetic measurement does not overturn a capture measurement
      whatever its error bars, and the two are not even in disagreement --
      they are separated by roughly a hundredfold in tick error, because a
      real channel jitters and a generated one does not.

      The cutoff half is settled the same way. 1250 Hz looked to dominate the
      shipped 1000, never worse and 13 percent better on bearing scatter at
      0.2 RMS; across six realisations that becomes 0.604 against 0.624 with a
      seed-to-seed spread of 0.37 to 0.79, which is a draw. 1000 stays.

      Two harness defects had to be fixed before any of this could be
      measured, and they are the real result of this item. `sweep_config` had
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

- [x] The simple tracker latched into detecting every other pulse. Fixed.
      The dead time blanks 96 percent of a rotation and the detector kept the
      first crossing in it, so at any real noise level a trigger arriving
      roughly once per rotation opened the window and masked the pulse behind
      it. Over sixteen draws that halved detection about one time in five,
      and the gate row read 0.894 with a standard error of 0.049 -- a value it
      never actually took.

      The simple tracker now searches the whole dead time for its largest
      sample rather than taking the first crossing. A pulse is several times
      the threshold and a trigger is barely over it, so the pulse wins
      whenever both are in the window. Detection goes to 0.995 to 0.997 with a
      standard error under 0.0004, the bimodality is gone, and mean bearing
      error improves from 67 to 60 degrees on the worst scenario.

      Shortening the dead time, which is the obvious fix, was measured first
      and rejected on numbers taken before the fix existed. Those numbers do
      not survive the fix and the rejection has to be restated: with the
      masking gone, the simple tracker prefers a shorter dead time at 0.2 RMS
      after all, monotonically, 9429 bearings and 47.4 degrees at 0.30 ms
      against 6166 and 88.6 at the shipped 0.6. Nothing separates any value at
      0.05 RMS or below.

      Shortening is still not the fix, for a better reason than the one first
      given: it does nothing at the noise levels the recordings actually show,
      it costs the DPLL -- 0.85 samples of tick error at 0.30 ms against 0.44
      at 0.6 -- and it would trade the shipped tracker's timing for a fallback
      tracker's coverage at three hundred times the measured channel noise.
      Taking the largest sample in the window costs neither.

      Deliberately not applied to the DPLL. Measured there it was slightly
      worse across the board -- tick error 0.0175 to 0.0180 on clean signal,
      bearing 0.87 to 0.90 degrees -- with nothing improved, because its
      timing gate already rejects the detections this arbitrates against and a
      wider window only lets a late noise sample outrank a clean pulse.

- [x] Sixteen files carried their own copy of the synthetic noise generator,
      and they had diverged three ways: the library's murmur finalizer, one
      missing its final shift-xor, and a bare LCG with no finalizer. Five
      copies of the jitter helper still used the coherent `sin(0.37 k)` that
      was replaced with white noise in one place only. Three were pasted
      inline rather than being functions at all. All of them now call
      `simulation::noise_at`, with a named seed constant per harness.

      None of the divergences was live, since every copy used one fixed seed,
      but two of them had already produced wrong measurements.

- [x] All three CI gates averaged their metrics over one noise draw. They now
      average over several: sixteen for the system pipeline, eight for the
      other two, pooled from raw samples where the harness keeps them so the
      percentiles still describe a distribution. Sixteen puts every row's
      standard error under 0.001 except the bimodal one above, which needs
      about a hundred draws and forty minutes to pin to 0.02 -- not worth it
      for one row, so its limits come from the measured spread instead.
      Runtime went to 52 and 76 seconds and about seven minutes.

- [x] Re-ran the conclusions that rested on the coherent jitter or on a single
      draw. One did not survive.

      The DPLL's advantage in the `noisy_jittered` pipeline scenario is gone:
      2.07 degrees against 5.92 has become 21.80 against 22.16. Nothing
      regressed. That scenario's doppler noise was raised to 0.8 to match the
      recordings, and at that level the doppler channel decides the bearing
      almost entirely, so a sample of tick jitter cannot show through it. The
      scenario stopped being limited by the thing it was being read for.

      The advantage itself is real, and larger than was ever claimed for it.
      Measured with the doppler channel quiet so the north channel limits,
      over eight draws: 2.08 degrees against 2.98 at 0.01 RMS of north noise,
      2.07 against 4.43 at 0.05, and 3.26 against 84.65 at 0.2, where the
      simple tracker also gives up a third of its bearings.

      The loop bandwidth sweep reproduces unchanged. Below 1 Hz never
      acquires; 1 Hz takes 2.78 seconds; the shipped 2 Hz takes 0.52 with full
      holdover and 0.0037 samples of steady-state tick error; above it
      acquisition keeps improving and steady state degrades monotonically, to
      0.0124 at 8 Hz. The trade and the choice both stand.

- [x] Five copies of the tick jitter helper used `sin(0.37 k)`, a coherent
      94 Hz tone rather than jitter -- forty-seven times the loop bandwidth,
      so any second-order loop rejects it by construction, and anything
      measured against it was measuring the stimulus being out of band. It had
      already earned the DPLL an advantage it had not. All five now draw from
      `simulation::noise_at`, each with its own seed, matching the system
      pipeline harness that was fixed earlier.

      The suite and all three gates pass unchanged, which is a weaker result
      than it sounds: it says the limits were not resting on the coherent
      stimulus, not that the stimulus made no difference.

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

- [x] Extend the comparison to N configurations, not two. `metrics/src/bin/sweep_config.rs`
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
      the difference. `metrics/src/bin/compare_config.rs`: both sides start from the
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

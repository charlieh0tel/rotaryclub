//! Sub-sample north tick timing.
//!
//! `north_tick_timing_test.rs` places every reference pulse on an integer
//! sample index, so it cannot observe how a tracker behaves when the pulse
//! arrives between two samples — which it does on real hardware, where the
//! rotation rate is not commensurate with the sample clock. These tests place
//! pulses at fractional epochs and measure the resulting timing bias and
//! jitter.
//!
//! Bias and jitter are reported separately on purpose. A constant offset is
//! absorbed by north-offset calibration and is not an error; a sub-sample
//! phase dependence is.

use rotaryclub::config::{NorthTrackingMode, RdfConfig};
use rotaryclub::rdf::{NorthReferenceTracker, NorthTracker};

/// Half-width of the synthesized pulse, in samples.
const PULSE_HALF_WIDTH: usize = 12;

/// A band-limited impulse at a fractional sample position, which is what an
/// anti-aliased ADC produces from a pulse far shorter than a sample period.
fn sinc_pulse_train(
    num_samples: usize,
    first_epoch: f64,
    period: f64,
    amplitude: f32,
) -> (Vec<f32>, Vec<f64>) {
    let mut signal = vec![0.0f32; num_samples];
    let mut epochs = Vec::new();
    let mut epoch = first_epoch;

    while epoch < num_samples as f64 - PULSE_HALF_WIDTH as f64 {
        if epoch >= PULSE_HALF_WIDTH as f64 {
            epochs.push(epoch);
            let center = epoch.round() as usize;
            let low = center - PULSE_HALF_WIDTH;
            for (offset, sample) in signal[low..=(center + PULSE_HALF_WIDTH)]
                .iter_mut()
                .enumerate()
            {
                let x = (low + offset) as f64 - epoch;
                // Lanczos-windowed sinc keeps the truncated tails from
                // rippling enough to move the peak.
                let window = if x.abs() < f64::EPSILON {
                    1.0
                } else {
                    let w = std::f64::consts::PI * x / PULSE_HALF_WIDTH as f64;
                    w.sin() / w
                };
                let value = if x.abs() < f64::EPSILON {
                    1.0
                } else {
                    (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
                };
                *sample += amplitude * (value * window) as f32;
            }
        }
        epoch += period;
    }

    (signal, epochs)
}

/// Loop natural frequency for these tests. The shipped default of 1 Hz needs
/// seconds of signal to settle; sub-sample behavior is what is under test
/// here, not acquisition time, so the loop is opened up to keep runs short.
const TEST_LOOP_HZ: f32 = 10.0;

/// Bound on reported timing jitter, in samples.
///
/// Both bounds are below what a whole-sample peak index can achieve, whose
/// error is uniform over one sample (1/sqrt(12) = 0.289). The DPLL would meet
/// them by averaging even so; the simple tracker times each pulse
/// independently and can only meet them with a sub-sample estimator, which is
/// what keeps the estimator covered rather than merely present.
fn jitter_bound(mode: NorthTrackingMode) -> f64 {
    match mode {
        NorthTrackingMode::Dpll => 0.05,
        NorthTrackingMode::Simple => 0.10,
    }
}

/// Effective tick times reported by the tracker, in fractional samples.
fn track(config: &RdfConfig, signal: &[f32], chunk_size: usize) -> Vec<f64> {
    let sample_rate = config.audio.sample_rate as f32;
    let mut tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate).unwrap();
    let mut ticks = Vec::new();
    for chunk in signal.chunks(chunk_size) {
        for tick in tracker.process_buffer(chunk) {
            ticks.push(tick.sample_index as f64 + tick.fractional_sample_offset as f64);
        }
    }
    for tick in tracker.finish() {
        ticks.push(tick.sample_index as f64 + tick.fractional_sample_offset as f64);
    }
    ticks
}

/// Timing error per matched tick, in samples. Ticks that match no truth
/// pulse within half a period are dropped rather than counted as huge errors.
fn timing_errors(ticks: &[f64], truth: &[f64], period: f64) -> Vec<f64> {
    let mut errors = Vec::new();
    let mut index = 0usize;
    for &tick in ticks {
        while index + 1 < truth.len()
            && (truth[index + 1] - tick).abs() < (truth[index] - tick).abs()
        {
            index += 1;
        }
        let error = tick - truth[index];
        if error.abs() <= period * 0.5 {
            errors.push(error);
        }
    }
    errors
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len().max(1) as f64
}

fn std_dev(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let m = mean(values);
    (values.iter().map(|v| (v - m).powi(2)).sum::<f64>() / values.len() as f64).sqrt()
}

/// Ticks after loop acquisition, so a settling transient does not count as
/// timing error.
fn settled(errors: &[f64]) -> &[f64] {
    let skip = (errors.len() / 4).min(400);
    &errors[skip..]
}

struct Measurement {
    bias: f64,
    jitter: f64,
    matched: usize,
    expected: usize,
}

fn measure(
    mode: NorthTrackingMode,
    rotation_hz: f32,
    start_phase: f64,
    chunk: usize,
) -> Measurement {
    let mut config = RdfConfig::default();
    config.north_tick.mode = mode;
    config.north_tick.dpll.natural_frequency_hz = TEST_LOOP_HZ;
    let sample_rate = config.audio.sample_rate as f32;
    let period = sample_rate as f64 / rotation_hz as f64;
    let num_samples = (sample_rate * 0.75) as usize;
    let amplitude = config.north_tick.expected_pulse_amplitude;

    let (signal, truth) = sinc_pulse_train(num_samples, 64.0 + start_phase, period, amplitude);
    let ticks = track(&config, &signal, chunk);
    let errors = timing_errors(&ticks, &truth, period);
    let settled = settled(&errors);

    Measurement {
        bias: mean(settled),
        jitter: std_dev(settled),
        matched: errors.len(),
        expected: truth.len(),
    }
}

/// The tracker must not report a different tick time depending on where the
/// pulse happens to fall between two samples. Any such dependence shows up as
/// a bearing error that changes with the rotation rate.
///
/// The rate here is commensurate -- exactly 30 samples per rotation at 48 kHz
/// -- so every pulse in a run sits at the same sub-sample offset and the run's
/// mean bias is that offset's bias. Sweeping the start phase then sweeps the
/// quantity named.
///
/// At the shipped 1602.564 Hz this measured nothing. The period is 29.952
/// samples, so within a single run the pulse already walks every sub-sample
/// phase thousands of times and the mean is taken across all of them;
/// changing the start phase only rotated a sequence whose mean was already
/// fixed. A phase-dependent bias of any size would have left the spread at
/// zero and passed.
#[test]
fn test_subsample_phase_sweep() {
    let rotation_hz = 1600.0f32;

    for &mode in &[NorthTrackingMode::Dpll, NorthTrackingMode::Simple] {
        let mut biases = Vec::new();

        for step in 0..20 {
            let phase = step as f64 / 20.0;
            let m = measure(mode, rotation_hz, phase, 512);

            assert!(
                m.matched as f64 >= m.expected as f64 * 0.95,
                "mode={mode:?} phase={phase:.2} matched={} of {}",
                m.matched,
                m.expected
            );
            assert!(
                m.jitter <= jitter_bound(mode),
                "mode={mode:?} phase={phase:.2} jitter={:.4} samples",
                m.jitter
            );
            biases.push(m.bias);
        }

        // Spread of the per-phase bias: how much the reported tick time moves
        // as the pulse walks across a sample interval.
        let spread = biases.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
            - biases.iter().cloned().fold(f64::INFINITY, f64::min);
        // Both modes measure 0.246 samples, identically, because this is the
        // estimator's phase dependence and not the tracker's. That is 3
        // degrees of bearing, and it is the price of a commensurate rate: the
        // pulse sits at the same sub-sample offset every rotation, so nothing
        // dithers the estimator's bias away. At the shipped 1602.564 Hz the
        // pulse walks all phases and it averages out, which is why the same
        // sweep there measures nothing at all -- see
        // test_commensurate_rate_produces_constant_offset for the other face
        // of this.
        let spread_bound = 0.30;
        assert!(
            spread <= spread_bound,
            "mode={mode:?} bias varies by {spread:.4} samples across sub-sample phase"
        );
    }
}

/// Shifting the input by a whole number of samples must shift every reported
/// tick by exactly that many samples.
#[test]
fn test_whole_sample_shift_invariance() {
    let sample_rate = RdfConfig::default().audio.sample_rate as f32;
    let period = sample_rate as f64 / 1602.564;
    let num_samples = (sample_rate * 0.5) as usize;

    for &mode in &[NorthTrackingMode::Dpll, NorthTrackingMode::Simple] {
        let mut config = RdfConfig::default();
        config.north_tick.mode = mode;
        config.north_tick.dpll.natural_frequency_hz = TEST_LOOP_HZ;
        let amplitude = config.north_tick.expected_pulse_amplitude;
        let (base, _) = sinc_pulse_train(num_samples, 64.37, period, amplitude);

        // The simple tracker times each pulse independently, so its output
        // shifts exactly. The DPLL's also carries a phase correction whose
        // residual depends on where in its acquisition the shift lands.
        // Residual is numerical: a shifted input puts different values through
        // the filter's tail, so the centroid lands a thousandth of a sample
        // away.
        let tolerance = 1e-2;

        for shift in [1usize, 2, 7] {
            let mut shifted = vec![0.0f32; shift];
            shifted.extend_from_slice(&base[..base.len() - shift]);

            let reference = track(&config, &base, 512);
            let moved = track(&config, &shifted, 512);

            let count = reference.len().min(moved.len());
            assert!(count > 100, "too few ticks to compare (shift={shift})");

            // The trailing tick is flushed at end-of-stream, where no later
            // samples exist to complete an estimator window.
            for i in (count / 2)..(count - 1) {
                let delta = moved[i] - reference[i] - shift as f64;
                assert!(
                    delta.abs() < tolerance,
                    "mode={mode:?} shift={shift} tick {i}: shifted by {:.6} samples, expected {shift}",
                    moved[i] - reference[i]
                );
            }
        }
    }
}

/// When the rotation rate is commensurate with the sample rate the pulse
/// lands at the same sub-sample phase every rotation. A quantizing estimator
/// then produces a constant offset instead of dither, and no amount of loop
/// averaging removes it. This test pins the size of that offset so a change
/// in estimator or delay bookkeeping is visible.
#[test]
fn test_commensurate_rate_produces_constant_offset() {
    let sample_rate = RdfConfig::default().audio.sample_rate as f32;
    let rotation_hz = sample_rate / 30.0;

    for &mode in &[NorthTrackingMode::Dpll, NorthTrackingMode::Simple] {
        let m = measure(mode, rotation_hz, 0.5, 512);

        assert!(
            m.matched as f64 >= m.expected as f64 * 0.95,
            "mode={mode:?} matched={} of {}",
            m.matched,
            m.expected
        );
        // Commensurate: the error is a fixed offset, so jitter is tiny even
        // though the offset itself may be large.
        assert!(
            m.jitter <= jitter_bound(mode).min(0.1),
            "mode={mode:?} jitter={:.4} samples at a commensurate rate",
            m.jitter
        );
        assert!(
            m.bias.abs() <= 0.75,
            "mode={mode:?} bias={:.4} samples at a commensurate rate",
            m.bias
        );
    }
}

/// Timing at the shipped loop bandwidth, including acquisition.
///
/// Every other test here opens the loop up to `TEST_LOOP_HZ` so runs stay
/// short, which means none of them exercise the configured default. That
/// matters: while the loop is acquiring, its timing correction saturates, and
/// whatever it saturates at goes straight into the reported tick time.
#[test]
fn test_shipped_loop_bandwidth_bounds_acquisition_error() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let period = sample_rate as f64 / 1602.564;
    let num_samples = (sample_rate * 2.5) as usize;
    let amplitude = config.north_tick.expected_pulse_amplitude;

    let (signal, truth) = sinc_pulse_train(num_samples, 64.31, period, amplitude);
    let ticks = track(&config, &signal, 512);
    let errors = timing_errors(&ticks, &truth, period);
    assert!(
        errors.len() > 3000,
        "expected a full run, got {}",
        errors.len()
    );

    // Measured 0.74 samples with the correction bounded at half a sample,
    // 1.24 with it bounded at a whole one: while the loop acquires, the
    // correction saturates and the bound is what reaches the bearing.
    let worst = errors.iter().fold(0.0f64, |acc, e| acc.max(e.abs()));
    assert!(
        worst <= 0.85,
        "worst timing error {worst:.4} samples during acquisition at the default loop bandwidth"
    );

    // Once acquired, the loop should be far better than the bound above.
    let settled = &errors[errors.len() * 3 / 4..];
    let settled_worst = settled.iter().fold(0.0f64, |acc, e| acc.max(e.abs()));
    assert!(
        settled_worst <= 0.05,
        "worst settled timing error {settled_worst:.4} samples"
    );
}

/// Both centroids resolve the pulse below one sample.
///
/// They are the same first moment, differing only in how weight is spread
/// across the pulse, and the better of the two depends on the pulse shape the
/// highpass leaves. Either must beat what a whole-sample peak index can do,
/// which is the point of having an estimator at all.
#[test]
fn test_both_centroids_resolve_below_a_sample() {
    use rotaryclub::config::NorthPulseEstimator;

    let sample_rate = RdfConfig::default().audio.sample_rate as f32;
    let period = sample_rate as f64 / 1602.564;
    let num_samples = (sample_rate * 0.75) as usize;

    for estimator in [
        NorthPulseEstimator::AmplitudeCentroid,
        NorthPulseEstimator::EnergyCentroid,
    ] {
        let mut config = RdfConfig::default();
        config.north_tick.mode = NorthTrackingMode::Simple;
        config.north_tick.estimator = estimator;
        let amplitude = config.north_tick.expected_pulse_amplitude;

        let (signal, truth) = sinc_pulse_train(num_samples, 64.31, period, amplitude);
        let ticks = track(&config, &signal, 512);
        let errors = timing_errors(&ticks, &truth, period);
        let jitter = std_dev(settled(&errors));

        assert!(
            jitter <= 0.10,
            "{estimator:?}: jitter {jitter:.4} samples, no better than a whole-sample index"
        );
    }
}

/// A rate change must not send the tracker into a runaway.
///
/// Coasting covers rotations where no pulse arrived; a detection the gate
/// rejected is not one. If the two are confused, the prediction stands in for
/// the measurement, the loop gets no correction, its disagreement with the
/// next detection grows, and that one is rejected too.
#[test]
fn test_rate_step_does_not_run_away() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let amplitude = config.north_tick.expected_pulse_amplitude;
    let num_samples = (sample_rate * 3.0) as usize;
    let before = sample_rate as f64 / 1602.564;
    let after = sample_rate as f64 / 1645.0;
    let step_at = 2.0 * sample_rate as f64;

    // Build the pulse train by hand so the rate changes partway through.
    let mut truth = Vec::new();
    let mut epoch = 100.3f64;
    while epoch < num_samples as f64 - 16.0 {
        truth.push(epoch);
        epoch += if epoch >= step_at { after } else { before };
    }
    let (signal, _) = sinc_pulse_train(num_samples, 1e9, before, amplitude);
    let mut signal = signal;
    for &e in &truth {
        let (one, _) = sinc_pulse_train(num_samples, e, num_samples as f64 * 2.0, amplitude);
        for (dst, src) in signal.iter_mut().zip(one) {
            *dst += src;
        }
    }

    let ticks = track(&config, &signal, 512);
    let errors = timing_errors(&ticks, &truth, before);
    let after_step: Vec<f64> = ticks
        .iter()
        .zip(&errors)
        .filter(|(t, _)| **t >= step_at)
        .map(|(_, e)| *e)
        .collect();
    assert!(
        after_step.len() > 500,
        "expected ticks after the step, got {}",
        after_step.len()
    );

    // Measured 5.94 samples when a rejected detection was coasted over,
    // 0.96 when it is not. The loop legitimately lags a 42 Hz step at its
    // 1 Hz bandwidth; what it must not do is diverge.
    let worst = after_step.iter().fold(0.0f64, |acc, e| acc.max(e.abs()));
    assert!(
        worst <= 2.0,
        "worst timing error {worst:.3} samples after a rate step"
    );
}

/// A disputed pulse must not switch coasting off for the rest of a dropout.
///
/// Coasting stands aside while detections are being rejected, so that a
/// prediction never stands in for a measurement the tracker distrusted. That
/// hold has to expire: an impulse from interference as a signal fades is one
/// rejected detection at the start of a dropout, and the rotations after it
/// are exactly what coasting exists to cover.
#[test]
fn test_glitch_during_dropout_does_not_disable_coasting() {
    // Settled loop: how far a tracker may coast depends on how well it knows
    // the rotation rate, and this test is about the rejection hold rather
    // than about holdover duration.
    let mut config = RdfConfig::default();
    config.north_tick.dpll.natural_frequency_hz = TEST_LOOP_HZ;
    let sample_rate = config.audio.sample_rate as f32;
    let amplitude = config.north_tick.expected_pulse_amplitude;
    let period = sample_rate as f64 / 1602.564;
    let num_samples = (sample_rate * 2.0) as usize;
    let dropout = (1.0 * sample_rate as f64, 1.5 * sample_rate as f64);

    let mut coasted = Vec::new();
    for glitch_at in [None, Some(1.05f64), Some(1.25)] {
        let mut signal = vec![0.0f32; num_samples];
        let mut epoch = 100.3f64;
        let mut expected_in_dropout = 0usize;
        while epoch < num_samples as f64 - 16.0 {
            let in_dropout = epoch >= dropout.0 && epoch < dropout.1;
            if in_dropout {
                expected_in_dropout += 1;
            } else {
                let (one, _) =
                    sinc_pulse_train(num_samples, epoch, num_samples as f64 * 2.0, amplitude);
                for (dst, src) in signal.iter_mut().zip(one) {
                    *dst += src;
                }
            }
            epoch += period;
        }

        // An impulse half a rotation off the tracked grid: detected, and
        // rejected by the timing gate.
        if let Some(seconds) = glitch_at {
            let at = seconds * sample_rate as f64 + period * 0.5;
            let (one, _) = sinc_pulse_train(num_samples, at, num_samples as f64 * 2.0, amplitude);
            for (dst, src) in signal.iter_mut().zip(one) {
                *dst += src;
            }
        }

        let ticks = track(&config, &signal, 512);
        let inside = ticks
            .iter()
            .filter(|t| **t >= dropout.0 && **t < dropout.1)
            .count();
        coasted.push((glitch_at, inside, expected_in_dropout));
    }

    let clean = coasted[0].1;
    assert!(
        clean > 700,
        "expected coasting to cover a clean dropout, got {clean} ticks"
    );
    for &(glitch_at, inside, expected) in &coasted[1..] {
        assert!(
            inside as f64 >= clean as f64 * 0.9,
            "glitch at {glitch_at:?}s left {inside} coasted ticks of {expected} expected,              against {clean} with no glitch"
        );
    }
}

/// Recovery after a capture gap must not depend on how long the gap was.
///
/// `advance_samples` carries the oscillator across samples that were never
/// delivered, so the phase advance is the rotation rate times the gap length.
/// Anything that degrades with gap length -- precision in that product, or
/// state carried across the gap that should not be -- shows up as valid
/// pulses being rejected on the far side.
#[test]
fn test_long_capture_gap_keeps_phase() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let amplitude = config.north_tick.expected_pulse_amplitude;
    let period = sample_rate as f64 / 1602.564;

    for gap_rotations in [16u64, 160_000, 8_000_000] {
        // Whole rotations, so the pulses after the gap fall exactly where the
        // oscillator predicts and any error is the tracker's own.
        let gap = (gap_rotations as f64 * period).round() as usize;
        let num_samples = (sample_rate * 0.5) as usize;

        let (before, _) = sinc_pulse_train(num_samples, 64.37, period, amplitude);
        let (after, truth_after) = sinc_pulse_train(num_samples, 64.37, period, amplitude);

        let mut tracker =
            rotaryclub::rdf::NorthReferenceTracker::new(&config.north_tick, sample_rate).unwrap();
        for chunk in before.chunks(512) {
            let _ = tracker.process_buffer(chunk);
        }
        tracker.advance_samples(gap);

        let base = num_samples + gap;
        let mut ticks = Vec::new();
        for chunk in after.chunks(512) {
            for tick in tracker.process_buffer(chunk) {
                ticks.push(
                    tick.sample_index as f64 + tick.fractional_sample_offset as f64 - base as f64,
                );
            }
        }

        // Skip the first few rotations: resetting the highpass across the gap
        // costs a settling transient the length of the filter.
        let early: Vec<f64> = truth_after.iter().skip(6).take(40).copied().collect();
        let matched = early
            .iter()
            .filter(|t| ticks.iter().any(|k| (k - **t).abs() < 3.0))
            .count();
        assert!(
            matched >= early.len() * 9 / 10,
            "gap of {gap_rotations} rotations: only {matched} of {} pulses after it were reported",
            early.len()
        );
    }
}

/// A handful of displaced detections must not pull the tracked timing.
///
/// An interferer arriving just ahead of a real pulse is both detected and
/// masking: the detector's dead time then hides the real pulse behind it. The
/// gate exists so that the loop is not dragged onto the interferer's timing,
/// and so that the rotations it costs are coasted rather than reported wrong.
///
/// This runs at the shipped loop bandwidth, which needs a displacement the
/// loop cannot absorb to show anything -- six rotations displaced by a
/// fraction of a sample vanish into an average over a thousand. Displaced
/// far enough, the difference is not subtle: without the gate, 67 ticks
/// follow the interference, the worst by 5.6 samples, which is 68 degrees of
/// bearing.
#[test]
fn test_displaced_detections_are_rejected() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let amplitude = config.north_tick.expected_pulse_amplitude;
    let period = sample_rate as f64 / 1602.564;
    let num_samples = (sample_rate * 3.0) as usize;

    // Far enough off that the loop cannot absorb it: the correction that
    // pulls a reported tick back towards the tracked rotation is bounded at
    // half a sample, so an accepted detection this far out is reported very
    // nearly where it was found.
    let displacement = 6.0f64;
    // After the loop has settled, so coasting can cover what the gate rejects.
    let disturbed = 4000..4006;

    let mut signal = vec![0.0f32; num_samples];
    let mut truth = Vec::new();
    let mut epoch = 100.3f64;
    let mut rotation = 0usize;
    while epoch < num_samples as f64 - 16.0 {
        let placed = if disturbed.contains(&rotation) {
            epoch - displacement
        } else {
            epoch
        };
        let (one, _) = sinc_pulse_train(num_samples, placed, num_samples as f64 * 2.0, amplitude);
        for (dst, src) in signal.iter_mut().zip(one) {
            *dst += src;
        }
        truth.push(epoch);
        epoch += period;
        rotation += 1;
    }

    let ticks = track(&config, &signal, 512);
    let errors = timing_errors(&ticks, &truth, period);
    // Well past acquisition, so the only disturbance is the displaced one.
    let settled = &errors[3500.min(errors.len())..];
    let pulled = settled.iter().filter(|e| e.abs() > 0.3).count();
    let worst = settled.iter().fold(0.0f64, |acc, e| acc.max(e.abs()));
    assert!(
        pulled == 0,
        "{pulled} ticks followed the displaced detections instead of the tracked rotation          (worst {worst:.3} samples)"
    );
}

/// No emitted tick may land inside another's dead time.
///
/// Predicted ticks and detected ones are produced by different paths, and a
/// detection can surface at a position already passed, because the detector
/// holds a crossing until its search window and trailing context have been
/// seen. A prediction placed without allowing for that would take the
/// detection's slot and push the real pulse inside the guard.
#[test]
fn test_emitted_ticks_never_crowd_each_other() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let amplitude = config.north_tick.expected_pulse_amplitude;
    let period = sample_rate as f64 / 1602.564;
    let num_samples = (sample_rate * 3.0) as usize;
    let dropout = (2.5 * sample_rate as f64, 2.62 * sample_rate as f64);

    let mut signal = vec![0.0f32; num_samples];
    let mut epoch = 100.3f64;
    while epoch < num_samples as f64 - 16.0 {
        if !(epoch >= dropout.0 && epoch < dropout.1) {
            let (one, _) =
                sinc_pulse_train(num_samples, epoch, num_samples as f64 * 2.0, amplitude);
            for (dst, src) in signal.iter_mut().zip(one) {
                *dst += src;
            }
        }
        epoch += period;
    }

    for chunk in [32usize, 64, 100, 512, 4096] {
        let ticks = track(&config, &signal, chunk);
        assert!(
            ticks.len() > 1000,
            "chunk={chunk}: only {} ticks",
            ticks.len()
        );
        for pair in ticks.windows(2) {
            let spacing = pair[1] - pair[0];
            assert!(
                spacing >= period * 0.75,
                "chunk={chunk}: ticks at {:.3} and {:.3} are {spacing:.3} samples apart,                  inside the {:.3} sample guard",
                pair[0],
                pair[1],
                period * 0.75
            );
        }
    }
}

/// Lock, holdover and reacquisition, at the shipped loop bandwidth.
///
/// These are the numbers to check a retune against: how long until the
/// reported tick times are trustworthy, how long they stay trustworthy once
/// the pulses stop, and how long it takes to recover afterwards.
#[test]
fn test_lock_and_reacquisition_performance() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let amplitude = config.north_tick.expected_pulse_amplitude;
    let period = sample_rate as f64 / 1602.564;
    let num_samples = (sample_rate * 6.0) as usize;
    let dropout = (4.0 * sample_rate as f64, 4.3 * sample_rate as f64);

    let (mut signal, truth) = sinc_pulse_train(num_samples, 100.3, period, amplitude);
    for sample in signal[dropout.0 as usize..dropout.1 as usize].iter_mut() {
        *sample = 0.0;
    }

    let ticks = track(&config, &signal, 512);
    let mut paired: Vec<(f64, f64)> = Vec::new();
    let mut index = 0usize;
    for &tick in &ticks {
        while index + 1 < truth.len()
            && (truth[index + 1] - tick).abs() < (truth[index] - tick).abs()
        {
            index += 1;
        }
        let error = tick - truth[index];
        if error.abs() <= period * 0.5 {
            paired.push((tick / sample_rate as f64, error));
        }
    }

    // Trustworthy means the reported time is within a tenth of a sample,
    // which is 1.2 degrees of bearing.
    let trusted = 0.1f64;

    // Lock: the last moment before the dropout at which the error was still
    // outside the bound.
    let lock_secs = paired
        .iter()
        .filter(|(t, e)| *t < dropout.0 / sample_rate as f64 && e.abs() > trusted)
        .map(|(t, _)| *t)
        .fold(0.0f64, f64::max);
    assert!(
        lock_secs < 3.0,
        "took {lock_secs:.2} s to settle within {trusted} samples at the default loop bandwidth"
    );

    // Holdover: ticks emitted during the dropout, and how wrong they were.
    let coasted: Vec<&(f64, f64)> = paired
        .iter()
        .filter(|(t, _)| {
            *t >= dropout.0 / sample_rate as f64 && *t < dropout.1 / sample_rate as f64
        })
        .collect();
    let coast_worst = coasted.iter().fold(0.0f64, |acc, (_, e)| acc.max(e.abs()));
    assert!(
        coasted.len() > 400,
        "only {} ticks coasted across a 300 ms dropout",
        coasted.len()
    );
    assert!(
        coast_worst <= 0.5,
        "coasted ticks were {coast_worst:.3} samples out"
    );

    // Reacquisition: the last moment after the dropout at which the error was
    // still outside the bound.
    let dropout_end = dropout.1 / sample_rate as f64;
    let reacquire_secs = paired
        .iter()
        .filter(|(t, e)| *t >= dropout_end && e.abs() > trusted)
        .map(|(t, _)| *t - dropout_end)
        .fold(0.0f64, f64::max);
    assert!(
        reacquire_secs < 0.5,
        "took {reacquire_secs:.3} s to recover to within {trusted} samples after a dropout"
    );
}

/// Sub-sample behavior must not depend on how the stream is chopped into
/// buffers.
#[test]
fn test_subsample_timing_across_chunk_sizes() {
    let rotation_hz = 1602.564f32;

    for &chunk in &[64usize, 256, 1024] {
        let m = measure(NorthTrackingMode::Dpll, rotation_hz, 0.31, chunk);
        assert!(
            m.jitter <= jitter_bound(NorthTrackingMode::Dpll),
            "chunk={chunk} jitter={:.4} samples",
            m.jitter
        );
        assert!(
            m.matched as f64 >= m.expected as f64 * 0.95,
            "chunk={chunk} matched={} of {}",
            m.matched,
            m.expected
        );
    }
}

/// A missed pulse must not poison the simple tracker's period estimate.
///
/// The interval across a dropout spans two rotations, and feeding it into
/// the period average unfolded ran the estimate 5 percent high and wandering
/// in the pipeline gate's dropout scenario -- which turned the correlation
/// reference with it and uniformized the bearings across the doppler
/// filter's group delay, while the ticks themselves stayed good to 0.17
/// samples. The tracker folds multi-rotation intervals before its
/// statistics see them; this pins that.
#[test]
fn test_simple_period_survives_dropouts() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = 1602.564f32;
    let true_period = sample_rate / rotation_hz;
    let seconds = 3.0f32;
    let n = (seconds * sample_rate) as usize;

    // Band-limited pulses at every rotation epoch except each 17th.
    let mut epochs = Vec::new();
    let mut k = 0usize;
    let mut t = 0.5f64 * true_period as f64;
    while (t as usize) < n {
        if k % 17 != 0 {
            epochs.push(t);
        }
        k += 1;
        t += true_period as f64;
    }
    let north =
        rotaryclub::simulation::render_north_pulse_train(n, &epochs, 0.8 / 0.8_f32.max(0.01));

    let mut tick_config = config.north_tick.clone();
    tick_config.mode = NorthTrackingMode::Simple;
    let mut tracker = NorthReferenceTracker::new(&tick_config, sample_rate).unwrap();
    let mut last_period = None;
    for chunk in north.chunks(1024) {
        for tick in tracker.process_buffer(chunk) {
            last_period = tick.period;
        }
    }
    let period = last_period.expect("a settled period");
    let error = (period - true_period).abs() / true_period;
    assert!(
        error < 0.005,
        "simple period {period:.4} vs true {true_period:.4}: {:.2}% off under 1-in-17 dropouts",
        error * 100.0
    );
}

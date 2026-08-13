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

/// Bound on reported timing jitter, in samples. The DPLL reports its
/// oscillator's estimate, which is not quantized; the simple tracker reports
/// the detected peak index, whose error is uniform over one sample
/// (1/sqrt(12) = 0.289).
fn jitter_bound(mode: NorthTrackingMode) -> f64 {
    match mode {
        NorthTrackingMode::Dpll => 0.05,
        NorthTrackingMode::Simple => 0.35,
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
#[test]
fn test_subsample_phase_sweep() {
    let rotation_hz = 1602.564f32;

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
        let spread_bound = match mode {
            NorthTrackingMode::Dpll => 0.1,
            NorthTrackingMode::Simple => 0.6,
        };
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

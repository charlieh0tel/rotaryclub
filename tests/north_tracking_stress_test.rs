use std::f32::consts::PI;

use rotaryclub::config::{NorthTrackingMode, RdfConfig};
use rotaryclub::rdf::{NorthReferenceTracker, NorthTick, NorthTracker};

#[derive(Debug, Clone)]
struct DetectionMetrics {
    detection_rate: f32,
    false_positive_rate: f32,
    mean_abs_timing_error_samples: f32,
    p95_abs_timing_error_samples: f32,
}

#[derive(Debug, Clone)]
struct StepResponseMetrics {
    pre_step_mean_hz: f32,
    post_step_mean_hz: f32,
    settle_time_secs: Option<f32>,
    max_abs_error_after_step_hz: f32,
}

#[derive(Debug, Clone, Copy)]
struct StepResponseEvalConfig {
    pre_window: (f32, f32),
    post_window: (f32, f32),
    target_post_hz: f32,
    settle_band_hz: f32,
    settle_consecutive_ticks: usize,
}

fn deterministic_jitter_samples(index: usize, max_abs_jitter: i32) -> i32 {
    if max_abs_jitter <= 0 {
        0
    } else {
        ((index as f32 * 0.37).sin() * max_abs_jitter as f32).round() as i32
    }
}

fn generate_pulse_positions<F, G>(
    start_time_secs: f32,
    duration_secs: f32,
    sample_rate: f32,
    mut freq_hz_at_time: F,
    mut keep_pulse_at_time: G,
    jitter_samples: i32,
) -> Vec<usize>
where
    F: FnMut(f32) -> f32,
    G: FnMut(f32) -> bool,
{
    let num_samples = (duration_secs * sample_rate) as usize;
    let mut positions = Vec::new();
    let mut t = start_time_secs;
    let mut pulse_index = 0usize;

    while t < duration_secs {
        let freq_hz = freq_hz_at_time(t).max(1.0);
        if keep_pulse_at_time(t) {
            let jitter = deterministic_jitter_samples(pulse_index, jitter_samples) as isize;
            let idx = (t * sample_rate).round() as isize + jitter;
            if idx >= 0 && (idx as usize) < num_samples {
                positions.push(idx as usize);
            }
        }
        t += 1.0 / freq_hz;
        pulse_index += 1;
    }

    positions.sort_unstable();
    positions.dedup();
    positions
}

fn build_north_signal(num_samples: usize, pulse_positions: &[usize], amplitude: f32) -> Vec<f32> {
    let mut signal = vec![0.0f32; num_samples];
    for &idx in pulse_positions {
        if idx < signal.len() {
            signal[idx] = amplitude;
        }
    }
    signal
}

fn run_north_tracker(config: &RdfConfig, north_signal: &[f32]) -> (Vec<NorthTick>, Option<f32>) {
    let sample_rate = config.audio.sample_rate as f32;
    let mut tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate).unwrap();
    let chunk_size = config.audio.buffer_size;
    let mut ticks = Vec::new();
    for chunk in north_signal.chunks(chunk_size) {
        ticks.extend(tracker.process_buffer(chunk));
    }
    (ticks, tracker.rotation_frequency())
}

fn percentile(values: &[f32], p: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    let idx = ((sorted.len() as f32 - 1.0) * p.clamp(0.0, 1.0)).round() as usize;
    sorted[idx]
}

fn detection_metrics(
    expected_pulses: &[usize],
    ticks: &[NorthTick],
    match_tolerance_samples: f32,
) -> DetectionMetrics {
    let expected: Vec<f32> = expected_pulses.iter().map(|&s| s as f32).collect();
    let detected: Vec<f32> = ticks
        .iter()
        .map(|tick| tick.sample_index as f32 + tick.fractional_sample_offset)
        .collect();

    let mut i = 0usize;
    let mut j = 0usize;
    let mut matched = 0usize;
    let mut errors = Vec::new();

    while i < expected.len() && j < detected.len() {
        let exp = expected[i];
        let det = detected[j];
        let err = (det - exp).abs();
        if err <= match_tolerance_samples {
            matched += 1;
            errors.push(err);
            i += 1;
            j += 1;
        } else if det < exp {
            j += 1;
        } else {
            i += 1;
        }
    }

    let expected_len = expected.len().max(1) as f32;
    let unmatched_detections = detected.len().saturating_sub(matched);
    DetectionMetrics {
        detection_rate: matched as f32 / expected_len,
        false_positive_rate: unmatched_detections as f32 / expected_len,
        mean_abs_timing_error_samples: mean(&errors).unwrap_or(0.0),
        p95_abs_timing_error_samples: percentile(&errors, 0.95),
    }
}

fn step_response_metrics(
    ticks: &[NorthTick],
    sample_rate: f32,
    step_time_secs: f32,
    eval: StepResponseEvalConfig,
) -> StepResponseMetrics {
    let tick_points: Vec<(f32, f32)> = ticks
        .iter()
        .map(|tick| {
            (
                tick.sample_index as f32 / sample_rate,
                tick_hz(tick, sample_rate),
            )
        })
        .collect();

    let pre_hz: Vec<f32> = tick_points
        .iter()
        .filter_map(|(t, hz)| {
            if *t > eval.pre_window.0 && *t < eval.pre_window.1 {
                Some(*hz)
            } else {
                None
            }
        })
        .collect();
    let post_hz: Vec<f32> = tick_points
        .iter()
        .filter_map(|(t, hz)| {
            if *t > eval.post_window.0 && *t < eval.post_window.1 {
                Some(*hz)
            } else {
                None
            }
        })
        .collect();

    let mut in_band_run = 0usize;
    let mut settle_time_secs = None;
    for (t, hz) in tick_points.iter().filter(|(t, _)| *t >= step_time_secs) {
        if (*hz - eval.target_post_hz).abs() <= eval.settle_band_hz {
            in_band_run += 1;
            if in_band_run >= eval.settle_consecutive_ticks {
                settle_time_secs = Some(*t - step_time_secs);
                break;
            }
        } else {
            in_band_run = 0;
        }
    }

    let max_abs_error_after_step_hz = tick_points
        .iter()
        .filter_map(|(t, hz)| {
            if *t >= step_time_secs {
                Some((hz - eval.target_post_hz).abs())
            } else {
                None
            }
        })
        .fold(0.0f32, f32::max);

    StepResponseMetrics {
        pre_step_mean_hz: mean(&pre_hz).unwrap_or(0.0),
        post_step_mean_hz: mean(&post_hz).unwrap_or(0.0),
        settle_time_secs,
        max_abs_error_after_step_hz,
    }
}

fn mean(values: &[f32]) -> Option<f32> {
    if values.is_empty() {
        None
    } else {
        Some(values.iter().sum::<f32>() / values.len() as f32)
    }
}

fn tick_hz(tick: &NorthTick, sample_rate: f32) -> f32 {
    tick.frequency * sample_rate / (2.0 * PI)
}

#[test]
fn test_north_tracking_amplitude_sweep() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;
    let duration_secs = 1.0;
    let start_time_secs = 0.05;
    let num_samples = (duration_secs * sample_rate) as usize;

    let pulse_positions = generate_pulse_positions(
        start_time_secs,
        duration_secs,
        sample_rate,
        |_| rotation_hz,
        |_| true,
        0,
    );
    for amplitude in [0.35f32, 0.5, 0.8, 1.2] {
        let north_signal = build_north_signal(num_samples, &pulse_positions, amplitude);
        let (ticks, freq_opt) = run_north_tracker(&config, &north_signal);
        let metrics = detection_metrics(&pulse_positions, &ticks, 3.0);

        assert!(
            metrics.detection_rate >= 0.90,
            "Amplitude {:.2}: detection rate {:.2} too low (expected {})",
            amplitude,
            metrics.detection_rate,
            pulse_positions.len()
        );
        assert!(
            metrics.false_positive_rate <= 0.05,
            "Amplitude {:.2}: false positive rate {:.2} too high",
            amplitude,
            metrics.false_positive_rate
        );
        assert!(
            metrics.mean_abs_timing_error_samples <= 1.2,
            "Amplitude {:.2}: mean timing error {:.2} samples too high",
            amplitude,
            metrics.mean_abs_timing_error_samples
        );
        assert!(
            metrics.p95_abs_timing_error_samples <= 2.5,
            "Amplitude {:.2}: p95 timing error {:.2} samples too high",
            amplitude,
            metrics.p95_abs_timing_error_samples
        );

        let freq = freq_opt.expect("Expected rotation frequency estimate");
        assert!(
            (freq - rotation_hz).abs() < 80.0,
            "Amplitude {:.2}: frequency {:.1}Hz too far from {:.1}Hz",
            amplitude,
            freq,
            rotation_hz
        );
    }
}

#[test]
fn test_north_tracking_threshold_sweep() {
    let base_config = RdfConfig::default();
    let sample_rate = base_config.audio.sample_rate as f32;
    let rotation_hz = base_config.doppler.expected_freq;
    let duration_secs = 1.0;
    let start_time_secs = 0.05;
    let num_samples = (duration_secs * sample_rate) as usize;

    let pulse_positions = generate_pulse_positions(
        start_time_secs,
        duration_secs,
        sample_rate,
        |_| rotation_hz,
        |_| true,
        0,
    );
    let north_signal = build_north_signal(num_samples, &pulse_positions, 0.8);

    for threshold in [0.08f32, 0.12, 0.15, 0.20, 0.25] {
        let mut config = base_config.clone();
        config.north_tick.threshold = threshold;
        let (ticks, freq_opt) = run_north_tracker(&config, &north_signal);
        let metrics = detection_metrics(&pulse_positions, &ticks, 3.0);

        assert!(
            metrics.detection_rate >= 0.88,
            "Threshold {:.2}: detection rate {:.2} too low",
            threshold,
            metrics.detection_rate
        );
        assert!(
            metrics.false_positive_rate <= 0.08,
            "Threshold {:.2}: false positive rate {:.2} too high",
            threshold,
            metrics.false_positive_rate
        );
        assert!(
            metrics.p95_abs_timing_error_samples <= 3.0,
            "Threshold {:.2}: p95 timing error {:.2} samples too high",
            threshold,
            metrics.p95_abs_timing_error_samples
        );

        let freq = freq_opt.expect("Expected rotation frequency estimate");
        assert!(
            (freq - rotation_hz).abs() < 100.0,
            "Threshold {:.2}: frequency {:.1}Hz too far from {:.1}Hz",
            threshold,
            freq,
            rotation_hz
        );
    }
}

#[test]
fn test_north_tracking_jitter_sweep() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;
    let duration_secs = 1.0;
    let start_time_secs = 0.05;
    let num_samples = (duration_secs * sample_rate) as usize;

    for jitter_samples in [0, 1, 2] {
        let pulse_positions = generate_pulse_positions(
            start_time_secs,
            duration_secs,
            sample_rate,
            |_| rotation_hz,
            |_| true,
            jitter_samples,
        );
        let north_signal = build_north_signal(num_samples, &pulse_positions, 0.8);
        let (ticks, freq_opt) = run_north_tracker(&config, &north_signal);
        let metrics = detection_metrics(&pulse_positions, &ticks, jitter_samples as f32 + 3.0);

        let min_detection_rate = match jitter_samples {
            0 => 0.95,
            1 => 0.90,
            _ => 0.85,
        };
        assert!(
            metrics.detection_rate >= min_detection_rate,
            "Jitter ±{} samples: detection rate {:.2} too low",
            jitter_samples,
            metrics.detection_rate
        );
        assert!(
            metrics.false_positive_rate <= 0.10,
            "Jitter ±{} samples: false positive rate {:.2} too high",
            jitter_samples,
            metrics.false_positive_rate
        );

        let max_p95_timing_error = match jitter_samples {
            0 => 2.5,
            1 => 3.5,
            _ => 5.0,
        };
        assert!(
            metrics.p95_abs_timing_error_samples <= max_p95_timing_error,
            "Jitter ±{} samples: p95 timing error {:.2} exceeds {:.2}",
            jitter_samples,
            metrics.p95_abs_timing_error_samples,
            max_p95_timing_error
        );

        let freq = freq_opt.expect("Expected rotation frequency estimate");
        let max_freq_error = match jitter_samples {
            0 => 50.0,
            1 => 80.0,
            _ => 130.0,
        };
        assert!(
            (freq - rotation_hz).abs() < max_freq_error,
            "Jitter ±{} samples: frequency {:.1}Hz too far from {:.1}Hz",
            jitter_samples,
            freq,
            rotation_hz
        );
    }
}

#[test]
fn test_north_tracking_frequency_step() {
    let mut config = RdfConfig::default();
    config.north_tick.dpll.natural_frequency_hz = 25.0;
    let sample_rate = config.audio.sample_rate as f32;
    let duration_secs = 1.4;
    let start_time_secs = 0.05;
    let step_time_secs = 0.7;
    let f1_hz = 1602.0;
    let f2_hz = 1570.0;
    let num_samples = (duration_secs * sample_rate) as usize;

    let pulse_positions = generate_pulse_positions(
        start_time_secs,
        duration_secs,
        sample_rate,
        |t| if t < step_time_secs { f1_hz } else { f2_hz },
        |_| true,
        1,
    );
    let north_signal = build_north_signal(num_samples, &pulse_positions, 0.8);
    let (ticks, _freq_opt) = run_north_tracker(&config, &north_signal);
    let det_metrics = detection_metrics(&pulse_positions, &ticks, 4.0);

    assert!(
        det_metrics.detection_rate >= 0.90,
        "Frequency step: detection rate {:.2} too low",
        det_metrics.detection_rate
    );
    assert!(
        det_metrics.false_positive_rate <= 0.08,
        "Frequency step: false positive rate {:.2} too high",
        det_metrics.false_positive_rate
    );
    assert!(
        det_metrics.p95_abs_timing_error_samples <= 4.0,
        "Frequency step: p95 timing error {:.2} too high",
        det_metrics.p95_abs_timing_error_samples
    );
    let step_metrics = step_response_metrics(
        &ticks,
        sample_rate,
        step_time_secs,
        StepResponseEvalConfig {
            pre_window: (0.25, 0.65),
            post_window: (0.95, 1.35),
            target_post_hz: f2_hz,
            settle_band_hz: 60.0,
            settle_consecutive_ticks: 10,
        },
    );

    assert!(
        (step_metrics.pre_step_mean_hz - f1_hz).abs() < 70.0,
        "Pre-step frequency {:.1}Hz too far from {:.1}Hz",
        step_metrics.pre_step_mean_hz,
        f1_hz
    );
    assert!(
        (step_metrics.post_step_mean_hz - f2_hz).abs() < 90.0,
        "Post-step frequency {:.1}Hz too far from {:.1}Hz",
        step_metrics.post_step_mean_hz,
        f2_hz
    );
    assert!(
        step_metrics.max_abs_error_after_step_hz < 120.0,
        "Step overshoot/error {:.1}Hz too high",
        step_metrics.max_abs_error_after_step_hz
    );
    let settle_time = step_metrics
        .settle_time_secs
        .expect("Frequency step should settle within test duration");
    assert!(
        settle_time < 0.35,
        "Frequency step settle time {:.3}s exceeds 0.35s",
        settle_time
    );
}

#[test]
fn test_north_tracking_dropout_reacquisition() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;
    // The dropout starts well after the loop has settled: how far the
    // tracker may coast depends on how well it knows the rotation rate, and
    // at the shipped 1 Hz loop bandwidth that takes a couple of seconds.
    let duration_secs = 4.0;
    let start_time_secs = 0.05;
    let dropout_start = 2.5;
    let dropout_end = 2.8;
    let num_samples = (duration_secs * sample_rate) as usize;

    let pulse_positions = generate_pulse_positions(
        start_time_secs,
        duration_secs,
        sample_rate,
        |_| rotation_hz,
        |t| !(dropout_start..=dropout_end).contains(&t),
        1,
    );
    // The rotation continues through the dropout even though the pulses do
    // not, so a coasted tick belongs at every one of these positions.
    let uninterrupted_positions = generate_pulse_positions(
        start_time_secs,
        duration_secs,
        sample_rate,
        |_| rotation_hz,
        |_| true,
        1,
    );
    let north_signal = build_north_signal(num_samples, &pulse_positions, 0.8);
    let (ticks, _freq_opt) = run_north_tracker(&config, &north_signal);
    let det_metrics = detection_metrics(&pulse_positions, &ticks, 4.0);
    // Scored against the uninterrupted train: a tick emitted during the
    // dropout is only legitimate if it lands where the missing pulse was.
    let coast_metrics = detection_metrics(&uninterrupted_positions, &ticks, 4.0);

    assert!(
        det_metrics.detection_rate >= 0.85,
        "Dropout: detection rate {:.2} too low",
        det_metrics.detection_rate
    );
    assert!(
        coast_metrics.false_positive_rate <= 0.08,
        "Dropout: false positive rate {:.2} too high",
        coast_metrics.false_positive_rate
    );
    assert!(
        coast_metrics.detection_rate >= 0.95,
        "Dropout: coasting left {:.2} of the rotation unaccounted for",
        1.0 - coast_metrics.detection_rate
    );

    let ticks_before: Vec<&NorthTick> = ticks
        .iter()
        .filter(|tick| (tick.sample_index as f32 / sample_rate) < dropout_start)
        .collect();
    let ticks_in_dropout: Vec<&NorthTick> = ticks
        .iter()
        .filter(|tick| {
            let t = tick.sample_index as f32 / sample_rate;
            (dropout_start..=dropout_end).contains(&t)
        })
        .collect();
    let ticks_after: Vec<&NorthTick> = ticks
        .iter()
        .filter(|tick| (tick.sample_index as f32 / sample_rate) > dropout_end)
        .collect();

    assert!(
        ticks_before.len() > 200,
        "Expected many ticks before dropout, got {}",
        ticks_before.len()
    );
    assert!(
        ticks_after.len() > 200,
        "Expected many ticks after dropout, got {}",
        ticks_after.len()
    );
    // Coasting keeps the reference alive across the dropout rather than
    // leaving a hole in the bearing output.
    let expected_in_dropout = ((dropout_end - dropout_start) * rotation_hz) as usize;
    assert!(
        ticks_in_dropout.len() >= expected_in_dropout * 9 / 10,
        "Expected coasting to cover the dropout: {} ticks, expected about {}",
        ticks_in_dropout.len(),
        expected_in_dropout
    );

    let first_after = ticks_after
        .first()
        .expect("Expected at least one tick after dropout");
    let first_after_time = first_after.sample_index as f32 / sample_rate;
    assert!(
        first_after_time - dropout_end < 0.05,
        "Reacquisition took too long: first post-dropout tick at {:.3}s",
        first_after_time
    );

    let post_hz: Vec<f32> = ticks
        .iter()
        .filter_map(|tick| {
            let t = tick.sample_index as f32 / sample_rate;
            if t > 0.9 && t < 1.3 {
                Some(tick_hz(tick, sample_rate))
            } else {
                None
            }
        })
        .collect();
    assert!(
        !post_hz.is_empty(),
        "Expected post-dropout frequency samples for verification"
    );
    let post_mean = mean(&post_hz).unwrap();
    assert!(
        (post_mean - rotation_hz).abs() < 90.0,
        "Post-dropout frequency {:.1}Hz too far from {:.1}Hz",
        post_mean,
        rotation_hz
    );
}

/// The detector dead time is what keeps noise triggers from taking the place
/// of real pulses at low SNR.
///
/// It covers 96% of a rotation at the default rate, which is also why the
/// timing gate can only act on late detections. Trading dead time for gate
/// reach was measured and rejected: the gate rejects what disagrees with the
/// tracked rotation, and a noise trigger arriving where a pulse is due does
/// not disagree with it.
#[test]
fn test_dead_time_rejects_noise_triggers() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;
    let duration_secs = 3.0;
    let num_samples = (duration_secs * sample_rate) as usize;

    let pulse_positions = generate_pulse_positions(
        0.05,
        duration_secs,
        sample_rate,
        |_| rotation_hz,
        |_| true,
        1,
    );
    let mut north_signal = build_north_signal(num_samples, &pulse_positions, 0.8);

    // Noise a quarter of the pulse amplitude, which is where the tradeoff
    // between dead time and detection becomes visible.
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    for sample in north_signal.iter_mut() {
        let mut acc = 0.0f32;
        for _ in 0..12 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            acc += (((state >> 33) as u32) as f32) / (u32::MAX as f32);
        }
        *sample += (acc - 6.0) * 0.2;
    }

    let (ticks, _freq) = run_north_tracker(&config, &north_signal);
    let metrics = detection_metrics(&pulse_positions, &ticks, 4.0);

    assert!(
        metrics.detection_rate >= 0.75,
        "detection rate {:.3} at noise 0.2; shortening the dead time drops this to about 0.25",
        metrics.detection_rate
    );
    assert!(
        metrics.false_positive_rate <= 0.15,
        "false positive rate {:.3} at noise 0.2; shortening the dead time raises this above 0.6",
        metrics.false_positive_rate
    );
}

/// Misconfiguration is refused at construction, with a message that names the
/// setting and what would fix it.
///
/// A tracker that starts and then silently detects nothing is far harder to
/// diagnose from a bearing display than one that refuses to start.
#[test]
fn test_north_tick_config_guardrails() {
    let sample_rate = RdfConfig::default().audio.sample_rate as f32;

    type BadConfig = (&'static str, fn(&mut RdfConfig), &'static str);
    let cases: [BadConfig; 8] = [
        (
            "threshold above pulse amplitude",
            |c| c.north_tick.threshold = 0.9,
            "expected_pulse_amplitude",
        ),
        (
            "threshold at zero",
            |c| c.north_tick.threshold = 0.0,
            "greater than 0",
        ),
        (
            "pulse amplitude above full scale",
            |c| c.north_tick.expected_pulse_amplitude = 1.5,
            "expected_pulse_amplitude",
        ),
        (
            "filter too short for three taps",
            |c| c.north_tick.fir_highpass_length_us = 20.0,
            "fir_highpass_length_us",
        ),
        (
            "cutoff above Nyquist",
            |c| c.north_tick.highpass_cutoff = 30_000.0,
            "Nyquist",
        ),
        (
            "cutoff below its own transition width",
            |c| c.north_tick.highpass_cutoff = 100.0,
            "highpass_transition_hz",
        ),
        ("absurd gain", |c| c.north_tick.gain_db = 200.0, "gain_db"),
        (
            "negative coast budget",
            |c| c.north_tick.max_coast_ms = -1.0,
            "max_coast_ms",
        ),
    ];

    for (name, break_it, expected_fragment) in cases {
        for mode in [NorthTrackingMode::Dpll, NorthTrackingMode::Simple] {
            let mut config = RdfConfig::default();
            config.north_tick.mode = mode;
            break_it(&mut config);

            match NorthReferenceTracker::new(&config.north_tick, sample_rate) {
                Ok(_) => panic!("{mode:?} accepted a config with {name}"),
                Err(error) => {
                    let text = error.to_string();
                    assert!(
                        text.contains(expected_fragment),
                        "{mode:?} rejected {name} but the message does not mention \
                         '{expected_fragment}': {text}"
                    );
                }
            }
        }
    }

    // The defaults must survive their own guardrails at both supported rates.
    for rate in [48_000.0f32, 96_000.0] {
        let config = RdfConfig::default();
        assert!(
            NorthReferenceTracker::new(&config.north_tick, rate).is_ok(),
            "default config rejected at {rate} Hz"
        );
    }
}

/// A north channel too quiet to detect must be visible, not silent.
///
/// The detection threshold has wide margin -- detection holds down to a pulse
/// amplitude of 0.3 against the 0.8 expected -- but below that it does not
/// degrade, it stops. There are no ticks, so no bearings, and nothing that
/// would show a problem. The tracker therefore reports how long it has been
/// since it last saw a pulse, which is the quantity a caller can act on.
#[test]
fn test_quiet_north_channel_is_observable() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;
    let duration_secs = 2.0;
    let num_samples = (duration_secs * sample_rate) as usize;

    let positions = generate_pulse_positions(
        0.05,
        duration_secs,
        sample_rate,
        |_| rotation_hz,
        |_| true,
        1,
    );

    for mode in [NorthTrackingMode::Dpll, NorthTrackingMode::Simple] {
        // Healthy: pulses at the expected amplitude.
        let mut healthy = config.clone();
        healthy.north_tick.mode = mode;
        let signal = build_north_signal(num_samples, &positions, 0.8);
        let mut tracker = NorthReferenceTracker::new(&healthy.north_tick, sample_rate).unwrap();
        for chunk in signal.chunks(512) {
            let _ = tracker.process_buffer(chunk);
        }
        assert!(
            tracker.samples_since_detection() < sample_rate as usize / 100,
            "{mode:?}: healthy channel reports {} samples since a detection",
            tracker.samples_since_detection()
        );

        // Too quiet: below where the filtered pulse crosses the threshold.
        let quiet = build_north_signal(num_samples, &positions, 0.1);
        let mut tracker = NorthReferenceTracker::new(&healthy.north_tick, sample_rate).unwrap();
        let mut ticks = 0usize;
        for chunk in quiet.chunks(512) {
            ticks += tracker.process_buffer(chunk).len();
        }
        assert_eq!(
            ticks, 0,
            "{mode:?}: expected a 0.1 channel to detect nothing"
        );
        assert!(
            tracker.samples_since_detection() >= num_samples - 512,
            "{mode:?}: silent channel reports only {} samples since a detection",
            tracker.samples_since_detection()
        );
    }
}

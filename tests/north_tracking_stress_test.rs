use std::f32::consts::PI;

use rotaryclub::config::RdfConfig;
use rotaryclub::rdf::{NorthReferenceTracker, NorthTick, NorthTracker};

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
    let expected = pulse_positions.len() as f32;

    for amplitude in [0.35f32, 0.5, 0.8, 1.2] {
        let north_signal = build_north_signal(num_samples, &pulse_positions, amplitude);
        let (ticks, freq_opt) = run_north_tracker(&config, &north_signal);
        let detected = ticks.len() as f32;
        let detection_rate = detected / expected;
        let false_positive_rate =
            ((ticks.len().saturating_sub(pulse_positions.len())) as f32) / expected.max(1.0);

        assert!(
            detection_rate >= 0.90,
            "Amplitude {:.2}: detection rate {:.2} too low (expected {})",
            amplitude,
            detection_rate,
            pulse_positions.len()
        );
        assert!(
            false_positive_rate <= 0.05,
            "Amplitude {:.2}: false positive rate {:.2} too high",
            amplitude,
            false_positive_rate
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
    let expected = pulse_positions.len() as f32;
    let north_signal = build_north_signal(num_samples, &pulse_positions, 0.8);

    for threshold in [0.08f32, 0.12, 0.15, 0.20, 0.25] {
        let mut config = base_config.clone();
        config.north_tick.threshold = threshold;
        let (ticks, freq_opt) = run_north_tracker(&config, &north_signal);
        let detected = ticks.len() as f32;
        let detection_rate = detected / expected;
        let false_positive_rate =
            ((ticks.len().saturating_sub(pulse_positions.len())) as f32) / expected.max(1.0);

        assert!(
            detection_rate >= 0.88,
            "Threshold {:.2}: detection rate {:.2} too low",
            threshold,
            detection_rate
        );
        assert!(
            false_positive_rate <= 0.08,
            "Threshold {:.2}: false positive rate {:.2} too high",
            threshold,
            false_positive_rate
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
        let expected = pulse_positions.len() as f32;
        let north_signal = build_north_signal(num_samples, &pulse_positions, 0.8);
        let (ticks, freq_opt) = run_north_tracker(&config, &north_signal);
        let detected = ticks.len() as f32;
        let detection_rate = detected / expected;

        let min_detection_rate = match jitter_samples {
            0 => 0.95,
            1 => 0.90,
            _ => 0.85,
        };
        assert!(
            detection_rate >= min_detection_rate,
            "Jitter ±{} samples: detection rate {:.2} too low",
            jitter_samples,
            detection_rate
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

    let pre_step_hz: Vec<f32> = ticks
        .iter()
        .filter_map(|tick| {
            let t = tick.sample_index as f32 / sample_rate;
            if t > 0.25 && t < 0.65 {
                Some(tick_hz(tick, sample_rate))
            } else {
                None
            }
        })
        .collect();
    let post_step_hz: Vec<f32> = ticks
        .iter()
        .filter_map(|tick| {
            let t = tick.sample_index as f32 / sample_rate;
            if t > 0.95 && t < 1.35 {
                Some(tick_hz(tick, sample_rate))
            } else {
                None
            }
        })
        .collect();

    assert!(
        pre_step_hz.len() > 200,
        "Expected many pre-step ticks, got {}",
        pre_step_hz.len()
    );
    assert!(
        post_step_hz.len() > 200,
        "Expected many post-step ticks, got {}",
        post_step_hz.len()
    );

    let pre_mean = mean(&pre_step_hz).unwrap();
    let post_mean = mean(&post_step_hz).unwrap();

    assert!(
        (pre_mean - f1_hz).abs() < 70.0,
        "Pre-step frequency {:.1}Hz too far from {:.1}Hz",
        pre_mean,
        f1_hz
    );
    assert!(
        (post_mean - f2_hz).abs() < 90.0,
        "Post-step frequency {:.1}Hz too far from {:.1}Hz",
        post_mean,
        f2_hz
    );
}

#[test]
fn test_north_tracking_dropout_reacquisition() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;
    let duration_secs = 1.4;
    let start_time_secs = 0.05;
    let dropout_start = 0.45;
    let dropout_end = 0.75;
    let num_samples = (duration_secs * sample_rate) as usize;

    let pulse_positions = generate_pulse_positions(
        start_time_secs,
        duration_secs,
        sample_rate,
        |_| rotation_hz,
        |t| !(dropout_start..=dropout_end).contains(&t),
        1,
    );
    let north_signal = build_north_signal(num_samples, &pulse_positions, 0.8);
    let (ticks, _freq_opt) = run_north_tracker(&config, &north_signal);

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
    assert!(
        ticks_in_dropout.len() <= 3,
        "Too many ticks during dropout: {}",
        ticks_in_dropout.len()
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

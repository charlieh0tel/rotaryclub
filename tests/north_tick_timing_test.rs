use rotaryclub::config::{NorthTrackingMode, RdfConfig};
use rotaryclub::rdf::{NorthReferenceTracker, NorthTick, NorthTracker};

fn generate_truth_pulses(
    sample_rate: f32,
    duration_secs: f32,
    start_time_secs: f32,
    rotation_hz: f32,
) -> Vec<usize> {
    let n = (duration_secs * sample_rate) as usize;
    let mut t = start_time_secs;
    let mut out = Vec::new();
    while t < duration_secs {
        let idx = (t * sample_rate).round() as isize;
        if idx >= 0 && (idx as usize) < n {
            out.push(idx as usize);
        }
        t += 1.0 / rotation_hz;
    }
    out.sort_unstable();
    out.dedup();
    out
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

fn deterministic_jitter_samples(index: usize, max_abs_jitter: i32) -> i32 {
    if max_abs_jitter <= 0 {
        0
    } else {
        ((index as f32 * 0.37).sin() * max_abs_jitter as f32).round() as i32
    }
}

fn jittered_positions(base: &[usize], max_abs_jitter: i32, max_index: usize) -> Vec<usize> {
    let mut out = Vec::with_capacity(base.len());
    for (k, &pos) in base.iter().enumerate() {
        let jitter = deterministic_jitter_samples(k, max_abs_jitter) as isize;
        let idx = (pos as isize + jitter).clamp(0, max_index as isize) as usize;
        out.push(idx);
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn add_deterministic_noise(signal: &mut [f32], noise_peak: f32) {
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    for sample in signal.iter_mut() {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
        let u = (((x >> 33) as u32) as f32) / (u32::MAX as f32);
        let noise = (2.0 * u - 1.0) * noise_peak;
        *sample += noise;
    }
}

fn add_impulses(signal: &mut [f32], stride: usize, amplitude: f32) {
    if stride == 0 {
        return;
    }
    for i in (stride / 2..signal.len()).step_by(stride) {
        signal[i] += amplitude;
    }
}

fn apply_deterministic_dropouts(positions: &[usize], stride: usize) -> Vec<usize> {
    if stride <= 1 {
        return positions.to_vec();
    }
    positions
        .iter()
        .enumerate()
        .filter_map(|(k, &p)| if k % stride == 0 { None } else { Some(p) })
        .collect()
}

fn match_timing_errors_samples(
    expected: &[usize],
    ticks: &[NorthTick],
    tolerance: f32,
) -> Vec<f32> {
    let expected: Vec<f32> = expected.iter().map(|&s| s as f32).collect();
    let detected: Vec<f32> = ticks
        .iter()
        .map(|tick| tick.sample_index as f32 + tick.fractional_sample_offset)
        .collect();

    let mut i = 0usize;
    let mut j = 0usize;
    let mut errors = Vec::new();

    while i < expected.len() && j < detected.len() {
        let err = (detected[j] - expected[i]).abs();
        if err <= tolerance {
            errors.push(err);
            i += 1;
            j += 1;
        } else if detected[j] < expected[i] {
            j += 1;
        } else {
            i += 1;
        }
    }
    errors
}

fn false_positive_rate(expected: &[usize], ticks: &[NorthTick], matched_count: usize) -> f32 {
    let expected_len = expected.len().max(1) as f32;
    let unmatched_detections = ticks.len().saturating_sub(matched_count);
    unmatched_detections as f32 / expected_len
}

fn mean(values: &[f32]) -> f32 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f32>() / values.len() as f32
    }
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

#[test]
fn test_north_tick_timing_error_across_chunk_sizes() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;
    let duration_secs = 1.2f32;
    let num_samples = (duration_secs * sample_rate) as usize;
    let pulse_amplitude = config.north_tick.expected_pulse_amplitude;

    let chunk_sizes = [32usize, 64, 128, 256, 512, 1024];
    let start_offsets = [0.013f32, 0.019, 0.027];

    for &chunk_size in &chunk_sizes {
        for &start_time_secs in &start_offsets {
            let expected =
                generate_truth_pulses(sample_rate, duration_secs, start_time_secs, rotation_hz);
            let north = build_north_signal(num_samples, &expected, pulse_amplitude);
            let mut tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate).unwrap();
            let mut detected = Vec::new();

            for chunk in north.chunks(chunk_size) {
                detected.extend(tracker.process_buffer(chunk));
            }

            let errors = match_timing_errors_samples(&expected, &detected, 3.0);
            let expected_count = expected.len().max(1);
            let detection_rate = errors.len() as f32 / expected_count as f32;
            let mean_abs_error = mean(&errors);
            let p95_abs_error = percentile(&errors, 0.95);

            assert!(
                detection_rate >= 0.95,
                "chunk_size={chunk_size}, start={start_time_secs:.3}s detection_rate={detection_rate:.3} (matched={} expected={})",
                errors.len(),
                expected.len()
            );
            assert!(
                mean_abs_error <= 1.0,
                "chunk_size={chunk_size}, start={start_time_secs:.3}s mean_abs_error={mean_abs_error:.3} samples",
            );
            assert!(
                p95_abs_error <= 2.0,
                "chunk_size={chunk_size}, start={start_time_secs:.3}s p95_abs_error={p95_abs_error:.3} samples",
            );
        }
    }
}

#[test]
fn test_north_tick_timing_with_jitter_and_noise() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;
    let duration_secs = 1.2f32;
    let num_samples = (duration_secs * sample_rate) as usize;
    let pulse_amplitude = config.north_tick.expected_pulse_amplitude;

    let chunk_sizes = [64usize, 256, 1024];
    let start_offsets = [0.011f32, 0.023, 0.031];

    for &chunk_size in &chunk_sizes {
        for &start_time_secs in &start_offsets {
            let base =
                generate_truth_pulses(sample_rate, duration_secs, start_time_secs, rotation_hz);
            let expected = jittered_positions(&base, 1, num_samples.saturating_sub(1));
            let mut north = build_north_signal(num_samples, &expected, pulse_amplitude * 0.85);
            add_deterministic_noise(&mut north, 0.025);
            let mut tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate).unwrap();
            let mut detected = Vec::new();

            for chunk in north.chunks(chunk_size) {
                detected.extend(tracker.process_buffer(chunk));
            }

            let errors = match_timing_errors_samples(&expected, &detected, 3.0);
            let expected_count = expected.len().max(1);
            let detection_rate = errors.len() as f32 / expected_count as f32;
            let mean_abs_error = mean(&errors);
            let p95_abs_error = percentile(&errors, 0.95);

            assert!(
                detection_rate >= 0.90,
                "chunk_size={chunk_size}, start={start_time_secs:.3}s detection_rate={detection_rate:.3} (matched={} expected={})",
                errors.len(),
                expected.len()
            );
            assert!(
                mean_abs_error <= 1.3,
                "chunk_size={chunk_size}, start={start_time_secs:.3}s mean_abs_error={mean_abs_error:.3} samples",
            );
            assert!(
                p95_abs_error <= 2.5,
                "chunk_size={chunk_size}, start={start_time_secs:.3}s p95_abs_error={p95_abs_error:.3} samples",
            );
        }
    }
}

#[test]
fn test_north_tick_timing_with_dropouts_and_impulses_across_modes() {
    let base_config = RdfConfig::default();
    let sample_rate = base_config.audio.sample_rate as f32;
    let rotation_hz = base_config.doppler.expected_freq;
    let duration_secs = 1.2f32;
    let num_samples = (duration_secs * sample_rate) as usize;
    let pulse_amplitude = base_config.north_tick.expected_pulse_amplitude;

    let chunk_sizes = [64usize, 256, 1024];
    let start_offsets = [0.011f32, 0.023, 0.031];
    let modes = [NorthTrackingMode::Dpll, NorthTrackingMode::Simple];

    for &mode in &modes {
        for &chunk_size in &chunk_sizes {
            for &start_time_secs in &start_offsets {
                let base =
                    generate_truth_pulses(sample_rate, duration_secs, start_time_secs, rotation_hz);
                let jittered = jittered_positions(&base, 1, num_samples.saturating_sub(1));
                let expected = apply_deterministic_dropouts(&jittered, 14);
                let mut north = build_north_signal(num_samples, &expected, pulse_amplitude * 0.9);
                add_deterministic_noise(&mut north, 0.02);
                add_impulses(&mut north, 211, 0.23);

                let mut config = base_config.clone();
                config.north_tick.mode = mode;
                let mut tracker =
                    NorthReferenceTracker::new(&config.north_tick, sample_rate).unwrap();
                let mut detected = Vec::new();
                for chunk in north.chunks(chunk_size) {
                    detected.extend(tracker.process_buffer(chunk));
                }

                let errors = match_timing_errors_samples(&expected, &detected, 3.0);
                let detection_rate = errors.len() as f32 / expected.len().max(1) as f32;
                let fp_rate = false_positive_rate(&expected, &detected, errors.len());
                let mean_abs_error = mean(&errors);
                let p95_abs_error = percentile(&errors, 0.95);
                let min_detection_rate = if mode == NorthTrackingMode::Simple {
                    0.30
                } else {
                    0.85
                };

                assert!(
                    detection_rate >= min_detection_rate,
                    "mode={mode:?}, chunk_size={chunk_size}, start={start_time_secs:.3}s detection_rate={detection_rate:.3} (min={min_detection_rate:.2})",
                );
                assert!(
                    fp_rate <= 0.15,
                    "mode={mode:?}, chunk_size={chunk_size}, start={start_time_secs:.3}s false_positive_rate={fp_rate:.3}",
                );
                assert!(
                    mean_abs_error <= 1.5,
                    "mode={mode:?}, chunk_size={chunk_size}, start={start_time_secs:.3}s mean_abs_error={mean_abs_error:.3} samples",
                );
                assert!(
                    p95_abs_error <= 2.8,
                    "mode={mode:?}, chunk_size={chunk_size}, start={start_time_secs:.3}s p95_abs_error={p95_abs_error:.3} samples",
                );
            }
        }
    }
}

#[test]
fn test_north_tick_timing_long_duration_drift_across_modes() {
    let base_config = RdfConfig::default();
    let sample_rate = base_config.audio.sample_rate as f32;
    let rotation_hz = base_config.doppler.expected_freq;
    let duration_secs = 10.0f32;
    let num_samples = (duration_secs * sample_rate) as usize;
    let pulse_amplitude = base_config.north_tick.expected_pulse_amplitude;
    let chunk_sizes = [256usize, 1024];
    let start_time_secs = 0.017f32;
    let modes = [NorthTrackingMode::Dpll, NorthTrackingMode::Simple];

    for &mode in &modes {
        for &chunk_size in &chunk_sizes {
            let expected =
                generate_truth_pulses(sample_rate, duration_secs, start_time_secs, rotation_hz);
            let north = build_north_signal(num_samples, &expected, pulse_amplitude);

            let mut config = base_config.clone();
            config.north_tick.mode = mode;

            let mut tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate).unwrap();
            let mut detected = Vec::new();
            for chunk in north.chunks(chunk_size) {
                detected.extend(tracker.process_buffer(chunk));
            }

            let errors = match_timing_errors_samples(&expected, &detected, 3.0);
            let detection_rate = errors.len() as f32 / expected.len().max(1) as f32;
            let fp_rate = false_positive_rate(&expected, &detected, errors.len());
            let mean_abs_error = mean(&errors);
            let p95_abs_error = percentile(&errors, 0.95);

            assert!(
                detection_rate >= 0.97,
                "mode={mode:?}, chunk_size={chunk_size} long_drift detection_rate={detection_rate:.3}",
            );
            assert!(
                fp_rate <= 0.03,
                "mode={mode:?}, chunk_size={chunk_size} long_drift false_positive_rate={fp_rate:.3}",
            );
            assert!(
                mean_abs_error <= 0.8,
                "mode={mode:?}, chunk_size={chunk_size} long_drift mean_abs_error={mean_abs_error:.3} samples",
            );
            assert!(
                p95_abs_error <= 1.5,
                "mode={mode:?}, chunk_size={chunk_size} long_drift p95_abs_error={p95_abs_error:.3} samples",
            );
        }
    }
}

#[test]
fn test_north_tick_timing_frequency_step_across_modes() {
    let base_config = RdfConfig::default();
    let sample_rate = base_config.audio.sample_rate as f32;
    let f1 = base_config.doppler.expected_freq;
    let f2 = f1 + 48.0;
    let duration_secs = 4.0f32;
    let step_time_secs = 2.0f32;
    let num_samples = (duration_secs * sample_rate) as usize;
    let pulse_amplitude = base_config.north_tick.expected_pulse_amplitude;
    let chunk_sizes = [256usize, 1024];
    let start_time_secs = 0.013f32;
    let modes = [NorthTrackingMode::Dpll, NorthTrackingMode::Simple];

    for &mode in &modes {
        for &chunk_size in &chunk_sizes {
            let expected = {
                let mut positions = Vec::new();
                let mut t = start_time_secs;
                while t < duration_secs {
                    let hz = if t < step_time_secs { f1 } else { f2 };
                    let idx = (t * sample_rate).round() as isize;
                    if idx >= 0 && (idx as usize) < num_samples {
                        positions.push(idx as usize);
                    }
                    t += 1.0 / hz;
                }
                positions.sort_unstable();
                positions.dedup();
                positions
            };

            let mut north = build_north_signal(num_samples, &expected, pulse_amplitude * 0.95);
            add_deterministic_noise(&mut north, 0.01);

            let mut config = base_config.clone();
            config.north_tick.mode = mode;
            let mut tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate).unwrap();
            let mut detected = Vec::new();
            for chunk in north.chunks(chunk_size) {
                detected.extend(tracker.process_buffer(chunk));
            }

            let errors = match_timing_errors_samples(&expected, &detected, 3.0);
            let detection_rate = errors.len() as f32 / expected.len().max(1) as f32;
            let fp_rate = false_positive_rate(&expected, &detected, errors.len());
            let mean_abs_error = mean(&errors);
            let p95_abs_error = percentile(&errors, 0.95);

            assert!(
                detection_rate >= 0.93,
                "mode={mode:?}, chunk_size={chunk_size} freq_step detection_rate={detection_rate:.3}",
            );
            assert!(
                fp_rate <= 0.08,
                "mode={mode:?}, chunk_size={chunk_size} freq_step false_positive_rate={fp_rate:.3}",
            );
            assert!(
                mean_abs_error <= 1.2,
                "mode={mode:?}, chunk_size={chunk_size} freq_step mean_abs_error={mean_abs_error:.3} samples",
            );
            assert!(
                p95_abs_error <= 2.3,
                "mode={mode:?}, chunk_size={chunk_size} freq_step p95_abs_error={p95_abs_error:.3} samples",
            );
        }
    }
}

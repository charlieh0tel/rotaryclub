use rotaryclub::config::RdfConfig;
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

fn match_timing_errors_samples(expected: &[usize], ticks: &[NorthTick], tolerance: f32) -> Vec<f32> {
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

fn main() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;
    let duration_secs = 1.2f32;
    let num_samples = (duration_secs * sample_rate) as usize;
    let pulse_amplitude = config.north_tick.expected_pulse_amplitude;

    let chunk_sizes = [32usize, 64, 128, 256, 512, 1024];
    let start_offsets = [0.011f32, 0.023, 0.031];

    println!(
        "scenario,chunk_size,start_offset_s,expected,matched,detection_rate,mean_abs_error_samples,p95_abs_error_samples"
    );

    for &(scenario_name, jitter_samples, noise_peak, amplitude_scale) in &[
        ("clean", 0i32, 0.0f32, 1.0f32),
        ("noisy_jittered", 1i32, 0.025f32, 0.85f32),
    ] {
        for &chunk_size in &chunk_sizes {
            for &start_time_secs in &start_offsets {
                let base =
                    generate_truth_pulses(sample_rate, duration_secs, start_time_secs, rotation_hz);
                let expected = jittered_positions(&base, jitter_samples, num_samples.saturating_sub(1));
                let mut north =
                    build_north_signal(num_samples, &expected, pulse_amplitude * amplitude_scale);
                if noise_peak > 0.0 {
                    add_deterministic_noise(&mut north, noise_peak);
                }

                let mut tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate).unwrap();
                let mut detected = Vec::new();
                for chunk in north.chunks(chunk_size) {
                    detected.extend(tracker.process_buffer(chunk));
                }

                let errors = match_timing_errors_samples(&expected, &detected, 3.0);
                let expected_count = expected.len().max(1);
                let matched = errors.len();
                let detection_rate = matched as f32 / expected_count as f32;
                let mean_abs_error = mean(&errors);
                let p95_abs_error = percentile(&errors, 0.95);

                println!(
                    "{},{},{:.3},{},{},{:.6},{:.6},{:.6}",
                    scenario_name,
                    chunk_size,
                    start_time_secs,
                    expected.len(),
                    matched,
                    detection_rate,
                    mean_abs_error,
                    p95_abs_error
                );
            }
        }
    }
}

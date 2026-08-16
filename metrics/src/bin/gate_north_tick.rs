use rotaryclub::config::{NorthTrackingMode, RdfConfig};
use rotaryclub::rdf::{NorthReferenceTracker, NorthTick, NorthTracker};
use rotaryclub::simulation::noise_at;

const DEFAULT_DURATION_SECS: f32 = 1.2;
const DEFAULT_CHUNK_SIZES: &[usize] = &[32usize, 64, 128, 256, 512, 1024];
const DEFAULT_START_OFFSETS: &[f32] = &[0.011f32, 0.023, 0.031];
const LONG_DRIFT_CHUNK_SIZES: &[usize] = &[256usize, 1024];
const LONG_DRIFT_START_OFFSETS: &[f32] = &[0.017f32];

struct Scenario {
    name: &'static str,
    jitter_samples: i32,
    noise_peak: f32,
    amplitude_scale: f32,
    dropout_stride: Option<usize>,
    impulse_stride: Option<usize>,
    impulse_amplitude: f32,
    duration_secs: f32,
    chunk_sizes: &'static [usize],
    start_offsets: &'static [f32],
    step_at_secs: Option<f32>,
    step_to_frequency_hz: Option<f32>,
}

#[derive(Clone, Copy)]
struct TimingMetrics {
    matched: usize,
    detection_rate: f32,
    false_positive_rate: f32,
    mean_abs_error_samples: f32,
    p95_abs_error_samples: f32,
}

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

/// Per-pulse timing jitter, in samples.
///
/// White. It used to be `sin(0.37 k)`, which repeats every 17 rotations: a
/// coherent 94 Hz modulation, forty-seven times the loop bandwidth, which any
/// second-order loop rejects by construction. Anything measured against it
/// was measuring the stimulus being out of band rather than the tracker doing
/// anything, and it once earned the DPLL an advantage it had not. Real jitter
/// has in-band content the loop has to follow.
///
/// Varies with the draw. Held fixed it was a constant part of the stimulus
/// rather than a source of variation, so the standard errors reported for a
/// jittered scenario described the noise alone and the jitter contributed
/// nothing to the spread the support check reads.
fn deterministic_jitter_samples(index: usize, max_abs_jitter: i32, draw: u64) -> i32 {
    if max_abs_jitter <= 0 {
        0
    } else {
        (noise_at(index, 0x1A77_E812_5EED_0005u64.wrapping_add(draw)) * max_abs_jitter as f32)
            .round() as i32
    }
}

fn jittered_positions(
    base: &[usize],
    max_abs_jitter: i32,
    max_index: usize,
    draw: u64,
) -> Vec<usize> {
    let mut out = Vec::with_capacity(base.len());
    for (k, &pos) in base.iter().enumerate() {
        let jitter = deterministic_jitter_samples(k, max_abs_jitter, draw) as isize;
        let idx = (pos as isize + jitter).clamp(0, max_index as isize) as usize;
        out.push(idx);
    }
    out.sort_unstable();
    out.dedup();
    out
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

fn add_deterministic_noise(signal: &mut [f32], noise_peak: f32, draw: u64) {
    for (i, sample) in signal.iter_mut().enumerate() {
        *sample += noise_at(i, 0x71C7_71C7_5EED_0001u64.wrapping_add(draw)) * noise_peak;
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

fn compute_timing_metrics(
    expected: &[usize],
    ticks: &[NorthTick],
    tolerance: f32,
) -> TimingMetrics {
    let expected: Vec<f32> = expected.iter().map(|&s| s as f32).collect();
    let detected: Vec<f32> = ticks
        .iter()
        .map(|tick| tick.sample_index as f32 + tick.fractional_sample_offset)
        .collect();

    let mut i = 0usize;
    let mut j = 0usize;
    let mut matched = 0usize;
    let mut errors = Vec::new();

    while i < expected.len() && j < detected.len() {
        let err = (detected[j] - expected[i]).abs();
        if err <= tolerance {
            matched += 1;
            errors.push(err);
            i += 1;
            j += 1;
        } else if detected[j] < expected[i] {
            j += 1;
        } else {
            i += 1;
        }
    }

    let expected_len = expected.len().max(1) as f32;
    let unmatched_detections = detected.len().saturating_sub(matched);

    TimingMetrics {
        matched,
        detection_rate: matched as f32 / expected_len,
        false_positive_rate: unmatched_detections as f32 / expected_len,
        mean_abs_error_samples: mean(&errors),
        p95_abs_error_samples: percentile(&errors, 0.95),
    }
}

/// Independent noise realisations averaged into each reported row.
const DRAWS: u64 = 8;

/// Standard error of the mean of one column over the draws.
fn se_of(runs: &[TimingMetrics], f: fn(&TimingMetrics) -> f32) -> f32 {
    let n = runs.len() as f32;
    if n < 2.0 {
        return 0.0;
    }
    let mean = runs.iter().map(f).sum::<f32>() / n;
    let var = runs
        .iter()
        .map(|r| (f(r) - mean) * (f(r) - mean))
        .sum::<f32>()
        / (n - 1.0);
    (var / n).sqrt()
}

fn average_timing_metrics(runs: &[TimingMetrics]) -> TimingMetrics {
    let n = runs.len() as f32;
    let mean = |f: fn(&TimingMetrics) -> f32| runs.iter().map(f).sum::<f32>() / n;
    TimingMetrics {
        matched: runs.iter().map(|m| m.matched).sum::<usize>() / runs.len(),
        detection_rate: mean(|m| m.detection_rate),
        false_positive_rate: mean(|m| m.false_positive_rate),
        mean_abs_error_samples: mean(|m| m.mean_abs_error_samples),
        p95_abs_error_samples: mean(|m| m.p95_abs_error_samples),
    }
}

fn main() {
    let base_config = RdfConfig::default();
    let sample_rate = base_config.audio.sample_rate as f32;
    let rotation_hz = base_config.doppler.expected_freq;
    let pulse_amplitude = base_config.north_tick.expected_pulse_amplitude;

    let modes = [
        ("dpll", NorthTrackingMode::Dpll),
        ("simple", NorthTrackingMode::Simple),
    ];

    let scenarios = [
        Scenario {
            name: "clean",
            jitter_samples: 0,
            noise_peak: 0.0,
            amplitude_scale: 1.0,
            dropout_stride: None,
            impulse_stride: None,
            impulse_amplitude: 0.0,
            duration_secs: DEFAULT_DURATION_SECS,
            chunk_sizes: DEFAULT_CHUNK_SIZES,
            start_offsets: DEFAULT_START_OFFSETS,
            step_at_secs: None,
            step_to_frequency_hz: None,
        },
        Scenario {
            name: "noisy_jittered",
            jitter_samples: 1,
            noise_peak: 0.025,
            amplitude_scale: 0.85,
            dropout_stride: None,
            impulse_stride: None,
            impulse_amplitude: 0.0,
            duration_secs: DEFAULT_DURATION_SECS,
            chunk_sizes: DEFAULT_CHUNK_SIZES,
            start_offsets: DEFAULT_START_OFFSETS,
            step_at_secs: None,
            step_to_frequency_hz: None,
        },
        Scenario {
            name: "dropout_burst",
            jitter_samples: 1,
            noise_peak: 0.02,
            amplitude_scale: 0.9,
            dropout_stride: Some(14),
            impulse_stride: None,
            impulse_amplitude: 0.0,
            duration_secs: DEFAULT_DURATION_SECS,
            chunk_sizes: DEFAULT_CHUNK_SIZES,
            start_offsets: DEFAULT_START_OFFSETS,
            step_at_secs: None,
            step_to_frequency_hz: None,
        },
        Scenario {
            name: "impulsive_interference",
            jitter_samples: 1,
            noise_peak: 0.02,
            amplitude_scale: 0.9,
            dropout_stride: None,
            impulse_stride: Some(211),
            impulse_amplitude: 0.23,
            duration_secs: DEFAULT_DURATION_SECS,
            chunk_sizes: DEFAULT_CHUNK_SIZES,
            start_offsets: DEFAULT_START_OFFSETS,
            step_at_secs: None,
            step_to_frequency_hz: None,
        },
        Scenario {
            name: "long_drift",
            jitter_samples: 0,
            noise_peak: 0.0,
            amplitude_scale: 1.0,
            dropout_stride: None,
            impulse_stride: None,
            impulse_amplitude: 0.0,
            duration_secs: 12.0,
            chunk_sizes: LONG_DRIFT_CHUNK_SIZES,
            start_offsets: LONG_DRIFT_START_OFFSETS,
            step_at_secs: None,
            step_to_frequency_hz: None,
        },
        Scenario {
            name: "freq_step",
            jitter_samples: 0,
            noise_peak: 0.01,
            amplitude_scale: 0.95,
            dropout_stride: None,
            impulse_stride: None,
            impulse_amplitude: 0.0,
            duration_secs: 4.0,
            chunk_sizes: LONG_DRIFT_CHUNK_SIZES,
            start_offsets: LONG_DRIFT_START_OFFSETS,
            step_at_secs: Some(2.0),
            step_to_frequency_hz: Some(rotation_hz + 48.0),
        },
    ];

    for &(mode_name, mode) in &modes {
        for scenario in &scenarios {
            let num_samples = (scenario.duration_secs * sample_rate) as usize;
            for &chunk_size in scenario.chunk_sizes {
                for &start_time_secs in scenario.start_offsets {
                    let base = if let (Some(step_t), Some(step_hz)) =
                        (scenario.step_at_secs, scenario.step_to_frequency_hz)
                    {
                        let mut positions = Vec::new();
                        let mut t = start_time_secs;
                        while t < scenario.duration_secs {
                            let hz = if t < step_t { rotation_hz } else { step_hz };
                            let idx = (t * sample_rate).round() as isize;
                            if idx >= 0 && (idx as usize) < num_samples {
                                positions.push(idx as usize);
                            }
                            t += 1.0 / hz;
                        }
                        positions.sort_unstable();
                        positions.dedup();
                        positions
                    } else {
                        generate_truth_pulses(
                            sample_rate,
                            scenario.duration_secs,
                            start_time_secs,
                            rotation_hz,
                        )
                    };
                    // Averaged over independent realisations of whatever the
                    // scenario randomises -- the noise and the pulse jitter
                    // both. A scenario with neither is identical in every
                    // draw, so only those rows actually cost anything.
                    let varies = scenario.noise_peak > 0.0 || scenario.jitter_samples > 0;
                    let draws = if varies { DRAWS } else { 1 };
                    // The truth positions move with the draw, so each draw is
                    // built and scored against its own truth.
                    let expected_for = |draw: u64| {
                        let mut expected = jittered_positions(
                            &base,
                            scenario.jitter_samples,
                            num_samples.saturating_sub(1),
                            draw,
                        );
                        if let Some(stride) = scenario.dropout_stride {
                            expected = apply_deterministic_dropouts(&expected, stride);
                        }
                        expected
                    };
                    let expected = expected_for(0);
                    let runs: Vec<TimingMetrics> = (0..draws)
                        .map(|draw| {
                            let expected = expected_for(draw);
                            let mut north = build_north_signal(
                                num_samples,
                                &expected,
                                pulse_amplitude * scenario.amplitude_scale,
                            );
                            if scenario.noise_peak > 0.0 {
                                add_deterministic_noise(&mut north, scenario.noise_peak, draw);
                            }
                            if let Some(stride) = scenario.impulse_stride {
                                add_impulses(&mut north, stride, scenario.impulse_amplitude);
                            }

                            let mut config = base_config.clone();
                            config.north_tick.mode = mode;

                            let mut tracker =
                                NorthReferenceTracker::new(&config.north_tick, sample_rate)
                                    .unwrap();
                            let mut detected = Vec::new();
                            for chunk in north.chunks(chunk_size) {
                                detected.extend(tracker.process_buffer(chunk));
                            }
                            compute_timing_metrics(&expected, &detected, 3.0)
                        })
                        .collect();
                    let metrics = average_timing_metrics(&runs);

                    // One JSON object per line. Numbers stay numbers, so
                    // the reader is not parsing formatted strings back into
                    // floats, and a column can be added without every
                    // consumer having to agree on position.
                    println!(
                        "{}",
                        serde_json::json!({
                            "mode": mode_name,
                            "scenario": scenario.name,
                            "chunk_size": chunk_size,
                            "start_offset_s": start_time_secs,
                            "expected": expected.len(),
                            "matched": metrics.matched,
                            "detection_rate": metrics.detection_rate,
                            "false_positive_rate": metrics.false_positive_rate,
                            "mean_abs_error_samples": metrics.mean_abs_error_samples,
                            "p95_abs_error_samples": metrics.p95_abs_error_samples,
                            // For `check` to test whether the row supports a
                            // verdict, not only whether it is inside a limit.
                            "detection_rate_se": se_of(&runs, |m| m.detection_rate),
                            "false_positive_rate_se": se_of(&runs, |m| m.false_positive_rate),
                            "mean_abs_error_samples_se":
                                se_of(&runs, |m| m.mean_abs_error_samples),
                            "p95_abs_error_samples_se": se_of(&runs, |m| m.p95_abs_error_samples),
                            "draws": draws,
                        })
                    );
                }
            }
        }
    }
}

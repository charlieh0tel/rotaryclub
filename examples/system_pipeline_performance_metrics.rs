use rotaryclub::config::{BearingMethod, NorthTrackingMode, RdfConfig};
use rotaryclub::processing::RdfProcessor;
use std::f32::consts::PI;
use std::time::Instant;

const BUFFER_SIZES: &[usize] = &[128, 256, 512];
const ITERATIONS: usize = 180;
const WARMUP_ITERATIONS: usize = 24;
const TICK_MATCH_TOLERANCE_SAMPLES: f32 = 2.0;

#[derive(Clone, Copy)]
struct Scenario {
    name: &'static str,
    amplitude: f32,
    noise_peak: f32,
    dc_offset: f32,
    second_tone_ratio: f32,
    third_tone_ratio: f32,
    north_jitter_samples: i32,
    north_dropout_stride: Option<usize>,
    north_impulse_stride: Option<usize>,
    north_impulse_amplitude: f32,
}

#[derive(Clone, Copy)]
struct Metrics {
    bearing_success_rate: f32,
    detection_rate: f32,
    false_positive_rate: f32,
    mean_us_per_sample: f64,
    p95_us_per_sample: f64,
    mean_abs_bearing_error_deg: f32,
    p95_abs_bearing_error_deg: f32,
    max_abs_bearing_error_deg: f32,
    mean_abs_tick_error_samples: f32,
    p95_abs_tick_error_samples: f32,
}

fn deterministic_noise_at(index: usize, seed: u64) -> f32 {
    let mut x = seed ^ ((index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
    let u = (((x >> 33) as u32) as f32) / (u32::MAX as f32);
    2.0 * u - 1.0
}

fn deterministic_jitter_samples(index: usize, max_abs_jitter: i32) -> i32 {
    if max_abs_jitter <= 0 {
        0
    } else {
        ((index as f32 * 0.37).sin() * max_abs_jitter as f32).round() as i32
    }
}

fn angle_error_deg(measured: f32, expected: f32) -> f32 {
    let mut err = (measured - expected).abs();
    if err > 180.0 {
        err = 360.0 - err;
    }
    err
}

fn percentile_f32(values: &[f32], p: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    let idx = ((sorted.len() as f32 - 1.0) * p.clamp(0.0, 1.0)).round() as usize;
    sorted[idx]
}

fn percentile_f64(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let idx = ((sorted.len() as f64 - 1.0) * p.clamp(0.0, 1.0)).round() as usize;
    sorted[idx]
}

fn mean_f32(values: &[f32]) -> f32 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f32>() / values.len() as f32
    }
}

fn mean_f64(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn expected_tick_positions(
    total_samples: usize,
    samples_per_rotation: f32,
    scenario: Scenario,
) -> Vec<usize> {
    let mut base = Vec::new();
    let mut t = 0.0f32;
    while t < total_samples as f32 {
        base.push(t.round() as usize);
        t += samples_per_rotation;
    }

    let mut jittered = Vec::with_capacity(base.len());
    for (i, p) in base.iter().enumerate() {
        let j = deterministic_jitter_samples(i, scenario.north_jitter_samples) as isize;
        let idx = (*p as isize + j).clamp(0, total_samples.saturating_sub(1) as isize) as usize;
        jittered.push(idx);
    }
    jittered.sort_unstable();
    jittered.dedup();

    if let Some(stride) = scenario.north_dropout_stride
        && stride > 1
    {
        return jittered
            .iter()
            .enumerate()
            .filter_map(|(i, p)| if i % stride == 0 { None } else { Some(*p) })
            .collect();
    }

    jittered
}

fn build_chunk(
    scenario: Scenario,
    chunk_start: usize,
    chunk_size: usize,
    expected_bearing_deg: f32,
    sample_rate: f32,
    rotation_hz: f32,
    tick_positions: &[usize],
) -> Vec<f32> {
    let omega = 2.0 * PI * rotation_hz / sample_rate;
    let bearing_rad = expected_bearing_deg.to_radians();
    let mut out = Vec::with_capacity(chunk_size * 2);

    for i in 0..chunk_size {
        let global = chunk_start + i;
        let t = global as f32;
        let p = omega * t - bearing_rad;

        let fundamental = p.sin();
        let second = (2.0 * p).sin();
        let third = (3.0 * p).sin();
        let noise = deterministic_noise_at(global, 0xBEEF_1234_5678_9ABC);
        let doppler = scenario.amplitude * fundamental
            + scenario.second_tone_ratio * second
            + scenario.third_tone_ratio * third
            + scenario.noise_peak * noise
            + scenario.dc_offset;

        let mut north = if tick_positions.binary_search(&global).is_ok() {
            0.8
        } else {
            0.0
        };
        north +=
            deterministic_noise_at(global, 0xFEED_9876_5432_1001) * (scenario.noise_peak * 0.35);
        north += scenario.dc_offset * 0.25;
        if let Some(stride) = scenario.north_impulse_stride
            && stride > 0
            && global % stride == stride / 2
        {
            north += scenario.north_impulse_amplitude;
        }

        out.push(doppler);
        out.push(north);
    }

    out
}

fn compute_detection_metrics(expected: &[f32], detected: &[f32]) -> (f32, f32, Vec<f32>) {
    let mut i = 0usize;
    let mut j = 0usize;
    let mut matched = 0usize;
    let mut errors = Vec::new();

    while i < expected.len() && j < detected.len() {
        let err = (detected[j] - expected[i]).abs();
        if err <= TICK_MATCH_TOLERANCE_SAMPLES {
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

    let denom = expected.len().max(1) as f32;
    let false_pos = detected.len().saturating_sub(matched) as f32 / denom;
    let det_rate = matched as f32 / denom;
    (det_rate, false_pos, errors)
}

fn run_case(
    north_mode: NorthTrackingMode,
    bearing_method: BearingMethod,
    scenario: Scenario,
    buffer_size: usize,
    expected_bearing_deg: f32,
) -> Metrics {
    let mut config = RdfConfig::default();
    config.north_tick.mode = north_mode;
    config.doppler.method = bearing_method;
    config.audio.buffer_size = buffer_size;
    config.bearing.smoothing_window = 1;

    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;
    let samples_per_rotation = sample_rate / rotation_hz;

    let total_chunks = WARMUP_ITERATIONS + ITERATIONS;
    let total_samples = total_chunks * buffer_size;
    let tick_positions = expected_tick_positions(total_samples, samples_per_rotation, scenario);

    let mut processor = RdfProcessor::new(&config, true, true).expect("rdf processor creation");

    let mut times_us_per_sample = Vec::with_capacity(ITERATIONS);
    let mut bearing_errors = Vec::new();
    let mut detected_ticks = Vec::new();

    for step in 0..total_chunks {
        let start = step * buffer_size;
        let chunk = build_chunk(
            scenario,
            start,
            buffer_size,
            expected_bearing_deg,
            sample_rate,
            rotation_hz,
            &tick_positions,
        );

        let t0 = Instant::now();
        let results = processor.process_audio(&chunk);
        let elapsed_us = t0.elapsed().as_secs_f64() * 1_000_000.0;

        if step >= WARMUP_ITERATIONS {
            times_us_per_sample.push(elapsed_us / buffer_size as f64);
            for r in results {
                detected_ticks
                    .push(r.north_tick.sample_index as f32 + r.north_tick.fractional_sample_offset);
                if let Some(b) = r.bearing {
                    bearing_errors.push(angle_error_deg(b.raw_bearing, expected_bearing_deg));
                }
            }
        }
    }

    let measurement_start = WARMUP_ITERATIONS * buffer_size;
    let expected_ticks: Vec<f32> = tick_positions
        .iter()
        .filter(|&&x| x >= measurement_start)
        .map(|&x| x as f32)
        .collect();
    let (detection_rate, false_positive_rate, tick_errors) =
        compute_detection_metrics(&expected_ticks, &detected_ticks);

    let bearing_success_rate = if expected_ticks.is_empty() {
        0.0
    } else {
        (bearing_errors.len() as f32 / expected_ticks.len() as f32).min(1.0)
    };

    Metrics {
        bearing_success_rate,
        detection_rate,
        false_positive_rate,
        mean_us_per_sample: mean_f64(&times_us_per_sample),
        p95_us_per_sample: percentile_f64(&times_us_per_sample, 0.95),
        mean_abs_bearing_error_deg: if bearing_errors.is_empty() {
            360.0
        } else {
            mean_f32(&bearing_errors)
        },
        p95_abs_bearing_error_deg: if bearing_errors.is_empty() {
            360.0
        } else {
            percentile_f32(&bearing_errors, 0.95)
        },
        max_abs_bearing_error_deg: if bearing_errors.is_empty() {
            360.0
        } else {
            bearing_errors.iter().copied().fold(0.0f32, f32::max)
        },
        mean_abs_tick_error_samples: mean_f32(&tick_errors),
        p95_abs_tick_error_samples: percentile_f32(&tick_errors, 0.95),
    }
}

fn north_mode_name(mode: NorthTrackingMode) -> &'static str {
    match mode {
        NorthTrackingMode::Dpll => "dpll",
        NorthTrackingMode::Simple => "simple",
    }
}

fn bearing_method_name(method: BearingMethod) -> &'static str {
    match method {
        BearingMethod::Correlation => "correlation",
        BearingMethod::ZeroCrossing => "zero_crossing",
    }
}

fn main() {
    let scenarios = [
        Scenario {
            name: "clean",
            amplitude: 1.0,
            noise_peak: 0.0,
            dc_offset: 0.0,
            second_tone_ratio: 0.0,
            third_tone_ratio: 0.0,
            north_jitter_samples: 0,
            north_dropout_stride: None,
            north_impulse_stride: None,
            north_impulse_amplitude: 0.0,
        },
        Scenario {
            name: "noisy_jittered",
            amplitude: 0.9,
            noise_peak: 0.08,
            dc_offset: 0.0,
            second_tone_ratio: 0.0,
            third_tone_ratio: 0.0,
            north_jitter_samples: 1,
            north_dropout_stride: None,
            north_impulse_stride: None,
            north_impulse_amplitude: 0.0,
        },
        Scenario {
            name: "harmonic_contaminated",
            amplitude: 0.9,
            noise_peak: 0.04,
            dc_offset: 0.0,
            second_tone_ratio: 0.20,
            third_tone_ratio: 0.12,
            north_jitter_samples: 0,
            north_dropout_stride: None,
            north_impulse_stride: Some(211),
            north_impulse_amplitude: 0.22,
        },
        Scenario {
            name: "low_snr_dc",
            amplitude: 0.45,
            noise_peak: 0.40,
            dc_offset: 0.20,
            second_tone_ratio: 0.0,
            third_tone_ratio: 0.0,
            north_jitter_samples: 1,
            north_dropout_stride: Some(17),
            north_impulse_stride: Some(97),
            north_impulse_amplitude: 0.30,
        },
    ];

    let north_modes = [NorthTrackingMode::Dpll, NorthTrackingMode::Simple];
    let bearing_methods = [BearingMethod::Correlation, BearingMethod::ZeroCrossing];
    let expected_bearing_deg = 62.0;

    println!(
        "north_mode,bearing_method,scenario,buffer_size,bearing_success_rate,detection_rate,false_positive_rate,mean_us_per_sample,p95_us_per_sample,mean_abs_bearing_error_deg,p95_abs_bearing_error_deg,max_abs_bearing_error_deg,mean_abs_tick_error_samples,p95_abs_tick_error_samples"
    );

    for north_mode in north_modes {
        for bearing_method in bearing_methods {
            for scenario in scenarios {
                for &buffer_size in BUFFER_SIZES {
                    let m = run_case(
                        north_mode,
                        bearing_method,
                        scenario,
                        buffer_size,
                        expected_bearing_deg,
                    );
                    println!(
                        "{},{},{},{},{:.6},{:.6},{:.6},{:.9},{:.9},{:.6},{:.6},{:.6},{:.6},{:.6}",
                        north_mode_name(north_mode),
                        bearing_method_name(bearing_method),
                        scenario.name,
                        buffer_size,
                        m.bearing_success_rate,
                        m.detection_rate,
                        m.false_positive_rate,
                        m.mean_us_per_sample,
                        m.p95_us_per_sample,
                        m.mean_abs_bearing_error_deg,
                        m.p95_abs_bearing_error_deg,
                        m.max_abs_bearing_error_deg,
                        m.mean_abs_tick_error_samples,
                        m.p95_abs_tick_error_samples,
                    );
                }
            }
        }
    }
}

use rotaryclub::config::RdfConfig;
use rotaryclub::rdf::{
    BearingCalculator, CorrelationBearingCalculator, NorthTick, ZeroCrossingBearingCalculator,
};
use rotaryclub::simulation::noise_at as deterministic_noise_at;
use std::f32::consts::PI;
use std::time::Instant;

const BUFFER_SIZES: &[usize] = &[128, 256, 512, 1024];
const ITERATIONS: usize = 360;
const WARMUP_ITERATIONS: usize = 24;

#[derive(Clone, Copy)]
struct Scenario {
    name: &'static str,
    amplitude: f32,
    noise_peak: f32,
    dc_offset: f32,
    second_tone_ratio: f32,
    third_tone_ratio: f32,
}

#[derive(Clone, Copy)]
enum Method {
    Correlation,
    ZeroCrossing,
}

impl Method {
    fn as_str(self) -> &'static str {
        match self {
            Method::Correlation => "correlation",
            Method::ZeroCrossing => "zero_crossing",
        }
    }
}

fn percentile_us(values_us: &[f64], p: f64) -> f64 {
    if values_us.is_empty() {
        return 0.0;
    }
    let mut sorted = values_us.to_vec();
    sorted.sort_by(f64::total_cmp);
    let idx = ((sorted.len() as f64 - 1.0) * p.clamp(0.0, 1.0)).round() as usize;
    sorted[idx]
}

fn percentile_deg(values_deg: &[f64], p: f64) -> f64 {
    if values_deg.is_empty() {
        return 360.0;
    }
    let mut sorted = values_deg.to_vec();
    sorted.sort_by(f64::total_cmp);
    let idx = ((sorted.len() as f64 - 1.0) * p.clamp(0.0, 1.0)).round() as usize;
    sorted[idx]
}

fn angle_error_deg(measured: f32, expected: f32) -> f64 {
    let mut err = (measured - expected).abs();
    if err > 180.0 {
        err = 360.0 - err;
    }
    err as f64
}

fn make_north_tick(sample_index: usize, samples_per_rotation: f32) -> NorthTick {
    NorthTick {
        sample_index,
        period: Some(samples_per_rotation),
        lock_quality: None,
        phase_variance: None,
        reference_variance: None,
        fractional_sample_offset: 0.0,
        phase: 0.0,
        frequency: 2.0 * PI / samples_per_rotation,
    }
}

fn make_doppler_buffer(
    scenario: Scenario,
    buffer_size: usize,
    omega: f32,
    phase_offset: f32,
    step_index: usize,
    draw: u64,
) -> Vec<f32> {
    let second_omega = omega * 2.0;
    let third_omega = omega * 3.0;
    (0..buffer_size)
        .map(|i| {
            let t = (step_index * buffer_size + i) as f32;
            let fundamental = (omega * t - phase_offset).sin();
            let second_tone = (second_omega * t - (phase_offset * 0.7)).sin();
            let third_tone = (third_omega * t - (phase_offset * 0.5)).sin();
            let noise = deterministic_noise_at(
                i + step_index * buffer_size,
                0xA5A5_1234_5EED_1111u64.wrapping_add(draw),
            );
            scenario.amplitude * fundamental
                + scenario.second_tone_ratio * second_tone
                + scenario.third_tone_ratio * third_tone
                + scenario.noise_peak * noise
                + scenario.dc_offset
        })
        .collect()
}

fn run_case(
    method: Method,
    scenario: Scenario,
    buffer_size: usize,
    expected_bearing_deg: f32,
    draw: u64,
) -> (usize, Vec<f64>, Vec<f64>) {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;
    let samples_per_rotation = sample_rate / rotation_hz;
    let omega = 2.0 * PI * rotation_hz / sample_rate;
    let phase_offset = 62.0f32.to_radians();
    let smoothing = 1usize;

    let mut calc: Box<dyn BearingCalculator> = match method {
        Method::Correlation => Box::new(
            CorrelationBearingCalculator::new(
                &config.doppler,
                &config.agc,
                config.bearing.confidence,
                sample_rate,
                smoothing,
            )
            .expect("correlation calculator creation must succeed"),
        ),
        Method::ZeroCrossing => Box::new(
            ZeroCrossingBearingCalculator::new(
                &config.doppler,
                &config.agc,
                config.bearing.confidence,
                sample_rate,
                smoothing,
            )
            .expect("zero-crossing calculator creation must succeed"),
        ),
    };

    for step in 0..WARMUP_ITERATIONS {
        let tick = make_north_tick(0, samples_per_rotation);
        let buffer = make_doppler_buffer(scenario, buffer_size, omega, phase_offset, step, draw);
        calc.preprocess(&buffer);
        let _ = calc.process_tick(&tick);
        calc.advance_buffer();
    }

    let mut measured_count = 0usize;
    let mut times_us = Vec::with_capacity(ITERATIONS);
    let mut errors_deg = Vec::with_capacity(ITERATIONS);
    for step in WARMUP_ITERATIONS..(WARMUP_ITERATIONS + ITERATIONS) {
        let tick = make_north_tick(0, samples_per_rotation);
        let buffer = make_doppler_buffer(scenario, buffer_size, omega, phase_offset, step, draw);

        let start = Instant::now();
        calc.preprocess(&buffer);
        let measurement = calc.process_tick(&tick);
        calc.advance_buffer();
        let elapsed_us = start.elapsed().as_secs_f64() * 1_000_000.0;
        times_us.push(elapsed_us);

        if let Some(m) = measurement {
            measured_count += 1;
            errors_deg.push(angle_error_deg(m.raw_bearing, expected_bearing_deg));
        }
    }
    (measured_count, times_us, errors_deg)
}

/// Independent noise realisations pooled into each reported row.
///
/// One draw is not a measurement. This harness is far steadier than the system
/// pipeline one -- it has no tracker to latch -- but the reason to pool is the
/// same, and it costs proportionally little here.
const DRAWS: u64 = 8;

fn main() {
    let scenarios = [
        Scenario {
            name: "clean",
            amplitude: 1.0,
            noise_peak: 0.0,
            dc_offset: 0.0,
            second_tone_ratio: 0.0,
            third_tone_ratio: 0.0,
        },
        Scenario {
            name: "noisy",
            amplitude: 0.9,
            noise_peak: 0.08,
            dc_offset: 0.0,
            second_tone_ratio: 0.0,
            third_tone_ratio: 0.0,
        },
        Scenario {
            name: "dc_offset",
            amplitude: 0.9,
            noise_peak: 0.03,
            dc_offset: 0.2,
            second_tone_ratio: 0.0,
            third_tone_ratio: 0.0,
        },
        Scenario {
            name: "multipath_like",
            amplitude: 0.8,
            noise_peak: 0.04,
            dc_offset: 0.0,
            second_tone_ratio: 0.35,
            third_tone_ratio: 0.0,
        },
        Scenario {
            name: "harmonic_contaminated",
            amplitude: 0.9,
            noise_peak: 0.04,
            dc_offset: 0.0,
            second_tone_ratio: 0.20,
            third_tone_ratio: 0.12,
        },
        Scenario {
            name: "low_snr_dc",
            amplitude: 0.45,
            noise_peak: 0.40,
            dc_offset: 0.20,
            second_tone_ratio: 0.0,
            third_tone_ratio: 0.0,
        },
    ];
    let methods = [Method::Correlation, Method::ZeroCrossing];
    let expected_bearing_deg = 62.0f32;

    for method in methods {
        for scenario in scenarios {
            for &buffer_size in BUFFER_SIZES {
                // Pooled over independent noise realisations rather than
                // taken from one. Pooling the raw per-iteration samples, not
                // the summaries, so the percentiles still describe a
                // distribution rather than an average of percentiles.
                let (mut measured_count, mut times_us, mut errors_deg) = (0usize, vec![], vec![]);
                // The extreme-value columns are kept per draw and averaged,
                // not taken over the pool. A maximum over eight times as many
                // samples is larger for that reason alone, which would make
                // the column mean something different from the limit set
                // against it; the mean of the per-run maxima is the same
                // quantity as before, measured more times.
                let mut per_draw_max_error: Vec<f64> = Vec::new();
                let mut per_draw_max_us: Vec<f64> = Vec::new();
                // Each draw's own summary, kept so the spread across draws can
                // be reported. The values below are still pooled; these say how
                // far a single draw moves, which is what decides whether a row
                // supports a verdict.
                let mut per_draw_success: Vec<f64> = Vec::new();
                let mut per_draw_mean_error: Vec<f64> = Vec::new();
                let mut per_draw_p95_error: Vec<f64> = Vec::new();
                let mut per_draw_mean_us: Vec<f64> = Vec::new();
                let mut per_draw_p95_us: Vec<f64> = Vec::new();
                for draw in 0..DRAWS {
                    let (c, t, e) =
                        run_case(method, scenario, buffer_size, expected_bearing_deg, draw);
                    measured_count += c;
                    per_draw_max_us.push(t.iter().copied().fold(0.0, f64::max));
                    per_draw_max_error.push(e.iter().copied().fold(0.0, f64::max));
                    per_draw_success.push(if t.is_empty() {
                        0.0
                    } else {
                        c as f64 / t.len() as f64
                    });
                    per_draw_mean_error.push(if e.is_empty() {
                        360.0
                    } else {
                        e.iter().sum::<f64>() / e.len() as f64
                    });
                    per_draw_p95_error.push(percentile_deg(&e, 0.95));
                    per_draw_mean_us.push(if t.is_empty() {
                        0.0
                    } else {
                        t.iter().sum::<f64>() / t.len() as f64
                    });
                    per_draw_p95_us.push(percentile_us(&t, 0.95));
                    times_us.extend(t);
                    errors_deg.extend(e);
                }
                let se_of = |v: &[f64], scale: f64| -> f64 {
                    let n = v.len() as f64;
                    if n < 2.0 {
                        return 0.0;
                    }
                    let m = v.iter().sum::<f64>() / n;
                    let var = v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (n - 1.0);
                    (var / n).sqrt() / scale
                };
                let mean_of = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
                let iterations = times_us.len();
                let sum_us: f64 = times_us.iter().sum();
                let mean_us = if iterations > 0 {
                    sum_us / iterations as f64
                } else {
                    0.0
                };
                let p95_us = percentile_us(&times_us, 0.95);
                let max_us = mean_of(&per_draw_max_us);
                let success_rate = if iterations > 0 {
                    measured_count as f64 / iterations as f64
                } else {
                    0.0
                };
                let mean_us_per_sample = mean_us / buffer_size as f64;
                let p95_us_per_sample = p95_us / buffer_size as f64;
                let mean_abs_bearing_error_deg = if errors_deg.is_empty() {
                    360.0
                } else {
                    errors_deg.iter().sum::<f64>() / errors_deg.len() as f64
                };
                let p95_abs_bearing_error_deg = percentile_deg(&errors_deg, 0.95);
                let max_abs_bearing_error_deg = mean_of(&per_draw_max_error);
                println!(
                    "{}",
                    serde_json::json!({
                        "method": method.as_str(),
                        "scenario": scenario.name,
                        "buffer_size": buffer_size,
                        "iterations": iterations,
                        "measured_count": measured_count,
                        "success_rate": success_rate,
                        "mean_us": mean_us,
                        "p95_us": p95_us,
                        "max_us": max_us,
                        "mean_us_per_sample": mean_us_per_sample,
                        "p95_us_per_sample": p95_us_per_sample,
                        "mean_abs_bearing_error_deg": mean_abs_bearing_error_deg,
                        "p95_abs_bearing_error_deg": p95_abs_bearing_error_deg,
                        "max_abs_bearing_error_deg": max_abs_bearing_error_deg,
                        // Spread across draws, so `check` can ask whether the
                        // row is precise enough to support its verdict.
                        "success_rate_se": se_of(&per_draw_success, 1.0),
                        "mean_us_per_sample_se": se_of(&per_draw_mean_us, buffer_size as f64),
                        "p95_us_per_sample_se": se_of(&per_draw_p95_us, buffer_size as f64),
                        "mean_abs_bearing_error_deg_se": se_of(&per_draw_mean_error, 1.0),
                        "p95_abs_bearing_error_deg_se": se_of(&per_draw_p95_error, 1.0),
                        "max_abs_bearing_error_deg_se": se_of(&per_draw_max_error, 1.0),
                        "draws": DRAWS,
                    })
                );
            }
        }
    }
}

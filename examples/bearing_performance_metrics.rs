use rotaryclub::config::RdfConfig;
use rotaryclub::rdf::{
    BearingCalculator, CorrelationBearingCalculator, NorthTick, ZeroCrossingBearingCalculator,
};
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

fn make_north_tick(sample_index: usize, samples_per_rotation: f32) -> NorthTick {
    NorthTick {
        sample_index,
        period: Some(samples_per_rotation),
        lock_quality: None,
        fractional_sample_offset: 0.0,
        phase: 0.0,
        frequency: 2.0 * PI / samples_per_rotation,
    }
}

fn deterministic_noise_at(index: usize, seed: u64) -> f32 {
    let mut x = seed ^ ((index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
    let u = (((x >> 33) as u32) as f32) / (u32::MAX as f32);
    2.0 * u - 1.0
}

fn make_doppler_buffer(
    scenario: Scenario,
    buffer_size: usize,
    omega: f32,
    phase_offset: f32,
    step_index: usize,
) -> Vec<f32> {
    let second_omega = omega * 1.01;
    (0..buffer_size)
        .map(|i| {
            let t = (step_index * buffer_size + i) as f32;
            let fundamental = (omega * t - phase_offset).sin();
            let second_tone = (second_omega * t - (phase_offset * 0.7)).sin();
            let noise = deterministic_noise_at(i + step_index * buffer_size, 0xA5A5_1234_5EED_1111);
            scenario.amplitude * fundamental
                + scenario.second_tone_ratio * second_tone
                + scenario.noise_peak * noise
                + scenario.dc_offset
        })
        .collect()
}

fn run_case(method: Method, scenario: Scenario, buffer_size: usize) -> (usize, Vec<f64>) {
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
                config.bearing.confidence_weights,
                sample_rate,
                smoothing,
            )
            .expect("correlation calculator creation must succeed"),
        ),
        Method::ZeroCrossing => Box::new(
            ZeroCrossingBearingCalculator::new(
                &config.doppler,
                &config.agc,
                config.bearing.confidence_weights,
                sample_rate,
                smoothing,
            )
            .expect("zero-crossing calculator creation must succeed"),
        ),
    };

    for step in 0..WARMUP_ITERATIONS {
        let tick = make_north_tick(step * buffer_size, samples_per_rotation);
        let buffer = make_doppler_buffer(scenario, buffer_size, omega, phase_offset, step);
        calc.preprocess(&buffer);
        let _ = calc.process_tick(&tick);
        calc.advance_buffer();
    }

    let mut measured_count = 0usize;
    let mut times_us = Vec::with_capacity(ITERATIONS);
    for step in WARMUP_ITERATIONS..(WARMUP_ITERATIONS + ITERATIONS) {
        let tick = make_north_tick(step * buffer_size, samples_per_rotation);
        let buffer = make_doppler_buffer(scenario, buffer_size, omega, phase_offset, step);

        let start = Instant::now();
        calc.preprocess(&buffer);
        let measurement = calc.process_tick(&tick);
        calc.advance_buffer();
        let elapsed_us = start.elapsed().as_secs_f64() * 1_000_000.0;
        times_us.push(elapsed_us);

        if measurement.is_some() {
            measured_count += 1;
        }
    }
    (measured_count, times_us)
}

fn main() {
    let scenarios = [
        Scenario {
            name: "clean",
            amplitude: 1.0,
            noise_peak: 0.0,
            dc_offset: 0.0,
            second_tone_ratio: 0.0,
        },
        Scenario {
            name: "noisy",
            amplitude: 0.9,
            noise_peak: 0.08,
            dc_offset: 0.0,
            second_tone_ratio: 0.0,
        },
        Scenario {
            name: "dc_offset",
            amplitude: 0.9,
            noise_peak: 0.03,
            dc_offset: 0.2,
            second_tone_ratio: 0.0,
        },
        Scenario {
            name: "multipath_like",
            amplitude: 0.8,
            noise_peak: 0.04,
            dc_offset: 0.0,
            second_tone_ratio: 0.35,
        },
    ];
    let methods = [Method::Correlation, Method::ZeroCrossing];

    println!(
        "method,scenario,buffer_size,iterations,measured_count,success_rate,mean_us,p95_us,max_us,mean_us_per_sample,p95_us_per_sample"
    );
    for method in methods {
        for scenario in scenarios {
            for &buffer_size in BUFFER_SIZES {
                let (measured_count, times_us) = run_case(method, scenario, buffer_size);
                let iterations = times_us.len();
                let sum_us: f64 = times_us.iter().sum();
                let mean_us = if iterations > 0 {
                    sum_us / iterations as f64
                } else {
                    0.0
                };
                let p95_us = percentile_us(&times_us, 0.95);
                let max_us = times_us.iter().copied().fold(0.0, f64::max);
                let success_rate = if iterations > 0 {
                    measured_count as f64 / iterations as f64
                } else {
                    0.0
                };
                let mean_us_per_sample = mean_us / buffer_size as f64;
                let p95_us_per_sample = p95_us / buffer_size as f64;
                println!(
                    "{},{},{},{},{},{:.6},{:.6},{:.6},{:.6},{:.9},{:.9}",
                    method.as_str(),
                    scenario.name,
                    buffer_size,
                    iterations,
                    measured_count,
                    success_rate,
                    mean_us,
                    p95_us,
                    max_us,
                    mean_us_per_sample,
                    p95_us_per_sample
                );
            }
        }
    }
}

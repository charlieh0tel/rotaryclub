//! Where does the half-sample bearing offset come from?
//!
//! `doppler.north_tick_timing_adjustment_us` defaults to half a sample and it
//! is not clear what it compensates. Two harnesses disagree: against a north
//! pulse placed at the true rotation epoch the trim costs six degrees, while
//! against the system pipeline example -- whose pulses sit at
//! `round(k * period)` -- removing it costs five.
//!
//! Neither generator is biased, so the disagreement has to come from
//! something else that differs between them. This varies the two candidates
//! independently, with everything else held fixed and no noise or jitter, and
//! reports mean bearing error. A factor that flips the sign of the offset is
//! the one carrying it.

use std::f32::consts::PI;

use rotaryclub::config::{BearingMethod, RdfConfig, RotationFrequency};
use rotaryclub::processing::RdfProcessor;

const HALF_SAMPLE_US: f32 = 10.416_667;

#[derive(Clone, Copy, PartialEq)]
enum Placement {
    /// Pulse at the exact rotation epoch, which is generally fractional.
    TrueEpoch,
    /// Pulse at the nearest sample to it, as the pipeline example does.
    Rounded,
}

/// Deterministic jitter, matching the system pipeline example's generator.
fn jitter_samples(index: usize, max_abs: i32) -> i32 {
    if max_abs <= 0 {
        0
    } else {
        ((index as f32 * 0.37).sin() * max_abs as f32).round() as i32
    }
}

/// Deterministic noise, so a run is repeatable.
fn noise_at(index: usize) -> f32 {
    let mut x = (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xBEEF_1234;
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    (((x >> 33) as u32) as f32 / (u32::MAX as f32)) * 2.0 - 1.0
}

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    /// One non-zero sample, as the pipeline example does.
    Impulse,
    /// Band-limited impulse, which is what an anti-aliased converter records.
    BandLimited,
}

#[allow(clippy::too_many_arguments)]
fn build_signal(
    num_samples: usize,
    sample_rate: f32,
    rotation_hz: f32,
    bearing_deg: f32,
    placement: Placement,
    shape: Shape,
    amplitude: f32,
    jitter: i32,
    noise_peak: f32,
) -> Vec<f32> {
    let period = sample_rate as f64 / rotation_hz as f64;
    let bearing = bearing_deg.to_radians();
    let omega = 2.0 * PI * rotation_hz / sample_rate;

    let mut north = vec![0.0f32; num_samples];
    let half = 12i64;
    let mut k = 0i64;
    loop {
        let epoch = k as f64 * period;
        if epoch >= num_samples as f64 - half as f64 {
            break;
        }
        let placed = match placement {
            Placement::TrueEpoch => epoch,
            Placement::Rounded => epoch.round(),
        } + jitter_samples(k as usize, jitter) as f64;
        match shape {
            Shape::Impulse => {
                let index = placed.round() as usize;
                if index < num_samples {
                    north[index] += amplitude;
                }
            }
            Shape::BandLimited => {
                let center = placed.round() as i64;
                for n in (center - half)..=(center + half) {
                    if n < 0 || n as usize >= num_samples {
                        continue;
                    }
                    let x = n as f64 - placed;
                    let value = if x.abs() < f64::EPSILON {
                        1.0
                    } else {
                        let px = std::f64::consts::PI * x;
                        let w = px / half as f64;
                        (px.sin() / px) * (w.sin() / w)
                    };
                    north[n as usize] += amplitude * value as f32;
                }
            }
        }
        k += 1;
    }

    let mut out = Vec::with_capacity(num_samples * 2);
    for (i, &tick) in north.iter().enumerate() {
        out.push((omega * i as f32 - bearing).sin() + noise_peak * noise_at(i));
        out.push(tick + noise_peak * 0.35 * noise_at(i ^ 0x5555));
    }
    out
}

fn measure(signal: &[f32], config: &RdfConfig, truth: f32) -> Option<f32> {
    let mut processor = RdfProcessor::new(config, false, true).ok()?;
    let bearings: Vec<f32> = processor
        .process_signal(signal)
        .iter()
        .filter_map(|r| r.bearing.map(|b| b.bearing_degrees))
        .collect();
    if bearings.len() < 6 {
        return None;
    }
    let (mut x, mut y) = (0.0f32, 0.0f32);
    for bearing in &bearings[3..] {
        let radians = bearing.to_radians();
        x += radians.cos();
        y += radians.sin();
    }
    let mean = y.atan2(x).to_degrees().rem_euclid(360.0);
    Some(((mean - truth + 540.0).rem_euclid(360.0)) - 180.0)
}

fn main() {
    let sample_rate = 48_000u32;
    let rotation_hz = 1602.564f32;
    let num_samples = (sample_rate as f32 * 0.5) as usize;
    let truth = 200.0f32;
    let one_sample_us = 1e6 / sample_rate as f32;

    println!(
        "mean bearing error, degrees. one sample = {:.1} deg = {:.2} us\n",
        360.0 * rotation_hz / sample_rate as f32,
        one_sample_us
    );

    let conditions: [(&str, Placement, Shape, i32, f32); 5] = [
        (
            "clean, true epoch",
            Placement::TrueEpoch,
            Shape::BandLimited,
            0,
            0.0,
        ),
        ("clean, impulse", Placement::Rounded, Shape::Impulse, 0, 0.0),
        ("jitter only", Placement::Rounded, Shape::Impulse, 1, 0.0),
        ("noise only", Placement::Rounded, Shape::Impulse, 0, 0.08),
        (
            "jitter + noise",
            Placement::Rounded,
            Shape::Impulse,
            1,
            0.08,
        ),
    ];

    let trims: [f32; 5] = [
        -HALF_SAMPLE_US,
        -HALF_SAMPLE_US / 2.0,
        0.0,
        HALF_SAMPLE_US / 2.0,
        HALF_SAMPLE_US,
    ];

    print!("{:<20}", "condition");
    for trim in trims {
        print!("{:>12}", format!("{:+.1}us", trim));
    }
    println!();

    for (name, placement, shape, jitter, noise) in conditions {
        print!("{name:<20}");
        for trim in trims {
            let mut config = RdfConfig::default();
            config.audio.sample_rate = sample_rate;
            config.apply_rotation(RotationFrequency::from_hz(rotation_hz));
            config.doppler.method = BearingMethod::Correlation;
            config.doppler.north_tick_timing_adjustment_us = trim;

            let signal = build_signal(
                num_samples,
                sample_rate as f32,
                rotation_hz,
                truth,
                placement,
                shape,
                config.north_tick.expected_pulse_amplitude,
                jitter,
                noise,
            );
            match measure(&signal, &config, truth) {
                Some(e) => print!("{:>12}", format!("{e:+.2}")),
                None => print!("{:>12}", "none"),
            }
        }
        println!();
    }
}

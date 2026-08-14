//! Why does the north AGC make things worse under heavy noise?
//!
//! Turning it on takes the stated bearing uncertainty from just above the
//! observed scatter to just below it at the extreme -- 81.3 degrees claimed
//! against 85.4 observed, where with it off the figure stays above. That is a
//! small margin in a regime where the bearing is already worthless, but the
//! direction is the unsafe one and the mechanism is worth knowing.
//!
//! The suspicion is that `observe` takes every detected peak without asking
//! whether it was a pulse, so under noise it adapts to false detections. This
//! reports the gain and the timing it produces against noise level.

use rotaryclub::config::{NorthTrackingMode, RdfConfig};
use rotaryclub::rdf::{NorthReferenceTracker, NorthTracker};

const PULSE_HALF_WIDTH: i64 = 12;

fn noise_at(index: usize, seed: u64) -> f32 {
    let mut x = (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ seed;
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 29;
    (((x >> 32) as u32) as f32 / (u32::MAX as f32)) * 2.0 - 1.0
}

fn build(num_samples: usize, period: f64, amplitude: f32, noise_rms: f32) -> (Vec<f32>, Vec<f64>) {
    let mut signal = vec![0.0f32; num_samples];
    let mut epochs = Vec::new();
    let mut k = 0i64;
    loop {
        let epoch = 100.0 + k as f64 * period;
        if epoch >= num_samples as f64 - PULSE_HALF_WIDTH as f64 {
            break;
        }
        epochs.push(epoch);
        k += 1;
        let center = epoch.round() as i64;
        for n in (center - PULSE_HALF_WIDTH)..=(center + PULSE_HALF_WIDTH) {
            if n < 0 || n as usize >= num_samples {
                continue;
            }
            let x = n as f64 - epoch;
            let value = if x.abs() < f64::EPSILON {
                1.0
            } else {
                let px = std::f64::consts::PI * x;
                let w = px / PULSE_HALF_WIDTH as f64;
                (px.sin() / px) * (w.sin() / w)
            };
            signal[n as usize] += amplitude * value as f32;
        }
    }
    if noise_rms > 0.0 {
        for (i, sample) in signal.iter_mut().enumerate() {
            let mut acc = 0.0f32;
            for j in 0..12 {
                acc += noise_at(i * 12 + j, 0x5EED_2222);
            }
            *sample += acc / 2.0 * noise_rms;
        }
    }
    (signal, epochs)
}

struct Result {
    detection: f64,
    false_positive: f64,
    tick_error: f64,
}

fn run_mode(mode: NorthTrackingMode, agc: bool, noise_rms: f32) -> Result {
    let mut config = RdfConfig::default();
    config.north_tick.mode = mode;
    config.north_tick.agc.enabled = agc;

    let sample_rate = config.audio.sample_rate as f32;
    let period = sample_rate as f64 / config.doppler.expected_freq as f64;
    let (signal, epochs) = build(
        (sample_rate * 4.0) as usize,
        period,
        config.north_tick.expected_pulse_amplitude,
        noise_rms,
    );

    let mut tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate).expect("tracker");
    let mut ticks = Vec::new();
    for chunk in signal.chunks(512) {
        for tick in tracker.process_buffer(chunk) {
            ticks.push(tick.sample_index as f64 + tick.fractional_sample_offset as f64);
        }
    }

    let start = epochs.len() / 2;
    let mut matched = 0usize;
    let mut errors = Vec::new();
    for epoch in &epochs[start..] {
        if let Some(t) = ticks
            .iter()
            .min_by(|a, b| (*a - epoch).abs().total_cmp(&(*b - epoch).abs()))
            && (t - epoch).abs() < 3.0
        {
            matched += 1;
            errors.push((t - epoch).abs());
        }
    }
    let expected = (epochs.len() - start).max(1);
    let late: Vec<&f64> = ticks.iter().filter(|t| **t > epochs[start]).collect();
    Result {
        detection: matched as f64 / expected as f64,
        false_positive: (late.len().saturating_sub(matched)) as f64 / expected as f64,
        tick_error: errors.iter().sum::<f64>() / errors.len().max(1) as f64,
    }
}

fn main() {
    println!("north channel noise against detection, false positives and tick error");
    println!(
        "{:>8}  {:>26}  {:>26}",
        "noise", "agc off (det/fp/err)", "agc on (det/fp/err)"
    );
    for mode in [NorthTrackingMode::Dpll, NorthTrackingMode::Simple] {
        println!("--- {mode:?}");
        for noise in [0.0f32, 0.05, 0.10, 0.20, 0.30, 0.40] {
            let off = run_mode(mode, false, noise);
            let on = run_mode(mode, true, noise);
            println!(
                "{:>8.2}  {:>8.3}/{:.3}/{:.4}  {:>8.3}/{:.3}/{:.4}",
                noise,
                off.detection,
                off.false_positive,
                off.tick_error,
                on.detection,
                on.false_positive,
                on.tick_error
            );
        }
    }
}

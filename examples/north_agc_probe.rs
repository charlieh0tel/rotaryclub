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
use rotaryclub::simulation::noise_at;

const PULSE_HALF_WIDTH: i64 = 12;

fn build(
    num_samples: usize,
    period: f64,
    amplitude: f32,
    noise_rms: f32,
    draw: u64,
) -> (Vec<f32>, Vec<f64>) {
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
                acc += noise_at(i * 12 + j, 0x5EED_2222u64.wrapping_add(draw));
            }
            *sample += acc / 2.0 * noise_rms;
        }
    }
    (signal, epochs)
}

struct Result {
    detection: f64,
    false_positive: f64,
    detection_se: f64,
    tick_error_se: f64,
    tick_error: f64,
}

fn run_mode(mode: NorthTrackingMode, agc: bool, noise_rms: f32, draw: u64) -> Result {
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
        draw,
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
        detection_se: 0.0,
        tick_error_se: 0.0,
    }
}

/// Independent noise realisations averaged into each cell.
///
/// This probe decided how the north AGC works -- that averaging every peak
/// drives a runaway, that a median over accepted pulses does not, when to
/// freeze -- and every one of those rows was a single draw. The effects it
/// found are large, but that was a judgement made by eye rather than a
/// measurement, which is the same mistake in a different place.
const DRAWS: u64 = 12;

/// Per-draw detection rates, so a wide error bar can be read for what it is.
///
/// A mean with a large standard error can mean a broad unimodal spread or two
/// values it is sitting between, and only the second makes the mean
/// meaningless. This project has already been caught by the second once.
fn spread_of(mode: NorthTrackingMode, agc: bool, noise_rms: f32) -> Vec<f64> {
    (0..DRAWS)
        .map(|d| run_mode(mode, agc, noise_rms, d).detection)
        .collect()
}

fn average(mode: NorthTrackingMode, agc: bool, noise_rms: f32) -> Result {
    let runs: Vec<Result> = (0..DRAWS)
        .map(|d| run_mode(mode, agc, noise_rms, d))
        .collect();
    let n = runs.len() as f64;
    let mean = |f: fn(&Result) -> f64| runs.iter().map(f).sum::<f64>() / n;
    let se = |f: fn(&Result) -> f64| {
        let m = mean(f);
        let var = runs.iter().map(|r| (f(r) - m) * (f(r) - m)).sum::<f64>() / (n - 1.0);
        (var / n).sqrt()
    };
    Result {
        detection: mean(|r| r.detection),
        false_positive: mean(|r| r.false_positive),
        tick_error: mean(|r| r.tick_error),
        detection_se: se(|r| r.detection),
        tick_error_se: se(|r| r.tick_error),
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
            let off = average(mode, false, noise);
            let on = average(mode, true, noise);
            println!(
                "{:>8.2}  {:>6.3}+-{:.3}/{:.3}/{:.4}+-{:.4}  {:>6.3}+-{:.3}/{:.3}/{:.4}+-{:.4}",
                noise,
                off.detection,
                off.detection_se,
                off.false_positive,
                off.tick_error,
                off.tick_error_se,
                on.detection,
                on.detection_se,
                on.false_positive,
                on.tick_error,
                on.tick_error_se
            );
        }
    }

    println!("\nper-draw detection where the error bar was widest");
    for (mode, noise) in [
        (NorthTrackingMode::Simple, 0.10f32),
        (NorthTrackingMode::Simple, 0.20),
    ] {
        let mut v = spread_of(mode, false, noise);
        v.sort_by(f64::total_cmp);
        println!(
            "  {mode:?} at {noise:.2}: {}",
            v.iter()
                .map(|x| format!("{x:.2}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
}

//! Why does a narrow loop earn almost no holdover?
//!
//! Bandwidths at or below 0.5 Hz coast three or four rotations where 1 Hz
//! coasts until the cap, and the predictions they do make are accurate to a
//! thousandth of a sample. This runs one bandwidth at a time so the budget's
//! own terms can be read at the moment the pulses stop.

use rotaryclub::config::{NorthTrackingMode, RdfConfig};
use rotaryclub::rdf::{NorthReferenceTracker, NorthTracker};

const HALF: i64 = 12;

fn build(num_samples: usize, period: f64, amplitude: f32, pulses_until: usize) -> Vec<f32> {
    let mut signal = vec![0.0f32; num_samples];
    let mut k = 0i64;
    loop {
        let epoch = 100.0 + k as f64 * period;
        if epoch >= num_samples as f64 - HALF as f64 {
            break;
        }
        k += 1;
        if epoch >= pulses_until as f64 {
            continue;
        }
        let center = epoch.round() as i64;
        for n in (center - HALF)..=(center + HALF) {
            if n < 0 || n as usize >= num_samples {
                continue;
            }
            let x = n as f64 - epoch;
            let value = if x.abs() < f64::EPSILON {
                1.0
            } else {
                let px = std::f64::consts::PI * x;
                let w = px / HALF as f64;
                (px.sin() / px) * (w.sin() / w)
            };
            signal[n as usize] += amplitude * value as f32;
        }
    }
    signal
}

fn main() {
    let bandwidth: f32 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(0.5);

    let base = RdfConfig::default();
    let sample_rate = base.audio.sample_rate as f32;
    let period = sample_rate as f64 / base.doppler.expected_freq as f64;

    let mut config = RdfConfig::default();
    config.north_tick.mode = NorthTrackingMode::Dpll;
    config.north_tick.dpll.natural_frequency_hz = bandwidth;
    config.north_tick.max_coast_ms = 5000.0;

    // A rate the loop was not configured for, with only enough time to be
    // part way to it, is the case the budget's second term exists to catch:
    // the rate is steadily wrong while its scatter is small.
    let (signal_period, settle_secs) = match std::env::args().nth(2).as_deref() {
        Some("converging") => (sample_rate as f64 / 1590.0, 0.30f32),
        _ => (period, 10.0),
    };
    let settle = (sample_rate * settle_secs) as usize;
    let total = settle + (sample_rate * 5.0) as usize;
    let signal = build(
        total,
        signal_period,
        config.north_tick.expected_pulse_amplitude,
        settle,
    );

    let mut tracker =
        NorthReferenceTracker::new(&config.north_tick, sample_rate).expect("tracker config");
    let mut coasted = 0usize;
    for (i, chunk) in signal.chunks(512).enumerate() {
        let at = i * 512;
        for tick in tracker.process_buffer(chunk) {
            if tick.sample_index > settle {
                coasted += 1;
            }
        }
        if at > settle && at < settle + 4096 {
            eprintln!("--- at {at}, just past the last pulse");
        }
    }
    println!(
        "bandwidth {bandwidth} Hz, {}: coasted {coasted} rotations",
        std::env::args().nth(2).unwrap_or_else(|| "settled".into())
    );
}

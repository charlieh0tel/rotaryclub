//! Does the detection threshold have margin, and does it need to adapt?
//!
//! `north_tick.threshold` and `north_tick.expected_pulse_amplitude` are both
//! absolute, so together they assume a signal level. A receiver delivering
//! half the expected pulse height sits closer to the threshold than intended,
//! and nothing in the pipeline says so. DESIGN.md argues adaptive
//! thresholding is unnecessary because the reference amplitude is
//! predictable; that is an assumption rather than a measurement.
//!
//! This sweeps actual pulse amplitude against threshold, at several noise
//! levels, and reports detection and false-positive rates. The useful region
//! is where both hold: detection near one, false positives near zero. How
//! wide that region is says how much level error the shipped pair tolerates.

use std::collections::BTreeSet;

use rotaryclub::config::{NorthTrackingMode, RdfConfig};
use rotaryclub::rdf::{NorthReferenceTracker, NorthTracker};

const ROTATION_HZ: f32 = 1602.564;

fn noise_at(index: usize, seed: u64) -> f32 {
    let mut x = (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ seed;
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 29;
    (((x >> 32) as u32) as f32 / (u32::MAX as f32)) * 2.0 - 1.0
}

/// Band-limited impulses at the true rotation epochs, plus noise.
fn build(
    num_samples: usize,
    sample_rate: f32,
    amplitude: f32,
    noise_rms: f32,
) -> (Vec<f32>, Vec<f64>) {
    let period = sample_rate as f64 / ROTATION_HZ as f64;
    let mut signal = vec![0.0f32; num_samples];
    let mut epochs = Vec::new();
    let half = 12i64;

    let mut k = 0i64;
    loop {
        let epoch = 100.3 + k as f64 * period;
        if epoch >= num_samples as f64 - half as f64 {
            break;
        }
        epochs.push(epoch);
        let center = epoch.round() as i64;
        for n in (center - half)..=(center + half) {
            if n < 0 || n as usize >= num_samples {
                continue;
            }
            let x = n as f64 - epoch;
            let value = if x.abs() < f64::EPSILON {
                1.0
            } else {
                let px = std::f64::consts::PI * x;
                let w = px / half as f64;
                (px.sin() / px) * (w.sin() / w)
            };
            signal[n as usize] += amplitude * value as f32;
        }
        k += 1;
    }

    if noise_rms > 0.0 {
        for (i, sample) in signal.iter_mut().enumerate() {
            // Twelve uniform draws approximate a normal.
            let mut acc = 0.0f32;
            for j in 0..12 {
                acc += noise_at(i * 12 + j, 0xBEEF_1234_5678_9ABC);
            }
            *sample += acc / 6.0 * noise_rms;
        }
    }

    (signal, epochs)
}

struct Rates {
    detection: f64,
    false_positive: f64,
}

fn run(amplitude: f32, threshold: f32, noise_rms: f32, sample_rate: f32) -> Rates {
    let mut config = RdfConfig::default();
    config.north_tick.mode = NorthTrackingMode::Simple;
    config.north_tick.threshold = threshold;
    let num_samples = (sample_rate * 1.5) as usize;
    let (signal, truth) = build(num_samples, sample_rate, amplitude, noise_rms);

    let Ok(mut tracker) = NorthReferenceTracker::new(&config.north_tick, sample_rate) else {
        return Rates {
            detection: f64::NAN,
            false_positive: f64::NAN,
        };
    };

    let mut ticks = Vec::new();
    for chunk in signal.chunks(512) {
        for tick in tracker.process_buffer(chunk) {
            ticks.push(tick.sample_index as f64 + tick.fractional_sample_offset as f64);
        }
    }

    let mut matched: BTreeSet<usize> = BTreeSet::new();
    let mut unmatched = 0usize;
    for tick in &ticks {
        let nearest = truth
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| (*a - tick).abs().total_cmp(&(*b - tick).abs()));
        match nearest {
            Some((index, epoch)) if (epoch - tick).abs() <= 3.0 => {
                matched.insert(index);
            }
            _ => unmatched += 1,
        }
    }

    let expected = truth.len().max(1) as f64;
    Rates {
        detection: matched.len() as f64 / expected,
        false_positive: unmatched as f64 / expected,
    }
}

fn main() {
    let sample_rate = RdfConfig::default().audio.sample_rate as f32;
    let shipped_threshold = RdfConfig::default().north_tick.threshold;
    let shipped_amplitude = RdfConfig::default().north_tick.expected_pulse_amplitude;

    println!(
        "shipped: threshold {shipped_threshold}, expected_pulse_amplitude {shipped_amplitude}"
    );
    println!("cells are detection / false positives, at the shipped threshold\n");

    let amplitudes = [1.0f32, 0.8, 0.6, 0.4, 0.3, 0.2, 0.15, 0.1];

    for noise_rms in [0.0f32, 0.05, 0.10, 0.20, 0.40, 0.80] {
        println!("noise rms {noise_rms}");
        print!("{:<12}", "amplitude");
        for a in amplitudes {
            print!("{a:>13.2}");
        }
        println!();

        print!("{:<12}", "detect/fp");
        for a in amplitudes {
            let r = run(a, shipped_threshold, noise_rms, sample_rate);
            print!(
                "{:>13}",
                format!("{:.2}/{:.2}", r.detection, r.false_positive)
            );
        }
        println!("\n");
    }

    println!("threshold sweep at the shipped amplitude {shipped_amplitude}, by noise level\n");
    print!("{:<12}", "threshold");
    let thresholds = [0.05f32, 0.10, 0.15, 0.25, 0.40, 0.60, 0.75];
    for t in thresholds {
        print!("{t:>13.2}");
    }
    println!();
    for noise_rms in [0.05f32, 0.20, 0.40, 0.80] {
        print!("{:<12}", format!("noise {noise_rms}"));
        for t in thresholds {
            let r = run(shipped_amplitude, t, noise_rms, sample_rate);
            print!(
                "{:>13}",
                format!("{:.2}/{:.2}", r.detection, r.false_positive)
            );
        }
        println!();
    }
}

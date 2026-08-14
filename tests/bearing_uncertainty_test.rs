//! Does the stated bearing uncertainty mean anything?
//!
//! `ConfidenceMetrics::bearing_uncertainty_deg` is the one number in the
//! confidence set that makes a checkable claim, so it is worth checking. Two
//! properties matter and they pull against each other: it has to grow as the
//! signal degrades, and it must not read lower than the scatter it describes.
//!
//! The second is the one with teeth. Reducing the reference term by the
//! averaging the loop performs on top of the detections is correct as
//! filter theory and wrong here, because as the signal degrades the tick's
//! error stops being scatter and becomes a displacement the loop follows.
//! That change passes every other test in the suite and makes this figure
//! understate a worthless bearing sixteenfold.

use std::f32::consts::PI;

use rotaryclub::config::{BearingMethod, RdfConfig};
use rotaryclub::processing::RdfProcessor;

const PULSE_HALF_WIDTH: i64 = 12;

fn noise_at(index: usize) -> f32 {
    let mut x = (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xA5A5_1234_9ABC_DEF0;
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 29;
    (((x >> 32) as u32) as f32 / (u32::MAX as f32)) * 2.0 - 1.0
}

/// Interleaved stereo: Doppler tone left, band-limited north pulses right.
///
/// The pulses sit at the true rotation epochs and the tone's north is at
/// sample zero, so the only bearing error present is the one the pipeline
/// makes.
fn build_signal(
    num_samples: usize,
    sample_rate: f32,
    rotation_hz: f32,
    bearing_deg: f32,
    amplitude: f32,
    noise: f32,
) -> Vec<f32> {
    let period = sample_rate as f64 / rotation_hz as f64;
    let omega = 2.0 * PI * rotation_hz / sample_rate;
    let bearing = bearing_deg.to_radians();

    let mut north = vec![0.0f32; num_samples];
    let mut k = 0i64;
    loop {
        let epoch = k as f64 * period;
        if epoch >= num_samples as f64 - PULSE_HALF_WIDTH as f64 {
            break;
        }
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
            north[n as usize] += amplitude * value as f32;
        }
    }

    let mut out = Vec::with_capacity(num_samples * 2);
    for (i, &tick) in north.iter().enumerate() {
        out.push((omega * i as f32 - bearing).sin() + noise * noise_at(i));
        out.push(tick + noise * 0.35 * noise_at(i ^ 0x5555));
    }
    out
}

struct Run {
    mean_abs_error_deg: f64,
    /// Mean absolute deviation of the error about its own mean: the part of
    /// the error that scatters, with any constant offset removed.
    scatter_deg: f64,
    mean_stated_sigma_deg: f64,
}

fn run(method: BearingMethod, noise: f32) -> Run {
    let mut config = RdfConfig::default();
    config.doppler.method = method;
    config.bearing.smoothing_window = 1;

    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;
    let truth = 200.0f32;
    let num_samples = (sample_rate * 4.0) as usize;

    let signal = build_signal(
        num_samples,
        sample_rate,
        rotation_hz,
        truth,
        config.north_tick.expected_pulse_amplitude,
        noise,
    );

    let mut processor = RdfProcessor::new(&config, false, true).expect("processor");
    let results = processor.process_signal(&signal);

    let mut errors = Vec::new();
    let mut signed = Vec::new();
    let mut stated = Vec::new();
    for result in &results {
        let Some(bearing) = result.bearing else {
            continue;
        };
        let error = ((bearing.raw_bearing - truth + 540.0).rem_euclid(360.0) - 180.0) as f64;
        errors.push(error.abs());
        signed.push(error);
        if let Some(sigma) = bearing.metrics.bearing_uncertainty_deg {
            stated.push(sigma as f64);
        }
    }

    // The loop is still acquiring at the start of the run, and what it does
    // there is not what this is measuring.
    let tail = |v: &[f64]| {
        let start = v.len() / 2;
        let slice = &v[start.min(v.len().saturating_sub(1))..];
        slice.iter().sum::<f64>() / slice.len().max(1) as f64
    };

    let signed_tail = &signed[signed.len() / 2..];
    let offset = signed_tail.iter().sum::<f64>() / signed_tail.len().max(1) as f64;
    let scatter = signed_tail.iter().map(|e| (e - offset).abs()).sum::<f64>()
        / signed_tail.len().max(1) as f64;

    Run {
        mean_abs_error_deg: tail(&errors),
        scatter_deg: scatter,
        mean_stated_sigma_deg: tail(&stated),
    }
}

#[test]
fn test_stated_uncertainty_grows_with_degradation() {
    for method in [BearingMethod::Correlation, BearingMethod::ZeroCrossing] {
        let quiet = run(method, 0.0);
        let noisy = run(method, 1.0);
        let ruined = run(method, 2.0);

        assert!(
            noisy.mean_stated_sigma_deg > quiet.mean_stated_sigma_deg,
            "{method:?}: uncertainty should grow with noise, got {:.3} at rest \
             and {:.3} under noise",
            quiet.mean_stated_sigma_deg,
            noisy.mean_stated_sigma_deg
        );
        assert!(
            ruined.mean_stated_sigma_deg > noisy.mean_stated_sigma_deg,
            "{method:?}: uncertainty should keep growing as the bearing falls \
             apart, got {:.3} then {:.3}",
            noisy.mean_stated_sigma_deg,
            ruined.mean_stated_sigma_deg
        );
        // The bearing really is ruined at this point, so a figure that still
        // reads like a usable measurement is not describing it.
        assert!(
            ruined.mean_stated_sigma_deg > 5.0,
            "{method:?}: a bearing {:.1} degrees out should not claim {:.2} \
             degrees of uncertainty",
            ruined.mean_abs_error_deg,
            ruined.mean_stated_sigma_deg
        );
    }
}

/// The figure describes precision, and precision is all it can describe.
///
/// It is built from the spread of the phase estimates, so it covers the part
/// of the error that scatters and cannot cover a constant displacement. That
/// limit is not academic: the zero-crossing method's error is almost entirely
/// a bias which grows with noise -- two degrees of offset against two degrees
/// of mean error at noise 0.3, six and a half against six -- so a figure that
/// covered its total error would have to be measuring something this one
/// structurally cannot see. Asserting against the scatter is asserting what
/// the number actually claims.
#[test]
fn test_stated_uncertainty_does_not_understate_the_scatter() {
    for method in [BearingMethod::Correlation, BearingMethod::ZeroCrossing] {
        for noise in [0.0f32, 0.3, 0.6, 1.0, 1.5, 2.0] {
            let measured = run(method, noise);
            assert!(
                measured.mean_stated_sigma_deg >= measured.scatter_deg,
                "{method:?} at noise {noise}: claimed {:.3} degrees of \
                 uncertainty against {:.3} degrees of scatter (total error \
                 {:.3}). A figure that reads better than the truth is worse \
                 than none.",
                measured.mean_stated_sigma_deg,
                measured.scatter_deg,
                measured.mean_abs_error_deg
            );
        }
    }
}

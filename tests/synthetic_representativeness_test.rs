//! The synthetic Doppler channel must resemble the recordings it stands in for.
//!
//! It did not. The generated channel was the rotation tone and nothing else:
//! `census_signal` measured its in-band fraction at 1.000 against
//! 0.002 to 0.075 on the captures in `data/`, and its harmonic content at zero
//! against 0.07 to 0.15. That is a factor of thirteen to five hundred on the
//! first, and it is not a cosmetic gap. The bearing uncertainty is the Doppler
//! phase spread over the independent count plus a reference term, and with no
//! interference there is no phase spread, so the term that dominates on real
//! signal was absent from every synthetic measurement of it. It was calibrated
//! wrongly twice on that basis.
//!
//! This pins the impaired generator inside the range the recordings occupy, so
//! the gap cannot quietly reopen.

use rotaryclub::config::RdfConfig;
use rotaryclub::simulation::{SignalImpairment, generate_impaired_signal, generate_test_signal};
use std::f32::consts::PI;

/// Power at one frequency, by direct correlation.
fn power_at(signal: &[f32], hz: f32, sample_rate: f32) -> f64 {
    let omega = 2.0 * PI * hz / sample_rate;
    let (mut i, mut q) = (0.0f64, 0.0f64);
    for (n, &s) in signal.iter().enumerate() {
        let phase = omega * n as f32;
        i += (s * phase.cos()) as f64;
        q += (s * phase.sin()) as f64;
    }
    let n = signal.len() as f64;
    2.0 * ((i / n) * (i / n) + (q / n) * (q / n))
}

struct Doppler {
    in_band_fraction: f64,
    harmonic_ratio: f64,
}

fn measure(interleaved: &[f32], sample_rate: f32, rotation_hz: f32) -> Doppler {
    let doppler: Vec<f32> = interleaved.iter().step_by(2).copied().collect();
    let total = doppler.iter().map(|s| (s * s) as f64).sum::<f64>() / doppler.len() as f64;
    let fundamental = power_at(&doppler, rotation_hz, sample_rate);
    let second = power_at(&doppler, rotation_hz * 2.0, sample_rate);
    let third = power_at(&doppler, rotation_hz * 3.0, sample_rate);
    Doppler {
        in_band_fraction: fundamental / total.max(1e-20),
        harmonic_ratio: ((second + third) / fundamental.max(1e-20)).sqrt(),
    }
}

#[test]
fn test_impaired_generator_matches_the_recordings() {
    let config = RdfConfig::default();
    let rate = config.audio.sample_rate;
    let rotation_hz = config.doppler.expected_freq;

    let signal = generate_impaired_signal(
        4.0,
        rate,
        rotation_hz,
        |_| 200.0,
        SignalImpairment::representative(),
    );
    let m = measure(&signal, rate as f32, rotation_hz);

    // The whole-channel fraction is deliberately NOT the match criterion, and
    // this asserts only that the tone is neither pristine nor drowned. Real
    // audio carries a great deal of energy below the doppler passband where it
    // does no harm, so matching the whole-channel ratio with voice-band noise
    // puts about ten times too much power where it hurts: at the cleanest
    // recording's whole-channel ratio, flat noise produced 20.7 degrees of
    // bearing error where that recording achieves 1.6. What is matched instead
    // is the noise power inside the passband, which the generator sets by
    // construction and so needs no assertion here.
    assert!(
        (0.02..0.60).contains(&m.in_band_fraction),
        "in-band fraction {:.4}: the tone is either drowned or barely impaired",
        m.in_band_fraction
    );
    assert!(
        (0.05..0.20).contains(&m.harmonic_ratio),
        "harmonic ratio {:.4} is outside the range the recordings occupy \
         (0.074 to 0.154)",
        m.harmonic_ratio
    );
}

/// The clean generator must stay clean, because a great many tests depend on
/// it being a pure tone and would otherwise shift under them silently.
#[test]
fn test_clean_generator_stays_clean() {
    let config = RdfConfig::default();
    let rate = config.audio.sample_rate;
    let rotation_hz = config.doppler.expected_freq;

    let signal = generate_test_signal(2.0, rate, rotation_hz, 200.0);
    let m = measure(&signal, rate as f32, rotation_hz);

    assert!(
        m.in_band_fraction > 0.99,
        "the clean generator should be essentially all tone, got {:.4}",
        m.in_band_fraction
    );
    assert!(
        m.harmonic_ratio < 0.01,
        "the clean generator should have no harmonics, got {:.4}",
        m.harmonic_ratio
    );
}

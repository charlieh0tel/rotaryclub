//! What does the interference in the doppler passband actually look like?
//!
//! At matched passband power the synthetic signal is two to four times harsher
//! than the recordings, and the stated bearing uncertainty reads about 1.3 on
//! synthetic against about 0.65 on the captures. That gap has been carried as
//! "flat noise is worse for phase estimation than shaped audio", which is a
//! hypothesis rather than a measurement -- and the levels are matched, so
//! whatever is different is not how much interference there is.
//!
//! Two candidates, both measurable from the three recordings we have.
//!
//! Shape: real audio is not flat across the passband. Within 500 Hz that tilt
//! is unlikely to be dramatic, but it has never been looked at.
//!
//! Time structure: this is the stronger suspect. Flat noise is stationary,
//! and speech is not -- it has syllables and gaps, so its power arrives in
//! bursts. A bearing is an average over a buffer, and an estimator meets a
//! burst very differently from a steady hiss of the same mean power. If the
//! recordings' interference is bursty then a scenario matched on mean power is
//! quiet most of the time and occasionally much worse, which would produce
//! exactly this: lower typical error, and the difference growing with how much
//! of the buffer the burst occupies.
//!
//! Reported per capture and for the synthetic generator at matched power:
//!
//!   in-band power     interference in the passband against the tone
//!   tilt              dB across the passband, low edge to high
//!   burst ratio       p95 of short-window power over its median
//!   envelope corr     lag-1 correlation of the short-window power
//!   active fraction   windows carrying at least a tenth of the median
//!
//! White stationary noise gives a burst ratio near 2, an envelope correlation
//! near zero, and an active fraction of one. Speech gives a large burst ratio,
//! a strongly correlated envelope, and an active fraction well under one.

use rotaryclub::audio::WavFileSource;
use rotaryclub::config::RdfConfig;
use rotaryclub::simulation::{SignalImpairment, generate_impaired_signal};
use std::f32::consts::PI;

/// Short window for the envelope, in milliseconds. Comparable to the buffers
/// a bearing is computed over, which is the timescale that matters here.
const WINDOW_MS: f32 = 20.0;

struct Census {
    tone_fade_db: f32,
    tone_corr: f32,
    in_band: f32,
    tilt_db: f32,
    burst_ratio: f32,
    envelope_corr: f32,
    active_fraction: f32,
}

/// Power at one frequency by direct correlation.
fn power_at(signal: &[f32], hz: f32, sample_rate: f32) -> f32 {
    let omega = 2.0 * PI * hz / sample_rate;
    let (mut i, mut q) = (0.0f32, 0.0f32);
    for (n, &s) in signal.iter().enumerate() {
        let phase = omega * n as f32;
        i += s * phase.cos();
        q += s * phase.sin();
    }
    let n = signal.len() as f32;
    2.0 * ((i / n) * (i / n) + (q / n) * (q / n))
}

/// A one-pole-per-edge bandpass, applied forwards only. Crude, but the same
/// filter is applied to every signal compared here, so the comparison holds.
fn bandpass(signal: &[f32], low: f32, high: f32, sample_rate: f32) -> Vec<f32> {
    let rc_low = 1.0 / (2.0 * PI * low);
    let rc_high = 1.0 / (2.0 * PI * high);
    let dt = 1.0 / sample_rate;
    let a_high = rc_low / (rc_low + dt);
    let a_low = dt / (rc_high + dt);
    let mut highpassed = Vec::with_capacity(signal.len());
    let (mut prev_in, mut prev_out) = (0.0f32, 0.0f32);
    for &s in signal {
        let out = a_high * (prev_out + s - prev_in);
        highpassed.push(out);
        prev_in = s;
        prev_out = out;
    }
    let mut out = Vec::with_capacity(signal.len());
    let mut acc = 0.0f32;
    for s in highpassed {
        acc += a_low * (s - acc);
        out.push(acc);
    }
    out
}

/// Remove the rotation tone and its first two harmonics, so what is left is
/// the interference rather than the signal.
fn remove_tone(signal: &mut [f32], tone_hz: f32, sample_rate: f32) {
    for k in 1..=3 {
        let hz = tone_hz * k as f32;
        let omega = 2.0 * PI * hz / sample_rate;
        let (mut i, mut q) = (0.0f32, 0.0f32);
        for (n, &s) in signal.iter().enumerate() {
            let phase = omega * n as f32;
            i += s * phase.cos();
            q += s * phase.sin();
        }
        let n = signal.len() as f32;
        let (i, q) = (i / n * 2.0, q / n * 2.0);
        for (n, s) in signal.iter_mut().enumerate() {
            let phase = omega * n as f32;
            *s -= i * phase.cos() + q * phase.sin();
        }
    }
}

/// Envelope of the rotation tone itself, window by window.
///
/// The interference envelope says what the noise is doing; this says what the
/// wanted signal is doing. A tone that fades is a channel with multipath or a
/// transmitter moving through one, and neither is in the generator.
fn tone_envelope(doppler: &[f32], sample_rate: f32, tone_hz: f32, window: usize) -> (f32, f32) {
    let amps: Vec<f32> = doppler
        .chunks(window.max(1))
        .filter(|c| c.len() == window.max(1))
        .map(|c| power_at(c, tone_hz, sample_rate).max(1e-20).sqrt())
        .collect();
    if amps.len() < 8 {
        return (f32::NAN, f32::NAN);
    }
    let mean = amps.iter().sum::<f32>() / amps.len() as f32;
    let var = amps.iter().map(|a| (a - mean) * (a - mean)).sum::<f32>();
    let cov: f32 = amps.windows(2).map(|w| (w[0] - mean) * (w[1] - mean)).sum();
    let corr = if var > 0.0 { cov / var } else { 0.0 };
    let mut sorted = amps.clone();
    sorted.sort_by(f32::total_cmp);
    let p95 = sorted[(sorted.len() as f32 * 0.95) as usize];
    let p05 = sorted[(sorted.len() as f32 * 0.05) as usize].max(1e-20);
    (20.0 * (p95 / p05).log10(), corr)
}

fn census(doppler: &[f32], sample_rate: f32, tone_hz: f32, low: f32, high: f32) -> Census {
    let tone_power = power_at(doppler, tone_hz, sample_rate);
    let (tone_fade_db, tone_corr) = tone_envelope(
        doppler,
        sample_rate,
        tone_hz,
        (WINDOW_MS * 1e-3 * sample_rate) as usize,
    );

    let mut residual = doppler.to_vec();
    remove_tone(&mut residual, tone_hz, sample_rate);
    let in_band_signal = bandpass(&residual, low, high, sample_rate);
    let in_band_power =
        in_band_signal.iter().map(|s| (s * s) as f64).sum::<f64>() / in_band_signal.len() as f64;

    // Tilt across the passband, measured at the two edges of the band on the
    // tone-removed signal.
    let lo = power_at(&residual, low + 40.0, sample_rate).max(1e-20);
    let hi = power_at(&residual, high - 40.0, sample_rate).max(1e-20);
    let tilt_db = 10.0 * (hi / lo).log10();

    // Envelope of the in-band interference, in windows the size of a bearing.
    let window = (WINDOW_MS * 1e-3 * sample_rate) as usize;
    let mut powers: Vec<f32> = in_band_signal
        .chunks(window.max(1))
        .filter(|c| c.len() == window.max(1))
        .map(|c| c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32)
        .collect();
    if powers.len() < 4 {
        return Census {
            tone_fade_db,
            tone_corr,
            in_band: (in_band_power / tone_power.max(1e-20) as f64) as f32,
            tilt_db,
            burst_ratio: f32::NAN,
            envelope_corr: f32::NAN,
            active_fraction: f32::NAN,
        };
    }

    let mean = powers.iter().sum::<f32>() / powers.len() as f32;
    let corr = {
        let var = powers.iter().map(|p| (p - mean) * (p - mean)).sum::<f32>();
        let cov: f32 = powers
            .windows(2)
            .map(|w| (w[0] - mean) * (w[1] - mean))
            .sum();
        if var > 0.0 { cov / var } else { 0.0 }
    };

    let mut sorted = powers.clone();
    sorted.sort_by(f32::total_cmp);
    let median = sorted[sorted.len() / 2];
    let p95 = sorted[(sorted.len() as f32 * 0.95) as usize];
    let active = powers.iter().filter(|p| **p > median * 0.1).count() as f32 / powers.len() as f32;
    powers.clear();

    Census {
        tone_fade_db,
        tone_corr,
        in_band: (in_band_power / tone_power.max(1e-20) as f64) as f32,
        tilt_db,
        burst_ratio: if median > 0.0 { p95 / median } else { f32::NAN },
        envelope_corr: corr,
        active_fraction: active,
    }
}

fn row(name: &str, c: &Census) {
    println!(
        "{name:<34} {:>11.1} {:>10.3} {:>10.3} {:>12.2} {:>13.3} {:>8.1} {:>9.3}",
        c.tone_fade_db,
        c.tone_corr,
        c.in_band,
        c.burst_ratio,
        c.envelope_corr,
        c.tilt_db,
        c.active_fraction
    );
}

fn main() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let tone_hz = config.doppler.expected_freq;
    let (low, high) = (config.doppler.bandpass_low, config.doppler.bandpass_high);

    println!("doppler passband {low:.0} to {high:.0} Hz, envelope in {WINDOW_MS:.0} ms windows\n");
    println!(
        "{:<34} {:>11} {:>10} {:>10} {:>12} {:>13} {:>8} {:>9}",
        "signal",
        "tone fade dB",
        "tone corr",
        "in-band",
        "burst ratio",
        "envelope corr",
        "tilt dB",
        "active"
    );

    for ratio in [0.2f32, 0.8, 6.5] {
        let signal = generate_impaired_signal(
            6.0,
            config.audio.sample_rate,
            tone_hz,
            |_| 200.0,
            SignalImpairment::at_passband_ratio(ratio),
        );
        let doppler: Vec<f32> = signal.iter().step_by(2).copied().collect();
        let c = census(&doppler, sample_rate, tone_hz, low, high);
        row(&format!("synthetic at ratio {ratio}"), &c);
    }

    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir("data")
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "wav"))
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    for path in paths {
        let Ok((samples, rate)) = WavFileSource::read_all(&path) else {
            continue;
        };
        let cap = (rate as usize * 2 * 6).min(samples.len());
        let doppler: Vec<f32> = samples[..cap].iter().step_by(2).copied().collect();
        let c = census(&doppler, rate as f32, tone_hz, low, high);
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        row(&name[..name.len().min(33)], &c);
    }
}

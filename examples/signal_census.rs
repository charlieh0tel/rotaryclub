//! How does the synthetic signal differ from the recordings?
//!
//! Conclusions drawn on synthetic signal have repeatedly failed on the
//! captures in `data/` -- the uncertainty composition was calibrated wrongly
//! twice on it -- but "synthetic is unrepresentative" has been an assertion,
//! not a measurement. This measures both against the same statistics, chosen
//! because the pipeline is sensitive to them, so the claim can be checked
//! rather than repeated.
//!
//! What each column is for:
//!
//!   north pulse amplitude and its spread   what the detection threshold meets
//!   north pulse width                      what the sub-sample estimator sees
//!   north floor                            what the threshold has to clear
//!   north interval spread                  what the loop has to track
//!   doppler in-band fraction               how much of the channel is signal
//!   doppler harmonic ratio                 what the bandpass has to reject
//!   doppler noise flatness                 whether the noise is white or shaped
//!
//! One quantity is deliberately absent: the noise power inside the doppler
//! passband, relative to the tone, which is what actually decides a bearing.
//! Measuring it here was tried and abandoned. Separating a weak tone from the
//! noise around it needs the two integrated over the same band, and the filter
//! mismatch leaves a floor near 0.1 that swamps a clean signal, while a capture
//! whose tone is 0.3 percent of the channel defeats the separation entirely --
//! ft-70d read 1.5 against an FFT measurement of 6.6.
//!
//! Measured by FFT instead, integrating the tone over plus or minus 8 Hz and
//! the rest of 1350 to 1850 Hz around it, the recordings give 0.199, 0.793 and
//! 6.579. Those are the numbers the synthetic generators are scaled to, and
//! they set their own ratio by construction so they need no measurement.
//!
//! Flatness is the spectral flatness of the out-of-band remainder, the
//! geometric mean of the power spectrum over its arithmetic mean. White noise
//! approaches 1; anything shaped, and voice especially, sits far below it.

use rotaryclub::audio::WavFileSource;
use rotaryclub::config::RdfConfig;
use rotaryclub::simulation::{SignalImpairment, generate_impaired_signal, generate_test_signal};
use std::f32::consts::PI;

struct Census {
    pulse_amplitude: f32,
    pulse_amplitude_spread: f32,
    pulse_width_samples: f32,
    north_floor: f32,
    interval_spread: f32,
    in_band_fraction: f32,
    harmonic_ratio: f32,
    noise_flatness: f32,
}

fn mean(v: &[f32]) -> f32 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f32>() / v.len() as f32
    }
}

fn std_dev(v: &[f32]) -> f32 {
    if v.len() < 2 {
        return 0.0;
    }
    let m = mean(v);
    (v.iter().map(|x| (x - m) * (x - m)).sum::<f32>() / v.len() as f32).sqrt()
}

/// Power at a frequency, by direct correlation. Cheaper than a transform and
/// exact at the frequency asked for, which is what matters here.
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

fn census(interleaved: &[f32], sample_rate: f32, rotation_hz: f32) -> Census {
    let doppler: Vec<f32> = interleaved.iter().step_by(2).copied().collect();
    let north: Vec<f32> = interleaved.iter().skip(1).step_by(2).copied().collect();
    let period = sample_rate / rotation_hz;

    // --- North channel: the pulses, and what sits between them ---
    let peak = north.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    let gate = peak * 0.5;
    let mut amplitudes = Vec::new();
    let mut widths = Vec::new();
    let mut positions = Vec::new();
    let mut idx = 0usize;
    while idx < north.len() {
        if north[idx] > gate {
            let start = idx;
            let mut best = north[idx];
            let mut best_at = idx;
            while idx < north.len() && north[idx] > gate {
                if north[idx] > best {
                    best = north[idx];
                    best_at = idx;
                }
                idx += 1;
            }
            amplitudes.push(best);
            widths.push((idx - start) as f32);
            positions.push(best_at as f32);
        }
        idx += 1;
    }

    // Everything more than a pulse-width away from a detected pulse.
    let mut floor_energy = 0.0f64;
    let mut floor_count = 0usize;
    let guard = 24usize;
    let mut next = 0usize;
    for (n, &s) in north.iter().enumerate() {
        while next < positions.len() && (positions[next] as usize) + guard < n {
            next += 1;
        }
        let near = next < positions.len() && (positions[next] as usize).abs_diff(n) <= guard;
        if !near {
            floor_energy += (s * s) as f64;
            floor_count += 1;
        }
    }
    let north_floor = if floor_count > 0 {
        (floor_energy / floor_count as f64).sqrt() as f32
    } else {
        0.0
    };

    let intervals: Vec<f32> = positions.windows(2).map(|w| w[1] - w[0]).collect();
    let kept: Vec<f32> = intervals
        .iter()
        .copied()
        .filter(|i| (*i - period).abs() < period * 0.5)
        .collect();

    // --- Doppler channel: how much of it is the tone ---
    //
    // The rotation rate of a real capture is not exactly the nominal one, and
    // correlating at a fixed frequency over several seconds decorrelates badly
    // when it is off: a fortieth of a hertz is a quarter turn of phase drift
    // over six seconds. Measuring at the nominal rate understated the tone by
    // an order of magnitude on the recordings and inflated every ratio built
    // on it, so the peak is found first.
    let mut tone_hz = rotation_hz;
    let mut best = 0.0f32;
    let mut probe = rotation_hz - 3.0;
    while probe <= rotation_hz + 3.0 {
        let p = power_at(&doppler, probe, sample_rate);
        if p > best {
            best = p;
            tone_hz = probe;
        }
        probe += 0.02;
    }

    let total_power = doppler.iter().map(|s| (s * s) as f64).sum::<f64>() / doppler.len() as f64;
    let fundamental = best;
    let second = power_at(&doppler, tone_hz * 2.0, sample_rate);
    let third = power_at(&doppler, tone_hz * 3.0, sample_rate);

    // Spectral flatness of what is left once the tone and its first two
    // harmonics are accounted for, sampled across the band.
    let mut bins = Vec::new();
    let mut f = 200.0f32;
    while f < sample_rate / 2.0 - 200.0 {
        let near_tone = [1.0, 2.0, 3.0]
            .iter()
            .any(|k| (f - tone_hz * k).abs() < 60.0);
        if !near_tone {
            bins.push(power_at(&doppler, f, sample_rate).max(1e-20));
        }
        f += 137.0;
    }
    // Noise power inside the doppler passband, against the tone. This is the
    // quantity that decides a bearing, and the one worth matching: the
    // whole-channel fraction differs between synthetic and real signal for a
    // reason that does not matter, which is that real audio carries a great
    // deal of energy below the passband where it does no harm.
    //
    // Measured by filtering and taking the power, not by summing correlations
    // at sampled frequencies. The latter was tried and undercounts by the
    // ratio of the analysis resolution to the sampling step -- about ninety
    // here -- because it samples the spectrum rather than integrating it.
    // Spectral flatness of what is left once the tone and its first two
    // harmonics are accounted for, sampled across the band.
    let mut bins = Vec::new();
    let mut f = 200.0f32;
    while f < sample_rate / 2.0 - 200.0 {
        let near_tone = [1.0, 2.0, 3.0]
            .iter()
            .any(|k| (f - tone_hz * k).abs() < 60.0);
        if !near_tone {
            bins.push(power_at(&doppler, f, sample_rate).max(1e-20));
        }
        f += 137.0;
    }
    // Noise power inside the doppler passband, against the tone. This is the
    // quantity that decides a bearing, and the one worth matching: the
    // whole-channel fraction differs between synthetic and real signal for a
    // reason that does not matter, which is that real audio carries a great
    // deal of energy below the passband where it does no harm.
    //
    // Measured by filtering and taking the power, not by summing correlations
    // at sampled frequencies. The latter was tried and undercounts by the
    // ratio of the analysis resolution to the sampling step -- about ninety
    // here -- because it samples the spectrum rather than integrating it.
    // Both terms are integrated, not sampled. A real doppler tone is not a
    // pure sinusoid -- it wanders over a few hertz -- so a correlation at one
    // frequency captures part of its energy while a filtered band captures
    // all of it, and mixing the two overstated the noise several-fold.

    let log_mean = bins.iter().map(|p| p.ln()).sum::<f32>() / bins.len().max(1) as f32;
    let arithmetic = bins.iter().sum::<f32>() / bins.len().max(1) as f32;
    let flatness = if arithmetic > 0.0 {
        log_mean.exp() / arithmetic
    } else {
        0.0
    };

    Census {
        pulse_amplitude: mean(&amplitudes),
        pulse_amplitude_spread: if amplitudes.is_empty() {
            0.0
        } else {
            std_dev(&amplitudes) / mean(&amplitudes).max(f32::EPSILON)
        },
        pulse_width_samples: mean(&widths),
        north_floor,
        interval_spread: std_dev(&kept),
        in_band_fraction: (fundamental as f64 / total_power.max(1e-20)) as f32,
        harmonic_ratio: ((second + third) / fundamental.max(1e-20)).sqrt(),
        noise_flatness: flatness,
    }
}

fn row(name: &str, c: &Census) {
    println!(
        "{name:<30} {:>7.3} {:>7.3} {:>6.1} {:>8.4} {:>7.3} {:>8.3} {:>7.3} {:>7.3}",
        c.pulse_amplitude,
        c.pulse_amplitude_spread,
        c.pulse_width_samples,
        c.north_floor,
        c.interval_spread,
        c.in_band_fraction,
        c.harmonic_ratio,
        c.noise_flatness
    );
}

fn main() {
    let config = RdfConfig::default();
    let rotation_hz = config.doppler.expected_freq;

    println!(
        "{:<30} {:>7} {:>7} {:>6} {:>8} {:>7} {:>8} {:>7} {:>7}",
        "signal", "pulse", "spread", "width", "floor", "jitter", "in-band", "harm", "flat"
    );

    // The generator the tests and sweeps use, clean.
    let synthetic = generate_test_signal(6.0, config.audio.sample_rate, rotation_hz, 200.0);
    let c = census(&synthetic, config.audio.sample_rate as f32, rotation_hz);
    row("synthetic, clean", &c);

    let impaired = generate_impaired_signal(
        6.0,
        config.audio.sample_rate,
        rotation_hz,
        |_| 200.0,
        SignalImpairment::representative(),
    );
    let c = census(&impaired, config.audio.sample_rate as f32, rotation_hz);
    row("synthetic, representative", &c);

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
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        // Cap the read so a long capture does not dominate the run.
        let cap = (rate as usize * 2 * 6).min(samples.len());
        let c = census(&samples[..cap], rate as f32, rotation_hz);
        row(&name[..name.len().min(29)], &c);
    }
}

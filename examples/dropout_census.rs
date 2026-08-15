//! When the rotation tone weakens in a recording, what else weakens with it?
//!
//! The tone's amplitude in 20 ms windows spans 17 to 133 dB across the three
//! captures, and that was read as multipath: two paths summing, cancelling
//! sometimes, moving the bearing as they do. The generator was given a second
//! path on that basis and it reproduced both the fade statistics and the
//! uncertainty calibration.
//!
//! It reproduced the numbers. That is not the same as being the mechanism, and
//! this project has a history of the difference. So this asks what actually
//! goes quiet, which multipath and the alternatives answer differently.
//!
//! A reflection removes the tone and leaves everything else: the carrier is
//! still there, the receiver still delivers audio, the switcher still runs, so
//! the channel stays as loud as it was and only the coherent part cancels.
//!
//! A transmitter unkeying, or a squelch closing, or the signal dropping out,
//! removes the carrier: the whole doppler channel goes quiet together, tone
//! and audio alike. The switcher is local hardware and does not care, so the
//! north pulses keep arriving.
//!
//! A gap in the recording removes both channels.
//!
//! Reported per capture, for the weakest fifth of windows against the
//! strongest fifth:
//!
//!   tone ratio        how much weaker the tone is, in dB
//!   channel ratio     how much weaker the whole doppler channel is, in dB
//!   north ratio       how much weaker the north channel is, in dB
//!
//! Multipath: tone falls, channel roughly holds, north holds. Loss of signal:
//! tone and channel fall together, north holds. Recording gap: all three fall.

use rotaryclub::audio::WavFileSource;
use rotaryclub::config::RdfConfig;
use std::f32::consts::PI;

const WINDOW_MS: f32 = 20.0;

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

/// Power inside the doppler passband, by direct correlation across the band.
/// Crude but consistent between the sets being compared.
fn in_band_power(signal: &[f32], sample_rate: f32, low: f32, high: f32) -> f32 {
    let mut total = 0.0f32;
    let mut hz = low;
    while hz <= high {
        total += power_at(signal, hz, sample_rate);
        hz += 25.0;
    }
    total
}

fn mean_power(signal: &[f32]) -> f32 {
    if signal.is_empty() {
        return 0.0;
    }
    signal.iter().map(|s| (s * s) as f64).sum::<f64>() as f32 / signal.len() as f32
}

fn db(strong: f32, weak: f32) -> f32 {
    10.0 * (strong.max(1e-20) / weak.max(1e-20)).log10()
}

fn main() {
    let config = RdfConfig::default();
    let tone_hz = config.doppler.expected_freq;

    println!("weakest fifth of windows against the strongest fifth, {WINDOW_MS:.0} ms windows\n");
    println!(
        "{:<34} {:>10} {:>13} {:>11} {:>11}",
        "capture", "tone dB", "channel dB", "north dB", "in-band dB"
    );

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
        let sample_rate = rate as f32;
        let doppler: Vec<f32> = samples.iter().step_by(2).copied().collect();
        let north: Vec<f32> = samples.iter().skip(1).step_by(2).copied().collect();
        let window = (WINDOW_MS * 1e-3 * sample_rate) as usize;

        // Index the windows by tone strength, then look at what the same
        // windows did on every other measure.
        let mut indexed: Vec<(f32, usize)> = doppler
            .chunks(window)
            .enumerate()
            .filter(|(_, c)| c.len() == window)
            .map(|(i, c)| (power_at(c, tone_hz, sample_rate), i))
            .collect();
        if indexed.len() < 10 {
            continue;
        }
        indexed.sort_by(|a, b| a.0.total_cmp(&b.0));

        let fifth = indexed.len() / 5;
        let weak = &indexed[..fifth];
        let strong = &indexed[indexed.len() - fifth..];

        let gather = |set: &[(f32, usize)], channel: &[f32]| -> (f32, f32, f32) {
            let mut tone = 0.0f32;
            let mut total = 0.0f32;
            let mut band = 0.0f32;
            for &(t, i) in set {
                let chunk = &channel[i * window..(i + 1) * window];
                tone += t;
                total += mean_power(chunk);
                band += in_band_power(chunk, sample_rate, 1350.0, 1850.0);
            }
            let n = set.len() as f32;
            (tone / n, total / n, band / n)
        };

        let (weak_tone, weak_channel, weak_band) = gather(weak, &doppler);
        let (strong_tone, strong_channel, strong_band) = gather(strong, &doppler);
        let (_, weak_north, _) = gather(weak, &north);
        let (_, strong_north, _) = gather(strong, &north);

        let name = path.file_name().unwrap_or_default().to_string_lossy();
        println!(
            "{:<34} {:>10.1} {:>13.1} {:>11.1} {:>11.1}",
            &name[..name.len().min(33)],
            db(strong_tone, weak_tone),
            db(strong_channel, weak_channel),
            db(strong_north, weak_north),
            db(strong_band, weak_band)
        );
    }
}

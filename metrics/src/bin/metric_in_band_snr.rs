//! In-band SNR of the recordings: the rotation tone against the interference
//! sharing its passband.
//!
//! This is the number every scenario in the repository is scaled to, and until
//! now it had no instrument. `census_signal` says so plainly -- it tried to
//! measure this from filtered power, abandoned it because the filter mismatch
//! left a floor near 0.1, and quotes an FFT measurement that appears nowhere
//! in the tree. METRICS.md's +7, +1 and -8 dB come from that vanished
//! measurement. So this reproduces it to the stated method and says whether it
//! agrees.
//!
//! Method, as `census_signal` describes it: integrate the tone over plus or
//! minus 8 Hz, and the interference over the rest of the Doppler passband,
//! 1350 to 1850 Hz. Both by direct correlation over Hann-windowed segments,
//! averaged across the recording; the same technique `census_signal` uses for
//! a single frequency, swept across the band.
//!
//! The synthetic generators are the check that matters. `at_passband_ratio(x)`
//! builds a signal whose in-band ratio is x by construction, so measuring it
//! back is a calibration with a known answer -- something the recordings can
//! never provide.
//!
//! What the measurement found is that the question needs a rule attached.
//! Every recording is bimodal by two to three orders of magnitude -- ft-70d
//! runs 0.013 at its tenth percentile and 14.3 at its ninetieth -- because a
//! recording is transmissions separated by squelch noise, and a segment of
//! squelch noise has no tone in it at all. So "the in-band SNR of this
//! recording" is not one number, and which number you get is decided entirely
//! by which segments you count. That rule was never recorded alongside the
//! +7, +1 and -8 dB, which is the reason they cannot be reproduced.
//!
//! So this reports the distribution rather than a single figure, plus one
//! explicitly defined summary: the median over the segments in the upper half
//! by tone power, which is the recording while somebody is talking. Any
//! scenario scaled to a recording should name which of these it means.

use rotaryclub::audio::WavFileSource;
use rotaryclub::config::RdfConfig;
use rotaryclub::simulation::{SignalImpairment, generate_impaired_signal};
use std::f32::consts::PI;

/// Segment length for the transform, in samples. At 48 kHz this is 2.93 Hz of
/// resolution, so the plus or minus 8 Hz tone band spans five bins and the
/// Hann window's own spreading, about two bins, sits comfortably inside it.
const SEGMENT: usize = 16384;

/// Half-width of the band counted as tone, in Hz.
const TONE_HALF_WIDTH_HZ: f32 = 8.0;

/// Power at one frequency in one segment, by direct correlation.
///
/// The same estimator `census_signal` uses, applied bin by bin instead of at a
/// single frequency.
fn power_at(segment: &[f32], hz: f32, sample_rate: f32) -> f64 {
    let omega = 2.0 * PI * hz / sample_rate;
    let (mut i, mut q) = (0.0f64, 0.0f64);
    for (n, &s) in segment.iter().enumerate() {
        let phase = omega * n as f32;
        i += (s * phase.cos()) as f64;
        q += (s * phase.sin()) as f64;
    }
    let n = segment.len() as f64;
    2.0 * ((i / n).powi(2) + (q / n).powi(2))
}

struct InBand {
    /// Per-segment interference-over-tone ratios, ascending.
    ratios: Vec<f64>,
    /// Median ratio over the segments in the upper half by tone power: the
    /// recording while somebody is transmitting.
    transmitting: f64,
    /// Whole-file ratio, summing both powers before dividing once. Dominated
    /// by the quiet, and reported so the gap between the two is visible.
    whole_file: f64,
    /// Where the tone was actually found, in Hz. A recording whose rotation
    /// sits away from nominal would otherwise have its tone counted as
    /// interference.
    tone_hz: f32,
    segments: usize,
}

impl InBand {
    fn pct(&self, q: f64) -> f64 {
        if self.ratios.is_empty() {
            return f64::NAN;
        }
        self.ratios[(((self.ratios.len() - 1) as f64) * q).round() as usize]
    }
}

fn to_db(noise_to_tone: f64) -> f64 {
    -10.0 * noise_to_tone.max(f64::MIN_POSITIVE).log10()
}

fn measure(doppler: &[f32], sample_rate: f32, band_low: f32, band_high: f32) -> Option<InBand> {
    if doppler.len() < SEGMENT {
        return None;
    }
    let resolution = sample_rate / SEGMENT as f32;
    let bins: Vec<f32> = {
        let mut v = Vec::new();
        let mut hz = band_low;
        while hz <= band_high {
            v.push(hz);
            hz += resolution;
        }
        v
    };

    // Hann, to keep the tone from smearing across the whole band and being
    // counted as the interference it is being compared against.
    let window: Vec<f32> = (0..SEGMENT)
        .map(|n| 0.5 - 0.5 * (2.0 * PI * n as f32 / SEGMENT as f32).cos())
        .collect();

    // Whole-file spectrum first, only to locate the tone. Per segment the
    // peak of a squelch-noise segment is wherever the noise happened to peak,
    // which would put the tone band in a different place every time.
    let mut spectrum = vec![0.0f64; bins.len()];
    let mut spectra: Vec<Vec<f64>> = Vec::new();
    for chunk in doppler.chunks_exact(SEGMENT) {
        let windowed: Vec<f32> = chunk.iter().zip(&window).map(|(s, w)| s * w).collect();
        let seg: Vec<f64> = bins
            .iter()
            .map(|&hz| power_at(&windowed, hz, sample_rate))
            .collect();
        for (slot, value) in spectrum.iter_mut().zip(&seg) {
            *slot += value;
        }
        spectra.push(seg);
    }
    if spectra.is_empty() {
        return None;
    }

    let peak = spectrum
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| i)?;
    let tone_hz = bins[peak];

    let split = |spec: &[f64]| {
        let (mut tone, mut noise) = (0.0f64, 0.0f64);
        for (power, &hz) in spec.iter().zip(&bins) {
            if (hz - tone_hz).abs() <= TONE_HALF_WIDTH_HZ {
                tone += power;
            } else {
                noise += power;
            }
        }
        (tone, noise)
    };

    let mut per_segment: Vec<(f64, f64)> = Vec::new(); // (tone power, ratio)
    for spec in &spectra {
        let (tone, noise) = split(spec);
        if tone > 0.0 {
            per_segment.push((tone, noise / tone));
        }
    }
    if per_segment.is_empty() {
        return None;
    }

    let mut ratios: Vec<f64> = per_segment.iter().map(|(_, r)| *r).collect();
    ratios.sort_by(f64::total_cmp);

    // The upper half by tone power: the recording while somebody is talking.
    let mut by_tone = per_segment.clone();
    by_tone.sort_by(|a, b| b.0.total_cmp(&a.0));
    let loud = &by_tone[..by_tone.len().div_ceil(2)];
    let mut loud_ratios: Vec<f64> = loud.iter().map(|(_, r)| *r).collect();
    loud_ratios.sort_by(f64::total_cmp);
    let transmitting = loud_ratios[loud_ratios.len() / 2];

    let (tone, noise) = split(&spectrum);
    if tone <= 0.0 {
        return None;
    }
    let segments = spectra.len();
    Some(InBand {
        ratios,
        transmitting,
        whole_file: noise / tone,
        tone_hz,
        segments,
    })
}

fn main() {
    let jsonl = std::env::args().any(|a| a == "--jsonl");
    let config = RdfConfig::default();
    let (low, high) = (config.doppler.bandpass_low, config.doppler.bandpass_high);

    if !jsonl {
        println!(
            "In-band SNR: rotation tone against interference sharing {low:.0}-{high:.0} Hz.\n\
             Tone integrated over plus or minus {TONE_HALF_WIDTH_HZ:.0} Hz of its peak.\n"
        );
        println!(
            "{:<40} {:>9} {:>9} {:>9} {:>9} {:>9} {:>6}",
            "source", "p10", "p50", "p90", "talking", "whole", "segs"
        );
        println!(
            "{:<40} {:>9} {:>9} {:>9} {:>9} {:>9} {:>6}",
            "", "ratio", "ratio", "ratio", "dB", "dB", ""
        );
    }

    // Calibration first: signals whose ratio is known by construction. If the
    // instrument cannot recover these, nothing it says about the recordings
    // means anything.
    for stated in [0.199f32, 0.793, 6.579] {
        let signal = generate_impaired_signal(
            20.0,
            config.audio.sample_rate,
            config.doppler.expected_freq,
            |_| 200.0,
            SignalImpairment::at_passband_ratio(stated),
        );
        let doppler: Vec<f32> = signal.iter().step_by(2).copied().collect();
        let Some(m) = measure(&doppler, config.audio.sample_rate as f32, low, high) else {
            continue;
        };
        if jsonl {
            println!(
                "{}",
                serde_json::json!({
                    "source": format!("synthetic at_passband_ratio({stated})"),
                    "kind": "calibration",
                    "stated_noise_to_tone": stated,
                    "transmitting": m.transmitting,
                    "transmitting_snr_db": to_db(m.transmitting),
                    "whole_file": m.whole_file,
                    "p10": m.pct(0.10),
                    "p50": m.pct(0.50),
                    "p90": m.pct(0.90),
                    "tone_hz": m.tone_hz,
                    "segments": m.segments,
                })
            );
        } else {
            println!(
                "{:<40} {:>9.3} {:>9.3} {:>9.3} {:>9.1} {:>9.1} {:>6}  stated {stated}",
                format!("synthetic, {stated} by construction"),
                m.pct(0.10),
                m.pct(0.50),
                m.pct(0.90),
                to_db(m.transmitting),
                to_db(m.whole_file),
                m.segments
            );
        }
    }

    let mut paths: Vec<_> = std::fs::read_dir("data")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "wav"))
        .collect();
    paths.sort();

    for path in paths {
        let Ok((samples, rate)) = WavFileSource::read_all(&path) else {
            continue;
        };
        let doppler: Vec<f32> = samples.iter().step_by(2).copied().collect();
        let Some(m) = measure(&doppler, rate as f32, low, high) else {
            continue;
        };
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if jsonl {
            println!(
                "{}",
                serde_json::json!({
                    "source": name,
                    "kind": "recording",
                    "transmitting": m.transmitting,
                    "transmitting_snr_db": to_db(m.transmitting),
                    "whole_file": m.whole_file,
                    "whole_file_snr_db": to_db(m.whole_file),
                    "p10": m.pct(0.10),
                    "p50": m.pct(0.50),
                    "p90": m.pct(0.90),
                    "tone_hz": m.tone_hz,
                    "segments": m.segments,
                })
            );
        } else {
            let short = name.split('_').next().unwrap_or(&name);
            let short = if short.len() > 38 {
                &short[..38]
            } else {
                short
            };
            println!(
                "{:<40} {:>9.3} {:>9.3} {:>9.3} {:>9.1} {:>9.1} {:>6}",
                short,
                m.pct(0.10),
                m.pct(0.50),
                m.pct(0.90),
                to_db(m.transmitting),
                to_db(m.whole_file),
                m.segments
            );
        }
    }
}

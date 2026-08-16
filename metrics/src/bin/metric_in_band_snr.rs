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
//! So this reports the distribution and the whole-file ratio, and nothing
//! else. A "median over the loudest half" summary was tried and retired: it
//! failed its own known-answer calibration by 3.4 dB (selecting on a tone
//! estimate that shares its noise with the ratio), and measured for
//! robustness it moved by 7 dB when the measurement window was merely
//! halved, with the same segments selected -- a 341 ms segment straddles
//! over-boundaries, so any single "while transmitting" number depends on
//! segment length, resolution and rule, each worth several dB. Quote a
//! percentile band with the rule stated instead.
//!
//! Noise inside the tone band is subtracted: the +-8 Hz collection band
//! carries 16.4 Hz of the 500 Hz noise floor, which read as tone and biased
//! the worst capture about 0.8 dB clean. The floor is estimated from the
//! rest of the band, per segment.

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
    /// Whole-file ratio, summing both powers before dividing once. Dominated
    /// by the quiet between transmissions; the per-segment percentiles carry
    /// the rest of the story.
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
        let mut noise_bins = 0usize;
        let mut tone_bins = 0usize;
        for (power, &hz) in spec.iter().zip(&bins) {
            if (hz - tone_hz).abs() <= TONE_HALF_WIDTH_HZ {
                tone += power;
                tone_bins += 1;
            } else {
                noise += power;
                noise_bins += 1;
            }
        }
        // The tone band also holds its share of the noise floor -- 16.4 Hz
        // of the 500 -- which read as tone and biased the worst capture
        // about 0.8 dB clean. Move the floor's share back where it belongs,
        // clamping at zero for a segment whose tone band holds less than
        // the floor predicts.
        if noise_bins > 0 && tone_bins > 0 {
            let floor_share = noise / noise_bins as f64 * tone_bins as f64;
            let corrected = (tone - floor_share).max(0.0);
            noise += tone - corrected;
            tone = corrected;
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

    let (tone, noise) = split(&spectrum);
    if tone <= 0.0 {
        return None;
    }
    let segments = spectra.len();
    Some(InBand {
        ratios,
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
            "{:<40} {:>9} {:>9} {:>9} {:>9} {:>6}",
            "source", "p10", "p50", "p90", "whole", "segs"
        );
        println!(
            "{:<40} {:>9} {:>9} {:>9} {:>9} {:>6}",
            "", "ratio", "ratio", "ratio", "dB", ""
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
            println!(
                "{:<40} {:>9.3} {:>9.3} {:>9.3} {:>9.1} {:>6}  stated {stated}",
                format!("synthetic, {stated} by construction"),
                m.pct(0.10),
                m.pct(0.50),
                m.pct(0.90),
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
                "{:<40} {:>9.3} {:>9.3} {:>9.3} {:>9.1} {:>6}",
                short,
                m.pct(0.10),
                m.pct(0.50),
                m.pct(0.90),
                to_db(m.whole_file),
                m.segments
            );
        }
    }
}

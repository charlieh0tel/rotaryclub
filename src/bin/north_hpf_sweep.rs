//! Sweep north-channel highpass settings against reference-pulse timing accuracy.
//!
//! The highpass exists to isolate the pulse transient from FM audio bleed, but
//! it also discards pulse energy that carries timing information. This tool
//! measures the tradeoff on real captures: for each cutoff/tap combination it
//! detects north pulses, estimates each pulse epoch, fits a constant rotation
//! rate, and reports the residual. A configuration that both detects every
//! pulse and leaves a small residual is a good one.
//!
//! Residuals are scored against a straight line, so a constant delay (filter
//! group delay included) is absorbed by the fit and does not affect the
//! comparison.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;

use rotaryclub::audio::WavFileSource;
use rotaryclub::config::RotationFrequency;
use rotaryclub::signal_processing::{DcRemover, FirHighpass, PeakDetector};

#[derive(Parser, Debug)]
#[command(name = "north_hpf_sweep")]
#[command(about = "Sweep north highpass cutoff/taps against pulse timing accuracy", long_about = None)]
struct Args {
    /// WAV files to analyze
    #[arg(required = true)]
    files: Vec<PathBuf>,

    /// North reference channel (0 = left, 1 = right)
    #[arg(long, default_value_t = 1)]
    channel: usize,

    /// Nominal rotation frequency (e.g. "1602.564", "624us")
    #[arg(long, default_value = "1602.564")]
    rotation: RotationFrequency,

    /// Highpass cutoffs in Hz to try; 0 means DC removal only
    #[arg(long, value_delimiter = ',', default_values_t = [0.0, 500.0, 1000.0, 2000.0, 3000.0, 5000.0, 8000.0])]
    cutoffs: Vec<f32>,

    /// Highpass tap counts to try
    #[arg(long, value_delimiter = ',', default_values_t = [31usize, 63, 127])]
    taps: Vec<usize>,

    /// Highpass transition bandwidth in Hz
    #[arg(long, default_value_t = 500.0)]
    transition: f32,

    /// Detection threshold as a fraction of the filtered peak amplitude
    #[arg(long, default_value_t = 0.35)]
    threshold: f32,

    /// Minimum interval between detections in milliseconds
    #[arg(long, default_value_t = 0.6)]
    min_interval_ms: f32,

    /// Emit CSV instead of a table
    #[arg(long)]
    csv: bool,
}

/// Timing residual statistics for one estimator, in samples.
struct Residuals {
    rms: f64,
    p95_abs: f64,
    period: f64,
}

impl Residuals {
    fn rms_degrees(&self) -> f64 {
        self.rms / self.period * 360.0
    }

    fn p95_degrees(&self) -> f64 {
        self.p95_abs / self.period * 360.0
    }
}

fn deinterleave(samples: &[f32], channels: usize, channel: usize) -> Vec<f32> {
    samples
        .iter()
        .skip(channel)
        .step_by(channels)
        .copied()
        .collect()
}

fn preprocess(
    input: &[f32],
    sample_rate: f32,
    cutoff: f32,
    taps: usize,
    transition: f32,
) -> Result<Vec<f32>> {
    let mut buffer = input.to_vec();
    if cutoff <= 0.0 {
        let mut dc = DcRemover::with_cutoff(sample_rate, 20.0);
        dc.process(&mut buffer);
    } else {
        let mut highpass = FirHighpass::new(cutoff, sample_rate, taps, transition)
            .with_context(|| format!("designing highpass at {cutoff} Hz, {taps} taps"))?;
        highpass.process_buffer(&mut buffer);
    }
    Ok(buffer)
}

/// Local-max index after each threshold crossing.
fn detect_peaks(signal: &[f32], threshold: f32, min_interval: usize, search: usize) -> Vec<usize> {
    let mut detector = PeakDetector::with_peak_search_window(threshold, min_interval, search);
    detector
        .find_all_peaks(signal)
        .into_iter()
        .filter_map(|(index, _)| usize::try_from(index).ok())
        .collect()
}

/// Energy centroid of the positive part of the signal around `peak`.
fn centroid_offset(signal: &[f32], peak: usize, half_width: usize) -> f64 {
    let low = peak.saturating_sub(half_width);
    let high = (peak + half_width).min(signal.len() - 1);
    let mut weighted = 0.0f64;
    let mut total = 0.0f64;
    for (offset, sample) in signal[low..=high].iter().enumerate() {
        let value = sample.max(0.0) as f64;
        let weight = value * value;
        weighted += weight * (low + offset) as f64;
        total += weight;
    }
    if total > 0.0 {
        weighted / total - peak as f64
    } else {
        0.0
    }
}

/// Fit `epoch = period * cycle + offset` with the cycle indices re-derived
/// from the running fit, so missed pulses do not shift the numbering.
fn fit_rate(epochs: &[f64], nominal_period: f64) -> Option<(f64, Vec<f64>)> {
    if epochs.len() < 16 {
        return None;
    }
    let mut period = nominal_period;
    let mut offset = epochs[0];
    let mut cycles: Vec<f64> = Vec::with_capacity(epochs.len());
    for _ in 0..6 {
        cycles.clear();
        cycles.extend(epochs.iter().map(|e| ((e - offset) / period).round()));
        let n = cycles.len() as f64;
        let mean_cycle = cycles.iter().sum::<f64>() / n;
        let mean_epoch = epochs.iter().sum::<f64>() / n;
        let mut covariance = 0.0;
        let mut variance = 0.0;
        for (cycle, epoch) in cycles.iter().zip(epochs) {
            covariance += (cycle - mean_cycle) * (epoch - mean_epoch);
            variance += (cycle - mean_cycle) * (cycle - mean_cycle);
        }
        if variance <= 0.0 {
            return None;
        }
        period = covariance / variance;
        offset = mean_epoch - period * mean_cycle;
        if !period.is_finite() || period <= 0.0 {
            return None;
        }
    }
    let residuals = epochs
        .iter()
        .zip(&cycles)
        .map(|(epoch, cycle)| epoch - (period * cycle + offset))
        .collect();
    Some((period, residuals))
}

fn summarize(epochs: &[f64], nominal_period: f64) -> Option<Residuals> {
    let (period, residuals) = fit_rate(epochs, nominal_period)?;
    // A handful of mis-detections should not dominate the comparison.
    let mut sorted: Vec<f64> = residuals.iter().map(|r| r.abs()).collect();
    sorted.sort_by(f64::total_cmp);
    let cut = ((sorted.len() as f64 * 0.99) as usize).min(sorted.len() - 1);
    let keep = sorted[cut];
    let kept: Vec<f64> = residuals.into_iter().filter(|r| r.abs() <= keep).collect();
    if kept.len() < 16 {
        return None;
    }
    let n = kept.len() as f64;
    let mean = kept.iter().sum::<f64>() / n;
    let rms = (kept.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n).sqrt();
    let mut absolute: Vec<f64> = kept.iter().map(|r| (r - mean).abs()).collect();
    absolute.sort_by(f64::total_cmp);
    let p95_abs = absolute[((absolute.len() - 1) as f64 * 0.95) as usize];
    Some(Residuals {
        rms,
        p95_abs,
        period,
    })
}

struct Row {
    cutoff: f32,
    taps: usize,
    detected: usize,
    expected: usize,
    rate_hz: f64,
    hard_limiter: Option<Residuals>,
    centroid: Option<Residuals>,
}

fn analyze_file(path: &PathBuf, args: &Args) -> Result<Vec<Row>> {
    let (samples, sample_rate) =
        WavFileSource::read_all(path).with_context(|| format!("reading {}", path.display()))?;
    if args.channel > 1 {
        bail!("channel must be 0 or 1");
    }
    let sample_rate = sample_rate as f32;
    let north = deinterleave(&samples, 2, args.channel);
    if north.is_empty() {
        bail!("{} contains no samples", path.display());
    }

    let rotation_hz = args.rotation.as_hz();
    let nominal_period = (sample_rate / rotation_hz) as f64;
    let min_interval = (args.min_interval_ms / 1000.0 * sample_rate) as usize;
    let expected = (north.len() as f64 / nominal_period).round() as usize;

    let mut rows = Vec::new();
    for &cutoff in &args.cutoffs {
        // Tap count is meaningless without a filter; report it once.
        let taps_list: Vec<usize> = if cutoff <= 0.0 {
            vec![0]
        } else {
            args.taps.clone()
        };
        for taps in taps_list {
            let filtered = preprocess(&north, sample_rate, cutoff, taps, args.transition)?;
            let peak = filtered.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
            if peak <= 0.0 {
                continue;
            }
            let peaks = detect_peaks(&filtered, args.threshold * peak, min_interval, 8);
            let peaks: Vec<usize> = peaks
                .into_iter()
                .filter(|&p| p >= 8 && p + 8 < filtered.len())
                .collect();

            let limiter_epochs: Vec<f64> = peaks.iter().map(|&p| p as f64).collect();
            let centroid_epochs: Vec<f64> = peaks
                .iter()
                .map(|&p| p as f64 + centroid_offset(&filtered, p, 3))
                .collect();

            let hard_limiter = summarize(&limiter_epochs, nominal_period);
            let centroid = summarize(&centroid_epochs, nominal_period);
            let rate_hz = centroid
                .as_ref()
                .or(hard_limiter.as_ref())
                .map(|r| sample_rate as f64 / r.period)
                .unwrap_or(0.0);

            rows.push(Row {
                cutoff,
                taps,
                detected: peaks.len(),
                expected,
                rate_hz,
                hard_limiter,
                centroid,
            });
        }
    }
    Ok(rows)
}

fn format_residual(residual: &Option<Residuals>) -> (String, String) {
    match residual {
        Some(r) => (
            format!("{:.3}", r.rms_degrees()),
            format!("{:.3}", r.p95_degrees()),
        ),
        None => ("-".into(), "-".into()),
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.csv {
        println!(
            "file,cutoff_hz,taps,detected,expected,rate_hz,limiter_rms_deg,limiter_p95_deg,centroid_rms_deg,centroid_p95_deg"
        );
    }

    for path in &args.files {
        let rows = analyze_file(path, &args)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if !args.csv {
            println!("\n=== {name} ===");
            println!(
                "{:>9} {:>5} {:>9} {:>10} {:>11} {:>11} {:>11} {:>11}",
                "cutoff",
                "taps",
                "detected",
                "rate (Hz)",
                "HL rms °",
                "HL p95 °",
                "CE rms °",
                "CE p95 °"
            );
        }

        for row in rows {
            let (hl_rms, hl_p95) = format_residual(&row.hard_limiter);
            let (ce_rms, ce_p95) = format_residual(&row.centroid);
            let cutoff = if row.cutoff <= 0.0 {
                "none".to_string()
            } else {
                format!("{:.0}", row.cutoff)
            };
            let taps = if row.taps == 0 {
                "-".to_string()
            } else {
                row.taps.to_string()
            };

            if args.csv {
                println!(
                    "{name},{cutoff},{taps},{},{},{:.4},{hl_rms},{hl_p95},{ce_rms},{ce_p95}",
                    row.detected, row.expected, row.rate_hz
                );
            } else {
                println!(
                    "{:>9} {:>5} {:>4}/{:<4} {:>10.4} {:>11} {:>11} {:>11} {:>11}",
                    cutoff,
                    taps,
                    row.detected,
                    row.expected,
                    row.rate_hz,
                    hl_rms,
                    hl_p95,
                    ce_rms,
                    ce_p95
                );
            }
        }
    }

    Ok(())
}

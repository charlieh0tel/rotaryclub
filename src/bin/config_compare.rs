//! Run two configurations over the same signal and report the difference.
//!
//! Every comparison on this work so far -- pulse estimator, highpass cutoff,
//! loop bandwidth, the loop's phase correction on and off -- has meant editing
//! a default, rebuilding, and diffing two report files by hand. That is slow,
//! and twice it produced numbers from a build that was one iteration stale.
//!
//! Both sides start from the shipped defaults and take dotted `key=value`
//! overrides, so a comparison says exactly what it changed:
//!
//!   config_compare -a north_tick.estimator=hard-limiter \
//!                  -b north_tick.estimator=energy-centroid
//!
//!   config_compare -a north_tick.dpll.natural_frequency_hz=1 \
//!                  -b north_tick.dpll.natural_frequency_hz=2 --noise 0.05
//!
//! The signal is a Doppler tone at a known bearing with band-limited north
//! pulses at the true rotation epochs. Placing them at the nearest whole
//! sample instead would put six degrees of bearing into every rotation and
//! swamp what is being compared.

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, ValueEnum};
use rotaryclub::config::{
    BearingMethod, NorthPulseEstimator, NorthTrackingMode, RdfConfig, RotationFrequency,
};
use rotaryclub::processing::RdfProcessor;
use std::f32::consts::PI;

/// Half-width, in samples, of the synthesized north pulse.
const PULSE_HALF_WIDTH: i64 = 12;

#[derive(Parser)]
#[command(
    about = "Run two configurations over the same signal and report the difference",
    long_about = None
)]
struct Args {
    /// Override for configuration A, as key=value. Repeatable.
    #[arg(short = 'a', long = "config-a", value_name = "KEY=VALUE")]
    a: Vec<String>,

    /// Override for configuration B, as key=value. Repeatable.
    #[arg(short = 'b', long = "config-b", value_name = "KEY=VALUE")]
    b: Vec<String>,

    /// Seconds of signal. A loop of bandwidth f needs several times 1/f to
    /// settle, and measuring before it has settled reports the transient.
    #[arg(long, default_value = "10.0")]
    seconds: f32,

    /// Peak amplitude of additive noise, in both channels.
    #[arg(long, default_value = "0.0")]
    noise: f32,

    /// Whole samples of deterministic jitter applied to each north pulse.
    #[arg(long, default_value = "0")]
    jitter: i32,

    /// Bearing the Doppler tone encodes, in degrees.
    #[arg(long, default_value = "200.0")]
    bearing: f32,

    /// Rotation rate of the simulated array, in Hz.
    #[arg(long)]
    rotation_hz: Option<f32>,

    /// List the keys that can be overridden, and exit.
    #[arg(long)]
    list_keys: bool,
}

/// Keys accepted by `--config-a` and `--config-b`.
///
/// Spelled out rather than derived: the configuration has no serde
/// representation, and a hand-written list at least fails loudly on a
/// misspelling instead of silently comparing a config against itself.
const KEYS: &[&str] = &[
    "audio.sample_rate",
    "audio.buffer_size",
    "bearing.smoothing_window",
    "doppler.method",
    "doppler.north_tick_timing_adjustment_us",
    "doppler.bandpass_taps",
    "north_tick.mode",
    "north_tick.estimator",
    "north_tick.gain_db",
    "north_tick.highpass_cutoff",
    "north_tick.highpass_transition_hz",
    "north_tick.fir_highpass_length_us",
    "north_tick.threshold",
    "north_tick.expected_pulse_amplitude",
    "north_tick.min_interval_ms",
    "north_tick.max_coast_ms",
    "north_tick.gate_sigma",
    "north_tick.dpll.initial_frequency_hz",
    "north_tick.dpll.natural_frequency_hz",
    "north_tick.dpll.damping_ratio",
];

fn parse_enum<T: ValueEnum>(value: &str, key: &str) -> Result<T> {
    T::from_str(value, true).map_err(|_| anyhow!("{key}: {value} is not one of its values"))
}

fn apply(config: &mut RdfConfig, assignment: &str) -> Result<()> {
    let (key, value) = assignment
        .split_once('=')
        .ok_or_else(|| anyhow!("expected key=value, got {assignment}"))?;

    let number = |what: &str| -> Result<f32> {
        value
            .parse::<f32>()
            .with_context(|| format!("{what} takes a number, got {value}"))
    };
    let count = |what: &str| -> Result<usize> {
        value
            .parse::<usize>()
            .with_context(|| format!("{what} takes a whole number, got {value}"))
    };

    match key {
        "audio.sample_rate" => config.audio.sample_rate = count(key)? as u32,
        "audio.buffer_size" => config.audio.buffer_size = count(key)?,
        "bearing.smoothing_window" => config.bearing.smoothing_window = count(key)?,
        "doppler.method" => config.doppler.method = parse_enum::<BearingMethod>(value, key)?,
        "doppler.north_tick_timing_adjustment_us" => {
            config.doppler.north_tick_timing_adjustment_us = number(key)?
        }
        "doppler.bandpass_taps" => config.doppler.bandpass_taps = count(key)?,
        "north_tick.mode" => config.north_tick.mode = parse_enum::<NorthTrackingMode>(value, key)?,
        "north_tick.estimator" => {
            config.north_tick.estimator = parse_enum::<NorthPulseEstimator>(value, key)?
        }
        "north_tick.gain_db" => config.north_tick.gain_db = number(key)?,
        "north_tick.highpass_cutoff" => config.north_tick.highpass_cutoff = number(key)?,
        "north_tick.highpass_transition_hz" => {
            config.north_tick.highpass_transition_hz = number(key)?
        }
        "north_tick.fir_highpass_length_us" => {
            config.north_tick.fir_highpass_length_us = number(key)?
        }
        "north_tick.threshold" => config.north_tick.threshold = number(key)?,
        "north_tick.expected_pulse_amplitude" => {
            config.north_tick.expected_pulse_amplitude = number(key)?
        }
        "north_tick.min_interval_ms" => config.north_tick.min_interval_ms = number(key)?,
        "north_tick.max_coast_ms" => config.north_tick.max_coast_ms = number(key)?,
        "north_tick.gate_sigma" => config.north_tick.gate_sigma = number(key)?,
        "north_tick.dpll.initial_frequency_hz" => {
            config.north_tick.dpll.initial_frequency_hz = number(key)?
        }
        "north_tick.dpll.natural_frequency_hz" => {
            config.north_tick.dpll.natural_frequency_hz = number(key)?
        }
        "north_tick.dpll.damping_ratio" => config.north_tick.dpll.damping_ratio = number(key)?,
        _ => bail!("{key} is not a key this compares; --list-keys lists them"),
    }
    Ok(())
}

fn noise_at(index: usize) -> f32 {
    let mut x = (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xC0FF_EE00_1234_5678;
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 29;
    (((x >> 32) as u32) as f32 / (u32::MAX as f32)) * 2.0 - 1.0
}

fn jitter_at(index: usize, max_abs: i32) -> f64 {
    if max_abs <= 0 {
        0.0
    } else {
        ((index as f32 * 0.37).sin() * max_abs as f32).round() as f64
    }
}

/// Interleaved stereo: Doppler tone left, north pulses right.
fn build_signal(args: &Args, config: &RdfConfig, rotation_hz: f32) -> (Vec<f32>, Vec<f64>) {
    let sample_rate = config.audio.sample_rate as f32;
    let num_samples = (sample_rate * args.seconds) as usize;
    let period = sample_rate as f64 / rotation_hz as f64;
    let omega = 2.0 * PI * rotation_hz / sample_rate;
    let bearing = args.bearing.to_radians();
    let amplitude = config.north_tick.expected_pulse_amplitude;

    let mut north = vec![0.0f32; num_samples];
    let mut epochs = Vec::new();
    let mut k = 0i64;
    loop {
        // The tone's north is at sample zero, so the pulses have to start
        // there too: offsetting them by a hundred samples would displace the
        // reference by a third of a rotation, which is 122 degrees of bearing.
        let epoch = k as f64 * period + jitter_at(k as usize, args.jitter);
        if epoch >= num_samples as f64 - PULSE_HALF_WIDTH as f64 {
            break;
        }
        epochs.push(epoch);
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
        out.push((omega * i as f32 - bearing).sin() + args.noise * noise_at(i));
        out.push(tick + args.noise * 0.35 * noise_at(i ^ 0x5555));
    }
    (out, epochs)
}

struct Measurement {
    tick_mean: f64,
    tick_p95: f64,
    bearing_mean: f64,
    bearing_p95: f64,
    bearing_offset: f64,
    snr_db: f64,
    signal_strength: f64,
    confidence: f64,
    reference_phase_sigma_deg: f64,
    stated_sigma_deg: f64,
    ticks: usize,
    bearings: usize,
}

fn percentile(values: &mut [f64], p: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    values.sort_by(f64::total_cmp);
    let idx = (((values.len() - 1) as f64) * p).round() as usize;
    values[idx]
}

/// The tail of the run, so a loop still acquiring is not reported as error.
fn tail<T>(values: &[T], fraction: f64) -> &[T] {
    if values.is_empty() {
        return values;
    }
    let start = ((values.len() as f64) * (1.0 - fraction)) as usize;
    &values[start.min(values.len() - 1)..]
}

fn measure(config: &RdfConfig, signal: &[f32], epochs: &[f64], truth: f32) -> Result<Measurement> {
    let mut processor = RdfProcessor::new(config, false, true)
        .map_err(|e| anyhow!("configuration rejected: {e}"))?;
    let results = processor.process_signal(signal);
    // The tracker's own estimate of how much its tick timing scatters,
    // expressed as the bearing degrees that scatter is worth.
    let reference_phase_sigma_deg = processor
        .phase_error_variance()
        .map(|v| (v.max(0.0).sqrt()).to_degrees() as f64)
        .unwrap_or(f64::NAN);

    let mut tick_errors = Vec::new();
    let mut bearing_errors = Vec::new();
    let mut snrs = Vec::new();
    let mut strengths = Vec::new();
    let mut stated = Vec::new();
    let mut confidences = Vec::new();
    let mut signed_errors = Vec::new();
    for result in &results {
        let time = result.north_tick.sample_index as f64
            + result.north_tick.fractional_sample_offset as f64;
        if let Some(epoch) = epochs
            .iter()
            .min_by(|a, b| (*a - time).abs().total_cmp(&(*b - time).abs()))
            && (time - epoch).abs() < 3.0
        {
            tick_errors.push((time - epoch).abs());
        }
        if let Some(bearing) = result.bearing {
            let error =
                (((bearing.bearing_degrees - truth) + 540.0).rem_euclid(360.0) - 180.0) as f64;
            bearing_errors.push(error.abs());
            snrs.push(bearing.metrics.snr_db as f64);
            strengths.push(bearing.metrics.signal_strength as f64);
            if let Some(u) = bearing.metrics.bearing_uncertainty_deg {
                stated.push(u as f64);
            }
            confidences.push(bearing.confidence as f64);
            signed_errors.push(error);
        }
    }

    // The offset is taken over the same tail as everything else. Computing it
    // over the whole run, which this did, folded each side's acquisition
    // transient into it -- so comparing two loop bandwidths reported a
    // difference in offset that was mostly a difference in how long each took
    // to acquire, the exact confound the tail exists to remove.
    let (mut cos_sum, mut sin_sum) = (0.0f64, 0.0f64);
    for error in tail(&signed_errors, 0.2) {
        let radians = error.to_radians();
        cos_sum += radians.cos();
        sin_sum += radians.sin();
    }

    let mut tick_tail = tail(&tick_errors, 0.2).to_vec();
    let mut bearing_tail = tail(&bearing_errors, 0.2).to_vec();
    let mean = |v: &[f64]| {
        if v.is_empty() {
            f64::NAN
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        }
    };

    let snr_tail = tail(&snrs, 0.2).to_vec();
    let strength_tail = tail(&strengths, 0.2).to_vec();
    let stated_tail = tail(&stated, 0.2).to_vec();
    let confidence_tail = tail(&confidences, 0.2).to_vec();

    Ok(Measurement {
        tick_mean: mean(&tick_tail),
        tick_p95: percentile(&mut tick_tail, 0.95),
        bearing_mean: mean(&bearing_tail),
        bearing_p95: percentile(&mut bearing_tail, 0.95),
        bearing_offset: sin_sum.atan2(cos_sum).to_degrees(),
        snr_db: mean(&snr_tail),
        signal_strength: mean(&strength_tail),
        confidence: mean(&confidence_tail),
        reference_phase_sigma_deg,
        stated_sigma_deg: mean(&stated_tail),
        ticks: tick_errors.len(),
        bearings: bearing_errors.len(),
    })
}

fn describe(overrides: &[String]) -> String {
    if overrides.is_empty() {
        "defaults".to_string()
    } else {
        overrides.join(" ")
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.list_keys {
        println!("keys accepted by --config-a and --config-b:");
        for key in KEYS {
            println!("  {key}");
        }
        return Ok(());
    }

    let mut config_a = RdfConfig::default();
    let mut config_b = RdfConfig::default();
    if let Some(hz) = args.rotation_hz {
        config_a.apply_rotation(RotationFrequency::from_hz(hz));
        config_b.apply_rotation(RotationFrequency::from_hz(hz));
    }
    for assignment in &args.a {
        apply(&mut config_a, assignment).context("configuration A")?;
    }
    for assignment in &args.b {
        apply(&mut config_b, assignment).context("configuration B")?;
    }

    let rotation_hz = args.rotation_hz.unwrap_or(config_a.doppler.expected_freq);
    let degrees_per_sample = 360.0 * rotation_hz / config_a.audio.sample_rate as f32;

    if args.a == args.b {
        eprintln!("warning: both sides are the same configuration\n");
    }

    // One signal is built, from A, and both sides are measured on it. Any key
    // that changes what the signal *is* therefore has to agree between the
    // two, or B is being scored against a stimulus built for A: setting
    // A to 48 kHz and B to 96 would hand B a tone at twice the frequency,
    // outside its own passband, and report that as a configuration
    // difference. Overriding them together is fine and still meaningful.
    for key in ["audio.sample_rate", "north_tick.expected_pulse_amplitude"] {
        let value_of = |side: &[String]| {
            side.iter()
                .filter_map(|a| a.split_once('='))
                .filter(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
                .next_back()
        };
        if value_of(&args.a) != value_of(&args.b) {
            bail!(
                "{key} differs between the two sides. It decides what the signal is, and \
                 one signal is built for both, so the comparison would not mean what it \
                 looks like. Set it the same on both sides, or leave it off both."
            );
        }
    }

    // One signal, both configurations, so nothing but the configuration
    // differs. Built from A because the two can disagree about sample rate,
    // which would otherwise silently change the stimulus as well.
    let (signal, epochs) = build_signal(&args, &config_a, rotation_hz);
    let a = measure(&config_a, &signal, &epochs, args.bearing)?;
    let b = measure(&config_b, &signal, &epochs, args.bearing)?;

    println!("A: {}", describe(&args.a));
    println!("B: {}", describe(&args.b));
    println!(
        "\n{:.1} s at {:.1} Hz, noise {}, jitter {} samples, bearing {} deg",
        args.seconds, rotation_hz, args.noise, args.jitter, args.bearing
    );
    println!(
        "one sample = {degrees_per_sample:.1} deg. errors are over the last fifth of the run\n"
    );

    println!("{:<28} {:>12} {:>12} {:>12}", "", "A", "B", "B - A");
    let row = |name: &str, a: f64, b: f64| {
        println!("{name:<28} {a:>12.4} {b:>12.4} {:>+12.4}", b - a);
    };
    row("tick error mean (samp)", a.tick_mean, b.tick_mean);
    row("tick error p95 (samp)", a.tick_p95, b.tick_p95);
    row(
        "tick error mean (deg)",
        a.tick_mean * degrees_per_sample as f64,
        b.tick_mean * degrees_per_sample as f64,
    );
    row("bearing error mean (deg)", a.bearing_mean, b.bearing_mean);
    row("bearing error p95 (deg)", a.bearing_p95, b.bearing_p95);
    row("bearing offset (deg)", a.bearing_offset, b.bearing_offset);
    row("snr (dB)", a.snr_db, b.snr_db);
    row("signal strength", a.signal_strength, b.signal_strength);
    row("stated sigma (deg)", a.stated_sigma_deg, b.stated_sigma_deg);
    row(
        "reference sigma (deg)",
        a.reference_phase_sigma_deg,
        b.reference_phase_sigma_deg,
    );
    row("confidence", a.confidence, b.confidence);
    println!(
        "{:<28} {:>12} {:>12} {:>+12}",
        "ticks matched",
        a.ticks,
        b.ticks,
        b.ticks as i64 - a.ticks as i64
    );
    println!(
        "{:<28} {:>12} {:>12} {:>+12}",
        "bearings produced",
        a.bearings,
        b.bearings,
        b.bearings as i64 - a.bearings as i64
    );
    println!("\nof {} rotations in the signal", epochs.len());

    Ok(())
}

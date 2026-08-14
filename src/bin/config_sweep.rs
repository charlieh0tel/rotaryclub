//! Sweep any number of axes against one generator and print the table.
//!
//! `config_compare` takes an A and a B, and most real questions are not two
//! configurations but a grid: the detection threshold needed threshold against
//! amplitude against noise against tracking mode, and the loop bandwidth
//! needed bandwidth against noise. Both were hand-rolled into their own
//! examples, each with its own copy of the signal generator, and that is
//! exactly how the same mislabelled noise axis came to live in two of them
//! independently -- twelve uniform draws over six rather than two, so every
//! row was measured at a third of the noise it claimed -- and how one of them
//! came to sweep in a tracking mode that does not ship.
//!
//! The point of this is not that a cross product is hard to write. It is that
//! the generator and the labels on the axes live in one place, so a sweep
//! cannot quietly disagree with another sweep about what "noise 0.2" means.
//! The signal comes from `simulation::generate_impaired_signal`, the same one
//! the tests use, and the stimulus axes name physical quantities that were
//! measured on the recordings rather than knob positions.
//!
//!   config_sweep --axis north_tick.threshold=0.10,0.15,0.25 \
//!                --axis north_noise=0.0,0.05,0.20
//!
//!   config_sweep --axis north_tick.dpll.natural_frequency_hz=1,2,4 \
//!                --axis doppler_noise=0.2,0.8,6.5

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, ValueEnum};
use rotaryclub::config::{
    BearingMethod, NorthPulseEstimator, NorthTrackingMode, RdfConfig, RotationFrequency,
};
use rotaryclub::processing::RdfProcessor;
use rotaryclub::simulation::{SignalImpairment, generate_impaired_signal};

#[derive(Parser)]
#[command(
    about = "Sweep configuration and stimulus axes against one generator",
    long_about = None
)]
struct Args {
    /// An axis to sweep, as `key=v1,v2,...`. Repeatable; the cross product of
    /// all axes is run. The key is either a dotted configuration key or one of
    /// the stimulus names listed by `--list-axes`.
    #[arg(long = "axis", value_name = "KEY=V1,V2,...", required_unless_present_any = ["list_axes"])]
    axes: Vec<String>,

    /// Seconds of signal per cell.
    #[arg(long, default_value = "6.0")]
    seconds: f32,

    /// List the axes that can be swept, and exit.
    #[arg(long)]
    list_axes: bool,
}

/// Stimulus axes, named for the physical quantity rather than the knob.
///
/// `doppler_noise` is the interfering audio power inside the doppler passband
/// relative to the tone, which is what decides a bearing; the recordings in
/// `data/` measure 0.199, 0.793 and 6.579. It is deliberately not the ratio
/// over the whole channel, which is the natural thing to reach for and
/// overstates the damage about tenfold because real audio sits well below the
/// passband.
///
/// `north_noise` is an RMS on the north channel. The recordings measure a
/// floor around 0.0006, so anything above 0.01 is beyond what has been seen.
const STIMULUS_AXES: &[(&str, &str)] = &[
    ("doppler_noise", "passband noise power relative to the tone"),
    ("north_noise", "RMS noise on the north channel"),
    ("bearing", "true bearing in degrees"),
    ("rotation_hz", "rotation rate"),
    ("pulse_amplitude", "north pulse amplitude before gain"),
];

const CONFIG_AXES: &[&str] = &[
    "audio.buffer_size",
    "bearing.smoothing_window",
    "doppler.method",
    "doppler.bandpass_taps",
    "north_tick.mode",
    "north_tick.estimator",
    "north_tick.agc.enabled",
    "north_tick.gain_db",
    "north_tick.highpass_cutoff",
    "north_tick.fir_highpass_length_us",
    "north_tick.threshold",
    "north_tick.min_interval_ms",
    "north_tick.max_coast_ms",
    "north_tick.gate_sigma",
    "north_tick.dpll.natural_frequency_hz",
    "north_tick.dpll.damping_ratio",
];

fn parse_enum<T: ValueEnum>(value: &str, key: &str) -> Result<T> {
    T::from_str(value, true).map_err(|_| anyhow!("{key}: {value} is not one of its values"))
}

fn apply_config(config: &mut RdfConfig, key: &str, value: &str) -> Result<bool> {
    let number = || -> Result<f32> {
        value
            .parse::<f32>()
            .with_context(|| format!("{key} takes a number, got {value}"))
    };
    let count = || -> Result<usize> {
        value
            .parse::<usize>()
            .with_context(|| format!("{key} takes a whole number, got {value}"))
    };

    match key {
        "audio.buffer_size" => config.audio.buffer_size = count()?,
        "bearing.smoothing_window" => config.bearing.smoothing_window = count()?,
        "doppler.method" => config.doppler.method = parse_enum::<BearingMethod>(value, key)?,
        "doppler.bandpass_taps" => config.doppler.bandpass_taps = count()?,
        "north_tick.mode" => config.north_tick.mode = parse_enum::<NorthTrackingMode>(value, key)?,
        "north_tick.estimator" => {
            config.north_tick.estimator = parse_enum::<NorthPulseEstimator>(value, key)?
        }
        "north_tick.agc.enabled" => {
            config.north_tick.agc.enabled = value
                .parse::<bool>()
                .with_context(|| format!("{key} takes true or false, got {value}"))?
        }
        "north_tick.gain_db" => config.north_tick.gain_db = number()?,
        "north_tick.highpass_cutoff" => config.north_tick.highpass_cutoff = number()?,
        "north_tick.fir_highpass_length_us" => config.north_tick.fir_highpass_length_us = number()?,
        "north_tick.threshold" => config.north_tick.threshold = number()?,
        "north_tick.min_interval_ms" => config.north_tick.min_interval_ms = number()?,
        "north_tick.max_coast_ms" => config.north_tick.max_coast_ms = number()?,
        "north_tick.gate_sigma" => config.north_tick.gate_sigma = number()?,
        "north_tick.dpll.natural_frequency_hz" => {
            config.north_tick.dpll.natural_frequency_hz = number()?
        }
        "north_tick.dpll.damping_ratio" => config.north_tick.dpll.damping_ratio = number()?,
        _ => return Ok(false),
    }
    Ok(true)
}

/// The stimulus for one cell. Two cells sharing this share a signal, which is
/// what keeps a comparison across configuration axes honest.
#[derive(Clone, Copy, PartialEq)]
struct Stimulus {
    doppler_noise: f32,
    north_noise: f32,
    bearing: f32,
    rotation_hz: f32,
    pulse_amplitude: f32,
}

impl Stimulus {
    fn default_for(config: &RdfConfig) -> Self {
        Self {
            doppler_noise: 0.8,
            north_noise: 0.0,
            bearing: 200.0,
            rotation_hz: config.doppler.expected_freq,
            pulse_amplitude: config.north_tick.expected_pulse_amplitude,
        }
    }

    fn apply(&mut self, key: &str, value: &str) -> Result<bool> {
        let number = || -> Result<f32> {
            value
                .parse::<f32>()
                .with_context(|| format!("{key} takes a number, got {value}"))
        };
        match key {
            "doppler_noise" => self.doppler_noise = number()?,
            "north_noise" => self.north_noise = number()?,
            "bearing" => self.bearing = number()?,
            "rotation_hz" => self.rotation_hz = number()?,
            "pulse_amplitude" => self.pulse_amplitude = number()?,
            _ => return Ok(false),
        }
        Ok(true)
    }
}

struct Cell {
    labels: Vec<String>,
    tick_error: f64,
    bearing_error: f64,
    /// Mean signed error: the part of it every estimate shares.
    bearing_bias: f64,
    /// Standard deviation about that: the part that scatters.
    ///
    /// Reported separately because the uncertainty figure models only the
    /// second. A displacement all the estimates share is invisible to a
    /// spread, so comparing `stated` against total error charges it for
    /// something it does not claim to measure.
    ///
    /// A standard deviation and not a mean absolute one, so it is the same
    /// quantity `stated` is. The two differ by a factor of 0.8 for a normal
    /// distribution, which is enough to turn a figure that is correct into
    /// one that looks like it understates by a fifth.
    bearing_scatter: f64,
    bearing_p95: f64,
    stated_sigma: f64,
    bearings: usize,
}

fn percentile(values: &mut [f64], p: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * p).round() as usize]
}

fn tail<T: Copy>(values: &[T], fraction: f64) -> Vec<T> {
    if values.is_empty() {
        return Vec::new();
    }
    let start = ((values.len() as f64) * (1.0 - fraction)) as usize;
    values[start.min(values.len() - 1)..].to_vec()
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        f64::NAN
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn measure(config: &RdfConfig, signal: &[f32], truth: f32, period: f64) -> Result<Cell> {
    let mut processor = RdfProcessor::new(config, false, true)
        .map_err(|e| anyhow!("configuration rejected: {e}"))?;
    let results = processor.process_signal(signal);

    let mut tick_errors = Vec::new();
    let mut bearing_errors = Vec::new();
    let mut signed_errors = Vec::new();
    let mut stated = Vec::new();
    for result in &results {
        let time = result.north_tick.sample_index as f64
            + result.north_tick.fractional_sample_offset as f64;
        let k = (time / period).round();
        let error = time - k * period;
        if error.abs() < 3.0 {
            tick_errors.push(error.abs());
        }
        if let Some(bearing) = result.bearing {
            let error = (((bearing.raw_bearing - truth) + 540.0).rem_euclid(360.0) - 180.0) as f64;
            bearing_errors.push(error.abs());
            signed_errors.push(error);
            if let Some(u) = bearing.metrics.bearing_uncertainty_deg {
                stated.push(u as f64);
            }
        }
    }

    let mut bearing_tail = tail(&bearing_errors, 0.2);
    let signed_tail = tail(&signed_errors, 0.2);
    let bias = mean(&signed_tail);
    let scatter = if signed_tail.is_empty() {
        f64::NAN
    } else {
        (signed_tail
            .iter()
            .map(|e| (e - bias) * (e - bias))
            .sum::<f64>()
            / signed_tail.len() as f64)
            .sqrt()
    };
    Ok(Cell {
        labels: Vec::new(),
        tick_error: mean(&tail(&tick_errors, 0.2)),
        bearing_error: mean(&bearing_tail.clone()),
        bearing_bias: bias,
        bearing_scatter: scatter,
        bearing_p95: percentile(&mut bearing_tail, 0.95),
        stated_sigma: mean(&tail(&stated, 0.2)),
        bearings: bearing_errors.len(),
    })
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.list_axes {
        println!("stimulus axes:");
        for (name, what) in STIMULUS_AXES {
            println!("  {name:<36} {what}");
        }
        println!("\nconfiguration axes:");
        for key in CONFIG_AXES {
            println!("  {key}");
        }
        return Ok(());
    }

    // Parse the axes, keeping their order so the table reads as written.
    let mut axes: Vec<(String, Vec<String>)> = Vec::new();
    for spec in &args.axes {
        let (key, values) = spec
            .split_once('=')
            .ok_or_else(|| anyhow!("expected key=v1,v2,..., got {spec}"))?;
        let values: Vec<String> = values.split(',').map(|v| v.trim().to_string()).collect();
        if values.is_empty() {
            bail!("{key} has no values");
        }
        axes.push((key.to_string(), values));
    }

    // Cross product, in the order the axes were given.
    let mut combinations: Vec<Vec<String>> = vec![Vec::new()];
    for (_, values) in &axes {
        combinations = combinations
            .iter()
            .flat_map(|prefix| {
                values.iter().map(move |v| {
                    let mut next = prefix.clone();
                    next.push(v.clone());
                    next
                })
            })
            .collect();
    }

    println!("{} cells, {:.1} s each\n", combinations.len(), args.seconds);

    // Work through the cells grouped by stimulus, so each signal is built
    // once however the axes were ordered, then report in the order asked for.
    // The signal is deterministic, so this is a saving rather than a
    // correctness matter -- but a sweep that rebuilds a six second signal per
    // cell is one nobody runs at the size that would answer anything.
    let mut order: Vec<usize> = (0..combinations.len()).collect();
    let stimulus_for = |combination: &Vec<String>| -> Result<Stimulus> {
        let mut config = RdfConfig::default();
        let mut stimulus = Stimulus::default_for(&config);
        for ((key, _), value) in axes.iter().zip(combination) {
            let _ = apply_config(&mut config, key, value)?;
            let _ = stimulus.apply(key, value)?;
        }
        Ok(stimulus)
    };
    let mut keys = Vec::with_capacity(combinations.len());
    for combination in &combinations {
        let s = stimulus_for(combination)?;
        keys.push((
            s.doppler_noise.to_bits(),
            s.north_noise.to_bits(),
            s.bearing.to_bits(),
            s.rotation_hz.to_bits(),
            s.pulse_amplitude.to_bits(),
        ));
    }
    order.sort_by_key(|&i| keys[i]);

    let mut cached: Option<(Stimulus, Vec<f32>)> = None;
    let mut rows: Vec<Option<Cell>> = (0..combinations.len()).map(|_| None).collect();

    for &index in &order {
        let combination = &combinations[index];
        let mut config = RdfConfig::default();
        let mut stimulus = Stimulus::default_for(&config);

        // The rotation is applied first, because `apply_rotation` rewrites
        // several fields from the defaults -- the dead time, the loop's
        // frequency bounds, the doppler passband -- and doing it afterwards
        // silently discarded any axis that touched one of them. Sweeping
        // min_interval_ms produced four identical rows before this was moved,
        // which is the failure this tool exists to make visible: a sweep that
        // is not sweeping what it says.
        for ((key, _), value) in axes.iter().zip(combination) {
            if stimulus.apply(key, value)? {
                continue;
            }
        }
        config.apply_rotation(RotationFrequency::from_hz(stimulus.rotation_hz));

        for ((key, _), value) in axes.iter().zip(combination) {
            let applied = apply_config(&mut config, key, value)? || stimulus.apply(key, value)?;
            if !applied {
                bail!("{key} is not an axis this sweeps; --list-axes lists them");
            }
        }
        config.bearing.smoothing_window = config.bearing.smoothing_window.max(1);

        // One signal per distinct stimulus, reused across the configuration
        // axes. Rebuilding it per cell would let two configurations be
        // compared against different noise, which is the confound this exists
        // to remove.
        let signal = match &cached {
            Some((cached_stimulus, signal)) if *cached_stimulus == stimulus => signal.clone(),
            _ => {
                let built = generate_impaired_signal(
                    args.seconds,
                    config.audio.sample_rate,
                    stimulus.rotation_hz,
                    |_| stimulus.bearing,
                    SignalImpairment {
                        passband_noise_to_tone: stimulus.doppler_noise,
                        north_noise_rms: stimulus.north_noise,
                        north_pulse_amplitude: stimulus.pulse_amplitude,
                        ..SignalImpairment::representative()
                    },
                );
                cached = Some((stimulus, built.clone()));
                built
            }
        };

        let period = config.audio.sample_rate as f64 / stimulus.rotation_hz as f64;
        let mut cell = measure(&config, &signal, stimulus.bearing, period)?;
        cell.labels = combination.clone();
        rows[index] = Some(cell);
    }

    let rows: Vec<Cell> = rows.into_iter().flatten().collect();

    let headers: Vec<&str> = axes.iter().map(|(k, _)| k.as_str()).collect();
    let width: Vec<usize> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            rows.iter()
                .map(|r| r.labels[i].len())
                .chain(std::iter::once(h.len()))
                .max()
                .unwrap_or(8)
                .max(6)
        })
        .collect();

    for (h, w) in headers.iter().zip(&width) {
        print!("{h:>w$} ", w = w);
    }
    println!(
        "{:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>8}",
        "tick", "bearing", "bias", "scatter", "b p95", "stated", "count"
    );

    for row in &rows {
        for (label, w) in row.labels.iter().zip(&width) {
            print!("{label:>w$} ", w = w);
        }
        println!(
            "{:>9.4} {:>9.3} {:>9.3} {:>9.3} {:>9.3} {:>9.3} {:>8}",
            row.tick_error,
            row.bearing_error,
            row.bearing_bias,
            row.bearing_scatter,
            row.bearing_p95,
            row.stated_sigma,
            row.bearings
        );
    }

    println!(
        "\ntick error in samples, bearings in degrees, over the last fifth of each run.\n\
         doppler_noise is passband noise power against the tone; the recordings in data/\n\
         measure 0.199, 0.793 and 6.579. north_noise is an RMS; they measure about 0.0006."
    );

    Ok(())
}

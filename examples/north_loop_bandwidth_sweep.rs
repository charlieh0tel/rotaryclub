//! What does the DPLL's loop bandwidth buy, and what does it cost?
//!
//! `dpll.natural_frequency_hz` ships at 1 Hz. A wider loop acquires faster and
//! follows a moving rate more closely; a narrower one averages more of the
//! measurement's noise out of the reported tick. The coasting budget sits on
//! the far side of that trade: it is derived from the scatter of the frequency
//! estimate, so a loop that admits more noise into its rate estimate earns a
//! shorter holdover, and a dropout it could once have covered becomes a gap.
//!
//! Nothing here has been measured together. This sweeps the bandwidth against
//! all three: how long acquisition takes from a rate the loop was not
//! configured for, what steady-state timing error remains once it has settled,
//! and how many rotations of holdover the settled loop earns.

use rotaryclub::config::{NorthTrackingMode, RdfConfig};
use rotaryclub::rdf::{NorthReferenceTracker, NorthTracker};

/// Half-width, in samples, of the synthesized north pulse.
const PULSE_HALF_WIDTH: i64 = 12;

/// Timing error, in samples, at or below which the loop counts as acquired.
const ACQUIRED_SAMPLES: f64 = 0.1;

/// Consecutive ticks that must hold inside `ACQUIRED_SAMPLES` before
/// acquisition is called, so a single passage through zero on the way to a
/// settled value is not mistaken for having arrived.
const ACQUIRED_RUN_TICKS: usize = 32;

fn noise_at(index: usize) -> f32 {
    let mut x = (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x5EED_1234_9ABC_DEF0;
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 29;
    (((x >> 33) as u32) as f32 / (u32::MAX as f32)) * 2.0 - 1.0
}

/// Band-limited impulses at the true rotation epochs.
///
/// `pulses_until` stops the pulses while the signal runs on, which is the
/// dropout the holdover measurement needs.
fn build(
    num_samples: usize,
    period: f64,
    amplitude: f32,
    noise_rms: f32,
    pulses_until: usize,
) -> (Vec<f32>, Vec<f64>) {
    let mut signal = vec![0.0f32; num_samples];
    let mut epochs = Vec::new();

    let mut k = 0i64;
    loop {
        let epoch = 100.0 + k as f64 * period;
        if epoch >= num_samples as f64 - PULSE_HALF_WIDTH as f64 {
            break;
        }
        epochs.push(epoch);
        k += 1;
        if epoch >= pulses_until as f64 {
            continue;
        }
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
            signal[n as usize] += amplitude * value as f32;
        }
    }

    if noise_rms > 0.0 {
        for (i, sample) in signal.iter_mut().enumerate() {
            // Twelve uniform draws approximate a normal.
            let mut acc = 0.0f32;
            for j in 0..12 {
                acc += noise_at(i * 12 + j);
            }
            *sample += acc / 6.0 * noise_rms;
        }
    }

    (signal, epochs)
}

fn config_at(bandwidth_hz: f32) -> RdfConfig {
    let mut config = RdfConfig::default();
    config.north_tick.mode = NorthTrackingMode::Dpll;
    config.north_tick.dpll.natural_frequency_hz = bandwidth_hz;
    config
}

fn run(config: &RdfConfig, signal: &[f32], sample_rate: f32) -> Vec<f64> {
    let mut tracker =
        NorthReferenceTracker::new(&config.north_tick, sample_rate).expect("tracker config");
    let mut ticks = Vec::new();
    for chunk in signal.chunks(512) {
        for tick in tracker.process_buffer(chunk) {
            ticks.push(tick.sample_index as f64 + tick.fractional_sample_offset as f64);
        }
    }
    ticks
}

/// Signed timing error of each tick against the epoch it belongs to.
fn errors_against(ticks: &[f64], epochs: &[f64]) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for tick in ticks {
        let nearest = epochs
            .iter()
            .min_by(|a, b| (*a - tick).abs().total_cmp(&(*b - tick).abs()))
            .copied();
        if let Some(epoch) = nearest
            && (tick - epoch).abs() < 3.0
        {
            out.push((*tick, tick - epoch));
        }
    }
    out
}

/// Seconds until the timing error settles inside `ACQUIRED_SAMPLES` and stays
/// there.
fn acquisition_seconds(errors: &[(f64, f64)], sample_rate: f32) -> Option<f64> {
    if errors.len() < ACQUIRED_RUN_TICKS {
        return None;
    }
    let mut run = 0usize;
    let mut run_started = 0.0f64;
    for (time, error) in errors {
        if error.abs() <= ACQUIRED_SAMPLES {
            if run == 0 {
                run_started = *time;
            }
            run += 1;
            if run >= ACQUIRED_RUN_TICKS {
                // Acquisition happened where the run began, not where it was
                // confirmed.
                return Some(run_started / sample_rate as f64);
            }
        } else {
            run = 0;
        }
    }
    None
}

fn mean_abs_tail(errors: &[(f64, f64)], tail_fraction: f64) -> f64 {
    if errors.is_empty() {
        return f64::NAN;
    }
    let start = ((errors.len() as f64) * (1.0 - tail_fraction)) as usize;
    let tail = &errors[start.min(errors.len() - 1)..];
    tail.iter().map(|(_, e)| e.abs()).sum::<f64>() / tail.len() as f64
}

fn main() {
    let base = RdfConfig::default();
    let sample_rate = base.audio.sample_rate as f32;
    let nominal_hz = base.doppler.expected_freq;
    let amplitude = base.north_tick.expected_pulse_amplitude;
    let nominal_period = sample_rate as f64 / nominal_hz as f64;
    let degrees_per_sample = 360.0 / nominal_period;

    // The rate the loop is configured to expect against the rate it gets. A
    // unit whose switching clock is a fraction of a percent off is the case
    // acquisition has to cover.
    let offset_hz = 1590.0f32;
    let offset_period = sample_rate as f64 / offset_hz as f64;

    // Long enough that even the narrowest loop here has settled before the
    // holdover measurement starts, so what it measures is the earned budget
    // and not how far acquisition had got.
    let settle_secs = 20.0f32;
    // The shipped max_coast_ms caps holdover at a second, which every loop at
    // or above 1 Hz reaches. Raising the cap for this measurement lets the
    // budget the loop earns be the thing that binds.
    let coast_cap_ms = 5000.0f32;

    println!(
        "shipped bandwidth {} Hz, damping {}. one sample = {:.1} deg",
        base.north_tick.dpll.natural_frequency_hz,
        base.north_tick.dpll.damping_ratio,
        degrees_per_sample
    );
    println!(
        "\nacquisition: seconds to hold within {ACQUIRED_SAMPLES} samples for \
         {ACQUIRED_RUN_TICKS} ticks,\n  starting at {:.1} Hz against a signal at {offset_hz} Hz",
        base.north_tick.dpll.initial_frequency_hz
    );
    println!("steady state: mean |error| over the last fifth of a ten second run");
    println!(
        "holdover: rotations predicted after the pulses stop, having settled for \
         {settle_secs} s,\n  with the coast cap raised to {coast_cap_ms} ms so the earned \
         budget is what binds\n"
    );

    println!(
        "{:>8} {:>10} {:>10} {:>10} {:>10} {:>10} {:>12}",
        "bw (Hz)", "acq (s)", "clean", "noise .05", "noise .15", "coast rot", "coast err"
    );

    for bandwidth in [0.25f32, 0.5, 1.0, 2.0, 4.0, 8.0] {
        let config = config_at(bandwidth);
        let n = (sample_rate * 10.0) as usize;

        // Acquisition, from a rate the loop was not configured for.
        let (signal, epochs) = build(n, offset_period, amplitude, 0.0, n);
        let acq = acquisition_seconds(
            &errors_against(&run(&config, &signal, sample_rate), &epochs),
            sample_rate,
        );

        // Steady state at the nominal rate, against three noise levels.
        let mut steady = Vec::new();
        for noise in [0.0f32, 0.05, 0.15] {
            let (sig, eps) = build(n, nominal_period, amplitude, noise, n);
            steady.push(mean_abs_tail(
                &errors_against(&run(&config, &sig, sample_rate), &eps),
                0.2,
            ));
        }

        // Holdover: settle, then take the pulses away and see how far the loop
        // predicts before it stops.
        let mut coast_config = config.clone();
        coast_config.north_tick.max_coast_ms = coast_cap_ms;
        let settle = (sample_rate * settle_secs) as usize;
        let total = settle + (sample_rate * 5.0) as usize;
        let (drop_signal, drop_epochs) = build(total, nominal_period, amplitude, 0.0, settle);
        let drop_ticks = run(&coast_config, &drop_signal, sample_rate);
        let coasted: Vec<f64> = drop_ticks
            .iter()
            .copied()
            .filter(|t| *t > settle as f64)
            .collect();
        let coast_error = coasted
            .last()
            .and_then(|last| {
                drop_epochs
                    .iter()
                    .min_by(|a, b| (*a - last).abs().total_cmp(&(*b - last).abs()))
                    .map(|epoch| last - epoch)
            })
            .unwrap_or(f64::NAN);

        let acq_text = match acq {
            Some(seconds) => format!("{seconds:.2}"),
            None => "never".into(),
        };
        println!(
            "{bandwidth:>8.2} {acq_text:>10} {:>10.4} {:>10.4} {:>10.4} {:>10} {:>12.3}",
            steady[0],
            steady[1],
            steady[2],
            coasted.len(),
            coast_error
        );
    }

    println!("\ntiming error in degrees of bearing, and holdover in seconds");
    println!(
        "{:>8} {:>12} {:>12} {:>12} {:>14}",
        "bw (Hz)", "clean (deg)", "n.05 (deg)", "n.15 (deg)", "coast (s)"
    );
    for bandwidth in [0.25f32, 0.5, 1.0, 2.0, 4.0, 8.0] {
        let config = config_at(bandwidth);
        let n = (sample_rate * 10.0) as usize;
        let mut steady = Vec::new();
        for noise in [0.0f32, 0.05, 0.15] {
            let (sig, eps) = build(n, nominal_period, amplitude, noise, n);
            steady.push(mean_abs_tail(
                &errors_against(&run(&config, &sig, sample_rate), &eps),
                0.2,
            ));
        }

        let mut coast_config = config.clone();
        coast_config.north_tick.max_coast_ms = coast_cap_ms;
        let settle = (sample_rate * settle_secs) as usize;
        let total = settle + (sample_rate * 5.0) as usize;
        let (drop_signal, _) = build(total, nominal_period, amplitude, 0.0, settle);
        let coasted = run(&coast_config, &drop_signal, sample_rate)
            .iter()
            .filter(|t| **t > settle as f64)
            .count();

        println!(
            "{:>8.2} {:>12.3} {:>12.3} {:>12.3} {:>14.3}",
            bandwidth,
            steady[0] * degrees_per_sample,
            steady[1] * degrees_per_sample,
            steady[2] * degrees_per_sample,
            coasted as f64 * nominal_period / sample_rate as f64
        );
    }
    println!("\nholdover against settle time, to separate convergence from structure");
    println!(
        "{:>8} {:>10} {:>10} {:>10} {:>10}",
        "bw (Hz)", "5 s", "10 s", "20 s", "40 s"
    );
    for bandwidth in [0.25f32, 0.5, 1.0, 2.0] {
        let mut cells = Vec::new();
        for secs in [5.0f32, 10.0, 20.0, 40.0] {
            let mut cfg = config_at(bandwidth);
            cfg.north_tick.max_coast_ms = 5000.0;
            let settle = (sample_rate * secs) as usize;
            let total = settle + (sample_rate * 5.0) as usize;
            let (sig, _) = build(total, nominal_period, amplitude, 0.0, settle);
            let coasted = run(&cfg, &sig, sample_rate)
                .iter()
                .filter(|t| **t > settle as f64)
                .count();
            cells.push(coasted);
        }
        println!(
            "{bandwidth:>8.2} {:>10} {:>10} {:>10} {:>10}",
            cells[0], cells[1], cells[2], cells[3]
        );
    }
}

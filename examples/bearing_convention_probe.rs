//! Where does the half-sample bearing offset come from?
//!
//! `doppler.north_tick_timing_adjustment_us` defaults to half a sample and it
//! is not clear what it compensates. Two harnesses disagree: against a north
//! pulse placed at the true rotation epoch the trim costs six degrees, while
//! against the system pipeline example -- whose pulses sit at
//! `round(k * period)` -- removing it costs five.
//!
//! Neither generator is biased, so the disagreement has to come from
//! something else that differs between them. This varies the two candidates
//! independently, with everything else held fixed and no noise or jitter, and
//! reports mean bearing error. A factor that flips the sign of the offset is
//! the one carrying it.

use std::f32::consts::PI;

use rotaryclub::config::{BearingMethod, RdfConfig, RotationFrequency};
use rotaryclub::processing::RdfProcessor;

const HALF_SAMPLE_US: f32 = 10.416_667;

#[derive(Clone, Copy, PartialEq)]
enum Placement {
    /// Pulse at the exact rotation epoch, which is generally fractional.
    TrueEpoch,
    /// Pulse at the nearest sample to it, as the pipeline example does.
    Rounded,
}

/// Deterministic jitter, matching the system pipeline example's generator.
fn jitter_samples(index: usize, max_abs: i32) -> i32 {
    if max_abs <= 0 {
        0
    } else {
        ((index as f32 * 0.37).sin() * max_abs as f32).round() as i32
    }
}

/// Deterministic noise, so a run is repeatable.
fn noise_at(index: usize) -> f32 {
    let mut x = (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xBEEF_1234;
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    (((x >> 33) as u32) as f32 / (u32::MAX as f32)) * 2.0 - 1.0
}

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    /// One non-zero sample, as the pipeline example does.
    Impulse,
    /// Band-limited impulse, which is what an anti-aliased converter records.
    BandLimited,
}

#[allow(clippy::too_many_arguments)]
fn build_signal(
    num_samples: usize,
    sample_rate: f32,
    rotation_hz: f32,
    bearing_deg: f32,
    placement: Placement,
    shape: Shape,
    amplitude: f32,
    jitter: i32,
    noise_peak: f32,
) -> Vec<f32> {
    let period = sample_rate as f64 / rotation_hz as f64;
    let bearing = bearing_deg.to_radians();
    let omega = 2.0 * PI * rotation_hz / sample_rate;

    let mut north = vec![0.0f32; num_samples];
    let half = 12i64;
    let mut k = 0i64;
    loop {
        let epoch = k as f64 * period;
        if epoch >= num_samples as f64 - half as f64 {
            break;
        }
        let placed = match placement {
            Placement::TrueEpoch => epoch,
            Placement::Rounded => epoch.round(),
        } + jitter_samples(k as usize, jitter) as f64;
        match shape {
            Shape::Impulse => {
                let index = placed.round() as usize;
                if index < num_samples {
                    north[index] += amplitude;
                }
            }
            Shape::BandLimited => {
                let center = placed.round() as i64;
                for n in (center - half)..=(center + half) {
                    if n < 0 || n as usize >= num_samples {
                        continue;
                    }
                    let x = n as f64 - placed;
                    let value = if x.abs() < f64::EPSILON {
                        1.0
                    } else {
                        let px = std::f64::consts::PI * x;
                        let w = px / half as f64;
                        (px.sin() / px) * (w.sin() / w)
                    };
                    north[n as usize] += amplitude * value as f32;
                }
            }
        }
        k += 1;
    }

    let mut out = Vec::with_capacity(num_samples * 2);
    for (i, &tick) in north.iter().enumerate() {
        out.push((omega * i as f32 - bearing).sin() + noise_peak * noise_at(i));
        out.push(tick + noise_peak * 0.35 * noise_at(i ^ 0x5555));
    }
    out
}

fn measure(signal: &[f32], config: &RdfConfig, truth: f32) -> Option<f32> {
    let mut processor = RdfProcessor::new(config, false, true).ok()?;
    let bearings: Vec<f32> = processor
        .process_signal(signal)
        .iter()
        .filter_map(|r| r.bearing.map(|b| b.bearing_degrees))
        .collect();
    if bearings.len() < 6 {
        return None;
    }
    let (mut x, mut y) = (0.0f32, 0.0f32);
    for bearing in &bearings[3..] {
        let radians = bearing.to_radians();
        x += radians.cos();
        y += radians.sin();
    }
    let mean = y.atan2(x).to_degrees().rem_euclid(360.0);
    Some(((mean - truth + 540.0).rem_euclid(360.0)) - 180.0)
}

fn main() {
    let sample_rate = 48_000u32;
    let rotation_hz = 1602.564f32;
    let num_samples = (sample_rate as f32 * 0.5) as usize;
    let truth = 200.0f32;
    let one_sample_us = 1e6 / sample_rate as f32;

    println!(
        "mean bearing error, degrees. one sample = {:.1} deg = {:.2} us\n",
        360.0 * rotation_hz / sample_rate as f32,
        one_sample_us
    );

    let conditions: [(&str, Placement, Shape, i32, f32); 5] = [
        (
            "clean, true epoch",
            Placement::TrueEpoch,
            Shape::BandLimited,
            0,
            0.0,
        ),
        ("clean, impulse", Placement::Rounded, Shape::Impulse, 0, 0.0),
        ("jitter only", Placement::Rounded, Shape::Impulse, 1, 0.0),
        ("noise only", Placement::Rounded, Shape::Impulse, 0, 0.08),
        (
            "jitter + noise",
            Placement::Rounded,
            Shape::Impulse,
            1,
            0.08,
        ),
    ];

    let trims: [f32; 5] = [
        -HALF_SAMPLE_US,
        -HALF_SAMPLE_US / 2.0,
        0.0,
        HALF_SAMPLE_US / 2.0,
        HALF_SAMPLE_US,
    ];

    print!("{:<20}", "condition");
    for trim in trims {
        print!("{:>12}", format!("{:+.1}us", trim));
    }
    println!();

    let probe_rate = |rate: u32, taps: usize| -> Option<f32> {
        let mut config = RdfConfig::default();
        config.audio.sample_rate = rate;
        config.apply_rotation(RotationFrequency::from_hz(rotation_hz));
        config.doppler.method = BearingMethod::Correlation;
        config.doppler.north_tick_timing_adjustment_us = 0.0;
        config.doppler.bandpass_taps = taps;
        let n = (rate as f32 * 0.5) as usize;
        let signal = build_signal(
            n,
            rate as f32,
            rotation_hz,
            truth,
            Placement::TrueEpoch,
            Shape::BandLimited,
            config.north_tick.expected_pulse_amplitude,
            0,
            0.0,
        );
        measure(&signal, &config, truth)
    };

    // Split the residual: how much of it is the north tracker mis-timing the
    // pulse, and how much is the bearing path mis-using a correct tick?
    //
    // Both rows place the pulse at the true epoch, but Shape::Impulse rounds it
    // to a whole sample on the way in. The gap between the rows is that
    // rounding, not anything the tracker does.
    for (shape_name, probe_shape) in [
        ("band-limited", Shape::BandLimited),
        ("impulse", Shape::Impulse),
    ] {
        let mut config = RdfConfig::default();
        config.audio.sample_rate = sample_rate;
        config.apply_rotation(RotationFrequency::from_hz(rotation_hz));
        let period = sample_rate as f64 / rotation_hz as f64;
        let signal = build_signal(
            num_samples,
            sample_rate as f32,
            rotation_hz,
            truth,
            Placement::TrueEpoch,
            probe_shape,
            config.north_tick.expected_pulse_amplitude,
            0,
            0.0,
        );
        let north: Vec<f32> = signal.iter().skip(1).step_by(2).copied().collect();

        let mut tracker =
            rotaryclub::rdf::NorthReferenceTracker::new(&config.north_tick, sample_rate as f32)
                .unwrap();
        let mut ticks = Vec::new();
        for chunk in north.chunks(512) {
            for tick in rotaryclub::rdf::NorthTracker::process_buffer(&mut tracker, chunk) {
                ticks.push(tick.sample_index as f64 + tick.fractional_sample_offset as f64);
            }
        }

        let mut errors = Vec::new();
        for tick in ticks.iter().skip(200) {
            let k = (tick / period).round();
            errors.push(tick - k * period);
        }
        let mean = errors.iter().sum::<f64>() / errors.len().max(1) as f64;
        println!(
            "north tracker alone, {shape_name:<13}: {:+.4} samples = {:+.3} deg, over {} ticks",
            mean,
            mean / period * 360.0,
            errors.len()
        );
    }

    // The loop bandwidth is about a hertz, so a half-second run is still
    // acquiring for most of its length. Sweeping duration separates what the
    // pipeline gets wrong in steady state from what it is still settling out of.
    println!("\nresidual at trim 0, against run length");
    println!("{:<14} {:>14}", "seconds", "error (deg)");
    for seconds in [0.5f32, 1.0, 2.0, 5.0, 10.0] {
        let mut config = RdfConfig::default();
        config.audio.sample_rate = sample_rate;
        config.apply_rotation(RotationFrequency::from_hz(rotation_hz));
        config.doppler.method = BearingMethod::Correlation;
        config.doppler.north_tick_timing_adjustment_us = 0.0;
        let n = (sample_rate as f32 * seconds) as usize;
        let signal = build_signal(
            n,
            sample_rate as f32,
            rotation_hz,
            truth,
            Placement::TrueEpoch,
            Shape::BandLimited,
            config.north_tick.expected_pulse_amplitude,
            0,
            0.0,
        );
        match measure(&signal, &config, truth) {
            Some(v) => println!("{seconds:<14.1} {v:>14.3}"),
            None => println!("{seconds:<14.1} {:>14}", "none"),
        }
    }

    println!("\nresidual at trim 0, against buffer size and AGC");
    println!(
        "{:<12} {:>14} {:>16}",
        "buffer", "agc on (deg)", "agc pinned (deg)"
    );
    for buffer_size in [128usize, 256, 512, 1024, 2048] {
        let mut cells = Vec::new();
        for pin_agc in [false, true] {
            let mut config = RdfConfig::default();
            config.audio.sample_rate = sample_rate;
            config.audio.buffer_size = buffer_size;
            config.apply_rotation(RotationFrequency::from_hz(rotation_hz));
            config.doppler.method = BearingMethod::Correlation;
            config.doppler.north_tick_timing_adjustment_us = 0.0;
            if pin_agc {
                // Fix the gain, so the only thing left in the doppler path is
                // the bandpass and the correlation itself.
                config.agc.min_gain = 1.0;
                config.agc.max_gain = 1.0;
            }
            let signal = build_signal(
                num_samples,
                sample_rate as f32,
                rotation_hz,
                truth,
                Placement::TrueEpoch,
                Shape::BandLimited,
                config.north_tick.expected_pulse_amplitude,
                0,
                0.0,
            );
            cells.push(match measure(&signal, &config, truth) {
                Some(v) => format!("{v:+.3}"),
                None => "none".into(),
            });
        }
        println!("{:<12} {:>14} {:>16}", buffer_size, cells[0], cells[1]);
    }
    println!();

    println!("\nresidual at trim 0, against doppler bandpass length");
    println!(
        "{:<10} {:>10} {:>12} {:>12}",
        "rate", "taps", "length (ms)", "error (deg)"
    );
    for (rate, taps) in [
        (48_000u32, 63usize),
        (48_000, 127),
        (48_000, 255),
        (96_000, 63),
        (96_000, 127),
        (96_000, 255),
        (96_000, 511),
    ] {
        let text = match probe_rate(rate, taps) {
            Some(v) => format!("{v:+.2}"),
            None => "none".into(),
        };
        println!(
            "{:<10} {:>10} {:>12.2} {:>12}",
            rate,
            taps,
            taps as f32 / rate as f32 * 1000.0,
            text
        );
    }
    println!();

    for (name, placement, shape, jitter, noise) in conditions {
        print!("{name:<20}");
        for trim in trims {
            let mut config = RdfConfig::default();
            config.audio.sample_rate = sample_rate;
            config.apply_rotation(RotationFrequency::from_hz(rotation_hz));
            config.doppler.method = BearingMethod::Correlation;
            config.doppler.north_tick_timing_adjustment_us = trim;

            let signal = build_signal(
                num_samples,
                sample_rate as f32,
                rotation_hz,
                truth,
                placement,
                shape,
                config.north_tick.expected_pulse_amplitude,
                jitter,
                noise,
            );
            match measure(&signal, &config, truth) {
                Some(e) => print!("{:>12}", format!("{e:+.2}")),
                None => print!("{:>12}", "none"),
            }
        }
        println!();
    }
}

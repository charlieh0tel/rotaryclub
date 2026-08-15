//! How short a transmission still yields a usable bearing?
//!
//! Defined in METRICS.md: the shortest burst duration T for which, over N
//! independent noise realisations, at least 90 percent yield a reported
//! bearing within E degrees of truth with a stated uncertainty at or below U.
//!
//! The burst sits inside a longer run of squelch-open receiver hiss, and the
//! north channel runs throughout, because it is generated locally and does not
//! stop when the transmitter does. So the loop is already locked when the
//! burst arrives and the floor is the Doppler path alone: bandpass settling
//! plus a filled work buffer.
//!
//! Two things keep the number honest.
//!
//! The score is one aggregate bearing over the burst, not the best bearing in
//! it. A burst yields many candidates and requiring merely that one be good is
//! nearly free -- a uniformly distributed bearing lands within 10 degrees of
//! truth about 5.6 percent of the time, so a long enough burst would pass on
//! chance alone.
//!
//! And every cell has a control: the identical criterion applied to the same
//! window with no burst in it, which is hiss throughout. A detection rate only
//! means something above its own false-alarm rate, and bearings computed on
//! hiss look exactly like bearings.

use rotaryclub::config::RdfConfig;
use rotaryclub::processing::RdfProcessor;
use rotaryclub::signal_processing::FirBandpass;
use rotaryclub::simulation::{SignalImpairment, generate_impaired_signal, noise_at};

const TRUTH_DEG: f32 = 200.0;
/// Bearing must land within this of truth.
const ERROR_LIMIT_DEG: f64 = 10.0;
/// And must say so: stated uncertainty at or below this.
const STATED_LIMIT_DEG: f64 = 10.0;
/// Detection rate the reported duration has to reach.
const REQUIRED_RATE: f64 = 0.90;
/// Independent noise realisations per cell.
const DRAWS: u64 = 48;
/// Lead-in before the burst, enough for the north loop to acquire from cold
/// with margin; in service it is already locked.
const LEAD_IN_SECS: f32 = 1.5;
const TAIL_SECS: f32 = 0.3;
/// RMS of the squelch-open hiss, in full-scale units.
///
/// Above the modulated signal rather than below it. Measured on the captures,
/// the doppler channel is 0.9 to 3.5 dB *louder* between overs than during
/// them, which is what an FM receiver with no carrier on it does.
const HISS_RMS: f32 = 0.9;

fn wrapped_diff(a: f64, b: f64) -> f64 {
    let d = (a - b).to_radians();
    d.sin().atan2(d.cos()).to_degrees()
}

/// Direction minimising the summed angular distance to the samples.
///
/// A median rather than a mean because a buffer straddling the start or end of
/// the burst is half hiss, and one such bearing should not drag the answer.
fn circular_median(angles: &[f64]) -> Option<f64> {
    let best = angles.iter().copied().min_by(|&a, &b| {
        let cost = |c: f64| -> f64 { angles.iter().map(|&x| wrapped_diff(x, c).abs()).sum() };
        cost(a).total_cmp(&cost(b))
    })?;
    Some(best)
}

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    Some(values[values.len() / 2])
}

/// Band-limited hiss, scaled to a target RMS.
fn hiss(len: usize, sample_rate: f32, seed: u64) -> Vec<f32> {
    let mut raw: Vec<f32> = (0..len).map(|i| noise_at(i, seed)).collect();
    if let Ok(mut band) = FirBandpass::new(300.0, 3400.0, sample_rate, 255, 150.0) {
        band.process_buffer(&mut raw);
    }
    let power = raw.iter().map(|s| (s * s) as f64).sum::<f64>() / len.max(1) as f64;
    if power > 0.0 {
        let scale = (HISS_RMS as f64 / power.sqrt()) as f32;
        for sample in raw.iter_mut() {
            *sample *= scale;
        }
    }
    raw
}

/// One trial. `burst_secs` of zero is the control: hiss throughout.
///
/// Returns whether the criterion was met.
fn trial(burst_secs: f32, noise: f32, buffer_size: usize, draw: u64) -> bool {
    let mut config = RdfConfig::default();
    config.audio.buffer_size = buffer_size;
    // Otherwise the first outputs of the burst carry pre-burst state and the
    // measurement is of the smoother's memory rather than of detection.
    config.bearing.smoothing_window = 1;
    let sample_rate = config.audio.sample_rate as f32;

    let total_secs = LEAD_IN_SECS + burst_secs + TAIL_SECS;
    let mut impairment = SignalImpairment::at_passband_ratio(noise);
    impairment.seed = impairment.seed.wrapping_add(draw);

    // Built full length so the north channel is continuous and the pulses land
    // where they belong; only the doppler channel is replaced outside the
    // burst.
    let mut signal = generate_impaired_signal(
        total_secs,
        config.audio.sample_rate,
        config.doppler.expected_freq,
        |_| TRUTH_DEG,
        impairment,
    );

    let frames = signal.len() / 2;
    let burst_start = (LEAD_IN_SECS * sample_rate) as usize;
    let burst_end = burst_start + (burst_secs * sample_rate) as usize;
    let noise_floor = hiss(frames, sample_rate, impairment.seed ^ 0x4849_5353);
    for frame in 0..frames {
        if frame < burst_start || frame >= burst_end {
            signal[frame * 2] = noise_floor[frame];
        }
    }

    let mut processor = RdfProcessor::new(&config, false, true).expect("processor");
    let mut bearings = Vec::new();
    let mut stated = Vec::new();
    for (chunk_index, chunk) in signal.chunks(buffer_size * 2).enumerate() {
        let results = processor.process_audio(chunk);
        // Everything this chunk reports describes the samples in it, so the
        // whole chunk is scored by where its midpoint falls.
        let midpoint = chunk_index * buffer_size + buffer_size / 2;
        let inside = midpoint >= burst_start && midpoint < burst_end.max(burst_start + 1);
        if !inside {
            continue;
        }
        for result in results {
            if let Some(bearing) = result.bearing {
                bearings.push(bearing.raw_bearing as f64);
                stated.push(
                    bearing
                        .metrics
                        .bearing_uncertainty_deg
                        .map_or(f64::INFINITY, |u| u as f64),
                );
            }
        }
    }

    let Some(reported) = circular_median(&bearings) else {
        return false;
    };
    let Some(per_buffer) = median(&mut stated) else {
        return false;
    };
    // The uncertainty of the aggregate, not of one buffer. The score is a
    // median over the burst's bearings, so the figure it must be judged
    // against is what that median claims -- which shrinks with the number of
    // looks behind it. Judging an aggregate against a single buffer's
    // uncertainty asks a long burst for a precision it never needed, and
    // makes the whole criterion insensitive to duration.
    //
    // Optimistic about the reference term, which is common to every look and
    // so does not average away. That part is not separately reported, and the
    // error is in the direction of crediting the system rather than the
    // reverse, so treat the resulting durations as a floor.
    let reported_stated = per_buffer / (bearings.len() as f64).sqrt();
    wrapped_diff(reported, TRUTH_DEG as f64).abs() <= ERROR_LIMIT_DEG
        && reported_stated <= STATED_LIMIT_DEG
}

fn snr_db(noise: f32) -> f64 {
    10.0 * (1.0 / noise as f64).log10()
}

fn main() {
    let jsonl = std::env::args().any(|a| a == "--jsonl");
    // Log-spaced, so the knee is resolved rather than interpolated between
    // two points an octave apart.
    let durations_ms = [
        20.0f32, 30.0, 45.0, 65.0, 95.0, 140.0, 200.0, 300.0, 440.0, 640.0, 940.0, 1400.0, 2000.0,
    ];
    let noises = [0.2f32, 0.8, 6.5];
    let buffers = [256usize, 1024];

    if !jsonl {
        println!(
            "bearing within {ERROR_LIMIT_DEG:.0} deg of truth and stating at most \
             {STATED_LIMIT_DEG:.0} deg,\n\
             over {DRAWS} draws. T90 is the shortest duration reaching \
             {:.0} percent.\n",
            REQUIRED_RATE * 100.0
        );
        print!("{:>7} {:>8}", "buffer", "snr dB");
        for ms in durations_ms {
            print!("{:>7}", format!("{ms:.0}"));
        }
        println!("{:>9} {:>9}", "T90", "control");
    }

    for &buffer_size in &buffers {
        for &noise in &noises {
            if !jsonl {
                print!("{buffer_size:>7} {:>8.0}", snr_db(noise));
            }
            let mut t90: Option<f32> = None;
            for ms in durations_ms {
                let hits = (0..DRAWS)
                    .filter(|&draw| trial(ms / 1000.0, noise, buffer_size, draw))
                    .count();
                let r = hits as f64 / DRAWS as f64;
                if t90.is_none() && r >= REQUIRED_RATE {
                    t90 = Some(ms);
                }
                if jsonl {
                    // Binomial standard error, so a curve drawn from this can
                    // carry the uncertainty it actually has.
                    let se = (r * (1.0 - r) / DRAWS as f64).sqrt();
                    println!(
                        "{}",
                        serde_json::json!({
                            "buffer_size": buffer_size,
                            "snr_db": snr_db(noise),
                            "passband_noise_to_tone": noise,
                            "duration_ms": ms,
                            "rate": r,
                            "rate_se": se,
                            "hits": hits,
                            "draws": DRAWS,
                            "error_limit_deg": ERROR_LIMIT_DEG,
                            "stated_limit_deg": STATED_LIMIT_DEG,
                        })
                    );
                } else {
                    print!("{r:>7.2}");
                }
            }
            let control_hits = (0..DRAWS)
                .filter(|&draw| trial(0.0, noise, buffer_size, draw))
                .count();
            let control = control_hits as f64 / DRAWS as f64;
            if jsonl {
                println!(
                    "{}",
                    serde_json::json!({
                        "buffer_size": buffer_size,
                        "snr_db": snr_db(noise),
                        "passband_noise_to_tone": noise,
                        "duration_ms": 0.0,
                        "rate": control,
                        "rate_se": (control * (1.0 - control) / DRAWS as f64).sqrt(),
                        "hits": control_hits,
                        "draws": DRAWS,
                        "control": true,
                    })
                );
            } else {
                match t90 {
                    Some(ms) => print!("{:>9}", format!("{ms:.0}ms")),
                    None => print!("{:>9}", ">2000ms"),
                }
                println!("{control:>9.2}");
            }
        }
    }
}

//! Which scatter should the stated uncertainty be calibrated against?
//!
//! The calibration tests measure the spread of reported bearings inside short
//! windows about the mean of that window, so that a target which moves during
//! a capture does not read as error. That estimator is blind to anything
//! shared across a window, and the reference-timing term is exactly that: an
//! error in the north epoch displaces a whole run of bearings together. So
//! the local figure measures the doppler term alone, while the stated figure
//! composes both, and the two cannot agree by construction.
//!
//! This prints both so the gap can be seen rather than argued about.

use rotaryclub::audio::{AudioSource, WavFileSource};
use rotaryclub::config::RdfConfig;
use rotaryclub::processing::RdfProcessor;
use rotaryclub::simulation::{SignalImpairment, generate_impaired_signal};

fn circular_mean(v: &[f64]) -> f64 {
    let (mut c, mut s) = (0.0, 0.0);
    for b in v {
        c += b.to_radians().cos();
        s += b.to_radians().sin();
    }
    s.atan2(c)
}

fn scatter_about(v: &[f64], mean: f64) -> f64 {
    let var = v
        .iter()
        .map(|b| {
            let d = (b.to_radians() - mean)
                .sin()
                .atan2((b.to_radians() - mean).cos());
            d * d
        })
        .sum::<f64>()
        / v.len() as f64;
    var.sqrt().to_degrees()
}

fn local_scatter(v: &[f64], window: usize) -> f64 {
    let mut acc = Vec::new();
    for chunk in v.chunks(window) {
        if chunk.len() < window {
            break;
        }
        acc.push(scatter_about(chunk, circular_mean(chunk)));
    }
    acc.iter().sum::<f64>() / acc.len().max(1) as f64
}

/// Ratio of stated to actual taken inside each window and then summarised,
/// rather than summarising each side over the whole run and dividing. The
/// two differ a great deal on real signal, where both quantities are strongly
/// skewed: a handful of bad stretches carry most of the scatter and most of
/// the stated uncertainty, so a median over all reports lands in the quiet
/// part of the run while a median over windows does not.
fn paired(raw: &[f64], stated: &[f64], window: usize) -> (f64, f64, f64) {
    let mut ratios = Vec::new();
    for (rc, sc) in raw.chunks(window).zip(stated.chunks(window)) {
        if rc.len() < window {
            break;
        }
        let actual = scatter_about(rc, circular_mean(rc));
        let mut claimed: Vec<f64> = sc.to_vec();
        claimed.sort_by(f64::total_cmp);
        ratios.push(claimed[claimed.len() / 2] / actual.max(1e-9));
    }
    ratios.sort_by(f64::total_cmp);
    (
        ratios[ratios.len() / 10],
        ratios[ratios.len() / 2],
        ratios[ratios.len() * 9 / 10],
    )
}

/// Fraction of a capture with no carrier on it, and the same statistics with
/// those stretches removed.
///
/// The ft-70d capture was made by keying up several times while walking
/// around the array, so between overs the receiver delivers full-scale hiss
/// with no tone in it. Every statistic taken over a whole capture includes
/// those stretches, and a "bearing" measured on receiver noise is a uniformly
/// distributed number. The calibration figure this project has been fitting
/// its generator to was measured that way.
fn report_gated(name: &str, raw: &[f64], stated: &[f64], snr: &[f64], floor_db: f64) {
    let kept: Vec<usize> = (0..raw.len()).filter(|&i| snr[i] >= floor_db).collect();
    let raw_kept: Vec<f64> = kept.iter().map(|&i| raw[i]).collect();
    let stated_kept: Vec<f64> = kept.iter().map(|&i| stated[i]).collect();
    let fraction = kept.len() as f64 / raw.len().max(1) as f64;
    let (_, p50, _) = paired(&raw_kept, &stated_kept, 512);
    println!(
        "{name:<34} {:>9.2} {:>10.3} {:>12.2}",
        floor_db, fraction, p50
    );
}

fn report(name: &str, raw: &[f64], stated: &[f64]) {
    let global = scatter_about(raw, circular_mean(raw));
    let mean_stated = stated.iter().sum::<f64>() / stated.len() as f64;
    let (p10, p50, p90) = paired(raw, stated, 512);
    println!(
        "{name:<34} {:>8.2} {:>8.2} {:>8.2} {:>7.2} {:>7.2} {:>7.2} {:>7.2}",
        local_scatter(raw, 512),
        global,
        mean_stated,
        mean_stated / global.max(1e-9),
        p10,
        p50,
        p90,
    );
}

fn main() {
    println!(
        "{:<34} {:>8} {:>8} {:>8} {:>7} {:>7} {:>7} {:>7}",
        "signal", "local512", "global", "stated", "r/glob", "p10", "p50", "p90"
    );

    let config = RdfConfig::default();
    for ratio in [0.2f32, 0.8, 6.5] {
        let signal = generate_impaired_signal(
            6.0,
            config.audio.sample_rate,
            config.doppler.expected_freq,
            |_| 200.0,
            SignalImpairment::at_passband_ratio(ratio),
        );
        let mut run = RdfConfig::default();
        run.bearing.smoothing_window = 1;
        let mut processor = RdfProcessor::new(&run, false, true).expect("processor");
        let (mut raw, mut stated) = (Vec::new(), Vec::new());
        for result in &processor.process_signal(&signal) {
            if let Some(b) = result.bearing
                && let Some(u) = b.metrics.bearing_uncertainty_deg
            {
                raw.push(b.raw_bearing as f64);
                stated.push(u as f64);
            }
        }
        // Truth is known and fixed here, so the scatter about it is the real
        // error, with no appeal to any local estimator at all.
        let about_truth = scatter_about(&raw, 200.0f64.to_radians());
        report(&format!("synthetic ratio {ratio}"), &raw, &stated);
        println!("{:<34} about truth {about_truth:>8.2}", "");
    }

    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir("data")
        .map(|e| {
            e.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "wav"))
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    for path in paths {
        let mut config = RdfConfig::default();
        config.bearing.smoothing_window = 1;
        let Ok(mut source) = WavFileSource::new(&path, config.audio.buffer_size) else {
            continue;
        };
        let mut processor = RdfProcessor::new(&config, false, true).expect("processor");
        let (mut raw, mut stated) = (Vec::new(), Vec::new());
        while let Ok(Some(buffer)) = AudioSource::next_buffer(&mut source) {
            for result in processor.process_audio(&buffer) {
                if let Some(b) = result.bearing
                    && let Some(u) = b.metrics.bearing_uncertainty_deg
                {
                    raw.push(b.raw_bearing as f64);
                    stated.push(u as f64);
                }
            }
        }
        if raw.len() < 2000 {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        report(&name[..name.len().min(33)], &raw, &stated);
    }

    println!(
        "\ncalibration with the no-carrier stretches removed\n{:<34} {:>9} {:>10} {:>12}",
        "capture", "floor dB", "kept", "ratio"
    );
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir("data")
        .map(|e| {
            e.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "wav"))
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    for path in paths {
        let mut config = RdfConfig::default();
        config.bearing.smoothing_window = 1;
        let Ok(mut source) = WavFileSource::new(&path, config.audio.buffer_size) else {
            continue;
        };
        let mut processor = RdfProcessor::new(&config, false, true).expect("processor");
        let (mut raw, mut stated, mut snr) = (Vec::new(), Vec::new(), Vec::new());
        while let Ok(Some(buffer)) = AudioSource::next_buffer(&mut source) {
            for result in processor.process_audio(&buffer) {
                if let Some(b) = result.bearing
                    && let Some(u) = b.metrics.bearing_uncertainty_deg
                {
                    raw.push(b.raw_bearing as f64);
                    stated.push(u as f64);
                    snr.push(b.metrics.snr_db as f64);
                }
            }
        }
        if raw.len() < 2000 {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        for floor in [-100.0, 0.0, 3.0, 6.0, 10.0] {
            report_gated(&name[..name.len().min(33)], &raw, &stated, &snr, floor);
        }
    }
}

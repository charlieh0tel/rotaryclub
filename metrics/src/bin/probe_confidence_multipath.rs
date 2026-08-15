//! Does the confidence figure earn its keep when a reflection is present?
//!
//! Every other impairment degrades a bearing's precision, and the stated
//! uncertainty is derived from the signal-to-noise ratio, so it sees them by
//! construction. Multipath is the one that does not work that way: the sum of
//! two paths points somewhere between them, so the bearing is wrong while the
//! tone is perfectly strong, and near a null the phase swings while the
//! in-band power barely moves. The calibration says as much -- with a
//! reflection the stated figure runs at 0.66 of the scatter, where without one
//! it runs at 1.09.
//!
//! That is a statement about scale. It does not say whether the figure still
//! *ranks* correctly, and ranking is what it is used for: the operational
//! question is whether discarding low-confidence bearings leaves better ones
//! behind. A figure that is uniformly too small is still useful if the worst
//! bearings are the ones it flags. A figure that flags the wrong ones is not,
//! however well calibrated its average is.
//!
//! So this reports, with and without a reflection:
//!
//!   kept              fraction surviving a confidence threshold
//!   median all        typical absolute error over every bearing
//!   median kept       typical absolute error over the survivors
//!   p90 all / kept    the tail, which is what a threshold is really for
//!   rank corr         Spearman correlation of stated uncertainty against
//!                     absolute error, over all bearings
//!
//! A useful figure makes "kept" better than "all" and has a positive rank
//! correlation. A blind one leaves them equal at any threshold.

use rotaryclub::config::RdfConfig;
use rotaryclub::processing::RdfProcessor;
use rotaryclub::simulation::{SignalImpairment, generate_impaired_signal};

const TRUTH_DEG: f32 = 200.0;
const CONFIDENCE_THRESHOLD: f32 = 0.5;

fn wrapped_error(measured: f64, truth: f64) -> f64 {
    let d = (measured - truth).to_radians();
    d.sin().atan2(d.cos()).to_degrees().abs()
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

/// Spearman correlation, so a monotone but non-linear relationship still
/// scores. The stated figure and the error are not expected to be
/// proportional -- only to move together.
fn rank_correlation(xs: &[f64], ys: &[f64]) -> f64 {
    let rank = |v: &[f64]| -> Vec<f64> {
        let mut order: Vec<usize> = (0..v.len()).collect();
        order.sort_by(|&a, &b| v[a].total_cmp(&v[b]));
        let mut ranks = vec![0.0; v.len()];
        for (position, &index) in order.iter().enumerate() {
            ranks[index] = position as f64;
        }
        ranks
    };
    let (rx, ry) = (rank(xs), rank(ys));
    let n = rx.len() as f64;
    let (mx, my) = (rx.iter().sum::<f64>() / n, ry.iter().sum::<f64>() / n);
    let mut cov = 0.0;
    let (mut vx, mut vy) = (0.0, 0.0);
    for i in 0..rx.len() {
        cov += (rx[i] - mx) * (ry[i] - my);
        vx += (rx[i] - mx) * (rx[i] - mx);
        vy += (ry[i] - my) * (ry[i] - my);
    }
    if vx <= 0.0 || vy <= 0.0 {
        return f64::NAN;
    }
    cov / (vx * vy).sqrt()
}

fn run(multipath: f32, noise: f32, draws: u64) -> (f64, f64, f64, f64, f64, f64) {
    let mut errors_all = Vec::new();
    let mut errors_kept = Vec::new();
    let mut stated_all = Vec::new();
    let mut kept = 0usize;
    let mut total = 0usize;

    for draw in 0..draws {
        let mut impairment = SignalImpairment::at_passband_ratio(noise);
        impairment.seed = impairment.seed.wrapping_add(draw);
        if multipath > 0.0 {
            let reference = SignalImpairment::multipath();
            impairment.multipath_ratio = multipath;
            impairment.multipath_bearing_offset_deg = reference.multipath_bearing_offset_deg;
            impairment.multipath_drift_hz = reference.multipath_drift_hz;
        }
        let signal = generate_impaired_signal(
            6.0,
            RdfConfig::default().audio.sample_rate,
            RdfConfig::default().doppler.expected_freq,
            |_| TRUTH_DEG,
            impairment,
        );

        let mut config = RdfConfig::default();
        config.bearing.smoothing_window = 1;
        let mut processor = RdfProcessor::new(&config, false, true).expect("processor");
        for result in &processor.process_signal(&signal) {
            let Some(bearing) = result.bearing else {
                continue;
            };
            let error = wrapped_error(bearing.raw_bearing as f64, TRUTH_DEG as f64);
            total += 1;
            errors_all.push(error);
            if let Some(stated) = bearing.metrics.bearing_uncertainty_deg {
                stated_all.push(stated as f64);
            } else {
                stated_all.push(f64::INFINITY);
            }
            if bearing.confidence >= CONFIDENCE_THRESHOLD {
                kept += 1;
                errors_kept.push(error);
            }
        }
    }

    let corr = rank_correlation(&stated_all, &errors_all);
    errors_all.sort_by(f64::total_cmp);
    errors_kept.sort_by(f64::total_cmp);
    (
        kept as f64 / total.max(1) as f64,
        percentile(&errors_all, 0.5),
        percentile(&errors_kept, 0.5),
        percentile(&errors_all, 0.9),
        percentile(&errors_kept, 0.9),
        corr,
    )
}

fn main() {
    println!(
        "confidence at or above {CONFIDENCE_THRESHOLD}, truth {TRUTH_DEG} degrees, 6 draws each\n"
    );
    println!(
        "{:<12} {:>7} {:>7} {:>11} {:>11} {:>9} {:>9} {:>10}",
        "multipath",
        "noise",
        "kept",
        "median all",
        "median kept",
        "p90 all",
        "p90 kept",
        "rank corr"
    );
    for &multipath in &[0.0f32, 0.45] {
        for &noise in &[0.2f32, 0.8] {
            let (kept, m_all, m_kept, p_all, p_kept, corr) = run(multipath, noise, 6);
            println!(
                "{multipath:<12.2} {noise:>7.1} {:>7.3} {m_all:>11.2} {m_kept:>11.2} \
                 {p_all:>9.2} {p_kept:>9.2} {corr:>10.3}",
                kept
            );
        }
    }
}

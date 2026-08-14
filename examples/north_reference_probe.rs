//! Why does the north tracker report a pulse late by 0.048 samples?
//!
//! For an impulse, the sub-sample estimator and the delay compensation are
//! computed the same way: an energy centroid over the same window, of the
//! same filter response. They should cancel exactly, and the reported tick
//! should be the input sample. They do not, by about a twentieth of a sample,
//! which is 0.57 degrees of bearing.
//!
//! This walks the pieces one at a time for a single impulse: where the
//! detector says the peak is, where the response's maximum tap actually is,
//! what the estimator measures around each, and what the compensation
//! expects.

use rotaryclub::config::RdfConfig;
use rotaryclub::rdf::{NorthReferenceTracker, NorthTracker};
use rotaryclub::signal_processing::{FirHighpass, PeakDetector};

/// The estimator's own weighting, replicated so the pieces can be compared
/// outside the tracker.
fn centroid(signal: &[f32], peak: usize, half_width: usize) -> f64 {
    let low = peak.saturating_sub(half_width);
    let high = (peak + half_width).min(signal.len() - 1);
    let mut weighted = 0.0f64;
    let mut total = 0.0f64;
    for (offset, sample) in signal[low..=high].iter().enumerate() {
        let value = sample.abs() as f64;
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

fn main() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let nt = &config.north_tick;

    let taps = {
        // Mirror the tracker's derivation of tap count from filter length.
        let t = (nt.fir_highpass_length_us * 1e-6 * sample_rate).round() as usize;
        if t.is_multiple_of(2) { t + 1 } else { t }
    };
    // The energy centroid's window, 85 us, as the tracker derives it.
    let half_width = ((85.0f32 * 1e-6 * sample_rate).round() as usize).max(1);

    let mut highpass = FirHighpass::new(
        nt.highpass_cutoff,
        sample_rate,
        taps,
        nt.highpass_transition_hz,
    )
    .expect("highpass");
    let group_delay = highpass.group_delay_samples();
    let peak_offset = highpass.peak_offset();
    let reference = highpass.centroid_offset(half_width, 2, false);

    println!("filter: {taps} taps, group delay {group_delay}, centroid window +/-{half_width}");
    println!("peak_offset (max tap from group delay): {peak_offset:+.4}");
    println!("centroid_offset, the compensation's reference: {reference:+.4}\n");

    // One impulse, far from either end.
    let position = 4000usize;
    let mut signal = vec![0.0f32; 12_000];
    signal[position] = nt.expected_pulse_amplitude;
    let mut filtered = signal.clone();
    highpass.process_buffer(&mut filtered);

    let argmax = filtered
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(i, _)| i)
        .unwrap();

    let min_interval = (nt.min_interval_ms / 1000.0 * sample_rate) as usize;
    // The same derivation the trackers use: a fraction of the pulse height as
    // it reaches the detector, which is after the filter.
    let threshold = nt.threshold_fraction * nt.expected_pulse_amplitude * highpass.peak_response();
    let search = {
        let crossing = highpass.threshold_crossing_offset(threshold, nt.expected_pulse_amplitude);
        ((highpass.peak_offset() - crossing).max(0.0)).ceil() as usize + 3
    };
    let mut detector = PeakDetector::with_peak_search_window(threshold, min_interval, search);
    detector.set_trailing_context(half_width);
    let detected = detector
        .find_all_peaks(&filtered)
        .first()
        .map(|(index, _)| *index)
        .expect("a detection");

    println!("impulse at {position}");
    println!(
        "  expected peak at {}",
        position + group_delay + peak_offset as usize
    );
    println!(
        "  filtered argmax at {argmax}, offset {:+}",
        argmax as isize - position as isize
    );
    println!(
        "  detector reports {detected}, offset {:+}",
        detected - position as isize
    );
    println!(
        "  detector agrees with argmax: {}",
        detected == argmax as isize
    );

    let at_detected = centroid(&filtered, detected as usize, half_width);
    let at_argmax = centroid(&filtered, argmax, half_width);
    println!("\n  centroid around detected: {at_detected:+.4}");
    println!("  centroid around argmax:   {at_argmax:+.4}");
    println!(
        "  compensation expects:     {:+.4}",
        reference - peak_offset
    );

    // What the whole tracker makes of the same pulse.
    let mut tracker = NorthReferenceTracker::new(nt, sample_rate).expect("tracker");
    let mut reported = Vec::new();
    for chunk in signal.chunks(512) {
        for tick in tracker.process_buffer(chunk) {
            reported.push(tick.sample_index as f64 + tick.fractional_sample_offset as f64);
        }
    }
    for tick in tracker.finish() {
        reported.push(tick.sample_index as f64 + tick.fractional_sample_offset as f64);
    }
    match reported.first() {
        Some(t) => println!(
            "\ntracker reports {t:.4}, error {:+.4} samples",
            t - position as f64
        ),
        None => println!("\ntracker reported nothing"),
    }

    // The loop's phase correction was written when a detection was a whole
    // sample index and the correction was the only sub-sample information
    // available. A fractional estimator supplies that directly, leaving the
    // correction nothing to fix -- so it contributes only the oscillator's own
    // residual phase wander. This is the cost of that, per estimator.
    println!("\ndpll timing against a pulse train, by estimator");
    println!("ten second run, mean tick error in samples by third");
    println!(
        "{:>18} {:>12} {:>12} {:>12}",
        "estimator", "first", "second", "third"
    );
    let period = sample_rate as f64 / config.doppler.expected_freq as f64;
    for estimator in [
        rotaryclub::config::NorthPulseEstimator::HardLimiter,
        rotaryclub::config::NorthPulseEstimator::AmplitudeCentroid,
        rotaryclub::config::NorthPulseEstimator::EnergyCentroid,
    ] {
        let mut cfg = RdfConfig::default();
        cfg.north_tick.estimator = estimator;

        let total = 480_000usize;
        let mut train = vec![0.0f32; total];
        let mut epochs = Vec::new();
        let mut k = 0i64;
        loop {
            let epoch = 200.0 + k as f64 * period;
            if epoch >= total as f64 - 40.0 {
                break;
            }
            epochs.push(epoch);
            let center = epoch.round() as i64;
            let half = 12i64;
            for n in (center - half)..=(center + half) {
                if n < 0 || n as usize >= total {
                    continue;
                }
                let x = n as f64 - epoch;
                let value = if x.abs() < f64::EPSILON {
                    1.0
                } else {
                    let px = std::f64::consts::PI * x;
                    let w = px / half as f64;
                    (px.sin() / px) * (w.sin() / w)
                };
                train[n as usize] += cfg.north_tick.expected_pulse_amplitude * value as f32;
            }
            k += 1;
        }

        let mut tracker =
            NorthReferenceTracker::new(&cfg.north_tick, sample_rate).expect("tracker");
        let mut ticks = Vec::new();
        for chunk in train.chunks(512) {
            for tick in tracker.process_buffer(chunk) {
                ticks.push(tick.sample_index as f64 + tick.fractional_sample_offset as f64);
            }
        }
        let mut errors = Vec::new();
        for tick in ticks.iter().skip(100) {
            let nearest = epochs
                .iter()
                .min_by(|a, b| (*a - tick).abs().total_cmp(&(*b - tick).abs()))
                .copied()
                .unwrap_or(*tick);
            if (tick - nearest).abs() < 3.0 {
                errors.push(tick - nearest);
            }
        }
        let third = errors.len() / 3;
        let seg = |s: &[f64]| s.iter().sum::<f64>() / s.len().max(1) as f64;
        println!(
            "{:>18} {:>12.4} {:>12.4} {:>12.4}",
            format!("{estimator:?}"),
            seg(&errors[..third]),
            seg(&errors[third..2 * third]),
            seg(&errors[2 * third..])
        );
    }
}

//! The north channel AGC: does it rescue a weak receiver without inventing
//! detections on a silent one?
//!
//! Pulse amplitude varies by nearly four across the two radios in `data/`,
//! 0.21 to 0.78, against a configured expectation of 0.8, and the doppler AGC
//! does not reach this channel. Below about 1.6 times the threshold the
//! detector falls off a cliff, so a quiet receiver is not a graceful
//! degradation, it is a loss of every other pulse and then all of them.

use rotaryclub::config::{NorthTrackingMode, RdfConfig};
use rotaryclub::rdf::{NorthReferenceTracker, NorthTracker};

const PULSE_HALF_WIDTH: i64 = 12;

/// Band-limited pulses at the true rotation epochs.
fn build(num_samples: usize, period: f64, amplitude: f32) -> (Vec<f32>, Vec<f64>) {
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
    (signal, epochs)
}

fn detection_rate(config: &RdfConfig, amplitude: f32) -> f64 {
    let sample_rate = config.audio.sample_rate as f32;
    let period = sample_rate as f64 / config.doppler.expected_freq as f64;
    let (signal, epochs) = build((sample_rate * 3.0) as usize, period, amplitude);

    let mut tracker =
        NorthReferenceTracker::new(&config.north_tick, sample_rate).expect("tracker config");
    let mut ticks = Vec::new();
    for chunk in signal.chunks(512) {
        for tick in tracker.process_buffer(chunk) {
            ticks.push(tick.sample_index as f64 + tick.fractional_sample_offset as f64);
        }
    }

    // Count epochs that got a tick, over the second half of the run so the
    // gain has settled.
    let start = epochs.len() / 2;
    let mut matched = 0usize;
    for epoch in &epochs[start..] {
        if ticks.iter().any(|t| (t - epoch).abs() < 3.0) {
            matched += 1;
        }
    }
    matched as f64 / (epochs.len() - start).max(1) as f64
}

#[test]
fn test_agc_rescues_a_weak_receiver() {
    let mut off = RdfConfig::default();
    off.north_tick.mode = NorthTrackingMode::Dpll;
    let mut on = off.clone();
    on.north_tick.agc.enabled = true;

    // Comfortably above the cliff either way.
    for amplitude in [0.8f32, 0.5] {
        assert!(
            detection_rate(&off, amplitude) > 0.98,
            "a healthy pulse of {amplitude} should detect without help"
        );
        assert!(
            detection_rate(&on, amplitude) > 0.98,
            "and must not be broken by turning the AGC on"
        );
    }

    // Below the cliff. The detection threshold is absolute, and the amplitude
    // at which detection collapses tracks it at about 1.6 times, so a receiver
    // delivering a fifth of the expected pulse is past it.
    let weak = 0.15f32;
    let without = detection_rate(&off, weak);
    let with = detection_rate(&on, weak);
    assert!(
        without < 0.5,
        "a pulse of {weak} should defeat the fixed-gain detector, got {without:.3}"
    );
    assert!(
        with > 0.95,
        "the AGC should recover it: {with:.3} against {without:.3} without"
    );
}

#[test]
fn test_agc_holds_gain_on_a_silent_channel() {
    let mut config = RdfConfig::default();
    config.north_tick.agc.enabled = true;
    let sample_rate = config.audio.sample_rate as f32;

    // Noise only, well below the threshold, and no pulses at all. A peak
    // tracker that adapts on anything will raise gain until this crosses the
    // threshold and then detect it.
    let mut signal = vec![0.0f32; (sample_rate * 3.0) as usize];
    for (i, sample) in signal.iter_mut().enumerate() {
        let mut x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xFEED;
        x ^= x >> 33;
        x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
        x ^= x >> 29;
        *sample = ((((x >> 32) as u32) as f32 / (u32::MAX as f32)) * 2.0 - 1.0) * 0.02;
    }

    let mut tracker =
        NorthReferenceTracker::new(&config.north_tick, sample_rate).expect("tracker config");
    let mut ticks = 0usize;
    for chunk in signal.chunks(512) {
        ticks += tracker.process_buffer(chunk).len();
    }

    assert_eq!(
        ticks, 0,
        "the AGC must not raise gain until the noise floor becomes detections"
    );
}

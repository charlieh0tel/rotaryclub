use rotaryclub::config::{AgcConfig, ConfidenceWeights, DopplerConfig};
use rotaryclub::rdf::{
    BearingCalculator, CorrelationBearingCalculator, NorthTick, ZeroCrossingBearingCalculator,
};
use std::f32::consts::PI;

fn make_north_tick(samples_per_rotation: f32) -> NorthTick {
    NorthTick {
        sample_index: 0,
        period: Some(samples_per_rotation),
        lock_quality: None,
        fractional_sample_offset: 0.0,
        phase: 0.0,
        frequency: 2.0 * PI / samples_per_rotation,
    }
}

fn make_signal(sample_rate: f32, rotation_freq: f32, bearing_degrees: f32, len: usize) -> Vec<f32> {
    let omega = 2.0 * PI * rotation_freq / sample_rate;
    let bearing_radians = bearing_degrees.to_radians();
    (0..len)
        .map(|i| (omega * i as f32 - bearing_radians).sin())
        .collect()
}

#[test]
fn test_correlation_returns_none_for_empty_buffer() {
    let sample_rate = 48_000.0;
    let doppler_config = DopplerConfig::default();
    let agc_config = AgcConfig::default();
    let mut calc = CorrelationBearingCalculator::new(
        &doppler_config,
        &agc_config,
        ConfidenceWeights::default(),
        sample_rate,
        1,
    )
    .unwrap();

    let tick = make_north_tick(sample_rate / doppler_config.expected_freq);
    let result = calc.process_buffer(&[], &tick);
    assert!(
        result.is_none(),
        "empty buffer should not produce a measurement"
    );
}

#[test]
fn test_correlation_returns_none_for_non_finite_frequency() {
    let sample_rate = 48_000.0;
    let rotation_freq = 1_602.0;
    let doppler_config = DopplerConfig {
        expected_freq: rotation_freq,
        bandpass_low: 1_500.0,
        bandpass_high: 1_700.0,
        ..Default::default()
    };
    let agc_config = AgcConfig::default();
    let mut calc = CorrelationBearingCalculator::new(
        &doppler_config,
        &agc_config,
        ConfidenceWeights::default(),
        sample_rate,
        1,
    )
    .unwrap();

    let signal = make_signal(sample_rate, rotation_freq, 45.0, 2048);
    let mut tick = make_north_tick(sample_rate / rotation_freq);
    tick.frequency = f32::NAN;
    let result = calc.process_buffer(&signal, &tick);
    assert!(
        result.is_none(),
        "non-finite north-tick frequency should not produce a measurement"
    );
}

#[test]
fn test_zero_crossing_returns_none_for_non_finite_period() {
    let sample_rate = 48_000.0;
    let rotation_freq = 1_602.0;
    let doppler_config = DopplerConfig {
        expected_freq: rotation_freq,
        bandpass_low: 1_500.0,
        bandpass_high: 1_700.0,
        ..Default::default()
    };
    let agc_config = AgcConfig::default();
    let mut calc = ZeroCrossingBearingCalculator::new(
        &doppler_config,
        &agc_config,
        ConfidenceWeights::default(),
        sample_rate,
        1,
    )
    .unwrap();

    let signal = make_signal(sample_rate, rotation_freq, 90.0, 2048);
    let mut tick = make_north_tick(sample_rate / rotation_freq);
    tick.period = Some(f32::NAN);
    let result = calc.process_buffer(&signal, &tick);
    assert!(
        result.is_none(),
        "non-finite north-tick period should not produce a measurement"
    );
}

#[test]
fn test_bearing_metrics_are_finite_and_bounded() {
    let sample_rate = 48_000.0;
    let rotation_freq = 1_602.0;
    let doppler_config = DopplerConfig {
        expected_freq: rotation_freq,
        bandpass_low: 1_500.0,
        bandpass_high: 1_700.0,
        ..Default::default()
    };
    let agc_config = AgcConfig::default();
    let signal = make_signal(sample_rate, rotation_freq, 135.0, 4096);
    let tick = make_north_tick(sample_rate / rotation_freq);

    let mut corr = CorrelationBearingCalculator::new(
        &doppler_config,
        &agc_config,
        ConfidenceWeights::default(),
        sample_rate,
        1,
    )
    .unwrap();
    let corr_m = corr
        .process_buffer(&signal, &tick)
        .expect("expected correlation measurement");
    assert!(corr_m.bearing_degrees.is_finite());
    assert!(corr_m.raw_bearing.is_finite());
    assert!(corr_m.metrics.snr_db.is_finite());
    assert!((0.0..=1.0).contains(&corr_m.metrics.coherence));
    assert!((0.0..=1.0).contains(&corr_m.metrics.signal_strength));
    assert!((0.0..=1.0).contains(&corr_m.confidence));

    let mut zc = ZeroCrossingBearingCalculator::new(
        &doppler_config,
        &agc_config,
        ConfidenceWeights::default(),
        sample_rate,
        1,
    )
    .unwrap();
    let zc_m = zc
        .process_buffer(&signal, &tick)
        .expect("expected zero-crossing measurement");
    assert!(zc_m.bearing_degrees.is_finite());
    assert!(zc_m.raw_bearing.is_finite());
    assert!(zc_m.metrics.snr_db.is_finite());
    assert!((0.0..=1.0).contains(&zc_m.metrics.coherence));
    assert!((0.0..=1.0).contains(&zc_m.metrics.signal_strength));
    assert!((0.0..=1.0).contains(&zc_m.confidence));
}

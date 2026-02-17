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

fn make_signal_aligned_to_tick(
    sample_rate: f32,
    rotation_freq: f32,
    bearing_degrees: f32,
    len: usize,
    start_sample: usize,
    tick_sample: usize,
) -> Vec<f32> {
    let omega = 2.0 * PI * rotation_freq / sample_rate;
    let bearing_radians = bearing_degrees.to_radians();
    (0..len)
        .map(|i| {
            let global = (start_sample + i) as f32;
            let rel = global - tick_sample as f32;
            (omega * rel - bearing_radians).sin()
        })
        .collect()
}

fn angular_error_deg(measured: f32, expected: f32) -> f32 {
    let mut err = (measured - expected).abs();
    if err > 180.0 {
        err = 360.0 - err;
    }
    err
}

#[derive(Clone, Copy)]
enum Method {
    Correlation,
    ZeroCrossing,
}

fn new_calculator(
    method: Method,
    doppler_config: &DopplerConfig,
    agc_config: &AgcConfig,
    sample_rate: f32,
) -> Box<dyn BearingCalculator> {
    match method {
        Method::Correlation => Box::new(
            CorrelationBearingCalculator::new(
                doppler_config,
                agc_config,
                ConfidenceWeights::default(),
                sample_rate,
                1,
            )
            .unwrap(),
        ),
        Method::ZeroCrossing => Box::new(
            ZeroCrossingBearingCalculator::new(
                doppler_config,
                agc_config,
                ConfidenceWeights::default(),
                sample_rate,
                1,
            )
            .unwrap(),
        ),
    }
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

#[test]
fn test_bearing_rotation_rate_mismatch_sweep() {
    let sample_rate = 48_000.0;
    let true_rotation_hz = 1_602.0;
    let expected_bearing = 62.0;
    let len = 4096;

    // Perturbation: rotation-rate mismatch between signal and north-tick model.
    let mismatches = [
        ("rotation_mismatch_-2.0pct", -0.02_f32),
        ("rotation_mismatch_-1.0pct", -0.01_f32),
        ("rotation_mismatch_+1.0pct", 0.01_f32),
        ("rotation_mismatch_+2.0pct", 0.02_f32),
    ];

    for method in [Method::Correlation, Method::ZeroCrossing] {
        let nominal_config = DopplerConfig {
            expected_freq: true_rotation_hz,
            bandpass_low: 1500.0,
            bandpass_high: 1700.0,
            ..Default::default()
        };
        let agc_config = AgcConfig::default();
        let mut nominal_calc = new_calculator(method, &nominal_config, &agc_config, sample_rate);
        let nominal_signal = make_signal(sample_rate, true_rotation_hz, expected_bearing, len);
        let nominal_tick = make_north_tick(sample_rate / true_rotation_hz);
        let nominal = nominal_calc
            .process_buffer(&nominal_signal, &nominal_tick)
            .expect("nominal case should yield a measurement");
        let nominal_err = angular_error_deg(nominal.raw_bearing, expected_bearing);

        for (name, frac) in mismatches {
            let model_rotation_hz = true_rotation_hz * (1.0 + frac);
            let doppler_config = DopplerConfig {
                expected_freq: model_rotation_hz,
                bandpass_low: 1500.0,
                bandpass_high: 1700.0,
                ..Default::default()
            };
            let mut calc = new_calculator(method, &doppler_config, &agc_config, sample_rate);
            let signal = make_signal(sample_rate, true_rotation_hz, expected_bearing, len);
            let tick = make_north_tick(sample_rate / model_rotation_hz);

            let m = calc
                .process_buffer(&signal, &tick)
                .expect("rotation mismatch should still yield a measurement");
            let err = angular_error_deg(m.raw_bearing, expected_bearing);

            let method_name = match method {
                Method::Correlation => "correlation",
                Method::ZeroCrossing => "zero_crossing",
            };
            assert!(
                (err - nominal_err).abs() <= 120.0,
                "perturbation={} method={} nominal_err={:.2} deg mismatch_err={:.2} deg",
                name,
                method_name,
                nominal_err,
                err
            );
            assert!(
                m.raw_bearing.is_finite() && m.bearing_degrees.is_finite() && m.confidence.is_finite(),
                "perturbation={} method={} should keep finite outputs",
                name,
                method_name
            );
        }
    }
}

#[test]
fn test_bearing_buffer_boundary_phase_jump_cases() {
    let sample_rate = 48_000.0;
    let rotation_hz = 1_602.0;
    let expected_bearing = 137.0;
    let samples_per_rotation = sample_rate / rotation_hz;
    let chunk_size = 256usize;
    let start_sample = 8 * chunk_size;

    // Perturbation: window-boundary phase jumps by placing north tick near buffer edges.
    let boundary_cases = [
        ("tick_at_center", chunk_size / 2),
        ("tick_at_start_edge", 0usize),
        ("tick_at_end_edge", chunk_size - 1),
    ];

    for method in [Method::Correlation, Method::ZeroCrossing] {
        let doppler_config = DopplerConfig {
            expected_freq: rotation_hz,
            bandpass_low: 1500.0,
            bandpass_high: 1700.0,
            ..Default::default()
        };
        let agc_config = AgcConfig::default();

        let mut errs: Vec<(&str, f32)> = Vec::new();
        for (case_name, offset) in boundary_cases {
            let mut calc = new_calculator(method, &doppler_config, &agc_config, sample_rate);
            let tick_sample = start_sample + offset;
            let signal = make_signal_aligned_to_tick(
                sample_rate,
                rotation_hz,
                expected_bearing,
                chunk_size,
                start_sample,
                tick_sample,
            );
            let tick = NorthTick {
                sample_index: tick_sample,
                period: Some(samples_per_rotation),
                lock_quality: None,
                fractional_sample_offset: 0.0,
                phase: 0.0,
                frequency: 2.0 * PI / samples_per_rotation,
            };
            let m = calc
                .process_buffer(&signal, &tick)
                .expect("boundary-phase-jump case should produce a measurement");
            let err = angular_error_deg(m.raw_bearing, expected_bearing);
            errs.push((case_name, err));

            let method_name = match method {
                Method::Correlation => "correlation",
                Method::ZeroCrossing => "zero_crossing",
            };
            assert!(
                m.raw_bearing.is_finite() && m.bearing_degrees.is_finite() && m.confidence.is_finite(),
                "perturbation={} method={} should keep finite outputs",
                case_name,
                method_name,
            );
        }

        let center = errs
            .iter()
            .find(|(name, _)| *name == "tick_at_center")
            .expect("center case exists")
            .1;
        for (name, err) in errs {
            let delta = (err - center).abs();
            let method_name = match method {
                Method::Correlation => "correlation",
                Method::ZeroCrossing => "zero_crossing",
            };
            assert!(
                delta <= 25.0,
                "perturbation={} method={} center_err={:.2} edge_err={:.2} delta={:.2}",
                name,
                method_name,
                center,
                err,
                delta
            );
        }
    }
}

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

fn make_signal_with_am_and_fade(
    sample_rate: f32,
    rotation_freq: f32,
    bearing_degrees: f32,
    len: usize,
    am_depth: f32,
    am_rate_hz: f32,
    fade_start_frac: f32,
    fade_width_frac: f32,
    fade_gain: f32,
) -> Vec<f32> {
    let omega = 2.0 * PI * rotation_freq / sample_rate;
    let bearing_radians = bearing_degrees.to_radians();
    let fade_start = (len as f32 * fade_start_frac.clamp(0.0, 1.0)) as usize;
    let fade_width = (len as f32 * fade_width_frac.clamp(0.0, 1.0)).max(1.0) as usize;
    let fade_end = (fade_start + fade_width).min(len);
    let two_pi = 2.0 * PI;

    (0..len)
        .map(|i| {
            let carrier = (omega * i as f32 - bearing_radians).sin();
            let t = i as f32 / sample_rate;
            let env = 1.0 + am_depth.clamp(0.0, 0.95) * (two_pi * am_rate_hz * t).sin();
            let fade = if i >= fade_start && i < fade_end {
                fade_gain.clamp(0.0, 1.0)
            } else {
                1.0
            };
            carrier * env * fade
        })
        .collect()
}

fn make_signal_with_harmonics(
    sample_rate: f32,
    rotation_freq: f32,
    bearing_degrees: f32,
    len: usize,
    second_ratio: f32,
    third_ratio: f32,
) -> Vec<f32> {
    let omega = 2.0 * PI * rotation_freq / sample_rate;
    let bearing_radians = bearing_degrees.to_radians();
    (0..len)
        .map(|i| {
            let p = omega * i as f32 - bearing_radians;
            let fundamental = p.sin();
            let second = (2.0 * p).sin();
            let third = (3.0 * p).sin();
            fundamental + second_ratio * second + third_ratio * third
        })
        .collect()
}

fn make_signal_with_channel_imbalance(
    sample_rate: f32,
    rotation_freq: f32,
    bearing_degrees: f32,
    len: usize,
    gain_imbalance: f32,
    phase_imbalance_deg: f32,
) -> Vec<f32> {
    let omega = 2.0 * PI * rotation_freq / sample_rate;
    let bearing_radians = bearing_degrees.to_radians();
    let phase_imbalance = phase_imbalance_deg.to_radians();
    let g = gain_imbalance.clamp(-0.9, 0.9);

    (0..len)
        .map(|i| {
            let p = omega * i as f32 - bearing_radians;
            // Proxy perturbation for channel gain/phase imbalance:
            // scale in-phase component and leak a phase-skewed quadrature component.
            (1.0 + g) * p.sin() + g * (p + phase_imbalance).cos()
        })
        .collect()
}

fn make_signal_with_impulsive_burst(
    sample_rate: f32,
    rotation_freq: f32,
    bearing_degrees: f32,
    len: usize,
    burst_start_frac: f32,
    burst_width_frac: f32,
    burst_amplitude: f32,
) -> Vec<f32> {
    let mut signal = make_signal(sample_rate, rotation_freq, bearing_degrees, len);
    let start = (len as f32 * burst_start_frac.clamp(0.0, 1.0)) as usize;
    let width = (len as f32 * burst_width_frac.clamp(0.0, 1.0)).max(1.0) as usize;
    let end = (start + width).min(len);
    for x in signal.iter_mut().take(end).skip(start) {
        *x += burst_amplitude;
    }
    signal
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

#[derive(Clone, Copy)]
struct RotationMismatchCase {
    mismatch_fraction: f32,
}

impl RotationMismatchCase {
    fn label(self) -> String {
        format!(
            "rotation_mismatch_{:+.1}pct",
            self.mismatch_fraction * 100.0
        )
    }
}

#[derive(Clone, Copy)]
struct BoundaryPhaseCase {
    label: &'static str,
    offset_samples: usize,
}

#[derive(Clone, Copy)]
struct AmFadeCase {
    am_depth: f32,
    fade_start_frac: f32,
    fade_width_frac: f32,
    fade_gain: f32,
}

impl AmFadeCase {
    fn label(self) -> String {
        if self.fade_width_frac <= 0.0 {
            format!("am_depth_{:.1}_no_fade", self.am_depth)
        } else {
            format!(
                "am_depth_{:.1}_short_fade_{:.1}pct_gain_{:.1}_at_{:.0}pct",
                self.am_depth,
                self.fade_width_frac * 100.0,
                self.fade_gain,
                self.fade_start_frac * 100.0
            )
        }
    }
}

#[derive(Clone, Copy)]
struct HarmonicCase {
    second_ratio: f32,
    third_ratio: f32,
}

impl HarmonicCase {
    fn label(self) -> String {
        format!(
            "harmonic_2f_{:.2}_3f_{:.2}",
            self.second_ratio, self.third_ratio
        )
    }
}

#[derive(Clone, Copy)]
struct ChannelImbalanceCase {
    gain_imbalance: f32,
    phase_imbalance_deg: f32,
}

impl ChannelImbalanceCase {
    fn label(self) -> String {
        format!(
            "channel_imbalance_gain_{:+.2}_phase_{:+.0}deg",
            self.gain_imbalance, self.phase_imbalance_deg
        )
    }
}

#[derive(Clone, Copy)]
struct ImpulseCase {
    burst_start_frac: f32,
    burst_width_frac: f32,
    burst_amplitude: f32,
}

impl ImpulseCase {
    fn label(self) -> String {
        if self.burst_amplitude == 0.0 {
            "impulse_none_reference".to_string()
        } else {
            format!(
                "impulse_{:.0}pct_width_{:.1}pct_amp_{:.1}",
                self.burst_start_frac * 100.0,
                self.burst_width_frac * 100.0,
                self.burst_amplitude
            )
        }
    }
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
        RotationMismatchCase {
            mismatch_fraction: -0.02,
        },
        RotationMismatchCase {
            mismatch_fraction: -0.01,
        },
        RotationMismatchCase {
            mismatch_fraction: 0.01,
        },
        RotationMismatchCase {
            mismatch_fraction: 0.02,
        },
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

        for case in mismatches {
            let model_rotation_hz = true_rotation_hz * (1.0 + case.mismatch_fraction);
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
            let label = case.label();
            assert!(
                (err - nominal_err).abs() <= 120.0,
                "perturbation={} method={} nominal_err={:.2} deg mismatch_err={:.2} deg",
                label,
                method_name,
                nominal_err,
                err
            );
            assert!(
                m.raw_bearing.is_finite() && m.bearing_degrees.is_finite() && m.confidence.is_finite(),
                "perturbation={} method={} should keep finite outputs",
                label,
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
        BoundaryPhaseCase {
            label: "tick_at_center",
            offset_samples: chunk_size / 2,
        },
        BoundaryPhaseCase {
            label: "tick_at_start_edge",
            offset_samples: 0,
        },
        BoundaryPhaseCase {
            label: "tick_at_end_edge",
            offset_samples: chunk_size - 1,
        },
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
        for case in boundary_cases {
            let mut calc = new_calculator(method, &doppler_config, &agc_config, sample_rate);
            let tick_sample = start_sample + case.offset_samples;
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
            errs.push((case.label, err));

            let method_name = match method {
                Method::Correlation => "correlation",
                Method::ZeroCrossing => "zero_crossing",
            };
            assert!(
                m.raw_bearing.is_finite() && m.bearing_degrees.is_finite() && m.confidence.is_finite(),
                "perturbation={} method={} should keep finite outputs",
                case.label,
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

#[test]
fn test_bearing_am_depth_and_brief_fade_sweep() {
    let sample_rate = 48_000.0;
    let rotation_hz = 1_602.0;
    let expected_bearing = 218.0;
    let len = 4096usize;
    let samples_per_rotation = sample_rate / rotation_hz;

    // Perturbation: AM depth sweep and brief fades.
    let cases = [
        AmFadeCase {
            am_depth: 0.2,
            fade_start_frac: 0.0,
            fade_width_frac: 0.0,
            fade_gain: 1.0,
        },
        AmFadeCase {
            am_depth: 0.5,
            fade_start_frac: 0.0,
            fade_width_frac: 0.0,
            fade_gain: 1.0,
        },
        AmFadeCase {
            am_depth: 0.8,
            fade_start_frac: 0.0,
            fade_width_frac: 0.0,
            fade_gain: 1.0,
        },
        AmFadeCase {
            am_depth: 0.5,
            fade_start_frac: 0.20,
            fade_width_frac: 0.05,
            fade_gain: 0.2,
        },
        AmFadeCase {
            am_depth: 0.8,
            fade_start_frac: 0.70,
            fade_width_frac: 0.05,
            fade_gain: 0.0,
        },
    ];

    for method in [Method::Correlation, Method::ZeroCrossing] {
        let doppler_config = DopplerConfig {
            expected_freq: rotation_hz,
            bandpass_low: 1500.0,
            bandpass_high: 1700.0,
            ..Default::default()
        };
        let agc_config = AgcConfig::default();
        let mut baseline_calc = new_calculator(method, &doppler_config, &agc_config, sample_rate);
        let baseline_signal = make_signal(sample_rate, rotation_hz, expected_bearing, len);
        let tick = make_north_tick(samples_per_rotation);
        let baseline = baseline_calc
            .process_buffer(&baseline_signal, &tick)
            .expect("baseline AM/fade reference should produce measurement");
        let baseline_err = angular_error_deg(baseline.raw_bearing, expected_bearing);

        for case in cases {
            let mut calc = new_calculator(method, &doppler_config, &agc_config, sample_rate);
            let signal = make_signal_with_am_and_fade(
                sample_rate,
                rotation_hz,
                expected_bearing,
                len,
                case.am_depth,
                8.0,
                case.fade_start_frac,
                case.fade_width_frac,
                case.fade_gain,
            );

            let m = calc
                .process_buffer(&signal, &tick)
                .expect("AM/fade perturbation should still produce a measurement");
            let err = angular_error_deg(m.raw_bearing, expected_bearing);
            let method_name = match method {
                Method::Correlation => "correlation",
                Method::ZeroCrossing => "zero_crossing",
            };
            let label = case.label();

            assert!(
                m.raw_bearing.is_finite() && m.bearing_degrees.is_finite() && m.confidence.is_finite(),
                "perturbation={} method={} should keep finite outputs",
                label,
                method_name
            );
            assert!(
                (err - baseline_err).abs() <= 120.0,
                "perturbation={} method={} baseline_err={:.2} deg perturb_err={:.2} deg",
                label,
                method_name,
                baseline_err,
                err
            );
        }
    }
}

#[test]
fn test_bearing_harmonic_contamination_sweep() {
    let sample_rate = 48_000.0;
    let rotation_hz = 1_602.0;
    let expected_bearing = 301.0;
    let len = 4096usize;
    let samples_per_rotation = sample_rate / rotation_hz;

    // Perturbation: harmonic contamination (2f/3f leakage).
    let cases = [
        HarmonicCase {
            second_ratio: 0.0,
            third_ratio: 0.0,
        },
        HarmonicCase {
            second_ratio: 0.10,
            third_ratio: 0.00,
        },
        HarmonicCase {
            second_ratio: 0.20,
            third_ratio: 0.00,
        },
        HarmonicCase {
            second_ratio: 0.00,
            third_ratio: 0.10,
        },
        HarmonicCase {
            second_ratio: 0.00,
            third_ratio: 0.20,
        },
        HarmonicCase {
            second_ratio: 0.15,
            third_ratio: 0.10,
        },
        HarmonicCase {
            second_ratio: 0.25,
            third_ratio: 0.15,
        },
    ];

    for method in [Method::Correlation, Method::ZeroCrossing] {
        let doppler_config = DopplerConfig {
            expected_freq: rotation_hz,
            bandpass_low: 1500.0,
            bandpass_high: 1700.0,
            ..Default::default()
        };
        let agc_config = AgcConfig::default();
        let tick = make_north_tick(samples_per_rotation);

        let mut ref_calc = new_calculator(method, &doppler_config, &agc_config, sample_rate);
        let ref_signal = make_signal_with_harmonics(
            sample_rate,
            rotation_hz,
            expected_bearing,
            len,
            0.0,
            0.0,
        );
        let reference = ref_calc
            .process_buffer(&ref_signal, &tick)
            .expect("harmonic reference should produce measurement");
        let reference_err = angular_error_deg(reference.raw_bearing, expected_bearing);

        for case in cases {
            let mut calc = new_calculator(method, &doppler_config, &agc_config, sample_rate);
            let signal = make_signal_with_harmonics(
                sample_rate,
                rotation_hz,
                expected_bearing,
                len,
                case.second_ratio,
                case.third_ratio,
            );
            let m = calc
                .process_buffer(&signal, &tick)
                .expect("harmonic perturbation should still produce a measurement");
            let err = angular_error_deg(m.raw_bearing, expected_bearing);
            let method_name = match method {
                Method::Correlation => "correlation",
                Method::ZeroCrossing => "zero_crossing",
            };
            let label = case.label();

            assert!(
                m.raw_bearing.is_finite() && m.bearing_degrees.is_finite() && m.confidence.is_finite(),
                "perturbation={} method={} should keep finite outputs",
                label,
                method_name
            );
            assert!(
                (err - reference_err).abs() <= 120.0,
                "perturbation={} method={} ref_err={:.2} deg perturb_err={:.2} deg",
                label,
                method_name,
                reference_err,
                err
            );
        }
    }
}

#[test]
fn test_bearing_channel_gain_phase_imbalance_sweep() {
    let sample_rate = 48_000.0;
    let rotation_hz = 1_602.0;
    let expected_bearing = 25.0;
    let len = 4096usize;
    let samples_per_rotation = sample_rate / rotation_hz;

    // Perturbation: channel gain/phase imbalance proxy.
    let cases = [
        ChannelImbalanceCase {
            gain_imbalance: 0.0,
            phase_imbalance_deg: 0.0,
        },
        ChannelImbalanceCase {
            gain_imbalance: 0.05,
            phase_imbalance_deg: 5.0,
        },
        ChannelImbalanceCase {
            gain_imbalance: -0.05,
            phase_imbalance_deg: -5.0,
        },
        ChannelImbalanceCase {
            gain_imbalance: 0.10,
            phase_imbalance_deg: 10.0,
        },
        ChannelImbalanceCase {
            gain_imbalance: -0.10,
            phase_imbalance_deg: -10.0,
        },
        ChannelImbalanceCase {
            gain_imbalance: 0.15,
            phase_imbalance_deg: 15.0,
        },
    ];

    for method in [Method::Correlation, Method::ZeroCrossing] {
        let doppler_config = DopplerConfig {
            expected_freq: rotation_hz,
            bandpass_low: 1500.0,
            bandpass_high: 1700.0,
            ..Default::default()
        };
        let agc_config = AgcConfig::default();
        let tick = make_north_tick(samples_per_rotation);

        let mut ref_calc = new_calculator(method, &doppler_config, &agc_config, sample_rate);
        let ref_signal = make_signal_with_channel_imbalance(
            sample_rate,
            rotation_hz,
            expected_bearing,
            len,
            0.0,
            0.0,
        );
        let reference = ref_calc
            .process_buffer(&ref_signal, &tick)
            .expect("channel-imbalance reference should produce measurement");
        let reference_err = angular_error_deg(reference.raw_bearing, expected_bearing);

        for case in cases {
            let mut calc = new_calculator(method, &doppler_config, &agc_config, sample_rate);
            let signal = make_signal_with_channel_imbalance(
                sample_rate,
                rotation_hz,
                expected_bearing,
                len,
                case.gain_imbalance,
                case.phase_imbalance_deg,
            );
            let m = calc
                .process_buffer(&signal, &tick)
                .expect("channel imbalance perturbation should still produce a measurement");
            let err = angular_error_deg(m.raw_bearing, expected_bearing);
            let method_name = match method {
                Method::Correlation => "correlation",
                Method::ZeroCrossing => "zero_crossing",
            };
            let label = case.label();

            assert!(
                m.raw_bearing.is_finite() && m.bearing_degrees.is_finite() && m.confidence.is_finite(),
                "perturbation={} method={} should keep finite outputs",
                label,
                method_name
            );
            assert!(
                (err - reference_err).abs() <= 120.0,
                "perturbation={} method={} ref_err={:.2} deg perturb_err={:.2} deg",
                label,
                method_name,
                reference_err,
                err
            );
        }
    }
}

#[test]
fn test_bearing_impulsive_burst_offset_sweep() {
    let sample_rate = 48_000.0;
    let rotation_hz = 1_602.0;
    let expected_bearing = 173.0;
    let len = 4096usize;
    let samples_per_rotation = sample_rate / rotation_hz;

    // Perturbation: short impulsive burst at varying offsets.
    let cases = [
        ImpulseCase {
            burst_start_frac: 0.0,
            burst_width_frac: 0.0,
            burst_amplitude: 0.0,
        },
        ImpulseCase {
            burst_start_frac: 0.05,
            burst_width_frac: 0.005,
            burst_amplitude: 2.0,
        },
        ImpulseCase {
            burst_start_frac: 0.50,
            burst_width_frac: 0.005,
            burst_amplitude: 2.0,
        },
        ImpulseCase {
            burst_start_frac: 0.90,
            burst_width_frac: 0.005,
            burst_amplitude: 2.0,
        },
        ImpulseCase {
            burst_start_frac: 0.10,
            burst_width_frac: 0.010,
            burst_amplitude: 3.0,
        },
        ImpulseCase {
            burst_start_frac: 0.60,
            burst_width_frac: 0.010,
            burst_amplitude: 3.0,
        },
    ];

    for method in [Method::Correlation, Method::ZeroCrossing] {
        let doppler_config = DopplerConfig {
            expected_freq: rotation_hz,
            bandpass_low: 1500.0,
            bandpass_high: 1700.0,
            ..Default::default()
        };
        let agc_config = AgcConfig::default();
        let tick = make_north_tick(samples_per_rotation);

        let mut ref_calc = new_calculator(method, &doppler_config, &agc_config, sample_rate);
        let ref_signal = make_signal(sample_rate, rotation_hz, expected_bearing, len);
        let reference = ref_calc
            .process_buffer(&ref_signal, &tick)
            .expect("impulse reference should produce measurement");
        let reference_err = angular_error_deg(reference.raw_bearing, expected_bearing);

        for case in cases {
            let mut calc = new_calculator(method, &doppler_config, &agc_config, sample_rate);
            let signal = make_signal_with_impulsive_burst(
                sample_rate,
                rotation_hz,
                expected_bearing,
                len,
                case.burst_start_frac,
                case.burst_width_frac,
                case.burst_amplitude,
            );
            let m = calc
                .process_buffer(&signal, &tick)
                .expect("impulsive perturbation should still produce a measurement");
            let err = angular_error_deg(m.raw_bearing, expected_bearing);
            let method_name = match method {
                Method::Correlation => "correlation",
                Method::ZeroCrossing => "zero_crossing",
            };
            let label = case.label();

            assert!(
                m.raw_bearing.is_finite() && m.bearing_degrees.is_finite() && m.confidence.is_finite(),
                "perturbation={} method={} should keep finite outputs",
                label,
                method_name
            );
            assert!(
                (err - reference_err).abs() <= 120.0,
                "perturbation={} method={} ref_err={:.2} deg perturb_err={:.2} deg",
                label,
                method_name,
                reference_err,
                err
            );
        }
    }
}

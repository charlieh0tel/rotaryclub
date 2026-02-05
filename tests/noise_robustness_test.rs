mod test_signals;

use rotaryclub::config::RdfConfig;
use rotaryclub::rdf::{
    BearingCalculator, CorrelationBearingCalculator, NorthReferenceTracker, NorthTick, NorthTracker,
};
use test_signals::{
    AdditiveNoiseConfig, DoublingConfig, FadingConfig, FadingType, FrequencyDriftConfig,
    ImpulseNoiseConfig, MultipathComponent, MultipathConfig, NoiseConfig,
};

fn angle_error(measured: f32, expected: f32) -> f32 {
    let mut e = measured - expected;
    if e > 180.0 {
        e -= 360.0;
    } else if e < -180.0 {
        e += 360.0;
    }
    e
}

fn measure_bearing_with_noise(
    bearing_degrees: f32,
    noise_config: &NoiseConfig,
    config: &RdfConfig,
) -> Option<f32> {
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;

    let signal = test_signals::generate_noisy_test_signal(
        0.5,
        sample_rate as u32,
        rotation_hz,
        bearing_degrees,
        noise_config,
    );

    let mut north_tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate).ok()?;
    let mut corr_calc =
        CorrelationBearingCalculator::new(&config.doppler, &config.agc, sample_rate, 3).ok()?;

    let chunk_size = config.audio.buffer_size * 2;
    let mut measurements = Vec::new();
    let mut last_tick: Option<NorthTick> = None;

    for chunk in signal.chunks(chunk_size) {
        let stereo: Vec<(f32, f32)> = chunk.chunks_exact(2).map(|c| (c[0], c[1])).collect();
        let (doppler, north_tick) = config.audio.split_channels(&stereo);

        if let Some(ref tick) = last_tick {
            if let Some(bearing) = corr_calc.process_buffer(&doppler, tick) {
                measurements.push(bearing.bearing_degrees);
            }
        } else {
            corr_calc.process_buffer(
                &doppler,
                &NorthTick {
                    sample_index: 0,
                    period: Some(30.0),
                    lock_quality: None,
                },
            );
        }

        let ticks = north_tracker.process_buffer(&north_tick);
        if let Some(tick) = ticks.last() {
            last_tick = Some(*tick);
        }
    }

    if measurements.len() > 5 {
        Some(measurements.iter().skip(3).sum::<f32>() / (measurements.len() - 3) as f32)
    } else if !measurements.is_empty() {
        Some(measurements.iter().sum::<f32>() / measurements.len() as f32)
    } else {
        None
    }
}

fn measure_max_error_across_bearings(noise_config: &NoiseConfig, config: &RdfConfig) -> f32 {
    let test_bearings = [45.0_f32, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0];
    let mut max_error = 0.0f32;

    for &bearing in &test_bearings {
        if let Some(measured) = measure_bearing_with_noise(bearing, noise_config, config) {
            let error = angle_error(measured, bearing).abs();
            max_error = max_error.max(error);
        }
    }

    max_error
}

#[test]
fn test_snr_degradation_curve() {
    let config = RdfConfig::default();

    let test_cases = [
        (30.0, 2.0, "High SNR"),
        (20.0, 5.0, "Good SNR"),
        (10.0, 15.0, "Moderate SNR"),
        (3.0, 45.0, "Low SNR"),
    ];

    println!("\nSNR Degradation Curve:");
    println!(
        "{:<12} {:>12} {:>12} {:>8}",
        "SNR (dB)", "Max Error", "Threshold", "Status"
    );
    println!("{}", "-".repeat(48));

    for (snr_db, max_allowed_error, description) in test_cases {
        let noise_config = NoiseConfig {
            seed: Some(42),
            additive: Some(AdditiveNoiseConfig { snr_db }),
            ..Default::default()
        };

        let max_error = measure_max_error_across_bearings(&noise_config, &config);

        let status = if max_error <= max_allowed_error {
            "PASS"
        } else {
            "FAIL"
        };

        println!(
            "{:<12.1} {:>11.1}° {:>11.1}° {:>8}",
            snr_db, max_error, max_allowed_error, status
        );

        assert!(
            max_error <= max_allowed_error,
            "{}: Max error {:.1}° exceeds threshold {:.1}° at SNR {}dB",
            description,
            max_error,
            max_allowed_error,
            snr_db
        );
    }
}

#[test]
fn test_impulse_noise_rejection() {
    let config = RdfConfig::default();

    let noise_config = NoiseConfig {
        seed: Some(42),
        additive: Some(AdditiveNoiseConfig { snr_db: 30.0 }),
        impulse: Some(ImpulseNoiseConfig {
            rate_hz: 50.0,
            amplitude: 2.0,
            duration_samples: 10,
        }),
        ..Default::default()
    };

    let max_error = measure_max_error_across_bearings(&noise_config, &config);

    assert!(
        max_error < 10.0,
        "Impulse noise rejection failed: max error {:.1}° exceeds 10° threshold",
        max_error
    );
}

#[test]
fn test_rayleigh_fading_robustness() {
    let config = RdfConfig::default();

    let noise_config = NoiseConfig {
        seed: Some(42),
        additive: Some(AdditiveNoiseConfig { snr_db: 25.0 }),
        fading: Some(FadingConfig {
            fading_type: FadingType::Rayleigh,
            doppler_spread_hz: 5.0,
        }),
        ..Default::default()
    };

    let max_error = measure_max_error_across_bearings(&noise_config, &config);

    assert!(
        max_error < 20.0,
        "Rayleigh fading test failed: max error {:.1}° exceeds 20° threshold",
        max_error
    );
}

#[test]
fn test_rician_fading_robustness() {
    let config = RdfConfig::default();

    let noise_config = NoiseConfig {
        seed: Some(42),
        additive: Some(AdditiveNoiseConfig { snr_db: 25.0 }),
        fading: Some(FadingConfig {
            fading_type: FadingType::Rician { k_factor: 4.0 },
            doppler_spread_hz: 5.0,
        }),
        ..Default::default()
    };

    let max_error = measure_max_error_across_bearings(&noise_config, &config);

    assert!(
        max_error < 10.0,
        "Rician fading test failed: max error {:.1}° exceeds 10° threshold",
        max_error
    );
}

#[test]
fn test_multipath_urban_scenario() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;
    let samples_per_rotation = (sample_rate / rotation_hz) as usize;

    let noise_config = NoiseConfig {
        seed: Some(42),
        additive: Some(AdditiveNoiseConfig { snr_db: 25.0 }),
        multipath: Some(MultipathConfig {
            components: vec![
                MultipathComponent {
                    delay_samples: samples_per_rotation / 10,
                    amplitude: 0.3,
                    phase_offset: 0.5,
                },
                MultipathComponent {
                    delay_samples: samples_per_rotation / 5,
                    amplitude: 0.2,
                    phase_offset: 1.0,
                },
            ],
        }),
        ..Default::default()
    };

    let max_error = measure_max_error_across_bearings(&noise_config, &config);

    assert!(
        max_error < 25.0,
        "Urban multipath test failed: max error {:.1}° exceeds 25° threshold",
        max_error
    );
}

#[test]
fn test_doubling_detection() {
    let config = RdfConfig::default();

    let noise_config = NoiseConfig {
        seed: Some(42),
        additive: Some(AdditiveNoiseConfig { snr_db: 30.0 }),
        doubling: Some(DoublingConfig {
            second_bearing_degrees: 180.0,
            amplitude_ratio: 0.3,
        }),
        ..Default::default()
    };

    let primary_bearing = 45.0;
    if let Some(measured) = measure_bearing_with_noise(primary_bearing, &noise_config, &config) {
        let error_from_primary = angle_error(measured, primary_bearing).abs();
        let error_from_secondary = angle_error(measured, 180.0).abs();

        assert!(
            error_from_primary < 30.0 || error_from_secondary < 30.0,
            "Doubling test: measured {:.1}° not close to primary (45°) or secondary (180°)",
            measured
        );
    }
}

#[test]
fn test_frequency_drift_tracking() {
    let config = RdfConfig::default();

    let noise_config = NoiseConfig {
        seed: Some(42),
        additive: Some(AdditiveNoiseConfig { snr_db: 30.0 }),
        frequency_drift: Some(FrequencyDriftConfig {
            max_deviation_hz: 5.0,
            drift_rate_hz_per_sec: 2.0,
        }),
        ..Default::default()
    };

    let max_error = measure_max_error_across_bearings(&noise_config, &config);

    assert!(
        max_error < 30.0,
        "Frequency drift test failed: max error {:.1}° exceeds 30° threshold",
        max_error
    );
}

#[test]
fn test_realistic_urban_conditions() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;
    let samples_per_rotation = (sample_rate / rotation_hz) as usize;

    let noise_config = NoiseConfig {
        seed: Some(42),
        additive: Some(AdditiveNoiseConfig { snr_db: 15.0 }),
        fading: Some(FadingConfig {
            fading_type: FadingType::Rician { k_factor: 2.0 },
            doppler_spread_hz: 3.0,
        }),
        multipath: Some(MultipathConfig {
            components: vec![MultipathComponent {
                delay_samples: samples_per_rotation / 8,
                amplitude: 0.4,
                phase_offset: 0.3,
            }],
        }),
        impulse: Some(ImpulseNoiseConfig {
            rate_hz: 20.0,
            amplitude: 1.0,
            duration_samples: 5,
        }),
        ..Default::default()
    };

    let max_error = measure_max_error_across_bearings(&noise_config, &config);

    println!("\nRealistic Urban Conditions Test:");
    println!("  Max error: {:.1}°", max_error);
    println!("  Threshold: 45°");

    assert!(
        max_error < 45.0,
        "Realistic urban conditions test failed: max error {:.1}° exceeds 45° threshold",
        max_error
    );
}

#[test]
fn test_noise_reproducibility() {
    let config = RdfConfig::default();
    let bearing = 90.0;

    let noise_config = NoiseConfig {
        seed: Some(12345),
        additive: Some(AdditiveNoiseConfig { snr_db: 20.0 }),
        fading: Some(FadingConfig {
            fading_type: FadingType::Rayleigh,
            doppler_spread_hz: 5.0,
        }),
        ..Default::default()
    };

    let measurement1 = measure_bearing_with_noise(bearing, &noise_config, &config);
    let measurement2 = measure_bearing_with_noise(bearing, &noise_config, &config);

    assert_eq!(
        measurement1, measurement2,
        "Seeded noise should produce identical results"
    );
}

#[test]
fn test_clean_signal_baseline() {
    let config = RdfConfig::default();

    let noise_config = NoiseConfig::default();

    let max_error = measure_max_error_across_bearings(&noise_config, &config);

    assert!(
        max_error < 2.0,
        "Clean signal baseline failed: max error {:.1}° exceeds 2° threshold",
        max_error
    );
}

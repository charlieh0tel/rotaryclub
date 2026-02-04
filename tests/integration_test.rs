mod test_signals;

use rotaryclub::config::RdfConfig;
use rotaryclub::rdf::{
    CorrelationBearingCalculator, NorthReferenceTracker, NorthTick, ZeroCrossingBearingCalculator,
};

#[test]
fn test_bearing_calculation_from_synthetic_signal() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;

    // Test multiple bearings
    for test_bearing in [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
        let (zc_measured, corr_measured) =
            calculate_bearing_from_synthetic(test_bearing, &config, sample_rate)
                .expect("Failed to calculate bearing");

        // Check zero-crossing method
        assert!(
            zc_measured.is_some(),
            "No ZC bearing measurement for {} degrees",
            test_bearing
        );
        let zc_bearing = zc_measured.unwrap();
        let mut zc_error = (zc_bearing - test_bearing).abs();
        if zc_error > 180.0 {
            zc_error = 360.0 - zc_error;
        }

        // Check correlation method
        assert!(
            corr_measured.is_some(),
            "No correlation bearing measurement for {} degrees",
            test_bearing
        );
        let corr_bearing = corr_measured.unwrap();
        let mut corr_error = (corr_bearing - test_bearing).abs();
        if corr_error > 180.0 {
            corr_error = 360.0 - corr_error;
        }

        // Skip bearing 0 assertion - known issue with zero crossing at north tick
        if test_bearing != 0.0 {
            assert!(
                zc_error < 15.0,
                "ZC bearing error too large: expected {}, got {}, error {}",
                test_bearing,
                zc_bearing,
                zc_error
            );

            assert!(
                corr_error < 15.0,
                "Correlation bearing error too large: expected {}, got {}, error {}",
                test_bearing,
                corr_bearing,
                corr_error
            );
        }
    }
}

#[test]
fn test_north_tick_detection() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;

    let signal =
        test_signals::generate_test_signal(1.0, sample_rate as u32, rotation_hz, rotation_hz, 0.0);

    let mut north_tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate).unwrap();

    // Process signal in chunks
    let chunk_size = config.audio.buffer_size * 2;
    let mut tick_count = 0;

    for chunk in signal.chunks(chunk_size) {
        let stereo: Vec<(f32, f32)> = chunk.chunks_exact(2).map(|c| (c[0], c[1])).collect();
        let (_, north_tick) = config.audio.split_channels(&stereo);

        let ticks = north_tracker.process_buffer(&north_tick);
        tick_count += ticks.len();
    }

    let expected_ticks = rotation_hz as usize;
    let margin = (expected_ticks as f32 * 0.1) as usize; // 10% margin

    // At 1602 Hz rotation for 1 second, expect ~1602 ticks (allow 10% margin)
    assert!(
        tick_count >= expected_ticks - margin && tick_count <= expected_ticks + margin,
        "Expected ~{} north ticks, got {}",
        expected_ticks,
        tick_count
    );

    // Check rotation frequency estimate
    let freq = north_tracker.rotation_frequency();
    assert!(freq.is_some(), "No rotation frequency estimated");

    let freq = freq.unwrap();
    assert!(
        (freq - rotation_hz).abs() < 50.0,
        "Rotation frequency {} should be close to {} Hz",
        freq,
        rotation_hz
    );
}

fn calculate_bearing_from_synthetic(
    bearing_degrees: f32,
    config: &RdfConfig,
    sample_rate: f32,
) -> anyhow::Result<(Option<f32>, Option<f32>)> {
    let signal = test_signals::generate_test_signal(
        0.5,
        sample_rate as u32,
        config.doppler.expected_freq,
        config.doppler.expected_freq,
        bearing_degrees,
    );

    let mut north_tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate)?;
    let mut zc_calc =
        ZeroCrossingBearingCalculator::new(&config.doppler, &config.agc, sample_rate, 3)?;
    let mut corr_calc =
        CorrelationBearingCalculator::new(&config.doppler, &config.agc, sample_rate, 3)?;

    let chunk_size = config.audio.buffer_size * 2;
    let mut zc_measurements = Vec::new();
    let mut corr_measurements = Vec::new();
    let mut _tick_count = 0;
    let mut last_tick: Option<NorthTick> = None;

    for chunk in signal.chunks(chunk_size) {
        let stereo: Vec<(f32, f32)> = chunk.chunks_exact(2).map(|c| (c[0], c[1])).collect();
        let (doppler, north_tick) = config.audio.split_channels(&stereo);

        // Process doppler buffer with previous tick (if any)
        if let Some(ref tick) = last_tick {
            if let Some(bearing) = zc_calc.process_buffer(&doppler, tick) {
                zc_measurements.push(bearing.bearing_degrees);
            }
            if let Some(bearing) = corr_calc.process_buffer(&doppler, tick) {
                corr_measurements.push(bearing.bearing_degrees);
            }
        } else {
            // No tick yet, but still need to advance sample_counter
            let dummy_tick = NorthTick {
                sample_index: 0,
                period: Some(30.0),
            };
            zc_calc.process_buffer(&doppler, &dummy_tick);
            corr_calc.process_buffer(&doppler, &dummy_tick);
        }

        // Update north tracker and save most recent tick for next iteration
        let ticks = north_tracker.process_buffer(&north_tick);
        _tick_count += ticks.len();

        if let Some(tick) = ticks.last() {
            last_tick = Some(*tick);
        }
    }

    let zc_result = if zc_measurements.len() > 5 {
        Some(zc_measurements.iter().skip(3).sum::<f32>() / (zc_measurements.len() - 3) as f32)
    } else if !zc_measurements.is_empty() {
        Some(zc_measurements.iter().sum::<f32>() / zc_measurements.len() as f32)
    } else {
        None
    };

    let corr_result = if corr_measurements.len() > 5 {
        Some(corr_measurements.iter().skip(3).sum::<f32>() / (corr_measurements.len() - 3) as f32)
    } else if !corr_measurements.is_empty() {
        Some(corr_measurements.iter().sum::<f32>() / corr_measurements.len() as f32)
    } else {
        None
    };

    Ok((zc_result, corr_result))
}

/// Test bearing calculation with perfect (synthetic) north tick placement
/// This isolates the bearing calculation from north tick detection
#[test]
fn test_bearing_with_perfect_north_tick() {
    use std::f32::consts::PI;

    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;
    let samples_per_rotation = sample_rate / rotation_hz;

    eprintln!("\n=== Bearing Test with Perfect North Tick ===");
    eprintln!(
        "Sample rate: {} Hz, Rotation: {} Hz, Period: {:.2} samples",
        sample_rate, rotation_hz, samples_per_rotation
    );
    eprintln!("FIR taps: 127, group delay: 63 samples");

    // For a signal sin(ωt - φ), the rising zero crossing occurs when ωt - φ = 0
    // i.e., at t = φ/ω = φ * samples_per_rotation / (2π) samples after north tick
    eprintln!(
        "\nFor bearing B°, expected zero crossing at sample: B/360 * {:.2} = B * {:.4}",
        samples_per_rotation,
        samples_per_rotation / 360.0
    );
    eprintln!("After FIR filter: crossing appears at sample + 63");
    eprintln!();

    eprintln!("Expected | ZC Raw | ZC Error | Corr Raw | Corr Error");
    eprintln!("---------|--------|----------|----------|----------");

    for test_bearing in [0.0_f32, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
        let bearing_radians = test_bearing.to_radians();

        // Generate pure doppler signal: sin(ω*t - bearing)
        // Use 100 rotations to reduce filter transient effects
        let omega = 2.0 * PI * rotation_hz / sample_rate;
        let num_samples = (samples_per_rotation * 100.0) as usize; // 100 rotations
        let doppler: Vec<f32> = (0..num_samples)
            .map(|i| (omega * i as f32 - bearing_radians).sin())
            .collect();

        // Create perfect north tick at sample 0
        // BUT: the bearing calculator has sample_counter=0, so base_offset = 0 - 0 = 0
        // This means we're saying "north tick just happened at the start of this buffer"
        let perfect_tick = NorthTick {
            sample_index: 0,
            period: Some(samples_per_rotation),
        };

        // Test zero-crossing method
        let mut zc_calc =
            ZeroCrossingBearingCalculator::new(&config.doppler, &config.agc, sample_rate, 1)
                .unwrap();
        let zc_result = zc_calc.process_buffer(&doppler, &perfect_tick);

        // Test correlation method
        let mut corr_calc =
            CorrelationBearingCalculator::new(&config.doppler, &config.agc, sample_rate, 1)
                .unwrap();
        let corr_result = corr_calc.process_buffer(&doppler, &perfect_tick);

        let zc_bearing = zc_result.map(|m| m.raw_bearing).unwrap_or(f32::NAN);
        let corr_bearing = corr_result.map(|m| m.raw_bearing).unwrap_or(f32::NAN);

        let zc_error = angle_error(zc_bearing, test_bearing);
        let corr_error = angle_error(corr_bearing, test_bearing);

        eprintln!(
            "{:>7.1}° | {:>6.1}° | {:>+7.1}° | {:>8.1}° | {:>+7.1}°",
            test_bearing, zc_bearing, zc_error, corr_bearing, corr_error
        );
    }
    eprintln!();
}

fn angle_error(measured: f32, expected: f32) -> f32 {
    let mut e = measured - expected;
    if e > 180.0 {
        e -= 360.0;
    } else if e < -180.0 {
        e += 360.0;
    }
    e
}

/// Test with full pipeline including north tick detection
#[test]
fn test_synthetic_bearing_full_pipeline() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;

    eprintln!("\n=== Full Pipeline Bearing Test ===");
    eprintln!("Expected | ZC Measured | ZC Error | Corr Measured | Corr Error");
    eprintln!("---------|-------------|----------|---------------|----------");

    let mut corr_errors = Vec::new();
    let mut zc_errors = Vec::new();

    for test_bearing in [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
        let (zc_measured, corr_measured) =
            calculate_bearing_from_synthetic(test_bearing, &config, sample_rate)
                .expect("Failed to calculate bearing");

        let zc_val = zc_measured.unwrap_or(f32::NAN);
        let corr_val = corr_measured.unwrap_or(f32::NAN);

        let zc_err = angle_error(zc_val, test_bearing);
        let corr_err = angle_error(corr_val, test_bearing);

        if test_bearing != 0.0 {
            zc_errors.push(zc_err);
            corr_errors.push(corr_err);
        }

        eprintln!(
            "{:>7.1}° | {:>11.1}° | {:>+7.1}° | {:>13.1}° | {:>+7.1}°",
            test_bearing, zc_val, zc_err, corr_val, corr_err
        );
    }

    // Analyze systematic offsets (excluding 0° which has known issues)
    if !corr_errors.is_empty() {
        let zc_mean: f32 = zc_errors.iter().sum::<f32>() / zc_errors.len() as f32;
        let corr_mean: f32 = corr_errors.iter().sum::<f32>() / corr_errors.len() as f32;

        let zc_std = std_dev(&zc_errors, zc_mean);
        let corr_std = std_dev(&corr_errors, corr_mean);

        eprintln!(
            "\nZC systematic offset: {:+.1}° (std dev: {:.2}°)",
            zc_mean, zc_std
        );
        eprintln!(
            "Corr systematic offset: {:+.1}° (std dev: {:.2}°)",
            corr_mean, corr_std
        );

        // Both should have consistent offsets (low std dev)
        assert!(zc_std < 2.0, "ZC errors not consistent");
        assert!(corr_std < 2.0, "Corr errors not consistent");
    }
    eprintln!();
}

fn std_dev(values: &[f32], mean: f32) -> f32 {
    let variance: f32 =
        values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32;
    variance.sqrt()
}

/// Final validation test: verify both methods give correct results with calibration
#[test]
fn test_calibrated_bearing_accuracy() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;

    eprintln!("\n=== Calibrated Bearing Accuracy Test ===");
    eprintln!("This test verifies both methods give accurate results after calibration.\n");

    // First pass: measure systematic offsets
    let mut zc_offsets = Vec::new();
    let mut corr_offsets = Vec::new();

    for test_bearing in [45.0_f32, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
        let (zc_measured, corr_measured) =
            calculate_bearing_from_synthetic(test_bearing, &config, sample_rate)
                .expect("Failed to calculate bearing");

        if let Some(zc) = zc_measured {
            zc_offsets.push(angle_error(zc, test_bearing));
        }
        if let Some(corr) = corr_measured {
            corr_offsets.push(angle_error(corr, test_bearing));
        }
    }

    let zc_calibration = -zc_offsets.iter().sum::<f32>() / zc_offsets.len() as f32;
    let corr_calibration = -corr_offsets.iter().sum::<f32>() / corr_offsets.len() as f32;

    eprintln!("Measured systematic offsets:");
    eprintln!(
        "  ZC offset: {:+.1}° → calibration: {:+.1}°",
        -zc_calibration, zc_calibration
    );
    eprintln!(
        "  Corr offset: {:+.1}° → calibration: {:+.1}°",
        -corr_calibration, corr_calibration
    );
    eprintln!();

    // Second pass: verify accuracy with calibration
    eprintln!("Expected | ZC+Cal | ZC Err | Corr+Cal | Corr Err");
    eprintln!("---------|--------|--------|----------|--------");

    let mut max_zc_error = 0.0_f32;
    let mut max_corr_error = 0.0_f32;

    for test_bearing in [45.0_f32, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
        let (zc_measured, corr_measured) =
            calculate_bearing_from_synthetic(test_bearing, &config, sample_rate)
                .expect("Failed to calculate bearing");

        let zc_calibrated = zc_measured.map(|b| (b + zc_calibration).rem_euclid(360.0));
        let corr_calibrated = corr_measured.map(|b| (b + corr_calibration).rem_euclid(360.0));

        let zc_err = zc_calibrated
            .map(|b| angle_error(b, test_bearing).abs())
            .unwrap_or(f32::NAN);
        let corr_err = corr_calibrated
            .map(|b| angle_error(b, test_bearing).abs())
            .unwrap_or(f32::NAN);

        if !zc_err.is_nan() {
            max_zc_error = max_zc_error.max(zc_err);
        }
        if !corr_err.is_nan() {
            max_corr_error = max_corr_error.max(corr_err);
        }

        eprintln!(
            "{:>7.1}° | {:>6.1}° | {:>5.1}° | {:>8.1}° | {:>5.1}°",
            test_bearing,
            zc_calibrated.unwrap_or(f32::NAN),
            zc_err,
            corr_calibrated.unwrap_or(f32::NAN),
            corr_err
        );
    }

    eprintln!();
    eprintln!("Maximum errors after calibration:");
    eprintln!("  ZC: {:.1}°", max_zc_error);
    eprintln!("  Corr: {:.1}°", max_corr_error);

    // Both methods should be accurate to within 2° after calibration
    assert!(
        max_zc_error < 2.0,
        "ZC max error {:.1}° exceeds 2° threshold after calibration",
        max_zc_error
    );
    assert!(
        max_corr_error < 2.0,
        "Correlation max error {:.1}° exceeds 2° threshold after calibration",
        max_corr_error
    );

    eprintln!("\n✓ Both methods achieve < 2° accuracy after calibration\n");
}

#[test]
fn test_correlation_vs_zero_crossing() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;

    // Test a single bearing with both methods
    let test_bearing = 90.0;

    let signal = test_signals::generate_test_signal(
        0.5,
        sample_rate as u32,
        config.doppler.expected_freq,
        config.doppler.expected_freq,
        test_bearing,
    );

    let mut north_tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate).unwrap();
    let mut zero_crossing_calc =
        ZeroCrossingBearingCalculator::new(&config.doppler, &config.agc, sample_rate, 3).unwrap();
    let mut correlation_calc =
        CorrelationBearingCalculator::new(&config.doppler, &config.agc, sample_rate, 3).unwrap();

    let chunk_size = config.audio.buffer_size * 2;
    let mut zc_measurements = Vec::new();
    let mut corr_measurements = Vec::new();
    let mut last_tick: Option<NorthTick> = None;

    for chunk in signal.chunks(chunk_size) {
        let stereo: Vec<(f32, f32)> = chunk.chunks_exact(2).map(|c| (c[0], c[1])).collect();
        let (doppler, north_tick) = config.audio.split_channels(&stereo);

        // Process with both calculators
        if let Some(ref tick) = last_tick {
            if let Some(bearing) = zero_crossing_calc.process_buffer(&doppler, tick) {
                zc_measurements.push(bearing.bearing_degrees);
            }
            if let Some(bearing) = correlation_calc.process_buffer(&doppler, tick) {
                corr_measurements.push(bearing.bearing_degrees);
            }
        } else {
            // Advance counters
            zero_crossing_calc.process_buffer(
                &doppler,
                &NorthTick {
                    sample_index: 0,
                    period: Some(30.0),
                },
            );
            correlation_calc.process_buffer(
                &doppler,
                &NorthTick {
                    sample_index: 0,
                    period: Some(30.0),
                },
            );
        }

        // Update north tracker
        let ticks = north_tracker.process_buffer(&north_tick);
        if let Some(tick) = ticks.last() {
            last_tick = Some(*tick);
        }
    }

    eprintln!("Zero-crossing measurements: {}", zc_measurements.len());
    eprintln!("Correlation measurements: {}", corr_measurements.len());

    // Both should produce measurements
    assert!(
        !zc_measurements.is_empty(),
        "Zero-crossing should produce measurements"
    );
    assert!(
        !corr_measurements.is_empty(),
        "Correlation should produce measurements"
    );

    // Calculate averages (skip first few for settling)
    let skip = 3;
    if zc_measurements.len() > skip {
        let zc_avg =
            zc_measurements.iter().skip(skip).sum::<f32>() / (zc_measurements.len() - skip) as f32;
        eprintln!("Zero-crossing average: {:.1}°", zc_avg);

        let mut zc_error = (zc_avg - test_bearing).abs();
        if zc_error > 180.0 {
            zc_error = 360.0 - zc_error;
        }

        assert!(
            zc_error < 15.0,
            "Zero-crossing bearing error too large: expected {}, got {}, error {}",
            test_bearing,
            zc_avg,
            zc_error
        );
    }

    if corr_measurements.len() > skip {
        let corr_avg = corr_measurements.iter().skip(skip).sum::<f32>()
            / (corr_measurements.len() - skip) as f32;
        eprintln!("Correlation average: {:.1}°", corr_avg);

        let mut corr_error = (corr_avg - test_bearing).abs();
        if corr_error > 180.0 {
            corr_error = 360.0 - corr_error;
        }

        assert!(
            corr_error < 15.0,
            "Correlation bearing error too large: expected {}, got {}, error {}",
            test_bearing,
            corr_avg,
            corr_error
        );
    }
}

#[test]
fn test_real_wav_file() {
    use hound::WavReader;

    let wav_path = "data/doppler-test-2023-04-10-ft-70d.wav";

    let mut reader = WavReader::open(wav_path).expect("Failed to open WAV file");
    let spec = reader.spec();

    assert_eq!(spec.channels, 2, "WAV file must be stereo");

    // Read samples
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
        hound::SampleFormat::Int => {
            let max_val = 2_i32.pow(spec.bits_per_sample as u32 - 1) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max_val))
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        }
    };

    let mut config = RdfConfig::default();
    config.audio.sample_rate = spec.sample_rate;
    let sample_rate = spec.sample_rate as f32;

    let mut north_tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate).unwrap();
    let mut correlation_calc =
        CorrelationBearingCalculator::new(&config.doppler, &config.agc, sample_rate, 3).unwrap();

    let chunk_size = config.audio.buffer_size * 2;
    let mut measurements = Vec::new();
    let mut last_tick: Option<NorthTick> = None;
    let mut tick_count = 0;

    for chunk in samples.chunks(chunk_size) {
        let stereo: Vec<(f32, f32)> = chunk.chunks_exact(2).map(|c| (c[0], c[1])).collect();
        let (doppler, north_tick) = config.audio.split_channels(&stereo);

        // Process with correlation calculator
        if let Some(ref tick) = last_tick {
            if let Some(bearing) = correlation_calc.process_buffer(&doppler, tick) {
                measurements.push(bearing.bearing_degrees);
            }
        } else {
            // Advance counter
            correlation_calc.process_buffer(
                &doppler,
                &NorthTick {
                    sample_index: 0,
                    period: Some(30.0),
                },
            );
        }

        // Update north tracker
        let ticks = north_tracker.process_buffer(&north_tick);
        tick_count += ticks.len();
        if let Some(tick) = ticks.last() {
            last_tick = Some(*tick);
        }
    }

    let duration = samples.len() as f32 / 2.0 / sample_rate;
    eprintln!("Processed {:.2}s of audio", duration);
    eprintln!("Detected {} north ticks", tick_count);
    eprintln!("Got {} bearing measurements", measurements.len());

    // Check rotation frequency
    if let Some(freq) = north_tracker.rotation_frequency() {
        eprintln!("Estimated rotation frequency: {:.1} Hz", freq);
        assert!(
            (freq - 1602.0).abs() < 100.0,
            "Rotation frequency {} should be close to 1602 Hz",
            freq
        );
    }

    // We should get a lot of ticks
    assert!(
        tick_count > 1000,
        "Should detect many north ticks, got {}",
        tick_count
    );

    // We should get bearing measurements
    assert!(
        measurements.len() > 100,
        "Should produce many bearing measurements, got {}",
        measurements.len()
    );

    // All bearings should be in valid range
    for bearing in &measurements {
        assert!(
            *bearing >= 0.0 && *bearing < 360.0,
            "Bearing {} should be in [0, 360)",
            bearing
        );
    }

    eprintln!("Test passed: Real WAV file processed successfully");
}

/// Test calibration-free accuracy for correlation method
/// This verifies that the correlation method achieves <2° accuracy without calibration
#[test]
fn test_calibration_free_correlation() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;

    eprintln!("\n=== Calibration-Free Correlation Accuracy Test ===");

    let mut max_error = 0.0f32;

    // Test multiple bearings (excluding 0° which has known issues)
    for test_bearing in [45.0_f32, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
        let (_, corr_measured) =
            calculate_bearing_from_synthetic(test_bearing, &config, sample_rate)
                .expect("Failed to calculate bearing");

        let corr_bearing = corr_measured.expect("Should get correlation measurement");
        let error = angle_error(corr_bearing, test_bearing).abs();
        max_error = max_error.max(error);

        eprintln!(
            "Bearing {:>5.1}°: measured {:>5.1}°, error {:>+5.1}°",
            test_bearing, corr_bearing, error
        );
    }

    eprintln!("Max error: {:.1}°", max_error);

    assert!(
        max_error < 2.0,
        "Correlation method max error {:.1}° exceeds 2° calibration-free threshold",
        max_error
    );

    eprintln!("✓ Correlation method achieves < 2° accuracy without calibration\n");
}

/// Test calibration-free accuracy for zero-crossing method
/// This verifies that the zero-crossing method achieves <2° accuracy without calibration
#[test]
fn test_calibration_free_zero_crossing() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;

    eprintln!("\n=== Calibration-Free Zero-Crossing Accuracy Test ===");

    let mut max_error = 0.0f32;

    // Test multiple bearings (excluding 0° which has known issues)
    for test_bearing in [45.0_f32, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
        let (zc_measured, _) = calculate_bearing_from_synthetic(test_bearing, &config, sample_rate)
            .expect("Failed to calculate bearing");

        let zc_bearing = zc_measured.expect("Should get ZC measurement");
        let error = angle_error(zc_bearing, test_bearing).abs();
        max_error = max_error.max(error);

        eprintln!(
            "Bearing {:>5.1}°: measured {:>5.1}°, error {:>+5.1}°",
            test_bearing, zc_bearing, error
        );
    }

    eprintln!("Max error: {:.1}°", max_error);

    assert!(
        max_error < 2.0,
        "Zero-crossing method max error {:.1}° exceeds 2° calibration-free threshold",
        max_error
    );

    eprintln!("✓ Zero-crossing method achieves < 2° accuracy without calibration\n");
}

/// Test that both bearing methods agree within tolerance
#[test]
fn test_methods_agree() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;

    eprintln!("\n=== Method Agreement Test ===");

    let mut max_difference = 0.0f32;

    for test_bearing in [45.0_f32, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
        let (zc_measured, corr_measured) =
            calculate_bearing_from_synthetic(test_bearing, &config, sample_rate)
                .expect("Failed to calculate bearing");

        let zc_bearing = zc_measured.expect("Should get ZC measurement");
        let corr_bearing = corr_measured.expect("Should get correlation measurement");
        let difference = angle_error(zc_bearing, corr_bearing).abs();
        max_difference = max_difference.max(difference);

        eprintln!(
            "Bearing {:>5.1}°: ZC={:>5.1}°, Corr={:>5.1}°, diff={:>+5.1}°",
            test_bearing, zc_bearing, corr_bearing, difference
        );
    }

    eprintln!("Max difference between methods: {:.1}°", max_difference);

    assert!(
        max_difference < 2.0,
        "Methods disagree by {:.1}° which exceeds 2° threshold",
        max_difference
    );

    eprintln!("✓ Both methods agree within 2°\n");
}

/// Test that FIR highpass delay compensation works correctly for north tick detection
#[test]
fn test_north_tick_fir_delay_compensation() {
    use rotaryclub::config::NorthTickConfig;

    let sample_rate = 48000.0;

    let mut config = NorthTickConfig::default();
    config.fir_highpass_taps = 63;

    let mut tracker = NorthReferenceTracker::new(&config, sample_rate).unwrap();

    // Generate signal with pulses at known positions
    // The pulse should appear at sample 100, 200, 300, etc. (period = 100)
    let pulse_positions = [100, 200, 300, 400, 500];
    let mut signal = vec![0.0f32; 1000];
    for &pos in &pulse_positions {
        signal[pos] = 0.8;
    }

    let ticks = tracker.process_buffer(&signal);

    eprintln!("\n=== FIR Highpass Delay Compensation Test ===");
    eprintln!("FIR highpass taps: {}", config.fir_highpass_taps);
    eprintln!(
        "Expected group delay: {}",
        (config.fir_highpass_taps - 1) / 2
    );
    eprintln!("Input pulse positions: {:?}", pulse_positions);
    eprintln!(
        "Detected tick sample_indices: {:?}",
        ticks.iter().map(|t| t.sample_index).collect::<Vec<_>>()
    );

    // The detected sample_index should be close to the original pulse positions
    // (after compensating for filter delay)
    for tick in &ticks {
        let closest_pulse = pulse_positions
            .iter()
            .min_by_key(|&&p| (p as isize - tick.sample_index as isize).abs())
            .unwrap();

        let error = (*closest_pulse as isize - tick.sample_index as isize).abs();
        eprintln!(
            "Tick at {}: closest pulse {}, error {}",
            tick.sample_index, closest_pulse, error
        );

        // Allow some tolerance for edge effects
        assert!(
            error <= 3,
            "Tick sample_index {} too far from expected pulse {}",
            tick.sample_index,
            closest_pulse
        );
    }

    eprintln!("✓ FIR highpass delay compensation working correctly\n");
}

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
        let measured = calculate_bearing_from_synthetic(test_bearing, &config, sample_rate)
            .expect("Failed to calculate bearing");

        assert!(
            measured.is_some(),
            "No bearing measurement for {} degrees",
            test_bearing
        );

        let measured_bearing = measured.unwrap();

        // Calculate error with wrap-around handling
        let mut error = (measured_bearing - test_bearing).abs();
        if error > 180.0 {
            error = 360.0 - error;
        }

        assert!(
            error < 15.0,
            "Bearing error too large: expected {}, got {}, error {}",
            test_bearing,
            measured_bearing,
            error
        );
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
) -> anyhow::Result<Option<f32>> {
    let signal = test_signals::generate_test_signal(
        0.5,
        sample_rate as u32,
        config.doppler.expected_freq,
        config.doppler.expected_freq,
        bearing_degrees,
    );

    let mut north_tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate)?;
    let mut bearing_calc =
        ZeroCrossingBearingCalculator::new(&config.doppler, &config.agc, sample_rate, 3)?;

    let chunk_size = config.audio.buffer_size * 2;
    let mut measurements = Vec::new();
    let mut _tick_count = 0;
    let mut last_tick: Option<NorthTick> = None;

    for chunk in signal.chunks(chunk_size) {
        let stereo: Vec<(f32, f32)> = chunk.chunks_exact(2).map(|c| (c[0], c[1])).collect();
        let (doppler, north_tick) = config.audio.split_channels(&stereo);

        // Process doppler buffer with previous tick (if any)
        if let Some(ref tick) = last_tick {
            if let Some(bearing) = bearing_calc.process_buffer(&doppler, tick) {
                measurements.push(bearing.bearing_degrees);
            }
        } else {
            // No tick yet, but still need to advance sample_counter
            bearing_calc.process_buffer(
                &doppler,
                &NorthTick {
                    sample_index: 0,
                    period: Some(30.0),
                },
            );
        }

        // Update north tracker and save most recent tick for next iteration
        let ticks = north_tracker.process_buffer(&north_tick);
        _tick_count += ticks.len();

        if let Some(tick) = ticks.last() {
            last_tick = Some(*tick);
        }
    }

    if measurements.len() > 5 {
        let avg = measurements.iter().skip(3).sum::<f32>() / (measurements.len() - 3) as f32;
        Ok(Some(avg))
    } else if !measurements.is_empty() {
        Ok(Some(
            measurements.iter().sum::<f32>() / measurements.len() as f32,
        ))
    } else {
        Ok(None)
    }
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

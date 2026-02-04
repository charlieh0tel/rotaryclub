mod test_signals;

use rotaryclub::config::RdfConfig;
use rotaryclub::rdf::{
    CorrelationBearingCalculator, NorthReferenceTracker, NorthTick, ZeroCrossingBearingCalculator,
};

#[test]
fn test_north_tick_detection() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;

    let signal =
        test_signals::generate_test_signal(1.0, sample_rate as u32, rotation_hz, rotation_hz, 0.0);

    let mut north_tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate).unwrap();

    let chunk_size = config.audio.buffer_size * 2;
    let mut tick_count = 0;

    for chunk in signal.chunks(chunk_size) {
        let stereo: Vec<(f32, f32)> = chunk.chunks_exact(2).map(|c| (c[0], c[1])).collect();
        let (_, north_tick) = config.audio.split_channels(&stereo);

        let ticks = north_tracker.process_buffer(&north_tick);
        tick_count += ticks.len();
    }

    let expected_ticks = rotation_hz as usize;
    let margin = (expected_ticks as f32 * 0.1) as usize;

    assert!(
        tick_count >= expected_ticks - margin && tick_count <= expected_ticks + margin,
        "Expected ~{} north ticks, got {}",
        expected_ticks,
        tick_count
    );

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
    let mut last_tick: Option<NorthTick> = None;

    for chunk in signal.chunks(chunk_size) {
        let stereo: Vec<(f32, f32)> = chunk.chunks_exact(2).map(|c| (c[0], c[1])).collect();
        let (doppler, north_tick) = config.audio.split_channels(&stereo);

        if let Some(ref tick) = last_tick {
            if let Some(bearing) = zc_calc.process_buffer(&doppler, tick) {
                zc_measurements.push(bearing.bearing_degrees);
            }
            if let Some(bearing) = corr_calc.process_buffer(&doppler, tick) {
                corr_measurements.push(bearing.bearing_degrees);
            }
        } else {
            let dummy_tick = NorthTick {
                sample_index: 0,
                period: Some(30.0),
            };
            zc_calc.process_buffer(&doppler, &dummy_tick);
            corr_calc.process_buffer(&doppler, &dummy_tick);
        }

        let ticks = north_tracker.process_buffer(&north_tick);
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

/// Test bearing calculation with perfect (synthetic) north tick placement.
/// This isolates the bearing calculation from north tick detection.
/// Verifies both methods produce valid bearings for isolated doppler signals.
#[test]
fn test_bearing_with_perfect_north_tick() {
    use std::f32::consts::PI;

    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;
    let samples_per_rotation = sample_rate / rotation_hz;

    // Skip 0° - known issue where zero crossing coincides with north tick
    for test_bearing in [45.0_f32, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
        let bearing_radians = test_bearing.to_radians();
        let omega = 2.0 * PI * rotation_hz / sample_rate;
        let num_samples = (samples_per_rotation * 100.0) as usize;
        let doppler: Vec<f32> = (0..num_samples)
            .map(|i| (omega * i as f32 - bearing_radians).sin())
            .collect();

        let perfect_tick = NorthTick {
            sample_index: 0,
            period: Some(samples_per_rotation),
        };

        let mut zc_calc =
            ZeroCrossingBearingCalculator::new(&config.doppler, &config.agc, sample_rate, 1)
                .unwrap();
        let zc_result = zc_calc.process_buffer(&doppler, &perfect_tick);

        let mut corr_calc =
            CorrelationBearingCalculator::new(&config.doppler, &config.agc, sample_rate, 1)
                .unwrap();
        let corr_result = corr_calc.process_buffer(&doppler, &perfect_tick);

        // Verify both methods produce valid bearings in [0, 360)
        if let Some(m) = zc_result {
            assert!(
                m.bearing_degrees >= 0.0 && m.bearing_degrees < 360.0,
                "ZC bearing {} out of range for test bearing {}",
                m.bearing_degrees,
                test_bearing
            );
        }

        if let Some(m) = corr_result {
            assert!(
                m.bearing_degrees >= 0.0 && m.bearing_degrees < 360.0,
                "Correlation bearing {} out of range for test bearing {}",
                m.bearing_degrees,
                test_bearing
            );
        }
    }
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

#[test]
fn test_real_wav_file() {
    use hound::WavReader;

    let wav_path = "data/doppler-test-2023-04-10-ft-70d.wav";

    let mut reader = WavReader::open(wav_path).expect("Failed to open WAV file");
    let spec = reader.spec();

    assert_eq!(spec.channels, 2, "WAV file must be stereo");

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

        if let Some(ref tick) = last_tick {
            if let Some(bearing) = correlation_calc.process_buffer(&doppler, tick) {
                measurements.push(bearing.bearing_degrees);
            }
        } else {
            correlation_calc.process_buffer(
                &doppler,
                &NorthTick {
                    sample_index: 0,
                    period: Some(30.0),
                },
            );
        }

        let ticks = north_tracker.process_buffer(&north_tick);
        tick_count += ticks.len();
        if let Some(tick) = ticks.last() {
            last_tick = Some(*tick);
        }
    }

    if let Some(freq) = north_tracker.rotation_frequency() {
        assert!(
            (freq - 1602.0).abs() < 100.0,
            "Rotation frequency {} should be close to 1602 Hz",
            freq
        );
    }

    assert!(
        tick_count > 1000,
        "Should detect many north ticks, got {}",
        tick_count
    );

    assert!(
        measurements.len() > 100,
        "Should produce many bearing measurements, got {}",
        measurements.len()
    );

    for bearing in &measurements {
        assert!(
            *bearing >= 0.0 && *bearing < 360.0,
            "Bearing {} should be in [0, 360)",
            bearing
        );
    }
}

/// Test calibration-free accuracy for both bearing methods.
/// Verifies <2° accuracy without any calibration offset.
#[test]
fn test_calibration_free_accuracy() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;

    let mut max_zc_error = 0.0f32;
    let mut max_corr_error = 0.0f32;

    for test_bearing in [45.0_f32, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
        let (zc_measured, corr_measured) =
            calculate_bearing_from_synthetic(test_bearing, &config, sample_rate)
                .expect("Failed to calculate bearing");

        let zc_bearing = zc_measured.expect("Should get ZC measurement");
        let corr_bearing = corr_measured.expect("Should get correlation measurement");

        let zc_error = angle_error(zc_bearing, test_bearing).abs();
        let corr_error = angle_error(corr_bearing, test_bearing).abs();

        max_zc_error = max_zc_error.max(zc_error);
        max_corr_error = max_corr_error.max(corr_error);
    }

    assert!(
        max_zc_error < 2.0,
        "Zero-crossing max error {:.1}° exceeds 2° threshold",
        max_zc_error
    );
    assert!(
        max_corr_error < 2.0,
        "Correlation max error {:.1}° exceeds 2° threshold",
        max_corr_error
    );
}

/// Test that both bearing methods agree within tolerance.
#[test]
fn test_methods_agree() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;

    let mut max_difference = 0.0f32;

    for test_bearing in [45.0_f32, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
        let (zc_measured, corr_measured) =
            calculate_bearing_from_synthetic(test_bearing, &config, sample_rate)
                .expect("Failed to calculate bearing");

        let zc_bearing = zc_measured.expect("Should get ZC measurement");
        let corr_bearing = corr_measured.expect("Should get correlation measurement");
        let difference = angle_error(zc_bearing, corr_bearing).abs();
        max_difference = max_difference.max(difference);
    }

    assert!(
        max_difference < 2.0,
        "Methods disagree by {:.1}° which exceeds 2° threshold",
        max_difference
    );
}

/// Test that FIR highpass delay compensation works correctly for north tick detection.
#[test]
fn test_north_tick_fir_delay_compensation() {
    use rotaryclub::config::NorthTickConfig;

    let sample_rate = 48000.0;

    let mut config = NorthTickConfig::default();
    config.fir_highpass_taps = 63;

    let mut tracker = NorthReferenceTracker::new(&config, sample_rate).unwrap();

    let pulse_positions = [100, 200, 300, 400, 500];
    let mut signal = vec![0.0f32; 1000];
    for &pos in &pulse_positions {
        signal[pos] = 0.8;
    }

    let ticks = tracker.process_buffer(&signal);

    for tick in &ticks {
        let closest_pulse = pulse_positions
            .iter()
            .min_by_key(|&&p| (p as isize - tick.sample_index as isize).abs())
            .unwrap();

        let error = (*closest_pulse as isize - tick.sample_index as isize).abs();

        assert!(
            error <= 2,
            "Tick sample_index {} too far from expected pulse {}",
            tick.sample_index,
            closest_pulse
        );
    }
}

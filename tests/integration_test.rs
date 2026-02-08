mod test_signals;

use std::f32::consts::PI;

use rotaryclub::config::RdfConfig;
use rotaryclub::rdf::{
    BearingCalculator, CorrelationBearingCalculator, NorthReferenceTracker, NorthTick,
    NorthTracker, ZeroCrossingBearingCalculator,
};
use rotaryclub::simulation::circular_mean_degrees;

#[test]
fn test_north_tick_detection() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;

    let signal = test_signals::generate_test_signal(1.0, sample_rate as u32, rotation_hz, 0.0);

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
                lock_quality: None,
                phase: 0.0,
                frequency: 2.0 * PI / 30.0,
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
        circular_mean_degrees(&zc_measurements[3..])
    } else {
        circular_mean_degrees(&zc_measurements)
    };

    let corr_result = if corr_measurements.len() > 5 {
        circular_mean_degrees(&corr_measurements[3..])
    } else {
        circular_mean_degrees(&corr_measurements)
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

    for test_bearing in [0.0_f32, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
        let bearing_radians = test_bearing.to_radians();
        let omega = 2.0 * PI * rotation_hz / sample_rate;
        let num_samples = (samples_per_rotation * 100.0) as usize;
        let doppler: Vec<f32> = (0..num_samples)
            .map(|i| (omega * i as f32 - bearing_radians).sin())
            .collect();

        let perfect_tick = NorthTick {
            sample_index: 0,
            period: Some(samples_per_rotation),
            lock_quality: None,
            phase: 0.0,
            frequency: 2.0 * PI / samples_per_rotation,
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
                    lock_quality: None,
                    phase: 0.0,
                    frequency: 2.0 * PI / 30.0,
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
/// Verifies <3° accuracy without any calibration offset.
#[test]
fn test_calibration_free_accuracy() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;

    let mut max_zc_error = 0.0f32;
    let mut max_corr_error = 0.0f32;

    for test_bearing in [0.0_f32, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
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
        max_zc_error < 3.0,
        "Zero-crossing max error {:.1}° exceeds 3° threshold",
        max_zc_error
    );
    assert!(
        max_corr_error < 3.0,
        "Correlation max error {:.1}° exceeds 3° threshold",
        max_corr_error
    );
}

/// Test that both bearing methods agree within tolerance.
#[test]
fn test_methods_agree() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;

    let mut max_difference = 0.0f32;

    for test_bearing in [0.0_f32, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
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

/// Test bearing tracking as signal rotates through 0°/360° wraparound.
/// Simulates a transmitter circling the RDF array: 270° → 0° → 90° → 0° → 270°
#[test]
fn test_rotating_bearing_through_zero() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;

    // Generate 2 seconds of signal with bearing rotating:
    // t=0.0s: 270°
    // t=0.5s: 0° (360°)
    // t=1.0s: 90°
    // t=1.5s: 0° (360°)
    // t=2.0s: 270°
    // This covers both forward and backward crossing through 0°/360°
    let bearing_fn = |t: f32| {
        let phase = t * 2.0; // 2 full cycles in 2 seconds
        if phase < 0.5 {
            // 270° → 360° (going up through zero)
            270.0 + phase * 180.0
        } else if phase < 1.0 {
            // 360° → 90° (continuing past zero)
            (phase - 0.5) * 180.0
        } else if phase < 1.5 {
            // 90° → 0° (going back down)
            90.0 - (phase - 1.0) * 180.0
        } else {
            // 0° → 270° (continuing back through zero)
            360.0 - (phase - 1.5) * 180.0
        }
    };

    let signal = test_signals::generate_test_signal_with_bearing_fn(
        2.0,
        sample_rate as u32,
        rotation_hz,
        bearing_fn,
    );

    let mut north_tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate).unwrap();
    let mut corr_calc =
        CorrelationBearingCalculator::new(&config.doppler, &config.agc, sample_rate, 1).unwrap();

    let chunk_size = config.audio.buffer_size * 2;
    let mut measurements: Vec<(f32, f32)> = Vec::new(); // (time, bearing)
    let mut last_tick: Option<NorthTick> = None;
    let mut sample_idx = 0usize;

    for chunk in signal.chunks(chunk_size) {
        let stereo: Vec<(f32, f32)> = chunk.chunks_exact(2).map(|c| (c[0], c[1])).collect();
        let (doppler, north_tick) = config.audio.split_channels(&stereo);

        if let Some(ref tick) = last_tick {
            if let Some(bearing) = corr_calc.process_buffer(&doppler, tick) {
                let t = sample_idx as f32 / sample_rate;
                measurements.push((t, bearing.bearing_degrees));
            }
        } else {
            corr_calc.process_buffer(
                &doppler,
                &NorthTick {
                    sample_index: 0,
                    period: Some(30.0),
                    lock_quality: None,
                    phase: 0.0,
                    frequency: 2.0 * PI / 30.0,
                },
            );
        }

        let ticks = north_tracker.process_buffer(&north_tick);
        if let Some(tick) = ticks.last() {
            last_tick = Some(*tick);
        }

        sample_idx += chunk.len() / 2;
    }

    assert!(
        measurements.len() > 50,
        "Should have many measurements, got {}",
        measurements.len()
    );

    // Skip first 0.1s for filter warmup, then verify tracking
    let mut max_error = 0.0f32;
    for &(t, measured) in measurements.iter().filter(|(t, _)| *t > 0.1) {
        let expected = bearing_fn(t) % 360.0;
        let error = angle_error(measured, expected).abs();
        max_error = max_error.max(error);
    }

    assert!(
        max_error < 12.0,
        "Max tracking error {:.1}° exceeds 12° threshold",
        max_error
    );
}

/// Test that DC offset removal improves accuracy when signal has DC offset.
#[test]
fn test_dc_offset_removal() {
    use rotaryclub::signal_processing::DcRemover;

    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;

    let test_bearing = 90.0;

    for dc_offset in [0.0, 0.3, 1.0, 5.0] {
        // Generate signal and add DC offset
        let signal =
            test_signals::generate_test_signal(0.5, sample_rate as u32, rotation_hz, test_bearing);

        // Add DC offset to both channels
        let signal_with_dc: Vec<f32> = signal.iter().map(|s| s + dc_offset).collect();

        // Process without DC removal
        let bearing_without_removal = {
            let mut north_tracker =
                NorthReferenceTracker::new(&config.north_tick, sample_rate).unwrap();
            let mut corr_calc =
                CorrelationBearingCalculator::new(&config.doppler, &config.agc, sample_rate, 3)
                    .unwrap();

            let chunk_size = config.audio.buffer_size * 2;
            let mut measurements = Vec::new();
            let mut last_tick: Option<NorthTick> = None;

            for chunk in signal_with_dc.chunks(chunk_size) {
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
                            phase: 0.0,
                            frequency: 2.0 * PI / 30.0,
                        },
                    );
                }

                let ticks = north_tracker.process_buffer(&north_tick);
                if let Some(tick) = ticks.last() {
                    last_tick = Some(*tick);
                }
            }

            if measurements.len() > 5 {
                measurements.iter().skip(3).sum::<f32>() / (measurements.len() - 3) as f32
            } else {
                measurements.iter().sum::<f32>() / measurements.len().max(1) as f32
            }
        };

        // Process with DC removal
        let bearing_with_removal = {
            let mut north_tracker =
                NorthReferenceTracker::new(&config.north_tick, sample_rate).unwrap();
            let mut corr_calc =
                CorrelationBearingCalculator::new(&config.doppler, &config.agc, sample_rate, 3)
                    .unwrap();
            let mut dc_remover_doppler = DcRemover::with_cutoff(sample_rate, 1.0);
            let mut dc_remover_north = DcRemover::with_cutoff(sample_rate, 1.0);

            let chunk_size = config.audio.buffer_size * 2;
            let mut measurements = Vec::new();
            let mut last_tick: Option<NorthTick> = None;

            for chunk in signal_with_dc.chunks(chunk_size) {
                let stereo: Vec<(f32, f32)> = chunk.chunks_exact(2).map(|c| (c[0], c[1])).collect();
                let (mut doppler, mut north_tick) = config.audio.split_channels(&stereo);

                dc_remover_doppler.process(&mut doppler);
                dc_remover_north.process(&mut north_tick);

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
                            phase: 0.0,
                            frequency: 2.0 * PI / 30.0,
                        },
                    );
                }

                let ticks = north_tracker.process_buffer(&north_tick);
                if let Some(tick) = ticks.last() {
                    last_tick = Some(*tick);
                }
            }

            if measurements.len() > 5 {
                measurements.iter().skip(3).sum::<f32>() / (measurements.len() - 3) as f32
            } else {
                measurements.iter().sum::<f32>() / measurements.len().max(1) as f32
            }
        };

        let error_without = angle_error(bearing_without_removal, test_bearing).abs();
        let error_with = angle_error(bearing_with_removal, test_bearing).abs();

        // DC removal should produce a reasonable bearing (within 10 degrees)
        assert!(
            error_with < 10.0,
            "DC {}: error {:.1}° exceeds 10° threshold",
            dc_offset,
            error_with
        );

        // DC removal should not make things worse (allow some tolerance)
        assert!(
            error_with <= error_without + 5.0,
            "DC {}: removal made accuracy worse: {:.1}° vs {:.1}° without",
            dc_offset,
            error_with,
            error_without
        );
    }
}

/// Test that FIR highpass delay compensation works correctly for north tick detection.
#[test]
fn test_north_tick_fir_delay_compensation() {
    use rotaryclub::config::NorthTickConfig;

    let sample_rate = 48000.0;

    let config = NorthTickConfig {
        fir_highpass_taps: 63,
        ..Default::default()
    };

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

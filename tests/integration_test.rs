mod test_signals;

use rotaryclub::config::RdfConfig;
use rotaryclub::rdf::{BearingCalculator, NorthReferenceTracker};

#[test]
#[ignore] // TODO: Fix sample counter alignment issues
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
#[ignore] // TODO: Improve peak detector for rapid pulses
fn test_north_tick_detection() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;

    let signal = test_signals::generate_test_signal(1.0, sample_rate as u32, 500.0, 500.0, 0.0);

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

    // At 500 Hz rotation for 1 second, expect ~500 ticks (allow some margin)
    assert!(
        tick_count >= 450 && tick_count <= 550,
        "Expected ~500 north ticks, got {}",
        tick_count
    );

    // Check rotation frequency estimate
    let freq = north_tracker.rotation_frequency();
    assert!(freq.is_some(), "No rotation frequency estimated");

    let freq = freq.unwrap();
    assert!(
        (freq - 500.0).abs() < 50.0,
        "Rotation frequency {} should be close to 500 Hz",
        freq
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
    let mut bearing_calc = BearingCalculator::new(&config.doppler, &config.agc, sample_rate, 3)?;

    let chunk_size = config.audio.buffer_size * 2;
    let mut measurements = Vec::new();

    for chunk in signal.chunks(chunk_size) {
        let stereo: Vec<(f32, f32)> = chunk.chunks_exact(2).map(|c| (c[0], c[1])).collect();
        let (doppler, north_tick) = config.audio.split_channels(&stereo);

        let ticks = north_tracker.process_buffer(&north_tick);

        for tick in ticks {
            if let Some(bearing) = bearing_calc.process_buffer(&doppler, &tick) {
                measurements.push(bearing.bearing_degrees);
            }
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

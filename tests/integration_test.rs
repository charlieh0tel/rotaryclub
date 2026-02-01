mod test_signals;

use rotaryclub::config::RdfConfig;
use rotaryclub::rdf::{BearingCalculator, NorthReferenceTracker, NorthTick};

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

    let signal = test_signals::generate_test_signal(1.0, sample_rate as u32, rotation_hz, rotation_hz, 0.0);

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
    let mut bearing_calc = BearingCalculator::new(&config.doppler, &config.agc, sample_rate, 3)?;

    let chunk_size = config.audio.buffer_size * 2;
    let mut measurements = Vec::new();
    let mut tick_count = 0;
    let mut last_tick: Option<NorthTick> = None;

    for chunk in signal.chunks(chunk_size) {
        let stereo: Vec<(f32, f32)> = chunk.chunks_exact(2).map(|c| (c[0], c[1])).collect();
        let (doppler, north_tick) = config.audio.split_channels(&stereo);

        // Process doppler buffer with previous tick (if any)
        if let Some(ref tick) = last_tick {
            if let Some(bearing) = bearing_calc.process_buffer(&doppler, tick) {
                measurements.push(bearing.bearing_degrees);
                eprintln!("Got bearing: {}", bearing.bearing_degrees);
            } else {
                eprintln!("Warning: no bearing calculated for tick at sample {}", tick.sample_index);
            }
        } else {
            // No tick yet, but still need to advance sample_counter
            bearing_calc.process_buffer(&doppler, &NorthTick { sample_index: 0, period: Some(30.0) });
        }

        // Update north tracker and save most recent tick for next iteration
        let ticks = north_tracker.process_buffer(&north_tick);
        tick_count += ticks.len();

        if let Some(tick) = ticks.last() {
            last_tick = Some(*tick);
            eprintln!("Tick at sample {} with period {:?}", tick.sample_index, tick.period);
        }
    }

    eprintln!("Bearing {}: ticks={}, measurements={}", bearing_degrees, tick_count, measurements.len());

    if !measurements.is_empty() {
        eprintln!("First 10 measurements: {:?}", &measurements[..measurements.len().min(10)]);
        eprintln!("Last 10 measurements: {:?}", &measurements[measurements.len().saturating_sub(10)..]);
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

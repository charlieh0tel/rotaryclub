use rotaryclub::config::RdfConfig;
use rotaryclub::rdf::{ZeroCrossingBearingCalculator, NorthReferenceTracker};
use std::f32::consts::PI;

fn main() -> anyhow::Result<()> {
    println!("=== Synthetic RDF Signal Test ===\n");

    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;

    // Test multiple bearings
    let test_bearings = [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0];

    println!("Testing bearing calculation with synthetic signals...\n");
    println!(
        "{:<15} {:<15} {:<15} {:<15}",
        "Expected (°)", "Measured (°)", "Error (°)", "Status"
    );
    println!("{}", "-".repeat(65));

    for &expected_bearing in &test_bearings {
        let measured = test_bearing(expected_bearing, &config, sample_rate)?;

        let error = if let Some(measured_bearing) = measured {
            let mut err = (measured_bearing - expected_bearing).abs();
            // Handle wrap-around (e.g., 359° vs 1°)
            if err > 180.0 {
                err = 360.0 - err;
            }

            let status = if err < 10.0 { "PASS" } else { "FAIL" };
            println!(
                "{:<15.1} {:<15.1} {:<15.1} {:<15}",
                expected_bearing, measured_bearing, err, status
            );
            err
        } else {
            println!(
                "{:<15.1} {:<15} {:<15} {:<15}",
                expected_bearing, "N/A", "N/A", "FAIL (no measurement)"
            );
            999.0
        };

        if error > 10.0 {
            println!("  WARNING: Large error detected!");
        }
    }

    println!("\nTest complete.");

    Ok(())
}

fn test_bearing(
    bearing_degrees: f32,
    config: &RdfConfig,
    sample_rate: f32,
) -> anyhow::Result<Option<f32>> {
    // Generate synthetic signal
    let duration_secs = 0.5; // 500ms should be plenty
    let rotation_hz = config.doppler.expected_freq;
    let doppler_hz = config.doppler.expected_freq;

    let signal = generate_test_signal(
        duration_secs,
        sample_rate as u32,
        rotation_hz,
        doppler_hz,
        bearing_degrees,
    );

    // Initialize trackers
    let mut north_tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate)?;
    let mut bearing_calc = ZeroCrossingBearingCalculator::new(&config.doppler, &config.agc, sample_rate, 3)?;

    // Process signal in chunks
    let chunk_size = config.audio.buffer_size * 2; // stereo
    let mut measurements = Vec::new();

    for chunk in signal.chunks(chunk_size) {
        // Convert to stereo samples
        let stereo: Vec<(f32, f32)> = chunk.chunks_exact(2).map(|c| (c[0], c[1])).collect();

        let (doppler, north_tick) = config.audio.split_channels(&stereo);

        // Process north tick
        let ticks = north_tracker.process_buffer(&north_tick);

        // Process doppler with each tick
        for tick in ticks {
            if let Some(bearing) = bearing_calc.process_buffer(&doppler, &tick) {
                measurements.push(bearing.bearing_degrees);
            }
        }
    }

    // Return average of measurements (skip first few for filter settling)
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

fn generate_test_signal(
    duration_secs: f32,
    sample_rate: u32,
    rotation_hz: f32,
    doppler_tone_hz: f32,
    bearing_degrees: f32,
) -> Vec<f32> {
    let num_samples = (duration_secs * sample_rate as f32) as usize;
    let mut samples = Vec::with_capacity(num_samples * 2);

    let bearing_radians = bearing_degrees.to_radians();
    let samples_per_rotation = sample_rate as f32 / rotation_hz;

    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;

        // Left channel: Doppler tone
        let rotation_phase = (i as f32 / samples_per_rotation) * 2.0 * PI;
        let phase_offset = rotation_phase + bearing_radians;
        let doppler = (doppler_tone_hz * t * 2.0 * PI + phase_offset).sin();

        // Right channel: North tick pulse
        let tick_phase = rotation_phase % (2.0 * PI);
        let north_tick = if tick_phase < 0.05 { 0.8 } else { 0.0 };

        samples.push(doppler);
        samples.push(north_tick);
    }

    samples
}
